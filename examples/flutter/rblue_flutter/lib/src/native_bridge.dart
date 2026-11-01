import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io' show Platform;
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import 'ffi_bindings.dart';

/// The seam between the typed `RblueClient` and the C ABI.
///
/// Implementations register the two callbacks (already typed, no raw pointers
/// visible to the client), forward `send`, and manage the handle lifetime.
/// Tests substitute an in-memory implementation with mock-backend semantics;
/// production uses [RealNativeBridge].
abstract interface class NativeBridge {
  /// Constructs the facade (`rblue_new`); null means the backend could not be
  /// constructed on this target.
  ffi.Pointer<RblueHandle>? create();

  /// Registers both callbacks. Passing `null` for one clears that slot. The
  /// registered functions may be invoked from arbitrary native threads.
  void registerCallbacks(
    ffi.Pointer<RblueHandle> handle, {
    void Function(int reqId, String? statusJson, Uint8List payload)? onResponse,
    void Function(String eventJson)? onEvent,
  });

  /// Dispatches one request (`rblue_request`); returns the synchronous
  /// rejection code (0 = accepted).
  int send(
    ffi.Pointer<RblueHandle> handle,
    int reqId,
    String method,
    Map<String, Object?> params,
    Uint8List payload,
  );

  /// Frees the facade (`rblue_free`): no callback fires after this returns.
  void dispose(ffi.Pointer<RblueHandle> handle);

  /// A `NativeFinalizer`-compatible function pointer freeing the handle, or
  /// null when the bridge does not support one (tests).
  ffi.Pointer<ffi.NativeFunction<ffi.Void Function(ffi.Pointer<ffi.Void>)>>? get freeFinalizer;
}

/// Production bridge over the ffigen bindings.
///
/// On Android the symbols live in the `librblue_c.so` the plugin's Kotlin side
/// loaded via `System.loadLibrary`; `DynamicLibrary.open` attaches to the same
/// library. On iOS the static library is force_load-ed into the Runner
/// binary, so the symbols resolve through `DynamicLibrary.process()`.
class RealNativeBridge implements NativeBridge {
  final ffi.DynamicLibrary _lib;
  final RblueFfiBindings _bindings;

  RealNativeBridge._(this._lib, this._bindings);

  static RealNativeBridge? _instance;

  /// The process-wide bridge (the C facade owns a private runtime per handle;
  /// only the symbol table is shared).
  factory RealNativeBridge.instance() {
    final existing = _instance;
    if (existing != null) return existing;
    final lib = _openLibrary();
    return _instance = RealNativeBridge._(lib, RblueFfiBindings(lib));
  }

  static ffi.DynamicLibrary _openLibrary() {
    if (Platform.isAndroid) {
      return ffi.DynamicLibrary.open('librblue_c.so');
    }
    if (Platform.isIOS || Platform.isMacOS) {
      return ffi.DynamicLibrary.process();
    }
    throw UnsupportedError(
      'rblue_flutter: ${Platform.operatingSystem} is not supported by this '
      'example; the C ABI ships for Android and iOS only',
    );
  }

  late final ffi.Pointer<ffi.NativeFunction<ffi.Void Function(ffi.Pointer<RblueHandle>)>>
      _freePtr = _lib.lookup('rblue_free');

  ffi.NativeCallable<rblue_response_cbFunction>? _responseCallable;
  ffi.NativeCallable<rblue_event_cbFunction>? _eventCallable;

  @override
  ffi.Pointer<RblueHandle>? create() {
    final handle = _bindings.rblueNew();
    return handle == ffi.nullptr ? null : handle;
  }

  @override
  void registerCallbacks(
    ffi.Pointer<RblueHandle> handle, {
    void Function(int reqId, String? statusJson, Uint8List payload)? onResponse,
    void Function(String eventJson)? onEvent,
  }) {
    // The callables must outlive the registration: they are kept in fields
    // and closed only with dispose().
    _responseCallable?.close();
    _eventCallable?.close();
    _responseCallable = onResponse == null ? null : _makeResponseCallable(onResponse);
    _eventCallable = onEvent == null ? null : _makeEventCallable(onEvent);
    final responseCode = _bindings.rblueSetResponseCallback(
      handle,
      _responseCallable?.nativeFunction ?? ffi.nullptr,
      ffi.nullptr,
    );
    final eventCode = _bindings.rblueSetEventCallback(
      handle,
      _eventCallable?.nativeFunction ?? ffi.nullptr,
      ffi.nullptr,
    );
    if (responseCode != RBLUE_REQUEST_OK || eventCode != RBLUE_REQUEST_OK) {
      throw StateError('rblue_flutter: callback registration failed ($responseCode/$eventCode)');
    }
  }

  ffi.NativeCallable<rblue_response_cbFunction> _makeResponseCallable(
    void Function(int, String?, Uint8List) onResponse,
  ) => ffi.NativeCallable<rblue_response_cbFunction>.listener(
        (int reqId, ffi.Pointer<ffi.Char> statusJson, ffi.Pointer<ffi.Uint8> payload,
            int payloadLen, ffi.Pointer<ffi.Void> userData) {
          String? status;
          if (statusJson != ffi.nullptr) {
            status = statusJson.cast<Utf8>().toDartString();
            // Ownership contract: the callback copies and returns the string.
            _bindings.rblueFreeString(statusJson);
          }
          final data = payloadLen == 0
              ? Uint8List(0)
              // Copy before returning: the borrow is only valid for the call.
              : Uint8List.fromList(payload.asTypedList(payloadLen));
          onResponse(reqId, status, data);
        },
      );

  ffi.NativeCallable<rblue_event_cbFunction> _makeEventCallable(
    void Function(String) onEvent,
  ) => ffi.NativeCallable<rblue_event_cbFunction>.listener(
        (ffi.Pointer<ffi.Char> eventJson, ffi.Pointer<ffi.Void> userData) {
          final json = eventJson.cast<Utf8>().toDartString();
          _bindings.rblueFreeString(eventJson);
          onEvent(json);
        },
      );

  @override
  int send(
    ffi.Pointer<RblueHandle> handle,
    int reqId,
    String method,
    Map<String, Object?> params,
    Uint8List payload,
  ) {
    final methodPtr = method.toNativeUtf8().cast<ffi.Char>();
    final paramsPtr = jsonEncode(params).toNativeUtf8().cast<ffi.Char>();
    ffi.Pointer<ffi.Uint8> payloadPtr = ffi.nullptr;
    if (payload.isNotEmpty) {
      payloadPtr = malloc.allocate<ffi.Uint8>(payload.length);
      payloadPtr.asTypedList(payload.length).setAll(0, payload);
    }
    try {
      return _bindings.rblueRequest(handle, reqId, methodPtr, paramsPtr, payloadPtr, payload.length);
    } finally {
      malloc.free(methodPtr);
      malloc.free(paramsPtr);
      if (payloadPtr != ffi.nullptr) {
        malloc.free(payloadPtr);
      }
    }
  }

  @override
  void dispose(ffi.Pointer<RblueHandle> handle) {
    _responseCallable?.close();
    _eventCallable?.close();
    _responseCallable = null;
    _eventCallable = null;
    _bindings.rblueFree(handle);
  }

  @override
  ffi.Pointer<ffi.NativeFunction<ffi.Void Function(ffi.Pointer<ffi.Void>)>>? get freeFinalizer =>
      _freePtr.cast();
}

import 'dart:async';
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

  /// Starts deterministic shutdown. Native producers stop synchronously; any
  /// queued ownership transfers drain before the listeners and handle release.
  void dispose(ffi.Pointer<RblueHandle> handle);
}

/// Production bridge over the ffigen bindings.
///
/// On Android the symbols live in the `librblue_c.so` the plugin's Kotlin side
/// loaded via `System.loadLibrary`; `DynamicLibrary.open` attaches to the same
/// library. On Apple platforms the static library is linked into the Runner,
/// so the symbols resolve through `DynamicLibrary.process()`. Windows ships a
/// DLL beside the application executable.
class RealNativeBridge implements NativeBridge {
  final RblueFfiBindings _bindings;

  RealNativeBridge._(this._bindings);

  /// Opens the process library for one client bridge. The operating system
  /// shares the loaded image, while callbacks remain scoped to this bridge and
  /// cannot overwrite another [RblueClient]'s registrations.
  factory RealNativeBridge.instance() {
    final lib = _openLibrary();
    return RealNativeBridge._(RblueFfiBindings(lib));
  }

  /// Creates a bridge over an explicitly loaded rblue-c library.
  ///
  /// Primarily useful for native integration tests and custom embedders that
  /// manage the library location themselves.
  factory RealNativeBridge.fromDynamicLibrary(ffi.DynamicLibrary library) =>
      RealNativeBridge._(RblueFfiBindings(library));

  static ffi.DynamicLibrary _openLibrary() {
    if (Platform.isAndroid) {
      return ffi.DynamicLibrary.open('librblue_c.so');
    }
    if (Platform.isIOS || Platform.isMacOS) {
      return ffi.DynamicLibrary.process();
    }
    if (Platform.isWindows) {
      return ffi.DynamicLibrary.open('rblue_c.dll');
    }
    throw UnsupportedError(
      'rblue_flutter: ${Platform.operatingSystem} is not supported; '
      'packaged targets are Android, iOS, macOS, and Windows',
    );
  }

  final _drained = Completer<void>();
  bool _disposing = false;
  bool _handleFreed = false;
  int _processedResponses = 0;
  int _processedEvents = 0;
  int? _targetResponses;
  int? _targetEvents;
  ffi.Pointer<RblueHandle>? _disposingHandle;
  ffi.NativeCallable<rblue_response_cbFunction>? _responseCallable;
  ffi.NativeCallable<rblue_event_cbFunction>? _eventCallable;

  /// Completes after native production stops, queued callbacks release their
  /// ownership transfers, both listeners close, and the handle is freed.
  Future<void> get drained => _drained.future;

  /// Callback counts observed by Dart. Exposed for shutdown diagnostics/tests.
  ({int responses, int events}) get processedCallbackCounts =>
      (responses: _processedResponses, events: _processedEvents);

  /// Final callback counts returned by `rblue_shutdown`, once disposal starts.
  ({int responses, int events})? get targetCallbackCounts {
    final responses = _targetResponses;
    final events = _targetEvents;
    if (responses == null || events == null) return null;
    return (responses: responses, events: events);
  }

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
    if (_responseCallable != null || _eventCallable != null) {
      throw StateError('rblue_flutter: callbacks are already registered');
    }
    _responseCallable = onResponse == null
        ? null
        : _makeResponseCallable(onResponse);
    _eventCallable = onEvent == null ? null : _makeEventCallable(onEvent);
    final responseCode = _bindings.rblueSetOwnedResponseCallback(
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
      throw StateError(
        'rblue_flutter: callback registration failed ($responseCode/$eventCode)',
      );
    }
  }

  ffi.NativeCallable<rblue_response_cbFunction> _makeResponseCallable(
    void Function(int, String?, Uint8List) onResponse,
  ) {
    late final ffi.NativeCallable<rblue_response_cbFunction> callable;
    callable = ffi.NativeCallable<rblue_response_cbFunction>.listener((
      int reqId,
      ffi.Pointer<ffi.Char> statusJson,
      ffi.Pointer<ffi.Uint8> payload,
      int payloadLen,
      ffi.Pointer<ffi.Void> userData,
    ) {
      String? status;
      if (statusJson != ffi.nullptr) {
        status = statusJson.cast<Utf8>().toDartString();
        _bindings.rblueFreeString(statusJson);
      }
      final Uint8List data;
      try {
        data = payloadLen == 0
            ? Uint8List(0)
            : Uint8List.fromList(payload.asTypedList(payloadLen));
      } finally {
        _bindings.rblueFreePayload(payload, payloadLen);
      }
      _processedResponses++;
      if (!_disposing) {
        onResponse(reqId, status, data);
      } else if (_processedResponses >= _targetResponses!) {
        callable.close();
        _responseCallable = null;
        _freeHandleWhenDrained();
      }
    });
    return callable;
  }

  ffi.NativeCallable<rblue_event_cbFunction> _makeEventCallable(
    void Function(String) onEvent,
  ) {
    late final ffi.NativeCallable<rblue_event_cbFunction> callable;
    callable = ffi.NativeCallable<rblue_event_cbFunction>.listener((
      ffi.Pointer<ffi.Char> eventJson,
      ffi.Pointer<ffi.Void> userData,
    ) {
      final json = eventJson.cast<Utf8>().toDartString();
      _bindings.rblueFreeString(eventJson);
      _processedEvents++;
      if (!_disposing) {
        onEvent(json);
      } else if (_processedEvents >= _targetEvents!) {
        callable.close();
        _eventCallable = null;
        _freeHandleWhenDrained();
      }
    });
    return callable;
  }

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
      return _bindings.rblueRequest(
        handle,
        reqId,
        methodPtr,
        paramsPtr,
        payloadPtr,
        payload.length,
      );
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
    if (_disposing) return;
    _disposing = true;
    _disposingHandle = handle;
    final responseCount = calloc<ffi.Uint64>();
    final eventCount = calloc<ffi.Uint64>();
    try {
      final code = _bindings.rblueShutdown(handle, responseCount, eventCount);
      if (code != RBLUE_REQUEST_OK) {
        throw StateError('rblue_flutter: native shutdown failed ($code)');
      }
      _targetResponses = responseCount.value;
      _targetEvents = eventCount.value;
    } finally {
      calloc.free(responseCount);
      calloc.free(eventCount);
    }

    if (_processedResponses >= _targetResponses!) {
      _responseCallable?.close();
      _responseCallable = null;
    }
    if (_processedEvents >= _targetEvents!) {
      _eventCallable?.close();
      _eventCallable = null;
    }
    _freeHandleWhenDrained();
  }

  void _freeHandleWhenDrained() {
    if (_handleFreed || _responseCallable != null || _eventCallable != null) {
      return;
    }
    final handle = _disposingHandle;
    if (handle == null) return;
    _bindings.rblueFree(handle);
    _disposingHandle = null;
    _handleFreed = true;
    if (!_drained.isCompleted) _drained.complete();
  }
}

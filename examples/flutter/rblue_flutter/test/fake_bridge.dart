import 'dart:convert' show jsonEncode;
import 'dart:ffi' as ffi;
import 'dart:typed_data';

import 'package:rblue_flutter/src/ffi_bindings.dart' show RblueHandle;
import 'package:rblue_flutter/src/native_bridge.dart';

/// In-memory stand-in for the rblue-c ABI with mock-backend semantics.
///
/// Tests drive it directly: `send` records requests, `respondTo` completes a
/// request (out of order on purpose), `emitEvent` pushes an envelope, and
/// `reset`/`simulateDispose` model the shutdown discipline. No native code
/// runs; the pointers are address tokens.
class FakeNativeBridge implements NativeBridge {
  /// Records every send as (reqId, method, params, payload).
  final sent =
      <({int reqId, String method, String params, Uint8List payload})>[];

  /// Callbacks the client registered (non-null after construction).
  void Function(int reqId, String? statusJson, Uint8List payload)? onResponse;
  void Function(String eventJson)? onEvent;

  /// The synchronous code the next send returns; 0 by default.
  int nextSyncCode = 0;

  /// Whether `create` returns a handle (false simulates a failed backend).
  var backendAvailable = true;

  /// Whether the handle was disposed (the client must stop using it).
  var disposed = false;

  /// Number of active registrations (0 or 2 in healthy states).
  var registrations = 0;

  final _handles = <ffi.Pointer<RblueHandle>>[];
  var _nextHandleAddress = 0x1000;

  /// Handles this fake handed out.
  List<ffi.Pointer<RblueHandle>> get handles => List.unmodifiable(_handles);

  @override
  ffi.Pointer<RblueHandle>? create() {
    if (!backendAvailable) return null;
    final handle = ffi.Pointer<RblueHandle>.fromAddress(_nextHandleAddress);
    _nextHandleAddress += 0x10;
    _handles.add(handle);
    disposed = false;
    return handle;
  }

  @override
  void registerCallbacks(
    ffi.Pointer<RblueHandle> handle, {
    void Function(int, String?, Uint8List)? onResponse,
    void Function(String)? onEvent,
  }) {
    this.onResponse = onResponse;
    this.onEvent = onEvent;
    registrations = (onResponse == null ? 0 : 1) + (onEvent == null ? 0 : 1);
  }

  @override
  int send(
    ffi.Pointer<RblueHandle> handle,
    int reqId,
    String method,
    Map<String, Object?> params,
    Uint8List payload,
  ) {
    if (disposed) {
      throw StateError('use after free');
    }
    if (nextSyncCode != 0) return nextSyncCode;
    sent.add((
      reqId: reqId,
      method: method,
      params: jsonEncode(params),
      payload: Uint8List.fromList(payload),
    ));
    return 0;
  }

  @override
  void dispose(ffi.Pointer<RblueHandle> handle) {
    disposed = true;
    onResponse = null;
    onEvent = null;
    registrations = 0;
  }

  /// Completes one request with a success payload; asserts it was sent.
  void respondTo(int reqId, [List<int> payload = const []]) {
    assert(
      sent.any((entry) => entry.reqId == reqId),
      'no request $reqId was sent',
    );
    onResponse!(reqId, null, Uint8List.fromList(payload));
  }

  /// Completes one request with a `status_json` error string.
  void failRequest(int reqId, String statusJson) {
    onResponse!(reqId, statusJson, Uint8List(0));
  }

  /// Pushes one event envelope through the event callback.
  void emitEvent(String envelopeJson) => onEvent!(envelopeJson);
}

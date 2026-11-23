import 'dart:async';
import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:typed_data';

import 'ffi_bindings.dart'
    show RblueHandle, RBLUE_REQUEST_NULL_HANDLE, RBLUE_REQUEST_OK;
import 'models.dart';
import 'native_bridge.dart';
import 'plugin_channel.dart';

/// One raw response delivery of the C ABI.
typedef RawResponse = ({String? statusJson, Uint8List payload});

/// Typed Dart client over the rblue-c message-passing C ABI.
///
/// - `request` responses are correlated by a client-chosen monotonic `req_id`
///   through one `Completer` each; responses arriving after a timeout (or
///   after [dispose]) are dropped.
/// - Adapter events and GATT notifications arrive on the two broadcast
///   streams; subscribe before [initialize]/[startScan] so nothing is missed.
/// - Errors are the `status_json` values of the C ABI, parsed into
///   [RblueException]; raw platform codes (Android GATT status 133) survive
///   in [RblueException.platformCode].
///
/// Lifecycle: create through [create] (which performs the Android-side JNI
/// initialization), use, then [dispose]. Native callback listeners keep this
/// client reachable, so explicit disposal is required.
class RblueClient {
  final NativeBridge _bridge;
  final ffi.Pointer<RblueHandle> _handle;
  final _pending = <int, Completer<RawResponse>>{};
  final _events = StreamController<AdapterEvent>.broadcast();
  final _notifications = StreamController<DeviceNotification>.broadcast();
  final _supervisorEvents = StreamController<SupervisorEvent>.broadcast();
  var _nextReqId = 1;
  var _disposed = false;

  RblueClient._(this._bridge, this._handle) {
    _bridge.registerCallbacks(
      _handle,
      onResponse: _onResponse,
      onEvent: _onEvent,
    );
  }

  /// Creates the client over [bridge], performing the platform
  /// initialization first (on Android this runs the JNI init through the
  /// plugin's method channel; see the plugin README).
  static Future<RblueClient> create({NativeBridge? bridge}) async {
    final native = bridge ?? RealNativeBridge.instance();
    await RbluePluginChannel.instance.initialize();
    final handle = native.create();
    if (handle == null) {
      throw RblueContractError(
        'rblue_new returned NULL: the backend cannot run here',
      );
    }
    return RblueClient._(native, handle);
  }

  /// Test seam: builds the client over an already-initialized bridge.
  factory RblueClient.forBridge(
    NativeBridge bridge,
    ffi.Pointer<RblueHandle> handle,
  ) => RblueClient._(bridge, handle);

  /// Broadcast stream of adapter events (state, discoveries, connections).
  Stream<AdapterEvent> get events => _events.stream;

  /// Broadcast stream of notification values from subscribed
  /// characteristics of connected devices.
  Stream<DeviceNotification> get notifications => _notifications.stream;

  /// Broadcast stream of application-level supervisor events (reconnect
  /// scheduled, breaker open, ...). Distinct from [events] and from the
  /// platform `auto_connect` flag on [connect].
  Stream<SupervisorEvent> get supervisorEvents => _supervisorEvents.stream;

  /// Backend version string, for the platform page and smoke checks.
  Future<String> backendVersion() => _request(
    'backend_version',
    decode: (payload) => jsonDecode(utf8.decode(payload))! as String,
  );

  /// Backend capability bitmask, using the [Capability] bit positions
  /// (`u32` on the Rust side).
  Future<int> capabilities() => _request('capabilities', decode: _decodeInt);

  /// Reads the connected device RSSI when supported by the backend.
  Future<int> readRssi(DeviceId device) =>
      _request('rssi', params: {'device': device.json}, decode: _decodeInt);

  /// Returns the current ATT MTU when supported by the backend.
  Future<int> mtu(DeviceId device) =>
      _request('mtu', params: {'device': device.json}, decode: _decodeInt);

  /// Maximum payload accepted for a characteristic and write mode.
  Future<int> maxWriteLength(
    DeviceId device,
    CharacteristicId characteristic, {
    WriteMode mode = WriteMode.withResponse,
  }) => _request(
    'max_write_len',
    params: {
      ..._characteristicParams(device, characteristic),
      'mode': mode.toJson(),
    },
    decode: _decodeInt,
  );

  static int _decodeInt(Uint8List payload) =>
      jsonDecode(utf8.decode(payload)) as int;

  /// Opens the adapter; on some platforms the permission prompt happens here.
  Future<void> initialize() => _request('initialize', decode: _noPayload);

  /// Queries the current radio/permission state of the adapter.
  ///
  /// This does not require [Capability.adapterPowerControl]; that bit only
  /// covers [setPowered]. Live transitions arrive as [AdapterStateChanged].
  Future<AdapterState> state() => _request(
    'state',
    decode: (payload) =>
        AdapterState.fromJson(jsonDecode(utf8.decode(payload))! as String),
  );

  /// Turns the adapter radio on (`true`) or off (`false`).
  ///
  /// Check [Capability.adapterPowerControl] first. Platforms that cannot
  /// honor the request return `Unsupported` with `feature: set_powered`.
  Future<void> setPowered({required bool powered}) =>
      _request('set_powered', params: {'powered': powered}, decode: _noPayload);

  /// Starts scanning with [filter] (defaults to the unfiltered scan).
  /// Discoveries arrive on [events].
  Future<void> startScan({ScanFilter? filter}) =>
      _request('start_scan', params: filter?.toJson(), decode: _noPayload);

  /// Stops the running scan.
  Future<void> stopScan() => _request('stop_scan', decode: _noPayload);

  /// Snapshot of the devices known to the adapter.
  Future<List<PeripheralInfo>> peripherals() => _peripheralList('peripherals');

  /// Finds BLE devices already connected to the system that match any of
  /// [services]. The list must be nonempty.
  ///
  /// Supported on Apple, Android, and Linux; other platforms return
  /// `Unsupported`. On Android, devices whose system UUID cache is empty are
  /// omitted. The caller must still [connect] locally before GATT operations.
  Future<List<PeripheralInfo>> systemConnectedPeripherals(
    List<String> services,
  ) => _peripheralList('system_connected_peripherals', {'services': services});

  Future<List<PeripheralInfo>> _peripheralList(
    String method, [
    Map<String, Object?>? params,
  ]) => _request(
    method,
    params: params,
    decode: (payload) => (jsonDecode(utf8.decode(payload))! as List<Object?>)
        .map(
          (value) =>
              PeripheralInfo.fromJson((value! as Map).cast<String, Object?>()),
        )
        .toList(),
  );

  /// Connects to the device. Resolves when the link is up; run
  /// [discoverServices] next.
  ///
  /// [autoConnect] is the platform/stack background-connection flag
  /// (`ConnectOptions.auto_connect`). Check [Capability.autoConnect] first;
  /// backends without that capability return `Unsupported`. This is not
  /// [RblueClient.autoConnect], which enables the reconnect supervisor.
  ///
  /// [mtu] is reserved for a connect-time ATT MTU request; backends that
  /// do not implement it return `Unsupported` with feature `request_mtu`.
  Future<void> connect(DeviceId device, {bool autoConnect = false, int? mtu}) =>
      _request(
        'connect',
        params: {
          'device': device.json,
          'auto_connect': autoConnect,
          'mtu': ?mtu,
        },
        decode: _noPayload,
      );

  /// Enables the application-level reconnect supervisor for [device]
  /// (`SupervisorHandle::auto_connect`). Optional [policy] uses the
  /// `*_ms` serde shape of [ReconnectPolicy].
  ///
  /// This is not the platform `auto_connect` flag on [connect].
  Future<void> autoConnect(DeviceId device, {ReconnectPolicy? policy}) =>
      _request(
        'auto_connect',
        params: {'device': device.json, 'policy': ?policy?.toJson()},
        decode: _noPayload,
      );

  /// Stops supervising [device] (`SupervisorHandle::unwatch`). Idempotent
  /// if the supervisor was never started.
  Future<void> unwatch(DeviceId device) =>
      _request('unwatch', params: {'device': device.json}, decode: _noPayload);

  /// Disconnects the device. Idempotent. Subscriptions of the device are
  /// dropped; subscribe again after reconnecting.
  Future<void> disconnect(DeviceId device) => _request(
    'disconnect',
    params: {'device': device.json},
    decode: _noPayload,
  );

  /// Discovers the GATT tree; must run on every connection (iOS requires it,
  /// Android caches may be stale). The returned characteristic ids are valid
  /// only for this connection.
  Future<ServiceSet> discoverServices(DeviceId device) => _request(
    'discover_services',
    params: {'device': device.json},
    decode: (payload) => ServiceSet.fromJson(
      (jsonDecode(utf8.decode(payload))! as Map).cast<String, Object?>(),
    ),
  );

  /// Reads the characteristic value.
  Future<Uint8List> read(DeviceId device, CharacteristicId characteristic) =>
      _request(
        'read',
        params: _characteristicParams(device, characteristic),
        decode: (payload) => payload,
      );

  /// Writes [data] to the characteristic. Oversize payloads fail with
  /// [RblueErrorKind.invalidLength] - never truncated, never chunked -
  /// unless [allowLongWrite] is true, in which case the payload is written
  /// as sequential MTU-sized chunks (`WriteOptions.allow_long_write`).
  Future<void> write(
    DeviceId device,
    CharacteristicId characteristic,
    List<int> data, {
    WriteMode mode = WriteMode.withResponse,
    bool allowLongWrite = false,
  }) => _request(
    'write',
    params: {
      ..._characteristicParams(device, characteristic),
      'mode': mode.toJson(),
      'allow_long_write': allowLongWrite,
    },
    payload: data,
    decode: _noPayload,
  );

  /// Subscribes to value changes; values arrive on [notifications].
  Future<void> subscribe(DeviceId device, CharacteristicId characteristic) =>
      _request(
        'subscribe',
        params: _characteristicParams(device, characteristic),
        decode: _noPayload,
      );

  /// Cancels a subscription. Idempotent.
  Future<void> unsubscribe(DeviceId device, CharacteristicId characteristic) =>
      _request(
        'unsubscribe',
        params: _characteristicParams(device, characteristic),
        decode: _noPayload,
      );

  /// Connection state of the device.
  Future<bool> isConnected(DeviceId device) => _request(
    'is_connected',
    params: {'device': device.json},
    decode: (payload) => jsonDecode(utf8.decode(payload))! as bool,
  );

  /// Stops native producers, fails pending requests, and closes the public
  /// streams. Internal FFI callbacks may continue briefly only to release
  /// already-queued native ownership transfers before the handle is freed.
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    for (final completer in _pending.values) {
      completer.completeError(
        RblueException.client('client disposed'),
        StackTrace.current,
      );
    }
    _pending.clear();
    _bridge.dispose(_handle);
    _events.close();
    _notifications.close();
    _supervisorEvents.close();
  }

  /// The per-request timeout; generous because real hardware is slow, short
  /// enough that the reconnect demo stays responsive.
  static const _defaultTimeout = Duration(seconds: 30);

  Map<String, Object?> _characteristicParams(
    DeviceId device,
    CharacteristicId characteristic,
  ) => {'device': device.json, 'characteristic': characteristic.toJson()};

  static void _noPayload(Uint8List payload) {}

  Future<T> _request<T>(
    String method, {
    Map<String, Object?>? params,
    List<int> payload = const [],
    required T Function(Uint8List payload) decode,
  }) async {
    if (_disposed) {
      throw RblueException.client('client disposed');
    }
    final reqId = _nextReqId++;
    final completer = Completer<RawResponse>();
    _pending[reqId] = completer;
    final code = _bridge.send(
      _handle,
      reqId,
      method,
      params ?? const {},
      Uint8List.fromList(payload),
    );
    if (code != RBLUE_REQUEST_OK) {
      _pending.remove(reqId);
      throw RblueException.client(
        'rblue_request rejected synchronously ($code): '
        '${code == RBLUE_REQUEST_NULL_HANDLE ? "null handle" : "invalid strings"}',
      );
    }
    RawResponse raw;
    try {
      raw = await completer.future.timeout(
        _defaultTimeout,
        onTimeout: () {
          _pending.remove(reqId);
          throw RblueException.client(
            'request "$method" (req $reqId) timed out',
          );
        },
      );
    } finally {
      _pending.remove(reqId);
    }
    final status = raw.statusJson;
    if (status != null) {
      throw RblueException.fromJson(status);
    }
    return decode(raw.payload);
  }

  void _onResponse(int reqId, String? statusJson, Uint8List payload) {
    final completer = _pending.remove(reqId);
    if (completer == null || completer.isCompleted) {
      return; // Late arrival after timeout or dispose: drop.
    }
    completer.complete((statusJson: statusJson, payload: payload));
  }

  void _onEvent(String eventJson) {
    Object? envelope;
    try {
      envelope = jsonDecode(eventJson);
    } on FormatException {
      _reportContractError('malformed event JSON: $eventJson');
      return;
    }
    if (envelope is! Map<String, Object?>) {
      _reportContractError('event envelope is not an object: $eventJson');
      return;
    }
    try {
      switch (envelope['type']) {
        case 'adapter_event':
          _events.add(AdapterEvent.fromJson(envelope['event']!));
        case 'notification':
          _notifications.add(DeviceNotification.fromJson(envelope));
        case 'supervisor_event':
          _supervisorEvents.add(SupervisorEvent.fromJson(envelope['event']!));
        case _:
          _reportContractError('unknown event type: $eventJson');
      }
    } on RblueContractError catch (error) {
      _reportContractError('$error in $eventJson');
    }
  }

  void _reportContractError(String detail) {
    // Contract violations are logged, never crash the streams; a chatty
    // malformed device must not take the app down.
    assert(() {
      throw RblueContractError(detail);
    }(), 'rblue contract violation: $detail');
  }
}

/// Scan filter (`rblue_api::ScanFilter`); omitted fields keep the backend
/// defaults (no filtering).
class ScanFilter {
  /// Keep only devices whose advertised name starts with this prefix.
  final String? namePrefix;

  /// Keep only devices advertising one of these service UUIDs (the only
  /// universally hardware-pushed-down filter).
  final List<String> services;

  /// Bluetooth SIG company ID for manufacturer-data filtering.
  final int? manufacturerId;

  /// Required prefix for the manufacturer data.
  final List<int> manufacturerDataPrefix;

  /// Minimum RSSI in dBm.
  final int? rssiThreshold;

  /// Builds a filter; pass no arguments for the unfiltered scan.
  const ScanFilter({
    this.namePrefix,
    this.services = const [],
    this.rssiThreshold,
    this.manufacturerId,
    this.manufacturerDataPrefix = const [],
  });

  /// The serde form; absent optional fields are serialized as null.
  Map<String, Object?> toJson() {
    if (manufacturerId == null && manufacturerDataPrefix.isNotEmpty) {
      throw ArgumentError('manufacturerDataPrefix requires manufacturerId');
    }
    return {
      'name_prefix': namePrefix,
      'services': services,
      'rssi_threshold': rssiThreshold,
      'manufacturer_data': manufacturerId == null
          ? null
          : {
              'company_id': manufacturerId,
              'data_prefix': manufacturerDataPrefix,
            },
    };
  }
}

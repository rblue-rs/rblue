import 'dart:async';
import 'dart:collection';
import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:rblue_flutter/rblue_flutter.dart';

/// Builds the client (async because of the platform init handshake).
typedef ClientFactory = Future<RblueClient> Function();

/// The example's single shared state: owns the [RblueClient], folds the
/// adapter event stream into viewable collections, and exposes the scan
/// toggle. Deliberately plain (a `ChangeNotifier`) - state management is not
/// the point of this example.
class AppState extends ChangeNotifier {
  /// Creates the state. [clientFactory] and [permissionRequester] are test
  /// seams; production uses `RblueClient.create` and the plugin channel.
  AppState({
    ClientFactory? clientFactory,
    Future<bool> Function()? permissionRequester,
  }) : _clientFactory = clientFactory ?? _defaultClientFactory,
       _permissionRequester =
           permissionRequester ??
           RbluePluginChannel.instance.requestBlePermissions;

  static Future<RblueClient> _defaultClientFactory() => RblueClient.create();

  final ClientFactory _clientFactory;
  final Future<bool> Function() _permissionRequester;
  RblueClient? _client;
  bool _disposed = false;

  /// Bounded session history; notification packets retain their full instance id.
  final logs = ListQueue<SessionLog>();
  final packets = ListQueue<ReceivedPacket>();
  final subscriptions = <(DeviceId, CharacteristicId)>{};
  final _subscriptionRevisions = <DeviceId, int>{};
  int notificationCount = 0;
  bool scanPending = false;

  /// Records an application operation, never a library-level diagnostic.
  void record(String action, String detail, {bool isError = false}) {
    if (_disposed) return;
    logs.addFirst(SessionLog(DateTime.now(), action, detail, isError));
    if (logs.length > 300) logs.removeLast();
    notifyListeners();
  }

  /// Adds success/error context around the existing C ABI call.
  Future<T> operation<T>(String action, Future<T> Function() run) async {
    try {
      final value = await run();
      record(action, '成功');
      return value;
    } catch (error) {
      record(action, error.toString(), isError: true);
      rethrow;
    }
  }

  /// Updates shared subscription state only after the backend acknowledges it.
  Future<void> setSubscription(
    DeviceId device,
    CharacteristicId id,
    bool enabled,
  ) async {
    final revision = _subscriptionRevisions.putIfAbsent(device, () => 0);
    await operation(
      '${enabled ? "subscribe" : "unsubscribe"} / ${device.display} / ${id.service.uuid} #${id.service.instance} / ${id.uuid} #${id.instance}',
      () => enabled
          ? client!.subscribe(device, id)
          : client!.unsubscribe(device, id),
    );
    if (_disposed || _subscriptionRevisions[device] != revision) return;
    if (enabled && isConnected(device)) {
      subscriptions.add((device, id));
    } else {
      subscriptions.remove((device, id));
    }
    notifyListeners();
  }

  /// Cancels acknowledged subscriptions before rediscovering a live GATT tree.
  Future<void> cancelDeviceSubscriptions(DeviceId device) async {
    for (final entry
        in subscriptions.where((entry) => entry.$1 == device).toList()) {
      await setSubscription(device, entry.$2, false);
    }
  }

  /// Drops connection-scoped subscription handles after disconnect/discovery.
  void invalidateSubscriptions(DeviceId device) {
    _subscriptionRevisions.update(
      device,
      (value) => value + 1,
      ifAbsent: () => 1,
    );
    subscriptions.removeWhere((entry) => entry.$1 == device);
    notifyListeners();
  }

  /// Clears the retained packets and receive counter without changing subscriptions.
  void clearPackets() {
    packets.clear();
    notificationCount = 0;
    notifyListeners();
  }

  /// Clears only the retained operation log.
  void clearLogs() {
    logs.clear();
    notifyListeners();
  }

  /// JSON snapshot for user-initiated clipboard export; identifiers are included.
  String sessionJson() => const JsonEncoder.withIndent('  ').convert({
    'exported_at': DateTime.now().toIso8601String(),
    'runtime': runtimeLabel,
    'adapter_state': adapterState.name,
    'permissions_granted': permissionsGranted,
    'retention': {'logs': 300, 'packets': 200},
    'notification_count': notificationCount,
    'logs': [
      for (final entry in logs)
        {
          'time': entry.time.toIso8601String(),
          'action': entry.action,
          'detail': entry.detail,
          'error': entry.isError,
        },
    ],
    'packets': [
      for (final packet in packets)
        {
          'time': packet.time.toIso8601String(),
          'device': packet.value.device.json,
          'characteristic': packet.value.id.toJson(),
          'data': packet.value.data.toList(),
        },
    ],
  });
  StreamSubscription<AdapterEvent>? _eventsSub;
  StreamSubscription<DeviceNotification>? _notificationsSub;

  /// Start-up in flight.
  bool starting = false;

  /// The client is up and `initialize` succeeded.
  bool ready = false;

  /// Whether the Android runtime permissions were granted (always true on
  /// iOS).
  bool permissionsGranted = true;

  /// Last failure, shown as a banner.
  String? lastError;

  /// Radio state as reported by the adapter.
  AdapterState adapterState = AdapterState.unknown;

  /// Whether a scan is running.
  bool scanning = false;

  /// Devices seen so far, keyed by their canonical DeviceId JSON.
  final Map<String, DiscoveredDevice> devices = {};

  /// Devices with an open connection, keyed like [devices].
  final Set<String> connected = {};

  /// The most recent notification of any device, for the platform page.
  DeviceNotification? lastNotification;

  RblueClient? get client => _client;

  /// Label for the app bar: backend name once ready.
  String get runtimeLabel => ready
      ? (adapterState == AdapterState.unknown ? 'ready' : adapterState.name)
      : starting
      ? 'starting'
      : 'offline';

  /// Canonical key for a device identity.
  String deviceKey(DeviceId device) => jsonEncode(device.json);

  /// Requests the platform BLE permissions (no-op on iOS).
  Future<bool> requestPermissions() => _permissionRequester();

  /// Brings the client up: permissions, plugin initialization, adapter
  /// initialization. Idempotent.
  Future<void> start() async {
    if (starting || ready || _disposed) return;
    starting = true;
    lastError = null;
    notifyListeners();
    try {
      permissionsGranted = await _permissionRequester();
      if (_disposed) return;
      await _eventsSub?.cancel();
      await _notificationsSub?.cancel();
      _client?.dispose();
      final client = await _clientFactory();
      if (_disposed) {
        client.dispose();
        return;
      }
      _client = client;
      _eventsSub = client.events.listen(_onEvent);
      _notificationsSub = client.notifications.listen((notification) {
        if (_disposed) return;
        lastNotification = notification;
        notificationCount++;
        packets.addFirst(ReceivedPacket(DateTime.now(), notification));
        if (packets.length > 200) packets.removeLast();
        notifyListeners();
      });
      await client.initialize();
      if (_disposed) return;
      ready = true;
      record('initialize', '蓝牙适配器已初始化');
    } on RblueException catch (error) {
      lastError = error.message;
      record('initialize.error', error.toString(), isError: true);
      if (error.kind == RblueErrorKind.permission) {
        permissionsGranted = false;
      }
    } catch (error) {
      lastError = error.toString();
    } finally {
      starting = false;
      notifyListeners();
    }
  }

  /// Starts or stops the scan.
  Future<void> toggleScan() async {
    final client = _client;
    if (client == null || !ready || scanPending) return;
    scanPending = true;
    notifyListeners();
    try {
      if (scanning) {
        await client.stopScan();
        scanning = false;
        record('scan.stop', '扫描已停止');
      } else {
        await client.startScan();
        scanning = true;
        record('scan.start', '扫描已开始');
      }
    } on RblueException catch (error) {
      lastError = error.message;
      record('scan.error', error.toString(), isError: true);
    } finally {
      scanPending = false;
      notifyListeners();
    }
  }

  /// Imports the matching system-connected peripherals into the device list.
  Future<int> findSystemConnected(List<String> services) async {
    final client = _client;
    if (!ready || client == null) return 0;
    final matches = await client.systemConnectedPeripherals(services);
    if (_disposed || !identical(client, _client)) return 0;
    for (final device in matches) {
      devices.putIfAbsent(
        deviceKey(device.id),
        () => DiscoveredDevice(
          id: device.id,
          advertisement: AdvertisementData(
            localName: device.name,
            services: const [],
            rssi: null,
            isConnectable: null,
            manufacturerData: null,
          ),
        ),
      );
    }
    record('system_connected.lookup', '找到 ${matches.length} 台系统已连接设备');
    notifyListeners();
    return matches.length;
  }

  /// The connection state of one device.
  bool isConnected(DeviceId device) => connected.contains(deviceKey(device));

  /// Current RSSI of a known device, when seen.
  int? rssiOf(DeviceId device) =>
      devices[deviceKey(device)]?.advertisement.rssi;

  void _onEvent(AdapterEvent event) {
    switch (event) {
      case AdapterStateChanged(:final state):
        adapterState = state;
        if (state == AdapterState.poweredOff ||
            state == AdapterState.unavailable ||
            state == AdapterState.unauthorized) {
          scanning = false;
          connected.clear();
          subscriptions.clear();
          _subscriptionRevisions.updateAll((_, value) => value + 1);
        }
      case AdapterDeviceDiscovered(:final device):
        devices[deviceKey(device.id)] = device;
      case AdapterDeviceUpdated(:final id, :final rssi):
        final key = deviceKey(id);
        final known = devices[key];
        if (known != null) {
          devices[key] = DiscoveredDevice(
            id: known.id,
            advertisement: AdvertisementData(
              localName: known.advertisement.localName,
              services: known.advertisement.services,
              rssi: rssi,
              isConnectable: known.advertisement.isConnectable,
              manufacturerData: known.advertisement.manufacturerData,
              serviceData: known.advertisement.serviceData,
              txPower: known.advertisement.txPower,
              flags: known.advertisement.flags,
            ),
          );
        }
      case AdapterDeviceConnected(:final id):
        connected.add(deviceKey(id));
        record('connected', id.display);
      case AdapterDeviceDisconnected(:final id):
        connected.remove(deviceKey(id));
        subscriptions.removeWhere((entry) => entry.$1 == id);
        _subscriptionRevisions.update(
          id,
          (value) => value + 1,
          ifAbsent: () => 1,
        );
        record('disconnected', id.display);
    }
    notifyListeners();
  }

  /// Devices sorted by RSSI (strongest first, missing values last).
  List<DiscoveredDevice> sortedDevices() {
    final list = devices.values.toList();
    list.sort((a, b) {
      final rssiA = a.advertisement.rssi;
      final rssiB = b.advertisement.rssi;
      if (rssiA == null && rssiB == null) return 0;
      if (rssiA == null) return 1;
      if (rssiB == null) return -1;
      return rssiB.compareTo(rssiA);
    });
    return list;
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _eventsSub?.cancel();
    _notificationsSub?.cancel();
    _client?.dispose();
    _client = null;
    super.dispose();
  }
}

/// Exposes the [AppState] below the home page; pages read it with
/// `AppStateScope.of(context)` and rebuild on notifications.
class AppStateScope extends InheritedNotifier<AppState> {
  const AppStateScope({
    super.key,
    required AppState super.notifier,
    required super.child,
  });

  /// The shared state.
  static AppState of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<AppStateScope>()!.notifier!;
}

/// One retained operation or backend event for the session timeline.
class SessionLog {
  const SessionLog(this.time, this.action, this.detail, this.isError);
  final DateTime time;
  final String action;
  final String detail;
  final bool isError;
}

/// A notification with its host receive timestamp.
class ReceivedPacket {
  const ReceivedPacket(this.time, this.value);
  final DateTime time;
  final DeviceNotification value;
}

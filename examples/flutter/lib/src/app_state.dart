import 'dart:async';
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
  })  : _clientFactory = clientFactory ?? _defaultClientFactory,
        _permissionRequester =
            permissionRequester ?? RbluePluginChannel.instance.requestBlePermissions;

  static Future<RblueClient> _defaultClientFactory() => RblueClient.create();

  final ClientFactory _clientFactory;
  final Future<bool> Function() _permissionRequester;
  RblueClient? _client;
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
    if (starting || ready) return;
    starting = true;
    lastError = null;
    notifyListeners();
    try {
      permissionsGranted = await _permissionRequester();
      final client = await _clientFactory();
      _client = client;
      _eventsSub = client.events.listen(_onEvent);
      _notificationsSub = client.notifications.listen((notification) {
        lastNotification = notification;
        notifyListeners();
      });
      await client.initialize();
      ready = true;
    } on RblueException catch (error) {
      lastError = error.message;
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
    if (client == null || !ready) return;
    try {
      if (scanning) {
        await client.stopScan();
        scanning = false;
      } else {
        await client.startScan();
        scanning = true;
      }
    } on RblueException catch (error) {
      lastError = error.message;
    } finally {
      notifyListeners();
    }
  }

  /// The connection state of one device.
  bool isConnected(DeviceId device) => connected.contains(deviceKey(device));

  /// Current RSSI of a known device, when seen.
  int? rssiOf(DeviceId device) => devices[deviceKey(device)]?.advertisement.rssi;

  void _onEvent(AdapterEvent event) {
    switch (event) {
      case AdapterStateChanged(:final state):
        adapterState = state;
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
            ),
          );
        }
      case AdapterDeviceConnected(:final id):
        connected.add(deviceKey(id));
      case AdapterDeviceDisconnected(:final id):
        connected.remove(deviceKey(id));
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
  void dispose() {
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
  const AppStateScope({super.key, required AppState super.notifier, required super.child});

  /// The shared state.
  static AppState of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<AppStateScope>()!.notifier!;
}

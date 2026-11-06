import 'dart:async';

import 'package:flutter/material.dart';
import 'package:rblue_flutter/rblue_flutter.dart';

import '../app_state.dart';
import '../reconnect_policy.dart';
import '../widgets/expressive_button.dart';
import 'scan_page.dart' show MaterialBannerTile;

/// Reconnect tab: connects to a device and, when the link drops, retries with
/// a visible exponential backoff (500 ms ×2, capped at 30 seconds, ±20%
/// jitter). On real hardware you trigger the drop by powering the peripheral
/// off; the demo watches the adapter's `DeviceDisconnected` event.
///
/// Automatic reconnect lives on the host side by design: the Rust supervisor
/// is not exported through the FFI surface (see examples/README.md), so the
/// example shows the pattern instead.
class ReconnectPage extends StatefulWidget {
  const ReconnectPage({super.key});

  @override
  State<ReconnectPage> createState() => _ReconnectPageState();
}

/// One line of the demo's activity log.
class LogEntry {
  /// Time of the entry, relative to the demo start.
  final Duration sinceStart;

  /// The message.
  final String text;

  /// Whether the entry reports a failure.
  final bool isError;

  const LogEntry(this.sinceStart, this.text, {this.isError = false});
}

class _ReconnectPageState extends State<ReconnectPage> {
  DeviceId? selectedId;
  DiscoveredDevice? device;
  ReconnectController? controller;
  bool restoreNotifications = true;
  StreamSubscription<AdapterEvent>? _eventsSub;

  @override
  void dispose() {
    _eventsSub?.cancel();
    controller?.stop();
    super.dispose();
  }

  void _select(DiscoveredDevice selected) {
    setState(() {
      selectedId = selected.id;
      device = selected;
      _eventsSub?.cancel();
      controller?.stop();
      controller = null;
    });
  }

  void _startDemo() {
    final state = AppStateScope.of(context);
    final selected = device;
    if (selected == null || state.client == null) return;
    _eventsSub?.cancel();
    _eventsSub = state.client!.events.listen((event) {
      if (event is AdapterDeviceDisconnected && event.id == selected.id) {
        controller?.onDisconnected();
      } else if (event is AdapterDeviceConnected && event.id == selected.id) {
        controller?.onConnected();
      }
    });
    controller?.stop();
    final wanted = restoreNotifications
        ? state.subscriptions
              .where((entry) => entry.$1 == selected.id)
              .map((entry) => entry.$2)
              .toList()
        : <CharacteristicId>[];
    setState(() {
      controller = ReconnectController(
        client: state.client!,
        device: selected,
        onSessionRestored: (services) async {
          state.invalidateSubscriptions(selected.id);
          for (final id in wanted) {
            if (!mounted || controller == null || controller!.stopped) return;
            final fresh = services.identifiedCharacteristics
                .where((entry) => entry.$1 == id && entry.$2.notifiable)
                .firstOrNull;
            if (fresh != null) {
              await state.setSubscription(selected.id, fresh.$1, true);
            } else {
              state.record(
                'reconnect.subscription',
                '特征已不存在: ${id.uuid} #${id.instance}',
                isError: true,
              );
            }
          }
        },
        onLog: () {
          if (!mounted) return;
          setState(() {});
          final current = controller;
          if (current != null && current.log.isNotEmpty) {
            final entry = current.log.first;
            state.record('reconnect', entry.text, isError: entry.isError);
          }
        },
      )..start();
    });
  }

  @override
  Widget build(BuildContext context) {
    final state = AppStateScope.of(context);
    return ListView(
      key: const PageStorageKey('reconnect'),
      padding: const EdgeInsets.all(16),
      children: [
        Text('复现断连，验证恢复。', style: Theme.of(context).textTheme.headlineSmall),
        const SizedBox(height: 12),
        const Text(
          '选择可连接设备并开始监听。将真实外设关闭再开启，观察退避、重试与服务重发现。此处不会注入虚构的 GATT 133 错误。',
        ),
        const SizedBox(height: 20),
        Card(
          child: Padding(
            padding: const EdgeInsets.all(16),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                DropdownButtonFormField<DeviceId>(
                  initialValue: selectedId,
                  decoration: const InputDecoration(labelText: '选择设备'),
                  isExpanded: true,
                  items: [
                    for (final candidate in state.sortedDevices().where(
                      (d) => d.advertisement.isConnectable != false,
                    ))
                      DropdownMenuItem(
                        value: candidate.id,
                        child: Text(
                          candidate.displayName,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                  ],
                  onChanged: state.ready
                      ? (id) {
                          if (id != null) {
                            _select(state.devices[state.deviceKey(id)]!);
                          }
                        }
                      : null,
                ),
                const SizedBox(height: 12),
                CheckboxListTile(
                  contentPadding: EdgeInsets.zero,
                  title: const Text('恢复开始监听时已有的通知订阅'),
                  value: restoreNotifications,
                  onChanged: controller != null
                      ? null
                      : (value) =>
                            setState(() => restoreNotifications = value!),
                ),
                const SizedBox(height: 16),
                SquishButtonGroup(
                  actions: [
                    (
                      label: '连接并监听',
                      tonal: false,
                      onPressed:
                          device == null || !state.ready || controller != null
                          ? null
                          : _startDemo,
                    ),
                    (
                      label: '停止监听',
                      tonal: true,
                      onPressed: controller == null
                          ? null
                          : () => setState(() {
                              controller?.stop();
                              controller = null;
                              _eventsSub?.cancel();
                            }),
                    ),
                  ],
                ),
                if (controller != null) ...[
                  const SizedBox(height: 16),
                  Text(
                    controller!.statusText,
                    style: Theme.of(context).textTheme.titleSmall,
                  ),
                ],
              ],
            ),
          ),
        ),
        const SizedBox(height: 16),
        const Text(
          '宿主层重连 · 500ms 起始退避，30s 上限，±20% jitter。重发现后旧特征实例失效；按新服务树匹配并恢复监听开始时的订阅。',
        ),
        const SizedBox(height: 16),
        if (controller?.lastError != null)
          MaterialBannerTile(
            icon: Icons.error_outline,
            text: controller!.lastError!,
          ),
        for (final entry in controller?.log ?? <LogEntry>[])
          ListTile(
            leading: Icon(
              entry.isError ? Icons.warning_amber : Icons.check_circle_outline,
            ),
            title: Text(entry.text),
            subtitle: Text('+${entry.sinceStart.inMilliseconds} ms'),
          ),
      ],
    );
  }
}

/// The reconnect state machine: connect, wait for the disconnect event, then
/// retry with exponential backoff until the link is back.
class ReconnectController {
  /// Reconnect policy: 500 ms ×2, 30-second cap, ±20% jitter.
  static const policy = ReconnectPolicy();

  ReconnectController({
    required this.client,
    required this.device,
    required this.onLog,
    required this.onSessionRestored,
  }) : _startedAt = DateTime.now();

  final RblueClient client;
  final DiscoveredDevice device;
  final void Function() onLog;
  final Future<void> Function(ServiceSet services) onSessionRestored;
  final DateTime _startedAt;
  final log = <LogEntry>[];
  Timer? _timer;
  var _attempt = 0;
  Duration? _pendingDelay;
  var _connecting = false;
  var stopped = false;
  String? lastError;

  /// Human-readable state line under the controls.
  String get statusText {
    if (_connecting) return 'connecting (attempt ${_attempt + 1})...';
    if (_timer != null) {
      final milliseconds = _pendingDelay?.inMilliseconds ?? 0;
      final seconds = (milliseconds / 1000).toStringAsFixed(1);
      return 'link down - retrying in ${seconds}s';
    }
    return 'connected - watching for drops';
  }

  /// Connects once and enters the watch state.
  Future<void> start() async {
    stopped = false;
    await _connect();
  }

  /// Aborts the demo; pending retries are cancelled.
  void stop() {
    stopped = true;
    _timer?.cancel();
    _timer = null;
    _pendingDelay = null;
  }

  /// Called by the page when `DeviceDisconnected` arrives.
  void onDisconnected() {
    if (stopped || _timer != null || _connecting) return;
    _attempt = 0;
    _log('link dropped - starting backoff retries', isError: true);
    _scheduleRetry();
  }

  /// Called by the page when `DeviceConnected` arrives.
  void onConnected() {
    if (stopped) return;
    _timer?.cancel();
    _timer = null;
    _pendingDelay = null;
    _connecting = false;
    _attempt = 0;
    _restoreSession();
  }

  Future<void> _restoreSession() async {
    try {
      final services = await client.discoverServices(device.id);
      if (stopped) return;
      await onSessionRestored(services);
      if (stopped) return;
      _log(
        'link re-established; services rediscovered; subscription restoration completed',
      );
    } on RblueException catch (error) {
      if (stopped) return;
      lastError =
          'reconnected, but service rediscovery failed: ${error.message}';
      _log(lastError!, isError: true);
    }
  }

  Future<void> _connect() async {
    if (stopped) return;
    _connecting = true;
    onLog();
    try {
      await client.connect(device.id);
      _connecting = false;
      if (stopped) return;
      _log('connected');
    } on RblueException catch (error) {
      _connecting = false;
      if (stopped) return;
      if (error.isAndroidGattStatus133) {
        lastError =
            'Platform status 133 (GATT_ERROR): the classic Android connect '
            'failure. The demo keeps retrying; move closer, re-scan, or '
            'toggle Bluetooth if it persists.';
      } else {
        lastError = error.message;
      }
      _log(
        'connect failed: ${error.message} (code ${error.platformCode ?? "n/a"})',
        isError: true,
      );
      _scheduleRetry();
    }
  }

  void _scheduleRetry() {
    if (stopped) return;
    final delay = policy.delayForAttempt(_attempt);
    _attempt += 1;
    _timer?.cancel();
    _pendingDelay = delay;
    _timer = Timer(delay, () {
      _timer = null;
      _pendingDelay = null;
      if (!stopped) _connect();
    });
    onLog();
  }

  void _log(String text, {bool isError = false}) {
    log.insert(
      0,
      LogEntry(DateTime.now().difference(_startedAt), text, isError: isError),
    );
    if (log.length > 50) {
      log.removeLast();
    }
    onLog();
  }
}

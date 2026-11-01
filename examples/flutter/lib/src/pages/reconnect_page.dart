import 'dart:async';

import 'package:flutter/material.dart';
import 'package:rblue_flutter/rblue_flutter.dart';

import '../app_state.dart';
import '../reconnect_policy.dart';
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
  DiscoveredDevice? device;
  ReconnectController? controller;
  StreamSubscription<AdapterEvent>? _eventsSub;

  @override
  void dispose() {
    _eventsSub?.cancel();
    controller?.stop();
    super.dispose();
  }

  void _select(DiscoveredDevice selected) {
    setState(() {
      device = selected;
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
      if (event is AdapterDeviceDisconnected &&
          event.id.json == selected.id.json) {
        controller?.onDisconnected();
      } else if (event is AdapterDeviceConnected &&
          event.id.json == selected.id.json) {
        controller?.onConnected();
      }
    });
    setState(() {
      controller = ReconnectController(
        client: state.client!,
        device: selected,
        onLog: () => setState(() {}),
      )..start();
    });
  }

  @override
  Widget build(BuildContext context) {
    final state = AppStateScope.of(context);
    return Padding(
      padding: const EdgeInsets.all(12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const Text(
            'Pick a device, connect, then power the peripheral off and on. '
            'The demo retries with a visible backoff and survives Android '
            'GATT status 133.',
          ),
          const SizedBox(height: 12),
          DropdownButton<DiscoveredDevice>(
            isExpanded: true,
            hint: const Text('choose a device'),
            value: device,
            items: [
              for (final candidate in state.sortedDevices())
                DropdownMenuItem(
                  value: candidate,
                  child: Text(candidate.displayName),
                ),
            ],
            onChanged: (candidate) => _select(candidate!),
          ),
          const SizedBox(height: 8),
          Row(
            children: [
              FilledButton(
                onPressed: device == null ? null : _startDemo,
                child: const Text('Connect + watch'),
              ),
              const SizedBox(width: 8),
              if (controller != null)
                OutlinedButton(
                  onPressed: () => setState(() {
                    controller?.stop();
                    controller = null;
                  }),
                  child: const Text('Stop'),
                ),
            ],
          ),
          if (controller != null) ...[
            const SizedBox(height: 8),
            Text(
              'status: ${controller!.statusText}',
              style: Theme.of(context).textTheme.titleSmall,
            ),
          ],
          Expanded(
            child: controller == null || controller!.log.isEmpty
                ? const SizedBox.shrink()
                : ListView(
                    children: [
                      if (controller!.lastError != null)
                        MaterialBannerTile(
                          icon: Icons.error_outline,
                          text: controller!.lastError!,
                        ),
                      for (final entry in controller!.log)
                        ListTile(
                          dense: true,
                          leading: Icon(
                            entry.isError
                                ? Icons.warning_amber
                                : Icons.check_circle_outline,
                            size: 18,
                          ),
                          title: Text(entry.text),
                          subtitle: Text('+${entry.sinceStart.inSeconds}s'),
                        ),
                    ],
                  ),
          ),
        ],
      ),
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
    required void Function() onLog,
  }) : _onLog = onLog,
       _startedAt = DateTime.now();

  final RblueClient client;
  final DiscoveredDevice device;
  final void Function() _onLog;
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
    if (stopped) return;
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
      await client.discoverServices(device.id);
      _log(
        'link re-established; services rediscovered; restore app subscriptions here',
      );
    } on RblueException catch (error) {
      lastError =
          'reconnected, but service rediscovery failed: ${error.message}';
      _log(lastError!, isError: true);
    }
  }

  Future<void> _connect() async {
    if (stopped) return;
    _connecting = true;
    _onLog();
    try {
      await client.connect(device.id);
      _connecting = false;
      _log('connected');
    } on RblueException catch (error) {
      _connecting = false;
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
    _onLog();
  }

  void _log(String text, {bool isError = false}) {
    log.insert(
      0,
      LogEntry(DateTime.now().difference(_startedAt), text, isError: isError),
    );
    if (log.length > 50) {
      log.removeLast();
    }
    _onLog();
  }
}

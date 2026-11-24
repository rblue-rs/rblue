import 'dart:async';

import 'package:flutter/material.dart';
import 'package:rblue_flutter/rblue_flutter.dart';

import '../app_state.dart';
import '../widgets/expressive_button.dart';
import 'scan_page.dart' show MaterialBannerTile;

/// The prefilled supervisor policy shown as a read-only summary.
const supervisorPolicy = ReconnectPolicy(
  initialBackoff: Duration(milliseconds: 500),
  maxBackoff: Duration(seconds: 30),
  jitterFraction: 0.20,
);

/// Reconnect tab: starts the application-level supervisor
/// ([RblueClient.autoConnect] / [RblueClient.unwatch]) for a selected device.
/// After a drop, the supervisor retries with the prefilled backoff policy
/// (500 ms start, 30 s cap, ±20% jitter). This is not the platform
/// `connect(autoConnect)` flag on the device page.
class ReconnectPage extends StatefulWidget {
  const ReconnectPage({super.key});

  @override
  State<ReconnectPage> createState() => _ReconnectPageState();
}

/// One line of the supervisor activity log.
class LogEntry {
  /// Time of the entry, relative to the session start.
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
  SupervisorSession? session;
  bool restoreNotifications = true;

  @override
  void dispose() {
    session?.dispose();
    super.dispose();
  }

  void _select(DiscoveredDevice selected) {
    session?.dispose();
    setState(() {
      selectedId = selected.id;
      device = selected;
      session = null;
    });
  }

  Future<void> _start() async {
    final state = AppStateScope.of(context);
    final selected = device;
    if (selected == null || state.client == null || session != null) return;
    final next = SupervisorSession(
      client: state.client!,
      state: state,
      device: selected,
      restoreNotifications: restoreNotifications,
      onUpdate: () {
        if (mounted) setState(() {});
      },
    );
    setState(() => session = next);
    await next.start();
    if (mounted) setState(() {});
  }

  Future<void> _stop() async {
    final current = session;
    if (current == null) return;
    await current.stop();
    if (mounted) setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    final state = AppStateScope.of(context);
    final unsupported = state.isUnsupported('auto_connect_supervisor');
    final running = session?.running ?? false;
    return ListView(
      key: const PageStorageKey('reconnect'),
      padding: const EdgeInsets.all(16),
      children: [
        const Text('这里启动的是应用监督器，断线后按退避策略重试。这不是设备页里的系统后台连接。'),
        const SizedBox(height: 12),
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
                  onChanged: state.ready && !running
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
                  onChanged: running
                      ? null
                      : (value) =>
                            setState(() => restoreNotifications = value!),
                ),
                const SizedBox(height: 16),
                SquishButtonGroup(
                  actions: [
                    (
                      label: '开始监督',
                      tonal: false,
                      onPressed:
                          device == null ||
                              !state.ready ||
                              running ||
                              unsupported
                          ? null
                          : _start,
                    ),
                    (
                      label: '停止监督',
                      tonal: true,
                      onPressed: running ? _stop : null,
                    ),
                  ],
                ),
                if (unsupported) ...[
                  const SizedBox(height: 12),
                  const Text('当前构建未提供应用监督器。'),
                ],
                if (session != null) ...[
                  const SizedBox(height: 16),
                  Text(
                    session!.statusText,
                    style: Theme.of(context).textTheme.titleSmall,
                  ),
                ],
              ],
            ),
          ),
        ),
        const Padding(
          padding: EdgeInsets.symmetric(vertical: 12),
          child: Text('500ms 起始退避 · 30s 上限 · ±20% jitter'),
        ),
        if (session?.lastError != null)
          MaterialBannerTile(
            icon: Icons.error_outline,
            text: session!.lastError!,
          ),
        for (final entry in session?.log ?? <LogEntry>[])
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

/// Host-side session around the application supervisor: start/stop the
/// native supervisor, record its events, and optionally rediscover plus
/// resubscribe after a successful reconnect.
class SupervisorSession {
  SupervisorSession({
    required this.client,
    required this.state,
    required this.device,
    required this.restoreNotifications,
    required this.onUpdate,
  }) : _startedAt = DateTime.now(),
       _wanted = restoreNotifications
           ? state.subscriptions
                 .where((entry) => entry.$1 == device.id)
                 .map((entry) => entry.$2)
                 .toList()
           : const <CharacteristicId>[];

  final RblueClient client;
  final AppState state;
  final DiscoveredDevice device;
  final bool restoreNotifications;
  final void Function() onUpdate;
  final DateTime _startedAt;
  final List<CharacteristicId> _wanted;
  final log = <LogEntry>[];
  StreamSubscription<SupervisorEvent>? _supervisorSub;
  StreamSubscription<AdapterEvent>? _eventsSub;
  var running = false;
  String statusText = '未开始';
  String? lastError;

  Future<void> start() async {
    if (running) return;
    lastError = null;
    _supervisorSub = client.supervisorEvents.listen(_onSupervisor);
    _eventsSub = client.events.listen(_onAdapter);
    try {
      await state.operation(
        'auto_connect / ${device.id.display}',
        () => client.autoConnect(device.id, policy: supervisorPolicy),
      );
      running = true;
      statusText = '监督中';
      _log('监督中');
    } on RblueException catch (error) {
      await _cancelSubscriptions();
      if (error.kind == RblueErrorKind.unsupported) {
        state.markUnsupported('auto_connect_supervisor');
        lastError = '当前构建未提供应用监督器。';
      } else {
        lastError = error.message;
      }
      statusText = lastError!;
      _log(lastError!, isError: true);
    }
  }

  Future<void> stop() async {
    if (!running && _supervisorSub == null) return;
    try {
      await state.operation(
        'unwatch / ${device.id.display}',
        () => client.unwatch(device.id),
      );
    } on RblueException catch (error) {
      if (error.kind == RblueErrorKind.unsupported) {
        state.markUnsupported('auto_connect_supervisor');
        lastError = '当前构建未提供应用监督器。';
      } else {
        lastError = error.message;
        _log(error.message, isError: true);
      }
    }
    running = false;
    statusText = '已停止';
    _log('已停止');
    _detach();
  }

  void dispose() {
    unawaited(stop());
  }

  void _onSupervisor(SupervisorEvent event) {
    if (!running) return;
    switch (event) {
      case SupervisorConnected(:final id) when id == device.id:
        statusText = '已重新连接';
        _log('已重新连接');
        unawaited(_restoreSession());
      case SupervisorReconnectScheduled(:final id, :final delay, :final attempt)
          when id == device.id:
        statusText = '正在退避';
        _log('正在退避（第 $attempt 次，${delay.inMilliseconds} ms）');
      case SupervisorAttemptFailed(
            :final id,
            :final attempt,
            :final platform133,
            :final nextRetry,
          )
          when id == device.id:
        final extra = [
          if (platform133) 'GATT 133',
          if (nextRetry != null) '下次 ${nextRetry.inMilliseconds} ms',
        ];
        final detail = extra.isEmpty ? '' : '（${extra.join('，')}）';
        lastError = '重连尝试失败（第 $attempt 次）$detail';
        _log(lastError!, isError: true);
      case SupervisorBreakerOpen(:final id, :final consecutiveFailures)
          when id == device.id:
        lastError = '熔断已打开（连续失败 $consecutiveFailures 次）';
        statusText = lastError!;
        _log(lastError!, isError: true);
      case SupervisorGaveUp(:final id, :final consecutiveFailures)
          when id == device.id:
        running = false;
        lastError = '监督器已放弃（连续失败 $consecutiveFailures 次）';
        statusText = lastError!;
        _log(lastError!, isError: true);
      default:
        break;
    }
  }

  void _onAdapter(AdapterEvent event) {
    if (!running) return;
    switch (event) {
      case AdapterDeviceDisconnected(:final id) when id == device.id:
        statusText = '已断开';
        _log('已断开');
      default:
        break;
    }
  }

  Future<void> _restoreSession() async {
    try {
      final services = await client.discoverServices(device.id);
      if (!running) return;
      state.invalidateSubscriptions(device.id);
      for (final id in _wanted) {
        if (!running) return;
        final fresh = services.identifiedCharacteristics
            .where((entry) => entry.$1 == id && entry.$2.notifiable)
            .firstOrNull;
        if (fresh != null) {
          await state.setSubscription(device.id, fresh.$1, true);
        } else {
          state.record(
            'reconnect.subscription',
            '特征已不存在: ${id.uuid} #${id.instance}',
            isError: true,
          );
        }
      }
      if (!running) return;
      _log('已重新发现服务并完成订阅恢复');
    } on RblueException catch (error) {
      if (!running) return;
      lastError = error.message;
      _log(error.message, isError: true);
    }
  }

  Future<void> _cancelSubscriptions() async {
    _detach();
  }

  void _detach() {
    unawaited(_supervisorSub?.cancel());
    unawaited(_eventsSub?.cancel());
    _supervisorSub = null;
    _eventsSub = null;
  }

  void _log(String text, {bool isError = false}) {
    log.insert(
      0,
      LogEntry(DateTime.now().difference(_startedAt), text, isError: isError),
    );
    if (log.length > 50) log.removeLast();
    state.record('reconnect', text, isError: isError);
    onUpdate();
  }
}

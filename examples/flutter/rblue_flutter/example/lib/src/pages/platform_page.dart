import 'package:flutter/material.dart';
import 'package:rblue_flutter/rblue_flutter.dart';

import '../app_state.dart';
import '../widgets/expressive_button.dart';

/// Platform tab: what this build runs on, the rblue backend version, and the
/// platform facts a BLE app has to deal with.
class PlatformPage extends StatefulWidget {
  const PlatformPage({super.key});

  @override
  State<PlatformPage> createState() => _PlatformPageState();
}

class _PlatformPageState extends State<PlatformPage> {
  String? backendVersion;
  String? error;
  bool _startedLoading = false;
  bool _powerBusy = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_startedLoading || !AppStateScope.of(context).ready) return;
    _startedLoading = true;
    _loadVersion();
  }

  Future<void> _loadVersion() async {
    final state = AppStateScope.of(context);
    final client = state.client;
    if (client == null) return;
    try {
      final version = await client.backendVersion();
      if (mounted) setState(() => backendVersion = version);
    } catch (e) {
      if (mounted) setState(() => error = e.toString());
    }
  }

  Future<void> _setPowered(AppState state, {required bool powered}) async {
    if (_powerBusy) return;
    setState(() => _powerBusy = true);
    try {
      await state.setPowered(powered: powered);
    } catch (_) {
      // [AppState.setPowered] already records the error in the session log.
    } finally {
      if (mounted) setState(() => _powerBusy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final state = AppStateScope.of(context);
    final platform = Theme.of(context).platform;
    final canPower =
        state.hasCapability(Capability.adapterPowerControl) &&
        !state.isUnsupported('set_powered');
    return ListView(
      padding: const EdgeInsets.all(16),
      children: [
        ListTile(
          leading: const Icon(Icons.memory),
          title: const Text('后端版本'),
          subtitle: Text(backendVersion ?? error ?? '读取中…'),
        ),
        ListTile(
          title: const Text('后端能力'),
          subtitle: _CapabilitiesView(bits: state.capabilities),
        ),
        ListTile(
          leading: const Icon(Icons.radio_button_checked),
          title: const Text('适配器状态'),
          subtitle: Text(state.adapterState.name),
        ),
        if (canPower)
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: SquishButtonGroup(
              actions: [
                (
                  label: '打开蓝牙',
                  tonal: false,
                  onPressed:
                      _powerBusy || state.adapterState == AdapterState.poweredOn
                      ? null
                      : () => _setPowered(state, powered: true),
                ),
                (
                  label: '关闭蓝牙',
                  tonal: true,
                  onPressed:
                      _powerBusy ||
                          state.adapterState == AdapterState.poweredOff
                      ? null
                      : () => _setPowered(state, powered: false),
                ),
              ],
            ),
          )
        else if (state.ready)
          const Padding(
            padding: EdgeInsets.fromLTRB(16, 0, 16, 8),
            child: Text('当前平台不能由本应用开关蓝牙电台'),
          ),
        ListTile(
          leading: const Icon(Icons.lock),
          title: const Text('权限'),
          subtitle: Text(state.permissionsGranted ? '已授予' : '未授予'),
        ),
        ListTile(
          leading: const Icon(Icons.message),
          title: const Text('最近通知'),
          subtitle: state.lastNotification == null
              ? const Text('暂无')
              : Text(
                  '${state.lastNotification!.id.uuid}: '
                  '${state.lastNotification!.dataAsHex}',
                ),
        ),
        const SizedBox(height: 16),
        Text('平台说明', style: Theme.of(context).textTheme.titleMedium),
        const _NoteCard(
          title: '系统后台连接与应用监督器',
          text:
              '系统后台连接是设备页里的平台 connect(autoConnect) 标志。应用监督器在重连页启动，断线后按退避策略重试。',
        ),
        if (platform == TargetPlatform.iOS) ...[
          const _NoteCard(
            title: 'iOS 蓝牙连接',
            text: '设备以 UUID 标识。按 GATT 服务 UUID 查找系统已连接的外设后，仍需在本 App 中连接才能读写。',
          ),
          const _NoteCard(
            title: 'iOS 写入长度',
            text: 'ATT MTU 由系统管理。写入时以当前特征和写入方式显示的最大长度为准。',
          ),
        ],
        if (platform == TargetPlatform.android) ...[
          const _NoteCard(
            title: 'Android 蓝牙权限',
            text: 'Android 12 及以上需要附近设备权限，较早版本扫描时需要位置权限。',
          ),
          const _NoteCard(
            title: 'Android GATT 133',
            text: '连接失败时可在日志里查看后端返回的原始状态码。',
          ),
        ],
        const _NoteCard(
          title: '写入行为',
          text:
              '关闭「允许长写」时，超长写入会返回 InvalidLength，不会截断或自动分包。打开后按 MTU 顺序分段写入，不保证外设协议层会重组。',
        ),
      ],
    );
  }
}

class _CapabilitiesView extends StatelessWidget {
  const _CapabilitiesView({required this.bits});

  final int? bits;

  static const _items = <(String, int)>[
    ('应用内配对', Capability.pairing),
    ('MTU 查询', Capability.mtuQuery),
    ('MTU 协商', Capability.mtuNegotiation),
    ('系统后台连接', Capability.autoConnect),
    ('电台控制', Capability.adapterPowerControl),
  ];

  @override
  Widget build(BuildContext context) {
    if (bits == null) return const Text('尚未读取');
    final lines = [
      for (final (label, bit) in _items)
        '$label：${Capability.has(bits!, bit) ? "已支持" : "不支持"}',
      '0x${bits!.toRadixString(16)}',
    ];
    return Text(lines.join('\n'));
  }
}

class _NoteCard extends StatelessWidget {
  const _NoteCard({required this.title, required this.text});

  final String title;
  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: Card(
        child: Padding(
          padding: const EdgeInsets.all(12),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(title, style: Theme.of(context).textTheme.titleSmall),
              const SizedBox(height: 4),
              Text(text),
            ],
          ),
        ),
      ),
    );
  }
}

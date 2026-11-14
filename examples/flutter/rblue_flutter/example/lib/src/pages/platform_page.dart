import 'package:flutter/material.dart';

import '../app_state.dart';

/// Platform tab: what this build runs on, the rblue backend version, and the
/// platform facts a BLE app has to deal with.
class PlatformPage extends StatefulWidget {
  const PlatformPage({super.key});

  @override
  State<PlatformPage> createState() => _PlatformPageState();
}

class _PlatformPageState extends State<PlatformPage> {
  String? backendVersion;
  int? capabilities;
  String? error;
  bool _startedLoading = false;

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
      final bits = await client.capabilities();
      if (mounted) {
        setState(() {
          backendVersion = version;
          capabilities = bits;
        });
      }
    } catch (e) {
      if (mounted) setState(() => error = e.toString());
    }
  }

  @override
  Widget build(BuildContext context) {
    final state = AppStateScope.of(context);
    final platform = Theme.of(context).platform;
    return ListView(
      padding: const EdgeInsets.all(16),
      children: [
        ListTile(
          leading: const Icon(Icons.memory),
          title: const Text('rblue backend'),
          subtitle: Text(backendVersion ?? error ?? 'loading...'),
        ),
        ListTile(
          title: const Text('Backend capabilities'),
          subtitle: Text(
            capabilities == null
                ? '尚未读取'
                : '0x${capabilities!.toRadixString(16)}',
          ),
        ),
        ListTile(
          leading: const Icon(Icons.radio_button_checked),
          title: const Text('Adapter state'),
          subtitle: Text(state.adapterState.name),
        ),
        ListTile(
          leading: const Icon(Icons.lock),
          title: const Text('Permissions granted'),
          subtitle: Text(state.permissionsGranted ? 'yes' : 'no'),
        ),
        ListTile(
          leading: const Icon(Icons.message),
          title: const Text('Last notification'),
          subtitle: state.lastNotification == null
              ? const Text('none yet')
              : Text(
                  '${state.lastNotification!.id.uuid}: '
                  '${state.lastNotification!.dataAsHex}',
                ),
        ),
        const SizedBox(height: 16),
        Text('平台说明', style: Theme.of(context).textTheme.titleMedium),
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
          text: '超出单次写入上限时，应用会提示错误。需要分包时，请按外设协议拆分数据。',
        ),
      ],
    );
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

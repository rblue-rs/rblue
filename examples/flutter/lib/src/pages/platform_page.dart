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
    return ListView(
      padding: const EdgeInsets.all(16),
      children: [
        Text('能力与环境', style: Theme.of(context).textTheme.titleMedium),
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
        Text('Platform notes', style: Theme.of(context).textTheme.titleMedium),
        const _NoteCard(
          title: 'No Bluetooth on simulators',
          text:
              'The Android emulator and the iOS simulator have no Bluetooth '
              'radio. Run on real hardware; the build itself works anywhere.',
        ),
        const _NoteCard(
          title: 'Android permissions',
          text:
              'API 31+ needs BLUETOOTH_SCAN and BLUETOOTH_CONNECT at '
              'runtime; API 30 and below need location permission for '
              'scanning. The example requests them before initialize().',
        ),
        const _NoteCard(
          title: 'Android GATT status 133',
          text:
              'The leading connect-failure cause. rblue preserves the raw '
              'code (Error::Platform); the reconnect demo special-cases the '
              'copy for it.',
        ),
        const _NoteCard(
          title: 'iOS specifics',
          text:
              'Info.plist carries NSBluetoothAlwaysUsageDescription. iOS '
              'never exposes a MAC address; identities are platform UUIDs '
              'that change when the app is reinstalled. The ATT MTU is '
              'system-managed.',
        ),
        const _NoteCard(
          title: 'Write semantics',
          text:
              'Oversize writes always fail with InvalidLength - values are '
              'never silently truncated (iOS native behavior) and never '
              'hiddenly chunked.',
        ),
        const _NoteCard(
          title: '当前门面边界',
          text: '主动 MTU 协商、配对、描述符读写尚未暴露；能力位只是后端声明，实际操作仍需处理 Unsupported。',
        ),
        const _NoteCard(
          title: 'Initialization order',
          text:
              'On Android the plugin runs the JNI init '
              '(com.rblue.RblueNative.nativeInit) before the first FFI call; '
              'iOS needs no such step. See the plugin README.',
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
    return Card(
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
    );
  }
}

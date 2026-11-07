import 'package:flutter/material.dart';

import '../app_state.dart';

/// Session-wide notification history, retained when switching destinations.
class NotificationsPage extends StatelessWidget {
  const NotificationsPage({super.key, required this.frozenPackets});

  final List<ReceivedPacket>? frozenPackets;

  @override
  Widget build(BuildContext context) {
    final state = AppStateScope.of(context);
    final packets = frozenPackets ?? state.packets.toList();
    final colors = Theme.of(context).colorScheme;
    return ListView(
      key: const PageStorageKey('notifications'),
      padding: const EdgeInsets.all(16),
      children: [
        Text(
          '${state.notificationCount} packets · ${state.subscriptions.length} 个订阅 · 最多保留 200 条',
        ),
        const SizedBox(height: 12),
        if (frozenPackets != null)
          const Padding(
            padding: EdgeInsets.symmetric(vertical: 8),
            child: Text('显示已暂停，后台仍在接收和计数。'),
          ),
        for (final subscription in state.subscriptions.toList())
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: Card(
              child: ListTile(
                title: Text(subscription.$2.uuid),
                subtitle: Text('订阅中 · ${subscription.$1.display}'),
                trailing: IconButton(
                  tooltip: '取消订阅',
                  icon: const Icon(Icons.stop_circle_outlined),
                  onPressed: () async {
                    try {
                      await state.setSubscription(
                        subscription.$1,
                        subscription.$2,
                        false,
                      );
                    } catch (error) {
                      if (context.mounted) {
                        ScaffoldMessenger.of(context)
                            .showSnackBar(SnackBar(content: Text('$error')));
                      }
                    }
                  },
                ),
              ),
            ),
          ),
        const SizedBox(height: 12),
        if (packets.isEmpty)
          const Padding(
            padding: EdgeInsets.all(24),
            child: Text(
              '暂无数据。请在设备详情中选择支持通知的特征并订阅。',
              textAlign: TextAlign.center,
            ),
          ),
        for (final packet in packets)
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: Card(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      '${packet.time.toLocal().toString().substring(11, 23)} · ${packet.value.data.length} bytes',
                      style: Theme.of(context).textTheme.labelMedium,
                    ),
                    const SizedBox(height: 6),
                    Text(
                      '${packet.value.device.display}\n${packet.value.id.service.uuid} / #${packet.value.id.service.instance}\n${packet.value.id.uuid} / #${packet.value.id.instance}',
                      style: Theme.of(context).textTheme.bodySmall,
                    ),
                    const SizedBox(height: 8),
                    SelectableText(
                      packet.value.dataAsHex,
                      style: TextStyle(
                        fontFamily: 'monospace',
                        color: colors.primary,
                      ),
                    ),
                    const SizedBox(height: 4),
                    SelectableText(packet.value.dataAsText),
                  ],
                ),
              ),
            ),
          ),
      ],
    );
  }
}

import 'package:flutter/material.dart';
import 'package:rblue_flutter/rblue_flutter.dart';

import '../app_state.dart';
import '../widgets/expressive_button.dart';

/// Mobile scan overview: search/filter discoveries before entering the inspector.
class ScanPage extends StatefulWidget {
  const ScanPage({super.key, required this.onOpenDevice});
  final ValueChanged<DiscoveredDevice> onOpenDevice;

  @override
  State<ScanPage> createState() => _ScanPageState();
}

class _ScanPageState extends State<ScanPage> {
  String query = '';
  int filter = 0;

  @override
  Widget build(BuildContext context) {
    final state = AppStateScope.of(context);
    final colors = Theme.of(context).colorScheme;
    final text = Theme.of(context).textTheme;
    final devices = state.sortedDevices().where((device) {
      final matches = '${device.displayName} ${device.id.display}'
          .toLowerCase()
          .contains(query.toLowerCase());
      return matches &&
          switch (filter) {
            1 => device.advertisement.isConnectable != false,
            2 => (device.advertisement.rssi ?? -999) > -60,
            _ => true,
          };
    }).toList();
    return CustomScrollView(
      key: const PageStorageKey('scan'),
      keyboardDismissBehavior: ScrollViewKeyboardDismissBehavior.onDrag,
      slivers: [
        SliverPadding(
          padding: const EdgeInsets.fromLTRB(16, 8, 16, 24),
          sliver: SliverList.list(
            children: [
              _Banner(state: state),
              Container(
                padding: const EdgeInsets.all(24),
                decoration: BoxDecoration(
                  color: colors.primaryContainer,
                  borderRadius: BorderRadius.circular(28),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      'RBLUE / BLE CENTRAL',
                      style: text.labelSmall?.copyWith(
                        color: colors.onPrimaryContainer,
                        letterSpacing: 1,
                      ),
                    ),
                    const SizedBox(height: 12),
                    Text(
                      '连接，清晰可见。',
                      style: text.headlineLarge?.copyWith(
                        color: colors.onPrimaryContainer,
                      ),
                    ),
                    const SizedBox(height: 8),
                    Text(
                      '发现设备，探索服务，读懂每一个字节。',
                      style: text.bodyMedium?.copyWith(
                        color: colors.onPrimaryContainer,
                      ),
                    ),
                    const SizedBox(height: 20),
                    SizedBox(
                      width: double.infinity,
                      child: ExpressiveButton(
                        key: const Key('scan-toggle'),
                        label: state.starting
                            ? '初始化中'
                            : state.scanPending
                            ? '处理中'
                            : state.scanning
                            ? '停止扫描'
                            : '开始扫描',
                        icon: state.scanning
                            ? Icons.stop_rounded
                            : Icons.radar_rounded,
                        selected: state.scanning,
                        onPressed:
                            state.ready &&
                                !state.scanPending &&
                                state.permissionsGranted &&
                                state.adapterState != AdapterState.poweredOff
                            ? state.toggleScan
                            : null,
                      ),
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 12),
              Container(
                padding: const EdgeInsets.all(14),
                decoration: BoxDecoration(
                  color: colors.tertiaryContainer,
                  borderRadius: BorderRadius.circular(16),
                ),
                child: Wrap(
                  spacing: 16,
                  runSpacing: 4,
                  children: [
                    Text(
                      '${state.connected.length} 台已连接',
                      style: TextStyle(color: colors.onTertiaryContainer),
                    ),
                    Text(
                      '${state.devices.length} 台已发现 · ${state.notificationCount} RX',
                      style: TextStyle(color: colors.onTertiaryContainer),
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 24),
              Row(
                children: [
                  Expanded(child: Text('附近设备', style: text.titleMedium)),
                  Text(
                    state.scanning ? '扫描中' : '扫描已停止',
                    style: text.labelMedium,
                  ),
                ],
              ),
              const SizedBox(height: 12),
              TextField(
                key: const Key('device-search'),
                decoration: const InputDecoration(
                  prefixIcon: Icon(Icons.search),
                  labelText: '搜索名称 / 设备标识',
                ),
                onChanged: (value) => setState(() => query = value),
              ),
              const SizedBox(height: 12),
              Wrap(
                spacing: 8,
                runSpacing: 8,
                children: [
                  for (final (index, label) in [
                    '全部设备',
                    '可连接',
                    '强信号 > -60',
                  ].indexed)
                    FilterChip(
                      label: Text(label),
                      selected: filter == index,
                      onSelected: (_) => setState(() => filter = index),
                    ),
                ],
              ),
              const SizedBox(height: 12),
              if (devices.isEmpty)
                Padding(
                  padding: const EdgeInsets.symmetric(vertical: 28),
                  child: Text(
                    state.devices.isEmpty
                        ? '尚未发现设备\n开始扫描，并确认外设已开启广播。'
                        : '没有匹配的设备，试试调整搜索或筛选。',
                    textAlign: TextAlign.center,
                  ),
                ),
              for (final device in devices) ...[
                _DeviceTile(
                  state: state,
                  device: device,
                  onTap: () => widget.onOpenDevice(device),
                ),
                const SizedBox(height: 4),
              ],
              const SizedBox(height: 8),
              Text('标识去重 · RSSI 排序 · 实际适配器数据', style: text.bodySmall),
            ],
          ),
        ),
      ],
    );
  }
}

/// Status banners: start-up errors, permission gaps, radio state.
class _Banner extends StatelessWidget {
  const _Banner({required this.state});

  final AppState state;

  @override
  Widget build(BuildContext context) {
    Widget? banner;
    if (state.lastError != null) {
      banner = MaterialBannerTile(
        icon: Icons.error_outline,
        text: state.lastError!,
        action: state.ready ? null : () => state.start(),
        actionLabel: state.ready ? null : '重试',
      );
    } else if (!state.permissionsGranted) {
      banner = const MaterialBannerTile(
        icon: Icons.lock_outline,
        text: 'Bluetooth permissions were not granted; scanning will fail.',
      );
    } else if (state.ready && state.adapterState == AdapterState.poweredOff) {
      banner = const MaterialBannerTile(
        icon: Icons.bluetooth_disabled,
        text: 'The Bluetooth radio is powered off. Turn it on and retry.',
      );
    }
    if (banner == null) return const SizedBox.shrink();
    return Padding(padding: const EdgeInsets.all(8), child: banner);
  }
}

/// Shared banner style, also used by the device page for GATT errors.
class MaterialBannerTile extends StatelessWidget {
  const MaterialBannerTile({
    super.key,
    required this.icon,
    required this.text,
    this.action,
    this.actionLabel,
  });

  final IconData icon;
  final String text;
  final VoidCallback? action;
  final String? actionLabel;

  @override
  Widget build(BuildContext context) {
    return Card(
      color: Theme.of(context).colorScheme.errorContainer,
      child: ListTile(
        leading: Icon(icon),
        title: Text(text),
        trailing: action == null
            ? null
            : TextButton(onPressed: action, child: Text(actionLabel!)),
      ),
    );
  }
}

class _DeviceTile extends StatelessWidget {
  const _DeviceTile({
    required this.state,
    required this.device,
    required this.onTap,
  });
  final AppState state;
  final DiscoveredDevice device;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).colorScheme;
    final connected = state.isConnected(device.id);
    final connectable = device.advertisement.isConnectable;
    return Material(
      color: connected ? colors.secondaryContainer : colors.surfaceContainerLow,
      borderRadius: BorderRadius.circular(16),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(16),
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Row(
            children: [
              Container(
                padding: const EdgeInsets.all(10),
                decoration: BoxDecoration(
                  color: colors.surfaceContainerLowest,
                  borderRadius: BorderRadius.circular(16),
                ),
                child: Icon(
                  connected ? Icons.bluetooth_connected : Icons.bluetooth,
                  color: colors.primary,
                ),
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      device.displayName,
                      style: Theme.of(context).textTheme.titleSmall,
                    ),
                    Text(
                      device.id.display,
                      style: Theme.of(context).textTheme.bodySmall,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                    const SizedBox(height: 4),
                    Text(
                      connected
                          ? '已连接'
                          : connectable == false
                          ? '仅广播'
                          : connectable == true
                          ? '可连接'
                          : '连接能力未知',
                      style: Theme.of(context).textTheme.labelSmall,
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 8),
              Text(
                '${device.advertisement.rssi ?? "—"}\ndBm',
                textAlign: TextAlign.end,
                style: Theme.of(context).textTheme.labelMedium,
              ),
            ],
          ),
        ),
      ),
    );
  }
}

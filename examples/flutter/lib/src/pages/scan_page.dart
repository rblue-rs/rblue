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
  static const _commonServices = [
    '0000180a-0000-1000-8000-00805f9b34fb', // Device Information
    '0000180f-0000-1000-8000-00805f9b34fb', // Battery Service
    '8327ad99-2d87-4a22-a8ce-6dd7971c0437', // RingConn data service
  ];

  String serviceQuery = '';
  String? lookupStatus;
  bool lookupPending = false;

  String? _serviceUuid(String value) {
    var uuid = value.trim().toLowerCase();
    if (RegExp(r'^[0-9a-f]{4}$').hasMatch(uuid)) uuid = '0000$uuid';
    if (RegExp(r'^[0-9a-f]{8}$').hasMatch(uuid)) {
      uuid = '$uuid-0000-1000-8000-00805f9b34fb';
    }
    if (!RegExp(
      r'^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$',
    ).hasMatch(uuid)) {
      return null;
    }
    return uuid;
  }

  Future<void> _findSystemConnected(
    AppState state,
    List<String> services,
  ) async {
    if (lookupPending) return;
    setState(() {
      lookupPending = true;
      lookupStatus = null;
    });
    try {
      final count = await state.findSystemConnected(services);
      if (!mounted) return;
      var status = count == 0
          ? '未找到匹配设备。可尝试自定义服务或普通扫描。'
          : '找到 $count 台。请选择设备并在本 App 中连接。';
      if (count == 0 && Theme.of(context).platform == TargetPlatform.android) {
        status = '$status系统 UUID 缓存为空的设备不会出现。';
      }
      setState(() => lookupStatus = status);
    } on RblueException catch (error) {
      if (!mounted) return;
      setState(() {
        lookupStatus = error.kind == RblueErrorKind.unsupported
            ? '当前平台不支持查找系统已连接设备。'
            : error.message;
      });
    } finally {
      if (mounted) setState(() => lookupPending = false);
    }
  }

  static const _filters = ['全部设备', '可连接', '强信号 > -60'];

  Future<void> _showOptions(AppState state) async {
    final form = GlobalKey<FormState>();
    final selection = await showModalBottomSheet<Object>(
      context: context,
      isScrollControlled: true,
      useSafeArea: true,
      showDragHandle: true,
      builder: (context) => SafeArea(
        top: false,
        child: SingleChildScrollView(
          padding: EdgeInsets.fromLTRB(
            20,
            0,
            20,
            20 + MediaQuery.viewInsetsOf(context).bottom,
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text('设备选项', style: Theme.of(context).textTheme.titleLarge),
              const SizedBox(height: 16),
              const Text('筛选附近设备'),
              Wrap(
                spacing: 8,
                children: [
                  for (final (index, label) in _filters.indexed)
                    FilterChip(
                      label: Text(label),
                      selected: filter == index,
                      onSelected: (_) => Navigator.pop(context, index),
                    ),
                ],
              ),
              const Divider(height: 24),
              ExpansionTile(
                tilePadding: EdgeInsets.zero,
                title: const Text('按自定义服务查找'),
                maintainState: true,
                children: [
                  Form(
                    key: form,
                    child: TextFormField(
                      initialValue: serviceQuery,
                      decoration: const InputDecoration(
                        labelText: 'GATT 服务 UUID',
                        hintText: '180F 或完整 UUID',
                      ),
                      autocorrect: false,
                      onChanged: (value) => serviceQuery = value,
                      validator: (value) => _serviceUuid(value ?? '') == null
                          ? '请输入有效的 GATT 服务 UUID'
                          : null,
                    ),
                  ),
                  TextButton(
                    onPressed:
                        state.ready &&
                            !lookupPending &&
                            !state.isUnsupported('system_connected_peripherals')
                        ? () {
                            if (form.currentState!.validate()) {
                              Navigator.pop(context, [
                                _serviceUuid(serviceQuery)!,
                              ]);
                            }
                          }
                        : null,
                    child: const Text('查找设备'),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
    if (!mounted) return;
    if (selection is int) setState(() => filter = selection);
    if (selection is List<String>) await _findSystemConnected(state, selection);
  }

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
                padding: const EdgeInsets.all(16),
                decoration: BoxDecoration(
                  color: colors.primaryContainer,
                  borderRadius: BorderRadius.circular(24),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Text(
                      '蓝牙扫描',
                      style: text.titleLarge?.copyWith(
                        color: colors.onPrimaryContainer,
                      ),
                    ),
                    const SizedBox(height: 4),
                    Text(
                      '${state.devices.length} 台已发现 · ${state.connected.length} 台已连接',
                      style: text.bodySmall?.copyWith(
                        color: colors.onPrimaryContainer,
                      ),
                    ),
                    const SizedBox(height: 12),
                    ExpressiveButton(
                      key: const Key('scan-toggle'),
                      height: 48,
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
                  ],
                ),
              ),
              const SizedBox(height: 8),
              Wrap(
                spacing: 8,
                runSpacing: 8,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: [
                  Text('附近设备', style: text.titleMedium),
                  ExpressiveButton(
                    key: const Key('find-system-connected'),
                    tonal: true,
                    height: 40,
                    label: '查找系统已连接',
                    icon: Icons.bluetooth_connected,
                    onPressed:
                        state.ready &&
                            !lookupPending &&
                            !state.isUnsupported('system_connected_peripherals')
                        ? () => _findSystemConnected(state, _commonServices)
                        : null,
                  ),
                  IconButton(
                    tooltip: filter == 0
                        ? '设备选项'
                        : '设备选项 · ${_filters[filter]}',
                    icon: Badge(
                      isLabelVisible: filter != 0,
                      child: const Icon(Icons.tune),
                    ),
                    onPressed: () => _showOptions(state),
                  ),
                ],
              ),
              if (state.isUnsupported('system_connected_peripherals') &&
                  lookupStatus == null)
                const Padding(
                  padding: EdgeInsets.only(bottom: 8),
                  child: Text('当前平台不支持查找系统已连接设备。'),
                ),
              if (filter != 0)
                Padding(
                  padding: const EdgeInsets.only(bottom: 8),
                  child: Text(
                    '筛选 · ${_filters[filter]}',
                    style: text.labelMedium,
                  ),
                ),
              if (lookupPending || lookupStatus != null)
                Padding(
                  padding: const EdgeInsets.only(bottom: 8),
                  child: Semantics(
                    liveRegion: true,
                    child: Text(lookupPending ? '正在查找系统已连接设备…' : lookupStatus!),
                  ),
                ),
              TextField(
                key: const PageStorageKey('device-search'),
                decoration: const InputDecoration(
                  prefixIcon: Icon(Icons.search),
                  labelText: '搜索名称 / 设备标识',
                ),
                onChanged: (value) => setState(() => query = value),
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
        text: '尚未授予蓝牙权限，扫描将会失败。',
      );
    } else if (state.ready && state.adapterState == AdapterState.poweredOff) {
      final canPower =
          state.hasCapability(Capability.adapterPowerControl) &&
          !state.isUnsupported('set_powered');
      banner = MaterialBannerTile(
        icon: Icons.bluetooth_disabled,
        text: canPower ? '蓝牙电台已关闭。' : '请在系统设置中打开蓝牙后重试。',
        action: canPower ? () => state.setPowered(powered: true) : null,
        actionLabel: canPower ? '打开蓝牙' : null,
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
                    if (state.isSystemConnected(device.id)) ...[
                      const SizedBox(height: 4),
                      Chip(
                        visualDensity: VisualDensity.compact,
                        labelPadding: const EdgeInsets.symmetric(horizontal: 4),
                        label: const Text('系统已连接'),
                      ),
                    ],
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

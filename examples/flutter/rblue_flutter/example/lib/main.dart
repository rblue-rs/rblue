import 'package:flutter/material.dart';
import 'package:rblue_flutter/rblue_flutter.dart';

import 'src/app_state.dart';
import 'src/pages/device_page.dart';
import 'src/pages/logs_page.dart';
import 'src/pages/notifications_page.dart';
import 'src/pages/platform_page.dart';
import 'src/pages/reconnect_page.dart';
import 'src/pages/scan_page.dart';
import 'src/theme.dart';
import 'src/widgets/spring_entrance.dart';

void main() => runApp(const RblueExampleApp());

/// Mobile M3 Expressive app using the existing rblue-c client, without demo data.
class RblueExampleApp extends StatefulWidget {
  const RblueExampleApp({super.key, this.state});
  final AppState? state;

  @override
  State<RblueExampleApp> createState() => _RblueExampleAppState();
}

class _RblueExampleAppState extends State<RblueExampleApp> {
  ThemeMode mode = ThemeMode.dark;

  @override
  Widget build(BuildContext context) => MaterialApp(
    title: 'rblue BLE Playground',
    debugShowCheckedModeBanner: false,
    theme: buildExampleTheme(Brightness.light),
    darkTheme: buildExampleTheme(Brightness.dark),
    themeMode: mode,
    home: HomePage(
      state: widget.state,
      onToggleTheme: (dark) => setState(() {
        mode = dark ? ThemeMode.dark : ThemeMode.light;
      }),
    ),
  );
}

/// Keeps section state and the bottom navigation alive in the device inspector.
class HomePage extends StatefulWidget {
  const HomePage({super.key, this.state, required this.onToggleTheme});
  final AppState? state;
  final ValueChanged<bool> onToggleTheme;

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  late final state = widget.state ?? AppState();
  int tab = 0;
  DiscoveredDevice? selected;
  List<ReceivedPacket>? frozenPackets;

  @override
  void initState() {
    super.initState();
    state.start();
  }

  @override
  void dispose() {
    if (widget.state == null) state.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AppStateScope(
    notifier: state,
    child: ListenableBuilder(
      listenable: state,
      builder: (context, _) {
        final inspector = tab == 0 && selected != null;
        final dark = Theme.of(context).brightness == Brightness.dark;
        return PopScope(
          canPop: !inspector,
          onPopInvokedWithResult: (didPop, _) {
            if (!didPop && inspector) setState(() => selected = null);
          },
          child: Scaffold(
            appBar: AppBar(
              title: Text(
                inspector
                    ? '设备详情'
                    : tab == 0
                    ? 'rblue.'
                    : _BottomToolbar.destinations[tab].$2,
                style: tab == 0 && !inspector
                    ? const TextStyle(fontSize: 28, fontWeight: FontWeight.w700)
                    : null,
              ),
              leading: inspector
                  ? IconButton(
                      tooltip: '返回设备列表',
                      onPressed: () => setState(() => selected = null),
                      icon: const Icon(Icons.arrow_back),
                    )
                  : null,
              actions: [
                if (!inspector && (tab == 0 || tab == 3))
                  IconButton(
                    tooltip: dark ? '切换浅色' : '切换深色',
                    icon: Icon(
                      dark
                          ? Icons.light_mode_outlined
                          : Icons.dark_mode_outlined,
                    ),
                    onPressed: () => widget.onToggleTheme(!dark),
                  ),
                if (inspector)
                  IconButton(
                    tooltip: '查看通知数据流',
                    icon: const Icon(Icons.graphic_eq),
                    onPressed: () => setState(() => tab = 1),
                  ),
                if (tab == 1) ...[
                  IconButton(
                    tooltip: frozenPackets == null ? '暂停显示' : '继续显示',
                    icon: Icon(
                      frozenPackets == null ? Icons.pause : Icons.play_arrow,
                    ),
                    onPressed: () => setState(() {
                      frozenPackets = frozenPackets == null
                          ? state.packets.toList()
                          : null;
                    }),
                  ),
                  IconButton(
                    tooltip: '清空数据',
                    icon: const Icon(Icons.delete_outline),
                    onPressed: () {
                      state.clearPackets();
                      if (frozenPackets != null) {
                        setState(() => frozenPackets = []);
                      }
                    },
                  ),
                ],
                if (tab == 4) ...[
                  IconButton(
                    tooltip: '复制会话 JSON',
                    icon: const Icon(Icons.copy_outlined),
                    onPressed: () => copySession(context, state),
                  ),
                  IconButton(
                    tooltip: '清空日志',
                    icon: const Icon(Icons.delete_outline),
                    onPressed: state.clearLogs,
                  ),
                ],
              ],
            ),
            body: SafeArea(
              top: false,
              child: SpringEntrance(
                section: (tab, tab == 0 ? selected?.id : null),
                child: IndexedStack(
                  index: tab,
                  children: [
                    IndexedStack(
                      index: selected == null ? 0 : 1,
                      children: [
                        ScanPage(
                          onOpenDevice: (device) =>
                              setState(() => selected = device),
                        ),
                        if (selected == null)
                          const SizedBox.shrink()
                        else
                          DeviceDetailPage(
                            key: ValueKey(selected!.id),
                            state: state,
                            device: selected!,
                          ),
                      ],
                    ),
                    NotificationsPage(frozenPackets: frozenPackets),
                    const ReconnectPage(),
                    const PlatformPage(),
                    const LogsPage(),
                  ],
                ),
              ),
            ),
            bottomNavigationBar: _BottomToolbar(
              selected: tab,
              onSelect: (index) => setState(() => tab = index),
            ),
          ),
        );
      },
    ),
  );
}

/// Floating toolbar for the example's five persistent destinations.
class _BottomToolbar extends StatelessWidget {
  const _BottomToolbar({required this.selected, required this.onSelect});

  final int selected;
  final ValueChanged<int> onSelect;

  static const destinations = [
    (Icons.radar, '设备'),
    (Icons.graphic_eq, '数据流'),
    (Icons.link, '重连'),
    (Icons.layers_outlined, '环境'),
    (Icons.terminal, '日志'),
  ];

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colors = theme.colorScheme;
    final duration = MediaQuery.disableAnimationsOf(context)
        ? Duration.zero
        : const Duration(milliseconds: 160);
    return SafeArea(
      top: false,
      minimum: const EdgeInsets.fromLTRB(12, 8, 12, 12),
      child: Center(
        heightFactor: 1,
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 460),
          child: Material(
            key: const Key('bottom-toolbar'),
            color: colors.surfaceContainerLow,
            shape: const StadiumBorder(),
            child: Padding(
              padding: const EdgeInsets.all(6),
              child: Row(
                children: [
                  for (final (index, (icon, label)) in destinations.indexed)
                    Expanded(
                      child: Semantics(
                        selected: index == selected,
                        button: true,
                        child: Material(
                          animationDuration: duration,
                          color: index == selected
                              ? colors.primary
                              : Colors.transparent,
                          shape: const StadiumBorder(),
                          child: InkWell(
                            customBorder: const StadiumBorder(),
                            onTap: () => onSelect(index),
                            child: ConstrainedBox(
                              constraints: const BoxConstraints(minHeight: 56),
                              child: Padding(
                                padding: const EdgeInsets.symmetric(
                                  vertical: 6,
                                ),
                                child: Column(
                                  mainAxisSize: MainAxisSize.min,
                                  mainAxisAlignment: MainAxisAlignment.center,
                                  children: [
                                    Icon(
                                      icon,
                                      size: 22,
                                      color: index == selected
                                          ? colors.onPrimary
                                          : colors.onSurfaceVariant,
                                    ),
                                    Text(
                                      label,
                                      textAlign: TextAlign.center,
                                      style: theme.textTheme.labelSmall
                                          ?.copyWith(
                                            color: index == selected
                                                ? colors.onPrimary
                                                : colors.onSurfaceVariant,
                                            fontWeight: index == selected
                                                ? FontWeight.w700
                                                : FontWeight.w500,
                                          ),
                                    ),
                                  ],
                                ),
                              ),
                            ),
                          ),
                        ),
                      ),
                    ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

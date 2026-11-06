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
  ThemeMode mode = ThemeMode.system;

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
        return PopScope(
          canPop: !inspector,
          onPopInvokedWithResult: (didPop, _) {
            if (!didPop && inspector) setState(() => selected = null);
          },
          child: Scaffold(
            appBar: AppBar(
              title: Text(
                inspector ? '设备详情' : 'rblue.',
                style: inspector
                    ? null
                    : const TextStyle(
                        fontSize: 28,
                        fontWeight: FontWeight.w700,
                      ),
              ),
              leading: inspector
                  ? IconButton(
                      tooltip: '返回设备列表',
                      onPressed: () => setState(() => selected = null),
                      icon: const Icon(Icons.arrow_back),
                    )
                  : null,
              actions: [
                IconButton(
                  tooltip: '复制会话 JSON',
                  onPressed: () => copySession(context, state),
                  icon: const Icon(Icons.ios_share),
                ),
                IconButton(
                  tooltip: '切换浅色 / 深色',
                  icon: Icon(
                    Theme.of(context).brightness == Brightness.dark
                        ? Icons.light_mode_outlined
                        : Icons.dark_mode_outlined,
                  ),
                  onPressed: () => widget.onToggleTheme(
                    Theme.of(context).brightness != Brightness.dark,
                  ),
                ),
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
                            onOpenNotifications: () => setState(() => tab = 1),
                          ),
                      ],
                    ),
                    const NotificationsPage(),
                    const ReconnectPage(),
                    const PlatformPage(),
                    const LogsPage(),
                  ],
                ),
              ),
            ),
            bottomNavigationBar: NavigationBar(
              selectedIndex: tab,
              onDestinationSelected: (index) => setState(() => tab = index),
              destinations: const [
                NavigationDestination(
                  icon: Icon(Icons.radar_outlined),
                  selectedIcon: Icon(Icons.radar),
                  label: '设备',
                ),
                NavigationDestination(
                  icon: Icon(Icons.graphic_eq_outlined),
                  selectedIcon: Icon(Icons.graphic_eq),
                  label: '数据流',
                ),
                NavigationDestination(
                  icon: Icon(Icons.link_outlined),
                  selectedIcon: Icon(Icons.link),
                  label: '重连',
                ),
                NavigationDestination(
                  icon: Icon(Icons.layers_outlined),
                  selectedIcon: Icon(Icons.layers),
                  label: '环境',
                ),
                NavigationDestination(
                  icon: Icon(Icons.terminal_outlined),
                  selectedIcon: Icon(Icons.terminal),
                  label: '日志',
                ),
              ],
            ),
          ),
        );
      },
    ),
  );
}

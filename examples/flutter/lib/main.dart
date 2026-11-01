import 'package:flutter/material.dart';

import 'src/app_state.dart';
import 'src/pages/platform_page.dart';
import 'src/pages/reconnect_page.dart';
import 'src/pages/scan_page.dart';

void main() {
  runApp(const RblueExampleApp());
}

/// Root of the rblue BLE usage example: one code base for Android and iOS,
/// driving the rblue-c C ABI through the local `rblue_flutter` plugin.
class RblueExampleApp extends StatelessWidget {
  const RblueExampleApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'rblue example',
      theme: ThemeData(colorSchemeSeed: Colors.indigo, useMaterial3: true),
      home: const HomePage(),
    );
  }
}

class HomePage extends StatefulWidget {
  const HomePage({super.key});

  @override
  State<HomePage> createState() => _HomePageState();
}

class _HomePageState extends State<HomePage> {
  late final AppState state;
  var tab = 0;

  static const pages = [ScanPage(), ReconnectPage(), PlatformPage()];

  @override
  void initState() {
    super.initState();
    state = AppState();
    state.start();
  }

  @override
  void dispose() {
    state.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AppStateScope(
      notifier: state,
      child: ListenableBuilder(
        listenable: state,
        builder: (context, _) => Scaffold(
          appBar: AppBar(
            title: Text('rblue example - ${state.runtimeLabel}'),
            actions: [
              if (state.ready)
                IconButton(
                  tooltip: state.scanning ? 'Stop scan' : 'Start scan',
                  icon: Icon(state.scanning ? Icons.stop : Icons.radar),
                  onPressed: state.toggleScan,
                ),
            ],
          ),
          body: pages[tab],
          bottomNavigationBar: NavigationBar(
            selectedIndex: tab,
            onDestinationSelected: (index) => setState(() => tab = index),
            destinations: const [
              NavigationDestination(icon: Icon(Icons.radar), label: 'Scan'),
              NavigationDestination(icon: Icon(Icons.link), label: 'Reconnect'),
              NavigationDestination(
                icon: Icon(Icons.info_outline),
                label: 'Platform',
              ),
            ],
          ),
        ),
      ),
    );
  }
}

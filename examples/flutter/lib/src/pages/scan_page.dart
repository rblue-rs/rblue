import 'package:flutter/material.dart';
import 'package:rblue_flutter/rblue_flutter.dart';

import '../app_state.dart';
import 'device_page.dart';

/// Scan tab: live list of seen devices, strongest signal first.
class ScanPage extends StatelessWidget {
  const ScanPage({super.key});

  @override
  Widget build(BuildContext context) {
    final state = AppStateScope.of(context);
    return Column(
      children: [
        _Banner(state: state),
        Expanded(
          child: state.devices.isEmpty
              ? const Center(
                  child: Text(
                    'No devices seen yet.\n'
                    'Start the scan and power up a peripheral.',
                    textAlign: TextAlign.center,
                  ),
                )
              : ListView(
                  children: [
                    for (final device in state.sortedDevices())
                      _DeviceTile(state: state, device: device),
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
        actionLabel: state.ready ? null : 'Retry',
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
  const _DeviceTile({required this.state, required this.device});

  final AppState state;
  final DiscoveredDevice device;

  @override
  Widget build(BuildContext context) {
    final connected = state.isConnected(device.id);
    final rssi = device.advertisement.rssi;
    return ListTile(
      leading: Icon(
        connected ? Icons.bluetooth_connected : Icons.bluetooth_searching,
        color: connected ? Colors.green : null,
      ),
      title: Text(device.displayName),
      subtitle: Text(
        '${device.id.display}\n'
        'services: ${device.advertisement.services.length}, '
        'connectable: ${device.advertisement.isConnectable ?? "?"}',
      ),
      isThreeLine: true,
      trailing: rssi == null ? null : Text('$rssi dBm'),
      onTap: () => Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => DeviceDetailPage(state: state, device: device),
        ),
      ),
    );
  }
}

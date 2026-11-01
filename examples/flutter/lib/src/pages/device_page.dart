import 'dart:async';

import 'package:flutter/material.dart';
import 'package:rblue_flutter/rblue_flutter.dart';

import '../app_state.dart';
import 'scan_page.dart' show MaterialBannerTile;

/// Device tab: connect, discover the GATT tree, read/write/subscribe, and
/// watch the notification console. Also demonstrates the unified write
/// semantics (an oversize write fails with `InvalidLength`, never truncates).
class DeviceDetailPage extends StatefulWidget {
  const DeviceDetailPage({super.key, required this.state, required this.device});

  final AppState state;
  final DiscoveredDevice device;

  @override
  State<DeviceDetailPage> createState() => _DeviceDetailPageState();
}

class _DeviceDetailPageState extends State<DeviceDetailPage> {
  static const oversizePayload = 600; // mock backends default to a 514 max

  RblueClient get client => widget.state.client!;
  DiscoveredDevice get device => widget.device;

  ServiceSet? services;
  String? error;
  bool connecting = false;
  final subscribed = <CharacteristicId>{};
  final _writeController = TextEditingController();
  StreamSubscription<DeviceNotification>? _notificationsSub;
  final _console = <String>[];

  @override
  void initState() {
    super.initState();
    _connectAndDiscover();
    _notificationsSub = client.notifications.listen(_onNotification);
  }

  @override
  void dispose() {
    _notificationsSub?.cancel();
    _writeController.dispose();
    super.dispose();
  }

  Future<void> _connectAndDiscover() async {
    setState(() {
      connecting = true;
      error = null;
    });
    try {
      await client.connect(device.id);
      services = await client.discoverServices(device.id);
      widget.state.connected.add(widget.state.deviceKey(device.id));
    } on RblueException catch (e) {
      error = _describe(e);
    } finally {
      if (mounted) {
        setState(() => connecting = false);
      }
    }
  }

  Future<void> _read(CharacteristicId characteristic) => _guarded(() async {
        final value = await client.read(device.id, characteristic);
        _console.insert(0, 'read ${characteristic.uuid}: ${_format(value)}');
      });

  Future<void> _write(CharacteristicId characteristic, Characteristic meta) =>
      _guarded(() async {
        final data = _writeController.text.codeUnits;
        await client.write(
          device.id,
          characteristic,
          data,
          mode: meta.writable ? WriteMode.withResponse : WriteMode.withoutResponse,
        );
        _console.insert(0, 'write ${characteristic.uuid}: ${_format(data)}');
      });

  Future<void> _writeOversize(CharacteristicId characteristic, Characteristic meta) =>
      _guarded(() async {
        // The unified write semantics: oversize is rejected with
        // InvalidLength - never truncated, never chunked.
        await client.write(
          device.id,
          characteristic,
          List.filled(oversizePayload, 0x41),
          mode: meta.writable ? WriteMode.withResponse : WriteMode.withoutResponse,
        );
        _console.insert(0, 'oversize write accepted?!');
      });

  Future<void> _toggleSubscription(CharacteristicId characteristic) =>
      _guarded(() async {
        if (subscribed.contains(characteristic)) {
          await client.unsubscribe(device.id, characteristic);
          subscribed.remove(characteristic);
        } else {
          await client.subscribe(device.id, characteristic);
          subscribed.add(characteristic);
        }
      });

  Future<void> _guarded(Future<void> Function() action) async {
    setState(() => error = null);
    try {
      await action();
    } on RblueException catch (e) {
      setState(() => error = _describe(e));
    } finally {
      if (mounted) setState(() {});
    }
  }

  void _onNotification(DeviceNotification notification) {
    if (!mounted) return;
    setState(() {
      _console.insert(
        0,
        'notify ${notification.id.uuid}: ${_format(notification.data)}',
      );
    });
  }

  String _describe(RblueException e) {
    if (e.isAndroidGattStatus133) {
      return 'Platform status 133 (GATT_ERROR): move the device closer, '
          're-scan, or toggle Bluetooth, then retry.';
    }
    return e.message;
  }

  String _format(List<int> data) {
    final text = String.fromCharCodes(data);
    final printable = data.isNotEmpty && data.every((byte) => byte >= 0x20 && byte < 0x7F);
    final hex = data.map((byte) => byte.toRadixString(16).padLeft(2, '0')).join(' ');
    return printable ? '$hex ("$text")' : hex;
  }

  @override
  Widget build(BuildContext context) {
    final connectedNow = widget.state.isConnected(device.id);
    return Scaffold(
      appBar: AppBar(
        title: Text(device.displayName),
      ),
      body: ListView(
        padding: const EdgeInsets.all(12),
        children: [
          Text(
            '${device.id.display} - ${connectedNow ? "connected" : "not connected"}',
            style: Theme.of(context).textTheme.bodySmall,
          ),
          if (connecting) const Padding(
              padding: EdgeInsets.all(12),
              child: Center(child: CircularProgressIndicator()),
            ),
          if (error != null)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 8),
              child: MaterialBannerTile(
                icon: Icons.error_outline,
                text: error!,
              ),
            ),
          if (services == null && !connecting)
            const Padding(
              padding: EdgeInsets.all(12),
              child: Text('Not discovered - connect first.'),
            ),
          if (services != null) ..._serviceCards(context),
        ],
      ),
    );
  }

  List<Widget> _serviceCards(BuildContext context) {
    final result = <Widget>[];
    for (final service in services!.services) {
      result.add(Card(
        child: Padding(
          padding: const EdgeInsets.all(12),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                '${service.primary ? "primary" : "secondary"} service',
                style: Theme.of(context).textTheme.labelSmall,
              ),
              Text(service.uuid, style: Theme.of(context).textTheme.titleSmall),
              const Divider(),
              for (final characteristic in service.characteristics)
                _CharacteristicTile(
                  parent: this,
                  serviceUuid: service.uuid,
                  characteristic: characteristic,
                ),
            ],
          ),
        ),
      ));
    }
    return result;
  }
}

class _CharacteristicTile extends StatelessWidget {
  const _CharacteristicTile({
    required this.parent,
    required this.serviceUuid,
    required this.characteristic,
  });

  final _DeviceDetailPageState parent;
  final String serviceUuid;
  final Characteristic characteristic;

  @override
  Widget build(BuildContext context) {
    final id = CharacteristicId(
      service: ServiceId(uuid: serviceUuid, instance: 0),
      uuid: characteristic.uuid,
      instance: 0,
    );
    final state = parent;
    final subscribed = state.subscribed.contains(id);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(characteristic.uuid, style: Theme.of(context).textTheme.bodyMedium),
          Wrap(
            spacing: 4,
            children: [
              if (characteristic.readable) const _PropertyChip('read'),
              if (characteristic.writable) const _PropertyChip('write'),
              if (characteristic.writableWithoutResponse)
                const _PropertyChip('write-nr'),
              if (characteristic.notifiable) const _PropertyChip('notify'),
            ],
          ),
          Wrap(
            spacing: 8,
            children: [
              if (characteristic.readable)
                OutlinedButton(
                  onPressed: () => state._read(id),
                  child: const Text('Read'),
                ),
              if (characteristic.notifiable)
                OutlinedButton(
                  onPressed: () => state._toggleSubscription(id),
                  child: Text(subscribed ? 'Unsubscribe' : 'Subscribe'),
                ),
            ],
          ),
          if (characteristic.writable || characteristic.writableWithoutResponse)
            Row(
              children: [
                Expanded(
                  child: TextField(
                    controller: state._writeController,
                    decoration: const InputDecoration(
                      hintText: 'value to write (UTF-8)',
                      isDense: true,
                    ),
                  ),
                ),
                const SizedBox(width: 8),
                OutlinedButton(
                  onPressed: () => state._write(id, characteristic),
                  child: const Text('Write'),
                ),
              ],
            ),
          if (characteristic.writable || characteristic.writableWithoutResponse)
            Row(
              children: [
                TextButton(
                  onPressed: () => state._writeOversize(id, characteristic),
                  child: const Text(
                    'Write 600 bytes (expect InvalidLength, never truncated)',
                  ),
                ),
              ],
            ),
        ],
      ),
    );
  }
}

class _PropertyChip extends StatelessWidget {
  const _PropertyChip(this.label);

  final String label;

  @override
  Widget build(BuildContext context) {
    return Chip(
      label: Text(label, style: Theme.of(context).textTheme.labelSmall),
      visualDensity: VisualDensity.compact,
    );
  }
}

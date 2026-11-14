import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:rblue_flutter/src/client.dart';
import 'package:rblue_flutter/src/models.dart';
import 'package:rblue_flutter_example/src/app_state.dart';

import '../../test/fake_bridge.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('AppState folds discovery, updates, and connections', () async {
    final bridge = FakeNativeBridge();
    final client = RblueClient.forBridge(bridge, bridge.create()!);
    final state = AppState(
      clientFactory: () async => client,
      permissionRequester: () async => true,
    );

    // start() runs initialize through the fake bridge; respond to it.
    final starting = state.start();
    while (bridge.sent.isEmpty) {
      await Future<void>.delayed(Duration.zero);
    }
    for (final call in bridge.sent.toList()) {
      bridge.respondTo(call.reqId);
    }
    await starting;
    expect(state.ready, isTrue);

    // A discovery, an RSSI update, and a connection event fold in.
    const id = {
      'mac': [
        [2, 0, 0, 0, 0, 1],
        'random_static',
      ],
    };
    final idJson = jsonEncode(id);
    bridge.emitEvent(
      '{"type":"adapter_event","event":{"device_discovered":{"id":$idJson,'
      '"advertisement":{"local_name":"simulated-sensor","services":[],'
      '"service_data":[],"manufacturer_data":null,"tx_power":null,'
      '"adv_flags":null,"rssi":-40,"is_connectable":true}}}}',
    );
    bridge.emitEvent(
      '{"type":"adapter_event","event":{"device_updated":[$idJson,-50]}}',
    );
    await Future<void>.delayed(Duration.zero);
    await Future<void>.delayed(Duration.zero);

    expect(state.devices, hasLength(1));
    expect(state.sortedDevices().single.displayName, 'simulated-sensor');
    expect(state.rssiOf(DeviceId.fromJson(id)), -50);

    bridge.emitEvent(
      '{"type":"adapter_event","event":{"device_connected":$idJson}}',
    );
    await Future<void>.delayed(Duration.zero);
    expect(state.isConnected(DeviceId.fromJson(id)), isTrue);

    client.dispose();
    state.dispose();
  });

  test('AppState reports permission failures as a banner state', () async {
    final bridge = FakeNativeBridge();
    final client = RblueClient.forBridge(bridge, bridge.create()!);
    final state = AppState(
      clientFactory: () async => client,
      permissionRequester: () async => false,
    );
    final starting = state.start();
    while (bridge.sent.isEmpty) {
      await Future<void>.delayed(Duration.zero);
    }
    for (final call in bridge.sent.toList()) {
      bridge.failRequest(
        call.reqId,
        '{"Permission":"bluetooth_permission_denied"}',
      );
    }
    await starting;

    expect(state.ready, isFalse);
    expect(state.permissionsGranted, isFalse);
    expect(state.lastError, contains('permission'));
    client.dispose();
    state.dispose();
  });
}

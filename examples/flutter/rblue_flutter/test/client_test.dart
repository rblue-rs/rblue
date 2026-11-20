import 'dart:async';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:rblue_flutter/src/client.dart';
import 'package:rblue_flutter/src/models.dart';

import 'fake_bridge.dart';

/// Starts a request, fails it with [statusJson], and returns the expectation.
///
/// The client sends synchronously before its first await, so the request just
/// started is the last one in `bridge.sent`.
Future<Object?> failsWith(
  FakeNativeBridge bridge,
  Future<Object?> future,
  String statusJson,
  Matcher matcher,
) {
  final call = bridge.sent.last;
  bridge.failRequest(call.reqId, statusJson);
  return expectLater(future, throwsA(matcher));
}

/// Swallows the expected failure so the test zone stays clean.
Future<void> quiet(Future<void> future) async {
  try {
    await future;
  } catch (_) {
    // expected by the caller's assertion path
  }
}

void main() {
  test(
    'peripherals decodes the native summary and manufacturer filter',
    () async {
      final bridge = FakeNativeBridge();
      final client = RblueClient.forBridge(bridge, bridge.create()!);
      final pending = client.peripherals();
      bridge.respondTo(
        bridge.sent.last.reqId,
        '[{"id":{"uuid":"device"},"name":"sensor"}]'.codeUnits,
      );
      final result = await pending;
      expect(result.single.name, 'sensor');
      expect(result.single.id.display, 'device');
      final scan = client.startScan(
        filter: const ScanFilter(
          manufacturerId: 42,
          manufacturerDataPrefix: [1, 2],
        ),
      );
      expect(bridge.sent.last.params, contains('"company_id":42'));
      bridge.respondTo(bridge.sent.last.reqId);
      await scan;
      client.dispose();
    },
  );

  test('device metrics use the native request protocol', () async {
    final bridge = FakeNativeBridge();
    final client = RblueClient.forBridge(bridge, bridge.create()!);
    final device = DeviceId.uuid('00000000-0000-0000-0000-00000000180f');
    final futures = [
      client.capabilities(),
      client.readRssi(device),
      client.mtu(device),
      client.maxWriteLength(device, _characteristic()),
    ];
    expect(bridge.sent.map((call) => call.method), [
      'capabilities',
      'rssi',
      'mtu',
      'max_write_len',
    ]);
    expect(bridge.sent.last.params, contains('with_response'));
    for (var i = 0; i < futures.length; i++) {
      bridge.respondTo(bridge.sent[i].reqId, [49, 50, 51]);
    }
    expect(await Future.wait(futures), [123, 123, 123, 123]);
    client.dispose();
  });

  test('responses are correlated by req_id, even out of order', () async {
    final bridge = FakeNativeBridge();
    final client = RblueClient.forBridge(bridge, bridge.create()!);

    // Two requests in flight; the second finishes first, like the Rust
    // out-of-order test in crates/rblue-c.
    final versionFuture = client.backendVersion();
    final initFuture = client.initialize();
    expect(bridge.sent.map((entry) => entry.method), [
      'backend_version',
      'initialize',
    ]);
    expect(bridge.sent[1].reqId, bridge.sent[0].reqId + 1);

    bridge.respondTo(2);
    bridge.respondTo(1, const [0x22, 0x6d, 0x6f, 0x63, 0x6b, 0x22]);

    expect(await versionFuture, 'mock');
    await initFuture;
    client.dispose();
  });

  test('monotonic req_ids and JSON params cross the bridge', () async {
    final bridge = FakeNativeBridge();
    final client = RblueClient.forBridge(bridge, bridge.create()!);
    final device = DeviceId.mac([2, 0, 0, 0, 0, 1], AddressType.randomStatic);

    unawaited(
      quiet(client.startScan(filter: const ScanFilter(namePrefix: 'sensor'))),
    );
    unawaited(quiet(client.connect(device)));
    expect(bridge.sent[0].params, contains('sensor'));
    expect(bridge.sent[1].params, contains('random_static'));
    client.dispose();
  });

  test('pair and unpair use the native request protocol', () async {
    final bridge = FakeNativeBridge();
    final client = RblueClient.forBridge(bridge, bridge.create()!);
    final device = DeviceId.mac([2, 0, 0, 0, 0, 1], AddressType.randomStatic);

    unawaited(quiet(client.pair(device)));
    unawaited(quiet(client.unpair(device)));
    expect(bridge.sent[0].method, 'pair');
    expect(bridge.sent[0].params, contains('random_static'));
    expect(bridge.sent[1].method, 'unpair');
    expect(bridge.sent[1].params, contains('random_static'));
    client.dispose();
  });

  test('status_json maps to typed errors, platform codes preserved', () async {
    final bridge = FakeNativeBridge();
    final client = RblueClient.forBridge(bridge, bridge.create()!);
    final device = DeviceId.uuid('00000000-0000-0000-0000-00000000180f');

    // Android GATT status 133 must keep its raw code for retry policies.
    await failsWith(
      bridge,
      client.connect(device),
      '{"Platform":{"code":133,"message":"GATT_ERROR"}}',
      isA<RblueException>()
          .having((e) => e.kind, 'kind', RblueErrorKind.platform)
          .having((e) => e.platformCode, 'platformCode', 133)
          .having((e) => e.isAndroidGattStatus133, 'is133', isTrue),
    );

    // InvalidLength carries len/max.
    await failsWith(
      bridge,
      client.write(device, _characteristic(), List.filled(600, 1)),
      '{"InvalidLength":{"len":600,"max":514}}',
      isA<RblueException>()
          .having((e) => e.kind, 'kind', RblueErrorKind.invalidLength)
          .having(
            (e) => e.message,
            'message',
            'data length 600 exceeds max 514',
          ),
    );

    // Facade-only argument errors surface as InvalidArgument.
    await failsWith(
      bridge,
      client.stopScan(),
      '{"InvalidArgument":{"message":"unknown method"}}',
      isA<RblueException>().having(
        (e) => e.kind,
        'kind',
        RblueErrorKind.invalidArgument,
      ),
    );
    client.dispose();
  });

  test('synchronous rejections throw without a completer', () async {
    final bridge = FakeNativeBridge();
    final client = RblueClient.forBridge(bridge, bridge.create()!);
    bridge.nextSyncCode = -1; // RBLUE_REQUEST_NULL_HANDLE
    await expectLater(
      client.initialize(),
      throwsA(
        isA<RblueException>().having(
          (e) => e.message,
          'message',
          contains('null handle'),
        ),
      ),
    );
    client.dispose();
  });

  test('adapter events and notifications land on their streams', () async {
    final bridge = FakeNativeBridge();
    final client = RblueClient.forBridge(bridge, bridge.create()!);

    final events = <AdapterEvent>[];
    final notifications = <DeviceNotification>[];
    final done = <Future<void>>[
      client.events.take(1).forEach(events.add),
      client.notifications.take(1).forEach(notifications.add),
    ];

    bridge.emitEvent(
      '{"type":"adapter_event","event":{"device_discovered":'
      '{"id":{"mac":[[2,0,0,0,0,1],"random_static"]},'
      '"advertisement":{"local_name":"sensor","services":[],'
      '"service_data":[],"manufacturer_data":null,"tx_power":null,'
      '"adv_flags":null,"rssi":-40,"is_connectable":true}}}}',
    );
    bridge.emitEvent(
      '{"type":"notification","device":{"mac":[[2,0,0,0,0,1],"random_static"]},'
      '"notification":{"id":{"service":{"uuid":"00000000-0000-0000-0000-00000000180f",'
      '"instance":0},"uuid":"00000000-0000-0000-0000-000000002a19","instance":0},'
      '"data":[104,105]}}',
    );
    await Future.wait(done);

    final discovered = events[0] as AdapterDeviceDiscovered;
    expect(discovered.device.displayName, 'sensor');
    expect(discovered.device.advertisement.rssi, -40);
    expect(notifications.single.dataAsText, 'hi');
    expect(
      notifications.single.id.uuid,
      '00000000-0000-0000-0000-000000002a19',
    );
    client.dispose();
  });

  test('dispose closes the handle and fails later calls', () async {
    final bridge = FakeNativeBridge();
    final client = RblueClient.forBridge(bridge, bridge.create()!);
    client.dispose();
    expect(bridge.disposed, isTrue);
    expect(() => client.initialize(), throwsA(isA<RblueException>()));
    // Idempotent.
    expect(() => client.dispose(), returnsNormally);
  });

  test('late responses after dispose are dropped silently', () async {
    final bridge = FakeNativeBridge();
    final client = RblueClient.forBridge(bridge, bridge.create()!);
    unawaited(quiet(client.initialize()));
    final call = bridge.sent.single;
    // Hold the registered handler: the real ABI guarantees no callback after
    // free, and a late delivery through a kept reference must still be
    // dropped silently instead of throwing into a completed completer.
    final handler = bridge.onResponse;
    client.dispose();
    expect(() => handler!(call.reqId, null, Uint8List(0)), returnsNormally);
  });
}

CharacteristicId _characteristic() => CharacteristicId(
  service: const ServiceId(
    uuid: '00000000-0000-0000-0000-00000000180f',
    instance: 0,
  ),
  uuid: '00000000-0000-0000-0000-000000002a19',
  instance: 0,
);

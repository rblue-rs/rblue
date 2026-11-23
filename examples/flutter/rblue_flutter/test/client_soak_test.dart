import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:rblue_flutter/src/client.dart';
import 'package:rblue_flutter/src/models.dart';

import 'fake_bridge.dart';

const _burst = 200;
const _notifyBacklog = 500;

void main() {
  test('out-of-order soak completes each req_id exactly once', () async {
    final bridge = FakeNativeBridge();
    final client = RblueClient.forBridge(bridge, bridge.create()!);

    final pending = <Future<String>>[
      for (var i = 0; i < _burst; i++) client.backendVersion(),
    ];
    expect(bridge.sent, hasLength(_burst));
    final reqIds = [for (final call in bridge.sent) call.reqId];

    // Complete newest-first so arrivals are deliberately out of send order.
    for (final reqId in reqIds.reversed) {
      bridge.respondTo(reqId, utf8.encode(jsonEncode('id-$reqId')));
    }

    final results = await Future.wait(pending);
    expect(results, hasLength(_burst));
    for (var i = 0; i < _burst; i++) {
      expect(results[i], 'id-${reqIds[i]}');
    }
    client.dispose();
  });

  test('notification backlog is delivered in order without loss', () async {
    final bridge = FakeNativeBridge();
    final client = RblueClient.forBridge(bridge, bridge.create()!);
    final done = client.notifications.take(_notifyBacklog).toList();

    for (var index = 0; index < _notifyBacklog; index++) {
      final data = [
        (index >> 24) & 0xff,
        (index >> 16) & 0xff,
        (index >> 8) & 0xff,
        index & 0xff,
      ];
      bridge.emitEvent(_notificationEnvelope(data));
    }

    final notifications = await done;
    expect(notifications, hasLength(_notifyBacklog));
    for (var index = 0; index < _notifyBacklog; index++) {
      expect(notifications[index].data, [
        (index >> 24) & 0xff,
        (index >> 16) & 0xff,
        (index >> 8) & 0xff,
        index & 0xff,
      ]);
    }
    client.dispose();
  });

  test(
    'dispose mid-flight fails pending requests and drops late completions',
    () async {
      final bridge = FakeNativeBridge();
      final client = RblueClient.forBridge(bridge, bridge.create()!);
      final pending = <Future<String>>[
        for (var i = 0; i < _burst; i++) client.backendVersion(),
      ];
      final reqIds = [for (final call in bridge.sent) call.reqId];
      final handler = bridge.onResponse;
      expect(handler, isNotNull);

      client.dispose();
      final outcomes = await Future.wait(
        pending.map(
          (future) => future.then<Object?>((value) => value, onError: (e) => e),
        ),
      );
      expect(
        outcomes,
        everyElement(
          isA<RblueException>().having(
            (e) => e.message,
            'message',
            contains('disposed'),
          ),
        ),
      );

      // A late native completion for a request that was in flight must not
      // throw (the completer is already finished) and must not re-complete.
      expect(
        () => handler!(
          reqIds.first,
          null,
          Uint8List.fromList(utf8.encode('"late"')),
        ),
        returnsNormally,
      );
    },
  );
}

String _notificationEnvelope(List<int> data) =>
    '{"type":"notification","device":{"mac":[[2,0,0,0,0,1],"random_static"]},'
    '"notification":{"id":{"service":{"uuid":"00000000-0000-0000-0000-00000000180f",'
    '"instance":0},"uuid":"00000000-0000-0000-0000-000000002a19","instance":0},'
    '"data":[${data.join(",")}]}}';

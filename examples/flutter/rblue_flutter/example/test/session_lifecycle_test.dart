import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rblue_flutter/rblue_flutter.dart';
import 'package:rblue_flutter_example/src/app_state.dart';
import 'package:rblue_flutter_example/src/pages/reconnect_page.dart';
import 'package:rblue_flutter_example/src/widgets/expressive_button.dart';

import 'ui_fixture.dart';

void main() {
  test(
    'late subscribe acknowledgement cannot restore an old connection handle',
    () async {
      final fixture = createUiFixture();
      await fixture.state.start();
      await fixture.state.client!.connect(testDeviceId);
      await Future<void>.delayed(Duration.zero);
      fixture.bridge.holdSubscription = true;
      const id = CharacteristicId(
        service: ServiceId(uuid: testService, instance: 0),
        uuid: testCharacteristic,
        instance: 0,
      );
      final pending = fixture.state.setSubscription(testDeviceId, id, true);
      await Future<void>.delayed(Duration.zero);
      final request = fixture.bridge.sent.lastWhere(
        (call) => call.method == 'subscribe',
      );
      await fixture.state.client!.disconnect(testDeviceId);
      await fixture.state.client!.connect(testDeviceId);
      await Future<void>.delayed(Duration.zero);
      fixture.bridge.respondTo(request.reqId);
      await pending;
      expect(fixture.state.subscriptions, isEmpty);
      fixture.state.dispose();
    },
  );

  test(
    'dispose during a permission prompt does not create a native client',
    () async {
      final permission = Completer<bool>();
      var factoryCalled = false;
      final state = AppState(
        permissionRequester: () => permission.future,
        clientFactory: () async {
          factoryCalled = true;
          throw StateError('unexpected client');
        },
      );
      final starting = state.start();
      state.dispose();
      permission.complete(true);
      await starting;
      expect(factoryCalled, isFalse);
    },
  );

  testWidgets('reduce motion applies pressed shapes without a spring delay', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(context).copyWith(disableAnimations: true),
          child: child!,
        ),
        home: Scaffold(
          body: ExpressiveButton(label: '测试按钮', onPressed: () {}),
        ),
      ),
    );
    final button = find.widgetWithText(FilledButton, '测试按钮');
    final gesture = await tester.startGesture(tester.getCenter(button));
    await tester.pump();
    var shape =
        tester.widget<FilledButton>(button).style!.shape!.resolve({})
            as RoundedRectangleBorder;
    expect(shape.borderRadius, BorderRadius.circular(12));
    await gesture.up();
    await tester.pump();
    shape =
        tester.widget<FilledButton>(button).style!.shape!.resolve({})
            as RoundedRectangleBorder;
    expect(shape.borderRadius, BorderRadius.circular(28));
    await tester.pumpAndSettle();
  });
  test(
    'supervisor rediscovers services before restoring fresh subscription ids',
    () async {
      final fixture = createUiFixture();
      await fixture.state.start();
      await Future<void>.delayed(Duration.zero);
      fixture.bridge.discover();
      await Future<void>.delayed(Duration.zero);
      await fixture.state.client!.connect(testDeviceId);
      await Future<void>.delayed(Duration.zero);
      const desired = CharacteristicId(
        service: ServiceId(uuid: testService, instance: 0),
        uuid: testCharacteristic,
        instance: 0,
      );
      await fixture.state.setSubscription(testDeviceId, desired, true);
      final session = SupervisorSession(
        client: fixture.state.client!,
        state: fixture.state,
        device: fixture.state.sortedDevices().single,
        restoreNotifications: true,
        onUpdate: () {},
      );
      addTearDown(() async {
        await session.stop();
        fixture.state.dispose();
      });
      await session.start();
      expect(session.running, isTrue);
      final auto = fixture.bridge.sent.lastWhere(
        (call) => call.method == 'auto_connect',
      );
      expect(jsonDecode(auto.params)['policy'], {
        'initial_backoff_ms': 500,
        'max_backoff_ms': 30000,
        'jitter_fraction': 0.2,
      });

      await fixture.state.client!.disconnect(testDeviceId);
      await Future<void>.delayed(Duration.zero);
      fixture.bridge.emitSupervisor({
        'reconnect_scheduled': {
          'id': testDeviceId.json,
          'delay': 500,
          'attempt': 1,
        },
      });
      await Future<void>.delayed(Duration.zero);
      expect(session.log.map((entry) => entry.text), contains('已断开'));
      expect(
        session.log.map((entry) => entry.text),
        contains('正在退避（第 1 次，500 ms）'),
      );

      fixture.bridge.emitEvent(
        jsonEncode({
          'type': 'adapter_event',
          'event': {'device_connected': testDeviceId.json},
        }),
      );
      fixture.bridge.emitSupervisor({'connected': testDeviceId.json});
      await Future<void>.delayed(Duration.zero);
      await Future<void>.delayed(Duration.zero);
      await Future<void>.delayed(Duration.zero);
      expect(fixture.state.subscriptions, contains((testDeviceId, desired)));
      final methods = fixture.bridge.sent.map((call) => call.method).toList();
      expect(
        methods.lastIndexOf('discover_services'),
        lessThan(methods.lastIndexOf('subscribe')),
      );
    },
  );
}

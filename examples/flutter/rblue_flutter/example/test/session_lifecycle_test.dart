import 'dart:async';

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
    'reconnect rediscovers services before restoring fresh subscription ids',
    () async {
      final fixture = createUiFixture();
      await fixture.state.start();
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
      var restored = 0;
      final first = Completer<void>();
      final second = Completer<void>();
      late final ReconnectController controller;
      controller = ReconnectController(
        client: fixture.state.client!,
        device: fixture.state.sortedDevices().single,
        onLog: () {},
        onSessionRestored: (services) async {
          fixture.state.invalidateSubscriptions(testDeviceId);
          final fresh = services.identifiedCharacteristics
              .firstWhere((entry) => entry.$1 == desired)
              .$1;
          await fixture.state.setSubscription(testDeviceId, fresh, true);
          restored++;
          if (restored == 1) first.complete();
          if (restored == 2) second.complete();
        },
      );
      final events = fixture.state.client!.events.listen((event) {
        if (event is AdapterDeviceConnected) controller.onConnected();
        if (event is AdapterDeviceDisconnected) controller.onDisconnected();
      });
      addTearDown(() async {
        controller.stop();
        await events.cancel();
        fixture.state.dispose();
      });
      await controller.start();
      await first.future.timeout(const Duration(seconds: 5));
      await fixture.state.client!.disconnect(testDeviceId);
      await second.future.timeout(const Duration(seconds: 5));
      expect(fixture.state.subscriptions, contains((testDeviceId, desired)));
      final methods = fixture.bridge.sent.map((call) => call.method).toList();
      expect(
        methods.lastIndexOf('discover_services'),
        lessThan(methods.lastIndexOf('subscribe')),
      );
    },
  );
}

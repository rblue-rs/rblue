import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rblue_flutter_example/main.dart';
import 'package:rblue_flutter_example/src/widgets/expressive_button.dart';
import 'package:rblue_flutter_example/src/payload.dart';

import 'ui_fixture.dart';

void main() {
  test('payload parsing preserves UTF-8 and rejects invalid hex', () {
    expect(parsePayload('你好', hex: false), utf8.encode('你好'));
    expect(parsePayload('48 65\n6c 6C 6F', hex: true), utf8.encode('Hello'));
    for (final value in ['F', '0x12', 'GG', '<script>']) {
      expect(() => parsePayload(value, hex: true), throwsFormatException);
    }
  });

  testWidgets(
    'mobile navigation and search survive inspector; GATT uses UTF-8 and instance ids',
    (tester) async {
      tester.view.physicalSize = const Size(390, 844);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final fixture = createUiFixture();
      addTearDown(fixture.state.dispose);
      await tester.pumpWidget(RblueExampleApp(state: fixture.state));
      await tester.pumpAndSettle();
      fixture.bridge.discover();
      await tester.pumpAndSettle();
      expect(find.byType(NavigationDestination), findsNWidgets(5));
      await tester.enterText(find.byKey(const Key('device-search')), 'Sensor');
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('Test Sensor'));
      await tester.tap(find.text('Test Sensor'));
      await tester.pumpAndSettle();
      expect(find.byType(NavigationBar), findsOneWidget);
      await tester.tap(find.text('连接设备'));
      await tester.pumpAndSettle();
      expect(fixture.state.connected, hasLength(1));
      // There are exactly two service-instance rows, not four duplicated rows.
      expect(find.byType(ListTile), findsNWidgets(2));
      await tester.scrollUntilVisible(
        find.byType(DropdownButtonFormField<bool>),
        180,
        scrollable: find.byType(Scrollable).first,
      );
      await tester.pumpAndSettle();
      // Open the encoding menu from the first dropdown, then choose text.
      await tester.tap(find.byType(DropdownButtonFormField<bool>));
      await tester.pumpAndSettle();
      await tester.tap(find.text('UTF-8 · 文本').last);
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.byKey(const Key('write-payload')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const Key('write-payload')), '你好');
      await tester.pumpAndSettle();
      expect(find.text('6 bytes'), findsOneWidget);
      await tester.ensureVisible(find.text('写入数据'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('写入数据'));
      await tester.pumpAndSettle();
      final write = fixture.bridge.sent.lastWhere(
        (call) => call.method == 'write',
      );
      expect(write.payload, utf8.encode('你好'));
      expect(
        jsonDecode(write.params)['characteristic']['service']['instance'],
        0,
      );
      await tester.ensureVisible(find.text('订阅通知'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('订阅通知'));
      await tester.pumpAndSettle();
      expect(fixture.state.subscriptions, hasLength(1));
      fixture.bridge.notify();
      await tester.pumpAndSettle();
      await tester.tap(find.text('数据流'));
      await tester.pumpAndSettle();
      expect(find.text('T=24'), findsOneWidget);
      await tester.tap(find.text('暂停显示'));
      fixture.bridge.notify([65]);
      await tester.pumpAndSettle();
      expect(fixture.state.notificationCount, 2);
      expect(find.text('A'), findsNothing);
      await tester.tap(find.text('继续显示'));
      await tester.pumpAndSettle();
      expect(find.text('A'), findsOneWidget);
      await tester.tap(find.text('设备'));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('返回设备列表'));
      await tester.pumpAndSettle();
      expect(
        tester
                .widget<TextField>(find.byKey(const Key('device-search')))
                .controller
                ?.text ??
            'Sensor',
        'Sensor',
      );
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets('all destinations fit small screens and large text', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.view.resetPhysicalSize);
    final fixture = createUiFixture();
    addTearDown(fixture.state.dispose);
    await tester.pumpWidget(RblueExampleApp(state: fixture.state));
    await tester.pumpAndSettle();
    fixture.bridge.discover();
    for (final size in [
      const Size(320, 740),
      const Size(390, 844),
      const Size(844, 390),
    ]) {
      tester.view.physicalSize = size;
      await tester.pumpAndSettle();
      for (final label in ['设备', '数据流', '重连', '环境', '日志']) {
        await tester.tap(find.text(label));
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull, reason: '$size / $label');
      }
    }
    tester.platformDispatcher.textScaleFactorTestValue = 2;
    addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
    tester.view.physicalSize = const Size(390, 844);
    await tester.pumpAndSettle();
    for (final label in ['设备', '数据流', '重连', '环境', '日志']) {
      await tester.tap(find.text(label));
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull, reason: '2x text / $label');
    }
    await tester.pumpWidget(const SizedBox.shrink());
  });

  testWidgets('button shape and neighbor width spring back on release', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Center(
            child: SizedBox(
              width: 320,
              child: SquishButtonGroup(
                actions: [
                  (label: '读取', onPressed: () {}, tonal: true),
                  (label: '写入', onPressed: () {}, tonal: false),
                  (label: '订阅', onPressed: () {}, tonal: true),
                ],
              ),
            ),
          ),
        ),
      ),
    );
    final before = tester.getSize(find.widgetWithText(FilledButton, '写入'));
    final neighbor = tester.getSize(find.widgetWithText(FilledButton, '读取'));
    final gesture = await tester.startGesture(
      tester.getCenter(find.text('写入')),
    );
    await tester.pump(const Duration(milliseconds: 80));
    await tester.pump(const Duration(milliseconds: 80));
    expect(
      tester.getSize(find.widgetWithText(FilledButton, '写入')).width,
      greaterThan(before.width),
    );
    expect(
      tester.getSize(find.widgetWithText(FilledButton, '读取')).width,
      lessThan(neighbor.width),
    );
    await gesture.up();
    await tester.pumpAndSettle();
    expect(
      tester.getSize(find.widgetWithText(FilledButton, '写入')).width,
      closeTo(before.width, 1),
    );
  });

  test('packet buffers and session export stay bounded', () async {
    final fixture = createUiFixture();
    await fixture.state.start();
    for (var index = 0; index < 210; index++) {
      fixture.bridge.notify();
    }
    await Future<void>.delayed(Duration.zero);
    for (var index = 0; index < 320; index++) {
      fixture.state.record('event', '$index');
    }
    expect(fixture.state.packets, hasLength(200));
    expect(fixture.state.logs, hasLength(300));
    expect(fixture.state.notificationCount, 210);
    final session = jsonDecode(fixture.state.sessionJson());
    expect(session['packets'], hasLength(200));
    expect(session['packets'][0]['characteristic']['service']['instance'], 0);
    fixture.state.dispose();
  });
}

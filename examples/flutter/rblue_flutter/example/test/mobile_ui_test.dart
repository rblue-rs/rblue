import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rblue_flutter/rblue_flutter.dart';
import 'package:rblue_flutter_example/main.dart';
import 'package:rblue_flutter_example/src/widgets/expressive_button.dart';
import 'package:rblue_flutter_example/src/payload.dart';

import 'ui_fixture.dart';

Finder destination(String label) => find.descendant(
  of: find.byKey(const Key('bottom-toolbar')),
  matching: find.text(label),
);

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
      expect(find.byKey(const Key('bottom-toolbar')), findsOneWidget);
      await tester.enterText(
        find.byKey(const PageStorageKey('device-search')),
        'Sensor',
      );
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('Test Sensor'));
      await tester.tap(find.text('Test Sensor'));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('bottom-toolbar')), findsOneWidget);
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
      await tester.tap(destination('数据流'));
      await tester.pumpAndSettle();
      expect(find.text('T=24'), findsOneWidget);
      await tester.tap(find.byTooltip('暂停显示'));
      fixture.bridge.notify([65]);
      await tester.pumpAndSettle();
      expect(fixture.state.notificationCount, 2);
      expect(find.text('A'), findsNothing);
      await tester.tap(find.byTooltip('继续显示'));
      await tester.pumpAndSettle();
      expect(find.text('A'), findsOneWidget);
      await tester.tap(find.text('设备'));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('返回设备列表'));
      await tester.pumpAndSettle();
      expect(
        tester
                .widget<TextField>(
                  find.byKey(const PageStorageKey('device-search')),
                )
                .controller
                ?.text ??
            'Sensor',
        'Sensor',
      );
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets(
    'environment shows notes for the current platform',
    (tester) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(390, 900);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.view.resetPhysicalSize);
      final fixture = createUiFixture();
      addTearDown(fixture.state.dispose);
      await tester.pumpWidget(RblueExampleApp(state: fixture.state));
      await tester.pumpAndSettle();
      await tester.tap(destination('环境'));
      await tester.pumpAndSettle();

      final ios =
          Theme.of(tester.element(find.text('平台说明'))).platform ==
          TargetPlatform.iOS;
      expect(find.text(ios ? 'iOS 蓝牙连接' : 'Android 蓝牙权限'), findsOneWidget);
      expect(find.text(ios ? 'Android 蓝牙权限' : 'iOS 蓝牙连接'), findsNothing);
      await tester.scrollUntilVisible(
        find.text('写入行为'),
        200,
        scrollable: find.byType(Scrollable).first,
      );
      await tester.pumpAndSettle();
      expect(find.text('写入行为'), findsOneWidget);
      expect(find.text('能力与环境'), findsNothing);

      final platformCard = find.ancestor(
        of: find.text(ios ? 'iOS 写入长度' : 'Android GATT 133'),
        matching: find.byType(Card),
      );
      final writeCard = find.ancestor(
        of: find.text('写入行为'),
        matching: find.byType(Card),
      );
      expect(
        tester.getTopLeft(writeCard).dy - tester.getBottomLeft(platformCard).dy,
        greaterThanOrEqualTo(8),
      );
    },
    variant: const TargetPlatformVariant({
      TargetPlatform.iOS,
      TargetPlatform.android,
    }),
  );

  testWidgets(
    'finds system devices without typing a UUID',
    (tester) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(390, 900);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.view.resetPhysicalSize);
      final fixture = createUiFixture();
      addTearDown(fixture.state.dispose);
      await tester.pumpWidget(RblueExampleApp(state: fixture.state));
      await tester.pumpAndSettle();
      await tester.tap(find.text('查找系统已连接'));
      await tester.pumpAndSettle();

      final request = fixture.bridge.sent.singleWhere(
        (call) => call.method == 'system_connected_peripherals',
      );
      expect(jsonDecode(request.params)['services'], hasLength(3));
      expect(find.text('System Sensor'), findsOneWidget);
      expect(find.text('系统已连接'), findsOneWidget);
      expect(find.text('找到 1 台。请选择设备并在本 App 中连接。'), findsOneWidget);
    },
    variant: const TargetPlatformVariant({
      TargetPlatform.iOS,
      TargetPlatform.android,
    }),
  );

  testWidgets(
    'small scan screen prioritizes results and retains a UUID draft',
    (tester) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(320, 640);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.view.resetPhysicalSize);
      final fixture = createUiFixture();
      addTearDown(fixture.state.dispose);
      await tester.pumpWidget(RblueExampleApp(state: fixture.state));
      await tester.pumpAndSettle();
      for (var index = 0; index < 20; index++) {
        fixture.bridge.discover(
          DeviceId.uuid(
            'C0EA7291-4C8A-40F2-A8F4-${index.toString().padLeft(12, '0')}',
          ),
        );
      }
      await tester.pumpAndSettle();
      expect(find.text('Test Sensor').first.hitTestable(), findsOneWidget);
      expect(find.text('查找系统已连接'), findsOneWidget);
      expect(find.text('全部设备'), findsNothing);
      await tester.tap(find.byTooltip('设备选项'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('按自定义服务查找'));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.widgetWithText(TextFormField, 'GATT 服务 UUID'),
        '180F',
      );
      Navigator.of(tester.element(find.text('设备选项'))).pop();
      await tester.pumpAndSettle();
      final scan = find.byKey(const PageStorageKey('scan'));
      final list = find
          .descendant(of: scan, matching: find.byType(Scrollable))
          .first;
      final position = tester.state<ScrollableState>(list).position;
      position.jumpTo(position.maxScrollExtent);
      await tester.pumpAndSettle();
      expect(find.byTooltip('设备选项'), findsNothing);
      position.jumpTo(0);
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('设备选项'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('按自定义服务查找'));
      await tester.pumpAndSettle();
      expect(find.text('180F'), findsOneWidget);
      await tester.tap(find.text('查找设备'));
      await tester.pumpAndSettle();
      final request = fixture.bridge.sent.singleWhere(
        (call) => call.method == 'system_connected_peripherals',
      );
      expect(jsonDecode(request.params)['services'], [
        '0000180f-0000-1000-8000-00805f9b34fb',
      ]);
      expect(tester.takeException(), isNull);
    },
    variant: const TargetPlatformVariant({
      TargetPlatform.iOS,
      TargetPlatform.android,
    }),
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
        await tester.tap(destination(label));
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull, reason: '$size / $label');
      }
    }
    tester.platformDispatcher.textScaleFactorTestValue = 2;
    addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
    tester.view.physicalSize = const Size(390, 844);
    await tester.pumpAndSettle();
    for (final label in ['设备', '数据流', '重连', '环境', '日志']) {
      await tester.tap(destination(label));
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

  testWidgets('unsupported system lookup disables further requests', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(390, 900);
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.view.resetPhysicalSize);
    final fixture = createUiFixture();
    fixture.bridge.unsupportedMethods.add('system_connected_peripherals');
    addTearDown(fixture.state.dispose);
    await tester.pumpWidget(RblueExampleApp(state: fixture.state));
    await tester.pumpAndSettle();
    await tester.tap(find.text('查找系统已连接'));
    await tester.pumpAndSettle();
    expect(find.text('当前平台不支持查找系统已连接设备。'), findsWidgets);
    final before = fixture.bridge.sent
        .where((call) => call.method == 'system_connected_peripherals')
        .length;
    await tester.tap(find.text('查找系统已连接'));
    await tester.pumpAndSettle();
    expect(
      fixture.bridge.sent
          .where((call) => call.method == 'system_connected_peripherals')
          .length,
      before,
    );
  });

  testWidgets('device tools use new APIs after a local connection', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(390, 900);
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.view.resetPhysicalSize);
    final fixture = createUiFixture();
    addTearDown(fixture.state.dispose);
    await tester.pumpWidget(RblueExampleApp(state: fixture.state));
    await tester.pumpAndSettle();
    fixture.bridge.discover();
    await tester.pumpAndSettle();
    await tester.tap(find.text('Test Sensor'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('连接选项'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('系统后台连接'));
    await tester.pumpAndSettle();
    await tester.enterText(find.widgetWithText(TextField, '连接后请求 MTU'), '185');
    await tester.pumpAndSettle();
    await tester.tap(find.text('连接设备'));
    await tester.pumpAndSettle();
    final connect = fixture.bridge.sent.lastWhere(
      (call) => call.method == 'connect',
    );
    expect(jsonDecode(connect.params)['auto_connect'], isTrue);
    expect(jsonDecode(connect.params)['mtu'], 185);

    await tester.tap(find.text('连接信息'));
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.text('请求 MTU').last);
    await tester.enterText(find.widgetWithText(TextField, '请求 MTU'), '185');
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, '请求 MTU'));
    await tester.pumpAndSettle();
    expect(find.text('已协商为 185 bytes'), findsOneWidget);
    await tester.ensureVisible(find.text('配对'));
    await tester.tap(find.text('配对'));
    await tester.pumpAndSettle();
    expect(find.text('已配对。'), findsOneWidget);
    await tester.tap(find.text('高吞吐'));
    await tester.pumpAndSettle();
    expect(find.textContaining('已请求改为高吞吐'), findsOneWidget);
    expect(
      fixture.bridge.sent.where((call) => call.method == 'request_mtu'),
      isNotEmpty,
    );
    expect(
      fixture.bridge.sent.where((call) => call.method == 'pair'),
      isNotEmpty,
    );
    expect(
      fixture.bridge.sent.where(
        (call) => call.method == 'set_connection_priority',
      ),
      isNotEmpty,
    );
  });

  testWidgets('long write switch is forwarded on oversize payloads', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(390, 900);
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.view.resetPhysicalSize);
    final fixture = createUiFixture();
    addTearDown(fixture.state.dispose);
    await tester.pumpWidget(RblueExampleApp(state: fixture.state));
    await tester.pumpAndSettle();
    fixture.bridge.discover();
    await tester.pumpAndSettle();
    await tester.tap(find.text('Test Sensor'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('连接设备'));
    await tester.pumpAndSettle();
    await tester.scrollUntilVisible(
      find.text('允许长写'),
      200,
      scrollable: find.byType(Scrollable).first,
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('允许长写'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('超长写入（已允许长写）'));
    await tester.pumpAndSettle();
    final write = fixture.bridge.sent.lastWhere(
      (call) => call.method == 'write',
    );
    expect(jsonDecode(write.params)['allow_long_write'], isTrue);
  });

  testWidgets(
    'environment can power the radio and lists readable capabilities',
    (tester) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(390, 900);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.view.resetPhysicalSize);
      final fixture = createUiFixture();
      addTearDown(fixture.state.dispose);
      await tester.pumpWidget(RblueExampleApp(state: fixture.state));
      await tester.pumpAndSettle();
      await tester.tap(destination('环境'));
      await tester.pumpAndSettle();
      expect(find.text('后端版本'), findsOneWidget);
      expect(find.text('后端能力'), findsOneWidget);
      expect(find.textContaining('应用内配对：已支持'), findsOneWidget);
      expect(find.text('系统后台连接与应用监督器'), findsOneWidget);
      await tester.ensureVisible(find.text('关闭蓝牙'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('关闭蓝牙'));
      await tester.pumpAndSettle();
      expect(
        fixture.bridge.sent.where((call) => call.method == 'set_powered'),
        isNotEmpty,
      );
    },
  );

  testWidgets('reconnect page starts the application supervisor', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(390, 900);
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.view.resetPhysicalSize);
    final fixture = createUiFixture();
    addTearDown(fixture.state.dispose);
    await tester.pumpWidget(RblueExampleApp(state: fixture.state));
    await tester.pumpAndSettle();
    fixture.bridge.discover();
    await tester.pumpAndSettle();
    await tester.tap(destination('重连'));
    await tester.pumpAndSettle();
    expect(find.text('这里启动的是应用监督器，断线后按退避策略重试。这不是设备页里的系统后台连接。'), findsOneWidget);
    expect(find.text('连接并监听'), findsNothing);
    await tester.tap(find.text('选择设备'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Test Sensor').last);
    await tester.pumpAndSettle();
    await tester.tap(find.text('开始监督'));
    await tester.pumpAndSettle();
    final request = fixture.bridge.sent.lastWhere(
      (call) => call.method == 'auto_connect',
    );
    final params = jsonDecode(request.params) as Map<String, Object?>;
    expect(params['policy'], {
      'initial_backoff_ms': 500,
      'max_backoff_ms': 30000,
      'jitter_fraction': 0.2,
    });
    expect(find.text('监督中'), findsWidgets);
    await tester.tap(find.text('停止监督'));
    await tester.pumpAndSettle();
    expect(
      fixture.bridge.sent.where((call) => call.method == 'unwatch'),
      isNotEmpty,
    );
    expect(find.text('已停止'), findsWidgets);
  });
}

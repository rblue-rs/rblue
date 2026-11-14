import 'dart:io' show Platform;

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rblue_flutter/src/plugin_channel.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('non-Android setup bypasses the method channel', () async {
    if (Platform.isAndroid) return;

    final calls = <MethodCall>[];
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(RbluePluginChannel.methodChannel, (
      call,
    ) async {
      calls.add(call);
      return false;
    });
    addTearDown(
      () => messenger.setMockMethodCallHandler(
        RbluePluginChannel.methodChannel,
        null,
      ),
    );

    await RbluePluginChannel.instance.initialize();
    expect(await RbluePluginChannel.instance.requestBlePermissions(), isTrue);
    expect(calls, isEmpty);
  });
}

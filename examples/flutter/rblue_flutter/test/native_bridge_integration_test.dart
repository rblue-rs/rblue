import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:rblue_flutter/src/native_bridge.dart';

void main() {
  final libraryPath = Platform.environment['RBLUE_C_TEST_LIBRARY'];

  test(
    'dispose drains every accepted native response before closing listeners',
    () async {
      final bridge = RealNativeBridge.fromDynamicLibrary(
        ffi.DynamicLibrary.open(libraryPath!),
      );
      final handle = bridge.create();
      expect(handle, isNotNull);

      var deliveredToClient = 0;
      bridge.registerCallbacks(
        handle!,
        onResponse: (_, _, _) => deliveredToClient++,
        onEvent: (_) {},
      );
      for (var requestId = 1; requestId <= 200; requestId++) {
        expect(
          bridge.send(
            handle,
            requestId,
            'backend_version',
            const {},
            Uint8List(0),
          ),
          0,
        );
      }

      bridge.dispose(handle);
      await bridge.drained.timeout(const Duration(seconds: 10));

      final processed = bridge.processedCallbackCounts;
      final target = bridge.targetCallbackCounts;
      expect(target, isNotNull);
      expect(processed.responses, target!.responses);
      expect(processed.events, target.events);
      expect(target.responses, greaterThan(0));
      expect(deliveredToClient, lessThanOrEqualTo(target.responses));
    },
    skip: libraryPath == null
        ? 'Set RBLUE_C_TEST_LIBRARY to a mock librblue_c build.'
        : false,
  );
}

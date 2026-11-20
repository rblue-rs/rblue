import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:rblue_flutter/src/native_bridge.dart';

const _drainTimeout = Duration(seconds: 30);

void main() {
  final libraryPath = Platform.environment['RBLUE_C_TEST_LIBRARY'];
  final skipReason = libraryPath == null
      ? 'Set RBLUE_C_TEST_LIBRARY to a mock librblue_c build.'
      : false;

  test(
    'dispose drains every accepted native response before closing listeners',
    () async {
      await _runAcceptedRequestDrain(
        libraryPath!,
        requestCount: 200,
        waitForAcceptanceWindow: true,
      );
    },
    skip: skipReason,
  );

  test(
    'high-concurrency soak completes each req_id at most once and drains fully',
    () async {
      final result = await _runAcceptedRequestDrain(
        libraryPath!,
        requestCount: 1000,
        waitForAcceptanceWindow: true,
      );
      expect(result.deliveredToClient, result.targets.responses);
      expect(result.completedIds, hasLength(result.targets.responses));
      expect(result.completedIds.toSet(), hasLength(result.completedIds.length));
      expect(
        result.completedIds.toSet(),
        everyElement(predicate<int>((id) => id >= 1 && id <= 1000)),
      );
    },
    skip: skipReason,
  );

  test(
    'dispose mid-flight drains only callbacks native shutdown reports',
    () async {
      final result = await _runAcceptedRequestDrain(
        libraryPath!,
        requestCount: 1000,
        waitForAcceptanceWindow: false,
      );
      expect(result.deliveredToClient, lessThanOrEqualTo(result.targets.responses));
      expect(result.completedIds.toSet(), hasLength(result.completedIds.length));
    },
    skip: skipReason,
  );
}

class _DrainResult {
  const _DrainResult({
    required this.deliveredToClient,
    required this.processed,
    required this.targets,
    required this.completedIds,
  });

  final int deliveredToClient;
  final ({int responses, int events}) processed;
  final ({int responses, int events}) targets;
  final List<int> completedIds;
}

/// Sends [requestCount] `backend_version` requests and disposes the handle.
///
/// When [waitForAcceptanceWindow] is true, the test yields once so the mock
/// runtime can accept work before shutdown; when false, dispose races the
/// in-flight requests, which is the mid-flight soak.
Future<_DrainResult> _runAcceptedRequestDrain(
  String libraryPath, {
  required int requestCount,
  required bool waitForAcceptanceWindow,
}) async {
  final bridge = RealNativeBridge.fromDynamicLibrary(
    ffi.DynamicLibrary.open(libraryPath),
  );
  final handle = bridge.create();
  expect(handle, isNotNull);

  var deliveredToClient = 0;
  final completedIds = <int>[];
  bridge.registerCallbacks(
    handle!,
    onResponse: (reqId, _, _) {
      deliveredToClient++;
      completedIds.add(reqId);
    },
    onEvent: (_) {},
  );
  for (var requestId = 1; requestId <= requestCount; requestId++) {
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
  if (waitForAcceptanceWindow) {
    // Let the facade runtime start the accepted requests so the high-count
    // drain is not accidentally just an immediate cancel.
    await Future<void>.delayed(Duration.zero);
  }

  bridge.dispose(handle);
  await bridge.drained.timeout(_drainTimeout);

  final processed = bridge.processedCallbackCounts;
  final target = bridge.targetCallbackCounts;
  expect(target, isNotNull);
  expect(processed.responses, target!.responses);
  expect(processed.events, target.events);
  expect(target.responses, greaterThan(0));
  expect(deliveredToClient, lessThanOrEqualTo(target.responses));
  return _DrainResult(
    deliveredToClient: deliveredToClient,
    processed: processed,
    targets: target,
    completedIds: completedIds,
  );
}

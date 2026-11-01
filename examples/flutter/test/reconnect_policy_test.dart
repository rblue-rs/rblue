import 'package:flutter_test/flutter_test.dart';

import 'package:rblue_flutter_example/src/reconnect_policy.dart';

void main() {
  const policy = ReconnectPolicy();

  test('backoff starts at 500ms and doubles without jitter', () {
    expect(
      policy.delayForAttempt(0, randomUnit: 0.5),
      const Duration(milliseconds: 500),
    );
    expect(
      policy.delayForAttempt(1, randomUnit: 0.5),
      const Duration(seconds: 1),
    );
    expect(
      policy.delayForAttempt(2, randomUnit: 0.5),
      const Duration(seconds: 2),
    );
    expect(
      policy.delayForAttempt(5, randomUnit: 0.5),
      const Duration(seconds: 16),
    );
    expect(
      policy.delayForAttempt(6, randomUnit: 0.5),
      const Duration(seconds: 30),
    );
    expect(
      policy.delayForAttempt(20, randomUnit: 0.5),
      const Duration(seconds: 30),
    );
  });

  test('jitter stays within plus or minus twenty percent and respects cap', () {
    expect(
      policy.delayForAttempt(0, randomUnit: 0),
      const Duration(milliseconds: 400),
    );
    expect(
      policy.delayForAttempt(0, randomUnit: 1),
      const Duration(milliseconds: 600),
    );
    expect(
      policy.delayForAttempt(6, randomUnit: 0),
      const Duration(seconds: 24),
    );
    expect(
      policy.delayForAttempt(6, randomUnit: 1),
      const Duration(seconds: 30),
    );
  });
}

import 'dart:math' as math;

/// Host-side reconnect backoff shared by the demo and its tests.
///
/// The base sequence starts at 500 ms, doubles, and caps at 30 seconds. Each
/// retry applies symmetric jitter (default ±20%) so multiple clients do not
/// reconnect in lockstep. Pass a deterministic [randomUnit] in tests.
class ReconnectPolicy {
  /// Creates the policy.
  const ReconnectPolicy({
    this.initial = const Duration(milliseconds: 500),
    this.maximum = const Duration(seconds: 30),
    this.jitterFraction = 0.20,
  }) : assert(jitterFraction >= 0 && jitterFraction <= 1);

  /// Initial base delay.
  final Duration initial;

  /// Maximum jittered delay.
  final Duration maximum;

  /// Fraction applied symmetrically around the base delay.
  final double jitterFraction;

  /// Computes one retry delay.
  ///
  /// [randomUnit] must be in `[0, 1]`; `0.5` yields no jitter.
  Duration delayForAttempt(int attempt, {double? randomUnit}) {
    final exponent = attempt.clamp(0, 62);
    final factor = math.pow(2, exponent).toDouble();
    final baseMs = math.min(
      maximum.inMilliseconds.toDouble(),
      initial.inMilliseconds * factor,
    );
    final unit = (randomUnit ?? math.Random().nextDouble()).clamp(0.0, 1.0);
    final jitter = (unit * 2 - 1) * jitterFraction;
    final jittered = (baseMs * (1 + jitter)).round();
    return Duration(milliseconds: jittered.clamp(0, maximum.inMilliseconds));
  }
}

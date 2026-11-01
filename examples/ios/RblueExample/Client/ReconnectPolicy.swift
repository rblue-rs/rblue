import Foundation

/// Swift-side automatic-reconnect backoff, mirroring the Android example:
/// 500 ms base doubling to a 30 s cap, with +-20% jitter so fleets of
/// clients do not reconnect in lockstep. Attempts are unlimited until the
/// connection is re-established or the client cancels (user disconnect).
struct ReconnectPolicy {
    let baseMilliseconds: Int
    let capMilliseconds: Int
    let jitterFraction: Double
    private(set) var attempt: Int
    private let randomSource: () -> Double

    /// # Parameters
    ///
    /// - `randomSource` - produces values in `0...1`; injectable for
    ///   deterministic tests.
    init(
        baseMilliseconds: Int = 500,
        capMilliseconds: Int = 30_000,
        jitterFraction: Double = 0.2,
        randomSource: @escaping () -> Double = { Double.random(in: 0 ... 1) }
    ) {
        self.baseMilliseconds = baseMilliseconds
        self.capMilliseconds = capMilliseconds
        self.jitterFraction = jitterFraction
        self.attempt = 0
        self.randomSource = randomSource
    }

    /// The next backoff delay in milliseconds and advances the attempt
    /// counter.
    mutating func nextDelayMilliseconds() -> Int {
        attempt += 1
        let exponent = Double(attempt - 1)
        let exponential = Double(baseMilliseconds) * pow(2, exponent)
        let capped = min(Double(capMilliseconds), exponential)
        let jitter = 1 + jitterFraction * (2 * randomSource() - 1)
        return Int((capped * jitter).rounded())
    }

    /// Returns the attempt counter to zero (used after a successful
    /// reconnect when a policy object is reused).
    mutating func reset() {
        attempt = 0
    }
}

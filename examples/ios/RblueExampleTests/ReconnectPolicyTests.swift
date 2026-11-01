import XCTest

@testable import RblueExample

/// Backoff sequence of the reconnect policy (500 ms doubling to a 30 s cap,
/// +-20% jitter) with an injectable random source for determinism.
final class ReconnectPolicyTests: XCTestCase {
    func testDoublingSequenceWithNeutralJitter() {
        var policy = ReconnectPolicy(
            baseMilliseconds: 500,
            capMilliseconds: 30_000,
            jitterFraction: 0.2,
            randomSource: { 0.5 }
        )
        var delays: [Int] = []
        for _ in 0 ..< 8 {
            delays.append(policy.nextDelayMilliseconds())
        }
        XCTAssertEqual(delays, [500, 1000, 2000, 4000, 8000, 16_000, 30_000, 30_000])
    }

    func testJitterBounds() {
        var low = ReconnectPolicy(
            baseMilliseconds: 1000, capMilliseconds: 30_000, jitterFraction: 0.2,
            randomSource: { 0 })
        var high = ReconnectPolicy(
            baseMilliseconds: 1000, capMilliseconds: 30_000, jitterFraction: 0.2,
            randomSource: { 1 })
        XCTAssertEqual(low.nextDelayMilliseconds(), 800)
        XCTAssertEqual(high.nextDelayMilliseconds(), 1200)
    }

    func testAttemptCounterAndReset() {
        var policy = ReconnectPolicy(
            baseMilliseconds: 100, capMilliseconds: 30_000, jitterFraction: 0,
            randomSource: { 0.5 }
        )
        _ = policy.nextDelayMilliseconds()
        _ = policy.nextDelayMilliseconds()
        XCTAssertEqual(policy.attempt, 2)
        policy.reset()
        XCTAssertEqual(policy.attempt, 0)
        XCTAssertEqual(policy.nextDelayMilliseconds(), 100)
    }
}

package com.rblue.example.client

import org.junit.Assert.assertEquals
import org.junit.Test

/** Backoff sequence of the reconnect policy (500 ms doubling to a 30 s cap,
 * +-20% jitter) with an injectable random source for determinism. */
class ReconnectPolicyTest {
    @Test
    fun doublingSequenceWithNeutralJitter() {
        val policy = ReconnectPolicy(
            baseMillis = 500,
            capMillis = 30_000,
            jitterFraction = 0.2,
            randomSource = { 0.5 },
        )
        val delays = LongArray(8) { policy.nextDelayMillis() }.toList()
        assertEquals(
            listOf(500L, 1000L, 2000L, 4000L, 8000L, 16_000L, 30_000L, 30_000L),
            delays,
        )
    }

    @Test
    fun jitterBounds() {
        val low = ReconnectPolicy(
            baseMillis = 1000,
            capMillis = 30_000,
            jitterFraction = 0.2,
            randomSource = { 0.0 },
        )
        val high = ReconnectPolicy(
            baseMillis = 1000,
            capMillis = 30_000,
            jitterFraction = 0.2,
            randomSource = { 1.0 },
        )
        assertEquals(800L, low.nextDelayMillis())
        assertEquals(1200L, high.nextDelayMillis())
    }

    @Test
    fun attemptCounterAndReset() {
        val policy = ReconnectPolicy(
            baseMillis = 100,
            capMillis = 30_000,
            jitterFraction = 0.0,
            randomSource = { 0.5 },
        )
        policy.nextDelayMillis()
        policy.nextDelayMillis()
        assertEquals(2, policy.attempt)
        policy.reset()
        assertEquals(0, policy.attempt)
        assertEquals(100L, policy.nextDelayMillis())
    }

    @Test
    fun defaultsMatchPlan() {
        // 500 ms base, 30 s cap, +-20% jitter (M-A4).
        assertEquals(500L, ReconnectPolicy.DEFAULT_BASE_MILLIS)
        assertEquals(30_000L, ReconnectPolicy.DEFAULT_CAP_MILLIS)
        assertEquals(0.2, ReconnectPolicy.JITTER_FRACTION, 0.0)
    }
}

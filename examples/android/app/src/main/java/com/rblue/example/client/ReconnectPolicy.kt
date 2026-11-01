package com.rblue.example.client

import kotlin.math.min
import kotlin.math.pow
import kotlin.math.roundToLong
import kotlin.random.Random

/**
 * Kotlin-side automatic-reconnect backoff: 500 ms base doubling to a 30 s
 * cap, with +-20% jitter so fleets of clients do not reconnect in lockstep.
 * Attempts are unlimited until the connection is re-established or the
 * client cancels (user disconnect). Same policy as the iOS example.
 */
class ReconnectPolicy(
    private val baseMillis: Long = DEFAULT_BASE_MILLIS,
    private val capMillis: Long = DEFAULT_CAP_MILLIS,
    private val jitterFraction: Double = JITTER_FRACTION,
    private val randomSource: () -> Double = { Random.nextDouble() },
) {
    /** Number of delays produced so far (1-based attempt counter). */
    var attempt: Int = 0
        private set

    /** The next backoff delay in milliseconds and advances the counter. */
    fun nextDelayMillis(): Long {
        attempt += 1
        val exponential = baseMillis.toDouble() * 2.0.pow(attempt - 1)
        val capped = min(capMillis.toDouble(), exponential)
        val jitter = 1.0 + jitterFraction * (2.0 * randomSource() - 1.0)
        return (capped * jitter).roundToLong()
    }

    /** Returns the attempt counter to zero (after a successful reconnect
     * when a policy object is reused). */
    fun reset() {
        attempt = 0
    }

    companion object {
        const val DEFAULT_BASE_MILLIS = 500L
        const val DEFAULT_CAP_MILLIS = 30_000L
        const val JITTER_FRACTION = 0.2
    }
}

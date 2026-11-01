package com.rblue

import android.content.Context

/**
 * Mock-backend variant of the process entry point.
 *
 * The mock `.so` is the same `librblue_ffi` facade built against the mock
 * backend: it contains no Android JNI code and no
 * `Java_com_rblue_RblueNative_nativeInit` symbol, so initialization must be a
 * no-op. `Rblue()` then selects the mock adapter and the app runs without any
 * Bluetooth hardware (emulators, CI).
 */
object RblueAndroid {
    /**
     * No-op for the mock backend. The `real` flavor's implementation performs
     * the JNI init; see its source for the ClassLoader discipline that any
     * future real-backend code must keep following.
     */
    @Suppress("UNUSED_PARAMETER")
    fun initialize(context: Context) {
        // Intentionally empty: the mock backend needs no JNI registration.
    }
}

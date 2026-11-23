package com.rblue

import android.content.Context

/**
 * JNI bridge to the rblue Android backend initialization, exported by
 * `librblue_ffi.so`.
 *
 * The package and method name are load-bearing: the native symbol is
 * `Java_com_rblue_RblueNative_nativeInit` (see
 * `crates/rblue-android/src/jni_shim.rs`), so this class must stay in
 * `com.rblue` with an external declaration named `nativeInit`.
 */
object RblueNative {
    init {
        System.loadLibrary("rblue_ffi")
    }

    /**
     * Runs the fork's JNI registration on the calling thread. Returns `true`
     * when the helper classes were initialized (or already were); `false`
     * with an error record on the Rust side otherwise.
     */
    external fun nativeInit(): Boolean

    /**
     * Forwards a `BluetoothAdapter` state code (or the shell sentinels
     * `-2` / `-3`) into the Rust radio watcher.
     */
    external fun nativeOnAdapterStateChanged(state: Int)
}

/**
 * Process-level entry point for rblue on Android.
 *
 * Call [initialize] once before constructing `uniffi.rblue_ffi.Rblue()`. The
 * fork resolves its `com.nonpolynomial.btleplug.android.impl` helper classes
 * and registers JNI native methods through the *calling thread's*
 * `ClassLoader`, so this must be invoked from an application thread (the main
 * thread is the safe choice) - never from a `static` initializer, where the
 * Android ClassLoader is not ready (see
 * `crates/rblue-android/docs/platform-notes.md`).
 *
 * The call is idempotent: repeated invocations are a no-op.
 */
object RblueAndroid {
    private const val FORK_HELPER_CLASS = "com.nonpolynomial.btleplug.android.impl.Adapter"

    @Volatile
    private var initialized = false

    /**
     * Initializes the Rust Android backend for this process.
     *
     * @param context any application context; only its `ClassLoader` is used
     *   for the fail-fast helper-class check
     * @throws IllegalStateException when the fork's helper classes are not
     *   resolvable (typically an AAR whose Java payload was stripped) or the
     *   native initialization reports failure
     */
    @Synchronized
    fun initialize(context: Context) {
        if (initialized) {
            return
        }
        // Fail fast with a readable message when the AAR payload was stripped
        // or the calling thread cannot see the application ClassLoader; the
        // native side would otherwise report a bare `false`.
        val loader = Thread.currentThread().contextClassLoader
            ?: context.applicationContext.classLoader
        try {
            Class.forName(FORK_HELPER_CLASS, false, loader)
        } catch (error: ClassNotFoundException) {
            throw IllegalStateException(
                "rblue: btleplug helper class $FORK_HELPER_CLASS is not visible from the " +
                    "calling thread's ClassLoader; initialize() must run on an application " +
                    "thread and the AAR must carry btleplug-java",
                error,
            )
        }
        if (!RblueNative.nativeInit()) {
            throw IllegalStateException(
                "rblue: native Android backend initialization failed; see the rblue " +
                    "error log for details",
            )
        }
        AdapterStateMonitor.start(context.applicationContext)
        initialized = true
    }
}

package com.rblue

/**
 * JNI entry into `librblue_c.so`. The package and name MUST match the
 * `Java_com_rblue_RblueNative_nativeInit` symbol the rblue-android crate
 * exports (see crates/rblue-android/src/jni_shim.rs); the shared object is
 * loaded eagerly because the `external` declaration resolves against it.
 */
object RblueNative {
    init {
        System.loadLibrary("rblue_c")
    }

    external fun nativeInit(): Boolean
}

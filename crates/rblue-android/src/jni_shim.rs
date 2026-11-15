//! JNI entry point for the AAR-side initialization discipline.
//!
//! The fork requires `platform::init` to run on a Java thread that can see
//! the application `ClassLoader`, before any adapter call. Calling that
//! through a UniFFI method would execute on a Rust runtime thread with the
//! wrong class-loader context, so the AAR (see `bindings/kotlin`) bridges
//! through this exported symbol instead:
//!
//! ```kotlin
//! // package com.rblue — must match the mangled symbol below
//! object RblueNative {
//!     init { System.loadLibrary("rblue_c") }
//!     external fun nativeInit(): Boolean
//! }
//! ```
//!
//! The `#[no_mangle]` JNI symbol itself is defined by consuming cdylibs
//! (`rblue-ffi`, `rblue-c`) so rustc puts it in the shared-library export
//! list. This module holds the implementation those wrappers call. The
//! backend's `init` is idempotent, so repeated calls are safe.

use std::ffi::c_void;

use jni::{
    errors::LogErrorAndDefault,
    sys::{jboolean, JNIEnv},
    EnvUnowned,
};

/// Implementation of `com.rblue.RblueNative.nativeInit()`. Returns `true`
/// when the fork's Java helper classes were initialized (or already were);
/// `false` with a `tracing::error!` record otherwise.
pub(crate) fn native_init<'caller>(mut unowned_env: EnvUnowned<'caller>) -> jboolean {
    let mut ok = false;
    unowned_env
        .with_env(|env| -> Result<(), jni::errors::Error> {
            ok = match crate::init(env) {
                Ok(()) => true,
                Err(err) => {
                    tracing::error!("rblue android init failed: {err}");
                    false
                }
            };
            Ok(())
        })
        .resolve::<LogErrorAndDefault>();
    ok
}

/// FFI trampoline used by consuming cdylibs.
///
/// # Safety
///
/// `env` must be a valid `JNIEnv*` for the current thread's JNI attachment.
/// `class` is the JNI class argument and is unused.
pub(crate) unsafe fn native_init_from_raw(env: *mut c_void, _class: *mut c_void) -> jboolean {
    let unowned_env = unsafe { EnvUnowned::from_raw(env as *mut JNIEnv) };
    native_init(unowned_env)
}

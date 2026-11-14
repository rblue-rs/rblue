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
//! The backend's `init` is idempotent, so repeated calls are safe.

use jni::{errors::LogErrorAndDefault, objects::JClass, sys::jboolean, EnvUnowned};

/// JNI entry: `com.rblue.RblueNative.nativeInit()`. Returns `true` when the
/// fork's Java helper classes were initialized (or already were); `false`
/// with a `tracing::error!` record otherwise.
#[no_mangle]
pub extern "system" fn Java_com_rblue_RblueNative_nativeInit<'caller>(
    mut unowned_env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
) -> jboolean {
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

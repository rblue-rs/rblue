//! # rblue-c
//!
//! Message-passing C ABI over the rblue cross-platform BLE facade: a small,
//! stable C surface (`rblue_new` / `rblue_request` / two callbacks) instead
//! of dozens of typed `extern` functions. It is the binding layer for the
//! Flutter example (`examples/flutter`), generated with ffigen from
//! `bindings/c/include/rblue.h`, and serves any other C-ABI consumer.
//!
//! ## Shape
//!
//! One facade handle per client, one request entry point, two callbacks:
//!
//! - [`rblue_request`] dispatches `method` + JSON params
//!   (+ optional binary payload) and resolves asynchronously through the
//!   registered response callback, correlated by the caller-chosen `req_id`.
//! - The response callback receives `req_id`, a `status_json` string (null on
//!   success) and a transient `payload` byte slice.
//! - The event callback receives one JSON envelope per adapter event or GATT
//!   notification.
//!
//! The `status_json` and event JSON are serde forms of `rblue_api` types, the
//! same contracts the UniFFI facade (Kotlin/Swift) uses:
//! `rblue_api::Error` for failures (with `Platform { code }` raw codes
//! preserved verbatim), `rblue_api::AdapterEvent` /
//! `rblue_api::Notification` wrapped in a `{"type": ...}` envelope for
//! events. Method arguments reuse the serde shapes of `ScanFilter`,
//! `DeviceId`, `CharacteristicId`, and so on. Malformed arguments never reach
//! the backend; they fail with `{"invalid_argument":{"message":...}}`,
//! mirroring the facade-only variant of `rblue-ffi`.
//!
//! ## Threading and memory contract
//!
//! - Callbacks fire on Rust worker threads of a facade-owned tokio runtime.
//!   Embedders must install **cross-thread** callbacks (Dart:
//!   `NativeCallable.listener`, never `isolateLocal`), and must not
//!   re-enter the same handle synchronously from inside a callback.
//! - Strings handed to callbacks are **owned by the callee**: copy them, then
//!   release them with [`rblue_free_string`]. The `payload` byte slice is
//!   a transient borrow by default. Async listeners must register with
//!   [`rblue_set_owned_response_callback`] and free their payloads with
//!   [`rblue_free_payload`].
//! - `rblue_request` copies the method, params, and payload before returning,
//!   so callers may free their buffers immediately.
//! - [`rblue_shutdown`] stops every native callback producer and reports
//!   the final delivery counts, allowing asynchronous embedders to drain their
//!   callback queues before [`rblue_free`] releases the handle. Explicit
//!   shutdown and free are required; an async callback closure may keep the
//!   client reachable, so a finalizer cannot replace this discipline.
//!
//! ## Android initialization
//!
//! The Android backend's JNI bridge requires a Java-side init before the
//! first adapter call. This crate exports
//! `Java_com_rblue_RblueNative_nativeInit` from `librblue_c.so` (the
//! implementation lives in `rblue-android`) and the host must call it: on
//! Android the `rblue_flutter` plugin declares the Kotlin class
//! `com.rblue.RblueNative` and invokes it when the plugin attaches (see the
//! plugin README). There is no such step on iOS.

#![warn(missing_docs)]

mod callbacks;
mod dispatch;
mod event;
mod ffi;
mod handle;

#[cfg(test)]
mod soak;
#[cfg(test)]
mod testing;

pub use callbacks::{RblueEventCallback, RblueResponseCallback};
pub use ffi::{
    rblue_free, rblue_free_payload, rblue_free_string, rblue_new, rblue_request,
    rblue_set_event_callback, rblue_set_owned_response_callback, rblue_set_response_callback,
    rblue_shutdown, RblueHandle, RBLUE_REQUEST_INVALID_STRING, RBLUE_REQUEST_NULL_HANDLE,
};

/// JNI entry: `com.rblue.RblueNative.nativeInit()`.
///
/// Defined in this cdylib so rustc includes the symbol in the `librblue_c.so`
/// export list. The implementation lives in `rblue-android`.
///
/// # Safety
///
/// This is a JNI entry point and is meant to be called only by the JVM when
/// Java/Kotlin code invokes `RblueNative.nativeInit()`. `env` must be a valid,
/// non-null `JNIEnv*` belonging to the calling thread, which must be attached
/// to the JVM for the duration of the call. `class` must be the valid `jclass`
/// local reference the JVM passes for `com.rblue.RblueNative` (it is unused).
/// Calling this from Rust with any other pointers is undefined behavior.
#[cfg(all(feature = "android", target_os = "android"))]
#[no_mangle]
pub unsafe extern "system" fn Java_com_rblue_RblueNative_nativeInit(
    env: *mut std::ffi::c_void,
    class: *mut std::ffi::c_void,
) -> bool {
    // SAFETY: the caller (the JVM) upholds this function's safety contract,
    // which is exactly the contract of `jni_native_init`.
    unsafe { rblue::platform::android::jni_native_init(env, class) }
}

/// Prevent the linker from dropping `Java_com_rblue_RblueNative_nativeInit`
/// as unused. Complements the `-u` / `--export-dynamic-symbol` flags in
/// `build.rs` on Android targets that still omit an unreferenced export.
#[cfg(all(feature = "android", target_os = "android"))]
#[used]
static KEEP_JNI_NATIVE_INIT: unsafe extern "system" fn(
    *mut std::ffi::c_void,
    *mut std::ffi::c_void,
) -> bool = Java_com_rblue_RblueNative_nativeInit;

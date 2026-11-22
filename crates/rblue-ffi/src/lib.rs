//! UniFFI facade for rblue (Kotlin / Swift).
//!
//! One async surface shared by both binding languages: async methods become
//! Kotlin `suspend fun` (through UniFFI foreign futures) and Swift `async`,
//! and adapter events plus GATT notifications are pushed to the host through
//! the [`EventSink`] callback interface, which the binding layers wrap into
//! `callbackFlow(BUFFERED)` (Kotlin) / `AsyncStream` (Swift). The backend is
//! selected with the same feature set as the aggregate `rblue` crate;
//! [`Rblue::new`] constructs it through `rblue::platform::default_adapter`.
//!
//! # Serialization contract
//!
//! Structured payloads cross the FFI as serde JSON strings, exactly like the
//! Driver-track control plane: identifiers ([`DeviceId`](rblue_api::DeviceId),
//! [`CharacteristicId`](rblue_api::CharacteristicId)), request values
//! ([`ScanFilter`](rblue_api::ScanFilter),
//! [`RequestDeviceOptions`](rblue_api::RequestDeviceOptions)), and results
//! ([`ServiceSet`](rblue_api::ServiceSet), the capability set). The JSON forms
//! are the `serde` representations of the `rblue-api` types (snake_case
//! externally tagged enums), so a value received through [`EventSink`] can be
//! passed back into a method argument verbatim. Scalars stay native: bytes as
//! `Vec<u8>`, booleans, and integers.
//!
//! Identifiers are opaque to the host: never construct or persist them, take
//! them from discovery/request results (see the `rblue-api` `DeviceId` notes
//! on RPA rotation and reinstall-sensitive UUIDs).
//!
//! # Event delivery and backpressure
//!
//! `set_event_sink` installs the sink and, on first install, starts one
//! pump task on a facade-owned tokio runtime. The pump consumes the
//! adapter's broadcast event stream plus a bounded (`capacity 256`) channel
//! of notifications and invokes the sink sequentially. Consequences,
//! mirrored by the plan's facade section:
//!
//! - The sink is called from pump worker threads, never the host main
//!   thread; implementations must not block (hand the value to a coroutine /
//!   actor and return). A sink that throws kills the pump.
//! - A slow sink applies backpressure through the bounded channel up into
//!   the backend's broadcast buffers - nothing is dropped silently at this
//!   layer. The binding layers must add their own buffering
//!   (`callbackFlow(BUFFERED)` with a drop-oldest policy and a
//!   dropped-event counter in Kotlin, `AsyncStream(bufferingPolicy:
//!   .bufferingNewest)` in Swift) and surface the drop count; for high-rate
//!   notify streams, prefer consuming the Rust-side aggregate stream instead
//!   of per-value callbacks.
//! - `subscribe` attaches the backend broadcast before it resolves, so
//!   values published afterwards are never missed. Values subscribed before
//!   the first `set_event_sink` buffer in the channel and flush on install.
//!   Replacing a sink swaps only the consumer; channel and forwarders keep
//!   running.
//!
//! # Binding generation
//!
//! The scaffolded library is generated in library mode (proc-macro
//! definitions, no UDL):
//!
//! ```text
//! cargo build -p rblue-ffi --release
//! cargo run -p rblue-ffi --bin uniffi-bindgen -- \
//!     generate --library <target/release librblue_ffi artifact> \
//!     --language kotlin|swift --out-dir bindings/<kotlin|swift>
//! ```
//!
//! `cargo xtask bind-kotlin` / `bind-swift` run exactly that (see the xtask
//! crate); generated code lives in `bindings/` and is never hand-edited.
//!
//! # wasm32
//!
//! The facade is compiled out on `wasm32-unknown-unknown`: the UniFFI surface
//! targets Kotlin/Swift, the web is served by the aggregate `rblue` crate's
//! `web` feature directly, and the facade's tokio runtime has no
//! multi-threaded form there. The crate still builds on wasm32 as an empty
//! library so workspace-wide web checks stay green.

#![warn(missing_docs)]

#[cfg(not(target_arch = "wasm32"))]
mod error;
#[cfg(not(target_arch = "wasm32"))]
mod facade;

#[cfg(not(target_arch = "wasm32"))]
pub use error::{PairingStatus, PermissionReason, RblueError};
#[cfg(not(target_arch = "wasm32"))]
pub use facade::{EventSink, Rblue};

#[cfg(not(target_arch = "wasm32"))]
uniffi::setup_scaffolding!();

/// JNI entry: `com.rblue.RblueNative.nativeInit()`.
///
/// Defined in this cdylib so rustc includes the symbol in the
/// `librblue_ffi.so` export list. The implementation lives in `rblue-android`.
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

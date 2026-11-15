//! Android (JNI) backend for rblue: a thin adapter shell over the forked
//! [`btleplug`] Android (droidplug) implementation - JNI calls into the
//! `com.nonpolynomial.btleplug.android.impl` helper classes, mirrored here
//! into the unified [`rblue_api`] model.
//!
//! # Entry points
//!
//! - `rblue_android::init(&mut env)` (Android targets only): the
//!   Android-specific first step. It installs the fork's JNI
//!   native methods and creates its global adapter, and must be called once
//!   with the application's `JNIEnv` - from a thread that can see the
//!   application `ClassLoader` (the AAR's `Rblue.initialize(context)` Kotlin
//!   wrapper guarantees this). Never call it from a `static` initializer;
//!   see `docs/platform-notes.md`.
//! - [`default_adapter`]: the system-default central. Construction does not
//!   touch the Bluetooth stack; call `Adapter::initialize` before any other
//!   operation (it fetches the fork's global adapter and spawns the event
//!   pump).
//! - [`backend_version`]: identifies the shell and the pinned fork revision.
//!
//! # Platform facts honored by this backend
//!
//! - **Raw GATT status codes are preserved, never stringified.** Every
//!   status integer the fork surfaces (the Java layer embeds it in its
//!   `RuntimeException` messages) is recovered into
//!   `rblue_api::Error::Platform { code, message }` verbatim - Android
//!   GATT status 133 above all, the connection supervisor's special case.
//!   Where the fork itself swallows the number (connect-attempt failures,
//!   disconnect events), the limitation is documented instead of faked.
//! - Android is MAC-first, but the fork never reports the address kind, so
//!   every identity carries `AddressType::Unknown` and must not be used as
//!   a persistent key across RPA rotations.
//! - The ATT MTU is negotiated by the fork's automatic `requestMtu(517)`
//!   at connect; there is no client request API through the fork's trait
//!   surface (`Capability::MtuNegotiation` absent, `MtuQuery` present).
//! - `pair()` reports `Unsupported`: Android's `createBond` is not exposed
//!   by the fork.
//!
//! See `docs/platform-notes.md` and `docs/error-mapping.md` in the crate
//! root for the full platform documentation, including the Java helper
//! build choreography (`build-java.sh`), ProGuard consumer rules, the
//! API 24+ floor, and the permission matrix.
//!
//! # Runtime requirement
//!
//! The fork drives its callbacks through tokio tasks over the JNI event
//! bridge, so every method of this backend must be called from within a
//! tokio runtime context (tokio::spawn / tokio::sync primitives are used
//! internally).
//!
//! # Platform gating
//!
//! The adapter/peripheral glue compiles only on Android targets, where the
//! fork's `platform` module exposes the `droidplug` types. Host builds keep
//! the pure mapping and hub modules (and their tests) so the translation
//! semantics stay verifiable without an Android toolchain run.

#![warn(missing_docs)]

mod hub;
mod mapping;

#[cfg(target_os = "android")]
mod adapter;
#[cfg(target_os = "android")]
mod jni_shim;
#[cfg(target_os = "android")]
mod peripheral;

#[cfg(target_os = "android")]
use std::sync::atomic::{AtomicBool, Ordering};

use std::sync::Arc;

use rblue_api::adapter::Adapter;
use rblue_api::error::Error;

/// Version string of this backend: the shell crate version plus the pinned
/// btleplug fork revision (see the `btleplug` git dependency in Cargo.toml).
pub const BACKEND_VERSION: &str = "android/btleplug 1af8299c291f24fca68dfb6b7ef1b5c0475892e1";

#[cfg(target_os = "android")]
static JNI_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Returns `true` once [`init`] has completed. [`init`] is only usable on
/// Android targets.
#[cfg(target_os = "android")]
pub(crate) fn jni_initialized() -> bool {
    JNI_INITIALIZED.load(Ordering::Acquire)
}

/// Initializes the fork's JNI bridge and its global adapter.
///
/// Must be called once before `Adapter::initialize`, with the `JNIEnv` of a
/// thread attached to the application's JVM - the fork resolves the
/// `com.nonpolynomial.btleplug.android.impl` helper classes and the
/// `io.github.gedgygedgy.rust` support classes through the calling thread's
/// `ClassLoader`, so an arbitrary Rust-spawned thread cannot do this. The
/// AAR's `Rblue.initialize(context)` wrapper calls in from the application
/// context; never call this from a `static` initializer (the Android
/// ClassLoader is not ready there - see `docs/platform-notes.md`).
///
/// The call is idempotent: the fork's own initialization is guarded, so a
/// second call is a no-op that re-reports success.
///
/// # Errors
///
/// [`Error::Backend`] when the JNI registration or the helper-class lookup
/// fails - typically the AAR's Java classes being absent (ProGuard stripped
/// them, or the `build-java.sh` output was not packaged).
#[cfg(target_os = "android")]
pub fn init(env: &mut jni::Env<'_>) -> Result<(), Error> {
    btleplug::platform::init(env).map_err(crate::mapping::map_btleplug_error)?;
    JNI_INITIALIZED.store(true, Ordering::Release);
    Ok(())
}

/// Forces the JNI `nativeInit` export into consuming cdylibs.
///
/// Rust never calls this entry point, so the linker would otherwise drop it
/// when linking `librblue_ffi.so` or `librblue_c.so`.
#[cfg(target_os = "android")]
pub fn keep_jni_native_init() {
    let _ = jni_shim::KEEP_JNI_NATIVE_INIT;
}

/// Returns the system-default Android central wrapped in the unified
/// [`Adapter`] trait.
///
/// Construction does not touch the Bluetooth stack; call `Adapter::initialize`
/// to open it (it fetches the fork's global adapter - which exists only
/// after `init` ran - and spawns the event pump).
///
/// # Errors
///
/// On non-Android targets this always fails with [`Error::Unsupported`]: the
/// backend only functions where the fork's `droidplug` implementation and a
/// JVM exist.
///
/// # Examples
///
/// ```no_run
/// # async fn example() -> Result<(), rblue_api::Error> {
/// use futures::StreamExt;
/// use rblue_api::Adapter;
/// use rblue_android::default_adapter;
///
/// // Android: call rblue_android::init(&mut env) with the application
/// // JNIEnv first (from the AAR's Rblue.initialize(context)).
/// let adapter = default_adapter()?;
/// adapter.initialize().await?;
/// adapter.start_scan(&rblue_api::ScanFilter::default()).await?;
/// let mut events = adapter.events();
/// while let Some(event) = events.next().await {
///     // Identifiers are personal data: never log them at info level or above.
///     let _ = event;
/// }
/// # Ok(())
/// # }
/// ```
#[cfg(target_os = "android")]
pub fn default_adapter() -> Result<Arc<dyn Adapter>, Error> {
    Ok(Arc::new(adapter::AndroidAdapter::new()))
}

/// Returns the system-default Android central wrapped in the unified
/// [`Adapter`] trait.
///
/// # Errors
///
/// On non-Android targets this always fails with [`Error::Unsupported`]: the
/// backend only functions where the fork's `droidplug` implementation and a
/// JVM exist. The usage example is shared with the Android variant so it
/// stays compile-checked on every host.
///
/// # Examples
///
/// ```no_run
/// # async fn example() -> Result<(), rblue_api::Error> {
/// use futures::StreamExt;
/// use rblue_api::Adapter;
/// use rblue_android::default_adapter;
///
/// let adapter = default_adapter()?;
/// adapter.initialize().await?;
/// adapter.start_scan(&rblue_api::ScanFilter::default()).await?;
/// let mut events = adapter.events();
/// while let Some(event) = events.next().await {
///     let _ = event;
/// }
/// # Ok(())
/// # }
/// ```
#[cfg(not(target_os = "android"))]
pub fn default_adapter() -> Result<Arc<dyn Adapter>, Error> {
    Err(Error::Unsupported {
        platform: mapping::PLATFORM,
        feature: "adapter",
    })
}

/// Identifies the backend build: shell crate and pinned btleplug fork
/// revision.
pub fn backend_version() -> &'static str {
    BACKEND_VERSION
}

#[cfg(test)]
mod tests {
    #[test]
    fn backend_version_names_the_pinned_revision() {
        assert_eq!(
            super::backend_version(),
            "android/btleplug 1af8299c291f24fca68dfb6b7ef1b5c0475892e1"
        );
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn host_construction_reports_unsupported() {
        assert!(matches!(
            super::default_adapter(),
            Err(rblue_api::Error::Unsupported {
                platform: "android",
                ..
            })
        ));
    }
}

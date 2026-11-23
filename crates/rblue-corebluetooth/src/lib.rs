//! macOS / iOS (CoreBluetooth) backend for rblue: a thin adapter shell over
//! the forked [`btleplug`] `corebluetooth` implementation
//! (`CBCentralManager` delegate callbacks bridged to the async world by the
//! fork, mirrored here into the unified [`rblue_api`] model).
//!
//! # Entry points
//!
//! - `default_adapter`: the system-default central. Call
//!   `rblue_api::Adapter::initialize` before any other
//!   operation; it awaits `centralManagerDidUpdateState`, after which every
//!   platform call is valid.
//! - [`backend_version`]: identifies the shell and the pinned fork revision.
//!
//! # Platform facts honored by this backend
//!
//! - Identity is `DeviceId::Uuid` only: CoreBluetooth never exposes a MAC.
//!   The UUID is stable per device per app install.
//! - `initialize` waits for the first state callback; a powered-off radio is
//!   [`Error::Permission`] (`BluetoothOff`), missing/unsupported hardware is
//!   [`Error::Unsupported`].
//! - Hardware scan filtering supports service UUIDs only;
//!   `name_prefix`/`rssi_threshold`/manufacturer filters are applied in
//!   software on the full advertisement stream. iOS background scanning
//!   yields nothing without a service-UUID filter.
//! - Unified write semantics: oversize payloads are rejected with
//!   [`Error::InvalidLength`] before reaching `writeValue`, which would
//!   silently truncate them in the `withoutResponse` mode.
//! - `max_write_len` maps to the `maximumWriteValueLength(for:)` semantics
//!   (ATT MTU - 3); `mtu` itself is system-managed and reports
//!   [`Error::Unsupported`], as do `pair` (implicit pairing) and
//!   `request_device` (web-only).
//!
//! See `docs/platform-notes.md` and `docs/error-mapping.md` in the crate
//! root for the full platform documentation.
//!
//! # Runtime requirement
//!
//! The fork drives its callbacks through tokio tasks, so every method of
//! this backend must be called from within a tokio runtime context
//! (tokio::spawn / tokio::sync primitives are used internally).
//!
//! # Platform gating
//!
//! The adapter/peripheral glue compiles only on Apple targets, where the
//! fork's `platform` module exposes the CoreBluetooth types. Host builds keep
//! the pure mapping and hub modules (and their tests) so the translation
//! semantics stay verifiable without an Apple machine.

#![warn(missing_docs)]

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod adapter;
mod hub;
mod mapping;
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod peripheral;

use std::sync::Arc;

use rblue_api::adapter::Adapter;
use rblue_api::error::Error;

/// Version string of this backend: the shell crate version plus the pinned
/// btleplug fork revision (see the `btleplug` git dependency in Cargo.toml).
pub const BACKEND_VERSION: &str = "corebluetooth/btleplug 983f3f1346bb1646281e0faf345a21c4a787833d";

/// Returns the system-default CoreBluetooth central wrapped in the unified
/// [`Adapter`] trait.
///
/// Construction does not touch the Bluetooth stack; call
/// `rblue_api::Adapter::initialize` to open it (this
/// is where the platform authorization prompt and the
/// `centralManagerDidUpdateState` wait happen).
///
/// # Errors
///
/// Never fails on Apple targets; the `Result` matches the cross-backend
/// contract that adapter acquisition can be fallible. On non-Apple hosts this
/// always fails with [`Error::Unsupported`]: the backend only functions where
/// the fork's CoreBluetooth implementation exists.
///
/// # Examples
///
/// ```no_run
/// # async fn example() -> Result<(), rblue_api::Error> {
/// use futures::StreamExt;
/// use rblue_api::Adapter;
/// use rblue_corebluetooth::default_adapter;
///
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
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub fn default_adapter() -> Result<Arc<dyn Adapter>, Error> {
    Ok(Arc::new(adapter::CoreBluetoothAdapter::new()))
}

/// Returns the system-default CoreBluetooth central wrapped in the unified
/// [`Adapter`] trait.
///
/// # Errors
///
/// On non-Apple targets this always fails with [`Error::Unsupported`]: the
/// backend only functions where the fork's CoreBluetooth implementation
/// exists. The usage example is shared with the Apple variant so it stays
/// compile-checked on every host.
///
/// # Examples
///
/// ```no_run
/// # async fn example() -> Result<(), rblue_api::Error> {
/// use futures::StreamExt;
/// use rblue_api::Adapter;
/// use rblue_corebluetooth::default_adapter;
///
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
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
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
            "corebluetooth/btleplug 983f3f1346bb1646281e0faf345a21c4a787833d"
        );
    }

    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    #[test]
    fn host_construction_reports_unsupported() {
        assert!(matches!(
            super::default_adapter(),
            Err(rblue_api::Error::Unsupported {
                platform: "corebluetooth",
                ..
            })
        ));
    }

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    #[test]
    fn default_adapter_constructs_without_touching_the_stack() {
        let adapter = super::default_adapter().expect("constructed");
        assert_eq!(adapter.id().as_str(), "default");
    }
}

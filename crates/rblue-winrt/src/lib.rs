//! Windows (WinRT) backend for rblue: a thin adapter shell over the forked
//! [`btleplug`] `winrtble` implementation (`BluetoothLEAdvertisementWatcher`
//! for scanning, `BluetoothLEDevice::FromBluetoothAddressAsync` plus
//! `GattSession` for connecting, mirrored here into the unified
//! [`rblue_api`] model).
//!
//! # Entry points
//!
//! - [`default_adapter`]: the system-default central. Call
//!   `Adapter::initialize` before any other
//!   operation; it enumerates the Bluetooth radios and reports the radio
//!   state.
//! - [`backend_version`]: identifies the shell and the pinned fork revision.
//!
//! # Platform facts honored by this backend
//!
//! - Windows 10 1809+ is the floor: `GattSession` (and with it
//!   `MaxPduSize`) requires it.
//! - A central can only connect to addresses its watcher has observed in
//!   this boot session. Resolvable private addresses rotate about every 15
//!   minutes, so reconnecting across a rotation means re-scanning; a
//!   `connect` on an address the OS does not know fails with
//!   [`Error::NotFound`].
//! - `max_write_len` is the negotiated ATT MTU (`GattSession.MaxPduSize`)
//!   minus the 3-byte ATT header; the MTU itself is negotiated by the stack,
//!   so `Capability::MtuNegotiation` is absent while `Capability::MtuQuery`
//!   is present. Connected-peripheral RSSI has no WinRT API (scan-time
//!   only), pairing exists in WinRT but the fork exposes no API for it, and
//!   `request_device` is web-only.
//! - `GattSession.MaintainConnection` is not set by the fork; the
//!   connection lives as long as the shell holds the peripheral's device
//!   and session objects. See `docs/platform-notes.md` for the details.
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
//! The adapter/peripheral glue compiles only on Windows targets, where the
//! fork's `platform` module exposes the `winrtble` types. Host builds keep
//! the pure mapping and hub modules (and their tests) so the translation
//! semantics stay verifiable without a Windows machine.

#![warn(missing_docs)]

mod hub;
mod mapping;

#[cfg(target_os = "windows")]
mod adapter;
#[cfg(target_os = "windows")]
mod peripheral;

use std::sync::Arc;

use rblue_api::adapter::Adapter;
use rblue_api::error::Error;

/// Version string of this backend: the shell crate version plus the pinned
/// btleplug fork revision (see the `btleplug` git dependency in Cargo.toml).
pub const BACKEND_VERSION: &str = "winrt/btleplug 983f3f1346bb1646281e0faf345a21c4a787833d";

/// Returns the system-default WinRT central wrapped in the unified
/// [`Adapter`] trait.
///
/// Construction does not touch the Bluetooth stack; call `Adapter::initialize`
/// to open it (this enumerates the radios, spawns the event pump, and
/// resolves the radio state).
///
/// # Errors
///
/// On non-Windows targets this always fails with [`Error::Unsupported`]:
/// the backend only functions where the fork's `winrtble` implementation
/// exists.
///
/// # Examples
///
/// ```no_run
/// # async fn example() -> Result<(), rblue_api::Error> {
/// use futures::StreamExt;
/// use rblue_api::Adapter;
/// use rblue_winrt::default_adapter;
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
#[cfg(target_os = "windows")]
pub fn default_adapter() -> Result<Arc<dyn Adapter>, Error> {
    Ok(Arc::new(adapter::WinRtAdapter::new()))
}

/// Returns the system-default WinRT central wrapped in the unified
/// [`Adapter`] trait.
///
/// # Errors
///
/// On non-Windows targets this always fails with [`Error::Unsupported`]:
/// the backend only functions where the fork's `winrtble` implementation
/// exists. The usage example is shared with the Windows variant so it stays
/// compile-checked on every host.
///
/// # Examples
///
/// ```no_run
/// # async fn example() -> Result<(), rblue_api::Error> {
/// use futures::StreamExt;
/// use rblue_api::Adapter;
/// use rblue_winrt::default_adapter;
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
#[cfg(not(target_os = "windows"))]
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
            "winrt/btleplug 983f3f1346bb1646281e0faf345a21c4a787833d"
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn host_construction_reports_unsupported() {
        assert!(matches!(
            super::default_adapter(),
            Err(rblue_api::Error::Unsupported {
                platform: "winrt",
                ..
            })
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn adapter_construction_does_not_touch_the_stack() {
        // Construction is pure; the btleplug manager is only created in
        // `initialize`. This runs on Windows machines only (the glue does
        // not compile elsewhere).
        let adapter = super::default_adapter().expect("constructed");
        assert_eq!(adapter.id().as_str(), "default");
    }
}

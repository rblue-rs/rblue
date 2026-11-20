//! Linux (BlueZ) backend for rblue: a thin adapter shell over the forked
//! [`btleplug`] `bluez` implementation (D-Bus to `bluetoothd` through
//! bluez-async, mirrored here into the unified [`rblue_api`] model).
//!
//! # Entry points
//!
//! - [`default_adapter`]: the system-default central. Call
//!   `Adapter::initialize` before any other operation; it connects to
//!   `org.bluez`, picks the first adapter, spawns the event pump (which
//!   imports BlueZ's persistent device cache), and resolves the radio state.
//! - [`backend_version`]: identifies the shell and the pinned fork revision -
//!   the probe the plan requires before the Phase 2 switch of this shell's
//!   interior to `bluer` (the [`rblue_api`] traits stay unchanged).
//!
//! # Platform facts honored by this backend
//!
//! - The ATT MTU is negotiated by the stack; there is no client request API
//!   (`Capability::MtuNegotiation` absent). The negotiated value is
//!   queryable through the D-Bus `MTU` property on recent BlueZ releases
//!   (`Capability::MtuQuery` present); where the property is absent, or
//!   before the first `discover_services`, the shell works with the ATT
//!   default of 23 (`max_write_len` = 20).
//! - BlueZ does not update the device RSSI property while connected, so
//!   `rssi()` reports [`Error::Unsupported`] instead of presenting stale
//!   scan-time data.
//! - BlueZ keeps devices in a persistent cache across sessions;
//!   `initialize` imports that cache into the registry as discoveries, so
//!   cache-known devices are connectable without a fresh scan and
//!   `Error::NotFound` means "this adapter instance never saw it".
//! - `pair()` / `unpair()` are implemented in the shell over D-Bus
//!   (`org.bluez.Device1.Pair` / `org.bluez.Adapter1.RemoveDevice`) and
//!   report [`Capability::Pairing`](rblue_api::Capability::Pairing). The
//!   fork's `Peripheral` trait still has no pairing API.
//! - `request_device` is web-only.
//!
//! See `docs/platform-notes.md` and `docs/error-mapping.md` in the crate
//! root for the full platform documentation, including the
//! `/etc/bluetooth/main.conf` deployment notes and the Phase 2 `bluer`
//! migration path.
//!
//! # Runtime requirement
//!
//! The fork drives its callbacks through tokio tasks on a D-Bus event
//! stream, so every method of this backend must be called from within a
//! tokio runtime context (tokio::spawn / tokio::sync primitives are used
//! internally).
//!
//! # Platform gating
//!
//! The adapter/peripheral glue compiles only on Linux targets, where the
//! fork's `platform` module exposes the `bluez` types (bluez-async needs
//! `libdbus` there). Host builds keep the pure mapping and hub modules (and
//! their tests) so the translation semantics stay verifiable without a
//! Linux machine.

#![warn(missing_docs)]

mod hub;
mod mapping;
mod pairing;

#[cfg(target_os = "linux")]
mod adapter;
#[cfg(target_os = "linux")]
mod peripheral;

use std::sync::Arc;

use rblue_api::adapter::Adapter;
use rblue_api::error::Error;

/// Version string of this backend: the shell crate version plus the pinned
/// btleplug fork revision (see the `btleplug` git dependency in Cargo.toml).
pub const BACKEND_VERSION: &str = "bluez/btleplug 1af8299c291f24fca68dfb6b7ef1b5c0475892e1";

/// Returns the system-default BlueZ central wrapped in the unified
/// [`Adapter`] trait.
///
/// Construction does not touch the Bluetooth stack; call `Adapter::initialize`
/// to open it (this connects to `org.bluez` over D-Bus, spawns the event
/// pump, and resolves the adapter state).
///
/// # Errors
///
/// On non-Linux targets this always fails with [`Error::Unsupported`]: the
/// backend only functions where the fork's `bluez` implementation exists.
///
/// # Examples
///
/// ```no_run
/// # async fn example() -> Result<(), rblue_api::Error> {
/// use futures::StreamExt;
/// use rblue_api::Adapter;
/// use rblue_bluez::default_adapter;
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
#[cfg(target_os = "linux")]
pub fn default_adapter() -> Result<Arc<dyn Adapter>, Error> {
    Ok(Arc::new(adapter::BluezAdapter::new()))
}

/// Returns the system-default BlueZ central wrapped in the unified
/// [`Adapter`] trait.
///
/// # Errors
///
/// On non-Linux targets this always fails with [`Error::Unsupported`]: the
/// backend only functions where the fork's `bluez` implementation exists.
/// The usage example is shared with the Linux variant so it stays
/// compile-checked on every host.
///
/// # Examples
///
/// ```no_run
/// # async fn example() -> Result<(), rblue_api::Error> {
/// use futures::StreamExt;
/// use rblue_api::Adapter;
/// use rblue_bluez::default_adapter;
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
#[cfg(not(target_os = "linux"))]
pub fn default_adapter() -> Result<Arc<dyn Adapter>, Error> {
    Err(Error::Unsupported {
        platform: mapping::PLATFORM,
        feature: "adapter",
    })
}

/// Identifies the backend build: shell crate and pinned btleplug fork
/// revision. The plan requires this probe so the Phase 2 `bluer` switch can
/// be detected at runtime.
pub fn backend_version() -> &'static str {
    BACKEND_VERSION
}

#[cfg(test)]
mod tests {
    #[test]
    fn backend_version_names_the_pinned_revision() {
        assert_eq!(
            super::backend_version(),
            "bluez/btleplug 1af8299c291f24fca68dfb6b7ef1b5c0475892e1"
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn host_construction_reports_unsupported() {
        assert!(matches!(
            super::default_adapter(),
            Err(rblue_api::Error::Unsupported {
                platform: "bluez",
                ..
            })
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn adapter_construction_does_not_touch_the_stack() {
        // Construction is pure; the D-Bus session is only created in
        // `initialize`. This runs on Linux machines only (the glue does not
        // compile elsewhere).
        let adapter = super::default_adapter().expect("constructed");
        assert_eq!(adapter.id().as_str(), "default");
    }
}

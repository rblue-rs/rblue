//! Web (Web Bluetooth) backend for rblue, targeting `wasm32-unknown-unknown`.
//!
//! This crate implements the [`rblue_api`] `Adapter`/`Peripheral` surface on
//! top of the browser's Web Bluetooth API. Its public surface is intentionally
//! small: [`default_adapter`] for construction, [`backend_version`] for
//! diagnostics, and `detect_watch_advertisements` for the one runtime
//! capability probe the platform offers.
//!
//! # Platform semantics (implementation plan section 3.5)
//!
//! - **No background scanning.** The browser never scans on its own:
//!   `start_scan`/`stop_scan` return [`Error::Unsupported`] and the
//!   capability set reports neither `ActiveScan` nor `PassiveScan`. Devices
//!   are obtained exclusively through `request_device`, which pops the
//!   browser chooser.
//! - **User gesture.** `request_device` must run inside a click/touch
//!   handler; the Rust core cannot verify this. The facade layer (the TS/npm
//!   package) is responsible for binding the call to a gesture.
//! - **`optionalServices` trap.** Every service the application will touch
//!   must be listed at chooser time. The backend merges
//!   `RequestDeviceOptions.optional_services` with the filter `services`
//!   automatically (see the `options` module).
//! - **Connections.** `connect` maps to `device.gatt().connect()`,
//!   `discover_services` to `getPrimaryServices()` (the browser
//!   permission-filters the result against `optionalServices`).
//! - **Notifications.** `subscribe` uses `startNotifications()` plus a
//!   `characteristicvaluechanged` listener; values are broadcast on the
//!   peripheral's notification stream.
//! - **Disconnects.** A `gattserverdisconnected` event is translated into
//!   `AdapterEvent::DeviceDisconnected` on the adapter event stream. The web
//!   does not reconnect by itself: the supervisor must call `connect` again
//!   and then re-discover and re-subscribe.
//! - **Writes and MTU.** The browser does not chunk writes; the unified write
//!   semantics reject payloads above a conservative 20 bytes (default ATT MTU
//!   23 minus 3) with [`Error::InvalidLength`] before any browser call. `mtu`
//!   itself is unsupported.
//! - **Pairing/RSSI.** `pair` and `rssi` are unsupported; the browser exposes
//!   neither.
//!
//! # Browser support
//!
//! Web Bluetooth ships in Chromium browsers only, in a secure context
//! (`https://` or `localhost`). Where `navigator.bluetooth` is missing
//! (Safari, Firefox), [`default_adapter`] still constructs an adapter, but
//! `initialize` - and every other entry point - reports
//! [`Error::Unsupported`] with feature `"bluetooth"`.
//!
//! # Binding strategy
//!
//! `web-sys` gates its Web Bluetooth bindings behind
//! `--cfg=web_sys_unstable_apis` (a `RUSTFLAGS` requirement that would leak
//! into every build of this crate and its dependents). The Web Bluetooth DOM
//! surface this backend needs is small, so it is declared directly with
//! `wasm_bindgen` in `backend.rs`; `web-sys` remains in use for the stable
//! `Window`/`Navigator`/`Event`/`EventTarget` pieces.
//!
//! # Threading
//!
//! Web Bluetooth is main-thread-only. JS values and futures are held in an
//! internal `Send`/`Sync` wrapper to satisfy the trait bounds; never move an
//! adapter into a Web Worker - construct one inside the worker's realm
//! instead.
//!
//! The pure building blocks (DOM-exception mapping, request-device option
//! assembly, id wrapping, the bounded broadcast bus) are compiled on every
//! target and unit-tested on the host; see the `error_map`, `options`, `id`,
//! and `broadcast` modules. Browser end-to-end tests are tracked in
//! `docs/platform-notes.md`.

#![warn(missing_docs)]

use std::sync::Arc;

use rblue_api::{Adapter, Error};

pub mod error_map;
pub mod id;
pub mod options;

#[cfg(target_arch = "wasm32")]
mod backend;
#[cfg(any(target_arch = "wasm32", test))]
mod broadcast;
#[cfg(any(target_arch = "wasm32", test))]
mod sync;

/// Returns the web adapter.
///
/// Construction never fails: where Web Bluetooth is unavailable
/// (non-Chromium browsers, non-browser runtimes) the adapter reports
/// [`Error::Unsupported`] with feature `"bluetooth"` from `initialize` and
/// from every other entry point.
///
/// On non-wasm targets this backend has no implementation and the call
/// always fails; build this crate for `wasm32-unknown-unknown` to get the
/// real adapter.
///
/// # Errors
///
/// On non-wasm targets, [`Error::Unsupported`] with platform `"host"` - the
/// web backend cannot run there.
#[cfg(target_arch = "wasm32")]
pub fn default_adapter() -> Result<Arc<dyn Adapter>, Error> {
    Ok(backend::default_adapter())
}

/// Non-wasm placeholder so the crate compiles and tests on the host; see the
/// wasm32 overload for the real implementation.
///
/// # Errors
///
/// Always [`Error::Unsupported`] with platform `"host"`.
#[cfg(not(target_arch = "wasm32"))]
pub fn default_adapter() -> Result<Arc<dyn Adapter>, Error> {
    Err(Error::Unsupported {
        platform: "host",
        feature: "web-bluetooth",
    })
}

/// Backend version string, for diagnostics and backend-swap probes.
#[must_use]
pub fn backend_version() -> &'static str {
    concat!("rblue-web/", env!("CARGO_PKG_VERSION"))
}

/// Runtime probe for `BluetoothDevice.watchAdvertisements()`.
///
/// The API only exists behind a Chrome flag, so it is detected by inspecting
/// `window.BluetoothDevice.prototype` rather than assumed. The adapter
/// caches the same probe and includes [`rblue_api::Capability::
/// WatchAdvertisements`] in `capabilities()` only after a successful
/// detection; this free function exposes the detection itself for facade
/// layers that want to decide independently of the capability set.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn detect_watch_advertisements() -> bool {
    backend::detect_watch_advertisements()
}

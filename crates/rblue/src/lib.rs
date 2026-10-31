//! # rblue
//!
//! **r**ust + **blue**(tooth) — cross-platform async Bluetooth Low Energy,
//! one unified API across Windows, macOS, Linux, iOS, Android,
//! Web Bluetooth, and HarmonyOS.
//!
//! Pronounced **"R-blue"**; member of the Blue\* family (BlueZ, bluer, bluest).
//!
//! ## This release
//!
//! This is a **name-reservation release**: the `rblue` crate name is now
//! permanently taken on crates.io. The unified API is in design and nothing
//! in `0.0.x` is usable yet — pin nothing, watch the repository instead.
//!
//! ## Planned layering
//!
//! - `rblue-api` — core traits: `Adapter` / `Peripheral` / GATT, error model,
//!   capability negotiation (runtime-agnostic, no_std-friendly later)
//! - platform backends — Windows (WinRT) · macOS/iOS (CoreBluetooth) ·
//!   Linux (BlueZ) · Android (JNI) · Web (web-sys / Web Bluetooth) ·
//!   HarmonyOS (ArkTS driver bridged over Node-API)
//! - bindings — Kotlin (UniFFI) · Swift (UniFFI) · TypeScript (wasm-bindgen) ·
//!   ArkTS (ohos-rs)
//!
//! ## Aggregate layer (this crate)
//!
//! Since the name reservation, this crate has grown into the L2 *aggregate*
//! of the workspace: it feature-selects the backend crates and re-exports
//! their single entry point plus the whole `rblue-api` type surface, so an
//! application only ever depends on `rblue`. The pattern is the same as
//! `btleplug::platform`: one [`platform`] module that resolves the backend
//! for the target at compile time.
//!
//! ### Backend features
//!
//! | Feature | Backend | Target |
//! |---|---|---|
//! | `mock` (default) | in-memory mock, no hardware | every target |
//! | `winrt` | Windows WinRT | `target_os = "windows"` |
//! | `corebluetooth` | macOS / iOS CoreBluetooth | `target_vendor = "apple"` |
//! | `bluez` | Linux BlueZ over D-Bus | `target_os = "linux"`, `target_env != "ohos"` |
//! | `android` | Android JNI (btleplug droidplug) | `target_os = "android"` |
//! | `web` | Web Bluetooth (web-sys) | `target_arch = "wasm32"` |
//! | `ohos` | HarmonyOS Driver track | `target_os = "linux"`, `target_env = "ohos"` |
//!
//! [`platform::default_adapter`] picks the compiled-in backend that matches
//! the current target (the automatic choice). Precedence when several
//! features are on:
//!
//! 1. the backend matching the target OS (the automatic choice),
//! 2. `mock` — it functions everywhere and is the default feature,
//! 3. otherwise the first enabled backend in the fixed order `web`, `ohos`,
//!    `winrt`, `corebluetooth`, `bluez`, `android`. Backends running away
//!    from their native target compile but fail at runtime with
//!    [`Error::Unsupported`], so cross-target experiments stay buildable.
//!
//! With no backend feature at all the crate refuses to compile with a
//! targeted error. Every compiled-in backend also stays reachable under its
//! own [`platform`] submodule (`platform::winrt`, `platform::mock`, ...),
//! which is how a host test can address a non-selected backend explicitly.
//!
//! ### Re-exports
//!
//! The full `rblue-api` type surface is re-exported flat at the crate root
//! and in [`prelude`] — traits ([`Adapter`], [`Peripheral`]), the
//! [`Error`] model, GATT and scan types, and the value-add layer modules
//! [`supervisor`], [`queue`], [`cache`], [`debounce`], and [`session`].
//! `rblue_api` itself is re-exported too, for paths the flat surface does
//! not cover.
//!
//! # Examples
//!
//! With the default features (the mock backend, no Bluetooth hardware):
//!
//! ```
//! use futures::StreamExt;
//! use rblue::prelude::*;
//!
//! # fn main() -> Result<(), Error> {
//! # futures::executor::block_on(async {
//! let adapter = rblue::default_adapter()?;
//! adapter.initialize().await?;
//!
//! adapter.start_scan(&ScanFilter::default()).await?;
//! // ... drain `adapter.events()` for discoveries ...
//! adapter.stop_scan().await?;
//! # Ok::<(), Error>(())
//! # })?;
//! # Ok(())
//! # }
//! ```
//!
//! Enable a real backend explicitly, e.g. for a macOS/iOS app:
//!
//! ```toml
//! [dependencies]
//! rblue = { version = "0.0.1", features = ["corebluetooth"] }
//! ```
//!
//! The [platform module](self) documents each backend's submodule and the
//! driver-track caveat for `ohos` (a `BleDriver` implementation must be
//! injected from ArkTS before the adapter can do anything).

#![warn(missing_docs)]

pub mod platform;

pub use rblue_api;

// The value-add layer lives in `rblue-api`; re-export the modules so
// `rblue::supervisor::...` style paths work next to the flat types.
pub use rblue_api::{cache, debounce, queue, session, supervisor};

// Flat type surface, kept in sync with the core crate's prelude by
// construction.
pub use rblue_api::prelude::*;

pub use platform::{backend_version, default_adapter};

pub mod prelude {
    //! Single glob import for the aggregate surface: everything from the
    //! `rblue-api` prelude plus the platform-selected entry points.
    //!
    //! ```
    //! use rblue::prelude::*;
    //! ```

    pub use crate::platform::{backend_version, default_adapter};
    pub use rblue_api::prelude::*;
}

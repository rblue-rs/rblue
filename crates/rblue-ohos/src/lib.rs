//! # rblue-ohos
//!
//! HarmonyOS (OHOS) backend for rblue: the Rust side of the `BleDriver`
//! protocol over the Node-API bridge.
//!
//! BLE on OHOS is only reachable from ArkTS (`@ohos.bluetooth.ble`); there is
//! no public C API a Rust crate could call. The platform is therefore served
//! by the rblue *Driver track*: an ArkTS `BleDriver` implementation drives the
//! OS BLE stack and is injected into this crate. The protocol shape is defined
//! once in [`rblue_driver`] and must not be forked per platform.
//!
//! Everything except the Node-API facade is pure Rust and fully host-testable:
//!
//! - [`DriverRegistry`] correlates requests and responses by `req_id` through
//!   a pending-request table of oneshot channels (configurable timeout,
//!   out-of-order completion, disconnect fail-fast), routes driver events into
//!   [`rblue_api::AdapterEvent`] broadcasts with snapshot-then-live
//!   subscription semantics, and decodes binary data-plane frames for hot-path
//!   payloads (notifications, read results, write data). JSON stays confined
//!   to the low-frequency control plane.
//! - [`DriverTransport`] is the injection seam between the core and the host.
//!   The ohos-rs facade wraps `ThreadsafeFunction`s behind it; tests inject an
//!   in-memory [`RecordingTransport`].
//! - [`OhosAdapter`] / [`OhosPeripheral`] implement the [`rblue_api`] traits
//!   on top of the registry.
//! - The `napi` feature (off by default, so host checks and tests never need
//!   the Node-API toolchain) adds the ohos-rs facade: `registerDriver`,
//!   `completeRequest`, `emitEvent`, `emitFrame`, and the async facade methods
//!   consumed by ArkTS.
//!
//! Target: `aarch64-unknown-linux-ohos` (arm64-v8a devices) and `x86_64`
//! (simulator). See `docs/` in this crate for the error mapping, platform
//! notes, and the driver round-trip PoC checklist that M4 executes.

#![warn(missing_docs)]

mod broadcast;
mod registry;

pub mod adapter;
pub mod capabilities;
#[cfg(feature = "napi")]
pub mod napi_facade;
pub mod peripheral;
pub mod transport;

pub use crate::adapter::OhosAdapter;
pub use crate::capabilities::ohos_capabilities;
pub use crate::peripheral::OhosPeripheral;
pub use crate::registry::{DriverRegistry, RegistryConfig};
pub use crate::transport::{DriverTransport, RecordingTransport, SentRequest};

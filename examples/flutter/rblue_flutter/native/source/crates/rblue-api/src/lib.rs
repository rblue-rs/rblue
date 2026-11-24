//! # rblue-api
//!
//! Core traits and data model of the rblue cross-platform Bluetooth Low Energy
//! library: the [`Adapter`] / [`Peripheral`] trait surface, the unified
//! [`Error`] model, and capability negotiation. Every backend (Windows,
//! macOS/iOS, Linux, Android, Web, HarmonyOS, and the in-process
//! `rblue-mock`) implements exactly this surface, and every backend must pass
//! the `contract` suite unchanged.
//!
//! ## Design constraints
//!
//! - Pure Rust: no platform crates, no async runtime, no FFI dependencies.
//! - Streams are [`futures::stream::BoxStream`] values, so the traits work
//!   under any executor (tokio on desktop/mobile, `wasm-bindgen-futures` on
//!   the web, the napi thread on OHOS).
//! - `serde` support for FFI and driver-protocol payloads is opt-in behind
//!   the `serde` feature; the default build keeps the dependency footprint
//!   minimal.
//! - This trait surface is the project contract: it is extended, never broken.
//!
//! ## Example
//!
//! The examples below take an `Arc<dyn Adapter>`; with the mock backend you
//! construct `rblue-mock`'s adapter and pass it here, so doctests and examples
//! never need Bluetooth hardware.
//!
//! ```no_run
//! use std::sync::Arc;
//! use futures::StreamExt;
//! use rblue_api::prelude::*;
//!
//! # async fn scan(adapter: Arc<dyn Adapter>) -> Result<(), Error> {
//! adapter.initialize().await?;
//! adapter.start_scan(&ScanFilter::default()).await?;
//! let mut events = adapter.events();
//! while let Some(event) = events.next().await {
//!     // Identifiers are personal data: never log them at info level or above.
//!     let _ = event;
//! }
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

pub mod adapter;
pub mod advertise;
pub mod cache;
pub mod capability;
pub mod contract;
pub mod debounce;
pub mod device;
pub mod error;
pub mod event;
#[cfg(test)]
mod fake;
pub mod gatt;
pub mod peripheral;
pub mod queue;
pub mod scan;
pub mod server;
pub mod session;
pub mod supervisor;

pub mod prelude {
    //! Single glob import for the common trait and type surface.
    //!
    //! ```
    //! use rblue_api::prelude::*;
    //! ```

    pub use crate::adapter::{Adapter, AdapterEventStream};
    pub use crate::advertise::AdvertiseParams;
    pub use crate::cache::{ServiceCache, ServiceCacheConfig, ValidationEvidence};
    pub use crate::capability::{Capability, CapabilitySet};
    pub use crate::debounce::{DebounceConfig, DebouncedEvent, ScanDebouncer};
    pub use crate::device::{AdapterId, AddressType, DeviceId, Rssi};
    pub use crate::error::{AttError, Error, PairingStatus, PermissionReason};
    pub use crate::event::{AdapterEvent, AdapterState, DisconnectReason};
    pub use crate::gatt::{
        Characteristic, CharacteristicId, CharacteristicProperties, Notification, Service,
        ServiceId, ServiceSet, WriteMode,
    };
    pub use crate::peripheral::{
        reject_invalid_att_mtu, ConnectOptions, ConnectionPriority, NotificationStream, Peripheral,
        WriteOptions, ATT_MTU_MAX, ATT_MTU_MIN,
    };
    pub use crate::queue::{GattOpPriority, GattOpQueue, GattOperation, OpHandle, QueueConfig};
    pub use crate::scan::{
        AdvertisementData, DiscoveredDevice, ManufacturerFilter, RequestDeviceOptions, ScanFilter,
    };
    pub use crate::server::{
        GattServerEvent, GattServerEventStream, LocalCharacteristic, LocalCharacteristicId,
        LocalCharacteristicSnapshot, LocalService, LocalServiceId, LocalServiceSnapshot,
    };
    pub use crate::session::{
        ReplayPolicy, RestoredSession, SavedSession, SessionRestoreConfig, SessionRestorer,
    };
    pub use crate::supervisor::{
        ConnectionSupervisor, ReconnectPolicy, SupervisorEvent, SupervisorHandle,
    };
}

pub use crate::adapter::{Adapter, AdapterEventStream};
pub use crate::advertise::AdvertiseParams;
pub use crate::cache::{ServiceCache, ServiceCacheConfig, ValidationEvidence};
pub use crate::capability::{Capability, CapabilitySet};
pub use crate::contract::{
    run_contract_suite, CONTRACT_CHARACTERISTIC_UUID, CONTRACT_SERVICE_UUID,
};
pub use crate::debounce::{DebounceConfig, DebouncedEvent, ScanDebouncer};
pub use crate::device::{AdapterId, AddressType, DeviceId, Rssi};
pub use crate::error::{AttError, Error, PairingStatus, PermissionReason};
pub use crate::event::{AdapterEvent, AdapterState, DisconnectReason};
pub use crate::gatt::{
    Characteristic, CharacteristicId, CharacteristicProperties, Notification, Service, ServiceId,
    ServiceSet, WriteMode,
};
pub use crate::peripheral::{
    reject_invalid_att_mtu, ConnectOptions, ConnectionPriority, NotificationStream, Peripheral,
    WriteOptions, ATT_MTU_MAX, ATT_MTU_MIN,
};
pub use crate::queue::{GattOpPriority, GattOpQueue, GattOperation, OpHandle, QueueConfig};
pub use crate::scan::{
    AdvertisementData, DiscoveredDevice, ManufacturerFilter, RequestDeviceOptions, ScanFilter,
};
pub use crate::server::{
    GattServerEvent, GattServerEventStream, LocalCharacteristic, LocalCharacteristicId,
    LocalCharacteristicSnapshot, LocalService, LocalServiceId, LocalServiceSnapshot,
};
pub use crate::session::{
    ReplayPolicy, RestoredSession, SavedSession, SessionRestoreConfig, SessionRestorer,
};
pub use crate::supervisor::{
    ConnectionSupervisor, ReconnectPolicy, SupervisorEvent, SupervisorHandle,
};

//! In-memory mock BLE backend for rblue.
//!
//! `rblue-mock` implements the [`rblue_api::Adapter`] /
//! [`rblue_api::Peripheral`] trait surface entirely in memory: no platform
//! stack, no Bluetooth hardware, and no async runtime dependency (futures
//! channels only). It is the zero-hardware baseline every other backend is
//! compared against and the engine for tests of the core value-add layer
//! (connection supervisor, request queueing, service caching).
//!
//! # Test control plane
//!
//! Beyond the trait surface, [`MockPeripheral`] exposes deterministic controls:
//!
//! - [Scripted advertisements](MockPeripheral::set_advertisement_script):
//!   [`AdvertisementData`](rblue_api::AdvertisementData) sequences with RSSI
//!   curves, plus appear/disappear ([`MockPeripheral::advertise`],
//!   [`MockPeripheral::disappear`]).
//! - [Forced disconnects](MockPeripheral::force_disconnect): proactively emit
//!   `AdapterEvent::DeviceDisconnected` with a chosen
//!   [`DisconnectReason`](rblue_api::DisconnectReason); subsequent GATT
//!   operations fail with `Error::NotConnected`.
//! - [Injected failures](MockPeripheral::set_failure): read/write/subscribe
//!   return the given ATT error or platform status; see
//!   `docs/error-mapping.md` in the crate root.
//! - [Scriptable GATT tree](MockPeripheral::add_service): services and
//!   characteristics with configurable properties, values, and write bounds;
//!   [`MockCharacteristic::with_echo`] implements the write-echo loop the
//!   contract fixture requires.
//! - [Manual notifications](MockPeripheral::push_notify): push values to
//!   subscribers as if the peripheral had sent them.
//! - [Connection priority](rblue_api::Peripheral::set_connection_priority):
//!   records the requested [`ConnectionPriority`](rblue_api::ConnectionPriority)
//!   once the mock session is connected.
//!
//! [`MockAdapter::with_contract_fixture`] builds the fixture the
//! `rblue_api::contract` suite requires, and the mock reports every
//! [`Capability`](rblue_api::Capability) bit, so reference flows never
//! degrade.
//!
//! # Example
//!
//! ```
//! use futures::StreamExt;
//! use rblue_api::prelude::*;
//! use rblue_mock::MockAdapter;
//!
//! # fn main() -> Result<(), rblue_api::Error> {
//! # futures::executor::block_on(async {
//! let adapter = MockAdapter::new();
//! let device = adapter.add_device(
//!     DeviceId::Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x01], AddressType::RandomStatic),
//!     "simulated-sensor",
//! );
//! device.set_advertisement_script(vec![AdvertisementData {
//!     local_name: Some("simulated-sensor".to_owned()),
//!     services: vec![uuid::Uuid::from_u128(0x180F)],
//!     rssi: Some(-40),
//!     ..AdvertisementData::default()
//! }]);
//!
//! adapter.initialize().await?;
//! let mut events = adapter.events();
//! adapter.start_scan(&ScanFilter::default()).await?;
//!
//! // The scan snapshot discovers the visible device...
//! let AdapterEvent::DeviceDiscovered(discovered) =
//!     events.next().await.expect("discovery")
//! else {
//!     panic!("expected a discovery");
//! };
//! assert_eq!(discovered.id, *device.id());
//!
//! // ...and the scripted step shows up as an update with the new RSSI.
//! assert!(device.advance_script());
//! let AdapterEvent::DeviceUpdated(_, rssi) = events.next().await.expect("update")
//! else {
//!     panic!("expected an update");
//! };
//! assert_eq!(rssi.as_i16(), -40);
//!
//! adapter.stop_scan().await?;
//! # Ok::<(), rblue_api::Error>(())
//! # })?;
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

mod adapter;
mod hub;
mod peripheral;

pub use crate::adapter::MockAdapter;
pub use crate::peripheral::{MockCharacteristic, MockFailure, MockOp, MockPeripheral, MockService};

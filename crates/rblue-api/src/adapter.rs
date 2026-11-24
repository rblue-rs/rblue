//! Adapter trait: central role plus optional peripheral advertising and GATT
//! server methods, gated by [`crate::Capability::Advertise`] and
//! [`crate::Capability::GattServer`].

use std::sync::Arc;

use futures::stream::BoxStream;

use crate::advertise::AdvertiseParams;
use crate::device::AdapterId;
use crate::event::{AdapterEvent, AdapterState};
use crate::peripheral::Peripheral;
use crate::scan::{RequestDeviceOptions, ScanFilter};
use crate::server::{
    GattServerEventStream, LocalCharacteristicId, LocalService, LocalServiceId,
    LocalServiceSnapshot,
};
use crate::{CapabilitySet, DeviceId, Error};

/// Stream of [`AdapterEvent`] values.
///
/// Subscription semantics: subscribing snapshots the device list known so far
/// (each device is delivered as a fresh `AdapterEvent::DeviceDiscovered`) and
/// then continues with live events. A subscription opened mid-scan therefore
/// still sees everything the adapter knows.
///
/// The stream is runtime-agnostic (`futures::stream::BoxStream`); dropping it
/// unsubscribes. Delivery is best-effort broadcast: a slow consumer may miss
/// intermediate `DeviceUpdated` values, never terminal ones such as
/// `DeviceDisconnected`.
pub type AdapterEventStream = BoxStream<'static, AdapterEvent>;

/// A Bluetooth adapter.
///
/// Central (GATT client) methods are always present. Peripheral advertising
/// and GATT server methods have default implementations that return
/// [`Error::Unsupported`]; backends implement them only when they report
/// [`crate::Capability::Advertise`] or [`crate::Capability::GattServer`].
///
/// 1.0 binds only the system default adapter; [`AdapterId`] is the reserved
/// seam for multi-adapter enumeration post-1.0. All methods on
/// `Arc<dyn Adapter>` are safe to call concurrently; backends serialize GATT
/// procedures per connection internally.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use futures::StreamExt;
/// use rblue_api::prelude::*;
///
/// # async fn first_discovered(adapter: Arc<dyn Adapter>) -> Result<Option<DeviceId>, Error> {
/// adapter.initialize().await?;
/// adapter.start_scan(&ScanFilter::default()).await?;
/// let mut events = adapter.events();
/// let discovered = events
///     .next()
///     .await
///     .map(|event| match event {
///         AdapterEvent::DeviceDiscovered(device) => Some(device.id),
///         _ => None,
///     })
///     .flatten();
/// adapter.stop_scan().await?;
/// # Ok(discovered)
/// # }
/// ```
#[async_trait::async_trait]
pub trait Adapter: Send + Sync {
    /// Identity of this adapter. Always [`AdapterId::default_adapter`] on 1.0.
    fn id(&self) -> &AdapterId;

    /// Capabilities of this backend: the static per-platform table refined by
    /// runtime probes (e.g. web `getAvailability`). Callers must gate
    /// optional features on this instead of catching
    /// [`Error::Unsupported`] after the fact.
    fn capabilities(&self) -> CapabilitySet;

    /// Opens or wakes the adapter.
    ///
    /// On some platforms the permission prompt happens here (e.g. the
    /// CoreBluetooth authorization callback, the Android runtime permission
    /// window). Resolves once the adapter state is known.
    ///
    /// # Errors
    ///
    /// [`Error::Permission`] when the user or the OS denies access,
    /// [`Error::Unsupported`] when the platform has no Bluetooth support at
    /// all.
    async fn initialize(&self) -> Result<(), Error>;

    /// Queries the current radio/permission state of this adapter.
    ///
    /// The method name is `state` (not `power_state`) because
    /// [`AdapterState`] also includes `Unauthorized` / `Unavailable` /
    /// `Unknown`. This query does not require
    /// [`crate::Capability::AdapterPowerControl`]; that bit only covers
    /// [`Adapter::set_powered`]. Live transitions continue to arrive as
    /// [`AdapterEvent::StateChanged`].
    ///
    /// The default reports [`AdapterState::Unknown`]. Backends that can
    /// read the radio or authorization state must override this.
    async fn state(&self) -> Result<AdapterState, Error> {
        Ok(AdapterState::Unknown)
    }

    /// Turns the adapter radio on (`true`) or off (`false`).
    ///
    /// Callers should check [`crate::Capability::AdapterPowerControl`] first.
    /// Platforms with no programmatic switch, or whose OS refuses to let the
    /// application toggle Bluetooth, omit the capability and return
    /// [`Error::Unsupported`] with `feature: "set_powered"` (or
    /// [`Error::Permission`] when that is a better fit). Successful
    /// transitions are also delivered as [`AdapterEvent::StateChanged`].
    ///
    /// Do not use `turn_on` / `turn_off` as public names.
    async fn set_powered(&self, _powered: bool) -> Result<(), Error> {
        Err(Error::Unsupported {
            platform: std::env::consts::OS,
            feature: "set_powered",
        })
    }

    /// Starts scanning with `filter`.
    ///
    /// Scanning continues until [`Adapter::stop_scan`] is called; every
    /// sighting is delivered as an `AdapterEvent::DeviceDiscovered` or
    /// `AdapterEvent::DeviceUpdated` on the [`Adapter::events`] stream.
    /// Calling `start_scan` while a scan is running replaces the filter.
    ///
    /// # Errors
    ///
    /// [`Error::Permission`] when the required scan permission is missing
    /// (location on Android API <= 30, Bluetooth runtime permissions on API
    /// 31+), [`Error::Unsupported`] where background scanning does not exist
    /// (web).
    async fn start_scan(&self, filter: &ScanFilter) -> Result<(), Error>;

    /// Stops the active scan.
    ///
    /// Events already queued (devices seen before the stop) may still be
    /// delivered afterwards.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidState`] when no scan is running, platform failures as
    /// [`Error::Platform`].
    async fn stop_scan(&self) -> Result<(), Error>;

    /// Subscribes to the adapter event stream; see [`AdapterEventStream`] for
    /// the snapshot-then-live subscription semantics.
    fn events(&self) -> AdapterEventStream;

    /// Snapshot of all devices known to this adapter: everything from the
    /// scan history plus connected devices.
    ///
    /// # Errors
    ///
    /// [`Error::NotConnected`] if the adapter was closed, platform failures
    /// as [`Error::Platform`].
    async fn peripherals(&self) -> Result<Vec<Arc<dyn Peripheral>>, Error>;

    /// Retrieves system-connected BLE peripherals matching any of `services`.
    ///
    /// `services` must be nonempty: an empty list returns
    /// [`Error::InvalidState`]. The result is the set of BLE peripherals the
    /// operating system already considers connected that match any given
    /// service UUID. The caller still needs to [`Peripheral::connect`]
    /// locally before GATT operations.
    ///
    /// Backends that cannot honor the lookup return [`Error::Unsupported`]
    /// with `feature: "system_connected_peripherals"`. Check
    /// [`crate::Capability::SystemConnectedPeripherals`] first when the caller
    /// wants to degrade gracefully.
    async fn system_connected_peripherals(
        &self,
        services: &[uuid::Uuid],
    ) -> Result<Vec<Arc<dyn Peripheral>>, Error> {
        if services.is_empty() {
            return Err(Error::InvalidState(
                "at least one service UUID is required".into(),
            ));
        }
        Err(Error::Unsupported {
            platform: std::env::consts::OS,
            feature: "system_connected_peripherals",
        })
    }

    /// Returns the peripheral handle for a known device.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] when the device was never seen (or its scan cache
    /// entry expired), platform failures as [`Error::Platform`].
    async fn peripheral(&self, id: &DeviceId) -> Result<Arc<dyn Peripheral>, Error>;

    /// Web-only entry point: pops the `navigator.bluetooth` device picker.
    /// Every other platform returns [`Error::Unsupported`].
    ///
    /// On the web this must run inside a user gesture (click/touch handler) -
    /// the facade layer (Kotlin/Swift/TS) is responsible for marking the call
    /// site accordingly.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] off the web, [`Error::Permission`] when the
    /// picker was dismissed or blocked by policy.
    async fn request_device(
        &self,
        opts: &RequestDeviceOptions,
    ) -> Result<Arc<dyn Peripheral>, Error>;

    /// Starts advertising as a peripheral with `params`.
    ///
    /// Advertising continues until [`Adapter::stop_advertising`] or the
    /// optional [`AdvertiseParams::timeout`] elapses. Calling
    /// `start_advertising` while already advertising replaces the payload
    /// (the same replacement rule as [`Adapter::start_scan`]).
    ///
    /// Check [`crate::Capability::Advertise`] first. Backends that do not
    /// implement peripheral advertising return [`Error::Unsupported`] with
    /// `feature: "start_advertising"`.
    ///
    /// Do not use `startAdvertising` as a public name.
    async fn start_advertising(&self, _params: &AdvertiseParams) -> Result<(), Error> {
        Err(unsupported("start_advertising"))
    }

    /// Stops an active advertisement.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidState`] when no advertisement is running,
    /// [`Error::Unsupported`] with `feature: "stop_advertising"` when the
    /// backend does not implement peripheral advertising.
    ///
    /// Do not use `stopAdvertising` as a public name.
    async fn stop_advertising(&self) -> Result<(), Error> {
        Err(unsupported("stop_advertising"))
    }

    /// Returns whether this adapter is currently advertising.
    ///
    /// Backends that do not implement peripheral advertising return
    /// [`Error::Unsupported`] with `feature: "is_advertising"`.
    async fn is_advertising(&self) -> Result<bool, Error> {
        Err(unsupported("is_advertising"))
    }

    /// Registers a local GATT service and returns its session-scoped identity.
    ///
    /// Check [`crate::Capability::GattServer`] first. Backends that do not
    /// implement a GATT server return [`Error::Unsupported`] with
    /// `feature: "add_local_service"`.
    ///
    /// Do not use `addService` / `startGattServer` as public names.
    async fn add_local_service(&self, _service: &LocalService) -> Result<LocalServiceId, Error> {
        Err(unsupported("add_local_service"))
    }

    /// Unregisters a previously added local service.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidState`] when `id` is not registered,
    /// [`Error::Unsupported`] with `feature: "remove_local_service"` when the
    /// backend does not implement a GATT server.
    async fn remove_local_service(&self, _id: &LocalServiceId) -> Result<(), Error> {
        Err(unsupported("remove_local_service"))
    }

    /// Snapshot of every currently registered local service, including stored
    /// characteristic values.
    ///
    /// Backends that do not implement a GATT server return
    /// [`Error::Unsupported`] with `feature: "local_services"`.
    async fn local_services(&self) -> Result<Vec<LocalServiceSnapshot>, Error> {
        Err(unsupported("local_services"))
    }

    /// Replaces the stored value of a local characteristic without notifying
    /// subscribers.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidState`] when `id` is not registered,
    /// [`Error::InvalidLength`] when `value` is longer than the characteristic
    /// bound, [`Error::Unsupported`] with
    /// `feature: "write_local_characteristic"` when the backend does not
    /// implement a GATT server.
    async fn write_local_characteristic(
        &self,
        _id: &LocalCharacteristicId,
        _value: &[u8],
    ) -> Result<(), Error> {
        Err(unsupported("write_local_characteristic"))
    }

    /// Replaces the stored value of a local characteristic and notifies or
    /// indicates every current subscriber.
    ///
    /// Having no subscribers is not an error (`Ok`). The characteristic must
    /// have the `NOTIFY` or `INDICATE` property.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidState`] when `id` is not registered or the
    /// characteristic cannot notify, [`Error::InvalidLength`] when `value` is
    /// longer than the characteristic bound, [`Error::Unsupported`] with
    /// `feature: "notify_local_characteristic"` when the backend does not
    /// implement a GATT server.
    async fn notify_local_characteristic(
        &self,
        _id: &LocalCharacteristicId,
        _value: &[u8],
    ) -> Result<(), Error> {
        Err(unsupported("notify_local_characteristic"))
    }

    /// Subscribes to GATT server events; see [`GattServerEventStream`] for
    /// snapshot-then-live semantics.
    ///
    /// Backends that do not implement a GATT server return
    /// [`Error::Unsupported`] with `feature: "server_events"`.
    fn server_events(&self) -> Result<GattServerEventStream, Error> {
        Err(unsupported("server_events"))
    }
}

/// Builds an [`Error::Unsupported`] for `feature` on the current OS.
pub(crate) fn unsupported(feature: &'static str) -> Error {
    Error::Unsupported {
        platform: std::env::consts::OS,
        feature,
    }
}

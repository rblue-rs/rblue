//! Central-role adapter trait.

use std::sync::Arc;

use futures::stream::BoxStream;

use crate::device::AdapterId;
use crate::event::AdapterEvent;
use crate::peripheral::Peripheral;
use crate::scan::{RequestDeviceOptions, ScanFilter};
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

/// A Bluetooth adapter in the central role.
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
    /// [`Capability::SystemConnectedPeripherals`] first when the caller
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
}

//! Connected-peripheral trait.

use futures::stream::BoxStream;

use crate::device::DeviceId;
use crate::error::Error;
use crate::gatt::{CharacteristicId, Notification, ServiceSet, WriteMode};

/// Stream of GATT notifications and indications from one connected
/// peripheral.
///
/// Semantics: broadcast - every subscriber receives every value, payload is
/// delivered only after the corresponding [`Peripheral::subscribe`]. The
/// [`Notification::id`] field routes values to the right characteristic.
/// Buffering/drop policy under backpressure is a backend detail; the core
/// value-add layer layers configurable buffering on top.
pub type NotificationStream = BoxStream<'static, Notification>;

/// Preferred connection interval / latency trade-off for an already
/// connected peripheral.
///
/// The public variants stay [`Low`](ConnectionPriority::Low),
/// [`Balanced`](ConnectionPriority::Balanced), and
/// [`High`](ConnectionPriority::High). Backends map them onto platform
/// presets (on Android: `CONNECTION_PRIORITY_LOW_POWER` / `BALANCED` /
/// `HIGH`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum ConnectionPriority {
    /// Lower power, higher latency.
    Low,
    /// Default balance of throughput and power.
    Balanced,
    /// Higher throughput, lower latency; typically used temporarily for
    /// bulk transfers.
    High,
}

/// A GATT client session with one peripheral.
///
/// Obtained from [`crate::Adapter::peripheral`] /
/// [`crate::Adapter::request_device`]. All methods are safe to call
/// concurrently; backends serialize operations per connection (the core adds
/// an explicit priority queue above them).
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use futures::StreamExt;
/// use rblue_api::prelude::*;
///
/// # async fn echo_roundtrip(
/// #     peripheral: Arc<dyn Peripheral>,
/// #     id: CharacteristicId,
/// # ) -> Result<Option<Notification>, Error> {
/// peripheral.connect().await?;
/// let services = peripheral.discover_services(&[]).await?;
/// let _ = &services;
/// peripheral.subscribe(&id).await?;
/// let mut notifications = peripheral.notifications();
/// peripheral.write(&id, b"ping", WriteMode::WithResponse).await?;
/// let first = notifications.next().await;
/// peripheral.unsubscribe(&id).await?;
/// # Ok(first)
/// # }
/// ```
#[async_trait::async_trait]
pub trait Peripheral: Send + Sync {
    /// Identity of this peripheral, stable for the adapter's lifetime (see
    /// [`DeviceId`] caveats about RPA rotation and reinstall-sensitive
    /// UUIDs).
    fn id(&self) -> &DeviceId;

    /// Advertised name of the device, when known. May be `None` before the
    /// first advertisement or when the platform does not cache names.
    fn name(&self) -> Option<String>;

    /// Connection state.
    ///
    /// # Errors
    ///
    /// Platform failures as [`Error::Platform`].
    async fn is_connected(&self) -> Result<bool, Error>;

    /// Connects (or re-connects) to the peripheral.
    ///
    /// Resolves once the link is up; the GATT tree is *not* discovered here -
    /// call [`Peripheral::discover_services`] next. Directly connecting to an
    /// address the adapter has not observed may be impossible (Windows only
    /// connects to watcher-observed RPA devices); re-scan first.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] when the device is unknown, [`Error::Timeout`] on
    /// connection timeout, [`Error::Permission`] when
    /// `BLUETOOTH_CONNECT`-style permission is missing,
    /// `Error::Platform{code}` with raw status codes such as Android GATT
    /// status 133.
    async fn connect(&self) -> Result<(), Error>;

    /// Disconnects from the peripheral. Idempotent: disconnecting an already
    /// disconnected peripheral succeeds.
    ///
    /// # Errors
    ///
    /// Platform failures as [`Error::Platform`].
    async fn disconnect(&self) -> Result<(), Error>;

    /// Discovers the GATT tree. Must be called on every connection: iOS
    /// CoreBluetooth requires it, and Android's stack cache may be stale
    /// after a firmware GATT-table change (the backend exposes a
    /// forced-refresh option for that case).
    ///
    /// # Arguments
    ///
    /// * `filter` - restrict discovery to these service UUIDs. An empty slice
    ///   discovers everything. Services missing after a filtered discovery
    ///   need a full (empty-filter) rediscovery.
    ///
    /// # Errors
    ///
    /// [`Error::NotConnected`] when called before connecting,
    /// [`Error::Disconnected`] when the link drops mid-discovery,
    /// [`Error::Att`] on ATT-level failures.
    ///
    /// The returned identifiers are valid only for this connection.
    async fn discover_services(&self, filter: &[uuid::Uuid]) -> Result<ServiceSet, Error>;

    /// Reads the current value of a characteristic.
    ///
    /// # Errors
    ///
    /// [`Error::NotConnected`], [`Error::Att`] when the read is not permitted
    /// or the procedure fails, [`Error::Disconnected`] when the link drops
    /// mid-read.
    async fn read(&self, ch: &CharacteristicId) -> Result<Vec<u8>, Error>;

    /// Writes `data` to a characteristic.
    ///
    /// Unified write semantics: when `data` exceeds
    /// [`Peripheral::max_write_len`] this returns [`Error::InvalidLength`] -
    /// values are never silently truncated (iOS native behavior) and never
    /// hiddenly chunked (the web does not chunk either).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidLength`] for oversize payloads, [`Error::NotConnected`],
    /// [`Error::Att`] for ATT-level failures (e.g. write not permitted).
    async fn write(&self, ch: &CharacteristicId, data: &[u8], mode: WriteMode)
        -> Result<(), Error>;

    /// Maximum payload accepted by [`Peripheral::write`] for this
    /// characteristic and mode, in bytes.
    ///
    /// Maps to iOS `maximumWriteValueLength(for:)`, Android `mtu - 3`, Windows
    /// `MaxPduSize - 3`, BlueZ `MTU - 3`; the web reports a conservative 20
    /// (default ATT MTU 23 minus 3). Because the MTU can change, callers
    /// should re-query before long transfers instead of caching.
    ///
    /// # Errors
    ///
    /// [`Error::NotConnected`], platform failures as [`Error::Platform`].
    async fn max_write_len(&self, ch: &CharacteristicId, mode: WriteMode) -> Result<usize, Error>;

    /// Subscribes to value changes of a characteristic.
    ///
    /// Whether a notification or an indication is used follows the
    /// characteristic properties (`NOTIFY`/`INDICATE`), which is exactly the
    /// distinction OHOS backends must honor when choosing between
    /// `setCharacteristicChangeNotification` and
    /// `setCharacteristicChangeIndication`. Values are delivered on the
    /// [`Peripheral::notifications`] stream. Subscribing twice succeeds
    /// idempotently.
    ///
    /// # Errors
    ///
    /// [`Error::NotConnected`], [`Error::Att`] when the CCCD write fails,
    /// [`Error::InvalidState`] when the characteristic has neither `NOTIFY`
    /// nor `INDICATE`.
    async fn subscribe(&self, ch: &CharacteristicId) -> Result<(), Error>;

    /// Cancels a subscription. Idempotent.
    ///
    /// # Errors
    ///
    /// [`Error::NotConnected`], platform failures as [`Error::Platform`].
    async fn unsubscribe(&self, ch: &CharacteristicId) -> Result<(), Error>;

    /// Broadcast stream of notification/indication values; see
    /// [`NotificationStream`] for semantics. Values flow only for
    /// characteristics subscribed via [`Peripheral::subscribe`].
    fn notifications(&self) -> NotificationStream;

    /// Actively initiates pairing/bonding. 1.0 placeholder:
    /// iOS/macOS pair implicitly when an encrypted characteristic is accessed
    /// and return [`Error::Unsupported`] here; Windows/BlueZ/Android have
    /// native APIs and implement this per the capability table
    /// (`Capability::Pairing`).
    ///
    /// # Errors
    ///
    /// [`Error::Pairing`] with the normalized failure status,
    /// [`Error::Unsupported`] on platforms without an active-pairing API.
    async fn pair(&self) -> Result<(), Error>;

    /// Reads the RSSI of the connected peripheral.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] where RSSI is scan-time only (Windows/Linux),
    /// [`Error::NotConnected`] when disconnected.
    async fn rssi(&self) -> Result<i16, Error>;

    /// Current ATT MTU.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] where the platform does not expose the value
    /// (iOS: system-managed, web); use [`Peripheral::max_write_len`] instead.
    async fn mtu(&self) -> Result<u16, Error>;

    /// Requests a connection-priority update for this peripheral.
    ///
    /// This is a request: the remote device or the controller may ignore or
    /// renegotiate the parameters. Callers should check
    /// [`crate::Capability::ConnectionPriority`] first.
    ///
    /// # Errors
    ///
    /// [`Error::NotConnected`] when the local session is down,
    /// [`Error::Unsupported`] with `feature: "set_connection_priority"` on
    /// platforms that have no stable equivalent API.
    async fn set_connection_priority(&self, _priority: ConnectionPriority) -> Result<(), Error> {
        if !self.is_connected().await? {
            return Err(Error::NotConnected);
        }
        Err(Error::Unsupported {
            platform: std::env::consts::OS,
            feature: "set_connection_priority",
        })
    }
}

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

/// Options for [`Peripheral::connect_with`].
///
/// `auto_connect` is the **platform/stack** background-connection flag
/// (Android `connectGatt(autoConnect=true)` and equivalents). It is not
/// [`crate::SupervisorHandle::auto_connect`], which is the application-level
/// reconnect supervisor (backoff, GATT 133 breaker, [`crate::ReconnectPolicy`]).
///
/// `mtu` reserves the connect-time ATT MTU request from the `request_mtu`
/// issue. Backends that do not yet implement an explicit MTU request return
/// [`Error::Unsupported`] with `feature: "request_mtu"` when this is `Some`.
///
/// [`Peripheral::connect`] is equivalent to
/// `connect_with(&ConnectOptions::default())` (`auto_connect: false`,
/// `mtu: None`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ConnectOptions {
    /// When `true`, request the platform's background/auto connection
    /// behaviour. When `false`, perform a direct connection attempt.
    ///
    /// Callers should check [`crate::Capability::AutoConnect`] first. A
    /// backend without a stack-level auto-connect API returns
    /// [`Error::Unsupported`] with `feature: "auto_connect"` if this is
    /// `true`; `false` must not fail for that reason.
    pub auto_connect: bool,
    /// Optional ATT MTU to request as part of the connection. `None`
    /// leaves MTU negotiation to the backend default.
    pub mtu: Option<u16>,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        Self {
            auto_connect: false,
            mtu: None,
        }
    }
}

impl ConnectOptions {
    /// Returns `true` when these options are equivalent to
    /// [`Peripheral::connect`] (no platform auto-connect, no explicit MTU).
    #[must_use]
    pub const fn is_default(&self) -> bool {
        !self.auto_connect && self.mtu.is_none()
    }

    /// Returns [`Error::Unsupported`] for any option this backend does not
    /// implement; `Ok(())` when [`Self::is_default`] holds.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] with `feature: "auto_connect"` or
    /// `feature: "request_mtu"`.
    pub fn reject_unsupported(&self, platform: &'static str) -> Result<(), Error> {
        if self.auto_connect {
            return Err(Error::Unsupported {
                platform,
                feature: "auto_connect",
            });
        }
        if self.mtu.is_some() {
            return Err(Error::Unsupported {
                platform,
                feature: "request_mtu",
            });
        }
        Ok(())
    }
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
    /// Equivalent to [`Peripheral::connect_with`] with
    /// [`ConnectOptions::default()`] (`auto_connect: false`, `mtu: None`).
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

    /// Connects (or re-connects) to the peripheral with [`ConnectOptions`].
    ///
    /// `opts.auto_connect` selects the platform/stack background connection
    /// flag, not the reconnect supervisor. Use
    /// [`crate::SupervisorHandle::auto_connect`] for supervised retries.
    ///
    /// The default implementation calls [`Peripheral::connect`] when
    /// [`ConnectOptions::is_default`] is true, and otherwise returns
    /// [`Error::Unsupported`] for `auto_connect` or `request_mtu`. Backends
    /// that support a flag override this method and should still reject
    /// options they do not implement rather than ignoring them.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] when a requested option is not available on
    /// this backend, plus the same failures as [`Peripheral::connect`].
    async fn connect_with(&self, opts: &ConnectOptions) -> Result<(), Error> {
        if opts.is_default() {
            return self.connect().await;
        }
        opts.reject_unsupported("unknown")
    }

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{AddressType, DeviceId};
    use crate::gatt::ServiceSet;
    use futures::{FutureExt, StreamExt};

    struct Stub;

    #[async_trait::async_trait]
    impl Peripheral for Stub {
        fn id(&self) -> &DeviceId {
            static ID: std::sync::OnceLock<DeviceId> = std::sync::OnceLock::new();
            ID.get_or_init(|| DeviceId::Mac([0; 6], AddressType::Public))
        }
        fn name(&self) -> Option<String> {
            None
        }
        async fn is_connected(&self) -> Result<bool, Error> {
            Ok(false)
        }
        async fn connect(&self) -> Result<(), Error> {
            Ok(())
        }
        async fn disconnect(&self) -> Result<(), Error> {
            Ok(())
        }
        async fn discover_services(&self, _filter: &[uuid::Uuid]) -> Result<ServiceSet, Error> {
            Ok(ServiceSet::default())
        }
        async fn read(&self, _ch: &CharacteristicId) -> Result<Vec<u8>, Error> {
            Ok(Vec::new())
        }
        async fn write(
            &self,
            _ch: &CharacteristicId,
            _data: &[u8],
            _mode: WriteMode,
        ) -> Result<(), Error> {
            Ok(())
        }
        async fn max_write_len(
            &self,
            _ch: &CharacteristicId,
            _mode: WriteMode,
        ) -> Result<usize, Error> {
            Ok(20)
        }
        async fn subscribe(&self, _ch: &CharacteristicId) -> Result<(), Error> {
            Ok(())
        }
        async fn unsubscribe(&self, _ch: &CharacteristicId) -> Result<(), Error> {
            Ok(())
        }
        fn notifications(&self) -> NotificationStream {
            futures::stream::empty().boxed()
        }
        async fn pair(&self) -> Result<(), Error> {
            Ok(())
        }
        async fn rssi(&self) -> Result<i16, Error> {
            Ok(0)
        }
        async fn mtu(&self) -> Result<u16, Error> {
            Ok(23)
        }
    }

    #[test]
    fn default_options_are_direct_connect() {
        let opts = ConnectOptions::default();
        assert!(!opts.auto_connect);
        assert_eq!(opts.mtu, None);
        assert!(opts.is_default());
    }

    #[test]
    fn default_connect_with_delegates_to_connect() {
        let result = Stub.connect_with(&ConnectOptions::default()).now_or_never();
        assert!(matches!(result, Some(Ok(()))));
    }

    #[test]
    fn connect_with_rejects_auto_connect_without_backend_support() {
        let opts = ConnectOptions {
            auto_connect: true,
            mtu: None,
        };
        let result = Stub.connect_with(&opts).now_or_never();
        assert!(
            matches!(
                result,
                Some(Err(Error::Unsupported {
                    platform: "unknown",
                    feature: "auto_connect",
                }))
            ),
            "{result:?}"
        );
    }

    #[test]
    fn connect_with_rejects_mtu_until_request_mtu_is_implemented() {
        let opts = ConnectOptions {
            auto_connect: false,
            mtu: Some(512),
        };
        let result = Stub.connect_with(&opts).now_or_never();
        assert!(
            matches!(
                result,
                Some(Err(Error::Unsupported {
                    platform: "unknown",
                    feature: "request_mtu",
                }))
            ),
            "{result:?}"
        );
    }
}

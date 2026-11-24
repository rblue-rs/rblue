//! Connected-peripheral trait.

use futures::stream::BoxStream;

use crate::device::DeviceId;
use crate::error::Error;
use crate::gatt::{CharacteristicId, DescriptorId, Notification, ServiceSet, WriteMode};

/// Stream of GATT notifications and indications from one connected
/// peripheral.
///
/// Semantics: broadcast - every subscriber receives every value, payload is
/// delivered only after the corresponding [`Peripheral::subscribe`]. The
/// [`Notification::id`] field routes values to the right characteristic.
/// Buffering/drop policy under backpressure is a backend detail; the core
/// value-add layer layers configurable buffering on top.
pub type NotificationStream = BoxStream<'static, Notification>;

/// Smallest ATT MTU defined by the Bluetooth Core Specification.
pub const ATT_MTU_MIN: u16 = 23;
/// Largest ATT MTU defined by the Bluetooth Core Specification.
pub const ATT_MTU_MAX: u16 = 517;

/// Returns [`Error::InvalidState`] when `mtu` is outside
/// [`ATT_MTU_MIN`]..=[`ATT_MTU_MAX`].
///
/// # Errors
///
/// [`Error::InvalidState`] when the value is below 23 or above 517.
pub fn reject_invalid_att_mtu(mtu: u16) -> Result<(), Error> {
    if (ATT_MTU_MIN..=ATT_MTU_MAX).contains(&mtu) {
        Ok(())
    } else {
        Err(Error::InvalidState(format!(
            "ATT MTU {mtu} is outside the valid range {ATT_MTU_MIN}..={ATT_MTU_MAX}"
        )))
    }
}

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

/// Options for [`Peripheral::connect_with`].
///
/// `auto_connect` is the **platform/stack** background-connection flag
/// (Android `connectGatt(autoConnect=true)` and equivalents). It is not
/// [`crate::SupervisorHandle::auto_connect`], which is the application-level
/// reconnect supervisor (backoff, GATT 133 breaker, [`crate::ReconnectPolicy`]).
///
/// `mtu` is an optional ATT MTU to request after the link is up, gated by
/// [`crate::Capability::MtuNegotiation`]. `Some(n)` requests `n` on backends
/// that implement [`Peripheral::request_mtu`]; backends without a request
/// API return [`Error::Unsupported`] with `feature: "request_mtu"`. `None`
/// leaves negotiation to the backend default and must not fail for lack of
/// a request API.
///
/// [`Peripheral::connect`] is equivalent to
/// `connect_with(&ConnectOptions::default())` (`auto_connect: false`,
/// `mtu: None`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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

/// Options for [`Peripheral::write_with`].
///
/// `allow_long_write` enables sequential, MTU-sized writes when `data`
/// exceeds [`Peripheral::max_write_len`]. This is an option on
/// [`Peripheral::write_with`], not a second public method name such as
/// `mtu_split` or `chunked_write`.
///
/// [`Peripheral::write`] keeps the unified default: oversize payloads
/// return [`Error::InvalidLength`] and are never silently truncated or
/// chunked.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct WriteOptions {
    /// When `true`, payloads longer than [`Peripheral::max_write_len`] are
    /// written as sequential chunks of that length. When `false`, oversize
    /// payloads fail with [`Error::InvalidLength`].
    pub allow_long_write: bool,
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
    /// that implement a flag or [`Peripheral::request_mtu`] override this
    /// method and should still reject options they do not implement rather
    /// than ignoring them.
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

    /// Writes `data` with [`WriteOptions`].
    ///
    /// When [`WriteOptions::allow_long_write`] is `false`, this is
    /// equivalent to [`Peripheral::write`] (oversize payloads still return
    /// [`Error::InvalidLength`]).
    ///
    /// When it is `true` and `data` is longer than the current
    /// [`Peripheral::max_write_len`], the payload is written as sequential
    /// chunks. `max_write_len` is re-queried before every chunk because the
    /// ATT MTU may change. A failed chunk stops the transfer and is
    /// returned; earlier chunks are not rolled back. The last chunk is the
    /// remainder of `data` and is never truncated.
    ///
    /// Backends that cannot reliably chunk the chosen [`WriteMode`] should
    /// override this method and return [`Error::Unsupported`] with
    /// `feature: "allow_long_write"` when the option is `true`; the default
    /// [`Peripheral::write`] path must still return [`Error::InvalidLength`].
    ///
    /// # Errors
    ///
    /// The same failures as [`Peripheral::write`], plus
    /// [`Error::Unsupported`] when a backend cannot honor long writes.
    async fn write_with(
        &self,
        ch: &CharacteristicId,
        data: &[u8],
        mode: WriteMode,
        opts: &WriteOptions,
    ) -> Result<(), Error> {
        if !opts.allow_long_write {
            return self.write(ch, data, mode).await;
        }
        let mut offset = 0;
        while offset < data.len() {
            let max = self.max_write_len(ch, mode).await?;
            if max == 0 {
                return Err(Error::InvalidLength {
                    len: data.len() - offset,
                    max: 0,
                });
            }
            let end = offset.saturating_add(max).min(data.len());
            self.write(ch, &data[offset..end], mode).await?;
            offset = end;
        }
        Ok(())
    }

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

    /// Reads the current value of a descriptor.
    ///
    /// Descriptor read/write is part of [`crate::Capability::GattClient`]; there
    /// is no separate capability bit. A missing descriptor returns
    /// [`Error::NotFound`] or [`Error::Att`]. A backend with no descriptor
    /// API returns [`Error::Unsupported`] with `feature: "read_descriptor"`.
    ///
    /// # Errors
    ///
    /// [`Error::NotConnected`], [`Error::NotFound`] or [`Error::Att`] when
    /// the descriptor is absent or the procedure fails,
    /// [`Error::Unsupported`] when the backend has no descriptor API,
    /// [`Error::Disconnected`] when the link drops mid-read.
    async fn read_descriptor(&self, id: &DescriptorId) -> Result<Vec<u8>, Error> {
        let _ = id;
        Err(Error::Unsupported {
            platform: "unknown",
            feature: "read_descriptor",
        })
    }

    /// Writes `data` to a descriptor.
    ///
    /// Unified write semantics match [`Peripheral::write`]: when `data`
    /// exceeds the current maximum write length this returns
    /// [`Error::InvalidLength`] - values are never silently truncated or
    /// chunked. Long writes via [`WriteOptions::allow_long_write`] apply to
    /// characteristic writes ([`Peripheral::write_with`]) only; this method
    /// has no options argument.
    ///
    /// A missing descriptor returns [`Error::NotFound`] or [`Error::Att`].
    /// A backend with no descriptor API returns [`Error::Unsupported`] with
    /// `feature: "write_descriptor"`.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidLength`] for oversize payloads, [`Error::NotConnected`],
    /// [`Error::NotFound`] or [`Error::Att`] when the descriptor is absent
    /// or the procedure fails, [`Error::Unsupported`] when the backend has
    /// no descriptor API.
    async fn write_descriptor(&self, id: &DescriptorId, data: &[u8]) -> Result<(), Error> {
        let _ = (id, data);
        Err(Error::Unsupported {
            platform: "unknown",
            feature: "write_descriptor",
        })
    }

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

    /// Actively initiates pairing/bonding.
    ///
    /// Gated by [`crate::Capability::Pairing`]. iOS/macOS/Web pair
    /// implicitly when an encrypted characteristic is accessed and return
    /// [`Error::Unsupported`] with `feature: "pair"` here. Android and
    /// Linux implement this when the capability is reported.
    ///
    /// Already-bonded devices succeed without starting a new procedure.
    ///
    /// # Errors
    ///
    /// [`Error::Pairing`] with the normalized failure status,
    /// [`Error::Unsupported`] on platforms without an active-pairing API.
    async fn pair(&self) -> Result<(), Error>;

    /// Removes an existing bond with this peripheral.
    ///
    /// Gated by [`crate::Capability::Pairing`]. Platforms without an
    /// active-unpair API return [`Error::Unsupported`] with
    /// `feature: "unpair"`. A device that is not bonded succeeds without
    /// doing work.
    ///
    /// # Errors
    ///
    /// [`Error::Pairing`] with the normalized failure status,
    /// [`Error::Unsupported`] on platforms without an active-unpair API.
    async fn unpair(&self) -> Result<(), Error> {
        Err(Error::Unsupported {
            platform: "unknown",
            feature: "unpair",
        })
    }

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

    /// Requests a new ATT MTU and returns the value actually negotiated.
    ///
    /// Gated by [`crate::Capability::MtuNegotiation`]. Platforms that
    /// negotiate automatically and have no request API (Windows, Apple,
    /// Linux, Web) return [`Error::Unsupported`] with
    /// `feature: "request_mtu"`. `mtu` must be in
    /// [`ATT_MTU_MIN`]..=[`ATT_MTU_MAX`]; the peer may still negotiate a
    /// smaller value than requested.
    ///
    /// # Errors
    ///
    /// [`Error::NotConnected`] when called while disconnected,
    /// [`Error::InvalidState`] when `mtu` is out of range,
    /// [`Error::Unsupported`] when the backend has no request API, or a
    /// platform failure.
    async fn request_mtu(&self, mtu: u16) -> Result<u16, Error> {
        if !self.is_connected().await? {
            return Err(Error::NotConnected);
        }
        let _ = mtu;
        Err(Error::Unsupported {
            platform: "unknown",
            feature: "request_mtu",
        })
    }

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

    #[test]
    fn reject_invalid_att_mtu_enforces_spec_range() {
        assert!(reject_invalid_att_mtu(ATT_MTU_MIN).is_ok());
        assert!(reject_invalid_att_mtu(ATT_MTU_MAX).is_ok());
        assert!(reject_invalid_att_mtu(512).is_ok());
        assert!(matches!(
            reject_invalid_att_mtu(ATT_MTU_MIN - 1),
            Err(Error::InvalidState(_))
        ));
        assert!(matches!(
            reject_invalid_att_mtu(ATT_MTU_MAX + 1),
            Err(Error::InvalidState(_))
        ));
    }

    #[test]
    fn default_request_mtu_requires_a_connection() {
        let result = Stub.request_mtu(512).now_or_never();
        assert!(
            matches!(result, Some(Err(Error::NotConnected))),
            "{result:?}"
        );
    }

    struct ConnectedStub;

    #[async_trait::async_trait]
    impl Peripheral for ConnectedStub {
        fn id(&self) -> &DeviceId {
            Stub.id()
        }
        fn name(&self) -> Option<String> {
            None
        }
        async fn is_connected(&self) -> Result<bool, Error> {
            Ok(true)
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
    fn default_request_mtu_is_unsupported_when_connected() {
        let result = ConnectedStub.request_mtu(512).now_or_never();
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

#[cfg(test)]
mod write_with_tests {
    use super::*;
    use crate::device::{AddressType, DeviceId};
    use crate::gatt::ServiceSet;
    use futures::{FutureExt, StreamExt};
    use std::sync::Mutex;

    struct ScriptedWriter {
        id: DeviceId,
        max: Mutex<Vec<usize>>,
        current_max: Mutex<usize>,
        writes: Mutex<Vec<Vec<u8>>>,
        fail_at: Option<usize>,
    }

    impl ScriptedWriter {
        fn new(max_sequence: Vec<usize>) -> Self {
            let current = max_sequence.first().copied().unwrap_or(20);
            Self {
                id: DeviceId::Mac([0; 6], AddressType::Public),
                max: Mutex::new(max_sequence),
                current_max: Mutex::new(current),
                writes: Mutex::new(Vec::new()),
                fail_at: None,
            }
        }

        fn written(&self) -> Vec<Vec<u8>> {
            self.writes.lock().expect("writes").clone()
        }
    }

    #[async_trait::async_trait]
    impl Peripheral for ScriptedWriter {
        fn id(&self) -> &DeviceId {
            &self.id
        }
        fn name(&self) -> Option<String> {
            None
        }
        async fn is_connected(&self) -> Result<bool, Error> {
            Ok(true)
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
            data: &[u8],
            _mode: WriteMode,
        ) -> Result<(), Error> {
            let max = *self.current_max.lock().expect("current_max");
            if data.len() > max {
                return Err(Error::InvalidLength {
                    len: data.len(),
                    max,
                });
            }
            let mut writes = self.writes.lock().expect("writes");
            if self.fail_at == Some(writes.len()) {
                return Err(Error::Disconnected);
            }
            writes.push(data.to_vec());
            Ok(())
        }
        async fn max_write_len(
            &self,
            _ch: &CharacteristicId,
            _mode: WriteMode,
        ) -> Result<usize, Error> {
            let mut seq = self.max.lock().expect("max");
            let value = if seq.len() <= 1 {
                seq.first().copied().unwrap_or(20)
            } else {
                seq.remove(0)
            };
            *self.current_max.lock().expect("current_max") = value;
            Ok(value)
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

    fn characteristic() -> CharacteristicId {
        CharacteristicId {
            service: crate::gatt::ServiceId {
                uuid: uuid::Uuid::nil(),
                instance: 0,
            },
            uuid: uuid::Uuid::nil(),
            instance: 0,
        }
    }

    #[test]
    fn default_options_reject_oversize() {
        let writer = ScriptedWriter::new(vec![4]);
        let data = vec![1, 2, 3, 4, 5];
        let result = writer
            .write_with(
                &characteristic(),
                &data,
                WriteMode::WithResponse,
                &WriteOptions::default(),
            )
            .now_or_never()
            .expect("ready");
        assert!(
            matches!(result, Err(Error::InvalidLength { len: 5, max: 4 })),
            "{result:?}"
        );
        assert!(writer.written().is_empty());
    }

    #[test]
    fn long_write_chunks_sequentially_without_truncating() {
        let writer = ScriptedWriter::new(vec![3]);
        let data = vec![1, 2, 3, 4, 5, 6, 7];
        let result = writer
            .write_with(
                &characteristic(),
                &data,
                WriteMode::WithResponse,
                &WriteOptions {
                    allow_long_write: true,
                },
            )
            .now_or_never()
            .expect("ready");
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            writer.written(),
            vec![vec![1, 2, 3], vec![4, 5, 6], vec![7]]
        );
    }

    #[test]
    fn long_write_requeries_max_write_len_before_each_chunk() {
        let writer = ScriptedWriter::new(vec![3, 1, 10]);
        let data = vec![1, 2, 3, 4, 5, 6];
        let result = writer
            .write_with(
                &characteristic(),
                &data,
                WriteMode::WithoutResponse,
                &WriteOptions {
                    allow_long_write: true,
                },
            )
            .now_or_never()
            .expect("ready");
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(writer.written(), vec![vec![1, 2, 3], vec![4], vec![5, 6]]);
    }

    #[test]
    fn long_write_stops_after_a_failed_chunk() {
        let mut writer = ScriptedWriter::new(vec![2]);
        writer.fail_at = Some(1);
        let data = vec![1, 2, 3, 4, 5];
        let result = writer
            .write_with(
                &characteristic(),
                &data,
                WriteMode::WithResponse,
                &WriteOptions {
                    allow_long_write: true,
                },
            )
            .now_or_never()
            .expect("ready");
        assert!(matches!(result, Err(Error::Disconnected)), "{result:?}");
        assert_eq!(writer.written(), vec![vec![1, 2]]);
    }
}

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

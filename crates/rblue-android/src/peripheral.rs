//! Android [`Peripheral`]: the GATT-client facade over the forked btleplug
//! peripheral object (JNI to the `com.nonpolynomial.btleplug.android.impl`
//! helper classes).
//!
//! The facade owns no platform state itself; it operates on the registry
//! entry shared with the adapter's event pump, which keeps the fork's
//! peripheral handle (one stable object per address), the discovered GATT
//! tree, and the subscription set of the current connection.
//!
//! The unified write semantics are enforced here before any platform call:
//! Android silently truncates oversize write-without-response payloads at
//! the stack level, so the length check is not optional.

use std::sync::Arc;

use btleplug::api::{CharPropFlags, Peripheral as BtlePeripheral, WriteType};
use futures::stream::StreamExt;
use rblue_api::device::DeviceId;
use rblue_api::error::{AttError, Error};
use rblue_api::gatt::{CharacteristicId, Notification, ServiceSet, WriteMode};
use rblue_api::peripheral::{ConnectOptions, ConnectionPriority, NotificationStream, Peripheral};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::adapter::DeviceEntry;
use crate::hub::broadcast_stream;
use crate::mapping::{
    att_invalid_handle, connection_parameter_preset, map_btleplug_error, max_write_len_from_mtu,
    notification_from, properties_to_flags, service_set_from_btleplug, ATT_WRITE_NOT_PERMITTED,
    PLATFORM,
};

/// How long the notification forwarder waits before re-acquiring the fork's
/// value stream after it ended or could not be acquired. Purely defensive:
/// a persistent failure must not spin the task hot.
const STREAM_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(100);

/// A GATT client session with one Android peripheral. Obtain via
/// [`Adapter::peripheral`](rblue_api::Adapter::peripheral); the registry only
/// contains devices this adapter instance has seen, so connecting an
/// unscanned address requires a scan first (Android's `connectGatt` itself
/// accepts arbitrary addresses, but the shell's registry is the contract
/// source).
pub(crate) struct AndroidPeripheral {
    entry: Arc<DeviceEntry>,
}

impl AndroidPeripheral {
    pub(crate) fn new(entry: Arc<DeviceEntry>) -> Self {
        Self { entry }
    }

    /// Resolves a session-scoped identifier against the discovered tree of
    /// the current connection and rebuilds the fork's characteristic value.
    /// The fork's Java layer addresses characteristics by UUID only, so the
    /// properties and descriptor list are carried for validation, not for
    /// addressing.
    fn resolve_characteristic(
        &self,
        id: &CharacteristicId,
    ) -> Result<btleplug::api::Characteristic, Error> {
        let Some(services) = self.entry.services() else {
            return Err(att_invalid_handle());
        };
        let service = services
            .services
            .iter()
            .filter(|service| service.uuid == id.service.uuid)
            .nth(id.service.instance as usize);
        let characteristic = service.and_then(|service| {
            service
                .characteristics
                .iter()
                .filter(|characteristic| characteristic.uuid == id.uuid)
                .nth(id.instance as usize)
        });
        let Some(characteristic) = characteristic else {
            return Err(att_invalid_handle());
        };
        Ok(btleplug::api::Characteristic {
            uuid: characteristic.uuid,
            service_uuid: id.service.uuid,
            properties: properties_to_flags(characteristic.properties),
            descriptors: characteristic
                .descriptors
                .iter()
                .map(|uuid| btleplug::api::Descriptor {
                    uuid: *uuid,
                    service_uuid: id.service.uuid,
                    characteristic_uuid: id.uuid,
                })
                .collect(),
        })
    }
}

#[async_trait::async_trait]
impl Peripheral for AndroidPeripheral {
    fn id(&self) -> &DeviceId {
        self.entry.id()
    }

    fn name(&self) -> Option<String> {
        self.entry.name()
    }

    async fn is_connected(&self) -> Result<bool, Error> {
        self.entry.is_connected().await
    }

    async fn connect(&self) -> Result<(), Error> {
        self.connect_with(&ConnectOptions::default()).await
    }

    async fn connect_with(&self, opts: &ConnectOptions) -> Result<(), Error> {
        if opts.auto_connect {
            return Err(Error::Unsupported {
                platform: PLATFORM,
                feature: "auto_connect",
            });
        }
        let desired = match opts.mtu {
            Some(mtu) => Some(mtu),
            None => crate::connect_mtu(),
        };
        if let Some(mtu) = desired {
            rblue_api::reject_invalid_att_mtu(mtu)?;
        }

        let already_connected = self.entry.is_connected().await?;
        let Some(peripheral) = self.entry.current_handle() else {
            return Err(Error::NotFound(self.entry.id().clone()));
        };
        if already_connected {
            // Zero-arg / default reconnects stay no-ops. An explicit
            // `ConnectOptions.mtu` still requests that MTU on the live link.
            if opts.mtu.is_some() {
                if let Some(mtu) = desired {
                    peripheral
                        .request_mtu(mtu)
                        .await
                        .map_err(map_btleplug_error)?;
                }
            }
            return Ok(());
        }

        // The fork's Java layer retries the fast-failing GATT status 133 /
        // HCI 0x3E connect attempts itself (up to 3 attempts) and then
        // reports `NotConnectedException` - the raw status number of the
        // failed connection attempt does not survive into Rust (fork
        // limitation, see docs/error-mapping.md). A connect that got the
        // link up but failed the configured connect-time MTU request does
        // carry its status: `Error::Platform`.
        peripheral.set_connect_mtu(desired);
        peripheral.connect().await.map_err(map_btleplug_error)?;
        // The fork refreshed its properties snapshot with the system-cached
        // GAP name; mirror it so name-based reconnect filtering sees the
        // most reliable name even without a fresh sighting.
        self.entry.refresh_name(&peripheral).await;
        Ok(())
    }

    async fn disconnect(&self) -> Result<(), Error> {
        // Idempotent; the fork's disconnect also emits a DeviceDisconnected
        // event, which the pump collapses onto the transition check.
        if !self.entry.is_connected().await? {
            return Ok(());
        }
        let Some(peripheral) = self.entry.current_handle() else {
            return Ok(());
        };
        // A failed disconnect reports its raw GATT status
        // (`"Unable to disconnect, status: N"` -> `Error::Platform`).
        peripheral.disconnect().await.map_err(map_btleplug_error)
    }

    async fn discover_services(&self, filter: &[Uuid]) -> Result<ServiceSet, Error> {
        let peripheral = self.entry.connected_handle().await?;
        // Android has no filtered discovery; the full tree is fetched and
        // filtered in software. Filtered results keep the contract's
        // caveat: identifiers for services outside the filter require a
        // full (empty-filter) rediscovery. The fork's Java layer rebuilds
        // the GATT cache walk on every call, so a firmware GATT-table
        // change is picked up on reconnection (BluetoothGatt.refresh()
        // reflection is not exposed by the fork - see
        // docs/platform-notes.md).
        peripheral
            .discover_services()
            .await
            .map_err(map_btleplug_error)?;
        let discovered = peripheral.services();
        let services = service_set_from_btleplug(&discovered, filter);
        self.entry.store_services(services.clone());
        Ok(services)
    }

    async fn read(&self, ch: &CharacteristicId) -> Result<Vec<u8>, Error> {
        let peripheral = self.entry.connected_handle().await?;
        let characteristic = self.resolve_characteristic(ch)?;
        peripheral
            .read(&characteristic)
            .await
            .map_err(map_btleplug_error)
    }

    async fn write(
        &self,
        ch: &CharacteristicId,
        data: &[u8],
        mode: WriteMode,
    ) -> Result<(), Error> {
        let peripheral = self.entry.connected_handle().await?;
        let characteristic = self.resolve_characteristic(ch)?;
        let required = match mode {
            WriteMode::WithResponse => CharPropFlags::WRITE,
            WriteMode::WithoutResponse => CharPropFlags::WRITE_WITHOUT_RESPONSE,
        };
        if !characteristic.properties.contains(required) {
            return Err(Error::Att(AttError {
                code: ATT_WRITE_NOT_PERMITTED,
                handle: None,
            }));
        }
        // Unified write semantics, checked before the platform call: the
        // Android stack silently truncates oversize payloads (both write
        // types enforce MTU - 3 per call).
        let max = max_write_len_from_mtu(peripheral.mtu());
        if data.len() > max {
            return Err(Error::InvalidLength {
                len: data.len(),
                max,
            });
        }
        peripheral
            .write(
                &characteristic,
                data,
                match mode {
                    WriteMode::WithResponse => WriteType::WithResponse,
                    WriteMode::WithoutResponse => WriteType::WithoutResponse,
                },
            )
            .await
            .map_err(map_btleplug_error)
    }

    async fn max_write_len(&self, ch: &CharacteristicId, _mode: WriteMode) -> Result<usize, Error> {
        let peripheral = self.entry.connected_handle().await?;
        self.resolve_characteristic(ch)?;
        // The fork's MTU is the value negotiated at connect (ATT default
        // 23 until then), so this is the conservative 20 only before the
        // first connect. Both write modes share the limit. See
        // docs/platform-notes.md.
        Ok(max_write_len_from_mtu(peripheral.mtu()))
    }

    async fn subscribe(&self, ch: &CharacteristicId) -> Result<(), Error> {
        let peripheral = self.entry.connected_handle().await?;
        let characteristic = self.resolve_characteristic(ch)?;
        if !characteristic
            .properties
            .intersects(CharPropFlags::NOTIFY | CharPropFlags::INDICATE)
        {
            return Err(Error::InvalidState(
                "characteristic has neither notify nor indicate".to_owned(),
            ));
        }
        // The fork writes the CCCD itself and picks indication over
        // notification when both are supported (Android requires an explicit
        // descriptor write; the property bits decide the value). A failed
        // descriptor write reports its raw GATT status (`Error::Platform`).
        peripheral
            .subscribe(&characteristic)
            .await
            .map_err(map_btleplug_error)?;
        self.entry.add_subscription(*ch);
        Ok(())
    }

    async fn unsubscribe(&self, ch: &CharacteristicId) -> Result<(), Error> {
        let peripheral = self.entry.connected_handle().await?;
        let characteristic = self.resolve_characteristic(ch)?;
        // The fork's Java layer disables the local notification routing and
        // writes the "disabled" CCCD value; btleplug documents
        // unsubscribing a non-subscribed characteristic as not-an-error, so
        // repeated unsubscribes stay idempotent.
        peripheral
            .unsubscribe(&characteristic)
            .await
            .map_err(map_btleplug_error)?;
        self.entry.remove_subscription(ch);
        Ok(())
    }

    fn notifications(&self) -> NotificationStream {
        let hub = DeviceEntry::notifications_hub(&self.entry);
        broadcast_stream(hub.sender.subscribe()).boxed()
    }

    async fn pair(&self) -> Result<(), Error> {
        crate::pairing::pair(self.entry.address()).await
    }

    async fn unpair(&self) -> Result<(), Error> {
        crate::pairing::unpair(self.entry.address()).await
    }

    async fn rssi(&self) -> Result<i16, Error> {
        // readRemoteRssi over the connected link (RssiDuringConnection
        // capability). A failed read reports its raw GATT status
        // (`"RSSI read failed, status: N"` -> `Error::Platform`).
        let peripheral = self.entry.connected_handle().await?;
        peripheral.read_rssi().await.map_err(map_btleplug_error)
    }

    async fn mtu(&self) -> Result<u16, Error> {
        // The negotiated ATT MTU cached by the fork after the connect-time
        // or explicit request; it changes when the stack renegotiates or a
        // later request succeeds. See docs/platform-notes.md.
        let peripheral = self.entry.connected_handle().await?;
        Ok(peripheral.mtu())
    }

    async fn request_mtu(&self, mtu: u16) -> Result<u16, Error> {
        let peripheral = self.entry.connected_handle().await?;
        rblue_api::reject_invalid_att_mtu(mtu)?;
        peripheral
            .request_mtu(mtu)
            .await
            .map_err(map_btleplug_error)
    }

    async fn set_connection_priority(&self, priority: ConnectionPriority) -> Result<(), Error> {
        // `connected_handle` enforces NotConnected before the fork call.
        // The fork maps Low/Balanced/High onto requestConnectionPriority
        // CONNECTION_PRIORITY_LOW_POWER / BALANCED / HIGH.
        let peripheral = self.entry.connected_handle().await?;
        peripheral
            .request_connection_parameters(connection_parameter_preset(priority))
            .await
            .map_err(map_btleplug_error)
    }
}

/// Broadcast hub behind [`Peripheral::notifications`]: one forwarder task per
/// device moves values from the fork's notification stream into this sender.
/// The fork's stream stays valid across connections (its Java-side queue
/// lives on the peripheral object), so subscribers survive reconnects.
pub(crate) struct NotificationHub {
    pub(crate) sender: broadcast::Sender<Notification>,
}

/// Starts the notification forwarder for `entry`. Called once, from the
/// first `notifications()` subscription.
pub(crate) fn spawn_notification_forwarder(
    entry: std::sync::Weak<DeviceEntry>,
    sender: broadcast::Sender<Notification>,
) {
    tokio::spawn(notification_forwarder(entry, sender));
}

/// Forwards GATT notifications from the fork's value stream into the hub.
/// Values are routed through the cached tree of the current connection and
/// the subscription set, so a value only reaches subscribers while the
/// characteristic is both discovered and subscribed on this connection.
async fn notification_forwarder(
    entry: std::sync::Weak<DeviceEntry>,
    sender: broadcast::Sender<Notification>,
) {
    loop {
        let Some(entry) = entry.upgrade() else {
            return;
        };
        // Entries are only created with a live fork handle.
        let Some(peripheral) = entry.current_handle() else {
            return;
        };
        match peripheral.notifications().await {
            Ok(stream) => {
                let mut stream = stream;
                while let Some(value) = stream.next().await {
                    let notification = entry.services().and_then(|services| {
                        notification_from(&services, |id| entry.is_subscribed(id), &value)
                    });
                    if let Some(notification) = notification {
                        let _ = sender.send(notification);
                    }
                }
                // The fork's value stream ended (its Java-side queue closed,
                // e.g. the JVM is going away); fall through and re-acquire.
            }
            Err(_) => {
                // Transient failure acquiring the value stream.
            }
        }
        tokio::time::sleep(STREAM_RETRY_DELAY).await;
    }
}

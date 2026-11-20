//! WinRT [`Peripheral`]: the GATT-client facade over the forked btleplug
//! peripheral object.
//!
//! The facade owns no platform state itself; it operates on the registry
//! entry shared with the adapter's event pump, which keeps the freshest
//! btleplug handle and the discovered GATT tree of the current connection.

use std::sync::Arc;

use btleplug::api::{CharPropFlags, Peripheral as BtlePeripheral, ValueNotification};
use futures::stream::StreamExt;
use rblue_api::device::DeviceId;
use rblue_api::error::{AttError, Error};
use rblue_api::gatt::{CharacteristicId, Notification, ServiceSet, WriteMode};
use rblue_api::peripheral::{ConnectionPriority, NotificationStream, Peripheral};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::adapter::DeviceEntry;
use crate::hub::broadcast_stream;
use crate::mapping::{
    att_invalid_handle, connection_parameter_preset, locate_characteristic, map_btleplug_error,
    max_write_len_from_mtu, properties_to_flags, service_set_from_btleplug,
    ATT_WRITE_NOT_PERMITTED, PLATFORM,
};

/// A GATT client session with one WinRT peripheral. Obtain via
/// [`Adapter::peripheral`](rblue_api::Adapter::peripheral) after the device
/// was sighted by a scan - Windows can only connect to addresses its watcher
/// has observed in this boot session.
pub(crate) struct WinRtPeripheral {
    entry: Arc<DeviceEntry>,
}

impl WinRtPeripheral {
    pub(crate) fn new(entry: Arc<DeviceEntry>) -> Self {
        Self { entry }
    }

    /// Resolves a session-scoped identifier against the discovered tree of
    /// the current connection and rebuilds the fork's characteristic value.
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
impl Peripheral for WinRtPeripheral {
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
        let Some(peripheral) = self.entry.current_handle() else {
            return Err(Error::NotFound(self.entry.id().clone()));
        };
        peripheral.connect().await.map_err(|error| match error {
            // `BluetoothLEDevice::FromBluetoothAddressAsync` could not
            // resolve the address: the OS has no cache entry for it in this
            // boot session. Windows never connects to addresses its watcher
            // has not observed (RPA rotation included) - re-scan first. See
            // docs/platform-notes.md.
            btleplug::Error::DeviceNotFound => Error::NotFound(self.entry.id().clone()),
            other => map_btleplug_error(other),
        })
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
        peripheral.disconnect().await.map_err(map_btleplug_error)
    }

    async fn discover_services(&self, filter: &[Uuid]) -> Result<ServiceSet, Error> {
        let peripheral = self.entry.connected_handle().await?;
        // Windows has no filtered discovery; the full tree is fetched and
        // filtered in software. Filtered results keep the contract's caveat:
        // identifiers for services outside the filter require a full
        // (empty-filter) rediscovery.
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
        // Unified write semantics: never rely on the platform behavior for
        // oversize payloads.
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
                    WriteMode::WithResponse => btleplug::api::WriteType::WithResponse,
                    WriteMode::WithoutResponse => btleplug::api::WriteType::WithoutResponse,
                },
            )
            .await
            .map_err(map_btleplug_error)
    }

    async fn max_write_len(&self, ch: &CharacteristicId, _mode: WriteMode) -> Result<usize, Error> {
        let peripheral = self.entry.connected_handle().await?;
        self.resolve_characteristic(ch)?;
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
        // The fork writes the CCCD; Windows picks notification or indication
        // from the property bits (the fork prefers indication when both are
        // supported).
        peripheral
            .subscribe(&characteristic)
            .await
            .map_err(map_btleplug_error)
    }

    async fn unsubscribe(&self, ch: &CharacteristicId) -> Result<(), Error> {
        let peripheral = self.entry.connected_handle().await?;
        let characteristic = self.resolve_characteristic(ch)?;
        peripheral
            .unsubscribe(&characteristic)
            .await
            .map_err(map_btleplug_error)
    }

    fn notifications(&self) -> NotificationStream {
        let hub = DeviceEntry::notifications_hub(&self.entry);
        broadcast_stream(hub.sender.subscribe()).boxed()
    }

    async fn pair(&self) -> Result<(), Error> {
        // WinRT exposes `DeviceInformation.Pairing.PairAsync`, but the fork
        // does not surface it; the capability is reported as absent instead
        // of pretending. Encrypted-characteristic access still triggers the
        // implicit system pairing dialog on Windows.
        Err(Error::Unsupported {
            platform: PLATFORM,
            feature: "pairing",
        })
    }

    async fn rssi(&self) -> Result<i16, Error> {
        // Windows exposes no connected-RSSI read; the fork's read_rssi would
        // return the last scan-time value from the property cache, which the
        // unified model refuses to present as a connected reading.
        Err(Error::Unsupported {
            platform: PLATFORM,
            feature: "rssi",
        })
    }

    async fn mtu(&self) -> Result<u16, Error> {
        // The negotiated ATT MTU is available through GattSession
        // (MtuQuery capability); it changes when the stack renegotiates.
        let peripheral = self.entry.connected_handle().await?;
        Ok(peripheral.mtu())
    }

    async fn set_connection_priority(&self, priority: ConnectionPriority) -> Result<(), Error> {
        // WinRT exposes RequestPreferredConnectionParameters with the same
        // balanced / throughput / power presets the fork already wraps.
        let peripheral = self.entry.connected_handle().await?;
        peripheral
            .request_connection_parameters(connection_parameter_preset(priority))
            .await
            .map_err(map_btleplug_error)
    }
}

/// Broadcast hub behind [`Peripheral::notifications`]: one forwarder task per
/// device moves values from the current btleplug handle's stream into this
/// sender, so subscribers survive handle swaps across reconnects.
pub(crate) struct NotificationHub {
    pub(crate) sender: broadcast::Sender<Notification>,
}

/// Starts the notification forwarder for `entry`. Called once, from the
/// first `notifications()` subscription.
pub(crate) fn spawn_notification_forwarder(
    entry: &Arc<DeviceEntry>,
    sender: broadcast::Sender<Notification>,
) {
    tokio::spawn(notification_forwarder(
        Arc::downgrade(entry),
        sender,
        entry.handle_generation(),
    ));
}

/// Forwards GATT notifications from the current btleplug handle into the
/// hub, re-subscribing whenever the adapter's event pump installs a new
/// handle (re-sighting after a disconnect rebuilds the fork's internal
/// object and its value stream).
async fn notification_forwarder(
    entry: std::sync::Weak<DeviceEntry>,
    sender: broadcast::Sender<Notification>,
    mut generation: tokio::sync::watch::Receiver<u64>,
) {
    loop {
        let Some(entry) = entry.upgrade() else {
            return;
        };
        let peripheral = loop {
            if let Some(peripheral) = entry.current_handle() {
                break peripheral;
            }
            if generation.changed().await.is_err() {
                return;
            }
        };
        let Ok(mut stream) = peripheral.notifications().await else {
            continue;
        };
        loop {
            tokio::select! {
                value = stream.next() => match value {
                    Some(value) => {
                        if let Some(notification) = notification_from(&entry, &value) {
                            let _ = sender.send(notification);
                        }
                    }
                    None => break,
                },
                changed = generation.changed() => match changed {
                    // Handle swapped: drop the stale stream and re-subscribe
                    // on the new object.
                    Ok(()) => break,
                    // Adapter dropped: the entry is gone.
                    Err(_) => return,
                },
            }
        }
    }
}

/// Maps a fork value notification onto the unified model using the cached
/// tree of the current connection. Without a tree (before discovery, or
/// after a disconnect cleared it) values cannot be addressed and are
/// dropped, mirroring the contract that identifiers are session-scoped.
fn notification_from(entry: &DeviceEntry, value: &ValueNotification) -> Option<Notification> {
    let services = entry.services()?;
    locate_characteristic(&services, value.service_uuid, value.uuid).map(|id| Notification {
        id,
        data: value.value.clone(),
    })
}

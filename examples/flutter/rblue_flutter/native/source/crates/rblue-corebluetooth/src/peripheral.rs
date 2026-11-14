//! CoreBluetooth [`Peripheral`]: the GATT-client facade over the forked
//! btleplug peripheral object.
//!
//! The facade owns no platform state itself; it operates on the registry
//! entry shared with the adapter's event pump, which keeps the freshest
//! btleplug handle and the discovered GATT tree of the current connection.

use std::collections::BTreeSet;
use std::sync::Arc;

use btleplug::api::{CharPropFlags, Peripheral as BtlePeripheral, ValueNotification};
use futures::stream::StreamExt;
use rblue_api::device::DeviceId;
use rblue_api::error::{AttError, Error};
use rblue_api::gatt::{
    Characteristic, CharacteristicId, CharacteristicProperties, Notification, Service, ServiceId,
    ServiceSet, WriteMode,
};
use rblue_api::peripheral::{NotificationStream, Peripheral};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::adapter::DeviceEntry;
use crate::hub::broadcast_stream;
use crate::mapping::{
    att_invalid_handle, map_btleplug_error, map_properties, properties_to_flags,
    ATT_WRITE_NOT_PERMITTED, PLATFORM,
};

/// A GATT client session with one CoreBluetooth peripheral. Obtain via
/// [`Adapter::peripheral`](rblue_api::Adapter::peripheral) after the device
/// was sighted by a scan.
pub(crate) struct CoreBluetoothPeripheral {
    entry: Arc<DeviceEntry>,
}

impl CoreBluetoothPeripheral {
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

    /// The maximum write payload for either mode, from the fork's ATT-MTU
    /// estimate sampled at discovery time.
    fn max_write_len_from(peripheral: &btleplug::platform::Peripheral) -> usize {
        usize::from(peripheral.mtu()).saturating_sub(3)
    }
}

#[async_trait::async_trait]
impl Peripheral for CoreBluetoothPeripheral {
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
        // After a disconnect the fork has no internal state for this device;
        // retrieve it by identifier so connectPeripheral has a target.
        self.entry.refresh_handle().await;
        let Some(peripheral) = self.entry.current_handle() else {
            return Err(Error::NotFound(self.entry.id().clone()));
        };
        peripheral.connect().await.map_err(|error| match error {
            btleplug::Error::RuntimeError(message) if message.ends_with("no longer available") => {
                Error::NotFound(self.entry.id().clone())
            }
            other => map_btleplug_error(other),
        })
    }

    async fn disconnect(&self) -> Result<(), Error> {
        // Idempotent; also avoids the fork's cancelPeripheralConnection
        // future, which never resolves for a peripheral that is not
        // connected.
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
        // CoreBluetooth has no filtered discovery; the full tree is fetched
        // and filtered in software. Filtered results keep the contract's
        // caveat: identifiers for services outside the filter require a
        // full (empty-filter) rediscovery.
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
        // Unified write semantics: never rely on the native writeValue
        // silently truncating oversize payloads.
        let max = Self::max_write_len_from(&peripheral);
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
        Ok(Self::max_write_len_from(&peripheral))
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
        // The fork writes the CCCD via setNotifyValue; CoreBluetooth itself
        // picks notification or indication from the property bits.
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
        // iOS/macOS pair implicitly when an encrypted characteristic is
        // accessed; there is no active-pairing API.
        Err(Error::Unsupported {
            platform: PLATFORM,
            feature: "pairing",
        })
    }

    async fn rssi(&self) -> Result<i16, Error> {
        let peripheral = self.entry.connected_handle().await?;
        peripheral.read_rssi().await.map_err(map_btleplug_error)
    }

    async fn mtu(&self) -> Result<u16, Error> {
        // CoreBluetooth manages the ATT MTU and does not expose it; use
        // max_write_len instead.
        Err(Error::Unsupported {
            platform: PLATFORM,
            feature: "mtu",
        })
    }
}

/// Converts the fork's discovered tree into the unified model, restricted to
/// `filter` (empty filter keeps everything). Instance indices are derived at
/// resolution time by counting same-UUID entries in the set order; the fork
/// returns a `BTreeSet`, so same-UUID duplicates (rare) are ordered by
/// UUID-derived sort order rather than hardware discovery order.
pub(crate) fn service_set_from_btleplug(
    discovered: &BTreeSet<btleplug::api::Service>,
    filter: &[Uuid],
) -> ServiceSet {
    let services = discovered
        .iter()
        .filter(|service| filter.is_empty() || filter.contains(&service.uuid))
        .map(|service| Service {
            uuid: service.uuid,
            primary: service.primary,
            characteristics: service
                .characteristics
                .iter()
                .map(|characteristic| Characteristic {
                    uuid: characteristic.uuid,
                    properties: map_properties(characteristic.properties),
                    descriptors: characteristic
                        .descriptors
                        .iter()
                        .map(|descriptor| descriptor.uuid)
                        .collect(),
                })
                .collect(),
        })
        .collect();
    ServiceSet { services }
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
/// after `DeviceServicesModified` cleared it) values cannot be addressed and
/// are dropped, mirroring the contract that identifiers are session-scoped.
fn notification_from(entry: &DeviceEntry, value: &ValueNotification) -> Option<Notification> {
    let services = entry.services()?;
    locate_characteristic(&services, value.service_uuid, value.uuid).map(|id| Notification {
        id,
        data: value.value.clone(),
    })
}

/// Finds the [`CharacteristicId`] for a notification: among same-UUID
/// candidates, prefer one that can actually notify or indicate.
pub(crate) fn locate_characteristic(
    services: &ServiceSet,
    service_uuid: Uuid,
    characteristic_uuid: Uuid,
) -> Option<CharacteristicId> {
    let mut preferred = None;
    let mut fallback = None;
    for (service_instance, service) in services
        .services
        .iter()
        .filter(|service| service.uuid == service_uuid)
        .enumerate()
    {
        for (characteristic_instance, characteristic) in service
            .characteristics
            .iter()
            .filter(|characteristic| characteristic.uuid == characteristic_uuid)
            .enumerate()
        {
            let id = CharacteristicId {
                service: ServiceId {
                    uuid: service_uuid,
                    instance: service_instance as u16,
                },
                uuid: characteristic_uuid,
                instance: characteristic_instance as u16,
            };
            let subscribable = characteristic
                .properties
                .intersects(CharacteristicProperties::NOTIFY | CharacteristicProperties::INDICATE);
            if preferred.is_none() && subscribable {
                preferred = Some(id);
            }
            if fallback.is_none() {
                fallback = Some(id);
            }
        }
    }
    preferred.or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::uuid;

    const SERVICE: Uuid = uuid!("0000180d-0000-1000-8000-00805f9b34fb");
    const CHARACTERISTIC: Uuid = uuid!("00002a37-0000-1000-8000-00805f9b34fb");

    fn characteristic(properties: CharacteristicProperties) -> Characteristic {
        Characteristic {
            uuid: CHARACTERISTIC,
            properties,
            descriptors: vec![],
        }
    }

    fn service_set(characteristics: Vec<Characteristic>) -> ServiceSet {
        ServiceSet {
            services: vec![Service {
                uuid: SERVICE,
                primary: true,
                characteristics,
            }],
        }
    }

    #[test]
    fn notification_is_located_in_the_tree() {
        let set = service_set(vec![characteristic(
            CharacteristicProperties::READ | CharacteristicProperties::NOTIFY,
        )]);
        let id = locate_characteristic(&set, SERVICE, CHARACTERISTIC).expect("located");
        assert_eq!(id.service.uuid, SERVICE);
        assert_eq!(id.service.instance, 0);
        assert_eq!(id.uuid, CHARACTERISTIC);
        assert_eq!(id.instance, 0);
    }

    #[test]
    fn unsubscribable_candidates_are_only_a_fallback() {
        let set = service_set(vec![
            characteristic(CharacteristicProperties::READ),
            characteristic(CharacteristicProperties::NOTIFY),
        ]);
        // Two same-UUID characteristics: the notify-capable one (instance 1)
        // is preferred over the read-only one (instance 0).
        let id = locate_characteristic(&set, SERVICE, CHARACTERISTIC).expect("located");
        assert_eq!(id.instance, 1);
    }

    #[test]
    fn unknown_characteristics_are_not_located() {
        let set = service_set(vec![characteristic(CharacteristicProperties::NOTIFY)]);
        assert_eq!(locate_characteristic(&set, SERVICE, Uuid::nil()), None);
        assert_eq!(
            locate_characteristic(&set, Uuid::nil(), CHARACTERISTIC),
            None
        );
        assert_eq!(
            locate_characteristic(&ServiceSet::default(), SERVICE, CHARACTERISTIC),
            None
        );
    }

    #[test]
    fn service_instances_count_same_uuid_services() {
        // Two distinguishable services sharing one UUID (primary vs.
        // secondary keeps the BTreeSet from deduplicating them).
        let primary = btleplug::api::Service {
            uuid: SERVICE,
            primary: true,
            characteristics: BTreeSet::new(),
        };
        let secondary = btleplug::api::Service {
            primary: false,
            ..primary.clone()
        };
        let mut discovered = BTreeSet::new();
        discovered.insert(primary);
        discovered.insert(secondary);

        let set = service_set_from_btleplug(&discovered, &[]);
        assert_eq!(set.services.len(), 2);
        assert!(set.services.iter().all(|service| service.uuid == SERVICE));
    }

    #[test]
    fn service_filter_keeps_matching_subset() {
        let other = btleplug::api::Service {
            uuid: uuid!("0000180f-0000-1000-8000-00805f9b34fb"),
            primary: true,
            characteristics: BTreeSet::new(),
        };
        let battery = btleplug::api::Service {
            uuid: SERVICE,
            primary: true,
            characteristics: BTreeSet::new(),
        };
        let mut discovered = BTreeSet::new();
        discovered.insert(other);
        discovered.insert(battery);

        let set = service_set_from_btleplug(&discovered, &[SERVICE]);
        assert_eq!(set.services.len(), 1);
        assert_eq!(set.services[0].uuid, SERVICE);

        let set = service_set_from_btleplug(&discovered, &[]);
        assert_eq!(set.services.len(), 2);
    }
}

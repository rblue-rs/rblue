//! Local GATT server and virtual-central control plane for the mock backend.

use std::collections::HashSet;
use std::sync::Arc;

use futures::channel::mpsc::unbounded;
use futures::stream::{BoxStream, StreamExt};

use rblue_api::device::DeviceId;
use rblue_api::error::{AttError, Error};
use rblue_api::event::DisconnectReason;
use rblue_api::gatt::{CharacteristicProperties, WriteMode};
use rblue_api::server::{
    GattServerEvent, LocalCharacteristicId, LocalCharacteristicSnapshot, LocalService,
    LocalServiceId, LocalServiceSnapshot,
};

use crate::adapter::{AdapterShared, DEFAULT_MAX_WRITE_LEN};
use crate::hub::BroadcastHub;

/// One registered local service and its characteristics.
#[derive(Debug, Clone)]
pub(crate) struct RegisteredService {
    pub(crate) id: LocalServiceId,
    pub(crate) primary: bool,
    pub(crate) characteristics: Vec<RegisteredCharacteristic>,
}

/// One registered local characteristic.
#[derive(Debug, Clone)]
pub(crate) struct RegisteredCharacteristic {
    pub(crate) id: LocalCharacteristicId,
    pub(crate) properties: CharacteristicProperties,
    pub(crate) value: Vec<u8>,
    pub(crate) max_len: usize,
}

/// A virtual central currently connected to the mock GATT server.
#[derive(Debug)]
pub(crate) struct ConnectedCentral {
    pub(crate) id: DeviceId,
    pub(crate) notifications: BroadcastHub<(LocalCharacteristicId, Vec<u8>)>,
}

/// In-memory GATT server bookkeeping stored on the mock adapter.
#[derive(Debug, Default)]
pub(crate) struct ServerState {
    pub(crate) services: Vec<RegisteredService>,
    pub(crate) centrals: Vec<ConnectedCentral>,
    pub(crate) subscriptions: HashSet<(DeviceId, LocalCharacteristicId)>,
}

impl ServerState {
    fn allocate_service_instance(&self, uuid: uuid::Uuid) -> u16 {
        let mut used: Vec<u16> = self
            .services
            .iter()
            .filter(|service| service.id.uuid == uuid)
            .map(|service| service.id.instance)
            .collect();
        used.sort_unstable();
        let next = used.len() as u16;
        used.into_iter()
            .enumerate()
            .find_map(|(expected, instance)| {
                let expected = expected as u16;
                (instance != expected).then_some(expected)
            })
            .unwrap_or(next)
    }

    fn snapshot(&self) -> Vec<LocalServiceSnapshot> {
        self.services
            .iter()
            .map(|service| LocalServiceSnapshot {
                id: service.id,
                primary: service.primary,
                characteristics: service
                    .characteristics
                    .iter()
                    .map(|characteristic| LocalCharacteristicSnapshot {
                        id: characteristic.id,
                        properties: characteristic.properties,
                        value: characteristic.value.clone(),
                        max_len: characteristic.max_len,
                    })
                    .collect(),
            })
            .collect()
    }

    fn find_characteristic_mut(
        &mut self,
        id: &LocalCharacteristicId,
    ) -> Result<&mut RegisteredCharacteristic, Error> {
        self.services
            .iter_mut()
            .find(|service| service.id == id.service)
            .and_then(|service| {
                service
                    .characteristics
                    .iter_mut()
                    .find(|characteristic| characteristic.id == *id)
            })
            .ok_or_else(|| Error::InvalidState("local characteristic is not registered".into()))
    }

    fn find_characteristic(
        &self,
        id: &LocalCharacteristicId,
    ) -> Result<&RegisteredCharacteristic, Error> {
        self.services
            .iter()
            .find(|service| service.id == id.service)
            .and_then(|service| {
                service
                    .characteristics
                    .iter()
                    .find(|characteristic| characteristic.id == *id)
            })
            .ok_or_else(|| Error::InvalidState("local characteristic is not registered".into()))
    }

    fn is_connected(&self, id: &DeviceId) -> bool {
        self.centrals.iter().any(|central| central.id == *id)
    }

    fn notify_subscribers(&self, characteristic: LocalCharacteristicId, value: &[u8]) {
        for central in &self.centrals {
            if self
                .subscriptions
                .contains(&(central.id.clone(), characteristic))
            {
                central
                    .notifications
                    .publish((characteristic, value.to_vec()));
            }
        }
    }
}

impl AdapterShared {
    pub(crate) fn add_local_service(
        &self,
        service: &LocalService,
    ) -> Result<LocalServiceId, Error> {
        let mut state = self.lock();
        let service_id = LocalServiceId {
            uuid: service.uuid,
            instance: state.server.allocate_service_instance(service.uuid),
        };
        let mut characteristics = Vec::with_capacity(service.characteristics.len());
        for declared in &service.characteristics {
            let instance = characteristics
                .iter()
                .filter(|registered: &&RegisteredCharacteristic| {
                    registered.id.uuid == declared.uuid
                })
                .count() as u16;
            let max_len = declared.max_len.unwrap_or(DEFAULT_MAX_WRITE_LEN);
            if declared.value.len() > max_len {
                return Err(Error::InvalidLength {
                    len: declared.value.len(),
                    max: max_len,
                });
            }
            characteristics.push(RegisteredCharacteristic {
                id: LocalCharacteristicId {
                    service: service_id,
                    uuid: declared.uuid,
                    instance,
                },
                properties: declared.properties,
                value: declared.value.clone(),
                max_len,
            });
        }
        state.server.services.push(RegisteredService {
            id: service_id,
            primary: service.primary,
            characteristics,
        });
        Ok(service_id)
    }

    pub(crate) fn remove_local_service(&self, id: &LocalServiceId) -> Result<(), Error> {
        let mut state = self.lock();
        let position = state
            .server
            .services
            .iter()
            .position(|service| service.id == *id)
            .ok_or_else(|| Error::InvalidState("local service is not registered".into()))?;
        let removed = state.server.services.remove(position);
        let removed_ids: HashSet<LocalCharacteristicId> = removed
            .characteristics
            .iter()
            .map(|characteristic| characteristic.id)
            .collect();
        let dropped: Vec<(DeviceId, LocalCharacteristicId)> = state
            .server
            .subscriptions
            .iter()
            .filter(|(_, characteristic)| removed_ids.contains(characteristic))
            .cloned()
            .collect();
        for (central, characteristic) in dropped {
            state
                .server
                .subscriptions
                .remove(&(central.clone(), characteristic));
            self.publish_server(GattServerEvent::SubscriptionChanged {
                central,
                characteristic,
                subscribed: false,
            });
        }
        Ok(())
    }

    pub(crate) fn local_services_snapshot(&self) -> Vec<LocalServiceSnapshot> {
        self.lock().server.snapshot()
    }

    pub(crate) fn write_local_characteristic(
        &self,
        id: &LocalCharacteristicId,
        value: &[u8],
    ) -> Result<(), Error> {
        let mut state = self.lock();
        let characteristic = state.server.find_characteristic_mut(id)?;
        if value.len() > characteristic.max_len {
            return Err(Error::InvalidLength {
                len: value.len(),
                max: characteristic.max_len,
            });
        }
        characteristic.value = value.to_vec();
        Ok(())
    }

    pub(crate) fn notify_local_characteristic(
        &self,
        id: &LocalCharacteristicId,
        value: &[u8],
    ) -> Result<(), Error> {
        let mut state = self.lock();
        let characteristic = state.server.find_characteristic_mut(id)?;
        if !characteristic
            .properties
            .intersects(CharacteristicProperties::NOTIFY | CharacteristicProperties::INDICATE)
        {
            return Err(Error::InvalidState(
                "local characteristic has neither NOTIFY nor INDICATE".into(),
            ));
        }
        if value.len() > characteristic.max_len {
            return Err(Error::InvalidLength {
                len: value.len(),
                max: characteristic.max_len,
            });
        }
        characteristic.value = value.to_vec();
        let stored = characteristic.value.clone();
        state.server.notify_subscribers(*id, &stored);
        Ok(())
    }

    pub(crate) fn server_event_stream(&self) -> BoxStream<'static, GattServerEvent> {
        let (tx, rx) = unbounded();
        {
            let state = self.lock();
            self.server_events.register(tx.clone());
            for central in &state.server.centrals {
                let _ = tx.unbounded_send(GattServerEvent::CentralConnected {
                    central: central.id.clone(),
                });
            }
        }
        rx.boxed()
    }

    pub(crate) fn connect_virtual_central(&self, id: DeviceId) -> Result<(), Error> {
        {
            let mut state = self.lock();
            if state.server.is_connected(&id) {
                return Err(Error::InvalidState(
                    "virtual central is already connected".into(),
                ));
            }
            state.server.centrals.push(ConnectedCentral {
                id: id.clone(),
                notifications: BroadcastHub::default(),
            });
        }
        self.publish_server(GattServerEvent::CentralConnected { central: id });
        Ok(())
    }

    pub(crate) fn disconnect_virtual_central(
        &self,
        id: &DeviceId,
        reason: Option<DisconnectReason>,
    ) -> Result<(), Error> {
        self.drop_virtual_central(id, reason, true)
    }

    pub(crate) fn drop_all_virtual_centrals(&self, reason: Option<DisconnectReason>) {
        let ids: Vec<DeviceId> = {
            let state = self.lock();
            state
                .server
                .centrals
                .iter()
                .map(|central| central.id.clone())
                .collect()
        };
        for id in ids {
            let _ = self.drop_virtual_central(&id, reason, true);
        }
    }

    fn drop_virtual_central(
        &self,
        id: &DeviceId,
        reason: Option<DisconnectReason>,
        required: bool,
    ) -> Result<(), Error> {
        let dropped: Vec<LocalCharacteristicId>;
        {
            let mut state = self.lock();
            let Some(position) = state
                .server
                .centrals
                .iter()
                .position(|central| central.id == *id)
            else {
                if required {
                    return Err(Error::InvalidState(
                        "virtual central is not connected".into(),
                    ));
                }
                return Ok(());
            };
            state.server.centrals.remove(position);
            dropped = state
                .server
                .subscriptions
                .iter()
                .filter_map(|(central, characteristic)| (central == id).then_some(*characteristic))
                .collect();
            state
                .server
                .subscriptions
                .retain(|(central, _)| central != id);
        }
        for characteristic in dropped {
            self.publish_server(GattServerEvent::SubscriptionChanged {
                central: id.clone(),
                characteristic,
                subscribed: false,
            });
        }
        self.publish_server(GattServerEvent::CentralDisconnected {
            central: id.clone(),
            reason,
        });
        Ok(())
    }

    pub(crate) fn virtual_read(
        &self,
        central: &DeviceId,
        id: &LocalCharacteristicId,
    ) -> Result<Vec<u8>, Error> {
        let value = {
            let state = self.lock();
            require_connected(&state.server, central)?;
            let characteristic = state.server.find_characteristic(id)?;
            if !characteristic
                .properties
                .contains(CharacteristicProperties::READ)
            {
                return Err(att(0x02));
            }
            characteristic.value.clone()
        };
        self.publish_server(GattServerEvent::CharacteristicRead {
            central: central.clone(),
            characteristic: *id,
        });
        Ok(value)
    }

    pub(crate) fn virtual_write(
        &self,
        central: &DeviceId,
        id: &LocalCharacteristicId,
        value: &[u8],
        mode: WriteMode,
    ) -> Result<(), Error> {
        {
            let mut state = self.lock();
            require_connected(&state.server, central)?;
            let characteristic = state.server.find_characteristic_mut(id)?;
            let permitted = match mode {
                WriteMode::WithResponse => characteristic
                    .properties
                    .contains(CharacteristicProperties::WRITE),
                WriteMode::WithoutResponse => characteristic
                    .properties
                    .contains(CharacteristicProperties::WRITE_WITHOUT_RESPONSE),
            };
            if !permitted {
                return Err(att(0x03));
            }
            if value.len() > characteristic.max_len {
                return Err(Error::InvalidLength {
                    len: value.len(),
                    max: characteristic.max_len,
                });
            }
            characteristic.value = value.to_vec();
        }
        self.publish_server(GattServerEvent::CharacteristicWrite {
            central: central.clone(),
            characteristic: *id,
            value: value.to_vec(),
            mode,
        });
        Ok(())
    }

    pub(crate) fn virtual_subscribe(
        &self,
        central: &DeviceId,
        id: &LocalCharacteristicId,
    ) -> Result<(), Error> {
        {
            let mut state = self.lock();
            require_connected(&state.server, central)?;
            let characteristic = state.server.find_characteristic(id)?;
            if !characteristic
                .properties
                .intersects(CharacteristicProperties::NOTIFY | CharacteristicProperties::INDICATE)
            {
                return Err(Error::InvalidState(
                    "local characteristic has neither NOTIFY nor INDICATE".into(),
                ));
            }
            state.server.subscriptions.insert((central.clone(), *id));
        }
        self.publish_server(GattServerEvent::SubscriptionChanged {
            central: central.clone(),
            characteristic: *id,
            subscribed: true,
        });
        Ok(())
    }

    pub(crate) fn virtual_unsubscribe(
        &self,
        central: &DeviceId,
        id: &LocalCharacteristicId,
    ) -> Result<(), Error> {
        {
            let mut state = self.lock();
            require_connected(&state.server, central)?;
            let _ = state.server.find_characteristic(id)?;
            state.server.subscriptions.remove(&(central.clone(), *id));
        }
        self.publish_server(GattServerEvent::SubscriptionChanged {
            central: central.clone(),
            characteristic: *id,
            subscribed: false,
        });
        Ok(())
    }

    pub(crate) fn virtual_notifications(
        &self,
        central: &DeviceId,
    ) -> Result<BoxStream<'static, (LocalCharacteristicId, Vec<u8>)>, Error> {
        let state = self.lock();
        require_connected(&state.server, central)?;
        let central = state
            .server
            .centrals
            .iter()
            .find(|entry| entry.id == *central)
            .expect("connected central is present");
        Ok(central.notifications.subscribe().boxed())
    }
}

fn require_connected(server: &ServerState, id: &DeviceId) -> Result<(), Error> {
    if server.is_connected(id) {
        Ok(())
    } else {
        Err(Error::NotConnected)
    }
}

fn att(code: u8) -> Error {
    Error::Att(AttError { code, handle: None })
}

/// Test control plane for a simulated remote central connected to the mock
/// GATT server.
///
/// The handle is only valid while the virtual central remains connected.
#[derive(Debug, Clone)]
pub struct MockVirtualCentral {
    shared: Arc<AdapterShared>,
    id: DeviceId,
}

impl MockVirtualCentral {
    pub(crate) fn new(shared: Arc<AdapterShared>, id: DeviceId) -> Self {
        Self { shared, id }
    }

    /// Identity of this virtual central.
    #[must_use]
    pub fn id(&self) -> &DeviceId {
        &self.id
    }

    /// Reads a local characteristic as this central, emitting a
    /// [`GattServerEvent::CharacteristicRead`].
    ///
    /// # Errors
    ///
    /// [`Error::NotConnected`] if this central has disconnected,
    /// [`Error::InvalidState`] if the characteristic is not registered,
    /// [`Error::Att`] if the characteristic is not readable.
    pub fn read(&self, id: &LocalCharacteristicId) -> Result<Vec<u8>, Error> {
        self.shared.virtual_read(&self.id, id)
    }

    /// Writes a local characteristic as this central, emitting a
    /// [`GattServerEvent::CharacteristicWrite`] after the value is stored.
    ///
    /// # Errors
    ///
    /// [`Error::NotConnected`], [`Error::InvalidState`], [`Error::Att`] when
    /// the write mode is not permitted, or [`Error::InvalidLength`].
    pub fn write(
        &self,
        id: &LocalCharacteristicId,
        value: &[u8],
        mode: WriteMode,
    ) -> Result<(), Error> {
        self.shared.virtual_write(&self.id, id, value, mode)
    }

    /// Subscribes this central to notifications or indications.
    ///
    /// # Errors
    ///
    /// [`Error::NotConnected`], [`Error::InvalidState`].
    pub fn subscribe(&self, id: &LocalCharacteristicId) -> Result<(), Error> {
        self.shared.virtual_subscribe(&self.id, id)
    }

    /// Unsubscribes this central from notifications or indications.
    ///
    /// # Errors
    ///
    /// [`Error::NotConnected`], [`Error::InvalidState`].
    pub fn unsubscribe(&self, id: &LocalCharacteristicId) -> Result<(), Error> {
        self.shared.virtual_unsubscribe(&self.id, id)
    }

    /// Stream of notification/indication payloads delivered to this central.
    ///
    /// # Errors
    ///
    /// [`Error::NotConnected`] if this central has disconnected.
    pub fn notifications(
        &self,
    ) -> Result<BoxStream<'static, (LocalCharacteristicId, Vec<u8>)>, Error> {
        self.shared.virtual_notifications(&self.id)
    }
}

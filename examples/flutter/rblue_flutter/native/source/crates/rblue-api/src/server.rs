//! Local GATT server types: registered services, identifiers, and events.
//!
//! These identifiers address attributes this adapter exposes as a peripheral.
//! They are not interchangeable with the client-side [`crate::ServiceId`] and
//! [`crate::CharacteristicId`] values returned by discovery. Identifiers are
//! valid until the local service is removed and must never be persisted.
//!
//! Application-level local descriptor CRUD is out of scope. Characteristics
//! with [`CharacteristicProperties::NOTIFY`] or
//! [`CharacteristicProperties::INDICATE`] have an implicit CCCD so a remote
//! central can subscribe.

use futures::stream::BoxStream;

use crate::device::DeviceId;
use crate::event::DisconnectReason;
use crate::gatt::{CharacteristicProperties, WriteMode};

/// Stream of [`GattServerEvent`] values.
///
/// Subscription semantics: subscribing snapshots currently connected centrals
/// (each is delivered as a fresh [`GattServerEvent::CentralConnected`]) and
/// then continues with live events. Dropping the stream unsubscribes.
pub type GattServerEventStream = BoxStream<'static, GattServerEvent>;

/// Declaration of a local GATT service to register with
/// [`crate::Adapter::add_local_service`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LocalService {
    /// Service UUID.
    pub uuid: uuid::Uuid,
    /// `true` for a primary service (the default), `false` for a secondary
    /// service.
    #[cfg_attr(feature = "serde", serde(default = "default_true"))]
    pub primary: bool,
    /// Characteristics contained in this service.
    #[cfg_attr(feature = "serde", serde(default))]
    pub characteristics: Vec<LocalCharacteristic>,
}

impl LocalService {
    /// A primary service with the given UUID and no characteristics yet.
    #[must_use]
    pub fn new(uuid: uuid::Uuid) -> Self {
        Self {
            uuid,
            primary: true,
            characteristics: Vec::new(),
        }
    }
}

/// Declaration of a local characteristic inside a [`LocalService`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LocalCharacteristic {
    /// Characteristic UUID.
    pub uuid: uuid::Uuid,
    /// Operations this characteristic permits, including whether a remote
    /// central may read, write, or subscribe.
    pub properties: CharacteristicProperties,
    /// Initial stored value.
    #[cfg_attr(feature = "serde", serde(default))]
    pub value: Vec<u8>,
    /// Maximum stored value length, in bytes. `None` lets the backend choose
    /// a default (typically the ATT write bound).
    #[cfg_attr(feature = "serde", serde(default))]
    pub max_len: Option<usize>,
}

impl LocalCharacteristic {
    /// A characteristic with an empty initial value and no explicit length
    /// bound.
    #[must_use]
    pub fn new(uuid: uuid::Uuid, properties: CharacteristicProperties) -> Self {
        Self {
            uuid,
            properties,
            value: Vec::new(),
            max_len: None,
        }
    }
}

/// Identity of a local service after it has been registered.
///
/// `instance` distinguishes multiple local services that share one UUID; it
/// is the zero-based index among currently registered services with that
/// UUID, assigned at registration. A `LocalServiceId` is only valid until
/// the service is removed and must never be persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LocalServiceId {
    /// Service UUID.
    pub uuid: uuid::Uuid,
    /// Zero-based index among same-UUID local services at registration.
    pub instance: u16,
}

/// Identity of a local characteristic after its service has been registered.
///
/// Like [`LocalServiceId`], this is only valid until the service is removed
/// and must never be persisted. It is not a client-side
/// [`crate::CharacteristicId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LocalCharacteristicId {
    /// The local service containing the characteristic.
    pub service: LocalServiceId,
    /// Characteristic UUID.
    pub uuid: uuid::Uuid,
    /// Zero-based index among same-UUID characteristics inside the service,
    /// in registration order. Almost always 0.
    pub instance: u16,
}

/// Snapshot of one registered local service, including current values.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LocalServiceSnapshot {
    /// Registration identity of this service.
    pub id: LocalServiceId,
    /// `true` for a primary service.
    pub primary: bool,
    /// Characteristics in registration order, with current stored values.
    pub characteristics: Vec<LocalCharacteristicSnapshot>,
}

/// Snapshot of one registered local characteristic, including its current
/// stored value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LocalCharacteristicSnapshot {
    /// Registration identity of this characteristic.
    pub id: LocalCharacteristicId,
    /// Operations the characteristic permits.
    pub properties: CharacteristicProperties,
    /// Current stored value.
    pub value: Vec<u8>,
    /// Maximum stored value length, in bytes.
    pub max_len: usize,
}

/// Events from the local GATT server about connected centrals and attribute
/// traffic.
///
/// This type is intentionally separate from [`crate::AdapterEvent`], which is
/// not marked `non_exhaustive` and must not gain new variants.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum GattServerEvent {
    /// A remote central connected to this peripheral.
    CentralConnected {
        /// Identity of the connecting central. Only valid for the current
        /// connection; must never be persisted.
        central: DeviceId,
    },
    /// A remote central disconnected.
    CentralDisconnected {
        /// Identity of the disconnected central.
        central: DeviceId,
        /// Why the connection dropped, when known.
        reason: Option<DisconnectReason>,
    },
    /// A remote central read a local characteristic. In this version the
    /// stored value is served automatically; the event is observational and
    /// the application cannot reject the read.
    CharacteristicRead {
        /// The central that issued the read.
        central: DeviceId,
        /// The characteristic that was read.
        characteristic: LocalCharacteristicId,
    },
    /// A remote central wrote a local characteristic. When the properties
    /// permit the write, the value is stored before this event is published.
    CharacteristicWrite {
        /// The central that issued the write.
        central: DeviceId,
        /// The characteristic that was written.
        characteristic: LocalCharacteristicId,
        /// The value that was written.
        value: Vec<u8>,
        /// Whether the write was acknowledged.
        mode: WriteMode,
    },
    /// A remote central subscribed or unsubscribed (implicit CCCD).
    SubscriptionChanged {
        /// The central that changed its subscription.
        central: DeviceId,
        /// The characteristic whose subscription changed.
        characteristic: LocalCharacteristicId,
        /// `true` when the central is now subscribed.
        subscribed: bool,
    },
}

#[cfg(feature = "serde")]
const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_service_new_is_primary_and_empty() {
        let uuid = uuid::Uuid::from_u128(0x180F);
        let service = LocalService::new(uuid);
        assert_eq!(service.uuid, uuid);
        assert!(service.primary);
        assert!(service.characteristics.is_empty());
    }
}

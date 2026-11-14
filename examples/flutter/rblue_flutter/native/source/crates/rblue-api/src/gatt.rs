//! GATT data model: services, characteristics, notifications, and the
//! session-scoped identifiers used to address them.

bitflags::bitflags! {
    /// GATT characteristic property bits (Bluetooth Core Spec, Vol 3 Part G
    /// 3.3.1.1 plus the Core Specification Supplement bits 8-9).
    ///
    /// Properties drive mode selection: notify vs. indicate chooses which
    /// platform API a backend must call (OHOS needs distinct
    /// notification/indication calls), and write mode (`WithResponse` needs
    /// `WRITE`, `WithoutResponse` needs `WRITE_WITHOUT_RESPONSE`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct CharacteristicProperties: u16 {
        /// 0x01: value may be broadcast in advertisements.
        const BROADCAST = 0x0001;
        /// 0x02: value may be read.
        const READ = 0x0002;
        /// 0x04: value may be written without a response (unacknowledged).
        const WRITE_WITHOUT_RESPONSE = 0x0004;
        /// 0x08: value may be written with a response (acknowledged).
        const WRITE = 0x0008;
        /// 0x10: value changes are notified (unacknowledged updates).
        const NOTIFY = 0x0010;
        /// 0x20: value changes are indicated (acknowledged updates).
        const INDICATE = 0x0020;
        /// 0x40: signed writes are supported.
        const AUTHENTICATED_SIGNED_WRITES = 0x0040;
        /// 0x80: extended properties live in the characteristic descriptor.
        const EXTENDED_PROPERTIES = 0x0080;
        /// 0x0100: reliable write (queued write) is supported.
        const RELIABLE_WRITE = 0x0100;
        /// 0x0200: the characteristic has writable auxiliaries.
        const WRITABLE_AUXILIARIES = 0x0200;
    }
}

/// One GATT service in a discovered tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Service {
    /// Service UUID.
    pub uuid: uuid::Uuid,
    /// `true` for a primary service, `false` for a secondary service.
    pub primary: bool,
    /// Characteristics contained in this service.
    pub characteristics: Vec<Characteristic>,
}

/// One GATT characteristic in a discovered tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Characteristic {
    /// Characteristic UUID.
    pub uuid: uuid::Uuid,
    /// Operations the characteristic permits; the basis for write mode and
    /// subscribe mechanism selection.
    pub properties: CharacteristicProperties,
    /// Descriptors declared by the characteristic. 1.0 only enumerates them;
    /// descriptor read/write is Phase 2.
    pub descriptors: Vec<uuid::Uuid>,
}

/// The discovered GATT tree of one connection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ServiceSet {
    /// All services returned by discovery.
    pub services: Vec<Service>,
}

/// Identity of a service within a connection.
///
/// `instance` distinguishes multiple services sharing one UUID; it is the
/// zero-based index among the services with that UUID, in the order
/// [`crate::Peripheral::discover_services`] returned them. Most devices have
/// exactly one instance per UUID (index 0).
///
/// Like [`CharacteristicId`], a `ServiceId` is only valid for the current
/// connection and must never be persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ServiceId {
    /// Service UUID.
    pub uuid: uuid::Uuid,
    /// Zero-based index among same-UUID services in discovery order.
    pub instance: u16,
}

/// Identity of a characteristic within one connected session.
///
/// The identifiers derive from connection-instance internals (object paths on
/// Windows/BlueZ, instance ids on Android), so they are **only valid for the
/// current connection and must never be persisted**. After a reconnect,
/// `discover_services` must run again and fresh identifiers be taken from its
/// result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CharacteristicId {
    /// The service containing the characteristic.
    pub service: ServiceId,
    /// Characteristic UUID.
    pub uuid: uuid::Uuid,
    /// Zero-based index among same-UUID characteristics inside the service,
    /// in discovery order. Almost always 0.
    pub instance: u16,
}

/// One GATT notification or indication payload.
///
/// The characteristic identity is included so a single broadcast stream can
/// serve every subscribed characteristic of the peripheral.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Notification {
    /// Which characteristic delivered the value.
    pub id: CharacteristicId,
    /// The value bytes.
    pub data: Vec<u8>,
}

/// Write mode for [`crate::Peripheral::write`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum WriteMode {
    /// Write with response (acknowledged, reliable; needs the `WRITE`
    /// property). The default mode.
    #[default]
    WithResponse,
    /// Write without response (unacknowledged, best-effort; needs the
    /// `WRITE_WITHOUT_RESPONSE` property, used for high-frequency small
    /// writes).
    WithoutResponse,
}

impl WriteMode {
    /// Returns `true` for the acknowledged [`WriteMode::WithResponse`].
    #[must_use]
    pub const fn with_response(self) -> bool {
        matches!(self, Self::WithResponse)
    }
}

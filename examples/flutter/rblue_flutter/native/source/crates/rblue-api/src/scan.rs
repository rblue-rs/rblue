//! Scan control and advertisement payloads.

use crate::device::DeviceId;

/// Filter applied to [`crate::Adapter::start_scan`].
///
/// Only [`ScanFilter::services`] can be pushed down to hardware filtering on
/// every platform: iOS/macOS support service-UUID hardware filters only, and
/// iOS background scanning silently yields nothing without one. The other
/// fields are software filters (the full advertisement stream is received and
/// then filtered - mind the power cost).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScanFilter {
    /// Keep only devices whose advertised local name starts with this prefix.
    pub name_prefix: Option<String>,
    /// Keep only devices advertising at least one of these service UUIDs.
    /// The only universally hardware-pushed-down filter field.
    pub services: Vec<uuid::Uuid>,
    /// Keep only devices whose manufacturer data matches this filter.
    pub manufacturer_data: Option<ManufacturerFilter>,
    /// Minimum RSSI in dBm; applied in software where the stack lacks support.
    pub rssi_threshold: Option<i16>,
}

/// Manufacturer-data filter: Bluetooth SIG company ID plus a data prefix.
///
/// Pushed down natively where the platform supports it; software-filtered
/// otherwise.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ManufacturerFilter {
    /// Bluetooth SIG company identifier.
    pub company_id: u16,
    /// The manufacturer-specific data must start with these bytes.
    pub data_prefix: Vec<u8>,
}

/// Parsed advertisement data for one device sighting.
///
/// The unified model is the *parsed* structure: iOS does not expose the raw AD
/// sequence and BlueZ over D-Bus only hands out parsed fields, so raw bytes
/// are intentionally not part of the contract.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AdvertisementData {
    /// Advertised local name, when included in the advertisement.
    pub local_name: Option<String>,
    /// Service UUIDs advertised.
    pub services: Vec<uuid::Uuid>,
    /// Service data entries: service UUID and its data.
    pub service_data: Vec<(uuid::Uuid, Vec<u8>)>,
    /// Manufacturer-specific data: company ID and data.
    pub manufacturer_data: Option<(u16, Vec<u8>)>,
    /// Advertised transmit power in dBm.
    pub tx_power: Option<i8>,
    /// Advertisement flags byte (LE-limited-discovery, BR/EDR-unsupported, ...).
    pub adv_flags: Option<u8>,
    /// Signal strength of this sighting in dBm.
    pub rssi: Option<i16>,
    /// Whether this advertisement is connectable. Available on iOS/Windows,
    /// `None` on platforms that do not report it.
    pub is_connectable: Option<bool>,
}

/// One device seen during a scan: its identity plus the latest parsed
/// advertisement.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DiscoveredDevice {
    /// Stable-per-session identifier of the device.
    pub id: DeviceId,
    /// Latest advertisement seen for the device.
    pub advertisement: AdvertisementData,
}

/// Options for [`crate::Adapter::request_device`] (web entry point).
///
/// On the web these map onto `navigator.bluetooth.requestDevice`:
/// `services` and `name_prefix` build the `filters` list,
/// `optional_services` becomes `optionalServices`, and `accept_all_devices`
/// switches to `acceptAllDevices` (mutually exclusive with any filter).
/// Other platforms return [`crate::Error::Unsupported`] for `request_device`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RequestDeviceOptions {
    /// Devices must advertise at least one of these service UUIDs.
    pub services: Vec<uuid::Uuid>,
    /// Devices whose advertised name starts with this prefix.
    pub name_prefix: Option<String>,
    /// Every service the caller may later use. On the web, services absent
    /// from this list (and from `services`) raise a `SecurityError` on
    /// `getPrimaryService`, so the facade layers merge `services` into it
    /// automatically.
    pub optional_services: Vec<uuid::Uuid>,
    /// Accept any device instead of filtering. Cannot be combined with
    /// `services`/`name_prefix` on the web.
    pub accept_all_devices: bool,
}

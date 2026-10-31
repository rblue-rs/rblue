//! Translation of btleplug fork values into the unified rblue model.
//!
//! The fork routes every BlueZ failure through `bluez_async::BluetoothError`
//! and flattens it into `btleplug::Error::Other`, so the mapping downcasts it
//! back out: D-Bus errors carry a *name* (`org.bluez.Error.*`) plus a message
//! instead of a numeric status code, and the name table below re-derives the
//! unified error - including the ATT error code bluetoothd encoded into the
//! message text. Names without a semantic equivalent keep the verbatim
//! `name: message` text in `Error::Backend`; this platform has no numeric
//! status codes, so `Error::Platform` is never produced. This table's
//! contract lives in `docs/error-mapping.md`.
//
// The downcast to `bluez_async::BluetoothError` only exists where the fork's
// bluez implementation exists; every other host exercises the pure string and
// D-Bus-name mapping functions (and their tests) below.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::collections::HashMap;

use btleplug::api::{CentralState, CharPropFlags, PeripheralProperties};
use rblue_api::device::{AddressType, DeviceId};
use rblue_api::error::{AttError, Error, PermissionReason};
use rblue_api::event::AdapterState;
use rblue_api::gatt::{
    Characteristic, CharacteristicId, CharacteristicProperties, Service, ServiceId, ServiceSet,
};
use rblue_api::scan::{AdvertisementData, ScanFilter};
use uuid::Uuid;

/// Platform name used in [`Error::Unsupported`] pairs.
pub(crate) const PLATFORM: &str = "bluez";

/// ATT status for addressing a stale or fabricated characteristic.
pub(crate) const ATT_INVALID_HANDLE: u8 = 0x01;
/// ATT status for reads the peer rejects (`Read Not Permitted`).
pub(crate) const ATT_READ_NOT_PERMITTED: u8 = 0x02;
/// ATT status for writes in a mode the property bits do not allow.
pub(crate) const ATT_WRITE_NOT_PERMITTED: u8 = 0x03;
/// ATT status for an attribute the peer does not have.
pub(crate) const ATT_ATTRIBUTE_NOT_FOUND: u8 = 0x0A;

/// `org.bluez.Error.*` names with a unified equivalent (BlueZ `src/error.c`).
const BLUEZ_PREFIX: &str = "org.bluez.Error.";
const BLUEZ_NOT_CONNECTED: &str = "org.bluez.Error.NotConnected";
const BLUEZ_NOT_PERMITTED: &str = "org.bluez.Error.NotPermitted";
const BLUEZ_NOT_AUTHORIZED: &str = "org.bluez.Error.NotAuthorized";
const BLUEZ_NOT_SUPPORTED: &str = "org.bluez.Error.NotSupported";
const BLUEZ_INVALID_OFFSET: &str = "org.bluez.Error.InvalidOffset";
const BLUEZ_INVALID_LENGTH: &str = "org.bluez.Error.InvalidLength";
const BLUEZ_DOES_NOT_EXIST: &str = "org.bluez.Error.DoesNotExist";
const BLUEZ_FAILED: &str = "org.bluez.Error.Failed";
const BLUEZ_CANCELED: &str = "org.bluez.Error.Canceled";
const BLUEZ_IN_PROGRESS: &str = "org.bluez.Error.InProgress";
const BLUEZ_NOT_READY: &str = "org.bluez.Error.NotReady";

/// D-Bus names BlueZ raises for `Device1.Connect()` on an already-connected
/// device (`AlreadyConnected` from the docs, `AlreadyExists` from the
/// implementation); the connect call site treats both as success.
const BLUEZ_ALREADY_CONNECTED_NAMES: [&str; 2] = [
    "org.bluez.Error.AlreadyConnected",
    "org.bluez.Error.AlreadyExists",
];

/// The unified error for stale or fabricated session identifiers: the
/// discovered tree cannot resolve them (invalid handle).
pub(crate) fn att_invalid_handle() -> Error {
    Error::Att(AttError {
        code: ATT_INVALID_HANDLE,
        handle: None,
    })
}

/// Maps a btleplug error into the unified model. Errors that carry a device
/// identity in the rblue model (`Error::NotFound`) are produced at the call
/// sites that know the id, not here.
pub(crate) fn map_btleplug_error(error: btleplug::Error) -> Error {
    match error {
        btleplug::Error::NotConnected => Error::NotConnected,
        btleplug::Error::TimedOut(_) => Error::Timeout,
        btleplug::Error::Other(source) => map_error_source(&*source),
        btleplug::Error::DeviceNotFound
        | btleplug::Error::PermissionDenied
        | btleplug::Error::UnexpectedCallback
        | btleplug::Error::UnexpectedCharacteristic
        | btleplug::Error::NoSuchCharacteristic
        | btleplug::Error::NoAdapterAvailable
        | btleplug::Error::NotSupported(_)
        | btleplug::Error::RuntimeError(_)
        | btleplug::Error::Uuid(_)
        | btleplug::Error::InvalidBDAddr(_) => Error::Backend(error.to_string()),
    }
}

/// Maps the fork's `Error::Other` payload. The BlueZ fork boxes either a
/// `bluez_async::BluetoothError` (every D-Bus round trip) or a plain string
/// (its own service/characteristic lookups, poisoned locks).
#[cfg(target_os = "linux")]
fn map_error_source(source: &(dyn std::error::Error + Send + Sync)) -> Error {
    if let Some(error) = source.downcast_ref::<bluez_async::BluetoothError>() {
        return map_bluetooth_error(error);
    }
    map_error_text(&source.to_string())
}

/// Host builds cannot name the fork's error type; the string path covers the
/// fork-authored messages, which is the shape the unit tests pin down.
#[cfg(not(target_os = "linux"))]
fn map_error_source(source: &(dyn std::error::Error + Send + Sync)) -> Error {
    map_error_text(&source.to_string())
}

/// Maps a `bluez_async::BluetoothError`. Structured failures keep their
/// meaning: D-Bus errors go through the name table, the service-discovery
/// wait timeout becomes `Error::Timeout`, and a missing UUID is a stale
/// session identifier (invalid handle).
#[cfg(target_os = "linux")]
fn map_bluetooth_error(error: &bluez_async::BluetoothError) -> Error {
    match error {
        bluez_async::BluetoothError::NoBluetoothAdapters => Error::Unsupported {
            platform: PLATFORM,
            feature: "adapter",
        },
        bluez_async::BluetoothError::DbusError(dbus_error) => {
            map_dbus_error(dbus_error.name(), dbus_error.message())
        }
        bluez_async::BluetoothError::ServiceDiscoveryTimedOut => Error::Timeout,
        bluez_async::BluetoothError::UuidNotFound { .. } => att_invalid_handle(),
        bluez_async::BluetoothError::XmlParseError(_)
        | bluez_async::BluetoothError::UuidParseError(_)
        | bluez_async::BluetoothError::FlagParseError(_)
        | bluez_async::BluetoothError::AddressTypeParseError(_)
        | bluez_async::BluetoothError::RequiredPropertyMissing(_)
        | bluez_async::BluetoothError::MacAddressParseError(_)
        | bluez_async::BluetoothError::ModaliasParseError(_)
        | bluez_async::BluetoothError::DeviceIdParseError(_) => Error::Backend(error.to_string()),
    }
}

/// Returns `true` when the error is BlueZ rejecting `Device1.Connect()` on a
/// device that is already connected.
#[cfg(target_os = "linux")]
pub(crate) fn is_already_connected(error: &btleplug::Error) -> bool {
    let btleplug::Error::Other(source) = error else {
        return false;
    };
    source
        .downcast_ref::<bluez_async::BluetoothError>()
        .is_some_and(|bluetooth_error| match bluetooth_error {
            bluez_async::BluetoothError::DbusError(dbus_error) => {
                already_connected_name(dbus_error.name())
            }
            _ => false,
        })
}

/// The already-connected check by D-Bus name, split out so it stays testable
/// on hosts that cannot name the fork's error types.
pub(crate) fn already_connected_name(name: Option<&str>) -> bool {
    name.is_some_and(|name| BLUEZ_ALREADY_CONNECTED_NAMES.contains(&name))
}

/// Maps a D-Bus error by its `name`/`message` pair - the pure core of the
/// platform error mapping (D-Bus errors carry names, not numeric codes).
///
/// bluetoothd reports peer ATT errors as dedicated D-Bus names and encodes
/// the ATT error in the message (`btd_error_att_code`), so the table reads
/// both: the name narrows the family, the message restores the exact ATT
/// code where bluetoothd does not give it a dedicated name (`Failed`).
pub(crate) fn map_dbus_error(name: Option<&str>, message: Option<&str>) -> Error {
    let message = message.unwrap_or_default();
    match name {
        Some(BLUEZ_NOT_CONNECTED) => Error::NotConnected,
        Some(BLUEZ_NOT_PERMITTED) => match att_error_from_message(message) {
            Some(code @ (ATT_READ_NOT_PERMITTED | ATT_WRITE_NOT_PERMITTED)) => {
                Error::Att(AttError { code, handle: None })
            }
            _ => Error::Backend(format!("{BLUEZ_NOT_PERMITTED}: {message}")),
        },
        Some(BLUEZ_NOT_AUTHORIZED) => match message.to_ascii_lowercase().as_str() {
            "insufficient authentication" => Error::Att(AttError {
                code: 0x05,
                handle: None,
            }),
            "insufficient authorization" => Error::Att(AttError {
                code: 0x08,
                handle: None,
            }),
            "insufficient key size" => Error::Att(AttError {
                code: 0x0C,
                handle: None,
            }),
            "insufficient encryption" => Error::Att(AttError {
                code: 0x0F,
                handle: None,
            }),
            _ => Error::Backend(format!("{BLUEZ_NOT_AUTHORIZED}: {message}")),
        },
        Some(BLUEZ_NOT_SUPPORTED) => {
            if message.eq_ignore_ascii_case("unsupported request") {
                Error::Att(AttError {
                    code: 0x06,
                    handle: None,
                })
            } else {
                Error::Backend(format!("{BLUEZ_NOT_SUPPORTED}: {message}"))
            }
        }
        Some(BLUEZ_INVALID_OFFSET) => Error::Att(AttError {
            code: 0x07,
            handle: None,
        }),
        Some(BLUEZ_INVALID_LENGTH) => Error::Att(AttError {
            code: 0x0D,
            handle: None,
        }),
        Some(BLUEZ_DOES_NOT_EXIST) => {
            if message.eq_ignore_ascii_case("attribute not found") {
                Error::Att(AttError {
                    code: ATT_ATTRIBUTE_NOT_FOUND,
                    handle: None,
                })
            } else {
                Error::Backend(format!("{BLUEZ_DOES_NOT_EXIST}: {message}"))
            }
        }
        Some(BLUEZ_FAILED) => match att_error_from_message(message) {
            Some(code) => Error::Att(AttError { code, handle: None }),
            None => Error::Backend(format!("{BLUEZ_FAILED}: {message}")),
        },
        Some(BLUEZ_CANCELED) => {
            // BlueZ cancels in-flight operations when the link drops.
            Error::Disconnected
        }
        Some(BLUEZ_IN_PROGRESS) => {
            Error::InvalidState(format!("operation already in progress: {message}"))
        }
        Some(BLUEZ_NOT_READY) => {
            // The adapter is powered down or bluetoothd is not ready yet;
            // the unified model's closest expected state is a radio that is
            // off (see docs/platform-notes.md).
            Error::Permission(PermissionReason::BluetoothOff)
        }
        Some(name) => Error::Backend(format!("{name}: {message}")),
        None => Error::Backend(format!("dbus error without a name: {message}")),
    }
}

/// Restores the ATT error code from bluetoothd's message text for the names
/// that do not carry a dedicated D-Bus error (`org.bluez.Error.Failed`), and
/// distinguishes read from write for `NotPermitted`. Matches the
/// `btd_error_att_code` message strings case-insensitively.
fn att_error_from_message(message: &str) -> Option<u8> {
    let message = message.to_ascii_lowercase();
    Some(match message.as_str() {
        "invalid handle" => ATT_INVALID_HANDLE,
        "read not permitted" => ATT_READ_NOT_PERMITTED,
        "write not permitted" => ATT_WRITE_NOT_PERMITTED,
        "invalid pdu" => 0x04,
        "invalid offset" => 0x07,
        "prepare queue full" => 0x09,
        "attribute not found" => ATT_ATTRIBUTE_NOT_FOUND,
        "attribute not long" => 0x0B,
        "invalid attribute length" => 0x0D,
        "unlikely error" => 0x0E,
        "insufficient resources" => 0x11,
        _ => return None,
    })
}

/// Normalizes the fork-authored string errors: its per-UUID service,
/// characteristic, and descriptor lookups report stale or fabricated session
/// identifiers as `"... with UUID {uuid} not found."` (same treatment as the
/// mock's invalid-handle errors); everything else has no structured code.
fn map_error_text(message: &str) -> Error {
    for prefix in [
        "Service with UUID ",
        "Characteristic with UUID ",
        "Descriptor with UUID ",
    ] {
        if message.starts_with(prefix) && message.ends_with(" not found.") {
            return att_invalid_handle();
        }
    }
    Error::Backend(message.to_owned())
}

/// Maps the central state reported by the fork into the unified model.
///
/// BlueZ only reports a powered flag, so `Unknown` never surfaces from this
/// fork; the arm keeps the mapping total.
pub(crate) fn map_central_state(state: CentralState) -> AdapterState {
    match state {
        CentralState::PoweredOn => AdapterState::PoweredOn,
        CentralState::PoweredOff => AdapterState::PoweredOff,
        CentralState::Unknown => AdapterState::Unknown,
    }
}

/// Builds the session [`DeviceId`] from a BlueZ address.
///
/// BlueZ reports only `public` or `random` (`AddressType` D-Bus property);
/// the random sub-type is refined from the two most significant bits of the
/// address (BLE Core Spec Vol 6 Part B 1.3 address header, carried by the
/// most significant octet): `01` is a resolvable private address, `11` a
/// static random address, the rest stays [`AddressType::Unknown`]. A sighting
/// without a reported type stays `Unknown` as well.
pub(crate) fn device_id_from(
    bytes: [u8; 6],
    reported: Option<btleplug::api::AddressType>,
) -> DeviceId {
    let address_type = match reported {
        Some(btleplug::api::AddressType::Public) => AddressType::Public,
        Some(btleplug::api::AddressType::Random) => random_address_type(bytes),
        None => AddressType::Unknown,
    };
    DeviceId::Mac(bytes, address_type)
}

fn random_address_type(bytes: [u8; 6]) -> AddressType {
    match bytes[5] >> 6 {
        0b01 => AddressType::ResolvablePrivate,
        0b11 => AddressType::RandomStatic,
        _ => AddressType::Unknown,
    }
}

/// Converts btleplug characteristic property bits. The low byte is identical
/// between the two models; rblue's Phase-2 bits are never set by this
/// backend.
pub(crate) fn map_properties(flags: CharPropFlags) -> CharacteristicProperties {
    CharacteristicProperties::from_bits_truncate(flags.bits() as u16)
}

/// Converts rblue property bits back into the fork's representation when
/// submitting GATT operations.
pub(crate) fn properties_to_flags(properties: CharacteristicProperties) -> CharPropFlags {
    CharPropFlags::from_bits_truncate(properties.bits() as u8)
}

/// Maximum write payload for the negotiated ATT MTU, minus the 3-byte ATT
/// header. BlueZ negotiates one ATT MTU per connection for both write modes,
/// so the mode does not change the limit. Before the first
/// `discover_services` (and on BlueZ releases without the characteristic
/// `MTU` property) the fork reports the ATT default of 23, so the
/// conservative limit is 20.
pub(crate) fn max_write_len_from_mtu(mtu: u16) -> usize {
    usize::from(mtu).saturating_sub(3)
}

/// Builds the parsed advertisement snapshot from the fork's device
/// properties.
///
/// BlueZ consolidates advertisement and scan response into cumulative device
/// properties and exposes no advertisement flags and no per-sighting
/// advertisement type, so `adv_flags` and `is_connectable` stay `None`. When
/// several manufacturer data entries are cached, the smallest company id is
/// reported to keep the single-entry model deterministic; service data is
/// sorted by UUID for the same reason.
pub(crate) fn advertisement_from_properties(
    properties: &PeripheralProperties,
) -> AdvertisementData {
    AdvertisementData {
        local_name: properties.local_name.clone(),
        services: properties.services.clone(),
        service_data: service_data_from(&properties.service_data),
        manufacturer_data: properties
            .manufacturer_data
            .iter()
            .min_by_key(|(company_id, _)| *company_id)
            .map(|(company_id, data)| (*company_id, data.clone())),
        tx_power: properties
            .tx_power_level
            .and_then(|power| i8::try_from(power).ok()),
        adv_flags: None,
        rssi: properties.rssi,
        is_connectable: None,
    }
}

/// Deterministic `service_data` ordering: sorted by UUID.
fn service_data_from(service_data: &HashMap<Uuid, Vec<u8>>) -> Vec<(Uuid, Vec<u8>)> {
    let mut entries: Vec<(Uuid, Vec<u8>)> = service_data
        .iter()
        .map(|(uuid, data)| (*uuid, data.clone()))
        .collect();
    entries.sort_unstable_by_key(|(uuid, _)| *uuid);
    entries
}

/// Replaces the cached manufacturer data with a `PropertiesChanged` value.
/// BlueZ delivers the complete new map, so this is a full replacement.
pub(crate) fn merge_manufacturer_data(
    advertisement: &mut AdvertisementData,
    manufacturer_data: &HashMap<u16, Vec<u8>>,
) {
    advertisement.manufacturer_data = manufacturer_data
        .iter()
        .min_by_key(|(company_id, _)| *company_id)
        .map(|(company_id, data)| (*company_id, data.clone()));
}

/// Replaces the cached service data with a `PropertiesChanged` value.
pub(crate) fn merge_service_data(
    advertisement: &mut AdvertisementData,
    service_data: &HashMap<Uuid, Vec<u8>>,
) {
    advertisement.service_data = service_data_from(service_data);
}

/// Replaces the cached advertised service UUIDs with a `PropertiesChanged`
/// value.
pub(crate) fn merge_services(advertisement: &mut AdvertisementData, services: &[Uuid]) {
    advertisement.services = services.to_vec();
}

/// Software part of the scan filter.
///
/// BlueZ pushes `services` down through the discovery filter
/// (`SetDiscoveryFilter`), so non-matching devices are not reported at all;
/// `name_prefix`, `manufacturer_data`, and `rssi_threshold` filter in
/// software on the full advertisement stream. Semantics match the mock
/// backend's `matches_scan_filter`.
pub(crate) fn matches_filter(
    advertisement: &AdvertisementData,
    filter: Option<&ScanFilter>,
) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    if let Some(prefix) = &filter.name_prefix {
        if !advertisement
            .local_name
            .as_deref()
            .is_some_and(|name| name.starts_with(prefix))
        {
            return false;
        }
    }
    if !filter.services.is_empty()
        && !filter
            .services
            .iter()
            .any(|uuid| advertisement.services.contains(uuid))
    {
        return false;
    }
    if let Some(expected) = &filter.manufacturer_data {
        match &advertisement.manufacturer_data {
            Some((company_id, data))
                if *company_id == expected.company_id
                    && data.starts_with(&expected.data_prefix) => {}
            _ => return false,
        }
    }
    if let Some(threshold) = filter.rssi_threshold {
        if advertisement.rssi.is_some_and(|rssi| rssi < threshold) {
            return false;
        }
    }
    true
}

/// Converts the fork's discovered tree into the unified model, restricted to
/// `filter` (empty filter keeps everything). Instance indices are derived at
/// resolution time by counting same-UUID entries in the set order; the fork
/// collapses same-UUID services and characteristics (its maps are keyed by
/// UUID), so duplicates are not addressable and indices are 0 in practice.
pub(crate) fn service_set_from_btleplug(
    discovered: &std::collections::BTreeSet<btleplug::api::Service>,
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
    use std::collections::HashMap;
    use uuid::uuid;

    fn other(message: &str) -> btleplug::Error {
        btleplug::Error::Other(message.to_owned().into())
    }

    fn backend_text(error: &Error) -> &str {
        match error {
            Error::Backend(text) => text,
            other => panic!("expected a backend error, got {other:?}"),
        }
    }

    fn att_code(error: &Error) -> u8 {
        match error {
            Error::Att(att) => att.code,
            other => panic!("expected an ATT error, got {other:?}"),
        }
    }

    #[test]
    fn structured_variants_preserve_meaning() {
        assert!(matches!(
            map_btleplug_error(btleplug::Error::NotConnected),
            Error::NotConnected
        ));
        assert!(matches!(
            map_btleplug_error(btleplug::Error::TimedOut(std::time::Duration::from_secs(1))),
            Error::Timeout
        ));
    }

    #[test]
    fn unstructured_variants_stay_backend() {
        let variants: [fn() -> btleplug::Error; 5] = [
            || btleplug::Error::DeviceNotFound,
            || btleplug::Error::PermissionDenied,
            || btleplug::Error::UnexpectedCallback,
            || btleplug::Error::NoSuchCharacteristic,
            || btleplug::Error::NotSupported("Can't add a Peripheral".to_owned()),
        ];
        for make_error in variants {
            let error = make_error();
            let debug = format!("{error:?}");
            assert!(
                matches!(map_btleplug_error(error), Error::Backend(_)),
                "unexpected mapping for {debug}"
            );
        }
    }

    #[test]
    fn not_connected_name_maps_to_not_connected() {
        assert!(matches!(
            map_dbus_error(Some(BLUEZ_NOT_CONNECTED), Some("Not connected")),
            Error::NotConnected
        ));
    }

    #[test]
    fn not_permitted_name_restores_the_att_direction() {
        assert_eq!(
            att_code(&map_dbus_error(
                Some(BLUEZ_NOT_PERMITTED),
                Some("Read Not Permitted")
            )),
            ATT_READ_NOT_PERMITTED
        );
        assert_eq!(
            att_code(&map_dbus_error(
                Some(BLUEZ_NOT_PERMITTED),
                Some("Write Not Permitted")
            )),
            ATT_WRITE_NOT_PERMITTED
        );
    }

    #[test]
    fn non_att_not_permitted_stays_backend_with_the_name() {
        let error = map_dbus_error(Some(BLUEZ_NOT_PERMITTED), Some("Operation not permitted"));
        assert_eq!(
            backend_text(&error),
            "org.bluez.Error.NotPermitted: Operation not permitted"
        );
    }

    #[test]
    fn not_authorized_name_restores_att_codes() {
        let cases = [
            ("Insufficient Authentication", 0x05),
            ("Insufficient Authorization", 0x08),
            ("Insufficient Key Size", 0x0C),
            ("Insufficient Encryption", 0x0F),
        ];
        for (message, code) in cases {
            assert_eq!(
                att_code(&map_dbus_error(Some(BLUEZ_NOT_AUTHORIZED), Some(message))),
                code,
                "unexpected code for {message}"
            );
        }
        let error = map_dbus_error(Some(BLUEZ_NOT_AUTHORIZED), Some("Not authorized"));
        assert_eq!(
            backend_text(&error),
            "org.bluez.Error.NotAuthorized: Not authorized"
        );
    }

    #[test]
    fn att_family_names_map_directly() {
        assert_eq!(
            att_code(&map_dbus_error(
                Some(BLUEZ_NOT_SUPPORTED),
                Some("Unsupported Request")
            )),
            0x06
        );
        assert_eq!(
            att_code(&map_dbus_error(
                Some(BLUEZ_INVALID_OFFSET),
                Some("Invalid Offset")
            )),
            0x07
        );
        assert_eq!(
            att_code(&map_dbus_error(
                Some(BLUEZ_INVALID_LENGTH),
                Some("Invalid Attribute length")
            )),
            0x0D
        );
        assert_eq!(
            att_code(&map_dbus_error(
                Some(BLUEZ_DOES_NOT_EXIST),
                Some("Attribute Not Found")
            )),
            ATT_ATTRIBUTE_NOT_FOUND
        );
    }

    #[test]
    fn failed_name_recovers_att_codes_from_messages() {
        let cases = [
            ("Invalid Handle", 0x01),
            ("Invalid PDU", 0x04),
            ("Prepare Queue Full", 0x09),
            ("Attribute Not Long", 0x0B),
            ("Unlikely Error", 0x0E),
            ("Insufficient Resources", 0x11),
        ];
        for (message, code) in cases {
            assert_eq!(
                att_code(&map_dbus_error(Some(BLUEZ_FAILED), Some(message))),
                code,
                "unexpected code for {message}"
            );
        }
    }

    #[test]
    fn failed_name_without_att_message_stays_backend() {
        let error = map_dbus_error(Some(BLUEZ_FAILED), Some("Software caused connection abort"));
        assert_eq!(
            backend_text(&error),
            "org.bluez.Error.Failed: Software caused connection abort"
        );
    }

    #[test]
    fn canceled_maps_to_disconnected_and_in_progress_to_invalid_state() {
        assert!(matches!(
            map_dbus_error(Some(BLUEZ_CANCELED), Some("Operation canceled")),
            Error::Disconnected
        ));
        assert!(matches!(
            map_dbus_error(Some(BLUEZ_IN_PROGRESS), Some("Already discovering")),
            Error::InvalidState(_)
        ));
    }

    #[test]
    fn not_ready_maps_to_bluetooth_off() {
        assert!(matches!(
            map_dbus_error(Some(BLUEZ_NOT_READY), Some("Adapter is not ready")),
            Error::Permission(PermissionReason::BluetoothOff)
        ));
    }

    #[test]
    fn unknown_and_nameless_dbus_errors_keep_the_name() {
        let error = map_dbus_error(
            Some("org.freedesktop.DBus.Error.ServiceUnknown"),
            Some("The name org.bluez was not provided by any .service files"),
        );
        assert_eq!(
            backend_text(&error),
            "org.freedesktop.DBus.Error.ServiceUnknown: \
             The name org.bluez was not provided by any .service files"
        );
        let error = map_dbus_error(None, Some("no error"));
        assert_eq!(backend_text(&error), "dbus error without a name: no error");
    }

    #[test]
    fn already_connected_names_are_recognized() {
        assert!(already_connected_name(Some(
            "org.bluez.Error.AlreadyConnected"
        )));
        assert!(already_connected_name(Some(
            "org.bluez.Error.AlreadyExists"
        )));
        assert!(!already_connected_name(Some("org.bluez.Error.Failed")));
        assert!(!already_connected_name(None));
    }

    #[test]
    fn missing_service_or_characteristic_is_invalid_handle() {
        for message in [
            "Service with UUID 0000180f-0000-1000-8000-00805f9b34fb not found.",
            "Characteristic with UUID 00002a37-0000-1000-8000-00805f9b34fb not found.",
            "Descriptor with UUID 00002902-0000-1000-8000-00805f9b34fb not found.",
        ] {
            assert!(
                matches!(
                    map_btleplug_error(other(message)),
                    Error::Att(AttError {
                        code: 0x01,
                        handle: None
                    })
                ),
                "unexpected mapping for {message}"
            );
        }
    }

    #[test]
    fn fork_texts_without_known_shapes_stay_backend() {
        for message in [
            "PoisonError { .. }",
            "Characteristic with UUID 00002a37-0000-1000-8000-00805f9b34fb not found",
        ] {
            assert!(
                matches!(map_btleplug_error(other(message)), Error::Backend(text) if text == message),
                "unexpected mapping for {message}"
            );
        }
    }

    #[test]
    fn central_state_maps_to_unified_states() {
        assert_eq!(
            map_central_state(CentralState::PoweredOn),
            AdapterState::PoweredOn
        );
        assert_eq!(
            map_central_state(CentralState::PoweredOff),
            AdapterState::PoweredOff
        );
        assert_eq!(
            map_central_state(CentralState::Unknown),
            AdapterState::Unknown
        );
    }

    #[test]
    fn public_addresses_keep_the_public_type() {
        let id = device_id_from(
            [0x00, 0x1A, 0x7D, 0xDA, 0x71, 0x13],
            Some(btleplug::api::AddressType::Public),
        );
        assert_eq!(
            id,
            DeviceId::Mac([0x00, 0x1A, 0x7D, 0xDA, 0x71, 0x13], AddressType::Public)
        );
    }

    #[test]
    fn random_addresses_are_refined_by_header_bits() {
        // Top two bits 11: static random.
        assert_eq!(
            device_id_from(
                [1, 2, 3, 4, 5, 0xC1],
                Some(btleplug::api::AddressType::Random)
            ),
            DeviceId::Mac([1, 2, 3, 4, 5, 0xC1], AddressType::RandomStatic)
        );
        // Top two bits 01: resolvable private (RPA).
        assert_eq!(
            device_id_from(
                [1, 2, 3, 4, 5, 0x5A],
                Some(btleplug::api::AddressType::Random)
            ),
            DeviceId::Mac([1, 2, 3, 4, 5, 0x5A], AddressType::ResolvablePrivate)
        );
        // Top two bits 00 (non-resolvable private) and 10 (reserved) stay
        // unknown.
        assert_eq!(
            device_id_from(
                [1, 2, 3, 4, 5, 0x0A],
                Some(btleplug::api::AddressType::Random)
            ),
            DeviceId::Mac([1, 2, 3, 4, 5, 0x0A], AddressType::Unknown)
        );
        assert_eq!(
            device_id_from(
                [1, 2, 3, 4, 5, 0x8A],
                Some(btleplug::api::AddressType::Random)
            ),
            DeviceId::Mac([1, 2, 3, 4, 5, 0x8A], AddressType::Unknown)
        );
    }

    #[test]
    fn unreported_address_types_stay_unknown() {
        assert_eq!(
            device_id_from([1, 2, 3, 4, 5, 6], None),
            DeviceId::Mac([1, 2, 3, 4, 5, 6], AddressType::Unknown)
        );
    }

    #[test]
    fn property_bits_convert_losslessly() {
        let flags = CharPropFlags::READ | CharPropFlags::WRITE | CharPropFlags::NOTIFY;
        let properties = map_properties(flags);
        assert_eq!(
            properties,
            CharacteristicProperties::READ
                | CharacteristicProperties::WRITE
                | CharacteristicProperties::NOTIFY
        );
        assert_eq!(properties_to_flags(properties), flags);
    }

    #[test]
    fn max_write_len_subtracts_the_att_header() {
        // ATT default MTU before any renegotiation (and on stacks without the
        // MTU property): 23 - 3 = 20.
        assert_eq!(max_write_len_from_mtu(23), 20);
        assert_eq!(max_write_len_from_mtu(247), 244);
        // Defensive: an implausibly small MTU never underflows.
        assert_eq!(max_write_len_from_mtu(2), 0);
    }

    #[test]
    fn advertisement_is_built_deterministically() {
        let mut manufacturer_data = HashMap::new();
        manufacturer_data.insert(0x004C, vec![1, 2]);
        manufacturer_data.insert(0x00E0, vec![3]);
        let mut service_data = HashMap::new();
        service_data.insert(Uuid::from_u128(0x180F), vec![7]);
        service_data.insert(Uuid::from_u128(0x180D), vec![5]);
        let properties = PeripheralProperties {
            local_name: Some("sensor".to_owned()),
            services: vec![Uuid::from_u128(0x180F)],
            manufacturer_data,
            service_data,
            rssi: Some(-40),
            tx_power_level: Some(4),
            ..PeripheralProperties::default()
        };
        let advertisement = advertisement_from_properties(&properties);
        assert_eq!(advertisement.local_name.as_deref(), Some("sensor"));
        assert_eq!(advertisement.manufacturer_data, Some((0x004C, vec![1, 2])));
        assert_eq!(
            advertisement.service_data,
            vec![
                (Uuid::from_u128(0x180D), vec![5]),
                (Uuid::from_u128(0x180F), vec![7])
            ]
        );
        assert_eq!(advertisement.rssi, Some(-40));
        assert_eq!(advertisement.tx_power, Some(4));
        assert_eq!(advertisement.adv_flags, None);
        assert_eq!(advertisement.is_connectable, None);
    }

    #[test]
    fn out_of_range_tx_power_is_dropped() {
        let properties = PeripheralProperties {
            tx_power_level: Some(300),
            ..PeripheralProperties::default()
        };
        assert_eq!(advertisement_from_properties(&properties).tx_power, None);
    }

    #[test]
    fn property_updates_replace_the_cached_fields() {
        let mut advertisement = AdvertisementData {
            rssi: Some(-60),
            ..AdvertisementData::default()
        };

        let mut manufacturer_data = HashMap::new();
        manufacturer_data.insert(0x0059, vec![9]);
        merge_manufacturer_data(&mut advertisement, &manufacturer_data);
        assert_eq!(advertisement.manufacturer_data, Some((0x0059, vec![9])));
        // A cleared property arrives as an empty map.
        merge_manufacturer_data(&mut advertisement, &HashMap::new());
        assert_eq!(advertisement.manufacturer_data, None);

        let mut service_data = HashMap::new();
        service_data.insert(Uuid::from_u128(0x180F), vec![7]);
        merge_service_data(&mut advertisement, &service_data);
        assert_eq!(
            advertisement.service_data,
            vec![(Uuid::from_u128(0x180F), vec![7])]
        );

        let services = [Uuid::from_u128(0x180F), Uuid::from_u128(0x180D)];
        merge_services(&mut advertisement, &services);
        assert_eq!(advertisement.services, services.to_vec());

        // RSSI keeps its cached value across the other merges.
        assert_eq!(advertisement.rssi, Some(-60));
    }

    fn advertisement(rssi: i16, name: &str) -> AdvertisementData {
        AdvertisementData {
            local_name: Some(name.to_owned()),
            rssi: Some(rssi),
            ..AdvertisementData::default()
        }
    }

    #[test]
    fn name_prefix_filter_is_applied_in_software() {
        let filter = ScanFilter {
            name_prefix: Some("loud".to_owned()),
            ..ScanFilter::default()
        };
        assert!(matches_filter(
            &advertisement(-40, "loud-device"),
            Some(&filter)
        ));
        assert!(!matches_filter(
            &advertisement(-40, "quiet-device"),
            Some(&filter)
        ));
    }

    #[test]
    fn rssi_threshold_filter_is_applied_in_software() {
        let filter = ScanFilter {
            rssi_threshold: Some(-50),
            ..ScanFilter::default()
        };
        assert!(matches_filter(&advertisement(-40, "any"), Some(&filter)));
        assert!(!matches_filter(&advertisement(-60, "any"), Some(&filter)));
        // A sighting without RSSI passes the threshold check: the platform
        // did not report a value to compare against.
        let mut no_rssi = advertisement(-40, "any");
        no_rssi.rssi = None;
        assert!(matches_filter(&no_rssi, Some(&filter)));
    }

    #[test]
    fn service_and_manufacturer_filters_match_mock_semantics() {
        let battery = uuid!("0000180f-0000-1000-8000-00805f9b34fb");
        let mut adv = advertisement(-40, "any");
        adv.services = vec![battery];
        adv.manufacturer_data = Some((0x004C, vec![1, 2, 3]));

        let other_service = ScanFilter {
            services: vec![uuid!("0000fd6f-0000-1000-8000-00805f9b34fb")],
            ..ScanFilter::default()
        };
        assert!(!matches_filter(&adv, Some(&other_service)));
        let same_service = ScanFilter {
            services: vec![battery],
            ..ScanFilter::default()
        };
        assert!(matches_filter(&adv, Some(&same_service)));

        let matching_manufacturer = ScanFilter {
            manufacturer_data: Some(rblue_api::ManufacturerFilter {
                company_id: 0x004C,
                data_prefix: vec![1, 2],
            }),
            ..ScanFilter::default()
        };
        assert!(matches_filter(&adv, Some(&matching_manufacturer)));
        let mismatching_manufacturer = ScanFilter {
            manufacturer_data: Some(rblue_api::ManufacturerFilter {
                company_id: 0x004C,
                data_prefix: vec![9],
            }),
            ..ScanFilter::default()
        };
        assert!(!matches_filter(&adv, Some(&mismatching_manufacturer)));
    }

    #[test]
    fn empty_filter_matches_everything() {
        assert!(matches_filter(&advertisement(-100, "any"), None));
        assert!(matches_filter(
            &advertisement(-100, "any"),
            Some(&ScanFilter::default())
        ));
    }

    #[test]
    fn service_instances_count_same_uuid_services() {
        // Two distinguishable services sharing one UUID (primary vs.
        // secondary keeps the BTreeSet from deduplicating them).
        let primary = btleplug::api::Service {
            uuid: uuid!("0000180d-0000-1000-8000-00805f9b34fb"),
            primary: true,
            characteristics: std::collections::BTreeSet::new(),
        };
        let secondary = btleplug::api::Service {
            primary: false,
            ..primary.clone()
        };
        let mut discovered = std::collections::BTreeSet::new();
        discovered.insert(primary);
        discovered.insert(secondary);

        let set = service_set_from_btleplug(&discovered, &[]);
        assert_eq!(set.services.len(), 2);
        assert!(set
            .services
            .iter()
            .all(|service| service.uuid == uuid!("0000180d-0000-1000-8000-00805f9b34fb")));
    }

    #[test]
    fn service_filter_keeps_matching_subset() {
        let heart_rate = btleplug::api::Service {
            uuid: uuid!("0000180d-0000-1000-8000-00805f9b34fb"),
            primary: true,
            characteristics: std::collections::BTreeSet::new(),
        };
        let battery = btleplug::api::Service {
            uuid: uuid!("0000180f-0000-1000-8000-00805f9b34fb"),
            primary: true,
            characteristics: std::collections::BTreeSet::new(),
        };
        let mut discovered = std::collections::BTreeSet::new();
        discovered.insert(heart_rate);
        discovered.insert(battery);

        let set = service_set_from_btleplug(
            &discovered,
            &[uuid!("0000180d-0000-1000-8000-00805f9b34fb")],
        );
        assert_eq!(set.services.len(), 1);
        assert_eq!(
            set.services[0].uuid,
            uuid!("0000180d-0000-1000-8000-00805f9b34fb")
        );

        let set = service_set_from_btleplug(&discovered, &[]);
        assert_eq!(set.services.len(), 2);
    }

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

    #[cfg(target_os = "linux")]
    mod linux {
        use super::*;

        fn dbus_error(name: &str, message: &str) -> btleplug::Error {
            btleplug::Error::Other(Box::new(bluez_async::BluetoothError::DbusError(
                dbus::Error::new_custom(name, message),
            )))
        }

        #[test]
        fn dbus_errors_round_trip_through_the_fork_type() {
            assert!(matches!(
                map_btleplug_error(dbus_error(BLUEZ_NOT_CONNECTED, "Not connected")),
                Error::NotConnected
            ));
            assert!(matches!(
                map_btleplug_error(dbus_error(
                    "org.freedesktop.DBus.Error.ServiceUnknown",
                    "The name org.bluez was not provided by any .service files"
                )),
                Error::Backend(_)
            ));
        }

        #[test]
        fn already_connected_is_detected_through_the_fork_type() {
            assert!(is_already_connected(&dbus_error(
                "org.bluez.Error.AlreadyExists",
                "Already Exists"
            )));
            assert!(!is_already_connected(&dbus_error(
                BLUEZ_NOT_CONNECTED,
                "Not connected"
            )));
            assert!(!is_already_connected(&other(
                "Service with UUID x not found."
            )));
        }

        #[test]
        fn no_adapters_reports_unsupported() {
            let error = map_btleplug_error(btleplug::Error::Other(Box::new(
                bluez_async::BluetoothError::NoBluetoothAdapters,
            )));
            assert!(matches!(
                error,
                Error::Unsupported {
                    platform: "bluez",
                    feature: "adapter"
                }
            ));
        }

        #[test]
        fn discovery_timeout_maps_to_timeout() {
            assert!(matches!(
                map_btleplug_error(btleplug::Error::Other(Box::new(
                    bluez_async::BluetoothError::ServiceDiscoveryTimedOut
                ))),
                Error::Timeout
            ));
        }
    }
}

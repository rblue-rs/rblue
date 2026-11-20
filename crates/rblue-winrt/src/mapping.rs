//! Translation of btleplug fork values into the unified rblue model.
//!
//! The fork flattens Windows failures into strings: GATT statuses surface as
//! the Debug form of `GattCommunicationStatus(N)` (raw Windows status value),
//! and thrown WinRT operations surface as the Debug form of
//! `windows::core::Error`, carrying `HRESULT(0x...)`. The mapping below
//! re-extracts those structured codes - numeric codes never end up demoted
//! into `Error::Backend` strings. This table's contract lives in
//! `docs/error-mapping.md`.
//
// The callers live behind the Windows-only glue; host builds keep this module
// for the unit tests that pin the mapping semantics down.
#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use std::collections::BTreeSet;

use btleplug::api::{CentralState, CharPropFlags, PeripheralProperties};
use rblue_api::device::{AddressType, DeviceId};
use rblue_api::error::{AttError, Error, PermissionReason};
use rblue_api::event::AdapterState;
use rblue_api::gatt::{
    Characteristic, CharacteristicId, CharacteristicProperties, Service, ServiceId, ServiceSet,
};
use rblue_api::scan::{AdvertisementData, ScanFilter};
use rblue_api::ConnectionPriority;
use uuid::Uuid;

/// Platform name used in [`Error::Unsupported`] pairs.
pub(crate) const PLATFORM: &str = "winrt";

/// ATT status for addressing a stale or fabricated characteristic.
pub(crate) const ATT_INVALID_HANDLE: u8 = 0x01;
/// ATT status for writes in a mode the property bits do not allow.
pub(crate) const ATT_WRITE_NOT_PERMITTED: u8 = 0x03;

/// Raw `GattCommunicationStatus` values (Windows SDK ordering).
const GATT_STATUS_UNREACHABLE: i32 = 1;
const GATT_STATUS_PROTOCOL_ERROR: i32 = 2;
const GATT_STATUS_ACCESS_DENIED: i32 = 3;

/// HRESULT Windows raises on the winrtble path when the peripheral cannot be
/// reached (connect/GATT operations on a device that is gone). Mapped to
/// `Error::NotConnected` per plan section 2.2.
const HRESULT_DEVICE_UNREACHABLE: u32 = 0x8062_0010;
/// HRESULT surfaced when a connection attempt expires. Mapped to
/// `Error::Timeout` per plan section 2.2.
const HRESULT_CONNECTION_ATTEMPT_EXPIRED: u32 = 0x8062_000A;
/// `E_ACCESSDENIED`: the operation was rejected by OS access policy.
const HRESULT_ACCESS_DENIED: u32 = 0x8007_0005;

/// Facility and prefix of the `E_BLUETOOTH_ATT_*` HRESULT family: the low
/// byte carries the ATT protocol error code (`0x80620000 | att_code`).
const ATT_HRESULT_PREFIX: u32 = 0x8062_0000;
const ATT_HRESULT_FAMILY_MASK: u32 = 0xFFFF_0000;

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

/// Maps the unified connection-priority enum onto the fork's WinRT
/// `RequestPreferredConnectionParameters` presets.
pub(crate) fn connection_parameter_preset(
    priority: ConnectionPriority,
) -> btleplug::api::ConnectionParameterPreset {
    match priority {
        ConnectionPriority::Low => btleplug::api::ConnectionParameterPreset::PowerOptimized,
        ConnectionPriority::Balanced => btleplug::api::ConnectionParameterPreset::Balanced,
        ConnectionPriority::High => btleplug::api::ConnectionParameterPreset::ThroughputOptimized,
    }
}

pub(crate) fn map_btleplug_error(error: btleplug::Error) -> Error {
    match error {
        btleplug::Error::PermissionDenied => Error::Permission(PermissionReason::Unauthorized),
        btleplug::Error::NotConnected => Error::NotConnected,
        btleplug::Error::TimedOut(_) => Error::Timeout,
        btleplug::Error::NotSupported(message) => map_not_supported(&message),
        btleplug::Error::RuntimeError(message) => map_status_string(&message),
        btleplug::Error::Other(source) => map_status_string(&source.to_string()),
        btleplug::Error::DeviceNotFound
        | btleplug::Error::NoAdapterAvailable
        | btleplug::Error::UnexpectedCallback
        | btleplug::Error::UnexpectedCharacteristic
        | btleplug::Error::NoSuchCharacteristic
        | btleplug::Error::Uuid(_)
        | btleplug::Error::InvalidBDAddr(_) => Error::Backend(error.to_string()),
    }
}

/// Normalizes fork-authored `NotSupported` messages.
///
/// - `"ProtocolError"`: the fork's rendering of
///   `GattCommunicationStatus::ProtocolError`; the raw status value (2) is
///   preserved in `Error::Platform`.
/// - `"... not found for ..."`: the fork's service/characteristic maps lack
///   the requested UUID - a stale or fabricated session identifier (the fork
///   rebuilds its maps per connection). Same treatment as the mock's
///   invalid-handle errors.
fn map_not_supported(message: &str) -> Error {
    if message == "ProtocolError" {
        return Error::Platform {
            code: GATT_STATUS_PROTOCOL_ERROR,
            message: "Windows GATT status ProtocolError".to_owned(),
        };
    }
    if message.contains("not found for") {
        return att_invalid_handle();
    }
    Error::Backend(message.to_owned())
}

/// Normalizes the free-form failure strings the fork produces from Windows.
///
/// Two structured shapes are recognized before anything else:
/// - `GattCommunicationStatus(N)`: the Debug form of the raw Windows GATT
///   status enum.
/// - `HRESULT(0x...)`: the Debug form of a thrown WinRT operation error.
///
/// Anything else has no structured code left to preserve and lands in
/// `Error::Backend` verbatim.
fn map_status_string(message: &str) -> Error {
    if let Some(status) = parse_gatt_status(message) {
        return map_gatt_status(status, message);
    }
    if let Some(code) = parse_hresult(message) {
        return map_hresult(code, message);
    }
    Error::Backend(message.to_owned())
}

/// Maps a raw `GattCommunicationStatus` value. `Unreachable` and
/// `AccessDenied` have unified equivalents; every other value (including
/// ones added by future SDKs) is preserved as `Error::Platform` so
/// supervisors can match on the raw code.
fn map_gatt_status(status: i32, message: &str) -> Error {
    match status {
        GATT_STATUS_UNREACHABLE => Error::NotConnected,
        GATT_STATUS_ACCESS_DENIED => Error::Permission(PermissionReason::Unauthorized),
        GATT_STATUS_PROTOCOL_ERROR => Error::Platform {
            code: GATT_STATUS_PROTOCOL_ERROR,
            message: "Windows GATT status ProtocolError".to_owned(),
        },
        _ => Error::Platform {
            code: status,
            message: format!("Windows GattCommunicationStatus {status}: {message}"),
        },
    }
}

/// Maps an extracted HRESULT. The `E_BLUETOOTH_ATT_*` family
/// (`0x80620000 | att_code`) becomes `Error::Att` with the low byte as the
/// ATT code, except the two connection-level codes the plan mandates
/// (`0x80620010` -> `NotConnected`, `0x8062000A` -> `Timeout`). HRESULTs
/// without a mapping keep their numeric code in `Error::Platform`.
fn map_hresult(code: i32, message: &str) -> Error {
    match code as u32 {
        HRESULT_DEVICE_UNREACHABLE => Error::NotConnected,
        HRESULT_CONNECTION_ATTEMPT_EXPIRED => Error::Timeout,
        HRESULT_ACCESS_DENIED => Error::Permission(PermissionReason::Unauthorized),
        att if att & ATT_HRESULT_FAMILY_MASK == ATT_HRESULT_PREFIX && att & 0xFF != 0 => {
            Error::Att(AttError {
                code: (att & 0xFF) as u8,
                handle: None,
            })
        }
        _ => Error::Platform {
            code,
            message: message.to_owned(),
        },
    }
}

/// Extracts the numeric status from a `GattCommunicationStatus(N)` Debug
/// string. Returns `None` when the shape is not present.
fn parse_gatt_status(message: &str) -> Option<i32> {
    let marker = "GattCommunicationStatus(";
    let start = message.find(marker)? + marker.len();
    let rest = &message[start..];
    let digits_end = rest
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(rest.len());
    if digits_end == 0 {
        return None;
    }
    rest[..digits_end].parse().ok()
}

/// Extracts the numeric code from an `HRESULT(0x...)` Debug string.
/// Returns `None` when the shape is not present.
fn parse_hresult(message: &str) -> Option<i32> {
    let marker = "HRESULT(0x";
    let start = message.find(marker)? + marker.len();
    let rest = &message[start..];
    let hex_end = rest
        .find(|ch: char| !ch.is_ascii_hexdigit())
        .unwrap_or(rest.len());
    if hex_end == 0 {
        return None;
    }
    u32::from_str_radix(&rest[..hex_end], 16)
        .ok()
        .map(|code| code as i32)
}

/// Maps the central state reported by the fork into the unified model.
///
/// Windows radio states other than On/Off (Disabled, NotAvailable, Unknown)
/// collapse into `CentralState::Unknown` inside the fork and keep the honest
/// [`AdapterState::Unknown`] in live events; the `initialize` result decides
/// the error side (see the adapter's `resolve_state`).
pub(crate) fn map_central_state(state: CentralState) -> AdapterState {
    match state {
        CentralState::PoweredOn => AdapterState::PoweredOn,
        CentralState::PoweredOff => AdapterState::PoweredOff,
        CentralState::Unknown => AdapterState::Unknown,
    }
}

/// Builds the session [`DeviceId`] from a Windows address.
///
/// Windows reports only `Public` or `Random` (`BluetoothAddressType`); the
/// random sub-type is refined from the two most significant bits of the
/// address (BLE Core Spec Vol 6 Part B 1.3 address header, carried by the
/// most significant octet): `01` is a resolvable private address, `11` a
/// static random address, the rest stays [`AddressType::Unknown`]. A
/// sighting without a reported type stays `Unknown` as well.
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

/// Maximum write payload for the negotiated ATT MTU (`GattSession
/// .MaxPduSize`, tracked live by the fork), minus the 3-byte ATT header.
/// Windows negotiates one ATT MTU per connection for both write modes, so
/// the mode does not change the limit.
pub(crate) fn max_write_len_from_mtu(mtu: u16) -> usize {
    usize::from(mtu).saturating_sub(3)
}

/// Builds the parsed advertisement snapshot from the fork's property cache.
///
/// The fork consolidates advertisement and scan response into cumulative
/// property state and exposes no advertisement flags and no per-sighting
/// advertisement type, so `adv_flags` and `is_connectable` stay `None`. When
/// several manufacturer data entries are cached, the smallest company id is
/// reported to keep the single-entry model deterministic.
pub(crate) fn advertisement_from_properties(
    properties: &PeripheralProperties,
) -> AdvertisementData {
    AdvertisementData {
        local_name: properties.local_name.clone(),
        services: properties.services.clone(),
        service_data: properties
            .service_data
            .iter()
            .map(|(uuid, data)| (*uuid, data.clone()))
            .collect(),
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

/// Software part of the scan filter.
///
/// Windows pushes nothing down to hardware here: the fork deliberately keeps
/// the OS advertisement filter empty (some Windows BLE drivers silently drop
/// matching advertisements when a 128-bit UUID filter is set) and filters
/// service UUIDs in the watcher handler, so `name_prefix`,
/// `manufacturer_data`, and `rssi_threshold` filter in software too.
/// Semantics match the mock backend's `matches_scan_filter`.
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
/// returns a `BTreeSet`, so same-UUID duplicates (rare, and additionally
/// collapsed inside the fork's per-UUID maps) are ordered by UUID-derived
/// sort order rather than hardware discovery order.
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

    #[test]
    fn permission_denied_maps_to_unauthorized() {
        assert!(matches!(
            map_btleplug_error(btleplug::Error::PermissionDenied),
            Error::Permission(PermissionReason::Unauthorized)
        ));
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
            || btleplug::Error::NoAdapterAvailable,
            || btleplug::Error::UnexpectedCallback,
            || btleplug::Error::UnexpectedCharacteristic,
            || btleplug::Error::NoSuchCharacteristic,
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
    fn gatt_status_unreachable_maps_to_not_connected() {
        let error = map_btleplug_error(other(
            "Windows UWP threw error on write: GattCommunicationStatus(1)",
        ));
        assert!(matches!(error, Error::NotConnected));
    }

    #[test]
    fn gatt_status_access_denied_maps_to_permission() {
        let error = map_btleplug_error(other(
            "Windows UWP threw error on read: GattCommunicationStatus(3)",
        ));
        assert!(matches!(
            error,
            Error::Permission(PermissionReason::Unauthorized)
        ));
    }

    #[test]
    fn gatt_status_protocol_error_preserves_the_raw_code() {
        // Status-level Debug string (write/read/subscribe paths).
        match map_btleplug_error(other(
            "Windows UWP threw error on subscribe: GattCommunicationStatus(2)",
        )) {
            Error::Platform { code, message } => {
                assert_eq!(code, 2);
                assert!(message.contains("ProtocolError"));
            }
            other => panic!("expected a platform error, got {other:?}"),
        }
        // The fork's rendering through `NotSupported` (connect path).
        match map_btleplug_error(btleplug::Error::NotSupported("ProtocolError".to_owned())) {
            Error::Platform { code, message } => {
                assert_eq!(code, 2);
                assert!(message.contains("ProtocolError"));
            }
            other => panic!("expected a platform error, got {other:?}"),
        }
    }

    #[test]
    fn device_unreachable_hresult_maps_to_not_connected() {
        // 0x80620010 as a signed HRESULT, the exact form in the fork's Debug
        // strings.
        let message = "Error { code: HRESULT(0x80620010), message: \"...\" }";
        assert!(matches!(
            map_btleplug_error(other(message)),
            Error::NotConnected
        ));
    }

    #[test]
    fn expired_connection_hresult_maps_to_timeout() {
        let message = "Error { code: HRESULT(0x8062000A), message: \"...\" }";
        assert!(matches!(map_btleplug_error(other(message)), Error::Timeout));
    }

    #[test]
    fn att_hresult_family_maps_to_att_error_codes() {
        // 0x80620003 = E_BLUETOOTH_ATT_WRITE_NOT_PERMITTED.
        let message = "Error { code: HRESULT(0x80620003), message: \"...\" }";
        match map_btleplug_error(other(message)) {
            Error::Att(att) => assert_eq!(att.code, 0x03),
            other => panic!("expected an ATT error, got {other:?}"),
        }
    }

    #[test]
    fn access_denied_hresult_maps_to_permission() {
        let message = "Error { code: HRESULT(0x80070005), message: \"Access is denied.\" }";
        assert!(matches!(
            map_btleplug_error(other(message)),
            Error::Permission(PermissionReason::Unauthorized)
        ));
    }

    #[test]
    fn unrecognized_hresult_is_preserved_as_platform_code() {
        let message = "Error { code: HRESULT(0x80070490), message: \"Element not found.\" }";
        match map_btleplug_error(other(message)) {
            Error::Platform { code, message } => {
                assert_eq!(code, 0x80070490u32 as i32);
                assert!(message.contains("Element not found"));
            }
            other => panic!("expected a platform error, got {other:?}"),
        }
    }

    #[test]
    fn messages_without_structured_codes_stay_backend() {
        for message in [
            "Communication Error:",
            "Can not subscribe to attribute",
            "something unexpected",
        ] {
            assert!(
                matches!(map_btleplug_error(other(message)), Error::Backend(text) if text == message),
                "unexpected mapping for {message}"
            );
        }
    }

    #[test]
    fn missing_service_or_characteristic_is_invalid_handle() {
        for message in [
            "Service not found for write",
            "Characteristic not found for subscribe",
            "Descriptor not found for write",
        ] {
            assert!(
                matches!(
                    map_btleplug_error(btleplug::Error::NotSupported(message.to_owned())),
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
    fn parsers_reject_malformed_payloads() {
        assert_eq!(parse_gatt_status("no marker here"), None);
        assert_eq!(parse_gatt_status("GattCommunicationStatus()"), None);
        assert_eq!(parse_gatt_status("GattCommunicationStatus(x)"), None);
        assert_eq!(parse_hresult("no marker here"), None);
        assert_eq!(parse_hresult("HRESULT(0x)"), None);
        assert_eq!(parse_hresult("HRESULT(zz)"), None);
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
        // Default ATT MTU before any renegotiation: 23 - 3 = 20.
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
            vec![(Uuid::from_u128(0x180F), vec![7])]
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
            characteristics: BTreeSet::new(),
        };
        let battery = btleplug::api::Service {
            uuid: uuid!("0000180f-0000-1000-8000-00805f9b34fb"),
            primary: true,
            characteristics: BTreeSet::new(),
        };
        let mut discovered = BTreeSet::new();
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
}

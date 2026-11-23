//! Translation of btleplug fork values into the unified rblue model.
//!
//! The fork's Android (droidplug) surface reports almost every failure as a
//! `btleplug::Error`, and the Java layer beneath it embeds the raw
//! `BluetoothGatt` status integers into its `RuntimeException` messages
//! (`"Unable to read characteristic, status: 133"`). This module recovers
//! those numbers verbatim into [`Error::Platform`] - the plan's hard rule,
//! because the connection supervisor matches the raw Android GATT status
//! (133 above all) instead of re-parsing text. Failures with no numeric code
//! and no permission shape keep their message text in `Error::Backend`. This
//! table's contract lives in `docs/error-mapping.md`.
//
// The consumers of most of these items live behind the Android-only glue
// (`mod adapter` / `mod peripheral`); on host builds only the tests below
// touch them, which is where the pure translation semantics get verified.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use std::collections::{BTreeSet, HashMap};

use btleplug::api::{BDAddr, CentralState, CharPropFlags, PeripheralProperties, ValueNotification};
use rblue_api::device::{AddressType, DeviceId};
use rblue_api::error::{AttError, Error, PermissionReason};
use rblue_api::event::AdapterState;
use rblue_api::gatt::{
    Characteristic, CharacteristicId, CharacteristicProperties, Notification, Service, ServiceId,
    ServiceSet,
};
use rblue_api::scan::{AdvertisementData, ScanFilter};
use rblue_api::ConnectionPriority;
use uuid::Uuid;

/// Platform name used in [`Error::Unsupported`] pairs.
pub(crate) const PLATFORM: &str = "android";

/// ATT status for addressing a stale or fabricated characteristic.
pub(crate) const ATT_INVALID_HANDLE: u8 = 0x01;
/// ATT status for writes in a mode the property bits do not allow.
pub(crate) const ATT_WRITE_NOT_PERMITTED: u8 = 0x03;

/// Marker the fork's Java layer prepends to the raw `BluetoothGatt` status
/// integer in its `RuntimeException` messages (stable strings authored in the
/// fork's `Peripheral.java`: `"Unable to read characteristic, status: N"`).
const STATUS_MARKER: &str = "status: ";

/// The unified error for stale or fabricated session identifiers: the
/// discovered tree cannot resolve them (invalid handle).
pub(crate) fn att_invalid_handle() -> Error {
    Error::Att(AttError {
        code: ATT_INVALID_HANDLE,
        handle: None,
    })
}

/// Maps the unified connection-priority enum onto the fork's Android
/// `requestConnectionPriority` presets: `Low` is `CONNECTION_PRIORITY_LOW_POWER`,
/// `Balanced` is `CONNECTION_PRIORITY_BALANCED`, and `High` is
/// `CONNECTION_PRIORITY_HIGH`.
pub(crate) fn connection_parameter_preset(
    priority: ConnectionPriority,
) -> btleplug::api::ConnectionParameterPreset {
    match priority {
        ConnectionPriority::Low => btleplug::api::ConnectionParameterPreset::PowerOptimized,
        ConnectionPriority::Balanced => btleplug::api::ConnectionParameterPreset::Balanced,
        ConnectionPriority::High => btleplug::api::ConnectionParameterPreset::ThroughputOptimized,
    }
}

/// Maps a btleplug error into the unified model. Errors that carry a device
/// identity in the rblue model (`Error::NotFound`) are produced at the call
/// sites that know the id, not here.
pub(crate) fn map_btleplug_error(error: btleplug::Error) -> Error {
    match error {
        btleplug::Error::NotConnected => Error::NotConnected,
        btleplug::Error::PermissionDenied => {
            // The fork raises this variant only for the `SecurityException`
            // around `connectGatt` - `BLUETOOTH_CONNECT` on API 31+.
            Error::Permission(PermissionReason::BluetoothPermissionDenied)
        }
        btleplug::Error::TimedOut(_) => Error::Timeout,
        // `NoBluetoothAdapterException` is thrown by the fork's Java layer
        // when `BluetoothAdapter.getDefaultAdapter()` returns null (no BLE
        // hardware) or `getBluetoothLeScanner()` returns null (radio off).
        // The two are indistinguishable through the fork; a switched-off
        // radio is the common real-world cause, so the unified model's
        // expected "radio off" state wins (see docs/error-mapping.md).
        btleplug::Error::NoAdapterAvailable => Error::Permission(PermissionReason::BluetoothOff),
        btleplug::Error::RuntimeError(message) => map_runtime_message(&message),
        btleplug::Error::DeviceNotFound
        | btleplug::Error::UnexpectedCallback
        | btleplug::Error::UnexpectedCharacteristic
        | btleplug::Error::NoSuchCharacteristic
        | btleplug::Error::NotSupported(_)
        | btleplug::Error::Uuid(_)
        | btleplug::Error::InvalidBDAddr(_)
        | btleplug::Error::Other(_) => Error::Backend(error.to_string()),
    }
}

/// Normalizes the fork's `RuntimeError` messages - the only channel through
/// which the Android GATT status integers reach Rust. Three shapes are
/// recognized, in order:
///
/// 1. `"... status: N"` (the fork's Java-authored GATT-callback failures):
///    the raw status becomes [`Error::Platform`] verbatim, including 133.
/// 2. Permission-denial texts (`SecurityException.getMessage()` from the
///    platform's permission checks): [`Error::Permission`] with the
///    narrowest [`PermissionReason`] the text identifies.
/// 3. Anything else: [`Error::Backend`] with the message preserved.
pub(crate) fn map_runtime_message(message: &str) -> Error {
    if let Some(code) = parse_gatt_status(message) {
        return Error::Platform {
            code,
            message: message.to_owned(),
        };
    }
    if let Some(reason) = permission_reason_from_message(message) {
        return Error::Permission(reason);
    }
    Error::Backend(message.to_owned())
}

/// Extracts the raw GATT status from the fork's `"... status: N"` messages.
///
/// The number must sit at the end of the message (the fork's stable format);
/// a `status:` token followed by further prose is not a status report and
/// stays unstructured.
pub(crate) fn parse_gatt_status(message: &str) -> Option<i32> {
    let start = message.rfind(STATUS_MARKER)? + STATUS_MARKER.len();
    let rest = &message[start..];
    if rest.is_empty() || !rest.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

/// Classifies the platform's permission-denial texts into the unified
/// [`PermissionReason`]. The messages are `SecurityException.getMessage()`
/// strings from `BluetoothLeScanner.startScan` (`"Need
/// android.permission.BLUETOOTH_SCAN permission for ..."` on API 31+,
/// `"Need ACCESS_COARSE_LOCATION or ACCESS_FINE_LOCATION permission to get
/// scan results"` on API 30 and below) and their relatives; matching is
/// case-insensitive on the stable permission-name substrings. A text that
/// mentions a permission without a recognizable name is still a permission
/// denial - the generic Bluetooth reason is the conservative reading.
pub(crate) fn permission_reason_from_message(message: &str) -> Option<PermissionReason> {
    let lowered = message.to_ascii_lowercase();
    for marker in [
        "access_fine_location",
        "access_coarse_location",
        "location permission",
    ] {
        if lowered.contains(marker) {
            return Some(PermissionReason::LocationDenied);
        }
    }
    for marker in [
        "bluetooth_scan",
        "bluetooth_connect",
        "android.permission.bluetooth",
    ] {
        if lowered.contains(marker) {
            return Some(PermissionReason::BluetoothPermissionDenied);
        }
    }
    if lowered.contains("permission") {
        return Some(PermissionReason::BluetoothPermissionDenied);
    }
    None
}

/// Maps a `BluetoothAdapter` state code into the unified model.
///
/// `12`/`15` are `STATE_ON` / `STATE_BLE_ON`. The transitional off/turning
/// codes (and `STATE_OFF`) are `PoweredOff`. The shell-local sentinels
/// `-2` (no adapter) and `-3` (`SecurityException` from `getState`) map to
/// `Unavailable` and `Unauthorized`.
pub(crate) fn map_android_bluetooth_state(state: i32) -> AdapterState {
    match state {
        -2 => AdapterState::Unavailable,
        -3 => AdapterState::Unauthorized,
        12 | 15 => AdapterState::PoweredOn,
        10 | 11 | 13 | 14 | 16 => AdapterState::PoweredOff,
        _ => AdapterState::Unknown,
    }
}

/// Maps the central state reported by the fork into the unified model.
///
/// The fork's Android adapter tracks no radio state and always reports
/// `Unknown`; the shell prefers [`map_android_bluetooth_state`] for
/// `Adapter::state` and live events. The other arms keep this mapping total.
pub(crate) fn map_central_state(state: CentralState) -> AdapterState {
    match state {
        CentralState::PoweredOn => AdapterState::PoweredOn,
        CentralState::PoweredOff => AdapterState::PoweredOff,
        CentralState::Unknown => AdapterState::Unknown,
    }
}

/// Builds the session [`DeviceId`] from an Android address.
///
/// Android is MAC-first, but the fork never reports the address kind (its
/// scan-result parsing hard-codes `address_type: None`), so every identity
/// from this backend carries [`AddressType::Unknown`]. The random sub-type
/// could be refined from the two most significant bits like the BlueZ shell
/// does - but Android's `BluetoothDevice.getAddress()` makes no such
/// guarantee, so no bits are guessed here.
pub(crate) fn device_id_from(address: BDAddr) -> DeviceId {
    DeviceId::Mac(address.into_inner(), AddressType::Unknown)
}

/// Returns the raw address bytes behind a [`DeviceId`], or `None` for the
/// non-MAC identity kinds Android never produces.
pub(crate) fn mac_bytes_from_id(id: &DeviceId) -> Option<[u8; 6]> {
    match id {
        DeviceId::Mac(address, _) => Some(*address),
        DeviceId::Uuid(_) | DeviceId::Web(_) => None,
    }
}

/// Converts btleplug characteristic property bits. The low byte is identical
/// between the two models; rblue's Phase-2 bits are never set by this
/// backend.
pub(crate) fn map_properties(flags: CharPropFlags) -> CharacteristicProperties {
    CharacteristicProperties::from_bits_truncate(flags.bits() as u16)
}

/// Converts rblue property bits back into the fork's representation when
/// rebuilding fork characteristic values for GATT operations.
pub(crate) fn properties_to_flags(properties: CharacteristicProperties) -> CharPropFlags {
    CharPropFlags::from_bits_truncate(properties.bits() as u8)
}

/// Maximum write payload for the negotiated ATT MTU, minus the 3-byte ATT
/// header. Android enforces the same limit for both write types (an
/// oversize write-without-response is silently truncated by the stack, which
/// is exactly why the shell pre-checks the length). Before the fork's
/// automatic MTU request completes, the value is the ATT default of 23, so
/// the conservative limit is 20.
pub(crate) fn max_write_len_from_mtu(mtu: u16) -> usize {
    usize::from(mtu).saturating_sub(3)
}

/// Builds the parsed advertisement snapshot from the fork's cached device
/// properties.
///
/// Android merges advertisement and scan response into one `ScanRecord`, and
/// the fork exposes no advertisement flags and no per-sighting connectability,
/// so `adv_flags` and `is_connectable` stay `None`. When several
/// manufacturer-data records are present, the smallest company id is reported
/// to keep the single-entry model deterministic; service data is sorted by
/// UUID for the same reason.
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

/// Software part of the scan filter.
///
/// Android pushes `services` down through the `ScanFilter` parcel
/// (`setServiceUuid`), so non-matching devices are not reported at all;
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
/// deduplicates same-UUID characteristics per service (its Java layer filters
/// repeats while walking the GATT cache) and the `BTreeSet` collapses
/// same-UUID services, so duplicates are not addressable and indices are 0
/// in practice.
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

/// Routes a fork value notification onto the unified model using the cached
/// tree of the current connection. Without a tree (before discovery, or
/// after a disconnect cleared it) values cannot be addressed and are
/// dropped, mirroring the contract that identifiers are session-scoped.
pub(crate) fn notification_from(
    services: &ServiceSet,
    subscribed: impl Fn(&CharacteristicId) -> bool,
    value: &ValueNotification,
) -> Option<Notification> {
    let id = locate_characteristic(services, value.service_uuid, value.uuid)?;
    if !subscribed(&id) {
        return None;
    }
    Some(Notification {
        id,
        data: value.value.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::str::FromStr as _;
    use uuid::uuid;

    fn runtime(message: &str) -> btleplug::Error {
        btleplug::Error::RuntimeError(message.to_owned())
    }

    fn backend_text(error: &Error) -> &str {
        match error {
            Error::Backend(text) => text,
            other => panic!("expected a backend error, got {other:?}"),
        }
    }

    fn platform_code(error: &Error) -> i32 {
        match error {
            Error::Platform { code, .. } => *code,
            other => panic!("expected a platform error, got {other:?}"),
        }
    }

    fn permission_reason(error: &Error) -> PermissionReason {
        match error {
            Error::Permission(reason) => *reason,
            other => panic!("expected a permission error, got {other:?}"),
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
        // The fork's connectGatt SecurityException is a missing
        // BLUETOOTH_CONNECT-style runtime permission.
        assert_eq!(
            permission_reason(&map_btleplug_error(btleplug::Error::PermissionDenied)),
            PermissionReason::BluetoothPermissionDenied
        );
    }

    #[test]
    fn no_adapter_maps_to_bluetooth_off() {
        // getBluetoothLeScanner() returns null on a powered-off radio; the
        // no-hardware case is indistinguishable and shares the mapping.
        assert_eq!(
            permission_reason(&map_btleplug_error(btleplug::Error::NoAdapterAvailable)),
            PermissionReason::BluetoothOff
        );
    }

    #[test]
    fn unstructured_variants_stay_backend() {
        let variants: [fn() -> btleplug::Error; 6] = [
            || btleplug::Error::DeviceNotFound,
            || btleplug::Error::UnexpectedCallback,
            || btleplug::Error::UnexpectedCharacteristic,
            || btleplug::Error::NoSuchCharacteristic,
            || btleplug::Error::NotSupported("x".to_owned()),
            || btleplug::Error::Other("jni error".to_owned().into()),
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
    fn gatt_status_133_becomes_platform_error() {
        // The plan's hard rule: Android GATT status 133 and every other raw
        // status code reach the supervisor as Error::Platform, never as a
        // string. This is the fork's discover-services failure text.
        let error = map_btleplug_error(runtime("Unable to discover services, status: 133"));
        assert_eq!(platform_code(&error), 133);
        match &error {
            Error::Platform { message, .. } => {
                assert_eq!(message, "Unable to discover services, status: 133")
            }
            other => panic!("expected a platform error, got {other:?}"),
        }
    }

    #[test]
    fn every_gatt_operation_status_shape_is_preserved() {
        // The stable "status: N" suffixes the fork's Java layer authors per
        // GATT callback (Peripheral.java).
        let cases = [
            ("Unable to disconnect, status: 22", 22),
            ("MTU negotiation failed, status: 133", 133),
            ("Unable to read characteristic, status: 8", 8),
            ("Unable to write characteristic, status: 19", 19),
            ("Unable to discover services, status: 34", 34),
            (
                "Unable to write client characteristic configuration descriptor, status: 257",
                257,
            ),
            ("Unable to read descriptor, status: 62", 62),
            ("RSSI read failed, status: 133", 133),
        ];
        for (message, code) in cases {
            assert_eq!(platform_code(&map_runtime_message(message)), code);
        }
    }

    #[test]
    fn status_parse_requires_digits_at_the_end() {
        assert_eq!(
            parse_gatt_status("Unable to reconnect, status: 133"),
            Some(133)
        );
        assert_eq!(parse_gatt_status("no status here"), None);
        assert_eq!(parse_gatt_status("status: "), None);
        assert_eq!(parse_gatt_status("status: abc"), None);
        // A status token followed by prose is not a status report.
        assert_eq!(parse_gatt_status("status: 5 and counting"), None);
        // Only the last token counts.
        assert_eq!(
            parse_gatt_status("status: 3 retried, status: 133"),
            Some(133)
        );
    }

    #[test]
    fn permission_texts_map_to_narrow_reasons() {
        // API 31+ scan denial.
        assert_eq!(
            permission_reason(&map_runtime_message(
                "Need android.permission.BLUETOOTH_SCAN permission for \
                 AttributionSource { uid = 1000, packageName = app }"
            )),
            PermissionReason::BluetoothPermissionDenied
        );
        // API 30 and below scan denial.
        assert_eq!(
            permission_reason(&map_runtime_message(
                "Need ACCESS_COARSE_LOCATION or ACCESS_FINE_LOCATION permission \
                 to get scan results"
            )),
            PermissionReason::LocationDenied
        );
        // A permission mention without a known name stays a permission error.
        assert_eq!(
            permission_reason(&map_runtime_message("some permission problem")),
            PermissionReason::BluetoothPermissionDenied
        );
    }

    #[test]
    fn non_status_non_permission_texts_stay_backend() {
        let error = map_runtime_message("Unable to reconnect to device");
        assert_eq!(backend_text(&error), "Unable to reconnect to device");
        let error = map_runtime_message("Java exception: java.lang.IllegalStateException");
        assert_eq!(
            backend_text(&error),
            "Java exception: java.lang.IllegalStateException"
        );
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
        // The only state the fork's Android adapter ever reports.
        assert_eq!(
            map_central_state(CentralState::Unknown),
            AdapterState::Unknown
        );
    }

    #[test]
    fn android_bluetooth_state_codes_map_to_unified_states() {
        assert_eq!(map_android_bluetooth_state(12), AdapterState::PoweredOn);
        assert_eq!(map_android_bluetooth_state(15), AdapterState::PoweredOn);
        for code in [10, 11, 13, 14, 16] {
            assert_eq!(map_android_bluetooth_state(code), AdapterState::PoweredOff);
        }
        assert_eq!(map_android_bluetooth_state(-2), AdapterState::Unavailable);
        assert_eq!(map_android_bluetooth_state(-3), AdapterState::Unauthorized);
        assert_eq!(map_android_bluetooth_state(0), AdapterState::Unknown);
    }

    #[test]
    fn addresses_carry_the_unknown_type() {
        let address = BDAddr::from_str("11:22:33:44:55:66").expect("parsed");
        assert_eq!(
            device_id_from(address),
            DeviceId::Mac([0x11, 0x22, 0x33, 0x44, 0x55, 0x66], AddressType::Unknown)
        );
        assert_eq!(
            mac_bytes_from_id(&device_id_from(address)),
            Some([0x11, 0x22, 0x33, 0x44, 0x55, 0x66])
        );
        assert_eq!(mac_bytes_from_id(&DeviceId::Uuid(uuid::Uuid::nil())), None);
        assert_eq!(mac_bytes_from_id(&DeviceId::Web("id".to_owned())), None);
    }

    #[test]
    fn property_bits_convert_losslessly() {
        let flags = CharPropFlags::READ | CharPropFlags::WRITE | CharPropFlags::INDICATE;
        let properties = map_properties(flags);
        assert_eq!(
            properties,
            CharacteristicProperties::READ
                | CharacteristicProperties::WRITE
                | CharacteristicProperties::INDICATE
        );
        assert_eq!(properties_to_flags(properties), flags);
    }

    #[test]
    fn max_write_len_subtracts_the_att_header() {
        // ATT default before the fork's automatic MTU request completes.
        assert_eq!(max_write_len_from_mtu(23), 20);
        assert_eq!(max_write_len_from_mtu(517), 514);
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
            address: BDAddr::from_str("11:22:33:44:55:66").expect("parsed"),
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
        // Android merges advertisement and scan response into one ScanRecord
        // and exposes neither flags nor connectability.
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

    #[test]
    fn notifications_route_through_subscriptions() {
        let set = service_set(vec![characteristic(CharacteristicProperties::NOTIFY)]);
        let id = locate_characteristic(&set, SERVICE, CHARACTERISTIC).expect("located");
        let value = ValueNotification {
            uuid: CHARACTERISTIC,
            service_uuid: SERVICE,
            value: vec![1, 2, 3],
        };

        let notification =
            notification_from(&set, |candidate| *candidate == id, &value).expect("routed");
        assert_eq!(notification.id, id);
        assert_eq!(notification.data, vec![1, 2, 3]);

        // A characteristic the session has not subscribed to never delivers
        // values (the stack can re-arm persisted CCCDs for bonded devices
        // after a reconnect - the shell's contract forbids the leak).
        assert!(notification_from(&set, |_| false, &value).is_none());
        // Without a discovered tree, values are not addressable at all.
        assert!(notification_from(&ServiceSet::default(), |_| true, &value).is_none());
    }

    #[test]
    fn connection_priority_maps_to_android_presets() {
        use btleplug::api::ConnectionParameterPreset;

        assert_eq!(
            connection_parameter_preset(ConnectionPriority::Low),
            ConnectionParameterPreset::PowerOptimized
        );
        assert_eq!(
            connection_parameter_preset(ConnectionPriority::Balanced),
            ConnectionParameterPreset::Balanced
        );
        assert_eq!(
            connection_parameter_preset(ConnectionPriority::High),
            ConnectionParameterPreset::ThroughputOptimized
        );
    }
}

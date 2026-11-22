//! Translation of btleplug fork values into the unified rblue model.
//!
//! The fork flattens CoreBluetooth `NSError`s into string messages, so the
//! error mapping is partly structural (btleplug variants) and partly
//! best-effort string normalization (ATT codes re-extracted from messages).
//! See `docs/error-mapping.md` for the full table.

#![cfg_attr(
    not(any(target_os = "macos", target_os = "ios")),
    allow(dead_code, unused_imports)
)]

use btleplug::api::{CentralState, CharPropFlags, PeripheralProperties};
use rblue_api::error::{AttError, Error, PermissionReason};
use rblue_api::event::AdapterState;
use rblue_api::gatt::CharacteristicProperties;
use rblue_api::scan::AdvertisementData;
use uuid::Uuid;

/// Platform name used in [`Error::Unsupported`] pairs.
pub(crate) const PLATFORM: &str = "corebluetooth";

/// ATT status for addressing a stale or fabricated characteristic.
pub(crate) const ATT_INVALID_HANDLE: u8 = 0x01;
/// ATT status for writes in a mode the property bits do not allow.
pub(crate) const ATT_WRITE_NOT_PERMITTED: u8 = 0x03;

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
        btleplug::Error::PermissionDenied => Error::Permission(PermissionReason::Unauthorized),
        btleplug::Error::NotConnected => Error::NotConnected,
        btleplug::Error::TimedOut(_) => Error::Timeout,
        btleplug::Error::RuntimeError(message) => map_runtime_error(&message),
        btleplug::Error::DeviceNotFound
        | btleplug::Error::NoAdapterAvailable
        | btleplug::Error::UnexpectedCallback
        | btleplug::Error::UnexpectedCharacteristic
        | btleplug::Error::NoSuchCharacteristic
        | btleplug::Error::NotSupported(_)
        | btleplug::Error::Uuid(_)
        | btleplug::Error::InvalidBDAddr(_)
        | btleplug::Error::Other(_) => Error::Backend(error.to_string()),
    }
}

/// Maps the string messages the fork produces for CoreBluetooth failures.
///
/// Three families are recognizable:
/// - ATT failures, surfaced by the fork as `(ATT error N.)` messages; the
///   code is re-extracted into [`Error::Att`].
/// - Stale-object failures (`"... no longer available"`, drained as
///   `"Device disconnected"` after a link drop).
/// - Everything else stays an unstructured [`Error::Backend`].
fn map_runtime_error(message: &str) -> Error {
    if message == "Device disconnected" {
        return Error::Disconnected;
    }
    if message.ends_with("no longer available") {
        // Stale session identifier (peripheral/service/characteristic dropped
        // by the fork after a disconnect or service modification).
        return Error::Att(AttError {
            code: ATT_INVALID_HANDLE,
            handle: None,
        });
    }
    if let Some(code) = parse_att_error_code(message) {
        return Error::Att(AttError { code, handle: None });
    }
    Error::Backend(message.to_owned())
}

/// Extracts an ATT error code from a CoreBluetooth error description.
///
/// The fork forwards localized descriptions; the two shapes seen in practice
/// are `"(ATT error 15.)"` and `"CBATTErrorDomain Code=15"`. Returns `None`
/// when no recognizable ATT code is present.
fn parse_att_error_code(message: &str) -> Option<u8> {
    let marker = [
        "ATT error ",
        "CBATTErrorDomain Code=",
        "CBATTErrorDomain error ",
    ]
    .into_iter()
    .find_map(|marker| message.find(marker).map(|index| (index, marker)))?;
    let rest = &message[marker.0 + marker.1.len()..];
    let digits_end = rest
        .char_indices()
        .find(|(_, ch)| !ch.is_ascii_digit())
        .map_or(rest.len(), |(index, _)| index);
    rest[..digits_end].parse::<u8>().ok()
}

/// Maps the central state reported by the fork into the unified model.
///
/// CoreBluetooth states the fork collapses into [`CentralState::Unknown`]
/// (unsupported hardware, resetting) map to [`AdapterState::Unavailable`];
/// the [`Error`] side of the story is decided by the adapter's
/// `resolve_state` (powered-off is a permission error, unavailable is an
/// unsupported-platform error).
pub(crate) fn map_central_state(state: CentralState) -> AdapterState {
    match state {
        CentralState::PoweredOn => AdapterState::PoweredOn,
        CentralState::PoweredOff => AdapterState::PoweredOff,
        CentralState::Unknown => AdapterState::Unavailable,
    }
}

/// Extracts the CB identifier UUID from the fork's opaque `PeripheralId`.
///
/// The fork keeps the field private and only implements `Display` (the
/// hyphenated UUID form) plus `From<Uuid>`, so the round trip goes through
/// the string form. The `platform::PeripheralId` type is Apple-specific, so
/// this helper is compiled only there.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) fn peripheral_uuid(id: &btleplug::platform::PeripheralId) -> Option<Uuid> {
    Uuid::parse_str(&id.to_string()).ok()
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

/// Builds the parsed advertisement snapshot from the fork's property cache.
///
/// CoreBluetooth exposes no advertisement flags and no per-sighting
/// connectability, so both fields stay `None`. When several manufacturer data
/// entries are cached, the smallest company id is reported to keep the
/// single-entry model deterministic.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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
    fn disconnect_drain_message_maps_to_disconnected() {
        assert!(matches!(
            map_btleplug_error(btleplug::Error::RuntimeError(
                "Device disconnected".to_owned()
            )),
            Error::Disconnected
        ));
    }

    #[test]
    fn stale_object_messages_map_to_invalid_handle() {
        for message in [
            "Peripheral no longer available",
            "Service no longer available",
            "Characteristic no longer available",
        ] {
            assert!(
                matches!(
                    map_btleplug_error(btleplug::Error::RuntimeError(message.to_owned())),
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
    fn att_error_codes_are_re_extracted() {
        let error = map_btleplug_error(btleplug::Error::RuntimeError(
            "The operation couldn't be completed. (ATT error 15.)".to_owned(),
        ));
        match error {
            Error::Att(att) => assert_eq!(att.code, 15),
            other => panic!("expected an ATT error, got {other:?}"),
        }

        let error = map_btleplug_error(btleplug::Error::RuntimeError(
            "The operation couldn’t be completed. (CBATTErrorDomain Code=7.)".to_owned(),
        ));
        match error {
            Error::Att(att) => assert_eq!(att.code, 7),
            other => panic!("expected an ATT error, got {other:?}"),
        }
    }

    #[test]
    fn unparseable_runtime_errors_stay_backend() {
        assert!(matches!(
            map_btleplug_error(btleplug::Error::RuntimeError(
                "something unexpected".to_owned()
            )),
            Error::Backend(_)
        ));
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
            AdapterState::Unavailable
        );
    }

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    #[test]
    fn peripheral_uuid_round_trips_through_the_display_form() {
        let uuid = uuid::Uuid::from_u128(0x9fb0_e9e1_7a34_4e2a_a4c5_9c34_a221_3f81);
        let id = btleplug::platform::PeripheralId::from(uuid);
        assert_eq!(peripheral_uuid(&id), Some(uuid));
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
    fn advertisement_is_built_deterministically() {
        let mut manufacturer_data = HashMap::new();
        manufacturer_data.insert(0x004C, vec![1, 2]);
        manufacturer_data.insert(0x00E0, vec![3]);
        let properties = PeripheralProperties {
            local_name: Some("sensor".to_owned()),
            services: vec![uuid::Uuid::from_u128(0x180F)],
            manufacturer_data,
            rssi: Some(-40),
            tx_power_level: Some(4),
            ..PeripheralProperties::default()
        };
        let advertisement = advertisement_from_properties(&properties);
        assert_eq!(advertisement.local_name.as_deref(), Some("sensor"));
        assert_eq!(advertisement.manufacturer_data, Some((0x004C, vec![1, 2])));
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
}

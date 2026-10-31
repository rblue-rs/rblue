//! Wrapping of browser device identifiers into the unified [`DeviceId`].

use rblue_api::DeviceId;

/// Wraps the browser's `BluetoothDevice.id` string into a [`DeviceId`].
///
/// The browser id is an opaque, per-origin string (not a MAC address and not
/// stable across origins), so it travels verbatim inside
/// [`DeviceId::Web`]. Application code must treat it as personal data and
/// must not compare it against ids from other platforms.
#[must_use]
pub fn wrap_bluetooth_device_id(id: &str) -> DeviceId {
    DeviceId::Web(id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_the_browser_id_verbatim() {
        let id = wrap_bluetooth_device_id(" 3N2/xxxxxx== ");
        assert_eq!(id, DeviceId::Web(" 3N2/xxxxxx== ".to_owned()));
    }

    #[test]
    fn wrapped_ids_compare_by_string_equality() {
        assert_eq!(wrap_bluetooth_device_id("a"), wrap_bluetooth_device_id("a"));
        assert_ne!(wrap_bluetooth_device_id("a"), wrap_bluetooth_device_id("b"));
    }

    #[test]
    fn wrapped_ids_do_not_equal_other_variants() {
        assert_ne!(
            wrap_bluetooth_device_id("a"),
            DeviceId::Mac([0; 6], rblue_api::AddressType::Unknown)
        );
    }
}

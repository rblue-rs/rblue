//! Shell-side BlueZ pairing via D-Bus `Device1.Pair` and
//! `Adapter1.RemoveDevice`.
//!
//! The forked btleplug backend does not expose these methods on the
//! `Peripheral` trait. The shell issues blocking D-Bus calls on a worker
//! thread using the device object path already stored on the registry
//! entry. Path helpers and error-name mapping are host-testable.

use rblue_api::error::{Error, PairingStatus, PermissionReason};

/// Prefix stored paths from the fork (`hci0/dev_XX_...`) with `/org/bluez/`.
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
pub(crate) fn normalize_device_path(stored: &str) -> String {
    if stored.starts_with("/org/bluez/") {
        stored.to_string()
    } else if stored.starts_with('/') {
        stored.to_string()
    } else {
        format!("/org/bluez/{stored}")
    }
}

/// Parent adapter object path for a BlueZ device path.
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
pub(crate) fn adapter_path_for_device(device_path: &str) -> Option<String> {
    let (parent, last) = device_path.rsplit_once('/')?;
    if last.is_empty() {
        return None;
    }
    // A device path is `/org/bluez/<adapter>/<device>`; reject anything
    // shallower (including `/org/bluez` itself).
    if parent == "/org/bluez" || parent.starts_with("/org/bluez/") {
        return Some(parent.to_string());
    }
    None
}

/// Maps a BlueZ / D-Bus error name from a pairing or unpair call.
///
/// This is deliberately separate from GATT [`crate::mapping::map_dbus_error`]:
/// `Canceled` and `InProgress` are pairing statuses here, not disconnects.
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
pub(crate) fn map_pairing_error_name(name: &str) -> Error {
    let short = name.rsplit('.').next().unwrap_or(name);
    match short {
        "AuthenticationFailed" => Error::Pairing(PairingStatus::AuthenticationFailed),
        "AuthenticationRejected" | "Rejected" => Error::Pairing(PairingStatus::Rejected),
        "AuthenticationTimeout" | "Timeout" | "Timedout" | "TimedOut" => {
            Error::Pairing(PairingStatus::TimedOut)
        }
        "AuthenticationCanceled" | "Canceled" | "Cancelled" => {
            Error::Pairing(PairingStatus::Rejected)
        }
        "InProgress" => Error::Pairing(PairingStatus::InProgress),
        "NotSupported" | "NotCapable" => Error::Pairing(PairingStatus::NotCapable),
        "NotAuthorized" | "NotPermitted" => Error::Permission(PermissionReason::Unauthorized),
        "NotReady" => Error::Permission(PermissionReason::BluetoothOff),
        "NoReply" => Error::Pairing(PairingStatus::TimedOut),
        _ => Error::Backend(format!("bluez pairing: {name}")),
    }
}

/// Names that mean "already paired" for `Device1.Pair` and should succeed.
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
pub(crate) fn is_already_paired_name(name: &str) -> bool {
    let short = name.rsplit('.').next().unwrap_or(name);
    short == "AlreadyExists"
}

/// Names that mean the device or bond is already gone for `RemoveDevice`.
#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
pub(crate) fn is_already_unpaired_name(name: &str) -> bool {
    let short = name.rsplit('.').next().unwrap_or(name);
    short == "DoesNotExist" || short == "DoesNotExistError"
}

#[cfg(target_os = "linux")]
pub(crate) async fn pair(stored_path: String) -> Result<(), Error> {
    spawn_blocking("pair", move || pair_blocking(&stored_path)).await
}

#[cfg(target_os = "linux")]
pub(crate) async fn unpair(stored_path: String) -> Result<(), Error> {
    spawn_blocking("unpair", move || unpair_blocking(&stored_path)).await
}

#[cfg(target_os = "linux")]
async fn spawn_blocking(
    feature: &'static str,
    work: impl FnOnce() -> Result<(), Error> + Send + 'static,
) -> Result<(), Error> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| Error::Backend(format!("bluez {feature}: join: {err}")))?
}

#[cfg(target_os = "linux")]
fn pair_blocking(stored_path: &str) -> Result<(), Error> {
    use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
    use dbus::blocking::Connection;

    let device_path = normalize_device_path(stored_path);
    let conn = Connection::new_system()
        .map_err(|err| Error::Backend(format!("bluez pair: system bus: {err}")))?;
    let timeout = std::time::Duration::from_secs(30);
    let dev = conn.with_path("org.bluez", device_path.clone(), timeout);
    match Properties::get::<bool>(&dev, "org.bluez.Device1", "Paired") {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(err) => {
            if let Some(name) = err.name() {
                if is_already_unpaired_name(name) {
                    // Fall through and let Pair produce a clearer error.
                } else {
                    return Err(map_pairing_dbus_error(&err));
                }
            } else {
                return Err(map_pairing_dbus_error(&err));
            }
        }
    }
    match dev.method_call("org.bluez.Device1", "Pair", ()) {
        Ok(()) => Ok(()),
        Err(err) => {
            if let Some(name) = err.name() {
                if is_already_paired_name(name) {
                    return Ok(());
                }
            }
            Err(map_pairing_dbus_error(&err))
        }
    }
}

#[cfg(target_os = "linux")]
fn unpair_blocking(stored_path: &str) -> Result<(), Error> {
    use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
    use dbus::blocking::Connection;
    use dbus::Path;

    let device_path = normalize_device_path(stored_path);
    let adapter_path = adapter_path_for_device(&device_path).ok_or_else(|| {
        Error::Backend(format!(
            "bluez unpair: cannot derive adapter path from {device_path}"
        ))
    })?;
    let conn = Connection::new_system()
        .map_err(|err| Error::Backend(format!("bluez unpair: system bus: {err}")))?;
    let timeout = std::time::Duration::from_secs(30);
    let dev = conn.with_path("org.bluez", device_path.clone(), timeout);
    match Properties::get::<bool>(&dev, "org.bluez.Device1", "Paired") {
        Ok(false) => return Ok(()),
        Ok(true) => {}
        Err(err) => {
            if let Some(name) = err.name() {
                if is_already_unpaired_name(name) {
                    return Ok(());
                }
            }
            return Err(map_pairing_dbus_error(&err));
        }
    }
    let adapter = conn.with_path("org.bluez", adapter_path, timeout);
    let path = Path::new(device_path.clone())
        .map_err(|_| Error::Backend(format!("bluez unpair: invalid device path {device_path}")))?;
    match adapter.method_call("org.bluez.Adapter1", "RemoveDevice", (path,)) {
        Ok(()) => Ok(()),
        Err(err) => {
            if let Some(name) = err.name() {
                if is_already_unpaired_name(name) {
                    return Ok(());
                }
            }
            Err(map_pairing_dbus_error(&err))
        }
    }
}

#[cfg(target_os = "linux")]
fn map_pairing_dbus_error(err: &dbus::Error) -> Error {
    match err.name() {
        Some(name) => map_pairing_error_name(name),
        None => Error::Backend(format!("bluez pairing: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_device_path_prefixes_fork_ids() {
        assert_eq!(
            normalize_device_path("hci0/dev_AA_BB_CC_DD_EE_FF"),
            "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF"
        );
        assert_eq!(
            normalize_device_path("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF"),
            "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF"
        );
    }

    #[test]
    fn adapter_path_for_device_uses_parent() {
        assert_eq!(
            adapter_path_for_device("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF").as_deref(),
            Some("/org/bluez/hci0")
        );
        assert_eq!(adapter_path_for_device("/org/bluez"), None);
        assert_eq!(adapter_path_for_device("hci0"), None);
    }

    #[test]
    fn map_pairing_error_name_covers_bluez_names() {
        assert!(matches!(
            map_pairing_error_name("org.bluez.Error.AuthenticationFailed"),
            Error::Pairing(PairingStatus::AuthenticationFailed)
        ));
        assert!(matches!(
            map_pairing_error_name("org.bluez.Error.AuthenticationRejected"),
            Error::Pairing(PairingStatus::Rejected)
        ));
        assert!(matches!(
            map_pairing_error_name("org.bluez.Error.AuthenticationTimeout"),
            Error::Pairing(PairingStatus::TimedOut)
        ));
        assert!(matches!(
            map_pairing_error_name("org.bluez.Error.InProgress"),
            Error::Pairing(PairingStatus::InProgress)
        ));
        assert!(matches!(
            map_pairing_error_name("org.bluez.Error.NotSupported"),
            Error::Pairing(PairingStatus::NotCapable)
        ));
        assert!(matches!(
            map_pairing_error_name("org.bluez.Error.NotReady"),
            Error::Permission(PermissionReason::BluetoothOff)
        ));
        assert!(matches!(
            map_pairing_error_name("org.bluez.Error.Failed"),
            Error::Backend(_)
        ));
        assert!(is_already_paired_name("org.bluez.Error.AlreadyExists"));
        assert!(is_already_unpaired_name("org.bluez.Error.DoesNotExist"));
    }
}

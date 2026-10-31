//! Mapping of Web Bluetooth failures onto the unified [`Error`] model.
//!
//! The authoritative table is documented in `docs/error-mapping.md` next to
//! this module; the mapping itself is pure Rust so it can be unit-tested on
//! the host. Extracting `name`/`message` from the JS exception object happens
//! in the backend (it needs `js-sys`); everything here is host-testable.

use rblue_api::{DeviceId, Error, PermissionReason};

/// Platform label used by every [`Error::Unsupported`] this backend reports.
pub const PLATFORM: &str = "web";

/// Where a failing Web Bluetooth call was made from; refines the mapping for
/// exception names whose meaning depends on the call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorSite {
    /// `navigator.bluetooth.requestDevice` (the device chooser).
    RequestDevice,
    /// A GATT operation on a known device. `feature` names the rblue
    /// operation, reported verbatim in [`Error::Unsupported`].
    Gatt {
        /// Device the operation targeted.
        device: DeviceId,
        /// rblue operation name (e.g. `"connect"`, `"write_with_response"`).
        feature: &'static str,
    },
}

/// The rblue operation a site refers to; the `feature` half of
/// [`Error::Unsupported`].
#[must_use]
pub fn feature_of(site: &ErrorSite) -> &'static str {
    match site {
        ErrorSite::RequestDevice => "request_device",
        ErrorSite::Gatt { feature, .. } => feature,
    }
}

/// The `name`/`message` pair of a rejected JS error (a `DOMException`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomError<'a> {
    /// `DOMException.name`, e.g. `"NetworkError"`.
    pub name: &'a str,
    /// `DOMException.message`, possibly empty.
    pub message: &'a str,
}

/// Maps a DOM exception onto the unified error model.
///
/// # Errors
///
/// Always returns a value; this function *is* the mapping.
#[must_use]
pub fn map_dom_error(err: DomError<'_>, site: &ErrorSite) -> Error {
    match err.name {
        // The chooser closed with no selection (or nothing matched) is an
        // expected, user-driven outcome, not a lookup miss: `request_device`
        // documents it as a permission-shaped error.
        "NotFoundError" => match site {
            ErrorSite::RequestDevice => Error::Permission(PermissionReason::Unauthorized),
            ErrorSite::Gatt { device, .. } => Error::NotFound(device.clone()),
        },
        // Out-of-gesture requestDevice, insecure context, or touching a
        // service that was not in `optionalServices`.
        "SecurityError" => Error::Permission(PermissionReason::Unauthorized),
        // Connection attempt failed, or the link dropped before/during the
        // operation.
        "NetworkError" | "AbortError" => Error::Disconnected,
        // The browser does not implement this operation.
        "NotSupportedError" => Error::Unsupported {
            platform: PLATFORM,
            feature: feature_of(site),
        },
        // Wrong state (connecting an unselected device, double
        // startNotifications) or malformed arguments.
        "InvalidStateError" | "TypeError" | "TypeMismatchError" => {
            Error::InvalidState(format!("{}: {}", err.name, err.message))
        }
        // No structured status exists on the web platform, so the fallback is
        // the string form; `Error::Platform` is reserved for platforms that
        // actually report numeric codes.
        other => Error::Backend(format!("{other}: {}", err.message)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> DeviceId {
        DeviceId::Web("device-42".to_owned())
    }

    fn gatt_site(feature: &'static str) -> ErrorSite {
        ErrorSite::Gatt {
            device: device(),
            feature,
        }
    }

    #[test]
    fn request_device_not_found_is_a_permission_shaped_outcome() {
        let err = map_dom_error(
            DomError {
                name: "NotFoundError",
                message: "User cancelled the requestDevice() chooser.",
            },
            &ErrorSite::RequestDevice,
        );
        assert!(matches!(
            err,
            Error::Permission(PermissionReason::Unauthorized)
        ));
    }

    #[test]
    fn gatt_not_found_names_the_device() {
        let err = map_dom_error(
            DomError {
                name: "NotFoundError",
                message: "GATT Service not found",
            },
            &gatt_site("discover_services"),
        );
        match err {
            Error::NotFound(id) => assert_eq!(id, device()),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn security_errors_are_permission_denials() {
        let err = map_dom_error(
            DomError {
                name: "SecurityError",
                message: "Must be handling a user gesture.",
            },
            &ErrorSite::RequestDevice,
        );
        assert!(matches!(
            err,
            Error::Permission(PermissionReason::Unauthorized)
        ));
    }

    #[test]
    fn link_failures_map_to_disconnected() {
        for name in ["NetworkError", "AbortError"] {
            let err = map_dom_error(
                DomError {
                    name,
                    message: "Connection lost.",
                },
                &gatt_site("read"),
            );
            assert!(matches!(err, Error::Disconnected), "name: {name}");
        }
    }

    #[test]
    fn unsupported_operations_carry_the_operation_name() {
        let err = map_dom_error(
            DomError {
                name: "NotSupportedError",
                message: "GATT Write With Response is not supported",
            },
            &gatt_site("write_with_response"),
        );
        match err {
            Error::Unsupported { platform, feature } => {
                assert_eq!(platform, "web");
                assert_eq!(feature, "write_with_response");
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn state_errors_keep_the_browser_message() {
        let err = map_dom_error(
            DomError {
                name: "InvalidStateError",
                message: "GATT is already connecting",
            },
            &gatt_site("connect"),
        );
        match err {
            Error::InvalidState(text) => {
                assert!(text.contains("InvalidStateError"));
                assert!(text.contains("GATT is already connecting"));
            }
            other => panic!("expected InvalidState, got {other:?}"),
        }
    }

    #[test]
    fn unknown_names_fall_back_to_backend_with_context() {
        let err = map_dom_error(
            DomError {
                name: "QuotaExceededError",
                message: "Too many open connections.",
            },
            &gatt_site("connect"),
        );
        match err {
            Error::Backend(text) => {
                assert!(text.contains("QuotaExceededError"));
                assert!(text.contains("Too many open connections."));
            }
            other => panic!("expected Backend, got {other:?}"),
        }
    }

    #[test]
    fn feature_of_matches_the_site() {
        assert_eq!(feature_of(&ErrorSite::RequestDevice), "request_device");
        assert_eq!(feature_of(&gatt_site("subscribe")), "subscribe");
    }
}

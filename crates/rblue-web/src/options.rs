//! Pure assembly of `navigator.bluetooth.requestDevice` options.
//!
//! [`rblue_api::RequestDeviceOptions`] is validated and reshaped into one of
//! the two option objects the browser accepts *before* any JS is touched, so
//! the validation rules are unit-testable on the host. The wasm backend only
//! serializes the resulting plan into a `js_sys::Object`.

use std::collections::HashSet;

use rblue_api::{Error, RequestDeviceOptions};

/// A validated request-device plan: one of the two shapes
/// `navigator.bluetooth.requestDevice` accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestDevicePlan {
    /// `acceptAllDevices: true`. No filter, so the chooser lists every
    /// nearby device; `optionalServices` is still applied.
    AcceptAll {
        /// Merged `optionalServices` list as canonical lowercase
        /// hyphenated UUID strings.
        optional_services: Vec<String>,
    },
    /// A single filter (`filters: [filter]`) over advertised service UUIDs
    /// and/or the advertised name prefix. Within one filter the fields
    /// combine with AND; the rblue model exposes exactly one filter.
    Filtered {
        /// Service UUIDs the device must advertise (any-of) as
        /// canonical lowercase hyphenated strings.
        services: Vec<String>,
        /// Required advertised-name prefix, when set.
        name_prefix: Option<String>,
        /// Merged `optionalServices` list as canonical lowercase
        /// hyphenated UUID strings.
        optional_services: Vec<String>,
    },
}

/// Validates and reshapes [`RequestDeviceOptions`] into a
/// [`RequestDevicePlan`].
///
/// # Errors
///
/// [`Error::InvalidState`] when the combination cannot be expressed as a
/// `requestDevice` options object: filters (services/name prefix) are
/// mutually exclusive with `accept_all_devices`, and at least one of them
/// must be present otherwise.
pub fn plan_request_device(opts: &RequestDeviceOptions) -> Result<RequestDevicePlan, Error> {
    let optional_services = merged_optional_services(opts);
    let has_filter = !opts.services.is_empty() || opts.name_prefix.is_some();
    match (opts.accept_all_devices, has_filter) {
        (true, true) => Err(Error::InvalidState(
            "accept_all_devices cannot be combined with services or name_prefix on the web"
                .to_owned(),
        )),
        (true, false) => Ok(RequestDevicePlan::AcceptAll { optional_services }),
        (false, false) => Err(Error::InvalidState(
            "request_device needs services or name_prefix, or accept_all_devices set".to_owned(),
        )),
        (false, true) => Ok(RequestDevicePlan::Filtered {
            services: opts
                .services
                .iter()
                .map(|uuid| uuid.hyphenated().to_string())
                .collect(),
            name_prefix: opts.name_prefix.clone(),
            optional_services,
        }),
    }
}

/// Builds the `optionalServices` list: the caller-supplied optional services
/// merged with every service used as a filter, deduplicated in first-seen
/// order.
///
/// The merge is mandatory: a service absent from `optionalServices` at chooser
/// time makes the browser raise a `SecurityError` the first time the
/// application touches it, even if it was used to select the device.
#[must_use]
pub fn merged_optional_services(opts: &RequestDeviceOptions) -> Vec<String> {
    let mut seen = HashSet::new();
    opts.optional_services
        .iter()
        .chain(opts.services.iter())
        .filter(|uuid| seen.insert(**uuid))
        .map(|uuid| uuid.hyphenated().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    const HEART_RATE: Uuid = Uuid::from_u128(0x0000_180D_0000_1000_8000_0080_5F9B_34FB);
    const BATTERY: Uuid = Uuid::from_u128(0x0000_180F_0000_1000_8000_0080_5F9B_34FB);

    fn options(services: &[Uuid], prefix: Option<&str>, extra: &[Uuid]) -> RequestDeviceOptions {
        RequestDeviceOptions {
            services: services.to_vec(),
            name_prefix: prefix.map(str::to_owned),
            optional_services: extra.to_vec(),
            accept_all_devices: false,
        }
    }

    fn accept_all_options(extra: &[Uuid]) -> RequestDeviceOptions {
        RequestDeviceOptions {
            services: Vec::new(),
            name_prefix: None,
            optional_services: extra.to_vec(),
            accept_all_devices: true,
        }
    }

    #[test]
    fn filter_covers_services_and_prefix() {
        let plan = plan_request_device(&options(&[HEART_RATE], Some("Pulse"), &[]))
            .expect("valid options");
        match plan {
            RequestDevicePlan::Filtered {
                services,
                name_prefix,
                optional_services,
            } => {
                assert_eq!(services, vec![HEART_RATE.hyphenated().to_string()]);
                assert_eq!(name_prefix.as_deref(), Some("Pulse"));
                assert_eq!(optional_services, vec![HEART_RATE.hyphenated().to_string()]);
            }
            other => panic!("expected Filtered, got {other:?}"),
        }
    }

    #[test]
    fn filter_services_join_optional_services() {
        let plan =
            plan_request_device(&options(&[HEART_RATE], None, &[BATTERY])).expect("valid options");
        match plan {
            RequestDevicePlan::Filtered {
                optional_services, ..
            } => {
                assert_eq!(
                    optional_services,
                    vec![
                        BATTERY.hyphenated().to_string(),
                        HEART_RATE.hyphenated().to_string()
                    ]
                );
            }
            other => panic!("expected Filtered, got {other:?}"),
        }
    }

    #[test]
    fn optional_services_dedupe_preserving_first_seen_order() {
        let merged =
            merged_optional_services(&options(&[HEART_RATE], None, &[BATTERY, HEART_RATE]));
        assert_eq!(
            merged,
            vec![
                BATTERY.hyphenated().to_string(),
                HEART_RATE.hyphenated().to_string()
            ]
        );
    }

    #[test]
    fn accept_all_devices_alone_is_valid() {
        let plan = plan_request_device(&accept_all_options(&[BATTERY])).expect("valid options");
        assert_eq!(
            plan,
            RequestDevicePlan::AcceptAll {
                optional_services: vec![BATTERY.hyphenated().to_string()],
            }
        );
    }

    #[test]
    fn accept_all_devices_rejects_filters() {
        let mut conflicting = accept_all_options(&[]);
        conflicting.services = vec![HEART_RATE];
        let err =
            plan_request_device(&conflicting).expect_err("filters exclude accept_all_devices");
        assert!(matches!(err, Error::InvalidState(_)));
    }

    #[test]
    fn missing_filter_is_rejected() {
        let err = plan_request_device(&options(&[], None, &[]))
            .expect_err("the chooser needs a filter or accept_all_devices");
        assert!(matches!(err, Error::InvalidState(_)));
    }

    #[test]
    fn uuids_render_as_lowercase_hyphenated_strings() {
        assert_eq!(
            BATTERY.hyphenated().to_string(),
            "0000180f-0000-1000-8000-00805f9b34fb"
        );
    }
}

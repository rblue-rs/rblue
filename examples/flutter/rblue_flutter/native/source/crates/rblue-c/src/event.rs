//! Event envelope serialization for the event callback.
//!
//! Adapter events and GATT notifications share one callback, so both arrive
//! wrapped in a tagged envelope whose `type` field the embedder switches on.
//! The inner payloads are the serde forms of the `rblue-api` types
//! (`AdapterEvent` external tagging, `Notification` as an object).

use rblue_api::{AdapterEvent, DeviceId, Notification, SupervisorEvent};

/// One message of the event stream: an adapter event or a notification of one
/// connected device.
///
/// Serialized as `{"type":"adapter_event","event":{...}}` or
/// `{"type":"notification","device":{...},"notification":{...}}`.
#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum EventEnvelope<'a> {
    /// Adapter-level event (state, discoveries, connections) as
    /// `rblue_api::AdapterEvent` JSON.
    AdapterEvent {
        /// The adapter event.
        event: &'a AdapterEvent,
    },
    /// GATT notification/indication value from one subscribed characteristic
    /// of one device.
    Notification {
        /// The device that delivered the value, as `DeviceId` JSON.
        device: &'a DeviceId,
        /// Characteristic identity and value bytes.
        notification: &'a Notification,
    },
    /// Application-level connection supervisor event, distinct from adapter
    /// connection events and from the platform `auto_connect` flag.
    SupervisorEvent {
        /// The supervisor event.
        event: &'a SupervisorEvent,
    },
}

impl EventEnvelope<'_> {
    /// Serializes the envelope; JSON serialization of the event model cannot
    /// fail.
    pub(crate) fn to_json(&self) -> String {
        serde_json::to_string(self).expect("event envelope JSON serialization cannot fail")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rblue_api::{AdapterState, AddressType, CharacteristicId, ServiceId};
    use uuid::Uuid;

    fn characteristic_id() -> CharacteristicId {
        CharacteristicId {
            service: ServiceId {
                uuid: Uuid::from_u128(0x180F),
                instance: 0,
            },
            uuid: Uuid::from_u128(0x2A19),
            instance: 0,
        }
    }

    #[test]
    fn adapter_event_envelope_is_tagged() {
        let envelope = EventEnvelope::AdapterEvent {
            event: &AdapterEvent::StateChanged(AdapterState::PoweredOn),
        };
        assert_eq!(
            envelope.to_json(),
            r#"{"type":"adapter_event","event":{"state_changed":"powered_on"}}"#
        );
    }

    #[test]
    fn notification_envelope_routes_device() {
        let device = DeviceId::Mac([2, 0, 0, 0, 0, 1], AddressType::RandomStatic);
        let notification = Notification {
            id: characteristic_id(),
            data: vec![1, 2, 3],
        };
        let envelope = EventEnvelope::Notification {
            device: &device,
            notification: &notification,
        };
        assert_eq!(
            envelope.to_json(),
            r#"{"type":"notification","device":{"mac":[[2,0,0,0,0,1],"random_static"]},"notification":{"id":{"service":{"uuid":"00000000-0000-0000-0000-00000000180f","instance":0},"uuid":"00000000-0000-0000-0000-000000002a19","instance":0},"data":[1,2,3]}}"#
        );
    }
}

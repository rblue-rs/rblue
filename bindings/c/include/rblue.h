/*
 * rblue C ABI - the message-passing facade over the rblue BLE library.
 *
 * This header is the ffigen input for the Flutter example
 * (examples/flutter/ffigen.yaml -> lib/src/ffi_bindings.dart) and the C view
 * of crates/rblue-c/src/ffi.rs. Keep the two in sync; the surface is small
 * enough that a handwritten, documented header beats generated prose.
 *
 * # Shape
 *
 * One handle per client, one request entry point, two callbacks:
 *
 *   rblue_new -> RblueHandle*
 *   rblue_request(handle, req_id, method, params_json, payload, len)
 *       -> 0 (response arrives through rblue_response_cb)
 *   rblue_response_cb(req_id, status_json, payload, payload_len, user_data)
 *   rblue_event_cb(event_json, user_data)
 *
 * # JSON contracts (serde forms of the rblue-api types)
 *
 * - `params_json`: a JSON object of the method's parameters, using the serde
 *   shapes of `ScanFilter`, `DeviceId`, `CharacteristicId`, `WriteMode`
 *   (snake_case fields, externally tagged enums). `NULL` counts as `{}`.
 * - `status_json`: NULL on success; otherwise `rblue_api::Error` serialized
 *   with serde's default (PascalCase) variant tags, e.g.
 *   `{"InvalidLength":{"len":600,"max":514}}` or
 *   `{"Platform":{"code":133,"message":"GATT_ERROR"}}` - raw platform codes
 *   are preserved verbatim. Facade-level argument errors (unknown method,
 *   unparseable params) never reach the backend and are reported as
 *   `{"InvalidArgument":{"message":...}}`.
 * - `event_json`: a tagged envelope,
 *   `{"type":"adapter_event","event":<AdapterEvent JSON>}`,
 *   `{"type":"notification","device":<DeviceId JSON>,"notification":<Notification JSON>}`,
 *   or `{"type":"supervisor_event","event":<SupervisorEvent JSON>}`.
 *   `AdapterEvent` uses its serde form, e.g.
 *   `{"device_discovered":{"id":...,"advertisement":...}}`,
 *   `{"device_disconnected":[<DeviceId>,null|"local"|"remote"|"timed_out"|"platform"]}`.
 *
 * # Methods
 *
 *   backend_version    {}                              -> JSON string
 *   initialize         {}                              -> (empty)
 *   state              {}                              -> AdapterState JSON (snake_case string)
 *   set_powered        {"powered": true|false}         -> (empty)
 *   start_scan         ScanFilter JSON                 -> (empty)
 *   stop_scan          {}                              -> (empty)
 *   system_connected_peripherals {"services":[<UUID>,...]} -> [{ "id":..,"name":.. }] (Apple, Android, Linux; Unsupported elsewhere)
 *   peripherals        {}                              -> [{"id":..,"name":..}]
 *   connect            {"device": <DeviceId>, "auto_connect"?: bool, "mtu"?: u16}
 *                                          -> (empty)
 *                      auto_connect defaults to false (platform/stack
 *                      background connection, not the supervisor).
 *   auto_connect       {"device": <DeviceId>, "policy"?: ReconnectPolicy}
 *                                          -> (empty)
 *                      Application-level SupervisorHandle::auto_connect.
 *                      policy uses *_ms duration fields; omitted fields keep
 *                      defaults. A later call with a different policy fails.
 *   unwatch            {"device": <DeviceId>}          -> (empty)
 *                      Stops supervising the device (SupervisorHandle::unwatch).
 *   disconnect         {"device": <DeviceId>}          -> (empty)
 *   capabilities       {} -> capability bitmask JSON (u32)
 *   rssi / mtu         {"device": <DeviceId>} -> integer JSON
 *   request_mtu        {"device": <DeviceId>, "mtu": <u16>} -> integer JSON
 *                      (negotiated ATT MTU). Check MtuNegotiation first.
 *   max_write_len      {"device":..,"characteristic":..,"mode":..} -> integer JSON
 *   discover_services  {"device": <DeviceId>}          -> ServiceSet JSON
 *   read               {"device":..,"characteristic":..} -> raw value bytes
 *   write              {"device":..,"characteristic":..,"mode":.., "allow_long_write"?: bool} + payload -> (empty)
 *                      allow_long_write defaults to false (oversize -> InvalidLength).
 *   subscribe          {"device":..,"characteristic":..} -> (empty)
 *   unsubscribe        {"device":..,"characteristic":..} -> (empty)
 *   is_connected       {"device": <DeviceId>}          -> true|false
 *   set_connection_priority {"device": <DeviceId>, "priority": "low"|"balanced"|"high"} -> (empty) (Android, Windows; Unsupported elsewhere)
 *
 * # Threading and ownership
 *
 * - Both callbacks fire on Rust worker threads. Hosts must install
 *   cross-thread callbacks (Dart: `NativeCallable.listener`, never
 *   `isolateLocal`) and must not re-enter the same handle synchronously
 *   from inside a callback.
 * - `status_json` / `event_json` strings are OWNED by the callee: copy them,
 *   then release with `rblue_free_string`.
 * - The default response callback borrows `payload` only for the call.
 *   Async listeners must register with `rblue_set_owned_response_callback`.
 * - `rblue_request` copies all inputs before returning.
 * - `rblue_shutdown` stops native callback production and reports the final
 *   number of response and event ownership transfers. Async hosts must drain
 *   those counts before closing callback listeners, then call `rblue_free`.
 * - `rblue_free` shuts down if needed and frees the handle; no native callback
 *   fires after shutdown returns. The handle must not be used afterwards or
 *   freed twice. Explicit shutdown/free is required.
 * - Android: `Java_com_rblue_RblueNative_nativeInit` (linked into
 *   `librblue_c.so` from the rblue-android crate) must be called from the
 *   host before the first adapter operation; see the rblue_flutter plugin.
 */
#ifndef RBLUE_H
#define RBLUE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/** Synchronous `rblue_request` outcomes: accepted, null handle, bad string. */
#define RBLUE_REQUEST_OK 0
#define RBLUE_REQUEST_NULL_HANDLE (-1)
#define RBLUE_REQUEST_INVALID_STRING (-2)

/** Opaque facade handle over one backend adapter. */
typedef struct RblueHandle RblueHandle;

/**
 * One completed request. `status_json` is NULL on success and otherwise an
 * owned string to release with rblue_free_string. `payload` is a transient
 * borrow (NULL when `payload_len` is 0).
 */
typedef void (*rblue_response_cb)(uint64_t req_id,
                                  const char* status_json,
                                  const uint8_t* payload,
                                  size_t payload_len,
                                  void* user_data);

/** One adapter event or notification envelope (owned string). */
typedef void (*rblue_event_cb)(const char* event_json, void* user_data);

/**
 * Constructs the facade over the platform-selected backend (mock by default;
 * the aggregate rblue crate's feature selection on mobile targets).
 *
 * @returns the handle, or NULL when the backend cannot run on this target.
 */
RblueHandle* rblue_new(void);

/**
 * Stops native callback production and writes the final number of response and
 * event callbacks accepted by the host. Output pointers may be NULL. The handle
 * remains allocated; call rblue_free after async callback queues reach these
 * counts. Repeated calls return the same values.
 */
int32_t rblue_shutdown(RblueHandle* handle,
                       uint64_t* response_count,
                       uint64_t* event_count);

/**
 * Shuts down if needed and frees the handle. No native callback fires after
 * shutdown returns. Null is a no-op.
 */
void rblue_free(RblueHandle* handle);

/**
 * Registers (or with NULL clears) the response callback. Returns
 * RBLUE_REQUEST_OK or RBLUE_REQUEST_NULL_HANDLE.
 */
int32_t rblue_set_response_callback(RblueHandle* handle,
                                    rblue_response_cb callback,
                                    void* user_data);

/**
 * Registers (or with NULL clears) the event callback; the first registration
 * starts the event pump, so install it right after rblue_new. Returns
 * RBLUE_REQUEST_OK or RBLUE_REQUEST_NULL_HANDLE.
 */
int32_t rblue_set_event_callback(RblueHandle* handle,
                                 rblue_event_cb callback,
                                 void* user_data);

/**
 * Dispatches one request; see the header comment for methods, contracts, and
 * the synchronous rejection codes.
 *
 * @param req_id caller-chosen correlation id (use unique monotonic ids)
 * @param method NUL-terminated method name; NULL is rejected synchronously
 * @param params_json NUL-terminated JSON object; NULL counts as "{}"
 * @param payload binary write data; NULL when payload_len is 0
 */
int32_t rblue_request(RblueHandle* handle,
                      uint64_t req_id,
                      const char* method,
                      const char* params_json,
                      const uint8_t* payload,
                      size_t payload_len);

/** Releases a string rblue-c handed to a callback (returns ownership). */
void rblue_free_string(char* string);

/* Async callback variant. Each nonempty payload remains valid until released
 * with rblue_free_payload, including after the callback returns. */
int32_t rblue_set_owned_response_callback(RblueHandle* handle,
                                         rblue_response_cb callback,
                                         void* user_data);

/* Release a payload from the owned response callback with its original length. */
void rblue_free_payload(const uint8_t* payload, size_t len);

#ifdef __cplusplus
}
#endif

#endif /* RBLUE_H */

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
 *   `{"type":"adapter_event","event":<AdapterEvent JSON>}` or
 *   `{"type":"notification","device":<DeviceId JSON>,"notification":<Notification JSON>}`.
 *   `AdapterEvent` uses its serde form, e.g.
 *   `{"device_discovered":{"id":...,"advertisement":...}}`,
 *   `{"device_disconnected":[<DeviceId>,null|"local"|"remote"|"timed_out"|"platform"]}`.
 *
 * # Methods
 *
 *   backend_version    {}                              -> JSON string
 *   initialize         {}                              -> (empty)
 *   start_scan         ScanFilter JSON                 -> (empty)
 *   stop_scan          {}                              -> (empty)
 *   peripherals        {}                              -> [{"id":..,"name":..}]
 *   connect            {"device": <DeviceId>}          -> (empty)
 *   disconnect         {"device": <DeviceId>}          -> (empty)
 *   capabilities       {} -> capability bitmask JSON
 *   rssi / mtu         {"device": <DeviceId>} -> integer JSON
 *   max_write_len      {"device":..,"characteristic":..,"mode":..} -> integer JSON
 *   discover_services  {"device": <DeviceId>}          -> ServiceSet JSON
 *   read               {"device":..,"characteristic":..} -> raw value bytes
 *   write              {"device":..,"characteristic":..,"mode":..} + payload -> (empty)
 *   subscribe          {"device":..,"characteristic":..} -> (empty)
 *   unsubscribe        {"device":..,"characteristic":..} -> (empty)
 *   is_connected       {"device": <DeviceId>}          -> true|false
 *
 * # Threading and ownership
 *
 * - Both callbacks fire on Rust worker threads. Hosts must install
 *   cross-thread callbacks (Dart: `NativeCallable.listener`, never
 *   `isolateLocal`) and must not re-enter the same handle synchronously
 *   from inside a callback.
 * - `status_json` / `event_json` strings are OWNED by the callee: copy them,
 *   then release with `rblue_free_string`.
 * - `payload` is a TRANSIENT borrow, valid only until the callback returns.
 * - `rblue_request` copies all inputs before returning.
 * - `rblue_free` shuts down deterministically: no callback fires after it
 *   returns; the handle must not be used afterwards and must not be freed
 *   twice (a NativeFinalizer is the embedder's leak backstop only).
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
 * Shuts the facade down and frees the handle. No callback fires after the
 * call returns. Null is a no-op.
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

#ifdef __cplusplus
}
#endif

#endif /* RBLUE_H */

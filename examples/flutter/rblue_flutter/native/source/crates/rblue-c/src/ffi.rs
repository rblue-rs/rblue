//! The C ABI surface. Every function is `#[no_mangle] extern "C"`, `unsafe`
//! is concentrated at the pointer edges, and every unsafe block carries its
//! justification.
//!
//! See the crate documentation for the threading, ownership, and shutdown
//! contract; `bindings/c/include/rblue.h` is the C view of this module and is
//! kept in sync by hand (it is deliberately too small to warrant cbindgen).

use std::ffi::{c_char, CStr, CString};
use std::os::raw::c_void;
use std::ptr;

use crate::callbacks::{RblueEventCallback, RblueResponseCallback};
use crate::dispatch::Request;
use crate::handle::Core;

/// Synchronous rejection codes of [`rblue_request`]: the call itself was
/// invalid, no response will arrive.
pub const RBLUE_REQUEST_NULL_HANDLE: i32 = -1;
/// Synchronous rejection code: `method` or `params_json` was null or not
/// valid UTF-8.
pub const RBLUE_REQUEST_INVALID_STRING: i32 = -2;

/// An opaque facade handle over one backend adapter. Created by
/// [`rblue_new`], released by [`rblue_free`]; every other function takes it
/// and must not be called with a null or freed handle.
pub struct RblueHandle {
    core: Core,
}

/// The opaque facade handle in C: `RblueHandle*`.
pub type RblueHandlePtr = *mut RblueHandle;

/// Constructs the facade over the platform-selected backend (mock by default;
/// the aggregate `rblue` crate's feature-driven selection on mobile targets).
///
/// # Errors
///
/// Returns null when the backend cannot be constructed on this target. On
/// Android the JNI bridge (`com.rblue.RblueNative.nativeInit`, from the host)
/// must have run first.
#[must_use]
#[no_mangle]
pub extern "C" fn rblue_new() -> RblueHandlePtr {
    match Core::new() {
        Ok(core) => Box::into_raw(Box::new(RblueHandle { core })),
        Err(error) => {
            tracing::error!("rblue_new failed: {error}");
            ptr::null_mut()
        }
    }
}

/// Shuts the facade down if needed and frees the handle. Native pump and
/// forwarder tasks stop before this returns. Async embedders should call
/// [`rblue_shutdown`], drain the reported ownership transfers, and only then
/// call this function. The handle is invalid afterwards; callers must not free
/// it twice nor call any other function with it.
///
/// # Safety
///
/// `handle` must be null (no-op) or a pointer returned by `rblue_new` that
/// has not been freed yet and that no concurrent call is using.
#[no_mangle]
pub unsafe extern "C" fn rblue_free(handle: RblueHandlePtr) {
    if handle.is_null() {
        return;
    }
    // SAFETY: the caller guarantees sole ownership of a live handle per the
    // `# Safety` contract; the box is reconstructed exactly once.
    let handle = unsafe { Box::from_raw(handle) };
    let _ = handle.core.shutdown();
    drop(handle);
}

/// Stops all callback producers and returns the final callback delivery counts.
///
/// The handle remains allocated so the embedder can drain asynchronous callback
/// queues before calling [`rblue_free`]. Repeated calls return the same counts.
/// Returns [`RBLUE_REQUEST_NULL_HANDLE`] for a null handle.
///
/// # Safety
///
/// `handle` must be a live handle and `response_count` / `event_count` must be
/// null or writable pointers. No request or callback registration may race this
/// call.
#[no_mangle]
pub unsafe extern "C" fn rblue_shutdown(
    handle: RblueHandlePtr,
    response_count: *mut u64,
    event_count: *mut u64,
) -> i32 {
    let Some(handle) = handle_ref(handle) else {
        return RBLUE_REQUEST_NULL_HANDLE;
    };
    let (responses, events) = handle.core.shutdown();
    if !response_count.is_null() {
        // SAFETY: the caller guarantees a writable output pointer.
        unsafe { response_count.write(responses) };
    }
    if !event_count.is_null() {
        // SAFETY: the caller guarantees a writable output pointer.
        unsafe { event_count.write(events) };
    }
    0
}

/// Registers (or with a null `callback` clears) the response callback.
///
/// Returns 0 on success, [`RBLUE_REQUEST_NULL_HANDLE`] for a null handle.
/// The callback fires on Rust worker threads: embedders must install a
/// cross-thread callback (Dart `NativeCallable.listener`) and must not
/// re-enter the same handle synchronously from inside it.
#[no_mangle]
pub extern "C" fn rblue_set_response_callback(
    handle: RblueHandlePtr,
    callback: Option<RblueResponseCallback>,
    user_data: *mut c_void,
) -> i32 {
    let Some(handle) = handle_ref(handle) else {
        return RBLUE_REQUEST_NULL_HANDLE;
    };
    handle
        .core
        .set_response_callback(callback, user_data, false);
    0
}

/// Registers a response callback that owns each nonempty payload.
/// Release the original pointer and length with [`rblue_free_payload`].
/// Returns 0 on success or [`RBLUE_REQUEST_NULL_HANDLE`] for a null handle.
#[no_mangle]
pub extern "C" fn rblue_set_owned_response_callback(
    handle: RblueHandlePtr,
    callback: Option<RblueResponseCallback>,
    user_data: *mut c_void,
) -> i32 {
    let Some(handle) = handle_ref(handle) else {
        return RBLUE_REQUEST_NULL_HANDLE;
    };
    handle.core.set_response_callback(callback, user_data, true);
    0
}

/// Releases a payload delivered to an owned response callback. Null is a no-op.
///
/// # Safety
/// Pass the original pointer and length, exactly once, from a callback
/// registered with [`rblue_set_owned_response_callback`].
#[no_mangle]
pub unsafe extern "C" fn rblue_free_payload(payload: *const u8, len: usize) {
    if !payload.is_null() {
        let slice = std::ptr::slice_from_raw_parts_mut(payload.cast_mut(), len);
        // SAFETY: the caller returns the exact allocation from Box::into_raw.
        drop(unsafe { Box::from_raw(slice) });
    }
}

/// Registers (or with a null `callback` clears) the event callback.
///
/// The first registration starts the event pump; events fired before it are
/// not delivered, so install the callback right after `rblue_new`. Returns 0
/// on success, [`RBLUE_REQUEST_NULL_HANDLE`] for a null handle.
#[no_mangle]
pub extern "C" fn rblue_set_event_callback(
    handle: RblueHandlePtr,
    callback: Option<RblueEventCallback>,
    user_data: *mut c_void,
) -> i32 {
    let Some(handle) = handle_ref(handle) else {
        return RBLUE_REQUEST_NULL_HANDLE;
    };
    handle.core.set_event_callback(callback, user_data);
    0
}

/// Dispatches one request; the response (or an argument error) arrives
/// asynchronously through the registered response callback, correlated by
/// `req_id` (caller-chosen; use unique monotonic ids).
///
/// `method` selects the operation (`initialize`, `start_scan`, `stop_scan`,
/// `peripherals`, `connect`, `disconnect`, `discover_services`, `read`,
/// `write`, `subscribe`, `unsubscribe`, `is_connected`,
/// `backend_version`, `start_advertising`, `stop_advertising`,
/// `is_advertising`, `add_local_service`, `remove_local_service`,
/// `local_services`, `write_local_characteristic`,
/// `notify_local_characteristic`); `params_json` is a JSON object of the method's
/// rblue-api-shaped parameters (null counts as `{}`); `payload` carries
/// binary write data (`payload` may be null when `payload_len` is 0).
///
/// The call copies all inputs before returning, so the caller may free its
/// buffers immediately.
///
/// Returns 0 when the request was accepted (a response will arrive, possibly
/// an `invalid_argument` one), [`RBLUE_REQUEST_NULL_HANDLE`] for a null
/// handle, [`RBLUE_REQUEST_INVALID_STRING`] when `method`/`params_json` is
/// null or not UTF-8.
///
/// # Safety
///
/// `handle` must be a live handle; `method`/`params_json` (when non-null)
/// must point to valid NUL-terminated strings; `payload` (when
/// `payload_len > 0`) must be readable for `payload_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rblue_request(
    handle: RblueHandlePtr,
    req_id: u64,
    method: *const c_char,
    params_json: *const c_char,
    payload: *const u8,
    payload_len: usize,
) -> i32 {
    let Some(handle) = handle_ref(handle) else {
        return RBLUE_REQUEST_NULL_HANDLE;
    };
    let method = match unsafe { borrowed_string(method) } {
        Ok(Some(method)) => method,
        Ok(None) | Err(()) => return RBLUE_REQUEST_INVALID_STRING,
    };
    let params = match unsafe { borrowed_string(params_json) } {
        Ok(Some(params)) => params,
        Ok(None) => "{}".to_owned(),
        Err(()) => return RBLUE_REQUEST_INVALID_STRING,
    };
    let payload = if payload_len == 0 {
        Vec::new()
    } else {
        // SAFETY: the caller guarantees `payload` is readable for
        // `payload_len` bytes per the `# Safety` contract; the bytes are
        // copied immediately, never stored by reference.
        unsafe { std::slice::from_raw_parts(payload, payload_len) }.to_vec()
    };
    handle.core.spawn_request(
        req_id,
        Request {
            method,
            params,
            payload,
        },
    );
    0
}

/// Releases a string that rblue-c handed to a callback (`status_json`,
/// `event_json`): the callback copies what it needs, then returns ownership.
///
/// # Safety
///
/// `string` must be null (no-op) or a pointer this library handed to a
/// callback exactly once, not yet released.
#[no_mangle]
pub unsafe extern "C" fn rblue_free_string(string: *mut c_char) {
    if string.is_null() {
        return;
    }
    // SAFETY: the caller guarantees the pointer came from one of this
    // library's `CString::into_raw` handoffs and was not released before.
    drop(unsafe { CString::from_raw(string) });
}

/// Null-safe borrow of the handle.
fn handle_ref(handle: RblueHandlePtr) -> Option<&'static RblueHandle> {
    if handle.is_null() {
        None
    } else {
        // SAFETY: `rblue_free`'s contract makes the handle live for as long
        // as any call can observe it; the facade itself is internally
        // synchronized, so a shared borrow is sound.
        Some(unsafe { &*handle })
    }
}

/// Copies a borrowed C string into owned UTF-8; `Ok(None)` for null.
///
/// # Errors
///
/// Returns `Err(())` when the pointer is non-null but not valid UTF-8 (or not
/// a valid C string, which the embedder contract forbids).
unsafe fn borrowed_string(string: *const c_char) -> Result<Option<String>, ()> {
    if string.is_null() {
        return Ok(None);
    }
    // SAFETY: the caller guarantees a valid NUL-terminated string per the
    // `# Safety` contract.
    let borrowed = unsafe { CStr::from_ptr(string) };
    borrowed
        .to_str()
        .map(|s| Some(s.to_owned()))
        .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;
    use std::time::Duration;

    use super::*;
    use crate::testing::Recorder;

    /// One facade handle with recorder callbacks installed through the real
    /// ABI entry points.
    struct Harness {
        handle: *mut RblueHandle,
        recorder: Recorder,
    }

    impl Harness {
        fn new() -> Self {
            let handle = rblue_new();
            assert!(!handle.is_null(), "the mock backend must construct");
            let recorder = Recorder::attach_to_handle(handle);
            Self { handle, recorder }
        }

        /// Dispatches one request through `rblue_request`.
        fn request(&self, req_id: u64, method: &str, params: &str) -> i32 {
            let method = CString::new(method).expect("test method has no NUL");
            let params = CString::new(params).expect("test params have no NUL");
            // SAFETY: the harness owns a live handle and both strings are
            // valid NUL-terminated buffers for the duration of the call.
            unsafe {
                rblue_request(
                    self.handle,
                    req_id,
                    method.as_ptr(),
                    params.as_ptr(),
                    std::ptr::null(),
                    0,
                )
            }
        }

        /// Awaits the next response (the recorder already enforces a
        /// deadline).
        async fn next_response(&mut self) -> (u64, Option<String>, Vec<u8>) {
            self.recorder.next_response().await
        }

        /// Frees the handle through the real ABI entry point and returns the
        /// recorder (its leaked senders stay valid, per the callback
        /// contract).
        fn free(self) -> Recorder {
            // SAFETY: the harness owns the handle and frees it exactly once.
            unsafe { rblue_free(self.handle) };
            self.recorder
        }
    }

    #[tokio::test]
    async fn null_arguments_are_rejected_synchronously() {
        // SAFETY: exercising the documented null-argument rejections; no
        // dereference happens on the null paths.
        let code = unsafe {
            rblue_request(
                std::ptr::null_mut(),
                1,
                c"initialize".as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
            )
        };
        assert_eq!(code, RBLUE_REQUEST_NULL_HANDLE);
        let harness = Harness::new();
        // SAFETY: null method pointer, a documented synchronous rejection.
        let code = unsafe {
            rblue_request(
                harness.handle,
                1,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                0,
            )
        };
        assert_eq!(code, RBLUE_REQUEST_INVALID_STRING);
    }

    #[tokio::test]
    async fn invalid_utf8_params_are_rejected_synchronously() {
        let harness = Harness::new();
        let method = CString::new("initialize").expect("no NUL");
        let invalid = CString::new([0xFF, 0xFE]).expect("bytes have no NUL");
        // SAFETY: live handle; `invalid` is deliberately not UTF-8.
        let code = unsafe {
            rblue_request(
                harness.handle,
                1,
                method.as_ptr(),
                invalid.as_ptr(),
                std::ptr::null(),
                0,
            )
        };
        assert_eq!(code, RBLUE_REQUEST_INVALID_STRING);
        harness.free();
    }

    #[tokio::test]
    async fn responses_carry_req_id_payload_and_owned_status_strings() {
        let mut harness = Harness::new();
        assert_eq!(harness.request(42, "backend_version", "{}"), 0);
        let (req_id, status, payload) = harness.next_response().await;
        assert_eq!(req_id, 42);
        assert!(status.is_none());
        assert_eq!(
            String::from_utf8(payload).expect("utf-8"),
            "\"mock/in-memory\""
        );

        // An argument error surfaces as an owned status string the callee
        // releases through rblue_free_string, per the ABI contract.
        assert_eq!(harness.request(43, "teleport", "{}"), 0);
        let (req_id, status, payload) = harness.next_response().await;
        assert_eq!(req_id, 43);
        let status = status.expect("unknown method must fail");
        assert!(status.contains("InvalidArgument"), "{status}");
        assert!(payload.is_empty());
        let owned = CString::new(status).expect("status has no NUL");
        // SAFETY: the string was handed to the recorder callback by this
        // library and has not been released yet.
        unsafe { rblue_free_string(owned.into_raw()) };
        harness.free();
    }

    #[tokio::test]
    async fn shutdown_reports_final_callback_counts_and_is_idempotent() {
        let mut harness = Harness::new();
        assert_eq!(harness.request(1, "backend_version", "{}"), 0);
        let _ = harness.next_response().await;
        let mut responses = 0;
        let mut events = 0;
        // SAFETY: the harness owns a live handle and both outputs are writable.
        assert_eq!(
            unsafe { rblue_shutdown(harness.handle, &mut responses, &mut events) },
            0,
        );
        assert_eq!(responses, 1);
        let first = (responses, events);
        responses = 0;
        events = 0;
        // SAFETY: shutdown is idempotent and the handle remains allocated.
        assert_eq!(
            unsafe { rblue_shutdown(harness.handle, &mut responses, &mut events) },
            0,
        );
        assert_eq!((responses, events), first);
        harness.free();
    }

    #[tokio::test]
    async fn no_callback_fires_after_free() {
        let mut harness = Harness::new();
        assert_eq!(harness.request(1, "backend_version", "{}"), 0);
        let (_, status, _) = harness.next_response().await;
        assert!(status.is_none());
        // After rblue_free, the shutdown contract forbids any further
        // callback delivery.
        let mut recorder = harness.free();
        tokio::time::sleep(Duration::from_millis(300)).await;
        recorder.expect_no_notification().await;
    }

    #[test]
    fn freeing_null_is_a_no_op() {
        // SAFETY: null handling is part of the contract of both functions.
        unsafe {
            rblue_free(std::ptr::null_mut());
            rblue_free_string(std::ptr::null_mut());
        }
    }
}

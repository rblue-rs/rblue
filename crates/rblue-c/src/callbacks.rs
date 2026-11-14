//! Registered host callbacks and their invocation.
//!
//! The C ABI stores raw function pointer + opaque `user_data` pairs. The
//! pointers are opaque pass-through values: this crate never dereferences
//! `user_data`, it only forwards it, so soundness reduces to the documented
//! embedder contract - `user_data` must outlive the registration and tolerate
//! calls from Rust worker threads (Dart `NativeCallable.listener` state
//! qualifies: the listener hands the arguments to the owning isolate).

use std::ffi::CString;
use std::os::raw::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Response callback: one completed [`super::ffi::rblue_request`].
///
/// `status_json` is null on success and otherwise an owned, rblue-c allocated
/// JSON string the callee must release with
/// [`super::ffi::rblue_free_string`]. `payload` is a transient borrow valid
/// only until the callback returns unless registered with the owned variant
/// (null when `payload_len` is 0).
pub type RblueResponseCallback = unsafe extern "C" fn(
    req_id: u64,
    status_json: *const std::os::raw::c_char,
    payload: *const u8,
    payload_len: usize,
    user_data: *mut c_void,
);

/// Event callback: one adapter event or GATT notification envelope.
///
/// `event_json` is an owned, rblue-c allocated JSON string the callee must
/// release with [`super::ffi::rblue_free_string`].
pub type RblueEventCallback =
    unsafe extern "C" fn(event_json: *const std::os::raw::c_char, user_data: *mut c_void);

/// A registered response or event callback.
struct Registered<F> {
    function: F,
    user_data: *mut c_void,
}

// SAFETY: `user_data` is an opaque pointer owned by the embedder. rblue-c
// only stores and passes it through; the embedder contract (crate docs)
// requires it to stay valid for the registration's lifetime and to be
// callable from Rust worker threads.
unsafe impl<F> Send for Registered<F> {}
// SAFETY: see the `Send` impl; the callback itself is a plain function
// pointer, and the shared state it points to is the embedder's contract.
unsafe impl<F> Sync for Registered<F> {}

/// The two callback slots of one facade handle.
#[derive(Default)]
pub(crate) struct Callbacks {
    response: Mutex<Option<(Registered<RblueResponseCallback>, bool)>>,
    event: Mutex<Option<Registered<RblueEventCallback>>>,
    delivered_responses: AtomicU64,
    delivered_events: AtomicU64,
}

impl Callbacks {
    /// Registers (or with `None` clears) the response callback.
    pub(crate) fn set_response(
        &self,
        function: Option<RblueResponseCallback>,
        user_data: *mut c_void,
        owned_payload: bool,
    ) {
        *self.lock_response() = function.map(|function| {
            (
                Registered {
                    function,
                    user_data,
                },
                owned_payload,
            )
        });
    }

    /// Registers (or with `None` clears) the event callback.
    pub(crate) fn set_event(&self, function: Option<RblueEventCallback>, user_data: *mut c_void) {
        *self.lock_event() = function.map(|function| Registered {
            function,
            user_data,
        });
    }

    /// Snapshots the registered response callback, if any.
    fn response(&self) -> Option<(RblueResponseCallback, *mut c_void, bool)> {
        self.lock_response()
            .as_ref()
            .map(|(registered, owned)| (registered.function, registered.user_data, *owned))
    }

    /// Snapshots the registered event callback, if any.
    fn event(&self) -> Option<(RblueEventCallback, *mut c_void)> {
        self.lock_event()
            .as_ref()
            .map(|registered| (registered.function, registered.user_data))
    }

    /// Delivers one response: takes ownership of `status_json` (already
    /// `into_raw`, released by the callee via `rblue_free_string`) and of
    /// `payload`, whose bytes are borrowed for the call. Without a registered
    /// callback the response is dropped, which frees the string here.
    pub(crate) fn deliver_response(
        &self,
        req_id: u64,
        status_json: Option<CString>,
        payload: Vec<u8>,
    ) {
        let Some((function, user_data, owned_payload)) = self.response() else {
            return;
        };
        let status_ptr = status_json
            .map(|status| status.into_raw())
            .unwrap_or(std::ptr::null_mut());
        let (payload_ptr, payload_len) = if owned_payload && !payload.is_empty() {
            let len = payload.len();
            (
                Box::into_raw(payload.into_boxed_slice())
                    .cast::<u8>()
                    .cast_const(),
                len,
            )
        } else {
            split_payload(&payload)
        };
        // Count before invoking the host. The native call returns only after an
        // asynchronous listener has accepted the ownership transfer into its
        // queue, so shutdown can use this as a precise drain target.
        self.delivered_responses.fetch_add(1, Ordering::AcqRel);
        // SAFETY: `function` came from the embedder through
        // `rblue_set_response_callback`; `status_ptr` is either null or a
        // valid `CString::into_raw` allocation whose ownership the callee
        // takes over, and `payload_ptr`/`payload_len` describe a live byte
        // buffer valid during the call, or owned by the receiver when requested.
        unsafe { function(req_id, status_ptr, payload_ptr, payload_len, user_data) };
    }

    /// Delivers one event: takes ownership of `event_json` (already
    /// `into_raw`, released by the callee via `rblue_free_string`). Without a
    /// registered callback the event is dropped, which frees the string here.
    pub(crate) fn deliver_event(&self, event_json: CString) {
        let Some((function, user_data)) = self.event() else {
            return;
        };
        let event_ptr = event_json.into_raw();
        self.delivered_events.fetch_add(1, Ordering::AcqRel);
        // SAFETY: `function` came from the embedder through
        // `rblue_set_event_callback`; `event_ptr` is a valid
        // `CString::into_raw` allocation whose ownership the callee takes
        // over.
        unsafe { function(event_ptr, user_data) };
    }

    /// Returns callback counts after all native producers have stopped.
    pub(crate) fn delivered_counts(&self) -> (u64, u64) {
        (
            self.delivered_responses.load(Ordering::Acquire),
            self.delivered_events.load(Ordering::Acquire),
        )
    }

    fn lock_response(
        &self,
    ) -> std::sync::MutexGuard<'_, Option<(Registered<RblueResponseCallback>, bool)>> {
        self.response
            .lock()
            .expect("response callback state must not be poisoned")
    }

    fn lock_event(&self) -> std::sync::MutexGuard<'_, Option<Registered<RblueEventCallback>>> {
        self.event
            .lock()
            .expect("event callback state must not be poisoned")
    }
}

/// Null-when-empty view of a response payload, per the ABI contract.
fn split_payload(payload: &[u8]) -> (*const u8, usize) {
    if payload.is_empty() {
        (std::ptr::null(), 0)
    } else {
        (payload.as_ptr(), payload.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_payload_survives_callback_return() {
        unsafe extern "C" fn capture(
            _req_id: u64,
            _status: *const std::os::raw::c_char,
            payload: *const u8,
            len: usize,
            user_data: *mut c_void,
        ) {
            // SAFETY: the test passes a live, exclusively borrowed slot.
            unsafe { *user_data.cast::<(*const u8, usize)>() = (payload, len) };
        }

        let callbacks = Callbacks::default();
        let mut captured = (std::ptr::null::<u8>(), 0usize);
        callbacks.set_response(
            Some(capture),
            std::ptr::from_mut(&mut captured).cast(),
            true,
        );
        callbacks.deliver_response(1, None, vec![0, 0xff, 0x80, 0x41]);

        let (payload, len) = captured;
        assert!(!payload.is_null());
        // SAFETY: ownership of the payload was transferred to this callback.
        let bytes = unsafe { std::slice::from_raw_parts(payload, len) }.to_vec();
        unsafe { crate::rblue_free_payload(payload, len) };
        assert_eq!(bytes, [0, 0xff, 0x80, 0x41]);
    }
}

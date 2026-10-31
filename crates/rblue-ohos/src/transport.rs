//! Injection seam between the Rust core and the host-language driver.
//!
//! The registry never touches Node-API: it formats a [`DriverRequest`] and
//! hands it to a [`DriverTransport`], which delivers it to the host. The
//! ohos-rs facade (feature `napi`) implements the trait over
//! `ThreadsafeFunction`s; tests inject the in-memory [`RecordingTransport`].

use std::sync::Mutex;

use rblue_api::Error;
use rblue_driver::DriverRequest;

/// The host side of the `BleDriver` protocol, as seen by the Rust core.
///
/// Delivery semantics: [`DriverTransport::send`] returns once the host has
/// *accepted* the request; completion is reported out of band through
/// [`crate::DriverRegistry::complete_request`] (control plane) or a binary
/// frame carrying the same request id (data plane), and may arrive in any
/// order.
pub trait DriverTransport: Send + Sync + 'static {
    /// Delivers `request` (control plane) together with `payload` (binary
    /// data-plane bytes; empty except for `Write`, where the payload must not
    /// travel through JSON).
    ///
    /// # Errors
    ///
    /// [`Error::Backend`] when the host rejects the request or cannot deliver
    /// it (e.g. the driver method is missing or the ThreadsafeFunction queue
    /// is closing).
    fn send(&self, request: DriverRequest, payload: &[u8]) -> Result<(), Error>;
}

/// One request recorded by [`RecordingTransport`].
#[derive(Debug, Clone, PartialEq)]
pub struct SentRequest {
    /// Control-plane request as handed to the transport.
    pub request: DriverRequest,
    /// Binary data-plane payload (write bytes); empty for other commands.
    pub payload: Vec<u8>,
}

/// In-memory transport that records every request instead of delivering it.
///
/// Test and PoC helper for exercising the registry without a host: a test
/// inspects [`RecordingTransport::sent`] and then resolves the request
/// itself through [`crate::DriverRegistry::complete_request`] or
/// [`crate::DriverRegistry::handle_frame`].
#[derive(Debug, Default)]
pub struct RecordingTransport {
    sent: Mutex<Vec<SentRequest>>,
}

impl RecordingTransport {
    /// Creates an empty recording transport.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of all recorded requests, oldest first.
    #[must_use]
    pub fn sent(&self) -> Vec<SentRequest> {
        self.lock().clone()
    }

    /// Number of recorded requests.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// `true` when no request was recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<SentRequest>> {
        self.sent
            .lock()
            .expect("recording transport mutex poisoned by a panicking sender")
    }
}

impl DriverTransport for RecordingTransport {
    fn send(&self, request: DriverRequest, payload: &[u8]) -> Result<(), Error> {
        self.lock().push(SentRequest {
            request,
            payload: payload.to_vec(),
        });
        Ok(())
    }
}

//! # rblue-driver
//!
//! The `BleDriver` protocol: the single Rust-side source of truth for the
//! rblue Driver track, where a host-language implementation (ArkTS on
//! HarmonyOS, optionally Kotlin on Android later) drives the platform BLE
//! stack and is injected into the Rust core.
//!
//! The protocol has two planes:
//!
//! - **Control plane (JSON)**: requests, responses, events, and the version
//!   handshake, defined by the serde types in [`command`]. Low-frequency
//!   traffic only.
//! - **Data plane (binary frames)**: hot-path payloads (write data, read
//!   results, notifications) ride [`frame`]-encoded `ArrayBuffer`s with a
//!   fixed little-endian header, never JSON.
//!
//! Every request carries a `command::RequestId` (`u64`; `u32`
//! would wrap around on high-frequency long-lived connections). The ArkTS
//! interface and the `.d.ts` bindings are generated from these Rust types, so
//! they cannot drift; the protocol shape must not be forked per platform.
//!
//! This crate is a pure protocol definition: no Node-API, no UniFFI, no I/O.

#![warn(missing_docs)]

pub mod command;
pub mod frame;

pub use crate::command::{
    is_protocol_compatible, BleDriverCommand, DriverEvent, DriverHello, DriverHelloAck,
    DriverRequest, DriverResponse, DriverValue, RequestId, PROTOCOL_VERSION,
};
pub use crate::frame::{
    decode_frame_header, encode_frame_header, FrameHeader, FrameHeaderError, FrameKind,
    FRAME_HEADER_LEN,
};

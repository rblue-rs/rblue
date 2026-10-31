//! Data-plane binary frame header for hot-path payloads.
//!
//! Hot-path traffic (write data, read results, notification values) never
//! goes through JSON: it rides an `ArrayBuffer` prefixed with a fixed,
//! little-endian header so the host-language driver can parse it with plain
//! `DataView` accessors:
//!
//! ```text
//! offset 0  : req_id   u64 (little-endian) - 0 for unsolicited events
//! offset 8  : kind     u8  (one of FrameKind)
//! offset 9  : payload_len u32 (little-endian, bytes of payload that follow)
//! offset 13 : payload ...
//! ```
//!
//! [`FRAME_HEADER_LEN`] is the exact on-wire header size.

use crate::command::RequestId;

/// Fixed header size in bytes: `u64` req_id + `u8` kind + `u32` payload_len.
pub const FRAME_HEADER_LEN: usize = core::mem::size_of::<u64>() + 1 + core::mem::size_of::<u32>();

/// What a data-plane frame carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FrameKind {
    /// Request-direction payload of a `write` command.
    WritePayload = 1,
    /// Response-direction payload of a `read` result.
    ReadResult = 2,
    /// Event-direction notification/indication value.
    Notification = 3,
}

impl TryFrom<u8> for FrameKind {
    type Error = FrameHeaderError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::WritePayload),
            2 => Ok(Self::ReadResult),
            3 => Ok(Self::Notification),
            other => Err(FrameHeaderError::UnknownKind(other)),
        }
    }
}

impl From<FrameKind> for u8 {
    fn from(kind: FrameKind) -> Self {
        kind as u8
    }
}

/// Parsed prefix of one data-plane frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameHeader {
    /// Correlation id of the request this payload belongs to; `0` for
    /// unsolicited event frames.
    pub req_id: RequestId,
    /// Payload direction and purpose.
    pub kind: FrameKind,
    /// Payload length in bytes following the header.
    pub payload_len: u32,
}

/// Frame header encode/decode failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FrameHeaderError {
    /// The provided buffer cannot hold a header.
    #[error("frame buffer too small: needed {needed} bytes, got {got}")]
    TooSmall {
        /// Bytes required (at least [`FRAME_HEADER_LEN`]).
        needed: usize,
        /// Bytes provided.
        got: usize,
    },
    /// The kind byte is not a valid [`FrameKind`].
    #[error("unknown frame kind {0}")]
    UnknownKind(u8),
    /// The buffer ends before the declared payload is complete.
    #[error("frame payload truncated: header declares {declared} bytes, {available} available")]
    PayloadTruncated {
        /// Payload length declared by the header.
        declared: usize,
        /// Payload bytes actually available.
        available: usize,
    },
}

/// Encodes `header` into the first [`FRAME_HEADER_LEN`] bytes of `out`.
///
/// # Errors
///
/// [`FrameHeaderError::TooSmall`] when `out` cannot hold a header.
pub fn encode_frame_header(header: FrameHeader, out: &mut [u8]) -> Result<(), FrameHeaderError> {
    if out.len() < FRAME_HEADER_LEN {
        return Err(FrameHeaderError::TooSmall {
            needed: FRAME_HEADER_LEN,
            got: out.len(),
        });
    }
    out[0..8].copy_from_slice(&header.req_id.to_le_bytes());
    out[8] = header.kind as u8;
    out[9..13].copy_from_slice(&header.payload_len.to_le_bytes());
    Ok(())
}

/// Decodes the header prefix of `buf` and returns it together with the exact
/// payload slice.
///
/// # Errors
///
/// [`FrameHeaderError::TooSmall`] when `buf` cannot hold a header,
/// [`FrameHeaderError::UnknownKind`] for an invalid kind byte,
/// [`FrameHeaderError::PayloadTruncated`] when the declared payload exceeds
/// the remaining bytes.
pub fn decode_frame_header(buf: &[u8]) -> Result<(FrameHeader, &[u8]), FrameHeaderError> {
    if buf.len() < FRAME_HEADER_LEN {
        return Err(FrameHeaderError::TooSmall {
            needed: FRAME_HEADER_LEN,
            got: buf.len(),
        });
    }
    let mut req_id_bytes = [0u8; 8];
    req_id_bytes.copy_from_slice(&buf[0..8]);
    let kind = FrameKind::try_from(buf[8])?;
    let mut payload_len_bytes = [0u8; 4];
    payload_len_bytes.copy_from_slice(&buf[9..13]);
    let payload_len = u32::from_le_bytes(payload_len_bytes) as usize;
    let available = buf.len() - FRAME_HEADER_LEN;
    if available < payload_len {
        return Err(FrameHeaderError::PayloadTruncated {
            declared: payload_len,
            available,
        });
    }
    let header = FrameHeader {
        req_id: u64::from_le_bytes(req_id_bytes),
        kind,
        payload_len: payload_len as u32,
    };
    let payload = &buf[FRAME_HEADER_LEN..FRAME_HEADER_LEN + payload_len];
    Ok((header, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_header() -> FrameHeader {
        FrameHeader {
            req_id: 0x0102_0304_0506_0708,
            kind: FrameKind::Notification,
            payload_len: 4,
        }
    }

    #[test]
    fn header_len_is_thirteen_bytes() {
        assert_eq!(FRAME_HEADER_LEN, 13);
    }

    #[test]
    fn roundtrip_with_payload() {
        const PAYLOAD: [u8; 4] = [0xAA, 0xBB, 0xCC, 0xDD];
        let mut buf = [0u8; FRAME_HEADER_LEN + PAYLOAD.len()];
        encode_frame_header(sample_header(), &mut buf).expect("encode");
        buf[FRAME_HEADER_LEN..].copy_from_slice(&PAYLOAD);
        let (header, decoded_payload) = decode_frame_header(&buf).expect("decode");
        assert_eq!(header, sample_header());
        assert_eq!(decoded_payload, &PAYLOAD);
    }

    #[test]
    fn byte_layout_is_little_endian() {
        let mut buf = [0u8; FRAME_HEADER_LEN];
        encode_frame_header(sample_header(), &mut buf).expect("encode");
        // req_id 0x0102030405060708 little-endian starts with the low byte.
        assert_eq!(
            &buf[0..8],
            &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]
        );
        assert_eq!(buf[8], 3, "notification kind");
        assert_eq!(&buf[9..13], &[4, 0, 0, 0]);
    }

    #[test]
    fn too_small_buffer_errors() {
        let mut buf = [0u8; FRAME_HEADER_LEN - 1];
        assert_eq!(
            encode_frame_header(sample_header(), &mut buf),
            Err(FrameHeaderError::TooSmall {
                needed: FRAME_HEADER_LEN,
                got: FRAME_HEADER_LEN - 1
            })
        );
        assert_eq!(
            decode_frame_header(&buf),
            Err(FrameHeaderError::TooSmall {
                needed: FRAME_HEADER_LEN,
                got: FRAME_HEADER_LEN - 1
            })
        );
    }

    #[test]
    fn unknown_kind_errors() {
        let mut buf = [0u8; FRAME_HEADER_LEN];
        encode_frame_header(sample_header(), &mut buf).expect("encode");
        buf[8] = 0xFF;
        assert_eq!(
            decode_frame_header(&buf),
            Err(FrameHeaderError::UnknownKind(0xFF))
        );
    }

    #[test]
    fn truncated_payload_errors() {
        let mut buf = [0u8; FRAME_HEADER_LEN + 2];
        let header = FrameHeader {
            req_id: 1,
            kind: FrameKind::WritePayload,
            payload_len: 3,
        };
        encode_frame_header(header, &mut buf).expect("encode");
        assert_eq!(
            decode_frame_header(&buf),
            Err(FrameHeaderError::PayloadTruncated {
                declared: 3,
                available: 2
            })
        );
    }

    #[test]
    fn zero_length_payload_is_valid() {
        let mut buf = [0u8; FRAME_HEADER_LEN];
        let header = FrameHeader {
            req_id: 9,
            kind: FrameKind::ReadResult,
            payload_len: 0,
        };
        encode_frame_header(header, &mut buf).expect("encode");
        let (decoded, payload) = decode_frame_header(&buf).expect("decode");
        assert_eq!(decoded, header);
        assert!(payload.is_empty());
    }
}

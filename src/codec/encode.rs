//! `ServiceSearchAttributeRequest` PDU encoding (PDU ID 0x06). This is the
//! only PDU this crate ever constructs -- continuation-page requests are
//! the same shape with a different `continuation_state` value, not a
//! separate PDU. Every field width here is fixed by the spec; the only
//! variable-length piece is the continuation-state blob itself.

use bytes::{BufMut, BytesMut};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceSearchAttributeRequest {
    pub transaction_id: u16,
    /// The single 16-bit service class UUID to search for (this crate only
    /// ever searches one UUID at a time: PBAP, MAP, or `PublicBrowseGroup`).
    pub service_uuid16: u16,
    pub maximum_attribute_byte_count: u16,
    /// The attribute-ID range to request, encoded as a single uint32
    /// (high 16 bits = range start, low 16 bits = range end), matching the
    /// `0x0000ffff` "all attributes" pattern seen in every captured fixture.
    pub attribute_id_range: u32,
    /// Empty for a fresh request; echoes the prior response's continuation
    /// state for a continuation-page follow-up request.
    pub continuation_state: Vec<u8>,
}

const PDU_ID_SERVICE_SEARCH_ATTRIBUTE_REQUEST: u8 = 0x06;

// Data element descriptor bytes for the fixed shapes this request always
// uses, matching the captured fixtures exactly:
const SEQUENCE_LEN_3: u8 = 0x35; // Sequence, size index 5 (1-byte length follows)
const UUID16_DESCRIPTOR: u8 = 0x19; // UUID, size index 1 (2-byte value)
const UINT32_DESCRIPTOR: u8 = 0x0a; // Unsigned Integer, size index 2 (4-byte value)

/// Reasons a `ServiceSearchAttributeRequest` cannot be encoded. Both are
/// spec violations (continuation state is defined as at most 16 bytes, and
/// a request body of this fixed shape can never approach 64KiB) rather than
/// conditions expected in normal operation -- treated as caller errors, not
/// silently truncated onto the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    ContinuationStateTooLong { len: usize },
    BodyTooLong { len: usize },
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodeError::ContinuationStateTooLong { len } => {
                write!(f, "continuation state is {len} bytes, exceeds the 255-byte limit")
            }
            EncodeError::BodyTooLong { len } => {
                write!(f, "request body is {len} bytes, exceeds the 65535-byte limit")
            }
        }
    }
}

impl std::error::Error for EncodeError {}

/// Encode a `ServiceSearchAttributeRequest` PDU.
pub fn encode(request: &ServiceSearchAttributeRequest) -> Result<Vec<u8>, EncodeError> {
    let mut body = BytesMut::new();

    // ServiceSearchPattern: Sequence[UUID16] -- always 3 bytes of payload
    // (1-byte UUID descriptor + 2-byte UUID value).
    body.put_u8(SEQUENCE_LEN_3);
    body.put_u8(0x03);
    body.put_u8(UUID16_DESCRIPTOR);
    body.put_u16(request.service_uuid16);

    // MaximumAttributeByteCount: bare uint16, not wrapped in a data element.
    body.put_u16(request.maximum_attribute_byte_count);

    // AttributeIDList: Sequence[UnsignedInt32] -- always 5 bytes of payload
    // (1-byte uint32 descriptor + 4-byte value).
    body.put_u8(SEQUENCE_LEN_3);
    body.put_u8(0x05);
    body.put_u8(UINT32_DESCRIPTOR);
    body.put_u32(request.attribute_id_range);

    // ContinuationState: 1-byte length + that many bytes of blob.
    let continuation_len = u8::try_from(request.continuation_state.len()).map_err(|_| {
        EncodeError::ContinuationStateTooLong {
            len: request.continuation_state.len(),
        }
    })?;
    body.put_u8(continuation_len);
    body.put_slice(&request.continuation_state);

    let param_len = u16::try_from(body.len())
        .map_err(|_| EncodeError::BodyTooLong { len: body.len() })?;

    let mut pdu = BytesMut::with_capacity(body.len() + 5);
    pdu.put_u8(PDU_ID_SERVICE_SEARCH_ATTRIBUTE_REQUEST);
    pdu.put_u16(request.transaction_id);
    pdu.put_u16(param_len);
    pdu.put_slice(&body);

    Ok(pdu.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_decode(s: &str) -> Vec<u8> {
        (0..s.trim().len())
            .step_by(2)
            .map(|i| {
                let byte_str = s.trim().get(i..i + 2).unwrap_or("00");
                u8::from_str_radix(byte_str, 16).unwrap_or(0)
            })
            .collect()
    }

    #[test]
    fn encodes_pbap_request_matching_fixture() -> Result<(), String> {
        let request = ServiceSearchAttributeRequest {
            transaction_id: 0x0000,
            service_uuid16: 0x112f,
            maximum_attribute_byte_count: 0xffff,
            attribute_id_range: 0x0000_ffff,
            continuation_state: Vec::new(),
        };
        let expected = hex_decode(include_str!("../../tests/fixtures/pbap_request.hex"));
        let actual = encode(&request).map_err(|e| e.to_string())?;
        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn encodes_map_request_matching_fixture() -> Result<(), String> {
        let request = ServiceSearchAttributeRequest {
            transaction_id: 0x0000,
            service_uuid16: 0x1132,
            maximum_attribute_byte_count: 0xffff,
            attribute_id_range: 0x0000_ffff,
            continuation_state: Vec::new(),
        };
        let expected = hex_decode(include_str!("../../tests/fixtures/map_request.hex"));
        let actual = encode(&request).map_err(|e| e.to_string())?;
        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn encodes_browse_first_page_request_matching_fixture() -> Result<(), String> {
        let request = ServiceSearchAttributeRequest {
            transaction_id: 0x0000,
            service_uuid16: 0x1002,
            maximum_attribute_byte_count: 0xffff,
            attribute_id_range: 0x0000_ffff,
            continuation_state: Vec::new(),
        };
        let expected = hex_decode(include_str!("../../tests/fixtures/browse_page0_request.hex"));
        let actual = encode(&request).map_err(|e| e.to_string())?;
        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn encodes_browse_continuation_page_request_matching_fixture() -> Result<(), String> {
        // Second page of the browse sequence: tid 0x0001, continuation
        // state echoing page0's response continuation blob `00f3`.
        let request = ServiceSearchAttributeRequest {
            transaction_id: 0x0001,
            service_uuid16: 0x1002,
            maximum_attribute_byte_count: 0xffff,
            attribute_id_range: 0x0000_ffff,
            continuation_state: vec![0x00, 0xf3],
        };
        let expected = hex_decode(include_str!("../../tests/fixtures/browse_page1_request.hex"));
        let actual = encode(&request).map_err(|e| e.to_string())?;
        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn rejects_continuation_state_over_255_bytes() {
        let request = ServiceSearchAttributeRequest {
            transaction_id: 0,
            service_uuid16: 0x1002,
            maximum_attribute_byte_count: 0xffff,
            attribute_id_range: 0x0000_ffff,
            continuation_state: vec![0u8; 256],
        };
        assert_eq!(
            encode(&request),
            Err(EncodeError::ContinuationStateTooLong { len: 256 })
        );
    }
}

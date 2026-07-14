//! SDP PDU header: PDU ID (1 byte) + Transaction ID (uint16) +
//! Parameter Length (uint16), per Bluetooth Core Spec Vol 3, Part B,
//! Section 4.3. Every SDP PDU (request or response) starts with this.

use nom::IResult;
use nom::number::complete::{be_u8, be_u16};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PduHeader {
    pub pdu_id: u8,
    pub transaction_id: u16,
    pub parameter_length: u16,
}

/// Parse a PDU header: PDU ID, Transaction ID, Parameter Length, in order.
pub fn pdu_header(input: &[u8]) -> IResult<&[u8], PduHeader> {
    let (rest, pdu_id) = be_u8(input)?;
    let (rest, transaction_id) = be_u16(rest)?;
    let (rest, parameter_length) = be_u16(rest)?;
    Ok((
        rest,
        PduHeader {
            pdu_id,
            transaction_id,
            parameter_length,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &[u8]) -> Result<(&[u8], PduHeader), String> {
        pdu_header(input).map_err(|e| format!("pdu_header parse failed: {e}"))
    }

    #[test]
    fn decodes_service_search_attribute_response_header() -> Result<(), String> {
        let (rest, header) = parse(&[0x07, 0x00, 0x00, 0x00, 0x99, 0xde, 0xad])?;
        assert_eq!(header.pdu_id, 0x07);
        assert_eq!(header.transaction_id, 0x0000);
        assert_eq!(header.parameter_length, 0x0099);
        assert_eq!(rest, &[0xde, 0xad]);
        Ok(())
    }

    #[test]
    fn decodes_service_search_attribute_request_header() -> Result<(), String> {
        let (rest, header) = parse(&[0x06, 0x00, 0x00, 0x00, 0x0f])?;
        assert_eq!(header.pdu_id, 0x06);
        assert_eq!(header.transaction_id, 0x0000);
        assert_eq!(header.parameter_length, 0x000f);
        assert_eq!(rest, &[] as &[u8]);
        Ok(())
    }

    #[test]
    fn decodes_nonzero_transaction_id_from_continuation_page() -> Result<(), String> {
        let (rest, header) = parse(&[0x06, 0x00, 0x01, 0x00, 0x11])?;
        assert_eq!(header.pdu_id, 0x06);
        assert_eq!(header.transaction_id, 0x0001);
        assert_eq!(header.parameter_length, 0x0011);
        assert_eq!(rest, &[] as &[u8]);
        Ok(())
    }

    #[test]
    fn truncated_header_is_an_error() {
        let result = pdu_header(&[0x07, 0x00, 0x00]);
        assert!(result.is_err());
    }

    #[test]
    fn empty_input_is_an_error() {
        let result = pdu_header(&[]);
        assert!(result.is_err());
    }
}

//! `ServiceSearchAttributeResponse` PDU decoding, split into two stages:
//!
//! - Framing (`raw_response`): extract the header, the raw attribute-list
//!   bytes, and the continuation state. This is pure length-prefixed byte
//!   slicing and always succeeds on any well-formed page, even a
//!   continuation fragment whose attribute bytes are deliberately
//!   incomplete mid-structure.
//! - Interpretation (`decode_attribute_lists`): parse a complete attribute
//!   bytes blob into structured `(attrID, value)` records. Only valid once
//!   all continuation pages have been reassembled into one blob -- calling
//!   this on a single fragment page's bytes is expected to fail.

use nom::IResult;
use nom::bytes::complete::take;
use nom::error::{Error as NomError, ErrorKind, ParseError as _};
use nom::number::complete::{be_u8, be_u16};
use nom::Err as NomErr;

use super::element::{DataElement, data_element};
use super::pdu_header::{PduHeader, pdu_header};

fn reject(input: &[u8]) -> nom::Err<NomError<&[u8]>> {
    NomErr::Error(NomError::from_error_kind(input, ErrorKind::Verify))
}

/// One matched service record's attributes: `(attribute ID, value)` pairs,
/// per the odd/even attrID-then-value pairing rule in Vol 3 Part B S2.5.
pub type AttributeList = Vec<(u16, DataElement)>;

/// The framing-only view of a `ServiceSearchAttributeResponse` PDU: header
/// plus the raw byte regions, uninterpreted. `attribute_bytes` may be a
/// fragment if this page is part of a continuation sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawServiceSearchAttributeResponse {
    pub header: PduHeader,
    pub attribute_bytes: Vec<u8>,
    pub continuation_state: Vec<u8>,
}

/// Parse just the framing of a `ServiceSearchAttributeResponse` PDU: header,
/// attribute-lists byte count, that many raw attribute bytes (uninterpreted),
/// and the trailing continuation state. Never attempts to interpret
/// `attribute_bytes` as data elements, so this succeeds even on a single
/// continuation-page fragment.
pub fn raw_response(input: &[u8]) -> IResult<&[u8], RawServiceSearchAttributeResponse> {
    let (rest, header) = pdu_header(input)?;
    let (rest, attribute_bytes_len) = be_u16(rest)?;
    let (rest, attribute_bytes) = take(attribute_bytes_len)(rest)?;

    let (rest, continuation_len) = be_u8(rest)?;
    let (rest, continuation_bytes) = take(continuation_len)(rest)?;

    Ok((
        rest,
        RawServiceSearchAttributeResponse {
            header,
            attribute_bytes: attribute_bytes.to_vec(),
            continuation_state: continuation_bytes.to_vec(),
        },
    ))
}

/// Parse the flat sequence contents of one record's attribute list into
/// `(attrID, value)` pairs. Odd/even pairing: every entry is a uint16
/// attribute ID immediately followed by that attribute's value element.
/// Rejects a record whose element count is odd or whose pairing does not
/// match `UnsignedInt(id), value` -- either shape violates the spec's
/// mandatory attrID/value pairing and must not be silently truncated.
fn attribute_list(
    elements: &[DataElement],
) -> Result<AttributeList, nom::Err<NomError<&'static [u8]>>> {
    if elements.len() % 2 != 0 {
        return Err(reject(&[]));
    }
    elements
        .chunks_exact(2)
        .map(|pair| match pair {
            [DataElement::UnsignedInt(id), value] => u16::try_from(*id)
                .map(|id| (id, value.clone()))
                .map_err(|_| reject(&[])),
            _ => Err(reject(&[])),
        })
        .collect()
}

/// Interpret a complete (fully reassembled, if it came from a paginated
/// response) attribute-lists byte blob as structured attribute lists, one
/// per matched service record.
pub fn decode_attribute_lists(attribute_bytes: &[u8]) -> IResult<&[u8], Vec<AttributeList>> {
    let (rest, outer) = data_element(attribute_bytes)?;
    let attribute_lists = match outer {
        DataElement::Sequence(records) => records
            .into_iter()
            .map(|record| match record {
                DataElement::Sequence(elements) => attribute_list(&elements),
                _ => Ok(AttributeList::new()),
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| reject(attribute_bytes))?,
        _ => Vec::new(),
    };
    Ok((rest, attribute_lists))
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

    fn parse_raw(input: &[u8]) -> Result<(&[u8], RawServiceSearchAttributeResponse), String> {
        raw_response(input).map_err(|e| format!("raw_response parse failed: {e}"))
    }

    fn find_attr(list: &AttributeList, id: u16) -> Option<&DataElement> {
        list.iter()
            .find(|(attr_id, _)| *attr_id == id)
            .map(|(_, v)| v)
    }

    #[test]
    fn decodes_pbap_response_fixture() -> Result<(), String> {
        let bytes = hex_decode(include_str!("../../tests/fixtures/pbap_response.hex"));
        let (rest, raw) = parse_raw(&bytes)?;
        assert_eq!(rest, &[] as &[u8]);
        assert_eq!(raw.header.pdu_id, 0x07);
        assert_eq!(raw.header.transaction_id, 0x0000);
        assert_eq!(raw.continuation_state, Vec::<u8>::new());

        let (leftover, attribute_lists) = decode_attribute_lists(&raw.attribute_bytes)
            .map_err(|e| format!("decode_attribute_lists failed: {e}"))?;
        assert_eq!(leftover, &[] as &[u8]);
        assert_eq!(attribute_lists.len(), 1);

        let record = attribute_lists
            .first()
            .ok_or("expected at least one attribute list")?;

        // ServiceRecordHandle (0x0000) -> uint32 0x4f49112f
        assert_eq!(
            find_attr(record, 0x0000),
            Some(&DataElement::UnsignedInt(0x4f49_112f))
        );

        // ServiceClassIDList (0x0001) -> Sequence[UUID16(PBAP = 0x112f)]
        assert_eq!(
            find_attr(record, 0x0001),
            Some(&DataElement::Sequence(vec![DataElement::Uuid16(0x112f)]))
        );

        // ProtocolDescriptorList (0x0004)
        assert_eq!(
            find_attr(record, 0x0004),
            Some(&DataElement::Sequence(vec![
                DataElement::Sequence(vec![DataElement::Uuid16(0x0100)]),
                DataElement::Sequence(vec![
                    DataElement::Uuid16(0x0003),
                    DataElement::UnsignedInt(0x0d),
                ]),
                DataElement::Sequence(vec![DataElement::Uuid16(0x0008)]),
            ]))
        );

        Ok(())
    }

    #[test]
    fn decodes_map_response_fixture() -> Result<(), String> {
        let bytes = hex_decode(include_str!("../../tests/fixtures/map_response.hex"));
        let (rest, raw) = parse_raw(&bytes)?;
        assert_eq!(rest, &[] as &[u8]);

        let (leftover, attribute_lists) = decode_attribute_lists(&raw.attribute_bytes)
            .map_err(|e| format!("decode_attribute_lists failed: {e}"))?;
        assert_eq!(leftover, &[] as &[u8]);
        assert_eq!(attribute_lists.len(), 1);

        let record = attribute_lists
            .first()
            .ok_or("expected at least one attribute list")?;

        // ServiceClassIDList (0x0001) -> Sequence[UUID16(MAP = 0x1132)]
        assert_eq!(
            find_attr(record, 0x0001),
            Some(&DataElement::Sequence(vec![DataElement::Uuid16(0x1132)]))
        );

        // ProtocolDescriptorList (0x0004): RFCOMM channel is 2 for MAP
        assert_eq!(
            find_attr(record, 0x0004),
            Some(&DataElement::Sequence(vec![
                DataElement::Sequence(vec![DataElement::Uuid16(0x0100)]),
                DataElement::Sequence(vec![
                    DataElement::Uuid16(0x0003),
                    DataElement::UnsignedInt(0x02),
                ]),
                DataElement::Sequence(vec![DataElement::Uuid16(0x0008)]),
            ]))
        );

        Ok(())
    }

    #[test]
    fn decode_attribute_lists_rejects_a_record_with_an_odd_element_count() {
        // Outer Sequence[ Sequence[UnsignedInt(0), UnsignedInt(0), UnsignedInt(0xff)] ]:
        // the inner record has 3 elements, violating the mandatory
        // attrID/value pairing, so this must be a decode error rather than
        // silently dropping the trailing UnsignedInt(0xff).
        let bytes = [0x35, 0x08, 0x35, 0x06, 0x08, 0x00, 0x08, 0x00, 0x08, 0xff];
        let result = decode_attribute_lists(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn raw_response_frames_a_continuation_fragment_without_decoding_it() -> Result<(), String> {
        // A deliberate mid-structure fragment (the response continues on the next page).
        // Framing must still succeed and report the right continuation state even
        // though the attribute bytes aren't a complete, decodable element.
        let bytes = hex_decode(include_str!(
            "../../tests/fixtures/browse_page0_response.hex"
        ));
        let (rest, raw) = parse_raw(&bytes)?;
        assert_eq!(rest, &[] as &[u8]);

        assert_eq!(raw.continuation_state, vec![0x00, 0xf3]);

        // Confirm the fragment genuinely doesn't decode as one complete
        // element on its own -- this is the behavior raw framing must not
        // depend on.
        assert!(decode_attribute_lists(&raw.attribute_bytes).is_err());

        Ok(())
    }

    #[test]
    fn raw_response_reports_empty_continuation_state_on_final_page() -> Result<(), String> {
        let bytes = hex_decode(include_str!(
            "../../tests/fixtures/browse_page7_response.hex"
        ));
        let (_, raw) = parse_raw(&bytes)?;
        assert_eq!(raw.continuation_state, Vec::<u8>::new());
        Ok(())
    }
}


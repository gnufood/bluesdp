//! Recursive SDP data element decoding: sequences, and the top-level
//! `DataElement` enum used to represent anything a real response can
//! contain, including attribute types this crate never needs to construct
//! (Text String, Boolean, etc.) which must still be walked past correctly.

use nom::bytes::complete::take;
use nom::error::{Error as NomError, ErrorKind, ParseError as _};
use nom::number::complete::{be_u8, be_u16, be_u32};
use nom::{Err as NomErr, IResult};

use super::descriptor::{ElementSize, descriptor};

const UINT_TYPE: u8 = 1;
const UUID_TYPE: u8 = 3;
const SEQUENCE_TYPE: u8 = 6;

/// A decoded SDP data element. Only the variants this crate's target
/// attributes actually need are modeled; every other real-world type
/// (Boolean, Text String, URL, Data Element Alternative, ...) decodes to
/// `Unknown` so sequence walking can skip past it correctly without
/// needing to represent its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataElement {
    UnsignedInt(u128),
    Uuid16(u16),
    Sequence(Vec<DataElement>),
    Unknown,
}

fn reject(input: &[u8]) -> nom::Err<NomError<&[u8]>> {
    NomErr::Error(NomError::from_error_kind(input, ErrorKind::Verify))
}

fn payload_len(input: &[u8], size: ElementSize) -> IResult<&[u8], usize> {
    match size {
        ElementSize::Fixed(n) => Ok((input, usize::from(n))),
        ElementSize::VarLen8 => {
            let (rest, n) = be_u8(input)?;
            Ok((rest, usize::from(n)))
        }
        ElementSize::VarLen16 => {
            let (rest, n) = be_u16(input)?;
            Ok((rest, usize::from(n)))
        }
        ElementSize::VarLen32 => {
            let (rest, n) = be_u32(input)?;
            Ok((rest, n as usize))
        }
    }
}

/// Parse a big-endian unsigned integer of exactly `body` 's length (1, 2, or
/// 4 bytes; wider uint widths are outside this crate's scope and rejected).
fn uint_value(body: &[u8]) -> IResult<&[u8], u128> {
    match body.len() {
        1 => {
            let (rest, v) = be_u8(body)?;
            Ok((rest, u128::from(v)))
        }
        2 => {
            let (rest, v) = be_u16(body)?;
            Ok((rest, u128::from(v)))
        }
        4 => {
            let (rest, v) = be_u32(body)?;
            Ok((rest, u128::from(v)))
        }
        _ => Err(reject(body)),
    }
}

/// Parse a big-endian 16-bit UUID value from `body` (exactly 2 bytes).
fn uuid16_value(body: &[u8]) -> IResult<&[u8], u16> {
    match body.len() {
        2 => be_u16(body),
        _ => Err(reject(body)),
    }
}

fn sequence_contents(body: &[u8]) -> IResult<&[u8], Vec<DataElement>> {
    let mut remaining = body;
    let mut elements = Vec::new();
    while !remaining.is_empty() {
        let (rest, element) = data_element(remaining)?;
        elements.push(element);
        remaining = rest;
    }
    Ok((remaining, elements))
}

/// Parse one SDP data element, recursing into sequence contents.
pub fn data_element(input: &[u8]) -> IResult<&[u8], DataElement> {
    let (after_descriptor, desc) = descriptor(input)?;
    let (after_len, len) = payload_len(after_descriptor, desc.size)?;
    let (rest, body) = take(len)(after_len)?;

    let element = match desc.element_type {
        UINT_TYPE => {
            let (_, value) = uint_value(body)?;
            DataElement::UnsignedInt(value)
        }
        UUID_TYPE if desc.size == ElementSize::Fixed(2) => {
            let (_, value) = uuid16_value(body)?;
            DataElement::Uuid16(value)
        }
        SEQUENCE_TYPE => {
            let (_, elements) = sequence_contents(body)?;
            DataElement::Sequence(elements)
        }
        _ => DataElement::Unknown,
    };

    Ok((rest, element))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &[u8]) -> Result<(&[u8], DataElement), String> {
        data_element(input).map_err(|e| format!("data_element parse failed: {e}"))
    }

    #[test]
    fn decodes_a_bare_uint() -> Result<(), String> {
        let (rest, el) = parse(&[0x08, 0xff])?;
        assert_eq!(el, DataElement::UnsignedInt(0xff));
        assert_eq!(rest, &[] as &[u8]);
        Ok(())
    }

    #[test]
    fn decodes_a_bare_uuid16() -> Result<(), String> {
        let (rest, el) = parse(&[0x19, 0x11, 0x2f])?;
        assert_eq!(el, DataElement::Uuid16(0x112f));
        assert_eq!(rest, &[] as &[u8]);
        Ok(())
    }

    #[test]
    fn decodes_flat_sequence_of_one_uuid() -> Result<(), String> {
        // Sequence, size index 5 (1-byte length): descriptor 0x35, len 0x03, then one UUID16
        let (rest, el) = parse(&[0x35, 0x03, 0x19, 0x01, 0x00])?;
        assert_eq!(
            el,
            DataElement::Sequence(vec![DataElement::Uuid16(0x0100)])
        );
        assert_eq!(rest, &[] as &[u8]);
        Ok(())
    }

    #[test]
    fn decodes_flat_sequence_of_uuid_and_uint() -> Result<(), String> {
        // Sequence[UUID16(0x0003), UnsignedInt(0x0d)] -- the RFCOMM protocol entry shape
        let (rest, el) = parse(&[0x35, 0x05, 0x19, 0x00, 0x03, 0x08, 0x0d])?;
        assert_eq!(
            el,
            DataElement::Sequence(vec![
                DataElement::Uuid16(0x0003),
                DataElement::UnsignedInt(0x0d),
            ])
        );
        assert_eq!(rest, &[] as &[u8]);
        Ok(())
    }

    #[test]
    fn decodes_nested_sequence_matching_protocol_descriptor_list_fixture() -> Result<(), String> {
        // A ProtocolDescriptorList shape: three inner sequences (L2CAP, RFCOMM+channel, OBEX).
        let bytes: Vec<u8> = hex_decode(
            "351135031901003505190003080d3503190008",
        );
        let (rest, el) = parse(&bytes)?;
        assert_eq!(
            el,
            DataElement::Sequence(vec![
                DataElement::Sequence(vec![DataElement::Uuid16(0x0100)]),
                DataElement::Sequence(vec![
                    DataElement::Uuid16(0x0003),
                    DataElement::UnsignedInt(0x0d),
                ]),
                DataElement::Sequence(vec![DataElement::Uuid16(0x0008)]),
            ])
        );
        assert_eq!(rest, &[] as &[u8]);
        Ok(())
    }

    #[test]
    fn skips_unknown_element_type_by_size_rule() -> Result<(), String> {
        // Boolean (type 5, size index 0 -> 1 byte): descriptor 0x28, value 0x01.
        // Not modeled as a distinct variant; must decode as Unknown and be
        // skippable without derailing sequence parsing.
        let (rest, el) = parse(&[0x28, 0x01])?;
        assert_eq!(el, DataElement::Unknown);
        assert_eq!(rest, &[] as &[u8]);
        Ok(())
    }

    #[test]
    fn skips_unknown_variable_length_element_type() -> Result<(), String> {
        // Text String (type 4, size index 5 -> 1-byte length): descriptor 0x25,
        // len 0x02, then 2 bytes of string payload.
        let (rest, el) = parse(&[0x25, 0x02, b'h', b'i'])?;
        assert_eq!(el, DataElement::Unknown);
        assert_eq!(rest, &[] as &[u8]);
        Ok(())
    }

    #[test]
    fn sequence_containing_unknown_element_still_parses_siblings() -> Result<(), String> {
        // Sequence containing [Boolean(unknown, 1 byte), UnsignedInt(1 byte)]
        // -- descriptor 0x35, len 0x04, then `28 01` (bool) `08 2a` (uint 0x2a)
        let (rest, el) = parse(&[0x35, 0x04, 0x28, 0x01, 0x08, 0x2a])?;
        assert_eq!(
            el,
            DataElement::Sequence(vec![DataElement::Unknown, DataElement::UnsignedInt(0x2a)])
        );
        assert_eq!(rest, &[] as &[u8]);
        Ok(())
    }

    #[test]
    fn uuid16_value_reads_two_byte_value() -> Result<(), String> {
        let (rest, value) =
            uuid16_value(&[0x11, 0x2f]).map_err(|e| format!("uuid16_value failed: {e}"))?;
        assert_eq!(value, 0x112f);
        assert_eq!(rest, &[] as &[u8]);
        Ok(())
    }

    #[test]
    fn uuid16_value_rejects_wrong_length_body() {
        let result = uuid16_value(&[0x11]);
        assert!(result.is_err());
    }

    #[test]
    fn empty_input_is_an_error() {
        let result = data_element(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn truncated_sequence_is_an_error() {
        // Descriptor says length 0x05 but only 2 bytes follow.
        let result = data_element(&[0x35, 0x05, 0x19, 0x01]);
        assert!(result.is_err());
    }

    fn hex_decode(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| {
                let byte_str = s.get(i..i + 2).unwrap_or("00");
                u8::from_str_radix(byte_str, 16).unwrap_or(0)
            })
            .collect()
    }
}

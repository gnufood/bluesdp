//! SDP data element descriptor byte: type (5 bits) + size index (3 bits),
//! per Bluetooth Core Spec Vol 3, Part B, Section 3.2.

use nom::IResult;
use nom::Parser as _;
use nom::bits::bits;
use nom::bits::complete::take as take_bits;
use nom::error::Error as NomError;
use nom::sequence::pair;

/// How to determine an element's payload size from its size index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementSize {
    /// Size index 0-4: payload width is implied by the element type
    /// (0 bytes for Nil, otherwise 1/2/4/8/16 bytes depending on type).
    Fixed(u8),
    /// Size index 5: an 8-bit length field follows, giving the payload width.
    VarLen8,
    /// Size index 6: a 16-bit length field follows, giving the payload width.
    VarLen16,
    /// Size index 7: a 32-bit length field follows, giving the payload width.
    VarLen32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Descriptor {
    pub element_type: u8,
    pub size: ElementSize,
}

/// Resolve a 3-bit size index to its sizing rule, given the element type
/// (needed only for the `Fixed` case, since fixed width depends on type).
///
/// `size_index` is always in 0-7 since it is read as exactly 3 bits.
fn resolve_size(element_type: u8, size_index: u8) -> ElementSize {
    match size_index {
        0 => ElementSize::Fixed(u8::from(element_type != 0)),
        1 => ElementSize::Fixed(2),
        2 => ElementSize::Fixed(4),
        3 => ElementSize::Fixed(8),
        4 => ElementSize::Fixed(16),
        5 => ElementSize::VarLen8,
        6 => ElementSize::VarLen16,
        _ => ElementSize::VarLen32,
    }
}

/// Parse one SDP data element descriptor byte: 5-bit type, 3-bit size index.
pub fn descriptor(input: &[u8]) -> IResult<&[u8], Descriptor> {
    let bit_parser = pair(take_bits(5usize), take_bits(3usize));
    let (rest, (element_type, size_index)): (&[u8], (u8, u8)) =
        bits::<_, _, NomError<(&[u8], usize)>, NomError<&[u8]>, _>(bit_parser).parse(input)?;

    let size = resolve_size(element_type, size_index);

    Ok((rest, Descriptor { element_type, size }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &[u8]) -> Result<(&[u8], Descriptor), String> {
        descriptor(input).map_err(|e| format!("descriptor parse failed: {e}"))
    }

    #[test]
    fn nil_has_fixed_zero_size() -> Result<(), String> {
        // Nil: type 0, size index 0
        let (rest, desc) = parse(&[0x00])?;
        assert_eq!(rest, &[] as &[u8]);
        assert_eq!(desc.element_type, 0);
        assert_eq!(desc.size, ElementSize::Fixed(0));
        Ok(())
    }

    #[test]
    fn uint_size_index_0_is_one_byte() -> Result<(), String> {
        // Unsigned Integer: type 1, size index 0
        let (_, desc) = parse(&[0x08])?;
        assert_eq!(desc.element_type, 1);
        assert_eq!(desc.size, ElementSize::Fixed(1));
        Ok(())
    }

    #[test]
    fn uint_size_index_2_is_four_bytes() -> Result<(), String> {
        // Unsigned Integer: type 1, size index 2
        let (_, desc) = parse(&[0x0a])?;
        assert_eq!(desc.element_type, 1);
        assert_eq!(desc.size, ElementSize::Fixed(4));
        Ok(())
    }

    #[test]
    fn uuid_size_index_1_is_two_bytes() -> Result<(), String> {
        // UUID: type 3, size index 1
        let (_, desc) = parse(&[0x19])?;
        assert_eq!(desc.element_type, 3);
        assert_eq!(desc.size, ElementSize::Fixed(2));
        Ok(())
    }

    #[test]
    fn uuid_size_index_4_is_sixteen_bytes() -> Result<(), String> {
        // UUID: type 3, size index 4
        let (_, desc) = parse(&[0x1c])?;
        assert_eq!(desc.element_type, 3);
        assert_eq!(desc.size, ElementSize::Fixed(16));
        Ok(())
    }

    #[test]
    fn sequence_size_index_5_reads_one_byte_length() -> Result<(), String> {
        // Sequence: type 6, size index 5 -> one length byte follows
        let (rest, desc) = parse(&[0x35, 0x03])?;
        assert_eq!(desc.element_type, 6);
        assert_eq!(desc.size, ElementSize::VarLen8);
        assert_eq!(rest, &[0x03]);
        Ok(())
    }

    #[test]
    fn sequence_size_index_6_reads_two_byte_length() -> Result<(), String> {
        // Sequence: type 6, size index 6 -> two length bytes follow
        let (rest, desc) = parse(&[0x36, 0x00, 0x10])?;
        assert_eq!(desc.element_type, 6);
        assert_eq!(desc.size, ElementSize::VarLen16);
        assert_eq!(rest, &[0x00, 0x10]);
        Ok(())
    }

    #[test]
    fn text_string_size_index_7_reads_four_byte_length() -> Result<(), String> {
        // Text String: type 4, size index 7 -> four length bytes follow
        let (rest, desc) = parse(&[0x27, 0x00, 0x00, 0x01, 0x00])?;
        assert_eq!(desc.element_type, 4);
        assert_eq!(desc.size, ElementSize::VarLen32);
        assert_eq!(rest, &[0x00, 0x00, 0x01, 0x00]);
        Ok(())
    }

    #[test]
    fn empty_input_is_an_error() {
        let result = descriptor(&[]);
        assert!(result.is_err());
    }
}

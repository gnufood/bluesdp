//! Attribute extraction: pull the RFCOMM channel number out of a decoded
//! `ProtocolDescriptorList` attribute. Depends only on `AttributeList` /
//! `DataElement` -- nothing about PDU framing or pagination.

use super::element::DataElement;
use super::response::AttributeList;

const PROTOCOL_DESCRIPTOR_LIST_ATTR_ID: u16 = 0x0004;
const RFCOMM_UUID: u16 = 0x0003;

/// Find the RFCOMM channel number in a service record's attribute list, by
/// locating `ProtocolDescriptorList` (0x0004) and, within it, the protocol
/// stack entry for RFCOMM (`Sequence[Uuid16(0x0003), UnsignedInt(channel)]`).
///
/// Returns `None` if the attribute is absent, is not shaped as expected, or
/// does not contain an RFCOMM entry -- e.g. a record for a protocol that
/// does not run over RFCOMM.
pub fn rfcomm_channel(attributes: &AttributeList) -> Option<u8> {
    let protocol_descriptor_list = attributes
        .iter()
        .find(|(id, _)| *id == PROTOCOL_DESCRIPTOR_LIST_ATTR_ID)
        .map(|(_, value)| value)?;

    let DataElement::Sequence(protocol_stack) = protocol_descriptor_list else {
        return None;
    };

    protocol_stack.iter().find_map(|entry| {
        let DataElement::Sequence(fields) = entry else {
            return None;
        };
        match fields.as_slice() {
            [DataElement::Uuid16(RFCOMM_UUID), DataElement::UnsignedInt(channel)] => {
                u8::try_from(*channel).ok()
            }
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::response::{decode_attribute_lists, raw_response};

    fn hex_decode(s: &str) -> Vec<u8> {
        (0..s.trim().len())
            .step_by(2)
            .map(|i| {
                let byte_str = s.trim().get(i..i + 2).unwrap_or("00");
                u8::from_str_radix(byte_str, 16).unwrap_or(0)
            })
            .collect()
    }

    fn first_record_attributes(fixture: &str) -> Result<AttributeList, String> {
        let bytes = hex_decode(fixture);
        let (_, raw) = raw_response(&bytes).map_err(|e| format!("raw_response failed: {e}"))?;
        let (_, mut lists) = decode_attribute_lists(&raw.attribute_bytes)
            .map_err(|e| format!("decode_attribute_lists failed: {e}"))?;
        if lists.is_empty() {
            return Err("expected at least one attribute list".to_string());
        }
        Ok(lists.remove(0))
    }

    #[test]
    fn finds_rfcomm_channel_13_in_pbap_fixture() -> Result<(), String> {
        let attributes = first_record_attributes(include_str!(
            "../../tests/fixtures/pbap_response.hex"
        ))?;
        assert_eq!(rfcomm_channel(&attributes), Some(13));
        Ok(())
    }

    #[test]
    fn finds_rfcomm_channel_2_in_map_fixture() -> Result<(), String> {
        let attributes = first_record_attributes(include_str!(
            "../../tests/fixtures/map_response.hex"
        ))?;
        assert_eq!(rfcomm_channel(&attributes), Some(2));
        Ok(())
    }

    #[test]
    fn returns_none_when_protocol_descriptor_list_is_absent() {
        let attributes: AttributeList = vec![(0x0000, DataElement::UnsignedInt(0x1234))];
        assert_eq!(rfcomm_channel(&attributes), None);
    }

    #[test]
    fn returns_none_when_protocol_descriptor_list_has_no_rfcomm_entry() {
        // Only an L2CAP entry, no RFCOMM -- e.g. a protocol that runs
        // directly over L2CAP without RFCOMM in between.
        let attributes: AttributeList = vec![(
            0x0004,
            DataElement::Sequence(vec![DataElement::Sequence(vec![DataElement::Uuid16(
                0x0100,
            )])]),
        )];
        assert_eq!(rfcomm_channel(&attributes), None);
    }

    #[test]
    fn returns_none_when_protocol_descriptor_list_is_wrong_shape() {
        // Malformed: 0x0004 present but not a Sequence at all.
        let attributes: AttributeList = vec![(0x0004, DataElement::UnsignedInt(0xff))];
        assert_eq!(rfcomm_channel(&attributes), None);
    }
}

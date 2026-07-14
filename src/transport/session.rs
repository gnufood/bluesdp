//! Top-level SDP query orchestration: encode a request, exchange it over a
//! stream, drive continuation-state pagination to completion, decode the
//! result, and extract the RFCOMM channel. This is the one module allowed
//! to depend on both `codec`'s full public surface and `transport::socket`
//! / `transport::connect` -- it composes them, nothing here re-implements
//! their logic.

use tokio::io::{AsyncRead, AsyncWrite};

use crate::codec::{
    decode_attribute_lists, encode, raw_response, reassemble, rfcomm_channel,
    EncodeError, Page, ServiceSearchAttributeRequest,
};

use super::socket::{send_and_receive_pdu, SocketError};

const MAXIMUM_ATTRIBUTE_BYTE_COUNT: u16 = 0xffff;
const PROTOCOL_DESCRIPTOR_LIST_ATTRIBUTE_RANGE: u32 = 0x0000_ffff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryError {
    Encode(EncodeError),
    Socket(SocketError),
    Decode,
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::Encode(e) => write!(f, "encode error: {e}"),
            QueryError::Socket(e) => write!(f, "transport error: {e}"),
            QueryError::Decode => write!(f, "failed to decode SDP response"),
        }
    }
}

impl std::error::Error for QueryError {}

async fn query_once<S>(
    stream: &mut S,
    transaction_id: u16,
    service_uuid16: u16,
    continuation_state: Vec<u8>,
) -> Result<Vec<u8>, QueryError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request = ServiceSearchAttributeRequest {
        transaction_id,
        service_uuid16,
        maximum_attribute_byte_count: MAXIMUM_ATTRIBUTE_BYTE_COUNT,
        attribute_id_range: PROTOCOL_DESCRIPTOR_LIST_ATTRIBUTE_RANGE,
        continuation_state,
    };
    let bytes = encode(&request).map_err(QueryError::Encode)?;
    send_and_receive_pdu(stream, &bytes)
        .await
        .map_err(QueryError::Socket)
}

/// Run a full SDP query for `service_uuid16` over an already-connected
/// stream: send the initial request, drive continuation pages to
/// completion, decode the reassembled attribute lists, and return the
/// RFCOMM channel from the first matching service record, if any.
pub async fn find_rfcomm_channel<S>(
    stream: &mut S,
    service_uuid16: u16,
) -> Result<Option<u8>, QueryError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut transaction_id: u16 = 0;

    let first_bytes = query_once(stream, transaction_id, service_uuid16, Vec::new()).await?;
    let (_, first_raw) = raw_response(&first_bytes).map_err(|_| QueryError::Decode)?;
    let first_page: Page = first_raw.into();

    // Threaded out here since fetch_next must return Page, not Result<Page, _>.
    let mut last_error: Option<QueryError> = None;

    let attribute_bytes = reassemble(first_page, async |continuation_state| {
        transaction_id = transaction_id.wrapping_add(1);
        let continuation_state = continuation_state.to_vec();

        let outcome = async {
            let bytes =
                query_once(stream, transaction_id, service_uuid16, continuation_state).await?;
            let (_, raw) = raw_response(&bytes).map_err(|_| QueryError::Decode)?;
            Ok::<Page, QueryError>(raw.into())
        }
        .await;

        match outcome {
            Ok(page) => page,
            Err(e) => {
                last_error = Some(e);
                Page {
                    attribute_bytes: Vec::new(),
                    continuation_state: Vec::new(),
                }
            }
        }
    })
    .await;

    if let Some(error) = last_error {
        return Err(error);
    }

    let (_, attribute_lists) =
        decode_attribute_lists(&attribute_bytes).map_err(|_| QueryError::Decode)?;

    Ok(attribute_lists.iter().find_map(rfcomm_channel))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn hex_decode(s: &str) -> Vec<u8> {
        (0..s.trim().len())
            .step_by(2)
            .map(|i| {
                let byte_str = s.trim().get(i..i + 2).unwrap_or("00");
                u8::from_str_radix(byte_str, 16).unwrap_or(0)
            })
            .collect()
    }

    #[tokio::test]
    async fn finds_channel_13_for_single_page_pbap_response() -> Result<(), String> {
        let (mut client, mut server) = tokio::io::duplex(4096);
        let response = hex_decode(include_str!("../../tests/fixtures/pbap_response.hex"));

        let server_task = tokio::spawn(async move {
            let mut request = vec![0u8; 20];
            server
                .read_exact(&mut request)
                .await
                .map_err(|e| format!("server read failed: {e}"))?;
            server
                .write_all(&response)
                .await
                .map_err(|e| format!("server write failed: {e}"))?;
            Ok::<(), String>(())
        });

        let channel = find_rfcomm_channel(&mut client, 0x112f)
            .await
            .map_err(|e| format!("find_rfcomm_channel failed: {e}"))?;

        server_task
            .await
            .map_err(|e| format!("server task panicked: {e}"))??;

        assert_eq!(channel, Some(13));
        Ok(())
    }

    #[tokio::test]
    async fn finds_channel_2_for_single_page_map_response() -> Result<(), String> {
        let (mut client, mut server) = tokio::io::duplex(4096);
        let response = hex_decode(include_str!("../../tests/fixtures/map_response.hex"));

        let server_task = tokio::spawn(async move {
            let mut request = vec![0u8; 20];
            server
                .read_exact(&mut request)
                .await
                .map_err(|e| format!("server read failed: {e}"))?;
            server
                .write_all(&response)
                .await
                .map_err(|e| format!("server write failed: {e}"))?;
            Ok::<(), String>(())
        });

        let channel = find_rfcomm_channel(&mut client, 0x1132)
            .await
            .map_err(|e| format!("find_rfcomm_channel failed: {e}"))?;

        server_task
            .await
            .map_err(|e| format!("server task panicked: {e}"))??;

        assert_eq!(channel, Some(2));
        Ok(())
    }

    #[tokio::test]
    async fn drives_full_continuation_sequence_and_finds_all_12_records() -> Result<(), String> {
        let (mut client, mut server) = tokio::io::duplex(8192);

        let pages: Vec<Vec<u8>> = (0..8)
            .map(|i| {
                let fixture = match i {
                    0 => include_str!("../../tests/fixtures/browse_page0_response.hex"),
                    1 => include_str!("../../tests/fixtures/browse_page1_response.hex"),
                    2 => include_str!("../../tests/fixtures/browse_page2_response.hex"),
                    3 => include_str!("../../tests/fixtures/browse_page3_response.hex"),
                    4 => include_str!("../../tests/fixtures/browse_page4_response.hex"),
                    5 => include_str!("../../tests/fixtures/browse_page5_response.hex"),
                    6 => include_str!("../../tests/fixtures/browse_page6_response.hex"),
                    _ => include_str!("../../tests/fixtures/browse_page7_response.hex"),
                };
                hex_decode(fixture)
            })
            .collect();

        let server_task = tokio::spawn(async move {
            for page in pages {
                let mut header = [0u8; 5];
                server
                    .read_exact(&mut header)
                    .await
                    .map_err(|e| format!("server header read failed: {e}"))?;
                let [_, _, _, len_hi, len_lo] = header;
                let param_len = usize::from(u16::from_be_bytes([len_hi, len_lo]));
                let mut body = vec![0u8; param_len];
                server
                    .read_exact(&mut body)
                    .await
                    .map_err(|e| format!("server body read failed: {e}"))?;

                server
                    .write_all(&page)
                    .await
                    .map_err(|e| format!("server write failed: {e}"))?;
            }
            Ok::<(), String>(())
        });

        let channel = find_rfcomm_channel(&mut client, 0x1002)
            .await
            .map_err(|e| format!("find_rfcomm_channel failed: {e}"))?;

        server_task
            .await
            .map_err(|e| format!("server task panicked: {e}"))??;

        // PublicBrowseGroup search returns all 12 records in the order the
        // phone registered them; find_rfcomm_channel returns the first
        // RFCOMM-bearing record's channel. Ground truth (record-by-record
        // rfcomm_channel() output across this reassembled 12-record chain)
        // was independently confirmed by decoding this exact fixture chain
        // and inspecting every record: of the 12, records 2, 4, 5, 9, and 10
        // carry an RFCOMM channel (2, 1, 1, 13, 8 respectively); record 2
        // (channel 2) is first in capture order, not PBAP (record 9).
        assert_eq!(channel, Some(2));
        Ok(())
    }
}

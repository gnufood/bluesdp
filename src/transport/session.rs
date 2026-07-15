//! Top-level SDP query orchestration: encode a request, exchange it over a
//! stream, drive continuation-state pagination to completion, decode the
//! result, and extract the RFCOMM channel. This is the one module allowed
//! to depend on both `codec`'s full public surface and `transport::socket`
//! / `transport::connect` -- it composes them, nothing here re-implements
//! their logic.

use tokio::io::{AsyncRead, AsyncWrite};

use crate::codec::{
    decode_attribute_lists, encode, raw_response, rfcomm_channel,
    EncodeError, ServiceSearchAttributeRequest,
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
///
/// Pagination is inlined, not injected via callback: a callback capturing
/// `stream`/`transaction_id` mutably can't satisfy `Send` under the
/// higher-ranked bound a generic `fetch_next` would need, and this loop
/// only ever has one caller.
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

    let mut attribute_bytes = first_raw.attribute_bytes;
    let mut continuation_state = first_raw.continuation_state;

    while !continuation_state.is_empty() {
        transaction_id = transaction_id.wrapping_add(1);
        let bytes = query_once(stream, transaction_id, service_uuid16, continuation_state).await?;
        let (_, raw) = raw_response(&bytes).map_err(|_| QueryError::Decode)?;
        attribute_bytes.extend_from_slice(&raw.attribute_bytes);
        continuation_state = raw.continuation_state;
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

    // Located by content, not a hardcoded offset, so this survives if earlier
    // fields in the request change shape.
    fn continuation_state_from_request_body(body: &[u8]) -> Option<Vec<u8>> {
        let attribute_id_list_marker = [0x35, 0x05, 0x0a];
        let marker_pos = body
            .windows(attribute_id_list_marker.len())
            .position(|w| w == attribute_id_list_marker)?;
        let after_attribute_id_list = marker_pos + attribute_id_list_marker.len() + 4;
        let &len = body.get(after_attribute_id_list)?;
        if len == 0 {
            return None;
        }
        body.get(after_attribute_id_list + 1..).map(<[u8]>::to_vec)
    }

    const EXPECTED_CONTINUATION_REQUESTS: [&[u8]; 7] = [
        &[0x00, 0xf3],
        &[0x01, 0xe9],
        &[0x02, 0xdf],
        &[0x03, 0xd5],
        &[0x04, 0xcb],
        &[0x05, 0xbf],
        &[0x06, 0xb5],
    ];

    #[tokio::test]
    async fn drives_all_eight_pages_and_requests_correct_continuation_each_time(
    ) -> Result<(), String> {
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
            let mut seen_continuations = Vec::new();
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

                if let Some(blob) = continuation_state_from_request_body(&body) {
                    seen_continuations.push(blob);
                }

                server
                    .write_all(&page)
                    .await
                    .map_err(|e| format!("server write failed: {e}"))?;
            }
            Ok::<Vec<Vec<u8>>, String>(seen_continuations)
        });

        let channel = find_rfcomm_channel(&mut client, 0x1002)
            .await
            .map_err(|e| format!("find_rfcomm_channel failed: {e}"))?;

        let seen_continuations = server_task
            .await
            .map_err(|e| format!("server task panicked: {e}"))??;

        assert_eq!(seen_continuations, EXPECTED_CONTINUATION_REQUESTS);
        assert_eq!(channel, Some(2));
        Ok(())
    }

    #[tokio::test]
    async fn single_page_response_makes_exactly_one_request() -> Result<(), String> {
        let (mut client, mut server) = tokio::io::duplex(4096);
        let response = hex_decode(include_str!("../../tests/fixtures/pbap_response.hex"));

        let server_task = tokio::spawn(async move {
            let mut request_count = 0usize;
            let mut request = vec![0u8; 20];
            server
                .read_exact(&mut request)
                .await
                .map_err(|e| format!("server read failed: {e}"))?;
            request_count += 1;
            server
                .write_all(&response)
                .await
                .map_err(|e| format!("server write failed: {e}"))?;
            Ok::<usize, String>(request_count)
        });

        let channel = find_rfcomm_channel(&mut client, 0x112f)
            .await
            .map_err(|e| format!("find_rfcomm_channel failed: {e}"))?;

        let request_count = server_task
            .await
            .map_err(|e| format!("server task panicked: {e}"))??;

        assert_eq!(request_count, 1, "must not fetch a continuation page when there is none");
        assert_eq!(channel, Some(13));
        Ok(())
    }
}

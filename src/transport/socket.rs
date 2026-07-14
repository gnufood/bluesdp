//! Byte-level SDP request/response exchange over any async stream. Knows
//! nothing about SDP PDU semantics beyond the PDU header's fixed shape
//! (needed only to know how many more bytes to read) -- encoding,
//! decoding, and continuation-state interpretation live in `codec`.

use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

/// PDU header size: 1-byte PDU ID + 2-byte transaction ID + 2-byte
/// parameter length (Bluetooth Core Spec Vol 3, Part B, Section 4.3).
const PDU_HEADER_LEN: usize = 5;

/// Matches `BlueZ`'s `SDP_RESPONSE_TIMEOUT` (lib/sdp.c / sdp.h), applied per
/// read while waiting for a response.
pub const RESPONSE_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketError {
    Io,
    Timeout,
}

impl std::fmt::Display for SocketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SocketError::Io => write!(f, "I/O error during SDP exchange"),
            SocketError::Timeout => write!(f, "timed out waiting for SDP response"),
        }
    }
}

impl std::error::Error for SocketError {}

/// Send `request` bytes and read back exactly one complete SDP PDU: the
/// 5-byte header, then however many more bytes `parameter_length` says
/// follow. Bounded by `RESPONSE_TIMEOUT`. Does not interpret the PDU
/// contents -- returns the raw bytes for `codec::response::raw_response`
/// to parse.
pub async fn send_and_receive_pdu<S>(stream: &mut S, request: &[u8]) -> Result<Vec<u8>, SocketError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::time::timeout(RESPONSE_TIMEOUT, stream.write_all(request))
        .await
        .map_err(|_| SocketError::Timeout)?
        .map_err(|_| SocketError::Io)?;

    let mut header = [0u8; PDU_HEADER_LEN];
    tokio::time::timeout(RESPONSE_TIMEOUT, stream.read_exact(&mut header))
        .await
        .map_err(|_| SocketError::Timeout)?
        .map_err(|_| SocketError::Io)?;

    let [_, _, _, len_hi, len_lo] = header;
    let param_len = usize::from(u16::from_be_bytes([len_hi, len_lo]));

    let mut body = vec![0u8; param_len];
    tokio::time::timeout(RESPONSE_TIMEOUT, stream.read_exact(&mut body))
        .await
        .map_err(|_| SocketError::Timeout)?
        .map_err(|_| SocketError::Io)?;

    let mut pdu = Vec::with_capacity(PDU_HEADER_LEN + param_len);
    pdu.extend_from_slice(&header);
    pdu.extend_from_slice(&body);
    Ok(pdu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sends_request_and_reads_back_a_single_page_response() -> Result<(), String> {
        let (mut client, mut server) = tokio::io::duplex(4096);

        let response = hex_decode(include_str!("../../tests/fixtures/pbap_response.hex"));
        let request = hex_decode(include_str!("../../tests/fixtures/pbap_request.hex"));

        let response_for_server = response.clone();
        let request_len = request.len();
        let server_task = tokio::spawn(async move {
            let mut received_request = vec![0u8; request_len];
            server
                .read_exact(&mut received_request)
                .await
                .map_err(|e| format!("server read failed: {e}"))?;
            server
                .write_all(&response_for_server)
                .await
                .map_err(|e| format!("server write failed: {e}"))?;
            Ok::<Vec<u8>, String>(received_request)
        });

        let received_response = send_and_receive_pdu(&mut client, &request)
            .await
            .map_err(|e| format!("send_and_receive_pdu failed: {e}"))?;

        let received_request = server_task
            .await
            .map_err(|e| format!("server task panicked: {e}"))??;

        assert_eq!(received_request, request);
        assert_eq!(received_response, response);
        Ok(())
    }

    #[tokio::test]
    async fn reads_exactly_one_pdu_even_when_more_bytes_are_pending_in_the_stream(
    ) -> Result<(), String> {
        // Regression guard for the framing question: a naive single read()
        // that returns "whatever's available" would over-read into the next
        // page's bytes. This proves send_and_receive_pdu reads exactly the
        // declared parameter_length and leaves the rest for the next call.
        let (mut client, mut server) = tokio::io::duplex(8192);

        let page0 = hex_decode(include_str!("../../tests/fixtures/browse_page0_response.hex"));
        let page1 = hex_decode(include_str!("../../tests/fixtures/browse_page1_response.hex"));
        let request = hex_decode(include_str!("../../tests/fixtures/browse_page0_request.hex"));

        let mut both_pages = page0.clone();
        both_pages.extend_from_slice(&page1);
        let request_len = request.len();

        let server_task = tokio::spawn(async move {
            let mut received_request = vec![0u8; request_len];
            server
                .read_exact(&mut received_request)
                .await
                .map_err(|e| format!("server read failed: {e}"))?;
            // Write both pages back-to-back in a single write, simulating
            // bytes for a second PDU already sitting in the stream buffer.
            server
                .write_all(&both_pages)
                .await
                .map_err(|e| format!("server write failed: {e}"))?;
            Ok::<(), String>(())
        });

        let first = send_and_receive_pdu(&mut client, &request)
            .await
            .map_err(|e| format!("first send_and_receive_pdu failed: {e}"))?;
        assert_eq!(first, page0, "must read exactly page0, not page0+page1");

        // Nothing has been sent for page1's request in this test (the
        // duplex has no request-gating), so directly read the remaining
        // bytes and confirm they are exactly page1, proving the first call
        // did not consume any of them.
        let second = send_and_receive_pdu(&mut client, &[])
            .await
            .map_err(|e| format!("second send_and_receive_pdu failed: {e}"))?;
        assert_eq!(second, page1);

        server_task
            .await
            .map_err(|e| format!("server task panicked: {e}"))??;
        Ok(())
    }

    fn hex_decode(s: &str) -> Vec<u8> {
        (0..s.trim().len())
            .step_by(2)
            .map(|i| {
                let byte_str = s.trim().get(i..i + 2).unwrap_or("00");
                u8::from_str_radix(byte_str, 16).unwrap_or(0)
            })
            .collect()
    }
}

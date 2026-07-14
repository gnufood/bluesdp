//! The crate's public API surface: a single entry point
//! (`query_rfcomm_channel`) plus the types needed to call it and interpret
//! its result. Everything else in this crate (`codec`, `transport`) is an
//! implementation detail.

use crate::codec::EncodeError;
use crate::transport::{connect_sdp, find_rfcomm_channel, QueryError, SocketError};

/// See the Bluetooth SIG assigned numbers document for the full list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Uuid16(pub u16);

impl Uuid16 {
    pub const PBAP: Uuid16 = Uuid16(0x112f);
    pub const MAP: Uuid16 = Uuid16(0x1132);
}

/// Everything that can go wrong resolving an RFCOMM channel over SDP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SdpError {
    /// Expected `AA:BB:CC:DD:EE:FF`.
    InvalidAddress(String),
    Connect(String),
    Encode(EncodeError),
    Transport(SocketError),
    Decode(String),
}

impl std::fmt::Display for SdpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SdpError::InvalidAddress(msg) => write!(f, "invalid Bluetooth address: {msg}"),
            SdpError::Connect(msg) => write!(f, "failed to connect to SDP server: {msg}"),
            SdpError::Encode(e) => write!(f, "failed to encode SDP request: {e}"),
            SdpError::Transport(e) => write!(f, "SDP transport error: {e}"),
            SdpError::Decode(msg) => write!(f, "failed to decode SDP response: {msg}"),
        }
    }
}

impl std::error::Error for SdpError {}

impl From<QueryError> for SdpError {
    fn from(error: QueryError) -> Self {
        match error {
            QueryError::Encode(e) => SdpError::Encode(e),
            QueryError::Socket(e) => SdpError::Transport(e),
            QueryError::Decode => SdpError::Decode("malformed SDP response".to_string()),
        }
    }
}

/// Resolve the RFCOMM channel a Bluetooth device's SDP server reports for
/// `service_uuid`, connecting to `addr` (formatted `AA:BB:CC:DD:EE:FF`)
/// over L2CAP with no dependency on libbluetooth.
///
/// Returns `Ok(None)` if the device has no service record matching
/// `service_uuid` -- that is a normal, non-error outcome, not a failure.
///
/// # Errors
///
/// `InvalidAddress` if `addr` doesn't parse; `Connect` if the L2CAP retry
/// budget is exhausted; `Transport` on I/O failure or timeout mid-exchange;
/// `Decode` if the response is malformed.
pub async fn query_rfcomm_channel(
    addr: &str,
    service_uuid: Uuid16,
) -> Result<Option<u8>, SdpError> {
    let address: bluer::Address = addr
        .parse()
        .map_err(|e: bluer::InvalidAddress| SdpError::InvalidAddress(e.to_string()))?;

    let mut stream = connect_sdp(address)
        .await
        .map_err(|e| SdpError::Connect(e.to_string()))?;

    find_rfcomm_channel(&mut stream, service_uuid.0)
        .await
        .map_err(SdpError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid16_pbap_constant_matches_assigned_number() {
        assert_eq!(Uuid16::PBAP, Uuid16(0x112f));
    }

    #[test]
    fn uuid16_map_constant_matches_assigned_number() {
        assert_eq!(Uuid16::MAP, Uuid16(0x1132));
    }

    #[test]
    fn uuid16_values_are_distinguishable() {
        assert_ne!(Uuid16::PBAP, Uuid16::MAP);
    }

    #[test]
    fn sdp_error_invalid_address_message_is_descriptive() {
        let message = SdpError::InvalidAddress("bad".to_string()).to_string();
        assert!(message.contains("invalid Bluetooth address"));
    }

    #[test]
    fn sdp_error_connect_message_is_descriptive() {
        let message = SdpError::Connect("no route".to_string()).to_string();
        assert!(message.contains("failed to connect"));
    }

    #[test]
    fn sdp_error_decode_message_is_descriptive() {
        let message = SdpError::Decode("truncated".to_string()).to_string();
        assert!(message.contains("failed to decode"));
    }

    #[tokio::test]
    async fn query_rfcomm_channel_rejects_invalid_address() {
        let result = query_rfcomm_channel("not-a-bluetooth-address", Uuid16::PBAP).await;
        assert!(matches!(result, Err(SdpError::InvalidAddress(_))));
    }

    #[tokio::test]
    async fn query_rfcomm_channel_rejects_empty_address() {
        let result = query_rfcomm_channel("", Uuid16::MAP).await;
        assert!(matches!(result, Err(SdpError::InvalidAddress(_))));
    }

    /// Set `BLUESDP_TEST_DEVICE_ADDR` to your own paired device's address to run
    /// the `--ignored` live tests locally.
    fn test_device_addr() -> String {
        std::env::var("BLUESDP_TEST_DEVICE_ADDR").unwrap_or_else(|_| "AA:BB:CC:DD:EE:FF".to_string())
    }

    #[tokio::test]
    #[ignore = "requires a real, paired Bluetooth device; run with --ignored"]
    async fn query_rfcomm_channel_finds_pbap_on_real_device() -> Result<(), String> {
        let channel = query_rfcomm_channel(&test_device_addr(), Uuid16::PBAP)
            .await
            .map_err(|e| format!("query_rfcomm_channel failed: {e}"))?;
        assert_eq!(channel, Some(13));
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires a real, paired Bluetooth device; run with --ignored"]
    async fn query_rfcomm_channel_finds_map_on_real_device() -> Result<(), String> {
        let channel = query_rfcomm_channel(&test_device_addr(), Uuid16::MAP)
            .await
            .map_err(|e| format!("query_rfcomm_channel failed: {e}"))?;
        assert_eq!(channel, Some(2));
        Ok(())
    }
}

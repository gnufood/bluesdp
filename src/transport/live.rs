//! Real `bluer::l2cap` socket wiring: open a connection to a remote
//! device's SDP server (PSM 1) through the bounded connect-with-retry
//! policy. This is the one module in the crate that touches actual
//! Bluetooth hardware -- everything it calls into (`connect_with_retry`,
//! `find_rfcomm_channel`) is already independently tested against
//! fixtures/in-memory streams; this module only adds the real transport.

use bluer::Address;
use bluer::l2cap::{SocketAddr, Stream};

use super::connect::{connect_with_retry, RetriesExhausted};

/// Bluetooth Core Spec Vol 3, Part B.
const SDP_PSM: u16 = 1;

pub async fn connect_sdp(addr: Address) -> Result<Stream, RetriesExhausted> {
    let socket_addr = SocketAddr::new(addr, bluer::AddressType::BrEdr, SDP_PSM);
    connect_with_retry(|| Stream::connect(socket_addr)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::session::find_rfcomm_channel;

    const UUID_PBAP: u16 = 0x112f;
    const UUID_MAP: u16 = 0x1132;

    /// Set `BLUESDP_TEST_DEVICE_ADDR` to your own paired device's address to run
    /// the `--ignored` live tests locally.
    fn test_device_addr() -> String {
        std::env::var("BLUESDP_TEST_DEVICE_ADDR").unwrap_or_else(|_| "AA:BB:CC:DD:EE:FF".to_string())
    }

    #[tokio::test]
    #[ignore = "requires a real, paired Bluetooth device; run with --ignored"]
    async fn finds_pbap_channel_on_real_device() -> Result<(), String> {
        let addr: Address = test_device_addr()
            .parse()
            .map_err(|e| format!("invalid test address: {e}"))?;
        let mut stream = connect_sdp(addr)
            .await
            .map_err(|e| format!("connect_sdp failed: {e}"))?;

        let channel = find_rfcomm_channel(&mut stream, UUID_PBAP)
            .await
            .map_err(|e| format!("find_rfcomm_channel failed: {e}"))?;

        assert_eq!(channel, Some(13));
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires a real, paired Bluetooth device; run with --ignored"]
    async fn finds_map_channel_on_real_device() -> Result<(), String> {
        let addr: Address = test_device_addr()
            .parse()
            .map_err(|e| format!("invalid test address: {e}"))?;
        let mut stream = connect_sdp(addr)
            .await
            .map_err(|e| format!("connect_sdp failed: {e}"))?;

        let channel = find_rfcomm_channel(&mut stream, UUID_MAP)
            .await
            .map_err(|e| format!("find_rfcomm_channel failed: {e}"))?;

        assert_eq!(channel, Some(2));
        Ok(())
    }
}

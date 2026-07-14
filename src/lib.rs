//! Pure-Rust SDP client: resolve an RFCOMM channel for a given service UUID
//! on a remote Bluetooth device, without linking against libbluetooth.

mod codec;
mod public;
mod transport;

pub use codec::EncodeError;
pub use public::{query_rfcomm_channel, SdpError, Uuid16};
pub use transport::SocketError;

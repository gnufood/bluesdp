//! Bluetooth transport: L2CAP socket connection and byte-level SDP
//! request/response exchange.

mod connect;
mod live;
mod session;
mod socket;

pub use live::connect_sdp;
pub use session::{find_rfcomm_channel, QueryError};
pub use socket::SocketError;

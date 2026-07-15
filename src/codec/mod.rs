//! SDP data element codec.

mod descriptor;
mod element;
mod encode;
mod extract;
mod pdu_header;
mod response;

pub use encode::{encode, EncodeError, ServiceSearchAttributeRequest};
pub use extract::rfcomm_channel;
pub use response::{decode_attribute_lists, raw_response};

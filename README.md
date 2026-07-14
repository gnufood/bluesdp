# bluesdp

A pure-Rust Bluetooth SDP client, scoped to one job: resolve the RFCOMM
channel a remote device's SDP server reports for a 16-bit service UUID.

## What it replaces

Resolving an RFCOMM channel normally means linking against `libbluetooth`
(via `bluez`'s `sdp_lib`) or shelling out to a C helper built on it. This
crate implements the relevant slice of the SDP protocol — PDU encoding,
data-element decoding, and continuation-state pagination — directly over
`bluer`'s L2CAP socket, with no FFI and no system Bluetooth library
dependency beyond BlueZ itself (via `bluer`/D-Bus).

## Scope

- 16-bit service UUIDs only (32-bit and 128-bit UUIDs are not implemented).
- No `ErrorResponse` PDU (0x01) handling — malformed/error responses from
  the remote device are surfaced as a decode error, not a distinct variant.
- No CLI; this is a library only.
- Retry and timeout policy are fixed constants, not configurable.

## API

```rust
pub async fn query_rfcomm_channel(
    addr: &str,
    service_uuid: Uuid16,
) -> Result<Option<u8>, SdpError>;
```

`addr` is a Bluetooth device address formatted `AA:BB:CC:DD:EE:FF`.
Returns `Ok(None)` if the device has no service record matching
`service_uuid` — a normal outcome, not an error.

```rust
use bluesdp::{query_rfcomm_channel, Uuid16};

let channel = query_rfcomm_channel("AA:BB:CC:DD:EE:FF", Uuid16::PBAP).await?;
```

### `Uuid16`

A 16-bit Bluetooth service class UUID.

```rust
pub struct Uuid16(pub u16);

impl Uuid16 {
    pub const PBAP: Uuid16; // 0x112f
    pub const MAP: Uuid16;  // 0x1132
}
```

Any other 16-bit service UUID can be passed as `Uuid16(0x____)`.

### `SdpError`

```rust
pub enum SdpError {
    InvalidAddress(String),
    Connect(String),
    Encode(EncodeError),
    Transport(SocketError),
    Decode(String),
}
```

`InvalidAddress` if `addr` doesn't parse; `Connect` if the L2CAP retry
budget is exhausted; `Encode`/`Transport`/`Decode` cover the request
encoding, socket I/O, and response parsing stages respectively.

## Requirements

Linux with BlueZ, and a device already paired at the OS level — this
crate performs SDP service discovery only, not pairing.

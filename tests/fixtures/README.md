# SDP fixtures

Each `.hex` file holds one raw SDP PDU: PDU ID + transaction ID + param length + body,
i.e. the L2CAP payload on PSM 1 with H4/HCI/L2CAP framing stripped. One hex string per
line, no whitespace.

- `pbap_request.hex` / `pbap_response.hex` — `ServiceSearchAttributeRequest`/`Response`
  for UUID 0x112F (PBAP). RFCOMM channel 13. Single packet (continuation state = 0x00).
- `map_request.hex` / `map_response.hex` — same, for UUID 0x1132 (MAP). RFCOMM channel 2.
  Also single-packet.
- `browse_page{0..7}_request.hex` / `browse_page{0..7}_response.hex` — an 8-page
  continuation sequence searching `PublicBrowseGroup` (0x1002) across the full attribute
  range (0x0000-0xffff). Returns 12 service records, exceeding the negotiated 256-byte
  L2CAP MTU and forcing continuation-state paging.

  Each page's request echoes the continuation-state field (length byte + blob) from the
  previous page's response exactly; page 7's response ends with continuation length 0,
  signaling completion.

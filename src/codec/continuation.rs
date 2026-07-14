//! SDP continuation-state driving loop: reassemble a paginated
//! `ServiceSearchAttributeResponse` by repeatedly following the
//! continuation-state field until the server reports completion.
//!
//! This module is transport- and encoder-agnostic: it only knows how to
//! read a response's continuation state and, via an injected callback,
//! ask for the next page keyed on that state. Building the actual next
//! request's wire bytes from that continuation blob is the request
//! encoder's job, not this loop's.

use super::response::RawServiceSearchAttributeResponse;

/// The only two fields a paginated response page contributes to
/// reassembly. Deliberately narrower than `RawServiceSearchAttributeResponse`
/// so this loop cannot depend on PDU-header details it never uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub attribute_bytes: Vec<u8>,
    pub continuation_state: Vec<u8>,
}

impl From<RawServiceSearchAttributeResponse> for Page {
    fn from(raw: RawServiceSearchAttributeResponse) -> Self {
        Page {
            attribute_bytes: raw.attribute_bytes,
            continuation_state: raw.continuation_state,
        }
    }
}

/// Drive continuation-state pagination to completion, starting from an
/// already-received first page. Calls `fetch_next` with the continuation
/// bytes from each page until one comes back with an empty continuation
/// state, then returns the concatenated `attribute_bytes` across every
/// page in order.
///
/// `fetch_next` is `AsyncFnMut` (stabilized Rust 1.85, RFC 3668) rather
/// than a plain `FnMut(&[u8]) -> Fut`: a real caller's callback needs its
/// returned future to borrow mutable state from its environment (e.g. a
/// `&mut` socket) across an `.await` point, which only a lending
/// `AsyncFnMut` permits -- a plain `FnMut` cannot let a captured reference
/// escape into its return value. Fixture-driven callers can still pass a
/// plain `async |continuation| { ... }` closure with no captures.
pub async fn reassemble(
    first: Page,
    mut fetch_next: impl AsyncFnMut(&[u8]) -> Page,
) -> Vec<u8> {
    let mut attribute_bytes = first.attribute_bytes;
    let mut continuation_state = first.continuation_state;

    while !continuation_state.is_empty() {
        let next = fetch_next(&continuation_state).await;
        attribute_bytes.extend_from_slice(&next.attribute_bytes);
        continuation_state = next.continuation_state;
    }

    attribute_bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::response::raw_response;

    fn hex_decode(s: &str) -> Vec<u8> {
        (0..s.trim().len())
            .step_by(2)
            .map(|i| {
                let byte_str = s.trim().get(i..i + 2).unwrap_or("00");
                u8::from_str_radix(byte_str, 16).unwrap_or(0)
            })
            .collect()
    }

    const RESPONSE_FIXTURES: [&str; 8] = [
        include_str!("../../tests/fixtures/browse_page0_response.hex"),
        include_str!("../../tests/fixtures/browse_page1_response.hex"),
        include_str!("../../tests/fixtures/browse_page2_response.hex"),
        include_str!("../../tests/fixtures/browse_page3_response.hex"),
        include_str!("../../tests/fixtures/browse_page4_response.hex"),
        include_str!("../../tests/fixtures/browse_page5_response.hex"),
        include_str!("../../tests/fixtures/browse_page6_response.hex"),
        include_str!("../../tests/fixtures/browse_page7_response.hex"),
    ];

    // No page0 entry: reassemble() starts from an already-fetched first page
    // and never re-requests it.
    const EXPECTED_CONTINUATION_REQUESTS: [&[u8]; 7] = [
        &[0x00, 0xf3],
        &[0x01, 0xe9],
        &[0x02, 0xdf],
        &[0x03, 0xd5],
        &[0x04, 0xcb],
        &[0x05, 0xbf],
        &[0x06, 0xb5],
    ];

    fn parsed_page(index: usize) -> Result<Page, String> {
        let bytes = hex_decode(
            RESPONSE_FIXTURES
                .get(index)
                .ok_or_else(|| format!("no fixture for page {index}"))?,
        );
        let (_, raw) = raw_response(&bytes).map_err(|e| format!("raw_response failed: {e}"))?;
        Ok(raw.into())
    }

    fn empty_page() -> Page {
        Page {
            attribute_bytes: Vec::new(),
            continuation_state: Vec::new(),
        }
    }

    #[tokio::test]
    async fn drives_all_eight_pages_and_requests_correct_continuation_each_time(
    ) -> Result<(), String> {
        let first = parsed_page(0)?;
        let mut next_index = 1usize;
        let mut seen_continuations = Vec::new();

        let result = reassemble(first, async |continuation| {
            seen_continuations.push(continuation.to_vec());
            let page = parsed_page(next_index).unwrap_or_else(|_| empty_page());
            next_index += 1;
            page
        })
        .await;

        assert_eq!(seen_continuations, EXPECTED_CONTINUATION_REQUESTS);

        let (leftover, attribute_lists) =
            super::super::response::decode_attribute_lists(&result)
                .map_err(|e| format!("decode_attribute_lists on reassembled bytes failed: {e}"))?;
        assert_eq!(leftover, &[] as &[u8]);
        assert_eq!(attribute_lists.len(), 12, "expected all 12 service records");

        Ok(())
    }

    #[tokio::test]
    async fn single_page_response_with_no_continuation_returns_immediately() -> Result<(), String>
    {
        let bytes = hex_decode(include_str!("../../tests/fixtures/pbap_response.hex"));
        let (_, raw) = raw_response(&bytes).map_err(|e| format!("raw_response failed: {e}"))?;
        let page: Page = raw.into();
        let expected = page.attribute_bytes.clone();

        let mut fetch_calls = 0usize;
        let result = reassemble(page, async |_| {
            fetch_calls += 1;
            empty_page()
        })
        .await;

        assert_eq!(fetch_calls, 0, "must not fetch when there is no continuation");
        assert_eq!(result, expected);
        Ok(())
    }

    #[tokio::test]
    async fn callback_future_can_borrow_external_mutable_state() -> Result<(), String> {
        // Regression guard for the AsyncFnMut bound: the closure below borrows
        // external_counter mutably across an .await, which a plain FnMut couldn't allow.
        let mut external_counter = 0u32;
        let mut next_index = 1usize;

        let first = parsed_page(0)?;
        let result = reassemble(first, async |continuation| {
            external_counter += 1;
            let page = parsed_page(next_index).unwrap_or_else(|_| empty_page());
            next_index += 1;
            tokio::task::yield_now().await;
            let _ = continuation;
            page
        })
        .await;

        assert_eq!(external_counter, 7, "should have fetched pages 1 through 7");

        let (leftover, attribute_lists) = super::super::response::decode_attribute_lists(&result)
            .map_err(|e| format!("decode_attribute_lists failed: {e}"))?;
        assert_eq!(leftover, &[] as &[u8]);
        assert_eq!(attribute_lists.len(), 12);

        Ok(())
    }
}

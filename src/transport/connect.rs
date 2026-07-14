//! Bounded connect-with-retry policy, generic over the actual connection
//! attempt so the retry/backoff logic is testable without a real socket.
//! The one real call site (wiring this to `bluer::l2cap::Stream::connect`)
//! lives in `transport::session`, exercised only by the live hardware test.

use std::time::Duration;

/// Number of connection attempts before giving up. Chosen as a bounded,
/// safer alternative to `BlueZ`'s own `SDP_RETRY_IF_BUSY`, which retries
/// `connect()` in an uncapped, zero-delay loop while `errno == EBUSY`
/// (lib/sdp.c) -- a real risk of spinning forever against a persistently
/// unavailable remote.
pub const MAX_CONNECT_ATTEMPTS: u32 = 5;

/// Delay between connection attempts.
pub const RETRY_DELAY: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetriesExhausted;

impl std::fmt::Display for RetriesExhausted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "connection failed after {MAX_CONNECT_ATTEMPTS} attempts")
    }
}

impl std::error::Error for RetriesExhausted {}

/// Attempt `connect` up to `MAX_CONNECT_ATTEMPTS` times, waiting
/// `RETRY_DELAY` between attempts, returning the first success or
/// `RetriesExhausted` if every attempt failed. `connect`'s error value
/// itself is discarded on failure (only whether it succeeded matters to
/// the retry loop) -- the real caller logs/propagates individual attempt
/// errors as needed via its own error type.
pub async fn connect_with_retry<T, E, F, Fut>(mut connect: F) -> Result<T, RetriesExhausted>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    for attempt in 0..MAX_CONNECT_ATTEMPTS {
        if let Ok(value) = connect().await {
            return Ok(value);
        }
        if attempt + 1 < MAX_CONNECT_ATTEMPTS {
            tokio::time::sleep(RETRY_DELAY).await;
        }
    }
    Err(RetriesExhausted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn succeeds_immediately_when_first_attempt_succeeds() -> Result<(), String> {
        let attempts = AtomicU32::new(0);
        let result = connect_with_retry(|| {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { Ok::<u32, ()>(42) }
        })
        .await;

        assert_eq!(result, Ok(42));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn succeeds_after_transient_failures_within_the_attempt_budget() -> Result<(), String> {
        let attempts = AtomicU32::new(0);
        let result = connect_with_retry(|| {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            async move {
                if n < 2 {
                    Err(())
                } else {
                    Ok(7u32)
                }
            }
        })
        .await;

        assert_eq!(result, Ok(7));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        Ok(())
    }

    #[tokio::test]
    async fn gives_up_after_max_attempts() -> Result<(), String> {
        let attempts = AtomicU32::new(0);
        let result: Result<u32, RetriesExhausted> = connect_with_retry(|| {
            attempts.fetch_add(1, Ordering::SeqCst);
            async { Err::<u32, ()>(()) }
        })
        .await;

        assert_eq!(result, Err(RetriesExhausted));
        assert_eq!(attempts.load(Ordering::SeqCst), MAX_CONNECT_ATTEMPTS);
        Ok(())
    }
}

//! Bandwidth limiting: a shared "next free slot" clock, so every worker of a download (or link) draws from one budget.

use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct Limiter {
    bytes_per_sec: u64,
    next: Mutex<Instant>,
}

impl Limiter {
    pub fn new(bytes_per_sec: u64) -> Self {
        Limiter { bytes_per_sec: bytes_per_sec.max(1), next: Mutex::new(Instant::now()) }
    }

    /// Reserve `n` bytes and say how long the caller must wait before using them.
    pub fn reserve(&self, n: u64) -> Duration {
        let now = Instant::now();
        let mut next = self.next.lock().unwrap();
        // Idle time does not bank up, so a stalled link cannot burst past the limit afterwards.
        let start = (*next).max(now);
        *next = start + Duration::from_secs_f64(n as f64 / self.bytes_per_sec as f64);
        start.saturating_duration_since(now)
    }

    pub async fn acquire(&self, n: u64) {
        let wait = self.reserve(n);
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_reservations_by_the_rate() {
        let l = Limiter::new(1000);
        assert!(l.reserve(500) < Duration::from_millis(50));
        let w = l.reserve(500);
        assert!(w > Duration::from_millis(400) && w <= Duration::from_millis(500), "{w:?}");
    }
}

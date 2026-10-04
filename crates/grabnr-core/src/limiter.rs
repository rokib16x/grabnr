//! Bandwidth limiting: a shared "next free slot" clock, so every worker of a download (or of many downloads) draws from one budget.
//! The rate can change while transfers run; 0 means unlimited.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct Limiter {
    bytes_per_sec: AtomicU64,
    next: Mutex<Instant>,
}

impl Limiter {
    pub fn new(bytes_per_sec: u64) -> Self {
        Limiter { bytes_per_sec: AtomicU64::new(bytes_per_sec), next: Mutex::new(Instant::now()) }
    }

    pub fn rate(&self) -> u64 {
        self.bytes_per_sec.load(Ordering::Relaxed)
    }

    /// Change the limit for everyone using this limiter; 0 removes it.
    pub fn set_rate(&self, bytes_per_sec: u64) {
        self.bytes_per_sec.store(bytes_per_sec, Ordering::Relaxed);
        // Forget time already promised at the old rate so the change takes effect at once.
        *self.next.lock().unwrap() = Instant::now();
    }

    /// Reserve `n` bytes and say how long the caller must wait before using them.
    pub fn reserve(&self, n: u64) -> Duration {
        let rate = self.rate();
        if rate == 0 {
            return Duration::ZERO;
        }
        let now = Instant::now();
        let mut next = self.next.lock().unwrap();
        // Idle time does not bank up, so a stalled link cannot burst past the limit afterwards.
        let start = (*next).max(now);
        *next = start + Duration::from_secs_f64(n as f64 / rate as f64);
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

    #[test]
    fn zero_means_unlimited_and_changes_apply_at_once() {
        let l = Limiter::new(100);
        l.reserve(1000); // promises 10 s
        l.set_rate(0);
        assert_eq!(l.reserve(1_000_000), Duration::ZERO);
        l.set_rate(1000);
        assert!(l.reserve(10) < Duration::from_millis(50), "old debt must be forgotten");
        assert_eq!(l.rate(), 1000);
    }
}

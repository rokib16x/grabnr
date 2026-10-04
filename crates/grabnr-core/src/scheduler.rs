//! Shared chunk queue. Every worker on every link leases from the same queue, so
//! faster links naturally take more chunks. When the queue runs dry, idle workers
//! race the slowest in-flight chunk ("tail racing"); the first copy to finish wins.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

pub const MIN_CHUNK: u64 = 1 << 20;
pub const MAX_CHUNK: u64 = 8 << 20;

/// Aim for roughly 16 chunks per link so work can rebalance, clamped to 1-8 MB.
pub fn chunk_size_for(total: u64, links: usize) -> u64 {
    (total / (links.max(1) as u64 * 16)).clamp(MIN_CHUNK, MAX_CHUNK)
}

/// Inclusive byte ranges covering `0..total`.
pub fn plan(total: u64, chunk: u64) -> Vec<(u64, u64)> {
    let mut v = Vec::new();
    let mut start = 0;
    while start < total {
        let end = (start + chunk).min(total) - 1;
        v.push((start, end));
        start = end + 1;
    }
    v
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lease {
    Chunk {
        idx: usize,
        start: u64,
        end: u64,
        raced: bool,
    },
    /// Nothing to take right now, but the download is not finished.
    Wait,
    Finished,
}

#[derive(Clone, Copy)]
enum State {
    Pending,
    Active { workers: u8, since: Instant },
    Done,
}

struct Inner {
    ranges: Vec<(u64, u64)>,
    state: Vec<State>,
    pending: VecDeque<usize>,
    attempts: Vec<u32>,
    done: usize,
    /// Bytes of each chunk already safely on disk, and the CRC-32 of them. Equals the chunk length once done.
    have: Vec<(u64, u32)>,
}

pub struct Scheduler(Mutex<Inner>);

impl Scheduler {
    pub fn new(ranges: Vec<(u64, u64)>, already_done: &[usize]) -> Self {
        let mut state = vec![State::Pending; ranges.len()];
        let mut done = 0;
        for &i in already_done {
            if i < state.len() && !matches!(state[i], State::Done) {
                state[i] = State::Done;
                done += 1;
            }
        }
        let pending = (0..ranges.len()).filter(|&i| matches!(state[i], State::Pending)).collect();
        let attempts = vec![0; ranges.len()];
        let have = ranges.iter().zip(&state).map(|(&(s, e), st)| (if matches!(st, State::Done) { e - s + 1 } else { 0 }, 0)).collect();
        Scheduler(Mutex::new(Inner { ranges, state, pending, attempts, done, have }))
    }

    /// Start chunks that were partly downloaded in an earlier run from where they stopped: `(index, bytes, crc)`.
    pub fn with_have(self, saved: &[(usize, u64, u32)]) -> Self {
        {
            let mut g = self.0.lock().unwrap();
            for &(i, len, crc) in saved {
                if let (Some(&(s, e)), Some(State::Pending)) = (g.ranges.get(i), g.state.get(i)) {
                    if len > 0 && len < e - s + 1 {
                        g.have[i] = (len, crc);
                    }
                }
            }
        }
        self
    }

    /// Bytes safely on disk across all chunks: finished ones in full, partial ones as far as they got.
    pub fn settled(&self) -> u64 {
        self.0.lock().unwrap().have.iter().map(|h| h.0).sum()
    }

    /// How much of the chunk is already on disk, and the checksum of those bytes.
    pub fn have(&self, idx: usize) -> (u64, u32) {
        self.0.lock().unwrap().have[idx]
    }

    /// Record that the first `len` bytes of a chunk are on disk (checksum `crc`) so a later fetch can continue there.
    /// Only accepted while a single worker is on the chunk, so racing copies cannot disagree about it.
    /// Returns how many bytes were newly settled.
    pub fn set_have(&self, idx: usize, len: u64, crc: u32) -> Option<u64> {
        let mut g = self.0.lock().unwrap();
        let (s, e) = g.ranges[idx];
        let sole = matches!(g.state[idx], State::Active { workers: 1, .. } | State::Pending);
        if !sole || len <= g.have[idx].0 || len > e - s {
            return None;
        }
        let delta = len - g.have[idx].0;
        g.have[idx] = (len, crc);
        Some(delta)
    }

    pub fn lease(&self) -> Lease {
        let mut g = self.0.lock().unwrap();
        while let Some(idx) = g.pending.pop_front() {
            if matches!(g.state[idx], State::Pending) {
                g.state[idx] = State::Active { workers: 1, since: Instant::now() };
                let (start, end) = g.ranges[idx];
                return Lease::Chunk { idx, start, end, raced: false };
            }
        }
        if g.done == g.ranges.len() {
            return Lease::Finished;
        }
        let slowest = g
            .state
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match s {
                State::Active { workers: 1, since } => Some((i, *since)),
                _ => None,
            })
            .min_by_key(|&(_, since)| since)
            .map(|(i, _)| i);
        match slowest {
            Some(idx) => {
                if let State::Active { workers, .. } = &mut g.state[idx] {
                    *workers += 1;
                }
                let (start, end) = g.ranges[idx];
                Lease::Chunk { idx, start, end, raced: true }
            }
            None => Lease::Wait,
        }
    }

    /// `Some(bytes newly settled)` if this call is the one that finished the chunk, `None` if it was already done.
    pub fn complete(&self, idx: usize) -> Option<u64> {
        let mut g = self.0.lock().unwrap();
        if matches!(g.state[idx], State::Done) {
            return None;
        }
        let (s, e) = g.ranges[idx];
        let delta = (e - s + 1) - g.have[idx].0;
        g.have[idx] = (e - s + 1, 0);
        g.state[idx] = State::Done;
        g.done += 1;
        Some(delta)
    }

    /// A worker gives a chunk back. Returns how many times it has failed so far.
    pub fn release(&self, idx: usize, failed: bool) -> u32 {
        let mut g = self.0.lock().unwrap();
        if failed {
            g.attempts[idx] += 1;
        }
        match g.state[idx] {
            State::Active { workers, since } if workers > 1 => g.state[idx] = State::Active { workers: workers - 1, since },
            State::Active { .. } => {
                g.state[idx] = State::Pending;
                g.pending.push_front(idx);
            }
            _ => {}
        }
        g.attempts[idx]
    }

    /// Chunks nobody has started yet.
    pub fn pending_len(&self) -> usize {
        self.0.lock().unwrap().pending.len()
    }

    pub fn is_done(&self, idx: usize) -> bool {
        matches!(self.0.lock().unwrap().state[idx], State::Done)
    }

    pub fn finished(&self) -> bool {
        let g = self.0.lock().unwrap();
        g.done == g.ranges.len()
    }

    pub fn len(&self) -> usize {
        self.0.lock().unwrap().ranges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_covers_everything() {
        let p = plan(10, 4);
        assert_eq!(p, vec![(0, 3), (4, 7), (8, 9)]);
        assert!(plan(0, 4).is_empty());
    }

    #[test]
    fn chunk_size_is_clamped() {
        assert_eq!(chunk_size_for(10 << 20, 2), MIN_CHUNK);
        assert_eq!(chunk_size_for(100 << 30, 2), MAX_CHUNK);
    }

    #[test]
    fn leases_in_order_then_finishes() {
        let s = Scheduler::new(plan(30, 10), &[]);
        let Lease::Chunk { idx: a, .. } = s.lease() else { panic!() };
        let Lease::Chunk { idx: b, .. } = s.lease() else { panic!() };
        let Lease::Chunk { idx: c, .. } = s.lease() else { panic!() };
        assert_eq!((a, b, c), (0, 1, 2));
        for i in [a, b, c] {
            assert!(s.complete(i).is_some());
        }
        assert_eq!(s.lease(), Lease::Finished);
    }

    #[test]
    fn races_the_oldest_in_flight_chunk_once_queue_is_empty() {
        let s = Scheduler::new(plan(20, 10), &[]);
        let Lease::Chunk { idx: slow, .. } = s.lease() else { panic!() };
        std::thread::sleep(std::time::Duration::from_millis(2));
        let Lease::Chunk { idx: fast, .. } = s.lease() else { panic!() };
        assert!(s.complete(fast).is_some());
        // queue empty: the idle worker races the slow chunk
        assert_eq!(s.lease(), Lease::Chunk { idx: slow, start: 0, end: 9, raced: true });
        // already two workers on it: nothing more to race
        assert_eq!(s.lease(), Lease::Wait);
        // first finisher wins, the second is told it lost
        assert!(s.complete(slow).is_some());
        assert!(s.complete(slow).is_none());
        assert_eq!(s.lease(), Lease::Finished);
    }

    #[test]
    fn released_chunk_returns_to_the_front() {
        let s = Scheduler::new(plan(30, 10), &[]);
        let Lease::Chunk { idx, .. } = s.lease() else { panic!() };
        assert_eq!(s.release(idx, true), 1);
        assert!(matches!(s.lease(), Lease::Chunk { idx: 0, raced: false, .. }));
    }

    #[test]
    fn resume_skips_done_chunks() {
        let s = Scheduler::new(plan(30, 10), &[0, 2]);
        assert!(matches!(s.lease(), Lease::Chunk { idx: 1, .. }));
        assert!(s.complete(1).is_some());
        assert!(s.finished());
    }

    #[test]
    fn partial_progress_is_remembered_only_for_a_sole_worker() {
        let s = Scheduler::new(plan(10, 5), &[]); // two chunks of 5 bytes
        assert_eq!(s.settled(), 0);
        let Lease::Chunk { idx, .. } = s.lease() else { panic!() };
        assert_eq!(s.set_have(idx, 3, 0xAB), Some(3));
        assert_eq!(s.have(idx), (3, 0xAB));
        assert_eq!(s.set_have(idx, 2, 1), None, "never goes backwards");
        assert_eq!(s.set_have(idx, 5, 1), None, "a whole chunk is complete, not partial");
        assert_eq!(s.settled(), 3);
        // a second worker joins the same chunk (tail racing): no more partial updates
        s.release(idx, false);
        let (Lease::Chunk { idx: a, .. }, Lease::Chunk { .. }) = (s.lease(), s.lease()) else { panic!() };
        assert_eq!(s.set_have(a, 4, 2), Some(1));
        let Lease::Chunk { idx: raced, raced: true, .. } = s.lease() else { panic!("expected a raced lease") };
        assert_eq!(s.set_have(raced, 5, 3), None);
        assert_eq!(s.complete(idx), Some(1), "finishing counts only what was not yet settled");
    }

    #[test]
    fn saved_partials_seed_the_scheduler() {
        let s = Scheduler::new(plan(10, 5), &[0]).with_have(&[(1, 2, 77), (0, 3, 1), (9, 1, 1), (1, 99, 5)]);
        assert_eq!(s.have(1), (2, 77), "an oversized entry is ignored");
        assert_eq!(s.have(0), (5, 0), "a finished chunk is not touched");
        assert_eq!(s.settled(), 7);
    }
}

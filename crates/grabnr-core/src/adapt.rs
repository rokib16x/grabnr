//! Adaptive per-link connection count and tail protection.
//!
//! A link starts with a few connections and adds two at a time while the extra
//! connections keep paying off. When a step stops helping, the count settles there.
//! A server that throttles lowers the ceiling for good.

pub const START_CONNS: usize = 4;
const GROW_BY: usize = 2;
/// A step must raise the link's speed by this factor to count as paying off.
const MIN_GAIN: f64 = 1.10;

#[derive(Debug, Clone)]
pub struct Ramp {
    pub allowed: usize,
    pub ceiling: usize,
    last_speed: f64,
    settled: bool,
}

impl Ramp {
    pub fn new(max: usize, adaptive: bool) -> Self {
        let max = max.max(1);
        let start = if adaptive { START_CONNS.min(max) } else { max };
        Ramp { allowed: start, ceiling: max, last_speed: 0.0, settled: !adaptive || start == max }
    }

    /// Lower the ceiling (server throttling); the ramp never grows past it again.
    pub fn cap(&mut self, to: usize) {
        self.ceiling = to.max(1).min(self.ceiling);
        self.allowed = self.allowed.min(self.ceiling);
        self.settled = true;
    }

    /// Feed the link's current speed; returns the connection count to use next.
    pub fn step(&mut self, speed: f64) -> usize {
        self.allowed = self.allowed.min(self.ceiling);
        if self.settled || speed <= 0.0 {
            return self.allowed;
        }
        if self.last_speed == 0.0 || speed >= self.last_speed * MIN_GAIN {
            self.last_speed = speed;
            if self.allowed < self.ceiling {
                self.allowed = (self.allowed + GROW_BY).min(self.ceiling);
            } else {
                self.settled = true;
            }
        } else {
            // The last step did not help: go back one notch and stop probing.
            self.allowed = self.allowed.saturating_sub(1).max(START_CONNS.min(self.ceiling));
            self.settled = true;
        }
        self.allowed
    }
}

/// A link far slower than the best one should not take the last chunks: the fast links finish them sooner.
pub fn is_slow_tail(mine: f64, best: f64, pending: usize, best_conns: usize) -> bool {
    best > 0.0 && mine < 0.25 * best && pending <= best_conns
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_while_it_pays_off_then_settles() {
        let mut r = Ramp::new(12, true);
        assert_eq!(r.allowed, 4);
        assert_eq!(r.step(10.0), 6); // first reading
        assert_eq!(r.step(15.0), 8); // +50%
        assert_eq!(r.step(15.5), 7); // +3%: not worth it, back one and settle
        assert_eq!(r.step(40.0), 7); // settled: stays put
    }

    #[test]
    fn stops_at_the_ceiling_and_respects_throttling() {
        let mut r = Ramp::new(6, true);
        assert_eq!(r.step(10.0), 6);
        assert_eq!(r.step(20.0), 6);
        let mut r = Ramp::new(16, true);
        r.step(10.0);
        r.cap(3);
        assert_eq!(r.allowed, 3);
        assert_eq!(r.step(100.0), 3);
    }

    #[test]
    fn fixed_mode_uses_the_configured_count() {
        let mut r = Ramp::new(8, false);
        assert_eq!(r.allowed, 8);
        assert_eq!(r.step(5.0), 8);
        assert_eq!(Ramp::new(2, true).allowed, 2);
    }

    #[test]
    fn slow_link_sits_out_the_tail() {
        assert!(is_slow_tail(1.0, 10.0, 3, 8));
        assert!(!is_slow_tail(1.0, 10.0, 30, 8), "plenty of chunks left, let it help");
        assert!(!is_slow_tail(5.0, 10.0, 3, 8), "half speed is fine");
        assert!(!is_slow_tail(0.0, 0.0, 0, 8), "nothing measured yet");
    }
}

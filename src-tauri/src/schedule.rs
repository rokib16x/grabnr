//! Time-of-day rules: pause downloads, cap the speed, or lift the cap, on chosen days and hours.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Mode {
    /// Hold every download until the window ends; running ones go back to the queue.
    Pause,
    /// Cap the total speed (KB/s).
    Limit { kbps: u64 },
    /// No cap, even if a general limit is set.
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// Monday = index 0 … Sunday = 6. For a window that crosses midnight, the day it starts on.
    pub days: [bool; 7],
    /// Minutes after midnight, 0..1440.
    pub start: u16,
    pub end: u16,
    pub mode: Mode,
}

impl Rule {
    /// Does this rule cover `minute` of `weekday` (Monday = 0)?
    pub fn covers(&self, weekday: usize, minute: u16) -> bool {
        if !self.enabled {
            return false;
        }
        let today = self.days[weekday % 7];
        let yesterday = self.days[(weekday + 6) % 7];
        match self.start.cmp(&self.end) {
            std::cmp::Ordering::Equal => today, // same start and end: all day
            std::cmp::Ordering::Less => today && minute >= self.start && minute < self.end,
            // Crosses midnight: the evening part belongs to today, the morning part to the day before.
            std::cmp::Ordering::Greater => (today && minute >= self.start) || (yesterday && minute < self.end),
        }
    }
}

/// What the schedule asks for right now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Default)]
pub struct Effect {
    pub hold: bool,
    /// Total speed cap in KB/s, 0 for none.
    pub limit_kbps: u64,
    /// Name of the rule that applies, if any.
    pub rule: Option<String>,
}

/// The first enabled rule covering this moment decides; otherwise only the general limit applies.
pub fn effect(rules: &[Rule], general_limit_kbps: u64, weekday: usize, minute: u16) -> Effect {
    let Some(r) = rules.iter().find(|r| r.covers(weekday, minute)) else {
        return Effect { hold: false, limit_kbps: general_limit_kbps, rule: None };
    };
    let name = Some(if r.name.is_empty() { "Schedule".to_string() } else { r.name.clone() });
    match r.mode {
        Mode::Pause => Effect { hold: true, limit_kbps: general_limit_kbps, rule: name },
        Mode::Full => Effect { hold: false, limit_kbps: 0, rule: name },
        Mode::Limit { kbps } => {
            let limit = if general_limit_kbps > 0 { kbps.min(general_limit_kbps) } else { kbps };
            Effect { hold: false, limit_kbps: limit, rule: name }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(days: &[usize], start: u16, end: u16, mode: Mode) -> Rule {
        let mut d = [false; 7];
        days.iter().for_each(|&i| d[i] = true);
        Rule { id: "r".into(), name: "Night".into(), enabled: true, days: d, start, end, mode }
    }

    #[test]
    fn same_day_window() {
        let r = rule(&[0, 1, 2, 3, 4], 9 * 60, 17 * 60, Mode::Pause);
        assert!(r.covers(0, 9 * 60));
        assert!(r.covers(4, 16 * 60 + 59));
        assert!(!r.covers(0, 17 * 60), "the end minute is outside");
        assert!(!r.covers(5, 12 * 60), "Saturday is not selected");
    }

    #[test]
    fn window_crossing_midnight_belongs_to_the_day_it_starts() {
        let r = rule(&[4], 22 * 60, 6 * 60, Mode::Pause); // Friday 22:00 to Saturday 06:00
        assert!(r.covers(4, 23 * 60));
        assert!(r.covers(5, 2 * 60), "Saturday 02:00 is still Friday night");
        assert!(!r.covers(5, 6 * 60));
        assert!(!r.covers(4, 2 * 60), "Friday 02:00 belongs to Thursday night, not selected");
        assert!(!r.covers(5, 23 * 60), "Saturday evening is not selected");
    }

    #[test]
    fn equal_start_and_end_means_all_day_and_disabled_rules_never_match() {
        let mut r = rule(&[6], 0, 0, Mode::Full);
        assert!(r.covers(6, 0) && r.covers(6, 1439) && !r.covers(0, 600));
        r.enabled = false;
        assert!(!r.covers(6, 600));
    }

    #[test]
    fn effects() {
        let rules = vec![rule(&[0], 8 * 60, 12 * 60, Mode::Limit { kbps: 2048 }), rule(&[0], 0, 0, Mode::Pause)];
        let e = effect(&rules, 0, 0, 9 * 60);
        assert_eq!((e.hold, e.limit_kbps), (false, 2048), "the first matching rule wins");
        assert_eq!(effect(&rules, 1024, 0, 9 * 60).limit_kbps, 1024, "the lower of rule and general limit");
        assert!(effect(&rules, 0, 0, 13 * 60).hold);
        assert_eq!(effect(&rules, 500, 3, 0), Effect { hold: false, limit_kbps: 500, rule: None });
        let full = vec![rule(&[0], 0, 0, Mode::Full)];
        assert_eq!(effect(&full, 500, 0, 5).limit_kbps, 0);
    }
}

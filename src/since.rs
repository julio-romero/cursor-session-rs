//! `--since`: keeping the sessions updated within a recent span of time.

use std::fmt;

use crate::model::SessionSummary;

/// A span of time back from now, such as `30d` or `12h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Since {
    amount: u64,
    unit: Unit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Seconds,
    Minutes,
    Hours,
    Days,
    Weeks,
}

impl Unit {
    fn suffix(self) -> char {
        match self {
            Unit::Seconds => 's',
            Unit::Minutes => 'm',
            Unit::Hours => 'h',
            Unit::Days => 'd',
            Unit::Weeks => 'w',
        }
    }

    fn millis(self) -> u64 {
        let seconds = match self {
            Unit::Seconds => 1,
            Unit::Minutes => 60,
            Unit::Hours => 60 * 60,
            Unit::Days => 24 * 60 * 60,
            Unit::Weeks => 7 * 24 * 60 * 60,
        };
        seconds * 1000
    }
}

/// Parses a span written as a whole number of at least 1 and one unit:
/// `s`, `m`, `h`, `d` or `w` (`90m`, `12h`, `30d`). A span too long to
/// store reaches back to the earliest time there is.
pub fn parse_since(text: &str) -> Result<Since, String> {
    let invalid = || {
        "expected a whole number of at least 1 and a unit: s, m, h, d or w (e.g. 30d, 12h)"
            .to_string()
    };
    let mut chars = text.chars();
    let unit = match chars.next_back() {
        Some('s') => Unit::Seconds,
        Some('m') => Unit::Minutes,
        Some('h') => Unit::Hours,
        Some('d') => Unit::Days,
        Some('w') => Unit::Weeks,
        _ => return Err(invalid()),
    };
    let digits = chars.as_str();
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    let amount = match digits.parse::<u64>() {
        Ok(0) => return Err(invalid()),
        Ok(amount) => amount,
        Err(_) => u64::MAX,
    };
    Ok(Since { amount, unit })
}

impl Since {
    /// The span in milliseconds, at most `i64::MAX`.
    pub fn millis(self) -> i64 {
        let millis = self.amount.saturating_mul(self.unit.millis());
        i64::try_from(millis).unwrap_or(i64::MAX)
    }

    /// The earliest updated time, in epoch milliseconds, that a session may
    /// have to be kept at `now_ms`.
    pub fn cutoff(self, now_ms: i64) -> i64 {
        now_ms.saturating_sub(self.millis())
    }
}

impl fmt::Display for Since {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.amount, self.unit.suffix())
    }
}

/// Whether `session` was updated at or after `cutoff_ms`, by the time the
/// UPDATED column of `list` shows: its update time, else its creation time.
/// A session with neither is not.
pub fn updated_since(session: &SessionSummary, cutoff_ms: i64) -> bool {
    session
        .updated_or_created_ms()
        .is_some_and(|shown| shown >= cutoff_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;

    #[test]
    fn spans_are_a_number_and_a_unit() {
        let millis = |text| parse_since(text).map(Since::millis);
        assert_eq!(millis("45s"), Ok(45_000));
        assert_eq!(millis("90m"), Ok(90 * 60_000));
        assert_eq!(millis("12h"), Ok(12 * 3_600_000));
        assert_eq!(millis("30d"), Ok(30 * 86_400_000));
        assert_eq!(millis("2w"), Ok(14 * 86_400_000));
        assert_eq!(millis("007d"), Ok(7 * 86_400_000));
        assert_eq!(parse_since("30d").unwrap().to_string(), "30d");
        // Too long to store: as far back as there is.
        assert_eq!(millis("99999999999999999999w"), Ok(i64::MAX));
        assert_eq!(millis("9999999999999999w"), Ok(i64::MAX));
        assert_eq!(parse_since("1w").unwrap().cutoff(0), -604_800_000);
        assert_eq!(
            parse_since("9999999999999999w").unwrap().cutoff(-10),
            i64::MIN
        );

        for invalid in [
            "", "d", "30", "0d", "-1d", "+1d", "1.5h", " 30d", "30d ", "30 d", "30D", "30y", "1dd",
            "d30", "3０d", "30ms", "٣d",
        ] {
            let err = parse_since(invalid).unwrap_err();
            assert!(err.starts_with("expected a whole number"), "{invalid:?}");
        }
    }

    #[test]
    fn sessions_pass_by_the_updated_time_shown() {
        let at = |created, updated| SessionSummary {
            created_at_ms: created,
            updated_at_ms: updated,
            ..SessionSummary::new("id", "title", Source::Agent)
        };
        assert!(updated_since(&at(None, Some(100)), 100));
        assert!(!updated_since(&at(None, Some(99)), 100));
        // Never updated: its creation time.
        assert!(updated_since(&at(Some(150), None), 100));
        assert!(!updated_since(&at(Some(50), None), 100));
        // A recent creation does not keep a session updated long ago.
        assert!(!updated_since(&at(Some(150), Some(50)), 100));
        // No time at all: never kept.
        assert!(!updated_since(&at(None, None), i64::MIN));
        // A time `list` cannot show is taken as missing.
        assert!(!updated_since(&at(None, Some(i64::MAX)), 100));
    }
}

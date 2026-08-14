//! The `special_rule` bit mask: post-processing applied to a column's raw draws
//! before they are shifted and rendered.
//!
//! A rule is a bit rather than a variant so rules compose, and an unknown bit is
//! an error rather than a silent no-op — a description asking for a rule this
//! build does not have must not quietly generate the unruled column instead.

use crate::error::DataGenError;

/// No post-processing; the draws are the values.
pub const RULE_NONE: u32 = 0;

/// Monotonically increase: the column's distribution becomes a *gap*
/// distribution. Each draw is taken as a non-negative gap and the emitted value
/// is the running sum, starting from the column's `shift` (or zero).
///
/// Non-decreasing rather than strictly increasing: there is no minimum-gap
/// field, so a zero gap repeats the previous value rather than being floored to
/// some invented step.
pub const RULE_MONOTONIC_INCREASE: u32 = 0b0001;

/// Every bit this build understands. Anything outside it is rejected.
pub const RULE_MASK_ALL: u32 = RULE_MONOTONIC_INCREASE;

/// Largest magnitude an `f64` still counts by ones. Past it a running sum stops
/// increasing rather than overflowing, which would break monotonicity silently.
const F64_EXACT_INTEGER_LIMIT: f64 = (1u64 << 53) as f64;

/// Reject a mask carrying a bit this build does not implement.
pub fn validate(bits: u32) -> Result<(), DataGenError> {
    let unknown = bits & !RULE_MASK_ALL;
    if unknown != 0 {
        return Err(DataGenError::BadParam(format!(
            "special_rule: unknown bit(s) {unknown:#06b}; this build knows \
             {RULE_MASK_ALL:#06b} (0b0001 = monotonically increase)"
        )));
    }
    Ok(())
}

/// Turns a stream of draws into a non-decreasing series.
///
/// The accumulator is a field, not a call-local, so the series continues across
/// chunk boundaries: N calls of `n` produce what one call of `N*n` would.
#[derive(Debug, Clone)]
pub struct MonotonicAcc {
    acc: f64,
    started: bool,
}

impl MonotonicAcc {
    /// `start` is the column's shift, so the first value of the series is the
    /// shift itself and no gap is applied to it.
    pub fn new(start: f64) -> Self {
        Self {
            acc: start,
            started: false,
        }
    }

    /// Fold the next draw into the series. Negative draws floor at zero, which
    /// repeats the previous value — a Normal gap distribution is half of it.
    pub fn push(&mut self, draw: f64) -> Result<f64, DataGenError> {
        if self.started {
            self.acc += draw.max(0.0);
        } else {
            self.started = true;
        }
        if !self.acc.is_finite() || self.acc.abs() > F64_EXACT_INTEGER_LIMIT {
            return Err(DataGenError::BadParam(format!(
                "special_rule: the monotonic series reached {}, past the range \
                 an f64 counts by ones; it would stop increasing",
                self.acc
            )));
        }
        Ok(self.acc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_bits_are_refused_by_name() {
        assert!(validate(RULE_NONE).is_ok());
        assert!(validate(RULE_MONOTONIC_INCREASE).is_ok());
        let err = validate(0b1000).unwrap_err().to_string();
        assert!(err.contains("unknown bit"), "{err}");
    }

    /// The first value is the start itself: a series whose first element already
    /// carried a gap could not begin at the shift the caller named.
    #[test]
    fn the_series_starts_at_start_and_never_decreases() {
        let mut acc = MonotonicAcc::new(100.0);
        let out: Vec<f64> = [5.0, -3.0, 2.0]
            .iter()
            .map(|d| acc.push(*d).unwrap())
            .collect();
        assert_eq!(out, vec![100.0, 100.0, 102.0]);
    }

    #[test]
    fn a_series_past_the_exact_integer_range_is_an_error() {
        let mut acc = MonotonicAcc::new(0.0);
        acc.push(0.0).unwrap();
        assert!(acc.push(1e18).is_err());
    }
}

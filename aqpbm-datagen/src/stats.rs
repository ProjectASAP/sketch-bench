//! Descriptive summary of a generated column.

use serde::{Deserialize, Serialize};

/// A human-facing summary of a generated column, embedded in the
/// `.meta.json` sidecar and printed by `workload describe`.
///
/// `min`/`max`/`first`/`last` are stored as `f64` for a uniform JSON
/// shape across dtypes. For very large integer values (e.g.
/// epoch-nanosecond timestamps beyond `2^53`) these are approximate —
/// they are a summary for humans, never the data itself (the `.bin`
/// stream holds the exact values).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BasicStats {
    pub count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<f64>,
}

/// Running summary over a chunked value stream.
///
/// The generator can emit a column in pieces (see
/// [`crate::datagen::Sink`]), so the sidecar's summary has to be
/// accumulated rather than computed from a whole slice. Every field is
/// order-independent except `first`/`last`, which is why chunks must be
/// pushed in emission order.
#[derive(Debug, Clone)]
pub struct StatsAcc {
    count: usize,
    /// How many values had a numeric summary. Differs from `count` only for
    /// value types that have none.
    numeric: usize,
    min: f64,
    max: f64,
    first: Option<f64>,
    last: Option<f64>,
}

impl StatsAcc {
    pub fn new() -> Self {
        Self {
            count: 0,
            numeric: 0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            first: None,
            last: None,
        }
    }

    /// Fold one chunk in. Must be called in emission order.
    ///
    /// `to_f64` returns `None` for value types with no numeric summary — a
    /// string column reports only its `count`, because min/max/first/last are
    /// numbers and a string's length is not its value. `count` therefore
    /// tracks every value while the numeric fields track only those that
    /// have one.
    pub(crate) fn push_slice<T, F: Fn(&T) -> Option<f64>>(&mut self, values: &[T], to_f64: F) {
        if values.is_empty() {
            return;
        }
        self.count += values.len();
        for v in values {
            let Some(x) = to_f64(v) else { continue };
            if x < self.min {
                self.min = x;
            }
            if x > self.max {
                self.max = x;
            }
            self.numeric += 1;
            if self.first.is_none() {
                self.first = Some(x);
            }
            self.last = Some(x);
        }
    }

    pub fn finish(self) -> BasicStats {
        if self.numeric == 0 {
            return BasicStats {
                count: self.count,
                min: None,
                max: None,
                first: None,
                last: None,
            };
        }
        BasicStats {
            count: self.count,
            min: Some(self.min),
            max: Some(self.max),
            first: self.first,
            last: self.last,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunked_accumulation_matches_whole_slice() {
        let all: Vec<i64> = (0..100).map(|i| (i * 37) % 61).collect();
        let mut whole_acc = StatsAcc::new();
        whole_acc.push_slice(&all, |x| Some(*x as f64));
        let whole = whole_acc.finish();

        let mut acc = StatsAcc::new();
        for chunk in all.chunks(7) {
            acc.push_slice(chunk, |x| Some(*x as f64));
        }
        assert_eq!(
            acc.finish(),
            whole,
            "chunk size must not change the summary"
        );
    }

    #[test]
    fn empty_stream_has_no_extremes() {
        let s = StatsAcc::new().finish();
        assert_eq!(s.count, 0);
        assert!(s.min.is_none() && s.max.is_none());
    }
}

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

impl BasicStats {
    /// Summarize a slice, projecting each element to `f64` via `to_f64`.
    pub(crate) fn summarize<T, F: Fn(&T) -> f64>(values: &[T], to_f64: F) -> Self {
        if values.is_empty() {
            return Self {
                count: 0,
                min: None,
                max: None,
                first: None,
                last: None,
            };
        }
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for v in values {
            let x = to_f64(v);
            if x < min {
                min = x;
            }
            if x > max {
                max = x;
            }
        }
        Self {
            count: values.len(),
            min: Some(min),
            max: Some(max),
            first: Some(to_f64(&values[0])),
            last: Some(to_f64(&values[values.len() - 1])),
        }
    }
}

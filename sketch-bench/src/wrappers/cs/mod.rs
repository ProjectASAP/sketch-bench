//! CountSketch wrappers — four types (`oxide` + 3× sketchlib), each declaring
//! `FrequencyOps`. Same shapes and the same three routes as the Count-Min
//! wrappers next door, including one generic row over the shared shape table.

use crate::params::*;
use sketch_oxide::Mergeable as _;

pub mod oxide;
pub mod polars;
pub mod sketchlib;

// `width = ceil(3/ε²).next_power_of_two()`, not `ceil(2/ε)` like CountMin.
//
// Inverting that for a target width needs `3/ε²` to land *on* `cols`, and
// `ε = sqrt(3/cols)` does not: the square root and the square do not round-trip
// in `f64`, so `3/ε²` came out a hair above `cols`, `ceil` took it to `cols + 1`
// and the power-of-two rounding doubled the table. Every request built twice the
// counters it named. Solving against `cols - 0.5` puts the quotient half an
// integer below the boundary, which no single ulp can cross.
fn dims_to_err(rows: usize, cols: usize) -> (f64, f64) {
    let epsilon = (3.0 / (cols as f64 - 0.5)).sqrt();
    let delta = (-(rows as f64 - 0.5)).exp();
    (epsilon, delta)
}

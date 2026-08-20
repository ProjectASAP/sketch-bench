use crate::params::*;
use crate::wrappers::BuildError;

pub mod oxide;
pub mod sketchlib;

pub use super::quantile_value::{QuantileValue, ToF64};

fn require_alpha(what: &str, alpha: f64) -> Result<(), BuildError> {
    if !(alpha > 0.0 && alpha < 1.0) {
        return Err(BuildError(format!(
            "{what}: alpha={alpha} outside (0, 1), which is the relative accuracy \
             this library accepts"
        )));
    }
    Ok(())
}

fn dd_footprint(alpha: f64, min: Option<f64>, max: Option<f64>, count: u64) -> usize {
    let (Some(min), Some(max)) = (min, max) else {
        return 0;
    };
    let (lo, hi) = (min.abs().min(max.abs()), min.abs().max(max.abs()));
    let log_gamma = ((1.0 + alpha) / (1.0 - alpha)).ln();
    let span = if lo > 0.0 && hi.is_finite() && log_gamma > 0.0 {
        ((hi.ln() - lo.ln()) / log_gamma).floor() as u64 + 1
    } else {
        count
    };
    let stores = if min < 0.0 && max > 0.0 { 2 } else { 1 };
    (span.saturating_mul(stores).min(count) as usize) * std::mem::size_of::<u64>()
}

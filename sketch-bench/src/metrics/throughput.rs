//! `items_inserted / wall_s` — the simplest + most-cited
//! benchmark number. Computed from `RunMetrics` at the
//! aggregation step rather than maintained incrementally,
//! because the value isn't stable until the phase closes.

/// Throughput for the insert phase, computed from raw counts +
/// nanoseconds.
#[derive(Debug, Clone, Copy)]
pub struct ItemsPerSec;

impl ItemsPerSec {
    pub fn compute(items: u64, wall_ns: u64) -> f64 {
        if wall_ns == 0 {
            return 0.0;
        }
        (items as f64) / (wall_ns as f64 / 1_000_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_per_sec_basic_math() {
        let tp = ItemsPerSec::compute(1_000_000, 100_000_000); // 100ms -> 10M items/sec
        assert!((tp - 10_000_000.0).abs() < 1.0);
    }

    #[test]
    fn zero_wall_ns_returns_zero() {
        assert_eq!(ItemsPerSec::compute(1000, 0), 0.0);
    }
}

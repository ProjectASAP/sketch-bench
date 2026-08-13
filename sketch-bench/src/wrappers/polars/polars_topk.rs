//! `topk/polars` — the exact top-k, reusing the frequency core's `group_by`
//! then sorting. An exact answer must come back at precision = recall = 1.0,
//! which is what makes this row a check on the ground truth.

use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::config::ParamSet;
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::accuracy::TopKOps;
use crate::params::TopkParams;

use super::PolarsFrequencyCore;

/// `topk/polars` — the exact top-k baseline, reusing the frequency baseline's
/// group_by then sorting. Its score checks the comparator: an exact answer must
/// come back at precision = recall = 1.0.
#[derive(Default)]
pub struct PolarsTopK(PolarsFrequencyCore);

impl InitSketch for PolarsTopK {
    /// The one polars baseline that reads its config: `k` is the prefix the
    /// comparator scores against, not a knob to shrug off, so an unreadable `k`
    /// must fail here rather than be silently scored at some other `k`.
    fn init(config: &ParamSet) -> Result<Self, BuildError> {
        let p: TopkParams = config.parse()?;
        if p.k == 0 {
            return Err(BuildError("topk needs k >= 1".into()));
        }
        Ok(Self::default())
    }
}

impl Accumulator for PolarsTopK {
    type Item = i64;
    #[inline(always)]
    fn update(&mut self, v: &i64) {
        self.0.update(v);
    }
    /// All of the cost is here, not in `update` — the same split the other
    /// polars baselines use, so the insert column stays a plain `Vec::push`.
    fn prepare(&mut self) {
        self.0.finalize();
    }
}

impl MemoryFootprint for PolarsTopK {
    fn memory_bytes(&self) -> usize {
        self.0.memory_bytes()
    }
}

impl TopKOps for PolarsTopK {
    type Key = i64;
    fn estimate_topk(&self, k: usize) -> Vec<(i64, u64)> {
        let mut out: Vec<(i64, u64)> = self.0.counts.iter().map(|(&k, &c)| (k, c)).collect();
        out.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out.truncate(k);
        out
    }
}

impl BenchImpl for PolarsTopK {
    type Params = TopkParams;
    const IMPL: &'static str = "polars";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topk_config(params: serde_json::Value) -> ParamSet {
        ParamSet {
            algorithm: "topk".to_string(),
            params,
        }
    }

    /// Exactness is no excuse for accepting a config the rest of the algorithm
    /// rejects: this row is scored at `k`, so an unreadable `k` is a build
    /// failure, not a default.
    #[test]
    fn a_k_that_cannot_be_read_is_a_build_error() {
        let err = match PolarsTopK::init(&topk_config(
            serde_json::json!({ "rows": 5, "cols": 2048, "kk": 5 }),
        )) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("a misspelled `k` must not build"),
        };
        assert!(err.contains("kk"), "error should name the bad key: {err}");

        assert!(PolarsTopK::init(&topk_config(
            serde_json::json!({ "rows": 5, "cols": 2048 })
        ))
        .is_err());
        assert!(PolarsTopK::init(&topk_config(
            serde_json::json!({ "rows": 5, "cols": 2048, "k": 0 })
        ))
        .is_err());
        assert!(PolarsTopK::init(&topk_config(
            serde_json::json!({ "rows": 5, "cols": 2048, "k": 5 })
        ))
        .is_ok());
    }
}

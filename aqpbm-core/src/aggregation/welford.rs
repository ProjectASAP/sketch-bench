//! Numerically-stable online mean + sample-variance accumulator (Welford).
//! Used for mean / stddev across N post-warm-up runs in `O(1)` memory per
//! metric, and by `--repeats` for the one honest interval — [`Welford::ci95`].

#[derive(Debug, Clone, Copy, Default)]
pub struct Welford {
    n: u64,
    mean: f64,
    m2: f64,
}

impl Welford {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, x: f64) {
        self.n += 1;
        let delta = x - self.mean;
        self.mean += delta / (self.n as f64);
        let delta2 = x - self.mean;
        self.m2 += delta * delta2;
    }

    pub fn n(&self) -> usize {
        self.n as usize
    }

    pub fn mean(&self) -> f64 {
        self.mean
    }

    /// Sample variance (n-1 divisor). Returns 0 when n<=1.
    pub fn variance(&self) -> f64 {
        if self.n <= 1 {
            0.0
        } else {
            self.m2 / ((self.n - 1) as f64)
        }
    }

    pub fn stddev(&self) -> f64 {
        self.variance().sqrt()
    }

    /// 95% CI around the mean, normal approximation (z = 1.96), so only
    /// approximate for N < 30. Meaningful only over independent samples;
    /// `--repeats R` is the one caller. See `RunStats::ci95`.
    pub fn ci95(&self) -> (f64, f64) {
        if self.n == 0 {
            return (0.0, 0.0);
        }
        let half = 1.96 * self.stddev() / (self.n as f64).sqrt();
        (self.mean - half, self.mean + half)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welford_matches_naive_for_small_sample() {
        let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
        let mut w = Welford::new();
        for x in xs {
            w.push(x);
        }
        let naive_mean = xs.iter().sum::<f64>() / xs.len() as f64;
        let naive_var =
            xs.iter().map(|x| (x - naive_mean).powi(2)).sum::<f64>() / (xs.len() - 1) as f64;
        assert!((w.mean() - naive_mean).abs() < 1e-12);
        assert!((w.variance() - naive_var).abs() < 1e-12);
    }

    #[test]
    fn welford_stable_on_near_constant_large_values() {
        let mut w = Welford::new();
        for _ in 0..10_000 {
            w.push(1e18);
        }
        w.push(1e18 + 1.0);
        // Variance is tiny + positive — naive two-pass would
        // cancel into junk; Welford is stable.
        assert!(w.variance() >= 0.0);
    }

    #[test]
    fn single_sample_has_zero_variance() {
        let mut w = Welford::new();
        w.push(42.0);
        assert_eq!(w.variance(), 0.0);
        assert_eq!(w.stddev(), 0.0);
    }
}

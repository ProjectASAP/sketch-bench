//! Memory metrics: RSS peak from `/proc/self/status:VmHWM`
//! (Linux) + optional jemalloc heap-peak (opt-in).

/// RSS peak in kB, pulled from `/proc/self/status:VmHWM`.
/// Returns `None` on non-Linux or if the file can't be read.
pub struct Rss;

impl Rss {
    pub fn peak_kb() -> Option<u64> {
        let s = std::fs::read_to_string("/proc/self/status").ok()?;
        for line in s.lines() {
            if let Some(rest) = line.strip_prefix("VmHWM:") {
                // VmHWM:   1234 kB
                return rest
                    .split_whitespace()
                    .next()
                    .and_then(|tok| tok.parse::<u64>().ok());
            }
        }
        None
    }
}

/// Jemalloc heap-peak, compiled in only when the
/// `heap-jemalloc` feature is enabled. Consumers linking the
/// default system allocator get `None` back from `peak_kb`.
pub struct JemallocPeak;

impl JemallocPeak {
    #[cfg(feature = "heap-jemalloc")]
    pub fn peak_kb() -> Option<u64> {
        use tikv_jemalloc_ctl::{epoch, stats};
        let _ = epoch::advance();
        stats::allocated::read().ok().map(|b| (b as u64) / 1024)
    }

    #[cfg(not(feature = "heap-jemalloc"))]
    pub fn peak_kb() -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss_peak_nonzero_on_linux() {
        let r = Rss::peak_kb();
        #[cfg(target_os = "linux")]
        assert!(r.is_some() && r.unwrap() > 0);
        #[cfg(not(target_os = "linux"))]
        assert!(r.is_none());
    }
}

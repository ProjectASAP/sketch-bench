//! Runtime toggle — a cheap atomic bool the controller can flip without
//! restarting the host, shared between the `Sampler` and whoever controls
//! sampling. Costs one `Relaxed` atomic load per op on the hot path: a single
//! cycle on any modern CPU, no CAS and no fence.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct RuntimeSwitch(Arc<AtomicBool>);

impl RuntimeSwitch {
    pub fn new(initial: bool) -> Self {
        Self(Arc::new(AtomicBool::new(initial)))
    }

    pub fn on() -> Self {
        Self::new(true)
    }

    pub fn off() -> Self {
        Self::new(false)
    }

    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub fn enable(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn disable(&self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

impl Default for RuntimeSwitch {
    fn default() -> Self {
        Self::on()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_default_is_on() {
        assert!(RuntimeSwitch::default().is_enabled());
    }

    #[test]
    fn enable_disable_toggles() {
        let s = RuntimeSwitch::off();
        assert!(!s.is_enabled());
        s.enable();
        assert!(s.is_enabled());
        s.disable();
        assert!(!s.is_enabled());
    }

    #[test]
    fn clones_share_state() {
        let a = RuntimeSwitch::off();
        let b = a.clone();
        a.enable();
        assert!(b.is_enabled());
    }
}

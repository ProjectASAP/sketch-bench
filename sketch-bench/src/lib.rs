//! `sketch-bench` — the sketches themselves: wrapped implementations, their
//! per-family construction parameters, the registry resolving
//! `("hll-hip", "lib")` to one of them, and the CSV rendering. A bundle, not a framework: to
//! benchmark a sketch of your own, depend on `aqpbm-core` and implement its
//! traits. This crate exists for the CLI. See `docs/DESIGN.md` §5.

pub mod registry;
pub mod legacy_csv;
pub mod params;
pub mod wrappers;

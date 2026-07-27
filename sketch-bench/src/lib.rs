//! `sketch-bench` — the sketches themselves: the wrapped implementations this
//! project measures, the per-family construction parameters they take, the
//! catalog that resolves `("hll", "oxide")` to one of them, and the legacy CSV
//! rendering.
//!
//! **This is a bundle of implementations, not a framework.** The framework —
//! the traits an implementation plugs into, the cell runners, the per-statistic
//! comparators — is `aqpbm-core`, and it holds no list of implementations. To
//! benchmark a sketch of your own, depend on that crate and implement its
//! traits; see its crate docs. Nothing here needs to know about you, and you
//! pay for none of the four sketch libraries this crate wraps.
//!
//! What this crate is *for* is the CLI: `sketchlib bench --sketch hll --impl
//! oxide` arrives holding two strings, and something has to turn those into a
//! concrete Rust type. That is [`catalog`], and it is the only reason a list of
//! implementations exists at all. See `docs/DESIGN.md` §5.

pub mod catalog;
pub mod legacy_csv;
pub mod params;
pub mod wrappers;

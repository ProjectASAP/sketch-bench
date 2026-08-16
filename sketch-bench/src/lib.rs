//! `sketch-bench` — the sketches themselves: the wrapped implementations, their
//! per-family construction parameters, and the registry that resolves
//! `("hll-hip", "lib")` to one of them.
//!
//! A bundle, not a framework. `aqpbm-core` measures *a* sketch and names none
//! of them; this crate is where the set lives, and where how-each-is-used
//! lives with it. See `docs/sketch-bench.md`.

/// Why a sketch could not be built at a requested config.
pub mod build_error;
/// The per-family construction parameter vocabularies.
pub mod params;
/// Which sketches are registered, what each supports, and how a request
/// resolves to one.
pub mod registry;
/// One directory per algorithm, one file per library.
pub mod wrappers;

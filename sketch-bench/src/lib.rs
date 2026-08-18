//! `sketch-bench` — the sketches themselves: the wrapped implementations, their
//! per-family construction parameters, and the registry that resolves
//! `("hll-hip", "lib")` to one. A bundle, not a framework. See `docs/sketch-bench.md`.

/// The per-family construction parameter vocabularies.
pub mod params;
/// Which sketches are registered, what each supports, and how a request
/// resolves to one.
pub mod registry;
/// What a frontend asks for, as one value.
pub mod request;
/// One directory per algorithm, one file per library.
pub mod wrappers;

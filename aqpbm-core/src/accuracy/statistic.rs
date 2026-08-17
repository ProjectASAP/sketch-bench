//! Once: one capability trait per statistic — `CardinalityOps`, `FrequencyOps`,
//! `QuantileOps`, `TopKOps` and the three subpopulation variants — each fixing
//! the signature every implementation had to answer through.
//!
//! They are gone. A sketch now states how it is queried by supplying a closure
//! at its row in `sketch_bench::registry::REGISTRY`, and `GroundTruth` no longer
//! touches a sketch at all.
//!
//! What the traits cost, concretely, and why the closure replaces them:
//!
//! - **They fixed `&self`.** `sketch_oxide`'s `KllSketch::quantile` needs
//!   `&mut self`, so the wrapper carried a `RefCell` and paid a borrow check on
//!   every query — for a signature detail, not for anything the sketch does.
//! - **They fixed the probe and answer types**, so a statistic none of the
//!   seven named could not be scored at all. That is what blocked scoring a
//!   heavy-hitter set or a moment estimate (`docs/sketch-bench.md`, first open
//!   question).
//!
//! What is lost with them: the nominal check. `impl FrequencyOps for X` used to
//! be a compiler-checked claim that X answers frequency, and a row naming a
//! comparator its sketch could not satisfy would not build. A closure is
//! checked only for shape, so `sketch-bench`'s registry tests carry that weight
//! now — see `every_row_answers_its_capability`.

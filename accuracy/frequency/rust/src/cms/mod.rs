//! CMS (Count-Min Sketch) runners used by the frequency
//! accuracy harness. Shared infrastructure
//! (`config.rs`, `seeds.rs`, `output.rs`, `baseline.rs`) lives at
//! the crate root.

pub mod datasketches;
pub mod oxide;
pub mod sketchlib;

//! Thin `Sketch`-trait newtypes over each of the 21 concrete
//! sketch implementations in the repo. One file per family;
//! `dispatch.rs` picks between them at CLI parse time.

pub mod cms;
pub mod countsketch;
pub mod dd;
pub mod elastic;
pub mod exact;
pub mod hll;
pub mod kll;
pub mod nitro;
pub mod parallel;
#[cfg(feature = "polars")]
pub mod polars;
pub mod univmon;

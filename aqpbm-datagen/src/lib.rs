//! Synthetic data generation — see `docs/aqpbm-datagen.md`. In-memory only: no
//! files, no workspace deps. Four separable axes: distribution ([`dist`]), rule
//! ([`rule`]), rendering ([`value`]), relation ([`table`]).

pub mod burst;
pub mod column;
pub mod dist;
pub mod error;
pub mod rule;
pub mod table;
pub mod value;

mod splitmix64;
#[cfg(test)]
mod test_support;

pub use burst::{inject_interval_bursts, BurstSpec};
pub use column::{ColumnSpec, DATA_TYPES};
pub use dist::{
    DataDistribution, Domain, NormalParameter, Sampler, UniformParameter, ZipfParameter,
};
pub use error::DataGenError;
pub use rule::{RULE_MASK_ALL, RULE_MONOTONIC_INCREASE, RULE_NONE};
pub use table::{GeneratedTable, TableDescription};
pub use value::{ColumnData, ColumnItem, StrCfg, StringOpts};

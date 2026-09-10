//! The table: what a caller describes, and what it gets back.
//!
//! One description covers both cases the crate serves — a single column for
//! sketch insertion, and a table for query processing — so nothing downstream
//! has to pick between two entry points.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::column::ColumnSpec;
use crate::error::DataGenError;
use crate::value::ColumnData;
use crate::{inject_interval_bursts, BurstSpec};

/// Scramble a latent rank before dividing it among connected columns. This
/// SplitMix64 finalizer spreads nearby ranks across the word while keeping the
/// mapping deterministic.
#[inline]
fn mix_connected_bits(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Take one equal-width chunk from a mixed 64-bit draw. Member zero receives
/// the least-significant chunk; any high remainder bits are unused.
#[inline]
fn connected_chunk(bits: u64, member: usize, group_size: usize) -> u64 {
    debug_assert!((2..=u64::BITS as usize).contains(&group_size));
    debug_assert!(member < group_size);
    let width = u64::BITS as usize / group_size;
    let mask = (1u64 << width) - 1;
    (bits >> (member * width)) & mask
}

/// A complete generation request.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableDescription {
    pub column_num: u32,
    pub column_label: Vec<String>,
    pub column_spec: Vec<ColumnSpec>,
    /// Groups of columns that vary together, named by label. See
    /// [`TableDescription::validate`] for what a group must satisfy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub column_connected: Vec<Vec<String>>,
    pub row_num: u64,
}

impl TableDescription {
    /// The single-column case, which is most of the callers: one label, one
    /// spec, no connections.
    pub fn single(label: impl Into<String>, spec: ColumnSpec, row_num: u64) -> Self {
        Self {
            column_num: 1,
            column_label: vec![label.into()],
            column_spec: vec![spec],
            column_connected: Vec::new(),
            row_num,
        }
    }

    /// Load a description from a `.yaml`/`.yml` (serde_norway) or otherwise JSON
    /// file.
    pub fn from_path(path: &Path) -> Result<Self, DataGenError> {
        let text = std::fs::read_to_string(path)?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("yaml") | Some("yml") => serde_norway::from_str(&text)
                .map_err(|e| DataGenError::BadParam(format!("spec yaml: {e}"))),
            _ => serde_json::from_str(&text)
                .map_err(|e| DataGenError::BadParam(format!("spec json: {e}"))),
        }
    }

    /// Every check that can be made before drawing: the counts agree, each
    /// column is self-consistent, and each connection group is one that can be
    /// served. Called by [`Self::generate`], and public so a frontend can reject
    /// a file without generating from it.
    pub fn validate(&self) -> Result<(), DataGenError> {
        // The doc's first required check: `column_num` and the actual column
        // count must agree, or every downstream index is off.
        if self.column_num as usize != self.column_spec.len() {
            return Err(DataGenError::BadParam(format!(
                "column_num says {} columns but column_spec holds {}",
                self.column_num,
                self.column_spec.len()
            )));
        }
        if self.column_label.len() != self.column_spec.len() {
            return Err(DataGenError::BadParam(format!(
                "column_label holds {} labels for {} columns",
                self.column_label.len(),
                self.column_spec.len()
            )));
        }
        if self.column_spec.is_empty() {
            return Err(DataGenError::BadParam(
                "a table with no columns generates nothing".into(),
            ));
        }
        // An empty table benchmarks nothing, yet every downstream stage would
        // accept it and report `0.0 items/sec`.
        if self.row_num == 0 {
            return Err(DataGenError::BadParam("row_num must be > 0".into()));
        }

        let index = self.label_index()?;

        for (i, spec) in self.column_spec.iter().enumerate() {
            spec.validate().map_err(|e| {
                DataGenError::BadParam(format!("column '{}': {e}", self.column_label[i]))
            })?;
        }

        let mut claimed: HashMap<&str, usize> = HashMap::new();
        for (g, group) in self.column_connected.iter().enumerate() {
            if group.len() < 2 {
                return Err(DataGenError::BadParam(format!(
                    "column_connected group {g} names {} column(s); a relation needs at least 2",
                    group.len()
                )));
            }
            if group.len() > u64::BITS as usize {
                return Err(DataGenError::BadParam(format!(
                    "column_connected group {g} names {} columns; a 64-bit draw can supply at \
                     most 64 non-empty chunks",
                    group.len()
                )));
            }
            let mut members = Vec::with_capacity(group.len());
            for label in group {
                let Some(&i) = index.get(label.as_str()) else {
                    return Err(DataGenError::BadParam(format!(
                        "column_connected group {g} names '{label}', which is not a column label"
                    )));
                };
                if let Some(&prev) = claimed.get(label.as_str()) {
                    return Err(DataGenError::BadParam(format!(
                        "column '{label}' appears in column_connected groups {prev} and {g}; \
                         a column varies with one group or none"
                    )));
                }
                claimed.insert(label.as_str(), g);
                members.push((label, i));
            }
            // A connected group is one latent draw mapped to a tuple, so every
            // member must agree on the distribution of that latent value.
            let (head_label, head) = members[0];
            for (label, i) in &members[1..] {
                if self.column_spec[*i].distribution != self.column_spec[head].distribution {
                    return Err(DataGenError::BadParam(format!(
                        "column_connected group {g}: '{head_label}' and '{label}' are related \
                         but draw differently; related columns need an identical distribution \
                         and identical parameters, seed included"
                    )));
                }
            }
            match self.column_spec[head].distribution.domain() {
                Some(domain) if domain.size > 0 => {}
                Some(_) => {
                    return Err(DataGenError::BadParam(format!(
                        "column_connected group {g}: the shared distribution needs a positive \
                         integer domain for chunk mapping"
                    )))
                }
                None => {
                    return Err(DataGenError::BadParam(format!(
                        "column_connected group {g}: the shared distribution must have a \
                         bounded integer domain for chunk mapping"
                    )))
                }
            }
        }

        Ok(())
    }

    /// Generate the whole table into memory.
    pub fn generate(&self) -> Result<GeneratedTable, DataGenError> {
        self.validate()?;
        let row_num = usize::try_from(self.row_num).map_err(|_| {
            DataGenError::BadParam(format!("row_num {} exceeds this platform", self.row_num))
        })?;

        // Which group each column belongs to, if any. Built from the labels once
        // so the generation loop below indexes rather than searches.
        let index = self.label_index()?;
        let mut group_of: Vec<Option<(usize, usize)>> = vec![None; self.column_spec.len()];
        for (g, group) in self.column_connected.iter().enumerate() {
            for (member, label) in group.iter().enumerate() {
                group_of[index[label.as_str()]] = Some((g, member));
            }
        }

        // Each latent value is drawn and mixed once. The generation loop below
        // gives every member a different bit chunk from that shared value, so a
        // repeated latent value always produces the same connected tuple.
        let mut group_bits: HashMap<usize, Vec<u64>> = HashMap::new();
        for (g, group) in self.column_connected.iter().enumerate() {
            let head = index[group[0].as_str()];
            let domain = self.column_spec[head]
                .distribution
                .domain()
                .expect("connected domains were validated above");
            let bits = self.column_spec[head]
                .draw(row_num)?
                .into_iter()
                .map(|draw| mix_connected_bits((draw - domain.lower) as u64))
                .collect();
            group_bits.insert(g, bits);
        }

        let mut data = Vec::with_capacity(self.column_spec.len());
        for (i, spec) in self.column_spec.iter().enumerate() {
            // Only one of these buffers is initialized. Either one lives just
            // long enough for this column's render pass.
            let own_draw;
            let connected_draw;
            let raw: &[f64] = match group_of[i] {
                Some((g, member)) => {
                    let domain = spec
                        .distribution
                        .domain()
                        .expect("connected domains were validated above");
                    let group_size = self.column_connected[g].len();
                    connected_draw = group_bits[&g]
                        .iter()
                        .map(|&bits| {
                            domain.lower
                                + (connected_chunk(bits, member, group_size) % domain.size) as f64
                        })
                        .collect::<Vec<_>>();
                    &connected_draw
                }
                None => {
                    own_draw = spec.draw(row_num)?;
                    &own_draw
                }
            };
            data.push(spec.render(raw).map_err(|e| {
                DataGenError::BadParam(format!("column '{}': {e}", self.column_label[i]))
            })?);
        }

        let table = GeneratedTable {
            column_num: self.column_num,
            column_title: self.column_label.clone(),
            data,
            row_num: self.row_num,
        };
        table.validate()?;
        Ok(table)
    }

    /// Label → column index, rejecting duplicates: `column_connected` names
    /// columns by label, so two columns sharing one would be ambiguous.
    fn label_index(&self) -> Result<HashMap<&str, usize>, DataGenError> {
        let mut index = HashMap::with_capacity(self.column_label.len());
        for (i, label) in self.column_label.iter().enumerate() {
            if index.insert(label.as_str(), i).is_some() {
                return Err(DataGenError::BadParam(format!(
                    "column_label '{label}' is used twice; labels name columns in \
                     column_connected, so they have to be distinct"
                )));
            }
        }
        Ok(index)
    }
}

/// A generated table. Each column can be handed downstream by ownership and
/// peeled with [`ColumnData::into_i64`] and friends.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedTable {
    pub column_num: u32,
    pub column_title: Vec<String>,
    pub data: Vec<ColumnData>,
    pub row_num: u64,
}

impl GeneratedTable {
    /// Inject the same deterministic interval-burst schedule into every column.
    /// Reusing the seed preserves row alignment for multi-column workloads.
    pub fn inject_bursts(&mut self, spec: BurstSpec) -> Result<(), DataGenError> {
        for column in &mut self.data {
            *column = match column {
                ColumnData::Int64(values) => {
                    ColumnData::Int64(inject_interval_bursts(values, spec)?)
                }
                ColumnData::Float64(values) => {
                    ColumnData::Float64(inject_interval_bursts(values, spec)?)
                }
                ColumnData::Unsigned64(values) => {
                    ColumnData::Unsigned64(inject_interval_bursts(values, spec)?)
                }
                ColumnData::String(values) => {
                    ColumnData::String(inject_interval_bursts(values, spec)?)
                }
            };
        }
        self.row_num = self.data.first().map_or(0, |column| column.len() as u64);
        self.validate()
    }

    /// The doc's two required checks, on the output side: the column count
    /// matches what was claimed, and every column is the same length.
    pub fn validate(&self) -> Result<(), DataGenError> {
        if self.column_num as usize != self.data.len() {
            return Err(DataGenError::BadParam(format!(
                "column_num says {} columns but the table holds {}",
                self.column_num,
                self.data.len()
            )));
        }
        if self.column_title.len() != self.data.len() {
            return Err(DataGenError::BadParam(format!(
                "column_title holds {} titles for {} columns",
                self.column_title.len(),
                self.data.len()
            )));
        }
        for (i, column) in self.data.iter().enumerate() {
            if column.len() as u64 != self.row_num {
                return Err(DataGenError::BadParam(format!(
                    "column '{}' holds {} rows, but the table claims {}",
                    self.column_title[i],
                    column.len(),
                    self.row_num
                )));
            }
        }
        Ok(())
    }

    pub fn column(&self, i: usize) -> Result<&ColumnData, DataGenError> {
        self.data.get(i).ok_or_else(|| {
            DataGenError::BadParam(format!(
                "column {i} was asked for, but the table holds {}",
                self.data.len()
            ))
        })
    }

    /// Take one column by index, giving up ownership of it.
    pub fn into_column(mut self, i: usize) -> Result<ColumnData, DataGenError> {
        if i >= self.data.len() {
            return Err(DataGenError::BadParam(format!(
                "column {i} was asked for, but the table holds {}",
                self.data.len()
            )));
        }
        Ok(self.data.swap_remove(i))
    }

    /// Take every column, in order.
    pub fn into_columns(self) -> Vec<ColumnData> {
        self.data
    }
}

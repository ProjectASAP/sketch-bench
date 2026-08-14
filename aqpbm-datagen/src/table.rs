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

/// A complete generation request.
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

    /// Load a description from a `.yaml`/`.yml` (serde_yaml) or otherwise JSON
    /// file.
    pub fn from_path(path: &Path) -> Result<Self, DataGenError> {
        let text = std::fs::read_to_string(path)?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("yaml") | Some("yml") => serde_yaml::from_str(&text)
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
            // The requirement the doc states: related columns are one draw seen
            // several ways, which only holds if they draw the same way.
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
        let mut group_of: Vec<Option<usize>> = vec![None; self.column_spec.len()];
        for (g, group) in self.column_connected.iter().enumerate() {
            for label in group {
                group_of[index[label.as_str()]] = Some(g);
            }
        }

        // A group is drawn once and every member renders from that one stream —
        // which is what makes the columns co-vary rather than merely share a
        // distribution.
        let mut group_draw: HashMap<usize, Vec<f64>> = HashMap::new();
        for (g, group) in self.column_connected.iter().enumerate() {
            let head = index[group[0].as_str()];
            group_draw.insert(g, self.column_spec[head].draw(row_num)?);
        }

        let mut data = Vec::with_capacity(self.column_spec.len());
        for (i, spec) in self.column_spec.iter().enumerate() {
            // Declared here and assigned only on the unconnected path, so an
            // unconnected column's draw lives exactly as long as its render.
            let own_draw;
            let raw: &[f64] = match group_of[i] {
                Some(g) => &group_draw[&g],
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

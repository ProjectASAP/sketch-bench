//! The multi-column stream: `n - 1` label columns joined into a key, plus one
//! value column.
//!
//! This is what the grouped (Hydra) rows ingest. As in [`super::numeric`],
//! generation happens in [`crate::dataset::DatasetSpec::build`] and only
//! [`LabeledDataset::from_table`] is offered here.

use aqpbm_datagen::{ColumnItem, DataGenError, GeneratedTable, TableDescription};

use super::{Dataset, DatasetDescription};

/// One record of a multi-column stream: the label columns joined with `;`,
/// plus the measured value.
///
/// The join happens once at generation, so a wrapper feeding a library that
/// takes `"a;b"` pays nothing for it on the insert path. The parts are not
/// stored alongside it because only the untimed comparator asks for them.
#[derive(Debug, Clone, PartialEq)]
pub struct Labeled<V> {
    pub key: String,
    pub value: V,
}

impl<V> Labeled<V> {
    /// The label columns, in the order they were generated. Private: splitting
    /// the key is an implementation detail of [`Self::label`], which is what
    /// callers actually want.
    fn labels(&self) -> std::str::Split<'_, char> {
        self.key.split(';')
    }

    /// The label in column `i`, or `None` past the last column.
    pub fn label(&self, i: usize) -> Option<&str> {
        self.labels().nth(i)
    }
}

/// A multi-column dataset: `n - 1` label columns followed by one value column,
/// all from one [`TableDescription`].
///
/// No new generator: the description already covers a table, and this type only
/// zips its columns into records. Which means a column's distribution, skew and
/// seed are all independently steerable, using the vocabulary that already
/// exists — including `column_connected`, so two label columns can be made to
/// co-vary.
#[derive(Debug, Clone)]
pub struct LabeledDataset<V> {
    items: Vec<Labeled<V>>,
    description: DatasetDescription,
}

impl<V: ColumnItem> LabeledDataset<V> {
    /// Zip an already-generated table into records: all but the last column are
    /// label columns and must be `data_type: string`, the last is the value
    /// column and must be this row's item type. See
    /// [`NumericDataset::from_table`](super::numeric::NumericDataset::from_table)
    /// for why the caller generates.
    pub fn from_table(
        spec: &TableDescription,
        table: GeneratedTable,
    ) -> Result<Self, DataGenError> {
        if spec.column_spec.len() < 2 {
            return Err(DataGenError::BadParam(format!(
                "this row ingests labelled records, so it needs at least one label \
                 column before the value column; the description has {} column(s)",
                spec.column_spec.len(),
            )));
        }
        let description = DatasetDescription::from_spec(spec);
        let labels_end = table.data.len() - 1;
        let titles = table.column_title.clone();
        let mut columns = table.into_columns();

        let value_column = columns.pop().expect("checked non-empty above");
        let values = V::from_column(value_column).map_err(|e| {
            DataGenError::BadParam(format!(
                "value column '{}': {e}. The value column's data_type has to be the \
                 row's item type",
                titles[labels_end],
            ))
        })?;

        let labels: Vec<Vec<String>> = columns
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                c.into_string().map_err(|e| {
                    DataGenError::BadParam(format!(
                        "label column '{}': {e}. Label columns are rendered as text, \
                         so their data_type has to be `string`",
                        titles[i],
                    ))
                })
            })
            .collect::<Result<_, _>>()?;

        let size = spec.row_num as usize;
        let mut items = Vec::with_capacity(size);
        for i in 0..size {
            let mut key = String::new();
            for (col_idx, col) in labels.iter().enumerate() {
                if col_idx > 0 {
                    key.push(';');
                }
                key.push_str(&col[i]);
            }
            items.push(Labeled {
                key,
                value: values[i].clone(),
            });
        }

        Ok(Self { items, description })
    }
}

impl<V: ColumnItem> Dataset for LabeledDataset<V> {
    type Item = Labeled<V>;

    fn description(&self) -> DatasetDescription {
        self.description.clone()
    }

    fn items(&self) -> &[Labeled<V>] {
        &self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqpbm_datagen::{
        ColumnSpec, DataDistribution, StringOpts, UniformParameter, ZipfParameter,
    };
    use std::collections::BTreeSet;

    /// Generate and zip — the two steps `DatasetSpec::build` takes before
    /// handing a labelled row its dataset.
    fn build(spec: &TableDescription) -> Result<LabeledDataset<i64>, DataGenError> {
        let table = spec.generate()?;
        LabeledDataset::from_table(spec, table)
    }

    /// A label column: `cardinality` distinct strings over `alphabet`.
    fn label_col(cardinality: u64, alphabet: &str, seed: u64) -> ColumnSpec {
        ColumnSpec {
            distribution: DataDistribution::Uniform(UniformParameter {
                lower_bound: 0.0,
                upper_bound: cardinality as f64,
                seed,
            }),
            shift: None,
            cardinality: None,
            special_rule: aqpbm_datagen::RULE_NONE,
            data_type: "string".into(),
            string: Some(StringOpts {
                alphabet: alphabet.to_string(),
                min_len: 3,
                max_len: 3,
            }),
        }
    }

    fn value_col(cardinality: u64, seed: u64, data_type: &str) -> ColumnSpec {
        ColumnSpec {
            distribution: DataDistribution::Zipf(ZipfParameter {
                skewness: 1.1,
                population_size: cardinality,
                seed,
            }),
            shift: None,
            cardinality: None,
            special_rule: aqpbm_datagen::RULE_NONE,
            data_type: data_type.into(),
            string: None,
        }
    }

    fn three_columns(size: u64) -> TableDescription {
        TableDescription {
            column_num: 3,
            column_label: vec!["key1".into(), "key2".into(), "value".into()],
            column_spec: vec![
                label_col(8, "abcd", 1),
                label_col(4, "wxyz", 2),
                value_col(50, 3, "i64"),
            ],
            column_connected: Vec::new(),
            row_num: size,
        }
    }

    #[test]
    fn columns_zip_into_records() {
        let w = build(&three_columns(200)).unwrap();
        assert_eq!(w.items().len(), 200);
        for r in w.items() {
            let labels: Vec<&str> = r.labels().collect();
            assert_eq!(labels.len(), 2, "two label columns => two labels: {r:?}");
            assert_eq!(r.label(0), Some(labels[0]));
            assert_eq!(r.label(1), Some(labels[1]));
            assert_eq!(r.label(2), None);
        }
        assert_eq!(w.description().shape, "columns");
        assert_eq!(w.description().size, 200);
        // The flat descriptor fields cannot hold a column list, so the whole
        // description must ride in the escape hatch or the run cannot be
        // reproduced.
        assert!(w.description().spec.is_some());
    }

    /// Each column's own spec steers it, so two columns given different
    /// alphabets draw from disjoint domains. This is the knob that decides
    /// whether a grouped sketch's key space aliases across columns.
    #[test]
    fn columns_are_independently_steerable() {
        let w = build(&three_columns(300)).unwrap();
        let col0: BTreeSet<&str> = w.items().iter().filter_map(|r| r.label(0)).collect();
        let col1: BTreeSet<&str> = w.items().iter().filter_map(|r| r.label(1)).collect();
        assert!(
            col0.len() <= 8,
            "column 0 respects its cardinality: {col0:?}"
        );
        assert!(
            col1.len() <= 4,
            "column 1 respects its cardinality: {col1:?}"
        );
        assert!(
            col0.is_disjoint(&col1),
            "distinct alphabets must give disjoint domains: {col0:?} vs {col1:?}"
        );
    }

    #[test]
    fn a_value_column_alone_is_refused() {
        let d = TableDescription::single("value", value_col(10, 1, "i64"), 8);
        let err = build(&d).unwrap_err().to_string();
        assert!(err.contains("label column"), "{err}");
    }

    /// The value column's `data_type` is what fixes the row a description can
    /// serve; a mismatch names the column and both types.
    #[test]
    fn a_value_column_of_the_wrong_type_names_the_column() {
        let mut d = three_columns(50);
        d.column_spec[2] = value_col(50, 3, "f64");
        let err = build(&d).unwrap_err().to_string();
        assert!(err.contains("value") && err.contains("f64"), "{err}");
    }

    #[test]
    fn a_label_column_that_is_not_text_names_the_column() {
        let mut d = three_columns(50);
        d.column_spec[0] = value_col(8, 1, "i64");
        let err = build(&d).unwrap_err().to_string();
        assert!(err.contains("key1"), "{err}");
    }

    #[test]
    fn generation_is_reproducible_from_the_column_seeds() {
        let a = build(&three_columns(150)).unwrap();
        let b = build(&three_columns(150)).unwrap();
        assert_eq!(a.items(), b.items());
    }

    /// `column_connected` reaches the record path: two label columns declared
    /// related draw one stream, so their labels co-vary instead of being
    /// independent.
    #[test]
    fn connected_label_columns_co_vary() {
        let mut d = three_columns(300);
        // Same distribution and seed, different alphabets — so the ranks match
        // and only the rendering differs.
        d.column_spec[0] = label_col(8, "abcd", 5);
        d.column_spec[1] = label_col(8, "wxyz", 5);
        d.column_connected = vec![vec!["key1".into(), "key2".into()]];
        let w = build(&d).unwrap();

        let pairs: BTreeSet<(&str, &str)> = w
            .items()
            .iter()
            .map(|r| (r.label(0).unwrap(), r.label(1).unwrap()))
            .collect();
        // One draw stream means one label of column 0 always accompanies one
        // label of column 1: as many pairs as there are ranks, not 8 * 8.
        assert!(pairs.len() <= 8, "columns did not co-vary: {pairs:?}");
    }
}

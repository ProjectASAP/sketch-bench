//! The plain item stream: one column, materialised at the row's item type.
//! No `generate` here on purpose — production goes through
//! [`crate::input_dataset::InputDataSetSpec::build`]; the file-backed one is [`super::load`].

use aqpbm_datagen::{ColumnItem, DataGenError, GeneratedTable, TableDescription};

use super::{InputDataSet, InputDataSetDescription};

/// A numeric dataset: the materialised item stream plus its provenance.
/// Construct from a generated table ([`Self::from_table`]) or a file
/// ([`NumericInputDataSet::load`](super::load)); the source shows up in
/// `description`, not in the type.
#[derive(Debug, Clone)]
pub struct NumericInputDataSet<T> {
    items: Vec<T>,
    description: InputDataSetDescription,
}

/// The key-shaped dataset: every hash-based algorithm (cms, countsketch, hll,
/// elastic, …) ingests these.
pub type I64InputDataSet = NumericInputDataSet<i64>;

/// The float dataset, consumed by the ordered algorithms (kll, dd) whose
/// libraries are `f64`-native.
pub type F64InputDataSet = NumericInputDataSet<f64>;

impl<T: ColumnItem> NumericInputDataSet<T> {
    /// Wrap an already-materialised item stream with its provenance.
    /// `description.size` is forced to `items.len()`: a description disagreeing
    /// with its data would corrupt every throughput denominator downstream.
    pub(crate) fn new(items: Vec<T>, mut description: InputDataSetDescription) -> Self {
        // `load` passes 0 as a placeholder, not knowing the count until it has
        // read the file. Any other value asserts what was produced — otherwise a
        // short generator is silently relabelled into a smaller dataset.
        debug_assert!(
            description.size == 0 || description.size == items.len(),
            "dataset description claims {} items but carries {}",
            description.size,
            items.len(),
        );
        description.size = items.len();
        Self { items, description }
    }

    /// Build from a table someone else already generated. The caller generates
    /// because generating needs an item type and this type is already at one;
    /// `spec` rides along because the record names what the data came from.
    pub fn from_table(
        spec: &TableDescription,
        table: GeneratedTable,
    ) -> Result<Self, DataGenError> {
        if spec.column_spec.len() != 1 {
            return Err(DataGenError::BadParam(format!(
                "this row ingests a plain `{}` stream, so it needs a one-column \
                 description; this one has {} columns",
                T::NAME,
                spec.column_spec.len(),
            )));
        }
        let description = InputDataSetDescription::from_spec(spec);
        let column = table.into_column(0)?;
        Ok(Self::new(T::from_column(column)?, description))
    }
}

impl<T: ColumnItem> InputDataSet for NumericInputDataSet<T> {
    type Item = T;
    fn description(&self) -> InputDataSetDescription {
        self.description.clone()
    }
    fn items(&self) -> &[T] {
        &self.items
    }
    fn into_parts(self) -> (InputDataSetDescription, Vec<T>) {
        (self.description, self.items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{build, uniform_column, zipf_column};

    #[test]
    fn uniform_is_reproducible_from_seed() {
        let spec = TableDescription::single("key", uniform_column(1000, 42, "i64"), 100);
        let a: I64InputDataSet = build(&spec).unwrap();
        let b: I64InputDataSet = build(&spec).unwrap();
        assert_eq!(a.items(), b.items());
    }

    #[test]
    fn uniform_items_in_expected_range() {
        let spec = TableDescription::single("key", uniform_column(100, 7, "i64"), 1000);
        let w: I64InputDataSet = build(&spec).unwrap();
        for v in w.items() {
            assert!((0..100).contains(v), "{v} outside 0..100");
        }
    }

    #[test]
    fn zipf_items_in_expected_range() {
        let spec = TableDescription::single("key", zipf_column(100, 1.1, 7, "i64"), 1000);
        let w: I64InputDataSet = build(&spec).unwrap();
        for v in w.items() {
            assert!(*v >= 1 && *v <= 100);
        }
    }

    /// A single-column row handed a table refuses it by name: zipping the
    /// columns down to one would run the measurement over a stream nobody
    /// asked for.
    #[test]
    fn a_multi_column_description_is_refused_by_a_plain_row() {
        let d = TableDescription {
            column_num: 2,
            column_label: vec!["a".into(), "b".into()],
            column_spec: vec![
                zipf_column(64, 1.0, 1, "i64"),
                zipf_column(64, 1.0, 2, "i64"),
            ],
            column_connected: Vec::new(),
            row_num: 10,
        };
        let err = build::<i64>(&d).unwrap_err().to_string();
        assert!(err.contains("one-column"), "{err}");
    }

    /// The description's `data_type` and the row's item type are two places
    /// naming one thing. Disagreement is an error naming both, never a coercion.
    #[test]
    fn a_data_type_disagreeing_with_the_item_type_is_refused() {
        let d = TableDescription::single("key", zipf_column(64, 1.0, 1, "f64"), 10);
        let err = build::<i64>(&d).unwrap_err().to_string();
        assert!(err.contains("f64") && err.contains("i64"), "{err}");
    }

    #[test]
    fn placeholder_description_size_is_filled_in() {
        // A description that disagrees with the data would silently skew every
        // throughput denominator; `new` is the one place that can catch
        // it, so it always wins over the caller's claim.
        let w = I64InputDataSet::new(
            vec![1, 2, 3],
            InputDataSetDescription {
                shape: "custom".into(),
                size: 0, // placeholder, as `load` passes
                cardinality: None,
                zipf_s: None,
                source_path: None,
                seed: None,
                spec: None,
            },
        );
        assert_eq!(w.description().size, 3);
    }
}

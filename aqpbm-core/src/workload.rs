//! Synthetic workload generators + adapters for file-backed test data.
//! See `docs/DESIGN.md` §4.3.
//!
//! One type per *item* type, not per source: provenance is data, not a type
//! parameter, so it lives in `description` and the source picks a constructor.

use serde::{Deserialize, Serialize};
use std::path::Path;

use aqpbm_datagen::{
    ColumnItem, ColumnSpec, DataDistribution, DataGenError, GeneratedTable, TableDescription,
    UniformParameter, ZipfParameter,
};

use crate::binfile;

/// Human-friendly description of a workload — serialised into
/// every report so a JSONL record can be re-run without
/// external metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkloadDescription {
    pub shape: String, // "uniform" | "zipf" | "normal" | "columns" | "file"
    pub size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zipf_s: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// Full `datagen` description, when the flat fields above cannot express it
    /// (several columns, a shift, a special rule, string options, …). Absent for
    /// a plain single `uniform` / `zipf` column and for `file`, whose flat
    /// fields already round-trip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<serde_json::Value>,
}

impl WorkloadDescription {
    /// Projection of a [`TableDescription`] into the report-facing descriptor.
    /// Lives here, not on the description, because it asks which of *this*
    /// type's fields can hold one — on the description the generator would be
    /// citing the report schema.
    pub fn from_spec(spec: &TableDescription) -> Self {
        let lead = spec.column_spec.first();
        let multi = spec.column_spec.len() > 1;

        let shape = match (multi, lead) {
            (true, _) => "columns".to_string(),
            (false, Some(c)) => c.distribution.tag().to_string(),
            (false, None) => "empty".to_string(),
        };
        let (cardinality, zipf_s) = match lead {
            Some(c) if !multi => (
                c.distribution.domain().map(|d| d.size),
                match &c.distribution {
                    DataDistribution::Zipf(z) => Some(z.skewness),
                    _ => None,
                },
            ),
            _ => (None, None),
        };

        WorkloadDescription {
            shape,
            size: spec.row_num as usize,
            cardinality,
            zipf_s,
            source_path: None,
            seed: lead.map(|c| c.distribution.seed()),
            spec: if fits_legacy_description(spec) {
                None
            } else {
                serde_json::to_value(spec).ok()
            },
        }
    }
}

/// Whether the flat `cardinality` / `zipf_s` fields fully describe `spec`.
/// True only for one plain column drawn uniform or zipf; everything else is
/// lossy there and is the one condition under which
/// [`WorkloadDescription::spec`] is set.
///
/// Anything that changes the values without changing those two fields has to
/// force the escape hatch, or two different workloads would share a group key
/// and anything pooling by workload would average them together.
fn fits_legacy_description(spec: &TableDescription) -> bool {
    let [column] = spec.column_spec.as_slice() else {
        return false;
    };
    column.string.is_none()
        && column.shift.is_none()
        && column.special_rule == aqpbm_datagen::RULE_NONE
        && matches!(
            column.distribution,
            DataDistribution::Uniform(_) | DataDistribution::Zipf(_)
        )
}

/// The abstract contract for a workload a measurement can
/// consume. An implementation produces an ordered `Vec<Item>`
/// plus an optional query stream.
pub trait Workload: Sized {
    type Item: Clone;
    fn description(&self) -> WorkloadDescription;
    fn items(&self) -> &[Self::Item];
}

// ---------- numeric workloads ----------

/// A numeric workload: the materialised item stream plus its provenance.
/// Construct from a generator (`uniform` / `zipf`) or a file (`load`); the
/// source shows up in `description`, not in the type.
#[derive(Debug, Clone)]
pub struct NumericWorkload<T> {
    items: Vec<T>,
    description: WorkloadDescription,
}

/// The key-shaped workload: every hash-based algorithm (cms, countsketch, hll,
/// elastic, …) ingests these, and the `String`/`Bytes` views derive from it.
pub type I64Workload = NumericWorkload<i64>;

/// The float workload, consumed by the ordered algorithms (kll, dd) whose
/// libraries are `f64`-native.
pub type F64Workload = NumericWorkload<f64>;

impl<T: ColumnItem> NumericWorkload<T> {
    /// Wrap an already-materialised item stream with its provenance.
    /// `description.size` is forced to `items.len()`: a description disagreeing
    /// with its data would corrupt every throughput denominator downstream.
    pub fn new(items: Vec<T>, mut description: WorkloadDescription) -> Self {
        // `load` passes 0 as a placeholder, not knowing the count until it has
        // read the file. Any other value asserts what was produced — otherwise a
        // short generator is silently relabelled into a smaller workload.
        debug_assert!(
            description.size == 0 || description.size == items.len(),
            "workload description claims {} items but carries {}",
            description.size,
            items.len(),
        );
        description.size = items.len();
        Self { items, description }
    }

    /// Generate in-process from a [`TableDescription`] — the one generator in
    /// the tool. A single-column description is what a row ingesting a plain
    /// stream wants; a column list belongs to [`LabeledWorkload`].
    pub fn generate(spec: &TableDescription) -> Result<Self, DataGenError> {
        let table = spec.generate()?;
        Self::from_table(spec, table)
    }

    /// Build from a table someone else already generated.
    ///
    /// The split exists because *who* generates matters: a frontend asks the
    /// registry what item type a row wants, generates once at that type, and
    /// hands the columns over. `spec` still rides along because the record
    /// names the description the data came from, which the columns alone do
    /// not carry.
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
        let description = WorkloadDescription::from_spec(spec);
        let column = table.into_column(0)?;
        Ok(Self::new(T::from_column(column)?, description))
    }

    /// Uniform in `[0, cardinality)`. Convenience over [`Self::generate`].
    pub fn uniform(size: usize, cardinality: u64, seed: u64) -> Self {
        Self::generate(&TableDescription::single(
            "key",
            ColumnSpec {
                distribution: DataDistribution::Uniform(UniformParameter {
                    lower_bound: 0.0,
                    upper_bound: cardinality as f64,
                    seed,
                }),
                shift: None,
                cardinality: None,
                special_rule: aqpbm_datagen::RULE_NONE,
                data_type: T::NAME.to_string(),
                string: None,
            },
            size as u64,
        ))
        .expect("uniform keys over a non-zero cardinality always generate")
    }

    /// Zipfian with `s`-parameter (skew exponent) over ranks
    /// `[1, cardinality]`. Convenience over [`Self::generate`].
    pub fn zipf(size: usize, cardinality: u64, s: f64, seed: u64) -> Result<Self, DataGenError> {
        Self::generate(&TableDescription::single(
            "key",
            ColumnSpec {
                distribution: DataDistribution::Zipf(ZipfParameter {
                    skewness: s,
                    population_size: cardinality,
                    seed,
                }),
                shift: None,
                cardinality: None,
                special_rule: aqpbm_datagen::RULE_NONE,
                data_type: T::NAME.to_string(),
                string: None,
            },
            size as u64,
        ))
    }
}

impl NumericWorkload<i64> {
    /// Load from a file, format from the extension: `.bin` (and anything else)
    /// is a little-endian `int64` stream; `.pcap` takes each IPv4 source address
    /// as big-endian `u32`; `.csv` parses column 0 below a header row.
    pub fn load(path: &Path) -> Result<Self, DataGenError> {
        let items = match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("pcap") => load_pcap(path)?,
            Some("csv") => load_csv(path)?,
            _ => {
                reject_non_i64_bin(path)?;
                load_bin(path)?
            }
        };
        if items.is_empty() {
            return Err(DataGenError::BadParam(format!(
                "file contained zero items: {}",
                path.display()
            )));
        }
        Ok(Self::new(
            items,
            WorkloadDescription {
                shape: "file".into(),
                size: 0, // overwritten by `new`
                cardinality: None,
                zipf_s: None,
                source_path: Some(path.display().to_string()),
                seed: None,
                spec: None,
            },
        ))
    }
}

impl<T: ColumnItem> Workload for NumericWorkload<T> {
    type Item = T;
    fn description(&self) -> WorkloadDescription {
        self.description.clone()
    }
    fn items(&self) -> &[T] {
        &self.items
    }
}

/// Reject a `.bin` whose sidecar declares an unreadable dtype. The stream is
/// header-less, so an `f64` file is byte-indistinguishable from `i64` — `0.093`
/// reads back as `4591388162153532928`. No sidecar means "assume i64".
fn reject_non_i64_bin(path: &Path) -> Result<(), DataGenError> {
    let Ok(Some(meta)) = binfile::read_meta(path) else {
        return Ok(());
    };
    if meta.dtype != "i64" {
        return Err(DataGenError::BadParam(format!(
            "{}: sidecar declares dtype {}, but the benchmark only consumes i64. \
             Re-generate with `--dtype i64`; reading it as i64 would silently \
             reinterpret the raw bytes and produce meaningless keys.",
            path.display(),
            meta.dtype,
        )));
    }
    Ok(())
}

fn load_bin(path: &Path) -> Result<Vec<i64>, DataGenError> {
    let bytes = std::fs::read(path).map_err(DataGenError::Io)?;
    if bytes.len() % 8 != 0 {
        return Err(DataGenError::BadParam(format!(
            "{}: size {} not a multiple of 8",
            path.display(),
            bytes.len()
        )));
    }
    Ok(bytes
        .chunks_exact(8)
        .map(|c| i64::from_le_bytes(c.try_into().unwrap()))
        .collect())
}

fn load_csv(path: &Path) -> Result<Vec<i64>, DataGenError> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).map_err(DataGenError::Io)?;
    let mut items = Vec::new();
    for (idx, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(DataGenError::Io)?;
        if idx == 0 {
            // Header row.
            continue;
        }
        let field = line.split(',').next().unwrap_or("").trim();
        if field.is_empty() {
            continue;
        }
        let v: i64 = field.parse().map_err(|e| {
            DataGenError::BadParam(format!(
                "{}: bad i64 on line {}: {e}",
                path.display(),
                idx + 1
            ))
        })?;
        items.push(v);
    }
    Ok(items)
}

fn load_pcap(path: &Path) -> Result<Vec<i64>, DataGenError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(DataGenError::Io)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).map_err(DataGenError::Io)?;
    if buf.len() < 24 {
        return Err(DataGenError::BadParam(format!(
            "{}: pcap smaller than header",
            path.display()
        )));
    }
    let big_endian = match &buf[..4] {
        [0xd4, 0xc3, 0xb2, 0xa1] | [0x4d, 0x3c, 0xb2, 0xa1] => false,
        [0xa1, 0xb2, 0xc3, 0xd4] | [0xa1, 0xb2, 0x3c, 0x4d] => true,
        _ => {
            return Err(DataGenError::BadParam(format!(
                "{}: unsupported pcap magic",
                path.display()
            )))
        }
    };
    let read_u32 = |b: &[u8]| {
        let a = [b[0], b[1], b[2], b[3]];
        if big_endian {
            u32::from_be_bytes(a)
        } else {
            u32::from_le_bytes(a)
        }
    };
    let linktype = read_u32(&buf[20..24]);
    let mut off = 24usize;
    let mut items = Vec::new();
    while off + 16 <= buf.len() {
        let incl_len = read_u32(&buf[off + 8..off + 12]) as usize;
        off += 16;
        if off + incl_len > buf.len() {
            return Err(DataGenError::BadParam(format!(
                "{}: truncated pcap record",
                path.display()
            )));
        }
        let pkt = &buf[off..off + incl_len];
        off += incl_len;
        if let Some(src) = extract_ipv4_src(pkt, linktype) {
            items.push(i64::from(src));
        }
    }
    Ok(items)
}

fn extract_ipv4_src(packet: &[u8], linktype: u32) -> Option<u32> {
    let ip = match linktype {
        1 => {
            // Ethernet: 14-byte header, require ethertype = 0x0800.
            if packet.len() < 34 || packet[12] != 0x08 || packet[13] != 0x00 {
                return None;
            }
            &packet[14..]
        }
        101 => packet,
        _ => return None,
    };
    if ip.len() < 20 || (ip[0] >> 4) != 4 {
        return None;
    }
    Some(u32::from_be_bytes([ip[12], ip[13], ip[14], ip[15]]))
}

// ---------- multi-column (labelled record) workloads ----------

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
    /// The label columns, in the order they were generated.
    pub fn labels(&self) -> std::str::Split<'_, char> {
        self.key.split(';')
    }

    /// The label in column `i`, or `None` past the last column.
    pub fn label(&self, i: usize) -> Option<&str> {
        self.labels().nth(i)
    }
}

/// A multi-column workload: `n - 1` label columns followed by one value column,
/// all from one [`TableDescription`].
///
/// No new generator: the description already covers a table, and this type only
/// zips its columns into records. Which means a column's distribution, skew and
/// seed are all independently steerable, using the vocabulary that already
/// exists — including `column_connected`, so two label columns can be made to
/// co-vary.
#[derive(Debug, Clone)]
pub struct LabeledWorkload<V> {
    items: Vec<Labeled<V>>,
    description: WorkloadDescription,
}

impl<V: ColumnItem> LabeledWorkload<V> {
    /// Zip a description's columns into records: all but the last are label
    /// columns and must be `data_type: string`, the last is the value column and
    /// must be this row's item type.
    pub fn generate(spec: &TableDescription) -> Result<Self, DataGenError> {
        let table = spec.generate()?;
        Self::from_table(spec, table)
    }

    /// Build from an already-generated table. See
    /// [`NumericWorkload::from_table`] for why the split exists.
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
        let description = WorkloadDescription::from_spec(spec);
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

impl<V: ColumnItem> Workload for LabeledWorkload<V> {
    type Item = Labeled<V>;

    fn description(&self) -> WorkloadDescription {
        self.description.clone()
    }

    fn items(&self) -> &[Labeled<V>] {
        &self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aqpbm_datagen::{StringOpts, RULE_MONOTONIC_INCREASE};

    fn zipf_column(cardinality: u64, s: f64, seed: u64, data_type: &str) -> ColumnSpec {
        ColumnSpec {
            distribution: DataDistribution::Zipf(ZipfParameter {
                skewness: s,
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

    /// Key length is the dominant cost on a hashing insert path, so two runs
    /// at different lengths are two measurements. They must not share a
    /// descriptor, or anything grouping by workload averages them together.
    #[test]
    fn string_options_reach_the_descriptor() {
        let base = zipf_column(64, 1.0, 1, "string");
        let with_opts = |min_len, max_len| {
            let mut c = base.clone();
            c.string = Some(StringOpts {
                alphabet: "ab".to_string(),
                min_len,
                max_len,
            });
            TableDescription::single("key", c, 32)
        };

        // A plain numeric column keeps the descriptor a record written before
        // the string axis existed would have had.
        let plain = WorkloadDescription::from_spec(&TableDescription::single(
            "key",
            zipf_column(64, 1.0, 1, "i64"),
            32,
        ));
        assert!(plain.spec.is_none(), "{plain:?}");

        let short = WorkloadDescription::from_spec(&with_opts(4, 4));
        let long = WorkloadDescription::from_spec(&with_opts(16, 16));
        assert!(short.spec.is_some());
        assert_ne!(
            serde_json::to_string(&short).unwrap(),
            serde_json::to_string(&long).unwrap(),
            "two key lengths must not share a group key"
        );
    }

    /// Anything that moves the values without moving `cardinality`/`zipf_s` has
    /// to force the full description into the record, or two different
    /// workloads pool as one.
    #[test]
    fn a_shift_or_a_rule_forces_the_full_description() {
        let mut shifted = zipf_column(64, 1.0, 1, "i64");
        shifted.shift = Some(15_000.0);
        let d = WorkloadDescription::from_spec(&TableDescription::single("key", shifted, 32));
        assert!(d.spec.is_some());

        let mut ruled = zipf_column(64, 1.0, 1, "i64");
        ruled.special_rule = RULE_MONOTONIC_INCREASE;
        let d = WorkloadDescription::from_spec(&TableDescription::single("key", ruled, 32));
        assert!(d.spec.is_some());
    }

    #[test]
    fn uniform_is_reproducible_from_seed() {
        let a = I64Workload::uniform(100, 1000, 42);
        let b = I64Workload::uniform(100, 1000, 42);
        assert_eq!(a.items(), b.items());
    }

    #[test]
    fn uniform_items_in_expected_range() {
        let w = I64Workload::uniform(1000, 100, 7);
        for v in w.items() {
            assert!((0..100).contains(v), "{v} outside 0..100");
        }
    }

    #[test]
    fn zipf_items_in_expected_range() {
        let w = I64Workload::zipf(1000, 100, 1.1, 7).unwrap();
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
        let err = I64Workload::generate(&d).unwrap_err().to_string();
        assert!(err.contains("one-column"), "{err}");
    }

    /// The description's `data_type` and the row's item type are two places
    /// naming one thing. Disagreement is an error naming both, never a coercion.
    #[test]
    fn a_data_type_disagreeing_with_the_item_type_is_refused() {
        let d = TableDescription::single("key", zipf_column(64, 1.0, 1, "f64"), 10);
        let err = I64Workload::generate(&d).unwrap_err().to_string();
        assert!(err.contains("f64") && err.contains("i64"), "{err}");
    }

    #[test]
    fn placeholder_description_size_is_filled_in() {
        // A description that disagrees with the data would silently skew every
        // throughput denominator; `new` is the one place that can catch
        // it, so it always wins over the caller's claim.
        let w = I64Workload::new(
            vec![1, 2, 3],
            WorkloadDescription {
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

    #[test]
    fn file_bin_roundtrip() {
        use std::io::Write;
        let dir = std::env::temp_dir();
        let path = dir.join("sketchlib_bin_roundtrip.bin");
        let mut f = std::fs::File::create(&path).unwrap();
        for v in [1i64, -2, 3, 4] {
            f.write_all(&v.to_le_bytes()).unwrap();
        }
        drop(f);
        let w = I64Workload::load(&path).unwrap();
        assert_eq!(w.items(), &[1, -2, 3, 4]);
        assert_eq!(w.description().shape, "file");
        assert_eq!(w.description().size, 4);
        std::fs::remove_file(&path).ok();
    }

    /// Write a `.bin` + sidecar at `data_type` and try to load it as i64.
    fn load_generated(data_type: &str, tag: &str) -> Result<I64Workload, DataGenError> {
        let path = std::env::temp_dir().join(format!("sketchlib_dtype_guard_{tag}.bin"));
        let spec = TableDescription::single("key", zipf_column(64, 1.0, 1, data_type), 32);
        let column = spec.generate().unwrap().into_column(0).unwrap();
        binfile::write_bin(&path, &column).unwrap();
        binfile::write_meta(&path, &binfile::BinMeta::new(&spec, &column)).unwrap();
        let out = I64Workload::load(&path);
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(binfile::sidecar_path(&path)).ok();
        out
    }

    #[test]
    fn bin_with_i64_sidecar_loads() {
        let w = load_generated("i64", "i64").expect("i64 must load");
        assert_eq!(w.items().len(), 32);
    }

    #[test]
    fn bin_with_non_i64_sidecar_is_rejected() {
        // An f64/u64 stream is byte-indistinguishable from i64, so
        // loading it would silently produce garbage keys rather than
        // fail. The sidecar is the only thing that can catch it.
        for tag in ["f64", "u64"] {
            let err = load_generated(tag, tag)
                .expect_err("a non-i64 stream must be rejected, not silently misread");
            let msg = err.to_string();
            assert!(
                msg.contains(tag),
                "error should name the offending item type: {msg}"
            );
        }
    }

    #[test]
    fn bin_without_sidecar_is_assumed_i64() {
        // Legacy `input/benchmark_data_*.bin` files have no sidecar and
        // must keep loading unchanged.
        use std::io::Write;
        let path = std::env::temp_dir().join("sketchlib_no_sidecar.bin");
        let mut f = std::fs::File::create(&path).unwrap();
        for v in [7i64, 8, 9] {
            f.write_all(&v.to_le_bytes()).unwrap();
        }
        drop(f);
        let w = I64Workload::load(&path).expect("no sidecar => assume i64");
        assert_eq!(w.items(), &[7, 8, 9]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn unreadable_sidecar_does_not_break_a_loadable_bin() {
        // A foreign `.meta.json`, or one from a future schema, must not
        // fail a file that loaded fine before the guard existed.
        use std::io::Write;
        let path = std::env::temp_dir().join("sketchlib_bad_sidecar.bin");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&5i64.to_le_bytes()).unwrap();
        drop(f);
        std::fs::write(binfile::sidecar_path(&path), "{\"not\":\"ours\"}").unwrap();
        let w = I64Workload::load(&path).expect("unparseable sidecar => fall back to i64");
        assert_eq!(w.items(), &[5]);
        std::fs::remove_file(&path).ok();
        std::fs::remove_file(binfile::sidecar_path(&path)).ok();
    }

    /// `workload generate` (to a file) and `sketchbench --spec` (in memory) must
    /// be the same workload, or a run cannot be reproduced from its own file.
    #[test]
    fn the_file_and_the_memory_path_agree() {
        let spec = TableDescription::single("key", zipf_column(500, 1.3, 7, "i64"), 3000);
        let path = std::env::temp_dir().join("sketchlib_file_agreement.bin");

        let column = spec.generate().unwrap().into_column(0).unwrap();
        binfile::write_bin(&path, &column).unwrap();

        let from_file = I64Workload::load(&path).unwrap();
        let from_memory = I64Workload::generate(&spec).unwrap();
        assert_eq!(from_file.items(), from_memory.items());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_string_column_cannot_be_written_to_a_bin() {
        let spec = TableDescription::single("key", zipf_column(64, 1.0, 1, "string"), 8);
        let column = spec.generate().unwrap().into_column(0).unwrap();
        let path = std::env::temp_dir().join("sketchlib_string_bin.bin");
        let err = binfile::write_bin(&path, &column).unwrap_err().to_string();
        assert!(err.contains("length field"), "{err}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn file_csv_skips_header_and_empty() {
        use std::io::Write;
        let path = std::env::temp_dir().join("sketchlib_csv_roundtrip.csv");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "key,value").unwrap();
        writeln!(f, "10,first").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "-5,second").unwrap();
        drop(f);
        let w = I64Workload::load(&path).unwrap();
        assert_eq!(w.items(), &[10, -5]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn file_pcap_rejects_empty_or_bad_magic() {
        use std::io::Write;
        let path = std::env::temp_dir().join("sketchlib_pcap_bad.pcap");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&[0u8; 32]).unwrap();
        drop(f);
        let err = I64Workload::load(&path).unwrap_err();
        assert!(err.to_string().contains("pcap magic"));
        std::fs::remove_file(&path).ok();
    }
}

#[cfg(test)]
mod labeled_tests {
    use super::*;
    use aqpbm_datagen::{StringOpts, UniformParameter};
    use std::collections::BTreeSet;

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
        let w = LabeledWorkload::<i64>::generate(&three_columns(200)).unwrap();
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
        let w = LabeledWorkload::<i64>::generate(&three_columns(300)).unwrap();
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
        let err = LabeledWorkload::<i64>::generate(&d)
            .unwrap_err()
            .to_string();
        assert!(err.contains("label column"), "{err}");
    }

    /// The value column's `data_type` is what fixes the row a description can
    /// serve; a mismatch names the column and both types.
    #[test]
    fn a_value_column_of_the_wrong_type_names_the_column() {
        let mut d = three_columns(50);
        d.column_spec[2] = value_col(50, 3, "f64");
        let err = LabeledWorkload::<i64>::generate(&d)
            .unwrap_err()
            .to_string();
        assert!(err.contains("value") && err.contains("f64"), "{err}");
    }

    #[test]
    fn a_label_column_that_is_not_text_names_the_column() {
        let mut d = three_columns(50);
        d.column_spec[0] = value_col(8, 1, "i64");
        let err = LabeledWorkload::<i64>::generate(&d)
            .unwrap_err()
            .to_string();
        assert!(err.contains("key1"), "{err}");
    }

    #[test]
    fn generation_is_reproducible_from_the_column_seeds() {
        let a = LabeledWorkload::<i64>::generate(&three_columns(150)).unwrap();
        let b = LabeledWorkload::<i64>::generate(&three_columns(150)).unwrap();
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
        let w = LabeledWorkload::<i64>::generate(&d).unwrap();

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

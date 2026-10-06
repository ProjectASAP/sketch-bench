//! Scoring a sketch against an exact answer: the [`GroundTruth`] trait and the
//! comparators that implement it. A comparator knows the *statistic* and which
//! generated columns it is taken over, not how to query a sketch — that is the
//! row's own closure, driven by `measurement`.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::rc::Rc;

use aqpbm_datagen::{ColumnData, ColumnItem, DataGenError, GeneratedTable};

use crate::error::RunError;

pub mod aggregate;
pub mod cardinality;
pub(crate) mod curve;
pub mod frequency;
pub mod heavy_hitter;
pub mod keyed;
pub mod quantile;
pub mod subpopulation;
pub mod topk;

/// The key an exact counter tallies a value under.
///
/// Counting needs `Eq + Hash` to tally and `Ord` to break ranking ties
/// deterministically, and `f64` has none of the three. Rather than shut the
/// float width out of the counting comparators, each width names the key it is
/// counted by: itself, for the three that can be one, and the bit pattern for
/// `f64` — the same choice [`distinct`] makes one screen down, and the same one
/// the frequency wrappers make when they feed a library that hashes.
///
/// The consequence is worth stating: keyed by bits, `0.0` and `-0.0` are two
/// keys and `NaN` equals itself. Both sides of the comparison have to agree on
/// that or the reported error is noise, which is why this lives next to the
/// scorers rather than in either one.
pub trait CountedValue: ColumnItem {
    type CountKey: Eq + std::hash::Hash + Ord + Clone;

    fn count_key(&self) -> Self::CountKey;
}

impl CountedValue for i64 {
    type CountKey = i64;
    #[inline(always)]
    fn count_key(&self) -> i64 {
        *self
    }
}

impl CountedValue for u64 {
    type CountKey = u64;
    #[inline(always)]
    fn count_key(&self) -> u64 {
        *self
    }
}

impl CountedValue for f64 {
    type CountKey = u64;
    #[inline(always)]
    fn count_key(&self) -> u64 {
        self.to_bits()
    }
}

impl CountedValue for String {
    type CountKey = String;
    #[inline(always)]
    fn count_key(&self) -> String {
        self.clone()
    }
}

/// The exact answer a sketch is scored against. Taken over the table
/// `aqpbm-datagen` produced, not over whatever a row materialised out of it:
/// the truth is a property of the data, and a row's item type is not.
pub trait GroundTruth {
    /// The exact answer, plus whatever [`probes`](Self::probes) and
    /// [`score`](Self::score) need to read off it.
    type Truth;
    /// One question for the sketch.
    type Probe;
    /// One answer from the sketch.
    type Answer;

    /// ① The exact answer, over the columns this comparator names. An error
    /// means the description and the comparator disagree about what the table
    /// holds, and it is raised before anything is measured.
    fn truth(&self, table: &GeneratedTable) -> Result<Self::Truth, DataGenError>;

    /// ② What to ask, in the order the answers come back. An empty set means
    /// there is nothing to ask, and the runner measures no queries.
    fn probes(&self, truth: &Self::Truth) -> Vec<Self::Probe>;

    /// ③ The error metrics. A key must state the population it is over
    /// (`are_top10` vs `are_all`), since the literature publishes both under
    /// the same word.
    fn score(
        &self,
        truth: &Self::Truth,
        probes: &[Self::Probe],
        answers: &[Self::Answer],
    ) -> BTreeMap<String, f64>;
}

/// How a row's answers turn into the named error metrics: the scorer
/// [`questions`] drew, boxed so a measurement carries it without naming a
/// comparator type.
pub type Score<A> = Rc<dyn Fn(&[A]) -> BTreeMap<String, f64>>;

/// Draw the questions: the exact answer over the generated table, the probes
/// off it, and a scorer holding both. Run once per row, outside every clock,
/// so no comparator type reaches a measurement.
#[allow(clippy::type_complexity)]
pub fn questions<G>(
    gt: G,
    table: &GeneratedTable,
) -> Result<(Rc<Vec<G::Probe>>, Score<G::Answer>), RunError>
where
    G: GroundTruth + 'static,
    G::Truth: 'static,
    G::Probe: 'static,
    G::Answer: 'static,
{
    let gt = Rc::new(gt);
    let truth = Rc::new(
        gt.truth(table)
            .map_err(|e| RunError::InputDataSet(e.into()))?,
    );
    let probes = Rc::new(gt.probes(&truth));
    let score: Score<G::Answer> = {
        let (gt, truth, probes) = (gt.clone(), truth.clone(), probes.clone());
        Rc::new(move |answers: &[G::Answer]| gt.score(&truth, &probes, answers))
    };
    Ok((probes, score))
}

pub(crate) fn scalar_error(truth: f64, answers: &[f64]) -> BTreeMap<String, f64> {
    let est = answers.first().copied().unwrap_or(0.0);
    let abs_err = (est - truth).abs();
    let rel_err = if truth > 0.0 { abs_err / truth } else { 0.0 };
    BTreeMap::from([
        ("truth".to_string(), truth),
        ("estimate".to_string(), est),
        ("absolute_error".to_string(), abs_err),
        ("relative_error".to_string(), rel_err),
    ])
}

pub(crate) fn f64_values(column: &ColumnData) -> Result<Vec<f64>, DataGenError> {
    match column {
        ColumnData::Int64(v) => Ok(v.iter().map(|&x| x as f64).collect()),
        ColumnData::Unsigned64(v) => Ok(v.iter().map(|&x| x as f64).collect()),
        ColumnData::Float64(v) => Ok(v.clone()),
        other => Err(DataGenError::TypeMismatch {
            held: other.kind(),
            wanted: "a numeric column",
        }),
    }
}

pub(crate) fn distinct(column: &ColumnData) -> usize {
    match column {
        ColumnData::Int64(v) => v.iter().collect::<HashSet<_>>().len(),
        ColumnData::Unsigned64(v) => v.iter().collect::<HashSet<_>>().len(),
        ColumnData::String(v) => v.iter().collect::<HashSet<_>>().len(),
        ColumnData::Float64(v) => v.iter().map(|f| f.to_bits()).collect::<HashSet<_>>().len(),
    }
}

/// Drive one comparator end to end — truth, probes, query, score — and hand back
/// the metric map. Test-only: production runs the same sequence inside
/// `measurement::query_measurement`, where it sits within the region `measure` times.
#[cfg(test)]
pub(crate) fn score_with<S, G, A>(
    gt: &G,
    query: &A,
    sketch: &mut S,
    table: &GeneratedTable,
) -> BTreeMap<String, f64>
where
    G: GroundTruth,
    A: Fn(&mut S, &G::Probe) -> G::Answer,
{
    let truth = gt
        .truth(table)
        .expect("the test's table matches its comparator");
    let probes = gt.probes(&truth);
    let answers: Vec<G::Answer> = probes.iter().map(|p| query(sketch, p)).collect();
    gt.score(&truth, &probes, &answers)
}

#[cfg(test)]
pub(crate) fn table_of(titles: &[&str], data: Vec<ColumnData>) -> GeneratedTable {
    GeneratedTable {
        column_num: data.len() as u32,
        column_title: titles.iter().map(|t| (*t).to_string()).collect(),
        row_num: data.first().map(|c| c.len()).unwrap_or(0) as u64,
        data,
    }
}

use std::collections::BTreeMap;

use aqpbm_datagen::{ColumnData, DataGenError, GeneratedTable};

use super::GroundTruth;

pub struct Windowed<G> {
    pub ground_truth: G,
    pub time_column: usize,
    pub group_columns: Vec<usize>,
    pub width: i64,
    pub ends: Vec<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowScore {
    pub end: i64,
    pub group: Vec<String>,
    pub scores: Option<BTreeMap<String, f64>>,
}

impl<G: GroundTruth> Windowed<G> {
    pub fn score<F>(
        &self,
        table: &GeneratedTable,
        mut answer: F,
    ) -> Result<Vec<WindowScore>, DataGenError>
    where
        F: FnMut(i64, &[String], &[G::Probe]) -> Option<Vec<G::Answer>>,
    {
        let times = table.column(self.time_column)?.as_i64()?;
        let labels = self
            .group_columns
            .iter()
            .map(|&column| table.column(column)?.as_string())
            .collect::<Result<Vec<_>, DataGenError>>()?;
        let mut groups: BTreeMap<Vec<&str>, Vec<usize>> = BTreeMap::new();
        for row in 0..times.len() {
            let group = labels.iter().map(|column| column[row].as_str()).collect();
            groups.entry(group).or_default().push(row);
        }
        let mut out = Vec::new();
        for (group, mut rows) in groups {
            rows.sort_by_key(|&row| times[row]);
            let group: Vec<String> = group.into_iter().map(str::to_string).collect();
            for &end in &self.ends {
                let from = rows.partition_point(|&row| times[row] <= end - self.width);
                let to = rows.partition_point(|&row| times[row] <= end);
                if from >= to {
                    continue;
                }
                let window = rows_of(table, &rows[from..to]);
                let truth = self.ground_truth.truth(&window)?;
                let probes = self.ground_truth.probes(&truth);
                let scores = answer(end, &group, &probes)
                    .map(|answers| self.ground_truth.score(&truth, &probes, &answers));
                out.push(WindowScore {
                    end,
                    group: group.clone(),
                    scores,
                });
            }
        }
        Ok(out)
    }
}

fn rows_of(table: &GeneratedTable, rows: &[usize]) -> GeneratedTable {
    GeneratedTable {
        column_num: table.column_num,
        column_title: table.column_title.clone(),
        row_num: rows.len() as u64,
        data: table
            .data
            .iter()
            .map(|column| gather(column, rows))
            .collect(),
    }
}

fn gather(column: &ColumnData, rows: &[usize]) -> ColumnData {
    match column {
        ColumnData::Int64(v) => ColumnData::Int64(rows.iter().map(|&r| v[r]).collect()),
        ColumnData::Unsigned64(v) => ColumnData::Unsigned64(rows.iter().map(|&r| v[r]).collect()),
        ColumnData::Float64(v) => ColumnData::Float64(rows.iter().map(|&r| v[r]).collect()),
        ColumnData::String(v) => ColumnData::String(rows.iter().map(|&r| v[r].clone()).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accuracy::cardinality::CardinalityGT;
    use crate::accuracy::table_of;

    fn table() -> GeneratedTable {
        table_of(
            &["time", "series", "value"],
            vec![
                ColumnData::Int64(vec![1, 2, 3, 4, 1, 2, 3, 4]),
                ColumnData::String(
                    ["a", "a", "a", "a", "b", "b", "b", "b"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                ),
                ColumnData::Int64(vec![7, 7, 8, 9, 5, 6, 7, 8]),
            ],
        )
    }

    fn windowed(group_columns: Vec<usize>) -> Windowed<CardinalityGT> {
        Windowed {
            ground_truth: CardinalityGT { column: 2 },
            time_column: 0,
            group_columns,
            width: 2,
            ends: vec![2, 4, 6],
        }
    }

    #[test]
    fn each_window_holds_the_rows_after_its_start_up_to_its_end() {
        let cells = windowed(vec![1])
            .score(&table(), |_, _, probes| Some(vec![0.0; probes.len()]))
            .unwrap();
        let truths: Vec<(i64, &str, f64)> = cells
            .iter()
            .map(|c| {
                (
                    c.end,
                    c.group[0].as_str(),
                    c.scores.as_ref().unwrap()["truth"],
                )
            })
            .collect();
        assert_eq!(
            truths,
            vec![(2, "a", 1.0), (4, "a", 2.0), (2, "b", 2.0), (4, "b", 2.0)]
        );
    }

    #[test]
    fn no_group_columns_puts_every_row_of_a_window_in_one_cell() {
        let cells = windowed(vec![])
            .score(&table(), |_, group, probes| {
                assert!(group.is_empty());
                Some(vec![2.5; probes.len()])
            })
            .unwrap();
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].scores.as_ref().unwrap()["truth"], 3.0);
        assert_eq!(cells[1].scores.as_ref().unwrap()["truth"], 3.0);
        assert_eq!(cells[1].scores.as_ref().unwrap()["estimate"], 2.5);
    }

    #[test]
    fn a_cell_without_an_answer_is_listed_unscored() {
        let cells = windowed(vec![1])
            .score(&table(), |end, _, probes| {
                (end == 4).then(|| vec![2.0; probes.len()])
            })
            .unwrap();
        assert!(cells
            .iter()
            .filter(|c| c.end == 2)
            .all(|c| c.scores.is_none()));
        assert!(cells
            .iter()
            .filter(|c| c.end == 4)
            .all(|c| c.scores.is_some()));
    }
}

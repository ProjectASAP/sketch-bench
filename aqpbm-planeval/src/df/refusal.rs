use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RefusalReason {
    PromqlOnly,
    TimeAxis,
    NoConstructor,
    Deferred { issue: String },
}

impl RefusalReason {
    pub const TAGS: [&'static str; 4] = ["promql_only", "time_axis", "no_constructor", "deferred"];

    pub fn tag(&self) -> &'static str {
        match self {
            RefusalReason::PromqlOnly => "promql_only",
            RefusalReason::TimeAxis => "time_axis",
            RefusalReason::NoConstructor => "no_constructor",
            RefusalReason::Deferred { .. } => "deferred",
        }
    }
}

impl fmt::Display for RefusalReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RefusalReason::Deferred { issue } => write!(f, "deferred({issue})"),
            other => f.write_str(other.tag()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Refusal {
    pub variant: String,
    pub reason: RefusalReason,
    pub detail: String,
}

impl Refusal {
    pub fn promql_only(variant: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            variant: variant.into(),
            reason: RefusalReason::PromqlOnly,
            detail: detail.into(),
        }
    }

    pub fn time_axis(variant: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            variant: variant.into(),
            reason: RefusalReason::TimeAxis,
            detail: detail.into(),
        }
    }

    pub fn no_constructor(variant: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            variant: variant.into(),
            reason: RefusalReason::NoConstructor,
            detail: detail.into(),
        }
    }

    pub fn deferred(
        variant: impl Into<String>,
        issue: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            variant: variant.into(),
            reason: RefusalReason::Deferred {
                issue: issue.into(),
            },
            detail: detail.into(),
        }
    }

    pub fn tag(&self) -> &'static str {
        self.reason.tag()
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} refused [{}]: {}",
            self.variant, self.reason, self.detail
        )
    }
}

impl std::error::Error for Refusal {}

impl From<Refusal> for datafusion::error::DataFusionError {
    fn from(refusal: Refusal) -> Self {
        datafusion::error::DataFusionError::External(Box::new(refusal))
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RefusalCounts {
    pub promql_only: usize,
    pub time_axis: usize,
    pub no_constructor: usize,
    pub deferred: usize,
    #[serde(default)]
    pub unclassified: usize,
}

impl RefusalCounts {
    pub fn add(&mut self, refusal: &Refusal) {
        match refusal.reason {
            RefusalReason::PromqlOnly => self.promql_only += 1,
            RefusalReason::TimeAxis => self.time_axis += 1,
            RefusalReason::NoConstructor => self.no_constructor += 1,
            RefusalReason::Deferred { .. } => self.deferred += 1,
        }
    }

    pub fn add_unclassified(&mut self, count: usize) {
        self.unclassified += count;
    }

    pub fn total(&self) -> usize {
        self.promql_only + self.time_axis + self.no_constructor + self.deferred + self.unclassified
    }
}

impl FromIterator<Refusal> for RefusalCounts {
    fn from_iter<I: IntoIterator<Item = Refusal>>(iter: I) -> Self {
        let mut counts = RefusalCounts::default();
        for refusal in iter {
            counts.add(&refusal);
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_has_a_distinct_tag() {
        let reasons = [
            RefusalReason::PromqlOnly,
            RefusalReason::TimeAxis,
            RefusalReason::NoConstructor,
            RefusalReason::Deferred {
                issue: "order-sensitive".into(),
            },
        ];
        let tags: Vec<&str> = reasons.iter().map(RefusalReason::tag).collect();
        assert_eq!(tags, RefusalReason::TAGS);
    }

    #[test]
    fn display_names_the_variant_the_reason_and_the_detail() {
        let refusal = Refusal::deferred(
            "AggIntent::Rate",
            "order-sensitive",
            "needs an ordered UDAF",
        );
        assert_eq!(
            refusal.to_string(),
            "AggIntent::Rate refused [deferred(order-sensitive)]: needs an ordered UDAF"
        );
        assert_eq!(refusal.tag(), "deferred");
    }

    #[test]
    fn counts_split_by_reason() {
        let counts: RefusalCounts = [
            Refusal::promql_only("AggIntent::TopK", "SQL top-k is Sort + Limit"),
            Refusal::time_axis(
                "QueryExpr::TimeRange",
                "the DAG carries no evaluation instant",
            ),
            Refusal::time_axis(
                "QueryExpr::TimeShift",
                "the DAG carries no evaluation instant",
            ),
            Refusal::no_constructor("SketchQuery::PointCount", "no producer-side key input edge"),
            Refusal::deferred(
                "QueryExpr::Dedup",
                "corpus",
                "no corpus query reaches DistinctOn",
            ),
        ]
        .into_iter()
        .collect();
        assert_eq!(counts.promql_only, 1);
        assert_eq!(counts.time_axis, 2);
        assert_eq!(counts.no_constructor, 1);
        assert_eq!(counts.deferred, 1);
        assert_eq!(counts.unclassified, 0);
        assert_eq!(counts.total(), 5);
    }

    #[test]
    fn an_interpreter_refusal_is_counted_without_being_given_a_class() {
        let mut counts = RefusalCounts::default();
        counts.add(&Refusal::time_axis("QueryExpr::TimeRange", "no instant"));
        counts.add_unclassified(3);
        assert_eq!(counts.time_axis, 1);
        assert_eq!(counts.unclassified, 3);
        assert_eq!(counts.total(), 4);
        assert_eq!(
            serde_json::to_string(&counts).unwrap(),
            "{\"promql_only\":0,\"time_axis\":1,\"no_constructor\":0,\"deferred\":0,\
             \"unclassified\":3}"
        );
    }
}

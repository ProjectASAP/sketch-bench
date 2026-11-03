use super::*;
use crate::wrappers::cms_heap::sketchlib::TopkAnswer;
use crate::wrappers::coco::CocoLib;
use crate::wrappers::count_min_fold::CountMinFoldLib;
use crate::wrappers::count_min_hll::CountMinHllLib;
use crate::wrappers::count_min_space_saving::CountMinSpaceSavingLib;
use crate::wrappers::count_sketch_fold::CountSketchFoldLib;
use crate::wrappers::count_sketch_l2_norm::CountSketchL2NormLib;
use crate::wrappers::count_sketch_space_saving::CountSketchSpaceSavingLib;
use crate::wrappers::elastic::ElasticLib;
use crate::wrappers::hll::sketchlib::HllLibErtl;
use crate::wrappers::kll::sketchlib::KllDynamicLib;
use crate::wrappers::kmv::KmvLib;
use crate::wrappers::row_bodies as rb;
use crate::wrappers::space_saving::SpaceSavingLib;
use crate::wrappers::uniform_sampling::UniformSamplingLib;
use crate::wrappers::univmon_q::UnivMonQLib;
use crate::wrappers::univmon_two_tier::{self as two_tier, UnivMonTwoTierLib};

type Row = Result<Measurements, RunError>;

macro_rules! folds {
    ($s:ty, $i:ty, $p:ty, $a:ty) => {
        Some((
            rb::merge::<$s, $i>,
            rb::merge_step::<$s, $i>,
            rb::merge_query::<$s, $i, $p, $a>,
        ))
    };
}

macro_rules! cardinality {
    ($req:ident, $description:ident, $table:ident, $want:ident, $s:ty, $t:ty) => {{
        cardinality_row::<$t>(
            $req,
            $description,
            $table,
            $want,
            rb::insert::<$s, $t>,
            rb::insert_step::<$s, $t>,
            rb::query::<$s, $t, (), f64>,
            folds!($s, $t, (), f64),
            None,
        )
    }};
}

macro_rules! frequency {
    ($req:ident, $description:ident, $table:ident, $want:ident, $s:ty, $t:ty) => {{
        frequency_row::<$t>(
            $req,
            $description,
            $table,
            $want,
            rb::insert::<$s, $t>,
            rb::insert_step::<$s, $t>,
            rb::query::<$s, $t, $t, u64>,
            folds!($s, $t, $t, u64),
            None,
        )
    }};
}

macro_rules! topk {
    ($req:ident, $description:ident, $table:ident, $want:ident, $s:ty, $t:ty) => {{
        topk_row::<$t>(
            $req,
            $description,
            $table,
            $want,
            rb::insert::<$s, $t>,
            rb::insert_step::<$s, $t>,
            rb::query::<$s, $t, (), TopkAnswer<$t>>,
            folds!($s, $t, (), TopkAnswer<$t>),
        )
    }};
}

macro_rules! quantile {
    ($req:ident, $description:ident, $table:ident, $want:ident, $s:ty, $t:ty) => {{
        quantile_row::<$t>(
            $req,
            $description,
            $table,
            $want,
            rb::insert::<$s, $t>,
            rb::insert_step::<$s, $t>,
            rb::query::<$s, $t, f64, f64>,
            folds!($s, $t, f64, f64),
            None,
        )
    }};
}

macro_rules! keyed {
    ($req:ident, $description:ident, $table:ident, $want:ident, $gt:ident, $s:ty, $k:ty) => {{
        keyed_row::<$k, _>(
            $req,
            $description,
            $table,
            $want,
            $gt::<$k>::over_columns(KEYED_KEY_COLUMN, value_column($description)),
            rb::insert::<$s, ($k, i64)>,
            rb::insert_step::<$s, ($k, i64)>,
            rb::query::<$s, ($k, i64), (), f64>,
            folds!($s, ($k, i64), (), f64),
            None,
        )
    }};
}

macro_rules! every_width {
    ($req:ident, $at:ident) => {
        match $req.width {
            Dtype::I64 => $at!(i64),
            Dtype::U64 => $at!(u64),
            Dtype::F64 => $at!(f64),
            Dtype::Str => $at!(String),
        }
    };
}

macro_rules! numeric_widths {
    ($req:ident, $at:ident) => {
        match $req.width {
            Dtype::I64 => $at!(i64),
            Dtype::U64 => $at!(u64),
            Dtype::F64 => $at!(f64),
            other => Err(no_build_at($req, other)),
        }
    };
}

fn string_only(req: &Requirement) -> RunError {
    RunError::Sketch(format!(
        "{}/{} keys by String in the library's own API, so it builds at --dtype string only; \
         --dtype {} is not it",
        req.variant,
        req.library,
        req.width.name(),
    ))
}

pub(crate) fn row_hll_ertl_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Row {
    use asap_sketchlib::{HllRegistersP12, HllRegistersP14, HllRegistersP16};
    macro_rules! at_precision {
        ($t:ty) => {
            match lg_k(req)? {
                12 => cardinality!(
                    req,
                    description,
                    table,
                    want,
                    HllLibErtl<HllRegistersP12>,
                    $t
                ),
                14 => cardinality!(
                    req,
                    description,
                    table,
                    want,
                    HllLibErtl<HllRegistersP14>,
                    $t
                ),
                16 => cardinality!(
                    req,
                    description,
                    table,
                    want,
                    HllLibErtl<HllRegistersP16>,
                    $t
                ),
                other => Err(unsupported_precision(other)),
            }
        };
    }
    every_width!(req, at_precision)
}

pub(crate) fn row_kmv_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Row {
    macro_rules! at {
        ($t:ty) => {
            cardinality!(req, description, table, want, KmvLib, $t)
        };
    }
    every_width!(req, at)
}

pub(crate) fn row_kll_dynamic_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Row {
    macro_rules! at {
        ($t:ty) => {
            quantile!(req, description, table, want, KllDynamicLib<$t>, $t)
        };
    }
    numeric_widths!(req, at)
}

pub(crate) fn row_uniform_sampling_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Row {
    macro_rules! at {
        ($t:ty) => {
            quantile!(req, description, table, want, UniformSamplingLib, $t)
        };
    }
    numeric_widths!(req, at)
}

macro_rules! frequency_and_topk_rows {
    ($frequency:ident, $topk:ident, $s:ty) => {
        pub(crate) fn $frequency(
            req: &Requirement,
            description: &TableDescription,
            table: GeneratedTable,
            want: &[(Operation, Metric)],
        ) -> Row {
            macro_rules! at {
                ($t:ty) => {
                    frequency!(req, description, table, want, $s, $t)
                };
            }
            every_width!(req, at)
        }

        pub(crate) fn $topk(
            req: &Requirement,
            description: &TableDescription,
            table: GeneratedTable,
            want: &[(Operation, Metric)],
        ) -> Row {
            macro_rules! at {
                ($t:ty) => {
                    topk!(req, description, table, want, $s, $t)
                };
            }
            every_width!(req, at)
        }
    };
}

frequency_and_topk_rows!(
    row_space_saving_lib,
    row_space_saving_topk_lib,
    SpaceSavingLib
);
frequency_and_topk_rows!(
    row_count_min_space_saving_lib,
    row_count_min_space_saving_topk_lib,
    CountMinSpaceSavingLib
);
frequency_and_topk_rows!(
    row_count_sketch_space_saving_lib,
    row_count_sketch_space_saving_topk_lib,
    CountSketchSpaceSavingLib
);
frequency_and_topk_rows!(
    row_count_min_fold_lib,
    row_count_min_fold_topk_lib,
    CountMinFoldLib
);
frequency_and_topk_rows!(
    row_count_sketch_fold_lib,
    row_count_sketch_fold_topk_lib,
    CountSketchFoldLib
);

pub(crate) fn row_elastic_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Row {
    match req.width {
        Dtype::Str => frequency!(req, description, table, want, ElasticLib, String),
        _ => Err(string_only(req)),
    }
}

pub(crate) fn row_elastic_topk_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Row {
    match req.width {
        Dtype::Str => topk!(req, description, table, want, ElasticLib, String),
        _ => Err(string_only(req)),
    }
}

pub(crate) fn row_coco_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Row {
    match req.width {
        Dtype::Str => frequency!(req, description, table, want, CocoLib, String),
        _ => Err(string_only(req)),
    }
}

pub(crate) fn row_count_sketch_l2_norm_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Row {
    macro_rules! at {
        ($k:ty) => {
            keyed!(
                req,
                description,
                table,
                want,
                KeyedL2NormGT,
                CountSketchL2NormLib,
                $k
            )
        };
    }
    every_width!(req, at)
}

pub(crate) fn row_count_min_hll_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Row {
    macro_rules! at {
        ($v:ty) => {{
            subpop_cardinality_row::<$v>(
                req,
                description,
                table,
                want,
                rb::insert::<CountMinHllLib, (String, $v)>,
                rb::insert_step::<CountMinHllLib, (String, $v)>,
                rb::query::<CountMinHllLib, (String, $v), GroupKey, f64>,
                folds!(CountMinHllLib, (String, $v), GroupKey, f64),
                None,
            )
        }};
    }
    every_width!(req, at)
}

macro_rules! two_tier_row {
    ($name:ident, $gt:ident, $statistic:ty) => {
        pub(crate) fn $name(
            req: &Requirement,
            description: &TableDescription,
            table: GeneratedTable,
            want: &[(Operation, Metric)],
        ) -> Row {
            macro_rules! at {
                ($k:ty) => {
                    keyed!(
                        req,
                        description,
                        table,
                        want,
                        $gt,
                        UnivMonTwoTierLib<$statistic>,
                        $k
                    )
                };
            }
            every_width!(req, at)
        }
    };
}

two_tier_row!(
    row_univmon_two_tier_cardinality_lib,
    KeyedCardinalityGT,
    two_tier::Cardinality
);
two_tier_row!(
    row_univmon_two_tier_l1_norm_lib,
    KeyedL1NormGT,
    two_tier::L1Norm
);
two_tier_row!(
    row_univmon_two_tier_l2_norm_lib,
    KeyedL2NormGT,
    two_tier::L2Norm
);
two_tier_row!(
    row_univmon_two_tier_entropy_lib,
    KeyedEntropyGT,
    two_tier::Entropy
);

pub(crate) fn row_univmon_q_quantile_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Row {
    macro_rules! at {
        ($t:ty) => {
            quantile!(req, description, table, want, UnivMonQLib, $t)
        };
    }
    numeric_widths!(req, at)
}

pub(crate) fn row_univmon_q_cardinality_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Row {
    macro_rules! at {
        ($t:ty) => {
            cardinality!(req, description, table, want, UnivMonQLib, $t)
        };
    }
    numeric_widths!(req, at)
}

pub(crate) fn row_univmon_q_frequency_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Row {
    macro_rules! at {
        ($t:ty) => {
            frequency!(req, description, table, want, UnivMonQLib, $t)
        };
    }
    numeric_widths!(req, at)
}

pub(crate) fn row_univmon_q_topk_lib(
    req: &Requirement,
    description: &TableDescription,
    table: GeneratedTable,
    want: &[(Operation, Metric)],
) -> Row {
    macro_rules! at {
        ($t:ty) => {
            topk!(req, description, table, want, UnivMonQLib, $t)
        };
    }
    numeric_widths!(req, at)
}

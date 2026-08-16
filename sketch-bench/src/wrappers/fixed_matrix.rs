//! The compile-time matrix shapes the `*-fastpath-fixedmatrix` rows offer.
//!
//! `asap_sketchlib::impl_fixed_matrix!` takes its dimensions as **literals** and
//! bakes them into a `Box<[i32; ROWS * COLS]>`, which is the whole point of the
//! storage: the compiler folds `row * COLS + col` and the bounds check away, and
//! pricing that against the runtime-sized `Vector2D` is what these rows exist
//! for. A shape is therefore a monomorphisation, not a value, and one that was
//! never compiled cannot be built at run time.
//!
//! So the set is written down here and dispatched over at run time, the same way
//! `hll`'s `lg_k` selects a register-storage type. Instantiating a shape is
//! close to free: a trial grid of 36 measured at no detectable compile time and
//! about 26 KiB of rlib each, which is why the table below can afford 52 and
//! still leave a caller having to try to fall outside it.
//!
//! # Adding a shape
//!
//! One line in [`fixed_matrix_shapes!`], and it needs a distinct type name.
//! Nothing else changes: the dispatch, the shape list the error message prints,
//! and the tests all read off this table.
//!
//! # Why the grid has a corner missing
//!
//! The widest shape is capped at 327,690 counters, which is what the row already
//! carried before it became sweepable. Past that, `Box::new([0i32; N])`
//! materialises the array on the stack before moving it to the heap in unoptimised
//! builds, and `cargo test` runs each test on a 2 MiB thread. Wide-and-deep is the
//! corner that trips it, so `cols = 65536` stops at 5 rows.

use std::rc::Rc;

use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::cell::{BenchItem, RunError, WorkloadData};
use aqpbm_core::config::ParamSet;
use aqpbm_core::request::Requirement;
use asap_sketchlib::{impl_fixed_matrix, DefaultXxHasher, FastPathHasher, MatrixStorage};

use crate::ops::{Body, SketchOps};
use crate::registry::GroundTruthCalculator;
use aqpbm_core::workload::WorkloadDescription;

///
/// A trait and not a closure because the shape is a **type**: the visitor is the
/// only way to hand a monomorphisation back to a caller that picked it with a
/// pair of runtime integers.
pub trait FixedMatrixVisitor {
    type Out;
    fn visit<M>(self) -> Self::Out
    where
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static;
}

macro_rules! fixed_matrix_shapes {
    ($( $name:ident => ($rows:literal, $cols:literal) ),* $(,)?) => {
        $( impl_fixed_matrix!($name, i32, $rows, $cols); )*

        /// Every shape compiled in, ascending. Read by the error message and by
        /// the tests, so neither can drift from what the dispatch offers.
        pub const FIXED_SHAPES: &[(usize, usize)] = &[ $( ($rows, $cols) ),* ];

        /// Hand `v` the storage type baked at `(rows, cols)`, or `None` when no
        /// such shape was compiled in.
        pub fn with_fixed_matrix<V: FixedMatrixVisitor>(
            rows: usize,
            cols: usize,
            v: V,
        ) -> Option<V::Out> {
            match (rows, cols) {
                $( ($rows, $cols) => Some(v.visit::<$name>()), )*
                _ => None,
            }
        }
    };
}

fixed_matrix_shapes!(
    M3x256 => (3, 256),
    M3x512 => (3, 512),
    M3x1024 => (3, 1024),
    M3x2048 => (3, 2048),
    M3x4096 => (3, 4096),
    M3x8192 => (3, 8192),
    M3x16384 => (3, 16384),
    M3x32768 => (3, 32768),
    M4x256 => (4, 256),
    M4x512 => (4, 512),
    M4x1024 => (4, 1024),
    M4x2048 => (4, 2048),
    M4x4096 => (4, 4096),
    M4x8192 => (4, 8192),
    M4x16384 => (4, 16384),
    M4x32768 => (4, 32768),
    M5x256 => (5, 256),
    M5x512 => (5, 512),
    M5x1024 => (5, 1024),
    M5x2048 => (5, 2048),
    M5x4096 => (5, 4096),
    M5x8192 => (5, 8192),
    M5x16384 => (5, 16384),
    M5x32768 => (5, 32768),
    M6x256 => (6, 256),
    M6x512 => (6, 512),
    M6x1024 => (6, 1024),
    M6x2048 => (6, 2048),
    M6x4096 => (6, 4096),
    M6x8192 => (6, 8192),
    M6x16384 => (6, 16384),
    M6x32768 => (6, 32768),
    M7x256 => (7, 256),
    M7x512 => (7, 512),
    M7x1024 => (7, 1024),
    M7x2048 => (7, 2048),
    M7x4096 => (7, 4096),
    M7x8192 => (7, 8192),
    M7x16384 => (7, 16384),
    M7x32768 => (7, 32768),
    M8x256 => (8, 256),
    M8x512 => (8, 512),
    M8x1024 => (8, 1024),
    M8x2048 => (8, 2048),
    M8x4096 => (8, 4096),
    M8x8192 => (8, 8192),
    M8x16384 => (8, 16384),
    M8x32768 => (8, 32768),
    M3x65536 => (3, 65536),
    M4x65536 => (4, 65536),
    M5x65536 => (5, 65536),
    M5x65538 => (5, 65538),
);

/// What to tell a caller who named a shape that was not compiled in.
///
/// Not "this shape is unsupported", which would read as a limit of the tool.
/// The shape is a type, so measuring a new one means adding source and
/// rebuilding, and the message is where that is spelled out: which file, which
/// line, what the macro generates, and what the alternative is if the caller
/// would rather not rebuild. A reader who has to reverse-engineer that from a
/// one-line refusal has been handed a puzzle instead of an answer.
pub fn unsupported_shape(what: &str, rows: usize, cols: usize) -> String {
    let widest = FIXED_SHAPES.iter().map(|s| s.0 * s.1).max().unwrap_or(0);
    let rows_offered: Vec<String> = {
        let mut v: Vec<usize> = FIXED_SHAPES.iter().map(|s| s.0).collect();
        v.sort_unstable();
        v.dedup();
        v.iter().map(|r| r.to_string()).collect()
    };
    let cols_here: Vec<String> = FIXED_SHAPES
        .iter()
        .filter(|s| s.0 == rows)
        .map(|s| s.1.to_string())
        .collect();
    let compiled_in = if cols_here.is_empty() {
        format!(
            "no shape at rows={rows} at all; the rows compiled in are {}",
            rows_offered.join(", ")
        )
    } else {
        format!(
            "at rows={rows} the cols compiled in are {}",
            cols_here.join(", ")
        )
    };
    format!(
        "{what} has no {rows}x{cols} matrix compiled in.\n\
         \n\
         This is not a shape the row rejects. Its storage carries the dimensions \
         in its *type*, which is what lets the compiler fold the index arithmetic \
         away, and pricing that is the whole reason this row exists. A shape \
         therefore has to exist in source before it can be measured, so adding one \
         means editing this benchmark and rebuilding it.\n\
         \n\
         To measure {rows}x{cols}, add one line to `fixed_matrix_shapes!` in \
         sketch-bench/src/wrappers/fixed_matrix.rs:\n\
         \n    \
         M{rows}x{cols} => ({rows}, {cols}),\n\
         \n\
         `asap_sketchlib::impl_fixed_matrix!` expands that into the storage struct \
         and its `Default`, `MatrixStorage` and `FastPathHasher` impls, which is \
         everything this row needs; the runtime dispatch and this message both read \
         the same table, so nothing else has to change. Then `cargo build \
         --release -p aqpbm-cli`.\n\
         \n\
         Compiled in now: {compiled_in}, and no shape exceeds {widest} counters \
         (wide-and-deep is capped so an unoptimised build does not build the array \
         on the stack).\n\
         \n\
         If you would rather not rebuild, the `*-fastpath-vector2d` row runs the \
         same FastPath hashing over runtime-sized storage and takes any dimensions."
    )
}

// ---------- how a fixed-matrix row runs ----------

/// The shape-independent half of a fixed-matrix row.
///
/// The one place a closure cannot be written at the row: the sketch type is a
/// GAT, so there is no single type to write one against. A generic method is
/// the stand-in, and the bodies still live in the wrapper file.
/// `'static` throughout, because a body outlives the call that built it: the row
/// hands its closures back to a frontend, and a boxed closure owns what it
/// captured. A compiled-in shape has no borrows anyway.
pub trait FixedMatrixRow: 'static {
    const ALGORITHM: &'static str;
    type At<
        M: MatrixStorage<Counter = i32> + FastPathHasher<DefaultXxHasher> + Default + Clone + 'static,
    >: 'static;
    fn shape(params: &ParamSet) -> Result<(usize, usize), RunError>;
    fn insert<M>(sketch: &mut Self::At<M>, v: &i64)
    where
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static;
    fn ops<M>() -> SketchOps<Self::At<M>, i64, i64, u64>
    where
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static;
}

/// Turn the requested `(rows, cols)` back into the monomorphisation that bakes
/// it in, then run every square against that.
pub fn run_fixed_matrix<W: FixedMatrixRow>(
    req: &Requirement,
    data: WorkloadData,
) -> Result<(WorkloadDescription, Vec<Body>), RunError> {
    let (rows, cols) = W::shape(&req.params)?;
    let wk = Rc::new(<i64 as BenchItem>::materialise(data)?);
    let gt = <FrequencyGT as GroundTruthCalculator<i64>>::build(&req.params);
    let visitor = RunFixedMatrix::<W> {
        req,
        wk,
        gt,
        _row: std::marker::PhantomData,
    };
    with_fixed_matrix(rows, cols, visitor)
        .unwrap_or_else(|| Err(RunError::Body(unsupported_shape(W::ALGORITHM, rows, cols))))
}

/// Carries the run's arguments into the monomorphisation the shape selected.
struct RunFixedMatrix<'a, W> {
    req: &'a Requirement,
    wk: Rc<aqpbm_core::workload::NumericWorkload<i64>>,
    gt: FrequencyGT,
    _row: std::marker::PhantomData<W>,
}

impl<W: FixedMatrixRow> FixedMatrixVisitor for RunFixedMatrix<'_, W> {
    type Out = Result<(WorkloadDescription, Vec<Body>), RunError>;
    fn visit<M>(self) -> Self::Out
    where
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static,
    {
        crate::ops::squares_for::<_, W::At<M>, i64, FrequencyGT, _>(
            self.req,
            self.wk,
            self.gt,
            W::insert::<M>,
            W::ops::<M>(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dispatch offers exactly the shapes the table lists. Two lists that
    /// could drift silently: a shape in the table but not the match would be a
    /// row nobody can select, and the error message would name it as available.
    #[test]
    fn every_listed_shape_dispatches() {
        struct Dims;
        impl FixedMatrixVisitor for Dims {
            type Out = (usize, usize);
            fn visit<M>(self) -> (usize, usize)
            where
                M: MatrixStorage<Counter = i32>
                    + FastPathHasher<DefaultXxHasher>
                    + Default
                    + Clone
                    + 'static,
            {
                let m = M::default();
                (m.rows(), m.cols())
            }
        }
        for &(rows, cols) in FIXED_SHAPES {
            let got = with_fixed_matrix(rows, cols, Dims)
                .unwrap_or_else(|| panic!("{rows}x{cols} is listed but does not dispatch"));
            assert_eq!(
                got,
                (rows, cols),
                "the type baked at {rows}x{cols} disagrees"
            );
        }
    }

    #[test]
    fn an_uncompiled_shape_dispatches_to_nothing() {
        struct Never;
        impl FixedMatrixVisitor for Never {
            type Out = ();
            fn visit<M>(self)
            where
                M: MatrixStorage<Counter = i32>
                    + FastPathHasher<DefaultXxHasher>
                    + Default
                    + Clone
                    + 'static,
            {
                panic!("no shape should have matched");
            }
        }
        assert!(with_fixed_matrix(5, 3000, Never).is_none());
        assert!(with_fixed_matrix(9, 2048, Never).is_none());
    }

    /// The shape names are unique and the table is free of duplicates, which the
    /// macro cannot check: two entries with one name is a compile error, but two
    /// names at one shape would make the second arm unreachable.
    #[test]
    fn no_shape_is_listed_twice() {
        let mut seen = std::collections::BTreeSet::new();
        for &s in FIXED_SHAPES {
            assert!(seen.insert(s), "{s:?} is listed twice");
        }
    }

    /// The refusal has to be actionable, because acting on it means writing
    /// source and rebuilding. Each assertion below is one thing a reader would
    /// otherwise have to go and find out: the line to add, the file to add it
    /// to, what that line generates, how to rebuild, and the way out that needs
    /// no rebuild at all.
    #[test]
    fn the_refusal_is_a_recipe_and_not_just_a_refusal() {
        let msg = unsupported_shape("cms-fastpath-fixedmatrix", 5, 3000);
        for expected in [
            "5x3000",                                    // what was asked for
            "M5x3000 => (5, 3000),",                     // the line to add
            "sketch-bench/src/wrappers/fixed_matrix.rs", // where it goes
            "impl_fixed_matrix!",                        // what expands it
            "MatrixStorage",                             // what that gives you
            "FastPathHasher",
            "cargo build", // how to pick it up
            "vector2d",    // the no-rebuild route
        ] {
            assert!(msg.contains(expected), "missing {expected:?} from:\n{msg}");
        }
        // It must not read as "the tool does not support this".
        assert!(msg.contains("not a shape the row rejects"), "{msg}");
    }
}

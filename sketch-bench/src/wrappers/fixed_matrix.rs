//! The compile-time matrix shapes the `*-fastpath-fixedmatrix` registrations
//! offer, and the dispatch that turns two runtime integers back into one.
//! `impl_fixed_matrix!` takes literal dimensions, so a shape is a
//! monomorphisation and one never compiled cannot be built at run time. Adding
//! one is a line in [`fixed_matrix_shapes!`] with a distinct type name.

use asap_sketchlib::{impl_fixed_matrix, DefaultXxHasher, FastPathHasher, MatrixStorage};

use aqpbm_core::accumulator::Accumulator;
use aqpbm_core::accuracy::frequency::FrequencyGT;
use aqpbm_core::accuracy::{FrequencyOps, GroundTruthName};
use aqpbm_core::cell::{RunError, WorkloadSpec};
use aqpbm_core::init::{BenchImpl, BuildError, InitSketch};
use aqpbm_core::memory_footprint::MemoryFootprint;
use aqpbm_core::registry::{run_scored, Numeric, Registration};
use aqpbm_core::runner::{BenchConfig, BenchReport};

use crate::params::{ParamSet, SketchParams};

/// Receives the storage type a `(rows, cols)` pair selects.
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

// The corner is missing on purpose: `Box::new([0i32; N])` materialises the array
// on the stack before moving it to the heap in unoptimised builds, and `cargo
// test` runs each test on a 2 MiB thread. Wide-and-deep is what trips that, so
// `cols = 65536` stops at 5 rows and the widest shape caps at 327,690 counters.
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
            assert_eq!(got, (rows, cols), "the type baked at {rows}x{cols} disagrees");
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
            "5x3000",                                        // what was asked for
            "M5x3000 => (5, 3000),",                         // the line to add
            "sketch-bench/src/wrappers/fixed_matrix.rs",     // where it goes
            "impl_fixed_matrix!",                            // what expands it
            "MatrixStorage",                                 // what that gives you
            "FastPathHasher",
            "cargo build",                                   // how to pick it up
            "vector2d",                                      // the no-rebuild route
        ] {
            assert!(msg.contains(expected), "missing {expected:?} from:\n{msg}");
        }
        // It must not read as "the tool does not support this".
        assert!(msg.contains("not a shape the row rejects"), "{msg}");
    }
}

// ---------- the registrations these shapes serve ----------

/// A registration whose `(rows, cols)` selects a *type* rather than sizing a field:
/// `impl_fixed_matrix!` bakes the dimensions in, which is what the row exists to
/// price, so this dispatch turns two runtime integers back into one
/// monomorphisation. Same shape as `hll`'s `lg_k` dispatch, and the same reason.
fn run_fixed_matrix<W: FixedMatrixRegistration>(
    cfg: &BenchConfig,
    spec: &WorkloadSpec,
    params: &ParamSet,
    width: Numeric,
) -> Result<Vec<BenchReport>, RunError> {
    let (rows, cols) = W::shape(params)?;
    let visitor = RunFixedMatrix::<W> {
        cfg,
        spec,
        params,
        width,
        _row: std::marker::PhantomData,
    };
    with_fixed_matrix(rows, cols, visitor).unwrap_or_else(|| {
        Err(RunError::Build(BuildError(unsupported_shape(
            W::ALGORITHM,
            rows,
            cols,
        ))))
    })
}

/// The half of a fixed-matrix registration independent of the storage type.
/// Implemented in the wrapper that owns the sketch.
pub trait FixedMatrixRegistration {
    const ALGORITHM: &'static str;
    /// The row's concrete type at storage `M`, which is what actually runs.
    type At<
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static,
    >: Accumulator<Item = i64>
        + InitSketch
        + BenchImpl
        + MemoryFootprint
        + FrequencyOps<Key = i64>;
    fn shape(params: &ParamSet) -> Result<(usize, usize), RunError>;
}

/// Carries the run's arguments into the monomorphisation the shape selected.
struct RunFixedMatrix<'a, W> {
    cfg: &'a BenchConfig,
    spec: &'a WorkloadSpec,
    params: &'a ParamSet,
    width: Numeric,
    _row: std::marker::PhantomData<W>,
}

impl<W: FixedMatrixRegistration> FixedMatrixVisitor for RunFixedMatrix<'_, W> {
    type Out = Result<Vec<BenchReport>, RunError>;

    fn visit<M>(self) -> Self::Out
    where
        M: MatrixStorage<Counter = i32>
            + FastPathHasher<DefaultXxHasher>
            + Default
            + Clone
            + 'static,
    {
        run_scored::<W::At<M>, FrequencyGT>(self.cfg, self.spec, self.params, self.width)
    }
}

/// The shape-dispatching counterpart of `scored`: no one storage type names a
/// row that exists at every shape, so identity comes off `W` and `P`.
pub const fn fixed_matrix_row<W: FixedMatrixRegistration, P: SketchParams>(
    description: &'static str,
) -> Registration {
    Registration {
        family: P::FAMILY,
        algorithm: W::ALGORITHM,
        impl_name: "lib",
        description,
        // Nameable without a storage type, which is why `NAME` is its own
        // trait: the dispatch scores every shape with `FrequencyGT`.
        ground_truth: Some(FrequencyGT::NAME),
        picks_width: false,
        takes_columns: false,
        run: run_fixed_matrix::<W>,
    }
}

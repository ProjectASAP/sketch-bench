//! How one sketch is driven, as functions.
//!
//! Six of them, written in the wrapper file that owns the sketch, in that
//! sketch's own terms. Nothing here forces two rows to agree on anything: the
//! `datasketches` Count-Min takes an owned `i64`, the `asap_sketchlib` one
//! takes a `&DataInput`, and each says so in its own `insert`.

use crate::build_error::BuildError;
use aqpbm_core::config::ParamSet;

/// Everything a measurement does *to* a sketch.
///
/// - `S` the sketch, `I` what it ingests.
/// - `P` one question, `A` one answer — both fixed by the row's comparator,
///   because a comparison only means anything if both sides answer the same
///   question.
///
/// `insert` is deliberately **not** here: it travels as a generic so it stays a
/// zero-sized `fn` item and inlines. Routing it through a pointer field cost
/// the fixed-matrix row 17% (340M → 280M items/sec) when it was measured.
/// Everything below runs once per measurement, or once per probe, so a pointer
/// is free there.
pub struct SketchOps<S, I, P, A> {
    /// Build a fresh one. Called per run, outside the timed region: every
    /// repeat of an insert has to start from empty or it is not measuring the
    /// same thing twice.
    ///
    /// Takes the worker count as well as the params, because the parallel rows
    /// need it and it is a *run* knob rather than a sketch parameter. Every
    /// other row ignores it.
    pub build: fn(&ParamSet, usize) -> Result<S, BuildError>,
    /// The footprint the sketch claims, read before the body drops it.
    pub memory: fn(&S) -> usize,
    /// Put one question to it. `&mut` because several libraries need it —
    /// `sketch_oxide`'s KLL sorts lazily on the first query.
    pub ask: fn(&mut S, &P) -> A,
    /// Absorb another built from the same `ParamSet`. `None` when the library
    /// has no merge — the honest answer, not a zero.
    pub merge: Option<fn(&mut S, &S)>,
    /// No more items are coming. Timed on its own, so a library that defers
    /// its build is not credited with a fast insert loop.
    pub prepare: Option<fn(&mut S)>,
    /// `I` appears only in `insert`, which lives outside this struct.
    pub _item: std::marker::PhantomData<fn(&I)>,
}

impl<S, I, P, A> SketchOps<S, I, P, A> {
    /// Run `prepare` if this row has one.
    #[inline(always)]
    pub fn run_prepare(&self, sketch: &mut S) {
        if let Some(prepare) = self.prepare {
            prepare(sketch);
        }
    }
}

impl<S, I, P, A> Clone for SketchOps<S, I, P, A> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<S, I, P, A> Copy for SketchOps<S, I, P, A> {}

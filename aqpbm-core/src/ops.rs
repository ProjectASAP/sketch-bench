//! [`SketchOps`] — how *one* sketch is driven, stated as functions.
//!
//! This is what replaced the `Accumulator` trait on the benchmark path. The
//! trait declared `update(&mut self, &Self::Item)` once, for everyone, so every
//! implementation had to be fed one item at a time through one borrow shape,
//! whatever its own API looked like. These are plain function pointers supplied
//! per row, so two rows need not agree on anything.
//!
//! Nothing here is called through `dyn`. The functions are baked into the
//! monomorphised runner the row's `fn` pointer names, so the insert loop
//! inlines exactly as it did when `update` was a trait method — see the
//! throughput checks in `docs/`.
//!
//! `Accumulator` still exists, for `sketch-runtime`'s embedded sampler: an app
//! wrapping its own sketch in a `Probe` implements it once and gets sampling.
//! That is a different product surface from benchmarking a library someone else
//! wrote, and only the latter needed the freedom.

/// Everything the runner does *to* a sketch, per row.
///
/// - `S` the sketch, `I` what it ingests.
/// - `P` one question, `A` one answer — both fixed by the row's comparator,
///   because a comparison only means something if both sides answer the same
///   question.
///
/// `merge` and `prepare` are `Option` because they are real operations a
/// library may simply not have; `None` is the honest answer and the record
/// carries `merge_supported: false` rather than a zero.
/// Note what is *not* here: `insert`. It travels as a separate generic
/// parameter, because a `fn` pointer in a struct field is an **indirect call**,
/// and the insert loop runs it a million times. Measured: routing the
/// fixed-matrix row's insert through a pointer cost 17% (340M -> 280M items/s).
/// As a generic it is a zero-sized `fn` item, monomorphised and inlined, and
/// the number comes back. Everything below is called once per run, or once per
/// probe, so a pointer is free there.
pub struct SketchOps<S, I, P, A> {
    /// Absorb another sketch built from the same `ParamSet`.
    pub merge: Option<fn(&mut S, &S)>,
    /// No more items are coming. Timed on its own clock, so a library that
    /// defers its build is not credited with a fast insert loop.
    pub prepare: Option<fn(&mut S)>,
    /// Put one question to the sketch. `&mut` because several libraries need
    /// it — `sketch_oxide`'s KLL sorts lazily — and nothing is gained by
    /// forcing them all through `&self`.
    pub ask: fn(&mut S, &P) -> A,
    /// `I` appears only in `insert`, which now lives outside this struct.
    pub _item: std::marker::PhantomData<fn(&I)>,
}

impl<S, I, P, A> SketchOps<S, I, P, A> {
    /// Run `prepare` if this row has one. The runner times the call either
    /// way, so a row without one reports a finalize cost of ~0 rather than a
    /// missing field.
    #[inline(always)]
    pub fn run_prepare(&self, sketch: &mut S) {
        if let Some(prepare) = self.prepare {
            prepare(sketch);
        }
    }
}

// `SketchOps` is a plain data struct of `fn` pointers, so it is `Copy` whatever
// `S`/`I`/`P`/`A` are; the derive would wrongly demand it of them.
impl<S, I, P, A> Clone for SketchOps<S, I, P, A> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<S, I, P, A> Copy for SketchOps<S, I, P, A> {}

//! What an ordered (quantile) sketch can ingest.
//!
//! Its own module because both `kll` and `dd` need it and neither owns it: the
//! constraint is "a value this benchmark can put in rank order", which is a
//! property of the item type, not of either algorithm. It lived under `kll`
//! and `dd` borrowed it from there, which made one algorithm depend on the
//! other for a definition that belongs to neither.

/// Widen to the type rank-error analysis is done in. Deliberately wider than
/// [`QuantileValue`]: a width the generator does not render can still be
/// converted, and the narrow types cost nothing to carry here.
pub trait ToF64 {
    fn to_f64(self) -> f64;
}

impl ToF64 for f64 {
    fn to_f64(self) -> f64 {
        self
    }
}
impl ToF64 for f32 {
    fn to_f64(self) -> f64 {
        self as f64
    }
}
impl ToF64 for i64 {
    fn to_f64(self) -> f64 {
        self as f64
    }
}
impl ToF64 for i32 {
    fn to_f64(self) -> f64 {
        self as f64
    }
}
impl ToF64 for u64 {
    fn to_f64(self) -> f64 {
        self as f64
    }
}
impl ToF64 for u32 {
    fn to_f64(self) -> f64 {
        self as f64
    }
}

/// A value an ordered (quantile) sketch can ingest. Adds a **total** order
/// over [`ToF64`]: `f64` is only partially ordered, so `partial_cmp().unwrap()`
/// panics on NaN. Use `f64::total_cmp`; integers just use `Ord::cmp`.
///
/// The three widths the generator renders that carry an order. `string` is the
/// one it renders that does not, so no impl exists for it and the quantile rows
/// cannot be instantiated at it — the refusal is the absence of an impl, not a
/// runtime check.
pub trait QuantileValue: ToF64 + Copy {
    fn total_cmp(&self, other: &Self) -> std::cmp::Ordering;

    fn data_input(&self) -> asap_sketchlib::DataInput<'_>;
}

impl QuantileValue for i64 {
    #[inline(always)]
    fn data_input(&self) -> asap_sketchlib::DataInput<'_> {
        asap_sketchlib::DataInput::I64(*self)
    }

    #[inline(always)]
    fn total_cmp(&self, other: &Self) -> std::cmp::Ordering {
        Ord::cmp(self, other)
    }
}

impl QuantileValue for u64 {
    #[inline(always)]
    fn data_input(&self) -> asap_sketchlib::DataInput<'_> {
        asap_sketchlib::DataInput::U64(*self)
    }

    #[inline(always)]
    fn total_cmp(&self, other: &Self) -> std::cmp::Ordering {
        Ord::cmp(self, other)
    }
}

impl QuantileValue for f64 {
    #[inline(always)]
    fn data_input(&self) -> asap_sketchlib::DataInput<'_> {
        asap_sketchlib::DataInput::F64(*self)
    }

    #[inline(always)]
    fn total_cmp(&self, other: &Self) -> std::cmp::Ordering {
        f64::total_cmp(self, other)
    }
}

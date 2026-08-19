//! `asap_sketchlib::CMSHeap` wrappers — a Count-Min sketch paired with a
//! fixed-capacity top-k heap. A separate family from `cms`: it takes a
//! different knob set (no `top_k` in `--config`, see `sketchlib::CMS_HEAP_TOP_K`)
//! and answers two different questions (per-key frequency, and top-k), so it
//! gets two registry rows per backend rather than one.

pub mod sketchlib;

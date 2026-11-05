//! Text that behaves exactly as in `CPython` 3.14 (Unicode 16.0).
//!
//! Everything above this crate calls it instead of the `char`/`str` methods of Rust, whose answers differ
//! (`isspace`, `isdigit`, `isalpha`, `lower`, `round`, `max`, string lengths ...). See each module.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![forbid(unsafe_code)]

mod bigint;
pub mod constants;
pub mod layouts;
pub mod normalize;
pub mod pyjson;
pub mod pynum;
pub mod pystr;
pub mod russian_text;
pub mod ucd;
mod unicode_tables;

pub use pyjson::{json_dumps, Indent, JsonOptions};
pub use pynum::{
    exp, float_repr, fmean, fsum, log, log2, log_base, max_by_key_first, max_first, min_by_key_first,
    pstdev, py_float, py_int, py_round, py_round_ndigits, py_round_to_int, sqrt, FsumError,
};
pub use ucd::UNICODE_VERSION;

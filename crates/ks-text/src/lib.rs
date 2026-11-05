//! Text that behaves exactly as in `CPython` 3.14 (Unicode 16.0).
//!
//! Everything above this crate calls it instead of the `char`/`str` methods of Rust, whose answers differ
//! (`isspace`, `isdigit`, `isalpha`, `lower`, `round`, `max`, string lengths ...). See each module.

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
    exp, float_repr, float_str, fmean, fsum, log, log2, log_base, max_by_key_first, max_first, min_by_key_first,
    min_first, pstdev, py_float, py_int, py_round, py_round_ndigits, py_round_to_int, sqrt, FsumError,
};
pub use ucd::UNICODE_VERSION;

/// Sizes of the generated tables, in bytes of read-only data (for the record and the size budget).
#[must_use]
pub fn table_bytes() -> usize {
    use unicode_tables as t;
    size_of_val(&t::FLAG_STARTS)
        + size_of_val(&t::FLAG_VALUES)
        + size_of_val(&t::ASCII_FLAGS)
        + size_of_val(&t::CYRILLIC_FLAGS)
        + size_of_val(&t::LOWER_RUNS)
        + size_of_val(&t::UPPER_RUNS)
        + size_of_val(&t::FOLD_RUNS)
        + size_of_val(&t::TITLE_RUNS)
        + size_of_val(&t::DECOMP_CP) * 3
        + size_of_val(&t::COMPOSE_A) * 3
        + size_of_val(&t::CCC_RUNS)
}

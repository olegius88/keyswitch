//! `LogCourier` for a terminal: `status`, `list` and `fetch` print to it on every system.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#[cfg_attr(coverage_nightly, coverage(off))] // One call, and it ends the process: its tests are in `cli_binary.rs`.
fn main() {
    logcourier::cli::entry();
}

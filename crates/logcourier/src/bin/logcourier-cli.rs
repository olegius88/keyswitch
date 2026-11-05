//! `LogCourier` for a terminal: `status`, `list` and `fetch` print to it on every system.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
fn main() {
    logcourier::cli::entry();
}

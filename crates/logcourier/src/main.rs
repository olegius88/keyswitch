//! `LogCourier` with its window: no console appears on Windows.
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    logcourier::cli::entry();
}

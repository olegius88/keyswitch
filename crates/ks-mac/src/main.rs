//! `KeySwitch` for macOS.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#[cfg_attr(coverage_nightly, coverage(off))] // The whole program: the live tests start it.
fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(ks_mac::macos_app::main(&arguments));
}

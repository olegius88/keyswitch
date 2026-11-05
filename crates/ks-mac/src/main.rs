//! `KeySwitch` for macOS.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(ks_mac::macos_app::main(&arguments));
}

//! `KeySwitch` for Windows. A windowed program: no console window, whatever started it.
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![windows_subsystem = "windows"]

#[cfg_attr(coverage_nightly, coverage(off))] // The whole program: its test is the installed one, in `e2e_windows.rs`.
fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let status = ks_win::windows_app::main(&arguments);
    #[cfg(windows)]
    ks_win::desktop_app::leave(status);
    #[cfg(not(windows))]
    std::process::exit(status);
}

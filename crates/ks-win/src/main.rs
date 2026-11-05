//! `KeySwitch` for Windows. A windowed program: no console window, whatever started it.
#![windows_subsystem = "windows"]

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let status = ks_win::windows_app::main(&arguments);
    #[cfg(windows)]
    ks_win::desktop_app::leave(status);
    #[cfg(not(windows))]
    std::process::exit(status);
}

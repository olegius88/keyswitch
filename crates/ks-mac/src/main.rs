//! KeySwitch for macOS.

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(ks_mac::macos_app::main(&arguments));
}

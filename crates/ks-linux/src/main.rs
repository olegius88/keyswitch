//! `KeySwitch` for Linux.

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(ks_linux::linux_app::main(&arguments));
}

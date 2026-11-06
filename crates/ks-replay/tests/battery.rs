//! Streams of public words typed through the engine: every word must come out as it did from the last Python build.
#![cfg(any(not(debug_assertions), coverage))]

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unpacked(name: &str, folder: &Path) -> PathBuf {
    let packed = std::fs::File::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(format!("{name}.gz"))).expect("the fixture");
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(packed).read_to_end(&mut bytes).expect("gzip");
    let path = folder.join(name);
    std::fs::write(&path, bytes).expect("the copy");
    path
}

#[test]
fn every_word_comes_out_as_it_did_from_python() {
    let folder = std::env::temp_dir().join(format!("ks-battery-{}", std::process::id()));
    std::fs::create_dir_all(&folder).expect("the folder");
    let battery = unpacked("battery-public.jsonl", &folder);
    let reference = unpacked("rows-public.json", &folder);
    let output = Command::new(env!("CARGO_BIN_EXE_ks-replay"))
        .arg(&battery)
        .arg(folder.join("report.json"))
        .arg("--compare")
        .arg(&reference)
        .env("FIELD_ENTER", "1")
        .env("KS_RESOURCES", concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("it runs");
    let text = String::from_utf8_lossy(&output.stdout);
    let shown = text.lines().filter(|line| line.starts_with("rows ") || line.starts_with("  ")).collect::<Vec<_>>().join("\n");
    assert!(output.status.success(), "{shown}\n{}", String::from_utf8_lossy(&output.stderr));
    assert!(shown.contains("differing 0"), "{shown}");
    std::fs::remove_dir_all(&folder).expect("cleaned");
}

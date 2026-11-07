//! Streams of public words typed through the engine: every word must come out as it did from the last Python build, without
//! Hunspell and with it.
#![cfg(any(not(debug_assertions), coverage))]

use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::fmt::Write as _;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn unpacked(name: &str, folder: &Path) -> PathBuf {
    let packed = std::fs::File::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(format!("{name}.gz"))).expect("the fixture");
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(packed).read_to_end(&mut bytes).expect("gzip");
    let path = folder.join(name);
    std::fs::write(&path, bytes).expect("the copy");
    path
}

/// The public battery replayed into `report` and compared with `reference`; `extra` goes on the command line.
fn replayed(folder: &Path, report: &Path, reference: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ks-replay"))
        .arg(folder.join("battery-public.jsonl"))
        .arg(report)
        .arg("--compare")
        .arg(reference)
        .args(extra)
        .env("FIELD_ENTER", "1")
        .env("KS_RESOURCES", concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("it runs")
}

/// The comparison lines of a replay, and that it found every word as the reference has it.
fn assert_same(output: &Output) {
    let text = String::from_utf8_lossy(&output.stdout);
    let shown = text.lines().filter(|line| line.starts_with("rows ") || line.starts_with("  ")).collect::<Vec<_>>().join("\n");
    assert!(output.status.success(), "{shown}\n{}", String::from_utf8_lossy(&output.stderr));
    assert!(shown.contains("differing 0"), "{shown}");
}

#[test]
fn every_word_comes_out_as_it_did_from_python() {
    let folder = std::env::temp_dir().join(format!("ks-battery-{}", std::process::id()));
    std::fs::create_dir_all(&folder).expect("the folder");
    unpacked("battery-public.jsonl", &folder);
    let reference = unpacked("rows-public.json", &folder);
    assert_same(&replayed(&folder, &folder.join("report.json"), &reference, &[]));
    std::fs::remove_dir_all(&folder).expect("cleaned");
}

fn sha256_of(path: &Path) -> String {
    let digest = Sha256::digest(std::fs::read(path).expect("a dictionary file"));
    digest.iter().fold(String::new(), |mut text, byte| {
        write!(text, "{byte:02x}").expect("a string takes it");
        text
    })
}

/// Whether the program finds Hunspell and, for both locales, the dictionary and affix files `model/intent_v1/config.json`
/// pins for the external evaluation: the Python reference with Hunspell was made on those, and its `reference_models(True)`
/// refuses any other.
fn pinned_dictionaries() -> Result<(), String> {
    let config: Value =
        serde_json::from_slice(&std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../model/intent_v1/config.json")).expect("the config"))
            .expect("the config is JSON");
    let spellers = ks_hunspell::hunspell_speller(ks_config::history::data_dir());
    for locale in ["en_US", "ru_RU"] {
        let speller = spellers(locale);
        if !speller.available() {
            return Err(format!("no Hunspell speller for {locale}"));
        }
        let dictionary = PathBuf::from(speller.source());
        let affix = dictionary.with_extension("aff");
        let pins = &config["external_evaluation"]["hunspell"][locale];
        for (path, pin) in [(dictionary, "dictionary_sha256"), (affix, "affix_sha256")] {
            if pins[pin].as_str() != Some(sha256_of(&path).as_str()) {
                return Err(format!("{} is not the file the reference was made with", path.display()));
            }
        }
    }
    Ok(())
}

/// Hunspell knows short words the lexicons do not (single Latin letters, two-letter forms), and so decides words the
/// replay without it decides otherwise: the replay with `--speller` must answer as the Python's replay whose reference
/// models spell-check (`reference_models(True)`), not as the one without.
#[test]
fn with_hunspell_every_word_comes_out_as_it_did_from_python_with_it() {
    if let Err(missing) = pinned_dictionaries() {
        eprintln!("{missing}: skipped");
        return;
    }
    let folder = std::env::temp_dir().join(format!("ks-battery-hunspell-{}", std::process::id()));
    std::fs::create_dir_all(&folder).expect("the folder");
    unpacked("battery-public.jsonl", &folder);
    let reference = unpacked("rows-public-hunspell.json", &folder);
    let report = folder.join("report.json");
    assert_same(&replayed(&folder, &report, &reference, &["--speller"]));
    // The dictionaries decide some of these words: the reference without them is not this one.
    let rows = |path: &Path| -> Vec<Value> {
        let document: Value = serde_json::from_slice(&std::fs::read(path).expect("a report")).expect("JSON");
        document["rows"].as_array().expect("rows").clone()
    };
    let without = rows(&unpacked("rows-public.json", &folder));
    let differing = rows(&report).iter().zip(&without).filter(|(with, without)| with != without).count();
    assert!(differing > 0, "Hunspell changed no word of the public battery");
    std::fs::remove_dir_all(&folder).expect("cleaned");
}

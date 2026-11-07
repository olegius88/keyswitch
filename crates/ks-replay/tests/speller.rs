//! The replay with the speller the installed program uses (`--speller`): it spell-checks with Hunspell where Hunspell and its
//! dictionaries are installed, and stops saying which is missing where they are not, never replaying without it.
#![cfg(any(not(debug_assertions), coverage))]

use ks_lang::SpellChecker as _;
use ks_replay::harness::{Bundle, spellers_ready};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("ks-replay-speller-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("a folder");
    path
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn program_speller() -> ks_lang::resources::SpellerFactory {
    ks_hunspell::hunspell_speller(ks_config::history::data_dir())
}

/// Whether this machine has Hunspell and both dictionaries, as the program would find them.
fn installed() -> bool {
    spellers_ready(&program_speller()).is_ok()
}

#[test]
fn a_speller_with_no_dictionary_stops_the_replay_and_names_the_locale() {
    let empty = scratch("empty");
    let nowhere = ks_hunspell::hunspell_speller_in(vec![empty.clone()]);
    let refused = spellers_ready(&nowhere).expect_err("no dictionary there");
    assert!(refused.contains("no Hunspell speller for en_US"), "{refused}");
    let stopped = Bundle::with_speller(None, &nowhere).err().expect("no replay without the speller");
    assert_eq!(stopped, refused);
    std::fs::remove_dir_all(empty).expect("cleaned");
}

#[test]
fn the_models_of_a_replay_spell_check_only_when_asked_to() {
    let plain = Bundle::load(None);
    assert!(plain.models().iter().all(|model| !model.speller.available()));
    plain.clean();
    match Bundle::with_speller(None, &program_speller()) {
        Ok(spelling) => {
            assert!(spelling.models().iter().all(|model| model.speller.available()));
            assert!(spelling.models()[0].speller.check("hello"));
            spelling.clean();
        }
        Err(error) => assert!(!installed() && error.starts_with("no Hunspell speller for"), "{error}"),
    }
}

fn battery(folder: &Path) -> PathBuf {
    let packed = std::fs::File::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/battery-public.jsonl.gz")).expect("the fixture");
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(packed).read_to_end(&mut bytes).expect("gzip");
    let path = folder.join("battery.jsonl");
    std::fs::write(&path, bytes).expect("the copy");
    path
}

#[test]
fn a_replay_run_with_the_speller_finishes_or_says_what_is_missing() {
    let folder = scratch("run");
    let battery = battery(&folder);
    let report = folder.join("report.json");
    let replay = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ks-replay"));
        command.arg(&battery).arg(&report).args(["--limit", "2", "--speller"]);
        command
    };
    let spelling = replay().output().expect("it runs");
    if installed() {
        assert!(spelling.status.success(), "{}", said(&spelling));
        assert!(report.is_file());
    } else {
        assert_eq!(spelling.status.code(), Some(2), "{}", said(&spelling));
    }
    // Files named as the library that are no libraries, first on the library path: a machine without Hunspell.
    let fake = folder.join("no-hunspell");
    std::fs::create_dir_all(&fake).expect("a folder");
    for name in ["libhunspell-1.7.so.0", "libhunspell-1.7.so", "libhunspell.so.0", "libhunspell.so"] {
        std::fs::write(fake.join(name), b"no library").expect("a file");
    }
    let missing = replay().env("LD_LIBRARY_PATH", &fake).output().expect("it runs");
    assert_eq!(missing.status.code(), Some(2), "{}", said(&missing));
    assert!(said(&missing).contains("ks-replay --speller: no Hunspell speller for en_US"), "{}", said(&missing));
    std::fs::remove_dir_all(folder).expect("cleaned");
}

//! The measurement of the EN/RU detector over lexicons and dictionaries written for it: the order a lexicon fills its
//! table in, a dictionary in another encoding, one too small to sample and one that is not there.
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]
mod support;

use ks_lang::Resources;
use ks_train::detector::{command, hunspell_words};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use support::{scratch, tree};

/// A resource folder of links to the repository's, with language models of its own.
fn resources_with_models(name: &str, models: &[(&str, &str)]) -> PathBuf {
    let folder = scratch(name).join("resources");
    std::fs::create_dir_all(folder.join("models")).expect("a folder");
    for entry in std::fs::read_dir(tree().join("resources")).expect("the resources") {
        let entry = entry.expect("an entry");
        if entry.file_name() != "models" {
            std::os::unix::fs::symlink(entry.path(), folder.join(entry.file_name())).expect("a link");
        }
    }
    for (file, text) in models {
        std::fs::write(folder.join("models").join(file), text).expect("a model");
    }
    folder
}

fn dictionaries(folder: &Path, files: &[(&str, &[u8])]) {
    std::fs::create_dir_all(folder).expect("a folder");
    for (file, bytes) in files {
        std::fs::write(folder.join(file), bytes).expect("a dictionary");
    }
}

/// Unigrams in the order a lexicon lists them, with what its reader leaves out: a marker, a repeat, a line without a
/// count, a word too short to keep; then the bigrams, which the order does not read.
const ENGLISH: &str = "\\data\\\nngram 1=8\n\n\\1-grams:\n900 <s>\n800 house\n700 house\n600 garden\n500 hello\nlonely\n5 ab\n400 window\n950 here\n\\2-grams:\n3 the house\n\\end\\\n";
const RUSSIAN: &str = "\\data\\\nngram 1=4\n\n\\1-grams:\n900 дом\n800 сад\n700 привет\n600 окно\n\\end\\\n";

#[test]
fn a_detector_is_measured_over_small_lexicons_and_dictionaries() {
    let resources = resources_with_models("detector-small", &[("en_US.lm", ENGLISH), ("ru_RU.lm", RUSSIAN)]);
    let hunspell = resources.parent().expect("a folder").join("hunspell");
    // An English dictionary in Latin-1 of two words that fit, and of Russian words typed in the English layout that it
    // forbids: each one the detector turns into the Russian word, against the dictionary. A Russian dictionary of no word
    // that fits, so that `руку`, one of the authored cases, is no word there and becomes the English `here`.
    let forbidden = ["ghbdtn", "ljv", "cfl", "jryj", "gjrf", "ctujlyz", "pfdnhf", "rkfdbfnehf", "hfcrkflrf", "ntcn", "ntrcn", "ckjdj", "rjulf", "jxtym", "vjq", "lkz", "rfr", "xnj", "zpsr", "heccrbq", "ghjuhfvvf", "yfcnhjqrb", "bcghfdbnm"];
    let english = format!("{}\nhouse/S\ngarden\nab\n{}", forbidden.len() + 3, forbidden.iter().map(|word| format!("{word}/!\n")).collect::<String>());
    dictionaries(&hunspell, &[("en_US.aff", b"SET ISO8859-1\nFORBIDDENWORD !\n"), ("en_US.dic", english.as_bytes()), ("ru_RU.aff", b"SET UTF-8\n"), ("ru_RU.dic", "2\nаб\nя1\n".as_bytes())]);
    let speller = ks_hunspell::hunspell_speller_in(vec![hunspell]);
    // One dictionary direction of two: the strict gate does not hold.
    assert_eq!(command(&Resources::with_speller(resources, speller), 5, 5, true), Ok(1));
}

#[test]
fn a_detector_without_dictionaries_or_readable_lexicons_is_measured_on_what_is_left() {
    // No Hunspell at all: no dictionary direction, and the strict gate does not hold.
    assert_eq!(command(&Resources::new(tree().join("resources")), 5, 5, true), Ok(1));
    // Lexicons that cannot be read: the fallback words alone.
    let resources = resources_with_models("detector-unreadable", &[("en_US.lm", ENGLISH), ("ru_RU.lm", RUSSIAN)]);
    for file in ["en_US.lm", "ru_RU.lm"] {
        std::fs::set_permissions(resources.join("models").join(file), std::fs::Permissions::from_mode(0o000)).expect("a mode");
    }
    let measured = command(&Resources::new(resources.clone()), 5, 5, false);
    for file in ["en_US.lm", "ru_RU.lm"] {
        std::fs::set_permissions(resources.join("models").join(file), std::fs::Permissions::from_mode(0o644)).expect("a mode");
    }
    assert_eq!(measured, Ok(0));
}

#[test]
fn a_dictionary_that_is_not_there_gives_no_sample() {
    let folder = scratch("detector-gone");
    assert!(hunspell_words(&folder.join("gone.dic").to_string_lossy(), 5).is_empty());
    dictionaries(&folder, &[("half.aff", b"SET UTF-8\n")]);
    assert!(hunspell_words(&folder.join("half.dic").to_string_lossy(), 5).is_empty());
    // A sample smaller than the dictionary is spread over it, both ends included.
    dictionaries(&folder, &[("many.aff", b"SET UTF-8\n"), ("many.dic", b"5\napple\nberry\ncherry\ndate\nelder\n")]);
    assert_eq!(hunspell_words(&folder.join("many.dic").to_string_lossy(), 3), ["apple", "cherry", "elder"]);
    // UTF-8 named without its dash is still UTF-8.
    dictionaries(&folder, &[("plain.aff", b"SET UTF8\n"), ("plain.dic", b"1\napple\n")]);
    assert_eq!(hunspell_words(&folder.join("plain.dic").to_string_lossy(), 5), ["apple"]);
}

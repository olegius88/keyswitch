//! The boundary-v2 model's tools over small inputs: the words of a phrase, the commands asked what they do not know, and
//! a whole experiment over a few frozen examples written by hand, in a tree of links whose config and installed model
//! are its own.
mod eval_support;
mod support;

use eval_support::{edit, linked, own, read};
use flate2::Compression;
use flate2::write::GzEncoder;
use ks_lang::Resources;
use ks_train::boundary::{canonical_tokens, command, corpus_provenance};
use serde_json::{Value, json};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Output;
use support::{digest, scratch, trainer, tree};

#[test]
fn the_words_of_a_phrase_keep_their_apostrophes_and_hyphens() {
    assert_eq!(canonical_tokens("Don’t re-use well-known_words, ёж; x- a- b 'c"), ["don't", "re-use", "well-known", "words", "еж", "x", "a", "b", "c"]);
    assert!(canonical_tokens("… — !").is_empty());
    assert_eq!(canonical_tokens("re-use"), ["re-use"]);
}

#[test]
fn a_boundary_command_it_does_not_know_is_refused() {
    let folder = scratch("boundary-unknown");
    let said = command(&tree(), &Resources::new(tree().join("resources")), "other", &folder, &folder).expect_err("refused");
    assert!(said.contains("unknown boundary command"), "{said}");
}

const CATEGORIES: [&str; 3] = ["word_ending", "literal_en", "literal_ru"];

/// A row whose label is told apart by a feature of its own on each candidate.
fn example(category: &str, label: usize) -> Value {
    let features: Vec<Value> = (0..2).map(|candidate| json!({format!("{category}:{label}:{candidate}"): 1.0})).collect();
    json!({"family": format!("{category}{label}"), "split": "x", "original": "x", "intended_word": "x", "labels": [label], "category": category, "features": features})
}

fn gz(lines: &[Value]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    for line in lines {
        encoder.write_all(serde_json::to_string(line).expect("json").as_bytes()).expect("in memory");
        encoder.write_all(b"\n").expect("in memory");
    }
    encoder.finish().expect("in memory")
}

/// Frozen examples of the four splits, so many of each kind; development also holds a row of two labels and one whose
/// candidates cannot be told apart.
fn examples(directory: &Path) {
    std::fs::create_dir_all(directory).expect("a folder");
    let mut hashes = serde_json::Map::new();
    for (split, copies) in [("train", 12), ("development", 2), ("calibration", 2), ("test", 4)] {
        let mut rows: Vec<Value> = Vec::new();
        for category in CATEGORIES {
            for label in 0..2 {
                rows.extend(std::iter::repeat_n(example(category, label), copies));
            }
        }
        if split == "development" {
            let mut both = example("word_ending", 0);
            both["labels"] = json!([0, 1]);
            rows.push(both);
            let mut alike = example("word_ending", 0);
            alike["features"] = json!([{"word_ending:0:0": 1.0}, {"word_ending:0:0": 1.0}]);
            rows.push(alike);
            let mut mislabelled = example("word_ending", 0);
            mislabelled["labels"] = json!([1]);
            rows.push(mislabelled);
        }
        let packed = gz(&rows);
        std::fs::write(directory.join(format!("{split}.jsonl.gz")), &packed).expect("a split");
        hashes.insert(split.to_owned(), json!(digest(&packed)));
    }
    let receipt = json!({"schema_version": 1, "provenance": {"tools/boundary_v2_corpus.py": "0"}, "sha256": hashes});
    std::fs::write(directory.join("corpus.json"), serde_json::to_vec(&receipt).expect("json")).expect("a receipt");
}

/// A tree whose boundary config is this one and whose installed boundary model is its own.
fn experiment_tree(name: &str, promotion: &Value) -> PathBuf {
    let root = linked(name);
    own(&root, "resources/models/boundary-v2.json");
    let config = json!({"schema_version": 1, "namespace": "small", "maximum_words_per_stratum": 3, "maximum_words_per_family": 1, "epochs": 30, "learning_rate": 0.12,
        "thresholds": [1.0, 0.6, 0.7], "promotion": promotion, "evidence_scope": "a small experiment"});
    std::fs::write(own(&root, "model/boundary_v2/config.json"), serde_json::to_vec(&config).expect("json")).expect("a config");
    root
}

fn promotion() -> Value {
    json!({"maximum_errors": 0, "minimum_decided_fraction_per_stratum": 0.8, "required_recall_strata": CATEGORIES, "minimum_test_rows": 10, "minimum_test_word_endings": 2})
}

fn step(root: &Path, action: &str, directory: &Path) -> Output {
    trainer().arg("boundary").arg("--tree").arg(root).arg(format!("--{action}")).arg("--directory").arg(directory).arg("--output").arg(directory).output().expect("the trainer starts")
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn done(output: &Output) {
    assert!(output.status.success(), "{}", said(output));
}

fn refused(output: &Output, words: &str) {
    assert!(!output.status.success() && said(output).contains(words), "{words}: {}", said(output));
}

/// A copy of an experiment's files, to change one of them.
fn copy_of(from: &Path, to: &Path) -> PathBuf {
    std::fs::create_dir_all(to).expect("a folder");
    for entry in std::fs::read_dir(from).expect("a folder").flatten() {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), to.join(entry.file_name())).expect("a copy");
        }
    }
    to.to_path_buf()
}

#[test]
fn a_small_boundary_experiment_is_trained_tested_promoted_and_checked() {
    let root = experiment_tree("boundary-small", &promotion());
    let folder = scratch("boundary-small-experiment");
    // Split files without a receipt are not written over.
    let half = folder.join("half");
    std::fs::create_dir_all(&half).expect("a folder");
    std::fs::write(half.join("train.jsonl.gz"), gz(&[])).expect("a split");
    refused(&step(&root, "freeze", &half), "refusing to overwrite");
    // The experiment, from training to the installed model.
    let experiment = folder.join("experiment");
    examples(&experiment);
    let pristine = copy_of(&experiment, &folder.join("pristine"));
    done(&step(&root, "train", &experiment));
    let seal = read(&experiment.join("seal.json"));
    assert!((seal["threshold_search"][1]["threshold"].as_f64().expect("a threshold") - 0.6).abs() < 1e-12, "{seal}");
    assert!(seal["development"]["counts"]["abstained"].as_i64().expect("a count") >= 1 && seal["development"]["counts"]["errors"].as_i64().expect("a count") >= 1, "{seal}");
    done(&step(&root, "test", &experiment));
    assert_eq!(read(&experiment.join("report.json"))["accepted"], json!(true));
    done(&step(&root, "verify", &experiment));
    done(&step(&root, "promote", &experiment));
    assert_eq!(std::fs::read(root.join("resources/models/boundary-v2.json")).expect("the model"), std::fs::read(experiment.join("candidate.json")).expect("the candidate"));
    // A candidate that is not the one trained.
    let changed = copy_of(&experiment, &folder.join("changed-candidate"));
    edit(&changed, "candidate.json", |candidate| candidate["threshold"] = json!(0.65));
    refused(&step(&root, "verify", &changed), "not reproducible");
    refused(&step(&root, "promote", &changed), "changed after seal");
    // A seal whose provenance or calibration is not what it was.
    let pins = copy_of(&experiment, &folder.join("changed-pins"));
    edit(&pins, "seal.json", |seal| seal["provenance"]["trainer"] = json!("0"));
    refused(&step(&root, "promote", &pins), "changed after seal");
    let calibration = copy_of(&experiment, &folder.join("changed-calibration"));
    edit(&calibration, "seal.json", |seal| seal["calibration"]["counts"]["errors"] = json!(5));
    refused(&step(&root, "promote", &calibration), "calibration gates failed");
    // Receipts written by this trainer have their sources checked; a split that is not the receipt's is refused.
    let current = copy_of(&pristine, &folder.join("current-receipt"));
    let provenance = corpus_provenance(&root).expect("the sources");
    edit(&current, "corpus.json", |receipt| receipt["provenance"] = provenance);
    done(&step(&root, "train", &current));
    let stale = copy_of(&pristine, &folder.join("stale-receipt"));
    edit(&stale, "corpus.json", |receipt| receipt["provenance"] = json!({}));
    refused(&step(&root, "train", &stale), "provenance changed");
    let mismatched = copy_of(&pristine, &folder.join("mismatched"));
    edit(&mismatched, "corpus.json", |receipt| receipt["sha256"]["development"] = json!("0"));
    refused(&step(&root, "train", &mismatched), "checksum mismatch");
    // The config changed after the seal.
    edit(&root, "model/boundary_v2/config.json", |config| config["epochs"] = json!(31));
    refused(&step(&root, "promote", &experiment), "changed after seal");
}

#[test]
fn a_boundary_experiment_short_of_its_gates_installs_nothing() {
    // A test too small for its gate, in rows or in word endings: tested, and not promoted.
    for (name, rows, endings) in [("boundary-few-rows", 1000, 2), ("boundary-few-endings", 10, 1000)] {
        let promotion = json!({"maximum_errors": 0, "minimum_decided_fraction_per_stratum": 0.8, "required_recall_strata": CATEGORIES, "minimum_test_rows": rows, "minimum_test_word_endings": endings});
        let root = experiment_tree(name, &promotion);
        let experiment = scratch(&format!("{name}-experiment")).join("experiment");
        examples(&experiment);
        done(&step(&root, "train", &experiment));
        done(&step(&root, "test", &experiment));
        assert_eq!(read(&experiment.join("report.json"))["accepted"], json!(false));
        refused(&step(&root, "promote", &experiment), "failed promotion gates");
    }
    // No threshold holds on calibration: the highest stands, and the test is not opened.
    let none = experiment_tree("boundary-no-threshold", &json!({"maximum_errors": -1, "minimum_decided_fraction_per_stratum": 0.8, "required_recall_strata": CATEGORIES, "minimum_test_rows": 1, "minimum_test_word_endings": 1}));
    let experiment = scratch("boundary-no-threshold-experiment").join("experiment");
    examples(&experiment);
    done(&step(&none, "train", &experiment));
    assert_eq!(read(&experiment.join("seal.json"))["threshold_search"].as_array().expect("the search").len(), 3);
    refused(&step(&none, "test", &experiment), "calibration gates failed");
    assert!(!experiment.join("report.json").exists());
}

#[test]
fn a_freeze_stops_at_a_lexicon_that_is_not_the_pinned_one() {
    let root = linked("boundary-lexicon");
    edit(&root, "model/intent_v1/config.json", |config| config["sources"]["languages"]["ru_RU"]["sha256"] = json!("0"));
    refused(&step(&root, "freeze", &scratch("boundary-lexicon-frozen").join("frozen")), "onboard lexicon checksum mismatch");
}

fn lexicon(words: &[&str]) -> String {
    let counts: String = words.iter().map(|word| format!("100 {word}\n")).collect();
    format!("\\data\\\nngram 1={}\n\n\\1-grams:\n{counts}\\end\\\n", words.len())
}

#[test]
fn a_freeze_over_small_lexicons_keeps_only_the_rows_a_boundary_can_be_learned_on() {
    let root = linked("boundary-small-lexicons");
    // Words whose keys are all ambiguous punctuation, one of seven such keys, and two words that end in the comma key
    // and drop, as their misspellings, different keys to the same seven: the draws were searched for such a pair.
    let lexicons = [("ru_RU", "small-ru.lm", lexicon(&["бюжхъэё", "бюж", "пьеодтвб", "рпьеодвб", "привет"])), ("en_US", "small-en.lm", lexicon(&["hello", "world"]))];
    for (_, file, text) in &lexicons {
        std::fs::write(root.join(file), text).expect("a lexicon");
    }
    edit(&root, "model/intent_v1/config.json", |config| {
        for (locale, file, text) in &lexicons {
            config["sources"]["languages"][locale]["path"] = json!(file);
            config["sources"]["languages"][locale]["sha256"] = json!(digest(text.as_bytes()));
        }
    });
    let frozen = scratch("boundary-small-lexicons-frozen").join("frozen");
    done(&step(&root, "freeze", &frozen));
    assert!(read(&frozen.join("corpus.json"))["rows"]["train"].as_i64().expect("a count") > 0);
}

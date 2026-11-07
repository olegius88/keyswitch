//! The prefix candidate's parents, read from splits written here: which rows become parents, with which typo and safety
//! forms, and what a split may not hold.
mod support;

use flate2::Compression;
use flate2::write::GzEncoder;
use ks_train::prefix::{ParentWord, load_parents};
use serde_json::{Value, json};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Output;
use support::{digest, recipe, refused, scratch, small_tree, succeeded, trainer};

/// A row of an old prefix split, of its first letter, as the old corpus wrote it.
fn old(sequence: i64, category: &str, text: &str, source: i64, family: &str, before: &str) -> Value {
    json!({"sequence": sequence, "category": category, "text": text, "source": source, "family": family, "before": before, "length": 1, "profile": "portable", "application": "Telegram", "role": "text"})
}

fn with(mut row: Value, changes: &[(&str, Value)]) -> Value {
    for (name, value) in changes {
        row[*name] = value.clone();
    }
    row
}

fn gz(text: &str) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(text.as_bytes()).expect("in memory");
    encoder.finish().expect("in memory")
}

/// A folder of old splits with a receipt that pins them.
fn splits(name: &str, rows: &[(&str, Vec<Value>)]) -> PathBuf {
    let folder = scratch(name);
    let mut receipt = json!({"sha256": {}});
    for (split, lines) in rows {
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        let packed = gz(&text);
        std::fs::write(folder.join(format!("{split}.jsonl.gz")), &packed).expect("a split");
        receipt["sha256"][*split] = json!(digest(&packed));
    }
    std::fs::write(folder.join("corpus.json"), receipt.to_string()).expect("a receipt");
    folder
}

fn identifiers(parents: &[ParentWord]) -> Vec<&str> {
    parents.iter().map(|parent| parent.identifier.as_str()).collect()
}

fn said(folder: &Path, split: &str) -> String {
    load_parents(folder, split, 10, 10).expect_err("a refusal")
}

#[test]
fn a_parent_brings_its_own_typo_and_safety_forms_and_nothing_else() {
    let rows = vec![
        old(1, "correct", "called", 0, "cal", "Tom "),
        // The typo of the fourth letter, three times over: the least identifier stays, 10 before 11 and 9 as text.
        old(9, "typo", "caled", 0, "cal", "Tom "),
        old(10, "typo", "caled", 0, "cal", "Tom "),
        old(11, "typo", "caled", 0, "cal", "Tom "),
        with(old(12, "identifier", "called_value", 0, "cal", "const value = "), &[("application", json!("Code")), ("role", json!("code"))]),
        with(old(13, "command", "called", 0, "cal", "$ "), &[("application", json!("WindowsTerminal")), ("role", json!("terminal"))]),
        with(old(14, "command", "called", 0, "cal", "$ "), &[("application", json!("WindowsTerminal")), ("role", json!("terminal"))]),
        old(15, "mixed_english", "called", 0, "cal", "в сообщении написано "),
        // Safety forms of Russian sources are never read.
        with(old(16, "command", "called", 1, "cal", "$ "), &[("application", json!("WindowsTerminal")), ("role", json!("terminal"))]),
        // The same word again is one parent; a fourth word is past the family's budget. The hash of each word orders them:
        // calm, called, calls, calla.
        old(17, "correct", "called", 0, "cal", "Ann "),
        old(18, "correct", "calm", 0, "cal", ""),
        old(19, "correct", "calls", 0, "cal", ""),
        old(27, "correct", "calla", 0, "cal", ""),
        old(20, "correct", "дом", 1, "lji", ""),
        // Rows that are no parent: of a longer prefix, of the other profile, of a category the candidate does not read.
        with(old(21, "correct", "other", 0, "oth", ""), &[("length", json!(2))]),
        with(old(22, "correct", "other", 0, "oth", ""), &[("profile", json!("reference_hunspell"))]),
        old(23, "wrong", "other", 0, "oth", ""),
        with(old(24, "correct", "other", 0, "oth", ""), &[("category", Value::Null)]),
        // A row without an application or a role is of no application, in a field of no role.
        with(old(25, "correct", "search", 0, "sea", ""), &[("application", Value::Null), ("role", Value::Null)]),
        with(old(26, "correct", "found", 0, "fou", ""), &[("role", json!("search"))]),
    ];
    let folder = splits("prefix-parents", &[("train", rows)]);
    let parents = load_parents(&folder, "train", 10, 3).expect("the parents");
    let found = identifiers(&parents);
    for wanted in ["prefix-v1:train:10:parent:prefix-v1:train:1", "prefix-v1:train:12:parent:prefix-v1:train:1", "prefix-v1:train:13:parent:prefix-v1:train:1", "prefix-v1:train:15:parent:prefix-v1:train:1", "prefix-v1:train:20"] {
        assert!(found.contains(&wanted), "{wanted}: {found:?}");
    }
    for unwanted in ["prefix-v1:train:9", "prefix-v1:train:11", "prefix-v1:train:14", "prefix-v1:train:16", "prefix-v1:train:21", "prefix-v1:train:22", "prefix-v1:train:23", "prefix-v1:train:24", "prefix-v1:train:27"] {
        assert!(!found.iter().any(|identifier| identifier.starts_with(&format!("{unwanted}:")) || *identifier == unwanted), "{unwanted}: {found:?}");
    }
    let family: Vec<&ParentWord> = parents.iter().filter(|parent| parent.family == "cal" && parent.category == "correct").collect();
    assert_eq!(family.len(), 3, "{found:?}");
    let plain = parents.iter().find(|parent| parent.family == "sea").expect("a parent");
    assert_eq!((plain.application.as_str(), plain.role), ("", ks_backend::FieldRole::Unknown));
    assert_eq!(parents.iter().find(|parent| parent.family == "fou").expect("a parent").role, ks_backend::FieldRole::Search);
    // Fewer families than there are: the hash of each name chooses which.
    assert_eq!(load_parents(&folder, "train", 1, 2).expect("the parents").iter().map(|parent| parent.family.as_str()).collect::<std::collections::BTreeSet<_>>().len(), 1);
    let roles: Vec<ks_backend::FieldRole> = ["unknown", "text", "code", "search", "terminal"].iter().map(|name| {
        let row = with(old(1, "correct", "word", 0, "wor", ""), &[("role", json!(name))]);
        load_parents(&splits("prefix-role", &[("development", vec![row])]), "development", 1, 1).expect("a parent")[0].role
    }).collect();
    assert_eq!(roles, [ks_backend::FieldRole::Unknown, ks_backend::FieldRole::Text, ks_backend::FieldRole::Code, ks_backend::FieldRole::Search, ks_backend::FieldRole::Terminal]);
}

#[test]
fn a_split_that_cannot_be_read_as_parents_is_refused() {
    let good = splits("prefix-good", &[("calibration", vec![old(1, "correct", "word", 0, "wor", "")])]);
    assert!(said(&good, "test").contains("only exposed"));
    let mut receipt: Value = serde_json::from_slice(&std::fs::read(good.join("corpus.json")).expect("a receipt")).expect("json");
    receipt["sha256"]["calibration"] = json!("0");
    std::fs::write(good.join("corpus.json"), receipt.to_string()).expect("a receipt");
    assert!(said(&good, "calibration").contains("checksum mismatch"));
    let refused = |rows: Vec<Value>, words: &str| {
        let folder = splits("prefix-bad", &[("train", rows)]);
        let answer = said(&folder, "train");
        assert!(answer.contains(words), "{words}: {answer}");
    };
    for field in ["sequence", "text", "source", "family", "before"] {
        refused(vec![with(old(1, "correct", "word", 0, "wor", ""), &[(field, Value::Null)])], "invalid old prefix parent metadata");
    }
    refused(vec![with(old(1, "correct", "word", 0, "wor", ""), &[("role", json!("window"))])], "invalid old prefix application metadata");
    refused(vec![old(1, "correct", "word", 0, "wor", ""), old(1, "typo", "wod", 0, "wor", "")], "duplicate old prefix parent identifier");
    let folder = scratch("prefix-not-json");
    std::fs::write(folder.join("train.jsonl.gz"), gz("{\n")).expect("a split");
    std::fs::write(folder.join("corpus.json"), json!({"sha256": {"train": digest(&std::fs::read(folder.join("train.jsonl.gz")).expect("a split"))}}).to_string()).expect("a receipt");
    assert!(!said(&folder, "train").is_empty());
    let folder = scratch("prefix-no-split");
    std::fs::write(folder.join("corpus.json"), "{}").expect("a receipt");
    assert!(!said(&folder, "train").is_empty());
}

/// A file of a small tree that is its own: a link into the repository is taken out first, never written through.
fn own_file(path: &Path, bytes: &[u8]) {
    let folder = path.parent().expect("a folder");
    assert!(!folder.symlink_metadata().expect("a folder").file_type().is_symlink(), "{}", folder.display());
    if path.symlink_metadata().is_ok_and(|meta| meta.file_type().is_symlink()) {
        std::fs::remove_file(path).expect("the link");
    }
    std::fs::write(path, bytes).expect("a file");
}

const ENGLISH: [&str; 8] = ["people", "garden", "window", "summer", "market", "doctor", "school", "friend"];
const RUSSIAN: [&str; 8] = ["погода", "дорога", "машина", "работа", "солнце", "письмо", "страна", "голова"];

/// Parents of their own families in each split: four English and four Russian words in TRAIN, two of each elsewhere.
fn parents_of(split: &str) -> Vec<Value> {
    let (from, count) = match split {
        "train" => (0, 4),
        "development" => (4, 2),
        _ => (6, 2),
    };
    let mut rows = Vec::new();
    for index in from..from + count {
        rows.push(old(i64::try_from(rows.len()).expect("small"), "correct", ENGLISH[index], 0, &format!("{split}-en-{index}"), "I think "));
        rows.push(old(i64::try_from(rows.len()).expect("small"), "correct", RUSSIAN[index], 1, &format!("{split}-ru-{index}"), "я думаю "));
    }
    rows
}

/// A small tree whose old prefix splits are these, and a recipe of a few epochs over them.
fn prefix_tree(name: &str, train: &[Value], development: &[Value], changes: &[(&str, Value)]) -> (PathBuf, PathBuf) {
    let small = small_tree(name, &recipe());
    let directory = small.join("model/prefix_v1");
    let mut receipt: Value = serde_json::from_slice(&std::fs::read(directory.join("corpus.json")).expect("a receipt")).expect("json");
    for (split, rows) in [("train", train.to_vec()), ("development", development.to_vec()), ("calibration", parents_of("calibration"))] {
        let text: String = rows.iter().map(|line| format!("{line}\n")).collect();
        let packed = gz(&text);
        own_file(&directory.join(format!("{split}.jsonl.gz")), &packed);
        receipt["sha256"][split] = json!(digest(&packed));
    }
    own_file(&directory.join("corpus.json"), receipt.to_string().as_bytes());
    let mut options: Value = serde_json::from_slice(&std::fs::read(small.join("model/prefix_v2/recipe.json")).expect("the recipe")).expect("json");
    options["maximum_families"] = json!({"train": 20, "development": 20, "calibration": 20});
    options["epochs"] = json!(2);
    options["minimum_feature_mass"] = json!(1.0);
    options["maximum_features"] = json!(3000);
    for (name, value) in changes {
        options[*name] = value.clone();
    }
    let path = small.join("prefix-recipe.json");
    std::fs::write(&path, options.to_string()).expect("a recipe");
    (small, path)
}

fn prefix_fit(tree: &Path, recipe_path: &Path, output: &Path, reuse: bool) -> Output {
    let mut command = trainer();
    command.arg("prefix").arg("--tree").arg(tree).arg("--recipe").arg(recipe_path).arg("--output").arg(output).args(["--jobs", "2"]);
    if reuse {
        command.arg("--reuse-installed");
    }
    command.output().expect("the trainer starts")
}

#[test]
fn a_prefix_fit_stops_on_splits_and_settings_it_cannot_use() {
    let output = |name: &str| scratch(name).join("out");
    // Not over an earlier experiment.
    let (small, path) = prefix_tree("prefix-there", &parents_of("train"), &parents_of("development"), &[]);
    let there = scratch("prefix-there-out");
    refused(&prefix_fit(&small, &path, &there, false), "refusing to overwrite");
    // A family of two splits.
    let mut development = parents_of("development");
    development.push(old(90, "correct", "letter", 0, "train-en-0", ""));
    let (small, path) = prefix_tree("prefix-overlap", &parents_of("train"), &development, &[]);
    refused(&prefix_fit(&small, &path, &output("prefix-overlap"), false), "families overlap");
    // No feature with that much mass.
    let (small, path) = prefix_tree("prefix-mass", &parents_of("train"), &parents_of("development"), &[("minimum_feature_mass", json!(1e9))]);
    refused(&prefix_fit(&small, &path, &output("prefix-mass"), false), "no prefix features survive");
    // DEVELOPMENT without a parent.
    let (small, path) = prefix_tree("prefix-empty", &parents_of("train"), &[old(1, "wrong", "letter", 0, "w", "")], &[]);
    refused(&prefix_fit(&small, &path, &output("prefix-empty"), false), "empty training or development split");
    // No operating point to choose an epoch at.
    let (small, path) = prefix_tree("prefix-points", &parents_of("train"), &parents_of("development"), &[("thresholds", json!([]))]);
    refused(&prefix_fit(&small, &path, &output("prefix-points"), false), "no zero-false development epoch frontier");
    // Epochs that move nothing choose the first; a parent of no letters takes its family's budget with it.
    let mut train = parents_of("train");
    train.push(old(91, "correct", "", 0, "train-none", ""));
    let (small, path) = prefix_tree("prefix-budget", &train, &parents_of("development"), &[("learning_rate", json!(0.0))]);
    let answer = prefix_fit(&small, &path, &output("prefix-budget"), false);
    refused(&answer, "physical-family budget drift");
    assert!(String::from_utf8_lossy(&answer.stdout).contains("prefix-v2 epoch 2"), "{}", String::from_utf8_lossy(&answer.stdout));
}

#[test]
fn an_installed_prefix_candidate_is_reused_only_when_its_seal_pins_this_tree() {
    let small = small_tree("prefix-reuse", &recipe());
    // Not a recipe at all: when the installed candidate is not reused the fit stops at once on reading it.
    let path = small.join("not-a-recipe.json");
    std::fs::write(&path, "not json").expect("a recipe");
    refused(&prefix_fit(&small, &path, &scratch("prefix-reuse-old").join("out"), true), "");
    let mut pins = serde_json::to_value(ks_train::fit::provenance(&small, &path).expect("the pins")).expect("json");
    for name in ["model/prefix_v1/corpus.json", "model/prefix_v1/config.json", "model/prefix_v1/train.jsonl.gz", "model/prefix_v1/development.jsonl.gz", "model/prefix_v1/calibration.jsonl.gz"] {
        pins[name] = json!(ks_train::corpus::checksum(&small.join(name)).expect("a file"));
    }
    let artifact = ks_train::corpus::checksum(&small.join("resources/models/prefix_policy_v1.json")).expect("the candidate");
    let seal_path = small.join("model/prefix_v2/candidate-seal.json");
    let mut seal: Value = serde_json::from_slice(&std::fs::read(&seal_path).expect("the seal")).expect("json");
    for (name, candidate, passed) in [("sha", "0".to_owned(), true), ("calibration", artifact.clone(), false), ("pinned", artifact, true)] {
        seal["provenance"] = pins.clone();
        seal["candidate_sha256"] = json!(candidate);
        seal["calibration_passed"] = json!(passed);
        own_file(&seal_path, &serde_json::to_vec(&seal).expect("json"));
        let output = scratch(&format!("prefix-reuse-{name}")).join("out");
        let answer = prefix_fit(&small, &path, &output, true);
        if name == "pinned" {
            succeeded(&answer);
            for file in ["prefix-candidate.json", "seal.json", "recipe.json"] {
                assert!(output.join(file).is_file(), "{file}");
            }
        } else {
            refused(&answer, "");
            assert!(!output.exists());
        }
    }
}

#[test]
fn the_prefix_corpus_verifies_against_what_it_froze_and_names_each_split_that_differs() {
    let small = small_tree("prefix-corpus-verify", &recipe());
    let directory = small.join("model/prefix_v1");
    let mut config: Value = serde_json::from_slice(&std::fs::read(directory.join("config.json")).expect("the config")).expect("json");
    config["maximum_words_per_language"] = json!(40);
    config["maximum_words_per_family"] = json!(1);
    own_file(&directory.join("config.json"), config.to_string().as_bytes());
    let frozen = scratch("prefix-corpus-frozen").join("frozen");
    let on = |tree: &Path| {
        let mut command = trainer();
        command.arg("prefix-corpus").arg("--tree").arg(tree);
        command
    };
    succeeded(&on(&small).arg("--freeze").arg(&frozen).output().expect("the trainer starts"));
    // The tree now holds what was frozen: every split is the same.
    for split in ["train", "development", "calibration", "test"] {
        own_file(&directory.join(format!("{split}.jsonl.gz")), &std::fs::read(frozen.join(format!("{split}.jsonl.gz"))).expect("a split"));
    }
    let receipt: Value = serde_json::from_slice(&std::fs::read(frozen.join("corpus.json")).expect("a receipt")).expect("json");
    own_file(&directory.join("corpus.json"), receipt.to_string().as_bytes());
    let same = succeeded(&on(&small).output().expect("the trainer starts"));
    assert_eq!(same.matches(": same").count(), 4, "{same}");
    // A frozen generation is never written over.
    refused(&on(&small).arg("--freeze").arg(&frozen).output().expect("the trainer starts"), "refusing to overwrite frozen prefix examples");
    // Other rows, and a receipt that pins other bytes, another count of rows, another count of families.
    own_file(&directory.join("train.jsonl.gz"), &gz("{}\n"));
    let mut changed = receipt;
    changed["sha256"]["development"] = json!("0");
    changed["rows"]["calibration"] = json!(0);
    changed["families"]["test"] = json!(0);
    own_file(&directory.join("corpus.json"), changed.to_string().as_bytes());
    let answer = on(&small).output().expect("the trainer starts");
    refused(&answer, "4 splits of the prefix corpus differ");
    let said = String::from_utf8_lossy(&answer.stdout);
    assert_eq!(said.matches(": DIFFERS").count(), 4, "{said}");
}

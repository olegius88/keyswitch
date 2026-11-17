//! The prefix candidate's parents, read from splits written here: which rows become parents, with which typo and safety
//! forms, and what a split may not hold.
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]
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

/// The word corpus of the prefix model: frozen from a small base and the tree's sources, verified byte for byte, refused
/// over a frozen generation or without held-out samples; and a fit over a word corpus, which reads each row in its own
/// situation, stops when its patience is spent and reports each class, or chooses its epoch and a threshold for each
/// direction by the key of the switch, keeps its frames, and whose model is measured without the engine.
mod words {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use ks_text::json::Kept;
    use ks_train::prefix_words::{Class, LowKeysRule, Quotas, Situation, WordConfig, WordRow};
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;
    use std::io::Write as _;
    use std::path::{Path, PathBuf};
    use super::support::{digest, recipe, refused, scratch, small_tree, succeeded, trainer};

    fn gz<T: Serialize>(lines: &[T]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        for line in lines {
            encoder.write_all(format!("{}\n", ks_text::json::dumps(line, &ks_text::JsonOptions::default())).as_bytes()).expect("in memory");
        }
        encoder.finish().expect("in memory")
    }

    fn read<T: for<'a> Deserialize<'a>>(path: &Path) -> T {
        ks_text::json::loads(std::fs::read(path).expect("a file the test reads")).expect("JSON of the shape the test reads")
    }

    fn write<T: Serialize>(path: &Path, value: &T) {
        std::fs::write(path, ks_text::json::dumps(value, &ks_text::JsonOptions::default())).expect("a file");
    }

    /// A row of the base corpus, as `base freeze` writes it.
    #[derive(Serialize)]
    struct BaseRow {
        original: &'static str,
        upos: &'static str,
        source: &'static str,
        before: &'static str,
        split: &'static str,
        after: &'static str,
    }

    #[derive(Serialize)]
    struct BaseSplit {
        path: &'static str,
        sha256: String,
    }

    #[derive(Serialize)]
    struct BaseManifest {
        namespace: &'static str,
        splits: BTreeMap<&'static str, BaseSplit>,
    }

    /// A base corpus of one training split: treebank words, a name, a Debian command and an address.
    fn base(folder: &Path) {
        let row = |original: &'static str, upos: &'static str, source: &'static str, before: &'static str| BaseRow { original, upos, source, before, split: "train", after: "" };
        let rows = [
            row("garden", "NOUN", "UD_English-EWT", "In the "),
            row("Kaplan", "PROPN", "UD_English-EWT", "Mr "),
            row("kaplan@iepa.com", "X", "UD_English-EWT", "Write to "),
            row("погодой", "NOUN", "UD_Russian-SynTagRus", "Под "),
            row("Тимчука", "PROPN", "UD_Russian-SynTagRus", "у "),
            row("aafire", "X", "Debian-trixie-main-amd64", ""),
        ];
        std::fs::create_dir_all(folder).expect("a folder");
        let packed = gz(&rows);
        std::fs::write(folder.join("train.jsonl.gz"), &packed).expect("a split");
        write(&folder.join("manifest.json"), &BaseManifest { namespace: "small-base", splits: [("train", BaseSplit { path: "train.jsonl.gz", sha256: digest(&packed) })].into_iter().collect() });
    }

    /// A config of schema three: every class three words, a fifth of the words hard by their first keys, an acceptance split.
    fn config(directory: &str) -> WordConfig {
        let classes = [Class::Frequent, Class::Rare, Class::Name, Class::Unknown, Class::EnglishKeys, Class::LowKeys, Class::Term, Class::ShortTerm, Class::SignLed, Class::Address];
        let all: BTreeMap<Class, usize> = classes.into_iter().map(|class| (class, 3)).collect();
        let quotas = ["train", "development", "calibration", "acceptance"].into_iter().map(|split| (split.to_owned(), Quotas { en: all.clone(), ru: all.clone() })).collect();
        WordConfig {
            schema_version: 3,
            kind: "prefix-words".to_owned(),
            directory: directory.to_owned(),
            namespace: "test-words".to_owned(),
            minimum_word_length: 3,
            maximum_word_length: 16,
            frequent_words_per_language: 50,
            development_per_mille: 250,
            calibration_per_mille: 250,
            situations_per_word: 3,
            sign_leaders: "<\"'[{;:".to_owned(),
            english_keys_minimum_nats: 0.0,
            english_keys_maximum_characters: 6,
            minimum_calibration_negatives: 0,
            minimum_calibration_hard_negatives: 0,
            quotas,
            evidence_scope: "test".to_owned(),
            low_keys: Some(LowKeysRule { characters: 4, per_mille: 200 }),
            acceptance_per_mille: Some(250),
            minimum_calibration_words_per_direction: Some(0),
            minimum_calibration_hard_words_per_direction: Some(0),
        }
    }

    /// A focus word of a development sample.
    #[derive(Serialize)]
    struct Sample {
        original: &'static str,
    }

    #[derive(Deserialize)]
    struct HeldOutSummary {
        words: usize,
        files: usize,
    }

    /// What a test reads of a receipt.
    #[derive(Deserialize)]
    struct Receipt {
        kind: String,
        held_out: HeldOutSummary,
        family_overlap: usize,
        sha256: BTreeMap<String, String>,
        low_keys_cut_nats: Option<BTreeMap<String, f64>>,
    }

    #[test]
    fn a_word_corpus_is_frozen_once_and_verified_byte_for_byte() {
        let small = small_tree("words-freeze", &recipe());
        let folder = scratch("words-freeze-inputs");
        base(&folder.join("base"));
        let sample = folder.join("development.jsonl.gz");
        std::fs::write(&sample, gz(&[Sample { original: "garden" }])).expect("a sample");
        let list = folder.join("words.txt");
        std::fs::write(&list, "aafire\n").expect("a list");
        let config_path = folder.join("config.json");
        write(&config_path, &config("model/words-frozen"));
        let on = |extra: &[&str]| {
            let mut command = trainer();
            command.arg("prefix-corpus").arg("--tree").arg(&small).arg("--config").arg(&config_path).arg("--base").arg(folder.join("base")).args(extra);
            command.output().expect("the trainer starts")
        };
        let frozen = small.join("model/words-frozen");
        let frozen_text = frozen.display().to_string();
        refused(&on(&["--freeze", &frozen_text]), "--hold-out");
        let (sample_text, list_text) = (sample.display().to_string(), list.display().to_string());
        let said = succeeded(&on(&["--hold-out", &sample_text, "--hold-out", &list_text, "--freeze", &frozen_text]));
        assert!(said.contains("\"calibration\"") && said.contains("\"acceptance\""), "{said}");
        let receipt: Receipt = read(&frozen.join("corpus.json"));
        assert_eq!((receipt.kind.as_str(), receipt.held_out.words, receipt.held_out.files, receipt.family_overlap), ("prefix-words", 2, 2, 0));
        assert!(receipt.sha256.contains_key("acceptance") && receipt.low_keys_cut_nats.is_some_and(|cuts| cuts.contains_key("en") && cuts.contains_key("ru")));
        // The held-out words are in no split; the name, the address part and the Russian word are, the hard ones marked.
        let mut texts = Vec::new();
        for split in ["train", "development", "calibration", "acceptance"] {
            let rows = ks_train::prefix_words::read_split(&frozen, split).expect("a split");
            texts.extend(rows.into_iter().map(|row| (row.text, row.class, row.hard)));
        }
        assert!(!texts.iter().any(|(text, _, _)| ["garden", "aafire"].contains(&text.as_str())));
        for (text, class) in [("Kaplan", Class::Name), ("iepa.com", Class::Address), ("Тимчука", Class::Name)] {
            assert!(texts.iter().any(|(found, found_class, _)| found == text && *found_class == class), "{text}");
        }
        assert!(texts.iter().any(|(_, class, hard)| *class == Class::SignLed && *hard));
        let verified = succeeded(&on(&[]));
        assert_eq!(verified.matches(": same").count(), 5, "{verified}");
        refused(&on(&["--hold-out", &sample_text, "--freeze", &frozen_text]), "refusing to overwrite");
        std::fs::write(frozen.join("development.jsonl.gz"), gz(&[Sample { original: "" }])).expect("a split");
        refused(&on(&[]), "1 files of the word corpus differ");
        // A config of schema three names its hard words and its acceptance split, and one of schema two neither.
        write(&config_path, &WordConfig { acceptance_per_mille: None, ..config("model/words-half") });
        refused(&on(&["--hold-out", &sample_text, "--freeze", &small.join("model/words-half").display().to_string()]), "and only one");
        // A calibration smaller than the config asks for is no corpus; a config of schema two writes no hard words.
        write(&config_path, &WordConfig { schema_version: 2, low_keys: None, acceptance_per_mille: None, minimum_calibration_hard_negatives: 1_000_000, ..config("model/words-strict") });
        let strict_text = small.join("model/words-strict").display().to_string();
        refused(&on(&["--hold-out", &sample_text, "--freeze", &strict_text]), "calibration holds");
    }

    /// A row of a word corpus.
    fn word(text: &str, group: i64, class: Class, situation: Situation, family: &str) -> WordRow {
        let before = match situation {
            Situation::Empty => "",
            Situation::Comment => "// note ",
            Situation::RussianText => "я думаю ",
            Situation::EnglishText | Situation::Natural => "I think ",
        };
        WordRow {
            before: before.to_owned(),
            class,
            family: family.to_owned(),
            group,
            paired: class.paired(),
            role: situation.role().to_owned(),
            situation,
            source: "test".to_owned(),
            text: text.to_owned(),
            hard: false,
        }
    }

    /// The rows of a split: words of each language typed both ways in three situations, and a token led by a sign key; the
    /// words of `same` in every split, each split's under families of its own, or each split its own words.
    fn rows_of(split: &str, same: bool) -> Vec<WordRow> {
        let english = ["people", "garden", "window", "summer", "market", "doctor", "school", "friend"];
        let russian = ["погода", "дорога", "машина", "работа", "солнце", "письмо", "страна", "голова"];
        let (from, count) = match (same, split) {
            (true, _) | (false, "train") => (0, 4),
            (false, "development") => (4, 2),
            (false, _) => (6, 2),
        };
        let mut rows = Vec::new();
        for index in from..from + count {
            for situation in [Situation::Empty, Situation::RussianText, Situation::Comment] {
                rows.push(word(english[index], 0, Class::Rare, situation, &format!("{split}-en-{index}")));
                rows.push(word(russian[index], 1, Class::Rare, situation, &format!("{split}-ru-{index}")));
            }
            rows.push(word(&format!("<{}", english[index]), 0, Class::SignLed, Situation::EnglishText, &format!("{split}-sign-{index}")));
        }
        rows
    }

    #[derive(Serialize)]
    struct Pins {
        kind: &'static str,
        sha256: BTreeMap<&'static str, String>,
    }

    /// A small word corpus under `model/<name>` of a tree.
    fn small_word_corpus(small: &Path, name: &str, same: bool) {
        let corpus = small.join("model").join(name);
        std::fs::create_dir_all(&corpus).expect("a folder");
        let mut sha256 = BTreeMap::new();
        for split in ["train", "development", "calibration"] {
            let packed = gz(&rows_of(split, same));
            std::fs::write(corpus.join(format!("{split}.jsonl.gz")), &packed).expect("a split");
            sha256.insert(split, digest(&packed));
        }
        write(&corpus.join("corpus.json"), &Pins { kind: "prefix-words", sha256 });
        std::fs::write(corpus.join("config.json"), "{}").expect("a config");
        std::fs::write(corpus.join("held-out.json.gz"), gz::<Sample>(&[])).expect("a held-out file");
    }

    /// The entry of a key a report must have.
    fn at<'a, T>(map: &'a BTreeMap<String, T>, key: &str) -> &'a T {
        map.get(key).expect("a key the report has")
    }

    /// A value of a recipe, as JSON text.
    fn kept(text: &str) -> Kept {
        ks_text::json::loads(text).expect("a JSON value")
    }

    /// The prefix recipe of the tree over the corpus `model/<name>`, with these changes, written beside the tree.
    fn word_recipe(small: &Path, name: &str, changes: &[(&str, &str)]) -> PathBuf {
        let mut options: BTreeMap<String, Kept> = read(&small.join("model/prefix_v2/recipe.json"));
        options.insert("corpus".to_owned(), Kept::text(format!("model/{name}")));
        for (key, value) in changes {
            options.insert((*key).to_owned(), kept(value));
        }
        let path = small.join(format!("{name}-recipe.json"));
        write(&path, &options);
        path
    }

    #[derive(Deserialize)]
    struct ClassCounts {
        sequences: i64,
        desired: i64,
    }

    #[derive(Deserialize)]
    struct DirectionCounts {
        desired: i64,
        kept: i64,
    }

    #[derive(Deserialize)]
    struct ThresholdRow {
        calibration: [DirectionCounts; 2],
    }

    /// What a test reads of `classes.json`.
    #[derive(Deserialize)]
    struct Classes {
        stopped_after_epoch: Option<i64>,
        epoch: i64,
        thresholds: [f64; 2],
        calibration: BTreeMap<String, ClassCounts>,
        calibration_by_threshold: BTreeMap<String, BTreeMap<String, ClassCounts>>,
        by_threshold: BTreeMap<String, ThresholdRow>,
        development_false: BTreeMap<String, Vec<String>>,
        calibration_false: Option<Kept>,
    }

    #[test]
    fn a_fit_reads_a_word_corpus_each_row_in_its_own_situation_and_reports_each_class() {
        let small = small_tree("words-fit", &recipe());
        small_word_corpus(&small, "words-fit", false);
        let changes = [
            ("epochs", "6"), ("early_stop_patience", "1"), ("learning_rate", "0.0"), ("minimum_feature_mass", "1.0"), ("support", "\"engine\""), ("convert_from_characters", "3"),
            ("minimum_prefix_characters", "3"), ("wait_on_known_terms", "true"), ("importance_normalization", "\"mean_one\""),
        ];
        let recipe_path = word_recipe(&small, "words-fit", &changes);
        let fit = |recipe: &Path, output: &Path| trainer().arg("prefix").arg("--tree").arg(&small).arg("--recipe").arg(recipe).arg("--output").arg(output).args(["--jobs", "2"]).output().expect("the trainer starts");
        let output = scratch("words-fit-out").join("out");
        let answer = fit(&recipe_path, &output);
        let (said, stderr) = (String::from_utf8_lossy(&answer.stdout).into_owned(), String::from_utf8_lossy(&answer.stderr).into_owned());
        // Nothing moves at a learning rate of zero: the first epoch is chosen, and the second spends the patience.
        assert!(said.contains("prefix-v2 epoch 2") && !said.contains("prefix-v2 epoch 3"), "{said}{stderr}");
        #[derive(Deserialize)]
        struct Seal {
            provenance: BTreeMap<String, String>,
        }
        let seal: Seal = read(&output.join("seal.json"));
        assert!(seal.provenance.contains_key("model/words-fit/held-out.json.gz"));
        let classes: Classes = read(&output.join("classes.json"));
        assert_eq!((classes.stopped_after_epoch, classes.epoch), (Some(2), 1));
        assert_eq!((at(&classes.calibration, "sign_led:0").sequences, at(&classes.calibration, "rare:1").desired), (4, 12));
        assert_eq!(at(at(&classes.calibration_by_threshold, "0.985"), "rare:1").desired, 12);
        assert!(at(&classes.development_false, "0.985").is_empty());
        assert!(classes.calibration_false.is_none(), "calibration is reported in numbers only");
        assert_eq!(at(&classes.by_threshold, "0.985").calibration[1].desired, 12);
        assert_eq!(classes.thresholds[0], classes.thresholds[1]);
        // The report is made again from the scores saved beside the seal, without a fit.
        let again = output.parent().expect("a folder").join("classes-again.json");
        succeeded(&trainer().arg("prefix-report").arg("--fit").arg(&output).arg("--output").arg(&again).output().expect("the trainer starts"));
        assert_eq!(std::fs::read(&again).expect("a report"), std::fs::read(output.join("classes.json")).expect("the classes"));
        // A recipe that names a folder without a word corpus.
        let elsewhere = word_recipe(&small, "prefix_v1", &changes);
        refused(&fit(&elsewhere, &scratch("words-fit-none").join("out")), "holds no word corpus");
    }

    /// The changes of a recipe that chooses its epoch by the switch key, over a small corpus a model learns at once.
    const KEY_CHANGES: [(&str, &str); 9] = [
        ("epochs", "30"), ("early_stop_patience", "10"), ("learning_rate", "0.5"), ("minimum_feature_mass", "1.0"), ("support", "\"engine\""),
        ("importance_normalization", "\"mean_one\""), ("epoch_selection", "\"switch_key\""), ("minimum_calibration_words_per_direction", "2"),
        ("minimum_calibration_hard_words_per_direction", "0"),
    ];

    #[derive(Deserialize)]
    struct KeyChoice {
        share_by_key: f64,
    }

    #[derive(Deserialize)]
    struct Cell {
        words: usize,
        hard: usize,
    }

    #[derive(Deserialize)]
    struct Profiles {
        portable: [DirectionCounts; 2],
    }

    /// What a test reads of the seal of a fit by the switch key.
    #[derive(Deserialize)]
    struct KeySeal {
        threshold: f64,
        thresholds: [f64; 2],
        epoch_selection: KeyChoice,
        calibration_cells: [Cell; 2],
        development: Profiles,
    }

    #[derive(Deserialize)]
    struct Artifact {
        conversion_thresholds: [f64; 2],
    }

    /// What a test reads of a probe.
    #[derive(Deserialize)]
    struct Probe {
        words: usize,
        thresholds: [f64; 2],
        minimum_characters: usize,
        by_threshold: BTreeMap<String, [DirectionCounts; 2]>,
        false_texts: Vec<String>,
    }

    #[test]
    fn a_fit_by_the_switch_key_takes_a_threshold_for_each_direction_and_reads_its_frames_back() {
        let small = small_tree("words-key", &recipe());
        small_word_corpus(&small, "words-key", true);
        let recipe_path = word_recipe(&small, "words-key", &KEY_CHANGES);
        let cache = scratch("words-key-frames");
        let fit = |output: &Path| {
            let answer = trainer().arg("prefix").arg("--tree").arg(&small).arg("--recipe").arg(&recipe_path).arg("--output").arg(output).arg("--frames-cache").arg(&cache).args(["--jobs", "2"]).output().expect("the trainer starts");
            assert!(output.join("seal.json").exists(), "{}{}", String::from_utf8_lossy(&answer.stdout), String::from_utf8_lossy(&answer.stderr));
            String::from_utf8_lossy(&answer.stdout).into_owned()
        };
        let (first, second) = (scratch("words-key-first").join("out"), scratch("words-key-second").join("out"));
        let said = fit(&first);
        assert!(said.contains("switched by key 4") && !said.contains("read from the cache"), "{said}");
        let said = fit(&second);
        for split in ["train", "development", "calibration"] {
            assert!(said.contains(&format!("{split}: frames read from the cache")), "{said}");
        }
        for name in ["prefix-candidate.json", "seal.json", "classes.json", "scores.json.gz"] {
            assert_eq!(std::fs::read(first.join(name)).expect("a file"), std::fs::read(second.join(name)).expect("a file"), "{name}");
        }
        let (seal, artifact, classes): (KeySeal, Artifact, Classes) = (read(&first.join("seal.json")), read(&first.join("prefix-candidate.json")), read(&first.join("classes.json")));
        assert_eq!(seal.threshold, seal.thresholds[0].max(seal.thresholds[1]));
        assert_eq!((artifact.conversion_thresholds, classes.thresholds), (seal.thresholds, seal.thresholds));
        assert!(seal.epoch_selection.share_by_key > 0.0 && seal.development.portable[1].desired > 0);
        // Typed in Latin: four English words and four led by a sign key, which are hard; in Cyrillic four Russian words.
        assert_eq!(seal.calibration_cells.map(|cell| (cell.words, cell.hard)), [(8, 4), (4, 0)]);

        // The model measured without the engine: on development, on calibration (numbers only) and on a list of words.
        let probe = scratch("words-key-probe");
        let model = first.join("prefix-candidate.json");
        let probed = |extra: &[&str], output: &Path| trainer().arg("prefix-probe").arg("--tree").arg(&small).arg("--model").arg(&model).arg("--recipe").arg(&recipe_path).arg("--output").arg(output).args(extra).output().expect("the trainer starts");
        let development = probe.join("development.json");
        succeeded(&probed(&["--split", "development"], &development));
        let found: Probe = read(&development);
        assert_eq!((found.words, found.thresholds, found.minimum_characters, at(&found.by_threshold, "0.985")[0].desired), (28, seal.thresholds, 4, 24));
        let calibration = probe.join("calibration.json");
        succeeded(&probed(&["--split", "calibration", "--minimum-characters", "3", "--frames-cache", &cache.display().to_string()], &calibration));
        let found: Probe = read(&calibration);
        assert_eq!((found.false_texts.len(), found.minimum_characters), (0, 3));
        let list = probe.join("words.txt");
        std::fs::write(&list, "garden\n\nпогода\n").expect("a list");
        let listed = probe.join("listed.json");
        succeeded(&probed(&["--words", &list.display().to_string()], &listed));
        let found: Probe = read(&listed);
        assert_eq!((found.words, at(&found.by_threshold, "0.985")[0].desired, at(&found.by_threshold, "0.985")[1].kept), (2, 2, 2));
        std::fs::write(&list, "garden\n123\n").expect("a list");
        refused(&probed(&["--words", &list.display().to_string()], &listed), "opens with a letter of neither layout");
        refused(&probed(&["--split", "test"], &listed), "only exposed train/development/calibration");
    }

    #[test]
    fn a_fit_on_a_calibration_cell_smaller_than_its_recipe_asks_is_refused() {
        let small = small_tree("words-small-cell", &recipe());
        small_word_corpus(&small, "words-small-cell", true);
        let mut changes = KEY_CHANGES.to_vec();
        changes.push(("minimum_calibration_words_per_direction", "1000"));
        let recipe_path = word_recipe(&small, "words-small-cell", &changes);
        let answer = trainer().arg("prefix").arg("--tree").arg(&small).arg("--recipe").arg(&recipe_path).arg("--output").arg(scratch("words-small-cell-out").join("out")).output().expect("the trainer starts");
        refused(&answer, "calibration typed in layout group 0 holds 8 words, 4 hard: the recipe asks for 1000 and 0");
    }
}

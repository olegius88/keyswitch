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

/// A prefix model of a schema whose only weight is the bias, at a value that converts every prefix it is asked about; its
/// version names itself.
fn eager_model(path: &Path, feature_version: u8) -> String {
    let weights = r#"{"bias":[0,40,0,0]}"#;
    let sum = digest(weights.as_bytes());
    let version = format!("prefix-v{feature_version}-{}", sum.get(..12).expect("a digest"));
    let text = format!(
        r#"{{"actions":["keep","convert","wait","suggest"],"conversion_threshold":0.985,"feature_version":{feature_version},"kind":"keyswitch.prefix-policy","prefix_feature_version":{feature_version},"version":"{version}","weights":{weights},"weights_sha256":"{sum}"}}"#
    );
    std::fs::write(path, text).expect("a model");
    version
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
    // A seal whose fields are not of their types is no seal of this tree's either.
    let text = String::from_utf8(std::fs::read(&seal_path).expect("the seal")).expect("text").replacen("\"calibration_passed\":", "\"calibration_passed\":5,\"was\":", 1);
    own_file(&seal_path, text.as_bytes());
    let output = scratch("prefix-reuse-untyped").join("out");
    refused(&prefix_fit(&small, &path, &output, true), "");
    assert!(!output.exists());
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

    /// The same, read back to be changed.
    #[derive(Serialize, Deserialize)]
    struct ReadManifest {
        namespace: String,
        splits: BTreeMap<String, ReadSplit>,
    }

    #[derive(Serialize, Deserialize)]
    struct ReadSplit {
        path: String,
        sha256: String,
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
            // Rows that give no word: a command of no lower Latin letters or none at all, a source of no treebank, a word
            // too short, names not written as names (no capital, a capital inside, a capital of two letters), a word of
            // no one layout; and an address whose short part is too short.
            row("Xvfb", "X", "Debian-trixie-main-amd64", ""),
            row("", "X", "Debian-trixie-main-amd64", ""),
            row("garden", "NOUN", "Tatoeba", ""),
            row("an", "DET", "UD_English-EWT", ""),
            row("kaplan", "PROPN", "UD_English-EWT", ""),
            row("ДеВито", "PROPN", "UD_Russian-SynTagRus", ""),
            row("ßtrasse", "PROPN", "UD_English-EWT", ""),
            row("naïve", "ADJ", "UD_English-EWT", ""),
            row("http://ab.example.org", "X", "UD_English-EWT", "See "),
            row("x@yz.com", "X", "UD_English-EWT", "Mail "),
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
            minimum_words_per_direction: Some(0),
            minimum_hard_words_per_direction: Some(0),
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
        // The tree's protected tokens, with a comment and an empty line, which are no tokens.
        let protected = small.join("resources/protected_tokens.txt");
        let tokens = std::fs::read_to_string(&protected).expect("the protected tokens");
        super::own_file(&protected, format!("# a comment\n\n{tokens}").as_bytes());
        let folder = scratch("words-freeze-inputs");
        base(&folder.join("base"));
        let sample = folder.join("development.jsonl.gz");
        std::fs::write(&sample, gz(&[Sample { original: "garden" }])).expect("a sample");
        // A word listed twice is held out once.
        let list = folder.join("words.txt");
        std::fs::write(&list, "aafire\naafire\n").expect("a list");
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
        // Made again from its sources into a new folder, the corpus has the tree's receipt; from other held-out words not.
        let again = scratch("words-freeze-again").join("again");
        let rebuilt = succeeded(&on(&["--hold-out", &sample_text, "--hold-out", &list_text, "--rebuild", &again.display().to_string()]));
        assert!(rebuilt.contains("corpus.json: same as the tree's"), "{rebuilt}");
        let other = scratch("words-freeze-other").join("other");
        refused(&on(&["--hold-out", &sample_text, "--rebuild", &other.display().to_string()]), "the rebuilt word corpus differs from its receipt");
        refused(&on(&["--hold-out", &sample_text, "--freeze", &frozen_text]), "refusing to overwrite");
        // A freeze into a folder that has its config keeps that one.
        let kept = small.join("model/words-kept");
        std::fs::create_dir_all(&kept).expect("a folder");
        std::fs::write(kept.join("config.json"), "{}").expect("a config");
        succeeded(&on(&["--hold-out", &sample_text, "--freeze", &kept.display().to_string()]));
        assert_eq!(std::fs::read_to_string(kept.join("config.json")).expect("the config"), "{}");
        std::fs::write(frozen.join("development.jsonl.gz"), gz(&[Sample { original: "" }])).expect("a split");
        refused(&on(&[]), "1 files of the word corpus differ");
        assert!(ks_train::prefix_words::read_split(&frozen, "development").unwrap_err().contains("checksum mismatch"));
        assert!(ks_train::prefix_words::read_split(&frozen, "test").unwrap_err().contains("no such split"));
        let receipt_bytes = std::fs::read(frozen.join("corpus.json")).expect("the receipt");
        std::fs::write(frozen.join("corpus.json"), [receipt_bytes.as_slice(), b"\n"].concat()).expect("a receipt");
        let answer = on(&[]);
        refused(&answer, "2 files of the word corpus differ");
        assert!(String::from_utf8_lossy(&answer.stdout).contains("corpus.json: DIFFERS"));
        // A config of no word corpus, or one without its sign keys.
        for other in [WordConfig { kind: "other".to_owned(), ..config("model/words-other") }, WordConfig { schema_version: 5, ..config("model/words-other") }] {
            write(&config_path, &other);
            refused(&on(&["--hold-out", &sample_text, "--freeze", &small.join("model/words-other").display().to_string()]), "not a word corpus config");
        }
        write(&config_path, &WordConfig { sign_leaders: String::new(), ..config("model/words-signs") });
        refused(&on(&["--hold-out", &sample_text, "--freeze", &small.join("model/words-signs").display().to_string()]), "a word corpus needs sign leaders");
        // A base whose training split is not the one its manifest names, or holds a row of another split.
        write(&config_path, &config("model/words-bases"));
        for (name, digest_changed) in [("changed", true), ("mixed", false)] {
            let other = folder.join(name);
            base(&other);
            let mut manifest: ReadManifest = read(&other.join("manifest.json"));
            let train = manifest.splits.get_mut("train").expect("a training split");
            if digest_changed {
                train.sha256 = "0".repeat(64);
            } else {
                let rows = [BaseRow { original: "garden", upos: "NOUN", source: "UD_English-EWT", before: "", split: "development", after: "" }];
                let packed = gz(&rows);
                std::fs::write(other.join("train.jsonl.gz"), &packed).expect("a split");
                train.sha256 = digest(&packed);
            }
            write(&other.join("manifest.json"), &manifest);
            let mut command = trainer();
            command.arg("prefix-corpus").arg("--tree").arg(&small).arg("--config").arg(&config_path).arg("--base").arg(&other);
            let answer = command.args(["--hold-out", &sample_text, "--freeze", &small.join("model/words-bases").display().to_string()]).output().expect("the trainer starts");
            refused(&answer, if digest_changed { "differs from its manifest" } else { "a row of another split" });
        }
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

    /// The changes of a recipe that chooses its epoch by the mean key, over a small corpus a model learns at once. Its floor
    /// is the installed model at the top of the grid, where it converts nothing: any epoch is as early.
    const KEY_CHANGES: [(&str, &str); 11] = [
        ("epochs", "30"), ("early_stop_patience", "10"), ("learning_rate", "0.5"), ("minimum_feature_mass", "1.0"), ("support", "\"engine\""),
        ("importance_normalization", "\"mean_one\""), ("epoch_selection", "\"mean_key\""), ("minimum_words_per_direction", "2"),
        ("minimum_hard_words_per_direction", "0"), ("floor_model", "\"resources/models/prefix_policy_v1.json\""), ("floor_threshold", "1.0"),
    ];

    /// `KEY_CHANGES` and the version of the installed model the floor names.
    fn key_recipe(small: &Path, name: &str, extra: &[(&str, &str)]) -> PathBuf {
        #[derive(Deserialize)]
        struct Installed {
            version: String,
        }
        let installed: Installed = read(&small.join("resources/models/prefix_policy_v1.json"));
        let version = format!("\"{}\"", installed.version);
        let mut changes = KEY_CHANGES.to_vec();
        changes.push(("floor_model_version", version.as_str()));
        changes.extend_from_slice(extra);
        word_recipe(small, name, &changes)
    }

    #[derive(Deserialize)]
    struct KeyChoice {
        measure: f64,
        counts: bool,
    }

    #[derive(Deserialize)]
    struct Floor {
        model_version: String,
        mean_key: [f64; 2],
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
        development_thresholds: [f64; 2],
        calibration_read: bool,
        calibration_passed: bool,
        floor: Floor,
        epoch_selection: KeyChoice,
        calibration_cells: [Cell; 2],
        development_cells: [Cell; 2],
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
        false_words: [usize; 2],
        false_texts: Vec<String>,
    }

    #[test]
    fn a_fit_by_the_mean_key_takes_a_threshold_for_each_direction_and_reads_its_frames_back() {
        let small = small_tree("words-key", &recipe());
        small_word_corpus(&small, "words-key", true);
        let recipe_path = key_recipe(&small, "words-key", &[]);
        let cache = scratch("words-key-frames");
        let fit = |output: &Path, extra: &[&str]| {
            let answer = trainer().arg("prefix").arg("--tree").arg(&small).arg("--recipe").arg(&recipe_path).arg("--output").arg(output).arg("--frames-cache").arg(&cache).args(["--jobs", "2"]).args(extra).output().expect("the trainer starts");
            assert!(output.join("seal.json").exists(), "{}{}", String::from_utf8_lossy(&answer.stdout), String::from_utf8_lossy(&answer.stderr));
            (answer.status.success(), String::from_utf8_lossy(&answer.stdout).into_owned())
        };
        // A fit that compares recipes reads no calibration: its thresholds are development's, and it succeeds unpassed.
        let probe = scratch("words-key-development").join("out");
        let (succeeded_unread, said) = fit(&probe, &["--development-only"]);
        assert!(succeeded_unread && !said.contains("calibration:"), "{said}");
        let unread: KeySeal = read(&probe.join("seal.json"));
        assert!(!unread.calibration_read && !unread.calibration_passed && unread.thresholds.map(f64::to_bits) == unread.development_thresholds.map(f64::to_bits));
        let (first, second) = (scratch("words-key-first").join("out"), scratch("words-key-second").join("out"));
        let (_, said) = fit(&first, &[]);
        // The development-only fit made the frames of training and development, not those of calibration.
        assert!(said.contains("mean key") && said.contains("development: frames read from the cache") && !said.contains("calibration: frames read from the cache"), "{said}");
        let (_, said) = fit(&second, &[]);
        for split in ["train", "development", "calibration"] {
            assert!(said.contains(&format!("{split}: frames read from the cache")), "{said}");
        }
        for name in ["prefix-candidate.json", "seal.json", "classes.json", "scores.json.gz"] {
            assert_eq!(std::fs::read(first.join(name)).expect("a file"), std::fs::read(second.join(name)).expect("a file"), "{name}");
        }
        let (seal, artifact, classes): (KeySeal, Artifact, Classes) = (read(&first.join("seal.json")), read(&first.join("prefix-candidate.json")), read(&first.join("classes.json")));
        assert_eq!(seal.threshold, seal.thresholds[0].max(seal.thresholds[1]));
        assert_eq!((artifact.conversion_thresholds, classes.thresholds), (seal.thresholds, seal.thresholds));
        assert!(seal.epoch_selection.measure > 0.0 && seal.epoch_selection.counts && seal.development.portable[1].desired > 0);
        assert!(seal.calibration_read && seal.thresholds.iter().zip(seal.development_thresholds).all(|(found, floor)| *found >= floor));
        // The floor converts nothing at the top of the grid: every word typed in the other layout counts its boundary.
        assert!(!seal.floor.model_version.is_empty() && seal.floor.mean_key.iter().all(|key| *key > 4.0));
        assert!(seal.development_cells.map(|cell| cell.words).iter().all(|words| *words > 0));
        // Typed in Latin: four English words and four led by a sign key, which are hard; in Cyrillic four Russian words.
        assert_eq!(seal.calibration_cells.map(|cell| (cell.words, cell.hard)), [(8, 4), (4, 0)]);

        // The model measured without the engine: on development, from the third letter in numbers only, and on a list of
        // words; never on calibration, which the fit read once, or on any other split.
        let probe = scratch("words-key-probe");
        let model = first.join("prefix-candidate.json");
        let probed = |extra: &[&str], output: &Path| trainer().arg("prefix-probe").arg("--tree").arg(&small).arg("--model").arg(&model).arg("--recipe").arg(&recipe_path).arg("--output").arg(output).args(extra).output().expect("the trainer starts");
        let development = probe.join("development.json");
        succeeded(&probed(&["--split", "development"], &development));
        let found: Probe = read(&development);
        assert_eq!((found.words, found.thresholds, found.minimum_characters, at(&found.by_threshold, "0.985")[0].desired), (28, seal.thresholds, 4, 24));
        let third = probe.join("third.json");
        succeeded(&probed(&["--split", "development", "--minimum-characters", "3", "--numbers-only", "--frames-cache", &cache.display().to_string()], &third));
        let found: Probe = read(&third);
        assert_eq!((found.false_texts.len(), found.minimum_characters), (0, 3));
        for split in ["calibration", "acceptance", "test"] {
            refused(&probed(&["--split", split], &probe.join("refused.json")), &format!("a probe reads development or a list of words, not {split}"));
        }
        let list = probe.join("words.txt");
        std::fs::write(&list, "garden\n\nпогода\n").expect("a list");
        let listed = probe.join("listed.json");
        succeeded(&probed(&["--words", &list.display().to_string()], &listed));
        let found: Probe = read(&listed);
        assert_eq!((found.words, at(&found.by_threshold, "0.985")[0].desired, at(&found.by_threshold, "0.985")[1].kept), (2, 2, 2));
        // The words typed as meant that the model converts, each once, by the layout they were typed in.
        assert!(found.false_words.iter().zip([1, 1]).all(|(words, most)| *words <= most));
        assert_eq!(found.false_words.iter().sum::<usize>(), found.false_texts.len());
        std::fs::write(&list, "garden\n123\n").expect("a list");
        refused(&probed(&["--words", &list.display().to_string()], &listed), "opens with a letter of neither layout");
    }

    #[test]
    fn a_fit_on_a_cell_smaller_than_its_recipe_asks_or_without_its_floor_is_refused() {
        let small = small_tree("words-small-cell", &recipe());
        small_word_corpus(&small, "words-small-cell", true);
        let fit = |recipe_path: &Path, name: &str| trainer().arg("prefix").arg("--tree").arg(&small).arg("--recipe").arg(recipe_path).arg("--output").arg(scratch(name).join("out")).output().expect("the trainer starts");
        let recipe_path = key_recipe(&small, "words-small-cell", &[("minimum_words_per_direction", "1000")]);
        refused(&fit(&recipe_path, "words-small-cell-out"), "development typed in layout group 0 holds");
        refused(&fit(&recipe_path, "words-small-cell-out"), "the recipe asks for 1000 and 0");
        let recipe_path = key_recipe(&small, "words-small-cell", &[("floor_model_version", "\"prefix-v2-000000000000\"")]);
        refused(&fit(&recipe_path, "words-floor-version"), "the recipe names prefix-v2-000000000000");
        let recipe_path = word_recipe(&small, "words-small-cell", &KEY_CHANGES);
        refused(&fit(&recipe_path, "words-floor-none"), "a fit by the mean key names its floor");
        // Only a fit by the mean key leaves calibration unread.
        let recipe_path = word_recipe(&small, "words-small-cell", &[("epochs", "2")]);
        let answer = trainer().arg("prefix").arg("--tree").arg(&small).arg("--recipe").arg(&recipe_path).arg("--output").arg(scratch("words-unread").join("out")).arg("--development-only").output().expect("the trainer starts");
        refused(&answer, "only a fit by the mean key leaves calibration unread");
        // A floor no epoch reaches: a model that converts every prefix at its fourth key.
        let eager = small.join("eager-floor.json");
        let version = format!("\"{}\"", super::eager_model(&eager, 2));
        let recipe_path = key_recipe(&small, "words-small-cell", &[("floor_model", "\"eager-floor.json\""), ("floor_model_version", &version), ("learning_rate", "0.0"), ("epochs", "2")]);
        refused(&fit(&recipe_path, "words-floor-eager"), "no epoch is as early as the installed model in both directions");
    }

    /// The frames of schemas three and four, read with the orthotactic models and the term lists, closed to a share of the
    /// training families; the probe of such a model and of one fitted on the prefix-v1 corpus; and what a word corpus, a
    /// list of known terms and an installed seal may not be.
    #[test]
    fn schemas_three_and_four_read_the_orthotactic_models_and_the_term_lists_in_a_fit_and_a_probe() {
        let small = small_tree("words-schemas", &recipe());
        small_word_corpus(&small, "words-schemas", false);
        let recipe_path = word_recipe(&small, "words-schemas", &[("runtime_feature_version", "4"), ("closed_term_families_per_mille", "1000"), ("epochs", "1"), ("minimum_feature_mass", "1.0"), ("support", "\"engine\"")]);
        let output = scratch("words-schemas-fit").join("out");
        // Whether its epochs find a threshold or not, the fit makes the frames of schema four, every training family's
        // without the term lists.
        let answer = trainer().arg("prefix").arg("--tree").arg(&small).arg("--recipe").arg(&recipe_path).arg("--output").arg(&output).args(["--jobs", "2"]).output().expect("the trainer starts");
        let said = String::from_utf8_lossy(&answer.stdout);
        assert!(said.contains("train: ") && said.contains("prefix-v2 epoch 1"), "{said}{}", String::from_utf8_lossy(&answer.stderr));
        // Models that convert every prefix: whatever is typed as meant, each word converted once.
        let list = scratch("words-schemas-list").join("words.txt");
        std::fs::write(&list, "garden\nпогода\n").expect("a list");
        for schema in [3, 4] {
            let model = scratch("words-schemas-models").join(format!("eager-{schema}.json"));
            super::eager_model(&model, schema);
            let probed = scratch("words-schemas-probe").join(format!("probe-{schema}.json"));
            succeeded(&trainer().arg("prefix-probe").arg("--tree").arg(&small).arg("--model").arg(&model).arg("--recipe").arg(&recipe_path).arg("--words").arg(&list).arg("--numbers-only").arg("--output").arg(&probed).output().expect("the trainer starts"));
            let found: Probe = read(&probed);
            assert_eq!((found.false_words, found.false_texts.len()), ([1, 1], 0), "{schema}");
        }
        // A probe of a recipe that names no word corpus reads the prefix-v1 corpus.
        let plain = scratch("words-schemas-plain").join("probe.json");
        let installed = small.join("resources/models/prefix_policy_v1.json");
        succeeded(&trainer().arg("prefix-probe").arg("--tree").arg(&small).arg("--model").arg(&installed).arg("--recipe").arg(small.join("model/prefix_v2/recipe.json")).arg("--split").arg("development").arg("--output").arg(&plain).output().expect("the trainer starts"));
        refused(&trainer().arg("prefix-probe").arg("--tree").arg(&small).arg("--model").arg(&installed).arg("--recipe").arg(&recipe_path).arg("--output").arg(&plain).output().expect("the trainer starts"), "one of --split and --words is required");
        // The parents of a word corpus: of its fitting splits only, and hard when their row says so.
        let corpus = small.join("model/words-schemas");
        assert!(ks_train::prefix::load_word_parents(&corpus, "acceptance").unwrap_err().contains("only exposed train/development/calibration"));
        let hard = small.join("model/words-hard");
        std::fs::create_dir_all(&hard).expect("a folder");
        let packed = gz(&[WordRow { hard: true, ..word("погода", 1, Class::Rare, Situation::Empty, "pog") }]);
        std::fs::write(hard.join("train.jsonl.gz"), &packed).expect("a split");
        write(&hard.join("corpus.json"), &Pins { kind: "prefix-words", sha256: [("train", digest(&packed))].into_iter().collect() });
        assert!(ks_train::prefix::load_word_parents(&hard, "train").expect("the parents").iter().all(|parent| parent.hard));
        // Known terms leave out comments and empty lines of the protected tokens.
        let resources = scratch("words-schemas-terms");
        std::fs::write(resources.join("identifiers.json"), r#"{"identifiers": ["Kubectl"]}"#).expect("identifiers");
        std::fs::write(resources.join("protected_tokens.txt"), "# a comment\n\ngit\n").expect("tokens");
        assert_eq!(ks_train::prefix::known_terms(&resources).expect("the terms"), ["git", "kubectl"]);
        // A report of no fit, and an inventory of the corpora named.
        refused(&trainer().arg("prefix-report").arg("--fit").arg(scratch("words-schemas-nothing")).arg("--output").arg(scratch("words-schemas-report").join("r.json")).output().expect("the trainer starts"), "scores.json.gz");
        let inventory = scratch("words-schemas-inventory").join("inventory.json");
        succeeded(&trainer().args(["base", "inventory"]).arg("--tree").arg(&small).arg("--corpus").arg(&corpus).arg("--output").arg(&inventory).output().expect("the trainer starts"));
        assert!(inventory.is_file());
    }
}

/// The acceptance samples of a prefix candidate: built once from a word corpus's acceptance split and from plain text,
/// without the words held out, and replayed through the engine for each pair named, the report in numbers only.
mod acceptance {
    use flate2::Compression;
    use flate2::read::MultiGzDecoder;
    use flate2::write::GzEncoder;
    use ks_train::prefix_acceptance::{AcceptanceReport, AcceptanceRow, Sample, SamplesReceipt};
    use ks_train::prefix_words::{Class, Situation, WordRow};
    use serde::Serialize;
    use std::collections::BTreeMap;
    use std::io::{BufRead as _, Write as _};
    use std::path::{Path, PathBuf};
    use super::support::{corpus, digest, recipe, refused, scratch, small_tree, succeeded, trainer};

    fn gz(text: &str) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(text.as_bytes()).expect("in memory");
        encoder.finish().expect("in memory")
    }

    fn lines<T: Serialize>(rows: &[T]) -> String {
        rows.iter().map(|row| ks_text::json::dumps(row, &ks_text::JsonOptions::default()) + "\n").collect()
    }

    fn read<T: for<'a> serde::Deserialize<'a>>(path: &Path) -> T {
        ks_text::json::loads(std::fs::read(path).expect("a file the test reads")).expect("JSON of the shape the test reads")
    }

    /// A word of the acceptance split, in an empty field.
    fn word(text: &str, group: i64, class: Class, hard: bool) -> WordRow {
        WordRow {
            before: String::new(),
            class,
            family: format!("family-{text}"),
            group,
            paired: class.paired(),
            role: Situation::Empty.role().to_owned(),
            situation: Situation::Empty,
            source: "test".to_owned(),
            text: text.to_owned(),
            hard,
        }
    }

    #[derive(Serialize)]
    struct Pins {
        kind: &'static str,
        sha256: BTreeMap<&'static str, String>,
    }

    /// A word corpus with an acceptance split: words of both languages typed both ways, one of them hard, a name, a term
    /// and a token led by a sign key, which are typed only as meant.
    fn word_corpus(folder: &Path) {
        std::fs::create_dir_all(folder).expect("a folder");
        let rows = [
            word("garden", 0, Class::Rare, false),
            word("window", 0, Class::Rare, false),
            word("Kaplan", 0, Class::Name, false),
            word("погода", 1, Class::Rare, false),
            // Its row says it is hard by its first keys: A3 types it too.
            word("машина", 1, Class::Rare, true),
            word("kubectl", 0, Class::Term, false),
            word("<window", 0, Class::SignLed, false),
            // A letter no layout has a key for: the replay cannot type it, and the samples leave it out.
            word("café", 0, Class::Rare, false),
            // A word in a second situation is the same word.
            WordRow { situation: Situation::Comment, before: "// ".to_owned(), ..word("garden", 0, Class::Rare, false) },
        ];
        let packed = gz(&lines(&rows));
        std::fs::write(folder.join("acceptance.jsonl.gz"), &packed).expect("a split");
        let pins = Pins { kind: "prefix-words", sha256: [("acceptance", digest(&packed))].into_iter().collect() };
        std::fs::write(folder.join("corpus.json"), ks_text::json::dumps(&pins, &ks_text::JsonOptions::default())).expect("a receipt");
    }

    #[derive(Serialize)]
    struct Inventory {
        aliases_sha256: Vec<String>,
    }

    /// A prefix model of schema four whose only weight is the bias, at a value that converts every prefix it is asked about.
    fn eager_model(path: &Path) {
        super::eager_model(path, 4);
    }

    /// The entry of a key a report must have.
    fn at<'a, T>(map: &'a BTreeMap<String, T>, key: &str) -> &'a T {
        map.get(key).expect("a key the report has")
    }

    fn rows_of(folder: &Path) -> Vec<AcceptanceRow> {
        let file = std::fs::File::open(folder.join("rows.jsonl.gz")).expect("the rows");
        std::io::BufReader::new(MultiGzDecoder::new(file)).lines().map(|line| ks_text::json::loads(line.expect("a line")).expect("a row")).collect()
    }

    struct Inputs {
        small: PathBuf,
        corpus: PathBuf,
        base: PathBuf,
        text: PathBuf,
        packed_text: PathBuf,
        inventory: PathBuf,
        list: PathBuf,
    }

    fn inputs(name: &str) -> Inputs {
        let small = small_tree(name, &recipe());
        let folder = scratch(&format!("{name}-inputs"));
        let corpus_folder = folder.join("words");
        word_corpus(&corpus_folder);
        let base = folder.join("base");
        corpus(&base);
        let text = folder.join("text.txt");
        std::fs::write(&text, "Мама мыла раму, «quick» brown fox 123abc: жёлтый!\n").expect("a text");
        let packed_text = folder.join("text.txt.gz");
        std::fs::write(&packed_text, gz("Длинная история about everything\n")).expect("a text");
        // An inventory names `hello` by its aliases, a list names `today`: neither is a word of plain text then.
        let inventory = folder.join("inventory.json");
        let aliases: Vec<String> = ks_train::physical::expanded_aliases("hello").into_iter().collect();
        std::fs::write(&inventory, ks_text::json::dumps(&Inventory { aliases_sha256: aliases }, &ks_text::JsonOptions::default())).expect("an inventory");
        let list = folder.join("held.txt");
        std::fs::write(&list, "today\n").expect("a list");
        Inputs { small, corpus: corpus_folder, base, text, packed_text, inventory, list }
    }

    fn build(inputs: &Inputs, output: &Path, minimums: [&str; 3]) -> std::process::Output {
        let mut command = trainer();
        command.arg("prefix-acceptance").arg("build").arg("--corpus").arg(&inputs.corpus).arg("--base").arg(&inputs.base).arg("--base-calibration");
        command.arg("--text").arg(&inputs.text).arg("--text").arg(&inputs.packed_text).arg("--inventory").arg(&inputs.inventory).arg("--hold-out").arg(&inputs.list);
        command.args(["--namespace", "test-acceptance", "--minimum-paired", minimums[0], "--minimum-plain", minimums[1], "--engine-plain", minimums[2]]).arg("--output").arg(output);
        command.output().expect("the trainer starts")
    }

    #[test]
    fn the_trainer_builds_the_term_lists_an_engine_of_schema_four_builds() {
        // The acceptance replay hands its engines the lists the trainer built, as the frames of a fit read them.
        let mut bundle = ks_replay::harness::Bundle::load(None);
        let eager = scratch("acceptance-lists").join("eager.json");
        eager_model(&eager);
        bundle.set_prefix_model(Some(std::sync::Arc::new(ks_context::VersionedPrefixModel::load(&eager).expect("the model"))));
        let session = bundle.session("Editor", 0, Vec::new());
        let engine = session.core.prefix_terms().expect("the lists of an engine of schema four");
        let resources = ks_replay::harness::reference_tree().join("resources");
        let trainer = ks_train::prefix::prefix_terms_of(&resources, &bundle.models().to_vec()).expect("the lists");
        assert!(engine.len()[0] > 1000 && *engine == trainer);
        bundle.give_back(session);
        bundle.clean();
    }

    #[test]
    fn the_samples_are_built_once_without_the_words_held_out_and_replayed_in_numbers_only() {
        let inputs = inputs("acceptance");
        let output = scratch("acceptance-samples").join("samples");
        succeeded(&build(&inputs, &output, ["2", "3", "2"]));
        let receipt: SamplesReceipt = read(&output.join("receipt.json"));
        let rows = rows_of(&output);
        let count = |sample: Sample, text: &str, other: bool| rows.iter().filter(|row| row.sample == sample && row.text == text && row.other == other).count();
        let has = |sample: Sample, text: &str, other: bool| count(sample, text, other) > 0;
        // A1 and A2 hold the paired words, A3 the tokens typed only as meant and the hard words; a word of two situations
        // is one.
        assert!(has(Sample::A1, "garden", true) && has(Sample::A2, "garden", false) && has(Sample::A1, "Kaplan", true) && has(Sample::A3, "машина", false));
        assert!(has(Sample::A3, "kubectl", false) && has(Sample::A3, "<window", false) && has(Sample::A3, "Kaplan", false));
        assert!(!has(Sample::A1, "kubectl", true) && !has(Sample::A3, "погода", false));
        assert_eq!(count(Sample::A1, "garden", true), 1);
        // Plain text: words of four letters or more of one layout, case-folded; not those held out or of the acceptance
        // split.
        for (text, there) in [("мама", true), ("quick", true), ("длинная", true), ("people", true), ("fox", false), ("123abc", false), ("hello", false), ("today", false), ("погода", false)] {
            assert_eq!(has(Sample::N2, text, false), there, "{text}");
        }
        // N1: the words whose keys read as letters in the other layout too.
        assert!(has(Sample::N1, "мама", true) && !has(Sample::N1, "жёлтый", true) && has(Sample::N2, "жёлтый", false));
        for direction in 0..2 {
            assert_eq!(at(&receipt.counts, "N2").engine[direction], 2);
            assert_eq!(rows.iter().filter(|row| row.sample == Sample::N1 && row.engine && row.direction() == direction).count(), 2);
        }
        assert!(rows.iter().filter(|row| row.sample != Sample::N1 && row.sample != Sample::N2).all(|row| row.engine));
        let listed = std::fs::read_to_string(output.join("n2-words.txt")).expect("the words of N2");
        assert_eq!(listed.lines().count(), at(&receipt.counts, "N2").words.iter().sum::<usize>());
        // The development and calibration splits of the base, never its training or test split; N2 by its sources, the
        // words of the base's development documents also in a list of their own.
        assert_eq!((receipt.bases.len(), receipt.texts.len(), receipt.inventories.len(), receipt.hold_out.len()), (2, 2, 1, 1));
        let of_development = std::fs::read_to_string(output.join("n2-development-words.txt")).expect("the words of N2 of the development documents");
        assert_eq!(of_development.lines().count(), at(&receipt.plain_by_source, "development").iter().sum::<usize>());
        assert!(["calibration", "text"].iter().all(|source| at(&receipt.plain_by_source, source).iter().sum::<usize>() > 0));
        assert!(of_development.lines().all(|word| listed.lines().any(|found| found == word)));
        // The word of a letter no layout has is in no sample: counted in A1, typed in Cyrillic, and in A2, typed in Latin.
        assert!(!rows.iter().any(|row| row.text.contains('é')));
        assert_eq!((at(&receipt.untyped, "A1"), at(&receipt.untyped, "A2")), (&[0, 1], &[1, 0]));
        refused(&build(&inputs, &output, ["2", "3", "2"]), "refusing to overwrite");
        refused(&build(&inputs, &scratch("acceptance-small").join("samples"), ["2", "100000", "2"]), "the samples ask for");

        // The replay: the installed pair, and the same context with a prefix model that converts every prefix.
        let eager = scratch("acceptance-eager").join("eager.json");
        eager_model(&eager);
        let models = inputs.small.join("resources/models");
        let installed = format!("installed={},{}", models.join("context_policy_v1.json").display(), models.join("prefix_policy_v1.json").display());
        let early = format!("eager={},{}", models.join("context_policy_v1.json").display(), eager.display());
        let run = |pairs: &[&str], reference: &str, report: &Path, extra: &[&str]| {
            let mut command = trainer();
            command.arg("prefix-acceptance").arg("run").arg("--tree").arg(&inputs.small).arg("--samples").arg(&output);
            for pair in pairs {
                command.arg("--pair").arg(pair);
            }
            command.args(["--reference", reference, "--jobs", "2"]).args(extra).arg("--output").arg(report);
            command.output().expect("the trainer starts")
        };
        let report_path = scratch("acceptance-report").join("report.json");
        let said = succeeded(&run(&[&installed, &early], "installed", &report_path, &[]));
        assert!(said.contains("eager: gates held false"), "{said}");
        let report: AcceptanceReport = read(&report_path);
        assert_eq!(report.gates.keys().map(String::as_str).collect::<Vec<_>>(), ["eager"]);
        let (reference, candidate) = (at(&report.pairs, "installed"), at(&report.pairs, "eager"));
        for pair in [reference, candidate] {
            assert!(pair.safety_by_sample.contains_key("N2") && pair.safety_by_profile.len() == 2);
            assert_eq!(at(&pair.safety_by_sample, "A3_closed")[0].rows, at(&pair.safety_by_sample, "A3")[0].rows);
            assert_eq!(at(&pair.earliness, "A1")[0].rows, 2 * at(&receipt.counts, "A1").words[0]);
        }
        // A model that converts every prefix switches words typed as meant away: more false early switches than the
        // installed one, and its gates do not hold.
        assert!(at(&candidate.safety, "all").false_early.words > at(&reference.safety, "all").false_early.words);
        let held = at(&report.gates, "eager");
        assert!(!at(&held.s1, "all").false_early_not_more && !held.all_held && held.execution);
        assert!(!at(&held.s1_alone, "A2")[0].false_early_not_more && held.s1_alone.contains_key("N2") && held.s2_corruption_alone.len() == 2);
        // The report names no text of a sample.
        let written = std::fs::read_to_string(&report_path).expect("the report");
        assert!(!["garden", "мама", "kubectl", "quick"].iter().any(|text| written.contains(text)));
        refused(&run(&[&installed], "installed", &report_path, &[]), "refusing to overwrite");
        refused(&run(&[&installed], "nobody", &scratch("acceptance-nobody").join("report.json"), &[]), "no pair is named nobody");
        // Samples that hold a row the replay cannot type fail the run.
        let untypable = scratch("acceptance-untypable").join("samples");
        std::fs::create_dir_all(&untypable).expect("a folder");
        let packed = gz(&lines(&[AcceptanceRow { sample: Sample::A2, text: "café".to_owned(), group: 0, class: "rare".to_owned(), other: false, engine: true }]));
        std::fs::write(untypable.join("rows.jsonl.gz"), &packed).expect("rows");
        let mut pinned: SamplesReceipt = read(&output.join("receipt.json"));
        pinned.sha256.insert("rows.jsonl.gz".to_owned(), digest(&packed));
        std::fs::write(untypable.join("receipt.json"), ks_text::json::dumps(&pinned, &ks_text::JsonOptions::default())).expect("a receipt");
        let mut command = trainer();
        command.arg("prefix-acceptance").arg("run").arg("--tree").arg(&inputs.small).arg("--samples").arg(&untypable).arg("--pair").arg(&installed);
        refused(&command.args(["--reference", "installed"]).arg("--output").arg(scratch("acceptance-untyped").join("report.json")).output().expect("the trainer starts"), "1 rows of the samples cannot be typed");
        // A pair whose context model is none.
        let broken = format!("broken={},{}", eager.display(), models.join("prefix_policy_v1.json").display());
        refused(&run(&[&broken], "broken", &scratch("acceptance-broken").join("report.json"), &[]), &eager.display().to_string());
        refused(&trainer().arg("prefix-acceptance").output().expect("the trainer starts"), "prefix-acceptance build or run");
        // A folder whose receipt is of something else.
        let elsewhere = scratch("acceptance-elsewhere").join("samples");
        std::fs::create_dir_all(&elsewhere).expect("a folder");
        let mut other: SamplesReceipt = read(&output.join("receipt.json"));
        other.kind = "other".to_owned();
        std::fs::write(elsewhere.join("receipt.json"), ks_text::json::dumps(&other, &ks_text::JsonOptions::default())).expect("a receipt");
        let mut command = trainer();
        command.arg("prefix-acceptance").arg("run").arg("--tree").arg(&inputs.small).arg("--samples").arg(&elsewhere).arg("--pair").arg(&installed);
        refused(&command.args(["--reference", "installed"]).arg("--output").arg(scratch("acceptance-other").join("report.json")).output().expect("the trainer starts"), "holds no acceptance samples");
        // Rows whose bytes are not those of the receipt.
        std::fs::write(output.join("rows.jsonl.gz"), gz("")).expect("rows");
        refused(&run(&[&installed], "installed", &scratch("acceptance-changed").join("report.json"), &[]), "differ from their receipt");
    }
}

//! The word corpus of the prefix model: frozen from a small base and the tree's sources, verified byte for byte, refused
//! over a frozen generation or without held-out samples; and a fit over a word corpus, which reads each row in its own
//! situation, stops when its patience is spent and reports each class.
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "these tests build their small files as untyped trees, as the other tests of this crate do")]
mod support;

use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::{Value, json};
use std::io::Write as _;
use std::path::Path;
use support::{digest, recipe, refused, scratch, small_tree, succeeded, trainer};

fn gz(lines: &[Value]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    for line in lines {
        encoder.write_all(format!("{line}\n").as_bytes()).expect("in memory");
    }
    encoder.finish().expect("in memory")
}

/// A base corpus of one training split: treebank words, a name, a Debian command and an address.
fn base(folder: &Path) {
    let row = |original: &str, upos: &str, source: &str, before: &str| json!({"original": original, "upos": upos, "source": source, "before": before, "split": "train", "after": ""});
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
    std::fs::write(folder.join("manifest.json"), json!({"namespace": "small-base", "splits": {"train": {"path": "train.jsonl.gz", "sha256": digest(&packed)}}}).to_string()).expect("a manifest");
}

fn config(directory: &str) -> Value {
    let all = json!({"frequent": 3, "rare": 3, "name": 3, "unknown": 3, "english_keys": 3, "term": 3, "short_term": 3, "sign_led": 3, "address": 3});
    json!({
        "schema_version": 2, "kind": "prefix-words", "directory": directory, "namespace": "test-words", "minimum_word_length": 3, "maximum_word_length": 16,
        "frequent_words_per_language": 50, "development_per_mille": 300, "calibration_per_mille": 300, "situations_per_word": 3, "sign_leaders": "<\"'[{;:",
        "english_keys_minimum_nats": 0.0, "english_keys_maximum_characters": 6, "minimum_calibration_negatives": 0, "minimum_calibration_hard_negatives": 0,
        "quotas": {"train": {"en": all, "ru": all}, "development": {"en": all, "ru": all}, "calibration": {"en": all, "ru": all}}, "evidence_scope": "test"
    })
}

#[test]
fn a_word_corpus_is_frozen_once_and_verified_byte_for_byte() {
    let small = small_tree("words-freeze", &recipe());
    let folder = scratch("words-freeze-inputs");
    base(&folder.join("base"));
    let sample = folder.join("development.jsonl.gz");
    std::fs::write(&sample, gz(&[json!({"original": "garden"})])).expect("a sample");
    let config_path = folder.join("config.json");
    std::fs::write(&config_path, config("model/words-frozen").to_string()).expect("a config");
    let on = |extra: &[&str]| {
        let mut command = trainer();
        command.arg("prefix-corpus").arg("--tree").arg(&small).arg("--config").arg(&config_path).arg("--base").arg(folder.join("base")).args(extra);
        command.output().expect("the trainer starts")
    };
    let frozen = small.join("model/words-frozen");
    let frozen_text = frozen.display().to_string();
    refused(&on(&["--freeze", &frozen_text]), "--hold-out");
    let sample_text = sample.display().to_string();
    let said = succeeded(&on(&["--hold-out", &sample_text, "--freeze", &frozen_text]));
    assert!(said.contains("\"calibration\""), "{said}");
    let receipt: Value = serde_json::from_slice(&std::fs::read(frozen.join("corpus.json")).expect("a receipt")).expect("json");
    assert_eq!((receipt["kind"].clone(), receipt["held_out"]["words"].clone(), receipt["family_overlap"].clone()), (json!("prefix-words"), json!(1), json!(0)));
    // The held-out word is in no split; the name, the address part and the command are.
    let mut texts = Vec::new();
    for split in ["train", "development", "calibration"] {
        let rows = ks_train::prefix_words::read_split(&frozen, split).expect("a split");
        texts.extend(rows.into_iter().map(|row| (row.text, row.class)));
    }
    assert!(!texts.iter().any(|(text, _)| text == "garden"));
    for (text, class) in [("Kaplan", "name"), ("iepa.com", "address"), ("Тимчука", "name")] {
        assert!(texts.iter().any(|(found, found_class)| found == text && found_class.as_str() == class), "{text}");
    }
    let verified = succeeded(&on(&[]));
    assert_eq!(verified.matches(": same").count(), 4, "{verified}");
    refused(&on(&["--hold-out", &sample_text, "--freeze", &frozen_text]), "refusing to overwrite");
    std::fs::write(frozen.join("development.jsonl.gz"), gz(&[json!({})])).expect("a split");
    refused(&on(&[]), "1 files of the word corpus differ");
    // A calibration smaller than the config asks for is no corpus.
    let mut strict = config("model/words-strict");
    strict["minimum_calibration_hard_negatives"] = json!(1_000_000);
    std::fs::write(&config_path, strict.to_string()).expect("a config");
    let strict_text = small.join("model/words-strict").display().to_string();
    refused(&on(&["--hold-out", &sample_text, "--freeze", &strict_text]), "calibration holds");
}

/// A row of a word corpus.
fn word(text: &str, group: i64, class: &str, paired: bool, situation: &str, family: &str) -> Value {
    let role = match situation {
        "empty" => "unknown",
        "comment" => "code",
        _ => "text",
    };
    let before = match situation {
        "empty" => "",
        "comment" => "// note ",
        "russian_text" => "я думаю ",
        _ => "I think ",
    };
    json!({"before": before, "class": class, "family": family, "group": group, "paired": paired, "role": role, "situation": situation, "source": "test", "text": text})
}

fn rows_of(split: &str) -> Vec<Value> {
    let english = ["people", "garden", "window", "summer", "market", "doctor", "school", "friend"];
    let russian = ["погода", "дорога", "машина", "работа", "солнце", "письмо", "страна", "голова"];
    let (from, count) = match split {
        "train" => (0, 4),
        "development" => (4, 2),
        _ => (6, 2),
    };
    let mut rows = Vec::new();
    for index in from..from + count {
        for situation in ["empty", "russian_text", "comment"] {
            rows.push(word(english[index], 0, "rare", true, situation, &format!("{split}-en-{index}")));
            rows.push(word(russian[index], 1, "rare", true, situation, &format!("{split}-ru-{index}")));
        }
        rows.push(word(&format!("<{}", english[index]), 0, "sign_led", false, "english_text", &format!("{split}-sign-{index}")));
    }
    rows
}

#[test]
fn a_fit_reads_a_word_corpus_each_row_in_its_own_situation_and_reports_each_class() {
    let small = small_tree("words-fit", &recipe());
    let corpus = small.join("model/words-fit");
    std::fs::create_dir_all(&corpus).expect("a folder");
    let mut pins = json!({});
    for split in ["train", "development", "calibration"] {
        let packed = gz(&rows_of(split));
        std::fs::write(corpus.join(format!("{split}.jsonl.gz")), &packed).expect("a split");
        pins[split] = json!(digest(&packed));
    }
    std::fs::write(corpus.join("corpus.json"), json!({"kind": "prefix-words", "sha256": pins}).to_string()).expect("a receipt");
    std::fs::write(corpus.join("config.json"), "{}").expect("a config");
    std::fs::write(corpus.join("held-out.json.gz"), gz(&[])).expect("a held-out file");
    let mut options: Value = serde_json::from_slice(&std::fs::read(small.join("model/prefix_v2/recipe.json")).expect("the recipe")).expect("json");
    for (name, value) in [
        ("corpus", json!("model/words-fit")), ("epochs", json!(6)), ("early_stop_patience", json!(1)), ("learning_rate", json!(0.0)), ("minimum_feature_mass", json!(1.0)),
        ("support", json!("engine")), ("convert_from_characters", json!(3)), ("minimum_prefix_characters", json!(3)), ("wait_on_known_terms", json!(true)),
        ("importance_normalization", json!("mean_one")),
    ] {
        options[name] = value;
    }
    let recipe_path = small.join("words-recipe.json");
    std::fs::write(&recipe_path, options.to_string()).expect("a recipe");
    let fit = |recipe: &Path, output: &Path| trainer().arg("prefix").arg("--tree").arg(&small).arg("--recipe").arg(recipe).arg("--output").arg(output).args(["--jobs", "2"]).output().expect("the trainer starts");
    let output = scratch("words-fit-out").join("out");
    let answer = fit(&recipe_path, &output);
    let said = String::from_utf8_lossy(&answer.stdout).into_owned();
    // Nothing moves at a learning rate of zero: the first epoch is chosen, and the second spends the patience.
    assert!(said.contains("prefix-v2 epoch 2") && !said.contains("prefix-v2 epoch 3"), "{said}{}", String::from_utf8_lossy(&answer.stderr));
    let seal: Value = serde_json::from_slice(&std::fs::read(output.join("seal.json")).expect("a seal")).expect("json");
    assert!(seal["provenance"]["model/words-fit/held-out.json.gz"].is_string());
    let classes: Value = serde_json::from_slice(&std::fs::read(output.join("classes.json")).expect("the classes")).expect("json");
    assert_eq!((classes["stopped_after_epoch"].clone(), classes["epoch"].clone()), (json!(2), json!(1)));
    assert_eq!(classes["calibration"]["sign_led:0"]["sequences"], json!(4));
    assert_eq!(classes["calibration"]["rare:1"]["desired"], json!(12));
    assert_eq!(classes["calibration_by_threshold"]["0.985"]["rare:1"]["desired"], json!(12));
    assert_eq!(classes["calibration_false"]["0.985"], json!([]));
    // A recipe that names a folder without a word corpus.
    options["corpus"] = json!("model/prefix_v1");
    std::fs::write(&recipe_path, options.to_string()).expect("a recipe");
    refused(&fit(&recipe_path, &scratch("words-fit-none").join("out")), "holds no word corpus");
}

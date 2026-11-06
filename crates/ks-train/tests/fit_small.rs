//! A whole training run on a small corpus: the frames of every curriculum, the features, the epochs, the seal.
mod support;

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Output;
use support::{corpus, recipe, refused, scratch, trainer, tree, write_recipe};

struct Run {
    corpus: PathBuf,
    ledger: PathBuf,
    folder: PathBuf,
}

fn prepared(name: &str) -> Run {
    let folder = scratch(name);
    let run = Run { corpus: folder.join("corpus"), ledger: folder.join("ledger"), folder };
    corpus(&run.corpus);
    std::fs::create_dir_all(&run.ledger).expect("a folder");
    run
}

fn fitted(run: &Run, name: &str, options: &Value) -> Output {
    let recipe_path = run.folder.join(format!("{name}-recipe.json"));
    write_recipe(&recipe_path, options);
    let output = run.folder.join(name);
    trainer()
        .arg("fit")
        .arg("--tree")
        .arg(tree())
        .arg("--corpus")
        .arg(&run.corpus)
        .arg("--output")
        .arg(&output)
        .arg("--recipe")
        .arg(&recipe_path)
        .arg("--ledger")
        .arg(&run.ledger)
        .args(["--jobs", "2"])
        .output()
        .expect("the trainer starts")
}

#[test]
fn a_recipe_that_cannot_be_is_refused_before_anything_is_written() {
    let run = prepared("refused");
    let mut bad = recipe();
    bad["schema_version"] = json!(9);
    assert!(!fitted(&run, "bad-schema", &bad).status.success());
    let mut warm = recipe();
    warm["warm_base"] = warm["frozen_base"].clone();
    refused(&fitted(&run, "warm", &warm), "without kept-neighbour weights");
    let mut capitals = recipe();
    capitals["english_capital_curriculum"]["words_by_length"]["3"] = json!(5);
    refused(&fitted(&run, "capitals", &capitals), "English-capital");
    let mut counted = recipe();
    counted["counted_token_curriculum"]["term_frames"] = json!(5);
    refused(&fitted(&run, "counted", &counted), "counted-token");
    // A folder that is there already is never written over.
    std::fs::create_dir_all(run.folder.join("there")).expect("a folder");
    refused(&fitted(&run, "there", &recipe()), "never overwrite");
}

#[test]
fn a_small_corpus_fits_over_the_frozen_base() {
    let run = prepared("frozen");
    let answer = fitted(&run, "frozen", &recipe());
    assert!(answer.status.success(), "{}", String::from_utf8_lossy(&answer.stderr));
    assert!(run.folder.join("frozen/context-action.json").is_file());
}

#[test]
fn a_small_corpus_fits_from_nothing() {
    let run = prepared("whole");
    let mut options = recipe();
    options.as_object_mut().expect("an object").remove("frozen_base");
    let answer = fitted(&run, "whole", &options);
    assert!(answer.status.success(), "{}", String::from_utf8_lossy(&answer.stderr));
}

fn base_of(path: &Path) -> Value {
    json!({"artifact": path, "sha256": support::digest(&std::fs::read(path).expect("a model"))})
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

/// The base of the shipped pair with two heads taken out, one fitted in the common epochs and one with its own.
fn carried_without(run: &Run, heads: &[&str]) -> PathBuf {
    let output = run.folder.join("carried.json");
    let root = tree();
    let pair = root.join(recipe()["frozen_base"]["artifact"].as_str().expect("a path"));
    let mut command = trainer();
    command.arg("carry").arg("--tree").arg(&root).arg("--pair").arg(&pair).arg("--output").arg(&output);
    for head in heads {
        command.args(["--refit", head]);
    }
    let made = command.output().expect("the trainer starts");
    assert!(made.status.success(), "{}", said(&made));
    output
}

#[test]
fn heads_are_fitted_onto_a_base_that_lacks_them() {
    let run = prepared("heads");
    let carried = carried_without(&run, &["letter", "term"]);
    let mut options = recipe();
    let labels = options["frozen_base"]["labels"].clone();
    let alone = options["frozen_base"]["carried"].clone();
    options["frozen_base"] = base_of(&carried);
    options["frozen_base"]["labels"] = labels;
    options["frozen_base"]["carried"] = alone;
    options["frozen_base"]["heads"] = json!({"letter": {"maximum_features": 300}, "term": {"maximum_features": 300}});
    options["term_curriculum"]["item_modulus"] = json!(1);
    let answer = fitted(&run, "heads", &options);
    let text = said(&answer);
    assert!(text.contains("epoch") || text.contains("no epoch repaired"), "{text}");
    // A head of the base cannot be fitted again, nor one named as carried be missing from it.
    let mut again = options.clone();
    again["frozen_base"]["heads"] = json!({"brief": {"maximum_features": 300}});
    refused(&fitted(&run, "heads-again", &again), "frozen with it");
    let mut missing = options.clone();
    missing["frozen_base"]["carried"] = json!({"letter": {}});
    refused(&fitted(&run, "heads-missing", &missing), "no weights for");
    let mut unknown = options.clone();
    unknown["frozen_base"]["heads"] = json!({"nothing": {"maximum_features": 1}});
    refused(&fitted(&run, "heads-unknown", &unknown), "no head nothing");
    let mut strange = options;
    strange["frozen_base"]["carried"] = json!({"nothing": {}});
    refused(&fitted(&run, "heads-strange", &strange), "no head nothing");
}

#[test]
fn a_warm_base_starts_a_fit_and_stands_alone() {
    let run = prepared("warm");
    let mut whole = recipe();
    whole.as_object_mut().expect("an object").remove("frozen_base");
    whole["epochs"] = json!(1);
    whole["english_capital_curriculum"]["words_by_length"] = json!({"3": 3, "4": 2});
    whole["english_capital_curriculum"]["russian_contrast"] = json!(true);
    whole["counted_token_curriculum"]["term_frames"] = json!(6);
    whole["counted_token_curriculum"]["abbreviation_frames"] = json!(6);
    whole.as_object_mut().expect("an object").remove("letter_curriculum");
    for budget in ["natural_frames", "term_frames", "abbreviation_frames", "typo_frames"] {
        whole["kept_neighbour_curriculum"][budget] = json!(0);
    }
    let first = fitted(&run, "first", &whole);
    assert!(first.status.success(), "{}", said(&first));
    // Without what a whole run keeps of its own: the kept-neighbour weights and the heads.
    let mut plain_model: Value = serde_json::from_slice(&std::fs::read(run.folder.join("first/context-action.json")).expect("a model")).expect("json");
    plain_model["weights"].as_object_mut().expect("weights").retain(|name, _| !name.starts_with(ks_context::constants::KEPT_FEATURE_PREFIX) && !ks_train::heads::HEAD_PREFIXES.iter().any(|head| name.starts_with(head.1)));
    let dumped = ks_text::json_dumps(&plain_model["weights"], &ks_text::JsonOptions { ensure_ascii: false, sort_keys: true, separators: Some((",".to_owned(), ":".to_owned())), ..ks_text::JsonOptions::default() });
    let hash = support::digest(dumped.as_bytes());
    plain_model["version"] = json!(format!("context-v3-{}", hash.get(..12).expect("a hash")));
    plain_model["weights_sha256"] = json!(hash);
    let plain = run.folder.join("plain.json");
    std::fs::write(&plain, serde_json::to_vec(&plain_model).expect("json")).expect("a file");
    let mut mixed = whole.clone();
    mixed["warm_base"] = base_of(&run.folder.join("first/context-action.json"));
    refused(&fitted(&run, "warm-with-heads", &mixed), "without kept-neighbour weights or heads");
    let mut warm = whole.clone();
    warm["warm_base"] = base_of(&plain);
    warm["warm_base"]["learning_rate"] = json!(0.01);
    let answer = fitted(&run, "warmed", &warm);
    println!("{}", said(&answer));
    let mut both = warm;
    both["frozen_base"] = base_of(&plain);
    refused(&fitted(&run, "both", &both), "not both");
}

#[test]
fn what_a_recipe_and_a_corpus_name_has_to_hold() {
    let run = prepared("names");
    let root = tree();
    let mut wrong = recipe();
    wrong["frozen_base"]["sha256"] = json!("0");
    refused(&fitted(&run, "wrong-digest", &wrong), "differs from the recipe");
    let mut old = recipe();
    old["frozen_base"] = base_of(&root.join("model/context_v3/baseline-context-v1.json"));
    refused(&fitted(&run, "old-model", &old), "is an action model");
    let mut version = recipe();
    version["feature_version"] = json!(1);
    refused(&fitted(&run, "old-recipe", &version), "invalid action recipe");
    let mut band = recipe();
    band["threshold_selection"]["maximum_threshold"] = json!(0.0);
    refused(&fitted(&run, "no-thresholds", &band), "serving band");
    // The ledger names a test read before, and the corpus how many it was frozen against.
    let ledger = run.folder.join("ledger");
    std::fs::write(ledger.join("read.access.json"), serde_json::to_vec(&json!({"test_membership": {"alias_sha256": ["a".repeat(64)]}})).expect("json")).expect("a record");
    std::fs::write(ledger.join("other.access.json"), b"{}").expect("a record");
    std::fs::write(ledger.join("notes.txt"), b"").expect("a file");
    let manifest_path = run.corpus.join("manifest.json");
    let bytes = std::fs::read(&manifest_path).expect("a manifest");
    let mut manifest: Value = serde_json::from_slice(&bytes).expect("json");
    manifest["metadata"] = json!({"refused_test_aliases": 5});
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).expect("json")).expect("a manifest");
    refused(&fitted(&run, "frozen-against", &recipe()), "the test ledger");
    manifest["metadata"] = json!({"refused_test_aliases": 1});
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).expect("json")).expect("a manifest");
    let _ = bytes;
    // A split with nothing in it.
    let empty = run.folder.join("empty-corpus");
    support::corpus_of(&empty, 0);
    let recipe_path = run.folder.join("empty-recipe.json");
    write_recipe(&recipe_path, &recipe());
    let answer = trainer().arg("fit").arg("--tree").arg(&root).arg("--corpus").arg(&empty).arg("--output").arg(run.folder.join("empty-output")).arg("--recipe").arg(&recipe_path).arg("--ledger").arg(&ledger).output().expect("the trainer starts");
    refused(&answer, "empty fitting split");
}

//! Whole fits on the small corpus that end where a recipe sends them: heads whose epochs repair nothing, a warm start a
//! gate rejects, a frozen base of its own with no labelling model, and two bases at once.
mod support;

use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::process::Output;
use support::{corpus, digest, recipe, refused, scratch, trainer, tree, write_recipe};

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
    trainer()
        .arg("fit")
        .arg("--tree")
        .arg(tree())
        .arg("--corpus")
        .arg(&run.corpus)
        .arg("--output")
        .arg(run.folder.join(name))
        .arg("--recipe")
        .arg(&recipe_path)
        .arg("--ledger")
        .arg(&run.ledger)
        .args(["--jobs", "2"])
        .output()
        .expect("the trainer starts")
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn read(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("a file")).expect("json")
}

fn base_of(path: &Path) -> Value {
    json!({"artifact": path, "sha256": digest(&std::fs::read(path).expect("a model"))})
}

/// An action model of these weights, written as a fit writes one.
fn model_of(path: &Path, weights: Map<String, Value>, threshold: f64) -> PathBuf {
    let weights = Value::Object(weights);
    let dumped = ks_text::json_dumps(&weights, &ks_text::JsonOptions { ensure_ascii: false, sort_keys: true, separators: Some((",".to_owned(), ":".to_owned())), ..ks_text::JsonOptions::default() });
    let hash = digest(dumped.as_bytes());
    let actions: Vec<&str> = ks_context::context_model::ACTIONS.iter().map(|action| action.as_str()).collect();
    let payload = json!({"actions": actions, "feature_version": 3, "weights": weights, "weights_sha256": hash, "version": format!("context-v3-{}", hash.get(..12).expect("a hash")), "conversion_threshold": threshold});
    std::fs::write(path, ks_train::artifact::artifact_bytes(&payload)).expect("a model");
    path.to_path_buf()
}

/// A model of a few plain features of the small corpus, without kept-neighbour weights or heads.
fn small_model(path: &Path) -> PathBuf {
    let weights: Map<String, Value> = ["bias", "source_known", "target_known"].iter().map(|name| ((*name).to_owned(), json!([0.5, -0.25, 0.125, -0.375]))).collect();
    model_of(path, weights, 0.99)
}

/// The shipped pair without what a whole run keeps of its own: the kept-neighbour weights and the heads.
fn plain_pair(path: &Path) -> PathBuf {
    let pair = read(&tree().join(recipe()["frozen_base"]["artifact"].as_str().expect("a path")));
    let mut weights = pair["weights"].as_object().expect("weights").clone();
    weights.retain(|name, _| !name.starts_with(ks_context::constants::KEPT_FEATURE_PREFIX) && !ks_train::heads::HEAD_PREFIXES.iter().any(|head| name.starts_with(head.1)));
    model_of(path, weights, pair["conversion_threshold"].as_f64().expect("a threshold"))
}

#[test]
fn heads_whose_epochs_repair_nothing_are_not_sealed() {
    let run = prepared("whole-heads");
    let carried = run.folder.join("carried.json");
    let pair = tree().join(recipe()["frozen_base"]["artifact"].as_str().expect("a path"));
    let made = trainer().arg("carry").arg("--tree").arg(tree()).arg("--pair").arg(&pair).arg("--output").arg(&carried).args(["--refit", "term", "--refit", "quote"]).output().expect("the trainer starts");
    assert!(made.status.success(), "{}", said(&made));
    let mut options = recipe();
    let (labels, alone) = (options["frozen_base"]["labels"].clone(), options["frozen_base"]["carried"].clone());
    options["frozen_base"] = base_of(&carried);
    options["frozen_base"]["labels"] = labels;
    options["frozen_base"]["carried"] = alone;
    // The quote head has no curriculum of its own: it has no frames of DEVELOPMENT to choose its epoch on.
    options["frozen_base"]["heads"] = json!({"term": {"maximum_features": 300}, "quote": {"maximum_features": 300}});
    options.as_object_mut().expect("an object").remove("quote_curriculum");
    options["term_curriculum"]["item_modulus"] = json!(1);
    options["first_curriculum"]["enabled"] = json!(false);
    // Epochs that move no weight leave every loss where it was, and no epoch repairs that much.
    options["learning_rate"] = json!(0.0);
    options["epoch_selection"]["minimum_net_benefit_per_profile"] = json!(1_000_000);
    let answer = fitted(&run, "heads", &options);
    refused(&answer, "no epoch repaired more than it broke on development");
    let text = said(&answer);
    assert!(text.contains("epoch 2: term_development_loss") && !text.contains("quote_development_loss"), "{text}");
}

#[test]
fn a_warm_start_that_calibration_rejects_is_sealed_as_rejected() {
    let run = prepared("whole-warm");
    let plain = plain_pair(&run.folder.join("plain.json"));
    let mut options = recipe();
    options.as_object_mut().expect("an object").remove("frozen_base");
    options["epochs"] = json!(1);
    options["warm_base"] = base_of(&plain);
    options["warm_base"]["learning_rate"] = json!(0.01);
    options["epoch_selection"]["minimum_net_benefit_per_profile"] = json!(0);
    options["gate_policy"]["minimum_calibration_net_benefit"] = json!(1_000_000);
    let answer = fitted(&run, "warm", &options);
    let path = run.folder.join("warm/candidate-seal.json");
    assert!(!answer.status.success() && path.is_file(), "{}", said(&answer));
    let seal = read(&path);
    assert_eq!(seal["stage"], json!("rejected-before-test"));
    assert_eq!(seal["warm_base"]["learning_rate"], json!(0.01));
    assert_eq!(seal["warm_base"]["sha256"], options["warm_base"]["sha256"]);
    assert_eq!(seal["frozen_base"], Value::Null);
}

#[test]
fn a_frozen_base_of_its_own_needs_no_labelling_model_and_no_second_base() {
    let run = prepared("whole-frozen");
    let small = small_model(&run.folder.join("small.json"));
    let other = small_model(&run.folder.join("other.json"));
    let mut options = recipe();
    options["frozen_base"] = base_of(&small);
    options["epochs"] = json!(1);
    let mut both = options.clone();
    both["warm_base"] = base_of(&other);
    both["warm_base"]["learning_rate"] = json!(0.01);
    refused(&fitted(&run, "both", &both), "not both");
    let answer = fitted(&run, "frozen", &options);
    let path = run.folder.join("frozen/candidate-seal.json");
    assert!(path.is_file(), "{}", said(&answer));
    let seal = read(&path);
    assert_eq!(seal["frozen_base"]["sha256"], options["frozen_base"]["sha256"]);
    assert!(seal["frozen_base"].get("labels").is_none());
    assert!(seal.get("lone_word_curriculum").is_none());
    let model = read(&run.folder.join("frozen/context-action.json"));
    assert_eq!(model["weights"]["bias"], json!([0.5, -0.25, 0.125, -0.375]));
}

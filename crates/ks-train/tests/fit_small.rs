//! A whole training run on a small corpus: the frames of every curriculum, the features, the epochs, the seal.
mod support;

use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::{Command, Output};
use support::{corpus, recipe, scratch, tree, write_recipe};

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

fn trainer() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ks-train"));
    command.env("KEYSWITCH_HUNSPELL_PATH", tree().join("model/intent_v1/sources/hunspell"));
    command
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

fn refused(output: &Output, words: &str) {
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success() && said.contains(words), "{words}: {said}");
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

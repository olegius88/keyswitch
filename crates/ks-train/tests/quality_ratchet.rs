//! What a release may not lose: the floors of the installed pair's receipt and of the engine replays. A new pair raises them
//! by hand in `tests/data/quality_floors.json`, on purpose.
use ks_train::corpus::read_json;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn floors() -> Value {
    read_json(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/quality_floors.json")).unwrap()
}

fn receipt() -> Value {
    read_json(&root().join("model/context_v3/release-receipt.json")).unwrap()
}

fn number(value: &Value) -> f64 {
    value.as_f64().unwrap_or_else(|| panic!("not a number: {value}"))
}

/// The candidate and baseline counts of every profile, by settings mode.
fn modes(receipt: &Value) -> Vec<(String, &'static str, Value, Value)> {
    let mut found = Vec::new();
    for (name, profile) in receipt["test"]["profiles"].as_object().unwrap() {
        found.push((name.clone(), "default", profile["counts"]["candidate"].clone(), profile["counts"]["baseline"].clone()));
        found.push((name.clone(), "early_off", profile["early_off"]["counts"]["candidate"].clone(), profile["early_off"]["counts"]["baseline"].clone()));
    }
    found
}

#[test]
fn the_receipt_passed_its_own_gates() {
    let (receipt, floors) = (receipt(), floors());
    assert_eq!(receipt["quality_gates_passed"], Value::Bool(true));
    assert!(number(&receipt["conversion_threshold"]) >= number(&floors["RECEIPT_MIN_CONVERSION_THRESHOLD"]));
    assert!(number(&receipt["prefix_conversion_threshold"]) >= number(&floors["RECEIPT_MIN_PREFIX_CONVERSION_THRESHOLD"]));
}

#[test]
fn the_installed_pair_passed_the_chat_test_against_the_pair_it_replaced() {
    let (receipt, floors) = (receipt(), floors());
    let block = &receipt["devchat_test"];
    assert!(block["floors"].as_object().unwrap().values().all(|held| *held == Value::Bool(true)));
    assert_eq!(block["pairs"]["released"]["model_version"], floors["RECEIPT_CALIBRATION_REFERENCE_MODEL"]);
    assert_eq!(block["pairs"]["candidate"]["sha256"], receipt["artifact_sha256"]);
}

#[test]
fn the_installed_pair_corrupts_nothing_and_executes_every_row() {
    let (receipt, floors) = (receipt(), floors());
    for (name, mode, candidate, _) in modes(&receipt) {
        let label = format!("{name} {mode}");
        assert!(number(&candidate["correct_text_corruptions"]) <= number(&floors["RECEIPT_MAX_CORRUPTIONS"]), "{label}");
        assert_eq!(candidate["preserved_correct"], candidate["initially_correct"], "{label}");
        assert!(number(&candidate["execution_errors"]) <= number(&floors["RECEIPT_MAX_EXECUTION_ERRORS"]), "{label}");
        assert!(number(&candidate["length_mismatches"]) <= number(&floors["RECEIPT_MAX_LENGTH_MISMATCHES"]), "{label}");
        assert!(number(&candidate["correction_layout_mismatches"]) <= number(&floors["RECEIPT_MAX_CORRECTION_LAYOUT_MISMATCHES"]), "{label}");
    }
}

#[test]
fn the_installed_pair_keeps_its_margin_and_its_share() {
    let (receipt, floors) = (receipt(), floors());
    for (name, mode, candidate, baseline) in modes(&receipt) {
        let label = format!("{name} {mode}");
        let margin = number(&candidate["exactly_restored"]) - number(&baseline["exactly_restored"]);
        assert!(margin >= number(&floors["RECEIPT_MIN_RESTORATION_MARGIN"][mode]), "{label}: margin {margin}");
        if let Some(share) = floors["RECEIPT_MIN_RESTORED_SHARE"].get(mode) {
            assert!(number(&candidate["exactly_restored"]) / number(&candidate["initially_wrong"]) >= number(share), "{label}");
        }
    }
}

#[test]
fn calibration_recall_holds_and_the_pair_nets_at_least_the_one_it_replaced() {
    let (receipt, floors) = (receipt(), floors());
    let calibration = &receipt["calibration"];
    assert!(number(&calibration["outside_lone_word"]["conversion_recall"]) >= number(&floors["RECEIPT_MIN_CALIBRATION_RECALL"]));
    // The replaced pair was measured on these very rows; another corpus needs it measured anew.
    assert_eq!(receipt["corpus_manifest_sha256"], floors["RECEIPT_CALIBRATION_REFERENCE_CORPUS"]);
    assert_ne!(receipt["model_version"], floors["RECEIPT_CALIBRATION_REFERENCE_MODEL"]);
    for (name, reference) in floors["RECEIPT_CALIBRATION_REFERENCE_NET"].as_object().unwrap() {
        let profile = &calibration["by_profile"][name];
        assert!(number(&profile["converted_correctly"]) - number(&profile["false_conversions"]) >= number(reference), "{name}");
    }
}

#[test]
fn the_prefix_replay_restores_at_least_as_much_as_shipped() {
    let (report, floors) = (read_json(&root().join("model/prefix_v1/engine-report.json")).unwrap(), floors());
    assert_eq!(report["passed"], Value::Bool(true));
    for (name, variants) in report["results"].as_object().unwrap() {
        let (candidate, without) = (&variants["candidate"], &variants["shipping_no_prefix"]);
        assert!(number(&candidate["exact"]) >= number(&floors["PREFIX_MIN_EXACT"]), "{name}");
        assert!(number(&candidate["restored"]) >= number(&floors["PREFIX_MIN_RESTORED"]), "{name}");
        assert!(number(&candidate["early_restored"]) >= number(&floors["PREFIX_MIN_EARLY_RESTORED"]), "{name}");
        assert!(number(&candidate["changed_correct"]) <= number(&floors["PREFIX_MAX_CHANGED_CORRECT"]), "{name}");
        assert!(number(&candidate["length_mismatches"]) <= number(&floors["PREFIX_MAX_LENGTH_MISMATCHES"]), "{name}");
        // The prefix model may never cost a restoration the engine made without it.
        assert!(number(&candidate["exact"]) >= number(&without["exact"]), "{name}");
        assert!(number(&candidate["restored"]) >= number(&without["restored"]), "{name}");
        assert!(number(&candidate["changed_correct"]) <= number(&without["changed_correct"]), "{name}");
    }
}

#[test]
fn the_boundary_replay_holds_every_hard_boundary() {
    let (report, floors) = (read_json(&root().join("model/boundary_v1/engine-regression.json")).unwrap(), floors());
    let active = &report["results"]["active_v2"];
    assert!(number(&active["exact"]) >= number(&floors["BOUNDARY_MIN_EXACT"]));
    assert!(number(&active["changed_correct"]) <= number(&floors["BOUNDARY_MAX_CHANGED_CORRECT"]));
    assert!(number(&active["length_mismatches"]) <= number(&floors["BOUNDARY_MAX_LENGTH_MISMATCHES"]));
    assert!(number(&active["injections_before_hard_boundary"]) <= number(&floors["BOUNDARY_MAX_INJECTIONS_BEFORE_HARD_BOUNDARY"]));
}

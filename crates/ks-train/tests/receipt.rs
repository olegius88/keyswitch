//! The receipt of the installed pair passes the checks, and a receipt whose numbers were changed, whose fields are gone or
//! that names other files does not.
use ks_train::corpus::read_json;
use ks_train::receipt::validate_evidence;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn check(receipt: &Value) -> Result<(), String> {
    let root = root();
    let models = root.join("resources/models");
    validate_evidence(receipt, &models.join("context_policy_v1.json"), &root, &models.join("prefix_policy_v1.json"), &models)
}

fn receipt() -> Value {
    read_json(&root().join("model/context_v3/release-receipt.json")).unwrap()
}

#[test]
fn the_receipt_of_the_installed_pair_holds() {
    check(&receipt()).unwrap();
}

#[test]
fn a_changed_number_is_refused() {
    let edits: [(&str, fn(&mut Value)); 9] = [
        ("another artifact", |value| value["artifact_sha256"] = json!("0".repeat(64))),
        ("gates not passed", |value| value["quality_gates_passed"] = json!(false)),
        ("another policy", |value| value["gate_policy"]["sequence_length_mismatches"] = json!(1)),
        ("another threshold", |value| value["conversion_threshold"] = json!(0.5)),
        ("restorations below the baseline with the gate still claimed", |value| value["test"]["profiles"]["portable"]["counts"]["candidate"]["exactly_restored"] = json!(0)),
        ("a corruption removed from one side", |value| {
            let cell = &mut value["test"]["profiles"]["portable"]["early_off"]["counts"]["candidate"]["correct_text_corruptions"];
            *cell = json!(cell.as_i64().unwrap() + 3);
        }),
        ("a calibration count changed", |value| {
            let cell = &mut value["calibration"]["by_profile"]["portable"]["converted_correctly"];
            *cell = json!(cell.as_i64().unwrap() + 1);
        }),
        ("the chat test compared another pair", |value| value["devchat_test"]["pairs"]["candidate"]["sha256"] = json!("0".repeat(64))),
        ("a chat floor claimed without its numbers", |value| {
            let cell = &mut value["devchat_test"]["results"]["candidate"]["correct"]["ok"];
            *cell = json!(cell.as_i64().unwrap() - 10_000);
        }),
    ];
    for (name, edit) in edits {
        let mut changed = receipt();
        edit(&mut changed);
        assert!(check(&changed).is_err(), "{name}");
    }
}

#[test]
fn every_top_level_field_is_required_and_no_other_is_taken() {
    let whole = receipt();
    for name in whole.as_object().unwrap().keys() {
        let mut changed = whole.clone();
        changed.as_object_mut().unwrap().remove(name);
        assert!(check(&changed).is_err(), "{name} may be missing");
    }
    let mut changed = whole;
    changed["private_path"] = json!(".t/private/report.json");
    assert!(check(&changed).is_err(), "an unexpected field is taken");
}

#[test]
fn a_claimed_acceptance_cannot_override_the_counts() {
    let edits: [(&str, fn(&mut Value)); 6] = [
        ("damage in the portable profile with the gates still claimed", |value| {
            let cell = &mut value["test"]["profiles"]["portable"]["counts"]["candidate"]["correct_text_corruptions"];
            *cell = json!(cell.as_i64().unwrap() + 1);
        }),
        ("damage in the reference profile with the gates still claimed", |value| {
            let cell = &mut value["test"]["profiles"]["reference_hunspell"]["counts"]["candidate"]["correct_text_corruptions"];
            *cell = json!(cell.as_i64().unwrap() + 1);
        }),
        ("a profile gone", |value| {
            value["test"]["profiles"].as_object_mut().unwrap().remove("reference_hunspell");
        }),
        ("the control without early switching gone", |value| {
            value["test"]["profiles"]["portable"].as_object_mut().unwrap().remove("early_off");
        }),
        ("a calibration profile gone", |value| {
            value["calibration"]["by_profile"].as_object_mut().unwrap().remove("portable");
        }),
        ("the chat test gone", |value| {
            value.as_object_mut().unwrap().remove("devchat_test");
        }),
    ];
    for (name, edit) in edits {
        let mut changed = receipt();
        edit(&mut changed);
        assert!(check(&changed).is_err(), "{name}");
    }
}

#[test]
fn the_rejected_artifact_is_blocked_by_its_digest() {
    let root = root();
    let models = root.join("resources/models");
    let rejected = root.join("model/context_v2/candidate.json");
    assert!(validate_evidence(&receipt(), &rejected, &root, &models.join("prefix_policy_v1.json"), &models).is_err());
}

#[test]
fn the_installed_prefix_model_is_bound_to_the_receipt() {
    let root = root();
    let models = root.join("resources/models");
    let other = root.join("model/prefix_v1/candidate.json");
    assert!(validate_evidence(&receipt(), &models.join("context_policy_v1.json"), &root, &other, &models).is_err());
}

//! A cycle after a fit, on a small corpus and a tree that holds its recipe: the candidate compared, replayed, sealed and tested.
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]
mod support;

use ks_train::evaluate::{EvaluationRequest, evaluation_identity, protocol, validate_candidate_seal, validate_prefix_seal};
use ks_train::artifact::canonical;
use ks_train::corpus::checksum;
use ks_train::receipt::{export_receipt, validate_full_report};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use support::{chat_messages, corpus, recipe, refused, scratch, small_tree, succeeded, trainer, write_recipe};

struct Fitted {
    tree: PathBuf,
    corpus: PathBuf,
    ledger: PathBuf,
    folder: PathBuf,
    stages: PathBuf,
    model: PathBuf,
    seal: PathBuf,
    prefix: PathBuf,
    prefix_seal: PathBuf,
}

static FITTED: LazyLock<Fitted> = LazyLock::new(|| {
    let folder = scratch("cycle");
    let tree = small_tree("cycle-tree", &recipe());
    let (data, ledger, stages, output) = (folder.join("corpus"), folder.join("ledger"), folder.join("stages"), folder.join("candidate"));
    corpus(&data);
    std::fs::create_dir_all(&ledger).expect("a folder");
    succeeded(
        &trainer().arg("fit").arg("--tree").arg(&tree).arg("--corpus").arg(&data).arg("--output").arg(&output).arg("--ledger").arg(&ledger).arg("--stages").arg(&stages).args(["--jobs", "2"]).output().expect("the trainer starts"),
    );
    let (model, seal) = (output.join("context-action.json"), output.join("candidate-seal.json"));
    let prefix_folder = folder.join("prefix");
    let prefix_recipe = tree.join("model/prefix_v2/recipe.json");
    let mut small: Value = read(&tree.join("model/prefix_v2/recipe.json"));
    small["maximum_families"] = json!({"train": 60, "development": 16, "calibration": 16});
    small["epochs"] = json!(3);
    small["minimum_feature_mass"] = json!(1.0);
    small["maximum_features"] = json!(1500);
    write_recipe(&prefix_recipe, &small);
    let answer = trainer().arg("prefix").arg("--tree").arg(&tree).arg("--output").arg(&prefix_folder).args(["--jobs", "2"]).output().expect("the trainer starts");
    println!("prefix: {}", String::from_utf8_lossy(&answer.stdout));
    // Three epochs on sixteen families calibrate nothing: the test below is about the checks around a replay, not about this model.
    let prefix_seal = prefix_folder.join("seal.json");
    let mut sealed = read(&prefix_seal);
    sealed["calibration_passed"] = json!(true);
    std::fs::write(&prefix_seal, serde_json::to_vec(&sealed).expect("json")).expect("a seal");
    Fitted { tree, corpus: data, ledger, folder, stages, model, seal, prefix: prefix_folder.join("prefix-candidate.json"), prefix_seal }
});

fn with_tree(command: &str, fitted: &Fitted) -> Command {
    let mut trainer = trainer();
    trainer.arg(command).arg("--tree").arg(&fitted.tree);
    trainer
}

fn read(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("a file")).expect("json")
}

fn evaluation(fitted: &Fitted, split: &str, output: &Path, ledger: &Path) -> Command {
    let mut command = with_tree("evaluate", fitted);
    command
        .arg("--candidate")
        .arg(&fitted.model)
        .arg("--seal")
        .arg(&fitted.seal)
        .arg("--prefix-candidate")
        .arg(&fitted.prefix)
        .arg("--prefix-seal")
        .arg(&fitted.prefix_seal)
        .arg("--corpus")
        .arg(&fitted.corpus)
        .arg("--output")
        .arg(output)
        .arg("--ledger")
        .arg(ledger)
        .args(["--split", split, "--jobs", "2"]);
    command
}

#[test]
fn every_stage_of_a_fit_is_written_down() {
    let fitted = &*FITTED;
    let written: Vec<_> = std::fs::read_dir(&fitted.stages).expect("the stages").collect();
    assert!(!written.is_empty());
    assert_eq!(read(&fitted.seal)["stage"], json!("sealed-before-test"));
}

#[test]
fn a_candidate_is_compared_with_the_installed_model_on_frames_of_two_splits() {
    let fitted = &*FITTED;
    let output = fitted.folder.join("compared.json");
    let said = succeeded(
        &with_tree("compare", fitted)
            .arg("--corpus")
            .arg(&fitted.corpus)
            .arg("--ledger")
            .arg(&fitted.ledger)
            .arg("--artifact")
            .arg(format!("installed={}", fitted.tree.join("resources/models/context_policy_v1.json").display()))
            .arg("--artifact")
            .arg(format!("candidate={}", fitted.model.display()))
            .arg("--output")
            .arg(&output)
            .args(["--jobs", "2"])
            .output()
            .expect("the trainer starts"),
    );
    assert!(said.contains("candidate"));
    let report = read(&output);
    assert!(report["calibration"]["portable"]["candidate"]["by_class"]["all"]["rows"].as_i64().expect("a count") > 0);
    refused(&with_tree("compare", fitted).arg("--corpus").arg(&fitted.corpus).arg("--ledger").arg(&fitted.ledger).arg("--artifact").arg("gone=/nonexistent").output().expect("the trainer starts"), "/nonexistent");
}

#[test]
fn a_development_replay_reports_and_can_promote_nothing() {
    let fitted = &*FITTED;
    let output = fitted.folder.join("development.json");
    succeeded(&evaluation(fitted, "development", &output, &fitted.ledger).output().expect("the trainer starts"));
    let report = read(&output);
    assert_eq!(report["promotion_passed"], json!(false));
    assert!(report["scope"].as_str().expect("a scope").contains("development"));
    let unsealed = fitted.folder.join("unsealed.json");
    succeeded(&evaluation(fitted, "development", &unsealed, &fitted.ledger).arg("--unsealed").output().expect("the trainer starts"));
    refused(&evaluation(fitted, "test", &unsealed, &fitted.ledger).arg("--unsealed").output().expect("the trainer starts"), "needs seals");
    refused(&evaluation(fitted, "train", &unsealed, &fitted.ledger).output().expect("the trainer starts"), "development or sealed test");
}

fn request<'a>(fitted: &'a Fitted, resources: &'a Path, split: &'a str, output: &'a Path, ledger: &'a Path) -> EvaluationRequest<'a> {
    EvaluationRequest {
        root: &fitted.tree,
        resource_dir: resources,
        artifact: &fitted.model,
        seal: &fitted.seal,
        prefix: &fitted.prefix,
        prefix_seal: &fitted.prefix_seal,
        corpus: &fitted.corpus,
        split,
        output,
        ledger,
        jobs: 1,
        unsealed: false,
    }
}

/// A seal changed in one place, written next to the others.
fn changed_seal(fitted: &Fitted, base: &Value, name: &str, change: impl FnOnce(&mut Value)) -> PathBuf {
    let mut seal = base.clone();
    change(&mut seal);
    let path = fitted.folder.join(format!("seal-{name}.json"));
    std::fs::write(&path, serde_json::to_vec(&seal).expect("json")).expect("a seal");
    path
}

#[test]
fn a_candidate_seal_that_does_not_hold_is_refused_before_the_test() {
    let fitted = &*FITTED;
    let good = read(&fitted.seal);
    let models = fitted.tree.join("resources/models");
    validate_candidate_seal(&fitted.tree, &fitted.model, &fitted.seal, &fitted.corpus, &models).expect("the seal holds");
    let refuses = |name: &str, words: &str, change: &dyn Fn(&mut Value)| {
        let path = changed_seal(fitted, &good, name, change);
        let said = validate_candidate_seal(&fitted.tree, &fitted.model, &path, &fitted.corpus, &models).expect_err(name);
        assert!(said.contains(words), "{name}: {said}");
    };
    refuses("schema", "unsupported candidate seal", &|seal| seal["schema_version"] = json!(2));
    refuses("stage", "not sealed before test", &|seal| seal["stage"] = json!("tested"));
    refuses("accessed", "not sealed before test", &|seal| seal["test_accessed"] = json!(true));
    refuses("artifact", "differs from seal", &|seal| seal["artifact_sha256"] = json!("0"));
    refuses("corpus", "corpus differs", &|seal| seal["corpus_manifest_sha256"] = json!("0"));
    refuses("recipe", "recipe differs", &|seal| seal["recipe"]["epochs"] = json!(99));
    refuses("policy", "gate policy differs", &|seal| seal["gate_policy"] = json!({}));
    refuses("no-pins", "names no provenance", &|seal| seal["provenance"] = json!(1));
    refuses("absolute", "repository-relative", &|seal| seal["provenance"] = json!({"/etc/passwd": "0"}));
    refuses("climbing", "repository-relative", &|seal| seal["provenance"] = json!({"../x": "0"}));
    refuses("changed", "provenance changed", &|seal| seal["provenance"] = json!({"Cargo.lock": "0"}));
    refuses("version", "identity differs", &|seal| seal["model_version"] = json!("other"));
    refuses("threshold", "identity differs", &|seal| seal["conversion_threshold"] = json!(0.123));
    refuses("no-profiles", "both declared", &|seal| seal["calibration"]["by_profile"] = json!({}));
    refuses("no-calibration", "both declared", &|seal| seal["calibration"] = json!({}));
    refuses("named", "both declared", &|seal| {
        let table = seal["calibration"]["by_profile"].as_object_mut().expect("the profiles");
        let moved = table.remove("portable").expect("a profile");
        table.insert("other".to_owned(), moved);
    });
    refuses("negative", "nonnegative", &|seal| seal["calibration"]["rows"] = json!(-1));
    refuses("none", "calibration gate failed", &|seal| seal["calibration"]["convert_rows"] = json!(0));
    refuses("few-rows", "calibration gate failed", &|seal| seal["calibration"]["rows"] = json!(0));
    refuses("too-right", "calibration gate failed", &|seal| seal["calibration"]["converted_correctly"] = json!(1_000_000));
    refuses("recall", "calibration gate failed", &|seal| seal["calibration"]["conversion_recall"] = json!(-1.0));
    refuses("wrong", "calibration gate failed", &|seal| seal["calibration"]["false_conversions"] = json!(1_000_000));
    refuses("sum", "does not match", &|seal| {
        let rows = seal["calibration"]["rows"].as_i64().expect("a count");
        seal["calibration"]["rows"] = json!(rows + 1);
    });
    // A model that cannot be read, under a seal that names exactly its bytes.
    let broken = fitted.folder.join("broken-model.json");
    std::fs::write(&broken, b"{}").expect("a file");
    let path = changed_seal(fitted, &good, "broken", |seal| seal["artifact_sha256"] = json!(checksum(&broken).expect("a checksum")));
    assert!(validate_candidate_seal(&fitted.tree, &broken, &path, &fitted.corpus, &models).is_err());
}

#[test]
fn a_prefix_seal_that_does_not_hold_is_refused_before_the_test() {
    let fitted = &*FITTED;
    let good = read(&fitted.prefix_seal);
    validate_prefix_seal(&fitted.tree, &fitted.prefix, &fitted.prefix_seal, false).expect("the seal holds");
    let refuses = |name: &str, words: &str, change: &dyn Fn(&mut Value)| {
        let path = changed_seal(fitted, &good, name, change);
        let said = validate_prefix_seal(&fitted.tree, &fitted.prefix, &path, true).expect_err(name);
        assert!(said.contains(words), "{name}: {said}");
    };
    refuses("prefix-schema", "unsupported prefix seal", &|seal| seal["schema_version"] = json!(2));
    refuses("prefix-promoted", "must not claim", &|seal| seal["promotion_accepted"] = json!(true));
    refuses("prefix-tested", "must not claim", &|seal| seal["independent_test_evaluated"] = json!(true));
    refuses("prefix-calibration", "did not pass calibration", &|seal| seal["calibration_passed"] = json!(false));
    let passing = |seal: &mut Value| seal["calibration_passed"] = json!(true);
    refuses("prefix-artifact", "differs from seal", &|seal| {
        passing(seal);
        seal["candidate_sha256"] = json!("0");
    });
    refuses("prefix-pins", "provenance changed", &|seal| {
        passing(seal);
        seal["provenance"] = json!({"Cargo.lock": "0"});
    });
    refuses("prefix-version", "identity differs", &|seal| {
        passing(seal);
        seal["model_version"] = json!("other");
    });
    refuses("prefix-threshold", "identity differs", &|seal| {
        passing(seal);
        seal["threshold"] = json!(0.123);
    });
    let broken = fitted.folder.join("broken-prefix.json");
    std::fs::write(&broken, b"{}").expect("a file");
    let path = changed_seal(fitted, &good, "prefix-broken", |seal| {
        passing(seal);
        seal["candidate_sha256"] = json!(checksum(&broken).expect("a checksum"));
    });
    assert!(validate_prefix_seal(&fitted.tree, &broken, &path, true).is_err());
}

#[test]
fn the_membership_of_a_test_is_checked_against_its_manifest() {
    let fitted = &*FITTED;
    let resources = fitted.tree.join("resources");
    let (output, ledger) = (fitted.folder.join("identity.json"), fitted.folder.join("identity-ledger"));
    let good = evaluation_identity(&request(fitted, &resources, "test", &output, &ledger)).expect("an identity");
    assert_eq!(good.value["schema_version"], json!(1));
    let copy = fitted.folder.join("identity-corpus");
    std::fs::create_dir_all(&copy).expect("a folder");
    let manifest = read(&fitted.corpus.join("manifest.json"));
    let membership = read(&fitted.corpus.join("test-membership.json"));
    let with = |changed: &Value, pin: bool| {
        let bytes = serde_json::to_vec(changed).expect("json");
        std::fs::write(copy.join("test-membership.json"), &bytes).expect("a file");
        let mut new = manifest.clone();
        new["test_membership_sha256"] = json!(if pin { support::digest(&bytes) } else { "0".to_owned() });
        std::fs::write(copy.join("manifest.json"), serde_json::to_vec(&new).expect("json")).expect("a file");
        let mut asked = request(fitted, &resources, "test", &output, &ledger);
        asked.corpus = &copy;
        evaluation_identity(&asked).map(|identity| identity.key).expect_err("refused")
    };
    assert!(with(&membership, false).contains("differs from manifest"));
    let mut empty = membership.clone();
    empty["row_ids_sha256"] = json!([]);
    assert!(with(&empty, true).contains("invalid test membership"));
    let mut bad = membership.clone();
    bad["family_ids_sha256"] = json!(["xyz"]);
    assert!(with(&bad, true).contains("invalid test membership"));
    let mut number = membership.clone();
    number["document_ids_sha256"] = json!([1]);
    assert!(with(&number, true).contains("invalid test membership"));
    let mut unsorted = membership.clone();
    unsorted["row_ids_sha256"] = json!(["b".repeat(64), "a".repeat(64)]);
    assert!(with(&unsorted, true).contains("sorted and unique"));
    let mut alias = membership;
    alias["alias_sha256"] = json!(["a".repeat(64)]);
    std::fs::write(copy.join("test-membership.json"), serde_json::to_vec(&alias).expect("json")).expect("a file");
    let mut new = manifest;
    new["test_membership_sha256"] = json!(checksum(&copy.join("test-membership.json")).expect("a checksum"));
    std::fs::write(copy.join("manifest.json"), serde_json::to_vec(&new).expect("json")).expect("a file");
    let mut asked = request(fitted, &resources, "test", &output, &ledger);
    asked.corpus = &copy;
    assert!(evaluation_identity(&asked).is_ok());
}

#[test]
fn a_protocol_changed_after_its_audit_stops_a_report() {
    let folder = scratch("protocol");
    std::fs::create_dir_all(folder.join("model/context_v3")).expect("a folder");
    std::fs::write(folder.join("model/context_v3/protocols.json"), serde_json::to_vec(&json!({"sequence": {}, "audited_sequence_protocol_sha256": "0"})).expect("json")).expect("a file");
    assert!(protocol(&folder).unwrap_err().contains("protocol changed"));
}

#[test]
fn the_test_is_read_once_with_seals_and_shares_nothing_with_one_read_before() {
    let fitted = &*FITTED;
    let output = fitted.folder.join("test.json");
    let ledger = fitted.folder.join("test-ledger");
    // Nobody else may be reading it.
    std::fs::create_dir_all(&ledger).expect("a folder");
    std::fs::write(ledger.join(".access-lock"), b"").expect("a lock");
    refused(&evaluation(fitted, "test", &output, &ledger).output().expect("the trainer starts"), "locked");
    std::fs::remove_file(ledger.join(".access-lock")).expect("the lock goes");
    // It shares a row with a test read before.
    let resources = fitted.tree.join("resources");
    let identity = evaluation_identity(&request(fitted, &resources, "test", &output, &ledger)).expect("an identity");
    let earlier = ledger.join("earlier.access.json");
    std::fs::write(&earlier, serde_json::to_vec(&json!({"test_membership": {"row_ids_sha256": identity.value["test_membership"]["row_ids_sha256"]}})).expect("json")).expect("a record");
    std::fs::write(ledger.join("notes.txt"), b"").expect("a file");
    refused(&evaluation(fitted, "test", &output, &ledger).output().expect("the trainer starts"), "overlaps");
    std::fs::remove_file(&earlier).expect("the record goes");
    // Its own record says what it was, whatever else the ledger holds.
    std::fs::write(&earlier, serde_json::to_vec(&json!({"test_membership": {"row_ids_sha256": ["0"]}})).expect("json")).expect("a record");
    let answer = evaluation(fitted, "test", &output, &ledger).output().expect("the trainer starts");
    let said = String::from_utf8_lossy(&answer.stdout);
    assert!(said.contains("promotion_passed"), "{said}{}", String::from_utf8_lossy(&answer.stderr));
    let report = read(&output);
    assert_eq!(report["split"], json!("test"));
    // Read again it gives the same bytes, and a record that differs stops it.
    let again = evaluation(fitted, "test", &output, &ledger).output().expect("the trainer starts");
    assert_eq!(again.status.code(), answer.status.code());
    std::fs::write(ledger.join(format!("{}.outcome.json", identity.key)), b"other").expect("a record");
    refused(&evaluation(fitted, "test", &output, &ledger).output().expect("the trainer starts"), "immutable test record differs");
    std::fs::write(ledger.join(format!("{}.access.json", identity.key)), b"other").expect("a record");
    refused(&evaluation(fitted, "test", &output, &ledger).output().expect("the trainer starts"), "");
}

#[test]
fn a_model_that_cannot_be_replayed_leaves_a_report_saying_so() {
    let fitted = &*FITTED;
    let broken = fitted.folder.join("unreadable.json");
    std::fs::write(&broken, b"{}").expect("a file");
    let output = fitted.folder.join("deep/er/failed.json");
    let mut command = with_tree("evaluate", fitted);
    command
        .arg("--candidate")
        .arg(&broken)
        .arg("--seal")
        .arg(&fitted.seal)
        .arg("--prefix-candidate")
        .arg(&fitted.prefix)
        .arg("--prefix-seal")
        .arg(&fitted.prefix_seal)
        .arg("--corpus")
        .arg(&fitted.corpus)
        .arg("--output")
        .arg(&output)
        .arg("--ledger")
        .arg(&fitted.ledger)
        .args(["--unsealed", "--jobs", "1"]);
    refused(&command.output().expect("the trainer starts"), "unreadable.json");
    assert!(read(&output)["execution_error"].is_string());
}

/// The sealed test of the small candidate, once for the tests below.
struct Sealed {
    report: PathBuf,
    identity: Value,
    key: String,
}

static SEALED: LazyLock<Sealed> = LazyLock::new(|| {
    let fitted = &*FITTED;
    let (report, ledger) = (fitted.folder.join("sealed.json"), fitted.folder.join("sealed-ledger"));
    let answer = evaluation(fitted, "test", &report, &ledger).output().expect("the trainer starts");
    assert!(report.is_file(), "{}", String::from_utf8_lossy(&answer.stderr));
    let resources = fitted.tree.join("resources");
    let identity = evaluation_identity(&request(fitted, &resources, "test", &report, &ledger)).expect("an identity");
    Sealed { report, identity: identity.value, key: identity.key }
});

const BLOCKS: [(&str, bool); 4] = [("portable", false), ("portable", true), ("reference_hunspell", false), ("reference_hunspell", true)];

fn block_of<'a>(report: &'a mut Value, profile: &str, early: bool) -> &'a mut Value {
    let block = &mut report["profiles"][profile];
    if early { &mut block["early_off"] } else { block }
}

/// The counts of every block recomputed from its cases, as the check does.
fn recount(report: &mut Value) {
    for (profile, early) in BLOCKS {
        let block = block_of(report, profile, early);
        for model in ["baseline", "candidate"] {
            let cases = block["cases"][model].as_array().expect("cases").clone();
            let count = |test: &dyn Fn(&Value) -> i64| cases.iter().map(test).sum::<i64>();
            let wrong = |case: &Value| case["initially_wrong"].as_bool().expect("a mode");
            let exact = |case: &Value| case["exact"].as_bool().expect("a result");
            let counts = json!({
                "rows": cases.len(), "initially_correct": count(&|case| i64::from(!wrong(case))), "initially_wrong": count(&|case| i64::from(wrong(case))),
                "preserved_correct": count(&|case| i64::from(!wrong(case) && exact(case))), "exactly_restored": count(&|case| i64::from(wrong(case) && exact(case))),
                "correct_text_corruptions": count(&|case| i64::from(!wrong(case) && !exact(case))), "length_mismatches": count(&|case| i64::from(case["length_mismatch"].as_bool().expect("a result"))),
                "injections": count(&|case| i64::try_from(case["corrections"].as_array().expect("corrections").len()).expect("few")), "execution_errors": count(&|case| i64::from(!case["error"].is_null())),
                "correction_layout_mismatches": count(&|case| i64::try_from(case["corrections"].as_array().expect("corrections").iter().filter(|item| item["actual_group"] != item["target_group"]).count()).expect("few")),
                "final_layout_mismatches": count(&|case| i64::from(!case["final_layout_matches"].as_bool().expect("a result"))),
            });
            block["counts"][model] = counts;
        }
    }
}

/// The same edit of the cases of every block and model, and the counts that follow from it.
fn everywhere(report: &mut Value, edit: &dyn Fn(&mut Vec<Value>)) {
    for (profile, early) in BLOCKS {
        for model in ["baseline", "candidate"] {
            edit(block_of(report, profile, early)["cases"][model].as_array_mut().expect("cases"));
        }
    }
    recount(report);
}

#[test]
fn a_report_is_recomputed_from_its_private_cases() {
    let sealed = &*SEALED;
    let good = read(&sealed.report);
    assert_eq!(good["identity"], sealed.identity);
    validate_full_report(&good, &sealed.identity).expect("the report holds");
    let refused = |words: &str, edit: &dyn Fn(&mut Value)| {
        let mut changed = good.clone();
        edit(&mut changed);
        let said = validate_full_report(&changed, &sealed.identity).expect_err(words);
        assert!(said.contains(words), "{words}: {said}");
    };
    // What was selected.
    refused("selection IDs", &|report| report["selection"]["source_ids"] = json!(1));
    refused("selection IDs", &|report| report["selection"]["selected_rows"] = json!(1));
    refused("invalid count", &|report| report["selection"]["selected_rows"] = json!(-1));
    refused("selection IDs", &|report| {
        let ids = report["selection"]["source_ids"].as_array_mut().expect("the IDs");
        let first = ids[0].clone();
        ids[1] = first;
    });
    refused("outside immutable test membership", &|report| report["selection"]["source_ids"][0] = json!("nobody"));
    refused("invalid unsupported", &|report| report["selection"]["unsupported"] = json!(1));
    refused("omitted", &|report| {
        let first = report["profiles"]["portable"]["cases"]["baseline"][0]["identifier"].clone();
        report["selection"]["unsupported"] = json!([{"identifier": first}]);
    });
    refused("missing case rows", &|report| report["profiles"]["portable"]["cases"]["baseline"] = json!(1));
    // One case, each way it can lie.
    let cases = good["profiles"]["portable"]["cases"]["baseline"].as_array().expect("cases");
    let right = cases.iter().position(|case| case["initially_wrong"] == json!(false)).expect("a correct case");
    let wrong = cases.iter().position(|case| case["initially_wrong"] == json!(true)).expect("a wrong case");
    let edited = |at: usize, words: &str, edit: &dyn Fn(&mut Value)| {
        refused(words, &|report| edit(&mut report["profiles"]["portable"]["cases"]["baseline"][at]));
    };
    for field in ["identifier", "document", "expected", "actual"] {
        edited(right, "invalid case text", &|case| case[field] = json!(1));
    }
    edited(right, "intervention mode", &|case| case["initially_wrong"] = json!("yes"));
    edited(wrong, "intervention mode", &|case| case["group"] = json!(null));
    edited(right, "final layout", &|case| case["final_group"] = json!("x"));
    edited(right, "final layout", &|case| case["expected_final_group"] = json!("x"));
    edited(right, "execution error", &|case| case["error"] = json!(1));
    edited(right, "contradicts", &|case| case["exact"] = json!(false));
    edited(right, "contradicts", &|case| case["length_mismatch"] = json!(true));
    edited(right, "contradicts", &|case| case["final_layout_matches"] = json!(!case["final_layout_matches"].as_bool().expect("a result")));
    edited(right, "contradicts", &|case| case["error"] = json!("it failed"));
    edited(right, "unexpectedly has a wrong intervention", &|case| case["intervention"] = json!({"event": 1}));
    edited(wrong, "invalid count", &|case| case["intervention"]["event"] = json!(-1));
    const EFFECTIVE: &str = "effective wrong intervention";
    edited(wrong, EFFECTIVE, &|case| case["intervention"]["event"] = json!(0));
    edited(wrong, EFFECTIVE, &|case| case["intervention"]["event"] = json!(100_000));
    edited(wrong, EFFECTIVE, &|case| case["intervention"]["source_group"] = json!(null));
    edited(wrong, EFFECTIVE, &|case| case["intervention"]["target_group"] = json!(9));
    edited(wrong, EFFECTIVE, &|case| case["intervention"]["source_group"] = case["intervention"]["target_group"].clone());
    edited(wrong, EFFECTIVE, &|case| case["intervention"]["expected"] = json!("?"));
    edited(wrong, EFFECTIVE, &|case| case["intervention"]["observed"] = json!("ab"));
    edited(wrong, EFFECTIVE, &|case| case["intervention"]["observed"] = case["intervention"]["expected"].clone());
    edited(wrong, "invalid case corrections", &|case| case["corrections"] = json!(1));
    let restored = cases.iter().position(|case| case["initially_wrong"] == json!(true) && case["exact"] == json!(true)).expect("a restored case");
    edited(restored, "restoration without injection", &|case| case["corrections"] = json!([]));
    edited(restored, "invalid count", &|case| case["corrections"][0]["event"] = json!(-1));
    edited(restored, "layout or event", &|case| case["corrections"][0]["event"] = json!(0));
    edited(restored, "layout or event", &|case| case["corrections"][0]["event"] = json!(100_000));
    for field in ["source_group", "target_group", "actual_group"] {
        edited(restored, "layout or event", &|case| case["corrections"][0][field] = json!(9));
    }
    for (text, caret) in [("before", "caret_before"), ("after", "caret_after")] {
        edited(restored, "text or caret", &|case| case["corrections"][0][text] = json!(1));
        edited(restored, "text or caret", &|case| case["corrections"][0][caret] = json!(100_000));
        edited(restored, "invalid count", &|case| case["corrections"][0][caret] = json!(-1));
    }
    edited(right, "invalid count", &|case| case["trimmed_left"] = json!(-1));
    edited(right, "invalid count", &|case| case["trimmed_right"] = json!(-1));
    // Cases against the list, the counts, each other and the membership.
    refused("duplicate", &|report| {
        let list = report["profiles"]["portable"]["cases"]["baseline"].as_array_mut().expect("cases");
        let first = list[right].clone();
        list.push(first);
    });
    edited(right, "duplicate, omitted or unselected", &|case| case["identifier"] = json!("nobody"));
    edited(right, "case document is outside", &|case| case["document"] = json!("nowhere"));
    refused("claimed counts differ", &|report| {
        let cell = &mut report["profiles"]["portable"]["counts"]["baseline"]["rows"];
        *cell = json!(cell.as_i64().expect("a count") + 1);
    });
    refused("different case inputs", &|report| {
        let cell = &mut report["profiles"]["portable"]["cases"]["candidate"][right]["trimmed_left"];
        *cell = json!(cell.as_i64().expect("a count") + 1);
    });
    // All blocks alike, and still not what was selected.
    refused("case coverage", &|report| {
        everywhere(report, &|cases| {
            let at = cases.iter().position(|case| case["initially_wrong"] == json!(false)).expect("a correct case");
            cases.remove(at);
        });
    });
    refused("paired correct/wrong", &|report| {
        everywhere(report, &|cases| {
            let at = cases.iter().position(|case| case["initially_wrong"] == json!(true)).expect("a wrong case");
            let cell = &mut cases[at]["trimmed_left"];
            *cell = json!(cell.as_i64().expect("a count") + 1);
        });
    });
    refused("document counts", &|report| report["selection"]["replayed_documents"]["0"] = json!(0));
}

/// A chat test that passed, as a ledger holds it, for this candidate.
fn chat_ledger(fitted: &Fitted, tree: &Path, folder: &Path) {
    std::fs::create_dir_all(folder).expect("a folder");
    let model = read(&fitted.model);
    let pair = |sha: String, version: &Value| json!({"sha256": sha, "model_version": version});
    let baseline_path = tree.join("model/context_v3/baseline-context-v1.json");
    let identity = json!({
        "protocol": "devchat-messages-v1", "bench": "small", "seed": "small", "messages_sha256": ["a".repeat(64), "b".repeat(64)], "test_membership": {"messages": ["a"], "questions": ["b"]},
        "pairs": {
            "candidate": pair(checksum(&fitted.model).expect("a checksum"), &model["version"]),
            "released": pair("c".repeat(64), &json!("released")),
            "baseline": pair(checksum(&baseline_path).expect("a checksum"), &read(&baseline_path)["version"]),
        },
    });
    let table = |right: i64| {
        let mode = |ok: i64| json!({"n": 10, "ok": ok, "errors": 0});
        json!({"correct": mode(10), "forget_ru": mode(right), "forget_en": mode(right), "start_wrong": mode(right)})
    };
    let even = json!({"gains": 0, "losses": 0});
    let result = json!({
        "split": "test", "messages": 1, "access": "d".repeat(64), "results": {"candidate": table(9), "released": table(9), "baseline": table(5)},
        "discordance": {"correct": even, "forget_ru": even, "forget_en": even, "start_wrong": even},
        "floors": {"every_message_typed": true, "f1_better_than_baseline": true, "f3_not_worse_than_released": true},
    });
    std::fs::write(folder.join("passed.access.json"), serde_json::to_vec(&identity).expect("json")).expect("a record");
    std::fs::write(folder.join("passed.result.json"), serde_json::to_vec(&result).expect("json")).expect("a record");
    let development = json!({"floors": {"every_message_typed": true, "f1_better_than_baseline": true, "f3_not_worse_than_released": true}});
    std::fs::write(folder.join(format!("{}.dev.json", checksum(&fitted.model).expect("a checksum"))), serde_json::to_vec(&development).expect("json")).expect("a record");
}

/// A report of the sealed test as a passing one: the candidate typed every text right, and the baseline lost one it had restored.
fn accepted_report(sealed: &Sealed) -> Value {
    let mut report = read(&sealed.report);
    for (profile, early) in BLOCKS {
        let block = block_of(&mut report, profile, early);
        for case in block["cases"]["candidate"].as_array_mut().expect("cases") {
            case["actual"] = case["expected"].clone();
            case["exact"] = json!(true);
            case["length_mismatch"] = json!(false);
            case["error"] = json!(null);
            if case["initially_wrong"] == json!(true) && case["corrections"].as_array().expect("corrections").is_empty() {
                case["corrections"] = json!([{"event": 1, "source_group": 0, "target_group": 1, "actual_group": 1, "before": "", "after": "", "caret_before": 0, "caret_after": 0}]);
            }
            for correction in case["corrections"].as_array_mut().expect("corrections") {
                correction["actual_group"] = correction["target_group"].clone();
            }
        }
        let cases = block["cases"]["baseline"].as_array_mut().expect("cases");
        if let Some(case) = cases.iter_mut().find(|case| case["initially_wrong"] == json!(true) && case["exact"] == json!(true)) {
            let text = case["expected"].as_str().expect("a text");
            let spoiled: String = std::iter::once('?').chain(text.chars().skip(1)).collect();
            case["actual"] = json!(spoiled);
            case["exact"] = json!(false);
        }
        for gate in block["gates"].as_object_mut().expect("the gates").values_mut() {
            *gate = json!(true);
        }
    }
    recount(&mut report);
    report["promotion_passed"] = json!(true);
    report
}

/// A ledger that confirms this report.
fn sealed_ledger(sealed: &Sealed, report: &Value, folder: &Path, report_path: &Path) {
    std::fs::create_dir_all(folder).expect("a folder");
    let bytes = canonical(report);
    std::fs::write(report_path, &bytes).expect("a report");
    std::fs::write(folder.join(format!("{}.outcome.json", sealed.key)), &bytes).expect("a record");
    std::fs::write(folder.join(format!("{}.access.json", sealed.key)), canonical(&sealed.identity)).expect("a record");
}

#[test]
fn a_receipt_is_exported_only_from_a_report_its_ledger_confirms() {
    let fitted = &*FITTED;
    let sealed = &*SEALED;
    let folder = scratch("export");
    let (report_path, ledger, chat, output) = (folder.join("report.json"), folder.join("ledger"), folder.join("chat"), folder.join("receipt.json"));
    let good = accepted_report(sealed);
    sealed_ledger(sealed, &good, &ledger, &report_path);
    chat_ledger(fitted, &fitted.tree, &chat);
    let resources = fitted.tree.join("resources");
    let asked = request(fitted, &resources, "test", &report_path, &ledger);
    let receipt = export_receipt(&asked, &report_path, &chat, &output).expect("a receipt");
    assert_eq!(receipt["quality_gates_passed"], json!(true));
    // The same again gives the same bytes, and a receipt that differs is not written over.
    export_receipt(&asked, &report_path, &chat, &output).expect("the same receipt");
    std::fs::write(&output, b"other").expect("a file");
    assert!(export_receipt(&asked, &report_path, &chat, &output).unwrap_err().contains("immutable test record differs"));
    std::fs::remove_file(&output).expect("the file goes");
    // Not an accepted test.
    let refused = |edit: &dyn Fn(&mut Value)| {
        let mut changed = good.clone();
        edit(&mut changed);
        let path = folder.join("changed.json");
        let (ledger, chat) = (folder.join("changed-ledger"), folder.join("chat"));
        sealed_ledger(sealed, &changed, &ledger, &path);
        let asked = request(fitted, &resources, "test", &path, &ledger);
        export_receipt(&asked, &path, &chat, &output).expect_err("refused")
    };
    let edits: [&dyn Fn(&mut Value); 7] = [
        &|report| report["schema_version"] = json!(2),
        &|report| report["split"] = json!("development"),
        &|report| report["identity"] = json!({}),
        &|report| report["protocol"] = json!("other"),
        &|report| report["gate_policy"] = json!("other"),
        &|report| report["promotion_passed"] = json!(false),
        &|report| report["execution_error"] = json!("failed"),
    ];
    for edit in edits {
        assert!(refused(edit).contains("not an accepted current sealed test"));
    }
    assert!(refused(&|report| report["selection"]["source_ids"] = json!(1)).contains("selection IDs"));
    // The ledger says otherwise.
    let (other_report, other_ledger) = (folder.join("other.json"), folder.join("other-ledger"));
    sealed_ledger(sealed, &good, &other_ledger, &other_report);
    let asked = request(fitted, &resources, "test", &other_report, &other_ledger);
    std::fs::write(other_ledger.join(format!("{}.outcome.json", sealed.key)), b"other").expect("a record");
    assert!(export_receipt(&asked, &other_report, &chat, &output).unwrap_err().contains("does not confirm"));
    sealed_ledger(sealed, &good, &other_ledger, &other_report);
    std::fs::write(other_ledger.join(format!("{}.access.json", sealed.key)), b"other").expect("a record");
    assert!(export_receipt(&asked, &other_report, &chat, &output).unwrap_err().contains("does not confirm"));
    sealed_ledger(sealed, &good, &other_ledger, &other_report);
    // A chat test that is not there, of another candidate, that read nothing, with no result, or that failed.
    let empty = folder.join("no-chat");
    std::fs::create_dir_all(&empty).expect("a folder");
    assert!(export_receipt(&asked, &other_report, &empty, &output).unwrap_err().contains("no developer-chat test"));
    assert!(export_receipt(&asked, &other_report, &folder.join("gone"), &output).is_err());
    std::fs::write(empty.join("notes.txt"), b"").expect("a file");
    let record = read(&chat.join("passed.access.json"));
    let mut other = record.clone();
    other["pairs"]["candidate"]["sha256"] = json!("0");
    std::fs::write(empty.join("a.access.json"), serde_json::to_vec(&other).expect("json")).expect("a record");
    let mut nothing = record.clone();
    nothing["test_membership"]["messages"] = json!([]);
    std::fs::write(empty.join("b.access.json"), serde_json::to_vec(&nothing).expect("json")).expect("a record");
    let mut none = record.clone();
    none.as_object_mut().expect("an object").remove("test_membership");
    std::fs::write(empty.join("c.access.json"), serde_json::to_vec(&none).expect("json")).expect("a record");
    assert!(export_receipt(&asked, &other_report, &empty, &output).unwrap_err().contains("no developer-chat test"));
    std::fs::write(empty.join("d.access.json"), serde_json::to_vec(&record).expect("json")).expect("a record");
    assert!(export_receipt(&asked, &other_report, &empty, &output).is_err());
    std::fs::write(empty.join("d.result.json"), serde_json::to_vec(&json!({"error": "it failed"})).expect("json")).expect("a record");
    assert!(export_receipt(&asked, &other_report, &empty, &output).unwrap_err().contains("failed to run"));
    // One message file named by itself.
    let mut single = record;
    single["messages_sha256"] = json!("e".repeat(64));
    std::fs::write(chat.join("passed.access.json"), serde_json::to_vec(&single).expect("json")).expect("a record");
    assert_eq!(export_receipt(&asked, &other_report, &chat, &output).expect("a receipt")["devchat_test"]["messages_sha256"], json!("e".repeat(64)));
    // `ks-train verify` of what was exported, and of its private report.
    succeeded(
        &with_tree("verify", fitted)
            .arg("--receipt")
            .arg(&output)
            .arg("--artifact")
            .arg(&fitted.model)
            .arg("--prefix")
            .arg(&fitted.prefix)
            .arg("--report")
            .arg(&other_report)
            .output()
            .expect("the trainer starts"),
    );
    succeeded(&with_tree("verify", fitted).arg("--receipt").arg(&output).arg("--artifact").arg(&fitted.model).arg("--prefix").arg(&fitted.prefix).arg("--evidence-only").output().expect("the trainer starts"));
    assert!(!with_tree("verify", fitted).arg("--receipt").arg(folder.join("gone.json")).output().expect("the trainer starts").status.success());
}

/// `cargo` and `git` that do what the case asks, by the environment: nothing here tests or changes a real tree.
fn stubs(folder: &Path) -> std::ffi::OsString {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = folder.join("bin");
    std::fs::create_dir_all(&bin).expect("a folder");
    let write = |name: &str, body: &str| {
        let path = bin.join(name);
        std::fs::write(&path, body).expect("a stub");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("a mode");
    };
    write("cargo", "#!/bin/sh\ncase \"$1\" in\n  test)\n    case \"$CARGO_TARGET_DIR\" in\n      *keyswitch-candidate*) [ -z \"$KS_STUB_SCRATCH_TESTS_FAIL\" ] ;;\n      *) [ -z \"$KS_STUB_TESTS_FAIL\" ] ;;\n    esac\n    exit $?\n    ;;\n  build)\n    mkdir -p \"$CARGO_TARGET_DIR/parity\"\n    printf '#!/bin/sh\\nexit %s\\n' \"${KS_STUB_FIELD:-0}\" > \"$CARGO_TARGET_DIR/parity/field-gate\"\n    chmod +x \"$CARGO_TARGET_DIR/parity/field-gate\"\n    exit \"${KS_STUB_BUILD:-0}\"\n    ;;\nesac\n");
    write("git", "#!/bin/sh\ncase \"$1\" in\n  diff) [ -z \"$KS_STUB_PENDING\" ] || echo a-changed-file; exit 0 ;;\n  worktree)\n    if [ \"$2\" = add ]; then\n      [ -z \"$KS_STUB_NO_WORKTREE\" ] || exit 1\n      mkdir -p \"$4/resources/models\" \"$4/model/context_v3\" \"$4/model/prefix_v2\"\n    fi\n    exit 0 ;;\nesac\n");
    let mut path = bin.into_os_string();
    path.push(":");
    path.push(std::env::var_os("PATH").expect("a path"));
    path
}

#[derive(Clone)]
struct Promotion {
    tree: PathBuf,
    report: PathBuf,
    ledger: PathBuf,
    chat: PathBuf,
    candidate: PathBuf,
    prefix: PathBuf,
    path: std::ffi::OsString,
    folder: PathBuf,
}

fn promotion(name: &str, report: &Value) -> Promotion {
    let fitted = &*FITTED;
    let sealed = &*SEALED;
    let folder = scratch(name);
    let tree = small_tree(&format!("{name}-tree"), &recipe());
    write_recipe(&tree.join("model/prefix_v2/recipe.json"), &read(&fitted.tree.join("model/prefix_v2/recipe.json")));
    let (report_path, ledger, chat) = (folder.join("report.json"), folder.join("ledger"), folder.join("chat"));
    sealed_ledger(sealed, report, &ledger, &report_path);
    chat_ledger(fitted, &tree, &chat);
    chat_messages(&tree, &folder.join("devchat"), &["привет как дела", "git status показывает"], 1);
    let path = stubs(&folder);
    Promotion { tree, report: report_path, ledger, chat, candidate: fitted.model.parent().expect("a folder").to_path_buf(), prefix: fitted.prefix.parent().expect("a folder").to_path_buf(), path, folder }
}

fn promote(run: &Promotion) -> Command {
    let fitted = &*FITTED;
    let mut command = trainer();
    command
        .arg("promote")
        .arg("--tree")
        .arg(&run.tree)
        .arg("--candidate")
        .arg(&run.candidate)
        .arg("--prefix-candidate")
        .arg(&run.prefix)
        .arg("--corpus")
        .arg(&fitted.corpus)
        .arg("--report")
        .arg(&run.report)
        .arg("--released")
        .arg(run.folder.join("released"))
        .arg("--devchat")
        .arg(run.folder.join("devchat"))
        .arg("--ledger")
        .arg(&run.ledger)
        .arg("--devchat-ledger")
        .arg(&run.chat)
        .args(["--jobs", "1"])
        .env("PATH", &run.path);
    command
}

fn told(command: &mut Command) -> (bool, String) {
    let output = command.output().expect("the trainer starts");
    (output.status.success(), format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)))
}

#[test]
fn a_candidate_is_installed_after_every_check_and_only_then() {
    let run = promotion("promote", &accepted_report(&SEALED));
    // Checks only: nothing is read or changed.
    let (held, said) = told(promote(&run).arg("--check-only"));
    assert!(held && said.contains("every check holds"), "{said}");
    // The installed pair goes in, with its receipt.
    let (held, said) = told(&mut promote(&run));
    assert!(held && said.contains("installed"), "{said}");
    assert_eq!(checksum(&run.tree.join("resources/models/context_policy_v1.json")).expect("a checksum"), checksum(&FITTED.model).expect("a checksum"));
    assert_eq!(read(&run.tree.join("model/context_v3/release-receipt.json"))["quality_gates_passed"], json!(true));
}

#[test]
fn a_check_that_fails_keeps_the_test_unread() {
    let run = promotion("preflight", &accepted_report(&SEALED));
    let fails = |variable: &str, value: &str, words: &str| {
        let (held, said) = told(promote(&run).arg("--check-only").env(variable, value));
        assert!(!held && said.contains(words) && said.contains("no test byte was read"), "{words}: {said}");
    };
    fails("KS_STUB_PENDING", "1", "uncommitted tracked files");
    fails("KS_STUB_TESTS_FAIL", "1", "the tests of the engine and the trainer");
    fails("KS_STUB_SCRATCH_TESTS_FAIL", "1", "with the candidate pair installed");
    fails("KS_STUB_FIELD", "1", "the owner's typing");
    fails("KS_STUB_BUILD", "1", "the owner's typing");
    let (held, said) = told(promote(&run).arg("--check-only").env("KS_STUB_NO_WORKTREE", "1"));
    assert!(!held && said.contains("no worktree"), "{said}");
    // No development result of the chat test, or one that holds nothing.
    let development = run.chat.join(format!("{}.dev.json", checksum(&FITTED.model).expect("a checksum")));
    let bytes = std::fs::read(&development).expect("a record");
    for held_nothing in [json!({"floors": {}}), json!({"floors": {"a": false}}), json!({})] {
        std::fs::write(&development, serde_json::to_vec(&held_nothing).expect("json")).expect("a record");
        let (held, said) = told(promote(&run).arg("--check-only"));
        assert!(!held && said.contains("developer-chat development"), "{said}");
    }
    std::fs::write(&development, bytes).expect("a record");
    // Seals that do not hold.
    let (other, seal) = (run.folder.join("other"), read(&FITTED.seal));
    std::fs::create_dir_all(&other).expect("a folder");
    std::fs::copy(&FITTED.model, other.join("context-action.json")).expect("a copy");
    let mut changed = seal;
    changed["stage"] = json!("tested");
    std::fs::write(other.join("candidate-seal.json"), serde_json::to_vec(&changed).expect("json")).expect("a seal");
    let (held, said) = told(promote(&Promotion { candidate: other.clone(), ..run.clone() }).arg("--check-only"));
    assert!(!held && said.contains("candidate seal"), "{said}");
    let mut prefix = read(&FITTED.prefix_seal);
    prefix["schema_version"] = json!(9);
    std::fs::copy(&FITTED.prefix, other.join("prefix-candidate.json")).expect("a copy");
    std::fs::write(other.join("seal.json"), serde_json::to_vec(&prefix).expect("json")).expect("a seal");
    let (held, said) = told(promote(&Promotion { prefix: other.clone(), ..run.clone() }).arg("--check-only"));
    assert!(!held && said.contains("prefix seal"), "{said}");
}

#[test]
fn a_test_that_does_not_pass_installs_nothing() {
    let installed = |run: &Promotion| checksum(&run.tree.join("resources/models/context_policy_v1.json")).expect("a checksum");
    // The report says no.
    let mut failed = accepted_report(&SEALED);
    failed["promotion_passed"] = json!(false);
    let run = promotion("failed", &failed);
    let before = installed(&run);
    let (held, said) = told(&mut promote(&run));
    assert!(!held && said.contains("did not pass"), "{said}");
    assert_eq!(installed(&run), before);
    // Passed, and short of the receipt's floors: by the margin, by the share.
    let mut short = accepted_report(&SEALED);
    let cell = &mut short["profiles"]["portable"]["early_off"]["counts"]["baseline"]["exactly_restored"];
    *cell = json!(cell.as_i64().expect("a count") + 1);
    let run = promotion("short", &short);
    let (held, said) = told(&mut promote(&run));
    assert!(!held && said.contains("short of the release receipt"), "{said}");
    let mut few = accepted_report(&SEALED);
    few["profiles"]["portable"]["early_off"]["counts"]["candidate"]["initially_wrong"] = json!(1_000_000);
    let run = promotion("few", &few);
    let (held, said) = told(&mut promote(&run));
    assert!(!held && said.contains("short of the release receipt"), "{said}");
    // The chat test recorded for it holds nothing.
    let run = promotion("chat-failed", &accepted_report(&SEALED));
    let result = run.chat.join("passed.result.json");
    let mut recorded = read(&result);
    recorded["floors"]["f1_better_than_baseline"] = json!(false);
    std::fs::write(&result, serde_json::to_vec(&recorded).expect("json")).expect("a record");
    let (held, said) = told(&mut promote(&run));
    assert!(!held && said.contains("developer-chat test did not pass"), "{said}");
    // No report yet: the test is read, and what it says stands.
    let run = promotion("read", &accepted_report(&SEALED));
    std::fs::remove_file(&run.report).expect("the report goes");
    std::fs::remove_dir_all(&run.ledger).expect("the ledger goes");
    let (held, said) = told(&mut promote(&run));
    assert!(!held && said.contains("sealed test"), "{said}");
    assert!(run.report.is_file());
    // The chat development file that is not there.
    let run = promotion("no-chat", &accepted_report(&SEALED));
    std::fs::remove_dir_all(&run.chat).expect("the ledger goes");
    let (held, said) = told(promote(&run).arg("--check-only"));
    assert!(!held && said.contains("developer-chat development"), "{said}");
}

#[test]
fn a_comparison_counts_what_each_artifact_gains_and_loses_against_the_first() {
    let fitted = &*FITTED;
    let installed = fitted.tree.join("resources/models/context_policy_v1.json");
    let with_threshold = |name: &str, threshold: f64| {
        let mut model = read(&installed);
        model["conversion_threshold"] = json!(threshold);
        let path = fitted.folder.join(format!("compare-{name}.json"));
        std::fs::write(&path, serde_json::to_vec(&model).expect("json")).expect("a model");
        path
    };
    let (eager, never) = (with_threshold("eager", 0.95), with_threshold("never", 1.0));
    let output = fitted.folder.join("compared-thresholds.json");
    let mut command = with_tree("compare", fitted);
    command.arg("--corpus").arg(&fitted.corpus).arg("--ledger").arg(&fitted.ledger).arg("--output").arg(&output).args(["--jobs", "2"]);
    for (name, path) in [("installed", installed.clone()), ("eager", eager), ("never", never), ("baseline", fitted.tree.join("model/context_v3/baseline-context-v1.json"))] {
        command.arg("--artifact").arg(format!("{name}={}", path.display()));
    }
    succeeded(&command.output().expect("the trainer starts"));
    let report = read(&output);
    let differing = |name: &str| ["calibration", "development"].iter().any(|split| report[split]["portable"][name]["differences"]["all"].as_object().is_some_and(|kinds| !kinds.is_empty()));
    assert!(differing("eager") || differing("never"), "{report}");
    assert!(!differing("installed"));
}

#[test]
fn a_seal_whose_calibration_breaks_even_or_whose_recipe_names_other_profiles_is_refused() {
    let fitted = &*FITTED;
    let good = read(&fitted.seal);
    let models = fitted.tree.join("resources/models");
    let even = changed_seal(fitted, &good, "even", |seal| {
        let right = seal["calibration"]["converted_correctly"].clone();
        seal["calibration"]["false_conversions"] = right;
    });
    assert!(validate_candidate_seal(&fitted.tree, &fitted.model, &even, &fitted.corpus, &models).unwrap_err().contains("calibration gate failed"));
    // A tree whose recipe, and the seal's with it, names one profile only.
    let mut recipe = good["recipe"].clone();
    recipe["profiles"] = json!(["portable"]);
    let tree = small_tree("one-profile", &recipe);
    let sealed = changed_seal(fitted, &good, "one-profile", |seal| seal["recipe"] = recipe.clone());
    assert!(validate_candidate_seal(&tree, &fitted.model, &sealed, &fitted.corpus, &tree.join("resources/models")).unwrap_err().contains("gate policy differs"));
}

#[test]
fn inputs_that_change_during_a_replay_stop_its_report() {
    let fitted = &*FITTED;
    let folder = scratch("changed-inputs");
    let seal = folder.join("candidate-seal.json");
    std::fs::copy(&fitted.seal, &seal).expect("a copy");
    let resources = fitted.tree.join("resources");
    let (output, ledger) = (folder.join("report.json"), folder.join("ledger"));
    let mut asked = request(fitted, &resources, "test", &output, &ledger);
    asked.seal = &seal;
    let identity = evaluation_identity(&asked).expect("an identity");
    ks_train::evaluate::unchanged_since(&asked, &identity).expect("nothing changed yet");
    std::fs::write(&seal, b"{}").expect("a file");
    assert!(ks_train::evaluate::unchanged_since(&asked, &identity).unwrap_err().contains("inputs changed"));
}

#[test]
fn a_sealed_test_that_cannot_be_replayed_keeps_the_record_of_its_failure() {
    let fitted = &*FITTED;
    // A tree of the same recipes whose frozen baseline prefix model, which neither seal pins, cannot be read.
    let tree = small_tree("broken-prefix-baseline", &recipe());
    write_recipe(&tree.join("model/prefix_v2/recipe.json"), &read(&fitted.tree.join("model/prefix_v2/recipe.json")));
    let baseline = tree.join("model/prefix_v2/baseline-prefix-v1.json");
    std::fs::remove_file(&baseline).expect("the link goes");
    std::fs::write(&baseline, b"{}").expect("a file");
    let folder = scratch("broken-prefix-baseline-test");
    let (output, ledger) = (folder.join("report.json"), folder.join("ledger"));
    let mut command = trainer();
    command.arg("evaluate").arg("--tree").arg(&tree);
    command
        .arg("--candidate")
        .arg(&fitted.model)
        .arg("--seal")
        .arg(&fitted.seal)
        .arg("--prefix-candidate")
        .arg(&fitted.prefix)
        .arg("--prefix-seal")
        .arg(&fitted.prefix_seal)
        .arg("--corpus")
        .arg(&fitted.corpus)
        .arg("--output")
        .arg(&output)
        .arg("--ledger")
        .arg(&ledger)
        .args(["--split", "test", "--jobs", "2"]);
    refused(&command.output().expect("the trainer starts"), "baseline-prefix-v1.json");
    assert!(read(&output)["execution_error"].is_string());
    let outcomes = std::fs::read_dir(&ledger).expect("the ledger").flatten().filter(|entry| entry.file_name().to_string_lossy().ends_with(".outcome.json")).count();
    assert_eq!(outcomes, 1);
}

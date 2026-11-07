//! The gate a package is built behind, over a tree of links to this repository with one file of its own changed: each
//! piece of evidence that no longer describes an installed model stops it.
mod eval_support;
mod support;

use eval_support::{edit, linked, own, read, sha};
use ks_train::gates;
use serde_json::{Value, json};
use std::path::Path;
use support::tree;

const ORTHO: &str = "model/ortho_v1";

fn ortho_file(name: &str) -> String {
    format!("{ORTHO}/{name}")
}

/// What the orthotactic gate says of a tree changed by `change`.
fn ortho_refuses(name: &str, words: &str, change: impl FnOnce(&Path)) {
    let root = linked(name);
    change(&root);
    let said = gates::ortho(&root).expect_err(name);
    assert!(said.contains(words), "{name}: {said}");
}

fn boundary_refuses(name: &str, words: &str, change: impl FnOnce(&Path)) {
    let root = linked(name);
    change(&root);
    let said = gates::boundary(&root).expect_err(name);
    assert!(said.contains(words), "{name}: {said}");
}

#[test]
fn the_orthotactic_evidence_of_the_repository_holds_and_each_change_of_it_stops_the_gate() {
    let held = gates::ortho(&linked("ortho-holds")).expect("the evidence holds");
    assert_eq!(held["model_version"], read(&tree().join(ortho_file("seal.json")))["model_version"]);
    // The receipts of the inputs.
    ortho_refuses("ortho-schema", "invalid metadata", |root| edit(root, &ortho_file("corpus-receipt.json"), |receipt| receipt["schema_version"] = json!(9)));
    ortho_refuses("ortho-tokens", "key-space corpus does not match", |root| edit(root, &ortho_file("corpus-receipt.json"), |receipt| receipt["tokens_sha256"] = json!("0")));
    ortho_refuses("ortho-evidence", "dictionary evidence changed", |root| edit(root, &ortho_file("known-receipt.json"), |receipt| receipt["evidence_sha256"] = json!("0")));
    ortho_refuses("ortho-known-tokens", "dictionary evidence changed", |root| edit(root, &ortho_file("known-receipt.json"), |receipt| receipt["tokens_sha256"] = json!("0")));
    ortho_refuses("ortho-forms", "web-lexicon-ru.json.gz does not match its receipt", |root| edit(root, &ortho_file("web-lexicon-receipt.json"), |receipt| receipt["forms_sha256"] = json!("0")));
    // The seal.
    let seal = ortho_file("seal.json");
    ortho_refuses("ortho-stage", "orthotactic seal changed", |root| edit(root, &seal, |seal| seal["stage"] = json!("tested")));
    ortho_refuses("ortho-candidate", "orthotactic seal changed", |root| edit(root, &seal, |seal| seal["candidate_sha256"] = json!("0")));
    ortho_refuses("ortho-no-pins", "a seal without provenance", |root| edit(root, &seal, |seal| seal["provenance"] = json!(null)));
    ortho_refuses("ortho-pin", "orthotactic provenance changed: model/ortho_v1/config.json", |root| edit(root, &seal, |seal| seal["provenance"]["model/ortho_v1/config.json"] = json!("0")));
    ortho_refuses("ortho-config", "configuration changed after the seal", |root| edit(root, &seal, |seal| seal["config"] = json!({})));
    // The report.
    let report = ortho_file("report.json");
    for (field, value) in [("seal_sha256", json!("0")), ("candidate_sha256", json!("0")), ("thresholds", json!({}))] {
        ortho_refuses(&format!("ortho-report-{field}"), "does not describe this candidate", |root| edit(root, &report, |report| report[field] = value));
    }
    ortho_refuses("ortho-not-passed", "did not pass its promotion gates", |root| edit(root, &report, |report| report["promotion_passed"] = json!(false)));
    // The independent test: its files, the report's account of it, and its population.
    for field in ["tokens_sha256", "evidence_sha256", "snapshot_tokens_sha256"] {
        ortho_refuses(&format!("ortho-independent-{field}"), "independent test files do not match their receipt", |root| edit(root, &ortho_file("independent/receipt.json"), |receipt| receipt[field] = json!("0")));
    }
    for field in ["set", "receipt_sha256", "tokens_sha256", "evidence_sha256", "source"] {
        ortho_refuses(&format!("ortho-described-{field}"), "does not describe the frozen independent test", |root| edit(root, &report, |report| report["independent_test"][field] = json!("other")));
    }
    ortho_refuses("ortho-no-counts", "no test counts", |root| edit(root, &report, |report| report["corpus_test"] = json!({})));
    ortho_refuses("ortho-population", "does not describe the frozen independent test", |root| {
        edit(root, &report, |report| report["corpus_test"]["counts"]["en:negative_types"] = json!(1));
    });
    // Without an independent test, the counts of the corpus test alone.
    let report_file: &str = &report;
    let alone = |change: fn(&mut Value)| {
        move |root: &Path| {
            edit(root, report_file, |report| {
                report["independent_test"] = json!(null);
                change(report);
            });
        }
    };
    ortho_refuses("ortho-alone-no-counts", "no test counts", alone(|report| report["corpus_test"] = json!({})));
    ortho_refuses("ortho-alone-few", "too small to bound the risk", alone(|report| {
        report["corpus_test"]["counts"]["en:negative_types"] = json!(0);
        report["corpus_test"]["counts"]["ru:negative_types"] = json!(0);
    }));
    ortho_refuses("ortho-alone-false", "reports false conversions", alone(|report| report["corpus_test"]["counts"]["ru:false_types"] = json!(1)));
    ortho_refuses("ortho-recall", "recall is below the promotion gate", |root| edit(root, &report, |report| report["recall"] = json!(0.1)));
    // The installed model.
    ortho_refuses("ortho-installed", "not the evaluated candidate", |root| std::fs::write(own(root, "resources/models/ortho_v1.json"), b"{}").expect("a file"));
    ortho_refuses("ortho-identity", "identity differs from the seal", |root| {
        edit(root, &ortho_file("candidate.json"), |candidate| candidate["version"] = json!("other"));
        let candidate = sha(root, &ortho_file("candidate.json"));
        edit(root, &seal, |seal| seal["candidate_sha256"] = json!(candidate));
        let sealed = sha(root, &seal);
        edit(root, &report, |report| {
            report["seal_sha256"] = json!(sealed);
            report["candidate_sha256"] = json!(candidate);
        });
        std::fs::copy(root.join(ortho_file("candidate.json")), own(root, "resources/models/ortho_v1.json")).expect("a copy");
    });
}

#[test]
fn the_boundary_evidence_of_the_repository_holds_and_each_change_of_it_stops_the_gate() {
    assert_eq!(gates::boundary(&linked("boundary-holds")).expect("the evidence holds")["accepted"], json!(true));
    boundary_refuses("boundary-seal", "sealed quality evidence changed", |root| edit(root, "model/boundary_v2/seal.json", |seal| seal["candidate_sha256"] = json!("0")));
    for field in ["candidate_sha256", "seal_sha256"] {
        boundary_refuses(&format!("boundary-report-{field}"), "sealed quality evidence changed", |root| edit(root, "model/boundary_v2/report.json", |report| report[field] = json!("0")));
    }
    boundary_refuses("boundary-rejected", "rejected by its test", |root| edit(root, "model/boundary_v2/report.json", |report| report["accepted"] = json!(false)));
    boundary_refuses("boundary-installed", "missing or changed", |root| std::fs::write(own(root, "resources/models/boundary-v2.json"), b"{}").expect("a file"));
    boundary_refuses("boundary-regression", "engine comparisons missing", |root| std::fs::write(own(root, "model/boundary_v1/engine-regression.json"), b"{}").expect("a file"));
}

#[test]
fn a_pair_sealed_by_this_trainer_has_its_sources_checked_too() {
    let root = linked("receipt-of-this-trainer");
    edit(&root, "model/context_v3/release-receipt.json", |receipt| receipt["provenance"]["Cargo.lock"] = json!("0"));
    assert!(gates::run(&root).is_err());
}

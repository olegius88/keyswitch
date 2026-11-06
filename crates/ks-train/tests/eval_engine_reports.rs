//! The engine reports of the boundary and prefix models: the gate over the kept boundary regression, and the reports
//! made again in a scratch tree, each way they refuse.
mod support;

use ks_train::engine_reports::verify_boundary_report;
use serde_json::{Value, json};
use support::tree;

fn read(path: &std::path::Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("a file")).expect("json")
}

#[test]
fn the_boundary_regression_gate_wants_every_comparison_and_every_text_exact() {
    let good = read(&tree().join("model/boundary_v1/engine-regression.json"));
    verify_boundary_report(&good).expect("the kept report holds");
    let refused = |words: &str, change: &dyn Fn(&mut Value)| {
        let mut report = good.clone();
        change(&mut report);
        let said = verify_boundary_report(&report).expect_err(words);
        assert!(said.contains(words), "{words}: {said}");
    };
    refused("comparisons missing", &|report| report["results"] = json!([]));
    refused("comparisons missing", &|report| {
        report["results"].as_object_mut().expect("the results").remove("legacy_no_segmentation");
    });
    refused("comparisons missing", &|report| {
        let results = report["results"].as_object_mut().expect("the results");
        let moved = results.remove("rejected_v1").expect("a comparison");
        results.insert("other".to_owned(), moved);
    });
    refused("exact-text gate failed", &|report| report["results"]["active_v2"]["exact"] = json!(1));
    for field in ["changed_correct", "length_mismatches", "injections_before_hard_boundary"] {
        refused("exact-text gate failed", &|report| report["results"]["active_v2"][field] = json!(1));
    }
}

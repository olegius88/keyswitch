//! The engine reports of the boundary and prefix models: the gate over the kept boundary regression, and the reports
//! made again in a scratch tree, each way they refuse.
mod eval_support;
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

fn engine(root: &std::path::Path, which: &str, extra: &[&str]) -> std::process::Output {
    support::trainer().arg("engine").arg(which).arg("--tree").arg(root).args(extra).args(["--jobs", "2"]).output().expect("the trainer starts")
}

fn said(output: &std::process::Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

#[test]
fn a_boundary_report_is_written_once_and_then_only_verified_or_refreshed() {
    let small = support::small_tree("engine-boundary-once", &support::recipe());
    let report = small.join("model/boundary_v1/engine-regression.json");
    let answer = engine(&small, "boundary", &[]);
    assert!(!answer.status.success() && said(&answer).contains("already exists"), "{}", said(&answer));
    std::fs::remove_file(&report).expect("the report goes");
    let answer = engine(&small, "boundary", &[]);
    assert!(answer.status.success(), "{}", said(&answer));
    assert!(report.is_file());
    // Without an orthotactic model the regression still runs, written elsewhere.
    std::fs::write(small.join("resources/models/ortho_v1.json"), b"{}").expect("a file");
    let output = support::scratch("engine-boundary-no-ortho").join("report.json");
    let _ = engine(&small, "boundary", &["--output", output.to_str().expect("a path")]);
    assert!(output.is_file());
}

/// The prefix test split of a small tree cut down to two sequences of each category, with its receipt's pin of it: a
/// replay of the prefix model over it takes seconds.
fn small_prefix_split(small: &std::path::Path) {
    use std::io::{BufRead as _, Write as _};
    let directory = small.join("model/prefix_v1");
    let file = std::fs::File::open(directory.join("test.jsonl.gz")).expect("the split");
    let mut kept: Vec<String> = Vec::new();
    let mut taken: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for line in std::io::BufReader::new(flate2::read::MultiGzDecoder::new(file)).lines() {
        let line = line.expect("a line");
        let row: Value = serde_json::from_str(&line).expect("json");
        let key = format!("{}:{}:{}", row["category"], row["length"], row["profile"]);
        let count = taken.entry(key).or_insert(0);
        if *count < 2 {
            *count += 1;
            kept.push(line);
        }
    }
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all((kept.join("\n") + "\n").as_bytes()).expect("in memory");
    let packed = encoder.finish().expect("in memory");
    std::fs::write(eval_support::own(small, "model/prefix_v1/test.jsonl.gz"), &packed).expect("a split");
    let corpus = directory.join("corpus.json");
    let mut receipt = read(&corpus);
    receipt["sha256"]["test"] = json!(support::digest(&packed));
    std::fs::write(&corpus, serde_json::to_vec(&receipt).expect("json")).expect("a file");
}

#[test]
fn a_prefix_report_refuses_a_changed_corpus_or_selection_and_a_candidate_short_of_its_gates() {
    let small = support::small_tree("engine-prefix", &support::recipe());
    let (corpus, report) = (small.join("model/prefix_v1/corpus.json"), small.join("model/prefix_v1/engine-report.json"));
    // A split that is not the one frozen.
    let kept = std::fs::read(&corpus).expect("the corpus");
    let mut changed = read(&corpus);
    changed["sha256"]["test"] = json!("0");
    std::fs::write(&corpus, serde_json::to_vec(&changed).expect("json")).expect("a file");
    let answer = engine(&small, "prefix", &["--verify"]);
    assert!(!answer.status.success() && said(&answer).contains("checksum mismatch"), "{}", said(&answer));
    std::fs::write(&corpus, &kept).expect("a file");
    small_prefix_split(&small);
    // A refresh may not change what was selected.
    let mut previous = read(&report);
    let ids = previous["sequence_ids"].clone();
    previous["sequence_ids"] = json!([0]);
    std::fs::write(&report, serde_json::to_vec(&previous).expect("json")).expect("a file");
    let answer = engine(&small, "prefix", &["--refresh-runtime"]);
    assert!(!answer.status.success() && said(&answer).contains("cannot change the observed selection"), "{}", said(&answer));
    // A report kept without an earlier one to name is not written over.
    previous["sequence_ids"] = ids;
    previous.as_object_mut().expect("a report").remove("runtime_regression");
    std::fs::write(&report, serde_json::to_vec(&previous).expect("json")).expect("a file");
    let answer = engine(&small, "prefix", &[]);
    assert!(!answer.status.success() && said(&answer).contains("already exists"), "{}", said(&answer));
    // No report yet, and a candidate that never switches early: the report is written, and its gate does not hold.
    std::fs::remove_file(&report).expect("the report goes");
    let candidate = eval_support::own(&small, "model/prefix_v1/candidate.json");
    let mut model = read(&candidate);
    model["conversion_threshold"] = json!(1.0);
    std::fs::write(&candidate, serde_json::to_vec(&model).expect("json")).expect("a file");
    let answer = engine(&small, "prefix", &[]);
    assert!(!answer.status.success() && said(&answer).contains("prefix engine gates failed"), "{}", said(&answer));
    assert_eq!(read(&report)["passed"], json!(false));
    assert!(read(&report).get("runtime_regression").is_none());
}

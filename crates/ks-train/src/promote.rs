//! A sealed context-v3 and prefix-v2 pair promoted: checks first, the sealed test once, then the install
//! (`promote_context_action_pair`). A sealed test is read once, so everything the candidate's seal pins has to be final
//! before that read: this refuses to touch the test until every check holds.
use ks_cast::To as _;
use crate::corpus::read_json;
use crate::evaluate::{EvaluationRequest, evaluate, validate_candidate_seal, validate_prefix_seal};
use crate::mixed::{DevchatRequest, devchat};
use crate::receipt::{export_receipt, validate_receipt};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

const CONTEXT_ARTIFACT: &str = "context-action.json";
const CONTEXT_SEAL: &str = "candidate-seal.json";
const PREFIX_ARTIFACT: &str = "prefix-candidate.json";
const PREFIX_SEAL: &str = "seal.json";
const INSTALLED_CONTEXT: &str = "resources/models/context_policy_v1.json";
const INSTALLED_CONTEXT_SEAL: &str = "model/context_v3/candidate-seal.json";
const INSTALLED_PREFIX: &str = "resources/models/prefix_policy_v1.json";
const INSTALLED_PREFIX_SEAL: &str = "model/prefix_v2/candidate-seal.json";
const RECEIPT: &str = "model/context_v3/release-receipt.json";
const TESTED_CRATES: [&str; 7] = ["ks-text", "ks-lang", "ks-context", "ks-engine", "ks-config", "ks-replay", "ks-train"];
/// The floors the release receipt holds the sealed test to: more rows restored than the frozen baseline pair, by a mode.
const RECEIPT_MIN_RESTORATION_MARGIN: [(&str, i64); 2] = [("default", 0), ("early_off", 1)];
const RECEIPT_MIN_RESTORED_SHARE: f64 = 0.97;

pub struct PromoteRequest<'a> {
    pub root: &'a Path,
    pub candidate: &'a Path,
    pub prefix_candidate: &'a Path,
    pub corpus: &'a Path,
    pub report: &'a Path,
    pub released_rows: &'a Path,
    /// The tree whose private folder holds the owner's typing; the tree itself unless it is elsewhere.
    pub field_root: &'a Path,
    pub devchat_directory: &'a Path,
    pub ledger: &'a Path,
    pub devchat_ledger: &'a Path,
    pub check_only: bool,
    pub jobs: usize,
}

fn run(command: &mut Command, label: &str) -> bool {
    println!("$ {label}");
    command.status().is_ok_and(|status| status.success())
}

fn tests_of(tree: &Path) -> Command {
    let mut command = Command::new("cargo");
    // A tree's tests read files beside their sources, whose paths are compiled in: a target folder of its own.
    command.current_dir(tree).env("CARGO_TARGET_DIR", tree.join("target")).args(["test", "--profile", "parity"]);
    for name in TESTED_CRATES {
        command.args(["-p", name]);
    }
    command
}

/// `preflight`: every check of the pinned files, before any test byte is read; the failures, empty if none.
fn preflight(request: &PromoteRequest<'_>) -> Result<Vec<String>, String> {
    let root = request.root;
    let mut failures = Vec::new();
    let changed = Command::new("git").current_dir(root).args(["diff", "--name-only", "HEAD"]).output().map_err(|error| error.to_string())?;
    let pending = String::from_utf8_lossy(&changed.stdout).trim().to_owned();
    if !pending.is_empty() {
        failures.push(format!("uncommitted tracked files: {}", pending.replace('\n', ", ")));
    }
    let models_dir = root.join("resources/models");
    if let Err(error) = validate_candidate_seal(root, &request.candidate.join(CONTEXT_ARTIFACT), &request.candidate.join(CONTEXT_SEAL), request.corpus, &models_dir) {
        failures.push(format!("candidate seal: {error}"));
    }
    if let Err(error) = validate_prefix_seal(root, &request.prefix_candidate.join(PREFIX_ARTIFACT), &request.prefix_candidate.join(PREFIX_SEAL), true) {
        failures.push(format!("prefix seal: {error}"));
    }
    if !run(&mut tests_of(root), "the tests of the engine and the trainer") {
        failures.push("the tests of the engine and the trainer".to_owned());
    }
    if failures.is_empty() {
        // The same tests with the candidate pair installed, in a scratch worktree of the commit; then the owner's typing
        // in that tree against the last release.
        let scratch = std::env::temp_dir().join(format!("keyswitch-candidate-{}", std::process::id()));
        if !run(Command::new("git").current_dir(root).args(["worktree", "add", "--detach"]).arg(&scratch).arg("HEAD"), "a worktree of the commit") {
            return Err("no worktree of the commit".to_owned());
        }
        for (source, target) in installed_files(request) {
            std::fs::copy(&source, scratch.join(target)).map_err(|error| error.to_string())?;
        }
        if !run(&mut tests_of(&scratch), "the tests with the candidate pair installed") {
            failures.push("the tests, with the candidate pair installed".to_owned());
        }
        let mut build = Command::new("cargo");
        build.current_dir(&scratch).env("CARGO_TARGET_DIR", scratch.join("target")).args(["build", "--profile", "parity", "-p", "ks-replay", "--bins"]);
        let built = run(&mut build, "the replay of the candidate tree");
        let mut field = Command::new(scratch.join("target/parity/field-gate"));
        field.current_dir(&scratch).arg("--root").arg(request.field_root).arg("--released").arg(request.released_rows).arg("--replay").arg(scratch.join("target/parity/ks-replay"));
        if !built || !run(&mut field, "the owner's typing against the last release") {
            failures.push("the owner's typing: a word the last release got right comes out wrong, or the field regression gate could not run".to_owned());
        }
        let _ = Command::new("git").current_dir(root).args(["worktree", "remove", "--force"]).arg(&scratch).status();
    }
    let development = request.devchat_ledger.join(format!("{}.dev.json", crate::corpus::checksum(&request.candidate.join(CONTEXT_ARTIFACT))?));
    let held = read_json(&development).ok().is_some_and(|record| record["floors"].as_object().is_some_and(|floors| !floors.is_empty() && floors.values().all(|value| value == &Value::Bool(true))));
    if !held {
        failures.push("no developer-chat development result of the candidate holds both floors (ks-train devchat --split dev)".to_owned());
    }
    Ok(failures)
}

fn installed_files(request: &PromoteRequest<'_>) -> [(PathBuf, &'static str); 4] {
    [
        (request.candidate.join(CONTEXT_ARTIFACT), INSTALLED_CONTEXT),
        (request.candidate.join(CONTEXT_SEAL), INSTALLED_CONTEXT_SEAL),
        (request.prefix_candidate.join(PREFIX_ARTIFACT), INSTALLED_PREFIX),
        (request.prefix_candidate.join(PREFIX_SEAL), INSTALLED_PREFIX_SEAL),
    ]
}

/// `restoration_shortfall`: where the sealed test leaves the candidate short of the release receipt's floors.
fn restoration_shortfall(report: &Value) -> Vec<String> {
    let mut failures = Vec::new();
    for (name, profile) in report["profiles"].as_object().into_iter().flatten() {
        for (mode, counts) in [("default", &profile["counts"]), ("early_off", &profile["early_off"]["counts"])] {
            let (candidate, baseline) = (&counts["candidate"], &counts["baseline"]);
            let restored = candidate["exactly_restored"].as_i64().unwrap_or(0);
            let margin = RECEIPT_MIN_RESTORATION_MARGIN.iter().find(|entry| entry.0 == mode).map_or(0, |entry| entry.1);
            if restored - baseline["exactly_restored"].as_i64().unwrap_or(0) < margin {
                failures.push(format!("{name}/{mode}: the margin over the baseline pair"));
            }
            if mode == "early_off" && restored.to::<f64>() < RECEIPT_MIN_RESTORED_SHARE * (candidate["initially_wrong"].as_i64().unwrap_or(0)).to::<f64>() {
                failures.push(format!("{name}/{mode}: the share restored"));
            }
        }
    }
    failures
}

/// `promote`: the checks, the sealed test (once), the install and the receipt.
pub fn promote(request: &PromoteRequest<'_>) -> Result<i32, String> {
    let failures = preflight(request)?;
    if !failures.is_empty() {
        println!("not ready for the sealed test; no test byte was read:");
        for failure in &failures {
            println!("  - {failure}");
        }
        return Ok(1);
    }
    println!("every check holds");
    if request.check_only {
        return Ok(0);
    }
    let root = request.root;
    let resource_dir = root.join("resources");
    let (artifact, seal, prefix, prefix_seal) = (request.candidate.join(CONTEXT_ARTIFACT), request.candidate.join(CONTEXT_SEAL), request.prefix_candidate.join(PREFIX_ARTIFACT), request.prefix_candidate.join(PREFIX_SEAL));
    let evaluation = EvaluationRequest { root, resource_dir: &resource_dir, artifact: &artifact, seal: &seal, prefix: &prefix, prefix_seal: &prefix_seal, corpus: request.corpus, split: "test", output: request.report, ledger: request.ledger, jobs: request.jobs, unsealed: false };
    let report = if request.report.exists() { read_json(request.report)? } else { evaluate(&evaluation)? };
    if report["promotion_passed"] != Value::Bool(true) {
        println!("the sealed test did not pass; nothing is installed");
        return Ok(1);
    }
    let shortfall = restoration_shortfall(&report);
    if !shortfall.is_empty() {
        println!("the sealed test leaves the candidate short of the release receipt's restoration floors ({}); nothing is installed and the developer-chat test is not read", shortfall.join("; "));
        return Ok(1);
    }
    // The developer-chat test, while the released pair is still the installed one it is compared with.
    let released = root.join(INSTALLED_CONTEXT);
    let baseline = root.join("model/context_v3/baseline-context-v1.json");
    let output = request.report.with_file_name("devchat-test.json");
    let result = devchat(&DevchatRequest { root, resource_dir: &resource_dir, split: "test", directory: request.devchat_directory, candidate: &artifact, released: &released, baseline: &baseline, output: &output, ledger: request.devchat_ledger, sequences_ledger: request.ledger, workers: request.jobs })?;
    if !result["floors"].as_object().is_some_and(|floors| floors.values().all(|value| value == &Value::Bool(true))) {
        println!("the sealed developer-chat test did not pass; nothing is installed");
        return Ok(1);
    }
    for (source, target) in installed_files(request) {
        std::fs::copy(&source, root.join(target)).map_err(|error| error.to_string())?;
    }
    // The receipt is an immutable record: the one of the replaced pair goes first.
    let receipt_path = root.join(RECEIPT);
    let _ = std::fs::remove_file(&receipt_path);
    let (installed_seal, installed_prefix, installed_prefix_seal) = (root.join(INSTALLED_CONTEXT_SEAL), root.join(INSTALLED_PREFIX), root.join(INSTALLED_PREFIX_SEAL));
    let installed = EvaluationRequest { artifact: &released, seal: &installed_seal, prefix: &installed_prefix, prefix_seal: &installed_prefix_seal, ..evaluation };
    let receipt = export_receipt(&installed, request.report, request.devchat_ledger, &receipt_path)?;
    validate_receipt(&receipt, &released, root, &installed_prefix, &resource_dir.join("models"))?;
    println!("installed {}", receipt["model_version"].as_str().unwrap_or(""));
    Ok(0)
}

//! A sealed context-v3 and prefix-v2 pair promoted: checks first, the sealed test once, then the install
//! (`promote_context_action_pair`). A sealed test is read once, so everything the candidate's seal pins has to be final
//! before that read: this refuses to touch the test until every check holds.
use ks_cast::To as _;
use crate::artifact::typed;
use crate::corpus::read_json;
use crate::evaluate::{EvaluationRequest, evaluate, validate_candidate_seal, validate_prefix_seal};
use crate::mixed::{DevchatRequest, devchat};
use crate::receipt::{export_receipt, validate_receipt};
use crate::sequence_report::{Profiles, Report};
use ks_text::{Indent, JsonOptions};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
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
const RECIPE: &str = "model/context_v3/recipe.json";
/// The crates of the program, whose tests run in the tree and again in a scratch tree with the candidate pair installed.
const PROGRAM_CRATES: [&str; 6] = ["ks-text", "ks-lang", "ks-context", "ks-engine", "ks-config", "ks-replay"];
/// The trainer's tests run in the tree only: they hold the kept records (the release receipt, the engine replays of the
/// boundary and prefix models) to the installed pair, and a candidate has no receipt before its sealed test is read.
const TRAINER_CRATE: &str = "ks-train";
/// The names the comparison of the calibration gives the two artifacts, and the file it is written to, beside the report.
const INSTALLED_NAME: &str = "installed";
const CANDIDATE_NAME: &str = "candidate";
const CALIBRATION_COMPARISON: &str = "calibration-comparison.json";
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

/// The tests of the program's crates in a tree, every test binary run whatever an earlier one says, so that a failure
/// names all the others.
fn tests_of(tree: &Path) -> Command {
    let mut command = Command::new("cargo");
    // A tree's tests read files beside their sources, whose paths are compiled in: a target folder of its own.
    command.current_dir(tree).env("CARGO_TARGET_DIR", tree.join("target")).args(["test", "--profile", "parity", "--no-fail-fast"]);
    for name in PROGRAM_CRATES {
        command.args(["-p", name]);
    }
    command
}

/// What the check of the calibration reads of a comparison (`compare`): each artifact's net on the calibration frames,
/// by profile.
#[derive(Deserialize)]
struct Comparison {
    calibration: BTreeMap<String, BTreeMap<String, Scored>>,
}

#[derive(Deserialize)]
struct Scored {
    by_class: Classes,
}

#[derive(Deserialize)]
struct Classes {
    all: Net,
}

#[derive(Deserialize)]
struct Net {
    net: i64,
}

/// `calibration_shortfall`: the profiles in which the candidate nets less on the calibration frames than the installed
/// pair. The quality ratchet holds the receipt to the installed pair's number there, so such a candidate could not be
/// installed whatever its sealed test said.
fn calibration_shortfall(comparison: &Comparison) -> Vec<String> {
    let net = |pairs: &BTreeMap<String, Scored>, name: &str| pairs.get(name).map(|scored| scored.by_class.all.net);
    comparison.calibration.iter().filter(|(_, pairs)| net(pairs, CANDIDATE_NAME) < net(pairs, INSTALLED_NAME)).map(|(profile, _)| profile.clone()).collect()
}

/// The candidate and the installed pair compared on the corpus's calibration frames, the comparison written beside the
/// report; the profiles the candidate falls short in.
fn calibration(request: &PromoteRequest<'_>) -> Result<Vec<String>, String> {
    let root = request.root;
    let inputs = crate::fit::load_inputs(root, &root.join("resources"), request.corpus, read_json(&root.join(RECIPE))?, request.ledger)?;
    let artifacts = [(INSTALLED_NAME.to_owned(), root.join(INSTALLED_CONTEXT)), (CANDIDATE_NAME.to_owned(), request.candidate.join(CONTEXT_ARTIFACT))];
    let report = crate::compare::compare(&inputs, &artifacts, request.jobs)?;
    let path = request.report.with_file_name(CALIBRATION_COMPARISON);
    let format = JsonOptions { ensure_ascii: false, sort_keys: true, indent: Some(Indent::Spaces(1)), ..JsonOptions::default() };
    let text = ks_text::json_dumps(&report, &format) + "\n";
    std::fs::write(&path, &text).map_err(|error| format!("{}: {error}", path.display()))?;
    let comparison: Comparison = ks_text::json::loads(text).map_err(|error| error.to_string())?;
    Ok(calibration_shortfall(&comparison))
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
    match calibration(request) {
        Ok(short) => failures.extend(short.iter().map(|profile| format!("the calibration nets less than the installed pair's in the {profile} profile: the quality ratchet would refuse the install"))),
        Err(error) => failures.push(format!("the calibration could not be compared with the installed pair's: {error}")),
    }
    if !run(tests_of(root).args(["-p", TRAINER_CRATE]), "the tests of the engine and the trainer") {
        failures.push("the tests of the engine and the trainer".to_owned());
    }
    if failures.is_empty() {
        // The program's tests with the candidate pair installed, in a scratch worktree of the commit; then the owner's
        // typing in that tree against the last release. The gate runs in the scratch tree: what it reads is named from
        // anywhere, and the owner's typing stays in the tree it lives in.
        let (field_root, released) = (std::path::absolute(request.field_root).map_err(|error| error.to_string())?, std::path::absolute(request.released_rows).map_err(|error| error.to_string())?);
        let scratch = std::env::temp_dir().join(format!("keyswitch-candidate-{}", std::process::id()));
        if !run(Command::new("git").current_dir(root).args(["worktree", "add", "--detach"]).arg(&scratch).arg("HEAD"), "a worktree of the commit") {
            return Err("no worktree of the commit".to_owned());
        }
        for (source, target) in installed_files(request) {
            std::fs::copy(&source, scratch.join(target)).map_err(|error| error.to_string())?;
        }
        if !run(&mut tests_of(&scratch), "the program's tests with the candidate pair installed") {
            failures.push("the tests, with the candidate pair installed".to_owned());
        }
        let mut build = Command::new("cargo");
        build.current_dir(&scratch).env("CARGO_TARGET_DIR", scratch.join("target")).args(["build", "--profile", "parity", "-p", "ks-replay", "--bins"]);
        let built = run(&mut build, "the replay of the candidate tree");
        let mut field = Command::new(scratch.join("target/parity/field-gate"));
        field.current_dir(&scratch).arg("--root").arg(&field_root).arg("--released").arg(&released).arg("--replay").arg(scratch.join("target/parity/ks-replay"));
        if !built || !run(&mut field, "the owner's typing against the last release") {
            failures.push("the owner's typing: a word the last release got right comes out wrong, or the field regression gate could not run".to_owned());
        }
        let _ = Command::new("git").current_dir(root).args(["worktree", "remove", "--force"]).arg(&scratch).status();
    }
    // Every condition under which the developer-chat test is read, checked by the read itself before any byte of either
    // sealed test: the development result, its floors, its runtime and pairs, the pinned messages, enough fresh ones.
    let (artifact, released, baseline) = (request.candidate.join(CONTEXT_ARTIFACT), root.join(INSTALLED_CONTEXT), root.join("model/context_v3/baseline-context-v1.json"));
    let resource_dir = root.join("resources");
    let output = request.report.with_file_name("devchat-test.json");
    let check = DevchatRequest {
        root,
        resource_dir: &resource_dir,
        split: "test",
        directory: request.devchat_directory,
        candidate: &artifact,
        released: &released,
        baseline: &baseline,
        output: &output,
        ledger: request.devchat_ledger,
        sequences_ledger: request.ledger,
        workers: request.jobs,
        check_only: true,
    };
    if let Err(error) = devchat(&check) {
        failures.push(format!("the developer-chat test could not be read: {error}"));
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

/// `restoration_shortfall`: where the sealed test leaves the candidate short of the release receipt's floors. The default
/// mode is held to its first plans, where every wrong word stands at the start of a text and so can be restored; its
/// sequences keep their gates. A report without the first plans of a profile is no test a receipt can be made of: an
/// error, before the developer-chat test is read.
fn restoration_shortfall(report: &Report) -> Result<Vec<String>, String> {
    let mut failures = Vec::new();
    for (name, profile) in report.profiles.iter().flat_map(Profiles::each) {
        let first = profile.first.as_ref().ok_or_else(|| format!("{name}: the report holds no first plans of the default mode, which the receipt's floors are read from"))?;
        let default = (&first.counts.candidate, &first.counts.baseline);
        let control = (&profile.early_off.counts.candidate, &profile.early_off.counts.baseline);
        for (mode, (candidate, baseline)) in [("default", default), ("early_off", control)] {
            let restored = candidate.exactly_restored;
            let margin = RECEIPT_MIN_RESTORATION_MARGIN.iter().find(|entry| entry.0 == mode).map_or(0, |entry| entry.1);
            if restored - baseline.exactly_restored < margin {
                failures.push(format!("{name}/{mode}: the margin over the baseline pair"));
            }
            if mode == "early_off" && restored.to::<f64>() < RECEIPT_MIN_RESTORED_SHARE * candidate.initially_wrong.to::<f64>() {
                failures.push(format!("{name}/{mode}: the share restored"));
            }
        }
    }
    Ok(failures)
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
    let report = if request.report.exists() { typed::<Report, _>(&read_json(request.report)?)? } else { evaluate(&evaluation)? };
    if !report.promotion_passed {
        println!("the sealed test did not pass; nothing is installed");
        return Ok(1);
    }
    let shortfall = restoration_shortfall(&report)?;
    if !shortfall.is_empty() {
        println!("the sealed test leaves the candidate short of the release receipt's restoration floors ({}); nothing is installed and the developer-chat test is not read", shortfall.join("; "));
        return Ok(1);
    }
    // The developer-chat test, while the released pair is still the installed one it is compared with.
    let released = root.join(INSTALLED_CONTEXT);
    let baseline = root.join("model/context_v3/baseline-context-v1.json");
    let output = request.report.with_file_name("devchat-test.json");
    let result = devchat(&DevchatRequest { root, resource_dir: &resource_dir, split: "test", directory: request.devchat_directory, candidate: &artifact, released: &released, baseline: &baseline, output: &output, ledger: request.devchat_ledger, sequences_ledger: request.ledger, workers: request.jobs, check_only: false })?;
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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::evaluate::Earliness;
    use crate::sequence_report::{AllPairs, EarlyOff, FirstBlock, FirstGates, Gated, Gates, PairCounts, Profile};

    fn comparison(text: &str) -> Comparison {
        ks_text::json::loads(text).expect("a comparison")
    }

    #[test]
    fn a_candidate_that_nets_less_on_the_calibration_than_the_installed_pair_falls_short_in_that_profile() {
        let scored = |installed: i64, candidate: i64| format!(r#"{{"installed": {{"by_class": {{"all": {{"net": {installed}}}}}}}, "candidate": {{"by_class": {{"all": {{"net": {candidate}}}}}}}}}"#);
        let both = comparison(&format!(r#"{{"calibration": {{"portable": {}, "reference_hunspell": {}}}, "development": {{}}}}"#, scored(13816, 13824), scored(13862, 13861)));
        assert_eq!(calibration_shortfall(&both), ["reference_hunspell"]);
        // Breaking even holds; a candidate the comparison does not name nets nothing.
        let even = comparison(&format!(r#"{{"calibration": {{"portable": {}}}}}"#, scored(5, 5)));
        assert!(calibration_shortfall(&even).is_empty());
        let unnamed = comparison(r#"{"calibration": {"portable": {"installed": {"by_class": {"all": {"net": 0}}}}}}"#);
        assert_eq!(calibration_shortfall(&unnamed), ["portable"]);
    }

    /// The default mode is held to its first plans, not to its sequences; early_off is held as before; a report without
    /// first plans holds no floor at all.
    #[test]
    fn the_default_floor_is_read_from_the_first_plans() {
        let counts = |candidate: i64, baseline: i64| Gated { candidate: PairCounts { exactly_restored: candidate, initially_wrong: candidate, ..PairCounts::default() }, baseline: PairCounts { exactly_restored: baseline, ..PairCounts::default() } };
        let held = || Gates {
            correct_text_preserved: true,
            length_preserved: true,
            net_restorations_at_least_baseline: true,
            enough_documents_us: true,
            enough_documents_ru: true,
            execution_succeeded: true,
            correction_layouts_match: true,
        };
        let profile = |sequences: Gated<PairCounts>, first: Gated<PairCounts>, control: Gated<PairCounts>| {
            let by_pair = |pairs: Gated<PairCounts>| AllPairs { baseline: pairs.baseline.clone(), candidate: pairs.candidate.clone(), candidate_context_baseline_prefix: pairs.candidate, installed: pairs.baseline };
            let none = || Gated { baseline: Vec::new(), candidate: Vec::new() };
            Profile {
                counts: sequences,
                cases: none(),
                gates: held(),
                ablation: None,
                installed: None,
                first: Some(FirstBlock {
                    counts: by_pair(first),
                    earliness: AllPairs { baseline: Earliness::default(), candidate: Earliness::default(), candidate_context_baseline_prefix: Earliness::default(), installed: Earliness::default() },
                    cases: AllPairs { baseline: Vec::new(), candidate: Vec::new(), candidate_context_baseline_prefix: Vec::new(), installed: Vec::new() },
                    gates: FirstGates { net_restorations_at_least_baseline: true, correct_text_preserved: true, length_preserved: true },
                }),
                early_off: EarlyOff { counts: control, cases: none(), gates: held() },
            }
        };
        let report = |sequences: Gated<PairCounts>, first: Gated<PairCounts>| {
            let same = || profile(sequences.clone(), first.clone(), counts(10, 9));
            Report {
                schema_version: 1,
                split: "test".to_owned(),
                identity: ks_text::json::loads(r#"{"unsealed": true}"#).expect("a tree"),
                protocol: ks_text::json::loads(r#"{"version": 5}"#).expect("a tree"),
                gate_policy: ks_text::json::loads("{}").expect("a tree"),
                promotion_passed: true,
                installed_reference: None,
                selection: None,
                profiles: Some(Profiles { portable: same(), reference_hunspell: same() }),
                execution_error: None,
                scope: None,
            }
        };
        let short = |profile: &str| vec![format!("{profile}/default: the margin over the baseline pair")];
        let mut names = short("portable");
        names.extend(short("reference_hunspell"));
        assert_eq!(restoration_shortfall(&report(counts(9, 9), counts(5, 6))).expect("first plans"), names);
        assert!(restoration_shortfall(&report(counts(5, 6), counts(6, 6))).expect("first plans").is_empty());
        // early_off keeps its margin of one row and its share.
        let mut behind = report(counts(9, 9), counts(6, 6));
        for (_, profile) in behind.profiles.iter_mut().flat_map(Profiles::each_mut) {
            profile.early_off.counts = counts(9, 9);
        }
        assert_eq!(restoration_shortfall(&behind).expect("first plans"), ["portable/early_off: the margin over the baseline pair", "reference_hunspell/early_off: the margin over the baseline pair"]);
        // No first plans: not zeros that would hold the floor of no margin.
        let mut without = report(counts(6, 6), counts(6, 6));
        without.profiles.as_mut().expect("the profiles").reference_hunspell.first = None;
        assert!(restoration_shortfall(&without).expect_err("no first plans").starts_with("reference_hunspell: the report holds no first plans"));
    }
}

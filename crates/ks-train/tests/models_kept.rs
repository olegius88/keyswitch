//! The models kept in the repository, replayed and made again in a scratch tree of it: what the release runs, and what a new cycle of each does.
mod support;

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use support::{recipe, scratch, small_tree, trainer, tree};

fn run(command: &mut Command) -> Output {
    command.output().expect("the trainer starts")
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn done(output: &Output) -> String {
    assert!(output.status.success(), "{}", said(output));
    said(output)
}

fn refused(output: &Output, words: &str) {
    assert!(!output.status.success() && said(output).contains(words), "{words}: {}", said(output));
}

fn on(root: &Path, command: &[&str]) -> Command {
    let mut trainer = trainer();
    trainer.args(command).arg("--tree").arg(root);
    trainer
}

fn copy_folder(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("a folder");
    for entry in std::fs::read_dir(from).expect("a folder").flatten() {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), to.join(entry.file_name())).expect("a copy");
        }
    }
}

#[test]
fn every_rejected_or_retired_model_replays() {
    let root = tree();
    let folder = scratch("retired");
    for family in ["boundary-v1", "prefix-v1", "ortho-v2", "context-v2"] {
        done(&run(on(&root, &["retired", family]).arg("--replay")));
    }
    let output = folder.join("context-engine.json");
    done(&run(on(&root, &["retired", "context-engine"]).arg("--output").arg(&output)));
    assert!(output.is_file());
    done(&run(&mut on(&root, &["retired", "context-engine"])));
    refused(&run(&mut on(&root, &["retired", "unheard-of"])), "unknown retired family");
    refused(&run(&mut on(Path::new("/nonexistent"), &["retired", "boundary-v1"])), "");
}

#[test]
fn the_engine_reports_are_refreshed_and_then_hold() {
    let small = small_tree("engine-reports", &recipe());
    for which in ["boundary", "prefix"] {
        done(&run(on(&small, &["engine", which]).arg("--refresh-runtime").args(["--jobs", "2"])));
        done(&run(on(&small, &["engine", which]).arg("--verify").args(["--jobs", "2"])));
    }
    let output = scratch("engine-output").join("report.json");
    done(&run(on(&small, &["engine", "boundary"]).arg("--output").arg(&output)));
    assert!(output.is_file());
    refused(&run(&mut on(&small, &["engine", "other"])), "");
    let report = small.join("model/boundary_v1/engine-regression.json");
    std::fs::write(&report, b"{}").expect("a file");
    refused(&run(on(&small, &["engine", "boundary"]).arg("--verify")), "");
}

#[test]
fn the_boundary_model_is_made_again_step_by_step() {
    let small = small_tree("boundary", &recipe());
    let folder = scratch("boundary-experiment");
    let experiment = folder.join("experiment");
    copy_folder(&tree().join("model/boundary_v2"), &experiment);
    let step = |action: &str| {
        let mut command = on(&small, &["boundary"]);
        command.arg(format!("--{action}")).arg("--directory").arg(&experiment);
        run(&mut command)
    };
    // The test was read: no new fit, and no second reading.
    refused(&step("train"), "test already observed");
    refused(&step("test"), "test already observed");
    std::fs::remove_file(experiment.join("report.json")).expect("the report goes");
    done(&step("train"));
    assert!(experiment.join("development-history").is_dir());
    done(&step("train"));
    done(&step("test"));
    done(&step("verify"));
    done(&step("promote"));
    assert_eq!(std::fs::read(small.join("resources/models/boundary-v2.json")).expect("the model"), std::fs::read(experiment.join("candidate.json")).expect("the candidate"));
    refused(&step("freeze"), "refusing to overwrite");
    // What no longer holds.
    let report = experiment.join("report.json");
    let bytes = std::fs::read(&report).expect("the report");
    std::fs::write(&report, b"{}").expect("a file");
    refused(&step("verify"), "not reproducible");
    refused(&step("promote"), "failed promotion gates");
    std::fs::write(&report, bytes).expect("a file");
    let seal: Value = serde_json::from_slice(&std::fs::read(experiment.join("seal.json")).expect("the seal")).expect("json");
    let mut changed = seal;
    changed["epochs"] = json!(-1);
    std::fs::write(experiment.join("seal.json"), serde_json::to_vec(&changed).expect("json")).expect("a file");
    refused(&step("verify"), "");
    assert!(said(&run(&mut on(&small, &["boundary"]))).contains("--freeze"));
    // Made from its sources in a new folder.
    let fresh = folder.join("fresh");
    let mut command = on(&small, &["boundary"]);
    command.arg("--freeze").arg("--directory").arg(&fresh);
    let answer = run(&mut command);
    println!("{}", said(&answer));
}

#[test]
fn the_orthotactic_model_is_made_again_step_by_step() {
    let small = small_tree("ortho", &recipe());
    let folder = scratch("ortho-experiment");
    let experiment = folder.join("experiment");
    let step = |action: &str| {
        let mut command = on(&small, &["ortho"]);
        command.arg(format!("--{action}")).arg("--output").arg(&experiment);
        run(&mut command)
    };
    refused(&step("promote"), "");
    done(&step("fit"));
    refused(&step("fit"), "do not refit");
    done(&step("verify"));
    refused(&step("promote"), "");
    done(&step("test"));
    refused(&step("test"), "test already observed");
    done(&step("promote"));
    let report = experiment.join("report.json");
    let bytes = std::fs::read(&report).expect("the report");
    let mut changed: Value = serde_json::from_slice(&bytes).expect("json");
    changed["promotion_passed"] = json!(false);
    std::fs::write(&report, serde_json::to_vec(&changed).expect("json")).expect("a file");
    refused(&step("promote"), "without a passing report");
    std::fs::remove_file(&report).expect("the report goes");
    let seal_path = experiment.join("seal.json");
    let mut seal: Value = serde_json::from_slice(&std::fs::read(&seal_path).expect("the seal")).expect("json");
    seal["candidate_sha256"] = json!("0");
    std::fs::write(&seal_path, serde_json::to_vec(&seal).expect("json")).expect("a file");
    refused(&step("test"), "changed after the seal");
    let candidate = experiment.join("candidate.json");
    std::fs::write(&candidate, b"{}").expect("a file");
    refused(&step("verify"), "not reproducible");
    assert!(said(&run(&mut on(&small, &["ortho"]))).contains("--fit"));
    // Inputs.
    done(&run(&mut on(&tree(), &["ortho", "--inputs"])));
    done(&run(&mut on(&small, &["ortho", "--lexicon-sources"])));
    let frozen = folder.join("frozen-inputs");
    let answer = run(on(&small, &["ortho", "--freeze-inputs"]).arg("--output").arg(&frozen));
    println!("{}", said(&answer));
    assert!(!run(&mut on(&tree(), &["ortho", "--freeze-independent", "/nonexistent", "--files", "/nonexistent"])).status.success());
    assert!(!run(&mut on(&tree(), &["ortho", "--freeze-independent", "/nonexistent", "--files", "/nonexistent", "--check-within", "x"])).status.success());
}

#[test]
fn the_prefix_corpus_is_made_from_its_config() {
    let small = small_tree("prefix-corpus", &recipe());
    let path = small.join("model/prefix_v1/config.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&path).expect("the config")).expect("json");
    let shrink = |config: &mut Value| {
        for (name, value) in config.as_object_mut().expect("a table").iter_mut() {
            if name.contains("families") || name.contains("words") {
                if let Some(number) = value.as_u64() {
                    *value = json!((number / 50).max(4));
                }
            }
        }
    };
    shrink(&mut config);
    std::fs::write(&path, serde_json::to_vec(&config).expect("json")).expect("a file");
    let folder = scratch("prefix-frozen");
    let output = folder.join("frozen");
    done(&run(on(&small, &["prefix-corpus"]).arg("--freeze").arg(&output)));
    assert!(output.join("train.jsonl.gz").is_file());
    refused(&run(on(&small, &["prefix-corpus"]).arg("--freeze").arg(&output)), "refusing to overwrite");
    // Against what is kept in the repository, it differs: a smaller config gives fewer rows.
    refused(&run(&mut on(&small, &["prefix-corpus"])), "differ");
    refused(&run(&mut on(Path::new("/nonexistent"), &["prefix-corpus"])), "");
}

#[test]
fn the_small_tools_do_what_they_say() {
    let root = tree();
    let folder = scratch("small-tools");
    // A base for a head to be fitted again.
    let pair = root.join("resources/models/context_policy_v1.json");
    let output = folder.join("carried.json");
    let carry = |head: &str| run(on(&root, &["carry"]).arg("--pair").arg(&pair).args(["--refit", head]).arg("--output").arg(&output));
    assert!(said(&carry("unheard of")).contains("no head"));
    let answer = done(&carry("letter"));
    assert!(answer.contains("context-v3-"), "{answer}");
    refused(&run(on(&root, &["carry"]).arg("--pair").arg(&pair).arg("--output").arg(&output)), "name a head");
    refused(&run(on(&root, &["carry"]).arg("--pair").arg("/nonexistent").args(["--refit", "letter"]).arg("--output").arg(&output)), "");
    // What can run on this tree.
    done(&run(&mut on(&root, &["doctor"])));
    let answer = run(on(&root, &["doctor"]).args(["--stage", "promote-pair", "--stage", "replay-boundary-v2"]).arg("--corpus").arg("/nonexistent").arg("--ledger").arg("/nonexistent"));
    refused(&answer, "Blocked requested stages");
    let corpus = folder.join("doctor-corpus");
    std::fs::create_dir_all(&corpus).expect("a folder");
    std::fs::write(corpus.join("manifest.json"), b"{}").expect("a file");
    done(&run(on(&root, &["doctor"]).args(["--stage", "promote-pair"]).arg("--corpus").arg(&corpus).arg("--ledger").arg(&corpus)));
    // The supplement.
    let input = folder.join("input.json");
    let (kept, receipt, removed) = (folder.join("kept.json"), folder.join("receipt.json"), folder.join("removed.txt"));
    let filter = || run(on(&root, &["supplement"]).arg("--input").arg(&input).arg("--output").arg(&kept).arg("--receipt").arg(&receipt).arg("--removed-list").arg(&removed));
    std::fs::write(&input, serde_json::to_vec(&json!({"name": "raw", "scope": "words.", "locale": "ru_RU", "selection": {"selected": 3}, "words": ["абвгдежз", "дома", "работа", "работах"]})).expect("json")).expect("a file");
    done(&filter());
    assert!(removed.is_file() && receipt.is_file());
    let no_list = run(on(&root, &["supplement"]).arg("--input").arg(&input).arg("--output").arg(&kept).arg("--receipt").arg(&receipt));
    done(&no_list);
    std::fs::write(&input, serde_json::to_vec(&json!({"locale": "en_US", "words": ["a"]})).expect("json")).expect("a file");
    refused(&filter(), "not a sorted Russian supplement");
    std::fs::write(&input, serde_json::to_vec(&json!({"locale": "ru_RU", "words": ["б", "а"]})).expect("json")).expect("a file");
    refused(&filter(), "not a sorted Russian supplement");
    std::fs::write(&input, serde_json::to_vec(&json!({"name": "opensubtitles-2018-ru-full-min10-outside-onboard-v4", "locale": "ru_RU", "words": ["а"]})).expect("json")).expect("a file");
    refused(&filter(), "already filtered");
    // The archive of the retired families.
    let small = small_tree("archive", &recipe());
    done(&run(on(&small, &["archive"]).arg("--write")));
    done(&run(&mut on(&small, &["archive"])));
    let record = small.join("model/archive-sha256.json");
    let mut changed: Value = serde_json::from_slice(&std::fs::read(&record).expect("the record")).expect("json");
    let table = changed.as_object_mut().expect("a table");
    let first = table.keys().next().expect("a file").clone();
    table[&first] = json!("0");
    table.insert("gone".to_owned(), json!("0"));
    std::fs::write(&record, serde_json::to_vec(&changed).expect("json")).expect("a file");
    refused(&run(&mut on(&small, &["archive"])), "archived files moved");
    std::fs::write(&record, b"[]").expect("a file");
    refused(&run(&mut on(&small, &["archive"])), "archived files moved");
    // The notices of third parties.
    done(&run(&mut on(&small, &["licenses"])));
    done(&run(on(&small, &["licenses"]).arg("--verify")));
    std::fs::write(small.join("packaging/third-party-licenses/linux.txt"), b"other").expect("a file");
    refused(&run(on(&small, &["licenses"]).arg("--verify")), "fell behind the lockfile");
    // The detector, on a sample, strictly.
    let answer = run(on(&root, &["detector"]).args(["--sample", "20", "--dictionary-sample", "20", "--strict"]));
    println!("{}", said(&answer));
    // The gate of coverage.
    let gate = |report: &Path| run(on(&root, &["coverage-gate"]).arg("--report").arg(report));
    refused(&gate(Path::new("/nonexistent")), "/nonexistent");
    let bad = folder.join("bad-report.json");
    std::fs::write(&bad, b"not json").expect("a file");
    refused(&gate(&bad), "");
    let outside = folder.join("outside-report.json");
    std::fs::write(&outside, serde_json::to_vec(&json!({"data": [{"files": [{"filename": "/elsewhere/lib.rs", "segments": [], "branches": []}]}]})).expect("json")).expect("a file");
    done(&gate(&outside));
    let full = folder.join("full-report.json");
    std::fs::write(&full, serde_json::to_vec(&json!({"data": [{"files": [{"filename": "/r/crates/x/src/lib.rs", "segments": [[1, 1, 1, true, true, false]], "branches": []}]}]})).expect("json")).expect("a file");
    done(&gate(&full));
    let gaps = folder.join("gaps-report.json");
    std::fs::write(&gaps, serde_json::to_vec(&json!({"data": [{"files": [{"filename": "/r/crates/x/src/lib.rs", "segments": [[1, 1, 0, true, true, false], [2, 1, 0, false, false, false]], "branches": [[3, 5, 3, 9, 1, 0, 0, 0, 4], [4, 5, 4, 9, 0, 1, 0, 0, 4], [5, 5, 5, 9, 0, 0, 0, 0, 4]]}]}]})).expect("json")).expect("a file");
    refused(&gate(&gaps), "code no test ran");
    let native = PathBuf::from("/nonexistent/native.txt");
    assert!(ks_train::coverage_gate::check(&root, &[], &native).is_err());
    let listed = folder.join("native.txt");
    std::fs::write(&listed, "crates/no-such-file.rs\n").expect("a file");
    assert!(ks_train::coverage_gate::check(&root, &[], &listed).unwrap_err().contains("files that are gone"));
    // How the trainer is asked.
    refused(&run(&mut trainer()), "usage");
    let no_tree = run(trainer().args(["gates"]));
    assert!(!no_tree.status.success());
    for command in ["prefix", "evaluate", "messages", "intent", "typo-evaluation", "deb-check", "coverage-gate", "licenses", "archive", "prefix-corpus", "detector", "ortho", "typo-frequency", "engine", "boundary", "doctor", "carry", "fit", "compare", "holdout", "capture", "retired"] {
        assert!(!run(trainer().arg(command)).status.success(), "{command}");
    }
    refused(&run(&mut on(&root, &["base"])), "base freeze|reconcile");
    refused(&run(&mut on(&root, &["base", "freeze"])), "--namespace is required");
    refused(&run(on(&root, &["base", "freeze"]).args(["--namespace", "x", "--source-root", "/nonexistent", "--output", "/nonexistent/x", "--max-documents", "many"])), "takes a number");
    refused(&run(on(&root, &["base", "freeze"]).args(["--namespace", "x", "--source-root", "/nonexistent", "--output", "/nonexistent/x", "--max-documents", "0"])), "must be positive");
    refused(&run(trainer().args(["base", "freeze"])), "--tree is required");
    let holdout = run(on(&root, &["holdout"]).args(["--base", "/x", "--ud-origin", "/x", "--technical-origin", "/x", "--prefix-inventory", "/x", "--namespace", "n", "--source-root", "/x"]));
    refused(&holdout, "exactly one technical source");
}

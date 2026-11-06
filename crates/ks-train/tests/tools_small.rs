//! The small tools of a cycle where what they read is made up: the notices of the crates over a stub of `cargo`, a base
//! for heads fitted again, what a machine lacks, the supplement and its receipt changed one field at a time.
mod support;

use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Output;
use support::{scratch, trainer};

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

fn executable(path: &Path, body: &str) {
    std::fs::write(path, body).expect("a file");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("a mode");
}

/// A `cargo` that answers `metadata` with the file beside it, fails when asked to, or says nothing a reader can parse.
fn cargo_stub(folder: &Path, metadata: &Path) -> std::ffi::OsString {
    let bin = folder.join("stub-bin");
    std::fs::create_dir_all(&bin).expect("a folder");
    executable(
        &bin.join("cargo"),
        &format!("#!/bin/sh\n[ -z \"$KS_CARGO_FAILS\" ] || {{ echo 'no lockfile here' >&2; exit 101; }}\n[ -z \"$KS_CARGO_GARBAGE\" ] || {{ echo 'not json'; exit 0; }}\ncat {}\n", metadata.display()),
    );
    let mut path = bin.into_os_string();
    path.push(":");
    path.push(std::env::var_os("PATH").expect("a PATH"));
    path
}

/// The metadata of one program and what it reaches: a dependency through a cycle, one only for its tests, one the graph
/// has no node of, a dependency that names no package, a package without a name and a node without an identifier.
fn metadata(folder: &Path) -> Value {
    let crate_folder = |name: &str| {
        let path = folder.join("registry").join(name);
        std::fs::create_dir_all(&path).expect("a folder");
        path
    };
    let used = crate_folder("used-1.0.0");
    std::fs::write(used.join("LICENSE-MIT"), "Copyright the authors\r\nPermission is granted.\r\n\r\n").expect("a file");
    std::fs::write(used.join("COPYING"), "Copying is allowed.\n").expect("a file");
    std::fs::write(used.join("README.md"), "not a notice\n").expect("a file");
    std::fs::create_dir_all(used.join("licenses")).expect("a folder of notices is not a notice");
    let bare = crate_folder("bare-2.0.0");
    let gone = crate_folder("gone-3.0.0");
    let own = crate_folder("ks-linux");
    let package = |id: &str, name: &str, version: &str, source: Value, folder: &Path| {
        json!({"id": id, "name": name, "version": version, "source": source, "manifest_path": folder.join("Cargo.toml"), "license": "MIT", "repository": format!("https://example.org/{name}")})
    };
    let registry = json!("registry+https://github.com/rust-lang/crates.io-index");
    let normal = json!([{"kind": null}]);
    json!({
        "packages": [
            package("own 0.1.0", "ks-linux", "0.1.0", Value::Null, &own),
            package("used 1.0.0", "used", "1.0.0", registry.clone(), &used),
            package("bare 2.0.0", "bare", "2.0.0", registry.clone(), &bare),
            package("gone 3.0.0", "gone", "3.0.0", registry.clone(), &gone),
            package("tested 4.0.0", "tested", "4.0.0", registry, &bare),
            {"id": "nameless 5.0.0", "version": "5.0.0"},
        ],
        "resolve": {"nodes": [
            {"id": "own 0.1.0", "deps": [
                {"pkg": "used 1.0.0", "dep_kinds": normal},
                {"pkg": "tested 4.0.0", "dep_kinds": [{"kind": "dev"}]},
                {"pkg": "gone 3.0.0", "dep_kinds": normal},
                {"dep_kinds": normal},
            ]},
            {"id": "used 1.0.0", "deps": [{"pkg": "own 0.1.0", "dep_kinds": normal}, {"pkg": "bare 2.0.0", "dep_kinds": normal}, {"pkg": "nameless 5.0.0", "dep_kinds": normal}]},
            {"id": "bare 2.0.0", "deps": []},
            {"id": "nameless 5.0.0"},
            {"id": "tested 4.0.0", "deps": []},
            {"deps": []},
        ]},
    })
}

fn licenses(root: &Path, path: &std::ffi::OsString, verify: bool, environment: &[(&str, &str)]) -> Output {
    let mut command = trainer();
    command.arg("licenses").arg("--tree").arg(root).env("PATH", path).envs(environment.iter().copied());
    if verify {
        command.arg("--verify");
    }
    command.output().expect("the trainer starts")
}

#[test]
fn the_notices_list_what_a_program_reaches_and_nothing_else() {
    let folder = scratch("licenses");
    let root = folder.join("tree");
    std::fs::create_dir_all(&root).expect("a folder");
    let described = folder.join("metadata.json");
    std::fs::write(&described, metadata(&folder).to_string()).expect("a file");
    let path = cargo_stub(&folder, &described);
    let written = done(&licenses(&root, &path, false, &[]));
    assert!(written.contains("linux: 3 crates") && written.contains("macos: 0 crates"), "{written}");
    let linux = std::fs::read_to_string(root.join("packaging/third-party-licenses/linux.txt")).expect("the list");
    assert!(linux.contains("used 1.0.0\nLicense: MIT\nSource: https://example.org/used\n"), "{linux}");
    assert!(linux.contains("--- COPYING ---\nCopying is allowed.\n") && linux.contains("--- LICENSE-MIT ---\nCopyright the authors\nPermission is granted.\n"), "{linux}");
    // A package the graph has no node of is still reached; one only for tests, a program of the workspace and a package
    // without a name are not listed.
    assert!(linux.contains("bare 2.0.0") && linux.contains("gone 3.0.0") && !linux.contains("README") && !linux.contains("tested") && !linux.contains("ks-linux") && !linux.contains("5.0.0"), "{linux}");
    done(&licenses(&root, &path, true, &[]));
    std::fs::remove_file(root.join("packaging/third-party-licenses/windows.txt")).expect("the list goes");
    refused(&licenses(&root, &path, true, &[]), "windows.txt");
    refused(&licenses(&root, &path, false, &[("KS_CARGO_FAILS", "1")]), "no lockfile here");
    refused(&licenses(&root, &path, false, &[("KS_CARGO_GARBAGE", "1")]), "");
}

/// A scratch tree of the repository's files: these linked, those copied so that a test may change them.
fn tree_of(name: &str, linked: &[&str], copied: &[&str]) -> PathBuf {
    let root = scratch(name);
    for (relative, link) in linked.iter().map(|path| (path, true)).chain(copied.iter().map(|path| (path, false))) {
        let (source, target) = (support::tree().join(relative), root.join(relative));
        std::fs::create_dir_all(target.parent().expect("a parent")).expect("a folder");
        if link {
            std::os::unix::fs::symlink(source, target).expect("a link");
        } else {
            std::fs::copy(source, target).expect("a copy");
        }
    }
    root
}

#[test]
fn a_base_keeps_every_head_but_those_fitted_again() {
    let pair = json!({"version": "context-v3-pair", "weights": {"letter|a": [1.0], "start|b": [2.0]}});
    let refit = |names: &[&str]| ks_train::small_tools::carried(&pair, &names.iter().map(|name| (*name).to_owned()).collect::<Vec<_>>());
    let base = refit(&["letter"]).expect("a base");
    assert_eq!(base["weights"], json!({"start|b": [2.0]}));
    assert!(base["version"].as_str().expect("a version").starts_with("context-v3-") && base["version"] != pair["version"]);
    assert!(refit(&["capitals"]).expect_err("a head the pair has no weights of").contains("no weights for a head"));
    assert!(refit(&[]).expect_err("no head").contains("name a head"));
    assert!(ks_train::small_tools::carried(&json!({"weights": []}), &["letter".to_owned()]).expect_err("weights in a list").contains("without weights"));
}

#[test]
fn a_machine_without_the_files_is_told_what_it_lacks() {
    let empty = scratch("doctor-empty");
    let answer = done(&trainer().arg("doctor").arg("--tree").arg(&empty).output().expect("the trainer starts"));
    assert!(answer.contains("[NO] reference_models") && answer.contains("missing model/prefix_v1/train.jsonl.gz"), "{answer}");
    assert!(answer.contains("[blocked] replay-boundary-v2"), "{answer}");
}

const SUPPLEMENT: &str = "resources/lexicon-supplement-ru_RU.json";
const SUPPLEMENT_RECEIPT: &str = "resources/lexicon-supplement-ru_RU.receipt.json";
const INTENT_CONFIG: &str = "model/intent_v1/config.json";

fn words_digest(words: &[&str]) -> String {
    let mut sorted = words.to_vec();
    sorted.sort_unstable();
    support::digest(sorted.iter().map(|word| format!("{word}\n")).collect::<String>().as_bytes())
}

/// A supplement of these kept words, and the receipt of a filter that dropped those forms, every digest as it writes them.
fn supplement(root: &Path, kept: &[&str], dropped: &[&str]) -> Value {
    let raw = serde_json::to_vec(&json!({"locale": "ru_RU", "words": kept})).expect("json");
    std::fs::write(root.join(SUPPLEMENT), &raw).expect("a file");
    let together: Vec<&str> = kept.iter().chain(dropped).copied().collect();
    let shipped: Value = serde_json::from_slice(&std::fs::read(support::tree().join(SUPPLEMENT_RECEIPT)).expect("the receipt")).expect("json");
    json!({"output": {"sha256": support::digest(&raw)}, "references": shipped["references"], "input": {"words": together.len(), "words_sha256": words_digest(&together)}, "dropped": {"forms": dropped}})
}

fn verify_supplement(root: &Path, receipt: &Value) -> Output {
    std::fs::write(root.join(SUPPLEMENT_RECEIPT), receipt.to_string()).expect("a receipt");
    trainer().arg("supplement").arg("--tree").arg(root).arg("--verify").output().expect("the trainer starts")
}

#[test]
fn a_supplement_holds_to_its_receipt_and_to_the_rule() {
    let root = tree_of("supplement", &["model/intent_v1/sources"], &[INTENT_CONFIG]);
    std::fs::create_dir_all(root.join("resources")).expect("a folder");
    // "петр" is a Hunspell word only as "Петр", "абба" only the start of "аббат".
    let good = supplement(&root, &["дома", "петр"], &["абба"]);
    let answer = done(&verify_supplement(&root, &good));
    assert!(answer.contains("\"kept\": 2") && answer.contains("\"dropped\": 1"), "{answer}");
    let changed = |edit: &dyn Fn(&mut Value)| {
        let mut receipt = good.clone();
        edit(&mut receipt);
        receipt
    };
    refused(&verify_supplement(&root, &changed(&|receipt| receipt["output"]["sha256"] = json!("0".repeat(64)))), "is not the recorded output");
    refused(&verify_supplement(&root, &changed(&|receipt| receipt["references"]["hunspell_affix"]["sha256"] = json!("0".repeat(64)))), "changed since the filter ran");
    refused(&verify_supplement(&root, &changed(&|receipt| receipt["input"]["words_sha256"] = json!("0".repeat(64)))), "not the recorded input");
    refused(&verify_supplement(&root, &changed(&|receipt| receipt["input"]["words"] = json!(4))), "not the recorded input");
    let kept_truncation = supplement(&root, &["абба", "дома"], &[]);
    refused(&verify_supplement(&root, &kept_truncation), "a kept form meets the drop rule: абба");
    let dropped_word = supplement(&root, &[], &["абба", "дома"]);
    refused(&verify_supplement(&root, &dropped_word), "a dropped form does not meet the rule");
    // The pins of the intent configuration, each changed.
    let good = supplement(&root, &["дома", "петр"], &["абба"]);
    let config_path = root.join(INTENT_CONFIG);
    let config: Value = serde_json::from_slice(&std::fs::read(&config_path).expect("the config")).expect("json");
    for pointer in ["/sources/languages/ru_RU/sha256", "/external_evaluation/hunspell/ru_RU/dictionary_sha256", "/external_evaluation/hunspell/ru_RU/affix_sha256"] {
        let mut other = config.clone();
        *other.pointer_mut(pointer).expect("a pin") = json!("0".repeat(64));
        std::fs::write(&config_path, other.to_string()).expect("a config");
        refused(&verify_supplement(&root, &good), "differs from the intent configuration");
    }
    std::fs::write(&config_path, config.to_string()).expect("a config");
    // A dictionary of the same bytes from another folder is not the pinned one.
    let elsewhere = scratch("supplement-hunspell");
    for name in ["ru_RU.dic", "ru_RU.aff"] {
        std::fs::copy(support::tree().join("model/intent_v1/sources/hunspell").join(name), elsewhere.join(name)).expect("a copy");
    }
    std::fs::write(root.join(SUPPLEMENT_RECEIPT), good.to_string()).expect("a receipt");
    let answer = trainer().arg("supplement").arg("--tree").arg(&root).arg("--verify").env("KEYSWITCH_HUNSPELL_PATH", &elsewhere).output().expect("the trainer starts");
    refused(&answer, "is not the one loaded");
    done(&verify_supplement(&root, &good));
}

/// Each command, given everything it asks for, on a tree that is not there: it says why it stops, and stops with 1.
#[test]
fn every_command_on_a_tree_that_is_not_there_says_why_it_stops() {
    let out = scratch("nothing-there");
    let output = |name: &str| out.join(name).display().to_string();
    let none = "/nonexistent/input";
    let commands: Vec<Vec<String>> = [
        vec!["prefix", "--output", &output("prefix")],
        vec!["evaluate", "--candidate", none, "--prefix-candidate", none, "--corpus", none, "--output", &output("evaluation.json")],
        vec!["evaluate", "--candidate", none, "--seal", none, "--prefix-candidate", none, "--prefix-seal", none, "--corpus", none, "--output", &output("evaluation.json"), "--split", "test", "--ledger", none, "--unsealed"],
        vec!["messages", "--out", &output("messages"), "--taiga", none],
        vec!["terms", "--messages", none, "--out", &output("terms.json")],
        vec!["intent", "--verify"],
        vec!["typo-evaluation", "synthetic", "--messages", none, "--out", &output("typo")],
        vec!["gates"],
        vec!["ortho", "--lexicon-sources", "--web-source", none],
        vec!["ortho", "--inputs"],
        vec!["ortho", "--freeze-inputs", "--output", &output("ortho-inputs")],
        vec!["ortho", "--verify"],
        vec!["typo-frequency"],
        vec!["typo-frequency", "--verify", "--private", none],
        vec!["engine", "prefix", "--verify"],
        vec!["boundary", "--verify"],
        vec!["compare", "--corpus", none, "--artifact", "installed=/nonexistent/model.json"],
        vec!["verify"],
        vec!["verify", "--evidence-only", "--receipt", none, "--artifact", none, "--prefix", none],
        vec!["promote", "--candidate", none, "--prefix-candidate", none, "--corpus", none, "--report", none, "--released", none, "--devchat", none, "--check-only"],
        vec!["capture", "--manifest", none, "--messages", none, "--model", none],
        vec!["mixed", "--messages", none, "--output", &output("mixed.json"), "--count", "1"],
        vec!["devchat", "--split", "dev", "--directory", none, "--candidate", none, "--output", &output("devchat.json")],
        vec!["holdout", "--base", none, "--ud-origin", none, "--technical-origin", none, "--prefix-inventory", none, "--namespace", "n", "--source-root", none, "--treebank", "x", "--sid-directory", none, "--output", &output("holdout")],
        vec!["fitting", "--base", none, "--ud-origin", none, "--technical-origin", none, "--prefix-inventory", none, "--namespace", "n", "--output", &output("fitting")],
        vec!["fit", "--corpus", none, "--output", &output("fit")],
        vec!["supplement", "--input", none],
        vec!["supplement", "--verify"],
        vec!["apt-build", "--repository", &output("apt"), "--package", none],
        vec!["apt-verify", "--repository", none, "--expect-version", "1.0.0"],
        vec!["licenses"],
        vec!["carry", "--pair", none, "--refit", "letter", "--output", &output("carried.json")],
    ]
    .into_iter()
    .map(|command| command.into_iter().map(str::to_owned).collect())
    .collect();
    let mut unexpected = Vec::new();
    for command in commands {
        let answer = trainer().args(&command).args(["--tree", "/nonexistent"]).output().expect("the trainer starts");
        if answer.status.code() != Some(1) || answer.stderr.is_empty() {
            unexpected.push(format!("{command:?}: {:?} {}", answer.status, said(&answer)));
        }
    }
    assert!(unexpected.is_empty(), "{}", unexpected.join("\n"));
}

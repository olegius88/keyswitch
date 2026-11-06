//! The tools as a person runs them, over the files of this repository: what they check, and how they stop.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tree() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("the repository")
}

fn train(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ks-train"))
        .args(arguments)
        .env("KEYSWITCH_HUNSPELL_PATH", tree().join("model/intent_v1/sources/hunspell"))
        .output()
        .expect("it runs")
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn on_tree(command: &[&str], rest: &[&str]) -> Output {
    let root = tree();
    let mut arguments: Vec<&str> = command.to_vec();
    arguments.extend(["--tree", root.to_str().expect("a path")]);
    arguments.extend(rest);
    train(&arguments)
}

#[test]
fn what_is_kept_in_the_repository_checks_out() {
    for command in [
        &["gates"][..],
        &["licenses"],
        &["archive"],
        &["doctor"],
        &["retired", "boundary-v1"],
        &["retired", "prefix-v1"],
        &["retired", "ortho-v2"],
        &["retired", "context-v2"],
        &["retired", "context-corpus"],
        &["supplement"],
    ] {
        let rest: &[&str] = if command == ["licenses"] || command == ["supplement"] { &["--verify"] } else { &[] };
        let output = on_tree(command, rest);
        assert!(output.status.success(), "{command:?}: {}", said(&output));
    }
}

#[test]
fn the_models_kept_reproduce() {
    for command in [&["boundary"][..], &["ortho"]] {
        let output = on_tree(command, &["--verify"]);
        assert!(output.status.success(), "{command:?}: {}", said(&output));
    }
}

#[test]
fn a_detector_is_measured_on_a_sample() {
    let output = on_tree(&["detector"], &["--sample", "20", "--dictionary-sample", "20"]);
    assert!(output.status.success(), "{}", said(&output));
}

//! The engine against what the previous engine did in its own tests: hotkeys, undo, selection commands, rules, focus
//! changes, foreign input, hook gaps, the queue, sensitive fields, typo correction and backend failures. Optimised
//! builds only: a debug build takes minutes over the same traces.
#![cfg(not(debug_assertions))]

use std::process::Command;

#[test]
fn the_engine_replays_every_recorded_trace() {
    let traces = concat!(env!("CARGO_MANIFEST_DIR"), "/traces");
    let status = Command::new(env!("CARGO_BIN_EXE_ks-trace")).args(["--all", traces, "--jobs", "4"]).status().expect("ks-trace");
    assert!(status.success(), "a trace differed; ks-trace printed which");
}

//! A chat-test read on which the engine fails for a pair: each message it failed on counts as an error, the read goes on
//! to its end and no floor of every message typed holds. A program of its own: the failure is set up in the folder the
//! bench's first worker types in, which every read of this process shares.
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]
mod support;

use ks_train::mixed::{DevchatRequest, devchat};
use serde_json::{Value, json};
use support::{chat_messages, recipe, scratch, small_tree};

#[test]
fn messages_the_engine_fails_on_count_as_errors() {
    let folder = scratch("chat-failing");
    let tree = small_tree("chat-failing-tree", &recipe());
    let directory = folder.join("messages");
    chat_messages(&tree, &directory, &["запусти git status", "docker запускает новый контейнер"], 2);
    let candidate = folder.join("candidate.json");
    std::fs::copy(tree.join("resources/models/context_policy_v1.json"), &candidate).expect("a copy");
    let baseline = tree.join("model/context_v3/baseline-context-v1.json");
    // A learning file the engine refuses to read, where the candidate is typed: the bench removes the folder after it.
    let worker = std::env::temp_dir().join(format!("ks-mixed-{}-0", std::process::id()));
    std::fs::create_dir_all(&worker).expect("a folder");
    std::fs::write(worker.join("learning.json"), r#"{"rules": {"²:слово": {}}}"#).expect("a file");
    let (resources, output, ledger) = (tree.join("resources"), folder.join("read.json"), folder.join("ledger"));
    let result = devchat(&DevchatRequest {
        root: &tree,
        resource_dir: &resources,
        split: "dev",
        directory: &directory,
        candidate: &candidate,
        released: &candidate,
        baseline: &baseline,
        output: &output,
        ledger: &ledger,
        sequences_ledger: &folder.join("sequences"),
        workers: 1,
    })
    .expect("a result");
    assert_eq!(result["floors"]["every_message_typed"], json!(false), "{result}");
    assert!(result.get("error").is_none());
    let table = &result["results"];
    for mode in ["correct", "forget_ru", "forget_en", "start_wrong"] {
        assert!(table["candidate"][mode]["errors"].as_i64() > Some(0), "{mode}: {table}");
        assert_eq!(table["candidate"][mode]["ok"].as_i64(), Some(0), "{mode}: {table}");
        assert_eq!(table["released"][mode]["errors"].as_i64(), Some(0), "{mode}: {table}");
    }
    let recorded: Value = serde_json::from_slice(&std::fs::read(&output).expect("the output")).expect("json");
    assert_eq!(recorded["floors"], result["floors"]);
}

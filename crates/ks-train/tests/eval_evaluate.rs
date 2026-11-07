//! The sequences of a development replay, over rows written out by hand: the windows trimmed, the keys typed, the
//! characters no layout has, and what a replay of the installed pair reports of them.
mod eval_support;
mod support;

use ks_train::corpus::CorpusRow;
use ks_train::evaluate::{select_rows, sequence_plan};
use serde_json::{Value, json};
use std::path::Path;
use std::process::Output;
use support::{row, scratch, trainer, tree, write_split};

fn corpus_row(identifier: &str, original: &str, group: Option<i64>, before: &str, after: &str) -> CorpusRow {
    CorpusRow {
        identifier: identifier.to_owned(),
        original: original.to_owned(),
        group,
        before: before.to_owned(),
        after: after.to_owned(),
        family: format!("family-{identifier}"),
        document: format!("document-{identifier}"),
        spacing: " ".to_owned(),
        literal_tail: String::new(),
        layout_representable: true,
        split: "development".to_owned(),
        quarantine_reasons: Vec::new(),
        lemma: original.to_owned(),
        language: "x".to_owned(),
        source: "test".to_owned(),
        source_file: "file".to_owned(),
        source_sentence: "0".to_owned(),
        source_token: "0".to_owned(),
        space_before: " ".to_owned(),
        upos: "NOUN".to_owned(),
        features: String::new(),
        misc: String::new(),
        alignment: "exact".to_owned(),
    }
}

/// A window of the corpus's full width before the focus (96 characters) and after it (64).
fn full_before(start: &str) -> String {
    let text = format!("{start}{}", " yy".repeat(40));
    text.chars().take(96).collect()
}

fn full_after(end: &str) -> String {
    let text = format!("{}{end}", "zz ".repeat(30));
    let characters: Vec<char> = text.chars().collect();
    characters[characters.len() - 64..].iter().collect()
}

#[test]
fn a_window_clipped_inside_a_word_loses_only_that_fragment() {
    let plan = sequence_plan(&corpus_row("clipped", "hello", Some(0), &full_before("xxx"), &full_after("zzz")), false);
    assert_eq!((plan.trimmed_left, plan.trimmed_right), (3, 3));
    assert!(plan.expected.starts_with(" yy") && plan.expected.ends_with("zz "), "{}", plan.expected);
    // A full window that starts or ends at a space keeps everything; one that is a single word loses all of it.
    let plan = sequence_plan(&corpus_row("spaced", "hello", Some(0), &full_before(" "), &full_after(" ")), false);
    assert_eq!((plan.trimmed_left, plan.trimmed_right), (0, 0));
    let plan = sequence_plan(&corpus_row("one-word", "hello", Some(0), &"w".repeat(96), &"z".repeat(64)), false);
    assert_eq!((plan.trimmed_left, plan.trimmed_right, plan.expected.as_str()), (96, 64, "hello"));
}

#[test]
fn the_keys_of_a_sequence_follow_the_letters_and_the_declared_layout() {
    // A wrong intervention on the first letter of the focus, typed in the other layout.
    let plan = sequence_plan(&corpus_row("wrong", "Hello", Some(0), "say ", ""), true);
    assert_eq!(plan.intervention_event, Some(5));
    assert_eq!(plan.keys[4].explicit_group, Some(1));
    assert!(plan.keys[4].shift);
    // A focus whose letters start after a digit; a Russian focus with ё in both cases.
    let plan = sequence_plan(&corpus_row("digit", "2nd", Some(0), "", ""), true);
    assert_eq!((plan.intervention_event, plan.keys[0].explicit_group, plan.keys[1].explicit_group), (Some(2), Some(0), Some(1)));
    let plan = sequence_plan(&corpus_row("yo", "Ёлка", Some(1), "ёж ", ""), false);
    assert_eq!((plan.expected_final_group, plan.unsupported.len()), (1, 0));
    // A character only the other layout has turns the layout over; one neither has is listed and not typed.
    let plan = sequence_plan(&corpus_row("number", "hello", Some(0), "№ ", " €"), false);
    assert_eq!(plan.keys[0].explicit_group, Some(1));
    assert_eq!(plan.unsupported, ["U+20AC"]);
    assert_eq!(plan.keys.len(), "№ hello ".chars().count());
    // A mixed focus has no layout of its own; a focus without letters takes its row's.
    let plan = sequence_plan(&corpus_row("mixed", "gitовый", None, "", ""), false);
    assert_eq!((plan.group, plan.intervention_event, plan.expected_final_group), (None, None, 1));
    let plan = sequence_plan(&corpus_row("digits", "123", Some(1), "", ""), false);
    assert_eq!(plan.keys[0].explicit_group, Some(1));
}

#[test]
fn one_focus_is_selected_per_document_and_group() {
    let rows = [
        corpus_row("a", "hello", Some(0), "", ""),
        CorpusRow { document: "document-a".to_owned(), ..corpus_row("b", "world", Some(0), "", "") },
        corpus_row("c", "привет", Some(1), "", ""),
        corpus_row("d", "gitовый", None, "", ""),
    ];
    let selected: Vec<&str> = select_rows(&rows).iter().map(|row| row.identifier.as_str()).collect();
    assert_eq!(selected.len(), 3, "{selected:?}");
    assert!(selected.contains(&"c") && selected.contains(&"d"));
}

fn development_row(index: usize, original: &str, group: Option<i64>, before: &str, after: &str) -> Value {
    let mut value = row("development", index, group.unwrap_or(1), &[original], 0);
    value["group"] = json!(group);
    value["before"] = json!(before);
    value["after"] = json!(after);
    value["document"] = json!(format!("document{index}"));
    value
}

/// A development replay of the installed pair over these rows, in this tree.
fn development_replay(root: &Path, folder: &Path, rows: &[Value]) -> Output {
    let corpus = folder.join("corpus");
    std::fs::create_dir_all(&corpus).expect("a folder");
    let mut manifest = json!({"splits": {}});
    write_split(&corpus, "development", rows, &mut manifest);
    std::fs::write(corpus.join("manifest.json"), serde_json::to_vec(&manifest).expect("json")).expect("a manifest");
    let models = tree().join("resources/models");
    trainer()
        .arg("evaluate")
        .arg("--tree")
        .arg(root)
        .arg("--candidate")
        .arg(models.join("context_policy_v1.json"))
        .arg("--prefix-candidate")
        .arg(models.join("prefix_policy_v1.json"))
        .arg("--corpus")
        .arg(&corpus)
        .arg("--output")
        .arg(folder.join("report.json"))
        .arg("--ledger")
        .arg(folder.join("ledger"))
        .args(["--unsealed", "--split", "development", "--jobs", "2"])
        .output()
        .expect("the trainer starts")
}

#[test]
fn a_development_replay_of_the_installed_pair_reports_what_it_could_not_type() {
    let folder = scratch("evaluate-rows");
    let rows = [
        development_row(0, "hello", Some(0), "we say ", " to you"),
        development_row(1, "привет", Some(1), "мы скажем ", " вам"),
        development_row(2, "ёлка", Some(1), "", ""),
        development_row(3, "gitовый", None, "это ", " коммит"),
        development_row(4, "café", Some(0), "a ", ""),
        development_row(5, "world", Some(0), &full_before("xxx"), &full_after("zzz")),
    ];
    let answer = development_replay(&tree(), &folder, &rows);
    assert!(answer.status.success(), "{}", String::from_utf8_lossy(&answer.stderr));
    let report: Value = serde_json::from_slice(&std::fs::read(folder.join("report.json")).expect("a report")).expect("json");
    let selection = &report["selection"];
    assert_eq!(selection["unsupported"], json!([{"identifier": "development:4", "codepoints": ["U+00E9"]}]));
    assert_eq!(selection["replayed_documents"], json!({"0": 2, "1": 2, "None": 1}));
    assert_eq!(selection["trimmed_rows"], json!(1));
    let gates = &report["profiles"]["portable"]["gates"];
    assert_eq!((gates["enough_documents_us"].clone(), gates["enough_documents_ru"].clone()), (json!(false), json!(false)));
    assert!(report["profiles"]["portable"]["ablation"]["candidate_context_baseline_prefix"]["cases"].is_array());
    assert!(!folder.join("ledger").exists(), "a development replay claims no test");
}

#[test]
fn a_replay_without_an_orthotactic_model_still_runs() {
    let root = eval_support::linked("evaluate-no-ortho");
    std::fs::write(eval_support::own(&root, "resources/models/ortho_v1.json"), b"{}").expect("a file");
    let folder = scratch("evaluate-no-ortho-rows");
    let answer = development_replay(&root, &folder, &[development_row(0, "hello", Some(0), "we say ", "")]);
    assert!(answer.status.success(), "{}", String::from_utf8_lossy(&answer.stderr));
}

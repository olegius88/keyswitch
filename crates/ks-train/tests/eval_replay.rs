//! Keys typed through the real engine over the model editor, in this process: plans a replay refuses to finish, and the
//! span frames captured from rows written by hand.
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]
mod support;

use ks_context::context_model::ContextModel;
use ks_lang::Resources;
use ks_train::corpus::CorpusRow;
use ks_train::physical::key_of;
use ks_train::replay::{Mode, Plan, Replayer, prefix_indexes};
use ks_train::spans::build_span_curriculum;
use ks_train::tools::{Lexicons, Tools, reference_models};
use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};
use support::{scratch, tree};

struct Models {
    tools: Tools,
    portable: Lexicons,
}

static MODELS: LazyLock<Models> = LazyLock::new(|| {
    let resources: Arc<Resources> = Resources::new(tree().join("resources"));
    let portable = reference_models(&tree(), &resources, false).expect("the reference lexicons");
    let tools = Tools::new(&tree(), resources, portable.clone(), BTreeSet::new()).expect("the tools");
    Models { tools, portable }
});

fn installed() -> ContextModel {
    let models = tree().join("resources/models");
    ContextModel::load_artifact(&models.join("context_policy_v1.json"), &models).expect("the installed model")
}

fn keys(text: &str, group: usize) -> Vec<ks_train::physical::PhysicalKey> {
    text.chars().map(|character| key_of(character, group).expect("a key").clone()).collect()
}

#[test]
fn a_replay_refuses_a_plan_it_cannot_type_as_asked() {
    let models = &*MODELS;
    let mut replayer = Replayer::new(&models.tools, &models.portable, (None, "unavailable".to_owned()), scratch("replay-plans"), installed(), None, prefix_indexes(&models.portable));
    // Default settings type one key per character.
    let mut extra = keys("hello ", 0);
    extra.push(extra[0].clone());
    let outcome = replayer.replay(&Plan::typed("extra".to_owned(), "document".to_owned(), 0, "hello ".to_owned(), extra, 0), Mode::Default);
    assert!(outcome.error.as_deref().is_some_and(|error| error.contains("one key per expected character")), "{outcome:?}");
    assert!(!outcome.exact);
    // A wrong intervention on a key that types the same in both layouts changes nothing.
    let mut plan = Plan::typed("same".to_owned(), "document".to_owned(), 0, "1ab ".to_owned(), keys("1ab ", 0), 0);
    plan.initially_wrong = true;
    plan.intervention_event = Some(1);
    let outcome = replayer.replay(&plan, Mode::EarlyOff);
    assert!(outcome.error.as_deref().is_some_and(|error| error.contains("did not change the focus letter")), "{outcome:?}");
    assert_eq!(outcome.value()["mode"], "early_off");
    // A plan typed as asked.
    let outcome = replayer.replay(&Plan::typed("plain".to_owned(), "document".to_owned(), 0, "hello world".to_owned(), keys("hello world", 0), 0), Mode::Default);
    assert!(outcome.error.is_none() && outcome.exact, "{outcome:?}");
}

fn train_row(identifier: &str, original: &str, group: Option<i64>, family: &str) -> CorpusRow {
    CorpusRow {
        identifier: identifier.to_owned(),
        original: original.to_owned(),
        group,
        before: String::new(),
        after: String::new(),
        family: family.to_owned(),
        document: format!("document-{identifier}"),
        spacing: " ".to_owned(),
        literal_tail: String::new(),
        layout_representable: true,
        split: "train".to_owned(),
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

#[test]
fn span_frames_are_captured_for_one_family_of_each_layout_with_an_anchor() {
    let models = &*MODELS;
    let mut replayer = Replayer::recording(&models.tools, &models.portable, (None, "unavailable".to_owned()), scratch("replay-spans"));
    let mut rows = vec![
        train_row("a", "hello", Some(0), "f-a"),
        train_row("b", "world", Some(0), "f-b"),
        train_row("c", "привет", Some(1), "f-c"),
        train_row("d", "дом", Some(1), "f-d"),
        train_row("e", "e-mail", Some(0), "f-e"),
        train_row("f", "ab", Some(0), "f-f"),
        // What no span is made of: no layout, an unknown one, a row no layout can type, no letters, too short or too long.
        train_row("g", "hello", None, "f-g"),
        train_row("h", "hello", Some(2), "f-h"),
        train_row("i", "", Some(0), "f-i"),
        train_row("j", &"x".repeat(65), Some(0), "f-j"),
        train_row("k", "123", Some(0), "f-k"),
        train_row("l", "café", Some(0), "f-l"),
    ];
    rows.push(CorpusRow { layout_representable: false, ..train_row("m", "hello", Some(0), "f-m") });
    let built = build_span_curriculum(&mut replayer, &rows, 2, "train");
    let report = built.report();
    assert_eq!(report["selected_families"], 2, "{report}");
    assert!(!built.frames.is_empty());
    // A layout whose rows are none of them an anchor gives no family of its own.
    let without = [train_row("a", "hello", Some(0), "f-a"), train_row("n", "ок", Some(1), "f-n"), train_row("o", "кот-пёс", Some(1), "f-o")];
    let built = build_span_curriculum(&mut replayer, &without, 4, "train");
    assert_eq!(built.report()["selected_group_1"], serde_json::Value::Null, "{}", built.report());
}

#[test]
fn a_focus_of_joined_parts_is_asked_about_in_parts() {
    let models = &*MODELS;
    let mut replayer = Replayer::recording(&models.tools, &models.portable, (None, "unavailable".to_owned()), scratch("replay-joined"));
    let rows = [
        train_row("a", "hello", Some(0), "f-a"),
        train_row("b", "e-mail", Some(0), "f-b"),
        train_row("c", "don't", Some(0), "f-c"),
        train_row("d", "привет", Some(1), "f-d"),
        train_row("e", "кое-что", Some(1), "f-e"),
        train_row("f", "вот,так", Some(1), "f-f"),
    ];
    let built = build_span_curriculum(&mut replayer, &rows, 8, "train");
    assert_eq!(built.report()["selected_families"], 6, "{}", built.report());
}

//! The sequences of a development replay, over rows written out by hand: the windows trimmed, the keys typed, the
//! characters no layout has, and what a replay of the installed pair reports of them.
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]
mod eval_support;
mod support;

use ks_train::corpus::CorpusRow;
use ks_train::evaluate::{first_plan, installed_reference, installed_unchanged_since, opening_letters, select_rows, sequence_plan, sequence_plans};
use ks_train::sequence_report::{Documents, InstalledReference, Report, Unsupported};
use serde_json::{Value, json};
use std::path::Path;
use std::process::Output;
use support::{digest, row, scratch, trainer, tree, write_split};

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

/// Protocol 5: the focus typed first, in an editor that opens in the layout its first letter is typed in.
#[test]
fn a_focus_typed_first_opens_the_editor_in_the_layout_of_its_first_letter() {
    let row = corpus_row("first", "Hello", Some(0), "we say ", ", мир");
    let wrong = first_plan(&row, true).expect("a word that opens with a letter of its layout");
    assert_eq!((wrong.expected.as_str(), wrong.initial_group, wrong.intervention_event, wrong.focus_letters, wrong.trimmed_left), ("Hello, мир", 1, Some(1), Some(5), 0));
    // No key of the word selects a layout; the first Russian letter after it does.
    assert!(wrong.keys[..7].iter().all(|key| key.explicit_group.is_none()), "{:?}", wrong.keys);
    assert_eq!((wrong.keys[7].explicit_group, wrong.expected_final_group), (Some(1), 1));
    let right = first_plan(&row, false).expect("the correct plan");
    assert_eq!((right.initial_group, right.intervention_event, right.keys.len()), (0, None, wrong.keys.len()));
    assert!(!right.initially_wrong && wrong.initially_wrong);
    // A Russian word opens the editor in English for its wrong plan; the window after it is trimmed as protocol 4 trims it.
    let russian = first_plan(&corpus_row("ru", "ёлка", Some(1), &full_before("xxx"), &full_after("zzz")), true).expect("a Russian word");
    assert_eq!((russian.initial_group, russian.trimmed_left, russian.trimmed_right), (0, 0, 3));
    assert!(russian.expected.starts_with("ёлка") && russian.expected.ends_with("zz "), "{}", russian.expected);
    // A glyph no layout has is listed, as in protocol 4.
    assert_eq!(first_plan(&corpus_row("euro", "hello", Some(0), "", " €"), false).expect("a plan").unsupported, ["U+20AC"]);
    // No first plans: a focus that opens with a digit, a sign or a letter of the other layout, and a focus of no layout.
    for (original, group) in [("2nd", Some(0)), ("@ghbdtn", Some(0)), ("Hello", Some(1)), ("gitовый", None)] {
        assert!(first_plan(&corpus_row("none", original, group, "", ""), true).is_none(), "{original}");
    }
    assert_eq!((opening_letters("don't"), opening_letters("2nd"), opening_letters("слово")), (3, 0, 5));
}

#[test]
fn every_row_protocol_4_types_has_first_plans_or_is_named_as_without_them() {
    let rows = [
        corpus_row("a", "hello", Some(0), "say ", ""),
        corpus_row("b", "2nd", Some(0), "the ", ""),
        corpus_row("c", "привет", Some(1), "", " €"),
        corpus_row("d", "gitовый", None, "", ""),
        corpus_row("e", "мир", Some(1), "", ""),
    ];
    let selected: Vec<&CorpusRow> = rows.iter().collect();
    let plans = sequence_plans(&selected);
    assert_eq!(plans.sequence.len(), 9);
    // A row protocol 4 cannot type has no first plans and is not named: it is in the selection's unsupported list.
    let first: Vec<(&str, bool)> = plans.first.iter().map(|plan| (plan.identifier.as_str(), plan.initially_wrong)).collect();
    assert_eq!(first, [("a", false), ("a", true), ("e", false), ("e", true)]);
    assert_eq!(plans.first_ineligible, ["b"]);
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

/// A development replay of the installed pair over these rows, in this tree, with more options; with another prefix
/// candidate when one is named.
fn development_replay(root: &Path, folder: &Path, rows: &[Value], prefix: Option<&Path>, extra: &[&str]) -> Output {
    let installed = tree().join("resources/models/prefix_policy_v1.json");
    let prefix = prefix.unwrap_or(installed.as_path());
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
        .arg(prefix)
        .arg("--corpus")
        .arg(&corpus)
        .arg("--output")
        .arg(folder.join("report.json"))
        .arg("--ledger")
        .arg(folder.join("ledger"))
        .args(["--unsealed", "--split", "development", "--jobs", "2"])
        .args(extra)
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
        development_row(6, "2nd", Some(0), "the ", " one"),
    ];
    let answer = development_replay(&tree(), &folder, &rows, None, &[]);
    assert!(answer.status.success(), "{}", String::from_utf8_lossy(&answer.stderr));
    let report: Report = ks_text::json::loads(std::fs::read(folder.join("report.json")).expect("a report")).expect("a report of the shape the evaluation writes");
    let (selection, profiles) = (report.selection.as_ref().expect("a selection"), report.profiles.as_ref().expect("profiles"));
    assert_eq!(selection.unsupported, [Unsupported { identifier: "development:4".to_owned(), codepoints: vec!["U+00E9".to_owned()] }]);
    assert_eq!(selection.replayed_documents, Documents { us: 3, ru: 2, none: 1 });
    assert_eq!(selection.trimmed_rows, 1);
    let gates = &profiles.portable.gates;
    assert_eq!((gates.enough_documents_us, gates.enough_documents_ru), (false, false));
    assert!(profiles.portable.ablation.as_ref().is_some_and(|pairs| pairs.contains_key("candidate_context_baseline_prefix")));
    assert!(!folder.join("ledger").exists(), "a development replay claims no test");
    // Protocol 5: the words that open with a letter of their layout typed first, by every pair of the default mode.
    assert_eq!(report.protocol_version(), Some(5));
    assert_eq!(selection.first_ineligible, ["development:6"]);
    for (name, profile) in profiles.each() {
        let first = profile.first.as_ref().expect("the first plans are of the default mode");
        for pair in ["baseline", "candidate", "candidate_context_baseline_prefix", "installed"] {
            let counts = first.counts.named(pair).expect("a pair");
            assert_eq!((counts.initially_wrong, counts.initially_correct), (4, 4), "{name} {pair}");
            let early = first.earliness.named(pair).expect("a pair");
            assert_eq!(early.inside_word + early.last_letter + early.boundary_key + early.later + early.never, 4, "{name} {pair}: {early:?}");
        }
    }
    // The candidate here is the installed pair: the reference replays the same models, and the report names them.
    let profile = &profiles.portable;
    let first = profile.first.as_ref().expect("the first plans");
    assert_eq!(first.counts.candidate, first.counts.installed);
    assert_eq!(first.earliness.candidate, first.earliness.installed);
    assert_eq!(profile.installed.as_ref().expect("the installed pair").counts, profile.counts.candidate);
    let models = tree().join("resources/models");
    let digest_of = |file: &str| digest(&std::fs::read(models.join(file)).expect("a model"));
    assert_eq!(report.installed_reference, Some(InstalledReference { context_sha256: digest_of("context_policy_v1.json"), prefix_sha256: digest_of("prefix_policy_v1.json") }));
    // A wrong first plan: its first key is the wrong letter, and every correction names the kind of plan that made it.
    let wanted = "development:1";
    let wrong = first.cases.candidate.iter().find(|case| case.identifier == wanted && case.initially_wrong).expect("a wrong first plan");
    let intervention = wrong.intervention.as_ref().expect("an intervention");
    assert_eq!((intervention.event, intervention.observed.as_str(), wrong.focus_letters), (1, "g", Some(6)));
    let kinds: Vec<&str> = first.cases.candidate.iter().chain(&profile.cases.candidate).flat_map(|case| &case.corrections).map(|item| item.kind.as_deref().expect("a kind")).collect();
    assert!(!kinds.is_empty() && kinds.iter().all(|kind| !kind.is_empty()), "{kinds:?}");
}

/// The installed pair a replay compares with is named by the digests of its two models, and one changed while the replay
/// runs stops its report.
#[test]
fn an_installed_pair_changed_during_a_replay_stops_its_report() {
    let root = scratch("installed-reference");
    let models = root.join("resources/models");
    std::fs::create_dir_all(&models).expect("a folder");
    for name in ["context_policy_v1.json", "prefix_policy_v1.json"] {
        std::fs::write(models.join(name), name).expect("a model");
    }
    let reference = installed_reference(&root).expect("the installed pair");
    assert_eq!(reference, InstalledReference { context_sha256: digest(b"context_policy_v1.json"), prefix_sha256: digest(b"prefix_policy_v1.json") });
    installed_unchanged_since(&root, &reference).expect("nothing changed yet");
    for name in ["prefix_policy_v1.json", "context_policy_v1.json"] {
        std::fs::write(models.join(name), b"another").expect("a model");
        assert!(installed_unchanged_since(&root, &reference).unwrap_err().contains("the installed pair changed during sequence replay"));
        std::fs::write(models.join(name), name).expect("a model");
    }
    std::fs::remove_file(models.join("prefix_policy_v1.json")).expect("the model goes");
    assert!(installed_unchanged_since(&root, &reference).is_err());
}

/// A prefix candidate of schema four is replayed with the term lists its engines read, built once for each profile.
#[test]
fn a_prefix_candidate_of_schema_four_is_replayed_with_its_term_lists() {
    let folder = scratch("evaluate-schema-four");
    let weights = r#"{"bias":[0,40,0,0]}"#;
    let sum = digest(weights.as_bytes());
    let version = format!("prefix-v4-{}", sum.get(..12).expect("a digest"));
    let model = folder.join("eager.json");
    let text = format!(
        r#"{{"actions":["keep","convert","wait","suggest"],"conversion_threshold":0.985,"feature_version":4,"kind":"keyswitch.prefix-policy","prefix_feature_version":4,"version":"{version}","weights":{weights},"weights_sha256":"{sum}"}}"#
    );
    std::fs::write(&model, text).expect("a model");
    let answer = development_replay(&tree(), &folder, &[development_row(0, "garden", Some(0), "", "")], Some(&model), &[]);
    assert!(answer.status.success(), "{}", String::from_utf8_lossy(&answer.stderr));
    let report: Report = ks_text::json::loads(std::fs::read(folder.join("report.json")).expect("a report")).expect("a report of the shape the evaluation writes");
    // A model that converts every prefix it is asked about switches the word typed as meant away at its fourth key.
    let first = report.profiles.as_ref().expect("profiles").portable.first.as_ref().expect("the first plans");
    assert_eq!(first.earliness.candidate.correct_first_plans.false_early, 1);
}

#[test]
fn a_replay_without_an_orthotactic_model_still_runs() {
    let root = eval_support::linked("evaluate-no-ortho");
    std::fs::write(eval_support::own(&root, "resources/models/ortho_v1.json"), b"{}").expect("a file");
    let folder = scratch("evaluate-no-ortho-rows");
    let answer = development_replay(&root, &folder, &[development_row(0, "hello", Some(0), "we say ", "")], None, &[]);
    assert!(answer.status.success(), "{}", String::from_utf8_lossy(&answer.stderr));
}

/// A replay kept in a cache is read back instead of being typed again, and the report is the one written without a
/// cache; a sealed test refuses a cache.
#[test]
fn a_development_replay_read_back_from_its_cache_writes_the_same_report() {
    let folder = scratch("evaluate-cache");
    let rows = [development_row(0, "hello", Some(0), "we say ", " to you"), development_row(1, "привет", Some(1), "мы скажем ", " вам")];
    let plain = development_replay(&tree(), &folder, &rows, None, &[]);
    assert!(plain.status.success(), "{}", String::from_utf8_lossy(&plain.stderr));
    assert!(!String::from_utf8_lossy(&plain.stdout).contains("replay cache"));
    let without = std::fs::read(folder.join("report.json")).expect("a report");
    let cache = folder.join("cache").display().to_string();
    // The candidate here is the installed pair: a replay of one may be read back for the other already in the first run.
    for read_back in [" of 20 replays read back", "replay cache: 20 of 20 replays read back"] {
        let answer = development_replay(&tree(), &folder, &rows, None, &["--cache", &cache]);
        let said = String::from_utf8_lossy(&answer.stdout).into_owned();
        assert!(answer.status.success() && said.contains(read_back), "{said}{}", String::from_utf8_lossy(&answer.stderr));
        assert_eq!(std::fs::read(folder.join("report.json")).expect("a report"), without);
    }
    let models = tree().join("resources/models");
    let sealed = trainer()
        .arg("evaluate")
        .arg("--tree")
        .arg(tree())
        .arg("--candidate")
        .arg(models.join("context_policy_v1.json"))
        .arg("--prefix-candidate")
        .arg(models.join("prefix_policy_v1.json"))
        .arg("--corpus")
        .arg(folder.join("corpus"))
        .arg("--output")
        .arg(folder.join("test.json"))
        .arg("--ledger")
        .arg(folder.join("ledger"))
        .args(["--split", "test", "--cache", &cache])
        .output()
        .expect("the trainer starts");
    assert!(!sealed.status.success() && String::from_utf8_lossy(&sealed.stderr).contains("without a cache"), "{}", String::from_utf8_lossy(&sealed.stderr));
    assert!(!folder.join("ledger").exists());
}

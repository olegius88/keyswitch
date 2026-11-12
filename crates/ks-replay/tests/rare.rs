//! What only a rare order of events brings: each guard of the engine asked directly, with the state it guards against.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;

use ks_backend::constants::{ALT_MASK, CONTROL_MASK, SHIFT_MASK, SUPER_MASK};
use ks_backend::{FieldContext, FieldReader, FieldRole, KeyEvent};
use ks_config::config::SettingsStore;
use ks_config::selection_commands::SelectionCommand;
use ks_engine::engine::{ConvertOutcome, CorrectionPlan, EngineCore, FieldWait, Hotkey, HotkeyProblem, InsertionPoint, KeptWord, PlanRef, QueueItem, Shared, Stroke, Strokes, WordEvaluation, WordOptions};
use ks_lang::detector::DetectionDecision;
use ks_lang::intent_model::CorrectionTrigger;
use ks_lang::early_switch::EarlySwitchDecision;
use ks_replay::harness::{Bundle, KEYS, PAUSE_KEYCODE, Session};
use serde_json::json;

fn key(character: char) -> Stroke {
    let (physical, group) = KEYS
        .iter()
        .find_map(|key| Some((key, key.characters.iter().position(|typed| *typed == character)?)))
        .unwrap_or_else(|| panic!("no key types {character}"));
    Arc::new(KeyEvent {
        pressed: true,
        keycode: physical.keycode,
        key_name: character.to_string(),
        character: character.to_string(),
        characters: physical.strings(),
        group: i32::try_from(group).expect("two layouts"),
        state: if physical.shift { SHIFT_MASK } else { 0 },
        ..KeyEvent::default()
    })
}

fn keys(text: &str) -> Strokes {
    text.chars().map(key).collect()
}

fn named(name: &str) -> KeyEvent {
    KeyEvent { pressed: true, key_name: name.to_owned(), characters: vec![String::new(), String::new()], ..KeyEvent::default() }
}

fn plan(word: &str, replacement: &str, boundary: Option<char>) -> PlanRef {
    let group = i64::from(key(word.chars().next().expect("a word")).group);
    Arc::new(CorrectionPlan::new(keys(word), boundary.map(key), group, 1 - group, word.to_owned(), replacement.to_owned(), 0.9, "firefox"))
}

/// A field that shows just what the test put there.
struct Fixed(Mutex<Option<FieldContext>>);

impl Fixed {
    fn new(before: &str, after: &str) -> Arc<Fixed> {
        Arc::new(Fixed(Mutex::new(Some(field(before, after)))))
    }

    fn show(&self, field: Option<FieldContext>) {
        *self.0.lock().expect("not poisoned") = field;
    }
}

fn field(before: &str, after: &str) -> FieldContext {
    let mut field = FieldContext::new("firefox", "1");
    before.clone_into(&mut field.before);
    after.clone_into(&mut field.after);
    field.role = FieldRole::Text;
    field
}

impl FieldReader for Fixed {
    fn read(&self, _application: &str, _window: i64) -> Option<FieldContext> {
        self.0.lock().expect("not poisoned").clone()
    }

    fn status(&self) -> String {
        "available".to_owned()
    }
}

fn session(bundle: &mut Bundle) -> Session {
    let mut session = bundle.session("firefox", 0, Vec::new());
    session.recovers = true;
    session.core.settings.set("diagnostics.technical_logging", json!(true), false).expect("in memory");
    session
}

fn set(core: &EngineCore, path: &str, value: serde_json::Value) {
    core.settings.set(path, value, false).expect("in memory");
}

fn queue(core: &EngineCore, event: KeyEvent) {
    core.shared.put_nowait(QueueItem::Key(Arc::new(event)));
}

#[test]
fn what_stands_after_the_word_is_read_up_to_the_first_key_that_is_no_text() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    let letter = |character| (*key(character)).clone();
    let stoppers = [
        KeyEvent { state: CONTROL_MASK, ..letter('a') },
        KeyEvent { state: ALT_MASK, ..letter('a') },
        KeyEvent { state: SUPER_MASK, ..letter('a') },
        KeyEvent { character: String::new(), ..letter('a') },
        KeyEvent { character: "\u{7}".to_owned(), ..letter('a') },
    ];
    let word = keys("ab");
    for stopper in &stoppers {
        core.strokes = word.clone();
        core.strokes.push(Arc::new(KeyEvent { pressed: false, ..letter('c') }));
        core.strokes.push(Arc::new(named("Shift_L")));
        core.strokes.push(key('d'));
        core.strokes.push(Arc::new(stopper.clone()));
        core.strokes.push(key('e'));
        assert_eq!(core.typed_ahead(&word), "d");
    }
    core.strokes = word.clone();
    for stopper in &stoppers {
        queue(core, KeyEvent { pressed: false, ..letter('c') });
        queue(core, named("Shift_L"));
        queue(core, letter('d'));
        queue(core, stopper.clone());
        queue(core, letter('e'));
        assert_eq!(core.typed_ahead(&word), "d");
        while !core.shared.queued().is_empty() {
            core.shared.get_nowait();
        }
    }
    core.shared.put_nowait(QueueItem::Stop);
    assert_eq!(core.typed_ahead(&word), "");
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn a_correction_is_given_up_when_its_keys_cannot_be_typed_again() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    session.backend.set_text("ghbdtn".chars().collect());
    session.backend.set_caret(6);
    let core = &mut session.core;
    core.shared.input_overflow.store(true, Ordering::Relaxed);
    assert!(!core.execute_correction(plan("ghbdtn", "привет", Some(' ')), None));
    core.shared.input_overflow.store(false, Ordering::Relaxed);
    let mut strange = (*plan("ghbdtn", "привет", Some(' '))).clone();
    strange.strokes.push(Arc::new(KeyEvent { characters: vec!["ab".to_owned(), "\u{1f600}".to_owned()], ..(*key('a')).clone() }));
    assert!(!core.execute_correction(Arc::new(strange), None));
    assert_eq!(session.backend.text().iter().collect::<String>(), "ghbdtn");
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn a_correction_waits_only_for_a_field_that_is_behind() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    session.backend.set_text("ghbdtn ".chars().collect());
    session.backend.set_caret(7);
    let reader = Fixed::new("ghbd", "");
    session.core.context_policy.reader = Some(reader.clone());
    let core = &mut session.core;
    let of_field = |word: &str, boundary| {
        let mut plan = (*plan(word, "привет", boundary)).clone();
        plan.context_field = "1".to_owned();
        Arc::new(plan)
    };
    // Each value of the setting, a value never offered among them, and a field showing less than the word.
    for value in ["off", "100", "300", "nonsense"] {
        set(core, "detection.field_catch_up", json!(value));
        core.field_wait = None;
        let done = core.execute_correction(of_field("ghbdtn", Some(' ')), None);
        assert!(!done);
        assert_eq!(core.field_wait.is_some(), value != "off", "{value}");
    }
    // The word is all there and only its boundary is missing; with the boundary after the caret, waiting mends nothing.
    set(core, "detection.field_catch_up", json!("300"));
    for (before, after, waits) in [("ghbdtn", "", true), ("ghbdtn", " ", false), ("ghbdtnx", "", false)] {
        reader.show(Some(field(before, after)));
        core.field_wait = None;
        core.execute_correction(of_field("ghbdtn", Some(' ')), None);
        assert_eq!(core.field_wait.is_some(), waits, "{before}|{after}");
    }
    // Nothing can be read: the correction is given up, the letters typed since left alone.
    reader.show(None);
    core.untracked_token = true;
    core.strokes = keys("ab");
    assert!(!core.execute_correction(of_field("ghbdtn", Some(' ')), None));
    assert!(core.untracked_token);
    // An early switch whose letters the field has not taken yet stays tracked, one the field never showed does not.
    let mut early = (*of_field("ghbdtn", None)).clone();
    early.mode = "early";
    let early = Arc::new(early);
    reader.show(Some(field("ghbd", "")));
    core.strokes = keys("ghbdtn");
    core.source_group = 0;
    assert!(!core.execute_correction(early.clone(), None));
    assert!(!core.strokes.is_empty());
    reader.show(Some(field("nothing like it", "")));
    assert!(!core.execute_correction(early, None));
    assert!(core.strokes.is_empty());
    // Retried only with no key down and nothing newer planned.
    let waiting = |core: &mut EngineCore| {
        core.field_wait = Some(FieldWait { plan: of_field("ghbdtn", Some(' ')), learning_prompt: None, deadline: f64::MAX, presses: core.presses_handled, rolled: 0 });
    };
    waiting(core);
    core.pressed.insert(50);
    core.retry_field_wait().expect("no failure");
    assert!(core.field_wait.is_some());
    core.pressed.clear();
    core.modifier_keycodes.insert(50);
    core.retry_field_wait().expect("no failure");
    assert!(core.field_wait.is_some());
    core.modifier_keycodes.clear();
    core.pending = Some(of_field("ghbdtn", Some(' ')));
    core.retry_field_wait().expect("no failure");
    assert!(core.field_wait.is_none());
    core.pending = None;
    waiting(core);
    core.clear_word(None, "");
    assert!(core.field_wait.is_none());
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn a_word_is_found_in_the_field_with_its_first_letter_in_any_case() {
    let around = |before: &str, after: &str, typed: &str, ahead: &str| EngineCore::_typed_in_field(&field(before, after), typed, ahead);
    assert_eq!(around("x ghbdtn", "", "ghbdtn", "").map(|found| found.0), Some("x ".to_owned()));
    // The editor made the first letter a capital: the keys typed are not what stands there.
    assert!(around("Ghbdtn", "", "ghbdtn", "").is_none());
    assert!(around("nothing", "", "ghbdtn", " a").is_none());
}

#[test]
fn an_early_plan_is_dropped_when_the_word_is_no_longer_what_it_was() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let early = {
        let mut early = (*plan("ghbdtn", "привет", None)).clone();
        early.mode = "early";
        Arc::new(early)
    };
    let core = &mut session.core;
    core.source_group = 0;
    core.strokes = keys("ghb");
    assert!(core.refresh_early_plan(&early).expect("no failure").is_none());
    core.strokes = keys("ghbdtx");
    assert!(core.refresh_early_plan(&early).expect("no failure").is_none());
    core.strokes = keys("ghbdtnz");
    core.source_group = 1;
    assert!(core.refresh_early_plan(&early).expect("no failure").is_none());
    core.source_group = 0;
    set(core, "detection.early_switch", json!(false));
    assert!(core.refresh_early_plan(&early).expect("no failure").is_none());
    set(core, "detection.early_switch", json!(true));
    set(core, "detection.early_switch_min_length", json!(50));
    assert!(core.refresh_early_plan(&early).expect("no failure").is_none());
    set(core, "detection.early_switch_min_length", json!(3));
    // A word the person excluded protects its prefix.
    core.strokes = keys("ghbdtnz");
    set(core, "exclusions.words", json!(["ghbdtnz"]));
    assert!(core.refresh_early_plan(&early).expect("no failure").is_none());
    set(core, "exclusions.words", json!([]));
    // Not letters in the other layout: `бит` typed in the second layout is `,bn` in the first.
    let sign = {
        let mut early = (*plan("бит", ",bn", None)).clone();
        early.mode = "early";
        Arc::new(early)
    };
    core.source_group = 1;
    core.strokes = keys("бит");
    assert!(core.refresh_early_plan(&sign).expect("no failure").is_none());
    core.source_group = 0;
    // A password field protects the prefix.
    let secret = Fixed::new("ghbdtn", "");
    let mut shown = field("ghbdtnz", "");
    shown.sensitive = true;
    secret.show(Some(shown));
    core.context_policy.reader = Some(secret);
    set(core, "detection.context_read_field", json!(true));
    core.strokes = keys("ghbdtnz");
    assert!(core.refresh_early_plan(&early).expect("no failure").is_none());
    // Asked again, the model no longer switches a word that reads as itself.
    core.context_policy.reader = None;
    let kept = {
        let mut early = (*plan("hello", "руддщ", None)).clone();
        early.mode = "early";
        Arc::new(early)
    };
    core.strokes = keys("hello");
    let _ = core.refresh_early_plan(&kept).expect("no failure");
    // The word changed before the keys came up: dropped with a note, the pending plan with it.
    core.pending = Some(early.clone());
    core.pressed.clear();
    core.strokes = keys("zz");
    let released = KeyEvent { pressed: false, ..(*key('z')).clone() };
    let _ = core.maybe_execute_pending(&released);
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn the_last_typo_correction_can_be_undone_only_while_nothing_came_after_it() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    let typo = {
        let mut typo = (*plan("ghbdtn", "привет", Some(' '))).clone();
        typo.mode = "typo";
        Arc::new(typo)
    };
    core.last_correction = Some(typo.clone());
    core.last_committed = Some(typo.clone());
    core.last_correction_time = core.monotonic();
    assert!(core.typo_correction_reversible());
    core.strokes = keys("a");
    assert!(!core.typo_correction_reversible());
    core.strokes.clear();
    core.symbol_strokes = keys(",");
    assert!(!core.typo_correction_reversible());
    core.symbol_strokes.clear();
    core.last_committed_stale = true;
    assert!(!core.typo_correction_reversible());
    core.last_committed_stale = false;
    for change in 0..3 {
        let mut committed = (*typo).clone();
        match change {
            0 => committed.strokes = keys("a"),
            1 => committed.trailing = keys(","),
            _ => committed.boundary = None,
        }
        core.last_committed = Some(Arc::new(committed));
        assert!(!core.typo_correction_reversible());
    }
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn a_prefix_the_model_was_not_trained_on_is_not_asked_about() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    let shown = field("ghbdtn", "");
    let baseline = |original: &str, replacement: &str, source, target| EarlySwitchDecision {
        should_switch: true,
        source_group: source,
        target_group: target,
        original: original.to_owned(),
        replacement: replacement.to_owned(),
        reason: "scheduled_prefix".to_owned(),
        source_evidence: None,
        target_evidence: None,
    };
    let unsupported = [
        baseline("ghb1dtn", "при1вет", 0, 1),
        baseline("gHbdtn", "привет", 0, 1),
        baseline("ghbdtn", "пРивет", 0, 1),
        baseline("ghbdtn", "привет", 2, 1),
        baseline("ghbdtn", "привет", 0, 2),
        baseline("ghbdtn", "привет", 0, 0),
        baseline("gh", "пр", 0, 1),
    ];
    for decision in unsupported {
        let (asked, _) = core.decide_prefix(decision, Some(&shown)).expect("no failure");
        assert!(!asked.reason.is_empty());
    }
    let models = std::mem::take(&mut core.models);
    let (asked, _) = core.decide_prefix(baseline("ghbdtn", "привет", 1, 0), Some(&shown)).expect("no failure");
    assert_eq!(asked.reason, "prefix_input_unsupported");
    core.models = models.clone();
    core.models.truncate(1);
    let (asked, _) = core.decide_prefix(baseline("ghbdtn", "привет", 1, 0), Some(&shown)).expect("no failure");
    assert_eq!(asked.reason, "prefix_input_unsupported");
    core.models = models;
    let (asked, _) = core.decide_prefix(baseline("руддщ", "hello", 1, 0), None).expect("no failure");
    assert!(!asked.reason.is_empty());
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn the_early_switch_looks_only_at_letters_and_reads_the_field_it_was_given() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    // Nothing typed yet; a key that is a sign in every layout does not end a prefix that could be a word.
    core.strokes.clear();
    core.maybe_early_switch().expect("no failure");
    core.source_group = 0;
    core.strokes = keys("ghb1");
    core.maybe_early_switch().expect("no failure");
    // A field reader the setting asks for and nobody gave; a field without the word in it, in each way it can differ.
    set(core, "detection.context_read_field", json!(true));
    core.context_policy.reader = None;
    let (_, protection) = core.early_prefix_field("ghbdtn", "firefox");
    assert_eq!(protection, "");
    let reader = Fixed::new("ghbdtn", "");
    core.context_policy.reader = Some(reader.clone());
    for (selection, application, field_id, before) in [
        (true, "firefox", "1", "ghbdtn"),
        (false, "another", "1", "ghbdtn"),
        (false, "firefox", "", "ghbdtn"),
        (false, "firefox", "1", "\u{20}"),
        (true, "firefox", "1", "\u{20}"),
        (false, "another", "1", "\u{20}"),
    ] {
        let mut shown = field(before, "");
        shown.selection = selection;
        shown.field_id = field_id.to_owned();
        shown.application = application.to_owned();
        reader.show(Some(shown));
        let _ = core.early_prefix_field("ghbdtn", "firefox");
    }
    // A key late after the switch counts only if it is of the layout the word came from.
    core.early_switch_at = Some(core.monotonic());
    core.early_switch_origin = Some(0);
    core.source_group = 1;
    assert!(core.late_stroke_after_early_switch(&key('a')));
    core.source_group = 0;
    assert!(!core.late_stroke_after_early_switch(&key('a')));
    // A word of a third layout is no typo, and a switch finished in an excluded program says nothing of its text.
    set(core, "detection.correct_typos", json!(true));
    assert!(core.typo_plan(&Arc::new(CorrectionPlan { target_group: 2, ..(*plan("ghbdtn", "привет", Some(' '))).clone() }), &keys("ghbdtn")).expect("no failure").is_none());
    set(core, "exclusions.applications", json!(["firefox"]));
    core.early_switch_origin = Some(0);
    core.finish_early_switch(keys("ghbdtn"), None, "firefox", 1, Vec::new());
    set(core, "exclusions.applications", json!(["   ", "other"]));
    assert!(!core.application_excluded("firefox"));
    set(core, "exclusions.applications", json!(5));
    assert!(!core.application_excluded("firefox"));
    core.shared.set_sensitive_context_window(Some(99));
    assert!(!core.application_excluded("firefox"));
    set(core, "exclusions.applications", json!([]));
    core.shared.set_sensitive_context_window(None);
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn double_capitals_are_fixed_only_where_shift_was_held_too_long() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    let space = key(' ');
    let of = |word: &str| plan(word, "?", Some(' '));
    // The rule itself: the word it knows, typed in its own layout.
    let fixed = core.fix_double_capitals(&of("HEllo"), &space);
    assert_eq!(fixed.original, "Hello");
    core.pending = None;
    // A third layout, a word the keys do not spell, letters typed into a word, a word the speller knows.
    let third = Arc::new(CorrectionPlan { source_group: 2, ..(*of("HEllo")).clone() });
    assert_eq!(core.fix_double_capitals(&third, &space).original, "HEllo");
    let short = Arc::new(CorrectionPlan { strokes: keys("HE"), ..(*of("HEllo")).clone() });
    assert_eq!(core.fix_double_capitals(&short, &space).original, "HEllo");
    core.insertion = Some(InsertionPoint { head: "ab".to_owned(), tail: String::new() });
    assert_eq!(core.fix_double_capitals(&of("HEllo"), &space).original, "HEllo");
    core.insertion = None;
    for known in ["MHz", "GHz", "OSes", "HBase", "XEmacs", "IJssel", "OFris", "PCs"] {
        let _ = core.fix_double_capitals(&of(known), &space);
        core.pending = None;
    }
    // Keys that were not typed with Shift, or do not spell one letter: their unshifted keys are unknown.
    let unshifted = Arc::new(CorrectionPlan { strokes: of("HEllo").strokes.iter().map(|stroke| Arc::new(KeyEvent { state: 0, ..(**stroke).clone() })).collect(), ..(*of("HEllo")).clone() });
    assert_eq!(core.fix_double_capitals(&unshifted, &space).original, "HEllo");
    let doubled = {
        let mut strokes = of("HEllo").strokes.clone();
        strokes[1] = Arc::new(KeyEvent { characters: vec![String::new(), String::new()], character: String::new(), ..(*strokes[1]).clone() });
        Arc::new(CorrectionPlan { strokes, ..(*of("HEllo")).clone() })
    };
    assert_eq!(core.fix_double_capitals(&doubled, &space).original, "HEllo");
    set(core, "detection.layouts", json!(["de", "ru"]));
    assert_eq!(core.fix_double_capitals(&of("HEllo"), &space).original, "HEllo");
    assert_eq!(core.layout_name(5), "");
    assert!(core.layout_letters(0).is_empty());
    assert!(!core.word_begun("иг", 1, "bu", 0).expect("no failure"));
    set(core, "detection.layouts", json!("us"));
    assert_eq!(core.layout_name(1), "ru");
    assert_eq!(core.layout_name(-1), "");
    assert_eq!(core.layout_name(7), "");
    set(core, "detection.layouts", json!(["us", "ru"]));
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn settings_edited_by_hand_read_as_their_defaults() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    assert_eq!(core.text_setting("no.such.path", "fallback"), "fallback");
    set(core, "detection.context_policy", json!(5));
    assert_eq!(core.text_setting("detection.context_policy", "assist"), "5");
    set(core, "detection.context_policy", json!("assist"));
    assert!(core.words_setting("no.such.list").is_empty());
    set(core, "exclusions.words", json!("ghbdtn"));
    assert!(core.words_setting("exclusions.words").is_empty());
    set(core, "exclusions.words", json!(["ghbdtn", 5]));
    assert_eq!(core.words_setting("exclusions.words"), ["ghbdtn"]);
    set(core, "exclusions.words", json!([]));
    assert!((core.bounded_setting("no.such.number", 2.0, 1.0, 3.0) - 2.0).abs() < f64::EPSILON);
    // A hotkey that is no text, and none at all.
    let press = (*key('a')).clone();
    set(core, "hotkeys.toggle", json!(5));
    assert!(!core.matches_hotkey("toggle", &press));
    assert!(!core.matches_hotkey("no_such_hotkey", &press));
    assert!(!core.matches_hotkey("toggle", &KeyEvent { pressed: false, ..press.clone() }));
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn the_shape_of_a_key_says_what_it_can_be() {
    let empty = KeyEvent { character: String::new(), ..(*key('a')).clone() };
    assert!(!EngineCore::_layout_dependent(&empty));
    let one_empty = KeyEvent { characters: vec![String::new(), "a".to_owned()], ..(*key('a')).clone() };
    assert!(!EngineCore::_layout_dependent(&one_empty));
    assert!(EngineCore::_layout_dependent(&key('a')));
    let same = KeyEvent { characters: vec!["1".to_owned(), "1".to_owned()], character: "1".to_owned(), ..(*key('a')).clone() };
    assert!(!EngineCore::_layout_dependent(&same));
    assert!(!EngineCore::_joins_parts(&KeyEvent { characters: Vec::new(), ..(*key('a')).clone() }));
    let dash = KeyEvent { characters: vec!["-".to_owned(), "-".to_owned()], character: "-".to_owned(), ..(*key('a')).clone() };
    assert!(EngineCore::_joins_parts(&dash));
    assert!(!EngineCore::_joins_parts(&KeyEvent { character: "+".to_owned(), ..dash }));
    assert!(EngineCore::_is_boundary(&named("Return")));
    assert!(!EngineCore::_is_boundary(&named("F1")));
}

#[test]
fn a_keyboard_with_the_rule_window_open_or_a_key_that_does_not_count() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    // Signs typed before the rule window opened are let go with the word.
    session.core.symbol_strokes = keys(",");
    session.core.shared.set_rule_editor_open(true);
    session.tap((*key('a')).clone());
    assert!(session.core.symbol_strokes.is_empty());
    session.core.shared.set_rule_editor_open(false);
    // Pause and Break reset nothing by themselves once they are no hotkey.
    set(&session.core, "hotkeys.convert_last", json!(""));
    session.core.source_group = 0;
    session.core.strokes = keys("ab");
    session.tap(KeyEvent { keycode: PAUSE_KEYCODE, ..named("Break") });
    session.tap(KeyEvent { keycode: PAUSE_KEYCODE, ..named("Pause") });
    assert_eq!(session.core.strokes.len(), 2);
    session.tap(named("F7"));
    assert!(session.core.strokes.is_empty());
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn a_space_after_a_pending_sign_only_if_it_is_just_a_space() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    let space = (*key(' ')).clone();
    let base = (*plan("ghbdtn", "привет", Some(','))).clone();
    assert!(!core.space_follows_pending_sign(&space));
    core.pending = Some(Arc::new(base.clone()));
    assert!(core.space_follows_pending_sign(&space));
    assert!(!core.space_follows_pending_sign(&KeyEvent { pressed: false, ..space.clone() }));
    assert!(!core.space_follows_pending_sign(&KeyEvent { state: CONTROL_MASK, ..space.clone() }));
    assert!(!core.space_follows_pending_sign(&KeyEvent { state: ALT_MASK, ..space.clone() }));
    assert!(!core.space_follows_pending_sign(&KeyEvent { state: SUPER_MASK, ..space.clone() }));
    assert!(!core.space_follows_pending_sign(&key('a')));
    let other = [
        CorrectionPlan { mode: "pause", ..base.clone() },
        CorrectionPlan { automatic: false, ..base.clone() },
        CorrectionPlan { kept_tail: 1, ..base.clone() },
        CorrectionPlan { head: keys("@"), ..base.clone() },
        CorrectionPlan { boundary: Some(key(' ')), ..base.clone() },
        CorrectionPlan { boundary: Some(Arc::new(KeyEvent { deferred: true, ..(*key(',')).clone() })), ..base.clone() },
    ];
    for plan in other {
        core.pending = Some(Arc::new(plan));
        assert!(!core.space_follows_pending_sign(&space));
    }
    core.pending = Some(Arc::new(CorrectionPlan { boundary: None, sign_replayed: true, ..base }));
    assert!(core.space_follows_pending_sign(&space));
    core.pending = None;
    // The key still held repeats: the first press of the hotkey is still going on, unless no press was noted.
    core.convert_keycode = i64::from(PAUSE_KEYCODE);
    core.convert_released = false;
    core.convert_press_at = 0.0;
    let pause = KeyEvent { keycode: PAUSE_KEYCODE, ..named("Pause") };
    let _ = core.double_convert_press(&pause);
    core.convert_press_at = core.monotonic();
    core.convert_released = false;
    assert!(!core.double_convert_press(&pause));
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn undo_goes_back_over_spaces_only_when_they_are_all_there_is() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    let word = plan("ghbdtn", "привет", Some(' '));
    let with = |trailing: &str, boundary: Option<Stroke>| Arc::new(CorrectionPlan { trailing: keys(trailing), boundary, ..(*word).clone() });
    assert!(core.undo_past_spaces().is_none());
    core.last_correction = Some(word.clone());
    core.last_correction_time = core.monotonic();
    assert!(core.undo_past_spaces().is_none());
    core.last_committed = Some(with(" ", Some(key(' '))));
    assert!(core.undo_past_spaces().is_some());
    // One thing at a time that stands in the way.
    core.pending = Some(word.clone());
    assert!(core.undo_past_spaces().is_none());
    core.pending = None;
    core.last_committed_stale = true;
    assert!(core.undo_past_spaces().is_none());
    core.last_committed_stale = false;
    core.strokes = keys("a");
    assert!(core.undo_past_spaces().is_none());
    core.strokes.clear();
    core.symbol_strokes = keys(",");
    assert!(core.undo_past_spaces().is_none());
    core.symbol_strokes.clear();
    let moved = Arc::new(CorrectionPlan { strokes: keys("a"), ..(*word).clone() });
    core.last_committed = Some(moved);
    assert!(core.undo_past_spaces().is_none());
    // Something other than the typed boundary stands first; nothing follows; a key that is no plain space follows.
    core.last_committed = Some(with(",", Some(key(' '))));
    assert!(core.undo_past_spaces().is_none());
    core.last_committed = Some(word.clone());
    assert!(core.undo_past_spaces().is_none());
    core.last_committed = Some(with(" ", Some(key('a'))));
    assert!(core.undo_past_spaces().is_none());
    let bare = Arc::new(KeyEvent { characters: Vec::new(), ..(*key(' ')).clone() });
    core.last_committed = Some(with(" ", Some(bare)));
    assert!(core.undo_past_spaces().is_none());
    let twisted = Arc::new(KeyEvent { characters: vec![" ".to_owned(), "x".to_owned()], ..(*key(' ')).clone() });
    core.last_committed = Some(with(" ", Some(twisted)));
    assert!(core.undo_past_spaces().is_none());
    core.last_committed = Some(with(" ", None));
    core.last_correction = Some(Arc::new(CorrectionPlan { trailing: keys(" "), boundary: None, ..(*word).clone() }));
    assert!(core.undo_past_spaces().is_none());
    // A plan undone by the key that follows it has its own question.
    core.last_correction = Some(word.clone());
    core.last_committed = Some(word.clone());
    core.strokes = keys("ghbdtn");
    core.schedule_undo(i64::from(PAUSE_KEYCODE));
    core.pending = None;
    core.strokes.clear();
    core.last_committed = Some(Arc::new(CorrectionPlan { trailing: keys(","), ..(*word).clone() }));
    core.schedule_undo(i64::from(PAUSE_KEYCODE));
    assert!(matches!(core.convert_outcome, None | Some(ConvertOutcome::Plan(_) | ConvertOutcome::Switch)));
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn the_journal_says_what_it_can_of_what_was_dropped() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    assert_eq!(core.learning_diagnostics(None, "ghbdtn")["enabled"], json!(true));
    assert!(core.learning_diagnostics(Some(0), "ghbdtn").is_object());
    core.log_pending_dropped("");
    core.pending = Some(plan("ghbdtn", "привет", Some(' ')));
    core.log_pending_dropped("");
    core.log_pending_dropped("a reason");
    set(core, "exclusions.applications", json!(["firefox"]));
    core.log_pending_dropped("excluded");
    set(core, "exclusions.applications", json!([]));
    core.pending = None;
    assert_eq!(core.field_reader_status(), "custom_reader");
    assert_eq!(core.field_reader_details()["status"], json!("custom_reader"));
    core.context_policy.reader = None;
    assert_eq!(core.field_reader_status(), "not_configured");
    assert_eq!(core.field_reader_details()["status"], json!("not_configured"));
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn a_word_part_is_cut_only_where_a_word_could_end() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    let boundary = core.boundary_model.take();
    let comma = key(',');
    assert!(!core.ambiguous_key_is_boundary(&comma).expect("no failure"));
    core.strokes = keys("ghb");
    core.source_group = -1;
    assert!(!core.ambiguous_key_is_boundary(&comma).expect("no failure"));
    core.source_group = 0;
    assert!(!core.ambiguous_key_is_boundary(&key('\'')).expect("no failure"));
    let dash = KeyEvent { character: "-".to_owned(), ..(*comma).clone() };
    assert!(!core.ambiguous_key_is_boundary(&dash).expect("no failure"));
    let _ = core.ambiguous_key_is_boundary(&comma).expect("no failure");
    core.boundary_model = boundary;
    core.strokes.clear();
    // The questions about the parts of a token, for a layout with nothing to convert to and with no model.
    let word = keys("ghbdtn");
    assert_eq!(core.literal_head(&word, 2), 0);
    assert_eq!(core.joined_head(&keys("a-b"), 2), 0);
    assert_eq!(core.replayed_signs(&word, 0), 0);
    assert!(!core.word_begun("иг", 2, "bu", 0).expect("no failure"));
    assert!(!core.word_begun("ьь", 1, "mm", 0).expect("no failure"));
    let empty_meaning = Arc::new(KeyEvent { characters: vec![",".to_owned(), String::new()], character: ",".to_owned(), ..(*comma).clone() });
    assert_eq!(core.replayed_signs(&[empty_meaning.clone()], 0), 0);
    let space_meaning = Arc::new(KeyEvent { characters: vec![",".to_owned(), " ".to_owned()], character: ",".to_owned(), ..(*comma).clone() });
    assert_eq!(core.replayed_signs(&[space_meaning.clone()], 0), 0);
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn with_one_language_there_is_nothing_to_convert_to() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    let models = core.models.clone();
    core.models.truncate(1);
    let word = keys("a/b");
    assert_eq!(core.literal_head(&word, 0), 0);
    assert_eq!(core.joined_split(&keys("a-b"), 0), 0);
    assert_eq!(core.replayed_signs(&word, 0), 0);
    let (whole, tail, certain, _) = core.completed_word(&keys("ab,"), 0);
    assert_eq!((whole.len(), tail.len(), certain), (3, 0, false));
    core.source_group = 0;
    core.strokes = keys("ab");
    core.schedule_manual_conversion(i64::from(PAUSE_KEYCODE));
    assert!(core.pending.is_none());
    core.models = models;
    core.strokes.clear();
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn the_tail_of_a_word_is_cut_off_only_if_it_can_be_a_tail() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    // Nothing but signs, more signs than a word ends with, a word the person excluded.
    assert!(!core.completed_word(&keys(",,"), 0).2);
    assert!(!core.completed_word(&keys("abc,,,,,,,,,,,,,,,,"), 0).2);
    set(core, "exclusions.words", json!(["ABC,"]));
    assert!(!core.completed_word(&keys("abc,"), 0).2);
    set(core, "exclusions.words", json!([]));
    // A token whose parts hold no letters is not split.
    assert_eq!(core.joined_split(&keys("1-2"), 0), 0);
    assert_eq!(core.joined_split(&keys("-ghbdtn"), 0), 0);
    // The sign after a word replayed with it: a key with no meaning in the other layout, or a space there, stays.
    let base = plan("ghbdtn", "привет", Some(','));
    for characters in [[",", ""], [",", " "], [",", ","]] {
        let odd = Arc::new(KeyEvent { character: ",".to_owned(), characters: characters.iter().map(|text| (*text).to_owned()).collect(), ..(*key(',')).clone() });
        let replayed = EngineCore::_replay_sign_with_word(&Arc::new(CorrectionPlan { boundary: Some(odd), ..(*base).clone() }), &[]);
        assert_eq!(replayed.strokes.len(), 6);
    }
    // What may follow a committed word and still let Pause reach it.
    core.last_committed = Some(plan("ghbdtn", "привет", Some(' ')));
    let bare = Arc::new(KeyEvent { characters: Vec::new(), ..(*key(' ')).clone() });
    let _ = core.pause_reaches_past(std::slice::from_ref(&bare));
    let twisted = Arc::new(KeyEvent { character: "x".to_owned(), characters: vec![" ".to_owned(), " ".to_owned()], ..(*key(' ')).clone() });
    let _ = core.pause_reaches_past(std::slice::from_ref(&twisted));
    assert!(core.reopenable_committed_word().is_some());
    core.strokes = keys("a");
    assert!(core.reopenable_committed_word().is_none());
    core.strokes.clear();
    core.last_committed = Some(Arc::new(CorrectionPlan { strokes: Vec::new(), ..(*plan("ghbdtn", "привет", Some(' '))).clone() }));
    assert!(core.reopenable_committed_word().is_none());
    core.last_committed = Some(Arc::new(CorrectionPlan { boundary: Some(Arc::new(KeyEvent { character: "ab".to_owned(), ..(*key(' ')).clone() })), ..(*plan("ghbdtn", "привет", Some(' '))).clone() }));
    assert!(core.reopenable_committed_word().is_none());
    // Symbols alone do not make a mention, nor does a mention stand with nothing typed after it.
    core.symbol_strokes = keys("@");
    core.strokes.clear();
    assert!(core.mention_head("telegram-desktop").is_empty());
    core.symbol_strokes.clear();
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn a_word_typed_into_text_asks_the_field_around_it() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let reader = Fixed::new("ghbdtn", "");
    session.core.context_policy.reader = Some(reader.clone());
    let core = &mut session.core;
    let alternatives = vec![(1, "привет".to_owned())];
    let word = keys("ghbdtn");
    let ask = |core: &mut EngineCore| core.decide_inside_word(&word, 0, &alternatives, "firefox", CorrectionTrigger::Space, Some(&key(' ')));
    core.insertion = Some(InsertionPoint { head: "ab".to_owned(), tail: String::new() });
    // In a selection, in another program: the field says nothing of this word.
    let mut selected = field("xghbdtn", "");
    selected.selection = true;
    reader.show(Some(selected));
    assert!(ask(core).expect("no failure").is_some());
    let mut elsewhere = field("xghbdtn", "");
    elsewhere.application = "another".to_owned();
    reader.show(Some(elsewhere));
    assert!(ask(core).expect("no failure").is_some());
    // The letters around it are of the source layout, the target, both, or of a layout with no letters at all.
    for before in ["йghbdtn", "xghbdtn", "йxghbdtn", "1ghbdtn"] {
        reader.show(Some(field(before, "")));
        let _ = ask(core);
    }
    for layouts in [json!(["us", "de"]), json!(["de", "ru"]), json!(["ru", "ru"]), json!(["de", "de"])] {
        set(core, "detection.layouts", layouts);
        for before in ["йghbdtn", "xghbdtn", "йxghbdtn"] {
            reader.show(Some(field(before, "")));
            let _ = ask(core);
        }
    }
    set(core, "detection.layouts", json!(["us", "ru"]));
    core.insertion = None;
    // Whether the word stands where it was typed.
    core.insertion = Some(InsertionPoint { head: "ab".to_owned(), tail: String::new() });
    let mut other = field("ghbdtn", "");
    other.application = "another".to_owned();
    reader.show(Some(other));
    assert!(core.insertion_holds("ghbdtn", "", "firefox"));
    // The text before the word, unless it is a password or in another program.
    set(core, "detection.context_read_field", json!(true));
    reader.show(Some(field("x ghbdtn", "")));
    assert_eq!(core.field_before("ghbdtn").as_deref(), Some("x "));
    let mut secret = field("x ghbdtn", "");
    secret.sensitive = true;
    reader.show(Some(secret));
    assert!(core.field_before("ghbdtn").is_none());
    let mut selected = field("x ghbdtn", "");
    selected.selection = true;
    reader.show(Some(selected));
    assert!(core.field_before("ghbdtn").is_none());
    let mut other = field("x ghbdtn", "");
    other.application = "another".to_owned();
    reader.show(Some(other));
    assert!(core.field_before("ghbdtn").is_none());
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn a_decision_asked_where_it_cannot_be_made_says_so() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    let alternatives = vec![(1, "привет".to_owned())];
    // A layout the boundary model does not know.
    assert!(!core.signs_are_letters("ghb,", "при,", 2, 1, 1));
    assert!(!core.signs_are_letters("ghb,", "при,", 0, 2, 1));
    // An excluded program is its own decision, and a rule of the person's forces the answer before the next word.
    set(core, "exclusions.applications", json!(["firefox"]));
    let excluded = core.decide_word("ghbdtn", &alternatives, 0, "firefox", CorrectionTrigger::Space, WordOptions::default()).expect("no failure");
    assert!(!excluded.reason.is_empty());
    set(core, "exclusions.applications", json!([]));
    // A letter with its own layout among its readings.
    let own = vec![(0, "z".to_owned()), (1, "я".to_owned())];
    let _ = core.decide_word("z", &own, 0, "firefox", CorrectionTrigger::Space, WordOptions::default()).expect("no failure");
    core.learning.add_rule(ks_config::learning::LearnedRule::new("ghbdtn", 0, 1)).expect("a rule");
    let forced = core.planned_baseline("ghbdtn", &alternatives, 0, "привет", 1).expect("no failure");
    assert!(forced.should_convert);
    // A layout the detector has no model of.
    assert!(core.planned_baseline("ghbdtn", &alternatives, 7, "привет", 1).is_err());
    let whole = baseline_of(core, "mongo-cthdbcf");
    assert!(core.joined_head(&keys("mongo-cthdbcf"), 0) > 0);
    assert!(core.decide_joined_part(&keys("mongo-cthdbcf"), &[], 0, &[5], "firefox", CorrectionTrigger::Space, Some(&key(' ')), &whole).is_err());
    // No more than so many programs are remembered.
    let typed = keys("ghbdtn");
    for application in 0..200 {
        core.remember_context(&format!("program {application}"), 0, &typed);
    }
    assert!(core.contexts.len() < 200);
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn the_head_of_a_joined_token_is_asked_in_the_field_its_part_was_read_in() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    core.settings.write(ks_config::constants::DETECTION_CONTEXT_READ_FIELD, true, false).expect("in memory");
    let token = keys("proxy-ghbkj;tybt");
    let score = core.models[0].score("proxy-ghbkj;tybt");
    let whole = DetectionDecision::new(true, "proxy-ghbkj;tybt", "зкщчн-приложение", 0, 1, 0.9, "test", score.clone(), score);
    // The field shows the part after other text, not after the head: the head is asked with nothing before it, and
    // it is still the model that answers about it.
    let reader = Fixed::new("текст ghbkj;tybt", "");
    core.context_policy.reader = Some(reader.clone());
    let parted = core.decide_joined_part(&token, &[], 0, &[1], "firefox", CorrectionTrigger::Space, Some(&key(' ')), &whole).expect("no failure");
    assert_eq!(parted.map(|(split, part)| (split, part.replacement)), Some((6, "приложение".to_owned())));
    // The field shows the head before the part, as the program reads it.
    reader.show(Some(field("текст proxy-ghbkj;tybt", "")));
    let parted = core.decide_joined_part(&token, &[], 0, &[1], "firefox", CorrectionTrigger::Space, Some(&key(' ')), &whole).expect("no failure");
    assert_eq!(parted.map(|(split, _)| split), Some(6));
    bundle.give_back(session);
    bundle.clean();
}

/// A word kept in a layout picked by hand while an early switch still waits to be finished: the slip and the capitals of
/// the word are not fixed over the correction under way.
#[test]
fn a_word_in_a_layout_picked_by_hand_leaves_a_pending_early_switch_alone() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    session.select_layout(1);
    session.select_layout(0);
    for character in "ghbdtn".chars() {
        session.tap((*key(character)).clone());
    }
    let early = Arc::new(CorrectionPlan { mode: "early", ..(*plan("ghbdtn", "привет", None)).clone() });
    session.core.pending = Some(early);
    session.core.handle_inner(key(' ')).expect("no failure");
    assert_eq!(session.core.pending.as_ref().map(|pending| pending.mode), Some("early"));
    bundle.give_back(session);
    bundle.clean();
}

fn baseline_of(core: &EngineCore, original: &str) -> DetectionDecision {
    let score = core.models[0].score(original);
    DetectionDecision::new(false, original, original, 1, 0, 0.0, "test", score.clone(), score)
}

#[test]
fn late_keys_of_a_correction_that_cannot_be_typed_again_refuse_it() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    session.backend.set_text("ghbdtn ".chars().collect());
    session.backend.set_caret(7);
    let core = &mut session.core;
    queue(core, KeyEvent { characters: vec!["ab".to_owned(), "\u{1f600}".to_owned()], ..(*key('a')).clone() });
    assert!(!core.execute_correction(plan("ghbdtn", "привет", Some(' ')), None));
    while core.shared.get_nowait().is_some() {}
    // A mention held back to its first letter's release, with keys typed since and without them.
    let hidden = |trailing: &str| {
        let mut plan = (*plan("@ghb", "@при", None)).clone();
        plan.mode = "mention_hidden";
        plan.trailing = keys(trailing);
        Arc::new(plan)
    };
    let up = KeyEvent { pressed: false, ..(*key('z')).clone() };
    for (trailing, typed) in [(",", "ghb"), ("", "ghb"), ("", "xyz"), ("", "xyzw"), ("", "g")] {
        core.pending = Some(hidden(trailing));
        core.pressed.clear();
        core.strokes = keys(typed);
        core.source_group = 0;
        let _ = core.maybe_execute_pending(&up);
        core.pending = None;
    }
    // The worker wakes for a correction waiting on its field sooner than for anything else.
    core.field_wait = Some(FieldWait { plan: plan("ghbdtn", "привет", Some(' ')), learning_prompt: None, deadline: f64::MAX, presses: core.presses_handled, rolled: 0 });
    assert!(core.loop_timeout() < 0.5);
    core.field_wait = None;
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn the_toggle_says_when_it_could_not_be_saved() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let blocker = std::env::temp_dir().join(format!("ks-settings-blocker-{}", std::process::id()));
    std::fs::write(&blocker, "").expect("a file");
    let good = std::mem::replace(&mut session.core.settings, Arc::new(SettingsStore::new(Some(blocker.join("settings.json")))));
    set(&session.core, "hotkeys.toggle", json!("Pause"));
    let pause = KeyEvent { keycode: PAUSE_KEYCODE, ..named("Pause") };
    assert!(session.core.handle_inner(Arc::new(pause)).is_err());
    session.core.settings = good;
    std::fs::remove_file(blocker).expect("removed");
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn the_selection_commands_wait_for_the_keys_and_for_nothing_else() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    session.backend.set_selection(Some("ghbdtn"));
    let core = &mut session.core;
    core.schedule_selection_command("no_such_hotkey");
    assert!(core.selection_pending.is_none());
    core.schedule_selection_command("selection_layout");
    if core.selection_pending.is_some() {
        core.modifier_keycodes.insert(50);
        core.advance_selection_command();
        assert!(core.selection_pending.is_some());
        core.modifier_keycodes.clear();
        core.advance_selection_command();
    }
    // A password field is refused by its window, another window's password says nothing here.
    core.shared.set_focus_window(5);
    core.shared.set_sensitive_context_window(Some(5));
    core.run_selection_command(SelectionCommand::Layout);
    core.shared.set_sensitive_context_window(Some(6));
    core.run_selection_command(SelectionCommand::Layout);
    core.shared.set_sensitive_context_window(None);
    session.backend.set_selection(Some("ghbdtn"));
    session.backend.set_release_failing(true);
    session.core.run_selection_command(SelectionCommand::Layout);
    session.backend.set_release_failing(false);
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn a_hotkey_and_a_plan_answer_for_themselves() {
    let hotkey = Hotkey::new("Pause");
    let pause = KeyEvent { keycode: PAUSE_KEYCODE, ..named("Pause") };
    assert!(hotkey.matches(&pause));
    assert!(!hotkey.matches(&KeyEvent { pressed: false, ..pause }));
    // A hotkey that can never fire says why before the settings store it. One that can has no problem, however it is
    // spelled.
    // A keyboard, by the names of its keys.
    let keyboard = |names: &'static [&'static str]| move |name: &str| names.contains(&name);
    let exists = keyboard(&["pause", "k", "f9"]);
    for value in ["Pause", "pause", "F9", "Ctrl+Alt+K", "<Ctrl>+<Alt>+k", " shift + super + f9 "] {
        assert_eq!(Hotkey::problem(value, exists), None, "{value}");
    }
    // No key besides the modifiers, or a modifier key in the key's place.
    for value in ["Ctrl", "Ctrl+Alt", "control+meta", "Shift_L", "Ctrl+Control_R", "+"] {
        assert_eq!(Hotkey::problem(value, exists), Some(HotkeyProblem::NoKey), "{value}");
    }
    assert_eq!(Hotkey::problem("Ctrl+K+F9", exists), Some(HotkeyProblem::SeveralKeys));
    // A key by a name the keyboard has none of comes back as it was typed.
    assert_eq!(Hotkey::problem("Левый Ctrl", exists), Some(HotkeyProblem::UnknownKey("Левый Ctrl".to_owned())));
    assert_eq!(Hotkey::problem("Ctrl+Pausee", exists), Some(HotkeyProblem::UnknownKey("Pausee".to_owned())));
    // Break is Pause where the keyboard names the key so, as `matches` takes it.
    assert_eq!(Hotkey::problem("Pause", keyboard(&["break"])), None);
    assert_eq!(Hotkey::problem("Pause", keyboard(&[])), Some(HotkeyProblem::UnknownKey("Pause".to_owned())));
    // And a hotkey of Pause fires on the key by either name.
    assert!(hotkey.matches(&KeyEvent { keycode: PAUSE_KEYCODE, ..named("Break") }));
    assert_eq!(HotkeyProblem::NoKey.message(), "Сочетанию нужна обычная клавиша: одни Ctrl, Alt, Shift и Super не срабатывают. Например, Ctrl+Alt+K");
    assert!(HotkeyProblem::SeveralKeys.message().starts_with("В сочетании может быть только одна обычная клавиша"));
    assert_eq!(
        HotkeyProblem::UnknownKey("Левый Ctrl".to_owned()).message(),
        "Клавиши «Левый Ctrl» программа не знает. Буквы и цифры пишутся латиницей, например Ctrl+Alt+K"
    );
    let word = plan("hello", "Hello", None);
    assert!(!word.case_fix());
    assert!(Arc::new(CorrectionPlan { replaced: keys("hello"), ..(*word).clone() }).case_fix());
    assert!(EngineCore::_is_boundary(&named("KP_Enter")));
}

#[test]
fn settings_and_prompts_reach_whoever_is_still_there() {
    let mut bundle = Bundle::load(None);
    let session = session(&mut bundle);
    let settings = session.core.settings.clone();
    // Whoever listens to the prompts hears each one.
    let heard = Arc::new(Mutex::new(0));
    let counter = heard.clone();
    session.core.shared.subscribe_learning_prompts(Arc::new(move |prompt| {
        if prompt.is_some() {
            *counter.lock().expect("not poisoned") += 1;
        }
    }));
    let offer = ks_engine::engine::LearningPrompt {
        source_group: 0, target_group: 1, original: "ghbdtn".to_owned(), replacement: "привет".to_owned(), application: "firefox".to_owned(),
        action: ks_config::learning::RuleAction::Convert,
    };
    session.core.shared.show_learning_prompt(offer);
    assert_eq!(*heard.lock().expect("not poisoned"), 1);
    // A second subscription outlives the engine: its callback finds nobody there and does nothing.
    Shared::subscribe_settings(&session.core.shared);
    bundle.give_back(session);
    settings.set("general.keep_history", json!(false), false).expect("in memory");
    bundle.clean();
}

#[test]
fn the_keys_of_a_word_are_read_by_the_model_of_its_other_layout() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    // No keys for the model to read at all; every shape a word may have.
    let short = plan("z", "я", Some(' '));
    let empty = Arc::new(CorrectionPlan { original: String::new(), ..(*short).clone() });
    assert!(!core.keys_lean_to_target_language(&empty, ""));
    assert!(!core.keys_of_target_language(&empty, ""));
    // A shape the model was told not to score.
    let loaded = core.context_policy.ortho.clone().expect("the model ships");
    let mut no_shapes = (*loaded).clone();
    no_shapes.unscored_shapes = vec!["lower".to_owned()];
    core.context_policy.ortho = Some(Arc::new(no_shapes));
    assert!(!core.keys_of_target_language(&plan("ghbdtn", "привет", Some(' ')), ""));
    core.context_policy.ortho = Some(loaded);
    for word in ["ghbdtn", "GHBDTN", "Ghbdtn", "gHBDTN", "GHbdtn", "ghb1dtn", "gh,bd", "123456"] {
        for before in ["", "начало "] {
            let _ = core.keys_of_target_language(&plan("ghbdtn", word, Some(' ')), before);
            let _ = core.keys_of_target_language(&Arc::new(CorrectionPlan { original: word.to_owned(), ..(*plan("ghbdtn", word, Some(' '))).clone() }), before);
        }
    }
    let ortho = core.context_policy.ortho.take();
    assert!(!core.keys_of_target_language(&plan("ghbdtn", "привет", Some(' ')), ""));
    assert!(!core.keys_lean_to_target_language(&plan("ghbdtn", "привет", Some(' ')), ""));
    core.context_policy.ortho = ortho;
    bundle.give_back(session);
    bundle.clean();
}

/// A letter kept at its space, asked again with the sign after it; each thing that keeps the question from being asked.
#[test]
fn a_kept_word_is_asked_again_only_as_it_was_kept() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let backend = session.backend.clone();
    session.core.context_policy.reader = Some(Arc::new(ks_replay::harness::CaretReader { backend }));
    session.select_layout(1);
    for character in "прохождения м ".chars() {
        session.tap((*key(character)).clone());
    }
    let space = key(' ');
    let core = &mut session.core;
    let kept = core.kept.clone();
    if !kept.is_empty() {
        let plus = Arc::new(KeyEvent { group: 1, ..(*key('+')).clone() });
        let decision = baseline_of(core, "+");
        let of = |change: &dyn Fn(&mut KeptWord)| {
            let mut word = (*kept[kept.len() - 1]).clone();
            change(&mut word);
            vec![Arc::new(word)]
        };
        let deferred = Arc::new(KeyEvent { deferred: true, ..(*space).clone() });
        let strokes = vec![plus.clone()];
        let _ = core.decide_letter_before_sign(&kept, &strokes, &space, &decision, "firefox");
        let _ = core.decide_letter_before_sign(&kept, &strokes, &deferred, &decision, "firefox");
        for original in ["", "a b"] {
            let other = baseline_of(core, original);
            let _ = core.decide_letter_before_sign(&kept, &strokes, &space, &other, "firefox");
        }
        let _ = core.decide_letter_before_sign(&of(&|word| word.deadline = 0.0), &strokes, &space, &decision, "firefox");
        let _ = core.decide_letter_before_sign(&of(&|word| word.window = -1), &strokes, &space, &decision, "firefox");
        let _ = core.decide_letter_before_sign(&kept, &strokes, &space, &decision, "another");
        let _ = core.decide_letter_before_sign(&kept, &strokes, &space, &DetectionDecision { source_group: 0, ..decision.clone() }, "firefox");
        let mixed = vec![Arc::new(KeyEvent { group: 0, ..(*plus).clone() })];
        let _ = core.decide_letter_before_sign(&kept, &mixed, &space, &decision, "firefox");
        core.context_policy.stream.clear();
        let _ = core.decide_letter_before_sign(&kept, &strokes, &space, &decision, "firefox");
    }
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn what_is_asked_of_a_lone_letter_and_of_a_waiting_word_directly() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    // A letter with its own layout among its readings; a letter nothing is known of.
    let decision = baseline_of(core, "z");
    let own = vec![(0, "z".to_owned()), (1, "я".to_owned())];
    let _ = core.stranded_letter(decision.clone(), "z", &own, 0);
    assert!(!core.stranded_letter(decision, "x", &vec![(0, "x".to_owned())], 0).should_convert);
    bundle.give_back(session);
    let mut found = None;
    for first in ["tot", "ещ", "yt", "lf", "да", "z", "я", "x", "ч", "gj", "по", "rfr", "как", "qr", "йк", "d", "в", "c", "с"] {
        let mut session = bundle.session("firefox", 0, "начало ".chars().collect());
        session.recovers = true;
        let backend = session.backend.clone();
        session.core.context_policy.reader = Some(Arc::new(ks_replay::harness::CaretReader { backend }));
        for character in format!("{first} ").chars() {
            let event = key(character);
            session.select_layout(event.group);
            session.tap((*event).clone());
        }
        if session.core.context_waiting.is_some() {
            found = Some(session);
            break;
        }
        bundle.give_back(session);
    }
    let mut session = found.expect("some word waits for its neighbour");
    let waiting = session.core.context_waiting.clone();
    let waiting = waiting.expect("some word waits for its neighbour");
    let core = &mut session.core;
    let space = key(' ');
    let next = keys("ghbdtn");
    let alternatives = vec![(1, "привет".to_owned())];
    let decision = DetectionDecision { source_group: waiting.plan.source_group, target_group: waiting.plan.target_group, ..baseline_of(core, "ghbdtn") };
    // Each step of a phrase asked with the word that waits: as it is, with the policy off, with the next word switched.
    let _ = core.resolve_context_wait(Some(&waiting), &next, Some(&space), &decision, "firefox", &alternatives, true);
    let _ = core.resolve_context_wait(Some(&waiting), &keys("привет"), Some(&space), &decision, "firefox", &alternatives, true);
    let _ = core.resolve_context_wait(Some(&waiting), &keys("привет"), Some(&space), &DetectionDecision { source_group: 1 - waiting.plan.source_group, ..decision.clone() }, "firefox", &alternatives, true);
    let aged = ks_engine::engine::WaitingContextWord { deadline: 0.0, ..waiting.clone() };
    let _ = core.resolve_context_wait(Some(&aged), &keys("привет"), Some(&space), &decision, "firefox", &alternatives, true);
    set(core, "detection.context_policy", json!("off"));
    assert!(core.resolve_context_wait(Some(&waiting), &next, Some(&space), &decision, "firefox", &alternatives, false).expect("no failure").is_none());
    set(core, "detection.context_policy", json!("assist"));
    // The same, beside a kept next word, for a next word that is nothing or more than one.
    for original in ["", "a b"] {
        let other = DetectionDecision { original: original.to_owned(), ..decision.clone() };
        assert!(core.decide_with_kept_neighbour(&waiting, &next, &space, &other, "firefox").expect("no failure").is_none());
    }
    // A word converted at a pause stays open only for a plain space.
    let paused = ks_engine::engine::ProvisionalWord { plan: plan("ghbdtn", "привет", None), window: core.focus_or_zero(), deadline: f64::MAX };
    let deferred = Arc::new(KeyEvent { deferred: true, ..(*space).clone() });
    assert!(core.provisional_after_pause(Some(&paused), &next, 1, &deferred).is_none());
    let _ = core.provisional_after_pause(Some(&paused), &next, 1, &space);
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn words_kept_for_later_go_along_only_while_the_model_can_be_asked() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    let baseline = baseline_of(core, "lf");
    let word = |segmentation_open| Arc::new(KeptWord {
        plan: plan("lf", "да", Some(' ')),
        baseline: Some(baseline.clone()),
        field: Some(field("", "")),
        window: 0,
        deadline: f64::MAX,
        segmentation_open,
    });
    let next = plan("ghbdtn", "привет", Some(' '));
    for segmentation_open in [false, true] {
        for before in ["", "начало ", "x "] {
            let kept = vec![Arc::new(KeptWord { field: Some(field(before, "")), ..(*word(segmentation_open)).clone() })];
            let _ = core.take_along(&kept, &next, 6, "firefox", false);
        }
    }
    let model = core.context_policy.model.take();
    let _ = core.take_along(&[word(false)], &next, 6, "firefox", false);
    core.context_policy.model = model;
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn the_field_reader_of_the_platform_says_how_it_is() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let clock = session.core.clock.clone();
    let core = &mut session.core;
    let factory: ks_engine::context_access::ReaderFactory = Arc::new(|| Err(ks_backend::ReaderFailure { failure_type: "os_error", failure_name: "OSError", failure_code: None, permanent: false }));
    let platform = Arc::new(ks_engine::context_access::PlatformFieldReader::new(factory, clock));
    core.platform_reader = Some(platform.clone());
    // Another reader beside it speaks for itself.
    assert_eq!(core.field_reader_status(), "custom_reader");
    assert_eq!(core.field_reader_details()["status"], json!("custom_reader"));
    core.context_policy.reader = Some(platform);
    assert_ne!(core.field_reader_status(), "custom_reader");
    assert!(core.field_reader_details().get("retry").is_some());
    core.context_policy.reader = None;
    assert!(core.field_reader_details().get("retry").is_none());
    core.platform_reader = None;
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn a_rule_is_offered_even_when_there_is_nothing_more_to_convert() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    let done = plan("ghbdtn", "привет", Some(' '));
    for origin in [plan("ghbdtn", "привет", Some(' ')), plan("руддщ", "hello", Some(' '))] {
        core.pending = None;
        core.strokes.clear();
        core.last_committed = None;
        core.last_correction = None;
        core.convert_outcome = Some(ConvertOutcome::Plan(done.clone()));
        core.last_requested_plan = Some(done.clone());
        core.correction_origin = Some(origin);
        core.request_rule_after_double_press(i64::from(PAUSE_KEYCODE));
        core.shared.dismiss_learning_prompt(None, "test");
    }
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn a_prompt_is_heard_going_as_it_was_coming() {
    let mut bundle = Bundle::load(None);
    let session = session(&mut bundle);
    let shared = session.core.shared.clone();
    let heard = Arc::new(Mutex::new((0, 0)));
    let counter = heard.clone();
    shared.subscribe_learning_prompts(Arc::new(move |prompt| {
        let mut counts = counter.lock().expect("not poisoned");
        if prompt.is_some() {
            counts.0 += 1;
        } else {
            counts.1 += 1;
        }
    }));
    let offer = || ks_engine::engine::LearningPrompt {
        source_group: 0, target_group: 1, original: "ghbdtn".to_owned(), replacement: "привет".to_owned(), application: "firefox".to_owned(),
        action: ks_config::learning::RuleAction::Convert,
    };
    shared.show_learning_prompt(offer());
    assert!(shared.dismiss_learning_prompt(None, "test"));
    shared.show_learning_prompt(offer());
    let _ = shared.confirm_learning_prompt(None);
    shared.show_learning_prompt(offer());
    assert!(shared.expire_learning_prompt(Some(f64::MAX)));
    assert_eq!(*heard.lock().expect("not poisoned"), (3, 4), "each of three prompts came and went, and a listener hears the state as it subscribes");
    drop(shared);
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn a_shared_state_nobody_subscribed_goes_quietly() {
    let mut bundle = Bundle::load(None);
    let session = session(&mut bundle);
    let core = &session.core;
    let lone = Shared::new(core.settings.clone(), core.backend.clone(), core.clock.clone(), 2, ks_engine::engine::EngineSnapshot::default());
    drop(lone);
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn the_rule_window_reads_a_rule_in_words() {
    let mut rule = ks_config::learning::LearnedRule::new("ghbdtn", 0, 1);
    rule.case_sensitive = false;
    let plain = ks_engine::rule_editor::condition_text(&rule);
    rule.case_sensitive = true;
    assert_ne!(ks_engine::rule_editor::condition_text(&rule), plain);
}

#[test]
fn a_word_nobody_can_place_is_noted_without_a_second_opinion() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let alternatives = vec![(1, "привет".to_owned())];
    let core = &mut session.core;
    for source_group in [None, Some(5), Some(0)] {
        let evaluation = WordEvaluation {
            trigger: "space",
            original: "ghbdtn",
            alternatives: &alternatives,
            application: "firefox",
            enabled: false,
            trigger_enabled: true,
            manual_layout_protected: false,
            application_excluded: false,
            decision: None,
            caret_unknown: false,
            protection: None,
            source_group,
            early_switch_origin: None,
            idle_ms: None,
            context: None,
            literal_head: "",
        };
        core.log_word_evaluation(evaluation).expect("no failure");
    }
    // Selections when the layouts setting is not a list.
    set(core, "detection.layouts", json!(5));
    session.backend.set_selection(Some("ghbdtn"));
    session.core.run_selection_command(SelectionCommand::Layout);
    set(&session.core, "detection.layouts", json!(["us", "ru"]));
    bundle.give_back(session);
    bundle.clean();
}

struct KnowsEverything;

impl ks_lang::spellcheck::SpellChecker for KnowsEverything {
    fn available(&self) -> bool {
        true
    }
    fn source(&self) -> &str {
        ""
    }
    fn check(&self, _word: &str) -> bool {
        true
    }
}

#[test]
fn a_word_the_speller_knows_keeps_its_capitals() {
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let core = &mut session.core;
    let roots = [std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/models"))];
    let saved = core.models[0].clone();
    core.models[0] = Arc::new(ks_lang::language_model::LanguageModel::load("en_US", Vec::<String>::new(), &roots, Box::new(KnowsEverything)));
    assert_eq!(core.fix_double_capitals(&plan("HEllo", "?", Some(' ')), &key(' ')).original, "HEllo");
    core.models[0] = saved;
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn a_token_of_a_sign_and_a_word_has_no_head_to_keep() {
    let mut bundle = Bundle::load(None);
    let session = session(&mut bundle);
    for token in ["+ghbdtn", "=ghbdtn", "_ghbdtn", "-ghbdtn"] {
        let found: Strokes = token.chars().map(|character| {
            let keyed = key(character);
            Arc::new(KeyEvent { characters: if character.is_alphabetic() { keyed.characters.clone() } else { vec![character.to_string(), character.to_string()] }, ..(*keyed).clone() })
        }).collect();
        assert_eq!(session.core.joined_split(&found, 0), 0, "{token}");
    }
    bundle.give_back(session);
    bundle.clean();
}

/// A word left as typed, in each state it can be asked in, beside a word that converts: at a boundary and at a pause.
#[test]
fn every_kept_word_is_asked_again_beside_a_converted_one() {
    use ks_context::context_policy::STRANDED_SHORT_WORD_REASON;
    use ks_lang::short_words::{ISOLATED_SHORT_WORD_REASON, SHORT_WORD_REASONS};
    let mut bundle = Bundle::load(None);
    let mut session = session(&mut bundle);
    let backend = session.backend.clone();
    session.core.context_policy.reader = Some(Arc::new(ks_replay::harness::CaretReader { backend }));
    let core = &mut session.core;
    let next = plan("ghbdtn", "привет", Some(' '));
    let score = core.models[0].score("lf");
    for (word, reading) in [("z", "я"), ("d", "в"), ("r", "к"), ("f", "а"), ("b", "и"), ("c", "с"), ("lf", "да"), ("yt", "не"), ("gj", "по"), ("rfr", "как")] {
        for convert in [false, true] {
            for reason in ["test", SHORT_WORD_REASONS[0], ISOLATED_SHORT_WORD_REASON, STRANDED_SHORT_WORD_REASON] {
                for segmentation_open in [false, true] {
                    for before in ["", "начало ", "x ", "как дела "] {
                        let baseline = DetectionDecision::new(convert, word, reading, 0, 1, 0.9, reason, score.clone(), score.clone());
                        // The text as the engine saw it typed, which the question must match.
                        core.context_policy.stream.text = format!("{before}{word} ghbdtn ");
                        let kept = vec![Arc::new(KeptWord {
                            plan: plan(word, reading, Some(' ')),
                            baseline: Some(baseline),
                            field: Some(field(before, "")),
                            window: core.focus_or_zero(),
                            deadline: f64::MAX,
                            segmentation_open,
                        })];
                        let _ = core.take_along(&kept, &next, 6, "firefox", false);
                        // The same at a pause, with the word not ended yet.
                        core.context_policy.stream.text = format!("{before}{word} ghbdtn");
                        core.kept = kept;
                        core.strokes = keys("ghbdtn");
                        core.source_group = 0;
                        let now = core.monotonic();
                        core.last_word_input_at = Some(now - 5.0);
                        core.pause_correction_pending = true;
                        let _ = core.maybe_correct_after_pause(Some(now));
                        core.pending = None;
                        core.strokes.clear();
                        core.kept.clear();
                    }
                }
            }
        }
    }
    // With no model to ask, the baseline answers: the word goes along with its neighbour.
    let model = core.context_policy.model.take();
    let baseline = DetectionDecision::new(true, "lf", "да", 0, 1, 0.9, SHORT_WORD_REASONS[0], score.clone(), score.clone());
    let kept = [Arc::new(KeptWord { plan: plan("lf", "да", Some(' ')), baseline: Some(baseline), field: Some(field("", "")), window: core.focus_or_zero(), deadline: f64::MAX, segmentation_open: false })];
    "lf ghbdtn ".clone_into(&mut core.context_policy.stream.text);
    let along = core.take_along(&kept, &next, 6, "firefox", false).expect("no failure");
    assert_eq!(along.original, "lf ghbdtn");
    core.context_policy.model = model;
    bundle.give_back(session);
    bundle.clean();
}

/// A waiting word of any kind, asked with its neighbour: the planned answer and the pair's answer may differ.
#[test]
fn a_waiting_word_and_its_neighbour_may_disagree() {
    use ks_lang::short_words::SHORT_WORD_REASONS;
    let mut bundle = Bundle::load(None);
    let mut found = None;
    for first in ["tot", "ещ", "yt", "lf", "да", "z", "я", "x", "ч", "gj", "по", "rfr", "как", "qr", "йк", "d", "в", "c", "с"] {
        let mut session = bundle.session("firefox", 0, "начало ".chars().collect());
        session.recovers = true;
        let backend = session.backend.clone();
        session.core.context_policy.reader = Some(Arc::new(ks_replay::harness::CaretReader { backend }));
        for character in format!("{first} ").chars() {
            let event = key(character);
            session.select_layout(event.group);
            session.tap((*event).clone());
        }
        if session.core.context_waiting.is_some() {
            found = Some(session);
            break;
        }
        bundle.give_back(session);
    }
    let mut session = found.expect("some word waits for its neighbour");
    let core = &mut session.core;
    let waiting = core.context_waiting.clone().expect("it waits");
    let score = core.models[0].score("x");
    let space = key(' ');
    for (word, reading) in [("ghbdtn", "привет"), ("hello", "руддщ"), ("lf", "да"), ("z", "я"), ("d", "в"), ("x", "ч"), ("the", "еру"), ("git", "пше"), ("rfr", "как")] {
        // A rule of the person's plans the conversion of any word: the pair may still say no.
        core.learning.add_rule(ks_config::learning::LearnedRule::new(word, 0, 1)).expect("a rule");
        for (next, next_reading) in [("ghbdtn", "привет"), ("hello", "руддщ"), ("the", "еру"), ("git", "пше"), ("world", "цщкдв"), ("d", "в"), ("vbh", "мир")] {
            for convert in [false, true] {
                for reason in ["test", SHORT_WORD_REASONS[0]] {
                    let decision = DetectionDecision::new(convert, word, reading, 0, 1, 0.9, reason, score.clone(), score.clone());
                    let held = ks_engine::engine::WaitingContextWord { plan: plan(word, reading, Some(' ')), decision, baseline: None, window: core.focus_or_zero(), deadline: f64::MAX, ..waiting.clone() };
                    for neighbour_converts in [false, true] {
                        let neighbour = DetectionDecision::new(neighbour_converts, next, next_reading, 0, 1, 0.9, "test", score.clone(), score.clone());
                        // The text as the engine saw it typed, which the pair must match.
                        core.context_policy.stream.text = format!("{word} {next} ");
                        let _ = core.resolve_context_wait(Some(&held), &keys(next), Some(&space), &neighbour, "firefox", &vec![(1, next_reading.to_owned())], false);
                    }
                }
            }
        }
    }
    bundle.give_back(session);
    bundle.clean();
}

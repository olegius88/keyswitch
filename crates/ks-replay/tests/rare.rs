//! What only a rare order of events brings: each guard of the engine asked directly, with the state it guards against.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::{Arc, Mutex};
use std::sync::atomic::Ordering;

use ks_backend::constants::{ALT_MASK, CONTROL_MASK, SHIFT_MASK, SUPER_MASK};
use ks_backend::{FieldContext, FieldReader, FieldRole, KeyEvent};
use ks_engine::engine::{ConvertOutcome, CorrectionPlan, EngineCore, FieldWait, InsertionPoint, PlanRef, QueueItem, Stroke, Strokes};
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
        core.field_wait = Some(FieldWait { plan: of_field("ghbdtn", Some(' ')), learning_prompt: None, deadline: f64::MAX });
    };
    waiting(core);
    core.pressed.insert(50);
    core.retry_field_wait();
    assert!(core.field_wait.is_some());
    core.pressed.clear();
    core.modifier_keycodes.insert(50);
    core.retry_field_wait();
    assert!(core.field_wait.is_some());
    core.modifier_keycodes.clear();
    core.pending = Some(of_field("ghbdtn", Some(' ')));
    core.retry_field_wait();
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
    // Not letters in the other layout.
    let sign = {
        let mut early = (*plan("ghb", "при", None)).clone();
        early.mode = "early";
        Arc::new(early)
    };
    core.strokes = keys("ghb1");
    assert!(core.refresh_early_plan(&sign).expect("no failure").is_none());
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
    for known in ["MHz", "PCs", "TVs", "CDs", "IDs", "DNA", "McDonald"] {
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

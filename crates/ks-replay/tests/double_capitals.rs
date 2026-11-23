//! Shift held one letter too long at the start of a word (`ПРивет`): the case is fixed where the fixed word is a word of
//! the lexicon, also for a word of three letters among the commonest of its language, a word with a sign after it, parts
//! joined by a hyphen and a word converted from the other layout; undo puts back the keys as typed. Terms, units,
//! plurals of abbreviations and code of the same shape stay as typed.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::Arc;

use ks_backend::KeyEvent;
use ks_backend::constants::{LOCK_MASK, SHIFT_MASK};
use ks_config::learning::{LearnedRule, RuleAction};
use ks_context::{ContextAction, ContextPrediction};
use ks_engine::engine::{CorrectionPlan, InsertionPoint, PlanRef, Stroke, Strokes};
use ks_replay::harness::{Bundle, KEYS, PAUSE_KEYCODE, Session};
use serde_json::json;

const ENGLISH: usize = 0;
const RUSSIAN: usize = 1;

/// A press of the key that types `character` in layout `layout`, in the layout the window is in, with `state` added to
/// its Shift: no layout is picked for it.
fn press(session: &mut Session, character: char, layout: usize, state: u32) {
    let key = KEYS.iter().find(|key| key.characters[layout] == character).unwrap_or_else(|| panic!("no key types {character:?}"));
    let group = session.backend.group();
    let strings = key.strings();
    session.tap(KeyEvent {
        pressed: true,
        keycode: key.keycode,
        key_name: key.characters[0].to_string(),
        character: strings[usize::try_from(group).expect("a layout")].clone(),
        characters: strings,
        group,
        state: state | if key.shift { SHIFT_MASK } else { 0 },
        ..KeyEvent::default()
    });
}

/// The keys of `text` as layout `layout` types them, in the layout the window is in.
fn type_keys(session: &mut Session, text: &str, layout: usize) {
    for character in text.chars() {
        press(session, character, layout, 0);
    }
    session.idle(1.0);
}

fn shown(session: &Session) -> String {
    session.backend.text().iter().collect()
}

/// The undo hotkey, pressed and let go.
fn undo(session: &mut Session) {
    session.core.schedule_undo_hotkey(i64::from(PAUSE_KEYCODE));
    session.handle(KeyEvent { pressed: false, keycode: PAUSE_KEYCODE, key_name: "Pause".to_owned(), ..KeyEvent::default() });
    session.idle(1.0);
}

/// `typed` keys of layout `layout` in a new field of `application` in that layout, and what the field shows then.
fn typed_in(bundle: &mut Bundle, application: &str, layout: usize, typed: &str) -> String {
    let mut session = bundle.session(application, i32::try_from(layout).expect("a layout"), Vec::new());
    type_keys(&mut session, typed, layout);
    let text = shown(&session);
    bundle.give_back(session);
    text
}

/// A key of the reference keyboard as layout `layout` types `character`, typed in that layout.
fn key(character: char, layout: usize) -> Stroke {
    let key = KEYS.iter().find(|key| key.characters[layout] == character).unwrap_or_else(|| panic!("no key types {character:?}"));
    Arc::new(KeyEvent {
        pressed: true,
        keycode: key.keycode,
        key_name: key.characters[0].to_string(),
        character: character.to_string(),
        characters: key.strings(),
        group: i32::try_from(layout).expect("a layout"),
        state: if key.shift { SHIFT_MASK } else { 0 },
        ..KeyEvent::default()
    })
}

fn keys(text: &str, layout: usize) -> Strokes {
    text.chars().map(|character| key(character, layout)).collect()
}

/// The conversion of `typed`, keys of layout `layout`, into the other layout, as the engine plans it at a space.
fn conversion(typed: &str, layout: usize) -> PlanRef {
    let strokes = keys(typed, layout);
    let other = 1 - layout;
    let replacement: String = strokes.iter().map(|stroke| stroke.characters[other].as_str()).collect();
    let source = i64::try_from(layout).expect("a layout");
    Arc::new(CorrectionPlan::new(strokes, Some(key(' ', layout)), source, 1 - source, typed.to_owned(), replacement, 1.0, "app"))
}

/// A word kept in its layout, as the engine plans it at a space.
fn kept(typed: &str, layout: usize) -> PlanRef {
    let source = i64::try_from(layout).expect("a layout");
    Arc::new(CorrectionPlan::new(keys(typed, layout), Some(key(' ', layout)), source, source, typed.to_owned(), typed.to_owned(), 0.0, "app"))
}

#[test]
fn three_letters_are_fixed_where_the_word_is_among_the_commonest_of_its_language() {
    let mut bundle = Bundle::load(None);
    for (layout, typed, fixed) in [(RUSSIAN, "КАк ", "Как "), (RUSSIAN, "ТАк ", "Так "), (ENGLISH, "ANd ", "And "), (ENGLISH, "YOu ", "You ")] {
        assert_eq!(typed_in(&mut bundle, "app", layout, typed), fixed, "{typed}");
    }
    // A word of four letters is fixed as before; one of three that its language uses less often stays.
    assert_eq!(typed_in(&mut bundle, "app", RUSSIAN, "СЛово "), "Слово ");
    bundle.clean();
}

#[test]
fn a_sign_after_the_word_and_parts_joined_by_a_hyphen() {
    let mut bundle = Bundle::load(None);
    for (layout, typed, fixed) in [
        // The sign ends the word, typed in the layout of the word.
        (RUSSIAN, "ТАк! ", "Так! "),
        (RUSSIAN, "КАк, ", "Как, "),
        // The sign is a letter of the Russian layout: the code guard took the word for an identifier, and its sign was
        // never split off.
        (ENGLISH, "NOt. ", "Not. "),
        (ENGLISH, "HEllo. ", "Hello. "),
        // Parts joined by a hyphen, all of them lowercase after the first two capitals.
        (RUSSIAN, "КТо-то ", "Кто-то "),
        (RUSSIAN, "ГДе-нибудь ", "Где-нибудь "),
    ] {
        assert_eq!(typed_in(&mut bundle, "app", layout, typed), fixed, "{typed}");
    }
    bundle.clean();
}

#[test]
fn a_word_converted_from_the_other_layout_gets_its_case_fixed_in_the_same_correction() {
    let mut bundle = Bundle::load(None);
    for (layout, typed, fixed) in [
        (ENGLISH, "VFibyf ", "Машина "),
        (RUSSIAN, "ЦЩкдв ", "World "),
        (RUSSIAN, "ФТв ", "And "),
        // The last key is a sign here and a letter in the layout the word converts to.
        (ENGLISH, "DCt[ ", "Всех "),
    ] {
        assert_eq!(typed_in(&mut bundle, "app", layout, typed), fixed, "{typed}");
    }
    bundle.clean();
}

#[test]
fn undo_puts_back_the_keys_as_typed() {
    let mut bundle = Bundle::load(None);
    for (layout, typed, fixed) in [
        (RUSSIAN, "КАк ", "Как "),
        (ENGLISH, "NOt. ", "Not. "),
        (RUSSIAN, "КТо-то ", "Кто-то "),
        (ENGLISH, "VFibyf ", "Машина "),
        (ENGLISH, "DCt[ ", "Всех "),
    ] {
        let mut session = bundle.session("app", i32::try_from(layout).expect("a layout"), Vec::new());
        type_keys(&mut session, typed, layout);
        assert_eq!(shown(&session), fixed, "{typed}");
        undo(&mut session);
        assert_eq!(shown(&session), typed, "{typed} undone");
        bundle.give_back(session);
    }
    bundle.clean();
}

/// A word converted at a space is asked about again when the next word goes the other way (`revert_with_next_word`):
/// taken back with it, it is typed as it was, Shift held too long included.
#[test]
fn a_converted_word_taken_back_with_its_neighbour_has_its_capitals_as_typed() {
    let mut bundle = Bundle::load(None);
    let mut session = bundle.session("app", 0, Vec::new());
    session.core.settings.set("detection.early_switch", json!(false), false).expect("in memory");
    let prediction = |action: ContextAction| {
        let mut probabilities = [0.0; 4];
        probabilities[usize::from(action == ContextAction::Convert)] = 1.0;
        ContextPrediction { action, probability: 1.0, probabilities, model_version: "test".to_owned(), supported: true }
    };
    // Both words convert alone; asked again with the next word after it, the first one stays.
    let model = session.core.context_policy.model.as_mut().expect("a model");
    model.answer = Some(Arc::new(move |item| prediction(if item.field.after.is_empty() { ContextAction::Convert } else { ContextAction::Keep })));
    type_keys(&mut session, "VFibyf ", ENGLISH);
    assert_eq!(shown(&session), "Машина ");
    type_keys(&mut session, "сщву ", RUSSIAN);
    assert_eq!(shown(&session), "VFibyf code ");
    session.core.context_policy.model.as_mut().expect("a model").answer = None;
    bundle.give_back(session);
    bundle.clean();
}

#[test]
fn terms_units_plurals_and_code_of_the_shape_stay_as_typed() {
    let mut bundle = Bundle::load(None);
    let english = [
        "PCs", "IDs", "CDs", "TVs", "VMs", "IPs", "PRs", "GBs", "URLs", "GHz", "MHz", "eM", "iPhone", "CMake", "JUnit", "OAuth", "VMware",
        "IPsec", "DBeaver", "ILogger", "IEnumerable", "QString", "TForm", "IList.Count", "NUnit.Framework", "PCs.", "IDs,", "URLs.", "GHz.",
    ];
    let russian = ["МГц", "МВт", "МПа", "кВт", "СШа", "ОСи", "МГц,", "МВт-ч"];
    for application in ["app", "code"] {
        for (layout, words) in [(ENGLISH, &english[..]), (RUSSIAN, &russian[..])] {
            for word in words {
                assert_eq!(typed_in(&mut bundle, application, layout, &format!("{word} ")), format!("{word} "), "{word} in {application}");
            }
        }
    }
    bundle.clean();
}

#[test]
fn caps_lock_a_field_kept_private_and_the_setting_off_keep_the_word() {
    let mut bundle = Bundle::load(None);
    // Under Caps Lock the capitals came without Shift and the lowercase letter with it.
    let mut session = bundle.session("app", 1, Vec::new());
    for (letter, shifted) in [('к', false), ('а', false), ('к', true)] {
        let key = KEYS.iter().find(|key| key.characters[RUSSIAN] == letter).expect("a key of the letter");
        let characters: Vec<String> = key.characters.iter().map(|character| if shifted { character.to_string() } else { character.to_uppercase().collect() }).collect();
        session.tap(KeyEvent {
            pressed: true,
            keycode: key.keycode,
            key_name: key.characters[0].to_string(),
            character: characters[RUSSIAN].clone(),
            characters,
            group: 1,
            state: LOCK_MASK | if shifted { SHIFT_MASK } else { 0 },
            ..KeyEvent::default()
        });
    }
    type_keys(&mut session, " ", RUSSIAN);
    assert_eq!(shown(&session), "КАк ");
    bundle.give_back(session);
    // A field the program reads nothing of, as a password's.
    let mut session = bundle.session("app", 1, Vec::new());
    let window = session.core.focus_or_zero();
    session.core.shared.set_sensitive_context_window(Some(window));
    type_keys(&mut session, "КАк ", RUSSIAN);
    assert_eq!(shown(&session), "КАк ");
    session.core.shared.set_sensitive_context_window(None);
    bundle.give_back(session);
    // With the fix turned off every shape stays, the sign after a word in the English layout too.
    for (layout, typed, kept) in [(RUSSIAN, "КАк ", "КАк "), (ENGLISH, "NOt. ", "NOt. "), (ENGLISH, "VFibyf ", "МАшина ")] {
        let mut session = bundle.session("app", i32::try_from(layout).expect("a layout"), Vec::new());
        session.core.settings.set("detection.fix_double_capitals", json!(false), false).expect("in memory");
        type_keys(&mut session, typed, layout);
        assert_eq!(shown(&session), kept, "{typed}");
        bundle.give_back(session);
    }
    // A word the early switch converted while it was typed keeps its case, with the sign split off its end.
    assert_eq!(typed_in(&mut bundle, "app", RUSSIAN, "руддщю "), "hello. ");
    bundle.clean();
}

/// Each thing that keeps the case of a word as typed, one at a time, asked of the plans themselves.
#[test]
fn what_keeps_a_case_as_typed_one_at_a_time() {
    let mut bundle = Bundle::load(None);
    let mut session = bundle.session("app", 1, Vec::new());
    let core = &mut session.core;
    core.settings.set("diagnostics.technical_logging", json!(true), false).expect("in memory");
    let space = key(' ', RUSSIAN);
    let fixed = |core: &mut ks_engine::engine::EngineCore, plan: &PlanRef| {
        let text = core.fix_double_capitals(plan, &space).original.clone();
        core.pending = None;
        text
    };
    assert_eq!(fixed(core, &kept("КАк", RUSSIAN)), "Как");
    // Shapes of no fix: two letters, a third capital, a capital after a hyphen, a letter or a digit after a sign.
    for word in ["КА", "КАК", "КТо-То", "КТо-", "КАк.ли", "КАк1", "Как"] {
        assert_eq!(fixed(core, &kept(word, RUSSIAN)), word, "{word}");
    }
    // A word the user excluded, or one a rule of theirs converts; letters typed into a word.
    core.settings.set("exclusions.words", json!(["как"]), false).expect("in memory");
    assert_eq!(fixed(core, &kept("КАк", RUSSIAN)), "КАк");
    core.settings.set("exclusions.words", json!([]), false).expect("in memory");
    for rule in [LearnedRule::new("как", 1, 0), LearnedRule { action: RuleAction::Keep, ..LearnedRule::new("как", 1, 0) }] {
        core.learning.add_rule(rule.clone()).expect("a rule");
        assert_eq!(fixed(core, &kept("КАк", RUSSIAN)), "КАк");
        core.learning.remove_rule(&rule).expect("the rule removed");
    }
    core.insertion = Some(InsertionPoint { head: "ab".to_owned(), tail: String::new() });
    assert_eq!(fixed(core, &kept("КАк", RUSSIAN)), "КАк");
    core.insertion = None;
    // Keys of another layout among the word's, and a key under Caps Lock.
    let mut mixed = (*kept("КАк", RUSSIAN)).clone();
    mixed.strokes[2] = key('r', ENGLISH);
    assert_eq!(fixed(core, &Arc::new(mixed)), "КАк");
    let mut locked = (*kept("КАк", RUSSIAN)).clone();
    locked.strokes[0] = Arc::new(KeyEvent { state: LOCK_MASK | SHIFT_MASK, ..(*locked.strokes[0]).clone() });
    assert_eq!(fixed(core, &Arc::new(locked)), "КАк");
    // A conversion with a mention head, a typo correction's keys, and a whole token that keeps its end as typed.
    let converted = conversion("VFibyf", ENGLISH);
    assert_eq!(core.fix_converted_capitals(&converted).replacement, "Машина");
    for plan in [
        CorrectionPlan { head: keys("@", ENGLISH), ..(*converted).clone() },
        CorrectionPlan { replaced_strokes: keys("Vfibyf", ENGLISH), ..(*converted).clone() },
        CorrectionPlan { kept_tail: 1, ..(*converted).clone() },
    ] {
        assert_eq!(core.fix_converted_capitals(&Arc::new(plan)).replacement, "МАшина");
    }
    // The case of a conversion's plan: a layout changed is no case fix, though the keys typed are kept for undo.
    let fix = core.fix_converted_capitals(&converted);
    assert!(!fix.replaced.is_empty() && !fix.case_fix());
    bundle.give_back(session);
    bundle.clean();
}

//! A typo correction the person erases, the undo of a correction found wrong a few words later, and a correction made only
//! where the word it types is a word of written text, chosen as clearly there as in the subtitle list.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::Arc;

use ks_backend::KeyEvent;
use ks_backend::constants::SHIFT_MASK;
use ks_engine::engine::{CorrectionPlan, Update};
use ks_replay::harness::{Bundle, KEYS, PAUSE_KEYCODE, Session};

/// A press of the key of `character`, in the layout that types it.
fn press(session: &mut Session, character: char) {
    let Some((key, group)) = KEYS.iter().find_map(|key| Some((key, key.characters.iter().position(|typed| *typed == character)?))) else {
        panic!("no key types {character:?}");
    };
    if usize::try_from(session.backend.group()).ok() != Some(group) && character != ' ' {
        session.select_layout(i32::try_from(group).expect("two layouts"));
    }
    session.tap(KeyEvent {
        pressed: true,
        keycode: key.keycode,
        key_name: character.to_string(),
        character: character.to_string(),
        characters: key.strings(),
        group: session.backend.group(),
        state: if key.shift { SHIFT_MASK } else { 0 },
        ..KeyEvent::default()
    });
}

/// A press of the key that types `character` in the layout the window is in.
fn press_as_laid_out(session: &mut Session, character: char) {
    let group = session.backend.group();
    let key = KEYS.iter().find(|key| usize::try_from(group).ok().and_then(|place| key.characters.get(place)) == Some(&character)).expect("a key of the layout");
    session.tap(KeyEvent {
        pressed: true,
        keycode: key.keycode,
        key_name: character.to_string(),
        character: character.to_string(),
        characters: key.strings(),
        group,
        state: if key.shift { SHIFT_MASK } else { 0 },
        ..KeyEvent::default()
    });
}

fn type_text(session: &mut Session, text: &str) {
    for character in text.chars() {
        press(session, character);
    }
}

fn erase(session: &mut Session, characters: usize) {
    for _ in 0..characters {
        session.tap_named("BackSpace");
    }
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

/// A field in the Russian layout where typos are corrected, and a word typed first whose keys a correction may need.
fn russian(bundle: &mut Bundle) -> Session {
    let mut session = bundle.session("app", 1, Vec::new());
    session.core.settings.set("detection.correct_typos", true.into(), false).expect("in memory");
    session.core.settings.set("diagnostics.technical_logging", true.into(), false).expect("in memory");
    type_text(&mut session, "он ");
    session.idle(1.0);
    session
}

/// `мржно` is `можно` with one slip, and the correction is made. Then the person erases it, as the person who meant the
/// word as typed does: a correction made again on the same keys would undo their work each time, and only the undo
/// hotkey got them out. The word typed again at the place is left, and is theirs: typed later, it is left too.
#[test]
fn a_typo_correction_erased_with_backspace_is_not_made_again_on_the_word_typed_there() {
    let mut bundle = Bundle::load(None);
    let mut session = russian(&mut bundle);
    type_text(&mut session, "мржно ");
    session.idle(1.0);
    assert_eq!(shown(&session), "он можно ", "the slip is corrected");
    erase(&mut session, "можно ".chars().count());
    type_text(&mut session, "мржно ");
    session.idle(1.0);
    assert_eq!(shown(&session), "он мржно ", "the word typed again where the correction was erased stays as typed");
    assert!(session.core.personal.contains("мржно"), "and it is the person's own word now");
    type_text(&mut session, "мржно ");
    session.idle(1.0);
    assert_eq!(shown(&session), "он мржно мржно ", "a word of the person's own is not corrected later either");
    bundle.give_back(session);

    // Erased through what was typed after it, the correction is erased all the same (another slip: `мржно` is the
    // person's own word now).
    let mut session = russian(&mut bundle);
    type_text(&mut session, "мтжно и ");
    session.idle(1.0);
    assert_eq!(shown(&session), "он можно и ");
    erase(&mut session, "можно и ".chars().count());
    type_text(&mut session, "мтжно ");
    session.idle(1.0);
    assert_eq!(shown(&session), "он мтжно ", "erased through the word after it");
    bundle.give_back(session);
    bundle.clean();
}

/// The person saw their own slip and pressed Backspace at once, as the correction of that slip came: those Backspaces were
/// meant for the text as typed, sooner than anyone reads the corrected word. Reaching that word they leave it to be
/// corrected as any word. A correction read and then erased is the person's, as above.
#[test]
fn backspaces_begun_before_the_correction_could_be_read_leave_the_word_to_be_corrected() {
    let mut bundle = Bundle::load(None);
    let mut session = russian(&mut bundle);
    type_text(&mut session, "мржно ");
    assert_eq!(shown(&session), "он можно ", "the slip is corrected at the space");
    erase(&mut session, "можно ".chars().count());
    type_text(&mut session, "мржно ");
    session.idle(1.0);
    assert_eq!(shown(&session), "он можно ", "erased before the correction could be read, the word is corrected again");
    assert!(!session.core.personal.contains("мржно"), "and is no word of the person's own");
    erase(&mut session, "можно ".chars().count());
    type_text(&mut session, "мржно ");
    session.idle(1.0);
    assert_eq!(shown(&session), "он мржно ", "erased once read, the correction is not made again");
    bundle.give_back(session);
    bundle.clean();
}

/// The person erased only the last letter of the corrected word and typed another one: the letter the correction changed
/// earlier in the word stayed, and correcting the word again changed it back with no one seeing why. A word the person is
/// editing after its typo correction is theirs to finish; the next word is corrected as any other.
#[test]
fn a_word_edited_after_its_typo_correction_is_the_persons_to_finish() {
    let mut bundle = Bundle::load(None);
    let mut session = russian(&mut bundle);
    type_text(&mut session, "мржно ");
    session.idle(1.0);
    assert_eq!(shown(&session), "он можно ");
    erase(&mut session, 2);
    type_text(&mut session, "г ");
    session.idle(1.0);
    assert_eq!(shown(&session), "он можнг ", "the edit stands as the person typed it");
    assert!(!session.core.personal.contains("можнг"), "an edit is no word of the person's own");
    type_text(&mut session, "мржно ");
    session.idle(1.0);
    assert_eq!(shown(&session), "он можнг можно ", "the next word is corrected again");
    bundle.give_back(session);

    // A click before the word is typed again: the place is no longer known to be the one erased.
    let mut session = russian(&mut bundle);
    type_text(&mut session, "мржно ");
    session.idle(1.0);
    erase(&mut session, 2);
    session.pointer();
    type_text(&mut session, " мржно ");
    session.idle(1.0);
    assert_eq!(shown(&session), "он можн можно ", "after a click the word is judged as any word");
    bundle.give_back(session);
    bundle.clean();
}

/// The subtitle list takes `формар` for `форман`, a name films say and written text never does, and `сьрока` for
/// `сорока`, where written text says `строка` far more often, one slip away as well: neither is corrected.
#[test]
fn a_typo_is_corrected_only_into_a_word_written_text_chooses_as_clearly() {
    let mut bundle = Bundle::load(None);
    let mut session = russian(&mut bundle);
    // The keys of the letters the subtitle list would bring.
    type_text(&mut session, "сорока ");
    session.idle(1.0);
    type_text(&mut session, "формар ");
    session.idle(1.0);
    type_text(&mut session, "сьрока ");
    session.idle(1.0);
    assert_eq!(shown(&session), "он сорока формар сьрока ", "no word of written text, or a tie between the two lists");
    // A slip both lists undo alike is still corrected.
    type_text(&mut session, "мржно ");
    session.idle(1.0);
    assert_eq!(shown(&session), "он сорока формар сьрока можно ");
    bundle.give_back(session);
    bundle.clean();
}

/// A wrong typo correction is seen once the next words are typed: the undo hotkey takes it back across them, types the
/// words after it again as they stood, and the word in progress goes on. The word put back is the person's own.
#[test]
fn the_undo_hotkey_takes_a_typo_correction_back_across_the_words_typed_after_it() {
    let mut bundle = Bundle::load(None);
    let mut session = russian(&mut bundle);
    type_text(&mut session, "мржно и та");
    session.idle(0.2);
    assert_eq!(shown(&session), "он можно и та");
    undo(&mut session);
    assert_eq!(shown(&session), "он мржно и та", "the word as typed, the text after it as it stood");
    assert_eq!(session.backend.group(), 1, "the layout as it was");
    assert!(session.core.personal.contains("мржно"));
    type_text(&mut session, "к ");
    session.idle(1.0);
    assert_eq!(shown(&session), "он мржно и так ", "the word in progress goes on");
    bundle.give_back(session);
    bundle.clean();
}

/// The same for a word converted to the other layout: the undo puts its keys back in the layout they were typed in, and
/// the text typed after the conversion, in the new layout, stays in it, as does the keyboard.
#[test]
fn the_undo_hotkey_takes_a_layout_correction_back_across_the_words_typed_after_it() {
    let mut bundle = Bundle::load(None);
    let mut session = bundle.session("app", 0, Vec::new());
    type_text(&mut session, "ghbdtn ");
    session.idle(1.0);
    assert_eq!(shown(&session), "привет ");
    assert_eq!(session.backend.group(), 1);
    type_text(&mut session, "как дела");
    press_as_laid_out(&mut session, '.');
    press_as_laid_out(&mut session, ' ');
    session.idle(0.2);
    undo(&mut session);
    assert_eq!(shown(&session), "ghbdtn как дела. ", "the word as typed, the words after it as they stood");
    assert_eq!(session.backend.group(), 1, "the layout the words after it were typed in");
    bundle.give_back(session);
    bundle.clean();
}

/// What the undo hotkey does not reach across: text the engine did not see typed key by key, keys an editor may turn into
/// something else, more keys than it types again, a key of another layout, and a correction too long ago. It then does
/// what it did: nothing, and the text stays.
#[test]
fn the_undo_hotkey_reaches_across_only_text_it_knows_key_by_key() {
    type Step = fn(&mut Session);
    let steps: [(&str, Step); 12] = [
        ("Backspace", |session| {
            type_text(session, "ии");
            session.tap_named("BackSpace");
        }),
        ("a bracket", |session| {
            type_text(session, "и ");
            press_as_laid_out(session, '(');
            type_text(session, "та");
        }),
        ("two spaces", |session| {
            press_as_laid_out(session, ' ');
            type_text(session, "и");
        }),
        ("two signs", |session| {
            press_as_laid_out(session, '.');
            press_as_laid_out(session, '.');
        }),
        ("too many keys", |session| type_text(session, "и так и так и так и так и так")),
        ("an arrow key", |session| {
            type_text(session, "и");
            session.tap_named("Left");
        }),
        ("another layout", |session| type_text(session, "ok")),
        ("too long ago", |session| {
            type_text(session, "и");
            session.idle(11.0);
        }),
        ("a function key", |session| {
            type_text(session, "и");
            session.tap_named("F5");
        }),
        // The editor changed the text by itself: the field read shows it.
        ("text the editor added", |session| {
            type_text(session, "и");
            session.backend.insert_at_caret("!");
        }),
        ("a command for the selection", |session| {
            type_text(session, "и");
            session.tap(KeyEvent { pressed: true, keycode: PAUSE_KEYCODE, key_name: "Pause".to_owned(), state: SHIFT_MASK, group: 1, ..KeyEvent::default() });
        }),
        ("the rule window", |session| {
            type_text(session, "и");
            session.core.shared.set_rule_editor_open(true);
            type_text(session, "да");
            session.core.shared.set_rule_editor_open(false);
        }),
    ];
    let mut bundle = Bundle::load(None);
    for (name, step) in steps {
        let mut session = russian(&mut bundle);
        type_text(&mut session, "мржно ");
        session.idle(0.2);
        step(&mut session);
        let before = shown(&session);
        undo(&mut session);
        assert_eq!(shown(&session), before, "after {name} the undo leaves the text");
        bundle.give_back(session);
    }
    // Spaces alone after the correction are reached as before (`undo_past_spaces`).
    let mut session = russian(&mut bundle);
    type_text(&mut session, "мржно ");
    session.idle(0.2);
    press_as_laid_out(&mut session, ' ');
    undo(&mut session);
    assert_eq!(shown(&session), "он мржно  ", "the spaces after the correction");
    bundle.give_back(session);
    // Within the bounds the same text is reached: a sign, a space and the word after it, typed nine seconds on.
    let mut session = russian(&mut bundle);
    type_text(&mut session, "мтжно");
    press_as_laid_out(&mut session, ',');
    type_text(&mut session, " и");
    session.idle(9.0);
    assert_eq!(shown(&session), "он можно, и");
    undo(&mut session);
    assert_eq!(shown(&session), "он мтжно, и", "a sign, a space and a word after the correction");
    bundle.give_back(session);
    // A correction the person made with Pause is theirs: the undo hotkey does not reach it across later words.
    let mut session = bundle.session("app", 0, Vec::new());
    type_text(&mut session, "ghbdtn");
    session.tap_named("Pause");
    session.idle(1.0);
    type_text(&mut session, " ok");
    let before = shown(&session);
    undo(&mut session);
    assert_eq!(shown(&session), before, "a conversion by hand");
    bundle.give_back(session);
    bundle.clean();
}

/// Shift held one letter too long (`HEllo`) is fixed at the word's boundary; the undo hotkey takes the fix back across
/// the word typed after it, as it takes back any other automatic correction.
#[test]
fn the_undo_hotkey_takes_a_case_fix_back_across_the_word_typed_after_it() {
    let mut bundle = Bundle::load(None);
    let mut session = bundle.session("app", 0, Vec::new());
    type_text(&mut session, "HEllo ");
    session.idle(1.0);
    assert_eq!(shown(&session), "Hello ");
    type_text(&mut session, "wor");
    undo(&mut session);
    assert_eq!(shown(&session), "HEllo wor");
    bundle.give_back(session);
    bundle.clean();
}

/// Each thing that keeps the undo hotkey from reaching the correction across the text after it, one at a time.
#[test]
fn what_keeps_the_undo_from_reaching_across_words_one_at_a_time() {
    let mut bundle = Bundle::load(None);
    let mut session = russian(&mut bundle);
    type_text(&mut session, "мржно и");
    session.idle(0.2);
    let core = &mut session.core;
    assert!(core.undo_across_words().is_some(), "within every bound");
    let correction = core.last_correction.clone().expect("the typo correction");
    let typed = core.typed_after_correction.clone();
    let when = core.last_correction_time;
    let changed = |change: &dyn Fn(&mut CorrectionPlan)| {
        let mut plan = (*correction).clone();
        change(&mut plan);
        Some(Arc::new(plan))
    };
    core.last_correction = None;
    assert!(core.undo_across_words().is_none(), "no correction");
    core.last_correction = changed(&|plan| plan.boundary = None);
    assert!(core.undo_across_words().is_none(), "a correction that ended no word");
    core.last_correction = changed(&|plan| plan.automatic = false);
    assert!(core.undo_across_words().is_none(), "a correction made by hand");
    core.last_correction = Some(correction.clone());
    core.typed_after_correction = None;
    assert!(core.undo_across_words().is_none(), "text not known key by key");
    core.typed_after_correction = Some(Vec::new());
    assert!(core.undo_across_words().is_none(), "nothing typed after it: the undo past spaces or the plain undo");
    core.typed_after_correction.clone_from(&typed);
    core.pending = Some(correction.clone());
    assert!(core.undo_across_words().is_none(), "a plan waiting for its keys");
    core.pending = None;
    core.last_correction_time = when - 11.0;
    assert!(core.undo_across_words().is_none(), "too long ago");
    core.last_correction_time = when;
    core.update(Update { current_group: Some(0), ..Update::default() });
    assert!(core.undo_across_words().is_none(), "another layout on");
    core.update(Update { current_group: Some(1), ..Update::default() });
    let word = std::mem::take(&mut core.strokes);
    core.strokes = correction.strokes.clone();
    assert!(core.undo_across_words().is_none(), "a word in progress that is not what was typed last");
    core.strokes = word;
    assert!(core.undo_across_words().is_some(), "every bound again");
    bundle.give_back(session);
    bundle.clean();
}

/// Pause past the spaces after a typo correction puts the word back as the undo hotkey does there; converting the
/// corrected word to the other layout made it letters of no language.
#[test]
fn pause_past_the_spaces_after_a_typo_correction_puts_the_word_back() {
    let mut bundle = Bundle::load(None);
    let mut session = russian(&mut bundle);
    type_text(&mut session, "мржно ");
    session.idle(0.2);
    press_as_laid_out(&mut session, ' ');
    session.tap_named("Pause");
    session.idle(1.0);
    assert_eq!(shown(&session), "он мржно  ");
    assert!(session.core.personal.contains("мржно"));
    bundle.give_back(session);
    bundle.clean();
}

//! The quote key of the Russian layout in Telegram, where the same key is `@` of the English one.
//!
//! A lone quote typed where a mention may begin shows as `@` at once, for Telegram opens its list of people only after a
//! real `@`, and the list is how a mention is made: the arrows and Enter, or a click. Typing on writes the quote back and
//! the word after it decides (`"ощрт` is `@john`, `"привет` a quotation). Everything else the quote key is used for in a
//! chat must give a quote: a quotation typed as the text goes, and quotes put around text already written, after the
//! caret was moved there with the keys or the mouse, after an erasure and after a paste.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ks_backend::constants::{CONTROL_MASK, SHIFT_MASK};
use ks_backend::{FieldContext, FieldReader, FieldRole, KeyEvent};
use ks_config::constants::DETECTION_CONTEXT_READ_FIELD;
use ks_replay::harness::{Backend, Bundle, KEYS, PAUSE_KEYCODE, Session, UnreadField};

const ENGLISH: i32 = 0;
const RUSSIAN: i32 = 1;

/// One step of a scene in Telegram.
#[derive(Clone, Copy)]
enum Step {
    /// The keys that type this text in the US layout, pressed in whatever layout is on: `@` is the quote key.
    Keys(&'static str),
    /// This many presses of the left arrow.
    Left(usize),
    /// This many presses of the right arrow.
    Right(usize),
    Home,
    End,
    /// The down arrow: in the list of people it picks the next one, and the caret stays.
    Down,
    /// The up arrow: in an empty field it brings the last message back to be edited, the caret at its end.
    Up,
    /// Delete takes the character after the caret.
    Delete,
    /// Backspace takes the character before the caret.
    Erase,
    /// A click that puts the caret before the character at this place.
    Click(usize),
    /// Ctrl+V with this text on the clipboard.
    Paste(&'static str),
    /// The layout picked by hand.
    Pick(i32),
    /// The conversion hotkey.
    Pause,
    /// No key for this many seconds.
    Idle(f64),
    /// The character right before the caret is this one now: `@` opens the list of people, a quote does not.
    Shows(char),
    /// The layout on now is this one.
    Layout(i32),
}
use Step::{Click, Delete, Down, End, Erase, Home, Idle, Keys, Layout, Left, Paste, Pause, Pick, Right, Shows, Up};

/// A scene: its name, the text in the field before it, its steps and the text it ends with.
type Scene = (&'static str, &'static str, Vec<Step>, &'static str);

fn named(session: &Session, name: &str, state: u32) -> KeyEvent {
    KeyEvent { pressed: true, key_name: name.to_owned(), characters: vec![String::new(), String::new()], group: session.backend.group(), state, ..KeyEvent::default() }
}

/// The key that types `english` in the US layout, as it types in the layout that is on.
fn key_event(session: &Session, english: char, state: u32) -> KeyEvent {
    let key = KEYS.iter().find(|key| key.characters[0] == english).expect("a key of the US layout");
    let group = session.backend.group();
    let strings = key.strings();
    KeyEvent {
        pressed: true,
        keycode: key.keycode,
        key_name: english.to_string(),
        character: strings[usize::try_from(group).expect("a layout of the two")].clone(),
        characters: strings,
        group,
        state: state | if key.shift { SHIFT_MASK } else { 0 },
        ..KeyEvent::default()
    }
}

/// A key the replay's field does not move its caret for: the field is changed here as Telegram changes it.
fn tap_then(session: &mut Session, name: &str, change: impl Fn(&mut Vec<char>, &mut usize)) {
    let event = named(session, name, 0);
    session.tap(event);
    let mut text = session.backend.text();
    let mut caret = session.backend.caret();
    change(&mut text, &mut caret);
    session.backend.set_text(text);
    session.backend.set_caret(caret);
}

/// Ctrl+V: the text lands as the V key goes down, before the engine hears of the key.
fn paste(session: &mut Session, text: &str) {
    let control = |session: &Session, pressed: bool| KeyEvent { pressed, ..named(session, "Control_L", CONTROL_MASK) };
    let down = control(session, true);
    session.handle(down);
    session.backend.insert_at_caret(text);
    let v = key_event(session, 'v', CONTROL_MASK);
    session.handle(v.clone());
    session.handle(KeyEvent { pressed: false, ..v });
    let up = control(session, false);
    session.handle(up);
    session.idle(0.1);
}

/// The steps played; what a `Shows` step found otherwise.
fn play(session: &mut Session, steps: &[Step]) -> Vec<String> {
    let mut wrong = Vec::new();
    for (number, step) in steps.iter().enumerate() {
        match *step {
            Keys(text) => {
                for character in text.chars() {
                    let event = key_event(session, character, 0);
                    session.tap(event);
                    session.idle(if character == ' ' { 0.3 } else { 0.1 });
                }
            }
            Left(count) => {
                for _ in 0..count {
                    session.tap_named("Left");
                }
            }
            Right(count) => {
                for _ in 0..count {
                    tap_then(session, "Right", |text, caret| *caret = (*caret + 1).min(text.len()));
                }
            }
            Home => tap_then(session, "Home", |_, caret| *caret = 0),
            End => tap_then(session, "End", |text, caret| *caret = text.len()),
            Down => session.tap_named("Down"),
            Up => session.tap_named("Up"),
            Delete => tap_then(session, "Delete", |text, caret| {
                if *caret < text.len() {
                    text.remove(*caret);
                }
            }),
            Erase => session.tap_named("BackSpace"),
            Click(place) => {
                session.backend.set_caret(place);
                session.pointer();
            }
            Paste(text) => paste(session, text),
            Pick(group) => session.select_layout(group),
            Pause => {
                let event = KeyEvent { keycode: PAUSE_KEYCODE, ..named(session, "Pause", 0) };
                session.tap(event);
            }
            Idle(seconds) => session.idle(seconds),
            Shows(expected) => {
                let caret = session.backend.caret();
                let before = session.backend.text().get(caret.wrapping_sub(1)).copied();
                if before != Some(expected) {
                    let shown: String = session.backend.text().iter().collect();
                    wrong.push(format!("step {number} shows `{shown}`, not `{expected}` before the caret"));
                }
            }
            Layout(expected) => {
                let group = session.backend.group();
                if group != expected {
                    wrong.push(format!("step {number} leaves layout {group}, not {expected}"));
                }
            }
        }
    }
    session.idle(3.0);
    wrong
}

/// Every scene played in Telegram's field, begun with its text and the caret at its end in the Russian layout; what went
/// otherwise than the scene says, scene by scene.
fn failures(bundle: &mut Bundle, readable: bool, scenes: &[Scene]) -> Vec<String> {
    let mut failed = Vec::new();
    for (scene, text, steps, ends) in scenes {
        let mut session = bundle.session("Telegram", RUSSIAN, text.chars().collect());
        session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, readable, false).expect("in memory");
        let mut wrong = play(&mut session, steps);
        let shown: String = session.backend.text().iter().collect();
        if shown != *ends {
            wrong.push(format!("ends as `{shown}`, not `{ends}`"));
        }
        bundle.give_back(session);
        failed.extend(wrong.into_iter().map(|what| format!("{scene} (field read: {readable}): {what}")));
    }
    failed
}

fn assert_none(failed: &[String]) {
    assert!(failed.is_empty(), "{} scenes went otherwise:\n{}", failed.len(), failed.join("\n"));
}

/// A mention: the quote is `@` as its key comes up, and stays while the list of people is used.
#[test]
fn the_quote_key_opens_a_mention_where_a_mention_begins() {
    let mut bundle = Bundle::load(None);
    let scenes: Vec<Scene> = vec![
        ("an empty field", "", vec![Keys("@"), Shows('@'), Down, Idle(1.0)], "@"),
        ("after a word and a space", "привет ", vec![Keys("@"), Shows('@'), Down], "привет @"),
        ("after a space typed after the text", "привет", vec![Keys(" @"), Shows('@'), Down], "привет @"),
        ("a click into an empty field", "", vec![Click(0), Keys("@"), Shows('@'), Down], "@"),
        ("a click at the end after a space", "привет ", vec![Click(7), Keys("@"), Shows('@')], "привет @"),
        // The list opened again: the `@` erased and the key pressed once more.
        ("erased and typed again", "", vec![Keys("@"), Shows('@'), Erase, Keys("@"), Shows('@'), Down], "@"),
        ("erased and typed again after a word", "привет ", vec![Keys("@"), Erase, Keys("@"), Shows('@')], "привет @"),
        // A name typed in the Russian layout and one typed after the English layout was picked.
        ("a name in the wrong layout", "", vec![Keys("@john ")], "@john "),
        ("a name after the layout was picked", "", vec![Keys("@"), Pick(ENGLISH), Keys("john ")], "@john "),
    ];
    let mut failed = failures(&mut bundle, true, &scenes);
    failed.extend(failures(&mut bundle, false, &scenes));
    bundle.clean();
    assert_none(&failed);
}

/// A quotation typed as the text goes keeps its quotes.
#[test]
fn a_quotation_typed_with_the_text_keeps_its_quotes() {
    let mut bundle = Bundle::load(None);
    let scenes: Vec<Scene> = vec![
        ("a quoted word in a sentence", "", vec![Keys("crfpfk @ghbdtn@ b eitk ")], "сказал \"привет\" и ушел "),
        ("a message that begins with a quotation", "", vec![Keys("@ghbdtn@ ")], "\"привет\" "),
        ("a quoted number", "", vec![Keys("d @2024@ ")], "в \"2024\" "),
        ("empty quotes", "", vec![Keys("@@ ")], "\"\" "),
        ("the quote written back by hand, then the word", "", vec![Keys("@"), Pause, Shows('"'), Keys("ghbdtn@ ")], "\"привет\" "),
        ("a closing quote typed after the word", "", vec![Keys("ghbdtn@"), Shows('"')], "привет\""),
    ];
    let mut failed = failures(&mut bundle, true, &scenes);
    failed.extend(failures(&mut bundle, false, &scenes));
    bundle.clean();
    assert_none(&failed);
}

/// Quotes put around text already written: the caret goes there with the keys or the mouse, and the quote is a quote at
/// once, with no `@` shown on the way. The field tells what stands on either side of the caret.
#[test]
fn quotes_put_around_written_text_are_quotes() {
    let mut bundle = Bundle::load(None);
    let mut failed = failures(&mut bundle, true, &[
        ("before the line and after it", "привет мир", vec![Home, Keys("@"), Shows('"'), End, Keys("@"), Shows('"')], "\"привет мир\""),
        ("around the last word, with the arrows", "привет мир", vec![Left(3), Keys("@"), Shows('"'), End, Keys("@"), Shows('"')], "привет \"мир\""),
        ("around the first word, with the arrows", "привет мир", vec![Home, Keys("@"), Right(6), Keys("@"), Shows('"')], "\"привет\" мир"),
        ("after a word already in the field", "привет", vec![Keys("@"), Shows('"')], "привет\""),
        ("closing in a message brought back to be edited", "привет", vec![Up, Keys("@"), Shows('"')], "привет\""),
        ("around a word, with the mouse", "сказал привет и ушел", vec![Click(7), Keys("@"), Shows('"'), Click(14), Keys("@"), Shows('"')], "сказал \"привет\" и ушел"),
        ("after the last word, with a click", "привет", vec![Click(6), Keys("@"), Shows('"')], "привет\""),
        ("after Delete took a character", "привет  мир", vec![Left(4), Delete, Keys("@"), Shows('"')], "привет \"мир"),
        ("a quote typed again over an erased one", "\"привет", vec![Home, Right(1), Erase, Keys("@"), Shows('"')], "\"привет"),
        // A letter erased at the end of the last word, then the quote that closes it.
        ("closing after a letter was erased", "", vec![Keys("ghbdtnn"), Idle(2.0), Erase, Keys("@"), Shows('"')], "привет\""),
        // Signs that open text are text the quote opens too.
        ("before a bracket", "(текст)", vec![Home, Keys("@"), Shows('"')], "\"(текст)"),
        ("before guillemets", "«цитата»", vec![Home, Keys("@"), Shows('"')], "\"«цитата»"),
        // An addressee put before a written line: the quote stays a quote, and Pause makes it the `@` that opens the
        // list of people. Pause converts a sign as it converts a word, so the English layout is on after it.
        ("Pause on a quote before a written line", "привет мир", vec![Home, Keys("@"), Shows('"'), Pause, Shows('@'), Layout(ENGLISH)], "@привет мир"),
    ]);
    // A mention may still begin among written text, with a space or the end of the line after the caret.
    failed.extend(failures(&mut bundle, true, &[
        ("after a space at the end, with the arrows", "привет ", vec![Left(1), Right(1), Keys("@"), Shows('@')], "привет @"),
        ("between two spaces", "привет  мир", vec![Left(4), Keys("@"), Shows('@')], "привет @ мир"),
        ("on a new line", "привет\n", vec![Click(7), Keys("@"), Shows('@')], "привет\n@"),
        ("before a comma", "привет , как дела", vec![Left(10), Keys("@"), Shows('@')], "привет @, как дела"),
    ]));
    bundle.clean();
    assert_none(&failed);
}

/// How a reader of the scene's own sees the field.
#[derive(Clone, Copy)]
enum Sight {
    /// The quote just typed is not in it yet.
    Late,
    /// Neither the quote just typed nor the space typed before it is in it yet.
    LateWithSpace,
    /// From the second read on, the `@` shown for the first quote and erased since stands where the quote is typed again.
    ErasedStillShown,
    /// A password field: nothing of it is the engine's to read.
    Sensitive,
    /// Text is selected in it.
    Selection,
    /// The field of another program.
    Elsewhere,
    /// The reader works but gets nothing from this field.
    Nothing,
}

struct SceneReader {
    backend: Arc<Backend>,
    sight: Sight,
    reads: AtomicUsize,
}

impl FieldReader for SceneReader {
    fn read(&self, application: &str, _window: i64) -> Option<FieldContext> {
        let text = self.backend.text();
        let caret = self.backend.caret();
        let mut field = FieldContext::new(application, "1");
        field.before = text[..caret].iter().collect();
        field.after = text[caret..].iter().collect();
        field.role = FieldRole::Text;
        let read = self.reads.fetch_add(1, Ordering::Relaxed);
        match self.sight {
            Sight::Late => {
                field.before = field.before.trim_end_matches('"').to_owned();
            }
            Sight::LateWithSpace => {
                field.before = field.before.trim_end_matches('"').trim_end_matches(' ').to_owned();
            }
            Sight::ErasedStillShown => {
                if read > 0 {
                    field.before = format!("{}@", field.before.trim_end_matches('"'));
                }
            }
            Sight::Sensitive => field.sensitive = true,
            Sight::Selection => field.selection = true,
            Sight::Elsewhere => "another program".clone_into(&mut field.application),
            Sight::Nothing => return None,
        }
        Some(field)
    }

    /// The window is no password field as it comes to the front: only the field read at the quote says so.
    fn sensitive(&self, _application: &str, _window: i64) -> Option<bool> {
        Some(false)
    }

    fn status(&self) -> String {
        "available".to_owned()
    }
}

fn reader_of(reader: impl FieldReader + 'static) -> Option<Arc<dyn FieldReader>> {
    Some(Arc::new(reader))
}

/// What the steps leave in a field that begins with `text`, the field read by the reader `reader` makes of it (none at
/// all for None).
fn read_by(bundle: &mut Bundle, reader: impl Fn(&Arc<Backend>) -> Option<Arc<dyn FieldReader>>, text: &str, steps: &[Step]) -> String {
    let mut session = bundle.session("Telegram", RUSSIAN, text.chars().collect());
    session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, true, false).expect("in memory");
    session.core.context_policy.reader = reader(&session.backend);
    let wrong = play(&mut session, steps);
    let shown: String = session.backend.text().iter().collect();
    bundle.give_back(session);
    format!("{shown}{}", wrong.join("; "))
}

/// The field is read only where it tells what stands around the caret: `@` where it cannot say, as before, and a quote
/// where it can, though the quote typed is not in it yet. Where the keys know better than a field that lags behind them,
/// the keys say.
#[test]
fn only_a_field_that_tells_keeps_the_quote() {
    let mut bundle = Bundle::load(None);
    let sees = |sight: Sight| move |backend: &Arc<Backend>| reader_of(SceneReader { backend: backend.clone(), sight, reads: AtomicUsize::new(0) });
    let quote = [Keys("@")];
    let mut failed = Vec::new();
    for (name, shown, ends) in [
        ("no reader", read_by(&mut bundle, |_| None, "привет", &quote), "привет@"),
        ("a field no reader reads", read_by(&mut bundle, |_| reader_of(UnreadField), "привет", &quote), "привет@"),
        ("the quote not in the field yet", read_by(&mut bundle, sees(Sight::Late), "привет", &quote), "привет\""),
        ("a password field", read_by(&mut bundle, sees(Sight::Sensitive), "привет", &quote), "привет@"),
        ("a selection", read_by(&mut bundle, sees(Sight::Selection), "привет", &quote), "привет@"),
        ("another program", read_by(&mut bundle, sees(Sight::Elsewhere), "привет", &quote), "привет@"),
        // Read with nothing from it: after a click the keys say, after a caret move the quote is left alone.
        ("nothing read after a click", read_by(&mut bundle, sees(Sight::Nothing), "привет", &[Click(6), Keys("@")]), "привет@"),
        ("nothing read after a caret move", read_by(&mut bundle, sees(Sight::Nothing), "привет", &[Up, Keys("@")]), "привет\""),
        // The space typed before the quote is not in the field yet: the key said it is there.
        ("a typed space not in the field yet", read_by(&mut bundle, sees(Sight::LateWithSpace), "привет", &[Keys(" @"), Shows('@')]), "привет @"),
        // The `@` erased and the quote typed again to open the list again, the field still showing the `@`.
        ("the list opened again over a field behind", read_by(&mut bundle, sees(Sight::ErasedStillShown), "привет ", &[Keys("@"), Erase, Keys("@"), Shows('@')]), "привет @"),
        // A click between the erasure and the quote: the quote is judged where it is now.
        ("a click after the erasure", read_by(&mut bundle, sees(Sight::Late), "привет мир ", &[Keys("@"), Shows('@'), Erase, Click(10), Keys("@"), Shows('"')]), "привет мир\" "),
    ] {
        if shown != ends {
            failed.push(format!("{name}: `{shown}`, not `{ends}`"));
        }
    }
    bundle.clean();
    assert_none(&failed);
}

/// Quotes around a paste. The quote after the pasted text closes it. A paste right after the `@` shown for a quote is
/// taken as it comes: a name pasted there is a mention, and the text after it shows the `@` to the person who typed it,
/// Pause before the paste gives the quote.
#[test]
fn quotes_around_a_paste() {
    let mut bundle = Bundle::load(None);
    let failed = failures(&mut bundle, true, &[
        ("a paste, then the closing quote", "", vec![Keys("@"), Pause, Paste("текст"), Keys("@"), Shows('"')], "\"текст\""),
        ("a name pasted after the shown @", "", vec![Keys("@"), Paste("john"), Keys(" ")], "@john "),
        ("a paste right after the shown @, then a quote", "", vec![Keys("@"), Paste("текст"), Keys("@"), Shows('"')], "@текст\""),
        ("a pasted text, then quotes around it", "", vec![Paste("текст"), Home, Keys("@"), Shows('"'), End, Keys("@"), Shows('"')], "\"текст\""),
    ]);
    bundle.clean();
    assert_none(&failed);
}

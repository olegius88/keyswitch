//! Tokens of two or three Russian letters whose Latin reading is a common word, typed through the engine: the class of
//! the common-reading head (context_action_features.common_question). A Russian token after a Russian word stays (`сайт
//! майл ру`, `у нас ща`); an English word typed in the Russian layout after English text, or first on a line under a
//! Russian line (`He said`), is converted. The gate words of the owner's typing are counted apart, a token outside the
//! class (`гы`, too rare in Russian text) and words of the lexicon (`шт`) only measured. Half of each family is held out of
//! the head's training frames by hash of the token's Cyrillic letters (the recipe's common_curriculum.held_out) and
//! counted apart from the other half. The numbers are printed by group; the floors hold for a pair that carries the head,
//! and a pair without it is only measured.
#![cfg(any(not(debug_assertions), coverage))]

use std::collections::BTreeMap;

use ks_backend::KeyEvent;
use ks_backend::constants::SHIFT_MASK;
use ks_config::constants::{DETECTION_CONTEXT_READ_FIELD, DETECTION_EARLY_SWITCH, DETECTION_RESPECT_MANUAL_LAYOUT};
use ks_context::constants::COMMON_FEATURE_PREFIX;
use ks_replay::harness::{Bundle, KEYS, PhysicalKey, Session};
use sha2::{Digest, Sha256};

/// One step of a case: the keys that print a text in a layout, typed in whatever layout is on; or the layout changed
/// under the person, no switch by hand (a slip).
#[derive(Clone)]
enum Step {
    Keys(String, usize),
    Slip(i32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Want {
    Convert,
    Keep,
}

struct Case {
    family: &'static str,
    frame: &'static str,
    /// The token's Cyrillic letters, casefolded: what the held-out half is drawn by.
    cyrillic: String,
    /// The text in the field before the first key, and the layout on.
    field: String,
    start: i32,
    steps: Vec<Step>,
    converted: String,
    kept: String,
    want: Want,
}

fn key_for(character: char, group: usize) -> &'static PhysicalKey {
    KEYS.iter().find(|key| key.characters[group] == character).unwrap_or_else(|| panic!("no key for {character:?} in layout {group}"))
}

/// The other reading of a text: the same keys in the other layout.
fn keys_of(text: &str, group: usize) -> String {
    text.chars().map(|character| key_for(character, group).characters[1 - group]).collect()
}

fn press(session: &mut Session, key: &PhysicalKey) {
    let group = session.backend.group();
    let strings = key.strings();
    session.tap(KeyEvent {
        pressed: true,
        keycode: key.keycode,
        key_name: key.characters[0].to_string(),
        character: strings[usize::try_from(group).expect("a layout")].clone(),
        characters: strings,
        group,
        state: if key.shift { SHIFT_MASK } else { 0 },
        ..KeyEvent::default()
    });
}

/// The field's text once a case is typed, in Firefox, with the field read or not.
fn typed(bundle: &mut Bundle, case: &Case, field: bool) -> String {
    let mut session = bundle.session("firefox", case.start, case.field.chars().collect());
    session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, field, false).expect("in memory");
    for step in &case.steps {
        match step {
            Step::Slip(group) => {
                // A layout the person did not pick protects nothing, as the capture's slip mode types it.
                session.core.settings.write(DETECTION_RESPECT_MANUAL_LAYOUT, false, false).expect("in memory");
                session.core.settings.write(DETECTION_EARLY_SWITCH, false, false).expect("in memory");
                session.select_layout(*group);
            }
            Step::Keys(text, group) => {
                for character in text.chars() {
                    press(&mut session, key_for(character, *group));
                    session.idle(if character == ' ' { 0.35 } else { 0.12 });
                }
            }
        }
    }
    session.idle(3.0);
    let shown: String = session.backend.text().iter().collect();
    bundle.give_back(session);
    shown
}

fn keys(text: &str, group: usize) -> Step {
    Step::Keys(text.to_owned(), group)
}

/// Russian tokens of the class typed as meant after a Russian word, with the words around them.
const RUSSIAN: &[(&str, &str, &str)] = &[
    ("ру", "сайт майл", "работает"),
    ("ща", "у нас", "дождь"),
    ("вк", "написал в", "вчера"),
    ("тз", "по", "всё готово"),
    ("ип", "оформил", "вчера"),
    ("тс", "написал", "в теме"),
    ("ид", "твой", "в базе"),
    ("тп", "позвонил в", "банка"),
    ("ии", "спросил у", "сегодня"),
    ("вс", "в", "встретимся"),
    ("зп", "пришла", "наконец"),
    ("ср", "до", "успею"),
    ("ес", "страны", "договорились"),
    ("ой", "ну", "всё"),
];

/// English words typed in the Russian layout after English text once the layout slipped, with the words around them.
const ENGLISH: &[(&str, &str, &str)] = &[
    ("he", "and then", "left"),
    ("of", "the end", "the line"),
    ("by", "sort the list", "name"),
    ("if", "let me know", "it works"),
    ("in", "it is", "the box"),
    ("it", "please fix", "today"),
    ("is", "this", "fine"),
    ("to", "we need", "go"),
    ("on", "it runs", "linux"),
    ("at", "see you", "noon"),
    ("us", "call", "later"),
    ("or", "tea", "coffee"),
    ("no", "there is", "way"),
    ("my", "this is", "code"),
    ("so", "it works", "far"),
    ("do", "what", "you"),
    ("we", "then", "can"),
    ("for", "thanks", "the help"),
    ("the", "open", "file"),
    ("are", "they", "here"),
    ("can", "you", "see"),
    ("not", "it is", "done"),
];

/// English sentences typed in the Russian layout first on a line under a Russian line: the opening word and the next.
const OPENINGS: &[(&str, &str)] = &[
    ("He", "said"),
    ("We", "can"),
    ("If", "you"),
    ("It", "is"),
    ("In", "fact"),
    ("Of", "course"),
    ("By", "the"),
    ("So", "what"),
    ("Do", "you"),
    ("My", "name"),
    ("No", "way"),
    ("Is", "it"),
    ("At", "home"),
    ("Or", "not"),
];

/// The Russian line above an opening.
const LINE_ABOVE: &str = "Вот что он написал вчера.\n";

/// Tokens of the class the owner's typing converts after Russian words, at a pause or a space (the field gate): they
/// stay converted.
const GATE: &[(&str, &str)] = &[("ин", "проверь сортировку"), ("сс", "запусти")];

/// Words of the Russian lexicon of the same shape, typed as meant after a Russian word: the head leaves them to the frozen
/// model.
const LEXICON: &[(&str, &str, &str)] = &[("шт", "купил пять", "яблок"), ("мы", "а", "пойдём"), ("он", "сказал что", "придёт")];

/// A Russian token too rare in Russian text for the class.
const OUTSIDE: &[(&str, &str, &str)] = &[("гы", "он такой", "смешной")];

/// Whether the head's training frames hold a token out: half of each family, by hash of its Cyrillic letters.
fn held_out(cyrillic: &str) -> bool {
    Sha256::digest(format!("common-held-out:{cyrillic}").as_bytes())[0] % 2 == 0
}

/// A Russian token typed as meant after a Russian word: at its space, before a Russian word, at a pause.
fn russian_cases(family: &'static str, token: &str, before: &str, after: &str, want: Want, cases: &mut Vec<Case>) {
    let other = keys_of(token, 1);
    let mut add = |frame: &'static str, text: String, other_text: String| {
        let (kept, converted) = (text.clone(), other_text);
        cases.push(Case { family, frame, cyrillic: token.to_owned(), field: String::new(), start: 1, steps: vec![keys(&text, 1)], converted, kept, want });
    };
    add("after-ru", format!("{before} {token} "), format!("{before} {other} "));
    add("inside-ru", format!("{before} {token} {after} "), format!("{before} {other} {after} "));
    add("at-pause", format!("{before} {token}"), format!("{before} {other}"));
}

/// An English word typed in the Russian layout once the layout slipped after English text: at its space, and before an
/// English word typed in the same layout.
fn english_cases(token: &str, before: &str, after: &str, cases: &mut Vec<Case>) {
    let cyrillic = keys_of(token, 0);
    let mut add = |frame: &'static str, typed: String| {
        let converted = format!("{before} {typed}");
        let kept = format!("{before} {}", keys_of(&typed, 0));
        let steps = vec![keys(&format!("{before} "), 0), Step::Slip(1), keys(&typed, 0)];
        cases.push(Case { family: "english-midline", frame, cyrillic: cyrillic.clone(), field: String::new(), start: 0, steps, converted, kept, want: Want::Convert });
    };
    add("after-en-slip", format!("{token} "));
    add("inside-en-slip", format!("{token} {after} "));
}

/// An English sentence typed in the Russian layout first on a line under a Russian line: its first two words, and its
/// first word alone left at a pause.
fn opening_cases(token: &str, next: &str, cases: &mut Vec<Case>) {
    let cyrillic = keys_of(&token.to_lowercase(), 0);
    let mut add = |frame: &'static str, typed: String| {
        let converted = format!("{LINE_ABOVE}{typed}");
        let kept = format!("{LINE_ABOVE}{}", keys_of(&typed, 0));
        cases.push(Case { family: "english-line-start", frame, cyrillic: cyrillic.clone(), field: LINE_ABOVE.to_owned(), start: 1, steps: vec![keys(&typed, 0)], converted, kept, want: Want::Convert });
    };
    add("line-start", format!("{token} {next} "));
    add("line-start-pause", token.to_owned());
}

/// A gate word typed in the Russian layout after Russian words, meant as its Latin reading: at a pause and at its space.
fn gate_cases(token: &str, before: &str, cases: &mut Vec<Case>) {
    let latin = keys_of(token, 1);
    for (frame, typed, latin_typed) in [("at-pause", token.to_owned(), latin.clone()), ("after-ru", format!("{token} "), format!("{latin} "))] {
        let converted = format!("{before} {latin_typed}");
        let kept = format!("{before} {typed}");
        cases.push(Case { family: "gate", frame, cyrillic: token.to_owned(), field: String::new(), start: 1, steps: vec![keys(&format!("{before} {typed}"), 1)], converted, kept, want: Want::Convert });
    }
}

fn all_cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for (token, before, after) in RUSSIAN {
        russian_cases("russian-after-word", token, before, after, Want::Keep, &mut cases);
    }
    for (token, before, after) in ENGLISH {
        english_cases(token, before, after, &mut cases);
    }
    for (token, next) in OPENINGS {
        opening_cases(token, next, &mut cases);
    }
    for (token, before) in GATE {
        gate_cases(token, before, &mut cases);
    }
    for (token, before, after) in LEXICON {
        russian_cases("lexicon-word", token, before, after, Want::Keep, &mut cases);
    }
    for (token, before, after) in OUTSIDE {
        russian_cases("outside-class", token, before, after, Want::Keep, &mut cases);
    }
    cases
}

/// The least right answers of each family and half (every frame and both field settings pooled) a pair carrying the head
/// gives: the numbers of the development candidate context-v3-8b77864d52b9 on corpus v86 (15.11.2026). The pair of 0.71.2
/// gave, in the same order: 21, 26, 44, 44, 2, 4, 14, 0, 38, 22.
const FLOORS: &[(&str, usize)] = &[
    ("english-line-start | held-out", 21),
    ("english-line-start | seen", 26),
    ("english-midline | held-out", 44),
    ("english-midline | seen", 44),
    ("gate | held-out", 2),
    ("gate | seen", 4),
    ("lexicon-word | seen", 14),
    ("outside-class | seen", 0),
    ("russian-after-word | held-out", 40),
    ("russian-after-word | seen", 24),
];

#[derive(Default)]
struct Tally {
    total: usize,
    right: usize,
    converted: usize,
    kept: usize,
}

#[test]
fn short_tokens_with_common_latin_readings() {
    let mut bundle = Bundle::load(None);
    let probe = bundle.session("firefox", 0, Vec::new());
    let answers = probe.core.context_policy.model.as_ref().is_some_and(|model| model.weights.names().any(|name| name.starts_with(COMMON_FEATURE_PREFIX)));
    let version = probe.core.context_policy.model.as_ref().map_or_else(String::new, |model| model.version.clone());
    bundle.give_back(probe);
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut families: BTreeMap<String, Tally> = BTreeMap::new();
    let mut misses: Vec<String> = Vec::new();
    for case in all_cases() {
        let half = if held_out(&case.cyrillic) { "held-out" } else { "seen" };
        for field in [false, true] {
            let shown = typed(&mut bundle, &case, field);
            let converted = shown == case.converted;
            let kept = shown == case.kept;
            let right = match case.want {
                Want::Convert => converted,
                Want::Keep => kept,
            };
            for tally in [tallies.entry(format!("{} | {} | {half}", case.family, case.frame)).or_default(), families.entry(format!("{} | {half}", case.family)).or_default()] {
                tally.total += 1;
                tally.right += usize::from(right);
                tally.converted += usize::from(converted);
                tally.kept += usize::from(kept);
            }
            if !right {
                misses.push(format!("MISS {} | {} | {half} | field={field} | {} | want {:?} | {:?}", case.family, case.frame, case.cyrillic, case.want, shown));
            }
        }
    }
    bundle.clean();
    println!("pair {version}, common-reading head: {answers}");
    for miss in &misses {
        println!("{miss}");
    }
    for (group, tally) in &tallies {
        println!("TALLY {group} | right {}/{} | converted {} | kept {}", tally.right, tally.total, tally.converted, tally.kept);
    }
    let mut below = Vec::new();
    for (family, tally) in &families {
        println!("FAMILY {family} | right {}/{}", tally.right, tally.total);
        let floor = FLOORS.iter().find(|entry| entry.0 == family).map_or(0, |entry| entry.1);
        if answers && tally.right < floor {
            below.push(format!("{family}: {} < {floor}", tally.right));
        }
    }
    assert!(below.is_empty(), "below the floors of a pair with the common-reading head:\n{}", below.join("\n"));
}

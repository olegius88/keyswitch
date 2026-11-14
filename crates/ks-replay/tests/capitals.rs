//! Words in capitals typed in either layout, alone and in Russian prose, through the engine: abbreviations, currency codes
//! and words typed with Caps Lock, the class of the upper-case head (context_action_features.upper_question). Each case says
//! by its meaning whether the text should be converted, kept, or either is right (both readings are abbreviations in use:
//! `РК` and `HR`, `США` and `CIF`). Half of each family is held out of the head's training frames by hash (the recipe's
//! upper_curriculum.held_out) and counted apart from the other half. The numbers are printed by group; the floors hold
//! for a pair that carries the head, and a pair without it is only measured.
#![cfg(any(not(debug_assertions), coverage))]

use std::collections::BTreeMap;

use ks_backend::KeyEvent;
use ks_backend::constants::SHIFT_MASK;
use ks_config::constants::{DETECTION_CONTEXT_READ_FIELD, DETECTION_EARLY_SWITCH, DETECTION_RESPECT_MANUAL_LAYOUT};
use ks_context::constants::UPPER_FEATURE_PREFIX;
use ks_replay::harness::{Bundle, KEYS, PhysicalKey, Session};
use sha2::{Digest, Sha256};

/// One step of a case: the keys that print a text in a layout, typed in whatever layout is on; a layout picked by hand;
/// or the layout changed under the person, no switch by hand (a slip).
#[derive(Clone)]
enum Step {
    Keys(String, i32),
    Pick(i32),
    Slip(i32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Want {
    Convert,
    Keep,
    Either,
}

struct Case {
    family: &'static str,
    frame: &'static str,
    token: String,
    /// The text in the field before the first key, and the layout on.
    field: &'static str,
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
            Step::Pick(group) => session.select_layout(*group),
            Step::Slip(group) => {
                // A layout the person did not pick protects nothing, as the capture's slip mode types it.
                session.core.settings.write(DETECTION_RESPECT_MANUAL_LAYOUT, false, false).expect("in memory");
                session.core.settings.write(DETECTION_EARLY_SWITCH, false, false).expect("in memory");
                session.select_layout(*group);
            }
            Step::Keys(text, group) => {
                for character in text.chars() {
                    press(&mut session, key_for(character, usize::try_from(*group).expect("a layout")));
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

fn keys(text: &str, group: i32) -> Step {
    Step::Keys(text.to_owned(), group)
}

/// Latin abbreviations, currency codes and words of technical text.
const LATIN: &[&str] = &[
    "KZT", "RUB", "USD", "EUR", "GBP", "CNY", "UAH", "BYN", "API", "URL", "HTTP", "HTTPS", "JSON", "SQL", "CPU", "GPU", "RAM", "SSD", "USB",
    "PDF", "HTML", "CSS", "DNS", "VPN", "SSH", "TCP", "UDP", "OK", "FAQ", "CEO", "HR", "PR", "QA", "UI", "UX", "IT", "AI", "ML", "NASA", "BMW",
    "IBM", "AMD", "XML", "YAML", "CLI", "SDK", "IDE", "CI", "LTS", "RFC", "ASAP", "FYI", "KPI", "CRM", "ERP", "SEO", "SMM", "UTM", "GTK", "XKB",
    "PHP", "AWS", "NPM", "WIFI", "TODO", "MVP", "DEV", "PROD", "ACK", "EOF",
];

/// Currency codes of ISO 4217 that Russian text rarely spells.
const CURRENCIES: &[&str] = &["KGS", "UZS", "TJS", "AZN", "GEL", "MDL", "PLN", "CHF", "JPY", "INR", "KRW", "THB", "VND", "AED", "SAR", "ILS", "HKD", "SGD", "SEK", "CZK", "HUF", "XCD"];

/// Russian abbreviations whose every letter is on a letter key.
const CYRILLIC: &[&str] = &[
    "СССР", "ВЛКСМ", "КПСС", "МВД", "РФ", "США", "НДС", "ООО", "ИП", "ДТП", "ТЗ", "СМИ", "ВУЗ", "МГУ", "ИНН", "СНИЛС", "ЗОР", "ВДВ", "ЧП", "МЧС",
    "ФНС", "НАТО", "ООН", "ЕС", "ЗАГС", "ГОСТ", "ЦСКА", "МСК", "РАН", "ВВП", "ЦРУ", "ФИО", "ПК", "ПО", "ОС", "ТВ", "НИИ", "АО", "ПАО", "ЗП", "ДР",
    "ОК", "ЛС", "ТП", "ЦА", "МФЦ", "ОСАГО", "ЕГРН", "ЧСВ", "ПКМ",
];

/// Real abbreviations that no source of the capitals table spells: no count, no list.
const LATIN_UNCOUNTED: &[&str] = &["IIRC", "LGTM", "PTAL", "EOD", "AFK", "NSFW", "OTOH", "NACK", "TBA", "ROFL", "LMAO", "FTW", "IDK", "TTYL"];
const CYRILLIC_UNCOUNTED: &[&str] = &["КТРУ", "ЛКК", "СЗВ", "ПМСМ"];

/// Russian abbreviations with a letter on the key of a sign in the English layout (`Б`, `Ж`, `Х`, `Э`, `Ю`): the boundary
/// model cuts the token at the sign, a question of the next cycle; counted on a line of their own.
const SIGN_KEYS: &[&str] = &["ФСБ", "ГИБДД", "ЖКХ", "ЕГЭ", "ОАЭ", "ЦБ", "РЖД", "СПБ", "БД", "ЖД", "КБ"];

/// Latin words in capitals longer than the head's class.
const LONG: &[&str] = &["NVIDIA", "WINDOWS", "README"];

/// Russian and English words typed with Caps Lock.
const CAPS_LOCK_RUSSIAN: &[&str] = &["НЕ", "НЕТ", "ВСЕ", "ДА", "ЧТО", "КАК", "ТАК", "ИЛИ", "НО", "ДЛЯ", "ЕЩЕ", "ОН", "МЫ", "ТЫ", "ГДЕ"];
const CAPS_LOCK_ENGLISH: &[&str] = &["YES", "NO", "THE", "AND", "STOP", "HELP", "WAIT", "NOT", "DONE", "WARN"];

/// Typed on purpose in the English layout after Russian text.
const ON_PURPOSE: &[&str] = &["OK", "NO", "TODO"];

/// Tokens both of whose readings are abbreviations in use: either answer is right.
const BOTH_LIVE: &[&str] = &["HR", "PR", "UI", "CI", "RUB", "RFC", "США", "ДТП", "ОК", "ЗП", "ЧСВ", "XCD"];

/// Whether the head's training frames hold a token out: half of each family, by hash.
fn held_out(token: &str) -> bool {
    Sha256::digest(format!("upper-held-out:{token}").as_bytes())[0] % 2 == 0
}

fn want(token: &str, wrong_layout: bool) -> Want {
    if BOTH_LIVE.contains(&token) {
        Want::Either
    } else if wrong_layout {
        Want::Convert
    } else {
        Want::Keep
    }
}

/// The cases of a Latin token: typed in the Russian layout (alone, before and after Russian words, inside a phrase whose
/// next word must not take the conversion back) and as meant (alone, after a layout picked by hand, after Russian text
/// with the English layout already on and the field read, inside a phrase).
fn latin_cases(family: &'static str, token: &str, cases: &mut Vec<Case>) {
    let other = keys_of(token, 0);
    let wrong = want(token, true);
    let meant = want(token, false);
    let mut add = |frame: &'static str, field: &'static str, start: i32, steps: Vec<Step>, converted: String, kept: String, want: Want| {
        cases.push(Case { family, frame, token: token.to_owned(), field, start, steps, converted, kept, want });
    };
    add("in-ru alone", "", 1, vec![keys(&format!("{token} "), 0)], format!("{token} "), format!("{other} "), wrong);
    add("in-ru then-ru", "", 1, vec![keys(&format!("{token} "), 0), keys("курс ", 1)], format!("{token} курс "), format!("{other} курс "), wrong);
    add("in-ru after-ru", "", 1, vec![keys("курс ", 1), keys(&format!("{token} "), 0)], format!("курс {token} "), format!("курс {other} "), wrong);
    add(
        "in-ru inside-ru", "", 1,
        vec![keys("цена в ", 1), keys(&format!("{token} "), 0), keys("за штуку ", 1)],
        format!("цена в {token} за штуку "), format!("цена в {other} за штуку "), wrong,
    );
    add("kept alone", "", 0, vec![keys(&format!("{token} "), 0)], format!("{other} "), format!("{token} "), meant);
    add("kept after-ru-pick", "", 1, vec![keys("курс ", 1), Step::Pick(0), keys(&format!("{token} "), 0)], format!("курс {other} "), format!("курс {token} "), meant);
    add("kept after-ru-on", "курс ", 0, vec![keys(&format!("{token} "), 0)], format!("курс {other} "), format!("курс {token} "), meant);
    add(
        "kept inside-ru", "", 1,
        vec![keys("курс ", 1), Step::Pick(0), keys(&format!("{token} "), 0), Step::Pick(1), keys("вырос ", 1)],
        format!("курс {other} вырос "), format!("курс {token} вырос "), meant,
    );
}

/// The cases of a Russian token: typed in the English layout (alone, before a Russian word, after Russian words with the
/// layout slipped) and as meant (alone, after a Russian word, kept before a Russian word).
fn cyrillic_cases(family: &'static str, token: &str, cases: &mut Vec<Case>) {
    let other = keys_of(token, 1);
    let wrong = want(token, true);
    let meant = want(token, false);
    let mut add = |frame: &'static str, start: i32, steps: Vec<Step>, converted: String, kept: String, want: Want| {
        cases.push(Case { family, frame, token: token.to_owned(), field: "", start, steps, converted, kept, want });
    };
    add("in-en alone", 0, vec![keys(&format!("{token} "), 1)], format!("{token} "), format!("{other} "), wrong);
    add("in-en then-ru", 0, vec![keys(&format!("{token} "), 1), keys("говорит ", 1)], format!("{token} говорит "), format!("{other} говорит "), wrong);
    add("in-en after-ru-slip", 1, vec![keys("в ", 1), Step::Slip(0), keys(&format!("{token} "), 1)], format!("в {token} "), format!("в {other} "), wrong);
    add("kept alone", 1, vec![keys(&format!("{token} "), 1)], format!("{other} "), format!("{token} "), meant);
    add("kept after-ru", 1, vec![keys("в ", 1), keys(&format!("{token} "), 1)], format!("в {other} "), format!("в {token} "), meant);
    add("kept then-ru", 1, vec![keys(&format!("{token} "), 1), keys("говорит ", 1)], format!("{other} говорит "), format!("{token} говорит "), meant);
}

/// A word typed with Caps Lock: Russian in the English layout and English in the Russian one, alone and after Russian
/// words with the layout slipped, and as meant.
fn caps_lock_cases(token: &str, cases: &mut Vec<Case>) {
    let russian = !token.is_ascii();
    let group = i32::from(russian);
    let other = keys_of(token, usize::from(russian));
    let mut add = |frame: &'static str, start: i32, steps: Vec<Step>, converted: String, kept: String, want: Want| {
        cases.push(Case { family: if russian { "capslock-russian" } else { "capslock-english" }, frame, token: token.to_owned(), field: "", start, steps, converted, kept, want });
    };
    add("in-other alone", 1 - group, vec![keys(&format!("{token} "), group)], format!("{token} "), format!("{other} "), Want::Convert);
    add("kept alone", group, vec![keys(&format!("{token} "), group)], format!("{other} "), format!("{token} "), Want::Keep);
    if russian {
        add("in-other after-ru-slip", 1, vec![keys("я ", 1), Step::Slip(0), keys(&format!("{token} "), 1)], format!("я {token} "), format!("я {other} "), Want::Convert);
        add("kept after-ru", 1, vec![keys("я ", 1), keys(&format!("{token} "), 1)], format!("я {other} "), format!("я {token} "), Want::Keep);
    }
}

fn all_cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for token in LATIN {
        latin_cases("latin", token, &mut cases);
    }
    for token in CURRENCIES {
        latin_cases("currency", token, &mut cases);
    }
    for token in LATIN_UNCOUNTED {
        latin_cases("latin-uncounted", token, &mut cases);
    }
    for token in LONG {
        latin_cases("latin-long", token, &mut cases);
    }
    for token in CYRILLIC {
        cyrillic_cases("cyrillic", token, &mut cases);
    }
    for token in CYRILLIC_UNCOUNTED {
        cyrillic_cases("cyrillic-uncounted", token, &mut cases);
    }
    for token in SIGN_KEYS {
        cyrillic_cases("cyrillic-sign-keys", token, &mut cases);
    }
    for token in CAPS_LOCK_RUSSIAN.iter().chain(CAPS_LOCK_ENGLISH) {
        caps_lock_cases(token, &mut cases);
    }
    for token in ON_PURPOSE {
        let mut purpose = Vec::new();
        latin_cases("on-purpose", token, &mut purpose);
        cases.extend(purpose.into_iter().filter(|case| case.frame.starts_with("kept")));
    }
    cases
}

/// The least right answers of each family and half (every frame and both field settings pooled) a pair carrying the head
/// gives: the numbers of the development candidate context-v3-a4d2e926e7d0 on corpus v86 (10.11.2026). The pair of 0.71.2
/// gave, in the same order: 23, 16, 56, 62, 116, 127, 264, 208, 51, 30, 14, 8, 505, 535, 48, 66, 106, 8, 16. Words whose
/// letters are on the keys of signs (`cyrillic-sign-keys`) are cut by the boundary model before any question: their floor
/// only keeps them where they are.
const FLOORS: &[(&str, usize)] = &[
    ("capslock-english | held-out", 22),
    ("capslock-english | seen", 16),
    ("capslock-russian | held-out", 46),
    ("capslock-russian | seen", 64),
    ("currency | held-out", 151),
    ("currency | seen", 168),
    ("cyrillic | held-out", 313),
    ("cyrillic | seen", 253),
    ("cyrillic-sign-keys | held-out", 51),
    ("cyrillic-sign-keys | seen", 30),
    ("cyrillic-uncounted | held-out", 14),
    ("cyrillic-uncounted | seen", 8),
    ("latin | held-out", 523),
    ("latin | seen", 566),
    ("latin-long | held-out", 48),
    ("latin-uncounted | held-out", 66),
    ("latin-uncounted | seen", 109),
    ("on-purpose | held-out", 8),
    ("on-purpose | seen", 16),
];

#[derive(Default)]
struct Tally {
    total: usize,
    right: usize,
    converted: usize,
    kept: usize,
}

#[test]
fn words_in_capitals() {
    let mut bundle = Bundle::load(None);
    let probe = bundle.session("firefox", 0, Vec::new());
    let answers = probe.core.context_policy.model.as_ref().is_some_and(|model| model.weights.names().any(|name| name.starts_with(UPPER_FEATURE_PREFIX)));
    let version = probe.core.context_policy.model.as_ref().map_or_else(String::new, |model| model.version.clone());
    bundle.give_back(probe);
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut families: BTreeMap<String, Tally> = BTreeMap::new();
    let mut misses: Vec<String> = Vec::new();
    for case in all_cases() {
        let half = if held_out(&case.token) { "held-out" } else { "seen" };
        for field in [false, true] {
            let shown = typed(&mut bundle, &case, field);
            let converted = shown == case.converted;
            let kept = shown == case.kept;
            let right = match case.want {
                Want::Convert => converted,
                Want::Keep => kept,
                Want::Either => converted || kept,
            };
            for tally in [tallies.entry(format!("{} | {} | {half}", case.family, case.frame)).or_default(), families.entry(format!("{} | {half}", case.family)).or_default()] {
                tally.total += 1;
                tally.right += usize::from(right);
                tally.converted += usize::from(converted);
                tally.kept += usize::from(kept);
            }
            if !right {
                misses.push(format!("MISS {} | {} | {half} | field={field} | {} | want {:?} | {:?}", case.family, case.frame, case.token, case.want, shown));
            }
        }
    }
    bundle.clean();
    println!("pair {version}, upper-case head: {answers}");
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
    assert!(below.is_empty(), "below the floors of a pair with the upper-case head:\n{}", below.join("\n"));
}

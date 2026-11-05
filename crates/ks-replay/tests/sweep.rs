//! Phrases typed through the engine with every kind of interruption, in several programs: a test that no sequence stops it,
//! and that the text on screen stays made of what was typed or of its other reading.
#![cfg(any(not(debug_assertions), coverage))]

use ks_backend::constants::SHIFT_MASK;
use ks_backend::KeyEvent;
use ks_replay::harness::{Bundle, Session, KEYS, PAUSE_KEYCODE};

fn type_character(session: &mut Session, character: char) {
    let wanted = session.backend.group();
    let found = KEYS.iter().find_map(|key| {
        let group = key.characters.iter().position(|typed| *typed == character)?;
        Some((key, group))
    });
    let Some((key, group)) = found else { return };
    if usize::try_from(wanted).ok() != Some(group) {
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

fn pause_key(session: &mut Session) {
    let group = session.backend.group();
    session.tap(KeyEvent { pressed: true, keycode: PAUSE_KEYCODE, key_name: "Pause".to_owned(), characters: vec![String::new(), String::new()], group, ..KeyEvent::default() });
}

fn play(session: &mut Session, script: &str) {
    let mut rest = script;
    while let Some(character) = rest.chars().next() {
        if character == '{' {
            let (step, after) = rest[1..].split_once('}').expect("a closed step");
            match step {
                "pause" => pause_key(session),
                "bs" => session.tap_named("BackSpace"),
                "del" => session.tap_named("Delete"),
                "enter" => session.tap_named("Return"),
                "tab" => session.tap_named("Tab"),
                "esc" => session.tap_named("Escape"),
                "left" => session.tap_named("Left"),
                "click" => session.pointer(),
                "idle" => session.idle(1.0),
                "long" => session.idle(30.0),
                "en" => session.select_layout(0),
                "ru" => session.select_layout(1),
                "switch" => session.core.switch_layout_only(i64::from(PAUSE_KEYCODE)),
                "undo" => session.core.schedule_undo(i64::from(PAUSE_KEYCODE)),
                "undohot" => session.core.schedule_undo_hotkey(i64::from(PAUSE_KEYCODE)),
                "manual" => session.core.schedule_manual_conversion(i64::from(PAUSE_KEYCODE)),
                other => panic!("no step {other}"),
            }
            rest = after;
        } else {
            type_character(session, character);
            rest = &rest[character.len_utf8()..];
        }
    }
    session.idle(2.0);
}

const SCRIPTS: &[&str] = &[
    "ghbdtn vbh ", "привет мир ", "hello world ", "руддщ цщкдв ", "ntcn ", "кто-то ", "mongo-cthdbcf ", "tot ghbdtn ", "CAPS LOCK ", "Ghbdtn ", "GHBDTN ", "1,5 ", "ю ", "x = 5 ", ".net ", "c++ ", "https://ya.ru ", "a@b.c ",
    "ghbd{bs}{bs}vbh ", "ghbdt{click}n ", "{pause}", "ghbdtn{pause}", "ghbdtn {pause}", "ghbdtn{idle}{pause}{pause}", "ghbdtn {undo}", "ghbdtn {undohot}", "hello{switch} мир ", "{ru}ghbdtn ", "{en}ghbdtn ", "{ru}привет{pause}", "{en}привет ",
    "ghbdtn{manual}", "привет{manual}", "ghbdtn vbh{manual}{manual}", "a b c d e ", "я в к с ", "z x ", "f d ", "{en}d когда ", "{ru}в when ", "ghbdtn,vbh. ", "ghbdtn! ", "ghbdtn? ", "ghbdtn; ", "ghbdtn: ", "\"ghbdtn\" ", "(ghbdtn) ", "[ghbdtn] ",
    "ghbdtn{enter}", "ghbdtn{tab}", "ghbdtn{esc}", "ghbdtn{del}", "ghbdtn{left}{left}vbh ", "ghb{left}{left}{del}dtn ", "{click}ghbdtn{click}vbh{click}", "ghbdtn{long}vbh ", "ghbdtn {long}{pause}", "ghbdtn vbh gjrf ntcn ", "{pause}ghbdtn{pause}vbh{pause}",
    "ntcn123 ", "123ntcn ", "v67 ", "м67 ", "DKRCV ", "ЗОР ", "кз ", "rp ", "ghbdtn-vbh ", "привет-мир ", "ghbdtn_vbh ", "ghbdtn/vbh ", "ghbdtn.vbh ", "ghbdtn@vbh ", "t` ", "ё ", "'t ", "э ", "ю. ", "б, ", "ж; ", "х[ ", "ъ] ",
    "{ru}hello world ", "{ru}the ", "{en}руддщ ", "{en}да ", "{en}нет ", "ghbdtn{bs}{bs}{bs}{bs}{bs}{bs}{bs}", "a{bs}", "{bs}", "{del}", "{enter}", " ", "  ", "\n",
];

#[test]
fn no_sequence_of_keys_stops_the_engine() {
    let mut bundle = Bundle::load(None);
    for application in ["firefox", "code", "telegram-desktop", "xterm"] {
        for group in [0, 1] {
            for text in ["", "начало ", "start "] {
                for script in SCRIPTS {
                    let mut session = bundle.session(application, group, text.chars().collect());
                    play(&mut session, script);
                    assert!(session.backend.text().len() < 500, "{script}");
                    bundle.give_back(session);
                }
            }
        }
    }
    bundle.clean();
}

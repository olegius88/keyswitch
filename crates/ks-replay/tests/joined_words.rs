//! A hyphen joins two words, and the part after it is decided as a word of its own, behind the head as typed: with the
//! field read as the program reads it, and right after a layout picked by hand. The part before it is decided alone
//! when the part after it is a word as typed.
#![cfg(any(not(debug_assertions), coverage))]

use ks_backend::KeyEvent;
use ks_backend::constants::SHIFT_MASK;
use ks_config::config::SettingsStore;
use ks_config::constants::{DETECTION_CONTEXT_READ_FIELD, DETECTION_CORRECT_ON_SPACE, EXCLUSIONS_APPLICATIONS};
use ks_replay::harness::{Bundle, KEYS, PAUSE_KEYCODE, Session};

/// The key that types `english` in the US layout, pressed in whatever layout is on.
fn press(session: &mut Session, english: char) {
    let key = KEYS.iter().find(|key| key.characters[0] == english).expect("a key of the US layout");
    let group = session.backend.group();
    let strings = key.strings();
    session.tap(KeyEvent {
        pressed: true,
        keycode: key.keycode,
        key_name: english.to_string(),
        character: strings[usize::try_from(group).expect("a layout of the two")].clone(),
        characters: strings,
        group,
        state: if key.shift { SHIFT_MASK } else { 0 },
        ..KeyEvent::default()
    });
}

fn keys(session: &mut Session, english: &str) {
    for character in english.chars() {
        press(session, character);
    }
}

/// What stands in the field after `english` was typed from an empty field in the layout `group`; `picked`, if any,
/// is a layout picked by hand after the first word and what was typed in it.
fn typed(bundle: &mut Bundle, group: i32, english: &str, picked: Option<(i32, &str)>, configure: fn(&SettingsStore)) -> String {
    let mut session = bundle.session("firefox", group, Vec::new());
    session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, true, false).expect("in memory");
    configure(&session.core.settings);
    keys(&mut session, english);
    if let Some((layout, rest)) = picked {
        session.select_layout(layout);
        keys(&mut session, rest);
    }
    let shown = session.backend.text().iter().collect();
    bundle.give_back(session);
    shown
}

fn as_shipped(_: &SettingsStore) {}

#[test]
fn the_part_after_a_hyphen_converts_alone_while_the_field_shows_the_whole_token() {
    let mut bundle = Bundle::load(None);
    // The model, asked about the head alone, keeps it; read again, the field showed the whole token and the head had no
    // answer, so the whole token converted (`зкщчн-приложение`).
    assert_eq!(typed(&mut bundle, 0, "proxy-ghbkj;tybt ", None, as_shipped), "proxy-приложение ");
    assert_eq!(typed(&mut bundle, 0, "docker-j,hfp ", None, as_shipped), "docker-образ ");
    // Kept whole before the parts were asked apart, and other signs that join parts, one or several in a row.
    assert_eq!(typed(&mut bundle, 0, "mongo-cthdbcf ", None, as_shipped), "mongo-сервиса ");
    assert_eq!(typed(&mut bundle, 0, "lid-ht;bvt ", None, as_shipped), "lid-режиме ");
    assert_eq!(typed(&mut bundle, 0, "redis+rkfcnth ", None, as_shipped), "redis+кластер ");
    assert_eq!(typed(&mut bundle, 0, "redis--rkfcnth ", None, as_shipped), "redis--кластер ");
    // Both parts in the wrong layout: the head converts too, and so does the whole token.
    assert_eq!(typed(&mut bundle, 0, "ghjrcb-ghbkj;tybt ", None, as_shipped), "прокси-приложение ");
    // Words of one language joined by a hyphen stay whole.
    assert_eq!(typed(&mut bundle, 0, "xnj-nj ", None, as_shipped), "что-то ");
    assert_eq!(typed(&mut bundle, 0, "built-in ", None, as_shipped), "built-in ");
    bundle.clean();
}

#[test]
fn a_term_typed_all_in_the_russian_layout_converts_whole_though_the_model_keeps_its_head_alone() {
    let mut bundle = Bundle::load(None);
    // The model converts `црз-куппук` whole and, asked about `црз` alone, keeps it: the head typed in the Russian layout
    // does not override the whole token (`црз-regger` in the owner's typing), as `мгу` for `vue` does not.
    assert_eq!(typed(&mut bundle, 1, "whp-regger ", None, as_shipped), "whp-regger ");
    assert_eq!(typed(&mut bundle, 1, "vue-email ", None, as_shipped), "vue-email ");
    bundle.clean();
}

#[test]
fn a_layout_picked_by_hand_keeps_the_word_up_to_the_hyphen() {
    let mut bundle = Bundle::load(None);
    // Russian, then English picked for a term and the Russian word after its hyphen typed on in English.
    assert_eq!(typed(&mut bundle, 1, "yjdsq ", Some((0, "frontend-vfufpby ")), as_shipped), "новый frontend-магазин ");
    // A token of English parts in the layout picked for it stays as typed, and so does a protected word with no hyphen.
    assert_eq!(typed(&mut bundle, 1, "yjdsq ", Some((0, "built-in ")), as_shipped), "новый built-in ");
    assert_eq!(typed(&mut bundle, 1, "yjdsq ", Some((0, "frontend ")), as_shipped), "новый frontend ");
    // Nothing is decided where nothing would be: a boundary that corrects nothing, an excluded program.
    let no_space = |settings: &SettingsStore| settings.write(DETECTION_CORRECT_ON_SPACE, false, false).expect("in memory");
    assert_eq!(typed(&mut bundle, 1, "yjdsq ", Some((0, "frontend-vfufpby ")), no_space), "новый frontend-vfufpby ");
    let excluded = |settings: &SettingsStore| settings.write(EXCLUSIONS_APPLICATIONS, vec!["firefox".to_owned()], false).expect("in memory");
    assert_eq!(typed(&mut bundle, 1, "yjdsq ", Some((0, "frontend-vfufpby ")), excluded), "новый frontend-vfufpby ");
    // A path keeps its literal head, and a sign split off the end of the word keeps the token as typed.
    assert_eq!(typed(&mut bundle, 1, "yjdsq ", Some((0, "docs/vfufpby ")), as_shipped), "новый docs/vfufpby ");
    assert_eq!(typed(&mut bundle, 1, "yjdsq ", Some((0, "hello. ")), as_shipped), "новый hello. ");
    bundle.clean();
}

#[test]
fn a_term_typed_in_the_russian_layout_before_a_russian_word_converts_alone() {
    let mut bundle = Bundle::load(None);
    // `зкщ-тарифу` is no word whole and neither is `pro-nfhbae`, so the model kept the token; asked alone before its sign,
    // `зкщ` is `pro`. The sign and the word after it are typed again as they were, and the Russian layout stays.
    let mut session = bundle.session("firefox", 1, Vec::new());
    session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, true, false).expect("in memory");
    keys(&mut session, "ljcnegty njkmrj pro-nfhbae ");
    session.idle(1.0);
    assert_eq!(session.backend.text().iter().collect::<String>(), "доступен только pro-тарифу ");
    assert_eq!(session.backend.group(), 1, "the layout the word after the sign was typed in");
    // The undo hotkey puts the token back as typed.
    session.core.schedule_undo_hotkey(i64::from(PAUSE_KEYCODE));
    session.handle(KeyEvent { pressed: false, keycode: PAUSE_KEYCODE, key_name: "Pause".to_owned(), ..KeyEvent::default() });
    session.idle(1.0);
    assert_eq!(session.backend.text().iter().collect::<String>(), "доступен только зкщ-тарифу ");
    bundle.give_back(session);
    // A term the lexicons do not know, which the model knows; one the model was unsure of whole, which waited for the next
    // word and stayed (`ьщтпщ-сервиса`), and the word after it in the layout that stayed.
    assert_eq!(typed(&mut bundle, 1, "ye;yj seo-jgnbvbpfwbz ", None, as_shipped), "нужно seo-оптимизация ");
    assert_eq!(typed(&mut bundle, 1, "mongo-cthdbcf ", None, as_shipped), "mongo-сервиса ");
    assert_eq!(typed(&mut bundle, 1, "mongo-cthdbcf ghjdthm ", None, as_shipped), "mongo-сервиса проверь ");
    // Russian words joined by a hyphen stay, and so does a part whose other reading is no letters: `экс` is `'rc`.
    assert_eq!(typed(&mut bundle, 1, "xnj-nj ", None, as_shipped), "что-то ");
    assert_eq!(typed(&mut bundle, 1, "ye;yj 'rc-xtvgbjy ", None, as_shipped), "нужно экс-чемпион ");
    // Heads the model is asked about and keeps: a word no lexicon knows (`псевдо`), and a word of both lexicons (`руку`,
    // `here`).
    assert_eq!(typed(&mut bundle, 1, "gctdlj-yfexysq ", None, as_shipped), "псевдо-научный ");
    assert_eq!(typed(&mut bundle, 1, "here-yjue ", None, as_shipped), "руку-ногу ");
    // Two signs: the parts stay whole.
    assert_eq!(typed(&mut bundle, 1, "pro-nfhbae-rkbtyn ", None, as_shipped), "зкщ-тарифу-клиент ");
    bundle.clean();
}

#[test]
fn undo_after_an_early_switch_inside_a_joined_token_puts_back_every_key_as_typed() {
    let mut bundle = Bundle::load(None);
    // `ьщтпщ-сервиса` typed in the Russian layout: the early switch makes `ьщтп` `mong` while it is typed, the rest comes in
    // English, and the part after the hyphen converts back at the space.
    let mut session = bundle.session("firefox", 1, Vec::new());
    session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, true, false).expect("in memory");
    keys(&mut session, "ljcnegty njkmrj mongo-cthdbcf ");
    session.idle(1.0);
    assert_eq!(session.backend.text().iter().collect::<String>(), "доступен только mongo-сервиса ");
    assert_eq!(session.backend.group(), 1, "the layout the word after the sign was typed in");
    // Undo takes back the whole token, the early switch's part too, as after the early switch of a word of one part.
    session.core.schedule_undo_hotkey(i64::from(PAUSE_KEYCODE));
    session.handle(KeyEvent { pressed: false, keycode: PAUSE_KEYCODE, key_name: "Pause".to_owned(), ..KeyEvent::default() });
    session.idle(1.0);
    assert_eq!(session.backend.text().iter().collect::<String>(), "доступен только ьщтпщ-сервиса ");
    bundle.give_back(session);
    // A sign split off the end goes with the rest, and so does a boundary both layouts type alike; one the converted part
    // takes as another sign (`?` is `,`) leaves the part corrected alone.
    for (typed, shown, undone) in [
        ("ljcnegty njkmrj mongo-cthdbcf/ ", "доступен только mongo-сервиса. ", Some("доступен только ьщтпщ-сервиса. ")),
        ("ljcnegty njkmrj mongo-cthdbcf! ", "доступен только mongo-сервиса! ", Some("доступен только ьщтпщ-сервиса! ")),
        ("ljcnegty njkmrj mongo-cthdbcf?", "доступен только mongo-сервиса,", None),
    ] {
        let mut session = bundle.session("firefox", 1, Vec::new());
        session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, true, false).expect("in memory");
        keys(&mut session, typed);
        session.idle(1.0);
        assert_eq!(session.backend.text().iter().collect::<String>(), shown);
        if let Some(undone) = undone {
            session.core.schedule_undo_hotkey(i64::from(PAUSE_KEYCODE));
            session.handle(KeyEvent { pressed: false, keycode: PAUSE_KEYCODE, key_name: "Pause".to_owned(), ..KeyEvent::default() });
            session.idle(1.0);
            assert_eq!(session.backend.text().iter().collect::<String>(), undone, "{typed}");
        }
        bundle.give_back(session);
    }
    bundle.clean();
}

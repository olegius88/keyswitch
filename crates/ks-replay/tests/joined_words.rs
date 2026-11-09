//! A hyphen joins two words, and the part after it is decided as a word of its own, behind the head as typed: with the
//! field read as the program reads it, and right after a layout picked by hand.
#![cfg(any(not(debug_assertions), coverage))]

use ks_backend::KeyEvent;
use ks_backend::constants::SHIFT_MASK;
use ks_config::config::SettingsStore;
use ks_config::constants::{DETECTION_CONTEXT_READ_FIELD, DETECTION_CORRECT_ON_SPACE, EXCLUSIONS_APPLICATIONS};
use ks_replay::harness::{Bundle, KEYS, Session};

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

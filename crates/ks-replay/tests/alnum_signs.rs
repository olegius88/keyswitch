//! A Latin token of letters and digits typed in the Russian layout with the comma or full stop after it typed there too
//! (`шзм6б` for `ipv6,`), before the rest of a Russian sentence: it converts with its sign, and the sentence goes on in
//! Russian. A Russian token of a digit and a letter (`5б`) stays.
#![cfg(any(not(debug_assertions), coverage))]

use ks_backend::KeyEvent;
use ks_backend::constants::SHIFT_MASK;
use ks_config::constants::DETECTION_CONTEXT_READ_FIELD;
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

/// What stands in the field after the keys of `english` were pressed from an empty field in the Russian layout.
fn typed(bundle: &mut Bundle, english: &str) -> String {
    let mut session = bundle.session("firefox", 1, Vec::new());
    session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, true, false).expect("in memory");
    for character in english.chars() {
        press(&mut session, character);
    }
    let shown = session.backend.text().iter().collect();
    bundle.give_back(session);
    shown
}

#[test]
fn a_token_of_letters_and_digits_converts_with_the_sign_after_it() {
    let mut bundle = Bundle::load(None);
    // `сервер включает ipv6, то`, every key pressed in the Russian layout.
    assert_eq!(typed(&mut bundle, "cthdth drk.xftn ipv6, nj "), "сервер включает ipv6, то ");
    assert_eq!(typed(&mut bundle, "vjltkm x64, gjnjv "), "модель x64, потом ");
    // A class at school, `в 5б классе`: the letter stays a letter.
    assert_eq!(typed(&mut bundle, "d 5, rkfcct "), "в 5б классе ");
}

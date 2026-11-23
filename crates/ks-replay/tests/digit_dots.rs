//! A version, an address or a numbered point typed in Russian text with the dot key of the English layout, which prints
//! `ю` in the Russian one (`1ю5ю3` for `1.5.3`): each sign becomes a dot, the digits stay, and the Russian layout stays on
//! for the words after it. Dots typed with the Russian layout's own key, a letter after digits (`3б`) and two signs side by
//! side stay as typed.
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

/// What stands in the field of `application` after the keys of `english` were pressed from an empty field in the
/// Russian layout, and the layout on at the end.
fn typed(bundle: &mut Bundle, application: &str, english: &str) -> (String, i32) {
    let mut session = bundle.session(application, 1, Vec::new());
    session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, true, false).expect("in memory");
    for character in english.chars() {
        press(&mut session, character);
    }
    let shown = (session.backend.text().iter().collect(), session.backend.group());
    bundle.give_back(session);
    shown
}

#[test]
fn every_sign_of_a_version_an_address_or_a_point_becomes_a_dot() {
    let mut bundle = Bundle::load(None);
    for application in ["firefox", "code"] {
        // `версии 1.5.3`, `обновил до 1.5.3 и всё`, every key pressed in the Russian layout.
        assert_eq!(typed(&mut bundle, application, "dthcbb 1.5.3 "), ("версии 1.5.3 ".to_owned(), 1), "{application}");
        assert_eq!(typed(&mut bundle, application, "j,yjdbk lj 1.5.3 b dc` "), ("обновил до 1.5.3 и всё ".to_owned(), 1), "{application}");
        assert_eq!(typed(&mut bundle, application, "1.5.3 "), ("1.5.3 ".to_owned(), 1), "{application}");
        // An address, alone and before the rest of the sentence.
        assert_eq!(typed(&mut bundle, application, "flhtc 192.168.1.1 "), ("адрес 192.168.1.1 ".to_owned(), 1), "{application}");
        assert_eq!(typed(&mut bundle, application, "gbyueq 10.0.0.1 gjnjv "), ("пингуй 10.0.0.1 потом ".to_owned(), 1), "{application}");
        // A numbered point, its last sign with no digit after it.
        assert_eq!(typed(&mut bundle, application, "geyrn 1.2. "), ("пункт 1.2. ".to_owned(), 1), "{application}");
        assert_eq!(typed(&mut bundle, application, "hfpltk 2.3. nfv "), ("раздел 2.3. там ".to_owned(), 1), "{application}");
        assert_eq!(typed(&mut bundle, application, "c,jhrf 1.2.3.4 ujnjdf "), ("сборка 1.2.3.4 готова ".to_owned(), 1), "{application}");
    }
}

#[test]
fn dots_of_the_russian_layout_a_letter_after_digits_and_signs_side_by_side_stay() {
    let mut bundle = Bundle::load(None);
    // `встреча 12.05.2024 в 12.30` with the dot key of the Russian layout.
    assert_eq!(typed(&mut bundle, "firefox", "dcnhtxf 12/05/2024 d 12/30 "), ("встреча 12.05.2024 в 12.30 ".to_owned(), 1));
    assert_eq!(typed(&mut bundle, "firefox", "flhtc 192/168/1/1 "), ("адрес 192.168.1.1 ".to_owned(), 1));
    // A house and a class: the letter stays a letter.
    assert_eq!(typed(&mut bundle, "firefox", "ljv 3, "), ("дом 3б ".to_owned(), 1));
    // Two signs side by side are no number.
    assert_eq!(typed(&mut bundle, "firefox", "1..2 "), ("1юю2 ".to_owned(), 1));
}

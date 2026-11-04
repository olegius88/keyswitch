//! Dumps what the keyboard map of the running X server answers, for the harness to compare with libX11's answers.
//! Runs only when `KS_LIVE_KEYMAP_DUMP` names the file to write.

use std::fmt::Write;

use ks_linux::x11_native;

#[test]
fn the_keyboard_map_of_the_server_is_dumped() {
    let Some(path) = std::env::var_os("KS_LIVE_KEYMAP_DUMP") else {
        eprintln!("KS_LIVE_KEYMAP_DUMP is not set: no live server to ask, skipped");
        return;
    };
    let api = x11_native::connect().expect("the display opens");
    let mut text = String::new();
    for keycode in 8..=255u8 {
        for group in 0..4u8 {
            for level in 0..2u8 {
                writeln!(text, "k {keycode} {group} {level} {}", api.keysym(keycode, group, level)).expect("write");
            }
        }
        for state in [0u16, 1, 2, 3, 4, 1 << 13, (1 << 13) | 1, (1 << 13) | 2, (1 << 13) | 3, 2 << 13] {
            writeln!(text, "l {keycode} {state} {}", api.lookup_keysym(keycode, state).unwrap_or(0)).expect("write");
        }
    }
    for keysym in [0xff08u32, 0xffe1, 0xff51, 0xffe3, 0x61, 0x20, 0x6c6] {
        writeln!(text, "c {keysym} {}", api.keycode_for_keysym(keysym)).expect("write");
    }
    std::fs::write(path, text).expect("the dump is written");
}

#[test]
fn the_characters_of_a_key_in_both_groups() {
    if std::env::var_os("KS_LIVE_KEYMAP_DUMP").is_none() {
        return;
    }
    let backend = ks_linux::x11_backend::X11Backend::new(Box::new(x11_native::connect), None);
    backend.open().expect("the display opens");
    // Keycode 42 is `g` on a PC keyboard: `п` in the second group.
    let characters: Vec<String> = (0..2).map(|group| backend.character_for_keycode(42, group, 0)).collect();
    println!("CHARACTERS {characters:?}");
    assert_eq!(characters, ["g", "п"]);
}

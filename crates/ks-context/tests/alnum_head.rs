//! The letters-and-digits head's class with a comma or a full stop at the end of the Latin reading: `шзм6б`, typed for
//! `ipv6,` before the rest of a Russian sentence, is asked and read as `шзм6` for `ipv6`.

use ks_context::context_action_features::{alnum_question, without_trailing_sign};
use ks_context::context_model::TermTables;
use std::path::PathBuf;

fn tables() -> TermTables {
    TermTables::load(&PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/models"))).expect("the shipped tables")
}

#[test]
fn a_comma_or_a_full_stop_after_a_token_of_letters_and_digits_is_set_aside() {
    assert_eq!(without_trailing_sign("шзм6б", "ipv6,"), ("шзм6", "ipv6"));
    assert_eq!(without_trailing_sign("шзм6ю", "ipv6."), ("шзм6", "ipv6"));
    // No sign at the end, another sign, a sign typed as itself, nothing at all: as they are.
    for (original, alternative) in [("шзм6", "ipv6"), ("шзм6х", "ipv6["), ("12.", "12,"), ("", "")] {
        assert_eq!(without_trailing_sign(original, alternative), (original, alternative));
    }
    let before = "сервер работает с разными адресами, включая ";
    assert_eq!(alnum_question("шзм6б", "ipv6,", before, &tables()), Ok(true));
    assert_eq!(alnum_question("ч64б", "x64,", before, &tables()), Ok(true));
    // Digits alone once the sign is set aside are no member: `5б` names a class at school.
    assert_eq!(alnum_question("5б", "5,", "он учится в ", &tables()), Ok(false));
}

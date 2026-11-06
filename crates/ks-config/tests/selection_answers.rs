//! The selection commands against what the Python version answered for the same text.

use ks_config::selection_commands::{KeyChoice, RefusalReason, SelectionCommand, plan_selection, refusal_message, typing_runs};
use ks_config::selection_text::{convert_layout, cycle_case, transliterate};
use std::collections::HashMap;

#[test]
fn mixed_text_is_retyped_by_its_minority() {
    let answers = [
        ("привет vbh", "us", "привет мир", "us", "ru"),
        ("привет vbh", "ru", "привет мир", "us", "ru"),
        ("hello руддщ", "us", "руддщ руддщ", "us", "ru"),
        ("hello руддщ", "ru", "hello hello", "ru", "us"),
        ("привет, vbh!", "us", "привет, мир!", "us", "ru"),
        ("(hello) руддщ?", "us", "(руддщ) руддщ,", "us", "ru"),
        ("(hello) руддщ?", "ru", "(hello) hello&", "ru", "us"),
        ("a б", "us", "ф б", "us", "ru"),
        ("a б", "ru", "a ,", "ru", "us"),
        ("ab б", "us", "ab ,", "ru", "us"),
        ("a бв", "us", "ф бв", "us", "ru"),
        ("vbh, привет", "ru", "мир, привет", "us", "ru"),
        ("ghbdtn", "ru", "привет", "us", "ru"),
        ("руддщ", "us", "hello", "ru", "us"),
        (" ", "us", " ", "us", "ru"),
        (" ", "ru", " ", "ru", "us"),
        ("1", "ru", "1", "ru", "us"),
        ("?/", "us", ",.", "us", "ru"),
        ("?/", "ru", "&|", "ru", "us"),
        ("привет v,h", "us", "привет мбр", "us", "ru"),
        ("hello р.л", "ru", "hello h/k", "ru", "us"),
    ];
    for (text, current, replaced, source, target) in answers {
        let conversion = convert_layout(text, current);
        assert_eq!((conversion.text.as_str(), conversion.source, conversion.target), (replaced, source, target), "{text:?} in {current}");
    }
}

#[test]
fn transliteration_reads_the_places_of_letters() {
    let answers = [
        ("tsk", "тск"), ("Tsk", "Тск"), ("ye", "е"), ("Ye", "Е"), ("yeli", "ели"), ("e ye", "е е"), ("mai", "май"), ("maik", "майк"),
        ("maia", "мая"), ("mai ", "май "), ("krasnii", "краснии"), ("krasnyi", "красный"), ("sinii", "синии"), ("i", "и"), ("ii", "ии"),
        ("ai", "ай"), ("oi", "ой"), ("ei", "ей"), ("ui", "уй"), ("syi", "сый"), ("pi", "пи"), ("vi", "ви"), ("l'v", "льв"), ("L’vov", "Львов"),
        ("'a", "'а"), ("a'", "а'"), ("a' b", "а' б"), ("İa", "İа"), ("rai", "рай"), ("mo'i", "моьи"), ("Привет, John", "Privet, John"),
        ("John, Привет", "John, Privet"), ("а b", "a b"), ("1", "1"), ("", ""), ("nochi", "ночи"), ("дом", "dom"), ("Ж", "Zh"),
        ("щука", "shchuka"), ("ъ", "ie"), ("ь", ""), ("Ё", "E"), ("hello я", "хелло я"), ("ab б", "аб б"), ("я ab", "я аб"),
    ];
    for (text, answer) in answers {
        assert_eq!(transliterate(text), answer, "{text:?}");
    }
}

#[test]
fn the_case_goes_round() {
    let answers = [
        ("hELLO wORLD", "Hello World"), ("hELLO", "Hello"), ("HELLO", "Hello"), ("hello", "HELLO"), ("Hello", "hello"), ("", ""), ("a", "A"),
        ("A", "a"), ("hi THERE", "hi there"), ("hELLo", "hello"), ("h", "H"), ("aBC DEF", "abc def"),
    ];
    for (text, answer) in answers {
        assert_eq!(cycle_case(text), answer, "{text:?}");
    }
}

#[test]
fn every_refusal_says_something_of_its_own() {
    use RefusalReason::{ClipboardBusy, ClipboardUnrestorable, Disabled, EmptyResult, ExcludedApplication, FocusChanged, LineBreaks, MultipleSelection, NoCaret, NoLayout, NoSelection, NotSettable, OwnWindow, ReadOnly, ReleaseTimeout, SensitiveField, Terminal, TooLong, Unsupported, Untypeable};
    let reasons = [
        Disabled, OwnWindow, ExcludedApplication, SensitiveField, Unsupported, NoSelection, MultipleSelection, ReadOnly, TooLong, LineBreaks,
        Untypeable, Terminal, ClipboardBusy, ClipboardUnrestorable, NotSettable, FocusChanged, ReleaseTimeout, NoLayout, EmptyResult, NoCaret,
    ];
    let mut messages: Vec<String> = reasons.iter().map(|reason| refusal_message(*reason)).collect();
    messages.sort();
    messages.dedup();
    assert_eq!(messages.len(), reasons.len());
    assert!(messages.iter().all(|message| !message.is_empty()));
}

#[test]
fn a_group_out_of_the_list_is_no_group() {
    let names = ["us".to_owned(), "ru".to_owned()];
    for group in [-1, 2, 9] {
        let plan = plan_selection(SelectionCommand::Layout, "ghbdtn", group, &names).expect("the text is typeable");
        assert_eq!(plan.replacement, "привет");
    }
    let table: HashMap<char, KeyChoice> = HashMap::from([('a', (38, false))]);
    for start in [-1, 1, 5] {
        let runs = typing_runs("aa", std::slice::from_ref(&table), start).expect("the group has it");
        assert_eq!(runs, [(0, vec![(38, false), (38, false)])]);
    }
}

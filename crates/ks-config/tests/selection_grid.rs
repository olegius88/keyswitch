//! The selection commands over every shape of text a user can select: a word, a phrase, either layout, signs, nothing.
use ks_config::selection_commands::{SelectionCommand, caret_steps, checked_selection, command_of_hotkey, joined_category, plan_selection};
use ks_config::selection_text::{convert_layout, cycle_case, transliterate};

const TEXTS: &[&str] = &[
    "", " ", "\n", "\t", "a", "я", "A", "Я", "hello", "HELLO", "Hello", "hELLO", "hello world", "Hello World", "HELLO WORLD", "привет", "ПРИВЕТ", "Привет", "Привет мир", "ghbdtn", "руддщ", "ёжик", "Ё", "ё", "щи", "Щи", "ЩИ", "ъ", "Ъ", "ь", "Ь", "дом ЁЖ", "a1b2", "123", "...",
    "Test-Case", "схема", "ЮЛЯ", "Юля", "ШЩ", "Цх", "йцукен", "qwerty", "qwertyuiop[]asdfghjkl;'zxcvbnm,./", "QWERTYUIOP{}ASDFGHJKL:\"ZXCVBNM<>?", "`~!@#$%^&*()_+", "ё!\"№;%:?*()_+", "йцукенгшщзхъфывапролджэячсмитьбю.", "ЙЦУКЕНГШЩЗХЪФЫВАПРОЛДЖЭЯЧСМИТЬБЮ,", "mixed смесь", "смесь mixed", "aя", "яa", "😀", "ß", "Σς", "ǅ", "ﬁ", "tschkh", "shch", "yo yu ya", "ts", "kh", "zh", "ch", "sh", "e'", "ye", "Yo", "YO", "Zh", "ZH", "хх", "Ых", "ый", "ий", "ье", "ъе", "тс", "кх", "\u{301}", "e\u{301}", "\u{200b}", "a\u{0}b",
];

#[test]
fn every_text_has_its_case_its_transliteration_and_its_other_layout() {
    for text in TEXTS {
        let mut seen = cycle_case(text);
        for _ in 0..4 {
            seen = cycle_case(&seen);
        }
        assert_eq!(cycle_case("").len(), 0);
        let _ = (seen, transliterate(text));
        for layout in ["us", "ru", "de", ""] {
            let conversion = convert_layout(text, layout);
            let _ = format!("{conversion:?}");
        }
    }
    assert_eq!(transliterate("Привет"), "Privet");
    assert_eq!(transliterate("hello"), "хелло");
}

#[test]
fn every_text_can_be_planned_or_refused() {
    let names = ["us".to_owned(), "ru".to_owned()];
    assert!(checked_selection(&"x".repeat(100_000)).is_err());
    for name in ["convert_selection", "case_selection", "transliterate_selection", "nothing", "toggle", "convert_last", "undo"] {
        let _ = command_of_hotkey(name);
    }
    for text in TEXTS {
        let _ = (checked_selection(text).is_ok(), caret_steps(text));
        let _ = text.chars().map(joined_category).filter(|joined| *joined).count();
        for command in [SelectionCommand::Layout, SelectionCommand::Case, SelectionCommand::Transliterate] {
            for group in [0, 1, 5] {
                let _ = format!("{:?}", plan_selection(command, text, group, &names));
            }
            assert!(!command.as_str().is_empty() && !command.title().is_empty());
        }
    }
}

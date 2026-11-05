//! The selection hotkeys: what they ask of a platform, and what every platform shares.
//!
//! A selection hotkey reads the text selected in the focused field, changes it (`selection_text`) and types the result
//! over it, as typing over a selection replaces it in any editor. The engine decides whether a command may run at all
//! (`KeySwitch` on, not its own window, not an excluded program or a password field) and when (every key of the hotkey
//! released, as for Pause). A backend that offers the two calls below reads and replaces; one that does not is reported
//! as unable to, and nothing is typed.
//!
//! `SelectedText`, `SelectionRefused`, `RefusalReason` and the `SOURCE_*` names are those of `ks-backend`, which the
//! backend trait speaks in (`supports_selection`, `read_selection`, `replace_selection`); they are re-exported here.
//! Python's `SelectionBackend` protocol is that part of the backend trait, so there is no second trait for it.

use ks_cast::To as _;
use crate::constants::{
    BASIC_MULTILINGUAL_PLANE_MAX_CODEPOINT, HOTKEY_SELECTION_CASE, HOTKEY_SELECTION_LAYOUT,
    HOTKEY_SELECTION_TRANSLITERATE, SELECTION_COMMAND_MAX_CHARACTERS,
};
use crate::selection_text::{convert_layout, cycle_case, transliterate};
use ks_text::russian_text::{quantity, CHARACTERS};
use std::collections::HashMap;

pub use ks_backend::selected_text::{SOURCE_ATSPI, SOURCE_CLIPBOARD, SOURCE_UIA};
pub use ks_backend::{RefusalReason, SelectedText, SelectionRefused};

/// What the status line says when a selection hotkey changes nothing; the reason itself goes into the log.
#[must_use]
pub fn refusal_message(reason: RefusalReason) -> String {
    use RefusalReason::{Disabled, OwnWindow, ExcludedApplication, SensitiveField, Unsupported, NoSelection, MultipleSelection, ReadOnly, TooLong, LineBreaks, Untypeable, Terminal, ClipboardBusy, ClipboardUnrestorable, NotSettable, FocusChanged, ReleaseTimeout, NoLayout, EmptyResult, NoCaret};
    match reason {
        Disabled => "Выделение не изменено: KeySwitch на паузе".to_string(),
        OwnWindow => "Выделение в окне KeySwitch не меняется".to_string(),
        ExcludedApplication => "Выделение не изменено: программа в исключениях".to_string(),
        SensitiveField => "Выделение в поле пароля не меняется".to_string(),
        Unsupported => "Выделение не прочитано: программа не сообщает выделенный текст".to_string(),
        NoSelection => "Нет выделенного текста".to_string(),
        MultipleSelection => "Несколько выделений сразу не меняются".to_string(),
        ReadOnly => "Выделенный текст нельзя изменить".to_string(),
        TooLong => format!(
            "Выделение не изменено: предел — {}",
            quantity(SELECTION_COMMAND_MAX_CHARACTERS.to::<f64>(), CHARACTERS)
        ),
        LineBreaks => "Выделение из нескольких строк не меняется: перевод строки набирался бы как Enter".to_string(),
        Untypeable => "Выделение не изменено: в нём есть знаки, которых нет на раскладках EN и RU".to_string(),
        Terminal => "В терминале выделение не заменяется".to_string(),
        ClipboardBusy => "Выделение не прочитано: буфер обмена занят другой программой".to_string(),
        ClipboardUnrestorable => "Выделение не прочитано: буфер обмена хранит данные, которые нельзя вернуть".to_string(),
        NotSettable => "Поле не даёт заменить выделение".to_string(),
        FocusChanged => "Выделение не изменено: окно сменилось".to_string(),
        ReleaseTimeout => "Выделение не изменено: не получено отпускание клавиш".to_string(),
        NoLayout => "Выделение не изменено: раскладка EN/RU не найдена".to_string(),
        EmptyResult => "Выделение не изменено: от него ничего не осталось бы".to_string(),
        NoCaret => "Выделение не прочитано: в окне нет поля ввода с курсором".to_string(),
    }
}

/// The command a selection hotkey runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionCommand {
    Layout,
    Case,
    Transliterate,
}

impl SelectionCommand {
    /// The name the log and the settings give it: "layout", "case", "transliterate".
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            SelectionCommand::Layout => "layout",
            SelectionCommand::Case => "case",
            SelectionCommand::Transliterate => "transliterate",
        }
    }

    /// `COMMAND_TITLES`: what the status line calls each command once it has changed the selection.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            SelectionCommand::Layout => "Раскладка выделения изменена",
            SelectionCommand::Case => "Регистр выделения изменён",
            SelectionCommand::Transliterate => "Выделение транслитерировано",
        }
    }
}

/// COMMANDS: the command of each selection hotkey, by its name under `hotkeys`.
pub const COMMANDS: [(&str, SelectionCommand); 3] = [
    (HOTKEY_SELECTION_LAYOUT, SelectionCommand::Layout),
    (HOTKEY_SELECTION_CASE, SelectionCommand::Case),
    (HOTKEY_SELECTION_TRANSLITERATE, SelectionCommand::Transliterate),
];

/// `COMMANDS.get(name)`.
#[must_use]
pub fn command_of_hotkey(name: &str) -> Option<SelectionCommand> {
    COMMANDS.iter().find(|(hotkey, _)| *hotkey == name).map(|(_, command)| *command)
}

/// The characters a selection may hold besides printable ones: none. A line break or a tab typed over a selection is
/// Enter or Tab to the program, which sends a chat message or leaves the field.
const LINE_BREAKS: [char; 8] = ['\r', '\n', '\t', '\u{b}', '\u{c}', '\u{85}', '\u{2028}', '\u{2029}'];

/// A selection hotkey pressed and waiting for its keys to come up; given up at `deadline`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingSelection {
    pub command: SelectionCommand,
    pub deadline: f64,
}

/// What a command makes of the selection, and the layout the keyboard is left in (None: where it was).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionPlan {
    pub replacement: String,
    pub source_group: Option<i64>,
    pub target_group: Option<i64>,
}

impl SelectionPlan {
    #[must_use]
    pub fn changes(&self, text: &str) -> bool {
        self.replacement != text
    }
}

/// The selected text, or `SelectionRefused` for a selection the hotkeys do not change.
pub fn checked_selection(text: &str) -> Result<&str, SelectionRefused> {
    if text.is_empty() {
        return Err(SelectionRefused::new(RefusalReason::NoSelection));
    }
    if text.chars().count() > SELECTION_COMMAND_MAX_CHARACTERS {
        return Err(SelectionRefused::new(RefusalReason::TooLong));
    }
    if text.chars().any(|character| LINE_BREAKS.contains(&character)) {
        return Err(SelectionRefused::new(RefusalReason::LineBreaks));
    }
    Ok(text)
}

/// What `command` makes of `text`. Only the layout command moves the keyboard: to the layout it converted to.
pub fn plan_selection(
    command: SelectionCommand,
    text: &str,
    current_group: i64,
    layout_names: &[String],
) -> Result<SelectionPlan, SelectionRefused> {
    match command {
        SelectionCommand::Case => Ok(SelectionPlan { replacement: cycle_case(text), source_group: None, target_group: None }),
        SelectionCommand::Transliterate => {
            // Only a selection of soft signs comes out empty (ICAO leaves Ь out); typing nothing would replace nothing.
            let replacement = transliterate(text);
            if replacement.is_empty() {
                return Err(SelectionRefused::new(RefusalReason::EmptyResult));
            }
            Ok(SelectionPlan { replacement, source_group: None, target_group: None })
        }
        SelectionCommand::Layout => {
            let current = if 0 <= current_group && current_group.to::<usize>() < layout_names.len() {
                layout_names[current_group.to::<usize>()].as_str()
            } else {
                ""
            };
            let conversion = convert_layout(text, current);
            let index_of = |name: &str| layout_names.iter().position(|item| item == name);
            match (index_of(conversion.source), index_of(conversion.target)) {
                (Some(source), Some(target)) => Ok(SelectionPlan {
                    replacement: conversion.text,
                    source_group: Some(source.to::<i64>()),
                    target_group: Some(target.to::<i64>()),
                }),
                _ => Err(SelectionRefused::new(RefusalReason::NoLayout)),
            }
        }
    }
}

/// A key that types a character in a layout group: its code, and whether Shift goes with it.
pub type KeyChoice = (i64, bool);

/// The keys that type `text`, run by run in one layout group, for a backend that types keys, not characters.
///
/// `tables` give each group's key for each character it types. A character is typed in the group the previous one was
/// typed in when that group has it (spaces, digits and signs on the same key in both layouts do not switch), and
/// otherwise in the first group that has it. A character no group types makes the whole text `SelectionRefused`, before
/// a single key is typed.
pub fn typing_runs(
    text: &str,
    tables: &[HashMap<char, KeyChoice>],
    start_group: i64,
) -> Result<Vec<(i64, Vec<KeyChoice>)>, SelectionRefused> {
    let mut runs: Vec<(i64, Vec<KeyChoice>)> = Vec::new();
    let mut group = start_group;
    for character in text.chars() {
        let in_group = 0 <= group && group.to::<usize>() < tables.len() && tables[group.to::<usize>()].contains_key(&character);
        if !in_group {
            match tables.iter().position(|table| table.contains_key(&character)) {
                Some(index) => group = index.to::<i64>(),
                None => return Err(SelectionRefused::new(RefusalReason::Untypeable)),
            }
        }
        if runs.last().is_none_or(|run| run.0 != group) {
            runs.push((group, Vec::new()));
        }
        let key = tables[group.to::<usize>()][&character];
        runs.last_mut().expect("a run was just pushed").1.push(key);
    }
    Ok(runs)
}

/// The code point ranges (first, last) of the characters of Unicode categories Mn, Me and Cf (combining marks and
/// format characters such as the zero-width joiner of an emoji sequence), as `unicodedata.category` of `CPython` 3.14
/// (Unicode 16.0) gives them. They are the characters the caret does not step over on their own: a selection with them
/// is not selected back key by key.
pub const JOINED_CATEGORY_RANGES: &[(u32, u32)] = &[
    (0xAD,0xAD), (0x300,0x36F), (0x483,0x489), (0x591,0x5BD), (0x5BF,0x5BF), (0x5C1,0x5C2), (0x5C4,0x5C5),
    (0x5C7,0x5C7), (0x600,0x605), (0x610,0x61A), (0x61C,0x61C), (0x64B,0x65F), (0x670,0x670), (0x6D6,0x6DD),
    (0x6DF,0x6E4), (0x6E7,0x6E8), (0x6EA,0x6ED), (0x70F,0x70F), (0x711,0x711), (0x730,0x74A), (0x7A6,0x7B0),
    (0x7EB,0x7F3), (0x7FD,0x7FD), (0x816,0x819), (0x81B,0x823), (0x825,0x827), (0x829,0x82D), (0x859,0x85B),
    (0x890,0x891), (0x897,0x89F), (0x8CA,0x902), (0x93A,0x93A), (0x93C,0x93C), (0x941,0x948), (0x94D,0x94D),
    (0x951,0x957), (0x962,0x963), (0x981,0x981), (0x9BC,0x9BC), (0x9C1,0x9C4), (0x9CD,0x9CD), (0x9E2,0x9E3),
    (0x9FE,0x9FE), (0xA01,0xA02), (0xA3C,0xA3C), (0xA41,0xA42), (0xA47,0xA48), (0xA4B,0xA4D), (0xA51,0xA51),
    (0xA70,0xA71), (0xA75,0xA75), (0xA81,0xA82), (0xABC,0xABC), (0xAC1,0xAC5), (0xAC7,0xAC8), (0xACD,0xACD),
    (0xAE2,0xAE3), (0xAFA,0xAFF), (0xB01,0xB01), (0xB3C,0xB3C), (0xB3F,0xB3F), (0xB41,0xB44), (0xB4D,0xB4D),
    (0xB55,0xB56), (0xB62,0xB63), (0xB82,0xB82), (0xBC0,0xBC0), (0xBCD,0xBCD), (0xC00,0xC00), (0xC04,0xC04),
    (0xC3C,0xC3C), (0xC3E,0xC40), (0xC46,0xC48), (0xC4A,0xC4D), (0xC55,0xC56), (0xC62,0xC63), (0xC81,0xC81),
    (0xCBC,0xCBC), (0xCBF,0xCBF), (0xCC6,0xCC6), (0xCCC,0xCCD), (0xCE2,0xCE3), (0xD00,0xD01), (0xD3B,0xD3C),
    (0xD41,0xD44), (0xD4D,0xD4D), (0xD62,0xD63), (0xD81,0xD81), (0xDCA,0xDCA), (0xDD2,0xDD4), (0xDD6,0xDD6),
    (0xE31,0xE31), (0xE34,0xE3A), (0xE47,0xE4E), (0xEB1,0xEB1), (0xEB4,0xEBC), (0xEC8,0xECE), (0xF18,0xF19),
    (0xF35,0xF35), (0xF37,0xF37), (0xF39,0xF39), (0xF71,0xF7E), (0xF80,0xF84), (0xF86,0xF87), (0xF8D,0xF97),
    (0xF99,0xFBC), (0xFC6,0xFC6), (0x102D,0x1030), (0x1032,0x1037), (0x1039,0x103A), (0x103D,0x103E),
    (0x1058,0x1059), (0x105E,0x1060), (0x1071,0x1074), (0x1082,0x1082), (0x1085,0x1086), (0x108D,0x108D),
    (0x109D,0x109D), (0x135D,0x135F), (0x1712,0x1714), (0x1732,0x1733), (0x1752,0x1753), (0x1772,0x1773),
    (0x17B4,0x17B5), (0x17B7,0x17BD), (0x17C6,0x17C6), (0x17C9,0x17D3), (0x17DD,0x17DD), (0x180B,0x180F),
    (0x1885,0x1886), (0x18A9,0x18A9), (0x1920,0x1922), (0x1927,0x1928), (0x1932,0x1932), (0x1939,0x193B),
    (0x1A17,0x1A18), (0x1A1B,0x1A1B), (0x1A56,0x1A56), (0x1A58,0x1A5E), (0x1A60,0x1A60), (0x1A62,0x1A62),
    (0x1A65,0x1A6C), (0x1A73,0x1A7C), (0x1A7F,0x1A7F), (0x1AB0,0x1ACE), (0x1B00,0x1B03), (0x1B34,0x1B34),
    (0x1B36,0x1B3A), (0x1B3C,0x1B3C), (0x1B42,0x1B42), (0x1B6B,0x1B73), (0x1B80,0x1B81), (0x1BA2,0x1BA5),
    (0x1BA8,0x1BA9), (0x1BAB,0x1BAD), (0x1BE6,0x1BE6), (0x1BE8,0x1BE9), (0x1BED,0x1BED), (0x1BEF,0x1BF1),
    (0x1C2C,0x1C33), (0x1C36,0x1C37), (0x1CD0,0x1CD2), (0x1CD4,0x1CE0), (0x1CE2,0x1CE8), (0x1CED,0x1CED),
    (0x1CF4,0x1CF4), (0x1CF8,0x1CF9), (0x1DC0,0x1DFF), (0x200B,0x200F), (0x202A,0x202E), (0x2060,0x2064),
    (0x2066,0x206F), (0x20D0,0x20F0), (0x2CEF,0x2CF1), (0x2D7F,0x2D7F), (0x2DE0,0x2DFF), (0x302A,0x302D),
    (0x3099,0x309A), (0xA66F,0xA672), (0xA674,0xA67D), (0xA69E,0xA69F), (0xA6F0,0xA6F1), (0xA802,0xA802),
    (0xA806,0xA806), (0xA80B,0xA80B), (0xA825,0xA826), (0xA82C,0xA82C), (0xA8C4,0xA8C5), (0xA8E0,0xA8F1),
    (0xA8FF,0xA8FF), (0xA926,0xA92D), (0xA947,0xA951), (0xA980,0xA982), (0xA9B3,0xA9B3), (0xA9B6,0xA9B9),
    (0xA9BC,0xA9BD), (0xA9E5,0xA9E5), (0xAA29,0xAA2E), (0xAA31,0xAA32), (0xAA35,0xAA36), (0xAA43,0xAA43),
    (0xAA4C,0xAA4C), (0xAA7C,0xAA7C), (0xAAB0,0xAAB0), (0xAAB2,0xAAB4), (0xAAB7,0xAAB8), (0xAABE,0xAABF),
    (0xAAC1,0xAAC1), (0xAAEC,0xAAED), (0xAAF6,0xAAF6), (0xABE5,0xABE5), (0xABE8,0xABE8), (0xABED,0xABED),
    (0xFB1E,0xFB1E), (0xFE00,0xFE0F), (0xFE20,0xFE2F), (0xFEFF,0xFEFF), (0xFFF9,0xFFFB), (0x101FD,0x101FD),
    (0x102E0,0x102E0), (0x10376,0x1037A), (0x10A01,0x10A03), (0x10A05,0x10A06), (0x10A0C,0x10A0F),
    (0x10A38,0x10A3A), (0x10A3F,0x10A3F), (0x10AE5,0x10AE6), (0x10D24,0x10D27), (0x10D69,0x10D6D),
    (0x10EAB,0x10EAC), (0x10EFC,0x10EFF), (0x10F46,0x10F50), (0x10F82,0x10F85), (0x11001,0x11001),
    (0x11038,0x11046), (0x11070,0x11070), (0x11073,0x11074), (0x1107F,0x11081), (0x110B3,0x110B6),
    (0x110B9,0x110BA), (0x110BD,0x110BD), (0x110C2,0x110C2), (0x110CD,0x110CD), (0x11100,0x11102),
    (0x11127,0x1112B), (0x1112D,0x11134), (0x11173,0x11173), (0x11180,0x11181), (0x111B6,0x111BE),
    (0x111C9,0x111CC), (0x111CF,0x111CF), (0x1122F,0x11231), (0x11234,0x11234), (0x11236,0x11237),
    (0x1123E,0x1123E), (0x11241,0x11241), (0x112DF,0x112DF), (0x112E3,0x112EA), (0x11300,0x11301),
    (0x1133B,0x1133C), (0x11340,0x11340), (0x11366,0x1136C), (0x11370,0x11374), (0x113BB,0x113C0),
    (0x113CE,0x113CE), (0x113D0,0x113D0), (0x113D2,0x113D2), (0x113E1,0x113E2), (0x11438,0x1143F),
    (0x11442,0x11444), (0x11446,0x11446), (0x1145E,0x1145E), (0x114B3,0x114B8), (0x114BA,0x114BA),
    (0x114BF,0x114C0), (0x114C2,0x114C3), (0x115B2,0x115B5), (0x115BC,0x115BD), (0x115BF,0x115C0),
    (0x115DC,0x115DD), (0x11633,0x1163A), (0x1163D,0x1163D), (0x1163F,0x11640), (0x116AB,0x116AB),
    (0x116AD,0x116AD), (0x116B0,0x116B5), (0x116B7,0x116B7), (0x1171D,0x1171D), (0x1171F,0x1171F),
    (0x11722,0x11725), (0x11727,0x1172B), (0x1182F,0x11837), (0x11839,0x1183A), (0x1193B,0x1193C),
    (0x1193E,0x1193E), (0x11943,0x11943), (0x119D4,0x119D7), (0x119DA,0x119DB), (0x119E0,0x119E0),
    (0x11A01,0x11A0A), (0x11A33,0x11A38), (0x11A3B,0x11A3E), (0x11A47,0x11A47), (0x11A51,0x11A56),
    (0x11A59,0x11A5B), (0x11A8A,0x11A96), (0x11A98,0x11A99), (0x11C30,0x11C36), (0x11C38,0x11C3D),
    (0x11C3F,0x11C3F), (0x11C92,0x11CA7), (0x11CAA,0x11CB0), (0x11CB2,0x11CB3), (0x11CB5,0x11CB6),
    (0x11D31,0x11D36), (0x11D3A,0x11D3A), (0x11D3C,0x11D3D), (0x11D3F,0x11D45), (0x11D47,0x11D47),
    (0x11D90,0x11D91), (0x11D95,0x11D95), (0x11D97,0x11D97), (0x11EF3,0x11EF4), (0x11F00,0x11F01),
    (0x11F36,0x11F3A), (0x11F40,0x11F40), (0x11F42,0x11F42), (0x11F5A,0x11F5A), (0x13430,0x13440),
    (0x13447,0x13455), (0x1611E,0x16129), (0x1612D,0x1612F), (0x16AF0,0x16AF4), (0x16B30,0x16B36),
    (0x16F4F,0x16F4F), (0x16F8F,0x16F92), (0x16FE4,0x16FE4), (0x1BC9D,0x1BC9E), (0x1BCA0,0x1BCA3),
    (0x1CF00,0x1CF2D), (0x1CF30,0x1CF46), (0x1D167,0x1D169), (0x1D173,0x1D182), (0x1D185,0x1D18B),
    (0x1D1AA,0x1D1AD), (0x1D242,0x1D244), (0x1DA00,0x1DA36), (0x1DA3B,0x1DA6C), (0x1DA75,0x1DA75),
    (0x1DA84,0x1DA84), (0x1DA9B,0x1DA9F), (0x1DAA1,0x1DAAF), (0x1E000,0x1E006), (0x1E008,0x1E018),
    (0x1E01B,0x1E021), (0x1E023,0x1E024), (0x1E026,0x1E02A), (0x1E08F,0x1E08F), (0x1E130,0x1E136),
    (0x1E2AE,0x1E2AE), (0x1E2EC,0x1E2EF), (0x1E4EC,0x1E4EF), (0x1E5EE,0x1E5EF), (0x1E8D0,0x1E8D6),
    (0x1E944,0x1E94A), (0xE0001,0xE0001), (0xE0020,0xE007F), (0xE0100,0xE01EF),
];

/// Whether the character is of category Mn, Me or Cf.
#[must_use]
pub fn joined_category(character: char) -> bool {
    let code = character.to::<u32>();
    let index = JOINED_CATEGORY_RANGES.partition_point(|range| range.1 < code);
    JOINED_CATEGORY_RANGES.get(index).is_some_and(|range| range.0 <= code)
}

/// How many presses of Shift+Left select `text` back from its end, or None where that cannot be counted.
///
/// The caret steps over one character a press, but over a combining mark together with its letter and over an
/// emoji sequence as a whole, and some editors count a character above the Basic Multilingual Plane as two; such a
/// result is left unselected rather than selected wrong.
#[must_use]
pub fn caret_steps(text: &str) -> Option<usize> {
    if text.chars().any(|character| character.to::<u32>() > BASIC_MULTILINGUAL_PLANE_MAX_CODEPOINT || joined_category(character)) {
        return None;
    }
    Some(text.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        vec!["us".to_string(), "ru".to_string()]
    }

    #[test]
    fn refusals_are_named_in_russian() {
        assert_eq!(refusal_message(RefusalReason::TooLong), "Выделение не изменено: предел — 1000 символов");
        assert_eq!(refusal_message(RefusalReason::NoSelection), "Нет выделенного текста");
    }

    #[test]
    fn checks() {
        assert_eq!(checked_selection("abc"), Ok("abc"));
        assert_eq!(checked_selection("").unwrap_err().reason, RefusalReason::NoSelection);
        assert_eq!(checked_selection(&"я".repeat(1001)).unwrap_err().reason, RefusalReason::TooLong);
        assert!(checked_selection(&"я".repeat(1000)).is_ok());
        for text in ["a\nb", "a\tb", "a\u{2028}b", "a\u{85}b"] {
            assert_eq!(checked_selection(text).unwrap_err().reason, RefusalReason::LineBreaks);
        }
        assert!(checked_selection("a\u{1c}b").is_ok());
    }

    #[test]
    fn plans() {
        let plan = plan_selection(SelectionCommand::Layout, "ghbdtn", 0, &names()).unwrap();
        assert_eq!((plan.replacement.as_str(), plan.source_group, plan.target_group), ("привет", Some(0), Some(1)));
        assert!(plan.changes("ghbdtn"));
        let plan = plan_selection(SelectionCommand::Case, "привет", 0, &names()).unwrap();
        assert_eq!((plan.replacement.as_str(), plan.source_group), ("ПРИВЕТ", None));
        assert_eq!(plan_selection(SelectionCommand::Transliterate, "ьь", 0, &names()).unwrap_err().reason, RefusalReason::EmptyResult);
        assert_eq!(plan_selection(SelectionCommand::Layout, "ghbdtn", 0, &["us".to_string()]).unwrap_err().reason, RefusalReason::NoLayout);
        // Out of range current group: the tie goes to the US layout.
        let plan = plan_selection(SelectionCommand::Layout, "?/", 9, &names()).unwrap();
        assert_eq!(plan.source_group, Some(0));
    }

    #[test]
    fn typing() {
        let mut us = HashMap::new();
        let mut ru = HashMap::new();
        us.insert('a', (30, false));
        us.insert(' ', (57, false));
        ru.insert('ф', (30, false));
        ru.insert(' ', (57, false));
        let runs = typing_runs("a фa", &[us.clone(), ru.clone()], 0).unwrap();
        assert_eq!(runs.iter().map(|r| (r.0, r.1.len())).collect::<Vec<_>>(), [(0, 2), (1, 1), (0, 1)]);
        assert_eq!(typing_runs("a!", &[us, ru], 0).unwrap_err().reason, RefusalReason::Untypeable);
    }

    #[test]
    fn caret() {
        assert_eq!(caret_steps("привет"), Some(6));
        assert_eq!(caret_steps("e\u{301}"), None);
        assert_eq!(caret_steps("a😀"), None);
        assert_eq!(caret_steps("a\u{200d}"), None);
        assert_eq!(caret_steps(""), Some(0));
    }
}

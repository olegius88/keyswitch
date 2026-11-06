//! A long walk through the engine, the same on every run: words in both layouts mixed with what a person does between them
//! (other windows, pauses of every length, Pause and the hotkeys, edits, clicks, fields that cannot be read, windows that
//! close under a correction). What must hold: nothing stops, the caret stays in the text, the text stays bounded.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::Arc;

use ks_backend::constants::{ALT_MASK, CONTROL_MASK, LOCK_MASK, SHIFT_MASK, SUPER_MASK};
use ks_backend::{FieldContext, FieldReader, FieldRole, KeyEvent};
use ks_config::learning::{LearnedRule, RuleAction, RuleMatch};
use ks_engine::engine::PlanRef;
use ks_replay::harness::{Backend, Bundle, CaretReader, KEYS, PAUSE_KEYCODE, Session, UnreadField};
use serde_json::{Value, json};

/// A small generator with a fixed start: the walk must be the same on every machine.
struct Dice(u64);

impl Dice {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, limit: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(limit).expect("small")).expect("small")
    }

    fn of<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    fn one_in(&mut self, odds: usize) -> bool {
        self.below(odds) == 0
    }
}

const WORDS: &[&str] = &[
    "ghbdtn", "привет", "hello", "руддщ", "vbh", "мир", "world", "цщкдв", "ntcn", "тест", "test", "еуые", "z", "я", "x", "ч", "d", "в", "r", "к", "f", "а",
    "b", "и", "c", "с", "tot", "ещ", "the", "еру", "and", "фтв", "lf", "да", "ytn", "нет", "yt", "не", "yf", "на", "gj", "по", "dct", "все", "rfr", "как",
    "ytn,", "ghbdtn.", "vbh!", "ghbdtn,vbh", "привет,мир", "кто-то", "rnj-nj", "mongo-cthdbcf", "ghbdtn-vbh", "docker-compose", "v67", "м67", "ntcn123", "123ntcn",
    "2ю", "2.", "1ю5", "DKRCV", "ЗОР", "GHBDTN", "ПРИВЕТ", "Ghbdtn", "Привет", "gHBDTN", "пРИВЕТ", "кз", "rp", "webjs", "vuejs", "htmx", "api", "фзш", "url", "гкд",
    "github", "пшерги", "git", "пше", "npm", "тзь", "lj,hsq", "хорошо", "[jhjij", "ю", ".", ",", "б", "'", "э", "ъ", "]", "[", "х", "doesn't", "вщуытэе", "o'clock",
    "user.name", "гыук.тфьу", "a.b", "ф.и", "foo_bar", "агбар", "/usr/bin", "c:\\x", "http://x.y", "x@y.z", "(ghbdtn)", "\"ghbdtn\"", "'ghbdtn'", "ghbdtn:", "ghbdtn;",
    "aa", "bbb", "ааа", "ууу", "нееет", "ytttn", "ghbbbdtn", "йцукен", "qwerty", "йй", "qq", "ъъ", "]]", "ьь", "mm", "ыы", "ss", "ё", "`", "ёлка", "`krf",
    "the", "of", "in", "to", "is", "щ", "ш", "о", "j", "i", "у", "e", "е", "t", "ф", "a", "т", "n", "м", "v", "л", "k", "ц", "w", "ы", "s",
];

const BOUNDARIES: &[&str] = &[" ", " ", " ", " ", " ", " ", ",", ".", "!", "?", ":", ";", "-", "'", "/", "\"", ")", "(", "б", "ю", "ж", "э"];

const APPLICATIONS: &[&str] = &["firefox", "code", "telegram-desktop", "xterm", "gnome-terminal", "chrome", "keyswitch", "libreoffice", ""];

const NAMED_KEYS: &[&str] = &[
    "BackSpace", "BackSpace", "BackSpace", "Delete", "Return", "KP_Enter", "Tab", "ISO_Left_Tab", "Escape", "Left", "Right", "Up", "Down", "Home", "End", "Page_Up", "Page_Down",
    "Insert", "F1", "Shift_L", "Control_L", "Alt_L", "Super_L", "Caps_Lock", "Num_Lock", "Menu", "XF86AudioPlay",
];

const IDLES: &[f64] = &[0.01, 0.05, 0.1, 0.3, 0.5, 0.7, 0.71, 1.0, 1.5, 2.0, 5.0, 16.0, 31.0, 120.0];

const SETTINGS: &[(&str, &[&str])] = &[
    ("enabled", &["false"]),
    ("general.keep_history", &["false"]),
    ("detection.minimum_length", &["1", "5", "\"x\""]),
    ("detection.confidence", &["0.1", "5.0", "\"0.9\"", "null"]),
    ("detection.correct_on_pause", &["false"]),
    ("detection.pause_delay_seconds", &["0.05", "5.0"]),
    ("detection.early_switch", &["false"]),
    ("detection.early_switch_min_length", &["1", "8"]),
    ("detection.correct_on_space", &["false"]),
    ("detection.correct_on_enter", &["false"]),
    ("detection.correct_on_tab", &["false"]),
    ("detection.correct_on_punctuation", &["false"]),
    ("detection.respect_manual_layout", &["false"]),
    ("detection.fix_double_capitals", &["false"]),
    ("detection.protect_code", &["false"]),
    ("detection.context_aware", &["false"]),
    ("detection.context_policy", &["\"off\"", "\"shadow\"", "\"assist\"", "\"auto\""]),
    ("detection.aggressive", &["true"]),
    ("detection.context_read_field", &["false"]),
    ("detection.hold_after_caret_move", &["true", "false"]),
    ("detection.intent_model_enabled", &["false"]),
    ("detection.correct_typos_english", &["true"]),
    ("detection.learning", &["false"]),
    ("detection.field_catch_up", &["false", "true"]),
    ("detection.correct_typos", &["true"]),
    ("detection.injected_input", &["true"]),
    ("exclusions.words", &["[\"ghbdtn\", \"привет\", \"tot\"]"]),
    ("exclusions.applications", &["[\"firefox\", \"code\"]"]),
    ("diagnostics.technical_logging", &["true"]),
];

/// The field as a program may report it.
struct Reported {
    backend: Arc<Backend>,
    role: FieldRole,
    sensitive: bool,
    selection: bool,
    field_id: &'static str,
}

impl FieldReader for Reported {
    fn read(&self, application: &str, _window: i64) -> Option<FieldContext> {
        let (text, caret) = (self.backend.text(), self.backend.caret());
        let mut field = FieldContext::new(application, self.field_id);
        field.before = text[..caret].iter().collect();
        field.after = text[caret..].iter().collect();
        field.role = self.role;
        field.sensitive = self.sensitive;
        field.selection = self.selection;
        Some(field)
    }

    fn status(&self) -> String {
        "available".to_owned()
    }
}

fn type_character(session: &mut Session, dice: &mut Dice, character: char, state: u32) {
    let found = KEYS.iter().find_map(|key| {
        let group = key.characters.iter().position(|typed| *typed == character)?;
        Some((key, group))
    });
    let Some((key, group)) = found else {
        return;
    };
    if usize::try_from(session.backend.group()).ok() != Some(group) && !dice.one_in(50) {
        session.select_layout(i32::try_from(group).expect("two layouts"));
    }
    session.tap(KeyEvent {
        pressed: true,
        keycode: key.keycode,
        key_name: character.to_string(),
        character: character.to_string(),
        characters: key.strings(),
        group: session.backend.group(),
        state: state | if key.shift { SHIFT_MASK } else { 0 },
        synthetic: dice.one_in(200),
        foreign: dice.one_in(200),
        ..KeyEvent::default()
    });
}

fn press_pause(session: &mut Session) {
    let group = session.backend.group();
    session.tap(KeyEvent {
        pressed: true,
        keycode: PAUSE_KEYCODE,
        key_name: "Pause".to_owned(),
        characters: vec![String::new(), String::new()],
        group,
        ..KeyEvent::default()
    });
}

fn change_reader(session: &mut Session, dice: &mut Dice) {
    let backend = session.backend.clone();
    let reader: Arc<dyn FieldReader> = match dice.below(8) {
        0 => Arc::new(UnreadField),
        1 => Arc::new(Reported { backend, role: FieldRole::Password, sensitive: true, selection: false, field_id: "secret" }),
        2 => Arc::new(Reported { backend, role: FieldRole::Terminal, sensitive: false, selection: false, field_id: "terminal" }),
        3 => Arc::new(Reported { backend, role: FieldRole::Code, sensitive: false, selection: false, field_id: "code" }),
        4 => Arc::new(Reported { backend, role: FieldRole::Search, sensitive: false, selection: true, field_id: "search" }),
        5 => Arc::new(Reported { backend, role: FieldRole::Text, sensitive: false, selection: false, field_id: "" }),
        6 => Arc::new(Reported { backend, role: FieldRole::Unknown, sensitive: false, selection: false, field_id: "other" }),
        _ => Arc::new(CaretReader { backend }),
    };
    session.core.context_policy.reader = Some(reader);
}

fn add_rule(session: &mut Session, dice: &mut Dice) {
    let source = i64::try_from(dice.below(2)).expect("small");
    let learning = session.core.learning.clone();
    let mut rule = LearnedRule::new(dice.of(WORDS), source, 1 - source);
    rule.action = *dice.of(&[RuleAction::Convert, RuleAction::Keep]);
    rule.r#match = *dice.of(&[RuleMatch::Exact, RuleMatch::Prefix, RuleMatch::Contains]);
    rule.case_sensitive = dice.one_in(3);
    let _ = learning.add_rule(rule);
}

/// What the engine holds about a word grows old while the person does something else: every guard on it must hold.
fn age_what_is_held(session: &mut Session, dice: &mut Dice) {
    let core = &mut session.core;
    let kind = dice.below(7);
    let changed = |plan: &PlanRef, kind: usize| {
        let mut plan = (**plan).clone();
        match kind {
            0 => "another program".clone_into(&mut plan.application),
            1 => plan.boundary = None,
            2 => plan.source_group = 1 - plan.source_group,
            _ => plan.target_group = plan.source_group,
        }
        Arc::new(plan)
    };
    if let Some(waiting) = &mut core.context_waiting {
        match kind {
            0 => waiting.window = -1,
            1 => waiting.deadline = 0.0,
            2 => waiting.baseline = None,
            _ => waiting.plan = changed(&waiting.plan, kind - 3),
        }
    }
    if let Some(word) = &mut core.provisional {
        match kind {
            0 => word.window = -1,
            1 => word.deadline = 0.0,
            _ => word.plan = changed(&word.plan, kind - 2),
        }
    }
    if let Some(word) = &mut core.paused_conversion {
        match kind {
            0 => word.window = -1,
            1 => word.deadline = 0.0,
            _ => word.plan = changed(&word.plan, kind - 2),
        }
    }
    core.kept = core
        .kept
        .iter()
        .map(|word| {
            let mut word = (**word).clone();
            match kind {
                0 => word.window = -1,
                1 => word.deadline = 0.0,
                2 => word.baseline = None,
                3 => word.field = None,
                4 => word.segmentation_open = true,
                _ => word.plan = changed(&word.plan, kind - 5),
            }
            Arc::new(word)
        })
        .collect();
    if let Some(plan) = &core.last_committed {
        core.last_committed = Some(changed(plan, kind % 4));
    }
    if let Some(plan) = &core.last_correction {
        core.last_correction = Some(changed(plan, kind % 4));
    }
}

fn step(session: &mut Session, dice: &mut Dice, window: &mut i64) {
    match dice.below(106) {
        100..=103 => age_what_is_held(session, dice),
        104..=105 => session.advance(*dice.of(IDLES)),
        0..=54 => {
            let word = *dice.of(WORDS);
            for character in word.chars() {
                type_character(session, dice, character, 0);
            }
            if !dice.one_in(6) {
                let boundary = *dice.of(BOUNDARIES);
                for character in boundary.chars() {
                    type_character(session, dice, character, 0);
                }
            }
        }
        55..=64 => session.tap_named(dice.of(NAMED_KEYS)),
        65..=69 => session.idle(*dice.of(IDLES)),
        70..=74 => press_pause(session),
        75 => {
            press_pause(session);
            press_pause(session);
        }
        76..=77 => session.core.schedule_manual_conversion(i64::from(PAUSE_KEYCODE)),
        78 => session.core.schedule_undo(i64::from(PAUSE_KEYCODE)),
        79 => session.core.schedule_undo_hotkey(i64::from(PAUSE_KEYCODE)),
        80 => session.core.switch_layout_only(i64::from(PAUSE_KEYCODE)),
        81..=82 => session.pointer(),
        83 => {
            let state = *dice.of(&[CONTROL_MASK, ALT_MASK, SUPER_MASK, LOCK_MASK, CONTROL_MASK | SHIFT_MASK]);
            let letter = *dice.of(&['c', 'v', 'z', 'a', 'x', 'с']);
            type_character(session, dice, letter, state);
        }
        84..=85 => {
            *window += 1;
            session.backend.focus(*window, dice.of(APPLICATIONS));
        }
        86 => session.backend.focus(*window, dice.of(APPLICATIONS)),
        87 => session.select_layout(i32::try_from(dice.below(2)).expect("small")),
        88 => change_reader(session, dice),
        89 => add_rule(session, dice),
        90 => {
            let (path, values) = *dice.of(SETTINGS);
            let value: Value = serde_json::from_str(dice.of(values)).expect("a value");
            let _ = session.core.settings.set(path, value, false);
        }
        91 => {
            session.backend.set_failing(true);
            press_pause(session);
            let word = *dice.of(WORDS);
            for character in word.chars() {
                type_character(session, dice, character, 0);
            }
            type_character(session, dice, ' ', 0);
            session.backend.set_failing(false);
        }
        92 => session.backend.set_caret(0),
        93 => {
            session.backend.set_text(Vec::new());
            session.backend.set_caret(0);
        }
        94 => session.backend.insert_at_caret(dice.of(WORDS)),
        95 => {
            let _ = session.core.shared.confirm_learning_prompt(None);
        }
        96 => {
            let _ = session.core.shared.dismiss_learning_prompt(None, "walk");
        }
        _ => session.handle(KeyEvent { pressed: true, key_name: "Return".to_owned(), characters: vec![String::new(), String::new()], deferred: true, group: session.backend.group(), ..KeyEvent::default() }),
    }
}

#[test]
fn a_long_walk_stops_nowhere() {
    let mut bundle = Bundle::load(None);
    for seed in 1..=WALKS {
        let mut dice = Dice(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_mul(0xbf58_476d_1ce4_e5b9));
        let group = i32::try_from(dice.below(2)).expect("small");
        let opening: Vec<char> = (*dice.of(&["", "", "начало ", "start ", "начало", "start", "привет, ", "git commit "])).chars().collect();
        let mut session = bundle.session(dice.of(APPLICATIONS), group, opening);
        session.recovers = true;
        let _ = session.core.settings.set("detection.correct_typos", json!(dice.one_in(3)), false);
        let mut window = 1;
        for _ in 0..STEPS {
            step(&mut session, &mut dice, &mut window);
            let (text, caret) = (session.backend.text(), session.backend.caret());
            assert!(caret <= text.len(), "seed {seed}");
            assert!(text.len() < 20_000, "seed {seed}");
        }
        session.idle(2.0);
        bundle.give_back(session);
    }
    bundle.clean();
}

const WALKS: u64 = 400;
const STEPS: usize = 60;

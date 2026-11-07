//! What the settings window shows and how it reads and writes a setting, apart from any toolkit (the Python's
//! `windows_ui_model.py`, `desktop_theme.py` and the parts of `desktop_ui.py` that are not drawing). It compiles and is
//! tested on every platform; `settings_window` draws it.

use ks_cast::To as _;
use std::fmt;

use ks_config::app_quirks::MENTION_HEADS;
use ks_config::constants::HINTS_SETTING;
use ks_config::history::HistoryEntry;
use ks_config::learning::LearnedRule;
use ks_context::constants::PREFIX_MAX_CHARACTERS;
use ks_context::typo_correction::{
    CORRECT_TYPOS_ENGLISH_DESCRIPTION, CORRECT_TYPOS_ENGLISH_TITLE, CORRECT_TYPOS_TITLE, correct_typos_description,
};
use ks_engine::constants::{
    CONFIDENCE_SETTING_MAX, CONFIDENCE_SETTING_MIN, EARLY_SWITCH_MIN_LENGTH_SETTING_MAX, EARLY_SWITCH_MIN_LENGTH_SETTING_MIN,
    FIELD_CATCH_UP_CHOICES, MINIMUM_WORD_LENGTH_SETTING_MAX, MINIMUM_WORD_LENGTH_SETTING_MIN, PAUSE_DELAY_SETTING_MAX_SECONDS,
    PAUSE_DELAY_SETTING_MIN_SECONDS, PREFIX_MIN_CHARACTERS, SELECTION_HOTKEYS, UNDO_AVAILABLE_WINDOW_SECONDS,
};
use ks_engine::engine::{LAYOUT_SWITCH_KEY_DESCRIPTION, LAYOUT_SWITCH_KEY_LABELS, LAYOUT_SWITCH_KEY_TITLE};
use ks_engine::rule_editor::{action_text, condition_text, other_layout_text};
use ks_text::pystr;
use ks_text::russian_text::{HOURS, SECONDS, quantity};
use serde_json::{Map, Value, json};

use crate::constants::{
    CONFIDENCE_SETTING_STEP, CONVERT_HOTKEY_NAME, HISTORY_LIMIT_SETTING_MAX, HISTORY_LIMIT_SETTING_MIN, HISTORY_LIMIT_SETTING_STEP,
    NUMBER_DISPLAY_DECIMAL_PLACES, PAUSE_DELAY_SETTING_STEP_SECONDS, UPDATE_CHECK_INTERVAL_MS,
};

/// The titles of the pages the hints name (`hints`); the GTK window's pages have the same titles, and the macOS window
/// calls the system page by its own one.
pub const AUTOCORRECTION_PAGE_TITLE: &str = "Автокоррекция";
pub const HOTKEYS_PAGE_TITLE: &str = "Горячие клавиши";
pub const SYSTEM_PAGE_TITLE: &str = "Внешний вид и система";
pub const MAC_SYSTEM_PAGE_TITLE: &str = "Внешний вид";

/// `PAGE_NAMES`: the pages in the order of the navigation.
pub const PAGE_NAMES: [(&str, &str); 10] = [
    ("overview", "Обзор"),
    ("autocorrection", AUTOCORRECTION_PAGE_TITLE),
    ("layouts", "Раскладки"),
    ("hotkeys", HOTKEYS_PAGE_TITLE),
    ("exclusions", "Исключения"),
    ("appearance", SYSTEM_PAGE_TITLE),
    ("history", "История"),
    ("updates", "Обновления"),
    ("maintenance", "Обслуживание"),
    ("about", "О программе"),
];

/// A word, not a pictogram: Segoe UI is documented to cover Cyrillic, while the revert arrows live in Segoe UI Symbol.
pub const RESET_BUTTON_LABEL: &str = "Сброс";
pub const RESET_BUTTON_HINT: &str = "Вернуть значение по умолчанию";

pub const SELECTION_HOTKEY_TITLES: [(&str, &str); 3] = [
    ("selection_layout", "Сменить раскладку выделенного текста"),
    ("selection_case", "Сменить регистр выделенного текста"),
    ("selection_transliterate", "Транслитерировать выделенный текст"),
];
pub const SELECTION_HOTKEY_DESCRIPTIONS: [(&str, &str); 3] = [
    (
        "selection_layout",
        "Выделенный текст, набранный не в той раскладке, перепечатывается в другой (ghbdtn → привет) и раскладка \
         переключается на неё. В выделении из двух алфавитов перепечатываются слова того, которого меньше \
         (привет vbh → привет мир)",
    ),
    (
        "selection_case",
        "По кругу: строчные → ПРОПИСНЫЕ → Каждое Слово С Прописной → строчные. Текст, набранный с включённым \
         Caps Lock (пРИВЕТ), исправляется сразу",
    ),
    (
        "selection_transliterate",
        "Кириллица латиницей по таблице ICAO Doc 9303 (Щука → Shchuka) и латиница кириллицей \
         (Privet → Привет). Мягкий знак эта таблица не передаёт",
    ),
];
/// The titles of the settings the hints name, in every settings window.
pub const EARLY_SWITCH_TITLE: &str = "Ранняя смена раскладки";
pub const HINTS_SETTING_TITLE: &str = "Показывать подсказки о возможностях";
pub const HINTS_SETTING_DESCRIPTION: &str =
    "Короткие пояснения у курсора и у значка: что умеет программа и что нового в выпуске. Каждое показывается один раз";
/// What both settings windows say of `detection.field_catch_up`.
pub const FIELD_CATCH_UP_TITLE: &str = "Ждать отстающее поле";
pub const FIELD_CATCH_UP_DESCRIPTION: &str = "Браузеры и редакторы на Electron показывают набранное с задержкой. Сколько ждать, \
                                              пока поле покажет слово, прежде чем отказаться от его исправления";
pub const FIELD_CATCH_UP_LABELS: [(&str, &str); 5] =
    [("off", "Не ждать"), ("100", "0,1 с"), ("300", "0,3 с"), ("500", "0,5 с"), ("1000", "1 с")];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    Bool,
    Choice,
    Int,
    Float,
    Text,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingSpec {
    pub path: String,
    pub title: String,
    pub description: String,
    pub kind: ControlKind,
    pub choices: Vec<(String, String)>,
    pub minimum: f64,
    pub maximum: f64,
    pub step: f64,
}

/// `SETTING_SPEC_DEFAULT_MAXIMUM`.
const SETTING_SPEC_DEFAULT_MAXIMUM: f64 = 100.0;

fn spec(path: &str, title: &str, description: &str, kind: ControlKind) -> SettingSpec {
    SettingSpec {
        path: path.to_owned(),
        title: title.to_owned(),
        description: description.to_owned(),
        kind,
        choices: Vec::new(),
        minimum: 0.0,
        maximum: SETTING_SPEC_DEFAULT_MAXIMUM,
        step: 1.0,
    }
}

fn number(mut base: SettingSpec, minimum: f64, maximum: f64, step: f64) -> SettingSpec {
    base.minimum = minimum;
    base.maximum = maximum;
    base.step = step;
    base
}

fn choice(mut base: SettingSpec, choices: &[(&str, &str)]) -> SettingSpec {
    base.choices = choices.iter().map(|(key, label)| ((*key).to_owned(), (*label).to_owned())).collect();
    base
}

use ControlKind::{Bool, Choice, Float, Int, Text};

#[must_use]
pub fn autocorrection_settings() -> Vec<SettingSpec> {
    vec![
        spec(
            "enabled",
            "Автоматически исправлять раскладку",
            "Отключает автоматические замены; ручные горячие клавиши остаются доступны.",
            Bool,
        ),
        number(
            spec(
                "detection.minimum_length",
                "Минимальная длина слова",
                "Минимум для базового детектора; явные правила, короткие исключения и контекстный помощник могут разрешить замену раньше.",
                Int,
            ),
            MINIMUM_WORD_LENGTH_SETTING_MIN,
            MINIMUM_WORD_LENGTH_SETTING_MAX,
            1.0,
        ),
        number(
            spec(
                "detection.confidence",
                "Порог уверенности",
                "Больше — строже резервные эвристики. Пороги обученных моделей не меняются.",
                Float,
            ),
            CONFIDENCE_SETTING_MIN,
            CONFIDENCE_SETTING_MAX,
            CONFIDENCE_SETTING_STEP,
        ),
        spec(
            "detection.fix_double_capitals",
            "Исправлять ДВе заглавные",
            "Слово, набранное с двумя заглавными в начале (ПРивет, HEllo), становится обычным (Привет, Hello), если \
             словарь его раскладки знает такое слово. Аббревиатуры, слова из одних заглавных, короткие слова и слова \
             из исключений не меняются. Отменяется горячей клавишей «Отменить исправление».",
            Bool,
        ),
        spec(
            "detection.context_aware",
            "Учитывать контекст",
            "Фраза и приложение помогают моделям выбрать действие; без контекста работает базовое распознавание. Контекст хранится только в памяти.",
            Bool,
        ),
        choice(
            spec(
                "detection.context_policy",
                "Контекстный ИИ-помощник",
                "Отдельные локальные модели для слов и префиксов. «Только наблюдать» не применяет их решения; базовая автокоррекция продолжает работать.",
                Choice,
            ),
            &[("assist", "Исправлять и предлагать"), ("shadow", "Только наблюдать"), ("off", "Выключен")],
        ),
        spec(
            "detection.context_read_field",
            "Читать контекст активного поля",
            "Читать текст рядом с курсором локально через доступность ОС. Распознанные защищённые поля исключаются; не все приложения сообщают о них.",
            Bool,
        ),
        {
            let labels: Vec<(&str, &str)> = FIELD_CATCH_UP_CHOICES
                .iter()
                .filter_map(|key| FIELD_CATCH_UP_LABELS.iter().find(|(name, _)| name == key).copied())
                .collect();
            choice(
                spec("detection.field_catch_up", FIELD_CATCH_UP_TITLE, &format!("{FIELD_CATCH_UP_DESCRIPTION}."), Choice),
                &labels,
            )
        },
        spec(
            "detection.hold_after_caret_move",
            "Не исправлять слово сразу после перемещения курсора",
            &format!(
                "Стрелки, Home и Page Up уводят курсор туда, где программа не видела набора, а чаще всего \
                 курсор переносят внутрь уже написанного, чтобы дописать или поправить слово. Такое слово \
                 не переключается автоматически; {CONVERT_HOTKEY_NAME} переключит его вручную. Если чтение контекста \
                 активного поля работает, текст перед курсором известен и правило не применяется."
            ),
            Bool,
        ),
        spec(
            "detection.protect_code",
            "Защищать код, URL и сокращения",
            "Оставлять технические токены и адреса без автокоррекции.",
            Bool,
        ),
        spec("detection.correct_typos", CORRECT_TYPOS_TITLE, &format!("{}.", correct_typos_description()), Bool),
        spec(
            "detection.correct_typos_english",
            CORRECT_TYPOS_ENGLISH_TITLE,
            &format!("{CORRECT_TYPOS_ENGLISH_DESCRIPTION}."),
            Bool,
        ),
        spec(
            "detection.intent_model_enabled",
            "Базовая модель слов (KSLM)",
            "При отключении работают резервные эвристики. Контекстные модели слов и префиксов настраиваются отдельно.",
            Bool,
        ),
        spec(
            "detection.aggressive",
            "Расширенное распознавание",
            "Когда ни один словарь не знает слово, решает символьная модель. С этой настройкой её планка ниже: больше незнакомых слов переводится, а слово никогда не превращается в строку с пунктуацией (проверяется отдельно). Включено по умолчанию.",
            Bool,
        ),
        spec(
            "detection.early_switch",
            EARLY_SWITCH_TITLE,
            "Менять раскладку уже по первым буквам, не дожидаясь конца слова. Включено по умолчанию: пара моделей context-v3 + prefix-v2 прошла запечатанную проверку в этом режиме без порчи правильно набранного текста. В assist с учётом контекста — обученная модель префиксов; без контекста или в off/shadow — словарный алгоритм.",
            Bool,
        ),
        number(
            spec(
                "detection.early_switch_min_length",
                "Символов до ранней смены",
                &format!(
                    "Минимум для проверки; значение {} действует только в словарном алгоритме. \
                     Модель проверяет префиксы длиной {PREFIX_MIN_CHARACTERS}–{PREFIX_MAX_CHARACTERS} и ждёт при сомнении.",
                    EARLY_SWITCH_MIN_LENGTH_SETTING_MIN.to::<i64>()
                ),
                Int,
            ),
            EARLY_SWITCH_MIN_LENGTH_SETTING_MIN,
            EARLY_SWITCH_MIN_LENGTH_SETTING_MAX,
            1.0,
        ),
        spec(
            "detection.learning",
            "Правила переключения",
            &format!(
                "Правила из списка на странице «Обслуживание» действуют, а двойное нажатие клавиши \
                 ручного преобразования ({CONVERT_HOTKEY_NAME}) предлагает правило для последнего слова. Правило \
                 появляется только по кнопке OK в окне правила."
            ),
            Bool,
        ),
        spec(
            "detection.injected_input",
            "Ввод от других программ",
            "Считать набором нажатия, которые присылают другие программы: удалённый доступ \
             (TeamViewer, AnyDesk), макросы, экранная клавиатура. Выключено по умолчанию: такой \
             ввод уже проверен на компьютере, где его набирают, а ожидание его отпусканий \
             задерживало Enter.",
            Bool,
        ),
    ]
}

/// Shown on the layouts page only, under «Ручное переключение».
#[must_use]
pub fn manual_switch_settings() -> Vec<SettingSpec> {
    vec![spec(
        "detection.respect_manual_layout",
        "Учитывать ручную смену языка",
        "Не исправлять первое слово после выбранной пользователем раскладки.",
        Bool,
    )]
}

#[must_use]
pub fn trigger_settings() -> Vec<SettingSpec> {
    vec![
        spec("detection.correct_on_pause", "Пауза в наборе", "Проверять текущее слово после паузы без ввода.", Bool),
        number(
            spec(
                "detection.pause_delay_seconds",
                "Пауза, секунд",
                "Сколько ждать без ввода, прежде чем проверить незавершённое слово.",
                Float,
            ),
            PAUSE_DELAY_SETTING_MIN_SECONDS,
            PAUSE_DELAY_SETTING_MAX_SECONDS,
            PAUSE_DELAY_SETTING_STEP_SECONDS,
        ),
        spec("detection.correct_on_space", "Пробел", "Проверять слово перед пробелом.", Bool),
        spec(
            "detection.correct_on_enter",
            "Перед Enter",
            "Сначала исправить слово, затем передать Enter приложению один раз. Shift+Enter не перехватывается.",
            Bool,
        ),
        spec(
            "detection.correct_on_tab",
            "Перед Tab",
            "Сначала исправить слово, затем перейти к следующему полю. Shift+Tab не перехватывается.",
            Bool,
        ),
        spec(
            "detection.correct_on_punctuation",
            "Знаки препинания",
            "Проверять слово перед точкой, запятой и другими разделителями.",
            Bool,
        ),
    ]
}

#[must_use]
pub fn hotkey_settings() -> Vec<SettingSpec> {
    let mut specs = vec![
        choice(
            spec("detection.layout_switch_key", LAYOUT_SWITCH_KEY_TITLE, &format!("{LAYOUT_SWITCH_KEY_DESCRIPTION}."), Choice),
            &LAYOUT_SWITCH_KEY_LABELS,
        ),
        spec("hotkeys.toggle", "Включить или приостановить", "Глобальная пауза автоматического переключения.", Text),
        spec(
            "hotkeys.convert_last",
            "Преобразовать последнее слово",
            "Ручная смена раскладки уже введённого слова. Двойное нажатие предлагает правило \
             переключения для этого слова.",
            Text,
        ),
        spec(
            "hotkeys.undo",
            "Отменить исправление",
            &format!(
                "Вернуть последнее исправление: слово станет таким, каким его набрали. Работает и после пробелов, \
                 набранных за исправленным словом; после любой другой клавиши отменять уже нечего. Доступно \
                 {} после исправления.",
                quantity(UNDO_AVAILABLE_WINDOW_SECONDS, SECONDS)
            ),
            Text,
        ),
    ];
    for name in SELECTION_HOTKEYS {
        let find = |table: &[(&'static str, &'static str)]| table.iter().find(|(key, _)| *key == name).map_or("", |(_, text)| *text);
        specs.push(spec(
            &format!("hotkeys.{name}"),
            find(&SELECTION_HOTKEY_TITLES),
            &format!("{}.", find(&SELECTION_HOTKEY_DESCRIPTIONS)),
            Text,
        ));
    }
    specs
}

#[must_use]
pub fn system_settings() -> Vec<SettingSpec> {
    vec![
        spec("general.autostart", "Запускать вместе с Windows", "Стартовать после входа пользователя в систему.", Bool),
        spec(
            "general.start_hidden",
            "Запускать свёрнутым в трей",
            "Не показывать окно настроек после автоматического запуска.",
            Bool,
        ),
        spec(
            "general.close_to_tray",
            "Закрывать окно в трей",
            "Кнопка закрытия скрывает окно, а движок продолжает работать.",
            Bool,
        ),
        spec(
            "appearance.show_indicator",
            "Показывать индикатор в области уведомлений",
            "Иконка показывает текущую раскладку и открывает меню KeySwitch.",
            Bool,
        ),
        choice(
            spec("appearance.indicator_style", "Вид индикатора", "Двухбуквенное обозначение или флаг страны.", Choice),
            &[("letters", "EN / RU"), ("flags", "Флаги стран")],
        ),
        choice(
            spec("appearance.theme", "Тема окна", "Системная, светлая или тёмная цветовая схема.", Choice),
            &[("system", "Системная"), ("light", "Светлая"), ("dark", "Тёмная")],
        ),
        spec("general.notifications", "Уведомления об исправлениях", "Показывать исходное и исправленное слово.", Bool),
        spec("general.sound", "Звук исправления", "Воспроизводить системный сигнал после замены.", Bool),
        spec(HINTS_SETTING, HINTS_SETTING_TITLE, &format!("{HINTS_SETTING_DESCRIPTION}."), Bool),
        spec("general.keep_history", "Сохранять историю", "Записывать локально только пары исправленных слов.", Bool),
        number(
            spec("history.limit", "Размер истории", "Максимальное число локально сохранённых исправлений.", Int),
            HISTORY_LIMIT_SETTING_MIN,
            HISTORY_LIMIT_SETTING_MAX,
            HISTORY_LIMIT_SETTING_STEP,
        ),
    ]
}

#[must_use]
pub fn update_settings() -> Vec<SettingSpec> {
    vec![
        spec(
            "updates.check_automatically",
            "Автоматически проверять обновления",
            &format!(
                "Проверять GitHub Releases после запуска и раз в {}.",
                quantity(UPDATE_CHECK_INTERVAL_MS.to::<f64>() / 1000.0 / 3600.0, HOURS)
            ),
            Bool,
        ),
        spec(
            "updates.install_automatically",
            "Автоматически устанавливать обновления",
            "Скачать проверенный Setup EXE, тихо обновить KeySwitch и перезапустить его.",
            Bool,
        ),
    ]
}

#[must_use]
pub fn diagnostic_settings() -> Vec<SettingSpec> {
    vec![spec(
        "diagnostics.technical_logging",
        "Записывать технический журнал",
        "Сохранять решения распознавания, оценки и названия приложений. Журнал может содержать введённые слова; в приложениях-исключениях текст скрывается.",
        Bool,
    )]
}

#[must_use]
pub fn application_settings() -> Vec<SettingSpec> {
    MENTION_HEADS.iter().map(|head| spec(head.setting, head.title, head.description, Bool)).collect()
}

// ------------------------------------------------------------------------------------------------ reading settings

/// Python's truth value of a JSON value, as `bool(self.settings.get(path, ...))` reads it.
#[must_use]
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(flag) => *flag,
        Value::Null => false,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

/// Python's `str(value)` of a settings value.
#[must_use]
pub fn py_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Null => "None".to_owned(),
        other @ (Value::Number(_) | Value::Array(_) | Value::Object(_)) => other.to_string(),
    }
}

/// The text a control shows for the stored value: a choice shows its label.
pub fn control_text(spec: &SettingSpec, stored: Option<&Value>) -> String {
    let text = stored.map_or_else(String::new, py_str);
    if spec.kind == Choice {
        return spec.choices.iter().find(|(key, _)| *key == text).map_or(text, |(_, label)| label.clone());
    }
    text
}

/// `_save_choice`: the stored value of the label that was picked; an unknown label is stored as it is.
#[must_use]
pub fn choice_value(spec: &SettingSpec, selected: &str) -> String {
    spec.choices.iter().find(|(_, label)| label == selected).map_or_else(|| selected.to_owned(), |(key, _)| key.clone())
}

/// What `_save_number` stores and shows for the text of a number box; None when the text is no number, which restores
/// the stored value.
#[must_use]
pub fn parse_number(spec: &SettingSpec, text: &str) -> Option<(Value, String)> {
    let cleaned = pystr::strip(text).replace(',', ".");
    let mut numeric: f64 = cleaned.parse().ok()?;
    if numeric.is_nan() {
        // max(minimum, min(maximum, nan)) is the maximum in Python.
        numeric = spec.maximum;
    }
    let numeric = spec.minimum.max(numeric.min(spec.maximum));
    if spec.kind == Int {
        let value = numeric.round_ties_even().to::<i64>();
        return Some((json!(value), value.to_string()));
    }
    let scale = 10f64.powi(NUMBER_DISPLAY_DECIMAL_PLACES);
    let value = (numeric * scale).round_ties_even() / scale;
    Some((json!(value), format!("{value:?}")))
}

/// The value one press of a spin arrow gives: the shown number plus `steps` of the setting's step.
#[must_use]
pub fn stepped_text(spec: &SettingSpec, shown: &str, steps: i32) -> String {
    let cleaned = pystr::strip(shown).replace(',', ".");
    let current: f64 = cleaned.parse().unwrap_or(spec.minimum);
    let next = (current + spec.step * f64::from(steps)).clamp(spec.minimum, spec.maximum);
    if spec.kind == Int { format!("{}", next.round_ties_even().to::<i64>()) } else { format!("{:?}", (next * 100.0).round_ties_even() / 100.0) }
}

// ------------------------------------------------------------------------------------------------------ exclusions

/// `_string_list`: the strings of a list setting that are not blank.
pub fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items.iter().map(py_str).filter(|item| !pystr::strip(item).is_empty()).collect(),
        _ => Vec::new(),
    }
}

/// `sorted(items, key=str.casefold)`.
#[must_use]
pub fn sorted_casefold(mut items: Vec<String>) -> Vec<String> {
    items.sort_by_cached_key(|item| pystr::casefold(item));
    items
}

/// `_add_application`: the list with the program added, or None when there is nothing to store.
#[must_use]
pub fn with_application(mut applications: Vec<String>, identifier: &str) -> Option<Vec<String>> {
    let normalized = pystr::casefold(pystr::strip(identifier));
    if normalized.is_empty() || applications.iter().any(|item| pystr::casefold(item) == normalized) {
        return None;
    }
    applications.push(normalized);
    Some(applications)
}

/// `_add_word`: the list with the word added, or None when it is blank or already there.
#[must_use]
pub fn with_word(mut words: Vec<String>, word: &str) -> Option<Vec<String>> {
    let word = pystr::strip(word);
    if word.is_empty() || words.iter().any(|item| pystr::casefold(item) == pystr::casefold(word)) {
        return None;
    }
    words.push(word.to_owned());
    Some(words)
}

/// `_remove_application` and `_remove_word`: the list without the value, compared by case folding.
#[must_use]
pub fn without(items: Vec<String>, value: &str) -> Vec<String> {
    let folded = pystr::casefold(value);
    items.into_iter().filter(|item| pystr::casefold(item) != folded).collect()
}

/// The label of a catalog entry in the combo box: `name — identifier`.
#[must_use]
pub fn catalog_label(name: &str, identifier: &str) -> String {
    format!("{name} — {identifier}")
}

// --------------------------------------------------------------------------------------------- status and listings

/// The engine's status line (`_apply_snapshot`).
#[must_use]
pub fn status_line(enabled: bool, running: bool) -> &'static str {
    if enabled && running {
        "Автокоррекция включена"
    } else if !enabled {
        "Автокоррекция на паузе"
    } else {
        "Движок остановлен"
    }
}

/// A history row (`_insert_history`): time, was, became, application, confidence.
#[must_use]
pub fn history_row(entry: &HistoryEntry) -> [String; 5] {
    [
        entry.timestamp.clone(),
        entry.original.clone(),
        entry.replacement.clone(),
        entry.application.clone(),
        format!("{:.2}", entry.confidence),
    ]
}

/// A row of the rules list (`_refresh_learning_rules`).
#[must_use]
pub fn rule_row(rule: &LearnedRule) -> [String; 4] {
    [rule.pattern.clone(), other_layout_text(&rule.pattern, rule.source_group), condition_text(rule), action_text(rule)]
}

#[must_use]
pub fn learning_count_text(count: usize) -> String {
    if count > 0 { format!("Правил: {count}") } else { "Правил пока нет".to_owned() }
}

/// `_refresh_log_state`: whether the journal is really being written, and how large. `status` is `log_status()`.
pub fn log_state_text(status: &Map<String, Value>, path: &str, rotation: &str) -> String {
    let size = status.get("size").and_then(Value::as_i64).unwrap_or(-1);
    let installed = status.get("installed").is_some_and(truthy);
    let state = if !installed {
        "журнал не ведётся: обработчик не установлен".to_owned()
    } else if size < 0 {
        "журнал не ведётся: файл недоступен".to_owned()
    } else if size == 0 {
        "файл пуст: записей ещё нет".to_owned()
    } else {
        format!("записывается, {:.1} КБ", size.to::<f64>() / 1024.0)
    };
    format!("Файл журнала: {path}\nСостояние: {state}\nРотация в режиме диагностики: {rotation}, включение начинает новый файл")
}

#[must_use]
pub fn locations_text(settings: &str, history: &str, rules: &str, log: &str) -> String {
    format!("Настройки: {settings}\nИстория: {history}\nПравила переключения: {rules}\nЖурнал: {log}")
}

#[must_use]
pub fn capture_hint_text() -> String {
    format!(
        "После закрытия подсказки переключитесь на нужное приложение. KeySwitch определит его через {}.",
        quantity(f64::from(crate::constants::APPLICATION_CAPTURE_DELAY_MS) / 1000.0, SECONDS)
    )
}

// ------------------------------------------------------------------------------------------------------ diagnostics

/// What the About page's report is made of.
pub struct DiagnosticsInput {
    pub version: String,
    pub available: bool,
    pub session_type: String,
    pub display: String,
    pub record_version: String,
    pub xtest_version: String,
    pub xkb_version: String,
    pub current_group: i64,
    pub current_layout: String,
    pub intent_model: Value,
    pub context_model: Map<String, Value>,
    pub prefix_model: Map<String, Value>,
    pub autostart: Value,
    pub technical_logging: bool,
    pub technical_log: String,
    pub log: Map<String, Value>,
    pub settings: String,
    pub data: String,
    pub error: String,
}

/// `_diagnostics`: the report as indented JSON with the Russian text left readable.
#[must_use]
pub fn diagnostics_text(input: &DiagnosticsInput, mode: Value, read_field: Value, last_action: &str, early_switch: Value) -> String {
    let mut context = input.context_model.clone();
    context.insert("mode".into(), mode);
    context.insert("read_field".into(), read_field);
    context.insert("last_action".into(), json!(last_action));
    let mut prefix = input.prefix_model.clone();
    prefix.insert("early_switch".into(), early_switch);
    let payload = json!({
        "keyswitch": input.version,
        "available": input.available,
        "session_type": input.session_type,
        "desktop": input.display,
        "keyboard_hook": input.record_version,
        "injection": input.xtest_version,
        "layouts": input.xkb_version,
        "current_group": input.current_group,
        "current_layout": input.current_layout,
        "intent_model": input.intent_model,
        "context_model": context,
        "prefix_model": prefix,
        "autostart": input.autostart,
        "technical_logging": input.technical_logging,
        "technical_log": input.technical_log,
        "log": input.log,
        "settings": input.settings,
        "data": input.data,
        "error": input.error,
    });
    let options = ks_text::pyjson::JsonOptions {
        ensure_ascii: false,
        indent: Some(ks_text::pyjson::Indent::Spaces(2)),
        ..ks_text::pyjson::JsonOptions::default()
    };
    ks_text::pyjson::json_dumps(&payload, &options)
}

// --------------------------------------------------------------------------------------------------------- updates

/// The update manager as the window uses it (the Python's `UpdateManager`); `ks-updates` implements it.
pub use ks_updates::{UpdateController, UpdateObserver, UpdatePhase, UpdateSnapshot};

/// The controller used until the real one is wired in: it never finds an update.
pub struct NoUpdates {
    pub version: String,
}

impl UpdateController for NoUpdates {
    fn snapshot(&self) -> UpdateSnapshot {
        UpdateSnapshot {
            phase: UpdatePhase::Idle,
            message: "Проверка обновлений недоступна в этой сборке".to_owned(),
            current_version: self.version.clone(),
            available_version: String::new(),
            release_url: String::new(),
            progress: 0,
        }
    }

    fn subscribe(&self, _callback: UpdateObserver) {}

    fn check(&self, _automatic: bool, _install_automatically: bool) -> bool {
        false
    }

    fn install_available(&self) -> bool {
        false
    }

    fn installation_failed(&self, _error: &dyn fmt::Display) {}
}

/// What `_apply_update_snapshot` shows for a snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateView {
    pub status: String,
    pub version: String,
    pub progress: String,
    pub check_enabled: bool,
    pub install_enabled: bool,
    pub release_enabled: bool,
}

#[must_use]
pub fn update_view(snapshot: &UpdateSnapshot) -> UpdateView {
    let busy = matches!(snapshot.phase, UpdatePhase::Checking | UpdatePhase::Downloading);
    UpdateView {
        status: snapshot.message.clone(),
        version: if snapshot.available_version.is_empty() {
            format!("Установлена версия {}", snapshot.current_version)
        } else {
            format!("Доступна версия {}", snapshot.available_version)
        },
        progress: if snapshot.phase == UpdatePhase::Downloading { format!("Загрузка: {}%", snapshot.progress) } else { String::new() },
        check_enabled: !busy,
        install_enabled: snapshot.phase == UpdatePhase::Available,
        release_enabled: !snapshot.release_url.is_empty(),
    }
}

// ------------------------------------------------------------------------------------------------------------ theme

/// `dark_theme`: the user's choice, or the system's when the choice is left to it; an unknown value reads as the system's.
pub fn dark_theme(theme: &str, system_dark: impl FnOnce() -> bool) -> bool {
    match theme {
        "light" => false,
        "dark" => true,
        _ => system_dark(),
    }
}

/// `dark_from_apps_use_light_theme`: 0 asks apps to be dark; anything else, and no value, is light.
#[must_use]
pub fn dark_from_apps_use_light_theme(value: Option<i64>) -> bool {
    value == Some(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// A Win32 COLORREF.
    #[must_use]
    pub fn colorref(self) -> u32 {
        u32::from(self.0) | u32::from(self.1) << 8 | u32::from(self.2) << 16
    }
}

/// The colours `_apply_theme` sets for each of the two schemes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub dark: bool,
    pub background: Rgb,
    pub content: Rgb,
    pub sidebar: Rgb,
    pub foreground: Rgb,
    pub muted: Rgb,
    pub accent: Rgb,
    pub navigation_hover: Rgb,
    pub error: Rgb,
    pub sidebar_text: Rgb,
    pub sidebar_muted: Rgb,
    /// Lines of a section frame and a field's border.
    pub line: Rgb,
    /// The face of an edit box and of a push button.
    pub field: Rgb,
}

#[must_use]
pub fn palette(dark: bool) -> Palette {
    let (background, content, sidebar, foreground, muted, line, field) = if dark {
        (Rgb(0x18, 0x1a, 0x1f), Rgb(0x20, 0x23, 0x2a), Rgb(0x11, 0x13, 0x18), Rgb(0xf4, 0xf6, 0xfb), Rgb(0xae, 0xb5, 0xc4), Rgb(0x3a, 0x3f, 0x4b), Rgb(0x2b, 0x2f, 0x38))
    } else {
        (Rgb(0xf4, 0xf6, 0xf9), Rgb(0xff, 0xff, 0xff), Rgb(0x18, 0x23, 0x3a), Rgb(0x18, 0x20, 0x33), Rgb(0x66, 0x70, 0x85), Rgb(0xd0, 0xd5, 0xdd), Rgb(0xff, 0xff, 0xff))
    };
    Palette {
        dark,
        background,
        content,
        sidebar,
        foreground,
        muted,
        accent: Rgb(0x4f, 0x7c, 0xff),
        navigation_hover: Rgb(0x29, 0x39, 0x57),
        error: Rgb(0xe5, 0x48, 0x4d),
        sidebar_text: Rgb(0xff, 0xff, 0xff),
        sidebar_muted: Rgb(0xae, 0xbb, 0xd1),
        line,
        field,
    }
}

/// The learning prompt's colours (`WindowsLearningPrompt`), the same in both schemes.
pub const PROMPT_BORDER: Rgb = Rgb(0x76, 0x57, 0xff);
pub const PROMPT_CARD: Rgb = Rgb(0x17, 0x1a, 0x21);
pub const PROMPT_TITLE_COLOR: Rgb = Rgb(0xff, 0xff, 0xff);
pub const PROMPT_WORD_COLOR: Rgb = Rgb(0x9a, 0x84, 0xff);
pub const PROMPT_HINT_COLOR: Rgb = Rgb(0xb7, 0xbd, 0xc9);

/// Where the prompt goes: centred over the caret, above it, kept inside the virtual screen. `screen` is
/// `(x, y, width, height)` of the virtual screen.
#[must_use]
pub fn prompt_position(anchor: (i32, i32), size: (i32, i32), screen: (i32, i32, i32, i32), gap: i32, margin: i32) -> (i32, i32) {
    let (width, height) = size;
    let x = anchor.0 - width / 2;
    let y = anchor.1 - height - gap;
    let (left, top, screen_width, screen_height) = screen;
    // max(low, min(value, high)) as the Python writes it: the lower bound wins when the screen is too small.
    let x = (left + margin).max(x.min(left + screen_width - width - margin));
    let y = (top + margin).max(y.min(top + screen_height - height - margin));
    (x, y)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use ks_config::config::default_settings;

    #[test]
    fn python_truth_and_python_text_of_every_kind_of_value() {
        for (value, truth, text) in [
            (json!(null), false, "None"),
            (json!(0), false, "0"),
            (json!(2.5), true, "2.5"),
            (json!(""), false, ""),
            (json!(["a"]), true, r#"["a"]"#),
            (json!({}), false, "{}"),
            (json!(true), true, "True"),
            (json!(false), false, "False"),
        ] {
            assert_eq!(truthy(&value), truth, "{value}");
            assert_eq!(py_str(&value), text, "{value}");
        }
    }

    #[test]
    fn controls_lists_rows_and_colours() {
        let plain = SettingSpec { path: "a".to_owned(), title: "t".to_owned(), description: "d".to_owned(), kind: Int, choices: Vec::new(), minimum: 1.0, maximum: 9.0, step: 1.0 };
        assert_eq!(control_text(&plain, Some(&json!(3))), "3");
        assert_eq!(parse_number(&plain, "nan").map(|(_, text)| text), Some("9".to_owned()));
        assert!(string_list(Some(&json!("a"))).is_empty() && string_list(None).is_empty());
        assert_eq!(with_word(vec!["a".to_owned()], " "), None);
        assert_eq!(catalog_label("Name", "id"), "Name — id");
        let row = history_row(&HistoryEntry::with_timestamp("now".to_owned(), "a", "b", "app", 0.5));
        assert_eq!(row[4], "0.50");
        assert_eq!(rule_row(&LearnedRule::new("ghbdtn", 0, 1))[0], "ghbdtn");
        assert!(locations_text("s", "h", "r", "l").contains("Журнал: l"));
        assert!(!palette(false).dark && palette(true).dark);
    }

    #[test]
    fn a_build_without_updates_does_nothing_when_asked() {
        let updates = NoUpdates { version: "1".to_owned() };
        updates.subscribe(std::sync::Arc::new(|_| {}));
        assert!(!updates.check(true, true) && !updates.install_available());
        updates.installation_failed(&"x");
        assert_eq!(updates.snapshot().current_version, "1");
    }

    fn find<'a>(specs: &'a [SettingSpec], path: &str) -> &'a SettingSpec {
        specs.iter().find(|spec| spec.path == path).unwrap()
    }

    #[test]
    fn every_setting_the_window_offers_exists_in_the_defaults() {
        let defaults = Value::Object(default_settings(Some("windows")));
        let lookup = |path: &str| path.split('.').try_fold(&defaults, |value, part| value.get(part)).is_some();
        let groups = [
            autocorrection_settings(),
            manual_switch_settings(),
            trigger_settings(),
            hotkey_settings(),
            system_settings(),
            update_settings(),
            diagnostic_settings(),
            application_settings(),
        ];
        let mut count = 0;
        for group in &groups {
            for spec in group {
                assert!(lookup(&spec.path), "{} has no default", spec.path);
                count += 1;
            }
        }
        assert_eq!(count, 18 + 1 + 6 + 7 + 11 + 2 + 1 + 1);
    }

    #[test]
    fn choices_keep_their_order_and_labels() {
        let settings = autocorrection_settings();
        let catch_up = find(&settings, "detection.field_catch_up");
        assert_eq!(catch_up.choices.first().unwrap(), &("off".to_owned(), "Не ждать".to_owned()));
        assert_eq!(catch_up.choices.last().unwrap().0, "1000");
        assert_eq!(control_text(catch_up, Some(&json!("300"))), "0,3 с");
        assert_eq!(choice_value(catch_up, "0,5 с"), "500");
        assert_eq!(choice_value(catch_up, "unknown"), "unknown");
    }

    #[test]
    fn texts_built_from_numbers_read_as_the_python_writes_them() {
        let hotkeys = hotkey_settings();
        assert!(find(&hotkeys, "hotkeys.undo").description.ends_with("Доступно 10 секунд после исправления."));
        assert!(find(&update_settings(), "updates.check_automatically").description.ends_with("раз в 6 часов."));
        assert!(find(&autocorrection_settings(), "detection.early_switch_min_length").description.contains("значение 3 действует"));
        assert_eq!(hotkeys.len(), 7);
        let tap = find(&hotkeys, "detection.layout_switch_key");
        assert_eq!((tap.kind, control_text(tap, Some(&json!(""))), control_text(tap, Some(&json!("Shift_L")))), (Choice, "Выключено".to_owned(), "Левый Shift".to_owned()));
        assert_eq!(choice_value(tap, "Правый Ctrl"), "Control_R");
        // The texts name the conversion hotkey of this system: Pause has no key on a Mac.
        let named = |path: &str| find(&autocorrection_settings(), path).description.contains(&format!("{CONVERT_HOTKEY_NAME} "));
        assert!(named("detection.hold_after_caret_move") && CONVERT_HOTKEY_NAME == if cfg!(target_os = "macos") { "Control+Option+K" } else { "Pause" });
        assert!(find(&autocorrection_settings(), "detection.learning").description.contains(&format!("({CONVERT_HOTKEY_NAME})")));
        assert!(capture_hint_text().ends_with("через 3 секунды."));
    }

    #[test]
    fn numbers_are_clamped_rounded_to_even_and_shown_as_python_shows_them() {
        let settings = trigger_settings();
        let pause = find(&settings, "detection.pause_delay_seconds");
        assert_eq!(parse_number(pause, "1,25"), Some((json!(1.25), "1.25".to_owned())));
        assert_eq!(parse_number(pause, "0.1"), Some((json!(0.3), "0.3".to_owned())));
        assert_eq!(parse_number(pause, " 99 "), Some((json!(5.0), "5.0".to_owned())));
        assert_eq!(parse_number(pause, "abc"), None);
        let system = system_settings();
        let limit = find(&system, "history.limit");
        assert_eq!(parse_number(limit, "10.5"), Some((json!(10), "10".to_owned())));
        assert_eq!(parse_number(limit, "12.5"), Some((json!(12), "12".to_owned())));
        assert_eq!(parse_number(limit, "13.5"), Some((json!(14), "14".to_owned())));
        assert_eq!(stepped_text(limit, "200", 1), "210");
        assert_eq!(stepped_text(pause, "0.7", -1), "0.6");
        assert_eq!(stepped_text(pause, "0.3", -1), "0.3");
    }

    #[test]
    fn exclusion_lists_compare_by_case_folding() {
        let stored = string_list(Some(&json!(["Code", " ", "  ", "Teams"])));
        assert_eq!(stored, ["Code", "Teams"]);
        assert_eq!(sorted_casefold(vec!["b".into(), "A".into(), "C".into()]), ["A", "b", "C"]);
        assert_eq!(with_application(vec!["code".into()], "  CODE "), None);
        assert_eq!(with_application(vec!["code".into()], " Teams "), Some(vec!["code".into(), "teams".into()]));
        assert_eq!(with_application(Vec::new(), "  "), None);
        assert_eq!(with_word(vec!["Привет".into()], "привет"), None);
        assert_eq!(with_word(Vec::new(), " Слово "), Some(vec!["Слово".into()]));
        assert_eq!(without(vec!["Code".into(), "x".into()], "code"), ["x"]);
    }

    #[test]
    fn status_and_log_texts() {
        assert_eq!(status_line(true, true), "Автокоррекция включена");
        assert_eq!(status_line(false, true), "Автокоррекция на паузе");
        assert_eq!(status_line(true, false), "Движок остановлен");
        assert_eq!(learning_count_text(0), "Правил пока нет");
        assert_eq!(learning_count_text(3), "Правил: 3");
        let mut status = Map::new();
        status.insert("installed".into(), json!(true));
        status.insert("size".into(), json!(2048));
        assert!(log_state_text(&status, "x.log", "5 МБ × 2 файлов").contains("Состояние: записывается, 2.0 КБ\n"));
        status.insert("size".into(), json!(0));
        assert!(log_state_text(&status, "x.log", "r").contains("файл пуст"));
        status.insert("size".into(), json!(-1));
        assert!(log_state_text(&status, "x.log", "r").contains("файл недоступен"));
        status.insert("installed".into(), json!(false));
        assert!(log_state_text(&status, "x.log", "r").contains("обработчик не установлен"));
    }

    #[test]
    fn update_view_follows_the_phase() {
        let mut snapshot = NoUpdates { version: "0.70.0".into() }.snapshot();
        let view = update_view(&snapshot);
        assert_eq!(view.version, "Установлена версия 0.70.0");
        assert!(view.check_enabled && !view.install_enabled && !view.release_enabled);
        snapshot.phase = UpdatePhase::Downloading;
        snapshot.progress = 40;
        snapshot.available_version = "0.71.0".into();
        snapshot.release_url = "https://example.invalid".into();
        let view = update_view(&snapshot);
        assert_eq!((view.version.as_str(), view.progress.as_str()), ("Доступна версия 0.71.0", "Загрузка: 40%"));
        assert!(!view.check_enabled && !view.install_enabled && view.release_enabled);
        snapshot.phase = UpdatePhase::Available;
        assert!(update_view(&snapshot).install_enabled);
    }

    #[test]
    fn the_theme_follows_the_choice_and_asks_the_system_only_when_left_to_it() {
        assert!(dark_theme("dark", || panic!("not asked")));
        assert!(!dark_theme("light", || panic!("not asked")));
        assert!(dark_theme("system", || true));
        assert!(dark_theme("anything", || true));
        assert!(dark_from_apps_use_light_theme(Some(0)));
        assert!(!dark_from_apps_use_light_theme(Some(1)));
        assert!(!dark_from_apps_use_light_theme(None));
        assert_eq!(palette(true).content, Rgb(0x20, 0x23, 0x2a));
        assert_eq!(Rgb(0x4f, 0x7c, 0xff).colorref(), 0x00ff_7c4f);
    }

    #[test]
    fn the_prompt_sits_above_the_caret_inside_the_screen() {
        let screen = (0, 0, 1920, 1080);
        assert_eq!(prompt_position((500, 400), (390, 80), screen, 12, 8), (305, 308));
        assert_eq!(prompt_position((5, 10), (390, 80), screen, 12, 8), (8, 8));
        assert_eq!(prompt_position((1900, 1070), (390, 80), screen, 12, 8), (1522, 978));
    }

    #[test]
    fn diagnostics_keep_the_pythons_keys_and_order() {
        let input = DiagnosticsInput {
            version: "0.70.0".into(),
            available: true,
            session_type: "win32".into(),
            display: "desktop".into(),
            record_version: "hook".into(),
            xtest_version: "SendInput".into(),
            xkb_version: "layouts".into(),
            current_group: 1,
            current_layout: "RU".into(),
            intent_model: json!({"status": "ok"}),
            context_model: Map::from_iter([("available".to_owned(), json!(true))]),
            prefix_model: Map::from_iter([("available".to_owned(), json!(false))]),
            autostart: json!({"effective": true}),
            technical_logging: false,
            technical_log: "log".into(),
            log: Map::new(),
            settings: "s".into(),
            data: "d".into(),
            error: String::new(),
        };
        let text = diagnostics_text(&input, json!("assist"), json!(false), "wait", json!(true));
        let keys: Vec<&str> = text.lines().filter(|line| line.starts_with("  \"")).map(|line| line.split('"').nth(1).unwrap()).collect();
        assert_eq!(
            keys,
            ["keyswitch", "available", "session_type", "desktop", "keyboard_hook", "injection", "layouts", "current_group", "current_layout",
             "intent_model", "context_model", "prefix_model", "autostart", "technical_logging", "technical_log", "log", "settings", "data", "error"]
        );
        assert!(text.contains("\"last_action\": \"wait\""));
    }
}

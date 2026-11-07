//! The hints: what sets each one off, where it shows, on which platforms, and what it says. Every name a text gives is
//! the constant the settings window or the tray menu shows it by, and every hotkey is the one in the settings.

use ks_config::constants::{HINTS_SETTING, HOTKEY_SELECTION_CASE, HOTKEY_SELECTION_LAYOUT, HOTKEY_SELECTION_TRANSLITERATE};
use ks_config::indicator::layout_label;
use ks_context::typo_correction::CORRECT_TYPOS_TITLE;
use ks_engine::CorrectionPlan;
use ks_engine::constants::UNDO_AVAILABLE_WINDOW_SECONDS;
use ks_text::pystr;
use ks_text::russian_text::{SECONDS, quantity};
use ks_updates::{RELEASE_PAGE_PREFIX, SemanticVersion};

use crate::constants::{
    AUTOMATIC_CORRECTIONS_FOR_HISTORY_HINT, AUTOMATIC_CORRECTIONS_FOR_NOTIFICATIONS_HINT, AUTOMATIC_CORRECTIONS_FOR_PAUSE_HINT,
    HINTS_INTRODUCED_VERSION, LAYOUT_SWITCH_KEY_VERSION, MANUAL_CONVERSIONS_FOR_SELECTION_HINT,
};
use crate::tray_model::{EXCLUSIONS_LABEL, HISTORY_LABEL, NOTIFICATIONS_LABEL, SOUND_LABEL};
use crate::windows_ui_model::{EARLY_SWITCH_TITLE, HINTS_SETTING_TITLE, MAC_SYSTEM_PAGE_TITLE, PAGE_NAMES};

/// What every hint card says on top and on its two buttons.
pub const HINT_TITLE: &str = "KeySwitch";
pub const HINT_OK_LABEL: &str = "Понятно";
pub const HINT_NEVER_LABEL: &str = "Больше не показывать подсказки";
/// The button of the Windows 11 taskbar behind which it puts the icons of new programs (README, «Установка в Windows»).
pub const WINDOWS_HIDDEN_ICONS_BUTTON: &str = "Показать скрытые значки";
/// `detection.layout_switch_key` of 0.70.0 and its title in every settings window, on the hotkeys page.
pub const LAYOUT_SWITCH_KEY_SETTING: &str = "detection.layout_switch_key";
pub const LAYOUT_SWITCH_KEY_TITLE: &str = "Переключать раскладку одной клавишей";

/// The modes of `CorrectionPlan` the triggers tell apart.
pub const MODE_EARLY: &str = "early";
pub const MODE_TYPO: &str = "typo";
pub const MODE_MANUAL: &str = "manual";
pub const MODE_SYMBOLS: &str = "symbols";
pub const MODE_UNDO: &str = "undo";
/// Automatic corrections that are no plain change of the layout of a finished word: they have hints of their own.
const NOT_LAYOUT_MODES: [&str; 3] = [MODE_EARLY, MODE_TYPO, MODE_SYMBOLS];
/// The manual conversions: Pause on a word, and on signs alone.
const MANUAL_MODES: [&str; 2] = [MODE_MANUAL, MODE_SYMBOLS];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Linux,
    MacOs,
}

pub const EVERY_PLATFORM: &[Platform] = &[Platform::Windows, Platform::Linux, Platform::MacOs];

/// The platform this build runs on.
#[cfg(windows)]
pub const RUNNING_PLATFORM: Platform = Platform::Windows;
#[cfg(target_os = "macos")]
pub const RUNNING_PLATFORM: Platform = Platform::MacOs;
#[cfg(not(any(windows, target_os = "macos")))]
pub const RUNNING_PLATFORM: Platform = Platform::Linux;

/// A page of the settings window, as every platform has it. The GTK window names its pages otherwise and maps them itself;
/// the Windows and macOS windows use [`SettingsPage::window_page`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsPage {
    Overview,
    Autocorrection,
    Hotkeys,
    Exclusions,
    System,
    History,
}

impl SettingsPage {
    fn window_index(self) -> usize {
        match self {
            SettingsPage::Overview => 0,
            SettingsPage::Autocorrection => 1,
            SettingsPage::Hotkeys => 3,
            SettingsPage::Exclusions => 4,
            SettingsPage::System => 5,
            SettingsPage::History => 6,
        }
    }

    /// The page's name in the Windows and the macOS settings window (`PAGE_NAMES`).
    #[must_use]
    pub fn window_page(self) -> &'static str {
        PAGE_NAMES[self.window_index()].0
    }

    /// The page's title on `platform`: the GTK window has the titles of the Windows one, the macOS window calls the
    /// system page by its own.
    #[must_use]
    pub fn title(self, platform: Platform) -> &'static str {
        if self == SettingsPage::System && platform == Platform::MacOs {
            return MAC_SYSTEM_PAGE_TITLE;
        }
        PAGE_NAMES[self.window_index()].1
    }
}

/// Where a hint shows: above the caret of the field typed in, near the tray icon (the menu bar item, a notification on
/// Linux), or as a card on top of a page of the settings window, with `control` (a settings path) marked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Caret,
    Tray,
    Settings { page: SettingsPage, control: Option<&'static str> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// About what the program can do, for everyone, set off by what happens.
    Intro,
    /// About a release, after an update from an older one.
    WhatsNew,
}

/// What a hint of the counters waits for besides them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Condition {
    NoManualConversion,
    Notifications,
    KeepHistory,
}

/// What sets a hint off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// A new installation, once its engine runs.
    FirstStart,
    /// An automatic change of the layout of a word: no case fix, early switch, typo or sign.
    LayoutCorrection,
    /// A correction of this mode.
    Mode(&'static str),
    /// A manual conversion of a word; the hint counts as found out when a rule prompt follows it.
    ManualConversion,
    /// The `at_least`-th automatic correction, or any after it, while `condition` holds.
    AutomaticCorrections { at_least: i64, condition: Condition },
    /// The `at_least`-th manual conversion, or any after it.
    ManualConversions { at_least: i64 },
    /// Its page of the settings window shown.
    SettingsPage,
    /// An update from a version older than the hint's.
    WhatsNew,
    /// Any update: once for every version updated to.
    Updated,
}

/// What a click on the text of a hint opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Settings { page: SettingsPage, setting: Option<&'static str> },
    ReleaseNotes,
}

/// A hotkey a text names; a hint whose hotkey is cleared in the settings is not shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyName {
    Undo,
    ConvertLast,
    SelectionLayout,
    SelectionCase,
    SelectionTransliterate,
}

impl HotkeyName {
    pub const ALL: [HotkeyName; 5] = [
        HotkeyName::Undo,
        HotkeyName::ConvertLast,
        HotkeyName::SelectionLayout,
        HotkeyName::SelectionCase,
        HotkeyName::SelectionTransliterate,
    ];

    /// Its name under `hotkeys` in the settings.
    #[must_use]
    pub fn setting(self) -> &'static str {
        match self {
            HotkeyName::Undo => "undo",
            HotkeyName::ConvertLast => "convert_last",
            HotkeyName::SelectionLayout => HOTKEY_SELECTION_LAYOUT,
            HotkeyName::SelectionCase => HOTKEY_SELECTION_CASE,
            HotkeyName::SelectionTransliterate => HOTKEY_SELECTION_TRANSLITERATE,
        }
    }
}

/// The hotkeys of the settings, as written there (`Ctrl+Alt+Z`); empty for one that is cleared.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hotkeys {
    pub undo: String,
    pub convert_last: String,
    pub selection_layout: String,
    pub selection_case: String,
    pub selection_transliterate: String,
}

impl Hotkeys {
    #[must_use]
    pub fn get(&self, name: HotkeyName) -> &str {
        match name {
            HotkeyName::Undo => &self.undo,
            HotkeyName::ConvertLast => &self.convert_last,
            HotkeyName::SelectionLayout => &self.selection_layout,
            HotkeyName::SelectionCase => &self.selection_case,
            HotkeyName::SelectionTransliterate => &self.selection_transliterate,
        }
    }
}

/// A hotkey as a Mac keyboard has it: the settings window of the Mac shows `Ctrl+Alt+Z` as everywhere, and the keys are
/// Control and Option, so a chord of them gets that reading in brackets: `Ctrl+Alt+Z (Control+Option+Z)`. The pieces are
/// read as `Hotkey::new` of the engine reads them.
#[must_use]
pub fn mac_chord(hotkey: &str) -> String {
    let text = hotkey.replace('<', "").replace('>', "+");
    let reading: Vec<String> = text
        .split('+')
        .map(pystr::strip)
        .filter(|piece| !piece.is_empty())
        .map(|piece| match pystr::casefold(piece).as_str() {
            "ctrl" | "control" => "Control".to_owned(),
            "alt" => "Option".to_owned(),
            "super" | "meta" => "Command".to_owned(),
            _ => piece.to_owned(),
        })
        .collect();
    let reading = reading.join("+");
    if reading == hotkey { reading } else { format!("{hotkey} ({reading})") }
}

/// What a text is made of besides constants: the platform, the version running and the hotkeys of the settings.
#[derive(Clone, Copy, Debug)]
pub struct Wording<'a> {
    pub platform: Platform,
    pub version: SemanticVersion,
    pub hotkeys: &'a Hotkeys,
}

impl Wording<'_> {
    fn key(&self, name: HotkeyName) -> String {
        let key = self.hotkeys.get(name);
        if self.platform == Platform::MacOs { mac_chord(key) } else { key.to_owned() }
    }

    fn page(&self, page: SettingsPage) -> &'static str {
        page.title(self.platform)
    }
}

/// One hint of the catalogue.
#[derive(Clone, Copy, Debug)]
pub struct Hint {
    /// Its name in `hints.json`; never changed once released.
    pub id: &'static str,
    pub group: Group,
    /// The release the hint came with, or for news the release of what it tells about; none for the news of any update.
    pub version: Option<&'static str>,
    pub trigger: Trigger,
    /// Where it may show; the first that is not in the settings window is where the trigger shows it.
    pub places: &'static [Place],
    pub platforms: &'static [Platform],
    pub opens: Option<Destination>,
    /// The hotkeys its text names.
    pub hotkeys: &'static [HotkeyName],
    pub(crate) text: fn(&Wording<'_>) -> String,
}

impl Hint {
    /// Its text, or None when a hotkey it names is cleared in the settings.
    #[must_use]
    pub fn text(&self, wording: &Wording<'_>) -> Option<String> {
        if self.hotkeys.iter().any(|name| pystr::strip(wording.hotkeys.get(*name)).is_empty()) {
            return None;
        }
        Some((self.text)(wording))
    }

    #[must_use]
    pub fn shows_on(&self, platform: Platform) -> bool {
        self.platforms.contains(&platform)
    }

    #[must_use]
    pub fn release(&self) -> Option<SemanticVersion> {
        self.version.map(|version| SemanticVersion::parse(version).expect("a test parses every version of the catalogue"))
    }

    /// Where the trigger shows it: its first place outside the settings window.
    #[must_use]
    pub fn shown_at(&self) -> Option<Place> {
        self.places.iter().copied().find(|place| !matches!(place, Place::Settings { .. }))
    }

    /// Its card on `page` of the settings window, if it has one.
    #[must_use]
    pub fn card_on(&self, page: SettingsPage) -> Option<Place> {
        self.places.iter().copied().find(|place| matches!(place, Place::Settings { page: shown, .. } if *shown == page))
    }

    /// Whether this correction sets the hint off, with the corrections counted so far (this one included).
    #[must_use]
    pub fn fired_by(&self, correction: CorrectionInfo, counted: Counted, holds: impl Fn(Condition) -> bool) -> bool {
        match self.trigger {
            Trigger::LayoutCorrection => {
                correction.automatic && !correction.case_fix && !NOT_LAYOUT_MODES.contains(&correction.mode)
            }
            Trigger::Mode(mode) => correction.mode == mode,
            Trigger::ManualConversion => correction.mode == MODE_MANUAL,
            Trigger::AutomaticCorrections { at_least, condition } => {
                correction.automatic && counted.automatic_corrections >= at_least && holds(condition)
            }
            Trigger::ManualConversions { at_least } => correction.manual() && counted.manual_conversions >= at_least,
            Trigger::FirstStart | Trigger::SettingsPage | Trigger::WhatsNew | Trigger::Updated => false,
        }
    }

    /// How far the counters of corrections must go for this hint: a hint that waits for no manual conversion must
    /// know of one.
    #[must_use]
    pub fn needs(&self) -> Counted {
        match self.trigger {
            Trigger::AutomaticCorrections { at_least, condition } => Counted {
                automatic_corrections: at_least,
                manual_conversions: i64::from(condition == Condition::NoManualConversion),
            },
            Trigger::ManualConversions { at_least } => Counted { automatic_corrections: 0, manual_conversions: at_least },
            Trigger::FirstStart
            | Trigger::LayoutCorrection
            | Trigger::Mode(_)
            | Trigger::ManualConversion
            | Trigger::SettingsPage
            | Trigger::WhatsNew
            | Trigger::Updated => Counted::default(),
        }
    }
}

/// What the hints need to know of a correction the engine made (`CorrectionPlan`), taken on the engine's thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CorrectionInfo {
    pub mode: &'static str,
    pub automatic: bool,
    pub case_fix: bool,
}

impl CorrectionInfo {
    #[must_use]
    pub fn of(plan: &CorrectionPlan) -> CorrectionInfo {
        CorrectionInfo { mode: plan.mode, automatic: plan.automatic, case_fix: plan.case_fix() }
    }

    /// A conversion the user asked for with the conversion hotkey.
    #[must_use]
    pub fn manual(&self) -> bool {
        MANUAL_MODES.contains(&self.mode)
    }
}

/// The corrections counted since the hints began (`hints.json`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counted {
    pub automatic_corrections: i64,
    pub manual_conversions: i64,
}

/// What a click on the text of a shown hint opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Opens {
    Settings { page: SettingsPage, setting: Option<&'static str> },
    Url(String),
}

impl Destination {
    #[must_use]
    pub fn resolve(self, version: SemanticVersion) -> Opens {
        match self {
            Destination::Settings { page, setting } => Opens::Settings { page, setting },
            Destination::ReleaseNotes => Opens::Url(format!("{RELEASE_PAGE_PREFIX}v{version}")),
        }
    }
}

// ------------------------------------------------------------------------------------------------------------- texts

fn first_start(wording: &Wording<'_>) -> String {
    let place = match wording.platform {
        Platform::Windows => format!("в области уведомлений или за стрелкой «{WINDOWS_HIDDEN_ICONS_BUTTON}»"),
        Platform::Linux => "на панели рабочего стола".to_owned(),
        Platform::MacOs => "в строке меню".to_owned(),
    };
    format!(
        "KeySwitch работает в фоне и сам исправит слово, набранное не в той раскладке. Значок {}/{} — {place}.",
        layout_label(0),
        layout_label(1)
    )
}

fn first_correction_undo(wording: &Wording<'_>) -> String {
    format!(
        "Слово перепечатано в другой раскладке. Если это лишнее, {} вернёт его как набрано. Доступно {} после исправления.",
        wording.key(HotkeyName::Undo),
        quantity(UNDO_AVAILABLE_WINDOW_SECONDS, SECONDS)
    )
}

fn early_switch(wording: &Wording<'_>) -> String {
    format!(
        "Раскладка сменилась прямо на полуслове: это «{EARLY_SWITCH_TITLE}» на странице «{}». Отменить — {}.",
        wording.page(SettingsPage::Autocorrection),
        wording.key(HotkeyName::Undo)
    )
}

fn typo(wording: &Wording<'_>) -> String {
    format!(
        "Исправлена опечатка, а не раскладка. Вернуть как набрано — {}. Настройка «{CORRECT_TYPOS_TITLE}» — на странице «{}».",
        wording.key(HotkeyName::Undo),
        wording.page(SettingsPage::Autocorrection)
    )
}

fn pause_converts(wording: &Wording<'_>) -> String {
    format!("Слово осталось не в той раскладке? {} перепечатает последнее слово в другой раскладке.", wording.key(HotkeyName::ConvertLast))
}

fn double_pause_rule(wording: &Wording<'_>) -> String {
    format!(
        "{} дважды подряд — KeySwitch предложит правило: всегда переводить это слово или всегда оставлять как есть.",
        wording.key(HotkeyName::ConvertLast)
    )
}

fn exclusions(_: &Wording<'_>) -> String {
    format!("Если в какой-то программе исправления мешают, добавьте её в исключения: меню значка → «{EXCLUSIONS_LABEL}».")
}

fn notifications_sound(_: &Wording<'_>) -> String {
    format!(
        "Уведомление о каждом исправлении можно выключить: меню значка → «{NOTIFICATIONS_LABEL}». Там же включается звук — «{SOUND_LABEL}»."
    )
}

fn history(_: &Wording<'_>) -> String {
    format!("Все исправления записаны: меню значка → «{HISTORY_LABEL}». Хранятся только пары слов, не весь набранный текст.")
}

fn selection_commands(wording: &Wording<'_>) -> String {
    format!(
        "Выделенный текст можно исправить: {} — раскладка, {} — регистр, {} — транслит.",
        wording.key(HotkeyName::SelectionLayout),
        wording.key(HotkeyName::SelectionCase),
        wording.key(HotkeyName::SelectionTransliterate)
    )
}

fn hints_intro(wording: &Wording<'_>) -> String {
    format!(
        "KeySwitch покажет по разу короткие подсказки о своих возможностях. Выключить: «{HINTS_SETTING_TITLE}» на странице «{}».",
        wording.page(SettingsPage::System)
    )
}

fn layout_switch_key(wording: &Wording<'_>) -> String {
    let release = SemanticVersion::parse(LAYOUT_SWITCH_KEY_VERSION).expect("a test parses every version of the catalogue");
    format!(
        "Новое в {}.{}: раскладку можно сменить одной клавишей — «{LAYOUT_SWITCH_KEY_TITLE}» на странице «{}». Выключено по умолчанию.",
        release.major,
        release.minor,
        wording.page(SettingsPage::Hotkeys)
    )
}

fn updated(wording: &Wording<'_>) -> String {
    format!("KeySwitch обновлён до {}. Настройки, правила и история сохранены.", wording.version)
}

// --------------------------------------------------------------------------------------------------------- catalogue

/// The hints in the order they are shown when several are due at once.
pub static CATALOGUE: [Hint; 13] = [
    Hint {
        id: "first_start",
        group: Group::Intro,
        version: Some(HINTS_INTRODUCED_VERSION),
        trigger: Trigger::FirstStart,
        places: &[Place::Tray],
        platforms: EVERY_PLATFORM,
        opens: None,
        hotkeys: &[],
        text: first_start,
    },
    Hint {
        id: "first_correction_undo",
        group: Group::Intro,
        version: Some(HINTS_INTRODUCED_VERSION),
        trigger: Trigger::LayoutCorrection,
        places: &[Place::Caret],
        platforms: EVERY_PLATFORM,
        opens: Some(Destination::Settings { page: SettingsPage::Hotkeys, setting: Some("hotkeys.undo") }),
        hotkeys: &[HotkeyName::Undo],
        text: first_correction_undo,
    },
    Hint {
        id: "early_switch",
        group: Group::Intro,
        version: Some(HINTS_INTRODUCED_VERSION),
        trigger: Trigger::Mode(MODE_EARLY),
        places: &[Place::Caret],
        platforms: EVERY_PLATFORM,
        opens: Some(Destination::Settings { page: SettingsPage::Autocorrection, setting: Some("detection.early_switch") }),
        hotkeys: &[HotkeyName::Undo],
        text: early_switch,
    },
    Hint {
        id: "typo",
        group: Group::Intro,
        version: Some(HINTS_INTRODUCED_VERSION),
        trigger: Trigger::Mode(MODE_TYPO),
        places: &[Place::Caret],
        platforms: EVERY_PLATFORM,
        opens: Some(Destination::Settings { page: SettingsPage::Autocorrection, setting: Some("detection.correct_typos") }),
        hotkeys: &[HotkeyName::Undo],
        text: typo,
    },
    Hint {
        id: "pause_converts",
        group: Group::Intro,
        version: Some(HINTS_INTRODUCED_VERSION),
        trigger: Trigger::AutomaticCorrections {
            at_least: AUTOMATIC_CORRECTIONS_FOR_PAUSE_HINT,
            condition: Condition::NoManualConversion,
        },
        places: &[Place::Caret],
        platforms: EVERY_PLATFORM,
        opens: Some(Destination::Settings { page: SettingsPage::Hotkeys, setting: Some("hotkeys.convert_last") }),
        hotkeys: &[HotkeyName::ConvertLast],
        text: pause_converts,
    },
    Hint {
        id: "double_pause_rule",
        group: Group::Intro,
        version: Some(HINTS_INTRODUCED_VERSION),
        trigger: Trigger::ManualConversion,
        places: &[Place::Caret],
        platforms: EVERY_PLATFORM,
        opens: Some(Destination::Settings { page: SettingsPage::Autocorrection, setting: Some("detection.learning") }),
        hotkeys: &[HotkeyName::ConvertLast],
        text: double_pause_rule,
    },
    Hint {
        id: "exclusions",
        group: Group::Intro,
        version: Some(HINTS_INTRODUCED_VERSION),
        trigger: Trigger::Mode(MODE_UNDO),
        places: &[Place::Caret],
        platforms: EVERY_PLATFORM,
        opens: Some(Destination::Settings { page: SettingsPage::Exclusions, setting: None }),
        hotkeys: &[],
        text: exclusions,
    },
    Hint {
        id: "notifications_sound",
        group: Group::Intro,
        version: Some(HINTS_INTRODUCED_VERSION),
        trigger: Trigger::AutomaticCorrections {
            at_least: AUTOMATIC_CORRECTIONS_FOR_NOTIFICATIONS_HINT,
            condition: Condition::Notifications,
        },
        places: &[Place::Tray],
        platforms: EVERY_PLATFORM,
        opens: Some(Destination::Settings { page: SettingsPage::System, setting: Some("general.notifications") }),
        hotkeys: &[],
        text: notifications_sound,
    },
    Hint {
        id: "history",
        group: Group::Intro,
        version: Some(HINTS_INTRODUCED_VERSION),
        trigger: Trigger::AutomaticCorrections { at_least: AUTOMATIC_CORRECTIONS_FOR_HISTORY_HINT, condition: Condition::KeepHistory },
        places: &[Place::Tray],
        platforms: EVERY_PLATFORM,
        opens: Some(Destination::Settings { page: SettingsPage::History, setting: None }),
        hotkeys: &[],
        text: history,
    },
    Hint {
        id: "selection_commands",
        group: Group::Intro,
        version: Some(HINTS_INTRODUCED_VERSION),
        trigger: Trigger::ManualConversions { at_least: MANUAL_CONVERSIONS_FOR_SELECTION_HINT },
        places: &[Place::Caret],
        platforms: EVERY_PLATFORM,
        opens: Some(Destination::Settings { page: SettingsPage::Hotkeys, setting: Some("hotkeys.selection_layout") }),
        hotkeys: &[HotkeyName::SelectionLayout, HotkeyName::SelectionCase, HotkeyName::SelectionTransliterate],
        text: selection_commands,
    },
    Hint {
        id: "hints_intro",
        group: Group::Intro,
        version: Some(HINTS_INTRODUCED_VERSION),
        trigger: Trigger::SettingsPage,
        places: &[Place::Settings { page: SettingsPage::Overview, control: None }],
        platforms: EVERY_PLATFORM,
        opens: Some(Destination::Settings { page: SettingsPage::System, setting: Some(HINTS_SETTING) }),
        hotkeys: &[],
        text: hints_intro,
    },
    Hint {
        id: "new_0_70_switch_layout_key",
        group: Group::WhatsNew,
        version: Some(LAYOUT_SWITCH_KEY_VERSION),
        trigger: Trigger::WhatsNew,
        places: &[Place::Tray, Place::Settings { page: SettingsPage::Hotkeys, control: Some(LAYOUT_SWITCH_KEY_SETTING) }],
        platforms: EVERY_PLATFORM,
        opens: Some(Destination::Settings { page: SettingsPage::Hotkeys, setting: Some(LAYOUT_SWITCH_KEY_SETTING) }),
        hotkeys: &[],
        text: layout_switch_key,
    },
    Hint {
        id: "updated",
        group: Group::WhatsNew,
        version: None,
        trigger: Trigger::Updated,
        places: &[Place::Tray],
        platforms: EVERY_PLATFORM,
        opens: Some(Destination::ReleaseNotes),
        hotkeys: &[],
        text: updated,
    },
];

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::constants::HINT_TEXT_MAX_CHARACTERS;
    use crate::windows_ui_model::{AUTOCORRECTION_PAGE_TITLE, HOTKEYS_PAGE_TITLE, SYSTEM_PAGE_TITLE};
    use ks_config::config::default_settings;
    use serde_json::Value;
    use std::collections::HashSet;

    fn shipped(platform: Platform) -> Hotkeys {
        let defaults = default_settings(Some(if platform == Platform::MacOs { "darwin" } else { "linux" }));
        let key = |name: HotkeyName| defaults["hotkeys"][name.setting()].as_str().unwrap().to_owned();
        Hotkeys {
            undo: key(HotkeyName::Undo),
            convert_last: key(HotkeyName::ConvertLast),
            selection_layout: key(HotkeyName::SelectionLayout),
            selection_case: key(HotkeyName::SelectionCase),
            selection_transliterate: key(HotkeyName::SelectionTransliterate),
        }
    }

    fn version() -> SemanticVersion {
        SemanticVersion::parse(ks_config::logsetup::VERSION).unwrap()
    }

    fn text_of(id: &str, platform: Platform, hotkeys: &Hotkeys) -> String {
        let hint = CATALOGUE.iter().find(|hint| hint.id == id).unwrap();
        hint.text(&Wording { platform, version: version(), hotkeys }).unwrap()
    }

    #[test]
    fn ids_are_unique_and_versions_parse_no_later_than_the_package() {
        let ids: HashSet<&str> = CATALOGUE.iter().map(|hint| hint.id).collect();
        assert_eq!(ids.len(), CATALOGUE.len());
        for hint in &CATALOGUE {
            // News of a release later than the package would never show; the release an intro hint came with is
            // the one this branch goes into.
            if let (Some(release), Group::WhatsNew) = (hint.release(), hint.group) {
                assert!(release <= version(), "{} is of a later release", hint.id);
            }
            assert_eq!(hint.version.is_none(), hint.trigger == Trigger::Updated, "{}", hint.id);
            assert_eq!(hint.group == Group::WhatsNew, matches!(hint.trigger, Trigger::WhatsNew | Trigger::Updated), "{}", hint.id);
            assert!(!hint.places.is_empty(), "{} shows nowhere", hint.id);
        }
    }

    #[test]
    fn every_platform_has_every_hint_and_a_settings_card_only_where_its_trigger_finds_it() {
        for platform in EVERY_PLATFORM {
            assert_eq!(CATALOGUE.iter().filter(|hint| hint.shows_on(*platform)).count(), CATALOGUE.len());
        }
        for hint in &CATALOGUE {
            let card = hint.places.iter().any(|place| matches!(place, Place::Settings { .. }));
            assert_eq!(card, matches!(hint.trigger, Trigger::SettingsPage | Trigger::WhatsNew), "{}", hint.id);
            // A trigger that fires on an event of the engine or of the start has a place outside the window.
            assert_eq!(hint.shown_at().is_none(), hint.trigger == Trigger::SettingsPage, "{}", hint.id);
        }
    }

    #[test]
    fn every_text_fits_and_has_nothing_left_to_fill() {
        for platform in EVERY_PLATFORM {
            let mut variants = vec![shipped(*platform)];
            // The macOS default of the conversion hotkey, Control+Option+K from 0.70.0 on, read as the Mac reads it.
            variants.push(Hotkeys { convert_last: "Ctrl+Alt+K".to_owned(), ..shipped(*platform) });
            for hotkeys in &variants {
                for hint in &CATALOGUE {
                    let text = hint.text(&Wording { platform: *platform, version: version(), hotkeys }).unwrap();
                    assert!(text.chars().count() <= HINT_TEXT_MAX_CHARACTERS, "{} on {platform:?}: {} characters", hint.id, text.chars().count());
                    assert!(!text.contains('{') && !text.contains('}'), "{text}");
                    for name in hint.hotkeys {
                        let key = hotkeys.get(*name);
                        assert!(text.contains(key), "{} names {key}", hint.id);
                    }
                }
            }
        }
    }

    #[test]
    fn texts_name_the_titles_the_windows_and_the_menu_show() {
        let windows = shipped(Platform::Windows);
        assert!(text_of("first_start", Platform::Windows, &windows).contains(&format!("«{WINDOWS_HIDDEN_ICONS_BUTTON}»")));
        assert!(text_of("first_start", Platform::Windows, &windows).contains("Значок EN/RU"));
        assert!(text_of("first_start", Platform::Linux, &windows).ends_with("на панели рабочего стола."));
        assert!(text_of("first_start", Platform::MacOs, &windows).ends_with("в строке меню."));
        let early = text_of("early_switch", Platform::Linux, &windows);
        assert!(early.contains(&format!("«{EARLY_SWITCH_TITLE}» на странице «{AUTOCORRECTION_PAGE_TITLE}»")), "{early}");
        assert!(text_of("typo", Platform::Windows, &windows).contains(&format!("«{CORRECT_TYPOS_TITLE}»")));
        assert!(text_of("exclusions", Platform::Windows, &windows).contains(&format!("«{EXCLUSIONS_LABEL}»")));
        let notifications = text_of("notifications_sound", Platform::Windows, &windows);
        assert!(notifications.contains(&format!("«{NOTIFICATIONS_LABEL}»")) && notifications.contains(&format!("«{SOUND_LABEL}»")));
        assert!(text_of("history", Platform::Windows, &windows).contains(&format!("«{HISTORY_LABEL}»")));
        assert!(text_of("hints_intro", Platform::Windows, &windows).contains(&format!("«{HINTS_SETTING_TITLE}» на странице «{SYSTEM_PAGE_TITLE}»")));
        assert!(text_of("hints_intro", Platform::MacOs, &windows).contains(&format!("на странице «{MAC_SYSTEM_PAGE_TITLE}»")));
        let news = text_of("new_0_70_switch_layout_key", Platform::Linux, &windows);
        assert!(news.starts_with("Новое в 0.70:") && news.contains(&format!("«{LAYOUT_SWITCH_KEY_TITLE}» на странице «{HOTKEYS_PAGE_TITLE}»")), "{news}");
        assert!(text_of("updated", Platform::Linux, &windows).contains(&format!("обновлён до {}.", version())));
        assert!(text_of("first_correction_undo", Platform::Linux, &windows).contains("Ctrl+Alt+Z вернёт его как набрано. Доступно 10 секунд"));
    }

    #[test]
    fn mac_texts_name_the_hotkey_of_the_settings_with_the_mac_keys() {
        let mac = shipped(Platform::MacOs);
        assert!(text_of("first_correction_undo", Platform::MacOs, &mac).contains("Ctrl+Alt+Z (Control+Option+Z) вернёт"));
        let configured = Hotkeys { convert_last: "Ctrl+Alt+K".to_owned(), ..mac.clone() };
        assert!(text_of("pause_converts", Platform::MacOs, &configured).contains("? Ctrl+Alt+K (Control+Option+K) перепечатает"));
        let other = Hotkeys { convert_last: "F13".to_owned(), ..mac.clone() };
        assert!(text_of("double_pause_rule", Platform::MacOs, &other).starts_with("F13 дважды подряд"));
        let selection = text_of("selection_commands", Platform::MacOs, &mac);
        assert!(selection.contains("Ctrl+Alt+L (Control+Option+L) — раскладка"), "{selection}");
        // Elsewhere the hotkey is named as the settings write it.
        assert!(text_of("pause_converts", Platform::Windows, &configured).contains("? Ctrl+Alt+K перепечатает"));
    }

    #[test]
    fn a_mac_chord_reads_control_option_and_command() {
        assert_eq!(mac_chord("Ctrl+Alt+Z"), "Ctrl+Alt+Z (Control+Option+Z)");
        assert_eq!(mac_chord("<ctrl><alt>k"), "<ctrl><alt>k (Control+Option+k)");
        assert_eq!(mac_chord("Super + Shift+X"), "Super + Shift+X (Command+Shift+X)");
        assert_eq!(mac_chord("Meta+Control+A"), "Meta+Control+A (Command+Control+A)");
        assert_eq!(mac_chord("Shift+Pause"), "Shift+Pause");
        assert_eq!(mac_chord("Pause"), "Pause");
    }

    #[test]
    fn a_hint_whose_hotkey_is_cleared_has_no_text() {
        let hint = CATALOGUE.iter().find(|hint| hint.id == "selection_commands").unwrap();
        let cleared = Hotkeys { selection_case: "  ".to_owned(), ..shipped(Platform::Linux) };
        assert_eq!(hint.text(&Wording { platform: Platform::Linux, version: version(), hotkeys: &cleared }), None);
    }

    #[test]
    fn pages_have_the_names_of_each_window() {
        let pages = [
            (SettingsPage::Overview, "overview", "Обзор"),
            (SettingsPage::Autocorrection, "autocorrection", "Автокоррекция"),
            (SettingsPage::Hotkeys, "hotkeys", "Горячие клавиши"),
            (SettingsPage::Exclusions, "exclusions", "Исключения"),
            (SettingsPage::System, "appearance", "Внешний вид и система"),
            (SettingsPage::History, "history", "История"),
        ];
        for (page, name, title) in pages {
            assert_eq!(page.window_page(), name);
            assert_eq!(page.title(Platform::Windows), title);
            assert_eq!(page.title(Platform::Linux), title);
        }
        assert_eq!(SettingsPage::System.title(Platform::MacOs), "Внешний вид");
        assert_eq!(SettingsPage::Hotkeys.title(Platform::MacOs), "Горячие клавиши");
    }

    #[test]
    fn hotkey_names_are_those_of_the_settings() {
        let defaults = default_settings(Some("linux"));
        let hotkeys = Hotkeys {
            undo: "u".into(),
            convert_last: "c".into(),
            selection_layout: "l".into(),
            selection_case: "k".into(),
            selection_transliterate: "t".into(),
        };
        let mut seen = Vec::new();
        for name in HotkeyName::ALL {
            assert!(defaults["hotkeys"].get(name.setting()).is_some_and(Value::is_string), "{name:?}");
            seen.push(hotkeys.get(name));
        }
        assert_eq!(seen, ["u", "c", "l", "k", "t"]);
    }

    #[test]
    fn settings_a_hint_opens_or_marks_exist() {
        let defaults = Value::Object(default_settings(Some("linux")));
        let exists = |path: &str| path.split('.').try_fold(&defaults, |value, part| value.get(part)).is_some();
        for hint in &CATALOGUE {
            if let Some(Destination::Settings { setting: Some(path), .. }) = hint.opens {
                // The layout switch key of 0.70.0 comes with the branch that brings it.
                assert!(path == LAYOUT_SWITCH_KEY_SETTING || exists(path), "{path}");
            }
        }
    }

    #[test]
    fn a_click_on_the_news_of_an_update_opens_its_release() {
        let release = SemanticVersion::parse("0.71.0").unwrap();
        assert_eq!(Destination::ReleaseNotes.resolve(release), Opens::Url("https://github.com/olegius88/keyswitch/releases/tag/v0.71.0".into()));
        let page = Destination::Settings { page: SettingsPage::Hotkeys, setting: None };
        assert_eq!(page.resolve(release), Opens::Settings { page: SettingsPage::Hotkeys, setting: None });
    }

    fn hint(trigger: Trigger) -> Hint {
        Hint { trigger, ..CATALOGUE[1] }
    }

    #[test]
    fn each_trigger_fires_on_its_correction() {
        let correction = |mode: &'static str, automatic: bool, case_fix: bool| CorrectionInfo { mode, automatic, case_fix };
        let none = Counted::default();
        let always = |_: Condition| true;
        let layout = hint(Trigger::LayoutCorrection);
        assert!(layout.fired_by(correction("boundary", true, false), none, always));
        assert!(!layout.fired_by(correction("manual", false, false), none, always));
        assert!(!layout.fired_by(correction("case", true, true), none, always));
        assert!(!layout.fired_by(correction(MODE_EARLY, true, false), none, always));
        let mode = hint(Trigger::Mode(MODE_TYPO));
        assert!(mode.fired_by(correction(MODE_TYPO, true, false), none, always));
        assert!(!mode.fired_by(correction(MODE_EARLY, true, false), none, always));
        let manual = hint(Trigger::ManualConversion);
        assert!(manual.fired_by(correction(MODE_MANUAL, false, false), none, always));
        assert!(!manual.fired_by(correction(MODE_SYMBOLS, false, false), none, always));
        let five = hint(Trigger::AutomaticCorrections { at_least: 5, condition: Condition::Notifications });
        let counted = |automatic_corrections: i64, manual_conversions: i64| Counted { automatic_corrections, manual_conversions };
        assert!(five.fired_by(correction("boundary", true, false), counted(5, 0), always));
        assert!(five.fired_by(correction("boundary", true, false), counted(6, 0), always));
        assert!(!five.fired_by(correction("boundary", true, false), counted(4, 0), always));
        assert!(!five.fired_by(correction("boundary", true, false), counted(5, 0), |_| false));
        assert!(!five.fired_by(correction(MODE_MANUAL, false, false), counted(5, 0), always));
        let three = hint(Trigger::ManualConversions { at_least: 3 });
        assert!(three.fired_by(correction(MODE_SYMBOLS, false, false), counted(0, 3), always));
        assert!(!three.fired_by(correction(MODE_SYMBOLS, false, false), counted(0, 2), always));
        assert!(!three.fired_by(correction(MODE_UNDO, false, false), counted(0, 3), always));
        for trigger in [Trigger::FirstStart, Trigger::SettingsPage, Trigger::WhatsNew, Trigger::Updated] {
            assert!(!hint(trigger).fired_by(correction("boundary", true, false), counted(99, 99), always));
            assert_eq!(hint(trigger).needs(), Counted::default());
        }
        assert_eq!((five.needs(), three.needs()), (counted(5, 0), counted(0, 3)));
        let alone = hint(Trigger::AutomaticCorrections { at_least: 5, condition: Condition::NoManualConversion });
        assert_eq!(alone.needs(), counted(5, 1));
    }

    #[test]
    fn a_correction_is_taken_from_its_plan() {
        let mut plan = CorrectionPlan::new(Vec::new(), None, 0, 1, "ghbdtn".into(), "привет".into(), 0.9, "app");
        assert_eq!(CorrectionInfo::of(&plan), CorrectionInfo { mode: "boundary", automatic: true, case_fix: false });
        plan.mode = MODE_SYMBOLS;
        plan.automatic = false;
        assert!(CorrectionInfo::of(&plan).manual());
    }

    #[test]
    fn places_of_a_hint_outside_and_inside_the_window() {
        let news = CATALOGUE.iter().find(|hint| hint.trigger == Trigger::WhatsNew).unwrap();
        assert_eq!(news.shown_at(), Some(Place::Tray));
        assert_eq!(news.card_on(SettingsPage::Hotkeys), Some(Place::Settings { page: SettingsPage::Hotkeys, control: Some(LAYOUT_SWITCH_KEY_SETTING) }));
        assert_eq!(news.card_on(SettingsPage::Overview), None);
    }

    #[test]
    fn this_build_knows_its_platform() {
        let expected = if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Linux
        };
        assert_eq!(RUNNING_PLATFORM, expected);
    }
}

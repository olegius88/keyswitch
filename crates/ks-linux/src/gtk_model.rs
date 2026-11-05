//! What the GTK windows say and compute, apart from the widgets (the Python's `ui.py` helpers and `constants/ui_gtk.py`):
//! geometry, the navigation, the texts a snapshot turns into, and the report the About page copies.

use ks_cast::To as _;
use ks_backend::BackendProbe;
use ks_engine::EngineSnapshot;
use ks_text::russian_text::{NounForms, quantity};

pub const GTK_WINDOW_DEFAULT_WIDTH_PIXELS: i32 = 1040;
pub const GTK_WINDOW_DEFAULT_HEIGHT_PIXELS: i32 = 720;
pub const GTK_WINDOW_MIN_WIDTH_PIXELS: i32 = 850;
pub const GTK_WINDOW_MIN_HEIGHT_PIXELS: i32 = 600;
pub const GTK_STANDARD_SPACING_PIXELS: i32 = 12;
pub const GTK_COMPACT_SPACING_PIXELS: i32 = 10;
pub const GTK_SIDEBAR_WIDTH_PIXELS: i32 = 250;
pub const BRAND_MARGIN_OUTER_PIXELS: i32 = 20;
pub const BRAND_MARGIN_BOTTOM_PIXELS: i32 = 12;
pub const BRAND_MARGIN_END_PIXELS: i32 = 16;
pub const BRAND_ICON_SIZE_PIXELS: i32 = 44;
pub const SIDEBAR_PRIVACY_MARGIN_START_PIXELS: i32 = 22;
pub const SIDEBAR_PRIVACY_MARGIN_BOTTOM_PIXELS: i32 = 18;
pub const PAGE_CONTENT_SPACING_PIXELS: i32 = 18;
pub const HERO_TOP_ROW_SPACING_PIXELS: i32 = 14;
pub const HERO_COPY_SPACING_PIXELS: i32 = 5;
pub const TEST_ENTRY_WIDTH_CHARACTERS: i32 = 30;
pub const STAT_CARD_SPACING_PIXELS: i32 = 3;
pub const CONTEXT_MODE_OFF_INDEX: u32 = 2;
pub const CHECKSUM_DISPLAY_LENGTH_CHARACTERS: usize = 12;
pub const HOTKEY_ENTRY_WIDTH_CHARACTERS: i32 = 20;
pub const MANUAL_APP_ENTRY_WIDTH_CHARACTERS: i32 = 22;
pub const WORDS_EDITOR_HEIGHT_PIXELS: i32 = 100;
pub const TEXT_EDITOR_PADDING_PIXELS: i32 = 10;
pub const APPLICATION_PICKER_DIALOG_SIDE_PIXELS: i32 = 620;
pub const DIALOG_CONTENT_MARGIN_PIXELS: i32 = 12;
pub const UPDATE_BUTTONS_SPACING_PIXELS: i32 = 8;
pub const GTK_HISTORY_PAGE_ENTRIES: i64 = 200;
pub const DASHBOARD_RECENT_HISTORY_COUNT: usize = 4;
pub const SPIN_VALUE_EPSILON: f64 = 1e-9;
pub const APPLICATION_CAPTURE_DELAY_MS: u32 = 3000;
pub const TEXT_SAVE_DEBOUNCE_MS: u32 = 450;

pub const LEARNING_PROMPT_ANCHOR_GAP_PIXELS: i32 = 12;
pub const LEARNING_PROMPT_WINDOW_TITLE: &str = "Обучение KeySwitch";
pub const LEARNING_PROMPT_GTK_WIDTH_PIXELS: i32 = 410;
pub const LEARNING_PROMPT_GTK_SPACING_PIXELS: i32 = 5;
pub const LEARNING_PROMPT_GTK_MARGIN_Y_PIXELS: i32 = 14;
pub const LEARNING_PROMPT_GTK_MARGIN_X_PIXELS: i32 = 18;
pub const MAX_CARET_SEARCH_NODES: usize = 4096;
pub const MAX_CARET_SEARCH_CHILDREN: usize = 512;

pub const CONTEXT_MODES: [&str; 3] = ["assist", "shadow", "off"];
pub const THEMES: [&str; 3] = ["system", "light", "dark"];
pub const INDICATOR_STYLES: [&str; 2] = ["letters", "flags"];

/// `MainWindow.NAVIGATION`: the page name, its title in the sidebar and its icon.
pub const NAVIGATION: [(&str, &str, &str); 9] = [
    ("dashboard", "Обзор", "view-grid-symbolic"),
    ("automation", "Автокоррекция", "preferences-system-symbolic"),
    ("languages", "Языки", "input-keyboard-symbolic"),
    ("hotkeys", "Горячие клавиши", "preferences-desktop-keyboard-shortcuts-symbolic"),
    ("exceptions", "Исключения", "action-unavailable-symbolic"),
    ("system", "Внешний вид и система", "preferences-desktop-theme-symbolic"),
    ("updates", "Обновления", "software-update-available-symbolic"),
    ("history", "История", "document-open-recent-symbolic"),
    ("diagnostics", "О программе", "help-about-symbolic"),
];

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
pub const FIELD_CATCH_UP_TITLE: &str = "Ждать отстающее поле";
pub const FIELD_CATCH_UP_DESCRIPTION: &str = "Браузеры и редакторы на Electron показывают набранное с задержкой. Сколько ждать, \
                                              пока поле покажет слово, прежде чем отказаться от его исправления";
pub const FIELD_CATCH_UP_LABELS: [(&str, &str); 5] =
    [("off", "Не ждать"), ("100", "0,1 с"), ("300", "0,3 с"), ("500", "0,5 с"), ("1000", "1 с")];

#[must_use]
pub fn table_text(table: &[(&'static str, &'static str)], name: &str) -> &'static str {
    table.iter().find(|(key, _)| *key == name).map_or("", |(_, text)| *text)
}

/// The position of `value` among `keys`, or `fallback` for an unknown one.
#[must_use]
pub fn index_of(keys: &[&str], value: &str, fallback: u32) -> u32 {
    keys.iter().position(|key| *key == value).map_or(fallback, ks_cast::To::low::<u32>)
}

/// `quantity` of a count in the noun forms.
#[must_use]
pub fn counted(value: f64, forms: NounForms) -> String {
    quantity(value, forms)
}

#[must_use]
pub fn plural_entries(count: usize) -> String {
    quantity(count.to::<f64>(), ["запись", "записи", "записей"])
}

#[must_use]
pub fn learning_summary(count: usize) -> String {
    if count > 0 { format!("Правил: {count}") } else { "Правил пока нет".to_owned() }
}

/// `_format_time`: an ISO moment in the local time zone as day.month.year · hour:minute, or the text as it came.
#[must_use]
pub fn format_time(value: &str) -> String {
    gtk::glib::DateTime::from_iso8601(value, None)
        .ok()
        .and_then(|moment| moment.to_local().ok())
        .and_then(|moment| moment.format("%d.%m.%Y · %H:%M").ok())
        .map_or_else(|| value.to_owned(), |text| text.to_string())
}

/// `_os_description`: `PRETTY_NAME` of an os-release file's text.
#[must_use]
pub fn os_pretty_name(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key == "PRETTY_NAME").then(|| value.trim_matches('"').to_owned())
    })
}

pub fn os_description() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| os_pretty_name(&text))
        .unwrap_or_else(crate::platform::platform_description)
}

/// What the sidebar, the overview and the languages page show of an engine snapshot (`_apply_engine_snapshot`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineView {
    pub detail: &'static str,
    pub title: &'static str,
    pub pill: &'static str,
    pub action: String,
    pub corrections: String,
    pub layout: &'static str,
    pub backend: &'static str,
    pub active_language: &'static str,
    pub subtitle: String,
}

pub const HERO_SUBTITLE: &str = "Печатайте как обычно — раскладка исправится после слова.";

#[must_use]
pub fn engine_view(snapshot: &EngineSnapshot, wayland: bool) -> EngineView {
    let running = snapshot.running;
    let active = running && snapshot.enabled;
    // The models load for several seconds before the engine can start: that is not a backend failure.
    if !running && snapshot.last_error.is_empty() && snapshot.last_action == ks_engine::engine_loader::LOADING_ACTION {
        return EngineView {
            detail: "Загрузка…",
            title: "Загрузка языковых моделей",
            pill: "ЗАГРУЗКА",
            action: snapshot.last_action.clone(),
            corrections: "0".to_owned(),
            layout: "—",
            backend: "…",
            active_language: "—",
            subtitle: HERO_SUBTITLE.to_owned(),
        };
    }
    EngineView {
        detail: if active { "Работает в фоне" } else if running { "На паузе" } else { "Backend недоступен" },
        title: if active { "Автокоррекция активна" } else if running { "Автокоррекция приостановлена" } else { "Нужна проверка backend" },
        pill: if active { "АКТИВНО" } else if running { "ПАУЗА" } else { "ОШИБКА" },
        action: if snapshot.current_word.is_empty() { snapshot.last_action.clone() } else { snapshot.current_word.clone() },
        corrections: snapshot.correction_count.to_string(),
        layout: match snapshot.current_group {
            0 => "EN",
            1 => "RU",
            _ => "—",
        },
        backend: if !running { "—" } else if wayland { "Wayland" } else { "X11" },
        active_language: match snapshot.current_group {
            0 => "EN · группа 1",
            1 => "RU · группа 2",
            _ => "—",
        },
        subtitle: if snapshot.last_error.is_empty() { HERO_SUBTITLE.to_owned() } else { snapshot.last_error.clone() },
    }
}

/// The group number as the diagnostics rows show it: from 1, a dash when unknown.
#[must_use]
pub fn group_text(current_group: i32) -> String {
    if current_group >= 0 { (current_group + 1).to_string() } else { "—".to_owned() }
}

/// What the About page's report says of the models and the settings (`_copy_diagnostics`).
pub struct ReportContext {
    pub version: String,
    pub os: String,
    pub intent_model: String,
    pub context_model: String,
    pub context_mode: String,
    pub read_field: String,
    pub technical_logging: bool,
    pub technical_log: String,
    pub log_state: String,
}

#[must_use]
pub fn diagnostics_report(probe: &BackendProbe, context: &ReportContext) -> String {
    let backend_lines = if probe.session_type == "wayland" {
        vec![
            format!("Desktop: {}", probe.display),
            format!("Capture: {}", probe.record_version),
            format!("Injection: {}", probe.xtest_version),
            format!("Layouts: {}", probe.xkb_version),
            format!("Active group: {}", probe.current_group),
        ]
    } else {
        vec![
            format!("DISPLAY: {}", probe.display),
            format!("XRecord: {}", probe.record_version),
            format!("XTEST: {}", probe.xtest_version),
            format!("XKB: {}", probe.xkb_version),
            format!("XKB group: {}", probe.current_group),
        ]
    };
    let mut lines = vec![format!("KeySwitch {}", context.version), format!("OS: {}", context.os), format!("Session: {}", probe.session_type)];
    lines.extend(backend_lines);
    lines.push(format!("Intent model: {}", context.intent_model));
    lines.push(format!("Context model: {}", context.context_model));
    lines.push(format!("Context mode: {}", context.context_mode));
    lines.push(format!("Read active field: {}", context.read_field));
    lines.push(format!("Technical logging: {}", if context.technical_logging { "True" } else { "False" }));
    lines.push(format!("Technical log: {}", context.technical_log));
    lines.push(format!("Log state: {}", context.log_state));
    lines.push(format!("Backend error: {}", if probe.error.is_empty() { "none" } else { probe.error.as_str() }));
    lines.join("\n")
}

/// Where the learning prompt goes: centred over the anchor, above it, under it when there is no room above, and inside the
/// monitor the anchor is on (`_position_above_anchor`). `monitor` is `(x, y, width, height)`.
#[must_use]
pub fn prompt_position(anchor: (i32, i32), size: (i32, i32), monitor: Option<(i32, i32, i32, i32)>) -> (i32, i32) {
    let (width, height) = size;
    let mut x = anchor.0 - width / 2;
    let mut y = anchor.1 - height - LEARNING_PROMPT_ANCHOR_GAP_PIXELS;
    if let Some((left, top, monitor_width, _)) = monitor {
        if y < top {
            y = anchor.1 + LEARNING_PROMPT_ANCHOR_GAP_PIXELS;
        }
        x = x.min(left + monitor_width - width).max(left);
    }
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snapshot_reads_as_the_window_says() {
        let mut snapshot = EngineSnapshot { running: true, enabled: true, current_group: 1, correction_count: 7, ..EngineSnapshot::default() };
        let view = engine_view(&snapshot, false);
        assert_eq!((view.pill, view.layout, view.backend, view.corrections.as_str()), ("АКТИВНО", "RU", "X11", "7"));
        snapshot.enabled = false;
        assert_eq!(engine_view(&snapshot, true).detail, "На паузе");
        snapshot.running = false;
        snapshot.last_error = "нет".to_owned();
        let view = engine_view(&snapshot, true);
        assert_eq!((view.pill, view.backend, view.subtitle.as_str()), ("ОШИБКА", "—", "нет"));
    }

    #[test]
    fn counts_and_times() {
        assert_eq!(plural_entries(1), "1 запись");
        assert_eq!(plural_entries(5), "5 записей");
        assert_eq!(learning_summary(0), "Правил пока нет");
        assert_eq!(format_time("not a time"), "not a time");
        assert!(format_time("2026-11-04T12:00:00+00:00").contains("04.11.2026"));
        assert_eq!(os_pretty_name("NAME=\"X\"\nPRETTY_NAME=\"Ubuntu 26.04\"\n").as_deref(), Some("Ubuntu 26.04"));
        assert_eq!(group_text(-1), "—");
        assert_eq!(group_text(1), "2");
    }

    #[test]
    fn the_prompt_stays_above_the_caret_and_on_the_monitor() {
        let monitor = Some((0, 0, 1280, 800));
        assert_eq!(prompt_position((640, 400), (400, 100), monitor), (440, 288));
        // Too near the top: under the caret.
        assert_eq!(prompt_position((640, 50), (400, 100), monitor), (440, 62));
        // Too near the edges: pushed back inside.
        assert_eq!(prompt_position((1270, 400), (400, 100), monitor), (880, 288));
        assert_eq!(prompt_position((10, 400), (400, 100), monitor), (0, 288));
    }

    #[test]
    fn the_report_names_the_backend_in_its_own_words() {
        let probe = BackendProbe { session_type: "x11".into(), display: ":0".into(), ..BackendProbe::default() };
        let context = ReportContext {
            version: "1".into(),
            os: "OS".into(),
            intent_model: "i".into(),
            context_model: "c".into(),
            context_mode: "assist".into(),
            read_field: "False".into(),
            technical_logging: false,
            technical_log: "log".into(),
            log_state: "s".into(),
        };
        let text = diagnostics_report(&probe, &context);
        assert!(text.contains("DISPLAY: :0") && text.ends_with("Backend error: none"), "{text}");
    }
}

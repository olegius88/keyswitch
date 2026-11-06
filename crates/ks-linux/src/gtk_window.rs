//! The settings window: GTK 4 and libadwaita, every page and setting of the Python's `ui.py` in its order.
//!
//! The window is built when it is shown and destroyed when it is closed to the tray, so the program holds no widgets while
//! it runs in the panel. Nothing here keeps the window alive from a signal handler: handlers hold the settings and plain data
//! and reach the window through `gtk_app`.

use ks_cast::To as _;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use adw::prelude::*;
use gtk::{gdk, gio, glib, pango};
use ks_backend::{BackendProbe, InputBackend};
use ks_config::app_quirks::MENTION_HEADS;
use ks_config::config::SettingsStore;
use ks_config::constants::{DEFAULT_CONFIDENCE_THRESHOLD, DEFAULT_FIELD_CATCH_UP, DEFAULT_MINIMUM_WORD_LENGTH, DEFAULT_PAUSE_DELAY_SECONDS};
use ks_config::history::HistoryStore;
use ks_config::learning::LearnedRule;
use ks_config::logsetup::{log_directory, log_path, log_status, rotation_summary};
use ks_context::constants::PREFIX_MAX_CHARACTERS;
use ks_context::typo_correction::{
    CORRECT_TYPOS_ENGLISH_DESCRIPTION, CORRECT_TYPOS_ENGLISH_TITLE, CORRECT_TYPOS_TITLE, correct_typos_description,
};
use ks_engine::EngineSnapshot;
use ks_engine::constants::{
    CONFIDENCE_SETTING_MAX, CONFIDENCE_SETTING_MIN, DEFAULT_EARLY_SWITCH_MIN_LENGTH, EARLY_SWITCH_MIN_LENGTH_SETTING_MAX,
    EARLY_SWITCH_MIN_LENGTH_SETTING_MIN, FIELD_CATCH_UP_CHOICES, MINIMUM_WORD_LENGTH_SETTING_MAX, MINIMUM_WORD_LENGTH_SETTING_MIN,
    PAUSE_DELAY_SETTING_MAX_SECONDS, PAUSE_DELAY_SETTING_MIN_SECONDS, PREFIX_MIN_CHARACTERS, SELECTION_HOTKEYS, UNDO_AVAILABLE_WINDOW_SECONDS,
};
use ks_engine::engine_loader::DeferredEngine;
use ks_engine::rule_editor::{RuleDraft, action_text, condition_text, draft_from_rule, other_layout_text};
use ks_text::russian_text::{HOURS, SECONDS};
use ks_updates::constants::{UPDATE_CHECK_INITIAL_DELAY_SECONDS, UPDATE_CHECK_INTERVAL_SECONDS};
use ks_updates::{UpdateController, UpdatePhase, UpdateSnapshot};
use serde_json::{Value, json};

use crate::gtk_app;
use crate::gtk_model::{with_live, GTK_STANDARD_SPACING_PIXELS, PAGE_CONTENT_SPACING_PIXELS, GTK_WINDOW_DEFAULT_WIDTH_PIXELS, GTK_WINDOW_DEFAULT_HEIGHT_PIXELS, GTK_WINDOW_MIN_WIDTH_PIXELS, GTK_WINDOW_MIN_HEIGHT_PIXELS, GTK_SIDEBAR_WIDTH_PIXELS, BRAND_MARGIN_OUTER_PIXELS, BRAND_MARGIN_BOTTOM_PIXELS, BRAND_MARGIN_END_PIXELS, BRAND_ICON_SIZE_PIXELS, GTK_COMPACT_SPACING_PIXELS, NAVIGATION, SIDEBAR_PRIVACY_MARGIN_START_PIXELS, SIDEBAR_PRIVACY_MARGIN_BOTTOM_PIXELS, engine_view, SPIN_VALUE_EPSILON, index_of, CONTEXT_MODES, CONTEXT_MODE_OFF_INDEX, GTK_HISTORY_PAGE_ENTRIES, format_time, plural_entries, DASHBOARD_RECENT_HISTORY_COUNT, learning_summary, CHECKSUM_DISPLAY_LENGTH_CHARACTERS, APPLICATION_CAPTURE_DELAY_MS, APPLICATION_PICKER_DIALOG_SIDE_PIXELS, DIALOG_CONTENT_MARGIN_PIXELS, ReportContext, os_description, diagnostics_report, counted, STAT_CARD_SPACING_PIXELS, TEXT_EDITOR_PADDING_PIXELS, WORDS_EDITOR_HEIGHT_PIXELS, TEXT_SAVE_DEBOUNCE_MS, HOTKEY_ENTRY_WIDTH_CHARACTERS, HERO_TOP_ROW_SPACING_PIXELS, HERO_COPY_SPACING_PIXELS, HERO_SUBTITLE, TEST_ENTRY_WIDTH_CHARACTERS, table_text, FIELD_CATCH_UP_LABELS, FIELD_CATCH_UP_TITLE, FIELD_CATCH_UP_DESCRIPTION, SELECTION_HOTKEY_TITLES, SELECTION_HOTKEY_DESCRIPTIONS, MANUAL_APP_ENTRY_WIDTH_CHARACTERS, THEMES, INDICATOR_STYLES, UPDATE_BUTTONS_SPACING_PIXELS, group_text};
use crate::system::AutostartManager;

const CONFIDENCE_SETTING_STEP: f64 = 0.1;
const PAUSE_DELAY_SETTING_STEP_SECONDS: f64 = 0.1;

/// What the engine reports of its models once it is built: the languages page shows it.
#[derive(Clone, Debug, Default)]
pub struct ModelInfo {
    pub loaded: bool,
    /// The words and the source of the model of each group.
    pub languages: Vec<(usize, String)>,
    pub intent_available: bool,
    pub intent_version: String,
    pub intent_checksum: String,
    pub intent_error: String,
    pub intent_summary: String,
    pub context_status: String,
}

pub type SharedModelInfo = Arc<Mutex<ModelInfo>>;

/// What the window works with: the stores, the engine and the platform.
pub struct Services {
    pub settings: Arc<SettingsStore>,
    pub history: Arc<HistoryStore>,
    pub engine: Arc<DeferredEngine>,
    pub updates: Arc<dyn UpdateController>,
    pub backend: Arc<dyn InputBackend>,
    pub models: SharedModelInfo,
    pub resource_dir: PathBuf,
    pub version: &'static str,
}

/// An installed application the exclusions can name.
#[derive(Clone)]
pub struct ApplicationChoice {
    pub name: String,
    pub identifier: String,
    pub desktop_id: String,
    pub icon: Option<gio::Icon>,
}

impl ApplicationChoice {
    fn search_text(&self) -> String {
        format!("{} {} {}", self.name, self.identifier, self.desktop_id).to_lowercase()
    }
}

/// `installed_application_choices`: visible desktop applications with the best `WM_CLASS` candidate.
#[must_use]
pub fn installed_application_choices() -> Vec<ApplicationChoice> {
    let shown = gio::AppInfo::all().into_iter().filter(AppInfoExt::should_show).map(|application| {
        let desktop_id = application.id().map(|id| id.to_string()).unwrap_or_default();
        RawChoice {
            desktop_id: desktop_id.strip_suffix(".desktop").unwrap_or(&desktop_id).to_owned(),
            startup_class: application.downcast_ref::<gio_unix::DesktopAppInfo>().and_then(gio_unix::DesktopAppInfo::startup_wm_class).map(|class| class.to_string()).unwrap_or_default(),
            executable: application.executable().file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
            display_name: application.display_name().to_string(),
            name: application.name().to_string(),
            icon: application.icon(),
        }
    });
    choices_of(shown)
}

/// What a desktop entry says of a program.
struct RawChoice {
    desktop_id: String,
    startup_class: String,
    executable: String,
    display_name: String,
    name: String,
    icon: Option<gio::Icon>,
}

/// The programs a person can choose from, by name, each once. A program with nothing to identify it is left out.
fn choices_of(items: impl Iterator<Item = RawChoice>) -> Vec<ApplicationChoice> {
    let mut choices: Vec<ApplicationChoice> = Vec::new();
    for item in items {
        let first_text = |texts: [&str; 3]| texts.into_iter().map(str::trim).find(|text| !text.is_empty()).unwrap_or_default().to_owned();
        let identifier = first_text([&item.startup_class, &item.executable, &item.desktop_id]);
        let name = first_text([&item.display_name, &item.name, &identifier]);
        let key = identifier.to_lowercase();
        if !identifier.is_empty() && choices.iter().all(|choice| choice.identifier.to_lowercase() != key) {
            choices.push(ApplicationChoice { name, identifier, desktop_id: item.desktop_id, icon: item.icon });
        }
    }
    choices.sort_by_key(|choice| (choice.name.to_lowercase(), choice.identifier.to_lowercase()));
    choices
}

/// A control that follows its setting when another part of the program changes it.
enum Control {
    Switch(adw::SwitchRow),
    Spin(adw::SpinRow),
    Combo(adw::ComboRow),
}

fn setting_flag(settings: &SettingsStore, path: &str, default: bool) -> bool {
    crate::linux_app::setting_bool(settings, path, default)
}

fn setting_text(settings: &SettingsStore, path: &str, default: &str) -> String {
    match settings.get(path) {
        Some(Value::String(text)) => text,
        Some(Value::Null) | None => default.to_owned(),
        Some(other) => other.to_string(),
    }
}

fn setting_number(settings: &SettingsStore, path: &str, default: f64) -> f64 {
    settings.get(path).and_then(|value| value.as_f64()).unwrap_or(default)
}

fn setting_strings(settings: &SettingsStore, path: &str) -> Vec<String> {
    match settings.get(path) {
        Some(Value::Array(items)) => items.iter().map(|item| item.as_str().map_or_else(|| item.to_string(), str::to_owned)).collect(),
        _ => Vec::new(),
    }
}

fn store(settings: &SettingsStore, path: &str, value: Value) {
    let _ = settings.set(path, value, true);
}

fn thousands(count: usize) -> String {
    let digits = count.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// Python's `repr` of the log status dictionary, which the copied report carries.
fn status_repr(status: &serde_json::Map<String, Value>) -> String {
    let items: Vec<String> = status
        .iter()
        .map(|(key, value)| {
            let shown = match value {
                Value::Bool(true) => "True".to_owned(),
                Value::Bool(false) => "False".to_owned(),
                Value::String(text) => format!("'{text}'"),
                other @ (Value::Null | Value::Number(_) | Value::Array(_) | Value::Object(_)) => other.to_string(),
            };
            format!("'{key}': {shown}")
        })
        .collect();
    format!("{{{}}}", items.join(", "))
}

fn label(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).build()
}

fn styled(widget: &impl IsA<gtk::Widget>, class: &str) {
    widget.add_css_class(class);
}

fn image(name: &str) -> gtk::Image {
    gtk::Image::from_icon_name(name)
}

fn action_row(title: &str, subtitle: &str) -> adw::ActionRow {
    adw::ActionRow::builder().title(title).subtitle(subtitle).build()
}

fn group(title: &str) -> adw::PreferencesGroup {
    adw::PreferencesGroup::builder().title(title).build()
}

fn group_with(title: &str, description: &str) -> adw::PreferencesGroup {
    adw::PreferencesGroup::builder().title(title).description(description).build()
}

fn centered(button: gtk::Button) -> gtk::Button {
    button.set_valign(gtk::Align::Center);
    button
}

fn button(text: &str) -> gtk::Button {
    centered(gtk::Button::with_label(text))
}

fn card(icon: &str, class: &str, text: &str) -> gtk::Box {
    let tip = gtk::Box::new(gtk::Orientation::Horizontal, GTK_STANDARD_SPACING_PIXELS);
    styled(&tip, class);
    tip.append(&image(icon));
    let message = gtk::Label::builder().label(text).xalign(0.0).wrap(true).hexpand(true).build();
    tip.append(&message);
    tip
}

fn clear_list(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn open_uri(uri: &str) -> Result<bool, String> {
    gio::AppInfo::launch_default_for_uri(uri, None::<&gio::AppLaunchContext>).map(|()| true).map_err(|error| error.message().to_owned())
}

pub fn apply_theme(theme: &str) {
    let scheme = match theme {
        "light" => adw::ColorScheme::ForceLight,
        "dark" => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    };
    adw::StyleManager::default().set_color_scheme(scheme);
}

thread_local! {
    static CSS_INSTALLED: Cell<bool> = const { Cell::new(false) };
}

fn install_css(resource_dir: &std::path::Path) {
    if CSS_INSTALLED.with(|flag| flag.replace(true)) {
        return;
    }
    let provider = gtk::CssProvider::new();
    provider.load_from_path(resource_dir.join("style.css"));
    add_provider_to(gdk::Display::default(), &provider);
}

/// The style for a display, if there is one.
pub fn add_provider_to(display: Option<gdk::Display>, provider: &gtk::CssProvider) {
    if let Some(display) = display {
        gtk::style_context_add_provider_for_display(&display, provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}

/// The widgets the engine's snapshot and the updater change.
struct Live {
    service_switch: gtk::Switch,
    sidebar_detail: gtk::Label,
    hero_title: gtk::Label,
    hero_subtitle: gtk::Label,
    hero_pill: gtk::Label,
    hero_action: gtk::Label,
    stat_corrections: gtk::Label,
    stat_layout: gtk::Label,
    stat_backend: gtk::Label,
    language_active: gtk::Label,
    history_total: gtk::Label,
    history_list: gtk::ListBox,
    dashboard_history: gtk::ListBox,
    update_status: adw::ActionRow,
    update_version: adw::ActionRow,
    update_check: gtk::Button,
    update_open: gtk::Button,
    learning_group: adw::PreferencesGroup,
    learning_status: adw::ActionRow,
    apps_group: adw::PreferencesGroup,
    active_app_row: adw::ActionRow,
    manual_entry: gtk::Entry,
    models_group: adw::PreferencesGroup,
}

pub struct MainWindow {
    pub window: adw::ApplicationWindow,
    pub services: Rc<Services>,
    toast_overlay: adw::ToastOverlay,
    stack: gtk::Stack,
    nav_list: gtk::ListBox,
    live: Live,
    controls: RefCell<HashMap<String, Control>>,
    save_sources: Rc<RefCell<HashMap<String, glib::SourceId>>>,
    rule_rows: RefCell<Vec<adw::ActionRow>>,
    application_rows: RefCell<Vec<adw::ActionRow>>,
    model_rows: RefCell<Vec<adw::ActionRow>>,
    catalog: RefCell<Option<Rc<Vec<ApplicationChoice>>>>,
    wayland: Cell<bool>,
    probe: BackendProbe,
}

/// A page of the stack: its content box, which the builders fill.
struct PageBuilder<'a> {
    stack: &'a gtk::Stack,
}

impl PageBuilder<'_> {
    fn page(&self, name: &str, title: &str, subtitle: &str) -> gtk::Box {
        let content = gtk::Box::new(gtk::Orientation::Vertical, PAGE_CONTENT_SPACING_PIXELS);
        styled(&content, "page-content");
        let heading = label(title);
        styled(&heading, "page-title");
        let description = gtk::Label::builder().label(subtitle).xalign(0.0).wrap(true).build();
        styled(&description, "page-subtitle");
        content.append(&heading);
        content.append(&description);
        let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).child(&content).build();
        self.stack.add_named(&scroll, Some(name));
        content
    }
}

impl MainWindow {
    #[must_use]
    pub fn new(application: &adw::Application, services: Rc<Services>) -> Rc<MainWindow> {
        install_css(&services.resource_dir);
        let window = adw::ApplicationWindow::builder().application(application).title("KeySwitch").build();
        window.set_default_size(GTK_WINDOW_DEFAULT_WIDTH_PIXELS, GTK_WINDOW_DEFAULT_HEIGHT_PIXELS);
        window.set_size_request(GTK_WINDOW_MIN_WIDTH_PIXELS, GTK_WINDOW_MIN_HEIGHT_PIXELS);
        window.add_css_class("keyswitch-window");
        let toast_overlay = adw::ToastOverlay::new();
        let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).hexpand(true).vexpand(true).build();
        let nav_list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::Single).build();
        let probe = services.backend.probe();
        let settings = &services.settings;

        let sidebar_detail = label("Запуск…");
        let service_switch = gtk::Switch::builder().active(setting_flag(settings, "enabled", true)).valign(gtk::Align::Center).build();
        let sidebar = Self::build_sidebar(&nav_list, &sidebar_detail, &service_switch);

        let header = adw::HeaderBar::new();
        header.set_show_title(false);
        let header_title = label("Настройки KeySwitch");
        styled(&header_title, "heading");
        header.set_title_widget(Some(&header_title));
        let menu = gio::Menu::new();
        menu.append(Some("Показать обзор"), Some("app.show"));
        menu.append(Some("Приостановить / продолжить"), Some("app.toggle"));
        menu.append(Some("Завершить KeySwitch"), Some("app.quit"));
        header.pack_end(&gtk::MenuButton::builder().icon_name("open-menu-symbolic").tooltip_text("Меню").menu_model(&menu).build());
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.append(&sidebar);
        body.append(&stack);
        toolbar.set_content(Some(&body));
        toast_overlay.set_child(Some(&toolbar));
        window.set_content(Some(&toast_overlay));

        let controls = RefCell::new(HashMap::new());
        let save_sources = Rc::new(RefCell::new(HashMap::new()));
        let mut builder = Builder {
            services: services.clone(),
            controls: &controls,
            save_sources: save_sources.clone(),
            probe: &probe,
            pages: PageBuilder { stack: &stack },
        };
        let dashboard = builder.dashboard(&sidebar_detail);
        let automation = builder.automation();
        let languages = builder.languages();
        builder.hotkeys();
        let exceptions = builder.exceptions();
        builder.system();
        let updates = builder.updates();
        let history = builder.history();
        builder.diagnostics();

        let live = Live {
            service_switch,
            sidebar_detail,
            hero_title: dashboard.hero_title,
            hero_subtitle: dashboard.hero_subtitle,
            hero_pill: dashboard.hero_pill,
            hero_action: dashboard.hero_action,
            stat_corrections: dashboard.stat_corrections,
            stat_layout: dashboard.stat_layout,
            stat_backend: dashboard.stat_backend,
            language_active: languages.active,
            history_total: history.total,
            history_list: history.list,
            dashboard_history: dashboard.history,
            update_status: updates.status,
            update_version: updates.version,
            update_check: updates.check,
            update_open: updates.open,
            learning_group: automation.group,
            learning_status: automation.status,
            apps_group: exceptions.group,
            active_app_row: exceptions.active_app_row,
            manual_entry: exceptions.manual_entry,
            models_group: languages.models,
        };
        let wayland = probe.session_type == "wayland";
        let this = Rc::new(MainWindow {
            window,
            services,
            toast_overlay,
            stack,
            nav_list,
            live,
            controls,
            save_sources,
            rule_rows: RefCell::new(Vec::new()),
            application_rows: RefCell::new(Vec::new()),
            model_rows: RefCell::new(Vec::new()),
            catalog: RefCell::new(None),
            wayland: Cell::new(wayland),
            probe,
        });
        this.connect();
        this.refresh_history();
        this.refresh_rules();
        this.refresh_application_exclusions();
        this.refresh_models();
        this.apply_engine_snapshot(&this.services.engine.snapshot());
        this.apply_update_snapshot(&this.services.updates.snapshot());
        this.nav_list.select_row(this.nav_list.row_at_index(0).as_ref());
        this
    }

    fn build_sidebar(nav_list: &gtk::ListBox, detail: &gtk::Label, service_switch: &gtk::Switch) -> gtk::Box {
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.set_size_request(GTK_SIDEBAR_WIDTH_PIXELS, -1);
        styled(&sidebar, "keyswitch-sidebar");

        let brand = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(GTK_STANDARD_SPACING_PIXELS)
            .margin_top(BRAND_MARGIN_OUTER_PIXELS)
            .margin_bottom(BRAND_MARGIN_BOTTOM_PIXELS)
            .margin_start(BRAND_MARGIN_OUTER_PIXELS)
            .margin_end(BRAND_MARGIN_END_PIXELS)
            .build();
        let icon = gtk::Image::from_icon_name("keyswitch");
        icon.set_pixel_size(BRAND_ICON_SIZE_PIXELS);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
        let title = label("KeySwitch");
        styled(&title, "brand-title");
        let caption = label("Умная раскладка");
        styled(&caption, "brand-caption");
        text.append(&title);
        text.append(&caption);
        brand.append(&icon);
        brand.append(&text);
        sidebar.append(&brand);

        let status = gtk::Box::new(gtk::Orientation::Horizontal, GTK_COMPACT_SPACING_PIXELS);
        styled(&status, "sidebar-status");
        let status_text = gtk::Box::new(gtk::Orientation::Vertical, 1);
        let status_title = label("Автокоррекция");
        styled(&status_title, "heading");
        styled(detail, "muted");
        detail.set_ellipsize(pango::EllipsizeMode::End);
        status_text.append(&status_title);
        status_text.append(detail);
        status_text.set_hexpand(true);
        status.append(&status_text);
        status.append(service_switch);
        sidebar.append(&status);

        styled(nav_list, "navigation-list");
        for (_, title, icon_name) in NAVIGATION {
            let row = gtk::ListBoxRow::new();
            let content = gtk::Box::new(gtk::Orientation::Horizontal, GTK_STANDARD_SPACING_PIXELS);
            styled(&content, "navigation-row");
            content.append(&image(icon_name));
            content.append(&gtk::Label::builder().label(title).xalign(0.0).hexpand(true).build());
            row.set_child(Some(&content));
            nav_list.append(&row);
        }
        sidebar.append(nav_list);
        sidebar.append(&gtk::Box::builder().vexpand(true).build());
        let privacy = gtk::Label::builder()
            .label("Локальная обработка\nБез облака и кейлогов")
            .xalign(0.0)
            .margin_start(SIDEBAR_PRIVACY_MARGIN_START_PIXELS)
            .margin_bottom(SIDEBAR_PRIVACY_MARGIN_BOTTOM_PIXELS)
            .build();
        styled(&privacy, "muted");
        sidebar.append(&privacy);
        sidebar
    }

    fn connect(&self) {
        let settings = self.services.settings.clone();
        self.live.service_switch.connect_active_notify(move |switch| store(&settings, "enabled", json!(switch.is_active())));
        let stack = self.stack.clone();
        self.nav_list.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let (name, _, _) = NAVIGATION[row.index().to::<usize>()];
            stack.set_visible_child_name(name);
            if name == "history" || name == "automation" {
                gtk_app::refresh_current(name);
            }
        });
        self.window.connect_close_request(|_| gtk_app::window_close_requested());
    }

    // ------------------------------------------------------------------------------------------------- shared helpers

    /// The settings this window shows a control of.
    #[must_use]
    pub fn control_paths(&self) -> Vec<String> {
        self.controls.borrow().keys().cloned().collect()
    }

    pub fn toast(&self, message: &str) {
        self.toast_overlay.add_toast(adw::Toast::new(message));
    }

    pub fn show_page(&self, page: &str) -> bool {
        let Some(index) = NAVIGATION.iter().position(|(name, _, _)| *name == page) else { return false };
        match self.nav_list.row_at_index(index.low::<i32>()) {
            Some(row) => {
                self.nav_list.select_row(Some(&row));
                true
            }
            None => false,
        }
    }

    // ---------------------------------------------------------------------------------------------------- live updates

    /// `_apply_engine_snapshot`.
    pub fn apply_engine_snapshot(&self, snapshot: &EngineSnapshot) {
        let live = &self.live;
        if live.service_switch.is_active() != snapshot.enabled {
            live.service_switch.set_active(snapshot.enabled);
        }
        let view = engine_view(snapshot, self.wayland.get());
        live.sidebar_detail.set_label(view.detail);
        live.hero_title.set_label(view.title);
        live.hero_pill.set_label(view.pill);
        live.hero_action.set_label(&view.action);
        live.stat_corrections.set_label(&view.corrections);
        live.stat_layout.set_label(view.layout);
        live.stat_backend.set_label(view.backend);
        live.language_active.set_label(view.active_language);
        live.hero_subtitle.set_label(&view.subtitle);
    }

    /// `_apply_setting_update`: a control follows a setting changed elsewhere.
    pub fn apply_setting_update(&self, path: &str, value: &Value) {
        if let Some(control) = self.controls.borrow().get(path) {
            let truth = crate::linux_app::truthy(value);
            match control {
                Control::Switch(row) if row.is_active() != truth => row.set_active(truth),
                Control::Spin(row) => {
                    if let Some(number) = value.as_f64()
                        && (row.value() - number).abs() > SPIN_VALUE_EPSILON
                    {
                        row.set_value(number);
                    }
                }
                Control::Combo(row) if path == "appearance.indicator_style" => {
                    let selected = u32::from(value == "flags");
                    if row.selected() != selected {
                        row.set_selected(selected);
                    }
                }
                Control::Combo(row) if path == "detection.context_policy" => {
                    let selected = index_of(&CONTEXT_MODES, value.as_str().unwrap_or(""), CONTEXT_MODE_OFF_INDEX);
                    if row.selected() != selected {
                        row.set_selected(selected);
                    }
                }
                Control::Switch(_) | Control::Combo(_) => {}
            }
        }
        if path == "exclusions.applications" {
            self.refresh_application_exclusions();
        }
        if path == "enabled" {
            let truth = crate::linux_app::truthy(value);
            if self.live.service_switch.is_active() != truth {
                self.live.service_switch.set_active(truth);
            }
        }
    }

    /// `_apply_update_snapshot`.
    pub fn apply_update_snapshot(&self, snapshot: &UpdateSnapshot) {
        let live = &self.live;
        live.update_status.set_subtitle(&snapshot.message);
        live.update_version.set_subtitle(&if snapshot.available_version.is_empty() {
            "Новых выпусков пока не найдено".to_owned()
        } else {
            format!("Доступна {}", snapshot.available_version)
        });
        live.update_check.set_sensitive(!matches!(snapshot.phase, UpdatePhase::Checking | UpdatePhase::Downloading));
        live.update_open.set_sensitive(!snapshot.release_url.is_empty());
    }

    pub fn refresh_history(&self) {
        let entries = self.services.history.read(Some(GTK_HISTORY_PAGE_ENTRIES)).unwrap_or_default();
        let live = &self.live;
        clear_list(&live.history_list);
        if entries.is_empty() {
            live.history_list.append(&action_row("История пока пуста", "Первое автоматическое исправление появится здесь"));
        }
        for entry in entries.iter().rev() {
            let application = if entry.application.is_empty() { "неизвестное приложение" } else { entry.application.as_str() };
            let row = action_row(
                &format!("{}  →  {}", entry.original, entry.replacement),
                &format!("{} · {application} · уверенность {}", format_time(&entry.timestamp), crate::linux_app::python_float(entry.confidence)),
            );
            row.add_prefix(&image("emblem-ok-symbolic"));
            live.history_list.append(&row);
        }
        live.history_total.set_label(&plural_entries(entries.len()));
        clear_list(&live.dashboard_history);
        if entries.is_empty() {
            live.dashboard_history.append(&action_row("Пока нет исправлений", "Попробуйте тестовое поле выше"));
        }
        let recent = &entries[entries.len().saturating_sub(DASHBOARD_RECENT_HISTORY_COUNT)..];
        for entry in recent.iter().rev() {
            live.dashboard_history.append(&action_row(&format!("{}  →  {}", entry.original, entry.replacement), &format_time(&entry.timestamp)));
        }
    }

    /// `refresh_rules`: the switching rules under the group's summary row.
    pub fn refresh_rules(&self) {
        let live = &self.live;
        for row in self.rule_rows.borrow_mut().drain(..) {
            live.learning_group.remove(&row);
        }
        let mut rows = Vec::new();
        for rule in self.services.engine.learning.rules() {
            let row = action_row(
                &rule.pattern,
                &format!(
                    "{} · {} · в другой раскладке «{}»",
                    condition_text(&rule),
                    action_text(&rule),
                    other_layout_text(&rule.pattern, rule.source_group)
                ),
            );
            // The letters are the user's own text, not Pango markup.
            row.set_use_markup(false);
            let delete = centered(gtk::Button::from_icon_name("user-trash-symbolic"));
            delete.set_tooltip_text(Some("Удалить правило"));
            delete.add_css_class("flat");
            let item = rule.clone();
            delete.connect_clicked(move |_| gtk_app::remove_rule(&item));
            row.add_suffix(&delete);
            row.set_activatable(true);
            let item = rule.clone();
            row.connect_activated(move |_| gtk_app::open_rule_editor_for(draft_from_rule(&item)));
            live.learning_group.add(&row);
            rows.push(row);
        }
        live.learning_status.set_subtitle(&learning_summary(rows.len()));
        *self.rule_rows.borrow_mut() = rows;
    }

    /// The languages page's model rows: they are filled once the engine has loaded its models.
    pub fn refresh_models(&self) {
        let live = &self.live;
        for row in self.model_rows.borrow_mut().drain(..) {
            live.models_group.remove(&row);
        }
        let info = self.services.models.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let mut rows = Vec::new();
        if !info.loaded {
            let row = action_row("Языковые модели", &self.services.engine.snapshot().last_action);
            live.models_group.add(&row);
            rows.push(row);
        }
        for (group, (words, source)) in info.languages.iter().enumerate() {
            let row = action_row(if group == 0 { "Английская" } else { "Русская" }, &format!("{} слов · {source}", thousands(*words)));
            row.add_suffix(&image("emblem-ok-symbolic"));
            live.models_group.add(&row);
            rows.push(row);
        }
        if info.loaded {
            let subtitle = if info.intent_available {
                let version = if info.intent_version.is_empty() { "unknown" } else { info.intent_version.as_str() };
                let checksum = if info.intent_checksum.is_empty() { "unknown".to_owned() } else { info.intent_checksum.chars().take(CHECKSUM_DISPLAY_LENGTH_CHARACTERS).collect() };
                format!("Версия {version} · {checksum}")
            } else {
                format!("Безопасный fallback · {}", info.intent_error)
            };
            let row = action_row("Линейная модель намерения", &subtitle);
            row.add_suffix(&image(if info.intent_available { "emblem-ok-symbolic" } else { "dialog-warning-symbolic" }));
            live.models_group.add(&row);
            rows.push(row);
        }
        *self.model_rows.borrow_mut() = rows;
    }

    fn application_catalog(&self) -> Rc<Vec<ApplicationChoice>> {
        self.catalog.borrow_mut().get_or_insert_with(|| Rc::new(installed_application_choices())).clone()
    }

    fn refresh_application_exclusions(&self) {
        let group = &self.live.apps_group;
        for row in self.application_rows.borrow_mut().drain(..) {
            group.remove(&row);
        }
        let catalog = self.application_catalog();
        let applications = setting_strings(&self.services.settings, "exclusions.applications");
        let mut rows = Vec::new();
        if applications.is_empty() {
            let row = action_row("Исключений пока нет", "Добавьте приложение одним из способов выше.");
            row.add_prefix(&image("emblem-ok-symbolic"));
            group.add(&row);
            rows.push(row);
        }
        for identifier in applications {
            let choice = catalog.iter().find(|choice| choice.identifier.to_lowercase() == identifier.to_lowercase());
            let row = action_row(choice.map_or(identifier.as_str(), |choice| choice.name.as_str()), &format!("WM_CLASS / executable: {identifier}"));
            match choice.and_then(|choice| choice.icon.as_ref()) {
                Some(icon) => row.add_prefix(&gtk::Image::from_gicon(icon)),
                None => row.add_prefix(&image("application-x-executable-symbolic")),
            }
            let remove = centered(gtk::Button::from_icon_name("list-remove-symbolic"));
            remove.set_tooltip_text(Some(&format!("Удалить {identifier} из исключений")));
            let value = identifier.clone();
            remove.connect_clicked(move |_| gtk_app::remove_application_exclusion(&value));
            row.add_suffix(&remove);
            group.add(&row);
            rows.push(row);
        }
        *self.application_rows.borrow_mut() = rows;
    }

    // ------------------------------------------------------------------------------------------- actions with a dialog

    pub fn add_application_exclusion(&self, identifier: &str, display_name: &str) {
        let value = identifier.trim();
        let shown = if display_name.is_empty() { value } else { display_name };
        if value.is_empty() {
            self.toast("Не удалось определить имя приложения");
            return;
        }
        let mut applications = setting_strings(&self.services.settings, "exclusions.applications");
        if applications.iter().any(|item| item.to_lowercase() == value.to_lowercase()) {
            self.toast(&format!("{shown} уже находится в исключениях"));
            return;
        }
        applications.push(value.to_owned());
        store(&self.services.settings, "exclusions.applications", json!(applications));
        self.toast(&format!("Добавлено в исключения: {shown}"));
    }

    pub fn remove_application_exclusion(&self, identifier: &str) {
        let applications: Vec<String> =
            setting_strings(&self.services.settings, "exclusions.applications").into_iter().filter(|item| item.to_lowercase() != identifier.to_lowercase()).collect();
        store(&self.services.settings, "exclusions.applications", json!(applications));
        self.toast(&format!("Удалено из исключений: {identifier}"));
    }

    fn add_manual_application(&self) {
        let value = self.live.manual_entry.text().trim().to_owned();
        self.add_application_exclusion(&value, "");
        if !value.is_empty() {
            self.live.manual_entry.set_text("");
        }
    }

    fn start_active_application_capture(&self) {
        self.live.active_app_row.set_subtitle(&format!("Переключитесь в нужное приложение — захват через {}…", capture_delay()));
        self.toast("KeySwitch скрыт: активируйте окно, которое нужно исключить");
        self.window.set_visible(false);
        glib::timeout_add_local_once(std::time::Duration::from_millis(u64::from(APPLICATION_CAPTURE_DELAY_MS)), gtk_app::finish_application_capture);
    }

    pub fn finish_active_application_capture(&self) {
        self.capture_application(self.services.backend.active_application().trim());
    }

    /// The program in front has been chosen: it goes on the list, unless it is none or this one.
    pub fn capture_application(&self, application: &str) {
        self.window.present();
        if application.is_empty() || application.to_lowercase() == "keyswitch" {
            self.live.active_app_row.set_subtitle("Окно не выбрано. Повторите и переключитесь в другое приложение.");
            self.toast("Не удалось выбрать внешнее приложение");
            return;
        }
        self.live.active_app_row.set_subtitle(&format!("Выбрано окно: {application}"));
        self.add_application_exclusion(application, "");
    }

    fn show_application_picker(&self) {
        let dialog = adw::Dialog::builder()
            .content_width(APPLICATION_PICKER_DIALOG_SIDE_PIXELS)
            .content_height(APPLICATION_PICKER_DIALOG_SIDE_PIXELS)
            .build();
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        let title = gtk::Label::new(Some("Выберите приложение"));
        styled(&title, "heading");
        header.set_title_widget(Some(&title));
        let close = gtk::Button::builder().icon_name("window-close-symbolic").tooltip_text("Закрыть").build();
        let weak = dialog.downgrade();
        close.connect_clicked(move |_| with_live(&weak, |dialog| {
            dialog.close();
        }));
        header.pack_start(&close);
        toolbar.add_top_bar(&header);

        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(GTK_STANDARD_SPACING_PIXELS)
            .margin_top(DIALOG_CONTENT_MARGIN_PIXELS)
            .margin_bottom(DIALOG_CONTENT_MARGIN_PIXELS)
            .margin_start(DIALOG_CONTENT_MARGIN_PIXELS)
            .margin_end(DIALOG_CONTENT_MARGIN_PIXELS)
            .build();
        let search = gtk::SearchEntry::builder().placeholder_text("Поиск по названию, WM_CLASS или executable").build();
        content.append(&search);
        let applications = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::Single).build();
        applications.add_css_class("boxed-list");
        let catalog = self.application_catalog();
        for choice in catalog.iter() {
            let row = action_row(&choice.name, &format!("{} · {}", choice.identifier, choice.desktop_id));
            if let Some(icon) = &choice.icon {
                row.add_prefix(&gtk::Image::from_gicon(icon));
            }
            applications.append(&row);
        }
        let searched = catalog.clone();
        let weak_search = search.downgrade();
        applications.set_filter_func(move |row| {
            let wanted = weak_search.upgrade().map(|search| search.text().trim().to_lowercase()).unwrap_or_default();
            searched[row.index().to::<usize>()].search_text().contains(&wanted)
        });
        let weak_list = applications.downgrade();
        search.connect_search_changed(move |_| with_live(&weak_list, |list| list.invalidate_filter()));
        let weak = dialog.downgrade();
        applications.connect_row_activated(move |_, row| {
            let choice = &catalog[row.index().to::<usize>()];
            gtk_app::add_application_exclusion(&choice.identifier, &choice.name);
            with_live(&weak, |dialog| {
                dialog.close();
            });
        });
        let scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&applications).build();
        content.append(&scroll);
        toolbar.set_content(Some(&content));
        dialog.set_child(Some(&toolbar));
        dialog.present(Some(&self.window));
        search.grab_focus();
    }

    fn confirm(&self, heading: &str, body: &str, response: &str, accept: &str, action: impl Fn() + 'static, close_response: bool) {
        let dialog = adw::AlertDialog::new(Some(heading), Some(body));
        dialog.add_response("cancel", "Отмена");
        dialog.add_response(response, accept);
        dialog.set_response_appearance(response, adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        if close_response {
            dialog.set_close_response("cancel");
        }
        let expected = response.to_owned();
        dialog.connect_response(None, move |_, chosen| {
            if chosen == expected {
                action();
            }
        });
        dialog.present(Some(&self.window));
    }

    pub fn confirm_clear_history(&self) {
        self.confirm(
            "Очистить историю?",
            "Будут удалены только пары исправленных слов. Настройки не изменятся.",
            "clear",
            "Очистить",
            gtk_app::clear_history,
            true,
        );
    }

    pub fn confirm_clear_learning(&self) {
        self.confirm(
            "Удалить все правила?",
            "Все правила переключения будут удалены. Обычные настройки и история не изменятся.",
            "all",
            "Удалить все",
            gtk_app::clear_rules,
            true,
        );
    }

    pub fn confirm_reset(&self) {
        self.confirm(
            "Сбросить настройки?",
            "Автокоррекция, горячие клавиши и исключения вернутся к значениям по умолчанию.",
            "reset",
            "Сбросить",
            gtk_app::reset_settings,
            false,
        );
    }

    pub fn copy_diagnostics(&self) {
        let settings = &self.services.settings;
        let info = self.services.models.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let loading = || self.services.engine.snapshot().last_action;
        let context = ReportContext {
            version: self.services.version.to_owned(),
            os: os_description(),
            intent_model: if info.loaded { info.intent_summary.clone() } else { loading() },
            context_model: if info.loaded { info.context_status.clone() } else { loading() },
            context_mode: setting_text(settings, "detection.context_policy", "assist"),
            read_field: python_bool(setting_flag(settings, "detection.context_read_field", false)),
            technical_logging: setting_flag(settings, "diagnostics.technical_logging", false),
            technical_log: log_path().display().to_string(),
            log_state: status_repr(&log_status()),
        };
        let text = diagnostics_report(&self.probe, &context);
        WidgetExt::display(&self.window).clipboard().set_text(&text);
        self.toast("Диагностика скопирована");
    }

    fn open_update_release(&self) {
        self.open_release(&self.services.updates.snapshot().release_url);
    }

    /// The page of the release, with what the system's browser answers.
    pub fn open_release(&self, url: &str) {
        if url.is_empty() {
            self.toast("Сначала проверьте наличие новой версии");
            return;
        }
        if let Err(message) = open_uri(url) {
            self.toast(&format!("Не удалось открыть выпуск: {message}"));
        }
    }

    pub fn open_log_directory(&self) {
        self.open_folder(&gio::File::for_path(log_directory()).uri());
    }

    /// A folder in the system's file manager, with what that answers.
    pub fn open_folder(&self, uri: &str) {
        if let Err(message) = open_uri(uri) {
            self.toast(&format!("Не удалось открыть папку журнала: {message}"));
        }
    }

    pub fn check_updates(&self) {
        if !self.services.updates.check(false, false) {
            self.toast("Проверка обновлений уже выполняется");
        }
    }

    /// Cancels the debounced saves of the text editors: the window is going away.
    pub fn flush_pending_saves(&self) {
        for (_, source) in self.save_sources.borrow_mut().drain() {
            source.remove();
        }
    }
}

/// What the diagnostics page says of the backend, in the words of its session.
fn backend_rows(probe: &BackendProbe, group: &str) -> [(&'static str, String); 5] {
    let display = if probe.display.is_empty() { "—" } else { probe.display.as_str() };
    let capture = if probe.available { &probe.record_version } else { &probe.error }.clone();
    if probe.session_type == "wayland" {
        [
            ("Сеанс", format!("{} · {display}", probe.session_type)),
            ("Перехват ввода", capture),
            ("Синтез ввода", probe.xtest_version.clone()),
            ("Раскладки", probe.xkb_version.clone()),
            ("Активная группа", group.to_owned()),
        ]
    } else {
        [
            ("Сеанс", format!("{} · DISPLAY {display}", probe.session_type)),
            ("XRecord", capture),
            ("XTEST", probe.xtest_version.clone()),
            ("XKB", probe.xkb_version.clone()),
            ("XKB-группа", group.to_owned()),
        ]
    }
}

fn python_bool(value: bool) -> String {
    if value { "True" } else { "False" }.to_owned()
}

fn capture_delay() -> String {
    counted(f64::from(APPLICATION_CAPTURE_DELAY_MS) / 1000.0, SECONDS)
}

// ------------------------------------------------------------------------------------------------------------- the pages

struct Dashboard {
    hero_title: gtk::Label,
    hero_subtitle: gtk::Label,
    hero_pill: gtk::Label,
    hero_action: gtk::Label,
    stat_corrections: gtk::Label,
    stat_layout: gtk::Label,
    stat_backend: gtk::Label,
    history: gtk::ListBox,
}

struct Automation {
    group: adw::PreferencesGroup,
    status: adw::ActionRow,
}

struct Languages {
    active: gtk::Label,
    models: adw::PreferencesGroup,
}

struct Exceptions {
    group: adw::PreferencesGroup,
    active_app_row: adw::ActionRow,
    manual_entry: gtk::Entry,
}

struct UpdatesPage {
    status: adw::ActionRow,
    version: adw::ActionRow,
    check: gtk::Button,
    open: gtk::Button,
}

struct HistoryPage {
    total: gtk::Label,
    list: gtk::ListBox,
}

struct Builder<'a> {
    services: Rc<Services>,
    controls: &'a RefCell<HashMap<String, Control>>,
    save_sources: Rc<RefCell<HashMap<String, glib::SourceId>>>,
    probe: &'a BackendProbe,
    pages: PageBuilder<'a>,
}

fn stat_card(parent: &gtk::Box, value: &str, caption: &str) -> gtk::Label {
    let card = gtk::Box::new(gtk::Orientation::Vertical, STAT_CARD_SPACING_PIXELS);
    styled(&card, "stat-card");
    card.set_hexpand(true);
    let number = label(value);
    styled(&number, "stat-value");
    let description = label(caption);
    styled(&description, "muted");
    card.append(&number);
    card.append(&description);
    parent.append(&card);
    number
}

impl Builder<'_> {
    fn settings(&self) -> Arc<SettingsStore> {
        self.services.settings.clone()
    }

    fn register(&self, path: &str, control: Control) {
        self.controls.borrow_mut().insert(path.to_owned(), control);
    }

    fn switch_row(&self, path: &str, title: &str, subtitle: &str) -> adw::SwitchRow {
        let row = adw::SwitchRow::builder().title(title).subtitle(subtitle).build();
        row.set_active(setting_flag(&self.services.settings, path, false));
        let settings = self.settings();
        let key = path.to_owned();
        row.connect_active_notify(move |row| store(&settings, &key, json!(row.is_active())));
        self.register(path, Control::Switch(row.clone()));
        row
    }

    /// A spin row; `whole` stores an integer, otherwise a float rounded to `digits`.
    fn spin_row(&self, path: &str, title: &str, subtitle: &str, range: (f64, f64, f64), digits: u32, default: f64) -> adw::SpinRow {
        let row = adw::SpinRow::with_range(range.0, range.1, range.2);
        row.set_title(title);
        row.set_subtitle(subtitle);
        row.set_digits(digits);
        row.set_value(setting_number(&self.services.settings, path, default));
        let settings = self.settings();
        let key = path.to_owned();
        row.connect_value_notify(move |row| {
            let value = row.value();
            let stored = match key.as_str() {
                "detection.minimum_length" | "detection.early_switch_min_length" => json!(value.to::<i64>()),
                "detection.pause_delay_seconds" => json!((value * 10.0).round() / 10.0),
                _ => json!(value),
            };
            store(&settings, &key, stored);
        });
        self.register(path, Control::Spin(row.clone()));
        row
    }

    fn combo_row(&self, path: &str, title: &str, subtitle: &str, labels: &[&str], keys: Vec<String>, selected: u32, track: bool) -> adw::ComboRow {
        let row = adw::ComboRow::builder().title(title).subtitle(subtitle).model(&gtk::StringList::new(labels)).build();
        row.set_selected(selected);
        let settings = self.settings();
        let key = path.to_owned();
        row.connect_selected_notify(move |row| {
            if let Some(value) = keys.get(row.selected().to::<usize>()) {
                if key == "appearance.theme" {
                    gtk_app::set_theme(value);
                } else {
                    store(&settings, &key, json!(value));
                }
            }
        });
        if track {
            self.register(path, Control::Combo(row.clone()));
        }
        row
    }

    fn text_editor(&self, path: &str) -> gtk::TextView {
        let view = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .top_margin(TEXT_EDITOR_PADDING_PIXELS)
            .bottom_margin(TEXT_EDITOR_PADDING_PIXELS)
            .left_margin(TEXT_EDITOR_PADDING_PIXELS)
            .right_margin(TEXT_EDITOR_PADDING_PIXELS)
            .build();
        view.set_size_request(-1, WORDS_EDITOR_HEIGHT_PIXELS);
        let buffer = view.buffer();
        buffer.set_text(&setting_strings(&self.services.settings, path).join("\n"));
        let sources = self.save_sources.clone();
        let settings = self.settings();
        let key = path.to_owned();
        buffer.connect_changed(move |buffer| {
            if let Some(old) = sources.borrow_mut().remove(&key) {
                old.remove();
            }
            let weak = buffer.downgrade();
            let (settings, sources_again, key_again) = (settings.clone(), sources.clone(), key.clone());
            let source = glib::timeout_add_local_once(std::time::Duration::from_millis(u64::from(TEXT_SAVE_DEBOUNCE_MS)), move || {
                sources_again.borrow_mut().remove(&key_again);
                with_live(&weak, |buffer| {
                    let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
                    let values: Vec<&str> = text.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
                    store(&settings, &key_again, json!(values));
                });
            });
            sources.borrow_mut().insert(key.clone(), source);
        });
        view
    }

    fn hotkey_row(&self, path: &str, title: &str, subtitle: &str) -> adw::ActionRow {
        let row = action_row(title, subtitle);
        let entry = gtk::Entry::builder().text(setting_text(&self.services.settings, path, "").as_str()).width_chars(HOTKEY_ENTRY_WIDTH_CHARACTERS).valign(gtk::Align::Center).build();
        let (settings, key) = (self.settings(), path.to_owned());
        entry.connect_activate({
            let (settings, key) = (settings.clone(), key.clone());
            move |entry| save_hotkey(&settings, &key, entry)
        });
        let focus = gtk::EventControllerFocus::new();
        let weak = entry.downgrade();
        focus.connect_leave(move |_| with_live(&weak, |entry| save_hotkey(&settings, &key, &entry)));
        entry.add_controller(focus);
        row.add_suffix(&entry);
        row
    }

    fn tip(&self, page: &gtk::Box, icon: &str, text: &str) {
        page.append(&card(icon, "privacy-card", text));
    }

    fn dashboard(&mut self, sidebar_detail: &gtk::Label) -> Dashboard {
        let _ = sidebar_detail;
        let page = self.pages.page("dashboard", "Обзор", "Состояние фоновой автокоррекции и быстрая проверка прямо в приложении.");
        let hero = gtk::Box::new(gtk::Orientation::Vertical, GTK_STANDARD_SPACING_PIXELS);
        styled(&hero, "hero-card");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, HERO_TOP_ROW_SPACING_PIXELS);
        let copy = gtk::Box::new(gtk::Orientation::Vertical, HERO_COPY_SPACING_PIXELS);
        let hero_title = label("KeySwitch готов");
        styled(&hero_title, "hero-title");
        let hero_subtitle = gtk::Label::builder().label(HERO_SUBTITLE).xalign(0.0).wrap(true).build();
        styled(&hero_subtitle, "hero-subtitle");
        copy.append(&hero_title);
        copy.append(&hero_subtitle);
        copy.set_hexpand(true);
        let hero_pill = gtk::Label::builder().label("АКТИВНО").valign(gtk::Align::Start).build();
        styled(&hero_pill, "status-pill");
        top.append(&copy);
        top.append(&hero_pill);
        hero.append(&top);
        let hero_action = label("Ожидание ввода");
        styled(&hero_action, "hero-word");
        hero_action.set_ellipsize(pango::EllipsizeMode::End);
        hero.append(&hero_action);
        page.append(&hero);

        let stats = gtk::Box::new(gtk::Orientation::Horizontal, GTK_STANDARD_SPACING_PIXELS);
        let stat_corrections = stat_card(&stats, "0", "исправлений");
        let stat_layout = stat_card(&stats, "—", "текущая раскладка");
        let stat_backend = stat_card(&stats, "…", "движок ввода");
        page.append(&stats);

        let check = group_with(
            "Проверить сейчас",
            "Выберите EN и напечатайте ghbdtn: после паузы появится «привет». Можно также нажать пробел. Затем попробуйте руддщ в RU.",
        );
        let test_row = action_row("Тестовое поле", "Глобальный перехват работает и внутри этого окна");
        let test_entry = gtk::Entry::builder()
            .placeholder_text("Начните печатать здесь…")
            .hexpand(true)
            .width_chars(TEST_ENTRY_WIDTH_CHARACTERS)
            .valign(gtk::Align::Center)
            .build();
        // The field is typed in as any other program is: the engine is told when it has the keyboard, since a layout picked
        // by hand elsewhere in this window says nothing of the text.
        let focus = gtk::EventControllerFocus::new();
        let engine = self.services.engine.clone();
        focus.connect_enter({
            let engine = engine.clone();
            move |_| engine.set_test_field_focused(true)
        });
        focus.connect_leave(move |_| engine.set_test_field_focused(false));
        test_entry.add_controller(focus);
        test_row.add_suffix(&test_entry);
        check.add(&test_row);
        page.append(&check);

        let recent = label("Последние исправления");
        styled(&recent, "section-heading");
        page.append(&recent);
        let history = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
        history.add_css_class("boxed-list");
        page.append(&history);
        Dashboard { hero_title, hero_subtitle, hero_pill, hero_action, stat_corrections, stat_layout, stat_backend, history }
    }

    fn automation(&mut self) -> Automation {
        let page = self.pages.page(
            "automation",
            "Автокоррекция",
            "Настройте момент срабатывания и насколько осторожно KeySwitch принимает решение.",
        );
        let settings = self.settings();
        let behavior = group("Распознавание");
        behavior.add(&self.switch_row(
            "enabled",
            "Автоматически исправлять раскладку",
            "Отключает автоматические замены; ручные горячие клавиши остаются доступны",
        ));
        behavior.add(&self.switch_row(
            "detection.respect_manual_layout",
            "Доверять ручной смене раскладки",
            "Не исправлять первое завершённое слово после переключения раскладки пользователем",
        ));
        behavior.add(&self.switch_row(
            "detection.fix_double_capitals",
            "Исправлять ДВе заглавные",
            "Слово, набранное с двумя заглавными в начале (ПРивет, HEllo), становится обычным (Привет, Hello), \
             если словарь его раскладки знает такое слово. Аббревиатуры, слова из одних заглавных, короткие \
             слова и слова из исключений не меняются",
        ));
        behavior.add(&self.spin_row(
            "detection.minimum_length",
            "Минимальная длина слова",
            "Минимум для базового детектора; явные правила, короткие исключения и контекстный помощник могут разрешить замену раньше",
            (MINIMUM_WORD_LENGTH_SETTING_MIN, MINIMUM_WORD_LENGTH_SETTING_MAX, 1.0),
            0,
            DEFAULT_MINIMUM_WORD_LENGTH.to::<f64>(),
        ));
        behavior.add(&self.spin_row(
            "detection.confidence",
            "Порог уверенности",
            "Выше — строже резервные эвристики; пороги обученных моделей не меняются",
            (CONFIDENCE_SETTING_MIN, CONFIDENCE_SETTING_MAX, CONFIDENCE_SETTING_STEP),
            1,
            DEFAULT_CONFIDENCE_THRESHOLD,
        ));
        behavior.add(&self.switch_row(
            "detection.aggressive",
            "Расширенное распознавание",
            "Когда ни один словарь не знает слово, решает символьная модель. С этой настройкой её планка ниже: больше незнакомых слов переводится, а слово никогда не превращается в строку с пунктуацией (проверяется отдельно). Включено по умолчанию.",
        ));
        behavior.add(&self.switch_row(
            "detection.context_aware",
            "Учитывать контекст",
            "Фраза и приложение помогают моделям выбрать действие; без контекста работает базовое распознавание. Контекст хранится только в памяти",
        ));
        let current = setting_text(&settings, "detection.context_policy", "assist");
        behavior.add(&self.combo_row(
            "detection.context_policy",
            "Контекстный ИИ-помощник",
            "Отдельные локальные модели для слов и префиксов. «Только наблюдать» не применяет их решения; базовая автокоррекция продолжает работать",
            &["Исправлять и предлагать", "Только наблюдать", "Выключен"],
            CONTEXT_MODES.iter().map(|mode| (*mode).to_owned()).collect(),
            index_of(&CONTEXT_MODES, &current, CONTEXT_MODE_OFF_INDEX),
            true,
        ));
        behavior.add(&self.switch_row(
            "detection.context_read_field",
            "Читать контекст активного поля",
            "Читать текст рядом с курсором локально через доступность ОС; распознанные защищённые поля исключаются, не все приложения сообщают о них",
        ));
        let catch_up = setting_text(&settings, "detection.field_catch_up", DEFAULT_FIELD_CATCH_UP);
        let catch_up = if FIELD_CATCH_UP_CHOICES.contains(&catch_up.as_str()) { catch_up } else { DEFAULT_FIELD_CATCH_UP.to_owned() };
        let catch_up_labels: Vec<&str> = FIELD_CATCH_UP_CHOICES.iter().map(|key| table_text(&FIELD_CATCH_UP_LABELS, key)).collect();
        behavior.add(&self.combo_row(
            "detection.field_catch_up",
            FIELD_CATCH_UP_TITLE,
            FIELD_CATCH_UP_DESCRIPTION,
            &catch_up_labels,
            FIELD_CATCH_UP_CHOICES.iter().map(|key| (*key).to_owned()).collect(),
            index_of(&FIELD_CATCH_UP_CHOICES, &catch_up, 0),
            true,
        ));
        behavior.add(&self.switch_row("detection.correct_typos", CORRECT_TYPOS_TITLE, &correct_typos_description()));
        behavior.add(&self.switch_row("detection.correct_typos_english", CORRECT_TYPOS_ENGLISH_TITLE, CORRECT_TYPOS_ENGLISH_DESCRIPTION));
        behavior.add(&self.switch_row("detection.protect_code", "Защищать код и сокращения", "Не трогать URL, пути, слова с цифрами, ALL-CAPS и camelCase"));
        behavior.add(&self.switch_row(
            "detection.intent_model_enabled",
            "Базовая модель слов (KSLM)",
            "При отключении работают резервные эвристики. Контекстные модели слов и префиксов настраиваются отдельно",
        ));
        behavior.add(&self.switch_row(
            "detection.early_switch",
            "Ранняя смена раскладки",
            "Менять раскладку уже по первым буквам, не дожидаясь конца слова. Включено по умолчанию: пара моделей context-v3 + prefix-v2 прошла запечатанную проверку в этом режиме без порчи правильно набранного текста. В assist с учётом контекста — обученная модель префиксов; без контекста или в off/shadow — словарный алгоритм.",
        ));
        behavior.add(&self.spin_row(
            "detection.early_switch_min_length",
            "Символов до ранней смены",
            &format!(
                "Минимум для проверки; значение {} действует только в словарном алгоритме. Модель проверяет префиксы длиной {PREFIX_MIN_CHARACTERS}–{PREFIX_MAX_CHARACTERS} и ждёт при сомнении",
                EARLY_SWITCH_MIN_LENGTH_SETTING_MIN.to::<i64>()
            ),
            (EARLY_SWITCH_MIN_LENGTH_SETTING_MIN, EARLY_SWITCH_MIN_LENGTH_SETTING_MAX, 1.0),
            0,
            DEFAULT_EARLY_SWITCH_MIN_LENGTH,
        ));
        page.append(&behavior);

        let quirks = group_with("Особенности программ", "Правила о синтаксисе самих приложений, а не о языке. Каждое включается отдельно.");
        for head in MENTION_HEADS {
            quirks.add(&self.switch_row(head.setting, head.title, head.description));
        }
        page.append(&quirks);

        let learning = group_with(
            "Правила переключения",
            "Правило говорит, что делать со словом, которое совпадает с сочетанием букв, начинается с него или содержит его: переводить в другую раскладку или нет. \
             Дважды нажмите Pause, затем Enter: окно правила откроется заполненным для последнего слова. Правило появляется только по кнопке OK.",
        );
        learning.add(&self.switch_row("detection.learning", "Правила действуют", "Двойное нажатие Pause предлагает правило для последнего слова"));
        let status = action_row("Правила пользователя", &learning_summary(self.services.engine.learning.count()));
        let add_rule = button("Добавить…");
        add_rule.connect_clicked(|_| gtk_app::open_rule_editor_for(RuleDraft::default()));
        status.add_suffix(&add_rule);
        let clear = button("Удалить все");
        clear.add_css_class("destructive-action");
        clear.connect_clicked(|_| gtk_app::with_window(MainWindow::confirm_clear_learning));
        status.add_suffix(&clear);
        learning.add(&status);
        page.append(&learning);

        let triggers = group("Исправлять после");
        triggers.add(&self.switch_row("detection.correct_on_pause", "Паузы в наборе", "Проверять текущее слово после паузы без ввода"));
        triggers.add(&self.spin_row(
            "detection.pause_delay_seconds",
            "Длительность паузы, с",
            "Сколько ждать без ввода, прежде чем проверить незавершённое слово",
            (PAUSE_DELAY_SETTING_MIN_SECONDS, PAUSE_DELAY_SETTING_MAX_SECONDS, PAUSE_DELAY_SETTING_STEP_SECONDS),
            1,
            DEFAULT_PAUSE_DELAY_SECONDS,
        ));
        triggers.add(&self.switch_row("detection.correct_on_space", "Пробела", "Основной и самый предсказуемый триггер"));
        for (path, title) in [("detection.correct_on_enter", "Enter"), ("detection.correct_on_tab", "Tab")] {
            let row = self.switch_row(path, title, "Замена после отправки или перехода небезопасна. Используйте пробел, паузу или Pause заранее.");
            row.set_sensitive(false);
            triggers.add(&row);
        }
        triggers.add(&self.switch_row("detection.correct_on_punctuation", "Знака препинания", "Точка, запятая, вопросительный знак и другие границы слова"));
        page.append(&triggers);

        page.append(&card(
            "dialog-information-symbolic",
            "privacy-card",
            "Автокоррекция срабатывает консервативно: целевая форма должна быть явно лучше исходной. Ручное преобразование доступно горячей клавишей Pause.",
        ));
        Automation { group: learning, status }
    }

    fn languages(&mut self) -> Languages {
        let page = self.pages.page(
            "languages",
            "Языки и раскладки",
            "KeySwitch использует две первые XKB-группы текущего сеанса и сопоставляет физические клавиши напрямую.",
        );
        let pair = group("Активная пара");
        let first = action_row("Группа 1", "Английский — системная раскладка us");
        first.add_prefix(&gtk::Label::new(Some("EN")));
        first.add_suffix(&image("object-select-symbolic"));
        pair.add(&first);
        let second = action_row("Группа 2", "Русский — системная раскладка ru");
        second.add_prefix(&gtk::Label::new(Some("RU")));
        pair.add(&second);
        let active_row = action_row("Сейчас активна", "Определяется через XKB");
        let active = label("—");
        styled(&active, "accent");
        active_row.add_suffix(&active);
        pair.add(&active_row);
        page.append(&pair);

        let models = group_with(
            "Локальные языковые модели",
            "Текст не отправляется в сеть. Используются словари, установленные вместе с Ubuntu Onboard, плюс небольшой встроенный словарь.",
        );
        page.append(&models);

        let example = group("Пример физического соответствия");
        example.add(&action_row("ghbdtn  ⇄  привет", "Одинаковые физические клавиши в группах EN и RU"));
        example.add(&action_row("руддщ  ⇄  hello", "Исправление работает в обе стороны"));
        page.append(&example);
        Languages { active, models }
    }

    fn hotkeys(&mut self) {
        let page = self.pages.page("hotkeys", "Горячие клавиши", "Комбинации распознаются глобально. Нажмите Enter после редактирования поля.");
        let commands = group("Команды");
        commands.add(&self.hotkey_row("hotkeys.toggle", "Пауза / продолжение", "Временно выключает автоматические исправления"));
        commands.add(&self.hotkey_row("hotkeys.convert_last", "Преобразовать последнее слово", "Меняет раскладку принудительно, без решения словаря"));
        commands.add(&self.hotkey_row(
            "hotkeys.undo",
            "Отменить исправление",
            &format!(
                "Возвращает последнее исправленное слово таким, каким его набрали, и после пробелов за ним; после любой другой клавиши отменять уже нечего. Доступно {} после исправления",
                counted(UNDO_AVAILABLE_WINDOW_SECONDS, SECONDS)
            ),
        ));
        for name in SELECTION_HOTKEYS {
            commands.add(&self.hotkey_row(&format!("hotkeys.{name}"), table_text(&SELECTION_HOTKEY_TITLES, name), table_text(&SELECTION_HOTKEY_DESCRIPTIONS, name)));
        }
        page.append(&commands);
        page.append(&{
            let syntax = gtk::Box::new(gtk::Orientation::Horizontal, GTK_STANDARD_SPACING_PIXELS);
            styled(&syntax, "privacy-card");
            syntax.append(&image("input-keyboard-symbolic"));
            syntax.append(&gtk::Label::builder().label("Формат: Ctrl+Alt+P, Ctrl+Shift+Space или одиночная клавиша Pause. Модификаторы должны совпасть точно.").xalign(0.0).wrap(true).build());
            syntax
        });
    }

    fn exceptions(&mut self) -> Exceptions {
        let page = self.pages.page(
            "exceptions",
            "Исключения и приватность",
            "Отключите исправления для отдельных приложений или слов, где раскладка намеренно необычна.",
        );
        let picker = group_with("Добавить приложение", "Выберите окно прицелом, найдите установленное приложение в каталоге или укажите WM_CLASS вручную.");
        let active_app_row = action_row("Выбор приложения", &format!("При выборе окна KeySwitch скроется на {} — переключитесь в нужное приложение.", capture_delay()));
        let catalog = button("Из списка…");
        catalog.connect_clicked(|_| gtk_app::with_window(MainWindow::show_application_picker));
        let capture = button("Выбрать окно");
        capture.add_css_class("suggested-action");
        capture.connect_clicked(|_| gtk_app::with_window(MainWindow::start_active_application_capture));
        active_app_row.add_suffix(&catalog);
        active_app_row.add_suffix(&capture);
        picker.add(&active_app_row);

        let manual_row = action_row("Добавить вручную", "Имя WM_CLASS или executable, например code, firefox или telegram-desktop");
        let manual_entry =
            gtk::Entry::builder().placeholder_text("WM_CLASS").width_chars(MANUAL_APP_ENTRY_WIDTH_CHARACTERS).valign(gtk::Align::Center).build();
        manual_entry.connect_activate(|_| gtk_app::with_window(MainWindow::add_manual_application));
        let manual_button = button("Добавить");
        manual_button.connect_clicked(|_| gtk_app::with_window(MainWindow::add_manual_application));
        manual_row.add_suffix(&manual_entry);
        manual_row.add_suffix(&manual_button);
        picker.add(&manual_row);
        page.append(&picker);

        let apps = group_with("Приложения-исключения", "KeySwitch не исправляет ввод, если WM_CLASS активного окна содержит одно из этих имён.");
        page.append(&apps);

        let words = group_with("Игнорируемые слова", "По одному слову на строку. Проверяется исходная, ошибочно выглядящая форма.");
        let view = self.text_editor("exclusions.words");
        let editor_row = adw::ActionRow::new();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(WORDS_EDITOR_HEIGHT_PIXELS)
            .hexpand(true)
            .child(&view)
            .build();
        editor_row.set_child(Some(&scroll));
        words.add(&editor_row);
        page.append(&words);
        self.tip(
            &page,
            "security-high-symbolic",
            "KeySwitch держит в памяти только текущее слово. Полный поток клавиш не записывается; в историю попадают только пары выполненных исправлений. Для менеджеров паролей предустановлены исключения.",
        );
        Exceptions { group: apps, active_app_row, manual_entry }
    }

    fn system(&mut self) {
        let page = self.pages.page("system", "Внешний вид и система", "Поведение окна, уведомлений и запуск вместе с рабочим столом.");
        let settings = self.settings();
        let appearance = group("Оформление");
        let theme = setting_text(&settings, "appearance.theme", "system");
        appearance.add(&self.combo_row(
            "appearance.theme",
            "Тема",
            "Можно следовать системной теме или зафиксировать вариант",
            &["Системная", "Светлая", "Тёмная"],
            THEMES.iter().map(|key| (*key).to_owned()).collect(),
            index_of(&THEMES, &theme, 0),
            false,
        ));
        appearance.add(&self.switch_row(
            "appearance.show_indicator",
            "Значок в системной панели",
            "Щелчок открывает меню быстрых действий, средняя кнопка ставит движок на паузу",
        ));
        let style = setting_text(&settings, "appearance.indicator_style", "letters");
        appearance.add(&self.combo_row(
            "appearance.indicator_style",
            "Вид индикатора раскладки",
            "Показывать EN/RU или флаги США и России, как в Punto Switcher",
            &["EN / RU", "Флаги стран"],
            INDICATOR_STYLES.iter().map(|key| (*key).to_owned()).collect(),
            index_of(&INDICATOR_STYLES, &style, 0),
            true,
        ));
        appearance.add(&self.switch_row("general.close_to_tray", "Сворачивать при закрытии окна", "Фоновая коррекция продолжит работать"));
        page.append(&appearance);

        let feedback = group("Обратная связь");
        feedback.add(&self.switch_row("general.notifications", "Уведомлять об исправлении", "Показывать исходное и исправленное слово"));
        feedback.add(&self.switch_row("general.sound", "Звуковой сигнал", "Короткий системный сигнал после замены"));
        feedback.add(&self.switch_row("general.keep_history", "Сохранять историю исправлений", "Только пары слов; полный ввод никогда не сохраняется"));
        page.append(&feedback);

        let startup = group("Запуск");
        let autostart = adw::SwitchRow::builder()
            .title("Запускать после входа в систему")
            .subtitle("После перезагрузки KeySwitch запустится при следующем входе в рабочий стол")
            .build();
        autostart.set_active(AutostartManager::new(None).enabled());
        autostart.connect_active_notify(|row| gtk_app::autostart_toggled(row.is_active(), row));
        startup.add(&autostart);
        startup.add(&self.switch_row("general.start_hidden", "Запускать свёрнутым", "Окно не показывается, пока не нажать значок в панели"));
        page.append(&startup);

        let maintenance = group("Обслуживание");
        let reset_row = action_row("Вернуть настройки по умолчанию", "История исправлений и правила переключения при этом не удаляются");
        let reset = button("Сбросить");
        reset.add_css_class("destructive-action");
        reset.connect_clicked(|_| gtk_app::with_window(MainWindow::confirm_reset));
        reset_row.add_suffix(&reset);
        maintenance.add(&reset_row);
        page.append(&maintenance);
    }

    fn updates(&mut self) -> UpdatesPage {
        let page = self.pages.page("updates", "Обновления", "Проверка стабильных выпусков KeySwitch и безопасный переход к системной установке.");
        let snapshot = self.services.updates.snapshot();
        let state = group("Состояние");
        state.add(&action_row("Установленная версия", self.services.version));
        let status = action_row("Проверка обновлений", &snapshot.message);
        state.add(&status);
        let version = action_row("Новая версия", "Новых выпусков пока не найдено");
        state.add(&version);
        page.append(&state);

        let policy = group("Автоматическая проверка");
        policy.add(&self.switch_row(
            "updates.check_automatically",
            "Проверять автоматически",
            &format!(
                "Первая проверка через {} после запуска, затем раз в {}",
                counted(UPDATE_CHECK_INITIAL_DELAY_SECONDS.to::<f64>(), SECONDS),
                counted(UPDATE_CHECK_INTERVAL_SECONDS.to::<f64>() / 3600.0, HOURS)
            ),
        ));
        policy.add(&action_row(
            "Установка в Ubuntu",
            "KeySwitch уведомит о выпуске и откроет его страницу. Системный DEB обновляется с подтверждением через APT; фоновая установка без прав администратора отключена.",
        ));
        page.append(&policy);

        let actions = group("Действия");
        let action = action_row("GitHub Releases", "Метаданные и SHA-256 загружаются только из официального репозитория");
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, UPDATE_BUTTONS_SPACING_PIXELS);
        let check = gtk::Button::with_label("Проверить сейчас");
        check.connect_clicked(|_| gtk_app::with_window(MainWindow::check_updates));
        let open = gtk::Button::with_label("Открыть выпуск");
        open.set_sensitive(!snapshot.release_url.is_empty());
        open.connect_clicked(|_| gtk_app::with_window(MainWindow::open_update_release));
        buttons.append(&check);
        buttons.append(&open);
        action.add_suffix(&buttons);
        actions.add(&action);
        page.append(&actions);
        UpdatesPage { status, version, check, open }
    }

    fn history(&mut self) -> HistoryPage {
        let page = self.pages.page("history", "История исправлений", "Здесь хранятся только реально заменённые пары слов, приложение и время.");
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, GTK_COMPACT_SPACING_PIXELS);
        let total = gtk::Label::builder().label(plural_entries(0)).xalign(0.0).hexpand(true).build();
        styled(&total, "section-heading");
        let clear = gtk::Button::builder().label("Очистить историю").icon_name("user-trash-symbolic").build();
        clear.add_css_class("destructive-action");
        clear.connect_clicked(|_| gtk_app::with_window(MainWindow::confirm_clear_history));
        controls.append(&total);
        controls.append(&clear);
        page.append(&controls);
        let list = gtk::ListBox::builder().selection_mode(gtk::SelectionMode::None).build();
        list.add_css_class("boxed-list");
        page.append(&list);
        HistoryPage { total, list }
    }

    fn diagnostics(&mut self) {
        let page = self.pages.page("diagnostics", "О программе", "Версия, состояние системных расширений и пути к локальным данным.");
        let about = group("KeySwitch");
        about.add(&action_row("Версия", self.services.version));
        about.add(&action_row("Назначение", "Автоматическое исправление раскладки для Ubuntu (X11 и Wayland)"));
        page.append(&about);

        let probe = self.probe;
        let diagnostics = group("Диагностика backend");
        let group_text = group_text(probe.current_group);
        for (title, value) in backend_rows(probe, &group_text) {
            diagnostics.add(&action_row(title, &value));
        }
        diagnostics.add(&action_row("Операционная система", &os_description()));
        page.append(&diagnostics);

        let locations = group("Локальные данные");
        locations.add(&action_row("Настройки", &self.services.settings.path.display().to_string()));
        locations.add(&action_row("История", &self.services.history.path.display().to_string()));
        locations.add(&action_row("Правила переключения", &self.services.engine.learning.path.display().to_string()));
        page.append(&locations);

        let technical = group_with(
            "Технический журнал",
            "Включайте на время воспроизведения проблемы. Журнал содержит решения модели, названия приложений и может содержать введённые слова; текст из приложений-исключений скрывается.",
        );
        technical.add(&self.switch_row(
            "diagnostics.technical_logging",
            "Записывать подробную диагностику распознавания",
            &format!("По умолчанию выключено; включение начинает новый файл, журнал хранится только локально и ротируется по {}", rotation_summary(true)),
        ));
        let log_row = action_row("Файл журнала", &log_path().display().to_string());
        let open_log = centered(gtk::Button::from_icon_name("folder-symbolic"));
        open_log.set_tooltip_text(Some("Открыть папку журнала"));
        open_log.add_css_class("flat");
        open_log.connect_clicked(|_| gtk_app::with_window(MainWindow::open_log_directory));
        log_row.add_suffix(&open_log);
        log_row.set_activatable_widget(Some(&open_log));
        technical.add(&log_row);
        page.append(&technical);

        let copy = gtk::Button::builder().label("Скопировать диагностику").icon_name("edit-copy-symbolic").halign(gtk::Align::Start).build();
        copy.connect_clicked(|_| gtk_app::with_window(MainWindow::copy_diagnostics));
        page.append(&copy);

        if !probe.available {
            page.append(&card("dialog-error-symbolic", "error-card", &probe.error));
        }
    }
}

fn save_hotkey(settings: &SettingsStore, path: &str, entry: &gtk::Entry) {
    let value = entry.text().trim().to_owned();
    if value.is_empty() {
        gtk_app::with_window(|window| window.toast("Комбинация не может быть пустой"));
        entry.set_text(&setting_text(settings, path, ""));
        return;
    }
    store(settings, path, json!(value));
    gtk_app::with_window(|window| window.toast("Горячая клавиша сохранена"));
}

/// A rule of the store, as the window passes it around.
pub type Rule = LearnedRule;

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn programs_are_named_and_listed_once() {
        let raw = |class: &str, executable: &str, id: &str, display: &str, name: &str| RawChoice {
            startup_class: class.to_owned(),
            executable: executable.to_owned(),
            desktop_id: id.to_owned(),
            display_name: display.to_owned(),
            name: name.to_owned(),
            icon: None,
        };
        let choices = choices_of(
            [
                raw("Class", "exe", "id", "Показ", "имя"),
                raw("class", "other", "id2", "Копия", "копия"),
                raw("", "exe2", "id3", "  ", "Имя"),
                raw("", "", "id4", "", ""),
                raw("  ", "", "", "Никто", "Никто"),
            ]
            .into_iter(),
        );
        let found: Vec<(&str, &str)> = choices.iter().map(|choice| (choice.name.as_str(), choice.identifier.as_str())).collect();
        assert_eq!(found, [("id4", "id4"), ("Имя", "exe2"), ("Показ", "Class")]);
        assert!(choices[2].search_text().contains("class"));
    }

    #[test]
    fn settings_edited_by_hand_read_as_they_can() {
        let settings = SettingsStore::new(None);
        assert_eq!(setting_text(&settings, "no.such.text", "x"), "x");
        settings.set("a.number", json!(5), false).unwrap();
        settings.set("a.null", json!(null), false).unwrap();
        settings.set("a.list", json!(["b", 5]), false).unwrap();
        assert_eq!(setting_text(&settings, "a.number", "x"), "5");
        assert_eq!(setting_text(&settings, "a.null", "x"), "x");
        assert_eq!(setting_strings(&settings, "a.list"), ["b", "5"]);
        assert!(setting_strings(&settings, "a.number").is_empty());
        assert_eq!(python_bool(false), "False");
        assert_eq!(python_bool(true), "True");
    }

    #[test]
    fn the_diagnostics_speak_the_words_of_the_session() {
        let probe = |session: &str, available: bool, display: &str| BackendProbe {
            session_type: session.to_owned(),
            available,
            display: display.to_owned(),
            record_version: "1.13".to_owned(),
            error: "why not".to_owned(),
            ..BackendProbe::default()
        };
        assert_eq!(backend_rows(&probe("wayland", true, "wayland-0"), "0")[0].1, "wayland · wayland-0");
        assert_eq!(backend_rows(&probe("wayland", false, ""), "0")[0].1, "wayland · —");
        assert_eq!(backend_rows(&probe("wayland", false, ""), "0")[1].1, "why not");
        assert_eq!(backend_rows(&probe("x11", true, ":0"), "0")[1], ("XRecord", "1.13".to_owned()));
        assert_eq!(backend_rows(&probe("x11", false, ""), "0")[0].1, "x11 · DISPLAY —");
        assert_eq!(backend_rows(&probe("x11", false, ""), "0")[1].1, "why not");
    }

    #[test]
    fn counts_are_grouped_like_python() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn the_log_status_reads_like_a_python_dict() {
        let mut status = serde_json::Map::new();
        status.insert("path".into(), json!("/x"));
        status.insert("installed".into(), json!(true));
        status.insert("size".into(), json!(-1));
        assert_eq!(status_repr(&status), "{'path': '/x', 'installed': True, 'size': -1}");
    }
}

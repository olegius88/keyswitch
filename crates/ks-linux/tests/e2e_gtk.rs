//! The whole window under `packaging/e2e.sh`: the program started as a user starts it, on a private display, with every control
//! on every page changed and every button pressed, and the application's own calls made. With `KS_E2E` unset, it is skipped.
#![expect(unsafe_code, reason = "the test program names its folders before any thread starts")]
#![expect(clippy::unwrap_used, reason = "a test or a probe stops on what it did not expect")]
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod support;

use adw::prelude::*;
use gtk::glib;
use gtk::glib::translate::IntoGlib;
use gtk::gdk;
use ks_engine::rule_editor::RuleDraft;
use ks_config::history::{HistoryEntry, HistoryStore};
use ks_config::learning::{LearnedRule, LearningStore, RuleAction, RuleMatch};
use ks_engine::engine::{EngineSnapshot, LearningPrompt};
use ks_linux::gtk_hint::{HintCalls, HintClicked, HintDriver, HintScreen};
use ks_linux::gtk_model::{HINT_ACTION_NEVER, HINT_ACTION_OPEN, HINT_PLACE_ATTEMPTS, HINT_PLACE_RETRY_MS, HINT_WINDOW_TITLE};
use ks_linux::gtk_prompt::PromptAnchor;
use ks_linux::gtk_window::{MainWindow, ModelInfo};
use ks_linux::tray::TrayAction;
use ks_ui::hints::catalogue::LAYOUT_SWITCH_KEY_TITLE;
use ks_ui::hints::{Click, CorrectionInfo, EngineInput, HINT_OK_LABEL, HintPresenter, HintView, Hints, Moment, Opens, Place, RUNNING_PLATFORM, SettingsPage};
use ks_ui::windows_ui_model::HINTS_SETTING_TITLE;
use ks_config::constants::HINTS_SETTING;
use ks_config::config::SettingsStore;
use ks_updates::{UpdatePhase, UpdateSnapshot};
use serde_json::json;
use ks_linux::gtk_app;

const ROWS_TRIED: i32 = 30;

/// What the walk did last, for the watchdog to say.
static LAST: Mutex<String> = Mutex::new(String::new());
/// The notification daemon of the private bus, there before the program sends anything: an installed daemon would be
/// activated by the first notification and hold its name.
static NOTIFICATIONS: std::sync::OnceLock<Notifications> = std::sync::OnceLock::new();

fn note(what: impl Into<String>) {
    *LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = what.into();
}

fn every_widget(widget: &gtk::Widget, found: &mut Vec<gtk::Widget>) {
    found.push(widget.clone());
    let mut child = widget.first_child();
    while let Some(next) = child {
        every_widget(&next, found);
        child = next.next_sibling();
    }
}

/// Buttons that end the test's own window or leave the machine.
fn left_alone(widget: &gtk::Widget) -> bool {
    widget.downcast_ref::<gtk::Button>().and_then(gtk::Button::label).is_some_and(|label| label.contains("Выйти") || label.contains("Установить"))
}

/// The widgets a user acts on; a window holds thousands more, which only draw.
fn acts(widget: &gtk::Widget) -> bool {
    widget.is::<adw::SwitchRow>()
        || widget.is::<adw::SpinRow>()
        || widget.is::<adw::ComboRow>()
        || widget.is::<adw::EntryRow>()
        || widget.is::<gtk::Entry>()
        || widget.is::<gtk::SpinButton>()
        || widget.is::<gtk::Switch>()
        || widget.is::<gtk::CheckButton>()
        || widget.is::<gtk::DropDown>()
        || widget.is::<gtk::Button>()
        || widget.is::<gtk::ListBox>()
}

fn change(widget: &gtk::Widget) {
    if let Some(row) = widget.downcast_ref::<adw::SwitchRow>() {
        row.set_active(!row.is_active());
        row.set_active(!row.is_active());
    } else if let Some(row) = widget.downcast_ref::<adw::SpinRow>() {
        row.set_value(row.value() + 1.0);
        row.set_value(row.value() - 1.0);
    } else if let Some(row) = widget.downcast_ref::<adw::ComboRow>() {
        let (count, chosen) = (row.model().map_or(0, |model| model.n_items()), row.selected());
        for item in 0..count {
            row.set_selected(item);
        }
        // Nothing chosen, as a list being rebuilt reports for a moment.
        row.set_selected(gtk::INVALID_LIST_POSITION);
        row.set_selected(chosen);
    } else if let Some(row) = widget.downcast_ref::<adw::EntryRow>() {
        let text = row.text();
        row.set_text("ghbdtn");
        row.emit_by_name::<()>("apply", &[]);
        row.set_text(&text);
    } else if let Some(entry) = widget.downcast_ref::<gtk::Entry>() {
        let text = entry.text();
        entry.set_text("ghbdtn");
        entry.emit_activate();
        // Nothing typed: a hotkey row says it cannot be empty and takes back what it had.
        entry.set_text("");
        entry.emit_activate();
        entry.set_text(&text);
    } else if let Some(search) = widget.downcast_ref::<gtk::SearchEntry>() {
        search.set_text("no such program at all");
        search.emit_by_name::<()>("search-changed", &[]);
        search.set_text("");
        search.emit_by_name::<()>("search-changed", &[]);
    } else if let Some(spin) = widget.downcast_ref::<gtk::SpinButton>() {
        spin.set_value(spin.value() + 1.0);
    } else if let Some(switch) = widget.downcast_ref::<gtk::Switch>() {
        switch.set_active(!switch.is_active());
    } else if let Some(check) = widget.downcast_ref::<gtk::CheckButton>() {
        check.set_active(!check.is_active());
    } else if let Some(drop_down) = widget.downcast_ref::<gtk::DropDown>() {
        let count = drop_down.model().map_or(0, |model| model.n_items());
        for item in 0..count {
            drop_down.set_selected(item);
        }
    } else if let Some(button) = widget.downcast_ref::<gtk::Button>() {
        if !left_alone(widget) {
            button.emit_clicked();
        }
    } else if let Some(list) = widget.downcast_ref::<gtk::ListBox>() {
        // A row can add a row (a program chosen from the catalog goes on the list): only the first ones are tried.
        for index in 0..ROWS_TRIED {
            if let Some(row) = list.row_at_index(index) {
                list.select_row(Some(&row));
                row.emit_activate();
            }
        }
    }
}

/// A key pressed on whatever listens for keys on this widget.
fn press_key(widget: &gtk::Widget, key: gdk::Key) {
    let controllers = widget.observe_controllers();
    for index in 0..controllers.n_items() {
        if let Some(controller) = controllers.item(index).and_then(|item| item.downcast::<gtk::EventControllerKey>().ok()) {
            let _: bool = controller.emit_by_name("key-pressed", &[&key.into_glib(), &0_u32, &gdk::ModifierType::empty()]);
        }
    }
}

/// The keyboard comes to this widget and goes.
fn focus_in_and_out(widget: &gtk::Widget) {
    let controllers = widget.observe_controllers();
    for index in 0..controllers.n_items() {
        if let Some(controller) = controllers.item(index).and_then(|item| item.downcast::<gtk::EventControllerFocus>().ok()) {
            controller.emit_by_name::<()>("enter", &[]);
            controller.emit_by_name::<()>("leave", &[]);
        }
    }
}

/// What a button opened: every control of a dialog in the window, then a row of its list or its own close button.
fn walk_dialogs(window: &gtk::Widget, round: usize) {
    let mut found = Vec::new();
    every_widget(window, &mut found);
    for dialog in found.iter().filter(|widget| widget.is::<adw::Dialog>() && !widget.is::<adw::AlertDialog>()) {
        let mut inside = Vec::new();
        every_widget(dialog, &mut inside);
        for widget in inside.iter().filter(|widget| acts(widget) || widget.is::<gtk::SearchEntry>()) {
            if !widget.is::<gtk::Button>() && !widget.is::<gtk::ListBox>() {
                note(format!("in a dialog: {}", widget.type_()));
                change(widget);
                pump(1);
            }
        }
        if round == 0 {
            // A row chosen closes the dialog.
            if let Some(row) = inside.iter().find_map(|widget| widget.downcast_ref::<gtk::ListBox>()).and_then(|list| list.row_at_index(0)) {
                note("a row of a dialog");
                row.emit_activate();
                pump(5);
            }
        } else if let Some(close) = inside.iter().filter_map(|widget| widget.downcast_ref::<gtk::Button>()).find(|button| button.tooltip_text().is_some_and(|text| text.contains("Закрыть"))) {
            note("the close button of a dialog");
            close.emit_clicked();
            pump(5);
        }
    }
}

fn pump(milliseconds: u64) {
    let context = glib::MainContext::default();
    let until = Instant::now() + Duration::from_millis(milliseconds);
    while Instant::now() < until {
        // One at a time: an animation always has more to do.
        if !context.iteration(false) {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Closes what the buttons opened: alert dialogs answer with each of their responses in turn over the runs.
fn close_dialogs(window: &gtk::Widget, answer: &str) {
    let mut found = Vec::new();
    every_widget(window, &mut found);
    for widget in found {
        if widget.root().is_none() {
            continue;
        }
        if let Some(dialog) = widget.downcast_ref::<adw::AlertDialog>() {
            if dialog.has_response(answer) {
                dialog.emit_by_name::<()>("response", &[&answer]);
            }
            dialog.force_close();
        } else if let Some(dialog) = widget.downcast_ref::<adw::Dialog>() {
            dialog.force_close();
        }
    }
    for top in gtk::Window::list_toplevels() {
        if let Ok(other) = top.downcast::<gtk::Window>()
            && other.upcast_ref::<gtk::Widget>() != window
            && other.is_visible()
        {
            other.close();
        }
    }
}

fn walk_everything() -> Result<(), String> {
    note("the window");
    let root = gtk_app::with_app_window().ok_or("the window did not open")?;
    let mut pages = 0;
    for page in ["automation", "dashboard", "diagnostics", "exceptions", "history", "hotkeys", "languages", "system", "updates", "nonexistent"] {
        pages += i32::from(gtk_app::with_window_result(|window| window.show_page(page)).unwrap_or(false));
        gtk_app::refresh_current(page);
        pump(20);
    }
    let mut everything = Vec::new();
    every_widget(&root, &mut everything);
    for widget in &everything {
        focus_in_and_out(widget);
    }
    // The word lists save a moment after the last key: a change taken over by the next, one left to save, one the window's end cancels.
    for view in everything.iter().filter_map(|widget| widget.downcast_ref::<gtk::TextView>()) {
        view.buffer().set_text("ghbdtn\n\n  привет  ");
        view.buffer().set_text("ghbdtn");
    }
    gtk_app::with_window(MainWindow::flush_pending_saves);
    for view in everything.iter().filter_map(|widget| widget.downcast_ref::<gtk::TextView>()) {
        view.buffer().set_text("ghbdtn");
    }
    pump(600);
    for (round, answer) in ["reset", "clear", "confirm", "ok", "cancel"].into_iter().enumerate() {
        let mut widgets = Vec::new();
        every_widget(&root, &mut widgets);
        // Every control once; after that only what opens a dialog, whose answer is what changes between the rounds.
        for widget in widgets.iter().filter(|widget| acts(widget) && (round == 0 || widget.is::<gtk::Button>())) {
            if widget.root().is_some() {
                note(format!(
                    "{answer} {} label {:?} icon {:?} tooltip {:?} row {:?}",
                    widget.type_(),
                    widget.downcast_ref::<gtk::Button>().and_then(gtk::Button::label),
                    widget.downcast_ref::<gtk::Button>().and_then(gtk::Button::icon_name),
                    widget.tooltip_text(),
                    widget.ancestor(adw::PreferencesRow::static_type()).and_then(|row| row.downcast::<adw::PreferencesRow>().ok()).map(|row| row.title()),
                ));
                change(widget);
                pump(1);
                if widget.is::<gtk::Button>() {
                    walk_dialogs(&root, round);
                    close_dialogs(&root, answer);
                }
            }
        }
        pump(50);
    }
    println!("pages {pages}");
    Ok(())
}

/// News that reaches the window after a newer change: a setting changed twice before the main loop runs, as two quick
/// toggles or a toggle from the tray and one from the engine are, and an engine snapshot older than the setting. The window
/// shows them and stores nothing back; a stored echo was a change of its own, reported late again, for good.
fn late_news() -> Result<(), String> {
    let mut outcome = Err("no application".to_owned());
    gtk_app::with_app(|app| {
        let settings = app.services.settings.clone();
        let paths = ["enabled", "detection.correct_on_space"];
        let heard = Arc::new(Mutex::new(Vec::new()));
        let log = heard.clone();
        let unsubscribe = settings.subscribe(Arc::new(move |path: &str, _: &serde_json::Value| {
            if paths.contains(&path) {
                log.lock().unwrap().push(path.to_owned());
            }
        }));
        let flag = |path: &str| settings.get(path) == Some(json!(true));
        let before: Vec<bool> = paths.iter().map(|path| flag(path)).collect();
        for (path, now) in paths.iter().zip(&before) {
            settings.set(path, json!(!now), false).unwrap();
            settings.set(path, json!(now), false).unwrap();
        }
        pump(300);
        gtk_app::with_window(|window| window.apply_engine_snapshot(&EngineSnapshot { enabled: !before[0], ..EngineSnapshot::default() }));
        pump(300);
        unsubscribe();
        let heard = heard.lock().unwrap().clone();
        let after: Vec<bool> = paths.iter().map(|path| flag(path)).collect();
        outcome = if heard.len() == 2 * paths.len() && after == before {
            Ok(())
        } else {
            Err(format!("the window stored what it was told: {} changes, the first {:?}; {before:?} became {after:?}", heard.len(), &heard[..heard.len().min(6)]))
        };
        // The switch shows the setting again.
        gtk_app::with_window(|window| window.apply_setting_update("enabled", &json!(before[0])));
    });
    outcome
}

fn window_calls() {
    let _ = gtk_app::with_window_result(|window| {
        window.toast("проба");
        window.apply_engine_snapshot(&EngineSnapshot { enabled: false, ..EngineSnapshot::default() });
        window.apply_engine_snapshot(&EngineSnapshot { enabled: true, last_error: "ошибка".to_owned(), ..EngineSnapshot::default() });
        for (path, value) in [
            ("enabled", json!(false)),
            ("enabled", json!(true)),
            ("detection.confidence_threshold", json!(0.5)),
            ("detection.confidence_threshold", json!("x")),
            ("appearance.theme", json!("dark")),
            ("appearance.theme", json!(5)),
            ("hotkeys.convert_last", json!("Pause")),
            ("nothing.at.all", json!(1)),
        ] {
            window.apply_setting_update(path, &value);
        }
        for phase in [UpdatePhase::Idle, UpdatePhase::Checking, UpdatePhase::Current, UpdatePhase::Available, UpdatePhase::Downloading, UpdatePhase::Installing, UpdatePhase::Error] {
            for available in ["", "9.9.9"] {
                window.apply_update_snapshot(&UpdateSnapshot { phase, message: "проба".to_owned(), current_version: "1".to_owned(), available_version: available.to_owned(), release_url: "https://example.invalid/".to_owned(), progress: 5 });
            }
        }
        window.refresh_history();
        window.refresh_rules();
        window.refresh_models();
        window.flush_pending_saves();
        assert!(!window.show_page("no such page"));
        // Every control hears every kind of value, the same one twice.
        for path in window.control_paths() {
            for value in [json!("x"), json!("x"), json!(5), json!(5), json!(true), json!("flags"), json!("flags"), json!("assist"), json!("assist"), json!(null)] {
                window.apply_setting_update(&path, &value);
            }
        }
        // The models as they may stand.
        let info = window.services.models.clone();
        let saved = info.lock().unwrap().clone();
        for model in [
            ModelInfo { loaded: false, ..ModelInfo::default() },
            ModelInfo { loaded: true, languages: vec![(5, "a".to_owned()), (6, "b".to_owned())], intent_available: true, ..ModelInfo::default() },
            ModelInfo { loaded: true, intent_available: true, intent_version: "v".to_owned(), intent_checksum: "abcdef0123456789".to_owned(), ..ModelInfo::default() },
            ModelInfo { loaded: true, intent_available: false, intent_error: "проба".to_owned(), ..ModelInfo::default() },
        ] {
            *info.lock().unwrap() = model;
            window.refresh_models();
            window.copy_diagnostics();
        }
        *info.lock().unwrap() = saved;
        window.refresh_models();
        // On the list already, and what cannot go on it.
        window.add_application_exclusion("Проба", "");
        window.add_application_exclusion("проба", "Имя");
        window.capture_application("KeySwitch");
        window.capture_application("");
        window.capture_application("firefox");
        // Asked twice in a row, the second time while the first still runs.
        window.check_updates();
        window.check_updates();
        window.open_release("");
        window.open_release("ksnone://no-such-handler");
        window.open_release("ksprobe://release");
        window.open_folder("ksnone://no-such-handler");
        window.open_log_directory();
        window.finish_active_application_capture();
    });
    pump(100);
}

/// What the lists of the window show when there is something to show.
fn seed() {
    let history = HistoryStore::new(None, 200);
    history.append(&HistoryEntry::create("ghbdtn", "привет", "firefox", 0.9)).unwrap();
    history.append(&HistoryEntry::create("ghbdtn", "привет", "", 0.9)).unwrap();
    let rules = LearningStore::new(None, None).unwrap();
    let mut keep = LearnedRule::new("ntcn", 0, 1);
    keep.action = RuleAction::Keep;
    for rule in [LearnedRule::new("ghbdtn", 0, 1), keep] {
        rules.add_rule(rule).unwrap();
    }
    let _ = rules.save();
    // An update from a version before the hints: the settings window carries the news.
    std::fs::write(ks_config::history::data_dir().join("hints.json"), r#"{"last_run_version": "0.66.0"}"#).unwrap();
}

/// What the tray, the engine and the update check call on the application.
fn app_calls() {
    let phases = [UpdatePhase::Available, UpdatePhase::Error, UpdatePhase::Current, UpdatePhase::Idle];
    gtk_app::with_app(|app| {
        for action in [TrayAction::ShowSettings, TrayAction::ToggleEngine, TrayAction::ToggleEngine, TrayAction::SwitchLayout, TrayAction::ToggleSound, TrayAction::ToggleSound, TrayAction::ToggleNotifications, TrayAction::ToggleNotifications, TrayAction::ShowHistory, TrayAction::ShowExceptions, TrayAction::ShowAbout] {
            app.tray_action(action);
        }
        for (path, value) in [
            ("enabled", json!(false)),
            ("enabled", json!(true)),
            ("general.sound", json!(true)),
            ("general.notifications", json!(false)),
            ("general.notifications", json!(true)),
            ("general.autostart", json!(false)),
            ("general.autostart", json!(true)),
            ("appearance.show_indicator", json!(false)),
            ("appearance.show_indicator", json!(false)),
            ("appearance.show_indicator", json!(true)),
            ("appearance.show_indicator", json!(true)),
            ("appearance.indicator_style", json!("flag")),
            ("appearance.theme", json!("dark")),
            ("updates.check_automatically", json!(false)),
            ("updates.check_automatically", json!(true)),
            ("*", json!(null)),
            ("nothing", json!(1)),
        ] {
            app.apply_setting(path, &value);
        }
        app.engine_start_failed("проба");
        for phase in phases {
            app.apply_update_snapshot(&UpdateSnapshot { phase, message: "проба".to_owned(), current_version: "1".to_owned(), available_version: "9.9.9".to_owned(), release_url: String::new(), progress: 0 });
        }
        app.apply_engine_snapshot(&EngineSnapshot::default());
        app.announce_correction(false, "ghbdtn", "привет");
        app.announce_correction(true, "ПРивет", "Привет");
        app.announce_selection_notice("Нет выделенного текста");
        let prompt = LearningPrompt { source_group: 0, target_group: 1, original: "ghbdtn".to_owned(), replacement: "привет".to_owned(), application: "app".to_owned(), action: RuleAction::Convert };
        app.apply_learning_prompt(Some(prompt.clone()), Some(PromptAnchor { x: 10, y: 10, window: None }));
        app.apply_learning_prompt(None, None);
        app.apply_learning_prompt(Some(prompt), None);
        app.apply_learning_prompt(None, None);
        app.rule_editor_closed();
        app.cancel_update_checks();
        app.schedule_update_checks();
        // Scheduled already: no second timer. The checks themselves, with and without the next ones.
        app.schedule_update_checks();
        app.first_update_check();
        app.schedule_update_checks();
        let _ = gtk_app::periodic_update_check();
        app.cancel_update_checks();
        let settings = app.services.settings.clone();
        settings.set("updates.check_automatically", json!(false), false).unwrap();
        app.first_update_check();
        app.schedule_update_checks();
        settings.set("updates.check_automatically", json!(true), false).unwrap();
        app.cancel_update_checks();
        // An update announced once, and with notifications off; a correction with neither sound nor a window to show it.
        let mut available = UpdateSnapshot::new(UpdatePhase::Available, "проба", "1");
        "8.8.8".clone_into(&mut available.available_version);
        app.apply_update_snapshot(&available);
        app.apply_update_snapshot(&available);
        settings.set("general.notifications", json!(false), false).unwrap();
        settings.set("general.sound", json!(false), false).unwrap();
        "8.8.9".clone_into(&mut available.available_version);
        app.apply_update_snapshot(&available);
        app.announce_correction(false, "ghbdtn", "привет");
        settings.set("general.notifications", json!(true), false).unwrap();
        if let Some(window) = gtk_app::with_app_window() {
            window.set_visible(false);
            app.announce_correction(false, "ghbdtn", "привет");
            window.set_visible(true);
        }
        app.drop_window();
        app.announce_correction(false, "ghbdtn", "привет");
        // The window built again from settings edited by hand.
        settings.set("detection.field_catch_up", json!("nonsense"), false).unwrap();
        app.show_window();
        settings.set("detection.field_catch_up", json!("300"), false).unwrap();
        // Actions and a second start, as the desktop asks them.
        for action in ["show", "toggle", "toggle"] {
            app.application.activate_action(action, None);
        }
        app.application.activate();
        offers(app);
        hints(app);
        rule_window(app);
        autostart(app);
        app.drop_window();
        app.show_window();
        app.show_page("history");
        app.toggle("general.sound", false);
    });
    pump(200);
}

/// The offer of a rule, in every way it can go.
fn offers(app: &Rc<gtk_app::App>) {
    let prompt = || LearningPrompt { source_group: 0, target_group: 1, original: "ghbdtn".to_owned(), replacement: "привет".to_owned(), application: "firefox".to_owned(), action: RuleAction::Convert };
    let prompt_widget = || gtk::Window::list_toplevels().into_iter().filter_map(|top| top.downcast::<gtk::Window>().ok()).find(|top| top.is_visible() && top.title().is_some_and(|title| title.contains("Обучение")));
    // Which monitor a point is on, for points off each edge.
    if let Some(display) = gdk::Display::default() {
        for (x, y) in [(10, 10), (-5, 10), (99_999, 10), (10, -5), (10, 99_999)] {
            let _ = ks_linux::gtk_prompt::monitor_at(&display, x, y);
        }
    }
    gtk_app::beep_on(None);
    // Anchors on no monitor, off each edge.
    for (x, y) in [(10, 10), (-5, 10), (99_999, 10), (10, -5), (10, 99_999)] {
        app.apply_learning_prompt(Some(prompt()), Some(PromptAnchor { x, y, window: Some(5) }));
        pump(5);
    }
    for key in [gdk::Key::Return, gdk::Key::KP_Enter, gdk::Key::Escape, gdk::Key::a] {
        app.apply_learning_prompt(Some(prompt()), None);
        pump(1);
        if let Some(window) = prompt_widget() {
            press_key(window.upcast_ref(), key);
        }
        pump(1);
    }
    // Closed by the window manager, with an offer and with none; a key with nothing on offer.
    app.apply_learning_prompt(Some(prompt()), None);
    if let Some(window) = prompt_widget() {
        window.close();
    }
    app.apply_learning_prompt(None, None);
    if let Some(window) = gtk::Window::list_toplevels().into_iter().filter_map(|top| top.downcast::<gtk::Window>().ok()).find(|top| top.title().is_some_and(|title| title.contains("Обучение"))) {
        press_key(window.upcast_ref(), gdk::Key::Return);
        window.close();
    }
    // Gone before it could be placed.
    app.apply_learning_prompt(Some(prompt()), Some(PromptAnchor { x: 10, y: 10, window: None }));
    let gone = app.prompt.borrow_mut().take();
    if let Some(window) = &gone {
        window.destroy();
    }
    pump(5);
    drop(gone);
    // The engine's own calls.
    if let Ok(engine) = app.services.engine.engine() {
        // Stopped and started by hand: whether it has started by now depends on how fast the models load.
        engine.stop();
        gtk_app::record_models(&engine, &app.services.models);
        app.tray_action(TrayAction::SwitchLayout);
        pump(20);
        engine.start().expect("the engine starts on the private server");
        gtk_app::record_models(&engine, &app.services.models);
        assert!(engine.select_alternate_group(), "{}", engine.snapshot().last_error);
        pump(20);
        app.tray_action(TrayAction::SwitchLayout);
        pump(20);
        let shared = engine.shared.clone();
        shared.show_learning_prompt(prompt());
        pump(20);
        let _ = shared.dismiss_learning_prompt(None, "проба");
        pump(5);
        shared.show_learning_prompt(prompt());
        assert!(shared.confirm_learning_prompt(None));
        pump(20);
        rule_closed(app);
        let plan = ks_engine::engine::CorrectionPlan::new(Vec::new(), None, 0, 1, "ghbdtn".to_owned(), "привет".to_owned(), 0.9, "firefox");
        for callback in shared.correction_callbacks() {
            callback(&plan);
        }
        pump(5);
    }
    let _ = gtk_app::prompt_anchor(&app.desktop);
}

/// The notification daemon of a desktop on the private bus: what the program sends it, and a button pressed on it.
struct Notifications {
    sent: Arc<Mutex<Vec<(u32, String, String, Vec<String>)>>>,
    closed: Arc<Mutex<Vec<u32>>>,
    connection: zbus::blocking::Connection,
}

struct NotificationServer {
    sent: Arc<Mutex<Vec<(u32, String, String, Vec<String>)>>>,
    closed: Arc<Mutex<Vec<u32>>>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl NotificationServer {
    #[expect(clippy::too_many_arguments, reason = "what the notification interface passes")]
    #[zbus(name = "Notify")]
    fn notify(&self, application: String, replaces: u32, icon: String, summary: String, body: String, actions: Vec<String>, hints: std::collections::HashMap<String, zbus::zvariant::OwnedValue>, timeout: i32) -> u32 {
        let _ = (application, replaces, icon, hints, timeout);
        let mut sent = self.sent.lock().unwrap();
        let id = u32::try_from(sent.len()).unwrap() + 100;
        sent.push((id, summary, body, actions));
        id
    }

    #[zbus(name = "CloseNotification")]
    fn close_notification(&self, id: u32) {
        self.closed.lock().unwrap().push(id);
    }

    #[zbus(name = "GetCapabilities")]
    fn get_capabilities(&self) -> Vec<String> {
        vec!["actions".to_owned(), "body".to_owned()]
    }
}

impl Notifications {
    fn start() -> Notifications {
        let (sent, closed): (Arc<Mutex<Vec<(u32, String, String, Vec<String>)>>>, Arc<Mutex<Vec<u32>>>) = (Arc::default(), Arc::default());
        let server = NotificationServer { sent: sent.clone(), closed: closed.clone() };
        let connection = zbus::blocking::connection::Builder::session()
            .and_then(|builder| builder.name("org.freedesktop.Notifications"))
            .and_then(|builder| builder.serve_at("/org/freedesktop/Notifications", server))
            .and_then(zbus::blocking::connection::Builder::build)
            .expect("the private bus takes the name of the notifications");
        Notifications { sent, closed, connection }
    }

    /// The user pressed the button `key` on the notification `id`.
    fn press(&self, id: u32, key: &str) {
        self.connection.emit_signal(None::<&str>, "/org/freedesktop/Notifications", "org.freedesktop.Notifications", "ActionInvoked", &(id, key)).expect("the signal is sent");
    }

    fn sent_after(&self, count: usize) -> (u32, String, String, Vec<String>) {
        let arrived = (0..2000).any(|_| {
            pump(5);
            self.sent.lock().unwrap().len() > count
        });
        assert!(arrived, "no notification arrived");
        self.sent.lock().unwrap().last().unwrap().clone()
    }
}

/// A label's click gesture answers as a press and a release on it do.
fn click_text(label: &gtk::Widget) {
    let controllers = label.observe_controllers();
    for index in 0..controllers.n_items() {
        if let Some(gesture) = controllers.item(index).and_then(|item| item.downcast::<gtk::GestureClick>().ok()) {
            gesture.emit_by_name::<()>("released", &[&1_i32, &0.0_f64, &0.0_f64]);
        }
    }
}

/// The card on the screen with this text: the program's own hints may be up beside the test's.
fn hint_card_widget(text: &str) -> Option<gtk::Window> {
    gtk::Window::list_toplevels().into_iter().filter_map(|top| top.downcast::<gtk::Window>().ok()).find(|top| {
        let mut inside = Vec::new();
        every_widget(top.upcast_ref(), &mut inside);
        top.is_visible() && top.title().is_some_and(|title| title == HINT_WINDOW_TITLE) && inside.iter().any(|widget| widget.downcast_ref::<gtk::Label>().is_some_and(|label| label.text() == text))
    })
}

fn hint_button(widget: &gtk::Widget, wanted: &str) -> Option<gtk::Button> {
    let mut inside = Vec::new();
    every_widget(widget, &mut inside);
    inside.iter().find_map(|widget| widget.downcast_ref::<gtk::Button>().filter(|button| button.label().is_some_and(|label| label == wanted)).cloned())
}

/// The cards of the hints in the settings window: on the first page the hint about the hints, whose text opens what it is
/// about; on the page of the hotkeys the news of the update, its row marked until the card goes.
fn hint_cards() -> Result<(), String> {
    let root = gtk_app::with_app_window().ok_or("the window did not open")?;
    let with_class = |class: &str| {
        let mut found = Vec::new();
        every_widget(&root, &mut found);
        found.into_iter().filter(|widget| widget.has_css_class(class)).collect::<Vec<_>>()
    };
    pump(100);
    let first = with_class("hint-card");
    if first.len() != 1 {
        return Err(format!("the first page has {} cards of hints", first.len()));
    }
    let mut parts = Vec::new();
    every_widget(&first[0], &mut parts);
    let text = parts.iter().find(|part| part.is::<gtk::Label>()).ok_or("a card of a hint has no text")?;
    click_text(text);
    pump(100);
    let mut rows = Vec::new();
    every_widget(&root, &mut rows);
    let switch = rows.iter().filter_map(|widget| widget.downcast_ref::<adw::SwitchRow>()).find(|row| row.title() == HINTS_SETTING_TITLE).ok_or("no switch of the hints")?;
    if !with_class("hint-card").is_empty() || !switch.is_drawable() {
        return Err(format!("a click on the text of the card: {} cards left, its page shown: {}", with_class("hint-card").len(), switch.is_drawable()));
    }
    gtk_app::with_window_result(|window| window.show_page("hotkeys"));
    pump(100);
    let news = with_class("hint-card");
    let marked = with_class("hint-target");
    let title = marked.first().and_then(|row| row.downcast_ref::<adw::ComboRow>()).map(PreferencesRowExt::title);
    if news.len() != 1 || marked.len() != 1 || title.as_deref() != Some(LAYOUT_SWITCH_KEY_TITLE) {
        return Err(format!("the news of the update: {} cards, marked row {title:?}", news.len()));
    }
    hint_button(&news[0], HINT_OK_LABEL).ok_or("a card of a hint has no button")?.emit_clicked();
    pump(100);
    if !with_class("hint-card").is_empty() || !with_class("hint-target").is_empty() {
        return Err("the card of the news stayed after it was dismissed".to_owned());
    }
    Ok(())
}

/// A hint to draw, as the hints hand one out.
fn drawn_hint(settings: &Arc<SettingsStore>, folder: &std::path::Path, origin: Instant) -> HintView {
    let mut hints = Hints::<PromptAnchor>::start(settings.clone(), folder, RUNNING_PLATFORM, &Moment::now(origin, 0));
    hints.settings_opened();
    let mut view = hints.settings_card(SettingsPage::Overview, &Moment::now(origin, 0)).expect("a new folder has not shown the card");
    view.place = Place::Caret;
    view.text.push_str(" (проверка)");
    view
}

/// The hints on the screen of this desktop, with the system around them played by the test: the card over a field with
/// no keyboard to give back, the notification, and the timer that serves them.
fn hints(app: &Rc<gtk_app::App>) {
    let settings = app.services.settings.clone();
    settings.set(HINTS_SETTING, json!(true), true).unwrap();
    pump(50);
    let origin = Instant::now();
    let folder = ks_config::history::data_dir().join("hints-of-the-test");
    let view = drawn_hint(&settings, &folder.join("view"), origin);
    let (answers, placed): (Rc<Cell<bool>>, Rc<Cell<u32>>) = (Rc::new(Cell::new(true)), Rc::default());
    let (clicks, pages, cards): (Rc<RefCell<Vec<(String, Click)>>>, Rc<RefCell<Vec<&'static str>>>, Rc<RefCell<Vec<String>>>) = (Rc::default(), Rc::default(), Rc::default());
    let calls = {
        let (answers, placed, clicks, pages, cards) = (answers.clone(), placed.clone(), clicks.clone(), pages.clone(), cards.clone());
        let clicked: HintClicked = Rc::new(move |id: &str, click| clicks.borrow_mut().push((id.to_owned(), click)));
        HintCalls {
            position: Box::new(move |title, _, _, _| {
                assert_eq!(title, HINT_WINDOW_TITLE);
                placed.set(placed.get() + 1);
                answers.get()
            }),
            restore: Box::new(|_| false),
            clicked,
            open_page: Box::new(move |page| pages.borrow_mut().push(page)),
            hide_card: Box::new(move |id| cards.borrow_mut().push(id.to_owned())),
        }
    };
    let mut screen = HintScreen::new(app.application.upcast_ref(), calls);
    let at = |x, y| PromptAnchor { x, y, window: Some(5) };
    // Anchors on no monitor, off each edge; a new card replaces the one before; placed once it is known.
    for (x, y) in [(10, 10), (-5, 10), (99_999, 10), (10, -5), (10, 99_999)] {
        assert!(screen.show_caret(&view, &at(x, y)));
        assert_eq!(screen.card_id(), Some(view.id));
        pump(3 * u64::from(HINT_PLACE_RETRY_MS));
    }
    assert_eq!(placed.get(), 5, "each card is placed once it is known");
    // A desktop that does not know the card yet is asked again, and for so long and no longer.
    placed.set(0);
    answers.set(false);
    assert!(screen.show_caret(&view, &at(10, 10)));
    let tried = (0..2000).any(|_| {
        pump(5);
        placed.get() >= HINT_PLACE_ATTEMPTS
    });
    pump(4 * u64::from(HINT_PLACE_RETRY_MS));
    assert!(tried && placed.get() == HINT_PLACE_ATTEMPTS, "tried as many times as it may be: {}", placed.get());
    answers.set(true);
    // Its buttons, its text and the window manager's close.
    let card = hint_card_widget(&view.text).expect("the card is on the screen");
    for (label, click) in [(view.ok, Click::Ok), (view.never, Click::Never)] {
        clicks.borrow_mut().clear();
        hint_button(card.upcast_ref(), label).expect("the card's button").emit_clicked();
        assert_eq!(clicks.borrow().as_slice(), [(view.id.to_owned(), click)]);
    }
    clicks.borrow_mut().clear();
    let mut parts = Vec::new();
    every_widget(card.upcast_ref(), &mut parts);
    for part in parts.iter().filter(|part| part.is::<gtk::Label>()) {
        click_text(part);
    }
    assert_eq!(clicks.borrow().as_slice(), [(view.id.to_owned(), Click::Text)]);
    clicks.borrow_mut().clear();
    card.close();
    assert_eq!(clicks.borrow().as_slice(), [(view.id.to_owned(), Click::Closed)]);
    // The window goes when it is its turn, and only then.
    screen.hide("another");
    assert!(screen.card_id().is_some());
    screen.hide(view.id);
    pump(20);
    assert!(hint_card_widget(&view.text).is_none() && screen.card_id().is_none());
    assert_eq!(cards.borrow().as_slice(), ["another", view.id]);
    screen.close_card();
    // The place near the icon: a notification, withdrawn when its turn comes.
    assert!(screen.show_tray(&view));
    screen.hide("another");
    screen.hide(view.id);
    screen.hide(view.id);
    // What a click on a text opens, a page by way of the main loop.
    screen.open(&Opens::Settings { page: SettingsPage::Hotkeys, setting: Some("hotkeys.undo") });
    assert!(pages.borrow().is_empty(), "the window is not opened while the hints are in use");
    pump(20);
    assert_eq!(pages.borrow().as_slice(), ["hotkeys"]);
    screen.open(&Opens::Url("ksprobe://release".to_owned()));
    screen.open(&Opens::Url("ksnone://release".to_owned()));
    pump(20);
    hint_driver(app, &settings, &folder.join("driver"), &view);
    // The application's own screen under a window manager: placed through the desktop, and the keyboard the manager gives
    // the card goes back to the window it came from.
    let typer = support::Typer::new().unwrap();
    let mut manager = support::start_window_manager(&typer).unwrap();
    let window = gtk_app::with_app_window().and_then(|window| window.downcast::<gtk::Window>().ok()).expect("the settings window");
    let settings_window = window.surface().and_then(|surface| surface.downcast::<gdk_x11::X11Surface>().ok()).map(|surface| i64::try_from(surface.xid()).unwrap());
    window.present();
    let focused = (0..600).any(|_| {
        pump(5);
        window.is_active()
    });
    assert!(focused, "the window manager gave the settings window no keyboard");
    assert!(app.hints.screen().show_caret(&view, &PromptAnchor { x: 20, y: 20, window: settings_window }));
    pump(4 * u64::from(HINT_PLACE_RETRY_MS));
    assert_eq!(app.hints.card_id(), Some(view.id));
    let card = hint_card_widget(&view.text).expect("the application's card");
    assert!(!card.is_active(), "the card took the keyboard when it was shown");
    card.present();
    let taken_back = (0..600).any(|_| {
        pump(5);
        !card.is_active() && window.is_active()
    });
    assert!(taken_back, "the keyboard did not go back from the card: card active {}, window active {}", card.is_active(), window.is_active());
    gtk_app::hint_clicked(view.id, Click::Ok);
    pump(20);
    assert_eq!(app.hints.card_id(), None);
    let _ = manager.kill();
    let _ = manager.wait();
    pump(200);
    // The notification's actions, each with the hint it is about.
    let hint = view.id.to_variant();
    app.application.activate_action(HINT_ACTION_OPEN, Some(&hint));
    pump(50);
    app.application.activate_action(HINT_ACTION_NEVER, Some(&hint));
    pump(50);
    assert_eq!(settings.get(HINTS_SETTING), Some(json!(false)));
    settings.set(HINTS_SETTING, json!(true), true).unwrap();
    pump(50);
    // The place near the icon on the private bus: what a desktop's notification daemon is sent, withdrawn when its turn
    // comes, and a press on its button reaches the application.
    let notifications = NOTIFICATIONS.get().expect("the daemon of the private bus");
    let sent = notifications.sent.lock().unwrap().len();
    assert!(app.hints.screen().show_tray(&view));
    let (id, title, body, actions) = notifications.sent_after(sent);
    assert_eq!((title.as_str(), body.as_str()), (view.title, view.text.as_str()));
    let key = actions.iter().position(|action| action == view.never).map(|at| actions[at - 1].clone()).expect("the button of the hint");
    // The system's id of the notification reaches the program with the daemon's answer; before it, nothing can be withdrawn.
    pump(100);
    app.hints.screen().hide(view.id);
    assert!((0..400).any(|_| {
        pump(5);
        notifications.closed.lock().unwrap().contains(&id)
    }), "the notification was not withdrawn");
    let sent = notifications.sent.lock().unwrap().len();
    assert!(app.hints.screen().show_tray(&view));
    let (id, _, _, _) = notifications.sent_after(sent);
    notifications.press(id, &key);
    assert!((0..400).any(|_| {
        pump(5);
        settings.get(HINTS_SETTING) == Some(json!(false))
    }), "the button of the notification did not turn the hints off");
    settings.set(HINTS_SETTING, json!(true), true).unwrap();
    pump(50);
    // A correction with the caret to be found on the engine's thread, and every setting heard.
    if let Ok(engine) = app.services.engine.engine() {
        let plan = ks_engine::engine::CorrectionPlan::new(Vec::new(), None, 0, 1, "ghbdtn".to_owned(), "привет".to_owned(), 0.9, "firefox");
        for callback in engine.shared.correction_callbacks() {
            callback(&plan);
        }
    }
    pump(20);
    // A card with no setting to mark, one whose setting has no row, and each kind of row marked.
    let clicked: HintClicked = Rc::new(gtk_app::hint_clicked);
    let shown = |place: Place, id: &'static str| -> HintView {
        let mut card = view.clone();
        card.id = id;
        card.place = place;
        card
    };
    let card_with = |control| shown(Place::Settings { page: SettingsPage::System, control }, "hint-of-a-test");
    gtk_app::with_window(|window| {
        for card in [shown(Place::Caret, "hint-of-a-test"), card_with(None), card_with(Some("nothing.at.all")), card_with(Some("general.sound")), card_with(Some("detection.minimum_length")), card_with(Some("detection.layout_switch_key"))] {
            window.show_hint_card("system", &card, &clicked);
            window.remove_hint_card(card.id);
        }
        window.remove_hint_card("hint-of-a-test");
    });
    // Asked for a page with no hints of its own.
    assert!(app.hints.page_shown("languages").is_none());
}

/// The timer of the hints: running while a hint waits or shows, and only then, with what the engine reports in the real order.
fn hint_driver(app: &Rc<gtk_app::App>, settings: &Arc<SettingsStore>, folder: &std::path::Path, view: &HintView) {
    let origin = Instant::now();
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(folder.join("hints.json"), format!(r#"{{"last_run_version": "{}"}}"#, ks_config::logsetup::VERSION)).unwrap();
    let hints = Hints::start(settings.clone(), folder, RUNNING_PLATFORM, &Moment::now(origin, 0));
    let clicks: Rc<RefCell<Vec<(String, Click)>>> = Rc::default();
    let recorder = clicks.clone();
    let clicked: HintClicked = Rc::new(move |id: &str, click| recorder.borrow_mut().push((id.to_owned(), click)));
    let calls = HintCalls { position: Box::new(|_, _, _, _| true), restore: Box::new(|_| false), clicked, open_page: Box::new(|_| {}), hide_card: Box::new(|_| {}) };
    let driver: Rc<HintDriver> = HintDriver::new(hints, origin, HintScreen::new(app.application.upcast_ref(), calls));
    assert!(driver.caret_anchor_wanted().load(std::sync::atomic::Ordering::Relaxed));
    if let Ok(engine) = app.services.engine.engine() {
        driver.engine_built(engine.shared.clone());
    }
    // Nothing is due: no timer. A setting heard, a window opened, a page asked for: the same.
    driver.settings_changed();
    driver.settings_opened();
    assert!(driver.page_shown("hotkeys").is_none());
    driver.input(EngineInput::Snapshot { running: true });
    // A correction sets it off; a second event finds the timer running; the card comes when the typing has settled.
    let correction = CorrectionInfo { mode: "boundary", automatic: true, case_fix: false };
    driver.input(EngineInput::Correction { correction, anchor: Some(PromptAnchor { x: 30, y: 30, window: None }) });
    driver.input(EngineInput::Prompt { shown: false });
    let came = (0..600).any(|_| {
        pump(5);
        driver.card_id().is_some()
    });
    assert!(came, "the card did not come");
    // Its timer hides it: taken away here by the click and it stops.
    let id = driver.card_id().unwrap();
    driver.clicked(id, Click::Ok);
    pump(600);
    assert!(driver.card_id().is_none());
    // Another waits behind the quiet period, and the program ends with its timer running.
    assert_ne!(id, view.id);
    let early = CorrectionInfo { mode: "early", automatic: true, case_fix: false };
    driver.input(EngineInput::Correction { correction: early, anchor: Some(PromptAnchor { x: 30, y: 30, window: None }) });
    pump(50);
    driver.close();
    drop(driver);
    pump(300);
}

/// What a confirmed offer opened goes again.
fn rule_closed(app: &Rc<gtk_app::App>) {
    if let Some(editor) = app.rule_editor.borrow_mut().take() {
        editor.destroy();
    }
    app.rule_editor_closed();
    pump(5);
}

/// The rule window: asked twice, confirmed with a good and a bad draft, closed each way.
fn rule_window(app: &Rc<gtk_app::App>) {
    let draft = |pattern: &str| RuleDraft { pattern: pattern.to_owned(), r#match: RuleMatch::Exact, case_sensitive: false, action: RuleAction::Convert, editing: None, application: String::new() };
    let widget = || gtk::Window::list_toplevels().into_iter().filter_map(|top| top.downcast::<gtk::Window>().ok()).find(|top| top.is_visible() && top.title().is_some_and(|title| title.contains("Правило")));
    for pattern in ["", "ghbdtn проба", "ghbdtn"] {
        app.open_rule_editor(draft(pattern), Some(5));
        app.open_rule_editor(draft("другое"), None);
        pump(5);
        if let Some(editor) = app.rule_editor.borrow().as_ref() {
            editor.confirm();
        }
        pump(5);
        app.rule_editor_closed();
    }
    // A draft the rule cannot be built from, alone in the window, so that its refusal is what the confirmation meets.
    for pattern in ["", "ghbdtn проба"] {
        app.open_rule_editor(draft(pattern), None);
        pump(5);
        if let Some(editor) = app.rule_editor.borrow().as_ref() {
            editor.confirm();
        }
        pump(5);
        if let Some(editor) = app.rule_editor.borrow_mut().take() {
            editor.destroy();
        }
        pump(5);
        app.rule_editor_closed();
    }
    // A rule that cannot be saved: a folder where its file goes.
    let rules = ks_config::history::data_dir().join("learning.json");
    let saved = std::fs::read(&rules).ok();
    let _ = std::fs::remove_file(&rules);
    std::fs::create_dir_all(&rules).unwrap();
    app.open_rule_editor(draft("cannotsave"), None);
    pump(5);
    if let Some(editor) = app.rule_editor.borrow().as_ref() {
        editor.confirm();
    }
    if let Some(editor) = app.rule_editor.borrow_mut().take() {
        editor.destroy();
    }
    pump(5);
    std::fs::remove_dir_all(&rules).unwrap();
    if let Some(saved) = saved {
        std::fs::write(&rules, saved).unwrap();
    }
    // Every control of it, the last button closing it; then the keys; then the window's own close.
    app.open_rule_editor(draft("ntcn"), None);
    pump(5);
    if let Some(window) = widget() {
        let mut inside = Vec::new();
        every_widget(window.upcast_ref(), &mut inside);
        for control in inside.iter().filter(|control| acts(control) && !control.is::<gtk::Button>()) {
            change(control);
        }
        for button in inside.iter().filter(|control| control.is::<gtk::Button>()) {
            if button.root().is_some() {
                change(button);
                pump(2);
            }
        }
    }
    pump(5);
    app.rule_editor_closed();
    app.open_rule_editor(draft("ntcn"), None);
    pump(5);
    if let Some(window) = widget() {
        press_key(window.upcast_ref(), gdk::Key::a);
        press_key(window.upcast_ref(), gdk::Key::Escape);
    }
    pump(5);
    app.rule_editor_closed();
    app.open_rule_editor(draft("ntcn"), None);
    pump(5);
    if let Some(window) = widget() {
        window.close();
    }
    pump(5);
    app.rule_editor_closed();
}

/// The autostart row, its failure included: a file where the entry's folder goes.
fn autostart(app: &Rc<gtk_app::App>) {
    let row = adw::SwitchRow::new();
    gtk_app::autostart_toggled(true, &row);
    app.reverting_autostart.set(true);
    gtk_app::autostart_toggled(true, &row);
    app.reverting_autostart.set(false);
    let folder = std::path::PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").unwrap()).join("autostart");
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::write(&folder, "").unwrap();
    gtk_app::autostart_toggled(true, &row);
    app.sync_autostart();
    std::fs::remove_file(&folder).unwrap();
}

fn application_calls() {
    for theme in ["dark", "light", "system"] {
        gtk_app::set_theme(theme);
    }
    gtk_app::add_application_exclusion("a.desktop", "Программа");
    gtk_app::remove_application_exclusion("a.desktop");
    gtk_app::finish_application_capture();
    gtk_app::clear_history();
    gtk_app::clear_rules();
    gtk_app::open_rule_editor_for(RuleDraft { pattern: "ghbdtn".to_owned(), r#match: RuleMatch::Exact, case_sensitive: false, action: RuleAction::Convert, application: String::new(), editing: None });
    pump(100);
    gtk_app::reset_settings();
    let _ = gtk_app::window_close_requested();
    pump(100);
    // To the tray when it may, else the end of the program; each of the ways to end.
    gtk_app::with_app(|app| {
        app.services.settings.set("general.close_to_tray", json!(false), false).unwrap();
        let _ = gtk_app::window_close_requested();
        app.services.settings.set("general.close_to_tray", json!(true), false).unwrap();
        app.services.settings.set("appearance.show_indicator", json!(false), false).unwrap();
        app.apply_setting("appearance.show_indicator", &json!(false));
        let _ = gtk_app::window_close_requested();
        // The program ends with its tray up.
        app.services.settings.set("appearance.show_indicator", json!(true), false).unwrap();
        app.apply_setting("appearance.show_indicator", &json!(true));
        app.tray_action(TrayAction::Quit);
        app.application.activate_action("quit", None);
    });
    // SAFETY: a signal to this process, which the program handles on its main loop.
    unsafe {
        libc::raise(libc::SIGTERM);
    }
    pump(100);
}

/// The desktop entries of the programs a person can choose from, and what opens a page or a folder.
fn programs(folder: &std::path::Path) {
    let applications = folder.join("xdg/applications");
    std::fs::create_dir_all(&applications).unwrap();
    let entry = |name: &str, lines: &str| std::fs::write(applications.join(format!("{name}.desktop")), format!("[Desktop Entry]\nType=Application\nExec=/bin/true %u\n{lines}\n")).unwrap();
    entry("proba", "Name=Проба\nIcon=dialog-information\nStartupWMClass=Проба");
    entry("copy", "Name=Копия\nIcon=dialog-information\nStartupWMClass=проба");
    entry("hidden", "Name=Скрытая\nNoDisplay=true");
    entry("plain", "Name=Простая\nIcon=dialog-warning\nStartupWMClass=Plain");
    entry("bare", "Name=Без значка\nStartupWMClass=Bare");
    entry("handler", "Name=Обработчик\nMimeType=x-scheme-handler/ksprobe;inode/directory;");
    std::fs::write(applications.join("mimeinfo.cache"), "[MIME Cache]\nx-scheme-handler/ksprobe=handler.desktop;\ninode/directory=handler.desktop;\n").unwrap();
    std::fs::write(applications.join("mimeapps.list"), "[Default Applications]\nx-scheme-handler/ksprobe=handler.desktop\ninode/directory=handler.desktop\n").unwrap();
}

/// The program as other sessions start it: a child of this test, which ends itself.
fn child(kind: &str) -> i32 {
    match kind {
        "norules" => {
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_secs(60));
                println!("E2E_FAILED: rules that cannot be opened left the program running");
                std::process::exit(4);
            });
            gtk_app::run(false, true)
        }
        "hidden" | "wayland" => {
            // Long enough for the models to load and the engine to say whether it started.
            glib::timeout_add_local_once(Duration::from_secs(if kind == "wayland" { 15 } else { 1 }), move || {
                gtk_app::with_app(|app| {
                    app.tray_action(TrayAction::SwitchLayout);
                    app.schedule_update_checks();
                    app.show_window();
                    app.show_page("diagnostics");
                    app.tray_action(TrayAction::Quit);
                });
            });
            gtk_app::run(kind == "hidden", kind != "wayland")
        }
        other => {
            println!("E2E_FAILED: no such child: {other}");
            5
        }
    }
}

fn run_child(kind: &str, folder: &std::path::Path, change: impl FnOnce(&mut std::process::Command, &std::path::Path)) {
    let place = folder.join(kind);
    std::fs::create_dir_all(&place).unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    // No session bus: each child is an application of its own, not a second start of this one, and no tray comes up.
    command.env("KS_E2E_CHILD", kind).env("KEYSWITCH_CONFIG_DIR", place.join("config")).env("KEYSWITCH_DATA_DIR", place.join("data")).env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/nonexistent");
    change(&mut command, &place);
    note(format!("the child {kind}"));
    let status = command.status().unwrap();
    println!("child {kind}: {status}");
    assert!(status.success() || kind == "norules", "the child {kind} failed");
    assert_ne!(status.code(), Some(4), "rules that cannot be opened left the program running");
}

fn main() {
    if std::env::var_os("KS_E2E").is_none() {
        println!("KS_E2E is not set: no private X server to drive, skipped");
        return;
    }
    if let Ok(kind) = std::env::var("KS_E2E_CHILD") {
        std::process::exit(child(&kind));
    }
    // Asked with nothing there yet.
    let mut there = false;
    gtk_app::with_app(|_| there = true);
    gtk_app::with_window(|_| there = true);
    assert!(!there, "no application yet");
    let folder = std::env::temp_dir().join(format!("keyswitch-e2e-gtk-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    programs(&folder);
    // SAFETY: no other thread exists yet.
    unsafe {
        std::env::set_var("KEYSWITCH_CONFIG_DIR", folder.join("config"));
        std::env::set_var("KEYSWITCH_DATA_DIR", folder.join("data"));
        // No program to open a folder or a page with: a button that opens one reports that it could not.
        std::env::set_var("XDG_DATA_HOME", folder.join("xdg"));
        std::env::set_var("XDG_DATA_DIRS", folder.join("xdg"));
        std::env::set_var("XDG_CONFIG_HOME", folder.join("xdg"));
        std::env::set_var("XDG_CONFIG_DIRS", folder.join("xdg"));
    }
    seed();
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(if cfg!(coverage) { 900 } else { 300 }));
        println!("E2E_FAILED: the walk did not end, last: {}", LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        std::process::exit(3);
    });
    let failed = Rc::new(Cell::new(false));
    let failure = failed.clone();
    glib::timeout_add_local_once(Duration::from_millis(500), move || {
        pump(200);
        note("late_news();");
        if let Err(error) = late_news() {
            println!("E2E_FAILED: {error}");
            failure.set(true);
        }
        note("hint_cards();");
        if let Err(error) = hint_cards() {
            println!("E2E_FAILED: {error}");
            failure.set(true);
        }
        note("window_calls();");
        window_calls();
        if let Err(error) = walk_everything() {
            println!("E2E_FAILED: {error}");
            failure.set(true);
        }
        note("app_calls();");
        app_calls();
        note("application_calls();");
        application_calls();
        if let Some(application) = gtk::gio::Application::default() {
            application.quit();
        }
    });
    let watcher = support::Watcher::new().unwrap();
    let _ = NOTIFICATIONS.set(Notifications::start());
    let code = gtk_app::run(false, false);
    drop(watcher);
    // Asked once it has gone, and the window's style with no display to go on.
    let late = adw::SwitchRow::new();
    gtk_app::autostart_toggled(true, &late);
    let mut there = false;
    gtk_app::with_app(|_| there = true);
    assert!(!there, "no application any more");
    ks_linux::gtk_window::add_provider_to(None, &gtk::CssProvider::new());
    // The other ways the program starts.
    run_child("hidden", &folder, |_, _| {});
    run_child("wayland", &folder, |command, _| {
        // The windows on X11 all the same: the first of the two a toolkit can use is not there.
        command.env("XDG_SESSION_TYPE", "wayland").env("WAYLAND_DISPLAY", "no-such-display").env("GDK_BACKEND", "wayland,x11");
    });
    run_child("norules", &folder, |_, place| {
        // An old file of rules with a key the migration cannot read.
        std::fs::create_dir_all(place.join("data")).unwrap();
        std::fs::write(place.join("data/learning.json"), r#"{"schema_version": 2, "rules": {"²:слово": {"target_group": 1, "confirmations": 9}}}"#).unwrap();
    });
    let _ = std::fs::remove_dir_all(&folder);
    if failed.get() || code != 0 {
        println!("E2E_FAILED: exit {code}");
        std::process::exit(1);
    }
    println!("E2E_OK");
}

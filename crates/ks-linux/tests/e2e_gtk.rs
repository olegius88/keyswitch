//! The whole window under `packaging/e2e.sh`: the program started as a user starts it, on a private display, with every control
//! on every page changed and every button pressed, and the application's own calls made. With `KS_E2E` unset, it is skipped.
#![expect(unsafe_code, reason = "the test program names its folders before any thread starts")]
#![expect(clippy::unwrap_used, reason = "a test or a probe stops on what it did not expect")]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Mutex;
use std::time::Duration;

mod support;

use adw::prelude::*;
use gtk::glib;
use gtk::glib::translate::IntoGlib;
use gtk::gdk;
use ks_engine::rule_editor::RuleDraft;
use ks_config::history::{HistoryEntry, HistoryStore};
use ks_config::learning::{LearnedRule, LearningStore, RuleAction, RuleMatch};
use ks_engine::engine::{EngineSnapshot, LearningPrompt};
use ks_linux::gtk_prompt::PromptAnchor;
use ks_linux::gtk_window::{MainWindow, ModelInfo};
use ks_linux::tray::TrayAction;
use ks_updates::{UpdatePhase, UpdateSnapshot};
use serde_json::json;
use ks_linux::gtk_app;

const ROWS_TRIED: i32 = 30;

/// What the walk did last, for the watchdog to say.
static LAST: Mutex<String> = Mutex::new(String::new());

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
    let until = std::time::Instant::now() + Duration::from_millis(milliseconds);
    while std::time::Instant::now() < until {
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
        gtk_app::record_models(&engine, &app.services.models);
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

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
use ks_engine::rule_editor::RuleDraft;
use ks_config::history::{HistoryEntry, HistoryStore};
use ks_config::learning::{LearnedRule, LearningStore, RuleAction, RuleMatch};
use ks_engine::engine::{EngineSnapshot, LearningPrompt};
use ks_linux::gtk_prompt::PromptAnchor;
use ks_linux::tray::TrayAction;
use ks_updates::{UpdatePhase, UpdateSnapshot};
use serde_json::json;
use ks_linux::gtk_app;

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
        entry.set_text(&text);
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
        let mut index = 0;
        while let Some(row) = list.row_at_index(index) {
            list.select_row(Some(&row));
            row.emit_activate();
            index += 1;
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
    for answer in ["reset", "clear", "confirm", "ok", "cancel"] {
        let mut widgets = Vec::new();
        every_widget(&root, &mut widgets);
        for widget in widgets.iter().filter(|widget| acts(widget)) {
            if widget.root().is_some() {
                note(format!("{answer} {} {}", widget.type_(), widget.downcast_ref::<gtk::Button>().and_then(gtk::Button::label).unwrap_or_default()));
                change(widget);
                pump(1);
                if widget.is::<gtk::Button>() {
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
    });
    pump(50);
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
        app.cancel_update_checks();
        app.drop_window();
        app.show_window();
        app.show_page("history");
        app.toggle("general.sound", false);
    });
    pump(200);
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
}

fn main() {
    if std::env::var_os("KS_E2E").is_none() {
        println!("KS_E2E is not set: no private X server to drive, skipped");
        return;
    }
    let folder = std::env::temp_dir().join(format!("keyswitch-e2e-gtk-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
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
        std::thread::sleep(Duration::from_secs(300));
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
    let _ = std::fs::remove_dir_all(&folder);
    if failed.get() || code != 0 {
        println!("E2E_FAILED: exit {code}");
        std::process::exit(1);
    }
    println!("E2E_OK");
}

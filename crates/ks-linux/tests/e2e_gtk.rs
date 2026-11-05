//! The whole window under `packaging/e2e.sh`: the program started as a user starts it, on a private display, with every control
//! on every page changed and every button pressed, and the application's own calls made. With `KS_E2E` unset, it is skipped.
#![expect(unsafe_code, reason = "the test program names its folders before any thread starts")]
#![expect(clippy::unwrap_used, reason = "a test or a probe stops on what it did not expect")]

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gtk::glib;
use ks_engine::rule_editor::RuleDraft;
use ks_config::learning::{RuleAction, RuleMatch};
use ks_linux::gtk_app;

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
    }
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(150));
        println!("E2E_FAILED: the walk did not end");
        std::process::exit(3);
    });
    let failed = Rc::new(Cell::new(false));
    let failure = failed.clone();
    glib::timeout_add_local_once(Duration::from_millis(500), move || {
        pump(200);
        if let Err(error) = walk_everything() {
            println!("E2E_FAILED: {error}");
            failure.set(true);
        }
        application_calls();
        if let Some(application) = gtk::gio::Application::default() {
            application.quit();
        }
    });
    let code = gtk_app::run(false, true);
    let _ = std::fs::remove_dir_all(&folder);
    if failed.get() || code != 0 {
        println!("E2E_FAILED: exit {code}");
        std::process::exit(1);
    }
    println!("E2E_OK");
}

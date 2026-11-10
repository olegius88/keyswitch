//! The real thing on X11: a GTK entry, keys through XTEST, RECORD into the engine, the correction through XTEST back.
//! Runs under `packaging/e2e.sh`, with `KS_E2E` set in a private X server; otherwise it says so and passes.
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]

use ks_cast::To as _;
mod support;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use gtk::prelude::*;
use ks_backend::InputBackend;
use ks_config::config::SettingsStore;
use ks_config::constants::DETECTION_CONVERT_LAST_KEY;
use ks_config::history::HistoryStore;
use ks_config::learning::LearningStore;
use ks_engine::rule_editor::{RuleDraft, build_rule, draft_from_prompt};
use ks_engine::{KeySwitchEngine, LearningPrompt};
use ks_linux::desktop_backend::DesktopBackend;
use ks_linux::gtk_app::prompt_anchor;
use ks_linux::gtk_hint::{HintCalls, HintClicked, HintDriver, HintScreen};
use ks_linux::gtk_model::prompt_position;
use ks_linux::gtk_prompt::{LearningPromptWindow, PromptAnchor, PromptCallbacks, RuleCallbacks, RuleEditorWindow};
use ks_linux::linux_app::{engine_builder, new_backend};
use ks_ui::hints::{Click, EngineInput, Hints, Moment, RUNNING_PLATFORM, subscribe};
use serde_json::json;
use support::{Case, IDLE_PAUSE, SETTLE, CONTEXT_RESOLUTION, Typer, entry_window, pump, pump_until, start_window_manager, WINDOW_MANAGER, STARTUP, judge, GRACE, INTER_CASE, PAUSE_KEYSYM, RETURN_KEYSYM, ESCAPE_KEYSYM, LEARNING_CONFIRMATION, MENU_LAYOUT, EARLY_SETUP, SLOW_CHARACTER, SHIFT_L_KEYSYM, SHIFT_R_KEYSYM, CONTROL_R_KEYSYM, dbus_call};

const CASES: [Case; 15] = [
    Case("EN pause correction", 0, "ghbdtn", "привет", 1, IDLE_PAUSE),
    Case("RU keys to English", 1, "hello ", "hello ", 0, SETTLE),
    Case("punctuation key is a Russian letter", 0, ",fpf ", "база ", 1, SETTLE),
    Case("return to EN before punctuation test", 1, "hello ", "hello ", 0, SETTLE),
    Case("ambiguous punctuation keeps its glyph after idle", 0, "ghbdtn,", "привет,", 1, IDLE_PAUSE),
    Case("manual layout switch protects next word", 0, "ghbdtn ", "ghbdtn ", 0, SETTLE),
    Case("manual protection is consumed once", 0, "ghbdtn ", "привет ", 1, SETTLE),
    // A lone two-letter word has no observable intent: it waits for its neighbour, and the two are decided together.
    Case("short Russian word switches to English with its neighbour", 1, "if hello ", "if hello ", 0, CONTEXT_RESOLUTION),
    Case("manual Russian selection protects short word", 1, "if ", "ша ", 1, SETTLE),
    Case("short-word protection is consumed once", 1, "if hello ", "if hello ", 0, CONTEXT_RESOLUTION),
    Case("context resolves a short word with the next word", 0, "e 'njuj ", "у этого ", 1, CONTEXT_RESOLUTION),
    Case("return to EN before internal punctuation", 1, "hello ", "hello ", 0, SETTLE),
    Case("learned boundaries keep the internal comma key", 0, "ghj,ktvf ", "проблема ", 1, SETTLE),
    Case("return to EN before isolated punctuation", 1, "hello ", "hello ", 0, SETTLE),
    Case("isolated dot stays literal after idle", 0, ".", ".", 0, IDLE_PAUSE),
];
const EXPECTED_HISTORY: [(&str, &str); 13] = [
    ("ghbdtn", "привет"),
    ("руддщ", "hello"),
    (",fpf", "база"),
    ("руддщ", "hello"),
    ("ghbdtn", "привет"),
    ("ghbdtn", "привет"),
    ("ша руддщ", "if hello"),
    ("ша руддщ", "if hello"),
    // A lone letter typed in one burst with its neighbour waits for it, and the history holds the joint phrase.
    ("e 'njuj", "у этого"),
    ("руддщ", "hello"),
    ("ghj,ktvf", "проблема"),
    ("руддщ", "hello"),
    ("hello", "руддщ"),
];

const OFFER_POSITION: i32 = 100;
/// Between the two taps of a double tap, as a person makes one.
const DOUBLE_TAP_GAP: u64 = 150;
/// The longest a hint takes to appear after a correction: the typing settles for a second, then the next timer tick.
const HINT_APPEARS: u64 = 3000;
/// How long a window manager takes to focus a window.
const FOCUS_ARRIVES: u64 = 3000;
const HINT_WINDOW_TITLE: &str = ks_linux::gtk_model::HINT_WINDOW_TITLE;
/// The private screen, and how far from its place a window manager may leave a window it has moved.
const SCREEN_WIDTH: i32 = 1280;
const SCREEN_HEIGHT: i32 = 800;
const PLACE_TOLERANCE: i32 = 3;
/// The caret-hint's button that says it was understood.
const HINT_OK: &str = ks_ui::hints::HINT_OK_LABEL;

enum Posted {
    Prompt(Option<LearningPrompt>),
    Rule(LearningPrompt),
}

fn history_of(history: &HistoryStore) -> Result<Vec<(String, String)>, String> {
    Ok(history.read(None).map_err(|error| error.to_string())?.into_iter().map(|item| (item.original, item.replacement)).collect())
}

fn run() -> Result<(), String> {
    gtk::init().map_err(|error| error.to_string())?;
    let folder = std::env::temp_dir().join(format!("keyswitch-e2e-x11-{}", std::process::id()));
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let result = scenario(&folder);
    let _ = std::fs::remove_dir_all(&folder);
    result
}

fn scenario(folder: &std::path::Path) -> Result<(), String> {
    let settings = Arc::new(SettingsStore::new(Some(folder.join("config.json"))));
    let set = |name: &str, value: serde_json::Value| settings.set(name, value, false).map_err(|error| format!("{error:?}"));
    set("general.keep_history", json!(true))?;
    // The burst-typed cases have no gaps between keys; prefix switching has its own slowly typed case at the end.
    set("detection.early_switch", json!(false))?;
    set("diagnostics.technical_logging", json!(true))?;
    let history = Arc::new(HistoryStore::new(Some(folder.join("history.jsonl")), 200));
    let learning = Arc::new(LearningStore::new(Some(folder.join("learning.json")), None).map_err(|error| format!("{error:?}"))?);
    let backend = new_backend();
    let group_backend = backend.clone();
    let group = move || group_backend.current_group().unwrap_or(-1);
    let engine: Arc<KeySwitchEngine> = Arc::new(engine_builder(settings.clone(), history.clone(), backend.clone(), backend.process_for_window(), learning.clone())().map_err(|error| error.to_string())?);
    let modes: Arc<Mutex<Vec<String>>> = Arc::default();
    {
        let modes = modes.clone();
        engine.subscribe_corrections(Arc::new(move |plan: &ks_engine::CorrectionPlan| modes.lock().expect("the modes").push(plan.mode.to_string())));
    }
    let typer = Typer::new()?;
    let application = gtk::Application::builder().application_id("io.github.olegius88.KeySwitchSourceE2E").build();
    application.register(None::<&gtk::gio::Cancellable>).map_err(|error| error.to_string())?;
    let (window, entry) = entry_window("KeySwitch E2E");

    let posted: Arc<Mutex<VecDeque<Posted>>> = Arc::default();
    {
        let posted = posted.clone();
        engine.subscribe_learning_prompts(Arc::new(move |prompt: Option<&LearningPrompt>| posted.lock().expect("the queue").push_back(Posted::Prompt(prompt.cloned()))));
    }
    {
        let posted = posted.clone();
        engine.subscribe_rule_requests(Arc::new(move |prompt: &LearningPrompt| posted.lock().expect("the queue").push_back(Posted::Rule(prompt.clone()))));
    }
    let prompt_window = {
        let (confirm, dismiss) = (engine.clone(), engine.clone());
        let (position, restore) = (backend.clone(), backend.clone());
        LearningPromptWindow::new(
            &application,
            PromptCallbacks {
                confirm: Box::new(move |prompt| confirm.confirm_learning_prompt(Some(prompt))),
                dismiss: Box::new(move |prompt| dismiss.dismiss_learning_prompt(Some(prompt))),
                position: Box::new(move |window, x, y| position.position_window(window, x, y)),
                restore: Box::new(move |window| restore.restore_window(window)),
            },
        )
    };
    let rule_windows: Rc<RefCell<Vec<RuleEditorWindow>>> = Rc::default();
    let offer_window: Rc<Cell<Option<i64>>> = Rc::default();
    let drain = {
        let (posted, engine, backend, application, rule_windows, offer_window) = (posted.clone(), engine.clone(), backend.clone(), application.clone(), rule_windows.clone(), offer_window.clone());
        let prompt_window = &prompt_window;
        move || {
            loop {
                let Some(item) = posted.lock().expect("the queue").pop_front() else { break };
                match item {
                    Posted::Prompt(None) => prompt_window.hide_prompt(),
                    Posted::Prompt(Some(prompt)) => {
                        // As the application does: the offer remembers the window the user typed in.
                        let window = backend.focused_window().map(|focus| focus.window);
                        prompt_window.show_prompt(prompt, Some(PromptAnchor { x: OFFER_POSITION, y: OFFER_POSITION, window }));
                    }
                    Posted::Rule(prompt) => {
                        let (accepting, focusing, restoring) = (engine.clone(), engine.clone(), backend.clone());
                        let window = offer_window.get();
                        let editor = RuleEditorWindow::new(
                            &application,
                            draft_from_prompt(&prompt),
                            RuleCallbacks {
                                accept: Box::new(move |draft: &RuleDraft| {
                                    let built = build_rule(draft).map_err(|error| error.to_string());
                                    match built.and_then(|rule| accepting.add_learning_rule(rule, None, &draft.application).map_err(|error| error.to_string())) {
                                        Ok(_) => String::new(),
                                        Err(error) => error,
                                    }
                                }),
                                closed: Box::new(move || {
                                    restoring.restore_window(window);
                                }),
                                focused: Box::new(move |opened| focusing.set_rule_editor_open(opened)),
                            },
                        );
                        editor.present();
                        rule_windows.borrow_mut().push(editor);
                    }
                }
            }
        }
    };
    // What the engine posts for the hints, handed to the driver that is on trial.
    let hint_inputs: Arc<Mutex<VecDeque<EngineInput<PromptAnchor>>>> = Arc::default();
    let driver: Rc<RefCell<Option<Rc<HintDriver>>>> = Rc::default();
    let settle = |milliseconds: u64| {
        let deadline = Instant::now() + std::time::Duration::from_millis(milliseconds);
        while Instant::now() < deadline {
            pump(10);
            drain();
            drain_hints(&hint_inputs, &driver);
        }
    };

    engine.start().map_err(|error| format!("{error:?}"))?;
    let original_group = group();
    settle(STARTUP);
    let prepare = |selected: i32, keys: &str| -> Result<(), String> {
        backend.switch_group(selected).map_err(|error| format!("{error:?}"))?;
        entry.grab_focus();
        typer.clear_field()?;
        typer.type_keys(keys)
    };
    let outcome = (|| -> Result<(), String> {
        for case in &CASES {
            prepare(case.1, case.2)?;
            let judged = judge(&entry, &group, case, GRACE);
            println!("  engine={:?}", engine.snapshot().last_action);
            judged?;
            settle(INTER_CASE);
        }
        // A double press: the first converts the word, the second offers a rule.
        prepare(0, "hello")?;
        typer.tap_keysym(PAUSE_KEYSYM)?;
        typer.tap_keysym(PAUSE_KEYSYM)?;
        settle(SETTLE);
        if entry.text() != "руддщ" || group() != 1 || engine.learning_prompt().is_none() || !prompt_window.is_visible() {
            return Err(format!("learning prompt: text={:?} group={} visible={}", entry.text().as_str(), group(), prompt_window.is_visible()));
        }
        offer_window.set(prompt_window.anchor_window());
        typer.tap_keysym(RETURN_KEYSYM)?;
        settle(LEARNING_CONFIRMATION);
        // Enter on the offer opened the rule window, filled in; nothing is stored before OK.
        {
            let windows = rule_windows.borrow();
            if windows.len() != 1 || windows[0].current_draft().pattern != "hello" || engine.learning().count() != 0 || engine.learning_prompt().is_some() {
                return Err(format!("rule window: windows={} rules={:?}", windows.len(), engine.learning().rules()));
            }
        }
        typer.tap_keysym(RETURN_KEYSYM)?;
        settle(LEARNING_CONFIRMATION);
        if engine.learning_prompt().is_some() || engine.learning().forced_target(0, "hello") != Some(1) || prompt_window.is_visible() {
            return Err("the rule was not stored by OK".to_owned());
        }
        prepare(0, "hello ")?;
        settle(SETTLE);
        if entry.text() != "hello " || group() != 0 {
            return Err(format!("manual override of a learned rule: text={:?} group={}", entry.text().as_str(), group()));
        }
        entry.grab_focus();
        typer.clear_field()?;
        typer.type_keys("hello ")?;
        settle(SETTLE);
        if entry.text() != "руддщ " || group() != 1 {
            return Err(format!("learned rule: text={:?} group={}", entry.text().as_str(), group()));
        }
        let actual = history_of(&history)?;
        let wanted: Vec<(String, String)> = EXPECTED_HISTORY.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect();
        if actual != wanted {
            return Err(format!("history: {actual:?}"));
        }
        if !engine.select_alternate_group() {
            return Err(format!("menu layout: {:?}", engine.snapshot().last_error));
        }
        settle(MENU_LAYOUT);
        if group() != 0 || engine.snapshot().current_group != 0 {
            return Err(format!("menu layout: backend={} engine={}", group(), engine.snapshot().current_group));
        }
        println!("MENU_LAYOUT_SELECTION_E2E_OK");
        settle(INTER_CASE);
        set("detection.context_policy", json!("assist"))?;
        set("detection.early_switch", json!(true))?;
        // The menu selection protects exactly one word; spend it on a word that stays English anyway, then type slowly.
        prepare(0, "hello ")?;
        settle(EARLY_SETUP);
        modes.lock().expect("the modes").clear();
        for character in "ghbd".chars() {
            typer.type_keys(&character.to_string())?;
            settle(SLOW_CHARACTER);
        }
        settle(SETTLE);
        if entry.text() != "hello прив" || group() != 1 {
            return Err(format!("prefix assist: text={:?} modes={:?}", entry.text().as_str(), modes.lock().expect("the modes")));
        }
        println!("PREFIX_ASSIST_MID_WORD_E2E_OK");
        for character in "tn ".chars() {
            typer.type_keys(&character.to_string())?;
            settle(SLOW_CHARACTER);
        }
        settle(SETTLE);
        let entries = history_of(&history)?;
        let last = entries.last().cloned();
        println!("early_switch text={:?} group={} modes={:?} last={last:?}", entry.text().as_str(), group(), modes.lock().expect("the modes"));
        if entry.text() != "hello привет " || group() != 1 || !modes.lock().expect("the modes").iter().any(|mode| mode == "early") || last != Some(("ghbdtn".to_owned(), "привет".to_owned())) || entries.len() != EXPECTED_HISTORY.len() + 1 {
            return Err("early switch".to_owned());
        }
        println!("EARLY_SWITCH_E2E_OK");
        // The layout switch key: a lone Left Shift switches the layout and keeps the next word as typed; Right Shift,
        // Shift held over a letter and the setting turned off switch nothing.
        set("detection.early_switch", json!(false))?;
        set("detection.layout_switch_key", json!("Shift_L"))?;
        prepare(1, "")?;
        settle(INTER_CASE);
        typer.tap_keysym(SHIFT_L_KEYSYM)?;
        settle(MENU_LAYOUT);
        if group() != 0 || engine.snapshot().current_group != 0 {
            return Err(format!("layout key: backend={} engine={}", group(), engine.snapshot().current_group));
        }
        typer.type_keys("ghbdtn ")?;
        settle(SETTLE);
        typer.tap_keysym(SHIFT_R_KEYSYM)?;
        typer.hold_keysym(SHIFT_L_KEYSYM, true)?;
        typer.type_keys("a")?;
        typer.hold_keysym(SHIFT_L_KEYSYM, false)?;
        set("detection.layout_switch_key", json!(""))?;
        typer.tap_keysym(SHIFT_L_KEYSYM)?;
        settle(SETTLE);
        if entry.text() != "ghbdtn A" || group() != 0 {
            return Err(format!("layout key: text={:?} group={}", entry.text().as_str(), group()));
        }
        println!("LAYOUT_KEY_TAP_E2E_OK");
        // The conversion key: a lone Right Ctrl converts the word just typed, as Pause does, and a second tap right after
        // offers a rule for it; Right Ctrl held over another key converts nothing. The word is not `hello`, which a rule of
        // the phase above converts by itself.
        let convert_key = |key: &str| settings.write(DETECTION_CONVERT_LAST_KEY, key.to_owned(), false).map_err(|error| format!("{error:?}"));
        convert_key("Control_R")?;
        prepare(0, "world")?;
        typer.tap_keysym(CONTROL_R_KEYSYM)?;
        settle(SETTLE);
        if (entry.text().as_str(), group()) != ("цщкдв", 1) {
            return Err(format!("conversion key: text={:?} group={}", entry.text().as_str(), group()));
        }
        prepare(0, "world")?;
        typer.tap_keysym(CONTROL_R_KEYSYM)?;
        settle(DOUBLE_TAP_GAP);
        typer.tap_keysym(CONTROL_R_KEYSYM)?;
        settle(SETTLE);
        if (entry.text().as_str(), group()) != ("цщкдв", 1) || engine.learning_prompt().is_none() {
            return Err(format!("conversion key twice: text={:?} group={} offered={}", entry.text().as_str(), group(), engine.learning_prompt().is_some()));
        }
        typer.tap_keysym(ESCAPE_KEYSYM)?;
        settle(LEARNING_CONFIRMATION);
        typer.hold_keysym(CONTROL_R_KEYSYM, true)?;
        typer.type_keys("c")?;
        typer.hold_keysym(CONTROL_R_KEYSYM, false)?;
        settle(SETTLE);
        if (entry.text().as_str(), group()) != ("цщкдв", 1) || engine.learning_prompt().is_some() {
            return Err(format!("conversion key with a letter: text={:?} group={} offered={}", entry.text().as_str(), group(), engine.learning_prompt().is_some()));
        }
        convert_key("")?;
        println!("CONVERT_KEY_TAP_E2E_OK");
        let names = dbus_call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus.ListNames", &[])?;
        if names.contains("org.freedesktop.portal.Desktop") || names.contains("org.freedesktop.portal.Documents") {
            return Err("GUI tests unexpectedly activated a desktop or document portal".to_owned());
        }
        println!("NO_DOCUMENT_PORTAL_E2E_OK");
        // A hint over the field the user types in, with no window manager and then with one: a real correction brings it,
        // and the keys, the Enter and the layout go on as if it were not there.
        set("detection.layout_switch_key", json!(""))?;
        hint_inputs.lock().expect("the queue").clear();
        let desktop = DesktopBackend {
            input: backend.clone(),
            process_for_window: backend.process_for_window(),
            position_window: {
                let placer = backend.clone();
                Arc::new(move |_, window, x, y| placer.position_window(window, x, y))
            },
            wayland: false,
        };
        let wants_anchor = Arc::new(AtomicBool::new(true));
        let inputs = hint_inputs.clone();
        subscribe(&engine.shared, wants_anchor, Arc::new(move || prompt_anchor(&desktop)), Arc::new(move |input| inputs.lock().expect("the queue").push_back(input)));
        let activated: Rc<Cell<u32>> = Rc::default();
        entry.connect_activate({
            let activated = activated.clone();
            move |_| activated.set(activated.get() + 1)
        });
        let stand = HintStand { folder, settings: &settings, application: &application, engine: &engine, backend: &backend, typer: &typer, entry: &entry, group: &group, activated: &activated };
        hint_over_the_field(&stand, "alone", &driver, &settle)?;
        println!("HINT_KEEPS_THE_KEYBOARD_E2E_OK");
        let mut manager = start_window_manager(&typer)?;
        let under_manager = window_manager_cases(&stand, &window, &driver, &settle);
        let _ = manager.kill();
        let _ = manager.wait();
        under_manager?;
        Ok(())
    })();
    if original_group >= 0 {
        let _ = backend.switch_group(original_group);
    }
    engine.stop();
    prompt_window.destroy();
    window.destroy();
    outcome
}

fn drain_hints(inputs: &Mutex<VecDeque<EngineInput<PromptAnchor>>>, driver: &RefCell<Option<Rc<HintDriver>>>) {
    loop {
        let Some(input) = inputs.lock().expect("the queue").pop_front() else { break };
        if let Some(driver) = driver.borrow().as_ref() {
            driver.input(input);
        }
    }
}

/// What the hints are tried on.
struct HintStand<'a> {
    folder: &'a std::path::Path,
    settings: &'a Arc<SettingsStore>,
    application: &'a gtk::Application,
    engine: &'a Arc<KeySwitchEngine>,
    backend: &'a Arc<ks_linux::x11_backend::X11Backend>,
    typer: &'a Typer,
    entry: &'a gtk::Entry,
    group: &'a dyn Fn() -> i32,
    activated: &'a Rc<Cell<u32>>,
}

/// The card of the hint on the screen, as a toplevel window of this process.
fn hint_card() -> Option<gtk::Window> {
    gtk::Window::list_toplevels().into_iter().filter_map(|top| top.downcast::<gtk::Window>().ok()).find(|top| top.is_visible() && top.title().is_some_and(|title| title == HINT_WINDOW_TITLE))
}

/// The X11 id of a window of this process.
fn x11_id(window: &gtk::Window) -> Option<u32> {
    window.surface().and_then(|surface| surface.downcast::<gdk_x11::X11Surface>().ok()).and_then(|surface| u32::try_from(surface.xid()).ok())
}

/// A button of a window, by its label.
fn button_with_label(widget: &gtk::Widget, wanted: &str) -> Option<gtk::Button> {
    let mut child = widget.first_child();
    while let Some(next) = child {
        if let Some(button) = next.downcast_ref::<gtk::Button>().filter(|button| button.label().is_some_and(|label| label == wanted)) {
            return Some(button.clone());
        }
        if let Some(found) = button_with_label(&next, wanted) {
            return Some(found);
        }
        child = next.next_sibling();
    }
    None
}

/// The middle of a window's button on the screen.
fn button_centre(window: &gtk::Window, button: &gtk::Button, typer: &Typer) -> Result<(i32, i32), String> {
    let origin = typer.window_origin(x11_id(window).ok_or("the card has no X11 window")?)?;
    let inside = button.compute_point(window, &gtk::graphene::Point::new(button.width().to::<f32>() / 2.0, button.height().to::<f32>() / 2.0)).ok_or("the button is not in the card")?;
    Ok((origin.0 + inside.x().to::<i32>(), origin.1 + inside.y().to::<i32>()))
}

/// A correction brings a hint over the field. The hint keeps nothing of the keyboard: the keys go into the field and switch
/// the layout, Enter is the field's and is not taken for an answer to it, Esc leaves it on the screen, and a click on it gives
/// the keyboard back at once.
fn hint_over_the_field(stand: &HintStand<'_>, name: &str, driver_slot: &RefCell<Option<Rc<HintDriver>>>, settle: &dyn Fn(u64)) -> Result<(), String> {
    let origin = Instant::now();
    // The hints of a version that has run before: no news and no first start to wait behind.
    let data = stand.folder.join(format!("hints-{name}"));
    std::fs::create_dir_all(&data).map_err(|error| error.to_string())?;
    std::fs::write(data.join("hints.json"), format!(r#"{{"last_run_version": "{}"}}"#, ks_config::logsetup::VERSION)).map_err(|error| error.to_string())?;
    let hints = Hints::start(stand.settings.clone(), &data, RUNNING_PLATFORM, &Moment::now(origin, 0));
    let clicks: Rc<RefCell<Vec<(String, Click)>>> = Rc::default();
    let recorder = clicks.clone();
    let clicked: HintClicked = Rc::new(move |id: &str, click| recorder.borrow_mut().push((id.to_owned(), click)));
    let (placer, restorer) = (stand.backend.clone(), stand.backend.clone());
    let restored: Rc<Cell<u32>> = Rc::default();
    let counted = restored.clone();
    let calls = HintCalls {
        position: Box::new(move |_, window, x, y| placer.position_window(window, x, y)),
        restore: Box::new(move |window| {
            counted.set(counted.get() + 1);
            restorer.restore_window(window)
        }),
        clicked,
        open_page: Box::new(|_| {}),
        hide_card: Box::new(|_| {}),
    };
    let driver = HintDriver::new(hints, origin, HintScreen::new(stand.application, calls));
    driver.engine_built(stand.engine.shared.clone());
    *driver_slot.borrow_mut() = Some(driver.clone());
    let outcome = (|| -> Result<(), String> {
        let focused = stand.typer.input_focus()?;
        // A layout chosen by hand protects the next word: that word stays as typed, and only the one after it is corrected.
        stand.backend.switch_group(0).map_err(|error| format!("{error:?}"))?;
        stand.typer.type_keys("hello ")?;
        settle(SETTLE);
        stand.typer.clear_field()?;
        stand.typer.type_keys("ghbdtn ")?;
        settle(SETTLE);
        if stand.entry.text() != "привет " || (stand.group)() != 1 {
            return Err(format!("{name}: the correction that brings the hint: text={:?} group={}", stand.entry.text().as_str(), (stand.group)()));
        }
        let deadline = Instant::now() + std::time::Duration::from_millis(HINT_APPEARS);
        while driver.card_id().is_none() && Instant::now() < deadline {
            settle(20);
        }
        if driver.card_id() != Some("first_correction_undo") {
            return Err(format!("{name}: no hint appeared: {:?}", driver.card_id()));
        }
        settle(INTER_CASE);
        let card = hint_card().ok_or_else(|| format!("{name}: no card on the screen"))?;
        // Above the place the correction was made at, inside the screen.
        let pointer = stand.backend.input_anchor().ok_or("no pointer")?;
        let placed = stand.typer.window_origin(x11_id(&card).ok_or("the card has no X11 window")?)?;
        let expected = prompt_position((pointer.x, pointer.y), (card.width(), card.height()), Some((0, 0, SCREEN_WIDTH, SCREEN_HEIGHT)));
        if (placed.0 - expected.0).abs() > PLACE_TOLERANCE || (placed.1 - expected.1).abs() > PLACE_TOLERANCE {
            return Err(format!("{name}: the card is at {placed:?}, expected {expected:?}, pointer {:?}", (pointer.x, pointer.y)));
        }
        // Not taken and handed back: the card never had the keyboard at all.
        if stand.typer.input_focus()? != focused || card.is_active() || restored.get() != 0 {
            return Err(format!("{name}: the card took the keyboard: {:#x}, card active {}, given back {} times", stand.typer.input_focus()?, card.is_active(), restored.get()));
        }
        // Keys typed while it is up: the text, the correction and the layout are those of a field with no hint over it.
        stand.typer.type_keys("hello ")?;
        settle(SETTLE);
        if stand.entry.text() != "привет hello " || (stand.group)() != 0 {
            return Err(format!("{name}: typing under the hint: text={:?} group={}", stand.entry.text().as_str(), (stand.group)()));
        }
        let before = stand.activated.get();
        stand.typer.tap_keysym(RETURN_KEYSYM)?;
        settle(INTER_CASE);
        if stand.activated.get() != before + 1 || stand.engine.shared.prompt_key_deadline() != 0.0 {
            return Err(format!("{name}: Enter did not reach the field: {} activations, deadline {}", stand.activated.get() - before, stand.engine.shared.prompt_key_deadline()));
        }
        stand.typer.tap_keysym(ESCAPE_KEYSYM)?;
        settle(INTER_CASE);
        if hint_card().is_none() || driver.card_id().is_none() || stand.typer.input_focus()? != focused {
            return Err(format!("{name}: Esc took the hint down or moved the keyboard"));
        }
        // A real click on «Понятно»: reported, and the keyboard is where it was for the next word.
        let button = button_with_label(card.upcast_ref(), HINT_OK).ok_or("the card has no button to dismiss it")?;
        let (x, y) = button_centre(&card, &button, stand.typer)?;
        stand.typer.click_at(x, y)?;
        if !pump_until(FOCUS_ARRIVES, || !clicks.borrow().is_empty()) {
            return Err(format!("{name}: the click on the button was not heard"));
        }
        if clicks.borrow()[0] != ("first_correction_undo".to_owned(), Click::Ok) {
            return Err(format!("{name}: the click reported {:?}", clicks.borrow()));
        }
        // A window manager gives the card the keyboard on a click; it is asked back at once.
        let given_back = restored.get();
        driver.clicked("first_correction_undo", Click::Ok);
        settle(INTER_CASE);
        if hint_card().is_some() || driver.card_id().is_some() {
            return Err(format!("{name}: the hint stayed after it was dismissed"));
        }
        if !pump_until(FOCUS_ARRIVES, || stand.typer.input_focus().is_ok_and(|window| window == focused)) {
            return Err(format!("{name}: the keyboard did not come back after the click: {:#x}", stand.typer.input_focus()?));
        }
        if name != "alone" && given_back == 0 {
            return Err(format!("{name}: the window manager gave the clicked card the keyboard and it was not asked back"));
        }
        stand.typer.type_keys("ghbdtn ")?;
        settle(SETTLE);
        if stand.entry.text() != "привет hello привет " || (stand.group)() != 1 {
            return Err(format!("{name}: typing after the hint: text={:?} group={}", stand.entry.text().as_str(), (stand.group)()));
        }
        Ok(())
    })();
    driver.close();
    *driver_slot.borrow_mut() = None;
    outcome
}

/// What the method rests on, under a real window manager. A plain window shown the same way takes the keyboard, so that
/// the card's not taking it says something; then the whole of it again, with the field's window focused by the manager.
fn window_manager_cases(stand: &HintStand<'_>, field: &gtk::Window, driver: &RefCell<Option<Rc<HintDriver>>>, settle: &dyn Fn(u64)) -> Result<(), String> {
    field.present();
    stand.entry.grab_focus();
    let field_id = x11_id(field).ok_or("the field's window has no X11 id")?;
    if !pump_until(FOCUS_ARRIVES, || field.is_active()) {
        return Err(format!("the window manager did not focus the field's window: {:#x}", stand.typer.input_focus()?));
    }
    let plain = gtk::Window::builder().title("A plain window").default_width(100).default_height(100).build();
    plain.set_visible(true);
    let taken = pump_until(FOCUS_ARRIVES, || plain.is_active());
    plain.destroy();
    if !taken {
        return Err(format!("{WINDOW_MANAGER} did not focus a new plain window, so it proves nothing about the card"));
    }
    field.present();
    stand.entry.grab_focus();
    if !pump_until(FOCUS_ARRIVES, || field.is_active()) {
        return Err(format!("the field's window did not get the keyboard back: {field_id:#x}"));
    }
    println!("WINDOW_MANAGER_FOCUSES_A_PLAIN_WINDOW_E2E_OK");
    pump(INTER_CASE);
    hint_over_the_field(stand, "under-window-manager", driver, settle)?;
    println!("HINT_KEEPS_THE_KEYBOARD_UNDER_A_WINDOW_MANAGER_E2E_OK");
    Ok(())
}

fn main() {
    if std::env::var_os("KS_E2E").is_none() {
        println!("KS_E2E is not set: no private X server to drive, skipped");
        return;
    }
    match run() {
        Ok(()) => println!("E2E_OK"),
        Err(error) => {
            println!("E2E_FAILED: {error}");
            std::process::exit(1);
        }
    }
}

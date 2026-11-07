//! The real thing on X11: a GTK entry, keys through XTEST, RECORD into the engine, the correction through XTEST back.
//! Runs under `packaging/e2e.sh`, with `KS_E2E` set in a private X server; otherwise it says so and passes.

mod support;

use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gtk::prelude::*;
use ks_backend::InputBackend;
use ks_config::config::SettingsStore;
use ks_config::history::HistoryStore;
use ks_config::learning::LearningStore;
use ks_engine::rule_editor::{RuleDraft, build_rule, draft_from_prompt};
use ks_engine::{KeySwitchEngine, LearningPrompt};
use ks_linux::gtk_prompt::{LearningPromptWindow, PromptAnchor, PromptCallbacks, RuleCallbacks, RuleEditorWindow};
use ks_linux::linux_app::{engine_builder, new_backend};
use serde_json::json;
use support::{Case, IDLE_PAUSE, SETTLE, CONTEXT_RESOLUTION, Typer, entry_window, pump, STARTUP, judge, GRACE, INTER_CASE, PAUSE_KEYSYM, RETURN_KEYSYM, LEARNING_CONFIRMATION, MENU_LAYOUT, EARLY_SETUP, SLOW_CHARACTER, SHIFT_L_KEYSYM, SHIFT_R_KEYSYM, dbus_call};

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
    let rule_windows: Rc<std::cell::RefCell<Vec<RuleEditorWindow>>> = Rc::default();
    let offer_window: Rc<std::cell::Cell<Option<i64>>> = Rc::default();
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
    let settle = |milliseconds: u64| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(milliseconds);
        while std::time::Instant::now() < deadline {
            pump(10);
            drain();
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
        let names = dbus_call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus.ListNames", &[])?;
        if names.contains("org.freedesktop.portal.Desktop") || names.contains("org.freedesktop.portal.Documents") {
            return Err("GUI tests unexpectedly activated a desktop or document portal".to_owned());
        }
        println!("NO_DOCUMENT_PORTAL_E2E_OK");
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

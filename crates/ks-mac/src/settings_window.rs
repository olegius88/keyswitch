//! The settings window in AppKit: tabs built from the same setting descriptions as on Windows, the lists and actions beside
//! them, the learning prompt and the rule editor.
use std::cell::RefCell;
use std::sync::Arc;

use ks_config::history::data_dir;
use ks_config::learning::{RuleAction, RuleMatch};
use ks_engine::LearningPrompt;
use ks_engine::rule_editor::{RuleDraft, build_rule, draft_from_prompt, layout_hint, prompt_word_line};
use ks_ui::windows_ui_model::*;
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use serde_json::{Value, json};

use crate::desktop_app::{Services, quit};
use crate::macos_app::VERSION;
use crate::macos_native::run_on_main;
use crate::macos_objc::{Id, open_path, string, text_of};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Point {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Size {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Rect {
    origin: Point,
    size: Size,
}

// SAFETY: the layouts are those of CGPoint, CGSize and CGRect, as their encodings say.
unsafe impl Encode for Point {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
// SAFETY: as above.
unsafe impl Encode for Size {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
// SAFETY: as above.
unsafe impl Encode for Rect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[Point::ENCODING, Size::ENCODING]);
}
// SAFETY: as above.
unsafe impl RefEncode for Rect {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect { origin: Point { x, y }, size: Size { width, height } }
}

const WINDOW_WIDTH: f64 = 760.0;
const WINDOW_HEIGHT: f64 = 620.0;
const PAGE_WIDTH: f64 = 700.0;
const MARGIN: f64 = 16.0;
const STYLE_TITLED_CLOSABLE_MINIATURIZABLE: u64 = 1 | 2 | 4;
const STYLE_PANEL_NONACTIVATING: u64 = 1 << 7;
const BUFFERED: u64 = 2;
const BUTTON_TYPE_SWITCH: u64 = 3;
const WINDOW_LEVEL_FLOATING: isize = 3;

const PAGES: [(&str, &str); 10] = [
    ("overview", "Обзор"),
    ("autocorrection", "Автокоррекция"),
    ("layouts", "Раскладки"),
    ("hotkeys", "Горячие клавиши"),
    ("exclusions", "Исключения"),
    ("appearance", "Внешний вид"),
    ("history", "История"),
    ("updates", "Обновления"),
    ("maintenance", "Обслуживание"),
    ("about", "О программе"),
];

// The actions of the plain buttons, by their tags.
const ACTION_SAVE_EXCLUSIONS: isize = 1;
const ACTION_REFRESH_HISTORY: isize = 2;
const RELEASES_PAGE: &str = "https://github.com/olegius88/keyswitch/releases";
const UPDATES_NOTE: &str = "На macOS обновление ставится вручную: скачайте архив со страницы выпусков и замените им приложение в папке «Программы». Разрешение на управление компьютером при этом сохранится.";
const ACTION_OPEN_RELEASES: isize = 3;
const ACTION_ADD_RULE: isize = 4;
const ACTION_CLEAR_RULES: isize = 5;
const ACTION_OPEN_LOGS: isize = 6;
const ACTION_RESET_SETTINGS: isize = 7;
const ACTION_QUIT: isize = 8;

struct Ui {
    services: Arc<Services>,
    window: Id,
    tabs: Id,
    target: Id,
    specs: Vec<SettingSpec>,
    controls: Vec<Id>,
    status: Id,
    applications: Id,
    words: Id,
    history: Id,
    rules: Id,
    prompt: Id,
    prompt_text: Id,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn with_ui<T>(action: impl FnOnce(&mut Ui) -> T) -> Option<T> {
    UI.with(|ui| ui.try_borrow_mut().ok().and_then(|mut ui| ui.as_mut().map(action)))
}

extern "C-unwind" fn control_changed(_this: &AnyObject, _selector: Sel, sender: *mut AnyObject) {
    // SAFETY: the sender is the control AppKit passes for the time of this call.
    let tag: isize = unsafe { msg_send![sender, tag] };
    let index = tag as usize;
    run_on_main(Box::new(move || {
        with_ui(|ui| ui.save_control(index));
    }));
}

extern "C-unwind" fn button_pressed(_this: &AnyObject, _selector: Sel, sender: *mut AnyObject) {
    // SAFETY: the sender is the button AppKit passes for the time of this call.
    let tag: isize = unsafe { msg_send![sender, tag] };
    run_on_main(Box::new(move || action(tag)));
}

extern "C-unwind" fn is_flipped(_this: &AnyObject, _selector: Sel) -> Bool {
    Bool::YES
}

fn register_classes() -> Option<&'static AnyClass> {
    let mut flipped = ClassBuilder::new(c"KeySwitchFlippedView", class!(NSView))?;
    // SAFETY: the function has the signature of the selector.
    unsafe { flipped.add_method(sel!(isFlipped), is_flipped as extern "C-unwind" fn(_, _) -> _) };
    flipped.register();
    let mut target = ClassBuilder::new(c"KeySwitchSettingsTarget", class!(NSObject))?;
    // SAFETY: the functions have the signatures of these selectors.
    unsafe {
        target.add_method(sel!(controlChanged:), control_changed as extern "C-unwind" fn(_, _, _));
        target.add_method(sel!(buttonPressed:), button_pressed as extern "C-unwind" fn(_, _, _));
    }
    Some(target.register())
}

fn add(parent: Id, child: Id) {
    // SAFETY: messages on the main thread to live views.
    unsafe {
        let _: () = msg_send![parent, addSubview: child];
    }
}

/// A wrapping label placed at `y`; says how tall it came out.
fn label(parent: Id, text: &str, y: f64, small: bool) -> f64 {
    // SAFETY: messages on the main thread; the label stays alive as a subview of its parent.
    unsafe {
        let field: Id = msg_send![class!(NSTextField), wrappingLabelWithString: string(text)];
        if small {
            let font: Id = msg_send![class!(NSFont), systemFontOfSize: 11.0_f64];
            let _: () = msg_send![field, setFont: font];
            let color: Id = msg_send![class!(NSColor), secondaryLabelColor];
            let _: () = msg_send![field, setTextColor: color];
        }
        let fitted: Size = msg_send![field, sizeThatFits: Size { width: PAGE_WIDTH - 2.0 * MARGIN, height: 10000.0 }];
        let _: () = msg_send![field, setFrame: rect(MARGIN, y, PAGE_WIDTH - 2.0 * MARGIN, fitted.height)];
        add(parent, field);
        fitted.height
    }
}

fn button(parent: Id, target: Id, title: &str, tag: isize, x: f64, y: f64, width: f64) -> Id {
    // SAFETY: messages on the main thread; the button stays alive as a subview of its parent.
    unsafe {
        let made: Id = msg_send![class!(NSButton), buttonWithTitle: string(title), target: target, action: sel!(buttonPressed:)];
        let _: () = msg_send![made, setTag: tag];
        let _: () = msg_send![made, setFrame: rect(x, y, width, 28.0)];
        add(parent, made);
        made
    }
}

fn text_area(parent: Id, y: f64, height: f64, editable: bool) -> Id {
    // SAFETY: messages on the main thread; the views stay alive through their parents.
    unsafe {
        let scroll: Id = msg_send![class!(NSTextView), scrollableTextView];
        let _: () = msg_send![scroll, setFrame: rect(MARGIN, y, PAGE_WIDTH - 2.0 * MARGIN, height)];
        let view: Id = msg_send![scroll, documentView];
        let _: () = msg_send![view, setEditable: editable];
        let _: () = msg_send![view, setRichText: false];
        // Quotes and dashes typed into a list must stay as typed.
        let _: () = msg_send![view, setAutomaticQuoteSubstitutionEnabled: false];
        let _: () = msg_send![view, setAutomaticDashSubstitutionEnabled: false];
        let _: () = msg_send![view, setAutomaticTextReplacementEnabled: false];
        add(parent, scroll);
        view
    }
}

fn set_area(view: Id, text: &str) {
    if view.is_null() {
        return;
    }
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: a message on the main thread to a live text view.
        unsafe {
            let _: () = msg_send![view, setString: string(text)];
        }
    });
}

fn area_text(view: Id) -> String {
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: a message on the main thread to a live text view.
        let value: Id = unsafe { msg_send![view, string] };
        text_of(value)
    })
}

fn set_label(field: Id, text: &str) {
    if field.is_null() {
        return;
    }
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: a message on the main thread to a live control.
        unsafe {
            let _: () = msg_send![field, setStringValue: string(text)];
        }
    });
}

fn new_page() -> Id {
    let class = AnyClass::get(c"KeySwitchFlippedView").unwrap_or(class!(NSView));
    // SAFETY: an owned view handed to the tab item, which keeps it.
    unsafe {
        let allocated: Id = msg_send![class, alloc];
        msg_send![allocated, initWithFrame: rect(0.0, 0.0, PAGE_WIDTH, 2000.0)]
    }
}

impl Ui {
    fn setting(&self, path: &str) -> Option<Value> {
        self.services.settings.get(path)
    }

    /// One setting: its control and its description; says where the next one starts.
    fn add_spec(&mut self, page: Id, spec: &SettingSpec, y: f64) -> f64 {
        let index = self.controls.len();
        let stored = self.setting(&spec.path);
        let control: Id;
        let mut y = y;
        // SAFETY: messages on the main thread; each control stays alive as a subview of the page.
        unsafe {
            match spec.kind {
                ControlKind::Bool => {
                    control = msg_send![class!(NSButton), checkboxWithTitle: string(mac_wording(&spec.title)), target: self.target, action: sel!(controlChanged:)];
                    let _: () = msg_send![control, setButtonType: BUTTON_TYPE_SWITCH];
                    let on: isize = stored.as_ref().is_some_and(truthy).into();
                    let _: () = msg_send![control, setState: on];
                    let _: () = msg_send![control, setFrame: rect(MARGIN, y, PAGE_WIDTH - 2.0 * MARGIN, 22.0)];
                    y += 24.0;
                }
                ControlKind::Choice => {
                    y += label(page, mac_wording(&spec.title), y, false) + 2.0;
                    let allocated: Id = msg_send![class!(NSPopUpButton), alloc];
                    control = msg_send![allocated, initWithFrame: rect(MARGIN, y, 320.0, 26.0), pullsDown: false];
                    for (_, title) in &spec.choices {
                        let _: () = msg_send![control, addItemWithTitle: string(title)];
                    }
                    let _: () = msg_send![control, selectItemWithTitle: string(&control_text(spec, stored.as_ref()))];
                    let _: () = msg_send![control, setTarget: self.target];
                    let _: () = msg_send![control, setAction: sel!(controlChanged:)];
                    y += 28.0;
                }
                ControlKind::Int | ControlKind::Float | ControlKind::Text => {
                    y += label(page, mac_wording(&spec.title), y, false) + 2.0;
                    control = msg_send![class!(NSTextField), textFieldWithString: string(&control_text(spec, stored.as_ref()))];
                    let width = if spec.kind == ControlKind::Text { 320.0 } else { 100.0 };
                    let _: () = msg_send![control, setFrame: rect(MARGIN, y, width, 24.0)];
                    let _: () = msg_send![control, setTarget: self.target];
                    let _: () = msg_send![control, setAction: sel!(controlChanged:)];
                    let cell: Id = msg_send![control, cell];
                    let _: () = msg_send![cell, setSendsActionOnEndEditing: true];
                    y += 26.0;
                }
            }
            let _: () = msg_send![control, setTag: index as isize];
        }
        add(page, control);
        self.controls.push(control);
        self.specs.push(spec.clone());
        if !spec.description.is_empty() {
            y += label(page, &spec.description, y, true);
        }
        y + 10.0
    }

    fn add_specs(&mut self, page: Id, specs: &[SettingSpec], mut y: f64) -> f64 {
        for spec in specs {
            y = self.add_spec(page, spec, y);
        }
        y
    }

    fn save_control(&mut self, index: usize) {
        let (Some(spec), Some(control)) = (self.specs.get(index).cloned(), self.controls.get(index).copied()) else { return };
        let settings = self.services.settings.clone();
        let value = objc2::rc::autoreleasepool(|_| {
            // SAFETY: messages on the main thread to a live control of this window.
            unsafe {
                match spec.kind {
                    ControlKind::Bool => {
                        let state: isize = msg_send![control, state];
                        Some(json!(state == 1))
                    }
                    ControlKind::Choice => {
                        let title: Id = msg_send![control, titleOfSelectedItem];
                        Some(json!(choice_value(&spec, &text_of(title))))
                    }
                    ControlKind::Int | ControlKind::Float => {
                        let shown: Id = msg_send![control, stringValue];
                        match parse_number(&spec, &text_of(shown)) {
                            Some((value, text)) => {
                                let _: () = msg_send![control, setStringValue: string(&text)];
                                Some(value)
                            }
                            None => {
                                let _: () = msg_send![control, setStringValue: string(&control_text(&spec, settings.get(&spec.path).as_ref()))];
                                None
                            }
                        }
                    }
                    ControlKind::Text => {
                        let shown: Id = msg_send![control, stringValue];
                        let text = text_of(shown);
                        (!text.trim().is_empty()).then(|| json!(text))
                    }
                }
            }
        });
        if let Some(value) = value {
            if let Err(error) = settings.set(&spec.path, value, true) {
                show_error(&format!("Настройка не сохранена: {error}"));
            }
        }
    }

    fn refresh_status(&self) {
        let snapshot = self.services.engine.snapshot();
        let mut text = format!(
            "{}\nРаскладка: {}\nИсправлений: {}\n{}",
            status_line(snapshot.enabled, snapshot.running),
            ks_config::indicator::layout_label(snapshot.current_group),
            snapshot.correction_count,
            snapshot.last_action
        );
        if !snapshot.last_error.is_empty() {
            text.push_str(&format!("\n{}", snapshot.last_error));
        }
        set_label(self.status, &text);
    }

    fn refresh_exclusions(&self) {
        set_area(self.applications, &string_list(self.setting("exclusions.applications").as_ref()).join("\n"));
        set_area(self.words, &string_list(self.setting("exclusions.words").as_ref()).join("\n"));
    }

    fn refresh_history(&self) {
        let rows = self.services.history.read(None).unwrap_or_default();
        let lines: Vec<String> = rows.iter().rev().map(|entry| {
            let row = history_row(entry);
            format!("{}  {} → {}  ({})", row[0], row[1], row[2], row[3])
        }).collect();
        set_area(self.history, &lines.join("\n"));
    }

    fn refresh_rules(&self) {
        let rules = self.services.learning.rules();
        let mut lines = vec![learning_count_text(rules.len())];
        lines.extend(rules.iter().map(|rule| rule_row(rule).join("  ·  ")));
        set_area(self.rules, &lines.join("\n"));
    }


    fn save_exclusions(&self) {
        let lines = |view: Id| -> Vec<String> { area_text(view).lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect() };
        let settings = &self.services.settings;
        let saved = settings.set("exclusions.applications", json!(lines(self.applications)), true).and_then(|()| settings.set("exclusions.words", json!(lines(self.words)), true));
        if let Err(error) = saved {
            show_error(&format!("Исключения не сохранены: {error}"));
        }
    }
}

fn action(tag: isize) {
    match tag {
        ACTION_SAVE_EXCLUSIONS => {
            with_ui(|ui| ui.save_exclusions());
        }
        ACTION_REFRESH_HISTORY => {
            with_ui(|ui| ui.refresh_history());
        }
        ACTION_OPEN_RELEASES => {
            if let Err(error) = std::process::Command::new("open").arg(RELEASES_PAGE).status() {
                alert_later(format!("Страница выпусков не открылась: {error}"));
            }
        }
        ACTION_ADD_RULE => rule_editor(None),
        ACTION_CLEAR_RULES => {
            with_ui(|ui| {
                let _ = ui.services.learning.clear();
                ui.refresh_rules();
            });
        }
        ACTION_OPEN_LOGS => {
            open_path(&data_dir().display().to_string());
        }
        ACTION_RESET_SETTINGS => {
            with_ui(|ui| {
                if let Err(error) = ui.services.settings.reset() {
                    show_error(&format!("Настройки не сброшены: {error}"));
                }
            });
        }
        ACTION_QUIT => quit(),
        _ => {}
    }
}

/// Builds the window, hidden: its pages read the settings now and are shown when asked for.
pub fn create(services: Arc<Services>) {
    let Some(target_class) = register_classes() else { return };
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: messages on the main thread; the window and the target are kept for the life of the program.
        unsafe {
            let allocated: Id = msg_send![target_class, alloc];
            let target: Id = msg_send![allocated, init];
            let allocated: Id = msg_send![class!(NSWindow), alloc];
            let window: Id = msg_send![allocated, initWithContentRect: rect(200.0, 200.0, WINDOW_WIDTH, WINDOW_HEIGHT), styleMask: STYLE_TITLED_CLOSABLE_MINIATURIZABLE, backing: BUFFERED, defer: true];
            let _: () = msg_send![window, setReleasedWhenClosed: false];
            let _: () = msg_send![window, setTitle: string("KeySwitch")];
            let allocated: Id = msg_send![class!(NSTabView), alloc];
            let tabs: Id = msg_send![allocated, initWithFrame: rect(0.0, 0.0, WINDOW_WIDTH, WINDOW_HEIGHT)];
            let _: () = msg_send![window, setContentView: tabs];
            let mut ui = Ui {
                services,
                window,
                tabs,
                target,
                specs: Vec::new(),
                controls: Vec::new(),
                status: std::ptr::null_mut(),
                applications: std::ptr::null_mut(),
                words: std::ptr::null_mut(),
                history: std::ptr::null_mut(),
                rules: std::ptr::null_mut(),
                prompt: std::ptr::null_mut(),
                prompt_text: std::ptr::null_mut(),
            };
            for (identifier, title) in PAGES {
                let page = new_page();
                let height = build_page(&mut ui, identifier, page);
                let _: () = msg_send![page, setFrame: rect(0.0, 0.0, PAGE_WIDTH, height + MARGIN)];
                let scroll: Id = msg_send![class!(NSScrollView), new];
                let _: () = msg_send![scroll, setHasVerticalScroller: true];
                let _: () = msg_send![scroll, setDocumentView: page];
                let allocated: Id = msg_send![class!(NSTabViewItem), alloc];
                let item: Id = msg_send![allocated, initWithIdentifier: string(identifier)];
                let _: () = msg_send![item, setLabel: string(title)];
                let _: () = msg_send![item, setView: scroll];
                let _: () = msg_send![tabs, addTabViewItem: item];
            }
            ui.refresh_status();
            ui.refresh_exclusions();
            ui.refresh_rules();
            UI.with(|slot| *slot.borrow_mut() = Some(ui));
        }
    });
}

/// The shared descriptions call these things by their Windows names.
fn mac_wording(title: &str) -> &str {
    match title {
        "Запускать вместе с Windows" => "Запускать при входе в систему",
        "Запускать свёрнутым в трей" => "Запускать свёрнутым в строку меню",
        "Закрывать окно в трей" => "Закрывать окно в строку меню",
        "Показывать индикатор в области уведомлений" => "Показывать индикатор в строке меню",
        other => other,
    }
}

fn build_page(ui: &mut Ui, identifier: &str, page: Id) -> f64 {
    let mut y = MARGIN;
    let target = ui.target;
    match identifier {
        "overview" => {
            // SAFETY: messages on the main thread; the label stays alive as a subview of the page.
            unsafe {
                ui.status = msg_send![class!(NSTextField), wrappingLabelWithString: string("")];
                let _: () = msg_send![ui.status, setFrame: rect(MARGIN, y, PAGE_WIDTH - 2.0 * MARGIN, 90.0)];
            }
            add(page, ui.status);
            y += 100.0;
            y += label(page, "Проверка в реальном поле: выберите EN и напечатайте ghbdtn — после пробела появится «привет».", y, false) + 6.0;
            // SAFETY: as above.
            unsafe {
                let field: Id = msg_send![class!(NSTextField), textFieldWithString: string("")];
                let _: () = msg_send![field, setFrame: rect(MARGIN, y, PAGE_WIDTH - 2.0 * MARGIN, 26.0)];
                add(page, field);
            }
            y += 36.0;
        }
        "autocorrection" => {
            y = ui.add_specs(page, &autocorrection_settings(), y);
            y = ui.add_specs(page, &trigger_settings(), y);
            y = ui.add_specs(page, &application_settings(), y);
        }
        "layouts" => y = ui.add_specs(page, &manual_switch_settings(), y),
        "hotkeys" => y = ui.add_specs(page, &hotkey_settings(), y),
        "exclusions" => {
            y += label(page, "В исключённых программах KeySwitch наблюдает раскладку, но не заменяет введённые слова. По одной записи в строке.", y, false) + 6.0;
            y += label(page, "Программы", y, false) + 2.0;
            ui.applications = text_area(page, y, 140.0, true);
            y += 150.0;
            y += label(page, "Слова", y, false) + 2.0;
            ui.words = text_area(page, y, 120.0, true);
            y += 130.0;
            button(page, target, "Сохранить", ACTION_SAVE_EXCLUSIONS, MARGIN, y, 140.0);
            y += 36.0;
        }
        "appearance" => y = ui.add_specs(page, &system_settings(), y),
        "history" => {
            button(page, target, "Обновить", ACTION_REFRESH_HISTORY, MARGIN, y, 140.0);
            y += 36.0;
            ui.history = text_area(page, y, 440.0, false);
            y += 450.0;
        }
        "updates" => {
            y += label(page, &format!("Установлена версия {VERSION}"), y, false) + 6.0;
            y += label(page, UPDATES_NOTE, y, true) + 10.0;
            button(page, target, "Открыть страницу выпусков", ACTION_OPEN_RELEASES, MARGIN, y, 240.0);
            y += 36.0;
        }
        "maintenance" => {
            y += label(page, "Правила переключения", y, false) + 4.0;
            ui.rules = text_area(page, y, 150.0, false);
            y += 160.0;
            button(page, target, "Добавить…", ACTION_ADD_RULE, MARGIN, y, 140.0);
            button(page, target, "Удалить все", ACTION_CLEAR_RULES, MARGIN + 150.0, y, 140.0);
            y += 40.0;
            y = ui.add_specs(page, &diagnostic_settings(), y);
            button(page, target, "Открыть папку журнала", ACTION_OPEN_LOGS, MARGIN, y, 200.0);
            button(page, target, "Сбросить настройки", ACTION_RESET_SETTINGS, MARGIN + 210.0, y, 180.0);
            y += 36.0;
        }
        _ => {
            y += label(page, &format!("KeySwitch {VERSION}"), y, false) + 6.0;
            y += label(page, "Локальная автоматическая замена раскладки EN/RU. Набранный текст никуда не отправляется.", y, false) + 6.0;
            y += label(page, &format!("Данные: {}", data_dir().display()), y, true) + 10.0;
            button(page, target, "Выйти из KeySwitch", ACTION_QUIT, MARGIN, y, 200.0);
            y += 36.0;
        }
    }
    y
}

/// Shows the window, on a page when one is named.
pub fn show(page: Option<&'static str>) {
    with_ui(|ui| {
        objc2::rc::autoreleasepool(|_| {
            // SAFETY: messages on the main thread to the window this module owns.
            unsafe {
                if let Some(page) = page {
                    let _: () = msg_send![ui.tabs, selectTabViewItemWithIdentifier: string(page)];
                }
                let application: Id = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![application, activateIgnoringOtherApps: true];
                let _: () = msg_send![ui.window, center];
                let _: () = msg_send![ui.window, makeKeyAndOrderFront: std::ptr::null_mut::<AnyObject>()];
            }
        });
        ui.refresh_status();
        ui.refresh_history();
        ui.refresh_rules();
    });
}

pub fn refresh_status() {
    with_ui(|ui| ui.refresh_status());
}

pub fn show_error(message: &str) {
    alert("KeySwitch", message);
}

fn alert(title: &str, message: &str) {
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: an alert run modally on the main thread.
        unsafe {
            let alert: Id = msg_send![class!(NSAlert), new];
            let _: () = msg_send![alert, setMessageText: string(title)];
            let _: () = msg_send![alert, setInformativeText: string(message)];
            let _: isize = msg_send![alert, runModal];
            let _: () = msg_send![alert, release];
        }
    });
}

/// The learning prompt: a panel that takes no focus, because Enter and Esc reach the engine, not the panel.
pub fn learning_prompt(prompt: Option<&LearningPrompt>) {
    with_ui(|ui| {
        objc2::rc::autoreleasepool(|_| {
            // SAFETY: messages on the main thread to the panel this module owns.
            unsafe {
                if ui.prompt.is_null() {
                    let allocated: Id = msg_send![class!(NSPanel), alloc];
                    ui.prompt = msg_send![allocated, initWithContentRect: rect(0.0, 0.0, 420.0, 90.0), styleMask: (1u64 | STYLE_PANEL_NONACTIVATING), backing: BUFFERED, defer: true];
                    let _: () = msg_send![ui.prompt, setReleasedWhenClosed: false];
                    let _: () = msg_send![ui.prompt, setLevel: WINDOW_LEVEL_FLOATING];
                    let _: () = msg_send![ui.prompt, setTitle: string("KeySwitch")];
                    ui.prompt_text = msg_send![class!(NSTextField), wrappingLabelWithString: string("")];
                    let _: () = msg_send![ui.prompt_text, setFrame: rect(12.0, 10.0, 396.0, 70.0)];
                    let content: Id = msg_send![ui.prompt, contentView];
                    let _: () = msg_send![content, addSubview: ui.prompt_text];
                }
                match prompt {
                    Some(prompt) => {
                        let text = format!("Добавить правило переключения?\n{}\nEnter — да, Esc — нет", prompt_word_line(prompt));
                        let _: () = msg_send![ui.prompt_text, setStringValue: string(&text)];
                        let _: () = msg_send![ui.prompt, center];
                        let _: () = msg_send![ui.prompt, orderFrontRegardless];
                    }
                    None => {
                        let _: () = msg_send![ui.prompt, orderOut: std::ptr::null_mut::<AnyObject>()];
                    }
                }
            }
        });
    });
}

/// The rule editor: an alert with the pattern, the condition and the action.
pub fn rule_editor(prompt: Option<&LearningPrompt>) {
    let Some(engine) = with_ui(|ui| ui.services.engine.clone()) else { return };
    let mut draft = prompt.map_or_else(RuleDraft::default, draft_from_prompt);
    engine.set_rule_editor_open(true);
    objc2::rc::autoreleasepool(|_| {
        // SAFETY: an alert run modally on the main thread with its accessory view; each is released or autoreleased here.
        unsafe {
            let alert: Id = msg_send![class!(NSAlert), new];
            let _: () = msg_send![alert, setMessageText: string("Правило переключения")];
            let hint = layout_hint(&draft);
            let _: () = msg_send![alert, setInformativeText: string(if hint.is_empty() { "Сочетание букв, как оно набирается" } else { &hint })];
            let box_view: Id = msg_send![class!(NSView), new];
            let _: () = msg_send![box_view, setFrame: rect(0.0, 0.0, 340.0, 96.0)];
            let field: Id = msg_send![class!(NSTextField), textFieldWithString: string(&draft.pattern)];
            let _: () = msg_send![field, setFrame: rect(0.0, 68.0, 340.0, 24.0)];
            let allocated: Id = msg_send![class!(NSPopUpButton), alloc];
            let condition: Id = msg_send![allocated, initWithFrame: rect(0.0, 34.0, 340.0, 26.0), pullsDown: false];
            let matches = [RuleMatch::Exact, RuleMatch::Prefix, RuleMatch::Contains];
            for item in matches {
                let _: () = msg_send![condition, addItemWithTitle: string(ks_engine::rule_editor::match_label(item))];
            }
            let selected = matches.iter().position(|item| *item == draft.r#match).unwrap_or(0) as isize;
            let _: () = msg_send![condition, selectItemAtIndex: selected];
            let allocated: Id = msg_send![class!(NSPopUpButton), alloc];
            let choice: Id = msg_send![allocated, initWithFrame: rect(0.0, 0.0, 340.0, 26.0), pullsDown: false];
            let actions = [RuleAction::Convert, RuleAction::Keep];
            for item in actions {
                let _: () = msg_send![choice, addItemWithTitle: string(ks_engine::rule_editor::action_label(item))];
            }
            let selected = actions.iter().position(|item| *item == draft.action).unwrap_or(0) as isize;
            let _: () = msg_send![choice, selectItemAtIndex: selected];
            for view in [field, condition, choice] {
                let _: () = msg_send![box_view, addSubview: view];
            }
            let _: () = msg_send![alert, setAccessoryView: box_view];
            let _: Id = msg_send![alert, addButtonWithTitle: string("OK")];
            let _: Id = msg_send![alert, addButtonWithTitle: string("Отмена")];
            let application: Id = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![application, activateIgnoringOtherApps: true];
            let answer: isize = msg_send![alert, runModal];
            // NSAlertFirstButtonReturn.
            if answer == 1000 {
                let pattern: Id = msg_send![field, stringValue];
                draft.pattern = text_of(pattern);
                let index: isize = msg_send![condition, indexOfSelectedItem];
                draft.r#match = matches[index.max(0) as usize];
                let index: isize = msg_send![choice, indexOfSelectedItem];
                draft.action = actions[index.max(0) as usize];
                match build_rule(&draft) {
                    Ok(rule) => {
                        if let Err(error) = engine.add_learning_rule(rule, draft.editing.as_ref(), &draft.application) {
                            alert_later(format!("Правило не сохранено: {error}"));
                        }
                    }
                    Err(error) => alert_later(error.to_string()),
                }
            }
            let _: () = msg_send![alert, release];
        }
    });
    engine.set_rule_editor_open(false);
    with_ui(|ui| ui.refresh_rules());
}

fn alert_later(message: String) {
    run_on_main(Box::new(move || show_error(&message)));
}

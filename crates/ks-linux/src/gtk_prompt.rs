//! The windows of the switching rules: the offer above the caret and the rule window (the Python's `learning_prompt.py`).

use ks_cast::To as _;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use ks_config::learning::{RULE_ACTIONS, RULE_MATCHES, RuleAction, RuleMatch};
use ks_engine::LearningPrompt;
use ks_engine::rule_editor::{
    ACTION_TITLE, CASE_LABEL, CONDITION_LEAD, CONDITION_TITLE, EXTRA_LEAD, PATTERN_LABEL, PROMPT_HINT, PROMPT_TITLE, RuleDraft, WINDOW_TITLE, action_label,
    layout_hint, match_label, prompt_word_line,
};

use crate::gtk_model::{
    LEARNING_PROMPT_GTK_MARGIN_X_PIXELS, LEARNING_PROMPT_GTK_MARGIN_Y_PIXELS, LEARNING_PROMPT_GTK_SPACING_PIXELS, LEARNING_PROMPT_GTK_WIDTH_PIXELS,
    LEARNING_PROMPT_WINDOW_TITLE, prompt_position,
};

/// The point the offer is placed over and the window the user typed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptAnchor {
    pub x: i32,
    pub y: i32,
    pub window: Option<i64>,
}

/// What the offer asks of the engine and of the window system.
pub struct PromptCallbacks {
    pub confirm: Box<dyn Fn(&LearningPrompt) -> bool>,
    pub dismiss: Box<dyn Fn(&LearningPrompt) -> bool>,
    /// Puts the window with this X11 id (0 when the surface has none) at the position.
    pub position: Box<dyn Fn(i64, i32, i32) -> bool>,
    /// Gives the keyboard back to the window the user typed in.
    pub restore: Box<dyn Fn(Option<i64>) -> bool>,
}

struct PromptInner {
    window: gtk::Window,
    word: gtk::Label,
    callbacks: PromptCallbacks,
    prompt: RefCell<Option<LearningPrompt>>,
    anchor: RefCell<Option<PromptAnchor>>,
}

/// Small keyboard-focused rule offer positioned above the active caret.
pub struct LearningPromptWindow {
    inner: Rc<PromptInner>,
}

fn boxed(spacing: i32) -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(spacing)
        .margin_top(LEARNING_PROMPT_GTK_MARGIN_Y_PIXELS)
        .margin_bottom(LEARNING_PROMPT_GTK_MARGIN_Y_PIXELS)
        .margin_start(LEARNING_PROMPT_GTK_MARGIN_X_PIXELS)
        .margin_end(LEARNING_PROMPT_GTK_MARGIN_X_PIXELS)
        .build()
}

fn label(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.0).build()
}

/// The monitor under a screen point, as `(x, y, width, height)`.
fn monitor_at(display: &gdk::Display, x: i32, y: i32) -> Option<(i32, i32, i32, i32)> {
    let monitors = display.monitors();
    (0..monitors.n_items()).filter_map(|index| monitors.item(index)?.downcast::<gdk::Monitor>().ok()).map(|monitor| monitor.geometry()).find_map(|area| {
        let inside = area.x() <= x && x < area.x() + area.width() && area.y() <= y && y < area.y() + area.height();
        inside.then(|| (area.x(), area.y(), area.width(), area.height()))
    })
}

impl PromptInner {
    fn position_above_anchor(&self) {
        let Some(anchor) = *self.anchor.borrow() else { return };
        let Some(surface) = self.window.surface() else { return };
        let window = surface.downcast::<gdk_x11::X11Surface>().map_or(0, |surface| surface.xid().bits::<i64>());
        let monitor = monitor_at(&WidgetExt::display(&self.window), anchor.x, anchor.y);
        let (x, y) = prompt_position((anchor.x, anchor.y), (self.window.width(), self.window.height()), monitor);
        (self.callbacks.position)(window, x, y);
    }

    fn key_pressed(&self, key: gdk::Key) -> glib::Propagation {
        let Some(prompt) = self.prompt.borrow().clone() else { return glib::Propagation::Proceed };
        if key == gdk::Key::Return || key == gdk::Key::KP_Enter {
            (self.callbacks.confirm)(&prompt);
            return glib::Propagation::Stop;
        }
        if key == gdk::Key::Escape {
            (self.callbacks.dismiss)(&prompt);
            return glib::Propagation::Stop;
        }
        (self.callbacks.dismiss)(&prompt);
        glib::Propagation::Proceed
    }
}

impl LearningPromptWindow {
    pub fn new(application: &gtk::Application, callbacks: PromptCallbacks) -> LearningPromptWindow {
        let window = gtk::Window::builder()
            .application(application)
            .title(LEARNING_PROMPT_WINDOW_TITLE)
            .decorated(false)
            .resizable(false)
            .modal(false)
            .hide_on_close(true)
            .focusable(true)
            .default_width(LEARNING_PROMPT_GTK_WIDTH_PIXELS)
            .default_height(-1)
            .build();
        window.add_css_class("learning-prompt");
        let content = boxed(LEARNING_PROMPT_GTK_SPACING_PIXELS);
        let question = label(PROMPT_TITLE);
        question.add_css_class("heading");
        let word = label("");
        word.add_css_class("learning-prompt-word");
        let hint = label(PROMPT_HINT);
        hint.add_css_class("dim-label");
        content.append(&question);
        content.append(&word);
        content.append(&hint);
        window.set_child(Some(&content));
        let inner = Rc::new(PromptInner { window, word, callbacks, prompt: RefCell::new(None), anchor: RefCell::new(None) });
        let controller = gtk::EventControllerKey::new();
        let weak: Weak<PromptInner> = Rc::downgrade(&inner);
        controller.connect_key_pressed(move |_, key, _, _| weak.upgrade().map_or(glib::Propagation::Proceed, |inner| inner.key_pressed(key)));
        inner.window.add_controller(controller);
        let weak = Rc::downgrade(&inner);
        inner.window.connect_close_request(move |_| {
            if let Some(inner) = weak.upgrade()
                && let Some(prompt) = inner.prompt.borrow().clone()
            {
                (inner.callbacks.dismiss)(&prompt);
            }
            glib::Propagation::Stop
        });
        LearningPromptWindow { inner }
    }

    /// The window the user typed in when the offer was made.
    pub fn anchor_window(&self) -> Option<i64> {
        self.inner.anchor.borrow().and_then(|anchor| anchor.window)
    }

    pub fn show_prompt(&self, prompt: LearningPrompt, anchor: Option<PromptAnchor>) {
        let inner = &self.inner;
        *inner.anchor.borrow_mut() = anchor;
        inner.word.set_text(&prompt_word_line(&prompt));
        *inner.prompt.borrow_mut() = Some(prompt);
        inner.window.present();
        inner.window.grab_focus();
        let weak = Rc::downgrade(inner);
        glib::idle_add_local_once(move || {
            if let Some(inner) = weak.upgrade() {
                inner.position_above_anchor();
            }
        });
    }

    pub fn hide_prompt(&self) {
        let inner = &self.inner;
        let anchor = inner.anchor.borrow_mut().take();
        *inner.prompt.borrow_mut() = None;
        inner.window.set_visible(false);
        if let Some(anchor) = anchor {
            (inner.callbacks.restore)(anchor.window);
        }
    }

    pub fn destroy(&self) {
        self.inner.window.destroy();
    }

    pub fn is_visible(&self) -> bool {
        self.inner.window.is_visible()
    }
}

/// What the rule window asks of the application.
pub struct RuleCallbacks {
    /// Turns the form into a rule: an empty answer, or what to fix, which the window shows instead of closing.
    pub accept: Box<dyn Fn(&RuleDraft) -> String>,
    pub closed: Box<dyn Fn()>,
    /// The window gained or lost the keyboard: the engine leaves what is typed here alone while it has it.
    pub focused: Box<dyn Fn(bool)>,
}

struct RuleForm {
    window: gtk::Window,
    draft: RuleDraft,
    entry: gtk::Entry,
    hint: gtk::Label,
    problem: gtk::Label,
    matches: Vec<(RuleMatch, gtk::CheckButton)>,
    actions: Vec<(RuleAction, gtk::CheckButton)>,
    case_sensitive: gtk::CheckButton,
    callbacks: RuleCallbacks,
}

/// The rule window: the letters, the condition, the case and the action. Enter in the letters field is OK and Escape is
/// Cancel.
pub struct RuleEditorWindow {
    form: Rc<RuleForm>,
}

fn radio_group<T: Copy + PartialEq>(
    values: &[T],
    text: impl Fn(T) -> &'static str,
    selected: T,
    into: &gtk::Box,
) -> Vec<(T, gtk::CheckButton)> {
    let mut buttons: Vec<(T, gtk::CheckButton)> = Vec::new();
    for value in values {
        let button = gtk::CheckButton::with_label(text(*value));
        button.set_group(buttons.first().map(|(_, first)| first));
        into.append(&button);
        buttons.push((*value, button));
    }
    if let Some((_, button)) = buttons.iter().find(|(value, _)| *value == selected) {
        button.set_active(true);
    }
    buttons
}

impl RuleForm {
    fn current_draft(&self) -> RuleDraft {
        let r#match = self.matches.iter().find(|(_, button)| button.is_active()).map_or(self.draft.r#match, |(value, _)| *value);
        let action = self.actions.iter().find(|(_, button)| button.is_active()).map_or(self.draft.action, |(value, _)| *value);
        RuleDraft {
            pattern: self.entry.text().to_string(),
            r#match,
            case_sensitive: self.case_sensitive.is_active(),
            action,
            application: self.draft.application.clone(),
            editing: self.draft.editing.clone(),
        }
    }

    fn pattern_changed(&self) {
        self.hint.set_text(&layout_hint(&self.current_draft()));
        self.problem.set_text("");
    }

    fn confirm(&self) {
        let problem = (self.callbacks.accept)(&self.current_draft());
        if problem.is_empty() {
            self.close();
        } else {
            self.problem.set_text(&problem);
        }
    }

    fn close(&self) {
        self.window.destroy();
        (self.callbacks.focused)(false);
        (self.callbacks.closed)();
    }
}

impl RuleEditorWindow {
    pub fn new(application: &gtk::Application, draft: RuleDraft, callbacks: RuleCallbacks) -> RuleEditorWindow {
        let window = gtk::Window::builder().application(application).title(WINDOW_TITLE).resizable(false).build();
        let content = boxed(LEARNING_PROMPT_GTK_SPACING_PIXELS);
        content.append(&label(PATTERN_LABEL));
        let entry = gtk::Entry::builder().text(draft.pattern.as_str()).build();
        content.append(&entry);
        let hint = label("");
        hint.add_css_class("dim-label");
        content.append(&hint);

        let condition = gtk::Box::new(gtk::Orientation::Vertical, LEARNING_PROMPT_GTK_SPACING_PIXELS);
        condition.append(&label(CONDITION_LEAD));
        let matches = radio_group(&RULE_MATCHES, match_label, draft.r#match, &condition);
        condition.append(&label(EXTRA_LEAD));
        let case_sensitive = gtk::CheckButton::builder().label(CASE_LABEL).active(draft.case_sensitive).build();
        condition.append(&case_sensitive);
        content.append(&gtk::Frame::builder().label(CONDITION_TITLE).child(&condition).build());

        let action = gtk::Box::new(gtk::Orientation::Vertical, LEARNING_PROMPT_GTK_SPACING_PIXELS);
        let actions = radio_group(&RULE_ACTIONS, action_label, draft.action, &action);
        content.append(&gtk::Frame::builder().label(ACTION_TITLE).child(&action).build());

        let problem = gtk::Label::builder().xalign(0.0).wrap(true).build();
        problem.add_css_class("error");
        content.append(&problem);
        let buttons = gtk::Box::builder().spacing(LEARNING_PROMPT_GTK_SPACING_PIXELS).halign(gtk::Align::End).build();
        let cancel = gtk::Button::with_label("Отмена");
        let ok = gtk::Button::with_label("OK");
        ok.add_css_class("suggested-action");
        buttons.append(&cancel);
        buttons.append(&ok);
        content.append(&buttons);
        window.set_child(Some(&content));

        let form = Rc::new(RuleForm { window, draft, entry, hint, problem, matches, actions, case_sensitive, callbacks });
        let weak = Rc::downgrade(&form);
        let with_form = move |action: fn(&RuleForm)| {
            let weak = weak.clone();
            move || {
                if let Some(form) = weak.upgrade() {
                    action(&form);
                }
            }
        };
        let changed = with_form(RuleForm::pattern_changed);
        form.entry.connect_changed(move |_| changed());
        let confirm = with_form(RuleForm::confirm);
        form.entry.connect_activate({
            let confirm = confirm.clone();
            move |_| confirm()
        });
        ok.connect_clicked({
            let confirm = confirm.clone();
            move |_| confirm()
        });
        let close = with_form(RuleForm::close);
        cancel.connect_clicked({
            let close = close.clone();
            move |_| close()
        });
        let controller = gtk::EventControllerKey::new();
        controller.connect_key_pressed({
            let close = close.clone();
            move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    close();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        });
        form.window.add_controller(controller);
        form.window.connect_close_request({
            let close = close.clone();
            move |_| {
                close();
                glib::Propagation::Stop
            }
        });
        let weak = Rc::downgrade(&form);
        form.window.connect_is_active_notify(move |window| {
            if let Some(form) = weak.upgrade() {
                (form.callbacks.focused)(window.is_active());
            }
        });
        form.pattern_changed();
        RuleEditorWindow { form }
    }

    pub fn present(&self) {
        self.form.window.present();
    }

    pub fn destroy(&self) {
        self.form.window.destroy();
    }

    /// The form as it stands, for tests and for the application's diagnostics.
    /// What OK does.
    pub fn confirm(&self) {
        self.form.confirm();
    }

    pub fn current_draft(&self) -> RuleDraft {
        self.form.current_draft()
    }
}

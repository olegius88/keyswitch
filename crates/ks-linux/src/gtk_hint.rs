//! The one-time hints on the screen of a Linux desktop: a card above the caret, a system notification for the place near
//! the tray icon, and the driver that hands them what `ks_ui::hints` decides.
//!
//! The card never takes the keyboard from the field typed in. It is a window of its own, shown without being presented,
//! and on X11 it tells the window manager not to focus it (`_NET_WM_USER_TIME` of 0). A window manager or a compositor
//! that gives it the keyboard all the same (a click on it, or GNOME and KDE on Wayland when it maps) has the keyboard
//! taken back at once.

use ks_cast::To as _;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gio, glib};
use ks_config::logsetup;
use ks_engine::Shared;
use ks_ui::constants::{HINT_CARD_WIDTH_PIXELS, HINT_TICK_INTERVAL_MS};
use ks_ui::hints::{Click, EngineInput, HintPresenter, HintView, Hints, Moment, Opens};

use crate::gtk_model::{
    HINT_ACTION_NEVER, HINT_ACTION_OPEN, HINT_NOTIFICATION_ID, HINT_PLACE_ATTEMPTS, HINT_PLACE_RETRY_MS, HINT_WINDOW_TITLE,
    LEARNING_PROMPT_GTK_MARGIN_X_PIXELS, LEARNING_PROMPT_GTK_MARGIN_Y_PIXELS, LEARNING_PROMPT_GTK_SPACING_PIXELS, hint_page, hint_page_name,
    prompt_position, with_shared,
};
use crate::gtk_prompt::{PromptAnchor, monitor_at};
use crate::gtk_window::open_uri;

const LOGGER: &str = "keyswitch.hints";

/// What the user did to the hint `id`.
pub type HintClicked = Rc<dyn Fn(&str, Click)>;

/// What the hints ask of the window system and of the settings window.
pub struct HintCalls {
    /// Puts the window with this title (on X11: this native id, 0 when its surface has none) at a screen position.
    pub position: Box<dyn Fn(&str, i64, i32, i32) -> bool>,
    /// Gives the keyboard back to the window the user typed in.
    pub restore: Box<dyn Fn(Option<i64>) -> bool>,
    pub clicked: HintClicked,
    /// Shows a page of the settings window.
    pub open_page: Box<dyn Fn(&'static str)>,
    /// Takes the card of this hint out of the settings window.
    pub hide_card: Box<dyn Fn(&str)>,
}

/// The text of a hint and its two buttons. A click on the text opens what the hint is about. In a card over another
/// window nothing here is `focusable`: the keyboard stays where it is.
#[must_use]
pub fn hint_content(view: &HintView, focusable: bool, clicked: &HintClicked) -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, LEARNING_PROMPT_GTK_SPACING_PIXELS);
    let text = gtk::Label::builder()
        .label(view.text.as_str())
        .wrap(true)
        .max_width_chars(1)
        .width_request(HINT_CARD_WIDTH_PIXELS - 2 * LEARNING_PROMPT_GTK_MARGIN_X_PIXELS)
        .hexpand(true)
        .xalign(0.0)
        .build();
    let id = view.id;
    let on_text = clicked.clone();
    let gesture = gtk::GestureClick::new();
    gesture.connect_released(move |_, _, _, _| on_text(id, Click::Text));
    text.add_controller(gesture);
    text.set_cursor_from_name(view.opens.as_ref().map(|_| "pointer"));
    content.append(&text);
    let buttons = gtk::Box::builder().spacing(LEARNING_PROMPT_GTK_SPACING_PIXELS).halign(gtk::Align::End).build();
    for (label, click) in [(view.never, Click::Never), (view.ok, Click::Ok)] {
        let button = gtk::Button::builder().label(label).focusable(focusable).focus_on_click(focusable).build();
        let on_button = clicked.clone();
        button.connect_clicked(move |_| on_button(id, click));
        buttons.append(&button);
    }
    content.append(&buttons);
    content
}

/// A card shown above the caret.
struct Card {
    window: gtk::Window,
    id: &'static str,
    anchor: PromptAnchor,
    calls: Rc<HintCalls>,
    attempts: Cell<u32>,
}

impl Card {
    /// Puts the card above the caret. A Wayland desktop finds it by title once it has been mapped, which may take a
    /// moment, so a card it does not know yet is tried again.
    fn place(&self) -> glib::ControlFlow {
        let window = self.window.surface().and_then(|surface| surface.downcast::<gdk_x11::X11Surface>().ok()).map_or(0, |surface| surface.xid().bits::<i64>());
        let monitor = monitor_at(&WidgetExt::display(&self.window), self.anchor.x, self.anchor.y);
        let (_, height, _, _) = self.window.measure(gtk::Orientation::Vertical, HINT_CARD_WIDTH_PIXELS);
        let (x, y) = prompt_position((self.anchor.x, self.anchor.y), (HINT_CARD_WIDTH_PIXELS, height), monitor);
        let placed = (self.calls.position)(HINT_WINDOW_TITLE, window, x, y);
        let left = self.attempts.get() - 1;
        self.attempts.set(left);
        if placed || left == 0 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
    }

    fn give_keyboard_back(&self) {
        (self.calls.restore)(self.anchor.window);
    }
}

/// The card above the caret, while one is shown.
pub struct HintWindow {
    card: Rc<Card>,
}

impl HintWindow {
    /// Shows the card, without asking for the keyboard.
    #[must_use]
    pub fn show(application: &gtk::Application, view: &HintView, anchor: &PromptAnchor, calls: Rc<HintCalls>) -> HintWindow {
        let window = gtk::Window::builder()
            .application(application)
            .title(HINT_WINDOW_TITLE)
            .decorated(false)
            .resizable(false)
            .modal(false)
            .focusable(false)
            .can_focus(false)
            .focus_on_click(false)
            .default_width(HINT_CARD_WIDTH_PIXELS)
            .default_height(-1)
            .build();
        window.add_css_class("hint-window");
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(LEARNING_PROMPT_GTK_SPACING_PIXELS)
            .margin_top(LEARNING_PROMPT_GTK_MARGIN_Y_PIXELS)
            .margin_bottom(LEARNING_PROMPT_GTK_MARGIN_Y_PIXELS)
            .margin_start(LEARNING_PROMPT_GTK_MARGIN_X_PIXELS)
            .margin_end(LEARNING_PROMPT_GTK_MARGIN_X_PIXELS)
            .build();
        let title = gtk::Label::builder().label(view.title).xalign(0.0).build();
        title.add_css_class("heading");
        content.append(&title);
        content.append(&hint_content(view, false, &calls.clicked));
        window.set_child(Some(&content));
        let card = Rc::new(Card { window, id: view.id, anchor: *anchor, calls, attempts: Cell::new(HINT_PLACE_ATTEMPTS) });
        let weak = Rc::downgrade(&card);
        card.window.connect_close_request(move |_| {
            with_shared(&weak, |card| (card.calls.clicked)(card.id, Click::Closed));
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(&card);
        card.window.connect_is_active_notify(move |window| {
            if window.is_active() {
                with_shared(&weak, Card::give_keyboard_back);
            }
        });
        // Asked of the window manager before the window is mapped: never give this window the keyboard, keep it off the
        // task list.
        WidgetExt::realize(&card.window);
        card.window.surface().and_then(|surface| surface.downcast::<gdk_x11::X11Surface>().ok()).iter().for_each(|surface| {
            surface.set_user_time(0);
            surface.set_skip_taskbar_hint(true);
            surface.set_skip_pager_hint(true);
        });
        card.window.set_visible(true);
        let weak = Rc::downgrade(&card);
        glib::timeout_add_local(Duration::from_millis(HINT_PLACE_RETRY_MS.into()), move || weak.upgrade().map_or(glib::ControlFlow::Break, |card| card.place()));
        HintWindow { card }
    }

    #[must_use]
    pub fn id(&self) -> &'static str {
        self.card.id
    }

    /// Taken down at once, and destroyed once whatever signal asked for it has finished.
    fn close(self) {
        self.card.window.set_visible(false);
        let window = self.card.window.clone();
        glib::idle_add_local_once(move || window.destroy());
    }
}

/// What draws the hints on a Linux desktop: the card above the caret, the notification, the cards of the settings window.
pub struct HintScreen {
    application: gtk::Application,
    calls: Rc<HintCalls>,
    card: Option<HintWindow>,
    notified: Option<&'static str>,
}

impl HintScreen {
    #[must_use]
    pub fn new(application: &gtk::Application, calls: HintCalls) -> HintScreen {
        HintScreen { application: application.clone(), calls: Rc::new(calls), card: None, notified: None }
    }

    /// The id of the card shown above the caret.
    #[must_use]
    pub fn card_id(&self) -> Option<&'static str> {
        self.card.as_ref().map(HintWindow::id)
    }

    /// Takes down the card above the caret, if there is one.
    pub fn close_card(&mut self) {
        self.card.take().into_iter().for_each(HintWindow::close);
    }
}

impl HintPresenter<PromptAnchor> for HintScreen {
    fn show_caret(&mut self, view: &HintView, anchor: &PromptAnchor) -> bool {
        self.close_card();
        self.card = Some(HintWindow::show(&self.application, view, anchor, self.calls.clone()));
        true
    }

    /// A notification: a tray icon of the `StatusNotifierItem` kind does not say where it is. A click on it opens what
    /// the hint is about, and the system shows it however it shows notifications.
    fn show_tray(&mut self, view: &HintView) -> bool {
        let notification = gio::Notification::new(view.title);
        notification.set_body(Some(view.text.as_str()));
        notification.set_icon(&gio::ThemedIcon::new("keyswitch"));
        let id = view.id.to_variant();
        notification.set_default_action_and_target_value(&format!("app.{HINT_ACTION_OPEN}"), Some(&id));
        notification.add_button_with_target_value(view.never, &format!("app.{HINT_ACTION_NEVER}"), Some(&id));
        self.application.send_notification(Some(HINT_NOTIFICATION_ID), &notification);
        self.notified = Some(view.id);
        true
    }

    fn hide(&mut self, id: &str) {
        if self.card_id() == Some(id) {
            self.close_card();
        }
        if self.notified == Some(id) {
            self.notified = None;
            self.application.withdraw_notification(HINT_NOTIFICATION_ID);
        }
        (self.calls.hide_card)(id);
    }

    fn open(&mut self, opens: &Opens) {
        match opens {
            Opens::Settings { page, .. } => {
                // The window reads the hints when it is built, and they are in use while this runs.
                let (calls, name) = (self.calls.clone(), hint_page_name(*page));
                glib::idle_add_local_once(move || (calls.open_page)(name));
            }
            Opens::Url(url) => {
                if let Err(error) = open_uri(url) {
                    logsetup::warning(LOGGER, &format!("Не удалось открыть {url}: {error}"));
                }
            }
        }
    }
}

/// The hints of a running program: what the engine reports and what the user does goes in here, and a timer asks what is
/// due for as long as a hint waits or is on screen.
pub struct HintDriver {
    hints: RefCell<Hints<PromptAnchor>>,
    screen: RefCell<HintScreen>,
    origin: Instant,
    engine: RefCell<Option<Arc<Shared>>>,
    timer: RefCell<Option<glib::SourceId>>,
}

impl HintDriver {
    /// `origin` is the instant `Moment`s are counted from, the one `hints` started with.
    #[must_use]
    pub fn new(hints: Hints<PromptAnchor>, origin: Instant, screen: HintScreen) -> Rc<HintDriver> {
        Rc::new(HintDriver { hints: RefCell::new(hints), screen: RefCell::new(screen), origin, engine: RefCell::new(None), timer: RefCell::new(None) })
    }

    fn moment(&self) -> Moment {
        let presses = self.engine.borrow().as_ref().map_or(0, |shared| shared.typed_presses.load(Ordering::Relaxed));
        Moment::now(self.origin, presses)
    }

    /// The timer runs while the hints have something to do, and not otherwise.
    fn run_timer(self: &Rc<Self>) {
        if !self.hints.borrow().needs_tick() || self.timer.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local(Duration::from_millis(HINT_TICK_INTERVAL_MS.into()), move || weak.upgrade().map_or(glib::ControlFlow::Break, |driver| driver.tick()));
        *self.timer.borrow_mut() = Some(source);
    }

    fn tick(&self) -> glib::ControlFlow {
        let moment = self.moment();
        self.hints.borrow_mut().tick(&mut *self.screen.borrow_mut(), &moment);
        if self.hints.borrow().needs_tick() {
            return glib::ControlFlow::Continue;
        }
        self.timer.borrow_mut().take();
        glib::ControlFlow::Break
    }

    /// The flag the engine's thread reads before it looks for the caret.
    #[must_use]
    pub fn caret_anchor_wanted(&self) -> Arc<AtomicBool> {
        self.hints.borrow().caret_anchor_wanted()
    }

    /// The engine exists: from now on its key presses tell whether the user is typing.
    pub fn engine_built(&self, shared: Arc<Shared>) {
        *self.engine.borrow_mut() = Some(shared);
    }

    pub fn input(self: &Rc<Self>, input: EngineInput<PromptAnchor>) {
        let moment = self.moment();
        self.hints.borrow_mut().engine_input(&mut *self.screen.borrow_mut(), input, &moment);
        self.run_timer();
    }

    pub fn clicked(self: &Rc<Self>, id: &str, click: Click) {
        let moment = self.moment();
        self.hints.borrow_mut().clicked(&mut *self.screen.borrow_mut(), id, click, &moment);
        self.run_timer();
    }

    pub fn settings_changed(self: &Rc<Self>) {
        let moment = self.moment();
        self.hints.borrow_mut().settings_changed(&mut *self.screen.borrow_mut(), &moment);
        self.run_timer();
    }

    pub fn settings_opened(&self) {
        self.hints.borrow_mut().settings_opened();
    }

    /// The card for the page `page` of the settings window, shown now.
    #[must_use]
    pub fn page_shown(&self, page: &str) -> Option<HintView> {
        let moment = self.moment();
        self.hints.borrow_mut().settings_card(hint_page(page)?, &moment)
    }

    /// The id of the card shown above the caret, for a test to wait on.
    #[must_use]
    pub fn card_id(&self) -> Option<&'static str> {
        self.screen.borrow().card_id()
    }

    /// What draws the hints, for a test to draw with.
    pub fn screen(&self) -> std::cell::RefMut<'_, HintScreen> {
        self.screen.borrow_mut()
    }

    /// The program ends: nothing stays on the screen.
    pub fn close(&self) {
        self.timer.borrow_mut().take().into_iter().for_each(glib::SourceId::remove);
        self.screen.borrow_mut().close_card();
    }
}

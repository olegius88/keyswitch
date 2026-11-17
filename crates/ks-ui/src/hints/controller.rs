//! What a platform wires: the engine's events in, a presenter out. The presenter only draws and reports clicks; the
//! rules, the file and the setting are kept here.
//!
//! A platform, on its main thread:
//! - calls `ks_config::installation::note_start` first thing, before the log is opened, and makes [`Hints::start`]
//!   right after its settings store, before anything writes a user file, so that a new installation is not taken for an
//!   update; a new installation gets its Shift keys there (`ks_config::installation::give_new_installation_keys`);
//! - says once whether the system takes both Shifts for itself ([`Hints::system_takes_both_shifts`]), where it can;
//! - subscribes with [`subscribe`] once the engine is built (`EngineLoader::when_built`), and hands each
//!   [`EngineInput`] it posts to [`Hints::engine_input`];
//! - calls [`Hints::settings_changed`] when a setting changes, [`Hints::settings_opened`] when its settings window
//!   opens, and [`Hints::settings_card`] when it builds a page;
//! - calls [`Hints::tick`] every `HINT_TICK_INTERVAL_MS` while [`Hints::needs_tick`];
//! - reports what the user did to a hint with [`Hints::clicked`].
//!
//! The presenter's methods never call back into [`Hints`]: a click arrives later, from the platform's event loop.

use ks_config::config::SettingsStore;
use ks_config::constants::{
    DEFAULT_BOTH_SHIFTS_TOGGLE, DEFAULT_DOUBLE_TAP_CONVERTS, DETECTION_BOTH_SHIFTS_TOGGLE, DETECTION_DOUBLE_TAP_CONVERTS, DETECTION_LAYOUT_SWITCH_KEY,
    HINTS_SETTING,
};
use ks_config::installation::{earlier_files_noted, give_new_installation_keys};
use ks_config::logsetup::{self, VERSION};
use ks_engine::{CorrectionPlan, EngineSnapshot, LearningPrompt, RememberedWord, Shared};
use ks_updates::SemanticVersion;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::catalogue::{CATALOGUE, CorrectionInfo, Hint, HotkeyName, Hotkeys, Opens, Place, Platform, SettingsPage, TapKeys};
use super::schedule::{HintSettings, HintView, Moment, Scheduler, Step};
use super::store::{HintState, HintStore, How, Loaded, Startup, startup_kind, user_files};
use crate::constants::{AUTO_SWITCH_NOTICE_SECONDS, AUTO_SWITCH_OFF_NOTICE, AUTO_SWITCH_ON_NOTICE};
use crate::windows_ui_model::truthy;

const LOGGER: &str = "keyswitch.hints";

/// The notice that both Shifts turned the automatic switching off or on: no hint of the catalogue. It shows each time, at
/// the caret (near the icon when there is none), also with the hints turned off, for `AUTO_SWITCH_NOTICE_SECONDS`, until
/// its «Больше не показывать».
pub const AUTO_SWITCH_NOTICE_ID: &str = "auto_switch";

/// What draws the hints of a platform. None of these takes the focus or a key from the field typed in.
pub trait HintPresenter<A> {
    /// A card above the caret at `anchor`. True when it is on screen, or will be after the platform's short show delay.
    fn show_caret(&mut self, view: &HintView, anchor: &A) -> bool;
    /// Near the tray icon or the menu bar item, or as a notification of the system.
    fn show_tray(&mut self, view: &HintView) -> bool;
    /// Take down whatever shows the hint `id`: a card, a notification, a card of the settings window.
    fn hide(&mut self, id: &str);
    /// What a click on the text of a hint leads to: the settings window on a page, or a page of the web.
    fn open(&mut self, opens: &Opens);
}

/// What the user did to a hint on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    /// «Понятно».
    Ok,
    /// «Больше не показывать подсказки»; «Отменить» of the notice of a remembered word.
    Never,
    /// The text of the hint.
    Text,
    /// The window manager or the system closed it.
    Closed,
}

/// What the engine's thread hands to the main thread.
#[derive(Clone, Debug, PartialEq)]
pub enum EngineInput<A> {
    Snapshot { running: bool },
    Correction { correction: CorrectionInfo, anchor: Option<A> },
    Prompt { shown: bool },
    /// A word left as typed only because its layout was picked by hand, where the model would have converted it.
    KeptByChoice { anchor: Option<A> },
    /// Both Shifts turned the automatic switching off or on.
    AutoSwitch { enabled: bool, anchor: Option<A> },
    /// A word remembered as a rule after it was erased and typed again as first typed.
    Remembered { word: RememberedWord, anchor: Option<A> },
}

/// The engine's subscriptions the hints need, made once the engine is built. On the engine's thread the caret is looked
/// for with `anchor`: at a correction only while `wants_anchor` says a hint at the caret may still show
/// (`Hints::caret_anchor_wanted`), at a word kept for a layout picked by hand and at a word remembered always, which is
/// seldom. `post` carries each input to the main thread.
pub fn subscribe<A: Send + 'static>(
    shared: &Shared, wants_anchor: Arc<AtomicBool>, anchor: Arc<dyn Fn() -> Option<A> + Send + Sync>,
    post: Arc<dyn Fn(EngineInput<A>) + Send + Sync>,
) {
    let deliver = post.clone();
    shared.subscribe(Arc::new(move |snapshot: &EngineSnapshot| deliver(EngineInput::Snapshot { running: snapshot.running })));
    let deliver = post.clone();
    let find = anchor.clone();
    shared.subscribe_corrections(Arc::new(move |plan: &CorrectionPlan| {
        let found = if wants_anchor.load(Ordering::Relaxed) { find() } else { None };
        deliver(EngineInput::Correction { correction: CorrectionInfo::of(plan), anchor: found });
    }));
    let deliver = post.clone();
    let find = anchor.clone();
    shared.subscribe_kept_by_choice(Arc::new(move || deliver(EngineInput::KeptByChoice { anchor: find() })));
    let deliver = post.clone();
    let find = anchor.clone();
    shared.subscribe_auto_switch(Arc::new(move |enabled: bool| deliver(EngineInput::AutoSwitch { enabled, anchor: find() })));
    let deliver = post.clone();
    shared.subscribe_remembered(Arc::new(move |word: &RememberedWord| deliver(EngineInput::Remembered { word: word.clone(), anchor: anchor() })));
    shared.subscribe_learning_prompts(Arc::new(move |prompt: Option<&LearningPrompt>| post(EngineInput::Prompt { shown: prompt.is_some() })));
}

fn flag(settings: &SettingsStore, path: &str) -> bool {
    settings.get(path).or_else(|| settings.default(path)).is_some_and(|value| truthy(&value))
}

fn hotkey(settings: &SettingsStore, name: HotkeyName) -> String {
    settings.get(&format!("hotkeys.{}", name.setting())).and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_default()
}

impl HintSettings {
    /// What the hints follow, read from the settings.
    #[must_use]
    pub fn read(settings: &SettingsStore) -> HintSettings {
        HintSettings {
            enabled: flag(settings, HINTS_SETTING),
            notifications: flag(settings, "general.notifications"),
            keep_history: flag(settings, "general.keep_history"),
            hotkeys: Hotkeys {
                undo: hotkey(settings, HotkeyName::Undo),
                convert_last: hotkey(settings, HotkeyName::ConvertLast),
                selection_layout: hotkey(settings, HotkeyName::SelectionLayout),
                selection_case: hotkey(settings, HotkeyName::SelectionCase),
                selection_transliterate: hotkey(settings, HotkeyName::SelectionTransliterate),
                tap: TapKeys {
                    switch: settings.read(DETECTION_LAYOUT_SWITCH_KEY).unwrap_or_default(),
                    double_tap: settings.read(DETECTION_DOUBLE_TAP_CONVERTS).unwrap_or(DEFAULT_DOUBLE_TAP_CONVERTS),
                    both_shifts: settings.read(DETECTION_BOTH_SHIFTS_TOGGLE).unwrap_or(DEFAULT_BOTH_SHIFTS_TOGGLE),
                },
            },
        }
    }
}

/// The version running.
#[must_use]
pub fn running_version() -> SemanticVersion {
    SemanticVersion::parse(VERSION).expect("the package version is three numbers")
}

fn place_name(place: Place) -> &'static str {
    match place {
        Place::Caret => "caret",
        Place::Tray => "tray",
        Place::Settings { .. } => "settings",
    }
}

/// The hints of one run of the program.
pub struct Hints<A> {
    scheduler: Scheduler<A>,
    store: HintStore,
    settings: Arc<SettingsStore>,
    wants_anchor: Arc<AtomicBool>,
    startup: Startup,
    /// Whether the system answers both Shifts itself: then no hint names them.
    both_shifts_taken: bool,
    /// When the notice of both Shifts goes, while it shows.
    notice_until: Option<f64>,
}

impl<A: Clone> Hints<A> {
    /// The hints of the catalogue, kept in `hints.json` of `data_dir`. What the program found in its folders at its very
    /// start (`ks_config::installation::note_start`) counts too: the log it has written since is no file of an earlier
    /// start.
    #[must_use]
    pub fn start(settings: Arc<SettingsStore>, data_dir: &Path, platform: Platform, moment: &Moment) -> Hints<A> {
        Hints::begin(&CATALOGUE, settings, data_dir, platform, running_version(), moment, earlier_files_noted().unwrap_or(false))
    }

    /// The same over any catalogue and version, for tests.
    #[must_use]
    pub fn with_catalogue(
        catalogue: &'static [Hint], settings: Arc<SettingsStore>, data_dir: &Path, platform: Platform, version: SemanticVersion,
        moment: &Moment,
    ) -> Hints<A> {
        Hints::begin(catalogue, settings, data_dir, platform, version, moment, false)
    }

    /// A new installation, the first start of which is told apart here, gets the Shift keys before its settings are read.
    fn begin(
        catalogue: &'static [Hint], settings: Arc<SettingsStore>, data_dir: &Path, platform: Platform, version: SemanticVersion,
        moment: &Moment, noted: bool,
    ) -> Hints<A> {
        let store = HintStore::in_folder(data_dir);
        // A file of the hints that cannot be read was still left by a version that had them.
        let older_files = noted || store.path.exists() || user_files(&settings.path, data_dir).iter().any(|path| path.exists());
        let (mut state, recorded) = match store.load() {
            Loaded::Read(state) => (state, true),
            Loaded::Missing | Loaded::SetAside(_) => (HintState::default(), false),
        };
        let startup = startup_kind(version, recorded.then_some(&state), older_files);
        if startup == Startup::Fresh {
            match give_new_installation_keys(&settings) {
                Ok(()) => logsetup::info(LOGGER, "new installation: the Shift keys are on"),
                // The keys hold for this run; the next start is no new installation and keeps the defaults.
                Err(error) => logsetup::warning(LOGGER, &format!("Не удалось сохранить настройки новой установки в {}: {error}", settings.path.display())),
            }
        }
        state.begin(startup, version, &moment.timestamp);
        let scheduler = Scheduler::new(catalogue, platform, version, HintSettings::read(&settings), state, moment);
        let mut hints =
            Hints { scheduler, store, settings, wants_anchor: Arc::new(AtomicBool::new(false)), startup, both_shifts_taken: false, notice_until: None };
        hints.write();
        hints.sync();
        hints
    }

    /// The settings the hints follow, with both Shifts left out where the system takes them.
    fn read_settings(&self) -> HintSettings {
        let mut read = HintSettings::read(&self.settings);
        read.hotkeys.tap.both_shifts &= !self.both_shifts_taken;
        read
    }

    /// Whether the system answers both Shifts itself (`InputBackend::system_takes_both_shifts`): the hints then do not
    /// name them.
    pub fn system_takes_both_shifts(&mut self, taken: bool) {
        self.both_shifts_taken = taken;
        // The hints stay on: nothing is taken down.
        let _ = self.scheduler.set_settings(self.read_settings());
    }

    /// What kind of start this was.
    #[must_use]
    pub fn startup(&self) -> Startup {
        self.startup
    }

    #[must_use]
    pub fn state(&self) -> &HintState {
        self.scheduler.state()
    }

    /// The flag the engine's thread reads before it looks for the caret (`subscribe`).
    #[must_use]
    pub fn caret_anchor_wanted(&self) -> Arc<AtomicBool> {
        self.wants_anchor.clone()
    }

    #[must_use]
    pub fn needs_tick(&self) -> bool {
        self.scheduler.needs_tick() || self.notice_until.is_some()
    }

    fn write(&self) {
        if let Err(error) = self.store.save(self.scheduler.state()) {
            // The hints go on from memory; the next change tries again.
            logsetup::warning(LOGGER, &format!("Подсказки не записаны в {}: {error}", self.store.path.display()));
        }
    }

    /// The file, when the state changed, and the flag of the engine's thread.
    fn sync(&mut self) {
        if self.scheduler.take_dirty() {
            self.write();
        }
        self.wants_anchor.store(self.scheduler.wants_caret_anchor(), Ordering::Relaxed);
    }

    fn apply(&mut self, presenter: &mut dyn HintPresenter<A>, step: Option<Step<A>>, moment: &Moment) {
        match step {
            None => {}
            Some(Step::Hide(id)) => presenter.hide(id),
            Some(Step::Caret(view, anchor)) => {
                let shown = presenter.show_caret(&view, &anchor);
                self.reported(&view, shown, moment);
            }
            Some(Step::Tray(view)) => {
                let shown = presenter.show_tray(&view);
                self.reported(&view, shown, moment);
            }
        }
    }

    fn reported(&mut self, view: &HintView, shown: bool, moment: &Moment) {
        if shown {
            logsetup::info(LOGGER, &format!("shown {} at {}", view.id, place_name(view.place)));
            self.scheduler.drawn(view, moment);
        } else {
            self.scheduler.not_drawn(view, moment);
        }
    }

    /// What is due now, then the file and the flag of the engine's thread.
    fn settle(&mut self, presenter: &mut dyn HintPresenter<A>, step: Option<Step<A>>, moment: &Moment) {
        self.apply(presenter, step, moment);
        let next = self.scheduler.tick(moment);
        self.apply(presenter, next, moment);
        self.sync();
    }

    /// An input `subscribe` posted.
    pub fn engine_input(&mut self, presenter: &mut dyn HintPresenter<A>, input: EngineInput<A>, moment: &Moment) {
        let step = match input {
            EngineInput::Snapshot { running } => {
                if running {
                    self.scheduler.engine_running(moment);
                }
                None
            }
            EngineInput::Correction { correction, anchor } => {
                self.scheduler.correction(correction, anchor, moment);
                None
            }
            EngineInput::Prompt { shown } => self.scheduler.prompt(shown, moment),
            EngineInput::KeptByChoice { anchor } => {
                self.scheduler.kept_by_choice(anchor, moment);
                None
            }
            EngineInput::AutoSwitch { enabled, anchor } => {
                self.auto_switch(presenter, enabled, anchor.as_ref(), moment);
                None
            }
            EngineInput::Remembered { word, anchor } => {
                self.scheduler.remembered(word, anchor, moment);
                None
            }
        };
        self.settle(presenter, step, moment);
    }

    /// Both Shifts turned the automatic switching off or on: a short notice says so at the caret, or near the icon where no
    /// caret was found, unless it was turned off with its «Больше не показывать».
    fn auto_switch(&mut self, presenter: &mut dyn HintPresenter<A>, enabled: bool, anchor: Option<&A>, moment: &Moment) {
        if self.scheduler.state().how(AUTO_SWITCH_NOTICE_ID).as_deref() == Some(How::Forever.name()) {
            return;
        }
        let text = if enabled { AUTO_SWITCH_ON_NOTICE } else { AUTO_SWITCH_OFF_NOTICE };
        let shown = match anchor {
            Some(anchor) => presenter.show_caret(&HintView::notice(AUTO_SWITCH_NOTICE_ID, Place::Caret, text), anchor),
            None => presenter.show_tray(&HintView::notice(AUTO_SWITCH_NOTICE_ID, Place::Tray, text)),
        };
        self.notice_until = shown.then_some(moment.seconds + AUTO_SWITCH_NOTICE_SECONDS);
    }

    /// Called every `HINT_TICK_INTERVAL_MS` while `needs_tick`.
    pub fn tick(&mut self, presenter: &mut dyn HintPresenter<A>, moment: &Moment) {
        if self.notice_until.is_some_and(|until| moment.seconds >= until) {
            self.notice_until = None;
            presenter.hide(AUTO_SWITCH_NOTICE_ID);
        }
        self.settle(presenter, None, moment);
    }

    /// A setting changed: the hints read theirs again.
    pub fn settings_changed(&mut self, presenter: &mut dyn HintPresenter<A>, moment: &Moment) {
        let step = self.scheduler.set_settings(self.read_settings());
        self.settle(presenter, step, moment);
    }

    /// The settings window opened.
    pub fn settings_opened(&mut self) {
        self.scheduler.settings_opened();
    }

    /// The card to put on top of `page`, which the settings window is building, with `control` marked; it counts as
    /// shown.
    pub fn settings_card(&mut self, page: SettingsPage, moment: &Moment) -> Option<HintView> {
        let card = self.scheduler.settings_card(page, moment);
        if let Some(card) = &card {
            logsetup::info(LOGGER, &format!("shown {} at {}", card.id, place_name(card.place)));
        }
        self.sync();
        card
    }

    /// What the user did to the hint `id`: it goes away, and «Больше не показывать подсказки» turns `general.hints` off
    /// («Больше не показывать» of a hint that repeats turns off that hint alone; «Отменить» of the notice of a remembered
    /// word takes the rule back).
    pub fn clicked(&mut self, presenter: &mut dyn HintPresenter<A>, id: &str, click: Click, moment: &Moment) {
        let how = match click {
            Click::Ok | Click::Closed => How::Ok,
            Click::Never => How::Forever,
            Click::Text => How::Click,
        };
        presenter.hide(id);
        if id == AUTO_SWITCH_NOTICE_ID {
            self.notice_until = None;
            if click == Click::Never {
                self.scheduler.notice_turned_off(id, moment);
            }
            self.settle(presenter, None, moment);
            return;
        }
        if click == Click::Never
            && let Some(word) = self.scheduler.notice_word(id)
        {
            let removed = word.forget();
            logsetup::info(LOGGER, &format!("{id}: the rule taken back, removed {removed}"));
        }
        if click == Click::Text
            && let Some(opens) = self.scheduler.opens_of(id)
        {
            presenter.open(&opens);
        }
        if self.scheduler.dismissed(id, how) {
            // The setting is the one the settings window shows, so the hints come back where they were turned off.
            if let Err(error) = self.settings.set(HINTS_SETTING, Value::Bool(false), true) {
                logsetup::warning(LOGGER, &format!("Не удалось сохранить настройку {HINTS_SETTING}: {error}"));
            }
        }
        self.settle(presenter, None, moment);
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::hints::catalogue::{HINT_UNDO_REMEMBERED_LABEL, REMEMBERED_HINT_ID};
    use ks_backend::{BackendError, BackendProbe, FocusInfo, InputBackend, Injection, KeyFilter, KeyListener};
    use ks_config::learning::{LearnedRule, LearningStore, RuleAction};
    use std::path::PathBuf;
    use std::sync::Mutex;

    type Anchor = (i32, i32);
    const AT: Anchor = (100, 200);
    const LAYOUT: CorrectionInfo = CorrectionInfo { mode: "boundary", automatic: true, case_fix: false };

    fn version(text: &str) -> SemanticVersion {
        SemanticVersion::parse(text).unwrap()
    }

    fn at(seconds: f64) -> Moment {
        Moment { seconds, timestamp: "2026-11-07T10:00:00+00:00".into(), typed_presses: 0 }
    }

    fn folder(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("ks-hints-controller-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    /// A presenter that writes down what it was asked to do.
    struct Screen {
        log: Vec<String>,
        draws: bool,
        /// The text and the second button of every card drawn.
        cards: Vec<(String, &'static str)>,
    }

    impl Screen {
        fn new(draws: bool) -> Screen {
            Screen { log: Vec::new(), draws, cards: Vec::new() }
        }
    }

    impl HintPresenter<Anchor> for Screen {
        fn show_caret(&mut self, view: &HintView, anchor: &Anchor) -> bool {
            self.log.push(format!("caret {} at {anchor:?}", view.id));
            self.cards.push((view.text.clone(), view.never));
            self.draws
        }

        fn show_tray(&mut self, view: &HintView) -> bool {
            self.log.push(format!("tray {}", view.id));
            self.cards.push((view.text.clone(), view.never));
            self.draws
        }

        fn hide(&mut self, id: &str) {
            self.log.push(format!("hide {id}"));
        }

        fn open(&mut self, opens: &Opens) {
            self.log.push(format!("open {opens:?}"));
        }
    }

    struct Setup {
        settings: Arc<SettingsStore>,
        data: PathBuf,
    }

    fn setup(name: &str) -> Setup {
        let root = folder(name);
        Setup { settings: Arc::new(SettingsStore::new(Some(root.join("config.json")))), data: root.join("data") }
    }

    fn hints(setup: &Setup) -> Hints<Anchor> {
        Hints::with_catalogue(&CATALOGUE, setup.settings.clone(), &setup.data, Platform::Linux, version("0.71.0"), &at(0.0))
    }

    fn file(setup: &Setup) -> String {
        std::fs::read_to_string(setup.data.join("hints.json")).unwrap()
    }

    #[test]
    fn a_new_installation_writes_its_file_and_is_told_where_the_program_is() {
        let setup = setup("fresh");
        let mut hints = Hints::<Anchor>::start(setup.settings.clone(), &setup.data, Platform::Linux, &at(0.0));
        assert_eq!(hints.startup(), Startup::Fresh);
        assert!(file(&setup).contains(&format!(r#""last_run_version": "{VERSION}""#)) && file(&setup).contains(r#""fresh_install": true"#));
        assert!(hints.caret_anchor_wanted().load(Ordering::Relaxed));
        let mut screen = Screen::new(true);
        hints.engine_input(&mut screen, EngineInput::Snapshot { running: false }, &at(1.0));
        assert!(!hints.needs_tick());
        hints.engine_input(&mut screen, EngineInput::Snapshot { running: true }, &at(1.0));
        assert!(hints.needs_tick());
        // The keys of a new installation first, then where the program is.
        hints.tick(&mut screen, &at(6.0));
        assert_eq!(screen.log, ["tray shift_keys"]);
        hints.clicked(&mut screen, "shift_keys", Click::Closed, &at(7.0));
        hints.tick(&mut screen, &at(700.0));
        assert_eq!(screen.log, ["tray shift_keys", "hide shift_keys", "tray first_start"]);
        hints.clicked(&mut screen, "first_start", Click::Closed, &at(701.0));
        assert_eq!(screen.log.last().unwrap(), "hide first_start");
        assert_eq!(hints.state().how("first_start").as_deref(), Some("ok"));
        assert!(file(&setup).contains(r#""how":"ok""#), "{}", file(&setup));
    }

    /// The settings of the keys tapped alone, as they read from `settings`.
    fn tap_keys(settings: &SettingsStore) -> (String, bool, bool) {
        let read = HintSettings::read(settings).hotkeys.tap;
        (read.switch, read.double_tap, read.both_shifts)
    }

    const EARLIER_SETTINGS: &str = r#"{"schema_version": 8, "detection": {"layout_switch_key": "", "convert_last_key": "", "minimum_length": 3}}"#;

    #[test]
    fn a_new_installation_gets_the_shift_keys_and_one_that_ran_before_keeps_its_settings() {
        // Nothing in the folders: the keys are on, and written into the settings file.
        let fresh = setup("keys-fresh");
        assert_eq!(hints(&fresh).startup(), Startup::Fresh);
        assert_eq!(tap_keys(&fresh.settings), ("Shift".to_owned(), true, true));
        let written = SettingsStore::new(Some(fresh.settings.path.clone()));
        assert_eq!(tap_keys(&written), ("Shift".to_owned(), true, true));
        // A folder of 0.73.0 with its settings file: an update, and the file stays as it was.
        let with_file = setup("keys-0-73-file");
        std::fs::write(&with_file.settings.path, EARLIER_SETTINGS).unwrap();
        std::fs::create_dir_all(&with_file.data).unwrap();
        std::fs::write(with_file.data.join("hints.json"), r#"{"last_run_version": "0.73.0"}"#).unwrap();
        with_file.settings.load();
        let later = |setup: &Setup| Hints::<Anchor>::with_catalogue(&CATALOGUE, setup.settings.clone(), &setup.data, Platform::Linux, version("0.74.0"), &at(0.0));
        assert_eq!(later(&with_file).startup(), Startup::Updated { from: Some(version("0.73.0")) });
        assert_eq!(tap_keys(&with_file.settings), (String::new(), false, false));
        assert_eq!(std::fs::read_to_string(&with_file.settings.path).unwrap(), EARLIER_SETTINGS);
        // A folder of 0.73.0 whose user never changed a setting: no settings file, and none is written.
        let without_file = setup("keys-0-73-none");
        std::fs::create_dir_all(&without_file.data).unwrap();
        std::fs::write(without_file.data.join("hints.json"), r#"{"last_run_version": "0.73.0"}"#).unwrap();
        assert_eq!(later(&without_file).startup(), Startup::Updated { from: Some(version("0.73.0")) });
        assert_eq!(tap_keys(&without_file.settings), (String::new(), false, false));
        assert!(!without_file.settings.path.exists());
        // A version before the hints that left only its log (0.66, 0.70): the start found it, so no new installation.
        let only_log = setup("keys-log");
        let mut started = Hints::<Anchor>::begin(&CATALOGUE, only_log.settings.clone(), &only_log.data, Platform::Linux, version("0.74.0"), &at(0.0), true);
        assert_eq!(started.startup(), Startup::Updated { from: None });
        assert_eq!(tap_keys(&only_log.settings), (String::new(), false, false));
        assert!(!only_log.settings.path.exists());
        // An installation without the keys hears nothing of them.
        let mut screen = Screen::new(true);
        started.engine_input(&mut screen, EngineInput::Snapshot { running: true }, &at(1.0));
        started.tick(&mut screen, &at(10.0));
        assert!(screen.log.is_empty(), "{:?}", screen.log);
    }

    #[test]
    fn both_shifts_the_system_takes_are_not_named() {
        let setup = setup("keys-taken");
        let mut hints = hints(&setup);
        assert!(hints.scheduler.settings().hotkeys.tap.both_shifts);
        hints.system_takes_both_shifts(true);
        assert!(!hints.scheduler.settings().hotkeys.tap.both_shifts);
        let mut screen = Screen::new(true);
        hints.settings_changed(&mut screen, &at(1.0));
        assert!(!hints.scheduler.settings().hotkeys.tap.both_shifts);
        hints.system_takes_both_shifts(false);
        assert!(hints.scheduler.settings().hotkeys.tap.both_shifts);
    }

    #[test]
    fn both_shifts_are_told_at_the_caret_for_a_moment() {
        let setup = setup("notice");
        let mut hints = hints(&setup);
        // Shown with the hints turned off too: it reports what the keys did.
        setup.settings.set(HINTS_SETTING, Value::Bool(false), true).unwrap();
        let mut screen = Screen::new(true);
        hints.settings_changed(&mut screen, &at(0.0));
        hints.engine_input(&mut screen, EngineInput::AutoSwitch { enabled: false, anchor: Some(AT) }, &at(1.0));
        assert_eq!(screen.log, ["caret auto_switch at (100, 200)"]);
        assert!(hints.needs_tick());
        hints.tick(&mut screen, &at(2.0));
        assert_eq!(screen.log.len(), 1);
        hints.tick(&mut screen, &at(3.0));
        assert_eq!(screen.log, ["caret auto_switch at (100, 200)", "hide auto_switch"]);
        assert!(!hints.needs_tick());
        // No caret found: near the icon. «Понятно» takes it down.
        hints.engine_input(&mut screen, EngineInput::AutoSwitch { enabled: true, anchor: None }, &at(4.0));
        assert_eq!(screen.log.last().unwrap(), "tray auto_switch");
        hints.clicked(&mut screen, AUTO_SWITCH_NOTICE_ID, Click::Ok, &at(4.5));
        assert_eq!((screen.log.last().unwrap().as_str(), hints.needs_tick()), ("hide auto_switch", false));
        // A presenter that could not draw it: nothing to take down.
        let mut blind = Screen::new(false);
        hints.engine_input(&mut blind, EngineInput::AutoSwitch { enabled: true, anchor: Some(AT) }, &at(5.0));
        assert!(!hints.needs_tick());
        // «Больше не показывать» turns this notice off, and only it.
        hints.engine_input(&mut screen, EngineInput::AutoSwitch { enabled: false, anchor: Some(AT) }, &at(6.0));
        hints.clicked(&mut screen, AUTO_SWITCH_NOTICE_ID, Click::Never, &at(6.5));
        assert_eq!(hints.state().how(AUTO_SWITCH_NOTICE_ID).as_deref(), Some("forever"));
        assert!(file(&setup).contains(AUTO_SWITCH_NOTICE_ID));
        let shown = screen.log.len();
        hints.engine_input(&mut screen, EngineInput::AutoSwitch { enabled: true, anchor: Some(AT) }, &at(7.0));
        assert_eq!(screen.log.len(), shown);
        assert_eq!(setup.settings.get(HINTS_SETTING), Some(Value::Bool(false)));
    }

    #[test]
    fn the_notice_says_off_or_on() {
        struct Texts(Vec<String>);
        impl HintPresenter<Anchor> for Texts {
            fn show_caret(&mut self, view: &HintView, _anchor: &Anchor) -> bool {
                self.0.push(format!("{} | {} | {}", view.text, view.ok, view.never));
                true
            }
            fn show_tray(&mut self, view: &HintView) -> bool {
                self.0.push(view.text.clone());
                true
            }
            fn hide(&mut self, _id: &str) {}
            fn open(&mut self, _opens: &Opens) {}
        }
        let setup = setup("notice-texts");
        let mut hints = hints(&setup);
        let mut texts = Texts(Vec::new());
        hints.engine_input(&mut texts, EngineInput::AutoSwitch { enabled: false, anchor: Some(AT) }, &at(1.0));
        hints.engine_input(&mut texts, EngineInput::AutoSwitch { enabled: true, anchor: None }, &at(2.0));
        assert_eq!(texts.0, ["Автопереключение выключено | Понятно | Больше не показывать", "Автопереключение включено"]);
    }

    #[test]
    fn a_start_is_told_apart_by_the_files_left() {
        let settings_file = setup("settings-file");
        settings_file.settings.save().unwrap();
        assert_eq!(hints(&settings_file).startup(), Startup::Updated { from: None });
        let earlier = setup("earlier");
        std::fs::create_dir_all(&earlier.data).unwrap();
        std::fs::write(earlier.data.join("hints.json"), r#"{"last_run_version": "0.70.0"}"#).unwrap();
        assert_eq!(hints(&earlier).startup(), Startup::Updated { from: Some(version("0.70.0")) });
        assert_eq!(hints(&earlier).startup(), Startup::Same);
        let garbage = setup("garbage");
        std::fs::create_dir_all(&garbage.data).unwrap();
        std::fs::write(garbage.data.join("hints.json"), "garbage").unwrap();
        assert_eq!(hints(&garbage).startup(), Startup::Updated { from: None });
        let history = setup("history");
        std::fs::create_dir_all(&history.data).unwrap();
        std::fs::write(history.data.join("history.jsonl"), "").unwrap();
        assert_eq!(hints(&history).startup(), Startup::Updated { from: None });
    }

    #[test]
    fn a_correction_brings_its_hint_at_the_caret_and_a_rule_prompt_takes_it_away() {
        let setup = setup("caret");
        let mut hints = hints(&setup);
        let mut screen = Screen::new(true);
        hints.engine_input(&mut screen, EngineInput::Correction { correction: LAYOUT, anchor: Some(AT) }, &at(0.0));
        assert!(screen.log.is_empty() && hints.needs_tick());
        hints.tick(&mut screen, &at(1.0));
        assert_eq!(screen.log, ["caret first_correction_undo at (100, 200)"]);
        hints.engine_input(&mut screen, EngineInput::Prompt { shown: true }, &at(2.0));
        assert_eq!(screen.log.last().unwrap(), "hide first_correction_undo");
        assert!(file(&setup).contains("first_correction_undo"));
    }

    #[test]
    fn a_click_on_the_text_opens_its_page_and_never_again_turns_the_setting_off() {
        let setup = setup("clicks");
        let mut hints = hints(&setup);
        let mut screen = Screen::new(true);
        hints.engine_input(&mut screen, EngineInput::Correction { correction: LAYOUT, anchor: Some(AT) }, &at(0.0));
        hints.tick(&mut screen, &at(1.0));
        hints.clicked(&mut screen, "first_correction_undo", Click::Text, &at(2.0));
        assert_eq!(
            screen.log[1..],
            ["hide first_correction_undo", r#"open Settings { page: Hotkeys, setting: Some("hotkeys.undo") }"#]
        );
        assert_eq!(hints.state().how("first_correction_undo").as_deref(), Some("click"));
        let early = CorrectionInfo { mode: "early", automatic: true, case_fix: false };
        hints.engine_input(&mut screen, EngineInput::Correction { correction: early, anchor: Some(AT) }, &at(700.0));
        hints.tick(&mut screen, &at(701.0));
        assert_eq!(screen.log.last().unwrap(), "caret early_switch at (100, 200)");
        hints.clicked(&mut screen, "early_switch", Click::Never, &at(702.0));
        assert_eq!(setup.settings.get(HINTS_SETTING), Some(Value::Bool(false)));
        assert!(!hints.caret_anchor_wanted().load(Ordering::Relaxed));
        assert_eq!(hints.state().how("early_switch").as_deref(), Some("forever"));
        // A text with nothing behind it opens nothing.
        hints.clicked(&mut screen, "first_start", Click::Text, &at(703.0));
        hints.clicked(&mut screen, "first_start", Click::Ok, &at(704.0));
        assert_eq!(screen.log[screen.log.len() - 2..], ["hide first_start", "hide first_start"]);
    }

    #[test]
    fn a_word_kept_for_a_layout_picked_by_hand_shows_at_the_caret_and_never_again_keeps_the_other_hints() {
        let setup = setup("kept");
        let mut hints = hints(&setup);
        let mut screen = Screen::new(true);
        hints.engine_input(&mut screen, EngineInput::KeptByChoice { anchor: Some(AT) }, &at(0.0));
        hints.tick(&mut screen, &at(1.0));
        assert_eq!(screen.log, ["caret kept_by_choice at (100, 200)"]);
        hints.clicked(&mut screen, "kept_by_choice", Click::Never, &at(2.0));
        assert_ne!(setup.settings.get(HINTS_SETTING), Some(Value::Bool(false)));
        assert_eq!(hints.state().how("kept_by_choice").as_deref(), Some("forever"));
        assert!(hints.caret_anchor_wanted().load(Ordering::Relaxed), "the other hints at the caret still show");
    }

    #[test]
    fn a_hint_the_presenter_could_not_draw_stays_unseen() {
        let setup = setup("undrawn");
        let mut hints = hints(&setup);
        let mut screen = Screen::new(false);
        hints.engine_input(&mut screen, EngineInput::Correction { correction: LAYOUT, anchor: Some(AT) }, &at(0.0));
        hints.tick(&mut screen, &at(1.0));
        assert_eq!(screen.log, ["caret first_correction_undo at (100, 200)"]);
        assert!(!hints.state().is_seen("first_correction_undo"));
    }

    #[test]
    fn the_hints_follow_their_setting_and_give_cards_to_the_settings_window() {
        let setup = setup("setting");
        let mut hints = hints(&setup);
        let mut screen = Screen::new(true);
        hints.engine_input(&mut screen, EngineInput::Correction { correction: LAYOUT, anchor: Some(AT) }, &at(0.0));
        hints.tick(&mut screen, &at(1.0));
        setup.settings.set(HINTS_SETTING, Value::Bool(false), true).unwrap();
        hints.settings_changed(&mut screen, &at(2.0));
        assert_eq!(screen.log.last().unwrap(), "hide first_correction_undo");
        hints.settings_opened();
        assert_eq!(hints.settings_card(SettingsPage::Overview, &at(3.0)), None);
        setup.settings.set(HINTS_SETTING, Value::Bool(true), true).unwrap();
        hints.settings_changed(&mut screen, &at(4.0));
        let card = hints.settings_card(SettingsPage::Overview, &at(5.0)).unwrap();
        assert_eq!(card.id, "hints_intro");
        assert!(file(&setup).contains("hints_intro"));
    }

    #[test]
    fn a_file_or_a_setting_that_cannot_be_written_leaves_the_hints_in_memory() {
        let root = folder("unwritable");
        std::fs::write(root.join("data"), "").unwrap();
        std::fs::write(root.join("settings"), "").unwrap();
        let settings = Arc::new(SettingsStore::new(Some(root.join("settings").join("config.json"))));
        let mut hints = Hints::<Anchor>::with_catalogue(&CATALOGUE, settings.clone(), &root.join("data"), Platform::Linux, version("0.71.0"), &at(0.0));
        assert_eq!(hints.startup(), Startup::Fresh);
        let mut screen = Screen::new(true);
        hints.engine_input(&mut screen, EngineInput::Snapshot { running: true }, &at(0.0));
        hints.tick(&mut screen, &at(5.0));
        // The keys of the new installation hold in memory, so the hint of them shows.
        assert_eq!(screen.log, ["tray shift_keys"]);
        hints.clicked(&mut screen, "shift_keys", Click::Never, &at(6.0));
        assert_eq!(settings.get(HINTS_SETTING), Some(Value::Bool(false)));
        assert_eq!(hints.state().how("shift_keys").as_deref(), Some("forever"));
    }

    #[test]
    fn the_settings_the_hints_follow_fall_back_to_the_defaults() {
        let root = folder("settings");
        std::fs::write(root.join("config.json"), r#"{"general": "x", "hotkeys": {"undo": 5}}"#).unwrap();
        let settings = SettingsStore::new(Some(root.join("config.json")));
        let read = HintSettings::read(&settings);
        assert!(read.enabled && read.notifications && read.keep_history);
        let shipped = if cfg!(target_os = "macos") { "Ctrl+Alt+K" } else { "Pause" };
        assert_eq!((read.hotkeys.undo.as_str(), read.hotkeys.convert_last.as_str()), ("", shipped));
        assert_eq!(running_version().to_string(), VERSION);
    }

    struct Minimal;

    impl InputBackend for Minimal {
        fn start(&self, _listener: KeyListener) -> Result<(), BackendError> {
            Ok(())
        }
        fn set_key_filter(&self, _predicate: Option<KeyFilter>) {}
        fn hold_input(&self) {}
        fn release_input(&self) -> Result<i32, BackendError> {
            Ok(0)
        }
        fn complete_action(&self, _deliver: bool) -> Result<i32, BackendError> {
            Ok(0)
        }
        fn stop(&self) {}
        fn close(&self) {}
        fn current_group(&self) -> Result<i32, BackendError> {
            Ok(0)
        }
        fn switch_group(&self, _group: i32) -> Result<(), BackendError> {
            Ok(())
        }
        fn active_application(&self) -> String {
            String::new()
        }
        fn focused_window(&self) -> Option<FocusInfo> {
            None
        }
        fn probe(&self) -> BackendProbe {
            BackendProbe::default()
        }
        fn inject_correction(&self, _injection: Injection<'_>) -> Result<i32, BackendError> {
            Ok(0)
        }
    }

    fn engine_of(root: &Path) -> Arc<Shared> {
        let settings = Arc::new(SettingsStore::new(Some(root.join("config.json"))));
        Shared::new(settings, Arc::new(Minimal), Arc::new(|| 0.0), 2, EngineSnapshot::default())
    }

    /// `pattern` remembered as a keep rule of the Russian layout, in a store of its own.
    fn remembered(root: &Path, shared: &Arc<Shared>, pattern: &str) -> (RememberedWord, Arc<LearningStore>) {
        let learning = Arc::new(LearningStore::new(Some(root.join(format!("learning-{pattern}.json"))), None).unwrap());
        let rule = learning.add_rule(LearnedRule { action: RuleAction::Keep, ..LearnedRule::new(pattern, 1, 0) }).unwrap();
        (RememberedWord::new(rule, learning.clone(), shared.clone()), learning)
    }

    fn told(word: &RememberedWord, anchor: Option<Anchor>) -> EngineInput<Anchor> {
        EngineInput::Remembered { word: word.clone(), anchor }
    }

    #[test]
    fn a_remembered_word_is_told_at_the_caret_and_its_undo_takes_the_rule_back() {
        let setup = setup("remembered");
        let shared = engine_of(&setup.data);
        let mut hints = hints(&setup);
        let mut screen = Screen::new(true);
        let (word, learning) = remembered(&setup.data, &shared, "тыгыдык");
        hints.engine_input(&mut screen, told(&word, Some(AT)), &at(0.0));
        // It waits for the typing to stop, as a hint at the caret does.
        hints.tick(&mut screen, &at(0.5));
        assert!(screen.log.is_empty());
        hints.tick(&mut screen, &at(1.0));
        assert_eq!(screen.log, ["caret remembered_word at (100, 200)"]);
        assert_eq!(screen.cards, [("Слово «тыгыдык» запомнено: больше не переводится.".to_owned(), HINT_UNDO_REMEMBERED_LABEL)]);
        // «Отменить»: the rule goes, the card goes, and nothing is written of it.
        hints.clicked(&mut screen, REMEMBERED_HINT_ID, Click::Never, &at(2.0));
        assert_eq!(learning.count(), 0);
        assert_eq!(screen.log.last().unwrap(), "hide remembered_word");
        assert!(!hints.state().is_seen(REMEMBERED_HINT_ID) && hints.state().how(REMEMBERED_HINT_ID).is_none());
        assert_ne!(setup.settings.get(HINTS_SETTING), Some(Value::Bool(false)), "the hints stay on");
        // The next word is told too, at once: no quiet period, and «Понятно» leaves the rule.
        let (next, kept) = remembered(&setup.data, &shared, "ксл");
        hints.engine_input(&mut screen, told(&next, Some(AT)), &at(3.0));
        hints.tick(&mut screen, &at(4.0));
        assert_eq!(screen.log.last().unwrap(), "caret remembered_word at (100, 200)");
        hints.clicked(&mut screen, REMEMBERED_HINT_ID, Click::Ok, &at(5.0));
        assert_eq!(kept.count(), 1);
        // Gone from the screen, its «Отменить» has nothing to take back.
        hints.clicked(&mut screen, REMEMBERED_HINT_ID, Click::Never, &at(6.0));
        assert_eq!(kept.count(), 1);
        // On its own it goes after a while, and the rule stays.
        hints.engine_input(&mut screen, told(&next, Some(AT)), &at(7.0));
        hints.tick(&mut screen, &at(8.0));
        hints.tick(&mut screen, &at(20.5));
        assert_eq!(screen.log.last().unwrap(), "hide remembered_word");
        assert!(!hints.needs_tick() && kept.count() == 1);
    }

    #[test]
    fn a_remembered_word_is_told_with_the_hints_off_past_their_limits_and_over_another_hint() {
        let setup = setup("remembered-limits");
        let shared = engine_of(&setup.data);
        let mut hints = hints(&setup);
        let mut screen = Screen::new(true);
        // A hint on screen gives way to the notice.
        hints.engine_input(&mut screen, EngineInput::Correction { correction: LAYOUT, anchor: Some(AT) }, &at(0.0));
        hints.tick(&mut screen, &at(1.0));
        let (first, _) = remembered(&setup.data, &shared, "один");
        hints.engine_input(&mut screen, told(&first, Some(AT)), &at(2.0));
        hints.tick(&mut screen, &at(3.0));
        hints.tick(&mut screen, &at(3.25));
        assert_eq!(
            screen.log,
            ["caret first_correction_undo at (100, 200)", "hide first_correction_undo", "caret remembered_word at (100, 200)"]
        );
        assert_eq!(hints.state().how("first_correction_undo").as_deref(), Some("timeout"));
        // A newer word takes the place of the one on screen, and the undo is of the newer one.
        let (second, second_store) = remembered(&setup.data, &shared, "два");
        hints.engine_input(&mut screen, told(&second, Some(AT)), &at(4.0));
        hints.tick(&mut screen, &at(5.0));
        hints.tick(&mut screen, &at(5.25));
        assert_eq!(screen.log[3..], ["hide remembered_word", "caret remembered_word at (100, 200)"]);
        assert_eq!(screen.cards.last().unwrap().0, "Слово «два» запомнено: больше не переводится.");
        // The hints turned off leave the notice on screen.
        setup.settings.set(HINTS_SETTING, Value::Bool(false), true).unwrap();
        hints.settings_changed(&mut screen, &at(6.0));
        assert_eq!(screen.log.len(), 5);
        hints.clicked(&mut screen, REMEMBERED_HINT_ID, Click::Never, &at(7.0));
        assert_eq!(second_store.count(), 0);
        // With the hints off, and long past the three of a day, a word is told all the same; the typing holds it back
        // however long it goes on.
        for (number, pattern) in ["три", "четыре", "пять", "шесть"].into_iter().enumerate() {
            let (word, _) = remembered(&setup.data, &shared, pattern);
            let start = 10.0 + 30.0 * f64::from(u8::try_from(number).unwrap());
            hints.engine_input(&mut screen, told(&word, Some(AT)), &Moment { typed_presses: 1, ..at(start) });
            for step in 1..20 {
                hints.tick(&mut screen, &Moment { typed_presses: step + 1, ..at(start + f64::from(u8::try_from(step).unwrap()) * 0.5) });
            }
            hints.tick(&mut screen, &Moment { typed_presses: 20, ..at(start + 11.0) });
            assert_eq!(screen.cards.last().unwrap().0, format!("Слово «{pattern}» запомнено: больше не переводится."));
            hints.clicked(&mut screen, REMEMBERED_HINT_ID, Click::Ok, &at(start + 12.0));
        }
        // «Больше не показывать подсказки» of another hint leaves a notice that waits for the typing to stop.
        let (waiting, _) = remembered(&setup.data, &shared, "семь");
        let busy = |seconds: f64| Moment { typed_presses: 30, ..at(seconds) };
        hints.engine_input(&mut screen, told(&waiting, Some(AT)), &busy(200.0));
        hints.clicked(&mut screen, "first_start", Click::Never, &busy(200.5));
        hints.tick(&mut screen, &busy(201.5));
        assert_eq!(screen.cards.last().unwrap().0, "Слово «семь» запомнено: больше не переводится.");
    }

    #[test]
    fn a_remembered_word_with_no_caret_or_no_card_is_told_near_the_tray_icon() {
        let setup = setup("remembered-tray");
        let shared = engine_of(&setup.data);
        let mut hints = hints(&setup);
        let mut screen = Screen::new(true);
        let (word, learning) = remembered(&setup.data, &shared, "тыгыдык");
        hints.engine_input(&mut screen, told(&word, None), &at(0.0));
        assert_eq!(screen.log, ["tray remembered_word"]);
        hints.clicked(&mut screen, REMEMBERED_HINT_ID, Click::Never, &at(1.0));
        assert_eq!(learning.count(), 0);
        // A card at the caret that could not be drawn goes to the tray at once.
        let mut failing = Screen::new(false);
        let (other, _) = remembered(&setup.data, &shared, "ксл");
        hints.engine_input(&mut failing, told(&other, Some(AT)), &at(2.0));
        hints.tick(&mut failing, &at(3.0));
        hints.tick(&mut failing, &at(3.25));
        assert_eq!(failing.log, ["caret remembered_word at (100, 200)", "tray remembered_word"]);
        assert!(!hints.needs_tick(), "the tray could not draw it either: let go");
        // A rule prompt takes the notice at the caret down, as any hint there.
        let (third, kept) = remembered(&setup.data, &shared, "три");
        hints.engine_input(&mut screen, told(&third, Some(AT)), &at(4.0));
        hints.tick(&mut screen, &at(5.0));
        hints.engine_input(&mut screen, EngineInput::Prompt { shown: true }, &at(6.0));
        assert_eq!(screen.log.last().unwrap(), "hide remembered_word");
        hints.clicked(&mut screen, REMEMBERED_HINT_ID, Click::Never, &at(7.0));
        assert_eq!(kept.count(), 1, "nothing on screen to take back");
        // One notice is one word of one store.
        let copy = third.clone();
        let (same_word, _) = remembered(&setup.data, &shared, "тыгыдык");
        assert!(format!("{third:?}").contains("три") && copy == third && word != third && same_word != word);
    }

    #[test]
    fn a_catalogue_without_the_notice_or_without_it_on_this_platform_tells_nothing() {
        let setup = setup("remembered-catalogue");
        let shared = engine_of(&setup.data);
        let (word, _) = remembered(&setup.data, &shared, "тыгыдык");
        let notice = CATALOGUE.iter().find(|hint| hint.notice()).copied().unwrap();
        let elsewhere: &'static [Hint] = Box::leak(vec![Hint { platforms: &[Platform::Windows], ..notice }].into_boxed_slice());
        let without: &'static [Hint] = Box::leak(vec![CATALOGUE[0]].into_boxed_slice());
        for catalogue in [elsewhere, without] {
            let mut hints = Hints::<Anchor>::with_catalogue(catalogue, setup.settings.clone(), &setup.data, Platform::Linux, version("0.71.0"), &at(0.0));
            let mut screen = Screen::new(true);
            hints.engine_input(&mut screen, told(&word, None), &at(0.0));
            assert!(screen.log.is_empty() && !hints.needs_tick());
        }
    }

    #[test]
    fn the_engine_hands_over_its_snapshots_corrections_and_prompts() {
        let root = folder("engine");
        let shared = engine_of(&root);
        let posted: Arc<Mutex<Vec<EngineInput<Anchor>>>> = Arc::new(Mutex::new(Vec::new()));
        let wants = Arc::new(AtomicBool::new(true));
        let sink = posted.clone();
        subscribe::<Anchor>(&shared, wants.clone(), Arc::new(|| Some(AT)), Arc::new(move |input| sink.lock().unwrap().push(input)));
        let plan = CorrectionPlan::new(Vec::new(), None, 0, 1, "ghbdtn".into(), "привет".into(), 0.9, "app");
        for callback in shared.correction_callbacks() {
            callback(&plan);
        }
        wants.store(false, Ordering::Relaxed);
        for callback in shared.correction_callbacks() {
            callback(&plan);
        }
        for callback in shared.kept_by_choice_callbacks() {
            callback();
        }
        for callback in shared.auto_switch_callbacks() {
            callback(false);
        }
        let (word, _) = remembered(&root, &shared, "тыгыдык");
        for callback in shared.remembered_callbacks() {
            callback(&word);
        }
        shared.show_learning_prompt(LearningPrompt {
            source_group: 0,
            target_group: 1,
            original: "ghbdtn".into(),
            replacement: "привет".into(),
            application: "app".into(),
            action: RuleAction::Convert,
        });
        let posted = posted.lock().unwrap().clone();
        assert_eq!(
            posted,
            [
                EngineInput::Snapshot { running: false },
                EngineInput::Prompt { shown: false },
                EngineInput::Correction { correction: LAYOUT, anchor: Some(AT) },
                EngineInput::Correction { correction: LAYOUT, anchor: None },
                EngineInput::KeptByChoice { anchor: Some(AT) },
                EngineInput::AutoSwitch { enabled: false, anchor: Some(AT) },
                EngineInput::Remembered { word, anchor: Some(AT) },
                EngineInput::Prompt { shown: true },
            ]
        );
    }
}

//! What a platform wires: the engine's events in, a presenter out. The presenter only draws and reports clicks; the
//! rules, the file and the setting are kept here.
//!
//! A platform, on its main thread:
//! - makes [`Hints::start`] right after its settings store, before anything writes a user file, so that a new
//!   installation is not taken for an update;
//! - subscribes with [`subscribe`] once the engine is built (`EngineLoader::when_built`), and hands each
//!   [`EngineInput`] it posts to [`Hints::engine_input`];
//! - calls [`Hints::settings_changed`] when a setting changes, [`Hints::settings_opened`] when its settings window
//!   opens, and [`Hints::settings_card`] when it builds a page;
//! - calls [`Hints::tick`] every `HINT_TICK_INTERVAL_MS` while [`Hints::needs_tick`];
//! - reports what the user did to a hint with [`Hints::clicked`].
//!
//! The presenter's methods never call back into [`Hints`]: a click arrives later, from the platform's event loop.

use ks_config::config::SettingsStore;
use ks_config::constants::HINTS_SETTING;
use ks_config::logsetup::{self, VERSION};
use ks_engine::{CorrectionPlan, EngineSnapshot, LearningPrompt, Shared};
use ks_updates::SemanticVersion;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::catalogue::{CATALOGUE, CorrectionInfo, Hint, HotkeyName, Hotkeys, Opens, Place, Platform, SettingsPage};
use super::schedule::{HintSettings, HintView, Moment, Scheduler, Step};
use super::store::{HintState, HintStore, How, Loaded, Startup, startup_kind, user_files};
use crate::windows_ui_model::truthy;

const LOGGER: &str = "keyswitch.hints";

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
    /// «Больше не показывать подсказки».
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
}

/// The engine's subscriptions the hints need, made once the engine is built. On the engine's thread the caret is looked
/// for with `anchor`, only while `wants_anchor` says a hint at the caret may still show (`Hints::caret_anchor_wanted`);
/// `post` carries each input to the main thread.
pub fn subscribe<A: Send + 'static>(
    shared: &Shared, wants_anchor: Arc<AtomicBool>, anchor: Arc<dyn Fn() -> Option<A> + Send + Sync>,
    post: Arc<dyn Fn(EngineInput<A>) + Send + Sync>,
) {
    let deliver = post.clone();
    shared.subscribe(Arc::new(move |snapshot: &EngineSnapshot| deliver(EngineInput::Snapshot { running: snapshot.running })));
    let deliver = post.clone();
    shared.subscribe_corrections(Arc::new(move |plan: &CorrectionPlan| {
        let found = if wants_anchor.load(Ordering::Relaxed) { anchor() } else { None };
        deliver(EngineInput::Correction { correction: CorrectionInfo::of(plan), anchor: found });
    }));
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
}

impl<A: Clone> Hints<A> {
    /// The hints of the catalogue, kept in `hints.json` of `data_dir`.
    #[must_use]
    pub fn start(settings: Arc<SettingsStore>, data_dir: &Path, platform: Platform, moment: &Moment) -> Hints<A> {
        Hints::with_catalogue(&CATALOGUE, settings, data_dir, platform, running_version(), moment)
    }

    /// The same over any catalogue and version, for tests.
    #[must_use]
    pub fn with_catalogue(
        catalogue: &'static [Hint], settings: Arc<SettingsStore>, data_dir: &Path, platform: Platform, version: SemanticVersion,
        moment: &Moment,
    ) -> Hints<A> {
        let store = HintStore::in_folder(data_dir);
        // A file of the hints that cannot be read was still left by a version that had them.
        let older_files = store.path.exists() || user_files(&settings.path, data_dir).iter().any(|path| path.exists());
        let (mut state, recorded) = match store.load() {
            Loaded::Read(state) => (state, true),
            Loaded::Missing | Loaded::SetAside(_) => (HintState::default(), false),
        };
        let startup = startup_kind(version, recorded.then_some(&state), older_files);
        state.begin(startup, version, &moment.timestamp);
        let scheduler = Scheduler::new(catalogue, platform, version, HintSettings::read(&settings), state, moment);
        let mut hints = Hints { scheduler, store, settings, wants_anchor: Arc::new(AtomicBool::new(false)), startup };
        hints.write();
        hints.sync();
        hints
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
        self.scheduler.needs_tick()
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
        };
        self.settle(presenter, step, moment);
    }

    /// Called every `HINT_TICK_INTERVAL_MS` while `needs_tick`.
    pub fn tick(&mut self, presenter: &mut dyn HintPresenter<A>, moment: &Moment) {
        self.settle(presenter, None, moment);
    }

    /// A setting changed: the hints read theirs again.
    pub fn settings_changed(&mut self, presenter: &mut dyn HintPresenter<A>, moment: &Moment) {
        let step = self.scheduler.set_settings(HintSettings::read(&self.settings));
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

    /// What the user did to the hint `id`: it goes away, and «Больше не показывать подсказки» turns `general.hints` off.
    pub fn clicked(&mut self, presenter: &mut dyn HintPresenter<A>, id: &str, click: Click, moment: &Moment) {
        let how = match click {
            Click::Ok | Click::Closed => How::Ok,
            Click::Never => How::Forever,
            Click::Text => How::Click,
        };
        presenter.hide(id);
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
    use ks_backend::{BackendError, BackendProbe, FocusInfo, InputBackend, Injection, KeyFilter, KeyListener};
    use ks_config::learning::RuleAction;
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
    }

    impl Screen {
        fn new(draws: bool) -> Screen {
            Screen { log: Vec::new(), draws }
        }
    }

    impl HintPresenter<Anchor> for Screen {
        fn show_caret(&mut self, view: &HintView, anchor: &Anchor) -> bool {
            self.log.push(format!("caret {} at {anchor:?}", view.id));
            self.draws
        }

        fn show_tray(&mut self, view: &HintView) -> bool {
            self.log.push(format!("tray {}", view.id));
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
        hints.tick(&mut screen, &at(6.0));
        assert_eq!(screen.log, ["tray first_start"]);
        hints.clicked(&mut screen, "first_start", Click::Closed, &at(7.0));
        assert_eq!(screen.log, ["tray first_start", "hide first_start"]);
        assert_eq!(hints.state().how("first_start").as_deref(), Some("ok"));
        assert!(file(&setup).contains(r#""how":"ok""#), "{}", file(&setup));
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
        hints.clicked(&mut screen, "first_start", Click::Never, &at(6.0));
        assert_eq!(settings.get(HINTS_SETTING), Some(Value::Bool(false)));
        assert_eq!(hints.state().how("first_start").as_deref(), Some("forever"));
    }

    #[test]
    fn the_settings_the_hints_follow_fall_back_to_the_defaults() {
        let root = folder("settings");
        std::fs::write(root.join("config.json"), r#"{"general": "x", "hotkeys": {"undo": 5}}"#).unwrap();
        let settings = SettingsStore::new(Some(root.join("config.json")));
        let read = HintSettings::read(&settings);
        assert!(read.enabled && read.notifications && read.keep_history);
        assert_eq!((read.hotkeys.undo.as_str(), read.hotkeys.convert_last.as_str()), ("", "Pause"));
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

    #[test]
    fn the_engine_hands_over_its_snapshots_corrections_and_prompts() {
        let root = folder("engine");
        let settings = Arc::new(SettingsStore::new(Some(root.join("config.json"))));
        let shared = Shared::new(settings, Arc::new(Minimal), Arc::new(|| 0.0), 2, EngineSnapshot::default());
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
                EngineInput::Prompt { shown: true },
            ]
        );
    }
}

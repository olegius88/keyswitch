//! The engine as the application keeps it: the worker thread and the calls made from other threads.

use super::logging::technical_event;
use super::shared::{Command, CorrectionCallback, LearningPromptCallback, Next, QueueItem, RuleRequestCallback, SnapshotCallback};
use super::*;
use crate::constants::{ENGINE_LOOP_MAX_WAKE_SECONDS, ENGINE_LOOP_MIN_WAKE_SECONDS, ENGINE_WORKER_JOIN_TIMEOUT_SECONDS, FIELD_CATCH_UP_RETRY_SECONDS};
use ks_backend::{BackendError, FieldReader};
use ks_config::indicator::alternate_layout_group;
use ks_config::learning::{LearnedRule, LearningError};
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The handle the application keeps. Everything here may be called from any thread but the keyboard hook's.
pub struct KeySwitchEngine {
    pub shared: Arc<Shared>,
    learning: Arc<LearningStore>,
    personal: Arc<PersonalWords>,
    backend_label: String,
    /// The state, until the worker takes it; the worker hands it back when it ends.
    core: Mutex<Option<EngineCore>>,
    _worker: Mutex<Option<JoinHandle<EngineCore>>>,
}

impl KeySwitchEngine {
    /// `KeySwitchEngine(settings, history, backend, learning, backend_label, context_reader)`. The resources folder and the
    /// clock are given here, as the Python finds its own; `context_reader` None is the reader of this platform, built from
    /// `reader_factory`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        settings: Arc<SettingsStore>, history: Arc<HistoryStore>, backend: Arc<dyn InputBackend>, learning: Option<Arc<LearningStore>>,
        backend_label: &str, context_reader: Option<Arc<dyn FieldReader>>, reader_factory: Option<crate::context_access::ReaderFactory>,
        resources: &Arc<Resources>, clock: Clock,
    ) -> EngineResult<KeySwitchEngine> {
        let locales: Vec<String> = settings.with_value("detection.language_models", |value| match value {
            Some(Value::Array(items)) => items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect(),
            _ => vec!["en_US".to_owned(), "ru_RU".to_owned()],
        });
        let mut models = Vec::new();
        for locale in locales.iter().take(ks_backend::constants::LAYOUT_GROUP_COUNT.to::<usize>()) {
            models.push(resources.engine_language_model(locale)?);
        }
        let (intent_model, intent_model_status) = resources.intent_model();
        let platform_reader = match (&context_reader, reader_factory) {
            (None, Some(factory)) => Some(Arc::new(crate::context_access::PlatformFieldReader::new(factory, clock.clone()))),
            _ => None,
        };
        let reader: Option<Arc<dyn FieldReader>> = context_reader.or_else(|| platform_reader.clone().map(|reader| reader as _));
        let context_policy = ContextPolicy::new(resources, reader, EngineCore::known_of(&models), Some(clock.clone()));
        let prefix_indexes = models.iter().map(|model| resources.prefix_index(model)).collect();
        let core = EngineCore::from_parts(EngineParts {
            settings,
            history,
            backend,
            learning,
            backend_label: backend_label.to_owned(),
            models,
            intent_model,
            intent_model_status,
            protected_tokens: resources.protected_tokens(),
            context_policy,
            boundary_model: resources.boundary_policy(),
            prefix_model: VersionedPrefixModel::default_model(&resources.resource_dir().join("models")).map(Arc::new),
            prefix_indexes,
            resource_dir: resources.resource_dir().to_path_buf(),
            clock,
            platform_reader,
        })?;
        Ok(KeySwitchEngine::of(core))
    }

    /// The handle of a core built from parts.
    pub fn of(core: EngineCore) -> KeySwitchEngine {
        KeySwitchEngine {
            shared: core.shared.clone(),
            learning: core.learning.clone(),
            personal: core._personal.clone(),
            backend_label: core.backend_label.clone(),
            core: Mutex::new(Some(core)),
            _worker: Mutex::new(None),
        }
    }

    /// The state, for a replay that drives the engine from its own thread: empty once a worker holds it.
    pub fn core(&self) -> std::sync::MutexGuard<'_, Option<EngineCore>> {
        self.core.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        self.shared.snapshot()
    }

    pub fn subscribe(&self, callback: SnapshotCallback) {
        self.shared.subscribe(callback);
    }

    pub fn subscribe_corrections(&self, callback: CorrectionCallback) {
        self.shared.subscribe_corrections(callback);
    }

    pub fn learning_prompt(&self) -> Option<LearningPrompt> {
        self.shared.learning_prompt()
    }

    pub fn subscribe_learning_prompts(&self, callback: LearningPromptCallback) {
        self.shared.subscribe_learning_prompts(callback);
    }

    pub fn subscribe_rule_requests(&self, callback: RuleRequestCallback) {
        self.shared.subscribe_rule_requests(callback);
    }

    pub fn confirm_learning_prompt(&self, prompt: Option<&LearningPrompt>) -> bool {
        self.shared.confirm_learning_prompt(prompt)
    }

    pub fn dismiss_learning_prompt(&self, prompt: Option<&LearningPrompt>) -> bool {
        self.shared.dismiss_learning_prompt(prompt, "dismissed")
    }

    pub fn set_rule_editor_open(&self, opened: bool) {
        self.shared.set_rule_editor_open(opened);
    }

    pub fn set_test_field_focused(&self, focused: bool) {
        self.shared.set_test_field_focused(focused);
    }

    pub fn learning(&self) -> &Arc<LearningStore> {
        &self.learning
    }

    /// OK in the rule window: keep the rule, in place of ``replacing`` if given.
    pub fn add_learning_rule(&self, rule: LearnedRule, replacing: Option<&LearnedRule>, application: &str) -> Result<LearnedRule, LearningError> {
        let stored = match replacing {
            None => self.learning.add_rule(rule)?,
            Some(old) => self.learning.replace_rule(old, rule)?,
        };
        let excluded = !application.is_empty() && self.shared._application_excluded(application);
        technical_event!(
            self.shared, "learning_rule_added",
            "pattern": if excluded { "<redacted>" } else { stored.pattern.as_str() },
            "match": stored.r#match.as_str(),
            "case_sensitive": stored.case_sensitive,
            "action": stored.action.as_str(),
            "source_group": stored.source_group,
            "target_group": stored.target_group,
            "replaced": replacing.is_some(),
            "application": application,
            "application_excluded": excluded,
        );
        let verb = if stored.action == ks_config::learning::RuleAction::Convert { "переводить" } else { "не переводить" };
        self.shared._update(Update { last_action: Some(format!("Правило добавлено: «{}» — {verb}", stored.pattern)), ..Update::default() });
        Ok(stored)
    }

    /// Forget the user's own words (personal_words).
    pub fn clear_personal_words(&self) {
        self.personal.clear();
        technical_event!(self.shared, "personal_words_cleared");
    }

    pub fn start(&self) -> Result<(), BackendError> {
        if self.shared._running.load(Ordering::Relaxed) {
            return Ok(());
        }
        let Some(core) = self.core.lock().ok().and_then(|mut core| core.take()) else {
            return Err(BackendError::Failed("Движок раскладки уже остановлен".to_owned()));
        };
        self.shared._running.store(true, Ordering::Relaxed);
        let worker = std::thread::Builder::new().name("keyswitch-engine".to_owned()).spawn(move || _run(core));
        let worker = worker.map_err(|error| BackendError::Failed(error.to_string()))?;
        *self._worker.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(worker);
        // The hook asks this before letting a key through to the window.
        self.shared.backend.set_key_filter(Some(self.shared.key_filter()));
        let started = self
            .shared
            .backend
            .start(self.shared.key_listener())
            .and_then(|()| self.shared.backend.current_group());
        match started {
            Ok(group) => {
                self.shared._update(Update {
                    running: Some(true),
                    backend: Some(self.backend_label.clone()),
                    current_group: Some(i64::from(group)),
                    last_error: Some(String::new()),
                    ..Update::default()
                });
                Ok(())
            }
            Err(error) => {
                self.shared._running.store(false, Ordering::Relaxed);
                self.shared.put_nowait(QueueItem::Stop);
                self.shared._update(Update {
                    running: Some(false),
                    backend: Some("недоступен".to_owned()),
                    last_error: Some(error.to_string()),
                    ..Update::default()
                });
                Err(error)
            }
        }
    }

    pub fn stop(&self) {
        self.shared.dismiss_learning_prompt(None, "dismissed");
        self.shared.backend.set_key_filter(None);
        if !self.shared._running.load(Ordering::Relaxed) {
            if let Some(core) = self.core.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_mut() {
                core.context_policy.stream.clear();
            }
            self.shared.backend.close();
            return;
        }
        self.shared._running.store(false, Ordering::Relaxed);
        self.shared.backend.stop();
        self.shared.put_nowait(QueueItem::Stop);
        let worker = self._worker.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        if let Some(worker) = worker
            && worker.thread().id() != std::thread::current().id()
        {
            let deadline = Instant::now() + Duration::from_secs_f64(ENGINE_WORKER_JOIN_TIMEOUT_SECONDS);
            while !worker.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if worker.is_finished()
                && let Ok(mut core) = worker.join()
            {
                core.context_policy.stream.clear();
                *self.core.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(core);
            }
        }
        self.shared.backend.close();
        self.shared._update(Update {
            running: Some(false),
            backend: Some("остановлен".to_owned()),
            current_word: Some(String::new()),
            ..Update::default()
        });
    }

    /// `enqueue`, for a listener the application builds itself.
    pub fn enqueue(&self, event: KeyEvent) {
        self.shared.enqueue(event);
    }

    /// Queue an explicit selection of the language opposite to the current one.
    pub fn select_alternate_group(&self) -> bool {
        let failed = |error: &str| {
            self.shared._update(Update {
                last_error: Some(error.to_owned()),
                last_action: Some("Язык из меню не переключён".to_owned()),
                ..Update::default()
            });
            false
        };
        if !self.shared._running.load(Ordering::Relaxed) {
            return failed("Движок раскладки не запущен");
        }
        let target = alternate_layout_group(self.shared.current_group());
        let Some(target) = target.filter(|target| 0 <= *target && (*target as usize) < self.shared.models_len) else {
            return failed("Текущая раскладка EN/RU не определена");
        };
        if !self.shared.put_nowait(QueueItem::LayoutSelection(target)) {
            return failed("Очередь ввода переполнена");
        }
        true
    }
}

/// `_run`: the worker's loop. It owns the core and hands it back when it ends.
fn _run(mut core: EngineCore) -> EngineCore {
    let shared = core.shared.clone();
    while shared._running.load(Ordering::Relaxed) {
        match shared.next(core._loop_timeout()) {
            Next::Command(Command::SettingsChanged(path, value)) => core._settings_changed(&path, &value),
            Next::Timeout => {
                // The timers replace text too (pause correction, deferred Enter):
                // an error there must not end this thread while the hook keeps
                // queueing keys and a held Enter waits for its release.
                if let Err(error) = core._run_timers() {
                    core._recover_from_error(&error);
                }
            }
            Next::Item(QueueItem::Stop) => break,
            Next::Item(QueueItem::LayoutSelection(group)) => core._apply_layout_selection(group),
            Next::Item(QueueItem::Key(event)) => {
                if let Err(error) = core._handle(event) {
                    core._recover_from_error(&error);
                }
            }
        }
    }
    if let Some(reader) = &core.platform_reader {
        reader.close();
    }
    core
}

impl EngineCore {
    /// What the worker does as it wakes: the settings other threads changed, applied on the engine's own thread.
    pub fn apply_commands(&mut self) {
        while let Some(Command::SettingsChanged(path, value)) = self.shared.take_command() {
            self._settings_changed(&path, &value);
        }
    }

    /// The context policy, handed on to the next engine of a replay.
    pub fn context_policy_out(self) -> ContextPolicy {
        self.context_policy
    }

    /// `_run_timers`: the eight timers of the live loop. The harness pumps its own five, one by one.
    pub fn _run_timers(&mut self) -> EngineResult<()> {
        self._expire_deferred_action()?;
        self._expire_manual_correction();
        self._poll_current_group()?;
        self._maybe_correct_after_pause(None)?;
        self._retry_field_wait();
        self._expire_learning_prompt(None);
        self._report_input_delay();
        self._report_unseen_input()?;
        Ok(())
    }

    /// Log, once per program, input that reached a program without any hook of KeySwitch.
    ///
    /// A program in front that reads the keyboard as raw input and has a keyboard hook of its own keeps every
    /// keyboard hook of the system silent (Windows 10 and 11, 28.10.2026): KeySwitch sees nothing typed there, and
    /// the log did not say so. The programs are logged to learn which do it before anything is built for them, and
    /// nothing else is done. Letting go of the text as well dropped words the hook saw typed in 0.64.0: on a laptop
    /// input no hook got came in the middle of words in 14 programs, and each time the word, the text before it and
    /// the last correction were forgotten, so Pause found no word to convert. A backend that cannot tell has no
    /// take_unseen_input.
    pub fn _report_unseen_input(&mut self) -> EngineResult<()> {
        let Some(unseen) = self.backend.take_unseen_input()? else {
            return Ok(());
        };
        let application = self.backend.active_application();
        if !self._unseen_applications.insert(application.clone()) {
            return Ok(());
        }
        technical_event!(
            self.shared, "input_unseen", "application": application, "application_excluded": self._application_excluded(&application),
            "unseen_ms": unseen, "silence_threshold_ms": HOOK_SILENCE_MS,
        );
        Ok(())
    }

    /// Log how late the keyboard hook saw keys, once a minute and only when one was late.
    ///
    /// A backend that cannot tell (X11 observes keys after the window has them) has no
    /// take_input_delay; see input_delay.
    pub fn _report_input_delay(&mut self) {
        let now = self.monotonic();
        if now < self._input_delay_due {
            return;
        }
        self._input_delay_due = now + INPUT_DELAY_REPORT_INTERVAL_SECONDS;
        let Some(report) = self.backend.take_input_delay() else {
            return;
        };
        if report.stalled() {
            technical_event!(
                self.shared, "input_delay",
                "keys": report.keys, "late_keys": report.late_keys, "worst_late_ms": report.worst_late_ms,
                "slow_callbacks": report.slow_callbacks, "worst_callback_ms": report.worst_callback_ms,
                "unstamped_keys": report.unstamped_keys, "foreign_keys": report.foreign_keys,
                "late_threshold_ms": INPUT_DELAY_LATE_MS,
                "slow_threshold_ms": INPUT_DELAY_SLOW_CALLBACK_MS, "unstamped_threshold_ms": INPUT_DELAY_UNSTAMPED_MS,
            );
        }
    }

    pub fn _recover_from_error(&mut self, error: &EngineError) {
        self._clear_word(None, "input_error");
        self.shared._update(Update {
            last_error: Some(error.to_string()),
            last_action: Some("Ошибка обработки ввода".to_owned()),
            ..Update::default()
        });
    }

    /// Wake exactly when the pause delay elapses, at most every 0.5 s.
    pub fn _loop_timeout(&self) -> f64 {
        if self._field_wait.is_some() {
            return FIELD_CATCH_UP_RETRY_SECONDS;
        }
        let Some(last_input) = self._last_word_input_at.filter(|_| self._pause_correction_pending) else {
            return ENGINE_LOOP_MAX_WAKE_SECONDS;
        };
        let remaining = last_input + self._pause_delay() - self.monotonic();
        let bounded = if remaining < ENGINE_LOOP_MAX_WAKE_SECONDS { remaining } else { ENGINE_LOOP_MAX_WAKE_SECONDS };
        if bounded > ENGINE_LOOP_MIN_WAKE_SECONDS { bounded } else { ENGINE_LOOP_MIN_WAKE_SECONDS }
    }

    pub fn _apply_layout_selection(&mut self, group: i64) {
        if let Err(error) = self.backend.switch_group(group.to::<i32>()) {
            technical_event!(self.shared, "layout_selection_failed", "requested_group": group, "error": error.to_string());
            self.shared._update(Update {
                last_error: Some(error.to_string()),
                last_action: Some("Язык из меню не переключён".to_owned()),
                ..Update::default()
            });
            return;
        }
        self._clear_word(None, "layout_selected");
        self._last_committed_stale = true;
        self._note_engine_switch(group);
        self._manual_layout_group = self.flag("detection.respect_manual_layout", true).then_some(group);
        self._manual_layout_observed_at = Some(self.monotonic());
        self._manual_layout_source = "menu".to_owned();
        self._manual_layout_word = false;
        technical_event!(
            self.shared, "layout_selected_from_menu",
            "selected_group": group,
            "protects_next_word": self._manual_layout_group == Some(group),
        );
        self.shared._update(Update {
            current_group: Some(group),
            last_action: Some(format!("Язык выбран из меню: {}", ks_config::indicator::layout_label(group))),
            last_error: Some(String::new()),
            ..Update::default()
        });
    }
}

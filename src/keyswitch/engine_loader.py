"""The keyboard engine built on a worker thread while the window and the tray come up.

Building the engine loads every model: about 7 s on a fast computer and three times that on a busy one (03.11.2026),
and until then the program showed nothing, neither its window nor its tray icon, so it looked as if it had not started.
`DeferredEngine` stands where the window code expects the engine: the switching rules, which the window shows at
once, are its own; subscriptions and the start wait for the engine and are passed on to it on the window's thread
(`post`); until then the snapshot says the models are loading, and so do the diagnostics, which name the models.
"""

from __future__ import annotations

import threading
from collections.abc import Callable

from .context_policy import ContextPolicy
from .engine import CorrectionPlan, EngineSnapshot, KeySwitchEngine, LearningPrompt
from .intent_model import IntentModelStatus
from .learning import LearnedRule, LearningStore
from .prefix_model import PrefixModel

LOADING_ACTION = "Загрузка языковых моделей…"


class DeferredEngine:
    def __init__(
        self,
        build: Callable[[], KeySwitchEngine],
        *,
        learning: LearningStore,
        enabled: bool,
        post: Callable[[Callable[[], None]], None],
        background: bool = True,
    ) -> None:
        self.learning = learning
        self._build = build
        self._post = post
        self._ready = threading.Event()
        self._engine: KeySwitchEngine | None = None
        self._error: Exception | None = None
        self._lock = threading.Lock()
        # Subscriptions made before the engine is built, each a call to make on it.
        self._subscriptions: list[Callable[[KeySwitchEngine], None]] = []
        self._start_failed: Callable[[Exception], None] | None = None
        self._stopped = False
        self._loading = EngineSnapshot(enabled=enabled, last_action=LOADING_ACTION)
        if background:
            threading.Thread(target=self._load, name="KeySwitchEngineLoader", daemon=True).start()
        else:
            self._load()

    def _load(self) -> None:
        try:
            engine = self._build()
        except Exception as error:  # reported on the window's thread, as a failed start is
            self._error = error
            self._ready.set()
            self._post(self._report_error)
            return
        with self._lock:
            self._engine = engine
        self._ready.set()
        self._post(self._attach)

    def _attach(self) -> None:
        """On the window's thread: hand the waiting subscriptions to the engine and start it if it was asked to."""

        engine = self._engine
        assert engine is not None
        with self._lock:
            subscriptions, self._subscriptions = self._subscriptions, []
            start_failed, stopped = self._start_failed, self._stopped
        for subscription in subscriptions:
            subscription(engine)
        if stopped:
            # The window closed while the models loaded: the engine never starts, and its stop closes the backend.
            engine.stop()
        elif start_failed is not None:
            try:
                engine.start()
            except Exception as error:
                start_failed(error)

    def _report_error(self) -> None:
        # Posted by the failed build and by a start asked for after it: the window hears of it once.
        with self._lock:
            start_failed, self._start_failed = self._start_failed, None
        if start_failed is not None and self._error is not None:
            start_failed(self._error)

    def engine(self) -> KeySwitchEngine:
        """The engine, waited for; the error that kept it from being built, raised again."""

        self._ready.wait()
        if self._engine is None:
            assert self._error is not None
            raise self._error
        return self._engine

    @property
    def ready(self) -> bool:
        """Whether the engine is built; the diagnostics show the loading or its error until it is, not wait for it."""

        return self._engine is not None

    @property
    def snapshot(self) -> EngineSnapshot:
        engine = self._engine
        if engine is not None:
            return engine.snapshot
        if self._error is not None:
            return EngineSnapshot(enabled=self._loading.enabled, last_error=str(self._error))
        return self._loading

    @property
    def learning_prompt(self) -> LearningPrompt | None:
        engine = self._engine
        return None if engine is None else engine.learning_prompt

    def _subscribe(self, subscription: Callable[[KeySwitchEngine], None],
                   late: Callable[[KeySwitchEngine], None] | None = None) -> None:
        """Subscribe now if the engine is built, else once it is; `late` is the call made then, if it differs."""

        with self._lock:
            engine = self._engine
            if engine is None:
                self._subscriptions.append(late or subscription)
                return
        subscription(engine)

    def subscribe(self, callback: Callable[[EngineSnapshot], None]) -> None:
        def late(engine: KeySwitchEngine) -> None:
            # Subscribed while the models loaded: the window learns at once what the loading snapshot stood for.
            engine.subscribe(callback)
            callback(engine.snapshot)

        self._subscribe(lambda engine: engine.subscribe(callback), late)

    def subscribe_corrections(self, callback: Callable[[CorrectionPlan], None]) -> None:
        self._subscribe(lambda engine: engine.subscribe_corrections(callback))

    def subscribe_learning_prompts(self, callback: Callable[[LearningPrompt | None], None]) -> None:
        self._subscribe(lambda engine: engine.subscribe_learning_prompts(callback))

    def subscribe_rule_requests(self, callback: Callable[[LearningPrompt], None]) -> None:
        self._subscribe(lambda engine: engine.subscribe_rule_requests(callback))

    def start(self, failed: Callable[[Exception], None]) -> None:
        """Start the engine once it is built, on the window's thread; `failed` gets the error that stopped it."""

        with self._lock:
            engine = self._engine
            if engine is None:
                self._start_failed = failed
                if self._error is not None:
                    self._post(self._report_error)
                return
        try:
            engine.start()
        except Exception as error:
            failed(error)

    def stop(self) -> None:
        with self._lock:
            self._stopped = True
            engine = self._engine
        if engine is not None:
            engine.stop()

    def confirm_learning_prompt(self, prompt: LearningPrompt | None = None) -> bool:
        engine = self._engine
        return engine is not None and engine.confirm_learning_prompt(prompt)

    def dismiss_learning_prompt(self, prompt: LearningPrompt | None = None) -> bool:
        engine = self._engine
        return engine is not None and engine.dismiss_learning_prompt(prompt)

    def set_rule_editor_open(self, opened: bool) -> None:
        engine = self._engine
        if engine is not None:
            engine.set_rule_editor_open(opened)

    def set_test_field_focused(self, focused: bool) -> None:
        engine = self._engine
        if engine is not None:
            engine.set_test_field_focused(focused)

    def select_alternate_group(self) -> bool:
        engine = self._engine
        return engine is not None and engine.select_alternate_group()

    def add_learning_rule(self, rule: LearnedRule, *, replacing: LearnedRule | None = None,
                          application: str = "") -> LearnedRule:
        return self.engine().add_learning_rule(rule, replacing=replacing, application=application)

    # The diagnostics name the models: there is nothing to show of them before they are loaded.
    @property
    def intent_model_status(self) -> IntentModelStatus:
        return self.engine().intent_model_status

    @property
    def context_policy(self) -> ContextPolicy:
        return self.engine().context_policy

    @property
    def prefix_model(self) -> PrefixModel | None:
        return self.engine().prefix_model

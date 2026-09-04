"""Keyboard event state machine and correction orchestration."""

from __future__ import annotations

import json
import logging
import queue
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass

from . import __version__
from .backend import InputBackend, KeyEvent
from .config import SettingsStore
from .detector import DetectionDecision, LanguageDetector
from .early_switch import (
    EarlySwitchDecision,
    EarlySwitchPolicy,
    PrefixIndex,
    early_switch_decision,
)
from .history import HistoryEntry, HistoryStore
from .indicator import alternate_layout_group, layout_label
from .language_model import LanguageModel, WordScore
from .learning import LearningStore
from .intent_model import CorrectionTrigger, LinearNgramModel
from .short_words import trusted_short_word_decision


MODIFIER_KEYS = {
    "Shift_L", "Shift_R", "Control_L", "Control_R", "Alt_L", "Alt_R",
    "Meta_L", "Meta_R", "Super_L", "Super_R", "ISO_Level3_Shift",
}
NAVIGATION_KEYS = {
    "Left", "Right", "Up", "Down", "Home", "End", "Page_Up", "Page_Down",
    "Escape", "Delete", "Insert",
}
PUNCTUATION = set(".,!?;:()[]{}—–-…\"«»")
PAUSE_CORRECTION_DELAY_SECONDS = 1.5
LEARNING_PROMPT_TIMEOUT_SECONDS = 8.0
# A layout change observed this soon after the engine switched the layout
# itself (correction, menu action) is the engine's own switch, not the user's.
ENGINE_SWITCH_GRACE_SECONDS = 1.5
# A key without a release for this long is treated as a lost key-up so a
# stuck entry can never block pause correction forever.
STALE_PRESS_SECONDS = 3.0
# A letter arriving in the old layout this soon after an early switch was
# pressed before the switch took effect and is converted on its own.
LATE_STROKE_GRACE_SECONDS = 0.5
EARLY_SWITCH_CONFIDENCE = 15.0
WORD_BOUNDARY_KEYS = {"space", "Return", "Tab", "ISO_Left_Tab"}
LOGGER = logging.getLogger(__name__)


@dataclass(frozen=True)
class _FocusChange:
    """What the focus probe found: another window, or one whose layout is moot."""

    changed: bool
    ignore_layout: bool


def _default_backend(group_count: int) -> InputBackend:
    """Load the Linux backend only when no platform backend was supplied."""

    from .x11_backend import X11Backend

    return X11Backend(group_count=group_count)


@dataclass(frozen=True)
class CorrectionPlan:
    strokes: tuple[KeyEvent, ...]
    boundary: KeyEvent | None
    source_group: int
    target_group: int
    original: str
    replacement: str
    confidence: float
    application: str
    automatic: bool = True
    # boundary | pause | manual | undo | early | symbols | late_stroke
    mode: str = "boundary"


@dataclass(frozen=True)
class EngineSnapshot:
    running: bool = False
    enabled: bool = True
    backend: str = "остановлен"
    current_group: int = -1
    current_word: str = ""
    correction_count: int = 0
    last_action: str = "Ожидание ввода"
    last_error: str = ""


@dataclass(frozen=True)
class LanguageContext:
    group: int
    words: dict[int, str]
    updated_at: float


@dataclass(frozen=True)
class LearningPrompt:
    source_group: int
    target_group: int
    original: str
    replacement: str
    application: str


@dataclass(frozen=True)
class _LayoutSelection:
    group: int


class Hotkey:
    MODIFIERS = {"ctrl", "control", "alt", "shift", "super", "meta"}

    def __init__(self, value: str) -> None:
        pieces = [piece.strip().casefold() for piece in value.replace("<", "").replace(">", "+").split("+") if piece.strip()]
        self.modifiers = {piece for piece in pieces if piece in self.MODIFIERS}
        keys = [piece for piece in pieces if piece not in self.MODIFIERS]
        self.key = keys[-1] if keys else ""

    def matches(self, event: KeyEvent) -> bool:
        if not event.pressed or not self.key:
            return False
        actual = set()
        if event.control:
            actual.add("ctrl")
        if event.alt:
            actual.add("alt")
        if event.shift:
            actual.add("shift")
        if event.super_key:
            actual.add("super")
        wanted = {"ctrl" if item == "control" else "super" if item == "meta" else item for item in self.modifiers}
        key_name = event.key_name.casefold()
        aliases = {"pause": {"pause", "break"}, "backspace": {"backspace"}}
        matches_key = key_name in aliases.get(self.key, {self.key})
        return matches_key and actual == wanted


class KeySwitchEngine:
    def __init__(
        self,
        settings: SettingsStore,
        history: HistoryStore,
        backend: InputBackend | None = None,
        learning: LearningStore | None = None,
        backend_label: str = "X11 RECORD + XTEST",
    ) -> None:
        self.settings = settings
        self.history = history
        locales: list[str] = settings.get(
            "detection.language_models", ["en_US", "ru_RU"]
        )
        self.models = {
            index: LanguageModel.load(locale)
            for index, locale in enumerate(locales[:2])
        }
        intent_model, self.intent_model_status = LinearNgramModel.try_load_default()
        self.detector = LanguageDetector(self.models, intent_model)
        self.backend: InputBackend = backend or _default_backend(len(self.models))
        self.backend_label = backend_label
        self.learning = learning or LearningStore(history.path.with_name("learning.json"))
        self._events: queue.Queue[KeyEvent | _LayoutSelection | None] = queue.Queue(
            maxsize=4096
        )
        self._worker: threading.Thread | None = None
        self._running = threading.Event()
        self._strokes: list[KeyEvent] = []
        self._source_group = -1
        self._last_word_input_at: float | None = None
        self._pause_correction_pending = False
        self._manual_layout_group: int | None = None
        self._pressed: set[int] = set()
        self._pressed_since: dict[int, float] = {}
        self._modifier_keycodes: set[int] = set()
        # Layout-dependent symbols typed right after a boundary (e.g. the RU
        # quote on Shift+2 meant as "@"); Pause converts them on their own.
        self._symbol_strokes: list[KeyEvent] = []
        # True once anything was typed after the last committed word, so Pause
        # must not rewrite that word any more.
        self._last_committed_stale = False
        self._early_switch_origin: int | None = None
        self._early_switch_at: float | None = None
        self._engine_switch_at: float | None = None
        self._engine_switch_group: int | None = None
        self._manual_layout_observed_at: float | None = None
        self._manual_layout_source = ""
        self._focus_window: int | None = None
        # Set while the user has undone an early switch of the word being
        # typed: that word is left alone even with manual layout respect off.
        self._early_switch_undone = False
        self._pause_deferral_logged = False
        # Built once per lexicon and cached process-wide, so the first early
        # switch decision does not stall the input thread.
        self._prefix_indexes = {
            group: PrefixIndex.for_language_model(model)
            for group, model in self.models.items()
        }
        self._pending: CorrectionPlan | None = None
        self._pending_trigger_keycode = -1
        self._last_committed: CorrectionPlan | None = None
        self._last_correction: CorrectionPlan | None = None
        self._pending_learning_action: tuple[str, int, str, int] | None = None
        self._learning_prompt: LearningPrompt | None = None
        self._learning_prompt_deadline: float | None = None
        self._contexts: dict[str, LanguageContext] = {}
        self._snapshot = EngineSnapshot(
            enabled=bool(settings.get("enabled", True)),
            correction_count=len(history.read()),
        )
        self._callbacks: list[Callable[[EngineSnapshot], None]] = []
        self._correction_callbacks: list[Callable[[CorrectionPlan], None]] = []
        self._learning_prompt_callbacks: list[
            Callable[[LearningPrompt | None], None]
        ] = []
        self._lock = threading.RLock()
        self.settings.subscribe(self._settings_changed)
        self._technical_session_event("engine_initialized")

    @property
    def snapshot(self) -> EngineSnapshot:
        with self._lock:
            return self._snapshot

    def subscribe(self, callback: Callable[[EngineSnapshot], None]) -> None:
        with self._lock:
            self._callbacks.append(callback)
        callback(self.snapshot)

    def subscribe_corrections(self, callback: Callable[[CorrectionPlan], None]) -> None:
        with self._lock:
            self._correction_callbacks.append(callback)

    @property
    def learning_prompt(self) -> LearningPrompt | None:
        with self._lock:
            return self._learning_prompt

    def subscribe_learning_prompts(
        self, callback: Callable[[LearningPrompt | None], None]
    ) -> None:
        with self._lock:
            self._learning_prompt_callbacks.append(callback)
            prompt = self._learning_prompt
        callback(prompt)

    def confirm_learning_prompt(
        self, prompt: LearningPrompt | None = None
    ) -> bool:
        with self._lock:
            current = self._learning_prompt
            if current is None or (prompt is not None and prompt != current):
                return False
            self._learning_prompt = None
            self._learning_prompt_deadline = None
            callbacks = tuple(self._learning_prompt_callbacks)
        required = int(self.settings.get("detection.learning_confirmations", 2))
        confirmations = self.learning.confirm_manual(
            current.source_group,
            current.original,
            current.target_group,
            required,
        )
        self._technical_event(
            "learning_prompt_confirmed",
            source_group=current.source_group,
            target_group=current.target_group,
            application=current.application,
            required_confirmations=required,
            confirmations=confirmations,
        )
        self._update(
            last_action=(
                f"{current.original} → {current.replacement} · правило выучено"
            )
        )
        for callback in callbacks:
            callback(None)
        return True

    def dismiss_learning_prompt(
        self, prompt: LearningPrompt | None = None, *, reason: str = "dismissed"
    ) -> bool:
        with self._lock:
            current = self._learning_prompt
            if current is None or (prompt is not None and prompt != current):
                return False
            self._learning_prompt = None
            self._learning_prompt_deadline = None
            callbacks = tuple(self._learning_prompt_callbacks)
        self._technical_event(
            "learning_prompt_dismissed",
            reason=reason,
            source_group=current.source_group,
            target_group=current.target_group,
            application=current.application,
        )
        for callback in callbacks:
            callback(None)
        return True

    def start(self) -> None:
        if self._running.is_set():
            return
        self._running.set()
        self._worker = threading.Thread(target=self._run, name="keyswitch-engine", daemon=True)
        self._worker.start()
        try:
            self.backend.start(self.enqueue)
            self._update(
                running=True,
                backend=self.backend_label,
                current_group=self.backend.current_group(),
                last_error="",
            )
        except Exception as error:
            self._running.clear()
            self._events.put(None)
            self._update(running=False, backend="недоступен", last_error=str(error))
            raise

    def stop(self) -> None:
        self.dismiss_learning_prompt()
        if not self._running.is_set():
            self.backend.close()
            return
        self._running.clear()
        self.backend.stop()
        try:
            self._events.put_nowait(None)
        except queue.Full:
            pass
        if self._worker and self._worker is not threading.current_thread():
            self._worker.join(timeout=2.0)
        self._worker = None
        self.backend.close()
        self._update(running=False, backend="остановлен", current_word="")

    def enqueue(self, event: KeyEvent) -> None:
        if event.synthetic:
            return
        try:
            self._events.put_nowait(event)
        except queue.Full:
            self._clear_word("Очередь ввода переполнена", reason="input_overflow")

    def select_alternate_group(self) -> bool:
        """Queue an explicit selection of the language opposite to the current one."""

        if not self._running.is_set():
            self._update(
                last_error="Движок раскладки не запущен",
                last_action="Язык из меню не переключён",
            )
            return False
        target = alternate_layout_group(self.snapshot.current_group)
        if target is None or target not in self.models:
            self._update(
                last_error="Текущая раскладка EN/RU не определена",
                last_action="Язык из меню не переключён",
            )
            return False
        try:
            self._events.put_nowait(_LayoutSelection(target))
        except queue.Full:
            self._update(
                last_error="Очередь ввода переполнена",
                last_action="Язык из меню не переключён",
            )
            return False
        return True

    def _run(self) -> None:
        while self._running.is_set():
            try:
                event = self._events.get(timeout=self._loop_timeout())
            except queue.Empty:
                self._poll_current_group()
                self._maybe_correct_after_pause()
                self._expire_learning_prompt()
                continue
            if event is None:
                break
            try:
                if isinstance(event, _LayoutSelection):
                    self._apply_layout_selection(event.group)
                else:
                    self._handle(event)
            except Exception as error:
                self._clear_word(reason="input_error")
                self._update(last_error=str(error), last_action="Ошибка обработки ввода")

    def _loop_timeout(self) -> float:
        """Wake exactly when the pause delay elapses, at most every 0.5 s."""

        last_input = self._last_word_input_at
        if not self._pause_correction_pending or last_input is None:
            return 0.5
        remaining = last_input + self._pause_delay() - time.monotonic()
        return max(0.01, min(0.5, remaining))

    def _pause_delay(self) -> float:
        try:
            delay = float(
                self.settings.get(
                    "detection.pause_delay_seconds", PAUSE_CORRECTION_DELAY_SECONDS
                )
            )
        except (TypeError, ValueError):
            delay = PAUSE_CORRECTION_DELAY_SECONDS
        return min(10.0, max(0.2, delay))

    def _apply_layout_selection(self, group: int) -> None:
        try:
            self.backend.switch_group(group)
        except Exception as error:
            self._technical_event(
                "layout_selection_failed",
                requested_group=group,
                error=str(error),
            )
            self._update(
                last_error=str(error),
                last_action="Язык из меню не переключён",
            )
            return
        self._clear_word(reason="layout_selected")
        self._last_committed_stale = True
        self._note_engine_switch(group)
        self._manual_layout_group = (
            group
            if bool(self.settings.get("detection.respect_manual_layout", True))
            else None
        )
        self._manual_layout_observed_at = time.monotonic()
        self._manual_layout_source = "menu"
        self._technical_event(
            "layout_selected_from_menu",
            selected_group=group,
            protects_next_word=self._manual_layout_group == group,
        )
        self._update(
            current_group=group,
            last_action=f"Язык выбран из меню: {layout_label(group)}",
            last_error="",
        )

    def _handle(self, event: KeyEvent) -> None:
        self._expire_learning_prompt()
        prompt = self.learning_prompt
        if prompt is not None and event.pressed and event.key_name not in MODIFIER_KEYS:
            unmodified = not (event.control or event.alt or event.super_key)
            if unmodified and event.key_name in {"Return", "KP_Enter"}:
                self.confirm_learning_prompt(prompt)
                return
            if unmodified and event.key_name == "Escape":
                self.dismiss_learning_prompt(prompt, reason="escape")
                return
            self.dismiss_learning_prompt(prompt, reason="other_key")
        # Only presses carry a meaningful group: a release reports whatever
        # layout was active when the finger came up, which is stale right
        # after the engine switched the layout itself.
        if event.pressed:
            # A key pressed before an early switch landed still reports the
            # old layout; that is a race, not the user switching back.
            if not self._late_stroke_after_early_switch(event):
                self._observe_group(event.group, source="keystroke")
            self._pressed.add(event.keycode)
            self._pressed_since[event.keycode] = time.monotonic()
        else:
            self._pressed.discard(event.keycode)
            self._pressed_since.pop(event.keycode, None)
        if event.key_name in MODIFIER_KEYS:
            if event.pressed:
                self._modifier_keycodes.add(event.keycode)
            else:
                self._modifier_keycodes.discard(event.keycode)
            self._maybe_execute_pending(event)
            return
        if not event.pressed:
            self._maybe_execute_pending(event)
            return
        if self._matches_hotkey("toggle", event):
            enabled = not bool(self.settings.get("enabled", True))
            self.settings.set("enabled", enabled)
            self._clear_word(
                "Автокоррекция включена" if enabled else "Автокоррекция на паузе",
                reason="engine_toggled",
            )
            return
        if self._matches_hotkey("convert_last", event):
            self._schedule_manual_conversion(event.keycode)
            return
        if self._matches_hotkey("undo", event):
            self._schedule_undo(event.keycode)
            return
        if event.control or event.alt or event.super_key:
            self._clear_word(reason="modifier_shortcut")
            return
        if event.key_name == "BackSpace":
            if self._strokes:
                self._strokes.pop()
                if self._strokes:
                    self._mark_word_activity()
                else:
                    self._source_group = -1
                    self._reset_pause_correction()
                    self._early_switch_origin = None
                    self._early_switch_at = None
                self._update(current_word=self._text_for_group(self._strokes, self._source_group))
            elif self._symbol_strokes:
                self._symbol_strokes.pop()
            else:
                self._last_committed_stale = True
            return
        if self._is_layout_letter(event):
            if not event.character.isalpha() and self._ambiguous_key_is_boundary(event):
                self._commit_word(event)
                return
            if self._late_stroke_after_early_switch(event):
                event = self._convert_late_stroke(event)
            if self._source_group not in (-1, event.group):
                self._clear_word(reason="layout_changed_mid_word")
            self._source_group = event.group
            self._strokes.append(event)
            self._mark_word_activity()
            self._update(
                current_group=event.group,
                current_word=self._text_for_group(self._strokes, event.group),
                last_error="",
            )
            self._maybe_early_switch()
            return
        if self._is_boundary(event):
            if self._strokes:
                self._commit_word(event)
                return
            # Nothing typed since the last boundary: remember layout-dependent
            # symbols so Pause converts just them (RU quote -> "@"), and never
            # rewrite the previous word after further input.
            if event.key_name in WORD_BOUNDARY_KEYS:
                self._symbol_strokes = []
            elif self._layout_dependent(event):
                self._symbol_strokes.append(event)
            self._last_committed_stale = True
            return
        if event.key_name in NAVIGATION_KEYS:
            # The caret moved: what was typed belongs to another position.
            self._last_committed_stale = True
            self._clear_word(reason="navigation")
            return
        if event.character:
            self._last_committed_stale = True
            if self._strokes:
                # A digit or another printable key inside a word ("зь2") stays
                # part of it, so Pause still converts the whole token. Nothing
                # changes for automatic correction: the detector treats a token
                # carrying a digit as code and leaves it alone.
                self._strokes.append(event)
                self._mark_word_activity()
                self._update(
                    current_word=self._text_for_group(self._strokes, self._source_group)
                )
                return
            if self._layout_dependent(event):
                # "@" typed in the US layout but meant as the RU quote.
                self._symbol_strokes.append(event)
                return
            self._clear_word(reason="non_word_key")

    @staticmethod
    def _layout_dependent(event: KeyEvent) -> bool:
        """A printable key whose character differs between the layouts."""

        return bool(event.character) and len(
            {character for character in event.characters if character}
        ) > 1

    def _early_switch_policy(self) -> EarlySwitchPolicy:
        try:
            minimum = int(self.settings.get("detection.early_switch_min_length", 4))
        except (TypeError, ValueError):
            minimum = 4
        return EarlySwitchPolicy(minimum_length=max(3, min(8, minimum)))

    def _maybe_early_switch(self) -> None:
        """Switch the layout as soon as the typed prefix proves it wrong."""

        if not bool(self.settings.get("detection.early_switch", True)):
            return
        if not bool(self.settings.get("enabled", True)):
            return
        if self._early_switch_origin is not None or self._pending is not None:
            return
        policy = self._early_switch_policy()
        if len(self._strokes) < policy.minimum_length:
            return
        source_group = self._source_group
        if source_group not in self.models:
            return
        if self._word_protected(source_group):
            return
        strokes = tuple(self._strokes)
        original = self._text_for_group(strokes, source_group)
        alternatives = {
            group: self._text_for_group(strokes, group)
            for group in self.models
            if group != source_group
        }
        decision = early_switch_decision(
            self._prefix_indexes,
            self.models,
            original,
            alternatives,
            source_group,
            policy=policy,
        )
        application = self.backend.active_application()
        excluded = self._application_excluded(application)
        if len(strokes) == policy.minimum_length or decision.should_switch:
            self._log_early_switch(decision, policy, application, excluded)
        if not decision.should_switch or excluded:
            return
        plan = CorrectionPlan(
            strokes,
            None,
            source_group,
            decision.target_group,
            original,
            decision.replacement,
            EARLY_SWITCH_CONFIDENCE,
            application,
            True,
            "early",
        )
        # The last letter's key is physically still down: a synthetic press of a
        # held key is ignored by the X server and the retyped letter would be
        # lost. Like boundary corrections, execute on that key's release and
        # absorb letters pressed before it (rollover typing).
        self._pending = plan
        self._pending_learning_action = None
        self._pending_trigger_keycode = strokes[-1].keycode
        self._technical_event(
            "early_switch_scheduled",
            trigger_keycode=strokes[-1].keycode,
            prefix_length=len(strokes),
        )

    def _refresh_early_plan(self, plan: CorrectionPlan) -> CorrectionPlan | None:
        """Extend a scheduled early switch with letters typed before release."""

        strokes = tuple(self._strokes)
        prefix = len(plan.strokes)
        if (
            len(strokes) < prefix
            or strokes[:prefix] != plan.strokes
            or self._source_group != plan.source_group
        ):
            return None
        if len(strokes) == prefix:
            return plan
        return CorrectionPlan(
            strokes,
            None,
            plan.source_group,
            plan.target_group,
            self._text_for_group(strokes, plan.source_group),
            self._text_for_group(strokes, plan.target_group),
            plan.confidence,
            plan.application,
            plan.automatic,
            plan.mode,
        )

    def _log_early_switch(
        self,
        decision: EarlySwitchDecision,
        policy: EarlySwitchPolicy,
        application: str,
        excluded: bool,
    ) -> None:
        payload = decision.as_dict()
        if excluded:
            payload["replacement"] = "<redacted>"
        self._technical_event(
            "early_switch_evaluation",
            original="<redacted>" if excluded else decision.original,
            prefix_length=len(decision.original),
            application=application,
            application_excluded=excluded,
            policy=policy.as_dict(),
            decision=payload,
        )

    def _late_stroke_after_early_switch(self, event: KeyEvent) -> bool:
        switched_at = self._early_switch_at
        return (
            switched_at is not None
            and event.group == self._early_switch_origin
            and event.group != self._source_group
            and time.monotonic() - switched_at <= LATE_STROKE_GRACE_SECONDS
        )

    def _convert_late_stroke(self, event: KeyEvent) -> KeyEvent:
        """Rewrite one letter that was pressed before the early switch landed."""

        target_group = self._source_group
        delay_ms = (
            None
            if self._early_switch_at is None
            else round((time.monotonic() - self._early_switch_at) * 1000)
        )
        try:
            self.backend.inject_correction((event,), target_group, None, event.group)
        except Exception as error:
            self._technical_event(
                "late_stroke_conversion_failed",
                source_group=event.group,
                target_group=target_group,
                delay_ms=delay_ms,
                error=str(error),
            )
            return event
        self._note_engine_switch(target_group)
        self._technical_event(
            "late_stroke_converted",
            source_group=event.group,
            target_group=target_group,
            delay_ms=delay_ms,
        )
        return KeyEvent(
            event.pressed,
            event.keycode,
            event.key_name,
            event.character_for(target_group),
            event.characters,
            target_group,
            event.state,
            event.timestamp,
            event.synthetic,
        )

    def _note_engine_switch(self, group: int) -> None:
        """Remember that the engine itself just switched the layout."""

        self._engine_switch_at = time.monotonic()
        self._engine_switch_group = group
        # The layout the engine chose supersedes the user's earlier manual
        # pick; otherwise that pick would revive when the engine returned to
        # its group minutes later.
        self._manual_layout_group = None

    def _word_protected(self, source_group: int) -> bool:
        """The current word must be neither corrected nor switched early."""

        if self._early_switch_undone:
            return True
        return (
            bool(self.settings.get("detection.respect_manual_layout", True))
            and self._manual_layout_group == source_group
        )

    def _protection_details(self) -> dict[str, object]:
        observed_at = self._manual_layout_observed_at
        return {
            "reason": (
                "early_switch_undone" if self._early_switch_undone else "manual_layout"
            ),
            "group": self._manual_layout_group,
            "source": self._manual_layout_source,
            "observed_ms_ago": (
                None
                if observed_at is None
                else round((time.monotonic() - observed_at) * 1000)
            ),
        }

    def _finish_early_switch(
        self,
        strokes: tuple[KeyEvent, ...],
        boundary: KeyEvent | None,
        application: str,
        final_group: int,
    ) -> None:
        """Record the completed word of an early switch as one correction."""

        origin = self._early_switch_origin
        self._early_switch_origin = None
        self._early_switch_at = None
        if origin is None:
            return
        original = self._text_for_group(strokes, origin)
        replacement = self._text_for_group(strokes, final_group)
        plan = CorrectionPlan(
            strokes,
            boundary,
            origin,
            final_group,
            original,
            replacement,
            EARLY_SWITCH_CONFIDENCE,
            application,
            True,
            "early",
        )
        self._last_correction = plan
        self._last_correction_time = time.monotonic()
        self._last_committed = plan
        self._last_committed_stale = False
        excluded = self._application_excluded(application)
        self._technical_event(
            "early_switch_completed",
            original="<redacted>" if excluded else original,
            replacement="<redacted>" if excluded else replacement,
            source_group=origin,
            target_group=final_group,
            application=application,
            application_excluded=excluded,
            word_length=len(strokes),
        )
        self._update(
            correction_count=self.snapshot.correction_count + 1,
            last_action=f"{original} → {replacement}",
        )
        if bool(self.settings.get("general.keep_history", True)):
            self.history.append(
                HistoryEntry.create(
                    original, replacement, application, EARLY_SWITCH_CONFIDENCE
                )
            )
        for callback in tuple(self._correction_callbacks):
            callback(plan)

    def _commit_word(self, boundary: KeyEvent) -> None:
        if not self._strokes:
            return
        strokes = tuple(self._strokes)
        self._reset_pause_correction()
        source_group = self._source_group
        original = self._text_for_group(strokes, source_group)
        alternatives = {
            group: self._text_for_group(strokes, group)
            for group in self.models
            if group != source_group
        }
        application = self.backend.active_application()
        context = self._context_for(application)
        plan = CorrectionPlan(
            strokes,
            boundary,
            source_group,
            next(iter(alternatives), source_group),
            original,
            next(iter(alternatives.values()), original),
            0.0,
            application,
            False,
        )
        self._last_committed = plan
        self._last_committed_stale = False
        self._symbol_strokes = []
        early_switch_origin = self._early_switch_origin
        manual_layout_selected = self._word_protected(source_group)
        # An explicit layout selection is the strongest available user intent.
        # It protects exactly one word even when an older learned rule exists.
        manual_layout_protected = manual_layout_selected
        protection = self._protection_details() if manual_layout_selected else None
        if manual_layout_selected:
            self._manual_layout_group = None
            self._early_switch_undone = False
        enabled = bool(self.settings.get("enabled", True))
        trigger_enabled = self._boundary_enabled(boundary)
        should_analyze = (
            enabled
            and trigger_enabled
            and not manual_layout_protected
        )
        excluded = self._application_excluded(application)
        decision: DetectionDecision | None = None
        if should_analyze and not excluded:
            decision = self._decide_word(
                original,
                alternatives,
                source_group,
                application,
                self._trigger_for_boundary(boundary),
            )
            if decision.should_convert:
                plan = self._plan_from_decision(strokes, boundary, application, decision)
                self._pending = plan
                self._pending_learning_action = None
                self._pending_trigger_keycode = boundary.keycode
            else:
                self._remember_context(application, source_group, strokes)
        else:
            self._remember_context(application, source_group, strokes)
        self._log_word_evaluation(
            trigger=self._trigger_for_boundary(boundary),
            original=original,
            alternatives=alternatives,
            application=application,
            enabled=enabled,
            trigger_enabled=trigger_enabled,
            manual_layout_protected=manual_layout_protected,
            application_excluded=excluded,
            decision=decision,
            protection=protection,
            source_group=source_group,
            early_switch_origin=early_switch_origin,
            context=context,
        )
        if decision is None or not decision.should_convert:
            self._finish_early_switch(strokes, boundary, application, source_group)
        else:
            self._early_switch_origin = None
            self._early_switch_at = None
        self._strokes = []
        self._source_group = -1
        self._early_switch_undone = False
        self._update(
            current_word="",
            current_group=boundary.group,
            last_action=(
                f"Ручная раскладка сохранена: {original}"
                if manual_layout_protected
                else None
            ),
        )

    @staticmethod
    def _is_layout_letter(event: KeyEvent) -> bool:
        return any(character.isalpha() for character in event.characters)

    def _ambiguous_key_is_boundary(self, event: KeyEvent) -> bool:
        """Resolve a key that is punctuation here but a letter in another layout.

        If the word accumulated before this key is already recognisable, the
        key is punctuation and can safely trigger a correction. Otherwise it is
        retained as a physical stroke (for example `,fpf` -> `база`).
        """

        if not self._strokes or self._source_group < 0:
            return False
        if event.character in {"'", "-"}:
            return False
        if any(
            not stroke.character.isalpha() and self._is_layout_letter(stroke)
            for stroke in self._strokes
        ):
            return False
        strokes = tuple(self._strokes)
        original = self._text_for_group(strokes, self._source_group)
        alternatives = {
            group: self._text_for_group(strokes, group)
            for group in self.models
            if group != self._source_group
        }
        application = self.backend.active_application()
        decision = self._decide_word(
            original,
            alternatives,
            self._source_group,
            application,
            "boundary_probe",
        )
        effective_length = max(
            len(LanguageModel.normalize(original)),
            *(len(LanguageModel.normalize(value)) for value in alternatives.values()),
        )
        protected_boundary = bool(
            self.settings.get("detection.protect_code", True)
        ) and self.detector.is_protected_token(original)
        ignored_words: list[str] = self.settings.get("exclusions.words", [])
        ignored_boundary = self.detector.token_key(original) in {
            self.detector.token_key(word)
            for word in ignored_words
        }
        natural_source_boundary = (
            effective_length >= 4 and decision.source_score.ngram_score >= -0.25
        )
        return decision.should_convert or protected_boundary or ignored_boundary or (
            decision.source_score.known
            and effective_length
            >= int(self.settings.get("detection.minimum_length", 3))
        ) or natural_source_boundary

    def _decide_word(
        self,
        original: str,
        alternatives: dict[int, str],
        source_group: int,
        application: str,
        trigger: CorrectionTrigger = "space",
    ) -> DetectionDecision:
        context_words, context_group = self._context_for(application)
        context_aware = bool(self.settings.get("detection.context_aware", True))
        ignored_words: list[str] = self.settings.get("exclusions.words", [])
        rejected_targets = (
            self.learning.rejected_targets(source_group, original)
            if bool(self.settings.get("detection.learning", True))
            else set()
        )
        protect_code = bool(self.settings.get("detection.protect_code", True))
        decision = self.detector.decide(
            original,
            alternatives,
            source_group,
            minimum_length=int(self.settings.get("detection.minimum_length", 3)),
            confidence_threshold=float(self.settings.get("detection.confidence", 2.0)),
            ignored_words=set(ignored_words),
            aggressive=bool(self.settings.get("detection.aggressive", False)),
            protect_code=protect_code,
            previous_words=context_words if context_aware else {},
            context_group=context_group if context_aware else None,
            forced_target_group=self._forced_target_group(source_group, original),
            rejected_targets=rejected_targets,
            trigger=trigger,
            use_intent_model=bool(
                self.settings.get("detection.intent_model_enabled", True)
            ),
        )
        if decision.should_convert:
            return decision
        short_decision = trusted_short_word_decision(
            self.detector,
            original,
            alternatives,
            source_group,
            ignored_words=ignored_words,
            rejected_targets=rejected_targets,
            protect_code=protect_code,
        )
        return decision if short_decision is None else short_decision

    def _forced_target_group(self, source_group: int, word: str) -> int | None:
        if not bool(self.settings.get("detection.learning", True)):
            return None
        confirmations = int(
            self.settings.get("detection.learning_confirmations", 2)
        )
        return self.learning.forced_target(source_group, word, confirmations)

    def _context_for(self, application: str) -> tuple[dict[int, str], int | None]:
        if not application.strip():
            return {}, None
        key = application.casefold()
        context = self._contexts.get(key)
        if context is None or time.monotonic() - context.updated_at > 45.0:
            return {}, None
        return dict(context.words), context.group

    def _remember_context(
        self,
        application: str,
        group: int,
        strokes: tuple[KeyEvent, ...] | list[KeyEvent],
    ) -> None:
        if group not in self.models or not strokes or not application.strip():
            return
        words = {
            candidate_group: self._text_for_group(strokes, candidate_group)
            for candidate_group in self.models
        }
        key = application.casefold()
        self._contexts.pop(key, None)
        self._contexts[key] = LanguageContext(group, words, time.monotonic())
        while len(self._contexts) > 32:
            self._contexts.pop(next(iter(self._contexts)))

    @staticmethod
    def _plan_from_decision(
        strokes: tuple[KeyEvent, ...],
        boundary: KeyEvent | None,
        application: str,
        decision: DetectionDecision,
        mode: str = "boundary",
    ) -> CorrectionPlan:
        return CorrectionPlan(
            strokes,
            boundary,
            decision.source_group,
            decision.target_group,
            decision.original,
            decision.replacement,
            decision.confidence,
            application,
            True,
            mode,
        )

    @staticmethod
    def _score_diagnostics(score: WordScore) -> dict[str, object]:
        return {
            "value": round(score.value, 6),
            "known": score.known,
            "frequency": score.frequency,
            "exact": score.exact,
            "spell_known": score.spell_known,
            "ngram_score": round(score.ngram_score, 6),
            "invalid_ratio": round(score.invalid_ratio, 6),
        }

    @classmethod
    def _decision_diagnostics(
        cls, decision: DetectionDecision
    ) -> dict[str, object]:
        return {
            "should_convert": decision.should_convert,
            "replacement": decision.replacement,
            "source_group": decision.source_group,
            "target_group": decision.target_group,
            "confidence": round(decision.confidence, 6),
            "reason": decision.reason,
            "source_score": cls._score_diagnostics(decision.source_score),
            "target_score": cls._score_diagnostics(decision.target_score),
            "model_probability": decision.model_probability,
            "model_threshold": decision.model_threshold,
            "model_version": decision.model_version,
        }

    def _technical_event(self, event: str, **fields: object) -> None:
        if not bool(self.settings.get("diagnostics.technical_logging", False)):
            return
        payload: dict[str, object] = {"schema": 1, "event": event}
        payload.update(fields)
        LOGGER.info(
            "TECHNICAL %s",
            json.dumps(
                payload,
                ensure_ascii=False,
                sort_keys=True,
                separators=(",", ":"),
                default=str,
            ),
        )

    def _technical_session_event(self, event: str) -> None:
        self._technical_event(
            event,
            keyswitch_version=__version__,
            backend=self.backend_label,
            intent_model=self.intent_model_status.as_dict(),
            language_models={
                str(group): {
                    "locale": model.locale,
                    "words": len(model.frequencies),
                    "source": model.source,
                }
                for group, model in self.models.items()
            },
            detection_settings={
                path: self.settings.get(f"detection.{path}")
                for path in (
                    "minimum_length",
                    "confidence",
                    "aggressive",
                    "protect_code",
                    "context_aware",
                    "respect_manual_layout",
                    "correct_on_space",
                    "correct_on_enter",
                    "correct_on_tab",
                    "correct_on_punctuation",
                    "correct_on_pause",
                    "pause_delay_seconds",
                    "early_switch",
                    "early_switch_min_length",
                    "learning",
                    "learning_confirmations",
                    "intent_model_enabled",
                )
            },
            hotkeys={
                name: self.settings.get(f"hotkeys.{name}")
                for name in ("toggle", "convert_last", "undo")
            },
        )

    def _log_word_evaluation(
        self,
        *,
        trigger: CorrectionTrigger,
        original: str,
        alternatives: dict[int, str],
        application: str,
        enabled: bool,
        trigger_enabled: bool,
        manual_layout_protected: bool,
        application_excluded: bool,
        decision: DetectionDecision | None,
        protection: dict[str, object] | None = None,
        source_group: int | None = None,
        early_switch_origin: int | None = None,
        idle_ms: int | None = None,
        context: tuple[dict[int, str], int | None] | None = None,
    ) -> None:
        if not bool(self.settings.get("diagnostics.technical_logging", False)):
            return
        decision_payload = (
            None if decision is None else self._decision_diagnostics(decision)
        )
        # Never put text typed inside an excluded application into the log,
        # even when detailed diagnostics are explicitly enabled.
        logged_original = "<redacted>" if application_excluded else original
        logged_alternatives: object = (
            {} if application_excluded else alternatives
        )
        if application_excluded and decision_payload is not None:
            decision_payload["replacement"] = "<redacted>"
        skipped_reason: str | None = None
        if application_excluded:
            skipped_reason = "application_excluded"
        elif not enabled:
            skipped_reason = "disabled"
        elif not trigger_enabled:
            skipped_reason = "trigger_disabled"
        elif manual_layout_protected:
            skipped_reason = "manual_layout_protected"
        # When the detector was not consulted, still record what it would have
        # said so that a missed correction can be told from a wrong verdict.
        shadow_payload: dict[str, object] | None = None
        if (
            decision is None
            and skipped_reason not in (None, "application_excluded")
            and source_group is not None
            and source_group in self.models
        ):
            shadow = self._decide_word(
                original, alternatives, source_group, application, trigger
            )
            shadow_payload = self._decision_diagnostics(shadow)
        context_words, context_group = (
            self._context_for(application) if context is None else context
        )
        self._technical_event(
            "word_evaluation",
            trigger=trigger,
            original=logged_original,
            alternatives=logged_alternatives,
            source_group=source_group,
            application=application,
            enabled=enabled,
            trigger_enabled=trigger_enabled,
            manual_layout_protected=manual_layout_protected,
            protection=protection,
            application_excluded=application_excluded,
            skipped_reason=skipped_reason,
            minimum_length=int(
                self.settings.get("detection.minimum_length", 3)
            ),
            confidence_threshold=float(
                self.settings.get("detection.confidence", 2.0)
            ),
            context={
                "group": context_group,
                "words": {} if application_excluded else context_words,
            },
            early_switch_origin=early_switch_origin,
            idle_ms=idle_ms,
            learning=self._learning_diagnostics(source_group, original),
            decision=decision_payload,
            shadow_decision=shadow_payload,
        )

    def _learning_diagnostics(
        self, source_group: int | None, word: str
    ) -> dict[str, object]:
        """What local learning knows about this word before the decision.

        A rule that has not reached the confirmation threshold changes nothing
        yet, so without these numbers a log line cannot be told apart from one
        where no rule exists at all.
        """

        if source_group is None:
            return {"enabled": bool(self.settings.get("detection.learning", True))}
        target, confirmations = self.learning.rule_state(source_group, word)
        return {
            "enabled": bool(self.settings.get("detection.learning", True)),
            "required_confirmations": int(
                self.settings.get("detection.learning_confirmations", 2)
            ),
            "rule_target": target,
            "confirmations": confirmations,
            "forced_target": self._forced_target_group(source_group, word),
            "rejected_targets": sorted(
                self.learning.rejected_targets(source_group, word)
            ),
        }

    def _log_word_discarded(self, reason: str) -> None:
        """Record a word that was thrown away before it could be corrected."""

        if not (self._strokes or self._symbol_strokes):
            return
        # Read the setting first: the application probe is a system call.
        if not bool(self.settings.get("diagnostics.technical_logging", False)):
            return
        application = self.backend.active_application()
        excluded = self._application_excluded(application)
        original = self._text_for_group(
            tuple(self._symbol_strokes) + tuple(self._strokes), self._source_group
        )
        self._technical_event(
            "word_discarded",
            reason=reason or "unspecified",
            original="<redacted>" if excluded else original,
            length=len(self._strokes),
            symbol_count=len(self._symbol_strokes),
            source_group=self._source_group,
            application=application,
            application_excluded=excluded,
        )

    def _mark_word_activity(self) -> None:
        self._last_word_input_at = time.monotonic()
        self._pause_correction_pending = True
        self._pause_deferral_logged = False

    def _reset_pause_correction(self) -> None:
        self._last_word_input_at = None
        self._pause_correction_pending = False

    def _maybe_correct_after_pause(self, *, now: float | None = None) -> None:
        if not self._pause_correction_pending:
            return
        if not bool(self.settings.get("detection.correct_on_pause", True)):
            self._reset_pause_correction()
            return
        if not bool(self.settings.get("enabled", True)):
            self._reset_pause_correction()
            return
        last_input = self._last_word_input_at
        if last_input is None:
            self._reset_pause_correction()
            return
        current_time = time.monotonic() if now is None else now
        idle_ms = round((current_time - last_input) * 1000)
        if current_time - last_input < self._pause_delay():
            return
        self._prune_stale_presses(current_time)
        deferral: str | None = None
        if self._pressed:
            deferral = "keys_pressed"
        elif self._modifier_keycodes:
            deferral = "modifiers_pressed"
        elif self._pending is not None:
            deferral = "correction_pending"
        if deferral is not None:
            if not self._pause_deferral_logged:
                self._pause_deferral_logged = True
                self._technical_event(
                    "pause_correction_deferred",
                    reason=deferral,
                    idle_ms=idle_ms,
                    pressed_keycodes=sorted(self._pressed),
                    modifier_keycodes=sorted(self._modifier_keycodes),
                )
            return

        self._pause_correction_pending = False
        strokes = tuple(self._strokes)
        source_group = self._source_group
        manual_layout_selected = self._word_protected(source_group)
        original = self._text_for_group(strokes, source_group)
        alternatives = {
            group: self._text_for_group(strokes, group)
            for group in self.models
            if group != source_group
        }
        application = self.backend.active_application()
        excluded = self._application_excluded(application)
        if manual_layout_selected:
            self._log_word_evaluation(
                trigger="pause",
                original=original,
                alternatives=alternatives,
                application=application,
                enabled=True,
                trigger_enabled=True,
                manual_layout_protected=True,
                application_excluded=excluded,
                decision=None,
                protection=self._protection_details(),
                source_group=source_group,
                early_switch_origin=self._early_switch_origin,
                idle_ms=idle_ms,
            )
            return
        if excluded:
            self._log_word_evaluation(
                trigger="pause",
                original=original,
                alternatives=alternatives,
                application=application,
                enabled=True,
                trigger_enabled=True,
                manual_layout_protected=False,
                application_excluded=True,
                decision=None,
                source_group=source_group,
                early_switch_origin=self._early_switch_origin,
                idle_ms=idle_ms,
            )
            return
        decision = self._decide_word(
            original, alternatives, source_group, application, "pause"
        )
        self._log_word_evaluation(
            trigger="pause",
            original=original,
            alternatives=alternatives,
            application=application,
            enabled=True,
            trigger_enabled=True,
            manual_layout_protected=False,
            application_excluded=False,
            decision=decision,
            source_group=source_group,
            early_switch_origin=self._early_switch_origin,
            idle_ms=idle_ms,
        )
        if not decision.should_convert:
            return

        self._early_switch_origin = None
        self._early_switch_at = None
        plan = self._plan_from_decision(strokes, None, application, decision, "pause")
        self._strokes = []
        self._source_group = -1
        self._early_switch_undone = False
        self._reset_pause_correction()
        self._update(current_word="")
        self._execute_correction(plan, None)

    def _prune_stale_presses(self, now: float) -> None:
        """Forget presses whose release was never delivered (focus changes)."""

        stale = [
            keycode
            for keycode, since in self._pressed_since.items()
            if now - since > STALE_PRESS_SECONDS
        ]
        if not stale:
            return
        for keycode in stale:
            self._pressed_since.pop(keycode, None)
            self._pressed.discard(keycode)
            self._modifier_keycodes.discard(keycode)
        self._technical_event("stale_presses_pruned", keycodes=sorted(stale))

    def _schedule_manual_conversion(self, trigger_keycode: int) -> None:
        """Pause: convert what was typed since the last boundary, or switch."""

        mode = "manual"
        learn = True
        if self._strokes:
            strokes = tuple(self._symbol_strokes) + tuple(self._strokes)
            source_group = self._source_group
            boundary = None
            application = self.backend.active_application()
            source = "current_word" if not self._symbol_strokes else "symbols_and_word"
            self._early_switch_origin = None
            self._early_switch_at = None
        elif self._symbol_strokes:
            strokes = tuple(self._symbol_strokes)
            source_group = self._symbol_strokes[-1].group
            boundary = None
            application = self.backend.active_application()
            source = "symbols"
            mode = "symbols"
            learn = False
        elif self._last_committed is not None and not self._last_committed_stale:
            strokes = self._last_committed.strokes
            source_group = self._last_committed.source_group
            boundary = self._last_committed.boundary
            application = self._last_committed.application
            source = "last_committed"
        else:
            self._switch_layout_only(trigger_keycode)
            return
        targets = [group for group in self.models if group != source_group]
        if not targets:
            return
        target = targets[0]
        original = self._text_for_group(strokes, source_group)
        replacement = self._text_for_group(strokes, target)
        learn = learn and self._learnable(replacement)
        self._pending = CorrectionPlan(
            strokes,
            boundary,
            source_group,
            target,
            original,
            replacement,
            99.0,
            application,
            False,
            mode,
        )
        self._pending_learning_action = (
            ("manual", source_group, original, target) if learn else None
        )
        self._pending_trigger_keycode = trigger_keycode
        excluded = self._application_excluded(application)
        self._technical_event(
            "manual_conversion_scheduled",
            source=source,
            original="<redacted>" if excluded else original,
            replacement="<redacted>" if excluded else replacement,
            source_group=source_group,
            target_group=target,
            application=application,
            application_excluded=excluded,
            symbol_count=len(self._symbol_strokes),
            learnable=learn,
        )
        self._strokes = []
        self._symbol_strokes = []
        self._source_group = -1
        self._early_switch_undone = False
        self._last_committed_stale = True
        self._reset_pause_correction()

    @staticmethod
    def _learnable(replacement: str) -> bool:
        """Only something that reads as a word may become a rule.

        A lone letter converted to punctuation ("б" -> ",") or a run of
        symbols must never be offered for learning, let alone be counted as a
        confirmation towards an automatic rule.
        """

        letters = sum(1 for character in replacement if character.isalpha())
        return letters >= 2 and all(
            character.isalpha() or character in "'-" for character in replacement
        )

    def _switch_layout_only(self, trigger_keycode: int) -> None:
        """Pause with nothing to convert just toggles the layout."""

        current = self.snapshot.current_group
        target = alternate_layout_group(current)
        if target is None or target not in self.models:
            self._update(last_action="Нет слова для ручного преобразования")
            return
        try:
            self.backend.switch_group(target)
        except Exception as error:
            self._technical_event(
                "layout_switch_failed",
                source="convert_last",
                requested_group=target,
                error=str(error),
            )
            self._update(last_error=str(error), last_action="Раскладка не переключена")
            return
        self._note_engine_switch(target)
        protects = bool(self.settings.get("detection.respect_manual_layout", True))
        self._manual_layout_group = target if protects else None
        self._manual_layout_observed_at = time.monotonic()
        self._manual_layout_source = "convert_last"
        self._technical_event(
            "layout_switched_without_word",
            trigger_keycode=trigger_keycode,
            previous_group=current,
            selected_group=target,
            protects_next_word=protects,
            last_committed_stale=self._last_committed_stale,
        )
        self._update(
            current_group=target,
            last_action=f"Раскладка переключена: {layout_label(target)}",
            last_error="",
        )

    def _schedule_early_switch_undo(self, origin: int, trigger_keycode: int) -> None:
        """Undo hotkey while an early-switched word is still being typed.

        The prefix is not a finished correction yet, so the generic undo would
        revert the *previous* correction and delete the wrong characters. This
        returns the prefix to the layout the user typed it in and protects
        the rest of the word from being switched again.
        """

        strokes = tuple(self._strokes)
        current_group = self._source_group
        self._pending = CorrectionPlan(
            strokes,
            None,
            current_group,
            origin,
            self._text_for_group(strokes, current_group),
            self._text_for_group(strokes, origin),
            99.0,
            self.backend.active_application(),
            False,
            "early_undo",
        )
        self._pending_learning_action = None
        self._pending_trigger_keycode = trigger_keycode
        self._technical_event(
            "early_switch_undo_scheduled",
            source_group=current_group,
            target_group=origin,
            prefix_length=len(strokes),
        )

    def _schedule_undo(self, trigger_keycode: int) -> None:
        early_origin = self._early_switch_origin
        if early_origin is not None and self._strokes:
            self._schedule_early_switch_undo(early_origin, trigger_keycode)
            return
        previous = self._last_correction
        if previous is None or time.monotonic() - getattr(self, "_last_correction_time", 0.0) > 10.0:
            self._update(last_action="Последнее исправление уже нельзя отменить")
            return
        self._pending = CorrectionPlan(
            previous.strokes,
            previous.boundary,
            previous.target_group,
            previous.source_group,
            previous.replacement,
            previous.original,
            99.0,
            previous.application,
            False,
            "undo",
        )
        self._pending_learning_action = (
            (
                "reject",
                previous.source_group,
                previous.original,
                previous.target_group,
            )
            if previous.automatic
            else None
        )
        self._pending_trigger_keycode = trigger_keycode

    def _maybe_execute_pending(self, event: KeyEvent) -> None:
        if self._pending is None:
            return
        if event.keycode == self._pending_trigger_keycode and not event.pressed:
            self._pending_trigger_keycode = -1
        if self._pending_trigger_keycode != -1 or self._modifier_keycodes:
            return
        plan, self._pending = self._pending, None
        learning_action, self._pending_learning_action = self._pending_learning_action, None
        if plan.mode in ("early", "early_undo"):
            refreshed = self._refresh_early_plan(plan)
            if refreshed is None:
                self._technical_event(
                    "early_switch_dropped",
                    reason="word_changed_before_release",
                    current_word_length=len(self._strokes),
                )
                return
            plan = refreshed
        self._execute_correction(plan, learning_action)

    def _execute_correction(
        self,
        plan: CorrectionPlan,
        learning_action: tuple[str, int, str, int] | None,
    ) -> None:
        application_excluded = self._application_excluded(plan.application)
        logged_original = "<redacted>" if application_excluded else plan.original
        logged_replacement = (
            "<redacted>" if application_excluded else plan.replacement
        )
        previous_group = self.snapshot.current_group
        started = time.monotonic()
        try:
            self.backend.inject_correction(
                plan.strokes,
                plan.target_group,
                plan.boundary,
                plan.source_group,
            )
        except Exception as error:
            self._technical_event(
                "correction_failed",
                mode=plan.mode,
                original=logged_original,
                replacement=logged_replacement,
                source_group=plan.source_group,
                target_group=plan.target_group,
                application=plan.application,
                application_excluded=application_excluded,
                automatic=plan.automatic,
                error=str(error),
            )
            self._update(last_error=str(error), last_action="Исправление не выполнено")
            return
        self._note_engine_switch(plan.target_group)
        self._technical_event(
            "correction_applied",
            mode=plan.mode,
            original=logged_original,
            replacement=logged_replacement,
            source_group=plan.source_group,
            target_group=plan.target_group,
            previous_group=previous_group,
            layout_switched=plan.source_group != plan.target_group,
            deleted_characters=len(plan.strokes) + (0 if plan.boundary is None else 1),
            injection_ms=round((time.monotonic() - started) * 1000),
            application=plan.application,
            application_excluded=application_excluded,
            automatic=plan.automatic,
            confidence=round(plan.confidence, 6),
            boundary=(None if plan.boundary is None else plan.boundary.key_name),
        )
        if plan.mode == "early":
            # The prefix is finished later; only then does it become a
            # correction that can be undone or listed in the history.
            self._early_switch_origin = plan.source_group
            self._early_switch_at = time.monotonic()
            self._source_group = plan.target_group
            self._update(
                current_group=plan.target_group,
                current_word=self._text_for_group(self._strokes, plan.target_group),
                last_error="",
            )
            return
        if plan.mode == "early_undo":
            self._early_switch_origin = None
            self._early_switch_at = None
            self._source_group = plan.target_group
            self._early_switch_undone = True
            self._manual_layout_group = (
                plan.target_group
                if bool(self.settings.get("detection.respect_manual_layout", True))
                else None
            )
            self._manual_layout_observed_at = time.monotonic()
            self._manual_layout_source = "early_undo"
            self._update(
                current_group=plan.target_group,
                current_word=self._text_for_group(self._strokes, plan.target_group),
                last_action=(
                    f"{plan.original} → {plan.replacement}"
                    " · раннее переключение отменено"
                ),
                last_error="",
            )
            return
        self._last_correction = plan
        self._last_correction_time = time.monotonic()
        if plan.mode == "symbols":
            self._update(
                current_group=plan.target_group,
                last_action=f"{plan.original} → {plan.replacement}",
                last_error="",
            )
            for callback in tuple(self._correction_callbacks):
                callback(plan)
            return
        # Pause right after a correction converts the same word back.
        self._last_committed = CorrectionPlan(
            plan.strokes,
            plan.boundary,
            plan.target_group,
            plan.source_group,
            plan.replacement,
            plan.original,
            plan.confidence,
            plan.application,
            False,
        )
        self._last_committed_stale = False
        self._remember_context(plan.application, plan.target_group, plan.strokes)
        learned_rule = False
        rejected_rule = False
        learning_prompt: LearningPrompt | None = None
        if learning_action is not None and bool(self.settings.get("detection.learning", True)):
            action, source_group, word, target_group = learning_action
            excluded = self._application_excluded(plan.application)
            if action == "manual":
                confirmations = self.learning.record_manual(
                    source_group, word, target_group
                )
                required = int(self.settings.get("detection.learning_confirmations", 2))
                learned_rule = confirmations >= required
                self._technical_event(
                    "learning_rule_recorded",
                    word="<redacted>" if excluded else word,
                    source_group=source_group,
                    target_group=target_group,
                    confirmations=confirmations,
                    required_confirmations=required,
                    active=learned_rule,
                    application=plan.application,
                    application_excluded=excluded,
                )
                if not learned_rule:
                    learning_prompt = LearningPrompt(
                        source_group,
                        target_group,
                        plan.original,
                        plan.replacement,
                        plan.application,
                    )
            elif action == "reject":
                self.learning.reject(source_group, word, target_group)
                rejected_rule = True
                self._technical_event(
                    "learning_rejection_recorded",
                    word="<redacted>" if excluded else word,
                    source_group=source_group,
                    target_group=target_group,
                    application=plan.application,
                    application_excluded=excluded,
                )
        count = self.snapshot.correction_count + (1 if plan.automatic else 0)
        action = f"{plan.original} → {plan.replacement}"
        if learned_rule:
            action += " · правило выучено"
        elif rejected_rule:
            action += " · ложное срабатывание запомнено"
        self._update(
            current_group=plan.target_group,
            correction_count=count,
            last_action=action,
            last_error="",
        )
        if plan.automatic and bool(self.settings.get("general.keep_history", True)):
            self.history.append(
                HistoryEntry.create(
                    plan.original, plan.replacement, plan.application, plan.confidence
                )
            )
        for callback in tuple(self._correction_callbacks):
            callback(plan)
        if learning_prompt is not None:
            self._show_learning_prompt(learning_prompt)

    def _show_learning_prompt(self, prompt: LearningPrompt) -> None:
        with self._lock:
            self._learning_prompt = prompt
            self._learning_prompt_deadline = (
                time.monotonic() + LEARNING_PROMPT_TIMEOUT_SECONDS
            )
            callbacks = tuple(self._learning_prompt_callbacks)
        excluded = self._application_excluded(prompt.application)
        self._technical_event(
            "learning_prompt_shown",
            original="<redacted>" if excluded else prompt.original,
            replacement="<redacted>" if excluded else prompt.replacement,
            source_group=prompt.source_group,
            target_group=prompt.target_group,
            application=prompt.application,
            timeout_seconds=LEARNING_PROMPT_TIMEOUT_SECONDS,
        )
        for callback in callbacks:
            callback(prompt)

    def _expire_learning_prompt(self, *, now: float | None = None) -> bool:
        with self._lock:
            deadline = self._learning_prompt_deadline
        if deadline is None:
            return False
        current_time = time.monotonic() if now is None else now
        if current_time < deadline:
            return False
        return self.dismiss_learning_prompt(reason="timeout")

    def _matches_hotkey(self, name: str, event: KeyEvent) -> bool:
        return Hotkey(str(self.settings.get(f"hotkeys.{name}", ""))).matches(event)

    def _is_boundary(self, event: KeyEvent) -> bool:
        return (
            event.key_name in {"space", "Return", "Tab", "ISO_Left_Tab"}
            or event.character in PUNCTUATION
        )

    def _boundary_enabled(self, event: KeyEvent) -> bool:
        if event.key_name == "space":
            return bool(self.settings.get("detection.correct_on_space", True))
        if event.key_name == "Return":
            return bool(self.settings.get("detection.correct_on_enter", True))
        if event.key_name in {"Tab", "ISO_Left_Tab"}:
            return bool(self.settings.get("detection.correct_on_tab", True))
        return bool(self.settings.get("detection.correct_on_punctuation", True))

    @staticmethod
    def _trigger_for_boundary(event: KeyEvent) -> CorrectionTrigger:
        if event.key_name == "space":
            return "space"
        if event.key_name == "Return":
            return "enter"
        if event.key_name in {"Tab", "ISO_Left_Tab"}:
            return "tab"
        return "punctuation"

    def _application_excluded(self, application: str) -> bool:
        normalized = application.casefold()
        applications: list[str] = self.settings.get(
            "exclusions.applications", []
        )
        return any(
            item.casefold() in normalized
            for item in applications
            if item.strip()
        )

    @staticmethod
    def _text_for_group(strokes: list[KeyEvent] | tuple[KeyEvent, ...], group: int) -> str:
        return "".join(stroke.character_for(group) for stroke in strokes)

    def _clear_word(self, action: str | None = None, *, reason: str = "") -> None:
        self._log_word_discarded(reason)
        self.dismiss_learning_prompt(reason="word_cleared")
        if self._early_switch_origin is not None and self._strokes:
            # The rewritten prefix stays on screen: record it so that the
            # history lists it and the undo hotkey can still revert it.
            self._finish_early_switch(
                tuple(self._strokes),
                None,
                self.backend.active_application(),
                self._source_group,
            )
            self._last_committed_stale = True
        self._early_switch_undone = False
        self._strokes = []
        self._symbol_strokes = []
        self._source_group = -1
        self._early_switch_origin = None
        self._early_switch_at = None
        self._reset_pause_correction()
        self._pending = None
        self._pending_learning_action = None
        if action is None:
            self._update(current_word="")
        else:
            self._update(current_word="", last_action=action)

    def _settings_changed(self, path: str, value: object) -> None:
        if path == "*":
            self.dismiss_learning_prompt(reason="settings_reloaded")
            self._update(enabled=bool(self.settings.get("enabled", True)))
            self._manual_layout_group = None
        elif path == "enabled":
            self._update(enabled=bool(value))
        elif path == "detection.respect_manual_layout" and not bool(value):
            self._manual_layout_group = None
        elif path == "detection.learning" and not bool(value):
            self.dismiss_learning_prompt(reason="learning_disabled")
        self._technical_event(
            "setting_changed", path=path, value=self._loggable_setting(path, value)
        )
        if path == "diagnostics.technical_logging" and bool(value):
            self._technical_session_event("technical_logging_enabled")

    @staticmethod
    def _loggable_setting(path: str, value: object) -> object:
        if path == "*":
            return "<all>"
        if isinstance(value, (bool, int, float)) or value is None:
            return value
        if isinstance(value, str):
            return value if len(value) <= 80 else value[:77] + "..."
        if isinstance(value, (list, tuple, set, dict)):
            return {"type": type(value).__name__, "items": len(value)}
        return type(value).__name__

    def _track_focus(self) -> _FocusChange:
        """Notice the user moving to another window."""

        focus = self.backend.focused_window()
        if focus is None or not focus.window:
            return _FocusChange(False, False)
        if focus.own:
            return _FocusChange(False, focus.isolated_layout)
        previous = self._focus_window
        self._focus_window = focus.window
        if previous is None or previous == focus.window:
            return _FocusChange(False, False)
        self._focus_changed(previous, focus.window)
        return _FocusChange(True, False)

    def _focus_changed(self, previous: int, window: int) -> None:
        """The unfinished word and the last committed one stay in the old window."""

        dropped = len(self._strokes)
        if self._strokes or self._symbol_strokes:
            self._clear_word(reason="focus_changed")
        self._last_committed_stale = True
        self._technical_event(
            "focus_changed",
            previous_window=previous,
            window=window,
            dropped_word_length=dropped,
        )

    def _observe_group(self, group: int, *, source: str = "poll") -> None:
        focus = self._track_focus()
        current_group = self.snapshot.current_group
        if not 0 <= group < len(self.models) or group == current_group:
            return
        if focus.ignore_layout:
            # Windows keeps a layout per window: the settings window or the
            # learning prompt of KeySwitch itself says nothing about the
            # layout the user types in, so it neither protects nor updates.
            self._technical_event(
                "layout_change_ignored",
                source=source,
                reason="own_window",
                previous_group=current_group,
                selected_group=group,
            )
            return
        switched_at = self._engine_switch_at
        engine_switch_ms = (
            None
            if switched_at is None
            else round((time.monotonic() - switched_at) * 1000)
        )
        # Only a change *to* the layout the engine itself just selected is the
        # engine's own switch; the user switching away right after a wrong
        # correction is manual and must protect the retyped word.
        initiated_by_engine = (
            engine_switch_ms is not None
            and engine_switch_ms <= ENGINE_SWITCH_GRACE_SECONDS * 1000
            and group == self._engine_switch_group
        )
        application = self.backend.active_application()
        respect = bool(self.settings.get("detection.respect_manual_layout", True))
        # A layout that arrived together with another window is that window's
        # own layout (Windows keeps one per window), not a choice of the user.
        protects = (
            current_group >= 0
            and respect
            and not initiated_by_engine
            and not focus.changed
        )
        self._technical_event(
            "layout_change_observed" if not protects else "manual_layout_observed",
            source=source,
            previous_group=current_group,
            selected_group=group,
            application=application,
            initiated_by_engine=initiated_by_engine,
            focus_changed=focus.changed,
            engine_switch_ms_ago=engine_switch_ms,
            respect_manual_layout=respect,
            protects_next_word=protects,
            current_word_length=len(self._strokes),
        )
        if focus.changed:
            # The manual pick belonged to the previous window.
            self._manual_layout_group = None
        if protects:
            self._manual_layout_group = group
            self._manual_layout_observed_at = time.monotonic()
            self._manual_layout_source = source
            self._update(
                current_group=group,
                last_action=(
                    "Ручная смена раскладки · следующее слово без автокоррекции"
                ),
            )
            return
        self._update(current_group=group)

    def _poll_current_group(self) -> None:
        self._observe_group(self.backend.current_group(), source="poll")

    def _update(
        self,
        *,
        running: bool | None = None,
        enabled: bool | None = None,
        backend: str | None = None,
        current_group: int | None = None,
        current_word: str | None = None,
        correction_count: int | None = None,
        last_action: str | None = None,
        last_error: str | None = None,
    ) -> None:
        with self._lock:
            current = self._snapshot
            self._snapshot = EngineSnapshot(
                running=current.running if running is None else running,
                enabled=current.enabled if enabled is None else enabled,
                backend=current.backend if backend is None else backend,
                current_group=(
                    current.current_group if current_group is None else current_group
                ),
                current_word=(
                    current.current_word if current_word is None else current_word
                ),
                correction_count=(
                    current.correction_count
                    if correction_count is None
                    else correction_count
                ),
                last_action=(
                    current.last_action if last_action is None else last_action
                ),
                last_error=current.last_error if last_error is None else last_error,
            )
            callbacks = tuple(self._callbacks)
            snapshot = self._snapshot
        for callback in callbacks:
            callback(snapshot)

"""Keyboard event state machine and correction orchestration."""

from __future__ import annotations

import queue
import threading
import time
from dataclasses import dataclass, replace
from typing import Callable

from .config import SettingsStore
from .detector import DetectionDecision, LanguageDetector
from .history import HistoryEntry, HistoryStore
from .language_model import LanguageModel
from .x11_backend import KeyEvent, X11Backend


MODIFIER_KEYS = {
    "Shift_L", "Shift_R", "Control_L", "Control_R", "Alt_L", "Alt_R",
    "Meta_L", "Meta_R", "Super_L", "Super_R", "ISO_Level3_Shift",
}
NAVIGATION_KEYS = {
    "Left", "Right", "Up", "Down", "Home", "End", "Page_Up", "Page_Down",
    "Escape", "Delete", "Insert",
}
PUNCTUATION = set(".,!?;:()[]{}—–-…\"«»")


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
        backend: X11Backend | None = None,
    ) -> None:
        self.settings = settings
        self.history = history
        locales = settings.get("detection.language_models", ["en_US", "ru_RU"])
        self.models = {
            index: LanguageModel.load(locale)
            for index, locale in enumerate(locales[:2])
        }
        self.detector = LanguageDetector(self.models)
        self.backend = backend or X11Backend(group_count=len(self.models))
        self._events: queue.Queue[KeyEvent | None] = queue.Queue(maxsize=4096)
        self._worker: threading.Thread | None = None
        self._running = threading.Event()
        self._strokes: list[KeyEvent] = []
        self._source_group = -1
        self._pressed: set[int] = set()
        self._modifier_keycodes: set[int] = set()
        self._pending: CorrectionPlan | None = None
        self._pending_trigger_keycode = -1
        self._last_committed: CorrectionPlan | None = None
        self._last_correction: CorrectionPlan | None = None
        self._snapshot = EngineSnapshot(
            enabled=bool(settings.get("enabled", True)),
            correction_count=len(history.read()),
        )
        self._callbacks: list[Callable[[EngineSnapshot], None]] = []
        self._correction_callbacks: list[Callable[[CorrectionPlan], None]] = []
        self._lock = threading.RLock()
        self.settings.subscribe(self._settings_changed)

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
                backend="X11 RECORD + XTEST",
                current_group=self.backend.current_group(),
                last_error="",
            )
        except Exception as error:
            self._running.clear()
            self._events.put(None)
            self._update(running=False, backend="недоступен", last_error=str(error))
            raise

    def stop(self) -> None:
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
            self._clear_word("Очередь ввода переполнена")

    def _run(self) -> None:
        while self._running.is_set():
            try:
                event = self._events.get(timeout=0.5)
            except queue.Empty:
                continue
            if event is None:
                break
            try:
                self._handle(event)
            except Exception as error:
                self._clear_word()
                self._update(last_error=str(error), last_action="Ошибка обработки ввода")

    def _handle(self, event: KeyEvent) -> None:
        if event.pressed:
            self._pressed.add(event.keycode)
        else:
            self._pressed.discard(event.keycode)
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
            self._clear_word("Автокоррекция включена" if enabled else "Автокоррекция на паузе")
            return
        if self._matches_hotkey("convert_last", event):
            self._schedule_manual_conversion(event.keycode)
            return
        if self._matches_hotkey("undo", event):
            self._schedule_undo(event.keycode)
            return
        if event.control or event.alt or event.super_key:
            self._clear_word()
            return
        if event.key_name == "BackSpace":
            if self._strokes:
                self._strokes.pop()
                self._update(current_word=self._text_for_group(self._strokes, self._source_group))
            return
        if event.character and event.character.isalpha():
            if self._source_group not in (-1, event.group):
                self._clear_word()
            self._source_group = event.group
            self._strokes.append(event)
            self._update(
                current_group=event.group,
                current_word=self._text_for_group(self._strokes, event.group),
                last_error="",
            )
            return
        if self._is_boundary(event):
            self._commit_word(event)
            return
        if event.key_name in NAVIGATION_KEYS or event.character:
            self._clear_word()

    def _commit_word(self, boundary: KeyEvent) -> None:
        if not self._strokes:
            return
        strokes = tuple(self._strokes)
        source_group = self._source_group
        original = self._text_for_group(strokes, source_group)
        alternatives = {
            group: self._text_for_group(strokes, group)
            for group in self.models
            if group != source_group
        }
        application = self.backend.active_application()
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
        should_analyze = bool(self.settings.get("enabled", True)) and self._boundary_enabled(boundary)
        if should_analyze and not self._application_excluded(application):
            decision = self.detector.decide(
                original,
                alternatives,
                source_group,
                minimum_length=int(self.settings.get("detection.minimum_length", 3)),
                confidence_threshold=float(self.settings.get("detection.confidence", 2.0)),
                ignored_words=set(self.settings.get("exclusions.words", [])),
                aggressive=bool(self.settings.get("detection.aggressive", False)),
            )
            if decision.should_convert:
                plan = self._plan_from_decision(strokes, boundary, application, decision)
                self._pending = plan
                self._pending_trigger_keycode = boundary.keycode
        self._strokes = []
        self._source_group = -1
        self._update(current_word="", current_group=boundary.group)

    @staticmethod
    def _plan_from_decision(
        strokes: tuple[KeyEvent, ...],
        boundary: KeyEvent,
        application: str,
        decision: DetectionDecision,
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
        )

    def _schedule_manual_conversion(self, trigger_keycode: int) -> None:
        if self._strokes:
            strokes = tuple(self._strokes)
            source_group = self._source_group
            boundary = None
            application = self.backend.active_application()
        elif self._last_committed is not None:
            strokes = self._last_committed.strokes
            source_group = self._last_committed.source_group
            boundary = self._last_committed.boundary
            application = self._last_committed.application
        else:
            self._update(last_action="Нет слова для ручного преобразования")
            return
        targets = [group for group in self.models if group != source_group]
        if not targets:
            return
        target = targets[0]
        self._pending = CorrectionPlan(
            strokes,
            boundary,
            source_group,
            target,
            self._text_for_group(strokes, source_group),
            self._text_for_group(strokes, target),
            99.0,
            application,
            False,
        )
        self._pending_trigger_keycode = trigger_keycode
        self._strokes = []
        self._source_group = -1

    def _schedule_undo(self, trigger_keycode: int) -> None:
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
        try:
            self.backend.inject_correction(plan.strokes, plan.target_group, plan.boundary)
        except Exception as error:
            self._update(last_error=str(error), last_action="Исправление не выполнено")
            return
        self._last_correction = plan
        self._last_correction_time = time.monotonic()
        count = self.snapshot.correction_count + (1 if plan.automatic else 0)
        action = f"{plan.original} → {plan.replacement}"
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

    def _application_excluded(self, application: str) -> bool:
        normalized = application.casefold()
        return any(
            item.casefold() in normalized
            for item in self.settings.get("exclusions.applications", [])
            if item.strip()
        )

    @staticmethod
    def _text_for_group(strokes: list[KeyEvent] | tuple[KeyEvent, ...], group: int) -> str:
        return "".join(stroke.character_for(group) for stroke in strokes)

    def _clear_word(self, action: str | None = None) -> None:
        self._strokes = []
        self._source_group = -1
        self._pending = None
        updates: dict[str, object] = {"current_word": ""}
        if action is not None:
            updates["last_action"] = action
        self._update(**updates)

    def _settings_changed(self, path: str, value: object) -> None:
        if path == "enabled":
            self._update(enabled=bool(value))

    def _update(self, **changes: object) -> None:
        with self._lock:
            self._snapshot = replace(self._snapshot, **changes)
            callbacks = tuple(self._callbacks)
            snapshot = self._snapshot
        for callback in callbacks:
            callback(snapshot)

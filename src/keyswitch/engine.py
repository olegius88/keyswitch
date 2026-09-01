"""Keyboard event state machine and correction orchestration."""

from __future__ import annotations

import queue
import threading
import time
from dataclasses import dataclass
from collections.abc import Callable

from .backend import InputBackend, KeyEvent
from .config import SettingsStore
from .detector import DetectionDecision, LanguageDetector
from .history import HistoryEntry, HistoryStore
from .indicator import alternate_layout_group, layout_label
from .language_model import LanguageModel
from .learning import LearningStore
from .intent_model import CorrectionTrigger, LinearNgramModel


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
        self._modifier_keycodes: set[int] = set()
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
        self.learning.confirm_manual(
            current.source_group,
            current.original,
            current.target_group,
            required,
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
        self, prompt: LearningPrompt | None = None
    ) -> bool:
        with self._lock:
            current = self._learning_prompt
            if current is None or (prompt is not None and prompt != current):
                return False
            self._learning_prompt = None
            self._learning_prompt_deadline = None
            callbacks = tuple(self._learning_prompt_callbacks)
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
            self._clear_word("Очередь ввода переполнена")

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
                event = self._events.get(timeout=0.5)
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
                self._clear_word()
                self._update(last_error=str(error), last_action="Ошибка обработки ввода")

    def _apply_layout_selection(self, group: int) -> None:
        try:
            self.backend.switch_group(group)
        except Exception as error:
            self._update(
                last_error=str(error),
                last_action="Язык из меню не переключён",
            )
            return
        self._clear_word()
        self._manual_layout_group = (
            group
            if bool(self.settings.get("detection.respect_manual_layout", True))
            else None
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
                self.dismiss_learning_prompt(prompt)
                return
            self.dismiss_learning_prompt(prompt)
        self._observe_group(event.group)
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
                if self._strokes:
                    self._mark_word_activity()
                else:
                    self._source_group = -1
                    self._reset_pause_correction()
                self._update(current_word=self._text_for_group(self._strokes, self._source_group))
            return
        if self._is_layout_letter(event):
            if not event.character.isalpha() and self._ambiguous_key_is_boundary(event):
                self._commit_word(event)
                return
            if self._source_group not in (-1, event.group):
                self._clear_word()
            self._source_group = event.group
            self._strokes.append(event)
            self._mark_word_activity()
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
        self._reset_pause_correction()
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
        manual_layout_selected = (
            bool(self.settings.get("detection.respect_manual_layout", True))
            and self._manual_layout_group == source_group
        )
        manual_layout_protected = (
            manual_layout_selected
            and self._forced_target_group(source_group, original) is None
        )
        if manual_layout_selected:
            self._manual_layout_group = None
        should_analyze = (
            bool(self.settings.get("enabled", True))
            and self._boundary_enabled(boundary)
            and not manual_layout_protected
        )
        excluded = self._application_excluded(application)
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
        self._strokes = []
        self._source_group = -1
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
        return self.detector.decide(
            original,
            alternatives,
            source_group,
            minimum_length=int(self.settings.get("detection.minimum_length", 3)),
            confidence_threshold=float(self.settings.get("detection.confidence", 2.0)),
            ignored_words=set(ignored_words),
            aggressive=bool(self.settings.get("detection.aggressive", False)),
            protect_code=bool(self.settings.get("detection.protect_code", True)),
            previous_words=context_words if context_aware else {},
            context_group=context_group if context_aware else None,
            forced_target_group=self._forced_target_group(source_group, original),
            rejected_targets=(
                self.learning.rejected_targets(source_group, original)
                if bool(self.settings.get("detection.learning", True))
                else set()
            ),
            trigger=trigger,
            use_intent_model=bool(
                self.settings.get("detection.intent_model_enabled", True)
            ),
        )

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

    def _mark_word_activity(self) -> None:
        self._last_word_input_at = time.monotonic()
        self._pause_correction_pending = True

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
        if current_time - last_input < PAUSE_CORRECTION_DELAY_SECONDS:
            return
        if self._pressed:
            return
        if self._modifier_keycodes:
            return
        if self._pending is not None:
            return

        self._pause_correction_pending = False
        strokes = tuple(self._strokes)
        source_group = self._source_group
        manual_layout_selected = (
            bool(self.settings.get("detection.respect_manual_layout", True))
            and self._manual_layout_group == source_group
        )
        original = self._text_for_group(strokes, source_group)
        if manual_layout_selected:
            if self._forced_target_group(source_group, original) is None:
                return
            self._manual_layout_group = None
        alternatives = {
            group: self._text_for_group(strokes, group)
            for group in self.models
            if group != source_group
        }
        application = self.backend.active_application()
        if self._application_excluded(application):
            return
        decision = self._decide_word(
            original, alternatives, source_group, application, "pause"
        )
        if not decision.should_convert:
            return

        plan = self._plan_from_decision(strokes, None, application, decision)
        self._strokes = []
        self._source_group = -1
        self._reset_pause_correction()
        self._update(current_word="")
        self._execute_correction(plan, None)

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
        self._pending_learning_action = (
            "manual",
            source_group,
            self._text_for_group(strokes, source_group),
            target,
        )
        self._pending_trigger_keycode = trigger_keycode
        self._strokes = []
        self._source_group = -1
        self._reset_pause_correction()

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
        self._execute_correction(plan, learning_action)

    def _execute_correction(
        self,
        plan: CorrectionPlan,
        learning_action: tuple[str, int, str, int] | None,
    ) -> None:
        try:
            self.backend.inject_correction(
                plan.strokes,
                plan.target_group,
                plan.boundary,
                plan.source_group,
            )
        except Exception as error:
            self._update(last_error=str(error), last_action="Исправление не выполнено")
            return
        self._last_correction = plan
        self._last_correction_time = time.monotonic()
        self._remember_context(plan.application, plan.target_group, plan.strokes)
        learned_rule = False
        rejected_rule = False
        learning_prompt: LearningPrompt | None = None
        if learning_action is not None and bool(self.settings.get("detection.learning", True)):
            action, source_group, word, target_group = learning_action
            if action == "manual":
                confirmations = self.learning.record_manual(
                    source_group, word, target_group
                )
                required = int(self.settings.get("detection.learning_confirmations", 2))
                learned_rule = confirmations >= required
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
        return self.dismiss_learning_prompt()

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

    def _clear_word(self, action: str | None = None) -> None:
        self.dismiss_learning_prompt()
        self._strokes = []
        self._source_group = -1
        self._reset_pause_correction()
        self._pending = None
        self._pending_learning_action = None
        if action is None:
            self._update(current_word="")
        else:
            self._update(current_word="", last_action=action)

    def _settings_changed(self, path: str, value: object) -> None:
        if path == "*":
            self.dismiss_learning_prompt()
            self._update(enabled=bool(self.settings.get("enabled", True)))
            self._manual_layout_group = None
        elif path == "enabled":
            self._update(enabled=bool(value))
        elif path == "detection.respect_manual_layout" and not bool(value):
            self._manual_layout_group = None
        elif path == "detection.learning" and not bool(value):
            self.dismiss_learning_prompt()

    def _observe_group(self, group: int) -> None:
        current_group = self.snapshot.current_group
        if not 0 <= group < len(self.models) or group == current_group:
            return
        if current_group >= 0 and bool(
            self.settings.get("detection.respect_manual_layout", True)
        ):
            self._manual_layout_group = group
            self._update(
                current_group=group,
                last_action=(
                    "Ручная смена раскладки · следующее слово без автокоррекции"
                ),
            )
            return
        self._update(current_group=group)

    def _poll_current_group(self) -> None:
        self._observe_group(self.backend.current_group())

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

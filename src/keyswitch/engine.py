"""Keyboard event state machine and correction orchestration."""

from __future__ import annotations

import itertools
import json
import logging
import queue
import threading
import time
import unicodedata
from collections.abc import Callable
from dataclasses import dataclass, replace
from typing import Final

from . import __version__
from .backend import InputBackend, KeyEvent, KeyDisposition
from .app_quirks import mention_head
from .boundary_model import BoundaryModel, MAX_SUFFIX, features as boundary_features
from .boundary_policy import BoundaryPolicy
from .config import SettingsStore
from .constants.settings_defaults import (
    CONFIDENCE_SETTING_MAX,
    CONFIDENCE_SETTING_MIN,
    DEFAULT_CONFIDENCE_THRESHOLD,
    DEFAULT_EARLY_SWITCH_MIN_LENGTH,
    DEFAULT_MINIMUM_WORD_LENGTH,
    DEFAULT_PAUSE_DELAY_SECONDS,
    EARLY_SWITCH_MIN_LENGTH_SETTING_MAX,
    EARLY_SWITCH_MIN_LENGTH_SETTING_MIN,
    MINIMUM_WORD_LENGTH_SETTING_MAX,
    MINIMUM_WORD_LENGTH_SETTING_MIN,
    PAUSE_DELAY_SETTING_MAX_SECONDS,
    PAUSE_DELAY_SETTING_MIN_SECONDS,
)
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
from .layouts import RU_KEYS, US_KEYS, LayoutPair
from .lexicon_supplement import supplement_words
from .learning import LearnedRule, LearningStore, RuleAction
from .intent_model import CorrectionTrigger, LinearNgramModel
from .context_policy import ContextPolicy, ContextResult
from .constants.models import (
    CONTEXT_ACTION_FEATURE_VERSION,
    PREFIX_MAX_CHARACTERS,
    PREFIX_MIN_CHARACTERS,
)
from .context_access import PlatformFieldReader
from .constants.units import MILLISECONDS_PER_SECOND
from .input_context import CONTEXT_TTL, FieldContext, FieldReader
from .prefix_model import PrefixInput, PrefixModel
from .prefix_schema import VersionedPrefixModel
from .settings_diagnostics import setting_change, settings_snapshot
from .short_words import ISOLATED_SHORT_WORD_REASON, is_short_word_override
from .word_decision import automatic_word_decision
from .constants.detection import (
    EARLY_SWITCH_CONFIDENCE,
    ENGINE_EVENT_QUEUE_MAX_SIZE,
    KEPT_WORDS_TAKEN_ALONG,
    MAX_REMEMBERED_APPLICATION_CONTEXTS,
    MAX_WORD_STROKES as MAX_WORD_STROKES,
    NATURAL_SOURCE_BOUNDARY_MIN_CHARACTERS,
    NATURAL_SOURCE_BOUNDARY_NGRAM_FLOOR,
    REPLAYED_SIGNS_MIN_STEM_LETTERS,
    TRUSTED_SHORT_WORD_MAX_LENGTH,
    UNSCORED_CORRECTION_CONFIDENCE,
)
from .constants.keyboard import LAYOUT_GROUP_COUNT, UNICODE_PACKET_KEY_NAME
from .constants.file_formats import LEGACY_RULE_CONFIRMATIONS_REQUIRED
from .constants.log_files import LOGGED_SCORE_DECIMALS
from .constants.text import BASIC_MULTILINGUAL_PLANE_MAX_CODEPOINT
from .constants.timing import (
    ACTION_TIMEOUT_SECONDS,
    DOUBLE_CONVERT_PRESS_WINDOW_SECONDS,
    ENGINE_LOOP_MAX_WAKE_SECONDS,
    ENGINE_LOOP_MIN_WAKE_SECONDS,
    ENGINE_SWITCH_GRACE_SECONDS,
    ENGINE_WORKER_JOIN_TIMEOUT_SECONDS,
    LATE_STROKE_GRACE_SECONDS,
    LEARNING_PROMPT_TIMEOUT_SECONDS,
    MANUAL_RELEASE_TIMEOUT_SECONDS,
    STALE_PRESS_SECONDS,
    UNDO_AVAILABLE_WINDOW_SECONDS,
)


MODIFIER_KEYS = {
    "Shift_L", "Shift_R", "Control_L", "Control_R", "Alt_L", "Alt_R",
    "Meta_L", "Meta_R", "Super_L", "Super_R", "ISO_Level3_Shift", "Caps_Lock",
}
NAVIGATION_KEYS = {
    "Left", "Right", "Up", "Down", "Home", "End", "Page_Up", "Page_Down",
    "Escape", "Delete", "Insert", "Pointer",
}
PUNCTUATION = set(".,!?;:()[]{}—–-…\"«»")
# Keys that answer the learning prompt: while it is shown they belong to
# KeySwitch, not to the text being typed.
PROMPT_KEYS = {"Return", "KP_Enter", "Escape"}
WORD_BOUNDARY_KEYS = {"space", "Return", "Tab", "ISO_Left_Tab"}
ACTION_BOUNDARY_KEYS = {"Return", "KP_Enter", "Tab", "ISO_Left_Tab"}
WORD_JOINERS = {"'", "’", "-", "‐", "‑"}
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
    # | mention_shown | mention_hidden
    mode: str = "boundary"
    context_field: str = ""
    # Literal punctuation before `boundary`, not replayed in the new layout.
    trailing: tuple[KeyEvent, ...] = ()
    # A symbol typed in front of the word that this application reads as the
    # start of a mention. It is judged with the word, not on its own, so it is
    # kept out of `strokes` until the correction runs (_execute_correction).
    head: tuple[KeyEvent, ...] = ()
    # A word converted at a space: its baseline decision and field, so the model can be
    # asked about it again once the next word shows which way that one went
    # (_revert_with_next_word).
    revisit: tuple[DetectionDecision, FieldContext | None] | None = None
    # The boundary sign was folded into `strokes` and is replayed in the new layout
    # (_replay_sign_with_word): the word ended there, though no boundary follows it.
    sign_replayed: bool = False


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
    context_action: str = ""
    context_model: str = ""


@dataclass(frozen=True)
class LanguageContext:
    group: int
    words: dict[int, str]
    updated_at: float


@dataclass(frozen=True)
class InsertionPoint:
    """The letters that already stood right before and after the caret when a word began there."""

    head: str
    tail: str

    @property
    def inside_word(self) -> bool:
        return bool(self.head or self.tail)


@dataclass(frozen=True)
class WaitingContextWord:
    plan: CorrectionPlan
    decision: DetectionDecision
    field: FieldContext
    window: int
    deadline: float
    diagnostic_id: int = 0
    # A word the model only suggested converting is asked again when its neighbour
    # arrives, never at a pause: a pause brings no new context to decide with.
    settles_on_pause: bool = True


# The baseline reason of the question a converted word is asked again (_revert_with_next_word).
REVISIT_REASON: Final = "слово после соседа, переведённого обратно"


@dataclass(frozen=True)
class ProvisionalWord:
    """An applied conversion of a word that the next word may still take back."""

    plan: CorrectionPlan
    window: int
    deadline: float


@dataclass(frozen=True)
class KeptWord:
    """A word left as typed at a space that the next word may still take along.

    `baseline` is the detector's decision, or None for a token the boundary model
    could not split (`dc\\``: `всё` or `вс` and a backtick), which was not decided at all.
    """

    plan: CorrectionPlan
    baseline: DetectionDecision | None
    field: FieldContext | None
    window: int
    deadline: float


@dataclass(frozen=True)
class LearningPrompt:
    """The rule a double press of the conversion hotkey offers to set up.

    ``original`` is the word as typed in ``source_group`` and ``replacement`` the same
    keys in ``target_group``; ``action`` is what the rule would do with it. An empty
    ``original`` offers an empty rule window: there was no word to start from.
    """

    source_group: int
    target_group: int
    original: str
    replacement: str
    application: str
    action: RuleAction = "convert"


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
        context_reader: FieldReader | None = None,
    ) -> None:
        self.settings = settings
        self.history = history
        locales: list[str] = settings.get(
            "detection.language_models", ["en_US", "ru_RU"]
        )
        self.models = {
            index: LanguageModel.load(locale, supplement_words(locale))
            for index, locale in enumerate(locales[:LAYOUT_GROUP_COUNT])
        }
        intent_model, self.intent_model_status = LinearNgramModel.try_load_default()
        self.detector = LanguageDetector(self.models, intent_model)
        self.backend: InputBackend = backend or _default_backend(len(self.models))
        self.backend_label = backend_label
        self.learning = learning or LearningStore(
            history.path.with_name("learning.json"),
            # Only reads a learning file of an older schema; the setting itself is gone.
            legacy_confirmations=int(settings.get(
                "detection.learning_confirmations", LEGACY_RULE_CONFIRMATIONS_REQUIRED)),
        )
        self.context_policy = ContextPolicy(context_reader or PlatformFieldReader(self.backend))
        self._context_result: ContextResult | None = None
        self.boundary_model: BoundaryModel | None = BoundaryPolicy.default()
        # Typed by the frozen base class: replays and tests substitute schema-one doubles.
        self.prefix_model: PrefixModel | None = VersionedPrefixModel.default()
        self._early_switch_confidence = EARLY_SWITCH_CONFIDENCE
        self._context_waiting: WaitingContextWord | None = None
        self._provisional: ProvisionalWord | None = None
        self._kept: tuple[KeptWord, ...] = ()
        # The baseline of the last word `_decide_word` decided, before the model spoke.
        self._last_baseline: DetectionDecision | None = None
        self._context_wait_sequence = 0
        self._sensitive_context_window: int | None = None
        self._typed_events = 0
        self._typed_presses = 0
        self._correction_sequence = 0
        self._input_overflow = threading.Event()
        self._untracked_token = False
        # Read from the keyboard hook without a lock: see `consumes_key`.
        self._prompt_key_deadline = 0.0
        self._deferred_action: KeyEvent | None = None
        self._action_deadline = 0.0
        self._action_keys = self._configured_action_keys()
        self._events: queue.Queue[KeyEvent | _LayoutSelection | None] = queue.Queue(
            maxsize=ENGINE_EVENT_QUEUE_MAX_SIZE
        )
        self._worker: threading.Thread | None = None
        self._running = threading.Event()
        self._strokes: list[KeyEvent] = []
        self._source_group = -1
        self._last_word_input_at: float | None = None
        self._pause_correction_pending = False
        self._manual_layout_group: int | None = None
        self._own_layout_ignored = False
        self._pressed: set[int] = set()
        self._pressed_since: dict[int, float] = {}
        self._modifier_keycodes: set[int] = set()
        # Layout-dependent symbols typed right after a boundary (e.g. the RU
        # quote on Shift+2 meant as "@"); Pause converts them on their own.
        self._symbol_strokes: list[KeyEvent] = []
        # The quote from _symbol_strokes that is on screen as "@" right now
        # (_show_mention_head), until the next key writes it back.
        self._mention_shown: KeyEvent | None = None
        # The character the previous key typed and the one typed before the
        # current key: empty at the start of the text, after a click, a caret
        # move or an erased character, where nothing is known to stand there.
        self._last_key_character = ""
        self._character_before_key = ""
        # After a click, a caret move or another window the engine has not seen
        # what stands around the caret; the next word reads it from the field
        # as it begins (_read_insertion).
        self._position_unknown = True
        self._insertion: InsertionPoint | None = None
        # True once anything was typed after the last committed word, so Pause
        # must not rewrite that word any more.
        self._last_committed_stale = False
        self._caret_moved = False
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
        # The key an application quirk has just rewritten; pressing it again undoes that.
        self._manual_release_deadline = 0.0
        self._last_committed: CorrectionPlan | None = None
        self._last_correction: CorrectionPlan | None = None
        # The rule offer to show once the pending plan has run: a double press
        # that had to put the word back first.
        self._learning_prompt_after: LearningPrompt | None = None
        # The first plan of the chain of corrections over the same keys: what
        # the engine itself did (automatic) or the first manual conversion.
        self._correction_origin: CorrectionPlan | None = None
        # The plan the last successful _execute_correction was asked to run.
        self._last_requested_plan: CorrectionPlan | None = None
        # The previous press of the conversion hotkey, for telling a double press.
        self._convert_press_at = 0.0
        self._convert_keycode = -1
        self._convert_released = False
        # What that press did: the plan it scheduled or "switch" for a layout toggle.
        self._convert_outcome: CorrectionPlan | str | None = None
        # Set while the rule window is open: its fields are not text to correct.
        self._input_suspended = False
        # Input other programs inject (remote control, macros) counts as typing
        # only with detection.injected_input; read by the hook, so cached here.
        self._injected_input = bool(settings.get("detection.injected_input", False))
        self._foreign_input_active = False
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
        self._rule_request_callbacks: list[Callable[[LearningPrompt], None]] = []
        self._lock = threading.RLock()
        self.settings.subscribe(self._settings_changed)
        self._technical_session_event("engine_initialized")
        if self.learning.migrated:
            self._technical_event("learning_rules_migrated", rules=self.learning.migrated)

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

    def subscribe_rule_requests(self, callback: Callable[[LearningPrompt], None]) -> None:
        """Be told when Enter on the prompt asks for the rule window."""

        with self._lock:
            self._rule_request_callbacks.append(callback)

    def confirm_learning_prompt(
        self, prompt: LearningPrompt | None = None
    ) -> bool:
        """Enter on the prompt: open the rule window, filled in from the prompt.

        Nothing is remembered yet. A rule exists only once the user presses OK in
        that window, which calls :meth:`add_learning_rule`.
        """

        with self._lock:
            current = self._learning_prompt
            if current is None or (prompt is not None and prompt != current):
                return False
            self._learning_prompt = None
            self._learning_prompt_deadline = None
            self._prompt_key_deadline = 0.0
            callbacks = tuple(self._learning_prompt_callbacks)
            requests = tuple(self._rule_request_callbacks)
        self._technical_event(
            "learning_prompt_confirmed",
            source_group=current.source_group,
            target_group=current.target_group,
            application=current.application,
            action=current.action,
            has_word=bool(current.original),
        )
        for callback in callbacks:
            callback(None)
        for request in requests:
            request(current)
        return True

    def add_learning_rule(
        self, rule: LearnedRule, *, replacing: LearnedRule | None = None, application: str = ""
    ) -> LearnedRule:
        """OK in the rule window: keep the rule, in place of ``replacing`` if given."""

        stored = (
            self.learning.add_rule(rule) if replacing is None
            else self.learning.replace_rule(replacing, rule)
        )
        excluded = bool(application) and self._application_excluded(application)
        self._technical_event(
            "learning_rule_added",
            pattern="<redacted>" if excluded else stored.pattern,
            match=stored.match,
            case_sensitive=stored.case_sensitive,
            action=stored.action,
            source_group=stored.source_group,
            target_group=stored.target_group,
            replaced=replacing is not None,
            application=application,
            application_excluded=excluded,
        )
        verb = "переводить" if stored.action == "convert" else "не переводить"
        self._update(last_action=f"Правило добавлено: «{stored.pattern}» — {verb}")
        return stored

    def set_rule_editor_open(self, opened: bool) -> None:
        """While the rule window has the keyboard its fields are typed into, not text to correct.

        The window reports when it gains and loses the keyboard, so typing elsewhere
        while it stays open is corrected as usual.
        """

        if opened == self._input_suspended:
            return
        self._input_suspended = opened
        self._technical_event("rule_editor_" + ("focused" if opened else "left"))

    def dismiss_learning_prompt(
        self, prompt: LearningPrompt | None = None, *, reason: str = "dismissed"
    ) -> bool:
        with self._lock:
            current = self._learning_prompt
            if current is None or (prompt is not None and prompt != current):
                return False
            self._learning_prompt = None
            self._learning_prompt_deadline = None
            self._prompt_key_deadline = 0.0
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
            # The hook asks this before letting a key through to the window.
            self.backend.set_key_filter(self.consumes_key)
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
        self.context_policy.stream.clear()
        self.dismiss_learning_prompt()
        self.backend.set_key_filter(None)
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
            self._worker.join(timeout=ENGINE_WORKER_JOIN_TIMEOUT_SECONDS)
        self._worker = None
        self.backend.close()
        self._update(running=False, backend="остановлен", current_word="")

    def enqueue(self, event: KeyEvent) -> None:
        if event.synthetic:
            return
        if event.key_name == "Pointer":
            self._prompt_key_deadline = 0.0
        # Physical events and presses are separate: a key-up is not text.
        self._typed_events += 1
        self._typed_presses += int(event.pressed)
        try:
            self._events.put_nowait(event)
        except queue.Full:
            # The hook must not mutate the worker's state or invoke UI/file
            # callbacks. The worker invalidates the text before its next action.
            self._input_overflow.set()

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
                # The timers replace text too (pause correction, deferred Enter):
                # an error there must not end this thread while the hook keeps
                # queueing keys and a held Enter waits for its release.
                try:
                    self._run_timers()
                except Exception as error:
                    self._recover_from_error(error)
                continue
            if event is None:
                break
            try:
                if isinstance(event, _LayoutSelection):
                    self._apply_layout_selection(event.group)
                else:
                    self._handle(event)
            except Exception as error:
                self._recover_from_error(error)
        reader = self.context_policy.reader
        if isinstance(reader, PlatformFieldReader):
            reader.close()

    def _run_timers(self) -> None:
        self._expire_deferred_action()
        self._expire_manual_correction()
        self._poll_current_group()
        self._maybe_correct_after_pause()
        self._expire_learning_prompt()

    def _recover_from_error(self, error: Exception) -> None:
        self._clear_word(reason="input_error")
        self._update(last_error=str(error), last_action="Ошибка обработки ввода")

    def _loop_timeout(self) -> float:
        """Wake exactly when the pause delay elapses, at most every 0.5 s."""

        last_input = self._last_word_input_at
        if not self._pause_correction_pending or last_input is None:
            return ENGINE_LOOP_MAX_WAKE_SECONDS
        remaining = last_input + self._pause_delay() - time.monotonic()
        return max(ENGINE_LOOP_MIN_WAKE_SECONDS, min(ENGINE_LOOP_MAX_WAKE_SECONDS, remaining))

    def _bounded_setting(self, path: str, default: float, minimum: float, maximum: float) -> float:
        """A numeric setting within the range the settings windows offer for it.

        Both windows take their ranges from the same constants, and a value edited
        into the settings file by hand is brought into that range here; one that is
        not a number at all falls back to the default.
        """

        try:
            value = float(self.settings.get(path, default))
        except (TypeError, ValueError):
            value = float(default)
        return min(maximum, max(minimum, value))

    def _pause_delay(self) -> float:
        return self._bounded_setting(
            "detection.pause_delay_seconds", DEFAULT_PAUSE_DELAY_SECONDS,
            PAUSE_DELAY_SETTING_MIN_SECONDS, PAUSE_DELAY_SETTING_MAX_SECONDS,
        )

    def _confidence_threshold(self) -> float:
        return self._bounded_setting(
            "detection.confidence", DEFAULT_CONFIDENCE_THRESHOLD, CONFIDENCE_SETTING_MIN, CONFIDENCE_SETTING_MAX)

    def _minimum_word_length(self) -> int:
        return int(self._bounded_setting(
            "detection.minimum_length", DEFAULT_MINIMUM_WORD_LENGTH,
            MINIMUM_WORD_LENGTH_SETTING_MIN, MINIMUM_WORD_LENGTH_SETTING_MAX,
        ))

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
        self._expire_manual_correction()
        if self._input_overflow.is_set():
            self._input_overflow.clear()
            self._complete_deferred_action(False, "input_overflow")
            self._clear_word("Очередь ввода переполнена", reason="input_overflow")
            self._untracked_token = True
            self.context_policy.stream.clear()
        if event.pressed:
            self._track_focus()
        if event.key_name == "Pointer":
            if event.pressed:
                self._log_input_edit(event, self.backend.active_application())
            self._sensitive_context_window = None
            self._convert_press_at = 0.0
            self._clear_word(reason="pointer_activity")
            self._untracked_token = False
            # A click often lands in an empty field, which is exactly where the first
            # word must still be corrected, so it is not treated as a caret move.
            self._caret_moved = False
            self._contexts.clear()
            self.context_policy.stream.clear()
            return
        if self._input_suspended:
            # The rule window is open: what is typed there is a rule, not text.
            if event.pressed and (self._strokes or self._symbol_strokes):
                self._clear_word(reason="rule_editor_open")
            self._convert_press_at = 0.0
            return
        if event.foreign and not self._accepts_injected(event):
            self._observe_foreign_input(event)
            return
        if event.pressed and event.key_name not in MODIFIER_KEYS:
            self._foreign_input_active = False
        self._expire_learning_prompt()
        prompt = self.learning_prompt
        if prompt is not None and event.pressed and event.key_name not in MODIFIER_KEYS:
            unmodified = not (event.control or event.alt or event.super_key or event.shift)
            if unmodified and event.key_name in {"Return", "KP_Enter"}:
                self.confirm_learning_prompt(prompt)
                if event.deferred:
                    # The prompt may have appeared after the hook deferred
                    # Enter but before the worker reached it. It still owns
                    # this key; release the barrier without submitting.
                    self.backend.complete_action(False)
                return
            if unmodified and event.key_name == "Escape":
                self.dismiss_learning_prompt(prompt, reason="escape")
                return
            self.dismiss_learning_prompt(prompt, reason="other_key")
        if event.pressed:
            application = self.backend.active_application()
            if (
                event.character and not self._strokes
                and self._sensitive_context_window is None
                and not self._application_excluded(application)
                and bool(self.settings.get("enabled", True))
                and bool(self.settings.get("detection.context_read_field", False))
                and self.context_policy.reader is not None
            ):
                field = self.context_policy.reader.read(application, self._focus_window or 0)
                if field is not None and field.sensitive:
                    self._sensitive_context_window = self._focus_window
                    self._cancel_context_wait("sensitive_field")
                    self.context_policy.stream.clear()
            context_enabled = (
                bool(self.settings.get("enabled", True))
                and bool(self.settings.get("detection.context_aware", True))
                and self.settings.get("detection.context_policy", "assist") != "off"
                and not self._application_excluded(application)
            )
            stream = self.context_policy.stream
            stream.focus(application, self._focus_window or 0)
            self._log_input_edit(event, application)
            if context_enabled:
                if not any(self._matches_hotkey(name, event) for name in ("toggle", "convert_last", "undo")):
                    stream.observe(event)
            else:
                stream.clear()
            if event.key_name == "BackSpace" or event.control or event.alt or event.super_key:
                self._cancel_context_wait("backspace" if event.key_name == "BackSpace" else "modifier_shortcut")
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
        if event.pressed and not event.synthetic:
            typed = "" if event.key_name in {"BackSpace", "Delete"} else event.character
            self._character_before_key = self._last_key_character
            self._last_key_character = typed if typed.isprintable() or typed.isspace() else ""
        if not event.pressed:
            if event.keycode == self._convert_keycode:
                self._convert_released = True
            self._maybe_execute_pending(event)
            return
        is_convert = self._matches_hotkey("convert_last", event)
        if not is_convert:
            # Only two presses of the hotkey with nothing in between are a double press.
            self._convert_press_at = 0.0
        if self._matches_hotkey("toggle", event):
            enabled = not bool(self.settings.get("enabled", True))
            self.settings.set("enabled", enabled)
            self._clear_word(
                "Автокоррекция включена" if enabled else "Автокоррекция на паузе",
                reason="engine_toggled",
            )
            return
        if is_convert:
            if self._double_convert_press(event):
                self._request_rule_after_double_press(event.keycode)
            else:
                self._schedule_manual_conversion(event.keycode)
            return
        if self._matches_hotkey("undo", event):
            self._schedule_undo(event.keycode)
            return
        if event.control or event.alt or event.super_key:
            self._clear_word(reason="modifier_shortcut")
            return
        if event.key_name == "BackSpace":
            reopenable = self._reopenable_committed_word()
            self._last_committed_stale = True
            self._log_pending_dropped("backspace")
            self._pending = None
            self._learning_prompt_after = None
            # Erasing is not appending. A caret moved into finished text makes the next
            # word untouchable because nobody knows what stands in front of it; a user
            # who then rubs that text out has answered the question - nothing does.
            self._caret_moved = False
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
                if not self._symbol_strokes:
                    # The "@" shown for the quote is what this key erased.
                    self._mention_shown = None
            elif reopenable is not None:
                # This Backspace took away the boundary that ended the last
                # word, so the caret stands right after that word again and
                # letters typed now continue it: "создаш" + "ь" is judged as
                # "создашь", not as a lone "ь" that no dictionary knows.
                # The word was already judged at that boundary, so reopening
                # alone does not arm the pause timer: a user who stops to
                # think after the Backspace must not get a second, unasked
                # judgement of the same letters. The next letter arms it.
                self._strokes = list(reopenable.strokes)
                self._source_group = reopenable.source_group
                self._technical_event(
                    "committed_word_reopened",
                    characters=len(self._strokes),
                    source_group=self._source_group,
                    boundary=reopenable.boundary.key_name if reopenable.boundary else "",
                )
                self._update(
                    current_word=self._text_for_group(self._strokes, self._source_group)
                )
            if self._mention_shown is not None and self._strokes:
                # The pending write-back went with the erased letter; the
                # letters left still stand after the "@".
                self._hide_mention_head(self._mention_shown, None, event.keycode)
            return
        if event.key_name in ACTION_BOUNDARY_KEYS:
            if event.deferred:
                self._deferred_action = event
                self._action_deadline = time.monotonic() + ACTION_TIMEOUT_SECONDS
                self._technical_event("action_deferred", key_name=event.key_name)
                if self._strokes and not self._untracked_token:
                    self._commit_word(event)
                return
            # These events have already reached the application. Enter may
            # submit a chat and Tab may focus another field; neither can be
            # undone with one BackSpace and safely replayed.
            self._clear_word(reason="action_boundary_already_delivered")
            self._untracked_token = False
            self._caret_moved = False
            self._contexts.clear()
            self.context_policy.stream.clear()
            self._sensitive_context_window = None
            return
        if self._sensitive_context_window is not None:
            return
        if self._untracked_token:
            if self._is_boundary(event):
                self._untracked_token = False
            return
        if event.character and not self._safe_text_stroke(event) and not (
            len(event.character) == 1 and event.character.isspace()
        ):
            self._technical_event(
                "input_not_representable", key_name=event.key_name,
                group=event.group,
                character_lengths=[len(char) for char in event.characters],
                character_categories=[unicodedata.category(char[0]) if char else "empty" for char in event.characters],
            )
            self._clear_word(reason="unrepresentable_text")
            self._untracked_token = not self._is_boundary(event)
            return
        if len(self._strokes) + len(self._symbol_strokes) >= MAX_WORD_STROKES:
            self._clear_word(reason="token_too_long")
            self._untracked_token = not self._is_boundary(event)
            return
        if self._strokes and event.character in WORD_JOINERS | {"_", "@", "/", "\\", "="}:
            self._strokes.append(event)
            self._mark_word_activity()
            self._update(current_word=self._text_for_group(self._strokes, self._source_group))
            return
        if self._is_layout_letter(event):
            self._last_committed_stale = True
            if not event.character.isalpha() and self._ambiguous_key_is_boundary(event):
                self._commit_word(event)
                return
            if self._late_stroke_after_early_switch(event):
                event = self._convert_late_stroke(event)
            if self._source_group not in (-1, event.group):
                self._clear_word(reason="layout_changed_mid_word")
            if not self._strokes and any(stroke.character in {"@", "_", "/", "\\"} for stroke in self._symbol_strokes):
                self._strokes = self._symbol_strokes
                self._symbol_strokes = []
            self._begin_word(event)
            self._source_group = event.group
            self._strokes.append(event)
            self._mark_word_activity()
            self._update(
                current_group=event.group,
                current_word=self._text_for_group(self._strokes, event.group),
                last_error="",
            )
            self._after_mention_head(event, None)
            self._maybe_early_switch()
            return
        if self._is_boundary(event):
            if not self._strokes and event.character == "-":
                self._source_group = event.group
                self._strokes.append(event)
                self._mark_word_activity()
                self._after_mention_head(event, None)
                return
            if self._strokes:
                self._commit_word(event)
                return
            self._log_pending_dropped("additional_boundary")
            self._pending = None
            self._learning_prompt_after = None
            # Nothing typed since the last boundary: remember layout-dependent
            # symbols so Pause converts just them (RU quote -> "@"), and never
            # rewrite the previous word after further input.
            if event.key_name in WORD_BOUNDARY_KEYS or event.character.isspace():
                self._symbol_strokes = []
            elif self._layout_dependent(event):
                self._symbol_strokes.append(event)
            self._last_committed_stale = True
            if self._mention_shown is not None:
                self._after_mention_head(event, event)
            elif len(self._symbol_strokes) == 1 and self._symbol_strokes[0] is event:
                self._show_mention_head(event)
            return
        if event.key_name in NAVIGATION_KEYS:
            # The caret moved: what was typed belongs to another position, and
            # the engine no longer knows what stands in front of the new one.
            self._last_committed_stale = True
            self._clear_word(reason="navigation")
            self._caret_moved = True
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
                # "@" typed in the US layout but meant as the RU quote, or the
                # other way round: what it is depends on the word that follows,
                # so it waits here until that word is decided (_mention_head).
                self._symbol_strokes.append(event)
                return
            # A leading digit or path marker belongs to the token too:
            # `2ghbdtn` must not be seen as the unrelated word `ghbdtn`.
            self._begin_word(event)
            self._source_group = event.group
            self._strokes.append(event)
            self._mark_word_activity()
            self._update(current_word=self._text_for_group(self._strokes, event.group))
            self._after_mention_head(event, None)
        elif event.key_name not in {"Pause", "Break"}:
            self._clear_word(reason="untracked_key")
            self._untracked_token = True

    @staticmethod
    def _safe_text_stroke(event: KeyEvent) -> bool:
        """Only one simple character per physical key can be backspaced.

        Composed text, surrogate pairs, combining marks and IME results do
        not have the one-key/one-deletion contract used by these backends.
        """

        return all(
            len(char) == 1
            and ord(char) <= BASIC_MULTILINGUAL_PLANE_MAX_CODEPOINT
            and unicodedata.category(char)[0] not in {"M", "C"}
            for char in event.characters
        )

    @staticmethod
    def _layout_dependent(event: KeyEvent) -> bool:
        """A printable key whose character differs between the layouts."""

        return bool(event.character) and len(
            {character for character in event.characters if character}
        ) > 1

    def _early_switch_policy(self) -> EarlySwitchPolicy:
        return EarlySwitchPolicy(minimum_length=int(self._bounded_setting(
            "detection.early_switch_min_length", DEFAULT_EARLY_SWITCH_MIN_LENGTH,
            EARLY_SWITCH_MIN_LENGTH_SETTING_MIN, EARLY_SWITCH_MIN_LENGTH_SETTING_MAX,
        )))

    def _caret_unknown(self) -> bool:
        """Whether this word is being typed where the engine cannot see the surroundings.

        An arrow key, Home or Page Up puts the caret somewhere the engine has not
        watched being typed. Until now the word that followed was analysed as if it
        opened an empty field, which is the one thing it is least likely to be: the
        caret is usually moved back *into* text, to finish or fix a word. Feeding
        that invented emptiness to the models is worse than saying nothing, so the
        word is left alone and Pause still converts it by hand.

        When the active field can be read, the surroundings are not a guess and the
        rule does not apply - the models get the real text before the caret.
        """

        if not self._caret_moved:
            return False
        if not bool(self.settings.get("detection.hold_after_caret_move", True)):
            return False
        if not bool(self.settings.get("detection.context_read_field", True)):
            return True
        reader = self.context_policy.reader
        return getattr(reader, "status", "not_requested") != "available"

    def _maybe_early_switch(self) -> None:
        """Switch the layout as soon as the typed prefix proves it wrong."""

        if self.boundary_model is not None and any(
            not stroke.character.isalpha() and self._is_layout_letter(stroke)
            for stroke in self._strokes
        ):
            return
        if not bool(self.settings.get("detection.early_switch", True)):
            return
        if not bool(self.settings.get("enabled", True)):
            return
        if self._caret_unknown():
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
        field: FieldContext | None = None
        confidence = EARLY_SWITCH_CONFIDENCE
        protection = self._early_prefix_protection(original, source_group, decision.target_group)
        if not excluded and not protection:
            field, protection = self._early_prefix_field(original, application)
            excluded = self._application_excluded(application)
        if protection:
            decision = replace(decision, should_switch=False, reason=protection)
        elif not excluded:
            decision, confidence = self._decide_prefix(decision, field)
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
            confidence,
            application,
            True,
            "early",
            context_field=field.field_id if field is not None and field.source != "observed" else "",
            head=self._mention_head(application),
        )
        # The last letter's key is physically still down: a synthetic press of a
        # held key is ignored by the X server and the retyped letter would be
        # lost. Like boundary corrections, execute on that key's release and
        # absorb letters pressed before it (rollover typing).
        self._pending = plan
        self._learning_prompt_after = None
        self._pending_trigger_keycode = strokes[-1].keycode
        self._technical_event(
            "early_switch_scheduled",
            trigger_keycode=strokes[-1].keycode,
            prefix_length=len(strokes),
        )

    def _early_prefix_protection(self, original: str, source: int, target: int) -> str:
        key = self.detector.token_key(original)
        ignored: list[str] = self.settings.get("exclusions.words", [])
        if any(self.detector.token_key(word).startswith(key) for word in ignored):
            return "excluded_word_prefix"
        if bool(self.settings.get("detection.learning", True)) and self.learning.keeps_continuation(source, original):
            return "learned_keep_rule"
        if bool(self.settings.get("detection.protect_code", True)) and self.detector.is_protected_token(original):
            return "protected_token"
        if self._context_waiting is not None:
            return "context_word_waiting"
        if self._insertion is not None and self._insertion.inside_word:
            # Letters typed into another word are not the start of one.
            return "inside_word"
        return ""

    def _early_prefix_field(self, original: str, application: str) -> tuple[FieldContext, str]:
        field = self.context_policy.stream.snapshot(original)
        if not bool(self.settings.get("detection.context_aware", True)):
            field = FieldContext(application, str(self._focus_window or 0))
        reader = self.context_policy.reader
        if bool(self.settings.get("detection.context_read_field", False)) and reader is not None:
            snapshot = reader.read(application, self._focus_window or 0)
            if snapshot is not None:
                snapshot = snapshot.bounded()
                if snapshot.sensitive:
                    self._sensitive_context_window = self._focus_window
                    self.context_policy.stream.clear()
                    self._contexts.clear()
                    self._update(current_word="", last_action="Защищённое поле: обработка отключена")
                    return snapshot, "sensitive_field"
                if snapshot.selection or not snapshot.field_id or snapshot.application != application or not snapshot.before.endswith(original):
                    return snapshot, "context_field_changed"
                field = replace(snapshot, before=snapshot.before[:-len(original)])
        return field, ""

    def _decide_prefix(self, baseline: EarlySwitchDecision, field: FieldContext | None) -> tuple[EarlySwitchDecision, float]:
        mode = str(self.settings.get("detection.context_policy", "assist"))
        if not bool(self.settings.get("detection.context_aware", True)) or mode not in {"assist", "shadow"}:
            return baseline, EARLY_SWITCH_CONFIDENCE
        supported = (PREFIX_MIN_CHARACTERS <= len(baseline.original) <= PREFIX_MAX_CHARACTERS
                     and baseline.replacement.isalpha()
                     and not any(char.isupper() for char in (baseline.original[1:] + baseline.replacement[1:]))
                     and baseline.source_group in {0, 1} and baseline.target_group == 1 - baseline.source_group
                     and 0 in self.models and 1 in self.models and 0 in self._prefix_indexes and 1 in self._prefix_indexes)
        model = self.prefix_model
        if model is None or field is None or not supported:
            reason = "prefix_model_unavailable" if model is None else "prefix_input_unsupported"
            return (replace(baseline, should_switch=False, reason=reason) if mode == "assist" else baseline), EARLY_SWITCH_CONFIDENCE
        prediction = model.predict(PrefixInput(baseline.original, baseline.replacement, baseline.source_group, field), self._prefix_indexes, self.models)
        self._technical_event(
            "prefix_decision", action=prediction.action, score=round(prediction.probability, LOGGED_SCORE_DECIMALS),
            model_version=prediction.model_version, mode=mode, baseline_convert=baseline.should_switch,
            policy_applied=mode == "assist", decision_source="prefix_model" if mode == "assist" else "prefix_index",
            final_action=prediction.action if mode == "assist" else "convert" if baseline.should_switch else "wait",
            prefix_length=len(baseline.original), source_group=baseline.source_group,
            context_source=field.source, before_characters=len(field.before), after_characters=len(field.after), field_role=field.role,
        )
        if mode == "shadow":
            return baseline, EARLY_SWITCH_CONFIDENCE
        return replace(baseline, should_switch=prediction.action == "convert", reason="префиксная модель: " + prediction.action), prediction.probability

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
        original = self._text_for_group(strokes, plan.source_group)
        replacement = self._text_for_group(strokes, plan.target_group)
        if plan.mode == "early" and (
            not bool(self.settings.get("detection.early_switch", True))
            or len(strokes) < self._early_switch_policy().minimum_length
            or self._early_prefix_protection(original, plan.source_group, plan.target_group)
            or not replacement.isalpha()
        ):
            return None
        refreshed = replace(plan, strokes=strokes, original=original, replacement=replacement)
        if plan.mode == "early":
            field, protection = self._early_prefix_field(original, plan.application)
            if protection:
                return None
            decision, confidence = self._decide_prefix(
                EarlySwitchDecision(True, plan.source_group, plan.target_group, original, replacement, "scheduled_prefix"), field,
            )
            if not decision.should_switch:
                return None
            refreshed = replace(refreshed, confidence=confidence)
        return refreshed

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
        """Schedule the mixed prefix for replacement after all keys are up."""

        target_group = self._source_group
        delay_ms = (
            None
            if self._early_switch_at is None
            else round((time.monotonic() - self._early_switch_at) * MILLISECONDS_PER_SECOND)
        )
        converted = replace(event, character=event.character_for(target_group), group=target_group)
        strokes = tuple(self._strokes) + (converted,)
        self._pending = CorrectionPlan(
            strokes, None, target_group, target_group,
            self._text_for_group(self._strokes, target_group) + event.character,
            self._text_for_group(strokes, target_group),
            EARLY_SWITCH_CONFIDENCE, self.backend.active_application(), True, "late_stroke",
        )
        self._pending_trigger_keycode = event.keycode
        self._learning_prompt_after = None
        self._technical_event(
            "late_stroke_scheduled",
            source_group=event.group,
            target_group=target_group,
            delay_ms=delay_ms,
        )
        return converted

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
                else round((time.monotonic() - observed_at) * MILLISECONDS_PER_SECOND)
            ),
        }

    def _finish_early_switch(
        self,
        strokes: tuple[KeyEvent, ...],
        boundary: KeyEvent | None,
        application: str,
        final_group: int,
        trailing: tuple[KeyEvent, ...] = (),
    ) -> None:
        """Record the completed word of an early switch as one correction.

        `strokes` and `trailing` are the committed word exactly as `_commit_word`
        recorded it: Pause, undo and Backspace delete `strokes + trailing + boundary`,
        so a literal comma left out here leaves one letter of the word on screen.
        """

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
            self._early_switch_confidence,
            application,
            True,
            "early",
            trailing=trailing,
        )
        self._remember_correction(plan)
        self._last_committed = CorrectionPlan(
            strokes, boundary, final_group, origin, replacement, original,
            self._early_switch_confidence, application, False, trailing=trailing,
        )
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
        self._record_history(original, replacement, application, EARLY_SWITCH_CONFIDENCE)
        for callback in tuple(self._correction_callbacks):
            callback(plan)

    def _record_history(self, original: str, replacement: str, application: str, confidence: float) -> None:
        """Store a correction the screen already shows; a failed write stops nothing.

        The text was replaced before this runs, so a full or locked disk is reported
        and the bookkeeping after it (undo, callbacks, the learning prompt) goes on.
        """

        if not bool(self.settings.get("general.keep_history", True)):
            return
        try:
            self.history.append(HistoryEntry.create(original, replacement, application, confidence))
        except OSError as error:
            self._technical_event("history_write_failed", error=type(error).__name__)
            self._update(last_error=f"История не записана: {error}")

    def _commit_word(self, boundary: KeyEvent) -> None:
        if not self._strokes:
            return
        if self._pending is not None and self._pending.mode != "early":
            self._log_pending_dropped("next_word_committed")
            self._pending = None
            self._learning_prompt_after = None
        typed = tuple(self._strokes)
        # Signs typed in front of the word's first letter: a quotation mark there is closed by one at its end.
        opened = (*self._symbol_strokes, *itertools.takewhile(lambda stroke: not stroke.character.isalpha(), typed))
        shown, self._mention_shown = self._mention_shown, None
        mention = self._mention_head(self.backend.active_application())
        head = self._literal_head(typed, self._source_group)
        strokes, trailing, segmentation_certain = self._completed_word(typed[head:], self._source_group)
        if self._insertion is not None and self._insertion.inside_word:
            # Typed into a word, every key is part of it: `,` between `те` and `е`
            # is the `б` of `тебе`, not punctuation in front of a word.
            head, strokes, trailing, segmentation_certain = 0, typed, (), True
        signs: tuple[KeyEvent, ...] = ()
        if not trailing and segmentation_certain and not (self._insertion is not None and self._insertion.inside_word):
            count = self._replayed_signs(strokes, self._source_group)
            if count:
                strokes, signs = strokes[:-count], strokes[-count:]
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
        # Pause on the last committed token still converts it whole: the
        # literal head only narrows what an automatic decision may replace.
        plan = CorrectionPlan(
            typed[:head] + strokes + signs,
            boundary,
            source_group,
            next(iter(alternatives), source_group),
            original + self._text_for_group(signs, source_group),
            next(iter(alternatives.values()), original) + self._text_for_group(signs, next(iter(alternatives), source_group)),
            0.0,
            application,
            False,
            trailing=trailing,
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
        caret_unknown = self._caret_unknown()
        self._caret_moved = False
        should_analyze = (
            enabled
            and trigger_enabled
            and not manual_layout_protected
            and segmentation_certain
            and not caret_unknown
        )
        excluded = self._application_excluded(application)
        decision: DetectionDecision | None = None
        waiting, self._context_waiting = self._context_waiting, None
        provisional, self._provisional = self._provisional, None
        kept, self._kept = self._kept, ()
        if should_analyze and not excluded:
            inside = None if trailing or head or signs else self._decide_inside_word(
                strokes, source_group, alternatives, application, self._trigger_for_boundary(boundary), boundary)
            decision = inside if inside is not None else self._decide_word(
                original,
                alternatives,
                source_group,
                application,
                self._trigger_for_boundary(boundary),
                literal_tail="".join(stroke.character for stroke in (*trailing, *signs)),
                boundary_text=boundary.character,
            )
            excluded = self._application_excluded(application)
            joint = None if trailing or head or signs else self._resolve_context_wait(
                waiting, strokes, boundary, decision, application, alternatives)
            if joint is not None and waiting is not None:
                joint = (self._take_along(kept, joint[0], len(waiting.plan.original), application), joint[1])
            elif waiting is not None and not head and (trailing or not decision.should_convert):
                # A waiting word whose neighbour did not convert either may still be settled by a
                # later word (`тщ ш`, then `огые`); one the model declined with a converted
                # neighbour has had its answer. A neighbour with a sign split off its end ends
                # the wait, and the waiting word is asked with it as a kept word (`tot ghbdtn,`).
                kept = (*kept, KeptWord(waiting.plan, waiting.decision, waiting.field, waiting.window, waiting.deadline))
            if joint is None and not (trailing or head or signs):
                joint = self._revert_with_next_word(provisional, strokes, boundary, decision, application)
            if joint is None and not head and decision.should_convert:
                taken = self._take_along(
                    kept, self._with_signs(replace(self._plan_from_decision(strokes, boundary, application, decision),
                                                   trailing=trailing), signs),
                    len(original), application)
                if taken.mode == "context_phrase":
                    joint = (taken, decision)
            if trailing or head:
                self._log_context_wait("context_wait_cancelled", waiting, "literal_tail" if trailing else "literal_head")
            if joint is not None:
                self._pending, decision = self._replay_sign_with_word(joint[0], opened), joint[1]
                self._learning_prompt_after = None
                self._pending_trigger_keycode = boundary.keycode
                decision = replace(decision, should_convert=True)
            elif decision.should_convert:
                plan = self._replay_sign_with_word(self._with_signs(replace(
                    self._plan_from_decision(strokes, boundary, application, decision),
                    trailing=trailing,
                    head=() if mention and self._quotation_closed(mention[0], trailing, boundary) else mention,
                ), signs), opened)
                if boundary.deferred:
                    # Enter/Tab has not reached the editor. Do not delete it
                    # as a character or include it in the text replacement.
                    plan = replace(plan, boundary=None, mode="before_action")
                # A boundary typed before the early switch's key was released
                # takes over the same word: say so instead of losing the plan.
                self._log_pending_dropped("superseded_by_boundary")
                self._pending = plan
                self._learning_prompt_after = None
                self._pending_trigger_keycode = boundary.keycode
                # A word converted at a space stays open for the next word: if that
                # one goes back the other way, the model is asked about this one again.
                if (inside is None and not trailing and not head and not signs and boundary.character == " "
                        and not boundary.deferred and self._last_baseline is not None):
                    field = self._context_result.field if self._context_result is not None else None
                    self._pending = replace(self._pending, revisit=(self._last_baseline, field))
                # A lone letter converts at once, but a user who keeps typing makes
                # that correction abort as unsafe; the next word then decides it,
                # exactly as it did while the letter waited for context. A short
                # word the model converts at the start of a message is the same
                # case: `z ctujlyz` typed in one go must still end as `я сегодня`.
                if decision.reason == ISOLATED_SHORT_WORD_REASON or self._opening_conversion(inside, original):
                    self._start_context_wait(plan, decision, boundary, trailing, head, fallback=True)
            else:
                self._remember_context(application, source_group, typed[:head] + strokes + signs)
                waits = False if signs else self._start_context_wait(plan, decision, boundary, trailing, head)
                if inside is None and not trailing and not head and not signs:
                    field = self._context_result.field if self._context_result is not None else None
                    # A word waiting for its neighbour joins the kept words only if the wait fails.
                    self._keep_for_next_word(kept, None if waits else KeptWord(
                        plan, self._last_baseline, field, self._focus_window or 0, time.monotonic() + CONTEXT_TTL), boundary)
                elif inside is None and trailing and not head and not signs and self._sign_reads_as_letter(plan):
                    # The boundary model split a sign off the end of the word and the word
                    # stayed, but only the whole token reads as a word: `t\`` is `её`, and
                    # only the next word can tell (`мать`). The model is asked about the
                    # whole token then.
                    whole = strokes + trailing
                    self._keep_for_next_word(kept, KeptWord(replace(
                        plan, strokes=typed[:head] + whole, original=self._text_for_group(whole, source_group),
                        replacement=self._text_for_group(whole, plan.target_group), trailing=(),
                    ), None, None, self._focus_window or 0, time.monotonic() + CONTEXT_TTL), boundary)
        else:
            self._log_context_wait("context_wait_cancelled", waiting, "analysis_skipped")
            self._remember_context(application, source_group, typed[:head] + strokes)
            # A token the boundary model could not split was not decided at all;
            # the next word may still show what it was (`dc\`` before `тот`).
            if (enabled and trigger_enabled and not manual_layout_protected and not caret_unknown
                    and not segmentation_certain and not excluded and not head):
                self._keep_for_next_word(kept, KeptWord(
                    plan, None, None, self._focus_window or 0, time.monotonic() + CONTEXT_TTL), boundary)
        self._log_word_evaluation(
            trigger=self._trigger_for_boundary(boundary),
            original=original,
            alternatives=alternatives,
            application=application,
            enabled=enabled,
            trigger_enabled=trigger_enabled,
            manual_layout_protected=manual_layout_protected,
            caret_unknown=caret_unknown,
            application_excluded=excluded,
            decision=decision,
            protection=protection,
            source_group=source_group,
            early_switch_origin=early_switch_origin,
            context=context,
            literal_head=self._text_for_group(typed[:head], source_group),
        )
        if decision is None or not decision.should_convert:
            self._finish_early_switch(typed[:head] + strokes + signs, boundary, application, source_group, trailing)
        else:
            self._early_switch_origin = None
            self._early_switch_at = None
        if mention and not (self._pending is not None and self._pending.head):
            self._technical_event(
                "mention_head_kept",
                reason=(
                    "word_not_analysed" if decision is None
                    else "converted_without_head" if decision.should_convert else "word_kept"
                ),
                shown=shown is not None,
                application=application,
                application_excluded=excluded,
            )
        if shown is not None and not (self._pending is not None and self._pending.head):
            # The word ended before the key after the "@" could write the quote
            # back (keys held over each other). A word that converts took the
            # head with it above; any other ending means it was not a mention.
            self._log_pending_dropped("mention_head_written_back")
            self._pending = self._mention_write_back(
                shown, typed[:len(typed) - len(trailing)], trailing,
                None if boundary.deferred else boundary, application,
            )
            self._learning_prompt_after = None
            self._pending_trigger_keycode = boundary.keycode
        self._strokes = []
        self._insertion = None
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

        if self.boundary_model is not None:
            # No completed-word classifier gets to cut an unfinished prefix.
            # Retain the key until a hard boundary or an idle evaluation.
            return False
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
            effective_length >= NATURAL_SOURCE_BOUNDARY_MIN_CHARACTERS
            and decision.source_score.ngram_score >= NATURAL_SOURCE_BOUNDARY_NGRAM_FLOOR
        )
        # A one- or two-letter word from the trusted list is recognisable, but
        # it is just as often the start of a longer word whose next letter
        # sits on this key: `j,ob[` is "общих", not "о" and a comma.
        recognisable = decision.should_convert and not is_short_word_override(decision)
        return recognisable or protected_boundary or ignored_boundary or (
            decision.source_score.known
            and effective_length
            >= self._minimum_word_length()
        ) or natural_source_boundary

    def _completed_word(
        self, strokes: tuple[KeyEvent, ...], source_group: int,
    ) -> tuple[tuple[KeyEvent, ...], tuple[KeyEvent, ...], bool]:
        model = self.boundary_model
        if model is None or source_group not in self.models:
            return strokes, (), True
        tail = 0
        for stroke in reversed(strokes):
            if stroke.character.isalpha() or not self._is_layout_letter(stroke):
                break
            tail += 1
        if not tail:
            return strokes, (), True
        targets = [group for group in self.models if group != source_group]
        if tail >= len(strokes) or tail > MAX_SUFFIX or not targets:
            self._technical_event(
                "boundary_guard", model_version=model.version,
                reason="no_word" if tail >= len(strokes) else "suffix_too_long" if tail > MAX_SUFFIX else "no_target",
                observed_characters=len(strokes), ambiguous_tail=tail,
            )
            return strokes, (), False
        original = self._text_for_group(strokes, source_group)
        if self._forced_target_group(source_group, original) is not None:
            return strokes, (), True  # Explicit full-token rule outranks segmentation.
        # Segmentation cannot turn an excluded token/path into an eligible word.
        ignored: list[str] = self.settings.get("exclusions.words", [])
        if ((bool(self.settings.get("detection.protect_code", True)) and self.detector.is_protected_token(original))
                or original.casefold() in {word.casefold() for word in ignored}
                or set(targets) <= self._rejected_targets(source_group, original)):
            return strokes, (), False
        target = targets[0]
        alternative = self._text_for_group(strokes, target)
        extract = model.extract if isinstance(model, BoundaryPolicy) else boundary_features
        prediction = model.predict(tuple(
            extract(original, alternative, length, self.models[source_group], self.models[target])
            for length in range(tail + 1)
        ))
        length = prediction.suffix_length
        self._technical_event(
            "boundary_decision", model_version=prediction.version,
            action="abstain" if length is None else "literal" if length else "word",
            score=round(prediction.probability, LOGGED_SCORE_DECIMALS), preserved_characters=length,
            candidates=tail + 1, observed_characters=len(strokes),
        )
        if length is None:
            return strokes, (), False
        return (strokes[:-length], strokes[-length:], True) if length else (strokes, (), True)

    def _literal_head(self, strokes: tuple[KeyEvent, ...], source_group: int) -> int:
        """Count leading strokes up to the last ``/`` that stay literal.

        A slash is punctuation in both layouts, so ``bild/c,jhrb`` or a chat
        ``/c,jhrb`` would hide the wrong-layout ``сборки`` behind the code
        guard. Only the word after the last slash is analysed, and only when
        the contextual policy can weigh the surrounding text: the legacy
        detector alone keeps treating the whole token as code. The head is
        never replaced. A head that itself looks like a wrong-layout word
        abstains: one span cannot fix two words, and manual Pause still
        converts the whole token.
        """

        if (
            source_group not in self.models
            or not bool(self.settings.get("detection.protect_code", True))
            or not bool(self.settings.get("detection.context_aware", True))
            or str(self.settings.get("detection.context_policy", "assist")) != "assist"
            or self.context_policy.model is None
        ):
            return 0
        slashes = [index for index, stroke in enumerate(strokes) if stroke.character == "/"]
        targets = [group for group in self.models if group != source_group]
        head = slashes[-1] + 1 if slashes else 0
        core = strokes[head:]
        if (
            not slashes or not targets or not core
            or not self.detector.is_protected_token(self._text_for_group(strokes, source_group))
            or self.detector.is_protected_token(self._text_for_group(core, source_group))
            or not all(
                stroke.character.isalpha() or self._is_layout_letter(stroke) or stroke.character in WORD_JOINERS
                for stroke in core
            )
        ):
            return 0
        segment: list[KeyEvent] = []
        for stroke in strokes[:head]:
            if stroke.character != "/":
                segment.append(stroke)
            elif segment and not self._head_segment_settled(tuple(segment), source_group, targets[0]):
                return 0
            else:
                segment = []
        return head

    def _head_segment_settled(self, segment: tuple[KeyEvent, ...], source_group: int, target: int) -> bool:
        """A head segment is settled when nothing suggests it needs conversion."""

        original = self._text_for_group(segment, source_group)
        if self.detector.is_protected_token(original):
            return True
        alternative = self._text_for_group(segment, target)
        if self.models[target].score(alternative).known:
            return False
        decision = self.detector.decide(
            original, {target: alternative}, source_group,
            minimum_length=1,
            confidence_threshold=self._confidence_threshold(),
            aggressive=bool(self.settings.get("detection.aggressive", False)),
            protect_code=True,
            use_intent_model=bool(self.settings.get("detection.intent_model_enabled", True)),
        )
        return not decision.should_convert

    def _planned_baseline(
        self, original: str, alternatives: dict[int, str], source_group: int,
        next_word: str, next_group: int,
    ) -> DetectionDecision:
        """The detector's verdict on a word once a planned neighbour is known.

        Same settings as the ordinary word decision; the only context is the
        neighbour in its planned layout and that layout's language: the converted
        next word of a waiting word, which is what the corpus curriculum supplies
        for planned frames, or the other reading of the waiting word before it.
        """
        ignored_words: list[str] = self.settings.get("exclusions.words", [])
        rejected_targets = self._rejected_targets(source_group, original)
        forced_target = self._forced_target_group(source_group, original)
        return automatic_word_decision(
            self.detector, original, alternatives, source_group,
            minimum_length=(1 if forced_target is not None else self._minimum_word_length()),
            confidence_threshold=self._confidence_threshold(),
            ignored_words=set(ignored_words),
            aggressive=bool(self.settings.get("detection.aggressive", False)),
            protect_code=bool(self.settings.get("detection.protect_code", True)),
            previous_words={next_group: next_word}, context_group=next_group,
            forced_target_group=forced_target, rejected_targets=rejected_targets, trigger="space",
            use_intent_model=bool(self.settings.get("detection.intent_model_enabled", True)),
        )

    def _begin_word(self, event: KeyEvent) -> None:
        if self._strokes:
            return
        if self._position_unknown:
            self._insertion = self._read_insertion(event)
        self._position_unknown = False

    def _read_insertion(self, event: KeyEvent) -> InsertionPoint | None:
        """What stands around the caret where a word begins after a click or a caret move.

        Clicking into written text and typing is how a word is fixed in place:
        `сд|лать` gets its missing `е`. The engine saw none of that text, so
        the letters were judged as a word of their own at the start of an empty
        field. The field is read once, as the word begins, and the letters
        right before and after the caret say whether this is a new word or the
        middle of an existing one (_decide_inside_word).
        """

        reader = self.context_policy.reader
        if reader is None or not bool(self.settings.get("detection.context_read_field", False)):
            return None
        application = self.backend.active_application()
        if self._application_excluded(application):
            return None
        snapshot = reader.read(application, self._focus_window or 0)
        if snapshot is None or snapshot.sensitive or snapshot.selection or snapshot.application != application:
            return None
        before = snapshot.before
        # The key may already have reached the editor, or not yet.
        if event.character and before.endswith(event.character):
            before = before[:-len(event.character)]
        point = InsertionPoint(self._letters_before(before), self._letters_after(snapshot.after))
        if point.inside_word:
            self._technical_event(
                "word_started_inside_text", application=application,
                head_letters=len(point.head), tail_letters=len(point.tail),
            )
        return point

    @staticmethod
    def _letters_before(text: str) -> str:
        start = len(text)
        while start and text[start - 1].isalpha():
            start -= 1
        return text[start:]

    @staticmethod
    def _letters_after(text: str) -> str:
        end = 0
        while end < len(text) and text[end].isalpha():
            end += 1
        return text[:end]

    def _layout_name(self, group: int) -> str:
        names: list[str] = self.settings.get("detection.layouts", ["us", "ru"])
        return names[group] if 0 <= group < len(names) else ""

    def _layout_letters(self, group: int) -> frozenset[str]:
        keys = {"us": US_KEYS, "ru": RU_KEYS}.get(self._layout_name(group), "")
        return frozenset(character for key in keys if key.isalpha() for character in (key, key.upper()))

    def _decide_inside_word(
        self, strokes: tuple[KeyEvent, ...], source_group: int, alternatives: dict[int, str],
        application: str, trigger: CorrectionTrigger, boundary: KeyEvent | None,
    ) -> DetectionDecision | None:
        """Letters typed into the middle of a word are judged as that whole word.

        `t` typed between `сд` and `лать` is not a word: the question is what the
        whole word is. When the letters around it are in the other layout, the
        word is put to the usual decision as if all of it had been typed in the
        fragment's layout - `cltkfnm` against `сделать` - and only the typed
        fragment is replaced. When they are in the fragment's own layout, the
        fragment agrees with its word and is left alone. At a boundary the word
        ends there, so only the letters before the caret belong to it.
        Returns None when the word was not begun inside text.
        """

        point = self._insertion
        target = next(iter(alternatives), None)
        reader = self.context_policy.reader
        if point is None or not point.inside_word or target is None or reader is None:
            return None
        fragment = self._text_for_group(strokes, source_group)
        closing = "" if boundary is None or boundary.deferred else boundary.character
        snapshot = reader.read(application, self._focus_window or 0)
        before: str | None = None
        if (snapshot is not None and not snapshot.sensitive and not snapshot.selection
                and snapshot.application == application):
            for suffix in ((fragment + closing, fragment) if closing else (fragment,)):
                if snapshot.before.endswith(suffix):
                    before = snapshot.before[:-len(suffix)]
                    break
        if snapshot is None or before is None:
            return self._kept_inside_word(fragment, source_group, "field_changed", point)
        head = self._letters_before(before)
        tail = "" if boundary is not None else self._letters_after(snapshot.after)
        if not head and not tail:
            return None
        surroundings = head + tail
        source_letters, target_letters = self._layout_letters(source_group), self._layout_letters(target)
        if not target_letters or not all(character in target_letters for character in surroundings) or any(
                character in source_letters for character in surroundings):
            same = bool(source_letters) and all(character in source_letters for character in surroundings)
            return self._kept_inside_word(fragment, source_group, "same_layout" if same else "mixed_layout", point)
        pair, source_name, target_name = LayoutPair(), self._layout_name(source_group), self._layout_name(target)
        whole = pair.translate(head, target_name, source_name) + fragment + pair.translate(tail, target_name, source_name)
        # The model is told that the word is being edited in place: every other letter of
        # it is already in the other layout.
        decision = self._decide_word(
            whole, {target: head + alternatives[target] + tail}, source_group, application, trigger,
            boundary_text=closing,
            field_override=replace(snapshot, before=before[:len(before) - len(head)], after=snapshot.after[len(tail):]),
            inside=True,
        )
        converted = decision.should_convert and decision.target_group == target
        self._technical_event(
            "inside_word_decision", application=application, surroundings="other_layout",
            head_letters=len(head), tail_letters=len(tail), converted=converted,
        )
        if not converted:
            # A wait or a suggestion is about a next word; this one is inside text.
            self._context_result = None
            return replace(decision, should_convert=False, original=fragment, replacement=fragment)
        return replace(decision, original=fragment, replacement=alternatives[target])

    def _kept_inside_word(
        self, fragment: str, source_group: int, reason: str, point: InsertionPoint,
    ) -> DetectionDecision:
        self._technical_event(
            "inside_word_decision", surroundings=reason,
            head_letters=len(point.head), tail_letters=len(point.tail), converted=False,
        )
        score = self.models[source_group].score(fragment)
        return DetectionDecision(
            False, fragment, fragment, source_group, source_group, 0.0,
            "слово набрано внутри другого слова", score, score,
        )

    def _decide_word(
        self,
        original: str,
        alternatives: dict[int, str],
        source_group: int,
        application: str,
        trigger: CorrectionTrigger = "space",
        *, literal_tail: str = "", boundary_text: str = "",
        field_override: FieldContext | None = None, inside: bool = False,
    ) -> DetectionDecision:
        self._context_result = None
        self._last_baseline = None
        decision = self._baseline_decision(original, alternatives, source_group, application, trigger)
        self._last_baseline = decision
        context_aware = bool(self.settings.get("detection.context_aware", True))
        ignored_words: list[str] = self.settings.get("exclusions.words", [])
        rejected_targets = self._rejected_targets(source_group, original)
        forced_target = self._forced_target_group(source_group, original)
        if (
            not context_aware or forced_target is not None or trigger == "boundary_probe"
            or self.detector.token_key(original) in {self.detector.token_key(word) for word in ignored_words}
            or self._application_excluded(application)
        ):
            # A token the user excluded, a layout they chose by hand and an excluded
            # application are their own decisions; the model is not asked about those.
            # "Protect code" is not in that list any more: it used to skip the model for
            # every token carrying a digit, so `зь2` stayed while the model said convert
            # with 0.9993 - and the model keeps `pm2`, `npm`, `git` and `h264` on its own
            # (measured 17.09.2026). The detector still refuses such tokens in the
            # baseline; telling code from a mistyped word is exactly what the model is for.
            return decision
        return self._consult_context_model(
            decision, original, alternatives, rejected_targets, application, trigger,
            literal_tail=literal_tail, boundary_text=boundary_text, field_override=field_override, inside=inside,
        )

    def _rejected_targets(self, source_group: int, original: str) -> set[int]:
        """Every other layout when a "keep" rule of the user fits the word."""

        if not bool(self.settings.get("detection.learning", True)) or not self.learning.keeps(source_group, original):
            return set()
        return {group for group in self.models if group != source_group}

    def _baseline_decision(
        self, original: str, alternatives: dict[int, str], source_group: int,
        application: str, trigger: CorrectionTrigger,
    ) -> DetectionDecision:
        """What the detector decides about a word before the context model is asked."""

        context_words, context_group = self._context_for(application)
        context_aware = bool(self.settings.get("detection.context_aware", True))
        ignored_words: list[str] = self.settings.get("exclusions.words", [])
        protect_code = bool(self.settings.get("detection.protect_code", True))
        forced_target = self._forced_target_group(source_group, original)
        rejected_targets = self._rejected_targets(source_group, original)
        return automatic_word_decision(
            self.detector,
            original,
            alternatives,
            source_group,
            minimum_length=(
                1 if forced_target is not None
                else self._minimum_word_length()
            ),
            confidence_threshold=self._confidence_threshold(),
            ignored_words=set(ignored_words),
            aggressive=bool(self.settings.get("detection.aggressive", False)),
            protect_code=protect_code,
            previous_words=context_words if context_aware else {},
            context_group=context_group if context_aware else None,
            forced_target_group=forced_target,
            rejected_targets=rejected_targets,
            trigger=trigger,
            use_intent_model=bool(
                self.settings.get("detection.intent_model_enabled", True)
            ),
            # Context is kept per named application (_remember_context).
            context_tracked=context_aware and bool(application.strip()),
        )

    def _consult_context_model(
        self, decision: DetectionDecision, original: str, alternatives: dict[int, str],
        rejected_targets: set[int], application: str, trigger: CorrectionTrigger,
        *, literal_tail: str, boundary_text: str, field_override: FieldContext | None, inside: bool,
    ) -> DetectionDecision:
        candidates = [(group, text) for group, text in alternatives.items() if group not in rejected_targets]
        if not candidates:
            return decision
        group, alternative = candidates[0]
        result = self.context_policy.decide(
            decision, alternative, group, self.detector, trigger,
            str(self.settings.get("detection.context_policy", "assist")),
            read_field=field_override is None and bool(self.settings.get("detection.context_read_field", False)),
            literal_tail=literal_tail, boundary_text=boundary_text, field_override=field_override, inside=inside,
        )
        self._context_result = result
        if result.field is not None and result.field.sensitive:
            self._sensitive_context_window = self._focus_window
            self.context_policy.stream.clear()
            self._contexts.clear()
            self._update(current_word="", last_action="Защищённое поле: обработка отключена")
        if result.prediction is not None:
            prediction = result.prediction
            self._update(context_action=prediction.action, context_model=prediction.model_version)
            field = result.field
            self._technical_event(
                "context_decision", action=prediction.action,
                score=round(prediction.probability, LOGGED_SCORE_DECIMALS), model_version=prediction.model_version,
                mode=self.settings.get("detection.context_policy", "assist"),
                applied=result.decision.should_convert, baseline_convert=decision.should_convert,
                baseline_reason=decision.reason,
                model_supported=prediction.supported,
                policy_applied=result.policy_applied,
                decision_source=result.decision_source,
                fallback_reason=result.fallback_reason,
                final_action="convert" if result.decision.should_convert else "keep",
                field_read_requested=bool(self.settings.get("detection.context_read_field", False)),
                field_reader_status=self._field_reader_status(),
                field_reader_details=self._field_reader_details(),
                context_source=field.source if field else "unavailable",
                before_characters=len(field.before) if field else 0,
                after_characters=len(field.after) if field else 0,
                field_role=field.role if field else "unknown",
                # Context content and surrounding sentences are never logged.
            )
            if prediction.action == "suggest" and self.settings.get("detection.context_policy", "assist") == "assist":
                self._update(last_action=f"Возможно: {original} → {alternative} · Pause для замены")
        return result.decision

    def _resolve_context_wait(
        self, waiting: WaitingContextWord | None, strokes: tuple[KeyEvent, ...],
        boundary: KeyEvent | None, decision: DetectionDecision, application: str,
        alternatives: dict[int, str],
    ) -> tuple[CorrectionPlan, DetectionDecision] | None:
        """Decide a waiting word together with the next word, at its boundary or at a pause."""

        if waiting is None or self.settings.get("detection.context_policy", "assist") != "assist":
            self._log_context_wait("context_wait_cancelled", waiting, "policy_disabled")
            return None
        previous = waiting.plan
        if (
            time.monotonic() > waiting.deadline or waiting.window != (self._focus_window or 0)
            or previous.application != application or previous.boundary is None
            or previous.source_group != decision.source_group
            or any(stroke.group != previous.source_group for stroke in (*previous.strokes, *strokes))
        ):
            self._log_context_wait(
                "context_wait_cancelled", waiting, "preconditions_changed",
                expired=time.monotonic() > waiting.deadline,
                same_window=waiting.window == (self._focus_window or 0),
                same_application=previous.application == application,
                boundary_available=previous.boundary is not None,
                same_layout=previous.source_group == decision.source_group,
                strokes_same_layout=all(stroke.group == previous.source_group for stroke in (*previous.strokes, *strokes)),
            )
            return None
        if not decision.should_convert:
            # The next word was judged after the waiting word as it was typed, and
            # that word is exactly what is in doubt: `tot d` reads as English only
            # while `tot` is taken for English. The model is asked about the next
            # word once more after the waiting word's other reading, `еще в`; the
            # pair converts only if it then converts the next word and, below, the
            # waiting word with it.
            converted = self._decide_after_waiting_word(
                waiting, decision, alternatives, boundary, previous.boundary.character)
            if converted is None:
                self._log_context_wait("context_wait_cancelled", waiting, "next_word_not_converted")
                return None
            decision = converted
        original = previous.original + previous.boundary.character + decision.original
        # At a pause the next word has no boundary yet; Enter/Tab has not reached the editor.
        closing = None if boundary is None or boundary.deferred else boundary
        suffix = "" if closing is None else closing.character
        if not self.context_policy.stream.text.endswith(original + suffix):
            self._log_context_wait("context_wait_cancelled", waiting, "observed_suffix_changed")
            return None
        group = decision.target_group
        alternative = self._text_for_group(previous.strokes, group)
        # A feature-version-3 context model re-decides the waiting word with the
        # converted next word as its language context, exactly as a word after an
        # existing neighbour would be; the installed feature-version-2 model keeps
        # the decision it was certified with.
        model = self.context_policy.model
        planned_baseline = (
            self._planned_baseline(previous.original, {group: alternative}, previous.source_group, decision.replacement, group)
            if model is not None and model.feature_version == CONTEXT_ACTION_FEATURE_VERSION else waiting.decision)
        result = self.context_policy.decide(
            planned_baseline, alternative, group, self.detector, "space", "assist",
            after=decision.replacement, field_override=waiting.field,
            boundary_text=previous.boundary.character,
            after_origin="planned_next_conversion",
        )
        if not result.decision.should_convert:
            self._log_context_wait("context_wait_cancelled", waiting, "lookahead_not_converted")
            return None
        self._technical_event("context_wait_resolved", wait_id=waiting.diagnostic_id, previous_characters=len(previous.original), next_characters=len(decision.original))
        return CorrectionPlan(
            previous.strokes + (previous.boundary,) + strokes,
            closing, previous.source_group, group,
            original, alternative + previous.boundary.character + decision.replacement,
            result.decision.confidence, application, True, "context_phrase", self._context_field_id(),
        ), decision

    def _decide_after_waiting_word(
        self, waiting: WaitingContextWord, decision: DetectionDecision,
        alternatives: dict[int, str], boundary: KeyEvent | None, boundary_character: str,
    ) -> DetectionDecision | None:
        """The next word's decision after the waiting word's other reading.

        The detector gets that reading as its previous word and the model gets it
        in front of the caret, exactly as they would after a word typed in that
        layout. Nothing is decided here that the model does not decide.
        """

        previous = waiting.plan
        group = previous.target_group
        alternative = alternatives[group]
        reading = self._text_for_group(previous.strokes, group)
        baseline = self._planned_baseline(decision.original, {group: alternative}, decision.source_group, reading, group)
        result = self.context_policy.decide(
            baseline, alternative, group, self.detector,
            "pause" if boundary is None else self._trigger_for_boundary(boundary), "assist",
            field_override=replace(waiting.field, before=waiting.field.before + reading + boundary_character),
        )
        return result.decision if result.decision.should_convert else None

    def _revert_with_next_word(
        self, provisional: ProvisionalWord | None, strokes: tuple[KeyEvent, ...],
        boundary: KeyEvent | None, decision: DetectionDecision, application: str,
    ) -> tuple[CorrectionPlan, DetectionDecision] | None:
        """Ask the model about a converted word again when the next word goes the other way.

        `ns` alone becomes `ты` and the layout follows, so the next word is typed in
        Russian. When the engine then converts that word into the language `ns` was
        typed in - `vs` and `code` come out as `мы сщву` - the first word is decided
        once more with the converted next word after it, exactly as a waiting word
        is (_resolve_context_wait). If the model no longer converts it, one
        correction returns both: `vs code`. The engine only asks; the model decides.
        """

        if (provisional is None or not decision.should_convert
                or self.settings.get("detection.context_policy", "assist") != "assist"):
            return None
        applied = provisional.plan
        if (
            applied.revisit is None or applied.boundary is None
            or decision.target_group != applied.source_group
            or time.monotonic() > provisional.deadline or provisional.window != (self._focus_window or 0)
            or applied.application != application
            or any(stroke.group != applied.target_group for stroke in strokes)
        ):
            return None
        original = applied.replacement + applied.boundary.character + decision.original
        closing = None if boundary is None or boundary.deferred else boundary
        if not self.context_policy.stream.text.endswith(original + ("" if closing is None else closing.character)):
            return None
        baseline, field = applied.revisit
        # The question is how to read the word now that its neighbour went the other
        # way, not what the detector thinks of it alone: it is asked with no baseline
        # conversion, as the model was taught it (train_context_model: short_revisit),
        # and no curated rule answers in the model's place.
        baseline = replace(baseline, should_convert=False, reason=REVISIT_REASON)
        again = self.context_policy.decide(
            baseline, applied.replacement, applied.target_group, self.detector, "space", "assist",
            after=decision.replacement, field_override=field,
            boundary_text=applied.boundary.character, after_origin="planned_next_conversion",
        )
        if again.decision.should_convert:
            return None
        self._technical_event(
            "converted_word_reverted", previous_characters=len(applied.original),
            next_characters=len(decision.original),
        )
        return CorrectionPlan(
            applied.strokes + (applied.boundary,) + strokes,
            closing, applied.target_group, applied.source_group,
            original, applied.original + applied.boundary.character + decision.replacement,
            decision.confidence, application, True, "context_phrase", self._context_field_id(),
        ), decision

    def _keep_for_next_word(
        self, kept: tuple[KeptWord, ...], word: KeptWord | None, boundary: KeyEvent,
    ) -> None:
        """Remember the words left as typed at a space until a later word shows their layout."""

        if boundary.character != " " or boundary.deferred:
            return
        if word is not None and not any(char.isalpha() for char in word.plan.original):
            # A token of signs alone (`.`, `1.2`, `...`) is never converted on its own, and a
            # converted next word does not make it a word either: `.` before `ghbdtn` is no `ю`.
            return
        chain = kept if word is None or word.plan.target_group == word.plan.source_group else (*kept, word)
        self._kept = chain[-KEPT_WORDS_TAKEN_ALONG:]

    def _sign_reads_as_letter(self, plan: CorrectionPlan) -> bool:
        """Only the whole token, the sign split off its end included, reads as a word in the other layout.

        `t\\`` is `её`, while `t` alone, `е`, is no word. When the word without the
        sign is a word itself, the boundary model's split stands.
        """

        model = self.models[plan.target_group]
        whole = self._text_for_group(plan.strokes + plan.trailing, plan.target_group)
        return model.score(whole).known and not model.score(plan.replacement).known

    def _take_along(
        self, kept: tuple[KeptWord, ...], plan: CorrectionPlan, first_characters: int, application: str,
    ) -> CorrectionPlan:
        """Ask the model about the words before a converted word again, nearest first.

        The first words of a line have nothing before them to tell their layout
        by: `руку` is a Russian word and `here` an English one, and `dc\\`` is `всё` or
        `вс` with a backtick. When a word typed in the same layout converts - `ерун` is
        `they` - the model is asked about the word before it once more with the
        converted word after it, as a waiting word is (_resolve_context_wait); while
        it converts them, one correction takes the words along: `hey here they`. The
        mirror of _revert_with_next_word; the engine only asks, the model decides.

        Only the first words of a line are taken along: a word with words before it
        on its line was decided with them, and asked again it reads to the model like
        a first word. A Russian word typed as intended before a term typed in the other
        layout was converted then: `склонируй репо пшерги` would become
        `склонируй htgj github`.
        """

        if self.settings.get("detection.context_policy", "assist") != "assist":
            return plan
        next_replacement = plan.replacement[:first_characters]
        # Enter/Tab has not reached the editor: it is neither in the text nor replaced. A sign
        # split off the converted word's end stays literal after the whole correction.
        closing = None if plan.boundary is None or plan.boundary.deferred else plan.boundary
        suffix = "".join(stroke.character for stroke in plan.trailing) + ("" if closing is None else closing.character)
        start, taken = plan, list[KeptWord]()
        for word in reversed(kept):
            previous = word.plan
            if (
                previous.boundary is None
                or plan.source_group != previous.source_group or plan.target_group != previous.target_group
                or time.monotonic() > word.deadline or word.window != (self._focus_window or 0)
                or previous.application != application
                or any(stroke.group != previous.source_group for stroke in (*previous.strokes, *plan.strokes))
                or not self.context_policy.stream.text.endswith(
                    previous.original + previous.boundary.character + plan.original + suffix)
                # Only a word whose other reading is a word is asked: a term typed as intended
                # before a Russian word typed in the English layout (`htop gjrfpsdftn`) is no
                # `рещз`, yet asked with `показывает` after it the model converts it.
                or not self.models[previous.target_group].score(previous.replacement).known
            ):
                break
            baseline = word.baseline if word.baseline is not None else self._baseline_decision(
                previous.original, {previous.target_group: previous.replacement}, previous.source_group,
                application, "space")
            again = self.context_policy.decide(
                baseline, previous.replacement, previous.target_group, self.detector, "space", "assist",
                after=next_replacement, field_override=word.field,
                boundary_text=previous.boundary.character, after_origin="planned_next_conversion",
            )
            if not again.decision.should_convert:
                break
            taken.append(word)
            plan = replace(
                plan, strokes=previous.strokes + (previous.boundary,) + plan.strokes, boundary=closing,
                original=previous.original + previous.boundary.character + plan.original,
                replacement=previous.replacement + previous.boundary.character + plan.replacement,
                automatic=True, mode="context_phrase", context_field=self._context_field_id(),
            )
            next_replacement = previous.replacement
        if not taken:
            return start
        farthest = taken[-1]
        before = (farthest.field.before if farthest.field is not None
                  else self.context_policy.stream.text[:-len(plan.original + suffix)])
        if any(char.isalpha() for char in before.rsplit("\n", 1)[-1]):
            self._technical_event("kept_words_left", words=len(taken), reason="words_before_on_line")
            return start
        for word in taken:
            self._technical_event(
                "kept_word_converted", previous_characters=len(word.plan.original),
                boundary_undecided=word.baseline is None,
            )
        return plan

    def _opening_conversion(self, inside: DetectionDecision | None, original: str) -> bool:
        """The model converted a short word that opens the message, with nothing before it."""

        result = self._context_result
        return (
            inside is None and result is not None and result.decision_source == "context_model"
            and result.decision.should_convert and result.field is not None and not result.field.before.strip()
            and len(LanguageModel.normalize(original)) <= TRUSTED_SHORT_WORD_MAX_LENGTH
        )

    def _start_context_wait(
        self, plan: CorrectionPlan, decision: DetectionDecision,
        boundary: KeyEvent, trailing: tuple[KeyEvent, ...], head: int, *, fallback: bool = False,
    ) -> bool:
        """Let a word at a space wait for its next word; say whether it waits.

        The model asks for it with a ``wait`` verdict, whatever the word's length:
        `tot` at the start of a message may be `еще`, and only its neighbour can
        tell. A ``suggest`` verdict - the model leans to converting but is not sure
        enough - waits too, for the same neighbour: `vs` after a Russian sentence
        was left as it was with p=0.984 even when the next word turned out to be
        `хотим` typed in the same wrong layout. Such a word is only asked again
        with its neighbour, not at a pause. A lone letter converted by the
        message-start rule waits as well, whatever the model said: if the user
        keeps typing, the immediate correction aborts as unsafe and the next word
        has to decide the letter; if it went through, the observed text no longer
        matches and the wait cancels itself.
        """

        result = self._context_result
        settles = result is not None and result.prediction is not None and (
            result.prediction.action == "wait" or fallback)
        if (
            result is not None and result.prediction is not None and result.field is not None
            and (settles or result.prediction.action == "suggest")
            and boundary.character == " "
            and not trailing and not head
            and not boundary.deferred
            and self.settings.get("detection.context_policy", "assist") == "assist"
        ):
            self._context_wait_sequence += 1
            # A word waits as long as the text around it stays context. The wait
            # used to lapse after ten seconds, so `tot`, a thought and then `ghbdtn`
            # twelve seconds later gave `tot привет` (0.31.0 log, 24.09.2026).
            # Anything that makes the pair stale ends the wait sooner: another
            # window, a caret move, Backspace, a changed field.
            self._context_waiting = WaitingContextWord(
                plan, decision, result.field, self._focus_window or 0, time.monotonic() + CONTEXT_TTL,
                self._context_wait_sequence, settles,
            )
            self._log_context_wait("context_wait_started", self._context_waiting, "model_wait" if settles else "model_suggest")
            return True
        return False

    def _log_context_wait(
        self, event: str, waiting: WaitingContextWord | None, reason: str, **fields: object,
    ) -> None:
        if waiting is None:
            return
        self._technical_event(
            event, wait_id=waiting.diagnostic_id, reason=reason,
            word_characters=len(waiting.plan.original),
            source_group=waiting.plan.source_group, target_group=waiting.plan.target_group,
            remaining_ms=max(0, round((waiting.deadline - time.monotonic()) * MILLISECONDS_PER_SECOND)),
            **fields,
        )

    def _cancel_context_wait(self, reason: str) -> None:
        self._log_context_wait("context_wait_cancelled", self._context_waiting, reason)
        self._context_waiting = None
        # Whatever ends a wait - another field, a caret move, Backspace, a manual
        # conversion - also ends a converted or kept word's claim on the next word.
        self._provisional = None
        self._kept = ()

    def _log_input_edit(self, event: KeyEvent, application: str) -> None:
        # Observe editing controls, not printable keystrokes or field contents.
        if not bool(self.settings.get("diagnostics.technical_logging", False)) or not bool(self.settings.get("enabled", True)) or self._application_excluded(application):
            return
        if event.control or event.alt or event.super_key:
            edit = "shortcut"
        elif event.key_name in {"BackSpace", "Delete"}:
            edit = "backspace" if event.key_name == "BackSpace" else "delete"
        elif event.key_name == "Pointer":
            edit = "pointer"
        elif event.key_name in NAVIGATION_KEYS:
            edit = "navigation"
        else:
            return
        self._technical_event(
            "input_edit_observed", edit=edit, application=application,
            group=event.group, tracked_word_characters=len(self._strokes),
            context_characters_before=len(self.context_policy.stream.text),
            last_committed_available=self._last_committed is not None and not self._last_committed_stale,
            wait_id=self._context_waiting.diagnostic_id if self._context_waiting is not None else None,
            text_verified=False,
        )

    def _forced_target_group(self, source_group: int, word: str) -> int | None:
        if not bool(self.settings.get("detection.learning", True)):
            return None
        return self.learning.forced_target(source_group, word)

    def _context_for(self, application: str) -> tuple[dict[int, str], int | None]:
        if not application.strip():
            return {}, None
        key = application.casefold()
        context = self._contexts.get(key)
        if context is None or time.monotonic() - context.updated_at > CONTEXT_TTL:
            return {}, None
        return dict(context.words), context.group

    def _remember_context(
        self,
        application: str,
        group: int,
        strokes: tuple[KeyEvent, ...] | list[KeyEvent],
    ) -> None:
        if group not in self.models or not strokes or not application.strip() or self._application_excluded(application):
            return
        words = {
            candidate_group: self._text_for_group(strokes, candidate_group)
            for candidate_group in self.models
        }
        key = application.casefold()
        self._contexts.pop(key, None)
        self._contexts[key] = LanguageContext(group, words, time.monotonic())
        while len(self._contexts) > MAX_REMEMBERED_APPLICATION_CONTEXTS:
            self._contexts.pop(next(iter(self._contexts)))

    def _replayed_signs(self, strokes: tuple[KeyEvent, ...], source_group: int) -> int:
        """Letters at the end of a token that are signs in the other layout, when the token is no word.

        `hello,` typed in the Russian layout is `руддщб`: the comma is the key of `б`. The
        word is judged without them and, if it converts, they are replayed with it; a
        token that is a word as typed (`хлеб`) keeps its letters.
        """

        targets = [group for group in self.models if group != source_group]
        if not targets:
            return 0
        count = 0
        for stroke in reversed(strokes):
            meant = stroke.character_for(targets[0])
            if not stroke.character.isalpha() or not meant or meant.isalpha() or meant.isspace():
                break
            count += 1
        if (not count or len(strokes) - count < REPLAYED_SIGNS_MIN_STEM_LETTERS
                or any(not stroke.character.isalpha() for stroke in strokes[:-count])
                or self.models[source_group].score(self._text_for_group(strokes, source_group)).known):
            # A token with digits or signs inside (`з+1ю` for `p+1.`) is judged whole, as before, and
            # so is one whose rest is a single letter: `чё` is not `ч` with a backtick.
            return 0
        return count

    def _with_signs(self, plan: CorrectionPlan, signs: tuple[KeyEvent, ...]) -> CorrectionPlan:
        if not signs:
            return plan
        return replace(
            plan, strokes=plan.strokes + signs,
            original=plan.original + self._text_for_group(signs, plan.source_group),
            replacement=plan.replacement + self._text_for_group(signs, plan.target_group),
        )

    @staticmethod
    def _replay_sign_with_word(plan: CorrectionPlan, opened: tuple[KeyEvent, ...]) -> CorrectionPlan:
        """Replay a sign typed right after a converted word in the new layout too.

        A person typing Russian in the English layout presses the keys of the Russian
        layout for signs as well: the comma is Shift+/, so `ghbdtn?` is `привет,`, and
        `ltkf&` is `дела?`; `support@mail.ru` typed in the Russian layout has `"` where
        `@` was meant. A key that is a sign in both layouts but a different one was
        pressed for the layout the word was meant in - unless it is a quotation mark and
        one opened the word (`opened`): then it closes the quotation, and `"john"` keeps
        both quotes.
        """

        boundary = plan.boundary
        if (boundary is None or boundary.deferred or plan.trailing or plan.head or boundary.character.isspace()
                or (boundary.character == '"' and any(stroke.character == '"' for stroke in opened))):
            return plan
        meant = boundary.character_for(plan.target_group)
        if not meant or meant == boundary.character or meant.isalpha() or meant.isspace():
            return plan
        return replace(
            plan, strokes=plan.strokes + (boundary,), boundary=None, sign_replayed=True,
            original=plan.original + boundary.character, replacement=plan.replacement + meant,
        )

    def _plan_from_decision(
        self,
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
            self._context_field_id(),
        )

    def _context_field_id(self) -> str:
        result = self._context_result
        return result.field.field_id if result is not None and result.field is not None and result.field.source != "observed" else ""

    @staticmethod
    def _score_diagnostics(score: WordScore) -> dict[str, object]:
        return {
            "value": round(score.value, LOGGED_SCORE_DECIMALS),
            "known": score.known,
            "frequency": score.frequency,
            "exact": score.exact,
            "spell_known": score.spell_known,
            "ngram_score": round(score.ngram_score, LOGGED_SCORE_DECIMALS),
            "invalid_ratio": round(score.invalid_ratio, LOGGED_SCORE_DECIMALS),
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
            "confidence": round(decision.confidence, LOGGED_SCORE_DECIMALS),
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
            context_model_status=self.context_policy.status,
            field_reader_status=self._field_reader_status(),
            field_reader_details=self._field_reader_details(),
            language_models={
                str(group): {
                    "locale": model.locale,
                    "words": len(model.frequencies),
                    "source": model.source,
                }
                for group, model in self.models.items()
            },
            settings=settings_snapshot(self.settings),
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
        caret_unknown: bool = False,
        protection: dict[str, object] | None = None,
        source_group: int | None = None,
        early_switch_origin: int | None = None,
        idle_ms: int | None = None,
        context: tuple[dict[int, str], int | None] | None = None,
        literal_head: str = "",
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
        elif caret_unknown:
            skipped_reason = "caret_moved"
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
            literal_head="<redacted>" if application_excluded and literal_head else literal_head,
            alternatives=logged_alternatives,
            source_group=source_group,
            application=application,
            enabled=enabled,
            trigger_enabled=trigger_enabled,
            manual_layout_protected=manual_layout_protected,
            caret_unknown=caret_unknown,
            protection=protection,
            application_excluded=application_excluded,
            skipped_reason=skipped_reason,
            minimum_length=self._minimum_word_length(),
            confidence_threshold=self._confidence_threshold(),
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

    def _log_pending_dropped(self, reason: str) -> None:
        """Record a scheduled correction that is thrown away before it runs.

        A correction waits for the release of the key that triggered it, so a
        shortcut or a focus change in between silently cancels it: from the
        outside the hotkey simply did nothing.
        """

        plan = self._pending
        if plan is None:
            return
        excluded = self._application_excluded(plan.application)
        self._technical_event(
            "pending_correction_dropped",
            reason=reason or "unspecified",
            mode=plan.mode,
            original="<redacted>" if excluded else plan.original,
            replacement="<redacted>" if excluded else plan.replacement,
            source_group=plan.source_group,
            target_group=plan.target_group,
            trigger_keycode=self._pending_trigger_keycode,
            application=plan.application,
            application_excluded=excluded,
        )

    def _learning_diagnostics(
        self, source_group: int | None, word: str
    ) -> dict[str, object]:
        """Which of the user's rules decides about this word, if any.

        Without it a log line cannot tell a correction the user's rule forced, or
        a word their rule kept, from one the model decided on its own.
        """

        enabled = bool(self.settings.get("detection.learning", True))
        if source_group is None:
            return {"enabled": enabled}
        rule = self.learning.match(source_group, word) if enabled else None
        return {
            "enabled": enabled,
            "rule": None if rule is None else {
                "pattern": rule.pattern,
                "match": rule.match,
                "case_sensitive": rule.case_sensitive,
                "action": rule.action,
                "target_group": rule.target_group,
            },
            "forced_target": self._forced_target_group(source_group, word),
            "rejected_targets": sorted(self._rejected_targets(source_group, word)),
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

    def _settle_context_wait_after_pause(self, now: float) -> None:
        """The user stopped typing: a wait with no next word coming has to decide.

        A waiting word is a word whose direction the model wanted the next word to
        settle. When the next word arrives, `_resolve_context_wait` decides the pair.
        When it does not, the wait used to lapse in silence and the word stood as
        typed - which is a refusal the user never asked for, and it is how `ша ` came
        to stay itself where the curated table names `if`. The pause is the moment
        that possibility ends, so the word is decided once more with the pause
        trigger, where a curated exception applies and the model still decides
        everything else.
        """

        waiting = self._context_waiting
        last = self._last_word_input_at
        if (waiting is None or self._pending is not None or self._strokes or last is None
                or now - last < self._pause_delay() or self._last_committed_stale
                or waiting.window != (self._focus_window or 0)
                or self.settings.get("detection.context_policy", "assist") != "assist"):
            return
        if not waiting.settles_on_pause:
            self._log_context_wait("context_wait_cancelled", waiting, "suggestion_not_settled")
            self._context_waiting = None
            return
        application = self.backend.active_application()
        if waiting.plan.application != application or self._application_excluded(application):
            self._log_context_wait("context_wait_cancelled", waiting, "preconditions_changed")
            self._context_waiting = None
            return
        result = self.context_policy.decide(
            waiting.decision, waiting.plan.replacement, waiting.plan.target_group, self.detector,
            "pause", str(self.settings.get("detection.context_policy", "assist")),
            field_override=waiting.field,
        )
        self._context_waiting = None
        if not result.decision.should_convert:
            self._log_context_wait("context_wait_cancelled", waiting, "pause_kept_the_word")
            return
        self._log_context_wait("context_wait_settled", waiting, "pause")
        self._pending = replace(waiting.plan, confidence=result.decision.confidence)
        self._learning_prompt_after = None
        self._pending_trigger_keycode = -1

    def _maybe_correct_after_pause(self, *, now: float | None = None) -> None:
        self._settle_context_wait_after_pause(time.monotonic() if now is None else now)
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
        idle_ms = round((current_time - last_input) * MILLISECONDS_PER_SECOND)
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
        if not self._strokes:
            return
        typed = tuple(self._strokes)
        head = self._literal_head(typed, self._source_group)
        strokes, trailing, segmentation_certain = self._completed_word(typed[head:], self._source_group)
        if self._insertion is not None and self._insertion.inside_word:
            # Typed into a word, every key is part of it: `,` between `те` and `е`
            # is the `б` of `тебе`, not punctuation in front of a word.
            head, strokes, trailing, segmentation_certain = 0, typed, (), True
        source_group = self._source_group
        manual_layout_selected = self._word_protected(source_group)
        original = self._text_for_group(strokes, source_group)
        literal_head = self._text_for_group(typed[:head], source_group)
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
                literal_head=literal_head,
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
                literal_head=literal_head,
            )
            return
        if not segmentation_certain:
            return
        inside = None if trailing or head else self._decide_inside_word(
            strokes, source_group, alternatives, application, "pause", None)
        decision = inside if inside is not None else self._decide_word(
            original, alternatives, source_group, application, "pause",
            literal_tail="".join(stroke.character for stroke in trailing),
        )
        excluded = self._application_excluded(application)
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
            literal_head=literal_head,
        )
        if not decision.should_convert:
            if self._mention_head(application):
                self._technical_event(
                    "mention_head_kept", reason="word_kept", shown=False,
                    application=application, application_excluded=excluded,
                )
            return

        self._early_switch_origin = None
        self._early_switch_at = None
        # A word converted at a pause is the neighbour a waiting word was waiting
        # for, exactly as at a space: `tot`, then `ghbdtn` and a pause before the
        # space converted `привет` alone, switched the layout, and the space in the
        # other layout ended the wait with `tot` left standing.
        waiting, self._context_waiting = self._context_waiting, None
        provisional, self._provisional = self._provisional, None
        kept, self._kept = self._kept, ()
        joint = None
        if trailing or head:
            self._log_context_wait("context_wait_cancelled", waiting, "literal_tail" if trailing else "literal_head")
        else:
            joint = self._resolve_context_wait(waiting, strokes, None, decision, application, alternatives)
            if joint is not None and waiting is not None:
                joint = (self._take_along(kept, joint[0], len(waiting.plan.original), application), joint[1])
            if joint is None:
                joint = self._revert_with_next_word(provisional, strokes, None, decision, application)
            if joint is None:
                taken = self._take_along(kept, self._plan_from_decision(strokes, None, application, decision, "pause"),
                                         len(original), application)
                if taken.mode == "context_phrase":
                    joint = (taken, decision)
        if joint is not None:
            plan = joint[0]
        else:
            mention = self._mention_head(application)
            plan = replace(
                self._plan_from_decision(strokes, None, application, decision, "pause"),
                trailing=trailing,
                head=() if mention and self._quotation_closed(mention[0], trailing, None) else mention,
            )
        self._strokes = []
        self._insertion = None
        self._source_group = -1
        self._early_switch_undone = False
        self._reset_pause_correction()
        self._update(current_word="")
        self._execute_correction(plan, None)

    @staticmethod
    def _quotation_closed(
        head: KeyEvent, trailing: tuple[KeyEvent, ...], boundary: KeyEvent | None
    ) -> bool:
        """The same key pressed against the end of the word closes a quotation.

        `"john"` is a quoted name, not a mention: a quote typed with no space
        after the word can only be the closing one, and then the quote in front
        of it was the opening one. The word is still judged on its own, so the
        name inside the quotation is corrected while both quotes stay.
        """

        tail = trailing + ((boundary,) if boundary is not None else ())
        return any(stroke.keycode == head.keycode for stroke in tail)

    def _mention_head(self, application: str) -> tuple[KeyEvent, ...]:
        """The pending symbol in front of the current word that may be a mention.

        The symbol stays out of the analysed word on purpose: the models read
        `ощрт` and `john`, not `"ощрт` and `@john`, and with the quote attached
        they fall below their own threshold. The word alone decides, and this
        head is then replaced together with it (_execute_correction).
        """

        if len(self._symbol_strokes) != 1 or not self._strokes:
            return ()
        stroke = self._symbol_strokes[0]
        return (stroke,) if self._mention_convention(stroke, application) else ()

    def _mention_convention(self, stroke: KeyEvent, application: str) -> bool:
        """Whether this application reads the symbol on this key as the start of a mention."""

        source_group = stroke.group
        target = next((group for group in self.models if group != source_group), None)
        if target is None or source_group not in self.models:
            return False
        typed = self._text_for_group((stroke,), source_group)
        meant = self._text_for_group((stroke,), target)
        return mention_head(
            application, typed, meant, lambda path: bool(self.settings.get(path, True))
        ) is not None

    def _show_mention_head(self, stroke: KeyEvent) -> None:
        """Write a lone quote as ``@`` at once where the application reads it as a mention.

        Telegram opens its member list only after a real ``@``, and that list is
        how a mention is made: in the collected logs every ``@`` in the chat was
        followed by the arrow keys, Enter or a click, never by letters. Waiting
        for a word that never comes left the quote a quote. So the ``@`` appears
        on the key's release, in place, without changing the layout: a message
        continued in Russian after the list stays Russian. It stays only while
        nothing is typed after it; the next printable key writes the quote back
        (_after_mention_head) and the word that follows decides, as before,
        whether the pair was ``@name`` or a quotation. A quote typed after a
        caret move or after the layout was chosen by hand is left alone.
        """

        application = self.backend.active_application()
        if (
            not bool(self.settings.get("enabled", True))
            or self._application_excluded(application)
            or self._caret_unknown()
            or self._word_protected(stroke.group)
            or not self._mention_convention(stroke, application)
            # A quote typed right against text closes a quotation: `привет,"`.
            # Only one at the start of the text or after a space, Enter or Tab
            # can open a mention.
            or (self._character_before_key and not self._character_before_key.isspace())
        ):
            return
        target = next(group for group in self.models if group != stroke.group)
        shown = replace(stroke, group=target, character=stroke.character_for(target))
        # Nothing is retyped in another layout: the "@" goes out as a literal of
        # the layout it belongs to, and the backends restore the current one after it.
        self._pending = CorrectionPlan(
            (), None, stroke.group, stroke.group, stroke.character, shown.character,
            UNSCORED_CORRECTION_CONFIDENCE, application, True, "mention_shown",
            trailing=(shown,),
        )
        self._learning_prompt_after = None
        self._pending_trigger_keycode = stroke.keycode

    def _after_mention_head(self, event: KeyEvent, boundary: KeyEvent | None) -> None:
        """A key typed after the quote: the member list was not what came next."""

        if self._pending is not None and self._pending.mode == "mention_shown":
            # Typed before the quote key came up: nothing was rewritten yet.
            self._log_pending_dropped("typing_continued")
            self._pending = None
            return
        head = self._mention_shown
        if head is None:
            return
        if event.group != head.group:
            # The user switched the layout after the "@" and typed on: a name
            # in the other layout after a real "@" is exactly a mention.
            self._mention_shown = None
            return
        self._hide_mention_head(head, boundary, event.keycode)

    def _hide_mention_head(self, head: KeyEvent, boundary: KeyEvent | None, trigger_keycode: int) -> None:
        if boundary is None and self._pending is not None and self._pending.mode == "mention_hidden":
            # Already waiting for the first letter to come up; it takes the
            # letters typed meanwhile along (_maybe_execute_pending).
            return
        self._log_pending_dropped("mention_head_written_back")
        self._pending = self._mention_write_back(
            head, tuple(self._strokes), (), boundary, self.backend.active_application()
        )
        self._learning_prompt_after = None
        self._pending_trigger_keycode = trigger_keycode

    def _mention_write_back(
        self,
        head: KeyEvent,
        strokes: tuple[KeyEvent, ...],
        trailing: tuple[KeyEvent, ...],
        boundary: KeyEvent | None,
        application: str,
    ) -> CorrectionPlan:
        """The plan that turns the shown ``@`` back into the quote, keeping what follows it."""

        shown_group = next(group for group in self.models if group != head.group)
        return CorrectionPlan(
            (head, *strokes), boundary, shown_group, head.group,
            self._text_for_group((head,), shown_group) + self._text_for_group(strokes, head.group),
            self._text_for_group((head, *strokes), head.group),
            UNSCORED_CORRECTION_CONFIDENCE, application, True, "mention_hidden",
            trailing=trailing,
        )

    def _prune_stale_presses(self, now: float, *, older_than: float = STALE_PRESS_SECONDS,
                             keep: int | None = None) -> None:
        """Forget presses whose release was never delivered (focus changes)."""

        stale = [
            keycode
            for keycode, since in self._pressed_since.items()
            if now - since > older_than and keycode != keep
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

        if self._pending is not None and not self._pending.automatic and not (self._strokes or self._symbol_strokes):
            self._technical_event(
                "manual_conversion_waiting", reason="previous_command_pending",
                pressed_keycodes=sorted(self._pressed),
                modifier_keycodes=sorted(self._modifier_keycodes),
            )
            self._update(last_action="Замена ожидает отпускания клавиш")
            return
        mode = "manual"
        trailing: tuple[KeyEvent, ...] = ()
        if self._strokes:
            strokes = tuple(self._symbol_strokes) + tuple(self._strokes)
            source_group = self._source_group
            boundary = None
            application = self.backend.active_application()
            source = "current_word" if not self._symbol_strokes else "symbols_and_word"
            self._mention_shown = None
            self._early_switch_origin = None
            self._early_switch_at = None
        elif self._symbol_strokes:
            strokes = tuple(self._symbol_strokes)
            source_group = self._symbol_strokes[-1].group
            if self._mention_shown is not None:
                # Pause on the "@" shown for a quote asks for the quote back, and
                # the quote is then kept whatever word follows it.
                source_group = next(group for group in self.models if group != source_group)
                self._mention_shown = None
                self._symbol_strokes = []
            boundary = None
            application = self.backend.active_application()
            source = "symbols"
            mode = "symbols"
        elif self._last_committed is not None and not self._last_committed_stale:
            strokes = self._last_committed.strokes
            source_group = self._last_committed.source_group
            boundary = self._last_committed.boundary
            application = self._last_committed.application
            source = "last_committed"
            trailing = self._last_committed.trailing
        else:
            self._switch_layout_only(trigger_keycode)
            return
        targets = [group for group in self.models if group != source_group]
        if not targets:
            return
        target = targets[0]
        original = self._text_for_group(strokes, source_group)
        replacement = self._text_for_group(strokes, target)
        plan = CorrectionPlan(
            strokes,
            boundary,
            source_group,
            target,
            original,
            replacement,
            UNSCORED_CORRECTION_CONFIDENCE,
            application,
            False,
            mode,
            trailing=trailing,
        )
        # Pause right after a correction converts the same keys back. That
        # teaches nothing: a rule comes only from the rule window, which a
        # double press of the hotkey offers.
        reversal = self._reversal_of_last_correction(plan)
        # A second Pause before the first one ran replaces the plan; without
        # this line the first conversion would vanish without a trace.
        self._log_pending_dropped("replaced_by_manual_conversion")
        self._pending = plan
        self._learning_prompt_after = None
        self._convert_outcome = plan
        self._pending_trigger_keycode = trigger_keycode
        self._manual_release_deadline = time.monotonic() + MANUAL_RELEASE_TIMEOUT_SECONDS
        excluded = self._application_excluded(application)
        self._technical_event(
            "manual_conversion_scheduled",
            wait_id=self._context_waiting.diagnostic_id if self._context_waiting is not None else None,
            source=source,
            original="<redacted>" if excluded else original,
            replacement="<redacted>" if excluded else replacement,
            source_group=source_group,
            target_group=target,
            application=application,
            application_excluded=excluded,
            symbol_count=len(self._symbol_strokes),
            reversal=(
                None
                if reversal is None
                else "automatic" if reversal.automatic else "manual"
            ),
        )
        # Explicit intent supersedes the model's old lookahead even while the
        # manual plan is waiting for release. Retain its ID in the event above,
        # but never let that stale wait block the following word's prefix.
        self._cancel_context_wait("manual_conversion")
        self._strokes = []
        self._insertion = None
        self._symbol_strokes = []
        self._source_group = -1
        self._early_switch_undone = False
        self._last_committed_stale = True
        self._reset_pause_correction()

    @staticmethod
    def _late_text_key(event: KeyEvent) -> bool:
        return (
            event.pressed
            and bool(event.character)
            and event.character.isprintable()
            and event.character != " "
            and event.key_name not in MODIFIER_KEYS
            and event.key_name not in NAVIGATION_KEYS
            and event.key_name != "BackSpace"
            and not (event.control or event.alt or event.super_key)
        )

    def _collect_late_input(self, plan: CorrectionPlan) -> tuple[KeyEvent, ...] | None:
        """Keys typed after the word but before its correction lands.

        Their characters already follow the word on screen, so the backend
        deletes them with the word and types them again in the new layout;
        they come back through the hook as fresh input. Called while the hook
        holds input, so the queue cannot grow in the meantime. Anything that is
        not plain text (Enter, a caret move, a shortcut) makes the outcome
        unpredictable; then nothing is touched and the keys stay queued.
        """

        planned = {id(stroke) for stroke in (*plan.strokes, *plan.trailing)}
        rollover = [stroke for stroke in self._strokes if id(stroke) not in planned]
        queued: list[KeyEvent | _LayoutSelection | None] = []
        while True:
            try:
                queued.append(self._events.get_nowait())
            except queue.Empty:
                break
        late = list(rollover)
        kept: list[KeyEvent | _LayoutSelection | None] = []
        usable = all(self._late_text_key(stroke) for stroke in rollover)
        for item in queued:
            if not isinstance(item, KeyEvent):
                usable = False
                kept.append(item)
            elif item.synthetic:
                kept.append(item)
            elif item.key_name in MODIFIER_KEYS:
                usable = False
                kept.append(item)
            elif not item.pressed or self._matches_hotkey("convert_last", item):
                # A key-up types nothing, and neither does a second press of the
                # conversion hotkey: it is answered once this correction lands.
                kept.append(item)
            elif self._late_text_key(item) and self._safe_text_stroke(item):
                late.append(item)
            else:
                usable = False
        if not usable or not late:
            for item in queued:
                self._events.put_nowait(item)
            return () if usable else None
        for item in kept:
            self._events.put_nowait(item)
        if rollover:
            self._strokes = [
                stroke for stroke in self._strokes if id(stroke) in planned
            ]
        return tuple(late)

    def _reopenable_committed_word(self) -> CorrectionPlan | None:
        """The last word, if one Backspace puts the caret right after it.

        Only a word ended by one plain boundary character qualifies: nothing
        was typed or clicked since (it is not stale), no literal punctuation
        sits between the word and that boundary, and the boundary is not
        Enter or Tab, whose effect on the text one Backspace cannot undo.
        """

        plan = self._last_committed
        if (
            plan is None
            or self._last_committed_stale
            or self._strokes
            or self._symbol_strokes
            or plan.trailing
            or not plan.strokes
            or plan.boundary is None
            or plan.boundary.key_name in ACTION_BOUNDARY_KEYS
            or len(plan.boundary.character) != 1
        ):
            return None
        return plan

    def _remember_correction(self, plan: CorrectionPlan) -> None:
        """Note a correction that landed and the chain of corrections it belongs to.

        Converting the same keys back continues the chain; anything else starts a
        new one. The chain's first plan is what a double press of the hotkey
        reasons about: the engine's own decision, or the user's first conversion.
        """

        if self._correction_origin is None or self._reversal_of_last_correction(plan) is None:
            self._correction_origin = plan
        self._last_correction = plan
        self._last_correction_time = time.monotonic()

    def _chain_origin(self, plan: CorrectionPlan) -> CorrectionPlan:
        """The chain ``plan`` would continue once it runs."""

        if self._correction_origin is not None and self._reversal_of_last_correction(plan) is not None:
            return self._correction_origin
        return plan

    @staticmethod
    def _wanted_group(origin: CorrectionPlan) -> int:
        """The layout the user wants the word in when they object to ``origin``.

        An automatic correction was the engine's idea, so the user wants the keys
        as typed; a manual conversion was theirs, so they want its result.
        """

        return origin.source_group if origin.automatic else origin.target_group

    def _double_convert_press(self, event: KeyEvent) -> bool:
        """Whether this press of the conversion hotkey completes a double press."""

        if (
            event.keycode == self._convert_keycode and not self._convert_released
            and self._convert_press_at
        ):
            # Auto-repeat of the held key: still the first press.
            return False
        now = time.monotonic()
        double = (
            bool(self.settings.get("detection.learning", True))
            and self._convert_released
            and event.keycode == self._convert_keycode
            and self._convert_outcome is not None
            and now - self._convert_press_at <= DOUBLE_CONVERT_PRESS_WINDOW_SECONDS
        )
        self._convert_press_at = 0.0 if double else now
        self._convert_keycode = event.keycode
        self._convert_released = False
        if not double:
            self._convert_outcome = None
        return double

    def _request_rule_after_double_press(self, trigger_keycode: int) -> None:
        """Second press of the hotkey: leave the word as the user wants it and offer a rule.

        The first press already did what a single press does. If that put the
        word where the user wants it - a wrong automatic correction undone, a
        missed one converted - the second press only opens the offer. If the
        first press undid a fix the user had made earlier, the second press puts
        the fix back, exactly as a single press would, and the offer follows.
        """

        outcome = self._take_convert_outcome()
        application = self.backend.active_application()
        if isinstance(outcome, CorrectionPlan) and outcome.mode != "symbols":
            if self._pending is outcome:
                origin = self._chain_origin(outcome)
                offer = self._rule_offer(origin, application)
                if self._wanted_group(origin) == outcome.source_group:
                    # The first press, still waiting for its keys, would undo
                    # the user's own fix: it never runs.
                    self._log_pending_dropped("rule_requested")
                    self._pending = None
                    self._manual_release_deadline = 0.0
                    self._show_learning_prompt(offer)
                else:
                    self._learning_prompt_after = offer
                return
            if self._last_requested_plan is outcome:
                origin = self._correction_origin or outcome
                offer = self._rule_offer(origin, application)
                if self._wanted_group(origin) == outcome.target_group:
                    self._show_learning_prompt(offer)
                    return
                self._schedule_manual_conversion(trigger_keycode)
                scheduled = self._take_convert_outcome()
                if isinstance(scheduled, CorrectionPlan) and self._pending is scheduled:
                    self._learning_prompt_after = offer
                else:
                    self._show_learning_prompt(offer)
                return
        if outcome == "switch":
            # The first press toggled the layout; the second puts it back.
            self._switch_layout_only(trigger_keycode)
            self._take_convert_outcome()
        self._show_learning_prompt(self._empty_rule_offer(application))

    def _take_convert_outcome(self) -> CorrectionPlan | str | None:
        """What the last press of the hotkey did, forgotten once read."""

        outcome, self._convert_outcome = self._convert_outcome, None
        return outcome

    def _rule_offer(self, origin: CorrectionPlan, application: str) -> LearningPrompt:
        if not any(character.isalpha() for character in origin.original):
            return self._empty_rule_offer(application)
        return LearningPrompt(
            origin.source_group, origin.target_group, origin.original, origin.replacement,
            application, "keep" if origin.automatic else "convert",
        )

    def _empty_rule_offer(self, application: str) -> LearningPrompt:
        current = self.snapshot.current_group
        group = current if current in self.models else next(iter(self.models), 0)
        target = next((item for item in self.models if item != group), group)
        return LearningPrompt(group, target, "", "", application)

    def _accepts_injected(self, event: KeyEvent) -> bool:
        """Whether input another program injected is treated as the user's typing.

        A key that carries a character instead of a key position (VK_PACKET on
        Windows) never is: it cannot be typed again in another layout.
        """

        return self._injected_input and event.key_name != UNICODE_PACKET_KEY_NAME

    def _observe_foreign_input(self, event: KeyEvent) -> None:
        """Input another program injected: the text changed in a way not typed here.

        Remote control (TeamViewer, AnyDesk), macros and on-screen keyboards send
        keys of their own. They are not this keyboard's typing - the machine where
        they are typed has its own switcher - so they are neither words nor
        hotkeys nor keys to wait for; only the text they change is no longer known.
        """

        if not event.pressed or event.key_name in MODIFIER_KEYS:
            return
        self._convert_press_at = 0.0
        if not self._foreign_input_active:
            self._foreign_input_active = True
            self._technical_event(
                "foreign_input_observed",
                key_name=event.key_name,
                application=self.backend.active_application(),
                word_length=len(self._strokes),
                action_waiting=self._deferred_action is not None,
            )
        if self._deferred_action is not None:
            # The user's own Enter is waiting for its keys to come up; the
            # injected key is held behind it and follows it. Nothing to undo.
            return
        self._clear_word(reason="foreign_input")
        self._untracked_token = False
        self._last_committed_stale = True
        self._contexts.clear()

    def _reversal_of_last_correction(
        self, plan: CorrectionPlan
    ) -> CorrectionPlan | None:
        """The last correction, if ``plan`` converts the same keys back."""

        previous = self._last_correction
        if (
            previous is None
            or plan.strokes != previous.strokes
            or plan.trailing != previous.trailing
            or plan.source_group != previous.target_group
            or plan.target_group != previous.source_group
        ):
            return None
        return previous

    def _switch_layout_only(self, trigger_keycode: int) -> None:
        """Pause with nothing to convert just toggles the layout."""

        current = self.snapshot.current_group
        target = alternate_layout_group(current)
        if target is None or target not in self.models:
            self._technical_event(
                "manual_conversion_impossible",
                reason="no_alternate_layout",
                current_group=current,
                trigger_keycode=trigger_keycode,
                last_committed_stale=self._last_committed_stale,
            )
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
        self._convert_outcome = "switch"
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
        self._log_pending_dropped("replaced_by_undo")
        self._pending = CorrectionPlan(
            strokes,
            None,
            current_group,
            origin,
            self._text_for_group(strokes, current_group),
            self._text_for_group(strokes, origin),
            UNSCORED_CORRECTION_CONFIDENCE,
            self.backend.active_application(),
            False,
            "early_undo",
        )
        self._learning_prompt_after = None
        self._pending_trigger_keycode = trigger_keycode
        self._manual_release_deadline = time.monotonic() + MANUAL_RELEASE_TIMEOUT_SECONDS
        self._technical_event(
            "early_switch_undo_scheduled",
            source_group=current_group,
            target_group=origin,
            prefix_length=len(strokes),
        )

    def _schedule_undo(self, trigger_keycode: int) -> None:
        if self._pending is not None and self._pending.mode == "early":
            self._log_pending_dropped("replaced_by_undo")
            self._pending = None
            self._learning_prompt_after = None
            self._early_switch_undone = True
            self._update(last_action="Раннее переключение отменено до замены")
            return
        early_origin = self._early_switch_origin
        if early_origin is not None and self._strokes:
            self._schedule_early_switch_undo(early_origin, trigger_keycode)
            return
        previous = self._last_correction
        if (
            previous is None
            or self._last_committed_stale
            or bool(self._strokes and tuple(self._strokes) != previous.strokes)
            or self._symbol_strokes
            or time.monotonic() - getattr(self, "_last_correction_time", 0.0) > UNDO_AVAILABLE_WINDOW_SECONDS
        ):
            self._technical_event("undo_unavailable", reason="text_changed_or_expired")
            self._update(last_action="Последнее исправление уже нельзя отменить")
            return
        self._log_pending_dropped("replaced_by_undo")
        self._pending = CorrectionPlan(
            previous.strokes,
            previous.boundary,
            previous.target_group,
            previous.source_group,
            previous.replacement,
            previous.original,
            UNSCORED_CORRECTION_CONFIDENCE,
            previous.application,
            False,
            "undo",
            trailing=previous.trailing,
        )
        # Undo puts the text back and teaches nothing; a rule that keeps the
        # word comes only from the rule window.
        self._learning_prompt_after = None
        self._pending_trigger_keycode = trigger_keycode
        self._manual_release_deadline = time.monotonic() + MANUAL_RELEASE_TIMEOUT_SECONDS

    def _maybe_execute_pending(self, event: KeyEvent) -> None:
        if self._deferred_action is not None and not self._pressed and not self._modifier_keycodes:
            plan, self._pending = self._pending, None
            self._learning_prompt_after = None
            succeeded = plan is None or self._execute_correction(plan, None)
            if self._deferred_action is not None:
                self._complete_deferred_action(succeeded, "corrected" if plan else "no_correction")
            self._clear_word(reason="action_completed")
            self._contexts.clear()
            self.context_policy.stream.clear()
            return
        if self._pending is None:
            return
        if event.keycode == self._pending_trigger_keycode and not event.pressed:
            self._pending_trigger_keycode = -1
        if self._pending_trigger_keycode != -1 or self._modifier_keycodes or self._pressed:
            return
        plan, self._pending = self._pending, None
        self._manual_release_deadline = 0.0
        learning_prompt, self._learning_prompt_after = self._learning_prompt_after, None
        if plan.mode in ("early", "early_undo", "late_stroke"):
            refreshed = self._refresh_early_plan(plan)
            if refreshed is None:
                self._technical_event(
                    "early_switch_dropped",
                    reason="word_changed_before_release",
                    current_word_length=len(self._strokes),
                )
                return
            plan = refreshed
        if plan.mode == "mention_hidden" and plan.boundary is None and not plan.trailing:
            # Letters typed before the first one came up stand after the "@" too.
            typed = tuple(self._strokes)
            if len(typed) >= len(plan.strokes) and typed[:len(plan.strokes) - 1] == plan.strokes[1:]:
                plan = self._mention_write_back(plan.strokes[0], typed, (), None, plan.application)
        self._execute_correction(plan, learning_prompt)

    def _execute_correction(
        self,
        plan: CorrectionPlan,
        learning_prompt: LearningPrompt | None,
    ) -> bool:
        requested = plan
        if plan.head:
            # The word decided; the mention head is rewritten with it, and the
            # rule the user may confirm still names the word alone.
            plan = replace(
                plan,
                strokes=plan.head + plan.strokes,
                original=self._text_for_group(plan.head, plan.source_group) + plan.original,
                replacement=self._text_for_group(plan.head, plan.target_group) + plan.replacement,
                head=(),
            )
            self._symbol_strokes = []
            self._mention_shown = None
            self._technical_event(
                "mention_head_applied",
                mode=plan.mode,
                application=plan.application,
                application_excluded=self._application_excluded(plan.application),
                source_group=plan.source_group,
                target_group=plan.target_group,
            )
        if self._track_focus().changed:
            self._technical_event("correction_aborted", mode=plan.mode, reason="focus_changed")
            return False
        if self._input_overflow.is_set():
            self._clear_word(reason="input_overflow")
            self._technical_event("correction_aborted", mode=plan.mode, reason="input_overflow")
            return False
        if any(not self._safe_text_stroke(stroke) for stroke in (*plan.strokes, *plan.trailing)):
            self._clear_word(reason="unrepresentable_text")
            self._technical_event(
                "correction_aborted", mode=plan.mode, reason="unrepresentable_text",
                character_lengths=[tuple(map(len, stroke.characters)) for stroke in plan.strokes],
            )
            return False
        if plan.context_field:
            reader = self.context_policy.reader
            field = None if reader is None else reader.read(plan.application, self._focus_window or 0)
            suffix = plan.original + "".join(stroke.character for stroke in plan.trailing) + (plan.boundary.character if plan.boundary else "")
            if (
                field is None or field.field_id != plan.context_field
                or field.application != plan.application or field.sensitive or field.selection
                or not field.before.endswith(suffix)
            ):
                typed_after_boundary = bool(self._strokes)
                self._clear_word(reason="context_field_changed")
                # The letters already typed for the next word stand somewhere
                # the engine can no longer vouch for. The rest of that token is
                # left alone rather than judged on its own: "все" must not turn
                # into "в" + "се" with only the tail converted.
                self._untracked_token = self._untracked_token or typed_after_boundary
                self._technical_event(
                    "correction_aborted", mode=plan.mode, reason="context_field_changed",
                    letters_untracked=typed_after_boundary,
                )
                return False
        application_excluded = self._application_excluded(plan.application)
        logged_original = "<redacted>" if application_excluded else plan.original
        logged_replacement = (
            "<redacted>" if application_excluded else plan.replacement
        )
        previous_group = self.snapshot.current_group
        started = time.monotonic()
        typed_before = self._typed_events
        presses_before = self._typed_presses
        self._correction_sequence += 1
        correction_id = self._correction_sequence
        # From here on the hook keeps the user's keys back; whatever was typed
        # before this moment is collected and typed again after the word.
        late: tuple[KeyEvent, ...] | None = ()
        held = 0
        try:
            try:
                self.backend.hold_input()
                late = self._collect_late_input(plan)
                if late is None:
                    self._technical_event(
                        "correction_aborted", correction_id=correction_id,
                        mode=plan.mode, reason="unsafe_input_after_word",
                    )
                    self._last_committed_stale = True
                    return False
                options = {"trailing": plan.trailing} if plan.trailing else {}
                held = self.backend.inject_correction(
                    plan.strokes,
                    plan.target_group,
                    plan.boundary,
                    plan.source_group,
                    late=late,
                    **options,
                )
            finally:
                held += self.backend.release_input()
        except Exception as error:
            self._technical_event(
                "correction_failed",
                correction_id=correction_id,
                mode=plan.mode,
                original=logged_original,
                replacement=logged_replacement,
                source_group=plan.source_group,
                target_group=plan.target_group,
                application=plan.application,
                application_excluded=application_excluded,
                automatic=plan.automatic,
                keys_during_injection=self._typed_events - typed_before,
                keypresses_during_injection=self._typed_presses - presses_before,
                late_keys=len(late or ()),
                text_verified=False,
                error=str(error),
            )
            self._early_switch_origin = None
            self._early_switch_at = None
            self._clear_word(reason="injection_failed")
            self._update(last_error=str(error), last_action="Ошибка замены · проверьте текст в приложении")
            return False
        self._note_engine_switch(plan.target_group)
        replayed_only_releases = (
            held > 0 and self._typed_events - typed_before == held
            and self._typed_presses == presses_before
        )
        context_reset_reason = "late_input" if late else "held_text_or_unknown" if held and not replayed_only_releases else ""
        if context_reset_reason:
            self.context_policy.stream.clear()
        else:
            self.context_policy.stream.replace_suffix(
                plan.original, plan.replacement,
                # The shown "@" is the replacement itself, not a literal after it.
                "" if plan.mode == "mention_shown" else
                "".join(stroke.character for stroke in plan.trailing) + (plan.boundary.character if plan.boundary else ""),
            )
        self._technical_event(
            "correction_applied",
            correction_id=correction_id,
            text_verified=False,
            mode=plan.mode,
            original=logged_original,
            replacement=logged_replacement,
            source_group=plan.source_group,
            target_group=plan.target_group,
            previous_group=previous_group,
            layout_switched=plan.source_group != plan.target_group,
            deleted_characters=len(plan.strokes) + len(plan.trailing) + (0 if plan.boundary is None else 1) + len(late),
            literal_characters=len(plan.trailing),
            replayed_strokes=len(plan.strokes),
            boundary_replayed=plan.boundary is not None,
            injection_ms=round((time.monotonic() - started) * MILLISECONDS_PER_SECOND),
            # Legacy field counts presses AND releases, not typed characters.
            keys_during_injection=self._typed_events - typed_before,
            keypresses_during_injection=self._typed_presses - presses_before,
            queued_events=self._events.qsize(),
            late_keys=len(late),
            held_keys=held,
            context_reset_reason=context_reset_reason,
            replayed_only_releases=replayed_only_releases,
            application=plan.application,
            application_excluded=application_excluded,
            automatic=plan.automatic,
            confidence=round(plan.confidence, LOGGED_SCORE_DECIMALS),
            boundary=(None if plan.boundary is None else plan.boundary.key_name),
        )
        if plan.mode in {"mention_shown", "mention_hidden"}:
            # Only the head changed; the word being typed goes on as it was.
            self._mention_shown = self._symbol_strokes[0] if plan.mode == "mention_shown" else None
            self._update(current_group=plan.target_group, last_error="")
            return True
        if plan.mode in {"early", "late_stroke"}:
            # The prefix is finished later; only then does it become a
            # correction that can be undone or listed in the history.
            if plan.mode == "early":
                self._early_switch_origin = plan.source_group
                self._early_switch_at = time.monotonic()
                self._early_switch_confidence = plan.confidence
            self._source_group = plan.target_group
            self._update(
                current_group=plan.target_group,
                current_word=self._text_for_group(self._strokes, plan.target_group),
                last_error="",
            )
            return True
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
            return True
        self._remember_correction(plan)
        self._last_requested_plan = requested
        if requested is self._convert_outcome and self._convert_press_at:
            # A second press of the hotkey made while this conversion was being
            # typed waited behind it in the hook; slow typing into a slow program
            # must not turn a double press into two single ones, so the window
            # for the second press starts once the word is on screen.
            self._convert_press_at = time.monotonic()
        if plan.mode == "symbols":
            self._update(
                current_group=plan.target_group,
                last_action=f"{plan.original} → {plan.replacement}",
                last_error="",
            )
            for callback in tuple(self._correction_callbacks):
                callback(plan)
            return True
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
            trailing=plan.trailing,
        )
        self._last_committed_stale = bool(context_reset_reason)
        self._provisional = (
            ProvisionalWord(plan, self._focus_window or 0, time.monotonic() + CONTEXT_TTL)
            if plan.revisit is not None and plan.boundary is not None and not context_reset_reason else None
        )
        if (plan.boundary is None and not plan.trailing and not plan.sign_replayed and not context_reset_reason
                and any(char.isalpha() for char in plan.replacement)):
            # Idle/manual correction did not end the word. Keep its physical
            # prefix so continued typing and Backspace still refer to the
            # whole token instead of a detached suffix.
            self._strokes = list(plan.strokes)
            self._source_group = plan.target_group
            self._update(current_word=plan.replacement)
            if not plan.automatic:
                self._manual_layout_group = plan.target_group
        self._remember_context(plan.application, plan.target_group, plan.strokes)
        count = self.snapshot.correction_count + (1 if plan.automatic else 0)
        action = f"{plan.original} → {plan.replacement}"
        self._update(
            current_group=plan.target_group,
            correction_count=count,
            last_action=action,
            last_error="",
        )
        if plan.automatic:
            self._record_history(plan.original, plan.replacement, plan.application, plan.confidence)
        for callback in tuple(self._correction_callbacks):
            callback(plan)
        if learning_prompt is not None:
            self._show_learning_prompt(learning_prompt)
        return True

    def _complete_deferred_action(self, deliver: bool, reason: str) -> None:
        action, self._deferred_action = self._deferred_action, None
        self._action_deadline = 0.0
        try:
            self.backend.complete_action(deliver)
            self._technical_event(
                "action_delivered" if deliver else "action_cancelled",
                key_name=action.key_name if action else "unknown", reason=reason,
                text_verified=False,
            )
            if not deliver:
                self._update(last_action="Enter/Tab не передан · проверьте текст и нажмите ещё раз")
        except Exception as error:
            self._technical_event("action_failed", reason=reason, error=str(error), text_verified=False)
            self._update(last_error=str(error), last_action="Ошибка Enter/Tab · проверьте приложение")

    def _expire_deferred_action(self) -> None:
        """Enter waited for every key to come up. Decide what a key that never did means.

        A press whose release was lost holds the action hostage: the engine forgets such
        a press only after three seconds, while the action gives up after two, so an
        Enter waiting behind an unrelated stuck key was dropped a second before the
        engine would have freed it - and a dropped Enter is a keystroke that vanishes
        with nothing on screen to explain it. A key already held when the Enter arrived
        and still held at the deadline is not coming up; it is forgotten and the action
        goes through. The action's own key is never forgotten that way: a release nobody
        saw is a press nobody finished, and that case keeps the cautious answer.
        """

        action = self._deferred_action
        if action is None or time.monotonic() < self._action_deadline:
            return
        now = time.monotonic()
        self._prune_stale_presses(now, older_than=now - (self._action_deadline - ACTION_TIMEOUT_SECONDS),
                                  keep=action.keycode)
        if self._pressed or self._modifier_keycodes:
            # Either the Enter itself was never seen to come up, or a key pressed after
            # it is still down. Both keep the cautious answer.
            self._clear_word(reason="action_release_timeout")
            return
        self._maybe_execute_pending(action)

    def _expire_manual_correction(self) -> None:
        if (
            self._pending is not None and not self._pending.automatic
            and self._manual_release_deadline > 0.0
            and time.monotonic() >= self._manual_release_deadline
        ):
            self._technical_event(
                "manual_conversion_timeout", reason="key_release_not_observed",
                pressed_keycodes=sorted(self._pressed),
                modifier_keycodes=sorted(self._modifier_keycodes),
            )
            # A timeout is not proof that a physical key is up. Do not inject,
            # switch layout or turn this uncertain attempt into learning.
            self._clear_word(
                "Замена отменена: не получено отпускание клавиш · проверьте текст",
                reason="manual_release_timeout",
            )

    def _field_reader_status(self) -> str:
        reader = self.context_policy.reader
        if reader is None:
            return "not_configured"
        return reader.status if isinstance(reader, PlatformFieldReader) else "custom_reader"

    def _field_reader_details(self) -> dict[str, object]:
        reader = self.context_policy.reader
        return {**reader.diagnostics(), "retry": reader.retry_diagnostics()} if isinstance(reader, PlatformFieldReader) else {"status": self._field_reader_status()}

    def _configured_action_keys(self) -> frozenset[str]:
        if not bool(self.settings.get("enabled", True)):
            return frozenset()
        keys: set[str] = set()
        if bool(self.settings.get("detection.correct_on_enter", True)):
            keys.update(("Return", "KP_Enter"))
        if bool(self.settings.get("detection.correct_on_tab", True)):
            keys.add("Tab")
        return frozenset(keys)

    def _show_learning_prompt(self, prompt: LearningPrompt) -> None:
        with self._lock:
            self._learning_prompt = prompt
            self._learning_prompt_deadline = (
                time.monotonic() + LEARNING_PROMPT_TIMEOUT_SECONDS
            )
            self._prompt_key_deadline = self._learning_prompt_deadline
            callbacks = tuple(self._learning_prompt_callbacks)
        excluded = self._application_excluded(prompt.application)
        self._technical_event(
            "learning_prompt_shown",
            original="<redacted>" if excluded else prompt.original,
            replacement="<redacted>" if excluded else prompt.replacement,
            source_group=prompt.source_group,
            target_group=prompt.target_group,
            application=prompt.application,
            action=prompt.action,
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

    def consumes_key(self, event: KeyEvent) -> KeyDisposition:
        """Whether KeySwitch answers this key itself and the window must not.

        Called from the keyboard hook while the system waits for the answer,
        so it only reads plain attributes: taking the engine lock here could
        hold the hook past the low-level timeout, and Windows then removes the
        hook without telling anyone.
        """

        if not event.pressed or event.synthetic:
            return False
        if self._input_suspended or (event.foreign and not self._accepts_injected(event)):
            return False
        if self._sensitive_context_window is not None:
            return False
        if event.key_name in MODIFIER_KEYS:
            return False
        if event.control or event.alt or event.super_key or event.shift:
            self._prompt_key_deadline = 0.0
            return False
        if time.monotonic() < self._prompt_key_deadline and event.key_name in PROMPT_KEYS:
            return True
        self._prompt_key_deadline = 0.0
        return "defer" if event.key_name in self._action_keys else False

    def _matches_hotkey(self, name: str, event: KeyEvent) -> bool:
        return Hotkey(str(self.settings.get(f"hotkeys.{name}", ""))).matches(event)

    def _is_boundary(self, event: KeyEvent) -> bool:
        return (
            event.key_name in WORD_BOUNDARY_KEYS | ACTION_BOUNDARY_KEYS
            or bool(event.character and event.character.isspace())
            or event.character in PUNCTUATION
            or bool(
                event.character and event.character not in {"_", "/", "\\", "@"}
                and unicodedata.category(event.character[0]).startswith("P")
            )
        )

    def _boundary_enabled(self, event: KeyEvent) -> bool:
        if event.key_name == "space" or event.character.isspace() and event.key_name not in ACTION_BOUNDARY_KEYS:
            return bool(self.settings.get("detection.correct_on_space", True))
        if event.key_name in {"Return", "KP_Enter"}:
            return bool(self.settings.get("detection.correct_on_enter", True))
        if event.key_name in {"Tab", "ISO_Left_Tab"}:
            return bool(self.settings.get("detection.correct_on_tab", True))
        return bool(self.settings.get("detection.correct_on_punctuation", True))

    @staticmethod
    def _trigger_for_boundary(event: KeyEvent) -> CorrectionTrigger:
        if event.key_name == "space" or event.character.isspace() and event.key_name not in ACTION_BOUNDARY_KEYS:
            return "space"
        if event.key_name in {"Return", "KP_Enter"}:
            return "enter"
        if event.key_name in {"Tab", "ISO_Left_Tab"}:
            return "tab"
        return "punctuation"

    def _application_excluded(self, application: str) -> bool:
        if self._sensitive_context_window is not None and self._sensitive_context_window == self._focus_window:
            return True
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
        self.context_policy.stream.clear()
        # After a click, a caret move or another window nothing is known to
        # stand before the next key.
        self._last_key_character = ""
        self._position_unknown = True
        self._insertion = None
        self._cancel_context_wait(reason or "word_cleared")
        if self._deferred_action is not None:
            self._complete_deferred_action(False, reason)
        self._log_word_discarded(reason)
        self._log_pending_dropped(reason)
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
        # A shown "@" followed by the arrow keys, a click or Enter was the member
        # list at work: it stays on screen and is no longer tracked.
        self._mention_shown = None
        self._source_group = -1
        self._early_switch_origin = None
        self._early_switch_at = None
        self._reset_pause_correction()
        self._pending = None
        self._learning_prompt_after = None
        self._manual_release_deadline = 0.0
        self._last_committed_stale = True
        if action is None:
            self._update(current_word="")
        else:
            self._update(current_word="", last_action=action)

    def _settings_changed(self, path: str, value: object) -> None:
        if path in {"*", "enabled", "detection.context_aware", "detection.context_policy", "detection.context_read_field", "exclusions.applications"}:
            self.context_policy.stream.clear()
            self._cancel_context_wait("settings_changed")
            self._sensitive_context_window = None
        self._action_keys = self._configured_action_keys()
        self._injected_input = bool(self.settings.get("detection.injected_input", False))
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
        change = setting_change(self.settings, path, value)
        if change is not None:
            self._technical_event("setting_changed", **change)
        if path == "diagnostics.technical_logging" and bool(value):
            self._technical_session_event("technical_logging_enabled")

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

        self._own_layout_ignored = False
        self._sensitive_context_window = None
        self._convert_press_at = 0.0
        dropped = len(self._strokes)
        self._clear_word(reason="focus_changed")
        self._untracked_token = False
        self._contexts.clear()
        self.context_policy.stream.clear()
        self._manual_layout_group = None
        self._last_committed_stale = True
        # A key held while the window changed reports its release to the window that
        # took over, and often to nobody the hook can see. Such a press then sits in
        # the engine's books until the stale timer forgets it three seconds later -
        # long enough for a withheld Enter, which gives up after two, to be dropped
        # while waiting for a key that will never come up. The moment the focus moves
        # is when that press stops meaning anything, so it is forgotten here rather
        # than by a timer. A key that really is still down loses nothing: its release,
        # if it ever arrives, simply finds nothing to clear.
        held = sorted(self._pressed | self._modifier_keycodes)
        self._pressed.clear()
        self._modifier_keycodes.clear()
        self._pressed_since.clear()
        self._technical_event(
            "focus_changed",
            previous_window=previous,
            window=window,
            dropped_word_length=dropped,
            released_keycodes=held,
        )

    def _observe_group(self, group: int, *, source: str = "poll") -> None:
        focus = self._track_focus()
        if not focus.ignore_layout:
            # Any look outside an own window ends the ignored episode, so the
            # next visit to the settings window is logged once again.
            self._own_layout_ignored = False
        current_group = self.snapshot.current_group
        if not 0 <= group < len(self.models) or group == current_group:
            return
        if focus.ignore_layout:
            # Windows keeps a layout per window: the settings window or the
            # learning prompt of KeySwitch itself says nothing about the
            # layout the user types in, so it neither protects nor updates.
            # Every poll repeats the observation; the log needs it once.
            if not self._own_layout_ignored:
                self._own_layout_ignored = True
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
            else round((time.monotonic() - switched_at) * MILLISECONDS_PER_SECOND)
        )
        # Only a change *to* the layout the engine itself just selected is the
        # engine's own switch; the user switching away right after a wrong
        # correction is manual and must protect the retyped word.
        initiated_by_engine = (
            engine_switch_ms is not None
            and engine_switch_ms <= ENGINE_SWITCH_GRACE_SECONDS * MILLISECONDS_PER_SECOND
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
            wait_id=self._context_waiting.diagnostic_id if self._context_waiting is not None else None,
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
        context_action: str | None = None,
        context_model: str | None = None,
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
                context_action=current.context_action if context_action is None else context_action,
                context_model=current.context_model if context_model is None else context_model,
            )
            callbacks = tuple(self._callbacks)
            snapshot = self._snapshot
        for callback in callbacks:
            callback(snapshot)

"""Observe global macOS keyboard events and inject physical corrections.

The module holds the whole decision half of the macOS backend and reaches the
system only through :class:`MacAPI`, so the behaviour below is exercised on any
platform with a substitute. ``macos_native`` supplies the real implementation.

Two things differ from the other platforms and shape what follows.

A Quartz event tap placed as an active filter may delete the event it is given,
which Windows cannot do: there a key already on its way to the window can only
be raced, never withheld. Here a withheld key is genuinely withheld, and the
backend puts it back itself by posting it again.

The system switches off a tap whose callback answers too slowly and says so with
an event of its own. A backend that ignores that message keeps a dead tap and
stops seeing the keyboard without any error, so the message is handled like any
other event and the tap is switched back on.
"""

from __future__ import annotations

import threading
import time
from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass, replace
from typing import Protocol

from .backend import BackendProbe, FocusInfo, KeyDisposition, KeyEvent, ScreenAnchor
from .constants.keyboard import (
    ALT_MASK,
    COMPLETED_ACTION_EVENT_COUNT,
    CONTROL_MASK,
    LOCK_MASK,
    POINTER_EVENT_GROUP,
    SHIFT_MASK,
    SUPER_MASK,
)
from .constants.timing import (
    KEYBOARD_LISTENER_START_TIMEOUT_SECONDS,
    KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS,
    LAYOUT_SWITCH_POLL_SECONDS,
    LAYOUT_SWITCH_TIMEOUT_SECONDS,
)
from .constants.macos import (
    EVENT_TAP_DISABLED_TYPES,
    MAC_ALT_KEYS,
    MAC_CONTROL_KEYS,
    MAC_SCRIPT_PROBE_KEYCODE,
    MAC_SHIFT_KEYS,
    MAC_SUPER_KEYS,
    MAC_VK_BACKSPACE,
    MAC_VK_CAPS_LOCK,
    MAC_VK_COMMAND,
    MAC_VK_CONTROL,
    MAC_VK_DOWN_ARROW,
    MAC_VK_END,
    MAC_VK_ESCAPE,
    MAC_VK_FORWARD_DELETE,
    MAC_VK_FUNCTION,
    MAC_VK_HELP,
    MAC_VK_HOME,
    MAC_VK_KEYPAD_ENTER,
    MAC_VK_LEFT_ARROW,
    MAC_VK_OPTION,
    MAC_VK_PAGE_DOWN,
    MAC_VK_PAGE_UP,
    MAC_VK_RETURN,
    MAC_VK_RIGHT_ARROW,
    MAC_VK_RIGHT_COMMAND,
    MAC_VK_RIGHT_CONTROL,
    MAC_VK_RIGHT_OPTION,
    MAC_VK_RIGHT_SHIFT,
    MAC_VK_SHIFT,
    MAC_VK_SPACE,
    MAC_VK_TAB,
    MAC_VK_UP_ARROW,
)
from .constants.text import CYRILLIC_CODEPOINT_RANGE


# The engine speaks X11 key names on every platform; the Windows backend
# translates into the same vocabulary.
KEY_NAMES = {
    MAC_VK_RETURN: "Return",
    MAC_VK_KEYPAD_ENTER: "KP_Enter",
    MAC_VK_TAB: "Tab",
    MAC_VK_SPACE: "space",
    MAC_VK_BACKSPACE: "BackSpace",
    MAC_VK_FORWARD_DELETE: "Delete",
    MAC_VK_ESCAPE: "Escape",
    MAC_VK_CAPS_LOCK: "Caps_Lock",
    MAC_VK_SHIFT: "Shift_L",
    MAC_VK_RIGHT_SHIFT: "Shift_R",
    MAC_VK_CONTROL: "Control_L",
    MAC_VK_RIGHT_CONTROL: "Control_R",
    MAC_VK_OPTION: "Alt_L",
    MAC_VK_RIGHT_OPTION: "Alt_R",
    MAC_VK_COMMAND: "Super_L",
    MAC_VK_RIGHT_COMMAND: "Super_R",
    MAC_VK_FUNCTION: "Function",
    MAC_VK_HELP: "Insert",
    MAC_VK_HOME: "Home",
    MAC_VK_END: "End",
    MAC_VK_PAGE_UP: "Page_Up",
    MAC_VK_PAGE_DOWN: "Page_Down",
    MAC_VK_LEFT_ARROW: "Left",
    MAC_VK_RIGHT_ARROW: "Right",
    MAC_VK_UP_ARROW: "Up",
    MAC_VK_DOWN_ARROW: "Down",
}


# A pointer event carries no key; the engine recognises it by this name.
POINTER_KEY_NAME = "Pointer"


# How long a layout change may take to become visible. Selecting an input
# source is asynchronous: the call returns before the window server has told
# the focused application, and typing into the old layout in between is exactly
# the error the correction exists to undo.

PERMISSION_MISSING_MESSAGE = (
    "Нет разрешения на перехват клавиатуры: включите KeySwitch в разделе "
    "«Конфиденциальность и безопасность» → «Универсальный доступ»"
)

SESSION_TYPE = "quartz"
DISPLAY_NAME = "macOS window server"
TAP_NAME = "CGEventTap"
POST_NAME = "CGEventPost"


class MacBackendError(RuntimeError):
    """The macOS backend cannot do what was asked."""


@dataclass(frozen=True)
class NativeKeyEvent:
    """One keyboard event as the tap saw it.

    ``event_type`` carries the tap's own messages, such as the one announcing
    that the system switched the tap off; ``keycode`` is meaningless then.
    """

    pressed: bool
    keycode: int
    timestamp: int
    injected: bool = False
    replayed: bool = False
    event_type: int = 0
    pointer: bool = False


@dataclass(frozen=True)
class NativeInput:
    pressed: bool
    keycode: int = 0
    replayed: bool = False


class MacAPI(Protocol):
    """Everything the backend needs from macOS, and nothing else."""

    def input_sources(self) -> tuple[str, ...]: ...

    def current_input_source(self) -> str: ...

    def select_input_source(self, identifier: str) -> bool: ...

    def translate_key(self, keycode: int, state: int, source: str) -> str: ...

    def post_inputs(self, inputs: tuple[NativeInput, ...]) -> int: ...

    def active_application(self) -> str: ...

    def focused_window(self) -> int: ...

    def window_process_id(self, window: int) -> int: ...

    def current_process_id(self) -> int: ...

    def input_anchor(self) -> ScreenAnchor | None: ...

    def caps_lock_enabled(self) -> bool: ...

    def accessibility_trusted(self, *, prompt: bool = False) -> bool: ...

    def open_accessibility_settings(self) -> bool: ...

    def layout_follows_window(self) -> bool: ...

    def run_event_tap(
        self,
        listener: Callable[[NativeKeyEvent], bool],
        ready: Callable[[], None],
    ) -> None: ...

    def enable_event_tap(self) -> None: ...

    def stop_event_tap(self) -> None: ...


def is_cyrillic(text: str) -> bool:
    return bool(text) and CYRILLIC_CODEPOINT_RANGE[0] <= ord(text[0]) <= CYRILLIC_CODEPOINT_RANGE[1]


def select_source_pair(
    sources: Iterable[str], translate: Callable[[int, int, str], str]
) -> tuple[str, str]:
    """Name the English and the Russian layout, in the engine's group order.

    The choice is made from what a key actually produces rather than from the
    identifier, because a layout's name is not a promise about its script:
    ``ABC``, ``US`` and ``British`` are all English, and a Russian layout may be
    ``Russian``, ``RussianWin`` or a third-party one.
    """

    unique = tuple(dict.fromkeys(sources))
    english = ""
    russian = ""
    for source in unique:
        character = translate(MAC_SCRIPT_PROBE_KEYCODE, 0, source)
        if not english and character.isascii() and character.isalpha():
            english = source
        elif not russian and is_cyrillic(character):
            russian = source
    if not english or not russian:
        raise MacBackendError(
            "В macOS должны быть включены английская и русская раскладки"
        )
    return english, russian


def key_name(keycode: int, character: str) -> str:
    """Name a key by its position, falling back to the letter it types.

    macOS has no per-key names, so a letter key is named after the character it
    produces in the English layout, which is how the engine's own key names for
    letters and digits are spelled.
    """

    if keycode in KEY_NAMES:
        return KEY_NAMES[keycode]
    if len(character) == 1 and character.isascii() and character.isalnum():
        return character.casefold()
    return f"VK_{keycode:02X}"


class MacBackend:
    """Observe global Quartz keyboard events and inject physical corrections."""

    def __init__(self, api: MacAPI | None = None) -> None:
        if api is None:
            from .macos_native import CtypesMacAPI

            api = CtypesMacAPI()
        self._api = api
        self._sources: tuple[str, str] | None = None
        self._listener: Callable[[KeyEvent], None] | None = None
        self._thread: threading.Thread | None = None
        self._running = threading.Event()
        self._ready = threading.Event()
        self._start_error: Exception | None = None
        self._pressed: set[int] = set()
        self._key_filter: Callable[[KeyEvent], KeyDisposition] | None = None
        self._consumed_keys: set[int] = set()
        # While a correction is being injected the user's own keys are kept
        # back here and posted again afterwards, so they can neither land
        # between the backspaces and the replacement nor be lost.
        self._hold_lock = threading.Lock()
        self._holding = False
        self._held: list[NativeKeyEvent] = []
        self._pointer_epoch = 0
        self._hold_pointer_epoch = 0
        self._hold_window = 0
        self._deferred_action: NativeKeyEvent | None = None
        self._action_prior_keys: set[int] = set()
        self._inject_lock = threading.Lock()
        self._tap_revivals = 0

    @property
    def running(self) -> bool:
        return self._running.is_set()

    @property
    def tap_revivals(self) -> int:
        """How often the system switched the tap off and it was revived."""

        return self._tap_revivals

    @property
    def sources(self) -> tuple[str, str]:
        if self._sources is None:
            self._sources = select_source_pair(
                self._api.input_sources(), self._api.translate_key
            )
        return self._sources

    def probe(self) -> BackendProbe:
        try:
            if not self._api.accessibility_trusted():
                return BackendProbe(
                    False, SESSION_TYPE, DISPLAY_NAME, TAP_NAME, POST_NAME, "—", -1,
                    PERMISSION_MISSING_MESSAGE,
                )
            sources = self.sources
            group = self._group_for_source(self._api.current_input_source())
            return BackendProbe(
                True, SESSION_TYPE, DISPLAY_NAME, TAP_NAME, POST_NAME,
                ",".join(sources), group,
            )
        except Exception as error:
            return BackendProbe(
                False, SESSION_TYPE, DISPLAY_NAME, "—", "—", "—", -1, str(error)
            )

    def start(self, listener: Callable[[KeyEvent], None]) -> None:
        if self._running.is_set():
            return
        self.sources
        self._listener = listener
        self._start_error = None
        self._ready.clear()
        self._running.set()
        self._thread = threading.Thread(target=self._tap_loop, name="keyswitch-tap", daemon=True)
        self._thread.start()
        self._ready.wait(timeout=KEYBOARD_LISTENER_START_TIMEOUT_SECONDS)
        error = self._startup_error()
        if error is not None:
            self._running.clear()
            raise error

    def _startup_error(self) -> Exception | None:
        """Read the failure through a call, so its assignment cannot be assumed."""

        return self._start_error

    def _tap_loop(self) -> None:
        try:
            self._api.run_event_tap(self._handle_native, self._mark_ready)
        except Exception as error:
            self._start_error = error
        finally:
            self._running.clear()
            self._ready.set()

    def _mark_ready(self) -> None:
        self._ready.set()

    def stop(self) -> None:
        if not self._running.is_set():
            return
        self._api.stop_event_tap()
        thread = self._thread
        if thread is not None and thread is not threading.current_thread():
            thread.join(timeout=KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS)
        self._running.clear()
        self._thread = None

    def close(self) -> None:
        self.stop()

    def current_group(self) -> int:
        return self._group_for_source(self._api.current_input_source())

    def _group_for_source(self, source: str) -> int:
        sources = self.sources
        return sources.index(source) if source in sources else 0

    def switch_group(self, group: int) -> None:
        sources = self.sources
        if not 0 <= group < len(sources):
            raise MacBackendError(f"Неизвестная группа раскладки: {group}")
        if not self._api.select_input_source(sources[group]):
            raise MacBackendError("macOS отказалась переключить раскладку")

    def active_application(self) -> str:
        return self._api.active_application()

    def permission_granted(self) -> bool:
        """Whether macOS currently lets the program watch the keyboard."""

        return self._api.accessibility_trusted()

    def request_permission(self) -> bool:
        """Ask for the permission and show where it is given.

        macOS grants nothing on request: it shows its own dialog, and the pane
        is opened as well because that dialog appears only once per program.
        """

        granted = self._api.accessibility_trusted(prompt=True)
        if not granted:
            self._api.open_accessibility_settings()
        return granted

    def focused_window(self) -> FocusInfo | None:
        window = self._api.focused_window()
        if not window:
            return None
        own = self._api.window_process_id(window) == self._api.current_process_id()
        return FocusInfo(window, own, self._api.layout_follows_window())

    def input_anchor(self) -> ScreenAnchor | None:
        return self._api.input_anchor()

    def restore_window(self, window: int | None) -> bool:
        """Put the focus back where it was before a prompt appeared.

        Windows has to be told to do this. macOS gives the focus back to the
        program the prompt covered on its own, because KeySwitch runs without a
        place in the dock and never becomes the active program, so there is
        nothing here to undo.
        """

        return False

    def keep_window_inactive(self, window: int) -> bool:
        """Show a window without taking the focus from what is being typed.

        Nothing is needed for the same reason: a program with no dock icon does
        not take the focus when one of its windows appears.
        """

        return False

    def set_key_filter(
        self, predicate: Callable[[KeyEvent], KeyDisposition] | None
    ) -> None:
        """Decide, inside the tap, which keys must not reach the window."""

        self._key_filter = predicate

    def _consumes(self, event: KeyEvent) -> KeyDisposition:
        """Ask the filter about a press, and hide the matching release.

        A window that saw no key-down must not receive the key-up either, so
        the release of a swallowed key is swallowed with it.
        """

        if event.synthetic:
            return False
        if not event.pressed:
            if event.keycode not in self._consumed_keys:
                return False
            self._consumed_keys.discard(event.keycode)
            return True
        if event.keycode in self._consumed_keys:
            return True
        predicate = self._key_filter
        decision = predicate(event) if predicate is not None else False
        if not decision:
            return False
        self._consumed_keys.add(event.keycode)
        return decision

    def hold_input(self) -> None:
        """Keep the user's keys back until the next correction has landed."""

        with self._hold_lock:
            if not self._holding:
                self._hold_window = self._api.focused_window()
                self._hold_pointer_epoch = self._pointer_epoch
            self._holding = True

    def release_input(self) -> int:
        """Post held events again before letting fresh physical input through.

        A reposted event carries its own mark: it reaches the engine but must
        never enter this buffer again. The lock is never held across a post,
        which calls the tap back on another thread.
        """

        active = self._deferred_action
        if active is not None:
            return 0
        count = 0
        try:
            while True:
                with self._hold_lock:
                    if self._deferred_action is not None:
                        # A reposted second Enter starts the next transaction.
                        # Let its preceding key-ups reach the worker, retaining
                        # all following text until that action is completed.
                        index = next((
                            index for index, item in enumerate(self._held)
                            if not item.pressed and item.keycode in self._action_prior_keys
                        ), None)
                        if index is None:
                            return count
                        item = self._held.pop(index)
                    elif self._held:
                        item = self._held.pop(0)
                    else:
                        self._holding = False
                        return count
                self._post_exact((NativeInput(item.pressed, item.keycode, replayed=True),))
                count += 1
        finally:
            # A failed post must never leave the keyboard captured.
            with self._hold_lock:
                if self._deferred_action is None:
                    self._holding = False

    def complete_action(self, deliver: bool) -> int:
        """Deliver a withheld Enter/Tab once, before releasing subsequent input."""

        action = self._deferred_action
        if action is None:
            return 0
        try:
            if deliver:
                if (
                    self._pointer_epoch != self._hold_pointer_epoch
                    or self._api.focused_window() != self._hold_window
                ):
                    raise MacBackendError("Enter/Tab не передан: место ввода изменилось")
                self._post_exact(tuple(
                    NativeInput(pressed, action.keycode) for pressed in (True, False)
                ))
        finally:
            self._deferred_action = None
            self._action_prior_keys.clear()
            self.release_input()
        return COMPLETED_ACTION_EVENT_COUNT if deliver else 0

    def _handle_native(self, native: NativeKeyEvent) -> bool:
        """Answer the tap: True swallows the event, False lets it through."""

        if native.event_type in EVENT_TAP_DISABLED_TYPES:
            # The system switches off a tap it considers unresponsive. Reviving
            # it here is the difference between a brief stall and a program
            # that has silently stopped seeing the keyboard.
            self._tap_revivals += 1
            self._api.enable_event_tap()
            return False
        if native.pointer:
            self._pointer_epoch += 1
            listener = self._listener
            if listener is not None:
                listener(KeyEvent(
                    True, 0, POINTER_KEY_NAME, "", ("", ""), POINTER_EVENT_GROUP, 0, native.timestamp
                ))
            return False
        if not native.injected and not native.replayed:
            with self._hold_lock:
                prior_release = not native.pressed and native.keycode in self._action_prior_keys
                action_key = self._deferred_action is not None and native.keycode in self._consumed_keys
                if self._holding and not prior_release and not action_key:
                    self._held.append(native)
                    return True
        if native.pressed:
            self._pressed.add(native.keycode)
        else:
            self._pressed.discard(native.keycode)
            with self._hold_lock:
                if native.keycode in self._action_prior_keys and any(
                    item.pressed and item.keycode == native.keycode for item in self._held
                ):
                    # An auto-repeat after Enter belongs to the held next
                    # input. Its key-up also has to follow that repost.
                    self._held.append(replace(native, replayed=False))
            self._action_prior_keys.discard(native.keycode)
        state = self._normalized_state()
        characters = tuple(
            self._api.translate_key(native.keycode, state, source) for source in self.sources
        )
        group = self.current_group()
        character = characters[group] if 0 <= group < len(characters) else ""
        event = KeyEvent(
            native.pressed,
            native.keycode,
            key_name(native.keycode, characters[0] if characters else ""),
            character,
            characters,
            group,
            state,
            native.timestamp,
            native.injected,
        )
        repeated_answer = not event.synthetic and event.pressed and event.keycode in self._consumed_keys
        consumed = self._consumes(event)
        if consumed == "defer":
            self.hold_input()
            self._deferred_action = native
            self._action_prior_keys = set(self._pressed)
            event = replace(event, deferred=True)
        listener = self._listener
        if listener is not None and not repeated_answer:
            listener(event)
        return bool(consumed)

    def _normalized_state(self) -> int:
        state = LOCK_MASK if self._api.caps_lock_enabled() else 0
        if self._pressed & MAC_SHIFT_KEYS:
            state |= SHIFT_MASK
        if self._pressed & MAC_CONTROL_KEYS:
            state |= CONTROL_MASK
        if self._pressed & MAC_ALT_KEYS:
            state |= ALT_MASK
        if self._pressed & MAC_SUPER_KEYS:
            state |= SUPER_MASK
        return state

    def inject_correction(
        self,
        strokes: Iterable[KeyEvent],
        target_group: int,
        boundary: KeyEvent | None,
        source_group: int | None = None,
        late: Sequence[KeyEvent] = (),
        trailing: Sequence[KeyEvent] = (),
    ) -> int:
        """Replace the word and return how many held keys were typed again.

        ``late`` are keys the user typed after the word but before this call:
        their characters already follow the word on screen, so they are deleted
        with it and typed again after the replacement, in the new layout. Keys
        arriving during the injection are held by the tap (see
        :meth:`hold_input`) and typed again last.
        """

        try:
            return self._inject_correction(strokes, target_group, boundary, source_group, late, trailing)
        finally:
            self.release_input()

    def _inject_correction(
        self, strokes: Iterable[KeyEvent], target_group: int,
        boundary: KeyEvent | None, source_group: int | None,
        late: Sequence[KeyEvent], trailing: Sequence[KeyEvent],
    ) -> int:
        if not 0 <= target_group < len(self.sources):
            raise MacBackendError(f"Неизвестная группа раскладки {target_group}")
        stroke_list = list(strokes)
        late_list = list(late)
        literal = tuple(trailing) + (() if boundary is None else (boundary,))
        rendered_source_group = (
            source_group if source_group is not None
            else stroke_list[0].group if stroke_list else target_group
        )
        if not 0 <= rendered_source_group < len(self.sources):
            raise MacBackendError(
                f"Неизвестная исходная группа раскладки {rendered_source_group}"
            )
        delete_count = len(stroke_list) + len(literal) + len(late_list)
        delete_inputs = tuple(
            NativeInput(pressed, MAC_VK_BACKSPACE)
            for _ in range(delete_count)
            for pressed in (True, False)
        )
        replay_inputs = tuple(
            item for stroke in stroke_list
            for item in self._stroke_inputs(stroke, group=target_group)
        )
        boundary_inputs = tuple(item for stroke in literal for item in self._stroke_inputs(stroke))
        # Typed again as the user's own input: the engine must see these keys
        # as the start of the next word, not as its own injection.
        late_inputs = tuple(
            item for stroke in late_list
            for item in self._stroke_inputs(stroke, synthetic=False, group=target_group)
        )
        preserve_boundary_layout = any(
            stroke.character_for(target_group) != stroke.character for stroke in literal
        )
        literal_group = literal[0].group if literal else rendered_source_group
        if not 0 <= literal_group < len(self.sources) or any(
            stroke.group != literal_group for stroke in literal
        ):
            raise MacBackendError("Некорректная раскладка пунктуации; замена отменена")
        late_deleted = False
        late_typed = False
        failure: Exception | None = None
        with self._inject_lock:
            try:
                self._switch_group(target_group)
                if self._holding and (
                    self._pointer_epoch != self._hold_pointer_epoch
                    or self._api.focused_window() != self._hold_window
                ):
                    raise MacBackendError("Место ввода изменилось до замены; текст не изменён")
                self._post_exact(
                    delete_inputs + replay_inputs
                    + (() if preserve_boundary_layout else boundary_inputs)
                )
                late_deleted = True
                if preserve_boundary_layout:
                    self._switch_group(literal_group)
                    self._post_exact(boundary_inputs)
                    self._switch_group(target_group)
                # A partial post is not an all-or-nothing failure; retrying this
                # whole batch could duplicate an already delivered prefix.
                late_typed = True
                self._post_exact(late_inputs)
            except Exception as error:
                failure = error
            restore = late_inputs if late_deleted and not late_typed else ()
            held_count = 0
            try:
                self._post_exact(restore)
                held_count = self.release_input()
            except Exception as error:
                # The primary failure explains more than a failed restore.
                failure = failure or error
        if failure is not None:
            raise failure
        return held_count

    def _stroke_inputs(
        self, stroke: KeyEvent, *, synthetic: bool = True, group: int | None = None
    ) -> tuple[NativeInput, ...]:
        result: list[NativeInput] = []
        rendered_group = stroke.group if group is None else group
        shifted = stroke.shift
        if stroke.character_for(rendered_group).isalpha():
            shifted ^= stroke.caps_lock != self._api.caps_lock_enabled()
        replayed = not synthetic
        if shifted:
            result.append(NativeInput(True, MAC_VK_SHIFT, replayed=replayed))
        result.extend((
            NativeInput(True, stroke.keycode, replayed=replayed),
            NativeInput(False, stroke.keycode, replayed=replayed),
        ))
        if shifted:
            result.append(NativeInput(False, MAC_VK_SHIFT, replayed=replayed))
        return tuple(result)

    def _switch_group(self, group: int) -> None:
        if self.current_group() == group:
            return
        self.switch_group(group)
        deadline = time.monotonic() + LAYOUT_SWITCH_TIMEOUT_SECONDS
        while time.monotonic() < deadline:
            if self.current_group() == group:
                return
            time.sleep(LAYOUT_SWITCH_POLL_SECONDS)
        raise MacBackendError("macOS не подтвердила смену раскладки")

    def _post_exact(self, inputs: tuple[NativeInput, ...]) -> None:
        if not inputs:
            return
        posted = self._api.post_inputs(inputs)
        if posted != len(inputs):
            raise MacBackendError(f"macOS приняла {posted} из {len(inputs)} событий ввода")

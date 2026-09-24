"""The macOS half of the keyboard backend, spoken through ctypes.

The module deliberately uses only the Python standard library. It is imported
on macOS alone; every decision it could make instead lives in
:mod:`keyswitch.macos_backend`, which is why that module can be verified on any
platform and this one holds nothing but calls into the system.

Every constant below was read from Apple's own headers on a Mac rather than
recalled: the virtual key codes from ``Events.h``, the event numbers and
modifier masks from ``CGEventTypes.h`` and ``IOLLEvent.h``.
"""

from __future__ import annotations

import ctypes
import os
import threading
from collections.abc import Callable
from typing import Final

from .backend import ALT_MASK, CONTROL_MASK, SHIFT_MASK, SUPER_MASK, ScreenAnchor
from .macos_objc import frontmost_application
from .macos_backend import NativeInput, NativeKeyEvent

# Event numbers, from IOLLEvent.h through CGEventTypes.h.
EVENT_LEFT_MOUSE_DOWN: Final = 1
EVENT_RIGHT_MOUSE_DOWN: Final = 3
EVENT_KEY_DOWN: Final = 10
EVENT_KEY_UP: Final = 11
EVENT_FLAGS_CHANGED: Final = 12
EVENT_SCROLL_WHEEL: Final = 22
EVENT_OTHER_MOUSE_DOWN: Final = 25
EVENT_TAP_DISABLED_BY_TIMEOUT: Final = 0xFFFFFFFE
EVENT_TAP_DISABLED_BY_USER_INPUT: Final = 0xFFFFFFFF

POINTER_EVENTS: Final = frozenset({
    EVENT_LEFT_MOUSE_DOWN, EVENT_RIGHT_MOUSE_DOWN, EVENT_OTHER_MOUSE_DOWN, EVENT_SCROLL_WHEEL,
})
KEY_EVENTS: Final = frozenset({EVENT_KEY_DOWN, EVENT_KEY_UP, EVENT_FLAGS_CHANGED})
TAP_DISABLED_EVENTS: Final = frozenset({
    EVENT_TAP_DISABLED_BY_TIMEOUT, EVENT_TAP_DISABLED_BY_USER_INPUT,
})

# Modifier bits of a Quartz event, from IOLLEvent.h.
FLAG_ALPHA_SHIFT: Final = 0x00010000
FLAG_SHIFT: Final = 0x00020000
FLAG_CONTROL: Final = 0x00040000
FLAG_ALTERNATE: Final = 0x00080000
FLAG_COMMAND: Final = 0x00100000

# Tap placement and options.
SESSION_EVENT_TAP: Final = 1
HEAD_INSERT_EVENT_TAP: Final = 0
EVENT_TAP_OPTION_DEFAULT: Final = 0
KEYBOARD_EVENT_KEYCODE_FIELD: Final = 9
EVENT_SOURCE_USER_DATA_FIELD: Final = 42

# The marks the backend puts on events it posts itself. A tap sees its own
# injections like any other event, and without a mark it would answer its own
# corrections as though the user had typed them.
MARK_INJECTED: Final = 0x4B53_0001
MARK_REPLAYED: Final = 0x4B53_0002

# UCKeyTranslate arguments.
UC_KEY_ACTION_DISPLAY: Final = 3
UC_KEY_TRANSLATE_NO_DEAD_KEYS_MASK: Final = 1
UC_MODIFIER_SHIFT: Final = 2  # shiftKey (0x0200) >> 8, the shape UCKeyTranslate wants.
UC_MODIFIER_ALPHA_LOCK: Final = 4  # alphaLock (0x0400) >> 8.
UC_MODIFIER_OPTION: Final = 8  # optionKey (0x0800) >> 8.
UC_MODIFIER_CONTROL: Final = 16  # controlKey (0x1000) >> 8.
TRANSLATED_LENGTH: Final = 8
TEXT_BUFFER_CAPACITY_BYTES: Final = 512

CF_STRING_ENCODING_UTF8: Final = 0x08000100
CF_NUMBER_SINT32_TYPE: Final = 3
WINDOW_LIST_ON_SCREEN_ONLY: Final = 1 << 0
WINDOW_LIST_EXCLUDE_DESKTOP: Final = 1 << 4
NULL_WINDOW_ID: Final = 0
# A window the user types into sits on the normal layer; menus, the dock and
# overlays sit above it and would otherwise be mistaken for the focus.
NORMAL_WINDOW_LAYER: Final = 0
# Index of the program name within the (window id, owner pid, name) tuple
# `CtypesMacAPI._front_window` returns.
FRONT_WINDOW_NAME_INDEX: Final = 2

RUN_LOOP_SLICE_SECONDS: Final = 0.2

AX_SUCCESS: Final = 0
# AXValue wrapping a CFRange, from AXValue.h.
AX_VALUE_TYPE_CF_RANGE: Final = 4

# The pane of System Settings that holds the switch KeySwitch needs. macOS has
# no call that grants the permission; the most a program may do is ask, and then
# take the user to the place where the answer is given.
ACCESSIBILITY_SETTINGS_URL: Final = (
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
)

FRAMEWORK_PATHS: Final = {
    "Carbon": (
        "/System/Library/Frameworks/Carbon.framework/Frameworks/HIToolbox.framework/HIToolbox",
        "/System/Library/Frameworks/Carbon.framework/Carbon",
    ),
}


class MacNativeError(RuntimeError):
    """The system refused something the backend needs."""


def _framework(name: str) -> ctypes.CDLL:
    candidates = FRAMEWORK_PATHS.get(name, (f"/System/Library/Frameworks/{name}.framework/{name}",))
    failures: list[str] = []
    for path in candidates:
        try:
            return ctypes.cdll.LoadLibrary(path)
        except OSError as error:
            failures.append(f"{path}: {error}")
    raise MacNativeError(f"не удалось загрузить {name}: " + "; ".join(failures))


_cf = _framework("CoreFoundation")
_cg = _framework("CoreGraphics")
_tis = _framework("Carbon")

_cf.CFRelease.argtypes = [ctypes.c_void_p]
_cf.CFRelease.restype = None
_cf.CFRetain.argtypes = [ctypes.c_void_p]
_cf.CFRetain.restype = ctypes.c_void_p
_cf.CFArrayGetCount.argtypes = [ctypes.c_void_p]
_cf.CFArrayGetCount.restype = ctypes.c_long
_cf.CFArrayGetValueAtIndex.argtypes = [ctypes.c_void_p, ctypes.c_long]
_cf.CFArrayGetValueAtIndex.restype = ctypes.c_void_p
_cf.CFStringGetCString.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_long, ctypes.c_uint32]
_cf.CFStringGetCString.restype = ctypes.c_bool
_cf.CFDataGetBytePtr.argtypes = [ctypes.c_void_p]
_cf.CFDataGetBytePtr.restype = ctypes.c_void_p
_cf.CFDictionaryGetValue.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
_cf.CFDictionaryGetValue.restype = ctypes.c_void_p
_cf.CFNumberGetValue.argtypes = [ctypes.c_void_p, ctypes.c_long, ctypes.c_void_p]
_cf.CFNumberGetValue.restype = ctypes.c_bool
_cf.CFDictionaryCreate.argtypes = [
    ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p,
    ctypes.c_long, ctypes.c_void_p, ctypes.c_void_p,
]
_cf.CFDictionaryCreate.restype = ctypes.c_void_p
_cf.CFStringCreateWithCString.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_uint32]
_cf.CFStringCreateWithCString.restype = ctypes.c_void_p
_cf.CFURLCreateWithString.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
_cf.CFURLCreateWithString.restype = ctypes.c_void_p
_cf.CFMachPortCreateRunLoopSource.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_long]
_cf.CFMachPortCreateRunLoopSource.restype = ctypes.c_void_p
_cf.CFRunLoopGetCurrent.restype = ctypes.c_void_p
_cf.CFRunLoopAddSource.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
_cf.CFRunLoopAddSource.restype = None
_cf.CFRunLoopRemoveSource.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
_cf.CFRunLoopRemoveSource.restype = None
_cf.CFRunLoopRunInMode.argtypes = [ctypes.c_void_p, ctypes.c_double, ctypes.c_bool]
_cf.CFRunLoopRunInMode.restype = ctypes.c_int32

_COMMON_MODES = ctypes.c_void_p.in_dll(_cf, "kCFRunLoopCommonModes")
_DEFAULT_MODE = ctypes.c_void_p.in_dll(_cf, "kCFRunLoopDefaultMode")

_cg.CGEventTapCreate.argtypes = [
    ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint64,
    ctypes.c_void_p, ctypes.c_void_p,
]
_cg.CGEventTapCreate.restype = ctypes.c_void_p
_cg.CGEventTapEnable.argtypes = [ctypes.c_void_p, ctypes.c_bool]
_cg.CGEventTapEnable.restype = None
_cg.CGEventGetIntegerValueField.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
_cg.CGEventGetIntegerValueField.restype = ctypes.c_int64
_cg.CGEventSetIntegerValueField.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_int64]
_cg.CGEventSetIntegerValueField.restype = None
_cg.CGEventGetFlags.argtypes = [ctypes.c_void_p]
_cg.CGEventGetFlags.restype = ctypes.c_uint64
_cg.CGEventGetTimestamp.argtypes = [ctypes.c_void_p]
_cg.CGEventGetTimestamp.restype = ctypes.c_uint64
_cg.CGEventCreate.argtypes = [ctypes.c_void_p]
_cg.CGEventCreate.restype = ctypes.c_void_p
_cg.CGEventCreateKeyboardEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint16, ctypes.c_bool]
_cg.CGEventCreateKeyboardEvent.restype = ctypes.c_void_p
_cg.CGEventPost.argtypes = [ctypes.c_uint32, ctypes.c_void_p]
_cg.CGEventPost.restype = None
_cg.CGWindowListCopyWindowInfo.argtypes = [ctypes.c_uint32, ctypes.c_uint32]
_cg.CGWindowListCopyWindowInfo.restype = ctypes.c_void_p

_WINDOW_NUMBER_KEY = ctypes.c_void_p.in_dll(_cg, "kCGWindowNumber")
_WINDOW_LAYER_KEY = ctypes.c_void_p.in_dll(_cg, "kCGWindowLayer")
_WINDOW_OWNER_PID_KEY = ctypes.c_void_p.in_dll(_cg, "kCGWindowOwnerPID")
_WINDOW_OWNER_NAME_KEY = ctypes.c_void_p.in_dll(_cg, "kCGWindowOwnerName")

_tis.TISCreateInputSourceList.argtypes = [ctypes.c_void_p, ctypes.c_bool]
_tis.TISCreateInputSourceList.restype = ctypes.c_void_p
_tis.TISCopyCurrentKeyboardInputSource.restype = ctypes.c_void_p
_tis.TISGetInputSourceProperty.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
_tis.TISGetInputSourceProperty.restype = ctypes.c_void_p
_tis.TISSelectInputSource.argtypes = [ctypes.c_void_p]
_tis.TISSelectInputSource.restype = ctypes.c_int32
_tis.LMGetKbdType.restype = ctypes.c_uint8
_tis.UCKeyTranslate.argtypes = [
    ctypes.c_void_p, ctypes.c_uint16, ctypes.c_uint16, ctypes.c_uint32,
    ctypes.c_uint32, ctypes.c_uint32, ctypes.POINTER(ctypes.c_uint32),
    ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_uint16),
]
_tis.UCKeyTranslate.restype = ctypes.c_int32

_SOURCE_ID_PROPERTY = ctypes.c_void_p.in_dll(_tis, "kTISPropertyInputSourceID")
_LAYOUT_DATA_PROPERTY = ctypes.c_void_p.in_dll(_tis, "kTISPropertyUnicodeKeyLayoutData")

_services = _framework("ApplicationServices")
_services.AXIsProcessTrusted.restype = ctypes.c_bool
_services.AXIsProcessTrustedWithOptions.argtypes = [ctypes.c_void_p]
_services.AXIsProcessTrustedWithOptions.restype = ctypes.c_bool
_services.LSOpenCFURLRef.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
_services.LSOpenCFURLRef.restype = ctypes.c_int32
_services.AXUIElementCreateSystemWide.restype = ctypes.c_void_p
_services.AXUIElementCopyAttributeValue.argtypes = [
    ctypes.c_void_p, ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p)]
_services.AXUIElementCopyAttributeValue.restype = ctypes.c_int32
_services.AXValueGetValue.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_void_p]
_services.AXValueGetValue.restype = ctypes.c_bool
_services.AXUIElementGetPid.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_int32)]
_services.AXUIElementGetPid.restype = ctypes.c_int32

_cf.CFGetTypeID.argtypes = [ctypes.c_void_p]
_cf.CFGetTypeID.restype = ctypes.c_ulong
_cf.CFStringGetTypeID.restype = ctypes.c_ulong

_PROMPT_OPTION = ctypes.c_void_p.in_dll(_services, "kAXTrustedCheckOptionPrompt")
_TRUE_VALUE = ctypes.c_void_p.in_dll(_cf, "kCFBooleanTrue")

_TAP_CALLBACK = ctypes.CFUNCTYPE(
    ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_void_p)


def _text(reference: int | None) -> str:
    if not reference:
        return ""
    buffer = ctypes.create_string_buffer(TEXT_BUFFER_CAPACITY_BYTES)
    if not _cf.CFStringGetCString(reference, buffer, len(buffer), CF_STRING_ENCODING_UTF8):
        return ""
    return buffer.value.decode("utf-8", "replace")


def _number(dictionary: int, key: ctypes.c_void_p) -> int:
    value = _cf.CFDictionaryGetValue(dictionary, key)
    if not value:
        return 0
    result = ctypes.c_int32(0)
    if not _cf.CFNumberGetValue(value, CF_NUMBER_SINT32_TYPE, ctypes.byref(result)):
        return 0
    return int(result.value)


def _prompt_options() -> int:
    """The one option that turns the silent check into a visible request."""

    keys = (ctypes.c_void_p * 1)(_PROMPT_OPTION)
    values = (ctypes.c_void_p * 1)(_TRUE_VALUE)
    return int(_cf.CFDictionaryCreate(
        None, ctypes.cast(keys, ctypes.c_void_p), ctypes.cast(values, ctypes.c_void_p), 1,
        ctypes.byref(ctypes.c_void_p.in_dll(_cf, "kCFTypeDictionaryKeyCallBacks")),
        ctypes.byref(ctypes.c_void_p.in_dll(_cf, "kCFTypeDictionaryValueCallBacks")),
    ) or 0)


def _translate_modifiers(state: int) -> int:
    """Turn the engine's modifier bits into what UCKeyTranslate expects."""

    modifiers = 0
    if state & SHIFT_MASK:
        modifiers |= UC_MODIFIER_SHIFT
    if state & ALT_MASK:
        modifiers |= UC_MODIFIER_OPTION
    if state & CONTROL_MASK:
        modifiers |= UC_MODIFIER_CONTROL
    return modifiers


class CtypesMacAPI:
    """Everything :class:`~keyswitch.macos_backend.MacBackend` asks of macOS."""

    def __init__(self) -> None:
        self._sources: dict[str, int] = {}
        self._layouts: dict[str, int] = {}
        self._order: tuple[str, ...] = ()
        self._tap: int = 0
        self._tap_source: int = 0
        self._callback: ctypes.CFUNCTYPE | None = None  # type: ignore[valid-type]
        self._listener: Callable[[NativeKeyEvent], bool] | None = None
        self._stop = threading.Event()

    # Input sources -----------------------------------------------------

    def _refresh_sources(self) -> None:
        array = _tis.TISCreateInputSourceList(None, False)
        if not array:
            raise MacNativeError("macOS не вернула список источников ввода")
        found: dict[str, int] = {}
        layouts: dict[str, int] = {}
        order: list[str] = []
        try:
            for index in range(_cf.CFArrayGetCount(array)):
                source = _cf.CFArrayGetValueAtIndex(array, index)
                layout = _tis.TISGetInputSourceProperty(source, _LAYOUT_DATA_PROPERTY)
                if not layout:
                    continue
                identifier = _text(_tis.TISGetInputSourceProperty(source, _SOURCE_ID_PROPERTY))
                if not identifier or identifier in found:
                    continue
                # The array owns what it holds; these have to outlive it.
                found[identifier] = _cf.CFRetain(source)
                layouts[identifier] = _cf.CFRetain(layout)
                order.append(identifier)
        finally:
            _cf.CFRelease(array)
        for reference in (*self._sources.values(), *self._layouts.values()):
            _cf.CFRelease(reference)
        self._sources, self._layouts, self._order = found, layouts, tuple(order)

    def input_sources(self) -> tuple[str, ...]:
        self._refresh_sources()
        return self._order

    def current_input_source(self) -> str:
        source = _tis.TISCopyCurrentKeyboardInputSource()
        if not source:
            return ""
        try:
            return _text(_tis.TISGetInputSourceProperty(source, _SOURCE_ID_PROPERTY))
        finally:
            _cf.CFRelease(source)

    def select_input_source(self, identifier: str) -> bool:
        source = self._sources.get(identifier)
        if source is None:
            self._refresh_sources()
            source = self._sources.get(identifier)
        if source is None:
            return False
        return int(_tis.TISSelectInputSource(source)) == 0

    def translate_key(self, keycode: int, state: int, source: str) -> str:
        layout = self._layouts.get(source)
        if layout is None:
            self._refresh_sources()
            layout = self._layouts.get(source)
        if layout is None:
            return ""
        pointer = _cf.CFDataGetBytePtr(layout)
        if not pointer:
            return ""
        dead_key_state = ctypes.c_uint32(0)
        length = ctypes.c_ulong(0)
        buffer = (ctypes.c_uint16 * TRANSLATED_LENGTH)()
        status = _tis.UCKeyTranslate(
            ctypes.c_void_p(pointer), keycode, UC_KEY_ACTION_DISPLAY,
            _translate_modifiers(state), _tis.LMGetKbdType(),
            UC_KEY_TRANSLATE_NO_DEAD_KEYS_MASK, ctypes.byref(dead_key_state),
            len(buffer), ctypes.byref(length), buffer,
        )
        if status != 0 or length.value == 0:
            return ""
        return "".join(chr(buffer[index]) for index in range(length.value))

    # Injection ---------------------------------------------------------

    def post_inputs(self, inputs: tuple[NativeInput, ...]) -> int:
        posted = 0
        for item in inputs:
            event = _cg.CGEventCreateKeyboardEvent(None, item.keycode, item.pressed)
            if not event:
                break
            try:
                _cg.CGEventSetIntegerValueField(
                    event, EVENT_SOURCE_USER_DATA_FIELD,
                    MARK_REPLAYED if item.replayed else MARK_INJECTED)
                _cg.CGEventPost(SESSION_EVENT_TAP, event)
            finally:
                _cf.CFRelease(event)
            posted += 1
        return posted

    # The focused window ------------------------------------------------

    def _front_window(self) -> tuple[int, int, str]:
        """The window id, its owning process and its program name."""

        focused, name = frontmost_application()
        if not focused:
            # Naming the wrong program is worse than naming none: the engine
            # would apply another application's exclusions to what is typed here.
            return 0, 0, ""
        info = _cg.CGWindowListCopyWindowInfo(
            WINDOW_LIST_ON_SCREEN_ONLY | WINDOW_LIST_EXCLUDE_DESKTOP, NULL_WINDOW_ID)
        if not info:
            return 0, 0, ""
        try:
            for index in range(_cf.CFArrayGetCount(info)):
                entry = _cf.CFArrayGetValueAtIndex(info, index)
                if _number(entry, _WINDOW_LAYER_KEY) != NORMAL_WINDOW_LAYER:
                    continue
                owner = _number(entry, _WINDOW_OWNER_PID_KEY)
                if owner != focused:
                    continue
                return _number(entry, _WINDOW_NUMBER_KEY), owner, name
        finally:
            _cf.CFRelease(info)
        # A program with no ordinary window - a menu bar agent, a full screen
        # view - is still the one being typed into.
        return 0, focused, name

    def active_application(self) -> str:
        return self._front_window()[FRONT_WINDOW_NAME_INDEX]

    def focused_window(self) -> int:
        return self._front_window()[0]

    def window_process_id(self, window: int) -> int:
        number, process, _name = self._front_window()
        return process if number == window else 0

    def current_process_id(self) -> int:
        return os.getpid()

    def input_anchor(self) -> ScreenAnchor | None:
        """Where a prompt should appear; not yet answered on macOS.

        The caret's position comes from the accessibility tree rather than from
        the window server, and the prompt places itself sensibly without it.
        """

        return None

    def accessibility_trusted(self, *, prompt: bool = False) -> bool:
        """Whether macOS lets this program watch the keyboard.

        Reading the answer never shows anything; only asking with the prompt
        option makes macOS put up its own request, which is the only way a
        program can bring the switch to the user's attention.
        """

        if not prompt:
            return bool(_services.AXIsProcessTrusted())
        options = _prompt_options()
        try:
            return bool(_services.AXIsProcessTrustedWithOptions(options))
        finally:
            if options:
                _cf.CFRelease(options)

    def open_accessibility_settings(self) -> bool:
        """Open the pane holding the switch, so nobody has to find it."""

        text = _cf.CFStringCreateWithCString(
            None, ACCESSIBILITY_SETTINGS_URL.encode("utf-8"), CF_STRING_ENCODING_UTF8)
        if not text:
            return False
        url = _cf.CFURLCreateWithString(None, text, None)
        _cf.CFRelease(text)
        if not url:
            return False
        try:
            return int(_services.LSOpenCFURLRef(url, None)) == 0
        finally:
            _cf.CFRelease(url)

    def caps_lock_enabled(self) -> bool:
        event = _cg.CGEventCreate(None)
        if not event:
            return False
        try:
            return bool(_cg.CGEventGetFlags(event) & FLAG_ALPHA_SHIFT)
        finally:
            _cf.CFRelease(event)

    def layout_follows_window(self) -> bool:
        """Whether each window carries a layout the user did not choose.

        macOS keeps one input source for the session unless "Automatically
        switch to a document's input source" is turned on in the Keyboard
        settings. Reading that preference is not implemented, so the answer is
        the behaviour every default installation has.
        """

        return False

    # The event tap -----------------------------------------------------

    def run_event_tap(
        self,
        listener: Callable[[NativeKeyEvent], bool],
        ready: Callable[[], None],
    ) -> None:
        self._listener = listener
        self._stop.clear()
        self._callback = _TAP_CALLBACK(self._on_event)
        mask = 0
        for event_type in (*KEY_EVENTS, *POINTER_EVENTS):
            mask |= 1 << event_type
        self._tap = _cg.CGEventTapCreate(
            SESSION_EVENT_TAP, HEAD_INSERT_EVENT_TAP, EVENT_TAP_OPTION_DEFAULT, mask,
            ctypes.cast(self._callback, ctypes.c_void_p), None)
        if not self._tap:
            raise MacNativeError(
                "macOS отказала в перехвате клавиатуры: разрешите KeySwitch в разделе "
                "«Конфиденциальность и безопасность» → «Универсальный доступ»"
            )
        self._tap_source = _cf.CFMachPortCreateRunLoopSource(None, self._tap, 0)
        _cf.CFRunLoopAddSource(_cf.CFRunLoopGetCurrent(), self._tap_source, _COMMON_MODES)
        _cg.CGEventTapEnable(self._tap, True)
        ready()
        try:
            while not self._stop.is_set():
                _cf.CFRunLoopRunInMode(_DEFAULT_MODE, RUN_LOOP_SLICE_SECONDS, False)
        finally:
            self._teardown()

    def _teardown(self) -> None:
        if self._tap:
            _cg.CGEventTapEnable(self._tap, False)
        if self._tap_source:
            _cf.CFRunLoopRemoveSource(_cf.CFRunLoopGetCurrent(), self._tap_source, _COMMON_MODES)
            _cf.CFRelease(self._tap_source)
            self._tap_source = 0
        if self._tap:
            _cf.CFRelease(self._tap)
            self._tap = 0
        self._callback = None

    def enable_event_tap(self) -> None:
        if self._tap:
            _cg.CGEventTapEnable(self._tap, True)

    def stop_event_tap(self) -> None:
        self._stop.set()

    def _on_event(self, _proxy: int, event_type: int, event: int, _info: int) -> int | None:
        listener = self._listener
        if listener is None:
            return event
        if event_type in TAP_DISABLED_EVENTS:
            listener(NativeKeyEvent(False, 0, 0, event_type=event_type))
            return event
        timestamp = int(_cg.CGEventGetTimestamp(event))
        if event_type in POINTER_EVENTS:
            listener(NativeKeyEvent(True, 0, timestamp, pointer=True))
            return event
        mark = int(_cg.CGEventGetIntegerValueField(event, EVENT_SOURCE_USER_DATA_FIELD))
        native = NativeKeyEvent(
            event_type == EVENT_KEY_DOWN,
            int(_cg.CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE_FIELD)),
            timestamp,
            injected=mark == MARK_INJECTED,
            replayed=mark == MARK_REPLAYED,
            event_type=event_type,
        )
        return None if listener(native) else event


class _CFRange(ctypes.Structure):
    _fields_ = [  # type: ignore[mutable-override]
        ("location", ctypes.c_long), ("length", ctypes.c_long),
    ]


class CtypesAccessibilityAPI:
    """The accessibility tree, for reading what stands before the caret.

    Attribute names are turned into CoreFoundation strings once and kept: a read
    happens on every word the user finishes, and building them again each time
    would be work for nothing.
    """

    def __init__(self) -> None:
        self._system = _services.AXUIElementCreateSystemWide()
        self._names: dict[str, int] = {}

    def _name(self, name: str) -> int:
        cached = self._names.get(name)
        if cached is None:
            cached = int(_cf.CFStringCreateWithCString(
                None, name.encode("utf-8"), CF_STRING_ENCODING_UTF8) or 0)
            self._names[name] = cached
        return cached

    def _attribute(self, element: int, name: str) -> int:
        """The attribute's value, owned by the caller, or zero."""

        if not element:
            return 0
        key = self._name(name)
        if not key:
            return 0
        value = ctypes.c_void_p(0)
        status = _services.AXUIElementCopyAttributeValue(element, key, ctypes.byref(value))
        if status != AX_SUCCESS:
            return 0
        return int(value.value or 0)

    def focused_element(self) -> int:
        return self._attribute(int(self._system or 0), "AXFocusedUIElement")

    def string_attribute(self, element: int, name: str) -> str | None:
        value = self._attribute(element, name)
        if not value:
            return None
        try:
            # An attribute may hold a number or a structure; asking a non-string
            # for its characters would read whatever happens to be there.
            if _cf.CFGetTypeID(value) != _cf.CFStringGetTypeID():
                return None
            return _text(value)
        finally:
            _cf.CFRelease(value)

    def range_attribute(self, element: int, name: str) -> tuple[int, int] | None:
        value = self._attribute(element, name)
        if not value:
            return None
        try:
            span = _CFRange()
            if not _services.AXValueGetValue(value, AX_VALUE_TYPE_CF_RANGE, ctypes.byref(span)):
                return None
            return int(span.location), int(span.length)
        finally:
            _cf.CFRelease(value)

    def release(self, element: int) -> None:
        if element:
            _cf.CFRelease(element)

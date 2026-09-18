"""The menu bar item, drawn by talking to the Objective-C runtime directly.

The module deliberately uses only the Python standard library, like the other
platform layers. AppKit has no C interface, so every call here is a message send
put together by hand; the menu it draws is described once in
:mod:`keyswitch.tray_model` and merely rendered here.

The menu is rebuilt when AppKit announces it is about to open, rather than
whenever the state changes. That way every AppKit call happens on the main
thread where AppKit requires it, while the engine keeps changing the state from
its own.

Nothing here runs an event loop. Tk owns the main thread and, on macOS, is an
AppKit program underneath, so it delivers the events this item needs.
"""

from __future__ import annotations

import ctypes
import logging
from collections.abc import Callable
from typing import Final

from .macos_objc import CType, ObjCError, objc_class, runtime, selector, send, string
from .tray_model import MenuEntry, TrayActions, TrayState, menu_entries

LOGGER: Final = logging.getLogger(__name__)

# NSApplication presents no dock icon and no menu bar of its own.
ACTIVATION_POLICY_ACCESSORY: Final = 1
# NSStatusItem asks for the width its content needs.
VARIABLE_STATUS_ITEM_LENGTH: Final = -1.0
CONTROL_STATE_ON: Final = 1
CONTROL_STATE_OFF: Final = 0
TARGET_CLASS_NAME: Final = b"KeySwitchMenuTarget"
# Objective-C type encodings: void return, object and selector, one object.
VOID_OBJECT_METHOD: Final = b"v@:@"


class MacTrayError(RuntimeError):
    """AppKit refused something the menu bar item needs."""


_objc = runtime


def _selector(name: str) -> ctypes.c_void_p:
    return selector(name)


def _class(name: str) -> ctypes.c_void_p:
    try:
        return objc_class(name)
    except ObjCError as error:
        raise MacTrayError(str(error)) from error


def _send(
    receiver: ctypes.c_void_p | int | None,
    message: str,
    *arguments: object,
    restype: CType = ctypes.c_void_p,
    argtypes: tuple[CType, ...] = (),
) -> int:
    return send(receiver, message, *arguments, restype=restype, argtypes=argtypes)


def _string(text: str) -> int:
    return string(text)


class StatusItemAdapter:
    """Draw the tray model as an item in the macOS menu bar."""

    def __init__(self) -> None:
        self._actions: TrayActions | None = None
        self._state: Callable[[], TrayState] | None = None
        self._entries: tuple[MenuEntry, ...] = ()
        self._item = 0
        self._menu = 0
        self._target = 0
        self._implementations: list[object] = []
        self._closed = False

    # Building ----------------------------------------------------------

    def _build_target(self) -> int:
        """A class that exists only at run time, to receive AppKit's calls."""

        handler = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p)
        chosen = handler(self._menu_item_chosen)
        about_to_open = handler(self._menu_about_to_open)
        apply_title = handler(self._apply_title)
        self._implementations = [chosen, about_to_open, apply_title]
        created = _objc.objc_allocateClassPair(_class("NSObject"), TARGET_CLASS_NAME, 0)
        if not created:
            raise MacTrayError("не удалось создать класс-получатель для меню")
        for selector, implementation in (
            ("keySwitchMenuItem:", chosen),
            ("menuNeedsUpdate:", about_to_open),
            ("keySwitchApplyTitle:", apply_title),
        ):
            if not _objc.class_addMethod(
                ctypes.c_void_p(created), _selector(selector),
                ctypes.cast(implementation, ctypes.c_void_p), VOID_OBJECT_METHOD,
            ):
                raise MacTrayError(f"AppKit отказалась принять метод {selector}")
        _objc.objc_registerClassPair(ctypes.c_void_p(created))
        return _send(_send(ctypes.c_void_p(created), "alloc"), "init")

    def start(
        self,
        actions: TrayActions,
        state: Callable[[], TrayState],
    ) -> None:
        self._actions = actions
        self._state = state
        application = _send(_class("NSApplication"), "sharedApplication")
        _send(application, "setActivationPolicy:", ACTIVATION_POLICY_ACCESSORY,
              restype=ctypes.c_bool, argtypes=(ctypes.c_long,))
        status_bar = _send(_class("NSStatusBar"), "systemStatusBar")
        item = _send(status_bar, "statusItemWithLength:",
                     ctypes.c_double(VARIABLE_STATUS_ITEM_LENGTH), argtypes=(ctypes.c_double,))
        if not item:
            raise MacTrayError("macOS не выдала место в строке меню")
        # The status bar hands out an item it does not keep for us.
        self._item = _send(item, "retain")
        self._target = self._build_target()
        self._menu = _send(_send(_class("NSMenu"), "alloc"), "init")
        # Without this AppKit greys out every item that has no target of its own.
        _send(self._menu, "setAutoenablesItems:", False,
              argtypes=(ctypes.c_bool,))
        _send(self._menu, "setDelegate:", self._target, argtypes=(ctypes.c_void_p,))
        _send(self._item, "setMenu:", self._menu, argtypes=(ctypes.c_void_p,))
        self.update(state())

    # Drawing -----------------------------------------------------------

    def update(self, state: TrayState) -> None:
        """Show the layout on the bar; the menu is rebuilt when it opens.

        The engine changes the state from its own thread, and AppKit may only be
        touched from the main one, so the new title is handed over rather than
        written here. Waiting for it would let a stalled main thread hold up the
        engine, so the call does not wait.
        """

        if self._closed or not self._item or not self._target:
            return
        _send(
            self._target, "performSelectorOnMainThread:withObject:waitUntilDone:",
            _selector("keySwitchApplyTitle:"), _string(state.label), False,
            argtypes=(ctypes.c_void_p, ctypes.c_void_p, ctypes.c_bool),
        )

    def _apply_title(self, _self: int, _selector: int, title: int) -> None:
        try:
            button = _send(self._item, "button")
            if button:
                _send(button, "setTitle:", title, argtypes=(ctypes.c_void_p,))
        except Exception:
            LOGGER.exception("не удалось обновить строку меню")

    def _rebuild_menu(self) -> None:
        actions, state = self._actions, self._state
        if actions is None or state is None or not self._menu:
            return
        self._entries = menu_entries(state(), actions)
        _send(self._menu, "removeAllItems")
        empty = _string("")
        for index, entry in enumerate(self._entries):
            if entry.separator:
                _send(self._menu, "addItem:", _send(_class("NSMenuItem"), "separatorItem"),
                      argtypes=(ctypes.c_void_p,))
                continue
            item = _send(
                _send(_class("NSMenuItem"), "alloc"),
                "initWithTitle:action:keyEquivalent:",
                _string(entry.label),
                _selector("keySwitchMenuItem:") if entry.action is not None else None,
                empty,
                argtypes=(ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p),
            )
            _send(item, "setTarget:", self._target, argtypes=(ctypes.c_void_p,))
            _send(item, "setTag:", index, argtypes=(ctypes.c_long,))
            _send(item, "setEnabled:", entry.enabled and entry.action is not None,
                  argtypes=(ctypes.c_bool,))
            if entry.checked is not None:
                _send(item, "setState:",
                      CONTROL_STATE_ON if entry.checked else CONTROL_STATE_OFF,
                      argtypes=(ctypes.c_long,))
            _send(self._menu, "addItem:", item, argtypes=(ctypes.c_void_p,))

    # AppKit's calls ----------------------------------------------------

    def _menu_about_to_open(self, _self: int, _selector: int, _menu: int) -> None:
        try:
            self._rebuild_menu()
        except Exception:
            # An exception thrown back into AppKit would end the process.
            LOGGER.exception("не удалось перестроить меню")

    def _menu_item_chosen(self, _self: int, _selector: int, sender: int) -> None:
        try:
            index = _send(sender, "tag", restype=ctypes.c_long)
            if 0 <= index < len(self._entries):
                action = self._entries[index].action
                if action is not None:
                    action()
        except Exception:
            LOGGER.exception("не удалось выполнить пункт меню")

    # Finishing ---------------------------------------------------------

    def notify(self, title: str, message: str) -> None:
        """Notifications need a signed bundle, so they wait for one."""

        LOGGER.info("%s: %s", title, message)

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        if self._item:
            status_bar = _send(_class("NSStatusBar"), "systemStatusBar")
            _send(status_bar, "removeStatusItem:", self._item, argtypes=(ctypes.c_void_p,))
            _send(self._item, "release")
            self._item = 0


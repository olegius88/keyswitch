"""The few Objective-C runtime calls the macOS layers share.

AppKit has no C interface, so reaching it means sending messages by hand. Both
the menu bar item and the input layer need the same handful of calls, and they
are written once here rather than twice.

The module deliberately uses only the standard library, and is imported on macOS
alone.
"""

from __future__ import annotations

import ctypes
import ctypes.util
from typing import Final, TYPE_CHECKING

if TYPE_CHECKING:  # ctypes spells its own base type privately.
    from ctypes import _CData

    CType = type[_CData]
else:
    CType = type


class ObjCError(RuntimeError):
    """The Objective-C runtime refused something."""


def library(path: str) -> ctypes.CDLL:
    try:
        return ctypes.cdll.LoadLibrary(path)
    except OSError as error:
        raise ObjCError(f"не удалось загрузить {path}: {error}") from error


runtime: Final = library(ctypes.util.find_library("objc") or "/usr/lib/libobjc.dylib")
appkit: Final = library("/System/Library/Frameworks/AppKit.framework/AppKit")
foundation: Final = library("/System/Library/Frameworks/Foundation.framework/Foundation")

runtime.objc_getClass.argtypes = [ctypes.c_char_p]
runtime.objc_getClass.restype = ctypes.c_void_p
runtime.sel_registerName.argtypes = [ctypes.c_char_p]
runtime.sel_registerName.restype = ctypes.c_void_p
runtime.objc_allocateClassPair.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_size_t]
runtime.objc_allocateClassPair.restype = ctypes.c_void_p
runtime.objc_registerClassPair.argtypes = [ctypes.c_void_p]
runtime.objc_registerClassPair.restype = None
runtime.class_addMethod.argtypes = [
    ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_char_p]
runtime.class_addMethod.restype = ctypes.c_bool


def selector(name: str) -> ctypes.c_void_p:
    return ctypes.c_void_p(runtime.sel_registerName(name.encode("utf-8")))


def objc_class(name: str) -> ctypes.c_void_p:
    handle = runtime.objc_getClass(name.encode("utf-8"))
    if not handle:
        raise ObjCError(f"класс {name} недоступен")
    return ctypes.c_void_p(handle)


def send(
    receiver: ctypes.c_void_p | int | None,
    message: str,
    *arguments: object,
    restype: CType = ctypes.c_void_p,
    argtypes: tuple[CType, ...] = (),
) -> int:
    """One Objective-C message, with the signature spelled out.

    ``objc_msgSend`` has no fixed prototype: it must be cast to the one the
    receiving method actually has, or the arguments land in the wrong registers.
    """

    prototype = ctypes.CFUNCTYPE(restype, ctypes.c_void_p, ctypes.c_void_p, *argtypes)
    call = ctypes.cast(runtime.objc_msgSend, prototype)
    result = call(receiver, selector(message), *arguments)
    return int(result) if result else 0


def string(text: str) -> int:
    return send(objc_class("NSString"), "stringWithUTF8String:",
                text.encode("utf-8"), argtypes=(ctypes.c_char_p,))


def text_of(reference: int) -> str:
    """The characters of an NSString.

    The answer is bytes rather than a number, so the message is sent through its
    own prototype: the general one returns an integer and would choke on them.
    """

    if not reference:
        return ""
    prototype = ctypes.CFUNCTYPE(ctypes.c_char_p, ctypes.c_void_p, ctypes.c_void_p)
    call = ctypes.cast(runtime.objc_msgSend, prototype)
    characters = call(reference, selector("UTF8String"))
    return characters.decode("utf-8", "replace") if characters else ""


def frontmost_application() -> tuple[int, str]:
    """The process the user is working in, and its name.

    ``NSWorkspace`` answers this directly. The accessibility tree can be asked
    the same question, but only once its connection is established, which it is
    not when a program has just started.
    """

    workspace = send(objc_class("NSWorkspace"), "sharedWorkspace")
    application = send(workspace, "frontmostApplication")
    if not application:
        return 0, ""
    process = send(application, "processIdentifier", restype=ctypes.c_int32)
    return int(process), text_of(send(application, "localizedName"))


appkit.NSBeep.argtypes = []
appkit.NSBeep.restype = None


def beep() -> None:
    """The system's own alert sound, whatever the user has chosen it to be."""

    appkit.NSBeep()


def open_path(path: str) -> bool:
    """Show a file or a folder in the Finder."""

    workspace = send(objc_class("NSWorkspace"), "sharedWorkspace")
    url = send(objc_class("NSURL"), "fileURLWithPath:", string(path),
               argtypes=(ctypes.c_void_p,))
    if not url:
        return False
    return bool(send(workspace, "openURL:", url,
                     restype=ctypes.c_bool, argtypes=(ctypes.c_void_p,)))

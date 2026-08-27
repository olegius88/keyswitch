"""Real bidirectional Win32 hook/SendInput/Tk correction E2E."""

from __future__ import annotations

import os
import sys
import tempfile
import threading
import time
from collections.abc import Callable
from pathlib import Path


def _running_on_windows() -> bool:
    return sys.platform == "win32"


def main() -> int:
    if not _running_on_windows():
        raise RuntimeError("Windows E2E must run on Windows")

    from keyswitch.config import SettingsStore
    from keyswitch.windows_backend import NativeInput, NativeKeyEvent
    from keyswitch.windows_native import CtypesWindowsAPI
    from keyswitch.windows_ui import PAGE_NAMES, WindowsApplication

    api = CtypesWindowsAPI()
    ready = threading.Event()
    received = threading.Event()
    events: list[NativeKeyEvent] = []
    errors: list[Exception] = []

    def listener(event: NativeKeyEvent) -> None:
        events.append(event)
        if event.virtual_key == 0x87:
            received.set()

    def hook_loop() -> None:
        try:
            api.run_keyboard_hook(listener, ready.set)
        except Exception as error:
            errors.append(error)
            ready.set()

    thread = threading.Thread(target=hook_loop, name="keyswitch-win32-e2e")
    thread.start()
    if not ready.wait(5.0):
        raise RuntimeError("WH_KEYBOARD_LL did not become ready")
    if errors:
        raise errors[0]
    try:
        inputs = (
            NativeInput(True, virtual_key=0x87),
            NativeInput(False, virtual_key=0x87),
        )
        if api.send_inputs(inputs) != len(inputs):
            raise RuntimeError("SendInput did not accept the F24 smoke sequence")
        if not received.wait(5.0):
            raise RuntimeError("WH_KEYBOARD_LL did not observe the injected F24 event")
        if not any(event.injected for event in events if event.virtual_key == 0x87):
            raise RuntimeError("Injected Win32 event was not marked synthetic")
    finally:
        api.stop_keyboard_hook()
        thread.join(timeout=5.0)
    if thread.is_alive():
        raise RuntimeError("Win32 hook thread did not stop")
    print("WINDOWS_HOOK_E2E_OK", flush=True)

    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        os.environ["KEYSWITCH_CONFIG_DIR"] = str(root / "config")
        os.environ["KEYSWITCH_DATA_DIR"] = str(root / "data")
        settings = SettingsStore()
        settings.set("general.autostart", False)
        settings.set("appearance.show_indicator", True)
        settings.set("general.notifications", False)
        settings.set("detection.respect_manual_layout", False)

        print("WINDOWS_UI_INIT_START", flush=True)
        visual_application = WindowsApplication(hidden=False, no_engine=True)
        try:
            for page_name, title in PAGE_NAMES:
                visual_application.show_page(page_name)
                visual_application.root.update()
                button = visual_application._navigation[page_name]
                if button.cget("text") != title:
                    raise RuntimeError(f"Navigation label is missing for {page_name}")
                if visual_application._active_page != page_name:
                    raise RuntimeError(f"Navigation did not activate {page_name}")
                if (
                    button.cget("background")
                    != visual_application._navigation_accent
                ):
                    raise RuntimeError(
                        f"Navigation highlight is missing for {page_name}"
                    )
                if button.cget("foreground") == button.cget("background"):
                    raise RuntimeError(
                        f"Navigation label has no contrast for {page_name}"
                    )
        finally:
            visual_application.shutdown()
        print("WINDOWS_UI_E2E_OK", flush=True)

        print("WINDOWS_ENGINE_INIT_START", flush=True)
        application = WindowsApplication(hidden=False, no_engine=False)
        print("WINDOWS_ENGINE_INIT_OK", flush=True)
        scenario_errors: list[Exception] = []
        completed: list[bool] = []
        layout_deadline = [0.0]
        focus_deadline = [0.0]

        def fail(error: Exception) -> None:
            scenario_errors.append(error)
            application.shutdown()

        def send_scans(scan_codes: tuple[int, ...]) -> None:
            user_inputs = tuple(
                NativeInput(pressed, scan_code=scan_code, synthetic=False)
                for scan_code in scan_codes
                for pressed in (True, False)
            )
            if api.send_inputs(user_inputs) != len(user_inputs):
                raise RuntimeError("SendInput did not accept the user word")

        def wait_for_text(
            expected: str,
            expected_group: int,
            on_success: Callable[[], None],
            deadline: float,
        ) -> None:
            if application.test_entry.get() == expected:
                if application.backend.current_group() != expected_group:
                    fail(RuntimeError(f"Unexpected layout after correction to {expected}"))
                    return
                on_success()
                return
            if time.monotonic() >= deadline:
                fail(
                    RuntimeError(
                        f"Timed out waiting for {expected!r}; "
                        f"field={application.test_entry.get()!r}"
                    )
                )
                return
            application.root.after(
                50,
                lambda: wait_for_text(
                    expected,
                    expected_group,
                    on_success,
                    deadline,
                ),
            )

        def finish_reverse() -> None:
            pairs = [
                (entry.original, entry.replacement)
                for entry in application.history.read()
            ]
            if pairs[-2:] != [("ghbdtn", "привет"), ("руддщ", "hello")]:
                fail(RuntimeError(f"Unexpected Windows correction history: {pairs}"))
                return
            print("WINDOWS_RU_TO_EN_E2E_OK", flush=True)
            completed.append(True)
            application.shutdown()

        def start_reverse() -> None:
            try:
                send_scans((0x23, 0x12, 0x26, 0x26, 0x18, 0x39))
                wait_for_text("hello ", 0, finish_reverse, time.monotonic() + 10.0)
            except Exception as error:
                fail(error)

        def wait_for_russian_layout() -> None:
            try:
                application.present()
                api.activate_window(application.root.winfo_id())
                application.test_entry.focus_force()
                application.root.update_idletasks()
                foreground = api.active_application().casefold()
                expected = Path(sys.executable).stem.casefold()
                ready_for_input = (
                    application.backend.current_group() == 1
                    and foreground == expected
                    and application.root.focus_get() is application.test_entry
                )
                if not ready_for_input:
                    if time.monotonic() >= layout_deadline[0]:
                        raise RuntimeError(
                            "Russian input preparation timed out; "
                            f"group={application.backend.current_group()}, "
                            f"expected_app={expected!r}, actual_app={foreground!r}, "
                            f"focus={application.root.focus_get()!r}"
                        )
                    application.root.after(50, wait_for_russian_layout)
                    return
                start_reverse()
            except Exception as error:
                fail(error)

        def prepare_reverse() -> None:
            try:
                application.present()
                api.activate_window(application.root.winfo_id())
                application.test_entry.focus_force()
                application.test_entry.delete(0, "end")
                application.root.update_idletasks()
                if application.test_entry.get():
                    raise RuntimeError("KeySwitch E2E field did not clear")
                if api.active_application().casefold() != Path(sys.executable).stem.casefold():
                    raise RuntimeError("KeySwitch E2E lost foreground before RU to EN pass")
                if not api.request_layout(application.backend.layouts[1]):
                    raise RuntimeError("Cannot select the Russian layout for E2E")
                layout_deadline[0] = time.monotonic() + 5.0
                application.root.after(100, wait_for_russian_layout)
            except Exception as error:
                fail(error)

        def finish_forward() -> None:
            print("WINDOWS_EN_TO_RU_E2E_OK", flush=True)
            application.root.after(300, prepare_reverse)

        def start_forward() -> None:
            try:
                application.present()
                if application.tray is None:
                    raise RuntimeError("Native Windows notification-area icon is missing")
                application.tray.set_indicator_style("flags")
                focus_deadline[0] = time.monotonic() + 5.0
                wait_for_test_focus()
            except Exception as error:
                fail(error)

        def wait_for_test_focus() -> None:
            try:
                application.present()
                api.activate_window(application.root.winfo_id())
                application.test_entry.focus_force()
                application.root.update_idletasks()
                foreground = api.active_application().casefold()
                expected = Path(sys.executable).stem.casefold()
                if foreground != expected:
                    if time.monotonic() >= focus_deadline[0]:
                        raise RuntimeError(
                            "KeySwitch E2E window did not become foreground; "
                            f"expected={expected!r}, actual={foreground!r}"
                        )
                    application.root.after(50, wait_for_test_focus)
                    return
                if application.root.focus_get() is not application.test_entry:
                    if time.monotonic() >= focus_deadline[0]:
                        raise RuntimeError("KeySwitch E2E entry did not receive focus")
                    application.root.after(50, wait_for_test_focus)
                    return
                if not api.request_layout(application.backend.layouts[0]):
                    raise RuntimeError("Cannot select the English layout for E2E")
                layout_deadline[0] = time.monotonic() + 5.0
                application.root.after(100, wait_for_english_layout)
            except Exception as error:
                fail(error)

        def wait_for_english_layout() -> None:
            try:
                if application.backend.current_group() != 0:
                    if time.monotonic() >= layout_deadline[0]:
                        raise RuntimeError("English layout selection timed out")
                    application.root.after(50, wait_for_english_layout)
                    return
                send_scans((0x22, 0x23, 0x30, 0x20, 0x14, 0x31, 0x39))
                wait_for_text("привет ", 1, finish_forward, time.monotonic() + 10.0)
            except Exception as error:
                fail(error)

        application.root.after(300, start_forward)
        if application.run() != 0:
            raise RuntimeError("Tk Windows UI smoke test returned a failure")
        if scenario_errors:
            raise scenario_errors[0]
        if not completed:
            raise RuntimeError("Windows correction scenario did not complete")

    print("WINDOWS_E2E_OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

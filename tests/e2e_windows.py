"""Real bidirectional Win32 hook/SendInput/Tk correction E2E."""

from __future__ import annotations

import logging
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
    from keyswitch.windows_tray import WindowsTrayState
    from keyswitch.windows_tray_native import ICON_SIZE, PystrayWindowsAdapter
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

    flag_expectations = {
        0: ((60, 59, 110, 255), (178, 34, 52, 255)),
        1: ((255, 255, 255, 255), (213, 43, 30, 255)),
    }
    for group, (top_left, bottom_right) in flag_expectations.items():
        flag = PystrayWindowsAdapter._render(
            WindowsTrayState(group=group, indicator_style="flags")
        )
        if flag.size != (ICON_SIZE, ICON_SIZE):
            raise RuntimeError(f"Unexpected Windows flag icon size: {flag.size}")
        if flag.getbbox() != (0, 0, ICON_SIZE, ICON_SIZE):
            raise RuntimeError(f"Windows flag does not fill the icon: {flag.getbbox()}")
        if flag.getpixel((0, 0)) != top_left:
            raise RuntimeError("Windows flag still has a decorative top-left frame")
        if flag.getpixel((ICON_SIZE - 1, ICON_SIZE - 1)) != bottom_right:
            raise RuntimeError("Windows flag still has a decorative bottom-right frame")
    print("WINDOWS_FULL_SIZE_FLAGS_E2E_OK", flush=True)

    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        os.environ["KEYSWITCH_CONFIG_DIR"] = str(root / "config")
        os.environ["KEYSWITCH_DATA_DIR"] = str(root / "data")
        settings = SettingsStore()
        settings.set("general.autostart", False)
        settings.set("appearance.show_indicator", True)
        settings.set("general.notifications", False)
        settings.set("detection.respect_manual_layout", False)
        # Synthetic typing has no inter-key gaps; prefix switching is covered
        # by unit tests and would race with the burst here.
        settings.set("detection.early_switch", False)
        settings.set("updates.check_automatically", False)

        print("WINDOWS_UI_INIT_START", flush=True)
        visual_application = WindowsApplication(hidden=False, no_engine=True)
        try:
            for page_name, title in PAGE_NAMES:
                visual_application.show_page(page_name)
                visual_application.root.update()
                page = visual_application._pages[page_name]
                viewport = visual_application._page_viewports[page_name]
                region = str(viewport.cget("scrollregion")).split()
                if len(region) != 4:
                    raise RuntimeError(f"Page {page_name} has no scroll region")
                if int(float(region[3])) < page.winfo_reqheight():
                    raise RuntimeError(
                        f"Page {page_name} cannot scroll to its full height"
                    )
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
            pause_variable = visual_application._boolean_variables.get(
                "detection.correct_on_pause"
            )
            if pause_variable is None or not pause_variable.get():
                raise RuntimeError("Pause correction switch is missing or disabled")
            pause_variable.set(False)
            visual_application._save_boolean(
                "detection.correct_on_pause", pause_variable
            )
            if visual_application.settings.get("detection.correct_on_pause") is not False:
                raise RuntimeError("Pause correction switch did not save the off state")
            pause_indicator = visual_application._setting_indicators[
                "detection.correct_on_pause"
            ][0]
            visual_application.root.update()
            if not pause_indicator.marker.winfo_ismapped():
                raise RuntimeError("A changed setting is not marked as changed")
            if not pause_indicator.reset.winfo_ismapped():
                raise RuntimeError("A changed setting offers no reset button")
            pause_indicator.reset.invoke()
            visual_application.root.update()
            if visual_application.settings.get("detection.correct_on_pause") is not True:
                raise RuntimeError("The reset button did not restore the default")
            if not pause_variable.get():
                raise RuntimeError("The switch did not follow the restored value")
            if pause_indicator.marker.winfo_ismapped():
                raise RuntimeError("A default setting is still marked as changed")

            visual_application.show_page("autocorrection")
            visual_application.root.update()
            settings_viewport = visual_application._page_viewports["autocorrection"]
            if settings_viewport.yview()[1] < 1.0:
                settings_viewport.event_generate(
                    "<MouseWheel>",
                    delta=-120,
                    x=20,
                    y=20,
                    rootx=settings_viewport.winfo_rootx() + 20,
                    rooty=settings_viewport.winfo_rooty() + 20,
                )
                visual_application.root.update()
                if settings_viewport.yview()[0] <= 0.0:
                    raise RuntimeError("The wheel does not scroll the settings page")
                settings_viewport.yview_moveto(0.0)
            technical_logging = visual_application._boolean_variables.get(
                "diagnostics.technical_logging"
            )
            if technical_logging is None or technical_logging.get():
                raise RuntimeError(
                    "Technical logging switch is missing or enabled by default"
                )
            technical_logging.set(True)
            visual_application._save_boolean(
                "diagnostics.technical_logging", technical_logging
            )
            if (
                visual_application.settings.get("diagnostics.technical_logging")
                is not True
            ):
                raise RuntimeError("Technical logging switch did not save")
            for update_path in (
                "updates.check_automatically",
                "updates.install_automatically",
            ):
                if update_path not in visual_application._boolean_variables:
                    raise RuntimeError(f"Update switch is missing: {update_path}")
        finally:
            visual_application.shutdown()
        print("WINDOWS_UI_E2E_OK", flush=True)

        # The E2E composes the application directly, so nothing installs the
        # file log: capture the engine's technical events in memory instead.
        recorded: list[str] = []

        class _Capture(logging.Handler):
            def emit(self, record: logging.LogRecord) -> None:
                recorded.append(record.getMessage())

        engine_logger = logging.getLogger("keyswitch.engine")
        engine_logger.setLevel(logging.INFO)
        engine_logger.addHandler(_Capture())

        print("WINDOWS_ENGINE_INIT_START", flush=True)
        application = WindowsApplication(hidden=False, no_engine=False)
        print("WINDOWS_ENGINE_INIT_OK", flush=True)
        scenario_errors: list[Exception] = []
        completed: list[bool] = []
        layout_deadline = [0.0]
        focus_deadline = [0.0]
        learning_deadline = [0.0]
        menu_layout_deadline = [0.0]

        def fail(error: Exception) -> None:
            # A scenario that fails in CI cannot be examined afterwards, so
            # print the visible state and the technical events the engine
            # recorded on the way in.
            print("WINDOWS_E2E_DIAGNOSTICS", flush=True)
            snapshot = application.engine.snapshot
            print(
                f"field={application.test_entry.get()!r} "
                f"group={application.backend.current_group()} "
                f"corrections={snapshot.correction_count} "
                f"action={snapshot.last_action!r} error={snapshot.last_error!r} "
                f"word={snapshot.current_word!r} "
                f"foreground={api.active_application()!r} "
                f"focus={application.root.focus_get()!r}",
                flush=True,
            )
            for line in recorded[-30:]:
                print(line[:600], flush=True)
            print("WINDOWS_E2E_DIAGNOSTICS_END", flush=True)
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

        def send_virtual_key(virtual_key: int) -> None:
            user_inputs = (
                NativeInput(True, virtual_key=virtual_key, synthetic=False),
                NativeInput(False, virtual_key=virtual_key, synthetic=False),
            )
            if api.send_inputs(user_inputs) != len(user_inputs):
                raise RuntimeError(
                    f"SendInput did not accept virtual key {virtual_key:#x}"
                )

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
            application.root.after(300, prepare_learning)

        def finish_learning() -> None:
            pairs = [
                (entry.original, entry.replacement)
                for entry in application.history.read()
            ]
            if pairs[-3:] != [
                ("ghbdtn", "привет"),
                ("руддщ", "hello"),
                ("hello", "руддщ"),
            ]:
                fail(RuntimeError(f"Unexpected learned Windows history: {pairs}"))
                return
            print("WINDOWS_LEARNING_PROMPT_E2E_OK", flush=True)
            application._select_alternate_layout()
            menu_layout_deadline[0] = time.monotonic() + 5.0
            application.root.after(50, wait_for_menu_layout_selection)

        def wait_for_menu_layout_selection() -> None:
            try:
                tray_group = application.tray.state.group if application.tray else -1
                selected = (
                    application.backend.current_group() == 0
                    and application.engine.snapshot.current_group == 0
                    and tray_group == 0
                )
                if not selected:
                    if time.monotonic() >= menu_layout_deadline[0]:
                        raise RuntimeError(
                            "Tray language selection timed out; "
                            f"backend={application.backend.current_group()}, "
                            f"engine={application.engine.snapshot.current_group}, "
                            f"tray={tray_group}"
                        )
                    application.root.after(50, wait_for_menu_layout_selection)
                    return
                print("WINDOWS_MENU_LAYOUT_SELECTION_E2E_OK", flush=True)
                completed.append(True)
                application.shutdown()
            except Exception as error:
                fail(error)

        def type_learned_word() -> None:
            try:
                send_scans((0x23, 0x12, 0x26, 0x26, 0x18, 0x39))
                wait_for_text(
                    "руддщ ", 1, finish_learning, time.monotonic() + 10.0
                )
            except Exception as error:
                fail(error)

        def wait_for_learned_input_focus() -> None:
            try:
                application.present()
                api.activate_window(application.root.winfo_id())
                application.test_entry.focus_force()
                application.root.update_idletasks()
                foreground = api.active_application().casefold()
                expected = Path(sys.executable).stem.casefold()
                ready = (
                    application.backend.current_group() == 0
                    and foreground == expected
                    and application.root.focus_get() is application.test_entry
                )
                if not ready:
                    if time.monotonic() >= learning_deadline[0]:
                        raise RuntimeError(
                            "Learned-rule input preparation timed out; "
                            f"group={application.backend.current_group()}, "
                            f"expected_app={expected!r}, actual_app={foreground!r}, "
                            f"focus={application.root.focus_get()!r}"
                        )
                    application.root.after(50, wait_for_learned_input_focus)
                    return
                type_learned_word()
            except Exception as error:
                fail(error)

        def prepare_learned_input() -> None:
            try:
                application.present()
                api.activate_window(application.root.winfo_id())
                application.test_entry.focus_force()
                application.test_entry.delete(0, "end")
                application.root.update_idletasks()
                if not api.request_layout(application.backend.layouts[0]):
                    raise RuntimeError(
                        "Cannot select English for the learned-rule pass"
                    )
                learning_deadline[0] = time.monotonic() + 5.0
                application.root.after(100, wait_for_learned_input_focus)
            except Exception as error:
                fail(error)

        def wait_for_learning_confirmation() -> None:
            try:
                required = int(
                    application.settings.get(
                        "detection.learning_confirmations", 2
                    )
                )
                confirmed = (
                    application.engine.learning_prompt is None
                    and application.learning_prompt.window.state() == "withdrawn"
                    and application.engine.learning.forced_target(
                        0, "hello", required
                    )
                    == 1
                )
                if not confirmed:
                    if time.monotonic() >= learning_deadline[0]:
                        raise RuntimeError(
                            "Learning confirmation timed out; "
                            f"prompt={application.engine.learning_prompt!r}, "
                            "window_state="
                            f"{application.learning_prompt.window.state()!r}"
                        )
                    application.root.after(50, wait_for_learning_confirmation)
                    return
                if application.root.state() != "zoomed":
                    raise RuntimeError(
                        "Learning prompt changed the maximized target window "
                        f"to {application.root.state()!r}"
                    )
                prepare_learned_input()
            except Exception as error:
                fail(error)

        def wait_for_learning_prompt() -> None:
            try:
                prompt = application.engine.learning_prompt
                popup = application.learning_prompt
                ready = (
                    application.test_entry.get() == "руддщ"
                    and application.backend.current_group() == 1
                    and prompt is not None
                    and popup.prompt == prompt
                    and popup.window.state() == "normal"
                )
                if not ready:
                    if time.monotonic() >= learning_deadline[0]:
                        raise RuntimeError(
                            "Learning prompt timed out; "
                            f"text={application.test_entry.get()!r}, "
                            f"group={application.backend.current_group()}, "
                            f"prompt={prompt!r}, window={popup.window.state()!r}"
                        )
                    application.root.after(50, wait_for_learning_prompt)
                    return
                anchor = popup.anchor
                if anchor is None or anchor.window is None:
                    raise RuntimeError("Learning prompt has no Win32 caret anchor")
                popup.window.update_idletasks()
                if popup.window.winfo_y() + popup.window.winfo_height() > anchor.y:
                    raise RuntimeError(
                        "Learning prompt was not positioned above the caret"
                    )
                send_virtual_key(0x0D)
                learning_deadline[0] = time.monotonic() + 5.0
                application.root.after(50, wait_for_learning_confirmation)
            except Exception as error:
                fail(error)

        def start_learning_input() -> None:
            try:
                send_scans((0x23, 0x12, 0x26, 0x26, 0x18))
                learning_deadline[0] = time.monotonic() + 10.0
                application.root.after(50, wait_for_typed_learning_word)
            except Exception as error:
                fail(error)

        def wait_for_typed_learning_word() -> None:
            try:
                ready = (
                    application.test_entry.get() == "hello"
                    and application.engine.snapshot.current_word == "hello"
                    and application.backend.current_group() == 0
                )
                if not ready:
                    if time.monotonic() >= learning_deadline[0]:
                        raise RuntimeError(
                            "Learning word did not settle before Pause; "
                            f"text={application.test_entry.get()!r}, "
                            "engine_word="
                            f"{application.engine.snapshot.current_word!r}, "
                            f"group={application.backend.current_group()}"
                        )
                    application.root.after(50, wait_for_typed_learning_word)
                    return
                send_virtual_key(0x13)
                learning_deadline[0] = time.monotonic() + 10.0
                application.root.after(50, wait_for_learning_prompt)
            except Exception as error:
                fail(error)

        def wait_for_learning_input_focus() -> None:
            try:
                application.present()
                api.activate_window(application.root.winfo_id())
                application.test_entry.focus_force()
                application.root.update_idletasks()
                foreground = api.active_application().casefold()
                expected = Path(sys.executable).stem.casefold()
                ready = (
                    application.backend.current_group() == 0
                    and foreground == expected
                    and application.root.focus_get() is application.test_entry
                    and application.root.state() == "zoomed"
                )
                if not ready:
                    if time.monotonic() >= learning_deadline[0]:
                        raise RuntimeError(
                            "Learning input preparation timed out; "
                            f"group={application.backend.current_group()}, "
                            f"expected_app={expected!r}, actual_app={foreground!r}, "
                            f"focus={application.root.focus_get()!r}, "
                            f"window_state={application.root.state()!r}"
                        )
                    application.root.after(50, wait_for_learning_input_focus)
                    return
                start_learning_input()
            except Exception as error:
                fail(error)

        def prepare_learning() -> None:
            try:
                application.present()
                api.activate_window(application.root.winfo_id())
                application.root.state("zoomed")
                application.test_entry.focus_force()
                application.test_entry.delete(0, "end")
                application.root.update_idletasks()
                if not api.request_layout(application.backend.layouts[0]):
                    raise RuntimeError("Cannot select English for learning E2E")
                learning_deadline[0] = time.monotonic() + 5.0
                application.root.after(100, wait_for_learning_input_focus)
            except Exception as error:
                fail(error)

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
                send_scans((0x22, 0x23, 0x30, 0x20, 0x14, 0x31))
                wait_for_text("привет", 1, finish_forward, time.monotonic() + 10.0)
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

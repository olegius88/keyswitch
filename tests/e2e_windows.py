"""Real bidirectional Win32 hook/SendInput/Tk correction E2E."""

from __future__ import annotations

import logging
import faulthandler
import io
import os
import sys
import tempfile
import threading
import time
from collections.abc import Callable
from pathlib import Path
from typing import cast


def _running_on_windows() -> bool:
    return sys.platform == "win32"


VK_F24 = 0x87
# A UI or hook step gets this long before the scenario reports it as timed out.
STANDARD_DEADLINE_SECONDS = 5.0
LONG_DEADLINE_SECONDS = 10.0
ENTER_SUBMIT_TIMEOUT_SECONDS = 10
# English flag: near-black top-left corner, red bottom-right stripe.
# Russian flag: white top-left stripe, red bottom-right stripe.
FLAG_PIXEL_EXPECTATIONS = {
    0: ((60, 59, 110, 255), (178, 34, 52, 255)),
    1: ((255, 255, 255, 255), (213, 43, 30, 255)),
}
EXPECTED_SCROLLREGION_FIELD_COUNT = 4
SCROLLREGION_BOTTOM_INDEX = 3
MOUSEWHEEL_DELTA = 120
WHEEL_EVENT_OFFSET = 20
DIAGNOSTIC_LOG_TAIL_LINES = 30
DIAGNOSTIC_LINE_CHARACTERS = 600
VK_HOME = 0x24
VK_SHIFT = 0x10
VK_END = 0x23
VK_BACK = 0x08
# Short retry poll interval, in milliseconds, for the async wait_for_* steps.
POLL_INTERVAL_MS = 50
# Slightly longer delay before starting the next phase of a scenario.
PHASE_START_DELAY_MS = 100
# UI settle delay, in milliseconds, after a phase completes.
PHASE_SETTLE_DELAY_MS = 300
ENTER_PREP_DELAY_MS = 200
# Physical (hardware) scan codes; together they spell "ghbdtn" (-> привет)
# or "hello" (-> руддщ) in the layout under test, plus Enter and Space.
SCAN_G = 0x22
SCAN_H = 0x23
SCAN_B = 0x30
SCAN_D = 0x20
SCAN_T = 0x14
SCAN_N = 0x31
SCAN_ENTER = 0x1C
SCAN_E = 0x12
SCAN_L = 0x26
SCAN_O = 0x18
SCAN_SPACE = 0x39
VK_RETURN = 0x0D
VK_PAUSE = 0x13
MAX_PROBE_ATTEMPTS = 3
PROBE_TIMEOUT_SECONDS = 3.0
HISTORY_TAIL_PAIR_COUNT = 2
HISTORY_TAIL_TRIPLE_COUNT = 3
WATCHDOG_SECONDS = 90


def main() -> int:
    if not _running_on_windows():
        raise RuntimeError("Windows E2E must run on Windows")

    from keyswitch.config import DEFAULT_LEARNING_CONFIRMATIONS, SettingsStore
    from keyswitch.windows_backend import NativeInput, NativeKeyEvent
    from keyswitch.windows_native import CtypesWindowsAPI
    from keyswitch.windows_tray import WindowsTrayState
    from keyswitch.windows_tray_native import ICON_SIZE, PystrayWindowsAdapter
    from keyswitch.windows_backend import WindowsBackend
    from keyswitch.windows_ui import PAGE_NAMES, WindowsApplication, WindowsServices

    api = CtypesWindowsAPI()
    ready = threading.Event()
    received = threading.Event()
    events: list[NativeKeyEvent] = []
    errors: list[Exception] = []

    def listener(event: NativeKeyEvent) -> bool:
        events.append(event)
        if event.virtual_key == VK_F24:
            received.set()
        return False

    def hook_loop() -> None:
        try:
            api.run_keyboard_hook(listener, ready.set)
        except Exception as error:
            errors.append(error)
            ready.set()

    thread = threading.Thread(target=hook_loop, name="keyswitch-win32-e2e")
    thread.start()
    if not ready.wait(STANDARD_DEADLINE_SECONDS):
        raise RuntimeError("WH_KEYBOARD_LL did not become ready")
    if errors:
        raise errors[0]
    try:
        inputs = (
            NativeInput(True, virtual_key=VK_F24),
            NativeInput(False, virtual_key=VK_F24),
        )
        if api.send_inputs(inputs) != len(inputs):
            raise RuntimeError("SendInput did not accept the F24 smoke sequence")
        if not received.wait(STANDARD_DEADLINE_SECONDS):
            raise RuntimeError("WH_KEYBOARD_LL did not observe the injected F24 event")
        if not any(event.injected for event in events if event.virtual_key == VK_F24):
            raise RuntimeError("Injected Win32 event was not marked synthetic")
    finally:
        api.stop_keyboard_hook()
        thread.join(timeout=STANDARD_DEADLINE_SECONDS)
    if thread.is_alive():
        raise RuntimeError("Win32 hook thread did not stop")
    print("WINDOWS_HOOK_E2E_OK", flush=True)

    for group, (top_left, bottom_right) in FLAG_PIXEL_EXPECTATIONS.items():
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
        visual_application = WindowsApplication(WindowsServices(), hidden=False, no_engine=True)
        try:
            for page_name, title in PAGE_NAMES:
                visual_application.show_page(page_name)
                visual_application.root.update()
                page = visual_application._pages[page_name]
                viewport = visual_application._page_viewports[page_name]
                region = str(viewport.cget("scrollregion")).split()
                if len(region) != EXPECTED_SCROLLREGION_FIELD_COUNT:
                    raise RuntimeError(f"Page {page_name} has no scroll region")
                if int(float(region[SCROLLREGION_BOTTOM_INDEX])) < page.winfo_reqheight():
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
                    delta=-MOUSEWHEEL_DELTA,
                    x=WHEEL_EVENT_OFFSET,
                    y=WHEEL_EVENT_OFFSET,
                    rootx=settings_viewport.winfo_rootx() + WHEEL_EVENT_OFFSET,
                    rooty=settings_viewport.winfo_rooty() + WHEEL_EVENT_OFFSET,
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
        application = WindowsApplication(WindowsServices(), hidden=False, no_engine=False)
        print("WINDOWS_ENGINE_INIT_OK", flush=True)
        scenario_errors: list[Exception] = []
        completed: list[bool] = []
        layout_deadline = [0.0]
        focus_deadline = [0.0]
        probe_deadline = [0.0]
        probe_attempts = [0]
        probe_confirmed = [False]
        learning_deadline = [0.0]
        menu_layout_deadline = [0.0]
        entered_messages: list[str] = []
        application.test_entry.bind(
            "<Return>", lambda _event: entered_messages.append(application.test_entry.get()), add=True,
        )

        def fail(error: Exception) -> None:
            # A scenario that fails in CI cannot be examined afterwards, so
            # print the visible state and the technical events the engine
            # recorded on the way in.
            scenario_errors.append(error)
            try:
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
                for line in recorded[-DIAGNOSTIC_LOG_TAIL_LINES:]:
                    print(line[:DIAGNOSTIC_LINE_CHARACTERS], flush=True)
                print("WINDOWS_E2E_DIAGNOSTICS_END", flush=True)
            finally:
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

        def clear_editor() -> None:
            # Real edits invalidate the engine's prefix; directly setting a
            # widget's value cannot be observed by a keyboard-only backend.
            # Tk's Ctrl+A binding is layout-dependent on Windows. Home and
            # Shift+End select the single-line field in both EN and RU.
            inputs = (
                NativeInput(True, virtual_key=VK_HOME, extended=True, synthetic=False),
                NativeInput(False, virtual_key=VK_HOME, extended=True, synthetic=False),
                NativeInput(True, virtual_key=VK_SHIFT, synthetic=False),
                NativeInput(True, virtual_key=VK_END, extended=True, synthetic=False),
                NativeInput(False, virtual_key=VK_END, extended=True, synthetic=False),
                NativeInput(False, virtual_key=VK_SHIFT, synthetic=False),
                NativeInput(True, virtual_key=VK_BACK, synthetic=False),
                NativeInput(False, virtual_key=VK_BACK, synthetic=False),
            )
            if api.send_inputs(inputs) != len(inputs):
                raise RuntimeError("Could not clear the editor through its keyboard")

        def finish_enter_submission() -> None:
            try:
                if entered_messages != ["привет"]:
                    raise RuntimeError(f"Enter submitted an uncorrected or duplicate message: {entered_messages!r}")
                if application.engine.snapshot.current_word:
                    raise RuntimeError("A submitted word is still eligible for correction")
                print("WINDOWS_CORRECT_BEFORE_ENTER_E2E_OK", flush=True)
                completed.append(True)
                application.shutdown()
            except Exception as error:
                fail(error)

        def wait_for_enter_submission(deadline: float) -> None:
            if entered_messages:
                application.root.after(PHASE_SETTLE_DELAY_MS, finish_enter_submission)
            elif time.monotonic() >= deadline:
                fail(RuntimeError("The intercepted Enter never reached the editor"))
            else:
                application.root.after(POLL_INTERVAL_MS, lambda: wait_for_enter_submission(deadline))

        def type_and_submit() -> None:
            try:
                inputs = tuple(
                    NativeInput(pressed, scan_code=scan, synthetic=False)
                    for scan in (SCAN_G, SCAN_H, SCAN_B, SCAN_D, SCAN_T, SCAN_N, SCAN_ENTER)
                    for pressed in (True, False)
                )
                if api.send_inputs(inputs) != len(inputs):
                    raise RuntimeError("The word and Enter were not accepted")
                wait_for_enter_submission(time.monotonic() + ENTER_SUBMIT_TIMEOUT_SECONDS)
            except Exception as error:
                fail(error)

        def prepare_enter_submission() -> None:
            application.settings.set("detection.early_switch", False)
            application.settings.set("detection.correct_on_pause", False)
            application.settings.set("detection.respect_manual_layout", False)
            application.settings.set("detection.correct_on_enter", True)
            application.test_entry.focus_force()
            clear_editor()
            application.root.after(ENTER_PREP_DELAY_MS, type_and_submit)

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
                POLL_INTERVAL_MS,
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
            if pairs[-HISTORY_TAIL_PAIR_COUNT:] != [("ghbdtn", "привет"), ("руддщ", "hello")]:
                fail(RuntimeError(f"Unexpected Windows correction history: {pairs}"))
                return
            print("WINDOWS_RU_TO_EN_E2E_OK", flush=True)
            application.root.after(PHASE_SETTLE_DELAY_MS, prepare_learning)

        def finish_learning() -> None:
            pairs = [
                (entry.original, entry.replacement)
                for entry in application.history.read()
            ]
            if pairs[-HISTORY_TAIL_TRIPLE_COUNT:] != [
                ("ghbdtn", "привет"),
                ("руддщ", "hello"),
                ("hello", "руддщ"),
            ]:
                fail(RuntimeError(f"Unexpected learned Windows history: {pairs}"))
                return
            print("WINDOWS_LEARNING_PROMPT_E2E_OK", flush=True)
            application._select_alternate_layout()
            menu_layout_deadline[0] = time.monotonic() + STANDARD_DEADLINE_SECONDS
            application.root.after(POLL_INTERVAL_MS, wait_for_menu_layout_selection)

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
                    application.root.after(POLL_INTERVAL_MS, wait_for_menu_layout_selection)
                    return
                print("WINDOWS_MENU_LAYOUT_SELECTION_E2E_OK", flush=True)
                prepare_enter_submission()
            except Exception as error:
                fail(error)

        def type_learned_word() -> None:
            try:
                send_scans((SCAN_H, SCAN_E, SCAN_L, SCAN_L, SCAN_O, SCAN_SPACE))
                wait_for_text(
                    "руддщ ", 1, finish_learning, time.monotonic() + LONG_DEADLINE_SECONDS
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
                    application.root.after(POLL_INTERVAL_MS, wait_for_learned_input_focus)
                    return
                type_learned_word()
            except Exception as error:
                fail(error)

        def prepare_learned_input() -> None:
            try:
                application.present()
                api.activate_window(application.root.winfo_id())
                application.test_entry.focus_force()
                clear_editor()
                application.root.update_idletasks()
                if not api.request_layout(cast(WindowsBackend, application.backend).layouts[0]):
                    raise RuntimeError(
                        "Cannot select English for the learned-rule pass"
                    )
                learning_deadline[0] = time.monotonic() + STANDARD_DEADLINE_SECONDS
                application.root.after(PHASE_START_DELAY_MS, wait_for_learned_input_focus)
            except Exception as error:
                fail(error)

        def wait_for_learning_confirmation() -> None:
            try:
                required = int(
                    application.settings.get(
                        "detection.learning_confirmations", DEFAULT_LEARNING_CONFIRMATIONS
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
                    application.root.after(POLL_INTERVAL_MS, wait_for_learning_confirmation)
                    return
                if application.root.state() != "zoomed":
                    raise RuntimeError(
                        "Learning prompt changed the maximized target window "
                        f"to {application.root.state()!r}"
                    )
                if entered_messages:
                    raise RuntimeError(f"Prompt Enter leaked to the editor: {entered_messages!r}")
                if application.test_entry.get() != "руддщ":
                    raise RuntimeError("Prompt confirmation changed the editor text")
                print("WINDOWS_PROMPT_ENTER_NO_LEAK_E2E_OK", flush=True)
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
                    application.root.after(POLL_INTERVAL_MS, wait_for_learning_prompt)
                    return
                anchor = popup.anchor
                if anchor is None or anchor.window is None:
                    raise RuntimeError("Learning prompt has no Win32 caret anchor")
                popup.window.update_idletasks()
                if popup.window.winfo_y() + popup.window.winfo_height() > anchor.y:
                    raise RuntimeError(
                        "Learning prompt was not positioned above the caret"
                    )
                send_virtual_key(VK_RETURN)
                learning_deadline[0] = time.monotonic() + STANDARD_DEADLINE_SECONDS
                application.root.after(POLL_INTERVAL_MS, wait_for_learning_confirmation)
            except Exception as error:
                fail(error)

        def start_learning_input() -> None:
            try:
                send_scans((SCAN_H, SCAN_E, SCAN_L, SCAN_L, SCAN_O))
                learning_deadline[0] = time.monotonic() + LONG_DEADLINE_SECONDS
                application.root.after(POLL_INTERVAL_MS, wait_for_typed_learning_word)
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
                    application.root.after(POLL_INTERVAL_MS, wait_for_typed_learning_word)
                    return
                send_virtual_key(VK_PAUSE)
                learning_deadline[0] = time.monotonic() + LONG_DEADLINE_SECONDS
                application.root.after(POLL_INTERVAL_MS, wait_for_learning_prompt)
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
                    application.root.after(POLL_INTERVAL_MS, wait_for_learning_input_focus)
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
                clear_editor()
                application.root.update_idletasks()
                if not api.request_layout(cast(WindowsBackend, application.backend).layouts[0]):
                    raise RuntimeError("Cannot select English for learning E2E")
                learning_deadline[0] = time.monotonic() + STANDARD_DEADLINE_SECONDS
                application.root.after(PHASE_START_DELAY_MS, wait_for_learning_input_focus)
            except Exception as error:
                fail(error)

        def start_reverse() -> None:
            try:
                send_scans((SCAN_H, SCAN_E, SCAN_L, SCAN_L, SCAN_O, SCAN_SPACE))
                wait_for_text("hello ", 0, finish_reverse, time.monotonic() + LONG_DEADLINE_SECONDS)
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
                    application.root.after(POLL_INTERVAL_MS, wait_for_russian_layout)
                    return
                start_reverse()
            except Exception as error:
                fail(error)

        def prepare_reverse() -> None:
            try:
                application.present()
                api.activate_window(application.root.winfo_id())
                application.test_entry.focus_force()
                clear_editor()
                layout_deadline[0] = time.monotonic() + STANDARD_DEADLINE_SECONDS
                application.root.after(POLL_INTERVAL_MS, wait_for_cleared_reverse_field)
            except Exception as error:
                fail(error)

        def wait_for_cleared_reverse_field() -> None:
            try:
                if application.test_entry.get():
                    if time.monotonic() >= layout_deadline[0]:
                        raise RuntimeError("KeySwitch E2E field did not clear")
                    application.root.after(POLL_INTERVAL_MS, wait_for_cleared_reverse_field)
                    return
                if api.active_application().casefold() != Path(sys.executable).stem.casefold():
                    raise RuntimeError("KeySwitch E2E lost foreground before RU to EN pass")
                if not api.request_layout(cast(WindowsBackend, application.backend).layouts[1]):
                    raise RuntimeError("Cannot select the Russian layout for E2E")
                layout_deadline[0] = time.monotonic() + STANDARD_DEADLINE_SECONDS
                application.root.after(PHASE_START_DELAY_MS, wait_for_russian_layout)
            except Exception as error:
                fail(error)

        def finish_forward() -> None:
            print("WINDOWS_EN_TO_RU_E2E_OK", flush=True)
            application.root.after(PHASE_SETTLE_DELAY_MS, prepare_reverse)

        def start_forward() -> None:
            try:
                application.present()
                if application.tray is None:
                    raise RuntimeError("Native Windows notification-area icon is missing")
                application.tray.set_indicator_style("flags")
                focus_deadline[0] = time.monotonic() + STANDARD_DEADLINE_SECONDS
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
                    application.root.after(POLL_INTERVAL_MS, wait_for_test_focus)
                    return
                if application.root.focus_get() is not application.test_entry:
                    if time.monotonic() >= focus_deadline[0]:
                        raise RuntimeError("KeySwitch E2E entry did not receive focus")
                    application.root.after(POLL_INTERVAL_MS, wait_for_test_focus)
                    return
                if not probe_confirmed[0]:
                    # Foreground and Tk focus can both hold while injected keys
                    # still go nowhere. Prove the field really receives input
                    # before the scenario starts depending on it: a lone space
                    # leaves the engine's word buffer empty.
                    clear_editor()
                    send_scans((SCAN_SPACE,))
                    probe_deadline[0] = time.monotonic() + PROBE_TIMEOUT_SECONDS
                    application.root.after(POLL_INTERVAL_MS, wait_for_input_probe)
                    return
                if not api.request_layout(cast(WindowsBackend, application.backend).layouts[0]):
                    raise RuntimeError("Cannot select the English layout for E2E")
                layout_deadline[0] = time.monotonic() + STANDARD_DEADLINE_SECONDS
                application.root.after(PHASE_START_DELAY_MS, wait_for_english_layout)
            except Exception as error:
                fail(error)

        def wait_for_input_probe() -> None:
            try:
                if application.test_entry.get() == " ":
                    probe_confirmed[0] = True
                    clear_editor()
                    application.root.update_idletasks()
                    focus_deadline[0] = time.monotonic() + STANDARD_DEADLINE_SECONDS
                    wait_for_test_focus()
                    return
                if time.monotonic() < probe_deadline[0]:
                    application.root.after(POLL_INTERVAL_MS, wait_for_input_probe)
                    return
                if probe_attempts[0] >= MAX_PROBE_ATTEMPTS:
                    raise RuntimeError(
                        "Injected input never reached the KeySwitch E2E field; "
                        f"field={application.test_entry.get()!r}, "
                        f"foreground={api.active_application()!r}"
                    )
                # Reclaim the foreground and try once more: the window can be
                # activated and focused and still not be the input target yet.
                probe_attempts[0] += 1
                print("WINDOWS_E2E_INPUT_RETRY", flush=True)
                focus_deadline[0] = time.monotonic() + STANDARD_DEADLINE_SECONDS
                application.root.after(PHASE_START_DELAY_MS, wait_for_test_focus)
            except Exception as error:
                fail(error)

        def wait_for_english_layout() -> None:
            try:
                if application.backend.current_group() != 0:
                    if time.monotonic() >= layout_deadline[0]:
                        raise RuntimeError("English layout selection timed out")
                    application.root.after(POLL_INTERVAL_MS, wait_for_english_layout)
                    return
                send_scans((SCAN_G, SCAN_H, SCAN_B, SCAN_D, SCAN_T, SCAN_N))
                wait_for_text("привет", 1, finish_forward, time.monotonic() + LONG_DEADLINE_SECONDS)
            except Exception as error:
                fail(error)

        application.root.after(PHASE_SETTLE_DELAY_MS, start_forward)
        if application.run() != 0:
            raise RuntimeError("Tk Windows UI smoke test returned a failure")
        if scenario_errors:
            raise scenario_errors[0]
        if not completed:
            raise RuntimeError("Windows correction scenario did not complete")

    print("WINDOWS_E2E_OK")
    return 0


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        if isinstance(stream, io.TextIOWrapper):
            stream.reconfigure(encoding="utf-8", errors="backslashreplace")
    faulthandler.dump_traceback_later(WATCHDOG_SECONDS, exit=True)
    try:
        raise SystemExit(main())
    finally:
        faulthandler.cancel_dump_traceback_later()

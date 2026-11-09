//! Win32 API numbers, Windows-specific limits and the timing of this crate, each value once, as in the Python's
//! `constants` folder (`windows.py`, and from `timing.py`, `text.py`, `units.py`, `macos.py` what these modules use).

// ------------------------------------------------------------------------------------------------ constants/timing.py

/// How long the application has to confirm a layout switch. A busy application runs the request late: VS Code Insiders
/// confirmed one about a second after it was posted, when the correction had already been given up after half a second
/// and the switch that arrived was taken for the user's own.
pub const WINDOWS_LAYOUT_SWITCH_TIMEOUT_SECONDS: f64 = 1.5;
/// Step at which a native backend polls for the layout it asked the operating system to switch to.
pub const LAYOUT_SWITCH_POLL_SECONDS: f64 = 0.01;
/// Windows 11 confirms the layout before its text service has it and loses keys sent at once: `клавиатура ` typed
/// right after the switch came out as `кла`, and whole after a Backspace burst and 30 ms, or after 100 ms. The deletion
/// needs no layout and goes before the pause.
pub const WINDOWS_LAYOUT_SETTLE_SECONDS: f64 = 0.05;
/// How long a layout chosen in the tray menu waits for the window the user worked in to come back to the front with its
/// keyboard focus, before it is switched wherever the front is.
pub const WORK_WINDOW_RETURN_TIMEOUT_SECONDS: f64 = 1.0;
/// A modifier the Windows hook saw go down is checked against the keyboard when the next key comes this many
/// milliseconds later or in another window: its release may have gone where the hook does not reach (a UAC prompt, the
/// Ctrl+Alt+Del screen, an elevated window).
pub const MODIFIER_RECHECK_GAP_MS: i64 = 1000;
/// How long `start()` waits for the keyboard hook to confirm it runs.
pub const KEYBOARD_LISTENER_START_TIMEOUT_SECONDS: f64 = 5.0;
/// How long `stop()` waits for the keyboard hook's thread to finish.
pub const KEYBOARD_LISTENER_STOP_TIMEOUT_SECONDS: f64 = 2.0;
/// Milliseconds UI Automation may take to reach a provider and to finish one request.
pub const UIA_CONNECTION_TIMEOUT_MS: u32 = 200;
pub const UIA_TRANSACTION_TIMEOUT_MS: u32 = 200;
/// How long an input time no hook got may wait before it counts as passing them with no later hook event either: a hook
/// thread kept from the processor got its keys at most 4.8 s late in VM runs under CPU load, while a program in front
/// that reads the keyboard as raw input and has a keyboard hook of its own keeps every keyboard hook silent for good.
pub const HOOK_SILENCE_MS: i64 = 10_000;
/// How many input times `GetLastInputInfo` must have moved through for input to count as kept from the hooks. A single
/// such input followed by events the hooks got is no program keeping keys away: inputs no hook got also came in Explorer,
/// Task Manager and `KeySwitch`'s own window, in the middle of words the hook saw typed.
pub const UNSEEN_INPUT_MIN_TIMES: usize = 3;
/// The selection hotkeys copy the selection where UI Automation cannot read it: the clipboard has this long to change,
/// to open for reading and to take the user's own data back. A copy that has not arrived by then is taken for no
/// selection, and the clipboard is left as it was.
pub const SELECTION_CLIPBOARD_TIMEOUT_SECONDS: f64 = 1.0;
/// Step at which the clipboard is looked at again while it has not changed or another program holds it open.
pub const SELECTION_CLIPBOARD_POLL_SECONDS: f64 = 0.02;
/// A selection hotkey's result pasted on Windows (`SLOW_INPUT_FIELD_CLASSES)`: the field takes the text when it handles
/// Ctrl+V, and the user's clipboard comes back once UI Automation shows the result selected, or after this long.
pub const SELECTION_PASTE_CONFIRM_SECONDS: f64 = 1.0;

// -------------------------------------------------------------------------------------------------- constants/text.py

/// Characters a platform field reader reads after the caret; the before side is bounded by
/// `FIELD_CONTEXT_MAX_CHARACTERS`, a different limit.
pub const FIELD_AFTER_CARET_MAX_CHARACTERS: i32 = 128;
/// The longest selection a selection hotkey changes (layout, case, transliteration). The replacement is typed key by
/// key, so the bound keeps one press of the hotkey within a moment of typing, and it bounds the read too: UI Automation
/// and the clipboard hand over no more than one character past it.
pub const SELECTION_COMMAND_MAX_CHARACTERS: usize = 1000;
/// The most bytes of the user's clipboard a selection hotkey keeps to put back after borrowing it; a bigger clipboard
/// is not borrowed.
pub const CLIPBOARD_SNAPSHOT_MAX_BYTES: usize = 64 * BYTES_PER_MEBIBYTE;

// ------------------------------------------------------------------------------------------------- constants/units.py

pub const MILLISECONDS_PER_SECOND: f64 = 1000.0;
pub const BYTES_PER_MEBIBYTE: usize = 1024 * 1024;

// ------------------------------------------------------------------------------------------------- constants/macos.py

/// A UTF-16 code unit is two bytes.
pub const UTF16_CODE_UNIT_BYTES: usize = 2;

// ----------------------------------------------------------------------------------------------- constants/windows.py

/// Win32 primary language ids of the two supported layouts.
pub const LANG_ENGLISH: i64 = 0x09;
pub const LANG_RUSSIAN: i64 = 0x19;
/// LOWORD(hkl) & PRIMARYLANGID(lgid), the Win32 macros `primary_language` inlines.
pub const LOWORD_MASK: i64 = 0xFFFF;
pub const PRIMARY_LANGID_MASK: i64 = 0x03FF;
/// The low 32 bits of an HKL, for display, and of a tick count; HKL is pointer-sized but only the low word carries the
/// language id and the high word the layout id.
pub const DWORD_MASK: i64 = 0xFFFF_FFFF;
/// The key and pointer event times the hook thread keeps for the check of input it never got (`take_unseen_input)`: about
/// four seconds of a mouse reporting a thousand times a second, eight checks of the engine's timer.
pub const HOOK_INPUT_TIMES_KEPT: usize = 4096;
pub const VK_BACK: u32 = 0x08;
pub const VK_TAB: u32 = 0x09;
pub const VK_RETURN: u32 = 0x0D;
pub const VK_SHIFT: u32 = 0x10;
pub const VK_CONTROL: u32 = 0x11;
pub const VK_MENU: u32 = 0x12;
pub const VK_PAUSE: u32 = 0x13;
pub const VK_CAPITAL: u32 = 0x14;
pub const VK_ESCAPE: u32 = 0x1B;
pub const VK_SPACE: u32 = 0x20;
pub const VK_PRIOR: u32 = 0x21;
pub const VK_NEXT: u32 = 0x22;
pub const VK_END: u32 = 0x23;
pub const VK_HOME: u32 = 0x24;
pub const VK_LEFT: u32 = 0x25;
pub const VK_UP: u32 = 0x26;
pub const VK_RIGHT: u32 = 0x27;
pub const VK_DOWN: u32 = 0x28;
pub const VK_INSERT: u32 = 0x2D;
pub const VK_DELETE: u32 = 0x2E;
pub const VK_LWIN: u32 = 0x5B;
pub const VK_RWIN: u32 = 0x5C;
pub const VK_LSHIFT: u32 = 0xA0;
pub const VK_RSHIFT: u32 = 0xA1;
pub const VK_LCONTROL: u32 = 0xA2;
pub const VK_RCONTROL: u32 = 0xA3;
pub const VK_LMENU: u32 = 0xA4;
pub const VK_RMENU: u32 = 0xA5;
// Punctuation keys of the US layout; named after the X11 keysyms so the log reads the same on both platforms instead of
// showing "VK_BC" for a comma.
pub const VK_OEM_1: u32 = 0xBA;
pub const VK_OEM_PLUS: u32 = 0xBB;
pub const VK_OEM_COMMA: u32 = 0xBC;
pub const VK_OEM_MINUS: u32 = 0xBD;
pub const VK_OEM_PERIOD: u32 = 0xBE;
pub const VK_OEM_2: u32 = 0xBF;
pub const VK_OEM_3: u32 = 0xC0;
pub const VK_OEM_4: u32 = 0xDB;
pub const VK_OEM_5: u32 = 0xDC;
pub const VK_OEM_6: u32 = 0xDD;
pub const VK_OEM_7: u32 = 0xDE;
/// A key event that carries a Unicode character in its scan code field (`SendInput` with `KEYEVENTF_UNICODE)`: remote
/// control and on-screen keyboards type with it.
pub const VK_PACKET: u32 = 0xE7;
/// A virtual key that types nothing ("Unassigned", Virtual-Key Codes, Microsoft Learn), pressed while Alt or Win is held
/// under a hotkey key `KeySwitch` keeps from the window: the window then sees a key between that modifier's press and its
/// release, which would otherwise open its menu bar (the Start menu for Win). `AutoHotkey` calls such a key the menu mask
/// key (`A_MenuMaskKey`) and names vkE8 as a good choice.
pub const VK_MENU_MASK: u32 = 0xE8;
// Alphanumeric keys: winuser.h defines no VK_0.. VK_9 / VK_A.. VK_Z constants because their values equal the ASCII
// digits and upper-case letters.
pub const VK_0: u32 = 0x30;
pub const VK_9: u32 = 0x39;
pub const VK_A: u32 = 0x41;
pub const VK_V: u32 = 0x56;
pub const VK_Z: u32 = 0x5A;
/// Scroll Lock, whose name a hotkey can use (Punto Switcher transliterates the selection with Alt+Scroll Lock).
pub const VK_SCROLL: u32 = 0x91;
/// UI Automation `TextPattern` id. `GetCurrentPattern` answers `S_OK` with a null pointer when the element does not support
/// it: a field without text is an ordinary answer.
pub const UIA_TEXT_PATTERN_ID: i32 = 10014;
/// `TextPatternRangeEndpoint_Start` and _End, and `TextUnit_Character` (UIAutomationClient.h).
pub const ENDPOINT_START: i32 = 0;
pub const ENDPOINT_END: i32 = 1;
pub const UNIT_CHARACTER: i32 = 0;
/// `CoCreateInstance`'s context in comtypes: an in-process or a local server.
pub const CLSCTX_SERVER: u32 = 5;
/// What a browser's text range reports in place of an embedded object, and as the whole text of an empty input (Edge):
/// a placeholder, never typed text.
pub const UIA_EMBEDDED_OBJECT_CHARACTER: char = '\u{FFFC}';
/// `UIA_IsReadOnlyAttributeId` (`VT_BOOL`), asked of a selection before a selection hotkey types over it: text that says it
/// is read-only is left alone (Text Attribute Identifiers, UIAutomationClient.h).
pub const UIA_IS_READ_ONLY_ATTRIBUTE_ID: i32 = 40015;
/// `UIA_EditControlTypeId` (Control Type Identifiers, UIAutomationClient.h): a focused element of this type is a field the
/// user types into, which lets the selection hotkeys copy its selection where it has no text pattern.
pub const UIA_EDIT_CONTROL_TYPE_ID: i32 = 50004;
/// The window classes of the console host and of Windows Terminal (`CONSOLE_WINDOW_CLASS` in
/// src/interactivity/win32/window.cpp and `XAML_HOSTING_WINDOW_CLASS_NAME` in src/cascadia/WindowsTerminal/IslandWindow.cpp
/// of microsoft/terminal). Typing does not replace text selected in a console, and Ctrl+C there stops the running
/// program when nothing is selected, so the selection hotkeys stay out of them.
pub const CONSOLE_WINDOW_CLASSES: [&str; 2] = ["ConsoleWindowClass", "CASCADIA_HOSTING_WINDOW_CLASS"];
/// The field of Notepad of Windows 11. Its text service takes keys sent in one burst asynchronously and loses some when
/// they replace a selection: `привет мир` typed over one came out as `привет `, and pasted it was whole. A selection
/// hotkey pastes its result there (`WindowsBackend::pasted_selection`).
pub const SLOW_INPUT_FIELD_CLASSES: [&str; 1] = ["RichEditD2DPT"];
/// The top-level window of Chrome, Edge and every Electron program. Chromium builds its accessibility tree the first time
/// a program asks; until it is there, UI Automation answers for the window and not for the field in it, so the first
/// selection hotkey in such a window found no field and the next one worked.
pub const CHROMIUM_WINDOW_CLASS: &str = "Chrome_WidgetWin_1";
/// How long a selection hotkey waits for that tree, and how often it asks.
pub const SELECTION_ACCESSIBILITY_WAIT_SECONDS: f64 = 1.5;
pub const SELECTION_ACCESSIBILITY_POLL_SECONDS: f64 = 0.1;
/// The shell's taskbars, on the main monitor and on the others, and the flyouts of the hidden notification icons of
/// Windows 10 and of Windows 11 22H2 and later (`IsSystemClass` in src/modules/GrabAndMove/GrabAndMove/main.cpp of
/// microsoft/PowerToys). A click on the tray icon brings them to the front, and what the user types in is not there: a
/// layout chosen in the tray menu goes past them to the window the user worked in.
pub const SHELL_TRAY_WINDOW_CLASSES: [&str; 4] =
    ["Shell_TrayWnd", "Shell_SecondaryTrayWnd", "NotifyIconOverflowWindow", "TopLevelWindowForOverflowXamlIsland"];
/// "The maximum length for lpszClassName is 256" (WNDCLASSW): `GetClassNameW` needs no longer buffer than this and its
/// terminating zero.
pub const WINDOW_CLASS_NAME_MAX_CHARACTERS: usize = 256;
/// Clipboard formats (Standard Clipboard Formats, winuser.h). `CF_UNICODETEXT` is what a selection copies as.
pub const CF_UNICODETEXT: u32 = 13;
pub const CF_BITMAP: u32 = 2;
pub const CF_METAFILEPICT: u32 = 3;
pub const CF_PALETTE: u32 = 9;
pub const CF_DIB: u32 = 8;
pub const CF_ENHMETAFILE: u32 = 14;
pub const CF_DIBV5: u32 = 17;
pub const CF_OWNERDISPLAY: u32 = 0x0080;
pub const CF_DSPBITMAP: u32 = 0x0082;
pub const CF_DSPMETAFILEPICT: u32 = 0x0083;
pub const CF_DSPENHMETAFILE: u32 = 0x008E;
pub const CF_PRIVATEFIRST: u32 = 0x0200;
pub const CF_PRIVATELAST: u32 = 0x02FF;
/// Formats whose data is a GDI object or a handle the owner frees, not a block of memory: their bytes cannot be kept and
/// put back. A bitmap and its palette come back by themselves from a DIB kept beside them (Synthesized Clipboard
/// Formats); anything else of this kind makes the clipboard one the selection hotkeys do not borrow.
pub const CLIPBOARD_HANDLE_FORMATS: [u32; 8] = [
    CF_BITMAP,
    CF_METAFILEPICT,
    CF_PALETTE,
    CF_ENHMETAFILE,
    CF_OWNERDISPLAY,
    CF_DSPBITMAP,
    CF_DSPMETAFILEPICT,
    CF_DSPENHMETAFILE,
];
pub const CLIPBOARD_FORMATS_SYNTHESIZED_FROM_DIB: [u32; 2] = [CF_BITMAP, CF_PALETTE];
pub const CLIPBOARD_DIB_FORMATS: [u32; 2] = [CF_DIB, CF_DIBV5];
/// The registered format whose DWORD zero keeps a clipboard item out of the clipboard history: the user's own data put
/// back after a copy is no new item (Cloud Clipboard and Clipboard History Formats).
pub const CLIPBOARD_HISTORY_FORMAT_NAME: &str = "CanIncludeInClipboardHistory";
/// `GlobalAlloc` flag the clipboard requires of the memory `SetClipboardData` takes.
pub const GMEM_MOVEABLE: u32 = 0x0002;
pub const ERROR_ALREADY_EXISTS: u32 = 183;
pub const SW_RESTORE: i32 = 9;
pub const WH_KEYBOARD_LL: i32 = 13;
pub const WH_MOUSE_LL: i32 = 14;
pub const HC_ACTION: i32 = 0;
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_KEYUP: u32 = 0x0101;
pub const WM_SYSKEYDOWN: u32 = 0x0104;
pub const WM_SYSKEYUP: u32 = 0x0105;
pub const WM_QUIT: u32 = 0x0012;
pub const WM_USER: u32 = 0x0400;
/// The first message number a program keeps for its own windows (`WM_APP`, winuser.h).
pub const WM_APP: u32 = 0x8000;
/// What the shell sends the settings window for a mouse event on the tray icon (the icon's callback message); the live
/// test sends it as the shell does.
pub const WM_TRAY: u32 = WM_APP + 2;
pub const WM_INPUTLANGCHANGEREQUEST: u32 = 0x0050;
pub const PM_NOREMOVE: u32 = 0x0000;
// Mouse messages; a button press invalidates the caret position. Observed only, never suppressed (see
// run_keyboard_hook's mouse_callback).
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_RBUTTONDOWN: u32 = 0x0204;
pub const WM_MBUTTONDOWN: u32 = 0x0207;
pub const WM_XBUTTONDOWN: u32 = 0x020B;
pub const WM_MOUSEWHEEL: u32 = 0x020A;
pub const WM_MOUSEHWHEEL: u32 = 0x020E;
pub const LLKHF_EXTENDED: u32 = 0x01;
pub const LLKHF_INJECTED: u32 = 0x10;
pub const KEYEVENTF_EXTENDEDKEY: u32 = 0x0001;
pub const KEYEVENTF_KEYUP: u32 = 0x0002;
pub const KEYEVENTF_UNICODE: u32 = 0x0004;
pub const KEYEVENTF_SCANCODE: u32 = 0x0008;
pub const INPUT_KEYBOARD: u32 = 1;
pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
// dwExtraInfo marks on the keys KeySwitch sends itself (synthetic) and replays; a replay of a key another program
// injected carries its own mark, so it is still known as injected.
pub const KEYSWITCH_EXTRA_INFO: usize = 0x4B53_5743;
pub const KEYSWITCH_REPLAY_INFO: usize = 0x4B53_5752;
pub const KEYSWITCH_FOREIGN_REPLAY_INFO: usize = 0x4B53_5746;
pub const GA_ROOT: u32 = 2;
pub const GWL_EXSTYLE: i32 = -20;
pub const WS_EX_TOOLWINDOW: i32 = 0x0000_0080;
pub const WS_EX_NOACTIVATE: i32 = 0x0800_0000;
pub const SWP_NOSIZE: u32 = 0x0001;
pub const SWP_NOMOVE: u32 = 0x0002;
pub const SWP_NOZORDER: u32 = 0x0004;
pub const SWP_NOACTIVATE: u32 = 0x0010;
pub const SWP_FRAMECHANGED: u32 = 0x0020;
/// A pointer to a 256-byte array holding one keyboard-state byte per virtual key (`ToUnicodeEx`'s lpKeyState /
/// `GetKeyboardState`). High bit set means down.
pub const KEYBOARD_STATE_ARRAY_SIZE: usize = 256;
pub const VK_STATE_DOWN_BIT: u8 = 0x80;
/// `GetAsyncKeyState` sets the most significant bit of its SHORT while the key is down.
pub const ASYNC_KEY_DOWN_BIT: i32 = 0x8000;
/// `SetThreadPriority`: one step above the normal priority of the process.
pub const THREAD_PRIORITY_ABOVE_NORMAL: i32 = 1;
pub const VIRTUAL_KEY_BYTE_MASK: u32 = 0xFF;
pub const TRANSLATED_TEXT_BUFFER_CHARACTERS: usize = 8;
/// `ToUnicodeEx` wFlags bit 2 (Windows 10 1607+): keyboard state is not changed.
pub const TO_UNICODE_KEEP_KEYBOARD_STATE_FLAG: u32 = 0x04;
/// `QueryFullProcessImageNameW`'s output buffer, in wide characters.
pub const PROCESS_IMAGE_NAME_BUFFER_CHARACTERS: usize = 32768;
/// StartupApproved\Run's first byte: 0x02 and 0x06 mean enabled, anything else (0x03 from Task Manager, 0x01 from older
/// builds) means the value is skipped.
pub const STARTUP_APPROVAL_ENABLED_BYTES: [u8; 2] = [0x02, 0x06];
pub const SHIFT_KEYS: [u32; 3] = [VK_SHIFT, VK_LSHIFT, VK_RSHIFT];
pub const CONTROL_KEYS: [u32; 3] = [VK_CONTROL, VK_LCONTROL, VK_RCONTROL];
pub const ALT_KEYS: [u32; 3] = [VK_MENU, VK_LMENU, VK_RMENU];
pub const SUPER_KEYS: [u32; 2] = [VK_LWIN, VK_RWIN];
/// `SHIFT_KEYS`, `CONTROL_KEYS`, `ALT_KEYS` and `SUPER_KEYS` together, in ascending order: the log names forgotten modifiers
/// in this order.
pub const MODIFIER_KEYS: [u32; 11] = [
    VK_SHIFT,
    VK_CONTROL,
    VK_MENU,
    VK_LWIN,
    VK_RWIN,
    VK_LSHIFT,
    VK_RSHIFT,
    VK_LCONTROL,
    VK_RCONTROL,
    VK_LMENU,
    VK_RMENU,
];
/// Not the wheel: a scroll moves the view, not the caret. Reported as a click it discarded the word being typed and the
/// words before it, 141 times a minute while a page was scrolled.
pub const POINTER_INVALIDATING_MESSAGES: [u32; 4] = [WM_LBUTTONDOWN, WM_RBUTTONDOWN, WM_MBUTTONDOWN, WM_XBUTTONDOWN];
/// The wheel, both ways: reported only as a turn, which ends a lone press of the layout switch key (Ctrl+wheel zooms,
/// Shift+wheel scrolls sideways).
pub const WHEEL_MESSAGES: [u32; 2] = [WM_MOUSEWHEEL, WM_MOUSEHWHEEL];

// ---------------------------------------------------------------------------- windows_registry.py, windows_instance_native.py

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Task Manager's Startup tab and Settings -> Apps -> Startup keep one approval value per Run entry here; a value marked
/// disabled is skipped at logon however often Run is rewritten.
pub const STARTUP_APPROVED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
pub const APP_PATHS_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\App Paths";
pub const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
/// The user's light or dark mode for apps, where the darkdetect library reads it.
pub const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
pub const APPS_USE_LIGHT_THEME_VALUE: &str = "AppsUseLightTheme";
/// "Key name: 255 characters" (Registry element size limits, Microsoft Learn), and the terminating zero.
pub const REGISTRY_KEY_NAME_BUFFER_CHARACTERS: usize = 256;
/// The first read of a value; a longer one is asked again at the size the system names.
pub const REGISTRY_VALUE_FIRST_BUFFER_BYTES: usize = 512;
pub const AUTOSTART_VALUE_NAME: &str = "KeySwitch";
pub const MUTEX_NAME: &str = r"Local\io.github.olegius88.KeySwitch";
/// The private window class of the settings window. A second start finds the running program by this class: the title
/// `KeySwitch` is shared with the learning prompt, which must not be the window brought forward.
pub const SETTINGS_WINDOW_CLASS: &str = "KeySwitchSettingsWindow";

// -------------------------------------------------------------------------- windows_app.py, desktop_ui.py, tray (this crate)

/// `--smoke-ui` closes the window this long after it is shown (constants/timing.py).
pub const SMOKE_UI_QUIT_AFTER_MS: u32 = 300;
/// How long quitting waits for the engine to stop before the process ends anyway: a loader or hook thread that does not
/// answer must never keep the program from exiting (the installer check of 0.66.0 hung at exit on Windows on Arm).
pub const ENGINE_STOP_WAIT_SECONDS: f64 = 5.0;
/// The window the tray icon belongs to, which is also the settings window until the real pages exist.
pub const SETTINGS_WINDOW_CLIENT_WIDTH_PIXELS: i32 = 560;
pub const SETTINGS_WINDOW_CLIENT_HEIGHT_PIXELS: i32 = 260;
pub const SETTINGS_WINDOW_MARGIN_PIXELS: i32 = 16;
pub const SETTINGS_BUTTON_WIDTH_PIXELS: i32 = 220;
pub const SETTINGS_BUTTON_HEIGHT_PIXELS: i32 = 30;
pub const SETTINGS_FONT_HEIGHT_PIXELS: i32 = 15;
pub const SETTINGS_OPEN_FOLDER_LABEL: &str = "Открыть папку настроек";
/// The unit of the device's DPI at which sizes are written.
pub const BASE_DPI: i32 = 96;
pub const BACKEND_LABEL: &str = "Win32 hook + SendInput";

// ----------------------------------------------------------------------------------------- constants/ui_desktop.py
// Pixels are at 96 dpi; the window scales them to the monitor's dpi.

pub const PAGE_SCROLL_STEP_PIXELS: i32 = 20;
pub const WHEEL_STEPS_PER_NOTCH: i32 = 3;
pub const WHEEL_DELTA: i32 = 120;
pub const MODIFIED_MARKER_WIDTH_PIXELS: i32 = 3;
pub const MODIFIED_MARKER_COLUMN_PIXELS: i32 = 12;
pub const RESET_BUTTON_COLUMN_PIXELS: i32 = 54;
pub const DESCRIPTION_INDENT_PIXELS: i32 = 24;
pub const PAGE_PADDING_X_PIXELS: i32 = 30;
pub const PAGE_CONTENT_PADDING_Y_PIXELS: i32 = 24;
pub const PAGE_CAPTION_PADDING_TOP_PIXELS: i32 = 3;
pub const PAGE_CAPTION_PADDING_BOTTOM_PIXELS: i32 = 18;
pub const PAGE_TITLE_FONT_SIZE_POINTS: i32 = 22;
pub const CARD_TITLE_FONT_SIZE_POINTS: i32 = 10;
pub const BRAND_LABEL_FONT_SIZE_POINTS: i32 = 20;
pub const LABELFRAME_TITLE_FONT_SIZE_POINTS: i32 = 11;
pub const RESET_BUTTON_FONT_SIZE_POINTS: i32 = 8;
pub const TEST_ENTRY_FONT_SIZE_POINTS: i32 = 14;
pub const BASE_FONT_SIZE_POINTS: i32 = 10;
pub const SETTING_ROW_PADDING_PIXELS: i32 = 5;
pub const SETTING_CONTROL_PADDING_LEFT_PIXELS: i32 = 16;
pub const CHOICE_CONTROL_WIDTH_PIXELS: i32 = 170;
pub const NUMBER_CONTROL_WIDTH_PIXELS: i32 = 90;
pub const TEXT_CONTROL_WIDTH_PIXELS: i32 = 190;
pub const MAIN_WINDOW_WIDTH_PIXELS: i32 = 1080;
pub const MAIN_WINDOW_HEIGHT_PIXELS: i32 = 760;
pub const MAIN_WINDOW_MIN_WIDTH_PIXELS: i32 = 900;
pub const MAIN_WINDOW_MIN_HEIGHT_PIXELS: i32 = 640;
pub const DESKTOP_SIDEBAR_WIDTH_PIXELS: i32 = 225;
pub const SIDEBAR_PADDING_X_PIXELS: i32 = 16;
pub const SIDEBAR_PADDING_Y_PIXELS: i32 = 18;
pub const SIDEBAR_ITEM_PADDING_X_PIXELS: i32 = 8;
pub const SIDEBAR_BRAND_PADDING_BOTTOM_PIXELS: i32 = 2;
pub const SIDEBAR_SUBTITLE_PADDING_BOTTOM_PIXELS: i32 = 18;
pub const SIDEBAR_SEPARATOR_PADDING_Y_PIXELS: i32 = 16;
pub const SIDEBAR_STATUS_PADDING_TOP_PIXELS: i32 = 4;
pub const NAVIGATION_BUTTON_PADDING_X_PIXELS: i32 = 12;
pub const NAVIGATION_BUTTON_HEIGHT_PIXELS: i32 = 34;
pub const NAVIGATION_BUTTON_SPACING_PIXELS: i32 = 2;
pub const SECTION_PADDING_X_PIXELS: i32 = 18;
pub const SECTION_PADDING_Y_PIXELS: i32 = 14;
pub const SECTION_BOTTOM_MARGIN_PIXELS: i32 = 14;
pub const SECTION_TITLE_HEIGHT_PIXELS: i32 = 22;
pub const STANDARD_GAP_PIXELS: i32 = 8;
pub const GROUP_GAP_PIXELS: i32 = 10;
pub const CARD_TITLE_GAP_PIXELS: i32 = 6;
pub const STATUS_LINE_GAP_PIXELS: i32 = 4;
pub const TEST_ENTRY_PADDING_TOP_PIXELS: i32 = 12;
pub const TEST_ENTRY_PADDING_BOTTOM_PIXELS: i32 = 2;
pub const AUTOCORRECTION_PRIMARY_COLUMN_WEIGHT: i32 = 3;
pub const AUTOCORRECTION_SECONDARY_COLUMN_WEIGHT: i32 = 2;
pub const AUTOCORRECTION_COLUMN_GAP_PIXELS: i32 = 14;
pub const QUIRKS_TOP_MARGIN_PIXELS: i32 = 14;
pub const MANUAL_SWITCH_NOTE_PADDING_TOP_PIXELS: i32 = 8;
pub const LAYOUT_LANGUAGE_LABEL_PADDING_TOP_PIXELS: i32 = 2;
pub const LAYOUT_LANGUAGE_LABEL_PADDING_BOTTOM_PIXELS: i32 = 10;
pub const APPLICATION_LIST_HEIGHT_ROWS: i32 = 6;
pub const WORD_LIST_HEIGHT_ROWS: i32 = 5;
pub const LEARNING_TREE_HEIGHT_ROWS: i32 = 6;
pub const HISTORY_TREE_MIN_ROWS: i32 = 14;
pub const HISTORY_TIME_COLUMN_WIDTH_PIXELS: i32 = 165;
pub const HISTORY_WORD_COLUMN_WIDTH_PIXELS: i32 = 130;
pub const HISTORY_APPLICATION_COLUMN_WIDTH_PIXELS: i32 = 150;
pub const HISTORY_CONFIDENCE_COLUMN_WIDTH_PIXELS: i32 = 100;
pub const LEARNING_WORD_COLUMN_WIDTH_PIXELS: i32 = 150;
pub const LEARNING_CONDITION_COLUMN_WIDTH_PIXELS: i32 = 200;
pub const LEARNING_ACTION_COLUMN_WIDTH_PIXELS: i32 = 160;
pub const DIAGNOSTICS_TEXT_HEIGHT_ROWS: i32 = 15;
pub const RULE_EDITOR_PATTERN_WIDTH_PIXELS: i32 = 300;
pub const RULE_EDITOR_TEXT_WRAP_PIXELS: i32 = 340;
pub const ROW_HEIGHT_PIXELS: i32 = 23;
pub const BUTTON_HEIGHT_PIXELS: i32 = 26;
pub const BUTTON_PADDING_X_PIXELS: i32 = 16;
pub const UPDOWN_WIDTH_PIXELS: i32 = 18;

// ------------------------------------------------------------------------------------------- constants/timing.py
pub const LEARNING_PROMPT_SHOW_DELAY_MS: u32 = 200;
/// constants/updates.py: the first automatic check after the start, and the interval between checks.
pub const UPDATE_CHECK_INITIAL_DELAY_MS: u32 = 30_000;

// ------------------------------------------------------------------------------------ constants/learning_prompt.py
pub const LEARNING_PROMPT_CARD_PADDING_X_PIXELS: i32 = 16;
pub const LEARNING_PROMPT_CARD_PADDING_Y_PIXELS: i32 = 12;
pub const LEARNING_PROMPT_TITLE_FONT_SIZE_POINTS: i32 = 10;
pub const LEARNING_PROMPT_WORD_FONT_SIZE_POINTS: i32 = 11;
pub const LEARNING_PROMPT_WORD_PADDING_TOP_PIXELS: i32 = 5;
pub const LEARNING_PROMPT_WORD_PADDING_BOTTOM_PIXELS: i32 = 2;
pub const LEARNING_PROMPT_HINT_FONT_SIZE_POINTS: i32 = 9;
pub const LEARNING_PROMPT_MINIMUM_WIDTH_PIXELS: i32 = 390;
pub const LEARNING_PROMPT_ANCHOR_GAP_PIXELS: i32 = 12;
pub const LEARNING_PROMPT_SCREEN_MARGIN_PIXELS: i32 = 8;
pub const LEARNING_PROMPT_CLASS: &str = "KeySwitchLearningPrompt";
// The one-time hint card (`hint_card`): the card of the rule prompt with a longer text, and two links on the last line.
pub const HINT_CARD_CLASS: &str = "KeySwitchHintCard";
pub const HINT_CARD_BORDER_PIXELS: i32 = 1;
pub const HINT_CARD_LINE_GAP_PIXELS: i32 = 6;
pub const HINT_CARD_LINK_GAP_PIXELS: i32 = 18;
/// The caret's anchor is the bottom of the caret: a card this far above it stays off a line of ordinary text.
pub const HINT_CARD_CARET_GAP_PIXELS: i32 = 28;
pub const RULE_EDITOR_CLASS: &str = "KeySwitchRuleEditor";
pub const PAGE_CLASS: &str = "KeySwitchPage";

// ---------------------------------------------------------------------------- constants/settings_defaults.py (UI)

// Shared with the macOS platform: they live in `ks-ui`.
pub use ks_ui::constants::{
    APPLICATION_CAPTURE_DELAY_MS, CONFIDENCE_SETTING_STEP, HISTORY_LIMIT_SETTING_MAX, HISTORY_LIMIT_SETTING_MIN,
    HISTORY_LIMIT_SETTING_STEP, NUMBER_DISPLAY_DECIMAL_PLACES, PAUSE_DELAY_SETTING_STEP_SECONDS, UPDATE_CHECK_INTERVAL_MS,
};

// ------------------------------------------------------------------------------------------ tray_icon.rs: the flag

/// A flag is as wide as the icon and 11/16 of it tall, centred: wider than tall, as a flag is (2:3 for Russia, 10:19 for
/// the United States), and not the square of a badge. At 16 pixels that is 16 by 11, with whole pixels for the bands.
pub const FLAG_HEIGHT_NUMERATOR: usize = 11;
pub const FLAG_HEIGHT_DENOMINATOR: usize = 16;
/// The corners of a flag are rounded by one icon pixel.
pub const FLAG_CORNER_RADIUS_ICON_PIXELS: usize = 1;
/// The edge of a flag, one canvas pixel wide (a quarter of an icon pixel): the white of the Russian flag keeps its outline
/// on a light taskbar.
pub const FLAG_EDGE_RGBA: [u8; 4] = [96, 96, 96, 255];
/// The dark rim of the slash across a switched-off flag, this much wider than the slash on the 64-pixel picture.
pub const FLAG_SLASH_RIM_PIXELS: f64 = 3.0;
pub const FLAG_SLASH_RIM_RGBA: [u8; 4] = [64, 64, 64, 255];
/// The stars of the canton, in icon pixels: the first row and column of them, and the step between them.
pub const US_FLAG_STAR_FIRST_ROW: usize = 1;
pub const US_FLAG_STAR_STEP: usize = 2;

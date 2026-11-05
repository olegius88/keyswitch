//! The rule window (the Python's `RuleEditorDialog`): the letters, the condition, the case and the action.
//!
//! It is a normal window that takes the keyboard, so the letters can be edited; the `focused` callback tells the engine
//! when the window gains and loses the keyboard, so what is typed here is left alone and typing elsewhere is not. Enter is
//! OK and Escape is Cancel. `accept` turns the form into a rule and answers with an empty string, or with what to fix, which
//! the window shows instead of closing.

use ks_cast::To as _;
use std::cell::RefCell;
use std::ptr::{null, null_mut};
use std::rc::Rc;

use ks_config::learning::{RuleAction, RuleMatch};
use ks_engine::rule_editor::{
    ACTION_TITLE, CASE_LABEL, CONDITION_LEAD, CONDITION_TITLE, EXTRA_LEAD, PATTERN_LABEL, RuleDraft, WINDOW_TITLE, action_label,
    layout_hint, match_label,
};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BeginPaint, EndPaint, HDC, PAINTSTRUCT};
use windows_sys::Win32::System::SystemServices::{SS_LEFT, SS_NOPREFIX};
use windows_sys::Win32::UI::Controls::{DRAWITEMSTRUCT, EM_SETSEL};
use windows_sys::Win32::UI::HiDpi::AdjustWindowRectExForDpi;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BS_DEFPUSHBUTTON, BS_OWNERDRAW, BS_PUSHBUTTON, CW_USEDEFAULT, DefWindowProcW, DestroyWindow, EN_CHANGE,
    ES_AUTOHSCROLL, GetSystemMetrics, IDC_ARROW, IsDialogMessageW, LoadCursorW, LoadIconW, MSG, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN,
    SW_RESTORE, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos, ShowWindow, WA_INACTIVE,
    WM_ACTIVATE, WM_CLOSE, WM_COMMAND, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DRAWITEM, WM_ERASEBKGND, WM_PAINT, WNDCLASSW,
    WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT, WS_EX_TOPMOST, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP,
    WS_VISIBLE,
};

use crate::constants::{RULE_EDITOR_CLASS, SECTION_PADDING_X_PIXELS, SECTION_PADDING_Y_PIXELS, STANDARD_GAP_PIXELS, GROUP_GAP_PIXELS, RULE_EDITOR_TEXT_WRAP_PIXELS, ROW_HEIGHT_PIXELS, BUTTON_HEIGHT_PIXELS};
use crate::win32_controls::{self as w32, Brushes, Fonts, Themed, scaled};
use crate::windows_native::wide;
use crate::windows_ui_model::Palette;

const ID_OK: usize = 1;
const ID_CANCEL: usize = 2;
const ID_PATTERN: usize = 101;
const ID_MATCH_FIRST: usize = 110;
const ID_CASE: usize = 120;
const ID_ACTION_FIRST: usize = 130;

const MATCHES: [RuleMatch; 3] = [RuleMatch::Contains, RuleMatch::Prefix, RuleMatch::Exact];
const ACTIONS: [RuleAction; 2] = [RuleAction::Convert, RuleAction::Keep];

pub struct Arguments {
    pub draft: RuleDraft,
    pub palette: Palette,
    pub dpi: i32,
    /// Turns the form into a rule: an empty string, or what to fix.
    pub accept: Box<dyn Fn(&RuleDraft) -> String>,
    pub closed: Box<dyn Fn()>,
    /// The window gained or lost the keyboard.
    pub focused: Box<dyn Fn(bool)>,
    /// Gives the keyboard to the window (`backend.restore_window`).
    pub activate: Box<dyn Fn(i64)>,
}

struct Controls {
    pattern_label: HWND,
    pattern: HWND,
    hint: HWND,
    condition_lead: HWND,
    matches: Vec<HWND>,
    extra_lead: HWND,
    case: HWND,
    actions: Vec<HWND>,
    problem: HWND,
    ok: HWND,
    cancel: HWND,
}

struct Dialog {
    window: HWND,
    fonts: Fonts,
    brushes: Brushes,
    dpi: i32,
    draft: RuleDraft,
    controls: Controls,
    problem: String,
    accept: Rc<dyn Fn(&RuleDraft) -> String>,
    closed: Rc<dyn Fn()>,
    focused: Rc<dyn Fn(bool)>,
    /// The two group frames, in client coordinates, for the paint.
    frames: [RECT; 2],
    active: bool,
}

thread_local! {
    static DIALOG: RefCell<Option<Dialog>> = const { RefCell::new(None) };
    static REGISTERED: RefCell<bool> = const { RefCell::new(false) };
}

fn register() -> bool {
    if REGISTERED.with(|flag| *flag.borrow()) {
        return true;
    }
    let class = wide(RULE_EDITOR_CLASS);
    // SAFETY: the class description points at strings that live through RegisterClassW, which copies them.
    let registered = unsafe {
        let instance = w32::hinstance();
        let description = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(dialog_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: LoadIconW(instance, std::ptr::without_provenance::<u16>(1)),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&raw const description) != 0
    };
    REGISTERED.with(|flag| *flag.borrow_mut() = registered);
    registered
}

#[must_use]
pub fn is_open() -> bool {
    DIALOG.with(|slot| slot.borrow().is_some())
}

/// `lift`: the open window comes to the front.
pub fn lift() {
    if let Some(window) = DIALOG.with(|slot| slot.borrow().as_ref().map(|dialog| dialog.window)) {
        // SAFETY: the window is ours.
        unsafe {
            ShowWindow(window, SW_RESTORE);
            SetForegroundWindow(window);
        }
    }
}

#[must_use]
pub fn is_dialog_message(message: &MSG) -> bool {
    let Some(window) = DIALOG.with(|slot| slot.borrow().as_ref().map(|dialog| dialog.window)) else { return false };
    // SAFETY: the window is ours and the message is the one the loop just read.
    unsafe { IsDialogMessageW(window, message) != 0 }
}

fn text_window(parent: HWND, text: &str, style: u32, ex_style: u32, id: usize, class: &str) -> HWND {
    w32::create_window(class, text, WS_CHILD | WS_VISIBLE | style, ex_style, parent, id, (0, 0, 10, 10))
}

/// `show`: made, laid out, put in the middle of the screen and given the keyboard.
pub fn open(arguments: Arguments) {
    if is_open() || !register() {
        return;
    }
    let dpi = arguments.dpi;
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
    let window = w32::create_window(RULE_EDITOR_CLASS, WINDOW_TITLE, style, WS_EX_TOPMOST | WS_EX_CONTROLPARENT, null_mut(), 0, (CW_USEDEFAULT, CW_USEDEFAULT, 400, 400));
    if window.is_null() {
        return;
    }
    let fonts = Fonts::new(dpi);
    let brushes = Brushes::new(arguments.palette);
    let draft = arguments.draft;
    let label = |text: &str| text_window(window, text, SS_LEFT | SS_NOPREFIX, 0, 0, "STATIC");
    let choice = |text: &str, id: usize| text_window(window, text, WS_TABSTOP | BS_OWNERDRAW.to::<u32>(), 0, id, "BUTTON");
    let controls = Controls {
        pattern_label: label(PATTERN_LABEL),
        pattern: text_window(window, &draft.pattern, WS_TABSTOP | ES_AUTOHSCROLL.to::<u32>(), WS_EX_CLIENTEDGE, ID_PATTERN, "EDIT"),
        hint: label(""),
        condition_lead: label(CONDITION_LEAD),
        matches: MATCHES.iter().enumerate().map(|(index, rule_match)| choice(match_label(*rule_match), ID_MATCH_FIRST + index)).collect(),
        extra_lead: label(EXTRA_LEAD),
        case: choice(CASE_LABEL, ID_CASE),
        actions: ACTIONS.iter().enumerate().map(|(index, action)| choice(action_label(*action), ID_ACTION_FIRST + index)).collect(),
        problem: label(""),
        ok: text_window(window, "OK", WS_TABSTOP | BS_DEFPUSHBUTTON.to::<u32>(), 0, ID_OK, "BUTTON"),
        cancel: text_window(window, "Отмена", WS_TABSTOP | BS_PUSHBUTTON.to::<u32>(), 0, ID_CANCEL, "BUTTON"),
    };
    for hwnd in [controls.pattern_label, controls.pattern, controls.hint, controls.condition_lead, controls.extra_lead, controls.problem, controls.ok, controls.cancel]
        .into_iter()
        .chain(controls.matches.iter().copied())
        .chain(std::iter::once(controls.case))
        .chain(controls.actions.iter().copied())
    {
        w32::set_font(hwnd, fonts.base);
    }
    w32::apply_control_theme(controls.pattern, arguments.palette.dark, Themed::Field);
    w32::apply_control_theme(controls.ok, arguments.palette.dark, Themed::Explorer);
    w32::apply_control_theme(controls.cancel, arguments.palette.dark, Themed::Explorer);
    w32::set_title_bar_dark(window, arguments.palette.dark);
    for (index, rule_match) in MATCHES.iter().enumerate() {
        w32::set_checked(controls.matches[index], *rule_match == draft.r#match);
    }
    w32::set_checked(controls.case, draft.case_sensitive);
    for (index, action) in ACTIONS.iter().enumerate() {
        w32::set_checked(controls.actions[index], *action == draft.action);
    }
    DIALOG.with(|slot| {
        *slot.borrow_mut() = Some(Dialog {
            window,
            fonts,
            brushes,
            dpi,
            draft,
            controls,
            problem: String::new(),
            accept: Rc::from(arguments.accept),
            closed: Rc::from(arguments.closed),
            focused: Rc::from(arguments.focused),
            frames: [RECT { left: 0, top: 0, right: 0, bottom: 0 }; 2],
            active: false,
        });
    });
    pattern_changed();
    // Centre the window on the screen and give it the keyboard.
    let (width, height) = outer_size(window);
    // SAFETY: plain metrics queries and calls on the window just made.
    unsafe {
        let x = ((GetSystemMetrics(SM_CXSCREEN) - width) / 2).max(0);
        let y = ((GetSystemMetrics(SM_CYSCREEN) - height) / 2).max(0);
        SetWindowPos(window, null_mut(), x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
        ShowWindow(window, SW_SHOW);
    }
    (arguments.activate)((window.to::<isize>()).to::<i64>());
    let pattern = DIALOG.with(|slot| slot.borrow().as_ref().map(|dialog| dialog.controls.pattern));
    if let Some(pattern) = pattern {
        // SAFETY: the window and the edit box are ours.
        unsafe {
            SetForegroundWindow(window);
            SetFocus(pattern);
        }
        w32::send(pattern, EM_SETSEL, 0, -1);
    }
}

/// The window size that has this client area.
fn outer_size(window: HWND) -> (i32, i32) {
    let (client, dpi) = DIALOG.with(|slot| {
        let slot = slot.borrow();
        let dialog = slot.as_ref().expect("the dialog is open");
        (w32::client_rect(dialog.window), dialog.dpi)
    });
    let mut rect = client;
    // SAFETY: fills the rectangle it is given.
    unsafe { AdjustWindowRectExForDpi(&raw mut rect, WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU, 0, WS_EX_TOPMOST, dpi.bits::<u32>()) };
    let _ = window;
    (rect.right - rect.left, rect.bottom - rect.top)
}

/// `current_draft`: the form as it stands.
fn current_draft(dialog: &Dialog) -> RuleDraft {
    let rule_match = MATCHES.iter().enumerate().find(|(index, _)| w32::is_checked(dialog.controls.matches[*index])).map_or(RuleMatch::Exact, |(_, value)| *value);
    let action = ACTIONS.iter().enumerate().find(|(index, _)| w32::is_checked(dialog.controls.actions[*index])).map_or(RuleAction::Convert, |(_, value)| *value);
    RuleDraft {
        pattern: w32::text_of(dialog.controls.pattern),
        r#match: rule_match,
        case_sensitive: w32::is_checked(dialog.controls.case),
        action,
        application: dialog.draft.application.clone(),
        editing: dialog.draft.editing.clone(),
    }
}

/// `_pattern_changed`: the hint under the letters follows them, and a message about the last attempt goes away.
fn pattern_changed() {
    let state = DIALOG.with(|slot| {
        slot.borrow_mut().as_mut().map(|dialog| {
            dialog.problem.clear();
            (layout_hint(&current_draft(dialog)), dialog.controls.hint, dialog.controls.problem)
        })
    });
    let Some((hint, hint_window, problem_window)) = state else { return };
    // The texts are set with no borrow held: setting one repaints the control, which asks this window for its colours.
    w32::set_text(hint_window, &hint);
    w32::set_text(problem_window, "");
    layout();
}

/// Places every control and sizes the window to its content, as Tk does for a window with no fixed size.
fn layout() {
    let moves = DIALOG.with(|slot| {
        let mut slot = slot.borrow_mut();
        let dialog = slot.as_mut()?;
        let dpi = dialog.dpi;
        let fonts = &dialog.fonts;
        let pad_x = scaled(SECTION_PADDING_X_PIXELS, dpi);
        let pad_y = scaled(SECTION_PADDING_Y_PIXELS, dpi);
        let gap = scaled(STANDARD_GAP_PIXELS, dpi);
        let group_gap = scaled(GROUP_GAP_PIXELS, dpi);
        let content = scaled(RULE_EDITOR_TEXT_WRAP_PIXELS, dpi);
        let inner = content - 2 * gap;
        let indent = group_gap;
        let controls = &dialog.controls;
        let mut moves: Vec<(HWND, i32, i32, i32, i32)> = Vec::new();
        let mut y = pad_y;
        let wrapped = |hwnd: HWND, width: i32| w32::wrapped_height(fonts.base, &w32::text_of(hwnd), width).max(w32::line_height(fonts.base));
        let height = wrapped(controls.pattern_label, content);
        moves.push((controls.pattern_label, pad_x, y, content, height));
        y += height + gap;
        let edit_height = scaled(ROW_HEIGHT_PIXELS, dpi);
        moves.push((controls.pattern, pad_x, y, content, edit_height));
        y += edit_height;
        let hint_height = wrapped(controls.hint, content);
        moves.push((controls.hint, pad_x, y, content, hint_height));
        y += hint_height + group_gap;
        let choice_height = |hwnd: HWND, width: i32| w32::wrapped_height(fonts.base, &w32::text_of(hwnd), width - scaled(24, dpi)).max(scaled(18, dpi));
        // The group frames: a title on the line, the lead, the choices.
        let title = w32::line_height(fonts.frame);
        let mut frames = [RECT { left: 0, top: 0, right: 0, bottom: 0 }; 2];
        let top = y;
        let mut cursor = y + title + gap / 2;
        let lead = wrapped(controls.condition_lead, inner);
        moves.push((controls.condition_lead, pad_x + gap, cursor, inner, lead));
        cursor += lead;
        for hwnd in &controls.matches {
            let h = choice_height(*hwnd, inner - indent);
            moves.push((*hwnd, pad_x + gap + indent, cursor, inner - indent, h));
            cursor += h;
        }
        cursor += gap;
        let extra = wrapped(controls.extra_lead, inner);
        moves.push((controls.extra_lead, pad_x + gap, cursor, inner, extra));
        cursor += extra;
        let case_height = choice_height(controls.case, inner - indent);
        moves.push((controls.case, pad_x + gap + indent, cursor, inner - indent, case_height));
        cursor += case_height + gap;
        frames[0] = RECT { left: pad_x, top: top + title / 2, right: pad_x + content, bottom: cursor };
        y = cursor + group_gap;
        let top = y;
        cursor = y + title + gap / 2;
        for hwnd in &controls.actions {
            let h = choice_height(*hwnd, inner);
            moves.push((*hwnd, pad_x + gap, cursor, inner, h));
            cursor += h;
        }
        cursor += gap;
        frames[1] = RECT { left: pad_x, top: top + title / 2, right: pad_x + content, bottom: cursor };
        y = cursor;
        let problem = w32::wrapped_height(fonts.base, &w32::text_of(controls.problem), content);
        let problem = if w32::text_of(controls.problem).is_empty() { 0 } else { problem };
        moves.push((controls.problem, pad_x, y + gap, content, problem));
        y += gap + problem + group_gap;
        let button_width = scaled(84, dpi);
        let button_height = scaled(BUTTON_HEIGHT_PIXELS, dpi);
        moves.push((controls.cancel, pad_x + content - button_width, y, button_width, button_height));
        moves.push((controls.ok, pad_x + content - 2 * button_width - gap, y, button_width, button_height));
        y += button_height + pad_y;
        dialog.frames = frames;
        Some((moves, 2 * pad_x + content, y))
    });
    let Some((moves, width, height)) = moves else { return };
    for (hwnd, x, y, w, h) in &moves {
        // SAFETY: the controls are ours.
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::MoveWindow(*hwnd, *x, *y, *w, *h, 1) };
    }
    let (window, dpi) = DIALOG.with(|slot| {
        let slot = slot.borrow();
        let dialog = slot.as_ref().expect("the dialog is open");
        (dialog.window, dialog.dpi)
    });
    let mut rect = RECT { left: 0, top: 0, right: width, bottom: height };
    // SAFETY: fills the rectangle; the window is ours.
    unsafe {
        AdjustWindowRectExForDpi(&raw mut rect, WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU, 0, WS_EX_TOPMOST, dpi.bits::<u32>());
        SetWindowPos(window, null_mut(), 0, 0, rect.right - rect.left, rect.bottom - rect.top, SWP_NOZORDER | SWP_NOACTIVATE | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOMOVE);
    }
    w32::invalidate(window);
}

fn confirm() {
    let Some((draft, accept, pattern)) = DIALOG.with(|slot| slot.borrow().as_ref().map(|dialog| (current_draft(dialog), dialog.accept.clone(), dialog.controls.pattern))) else {
        return;
    };
    let problem = accept(&draft);
    if problem.is_empty() {
        close();
        return;
    }
    let problem_window = DIALOG.with(|slot| {
        slot.borrow_mut().as_mut().map(|dialog| {
            dialog.problem = problem.clone();
            dialog.controls.problem
        })
    });
    if let Some(problem_window) = problem_window {
        w32::set_text(problem_window, &problem);
    }
    layout();
    // SAFETY: the edit box is ours.
    unsafe { SetFocus(pattern) };
}

/// `_close`: the window goes, the engine is told the keyboard is no longer here, and the owner is told it is closed.
fn close() {
    let Some(dialog) = DIALOG.with(|slot| slot.borrow_mut().take()) else { return };
    // SAFETY: the window belongs to this thread.
    unsafe { DestroyWindow(dialog.window) };
    dialog.fonts.destroy();
    dialog.brushes.destroy();
    (dialog.focused)(false);
    (dialog.closed)();
}

fn click(id: usize) {
    DIALOG.with(|slot| {
        let slot = slot.borrow();
        let Some(dialog) = slot.as_ref() else { return };
        let controls = &dialog.controls;
        match id {
            id if (ID_MATCH_FIRST..ID_MATCH_FIRST + MATCHES.len()).contains(&id) => {
                for (index, hwnd) in controls.matches.iter().enumerate() {
                    w32::set_checked(*hwnd, ID_MATCH_FIRST + index == id);
                }
            }
            id if (ID_ACTION_FIRST..ID_ACTION_FIRST + ACTIONS.len()).contains(&id) => {
                for (index, hwnd) in controls.actions.iter().enumerate() {
                    w32::set_checked(*hwnd, ID_ACTION_FIRST + index == id);
                }
            }
            ID_CASE => w32::set_checked(controls.case, !w32::is_checked(controls.case)),
            _ => {}
        }
    });
}

fn paint(window: HWND) {
    // SAFETY: PAINTSTRUCT is plain data filled by BeginPaint.
    let mut paint: PAINTSTRUCT = unsafe { std::mem::zeroed() };
    // SAFETY: balanced with EndPaint below on the same window.
    let hdc: HDC = unsafe { BeginPaint(window, &raw mut paint) };
    DIALOG.with(|slot| {
        if let Some(dialog) = slot.borrow().as_ref() {
            let palette = dialog.brushes.palette;
            w32::fill_rect(hdc, &w32::client_rect(window), dialog.brushes.content);
            for (frame, title) in dialog.frames.iter().zip([CONDITION_TITLE, ACTION_TITLE]) {
                w32::frame_rect(hdc, frame, palette.line.colorref());
                let text = format!(" {title} ");
                let height = w32::line_height(dialog.fonts.frame);
                let mut text_rect = RECT { left: frame.left + scaled(SECTION_PADDING_X_PIXELS - 6, dialog.dpi), top: frame.top - height / 2, right: frame.right, bottom: frame.top + height };
                let width = w32::text_width(dialog.fonts.frame, &text);
                w32::fill_rect(hdc, &RECT { left: text_rect.left, top: text_rect.top, right: text_rect.left + width, bottom: text_rect.bottom }, dialog.brushes.content);
                w32::draw_label_text(hdc, &text, &mut text_rect, dialog.fonts.frame, palette.foreground.colorref());
            }
        }
    });
    // SAFETY: ends the paint begun above.
    unsafe { EndPaint(window, &raw const paint) };
}

unsafe extern "system" fn dialog_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(window);
            0
        }
        WM_COMMAND => {
            let id = wparam & 0xFFFF;
            let code = ((wparam >> 16) & 0xFFFF).low::<u32>();
            match id {
                ID_OK => confirm(),
                ID_CANCEL => close(),
                ID_PATTERN if code == EN_CHANGE => pattern_changed(),
                other => click(other),
            }
            // A change of the condition or the action leaves the hint as it is, but the pattern's own message goes.
            0
        }
        WM_DRAWITEM => {
            // SAFETY: for this message `lparam` points at the structure the system gave.
            let item = unsafe { &*lparam.pointer::<DRAWITEMSTRUCT>() };
            DIALOG.with(|slot| {
                if let Some(dialog) = slot.borrow().as_ref() {
                    let radio = item.CtlID.to::<usize>() != ID_CASE;
                    w32::draw_choice(item, &dialog.brushes, dialog.fonts.base, dialog.dpi, radio);
                }
            });
            1
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => DIALOG.with(|slot| {
            let slot = slot.borrow();
            let Some(dialog) = slot.as_ref() else { return 0 };
            let palette = dialog.brushes.palette;
            let hdc = wparam.pointer_mut::<std::ffi::c_void>();
            if message == WM_CTLCOLOREDIT {
                w32::set_label_colors(hdc, palette.foreground.colorref(), palette.field.colorref());
                return dialog.brushes.field.to::<LRESULT>();
            }
            let control = lparam.pointer_mut::<std::ffi::c_void>();
            let color = if control == dialog.controls.problem {
                palette.error
            } else if control == dialog.controls.hint {
                palette.muted
            } else {
                palette.foreground
            };
            w32::set_label_colors(hdc, color.colorref(), palette.content.colorref());
            dialog.brushes.content.to::<LRESULT>()
        }),
        WM_ACTIVATE => {
            // Only the keyboard leaving the window counts; moving between its own fields never reaches here.
            let active = (wparam & 0xFFFF).low::<u32>() != WA_INACTIVE;
            let focused = DIALOG.with(|slot| {
                let mut slot = slot.borrow_mut();
                let dialog = slot.as_mut()?;
                if dialog.active == active {
                    return None;
                }
                dialog.active = active;
                Some(dialog.focused.clone())
            });
            if let Some(focused) = focused {
                focused(active);
            }
            0
        }
        WM_CLOSE => {
            close();
            0
        }
        WM_DESTROY => 0,
        // SAFETY: the default procedure takes the arguments the system gave this one.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

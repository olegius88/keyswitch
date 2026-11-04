//! Platform-neutral keyboard backend contracts.

use std::sync::Arc;

use crate::constants::{ALT_MASK, CONTROL_MASK, LOCK_MASK, SHIFT_MASK, SUPER_MASK};
use crate::input_delay::{HookGap, InputDelayReport};
use crate::selected_text::{SelectedText, SelectionError, SelectionRefused};

/// What the key filter says about a key: pass it on, consume it, or hold it until the engine decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyDisposition {
    Pass,
    Consume,
    Defer,
}

/// A backend call that failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendError {
    /// The operation failed; the message goes to the log.
    Failed(String),
    /// The layout was asked for, but the application did not confirm it in time.
    ///
    /// The request stays posted: the layout may still arrive, and when it does it is the
    /// engine's switch, not the user's.
    LayoutSwitchUnconfirmed { message: String, group: i32 },
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendError::Failed(message) | BackendError::LayoutSwitchUnconfirmed { message, .. } => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for BackendError {}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct KeyEvent {
    pub pressed: bool,
    pub keycode: u32,
    pub key_name: String,
    pub character: String,
    pub characters: Vec<String>,
    pub group: i32,
    pub state: u32,
    pub timestamp: i64,
    pub synthetic: bool,
    pub deferred: bool,
    /// Injected by another program (remote control, a macro, an on-screen keyboard),
    /// not typed on this keyboard; only the Windows backend can tell.
    pub foreign: bool,
}

impl KeyEvent {
    pub fn shift(&self) -> bool {
        self.state & SHIFT_MASK != 0
    }

    pub fn caps_lock(&self) -> bool {
        self.state & LOCK_MASK != 0
    }

    pub fn control(&self) -> bool {
        self.state & CONTROL_MASK != 0
    }

    pub fn alt(&self) -> bool {
        self.state & ALT_MASK != 0
    }

    pub fn super_key(&self) -> bool {
        self.state & SUPER_MASK != 0
    }

    pub fn character_for(&self, group: i32) -> &str {
        usize::try_from(group).ok().and_then(|index| self.characters.get(index)).map_or("", String::as_str)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BackendProbe {
    pub available: bool,
    pub session_type: String,
    pub display: String,
    pub record_version: String,
    pub xtest_version: String,
    pub xkb_version: String,
    pub current_group: i32,
    pub error: String,
}

/// A screen position captured before a learning prompt takes focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenAnchor {
    pub x: i32,
    pub y: i32,
    pub window: Option<i64>,
}

/// The focused top-level window as the backend sees it.
///
/// `window` is the platform identity (an HWND or an X window id). `own` marks windows of KeySwitch
/// itself (settings, the learning prompt), which never count as the user moving to another window,
/// except while the test field of the settings window has the keyboard: the window reports that to
/// the engine (`KeySwitchEngine::set_test_field_focused`). `isolated_layout` says that the window
/// carries a keyboard layout of its own that the user did not choose (Windows keeps one per
/// window), so a layout change seen while it is focused must be ignored; on X11 the layout is global.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FocusInfo {
    pub window: i64,
    pub own: bool,
    pub isolated_layout: bool,
}

/// Receives every key the backend sees, on the backend's own thread.
pub type KeyListener = Arc<dyn Fn(KeyEvent) + Send + Sync>;
/// Answers on the hook's thread whether a key is consumed. It must read atomics only and never wait.
pub type KeyFilter = Arc<dyn Fn(&KeyEvent) -> KeyDisposition + Send + Sync>;

/// One correction to type: the Python keyword arguments of `inject_correction`, spelled out.
#[derive(Clone, Copy, Debug)]
pub struct Injection<'a> {
    pub strokes: &'a [KeyEvent],
    pub target_group: i32,
    pub boundary: Option<&'a KeyEvent>,
    pub source_group: Option<i32>,
    pub late: &'a [KeyEvent],
    pub trailing: &'a [KeyEvent],
    pub kept_tail: usize,
    /// How many characters the word occupies on screen when that differs from the keys it types
    /// (a typo correction); None when the backend erases one character per stroke.
    pub erased: Option<usize>,
}

/// Operations required by the correction engine and diagnostics UI.
pub trait InputBackend: Send + Sync {
    fn start(&self, listener: KeyListener) -> Result<(), BackendError>;
    fn set_key_filter(&self, predicate: Option<KeyFilter>);
    fn hold_input(&self);
    /// Types the held keys again and says how many; a failed send is an error the engine reports.
    fn release_input(&self) -> Result<i32, BackendError>;
    /// Delivers a withheld Enter/Tab, or forgets it; says how many key events it sent.
    fn complete_action(&self, deliver: bool) -> Result<i32, BackendError>;
    fn stop(&self);
    fn close(&self);
    fn current_group(&self) -> Result<i32, BackendError>;
    fn switch_group(&self, group: i32) -> Result<(), BackendError>;
    fn active_application(&self) -> String;
    fn focused_window(&self) -> Option<FocusInfo>;
    fn probe(&self) -> BackendProbe;
    /// Types a correction and says how many key events it sent.
    fn inject_correction(&self, injection: Injection<'_>) -> Result<i32, BackendError>;

    /// Whether `inject_correction` can erase another number of characters than it types. A typo
    /// correction types more or fewer keys than the word on screen has (`клвиатура` is `клавиатура`);
    /// a backend that cannot is offered only corrections of the word's own length.
    fn erases_any_length(&self) -> bool {
        true
    }

    /// Whether the backend holds a hotkey back from the application itself.
    fn swallows_hotkeys(&self) -> bool {
        false
    }

    /// The input the hook did not see since the last call, on backends that can tell.
    fn take_unseen_input(&self) -> Result<Option<i64>, BackendError> {
        Ok(None)
    }

    fn take_input_delay(&self) -> Option<InputDelayReport> {
        None
    }

    fn take_hook_gap(&self) -> Option<HookGap> {
        None
    }

    /// Whether the selection hotkeys can work here (Python: `isinstance(backend, SelectionBackend)`).
    fn supports_selection(&self) -> bool {
        false
    }

    /// Reads the selection, or says why it is left as it is.
    fn read_selection(&self, _application: &str, _window: i64) -> Result<SelectedText, SelectionError> {
        Err(SelectionRefused::new(crate::selected_text::RefusalReason::Unsupported).into())
    }

    /// Types `replacement` over the selection, leaves the layout in `final_group` (or where it was, for None),
    /// and says whether it selected the result again so that the next press acts on it.
    fn replace_selection(
        &self,
        _selected: &SelectedText,
        _replacement: &str,
        _final_group: Option<i32>,
    ) -> Result<bool, SelectionError> {
        Err(SelectionRefused::new(crate::selected_text::RefusalReason::Unsupported).into())
    }

    fn input_anchor(&self) -> Option<ScreenAnchor> {
        None
    }

    fn restore_window(&self, _window: Option<i64>) -> bool {
        false
    }

    fn keep_window_inactive(&self, _window: i64) -> bool {
        false
    }
}

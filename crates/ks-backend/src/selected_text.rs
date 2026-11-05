//! What a backend reads of the selection, and why it may leave the selection as it is.

use crate::backend::BackendError;

/// Where the selected text was read; the log names it.
pub const SOURCE_UIA: &str = "uia";
pub const SOURCE_CLIPBOARD: &str = "clipboard";
pub const SOURCE_ATSPI: &str = "atspi";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalReason {
    Disabled,
    OwnWindow,
    ExcludedApplication,
    SensitiveField,
    Unsupported,
    NoSelection,
    MultipleSelection,
    ReadOnly,
    TooLong,
    LineBreaks,
    Untypeable,
    Terminal,
    ClipboardBusy,
    ClipboardUnrestorable,
    NotSettable,
    FocusChanged,
    ReleaseTimeout,
    NoLayout,
    EmptyResult,
    NoCaret,
}

impl RefusalReason {
    /// The name the log gives the reason.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            RefusalReason::Disabled => "disabled",
            RefusalReason::OwnWindow => "own_window",
            RefusalReason::ExcludedApplication => "excluded_application",
            RefusalReason::SensitiveField => "sensitive_field",
            RefusalReason::Unsupported => "unsupported",
            RefusalReason::NoSelection => "no_selection",
            RefusalReason::MultipleSelection => "multiple_selection",
            RefusalReason::ReadOnly => "read_only",
            RefusalReason::TooLong => "too_long",
            RefusalReason::LineBreaks => "line_breaks",
            RefusalReason::Untypeable => "untypeable",
            RefusalReason::Terminal => "terminal",
            RefusalReason::ClipboardBusy => "clipboard_busy",
            RefusalReason::ClipboardUnrestorable => "clipboard_unrestorable",
            RefusalReason::NotSettable => "not_settable",
            RefusalReason::FocusChanged => "focus_changed",
            RefusalReason::ReleaseTimeout => "release_timeout",
            RefusalReason::NoLayout => "no_layout",
            RefusalReason::EmptyResult => "empty_result",
            RefusalReason::NoCaret => "no_caret",
        }
    }
}

/// The selection is left as it is, for a reason the log and the status line name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionRefused {
    pub reason: RefusalReason,
}

impl SelectionRefused {
    #[must_use]
    pub fn new(reason: RefusalReason) -> Self {
        SelectionRefused { reason }
    }
}

/// The selected text and where it was read (SOURCE_*).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedText {
    pub text: String,
    pub source: String,
}

/// Why a selection hotkey changed nothing: a refusal the log calls `refused`, or a failure of the backend the log calls
/// `failed` and whose message goes to the status line. The Python raises one or the other through the same call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionError {
    Refused(SelectionRefused),
    Failed(BackendError),
}

impl From<SelectionRefused> for SelectionError {
    fn from(refused: SelectionRefused) -> Self {
        SelectionError::Refused(refused)
    }
}

impl From<BackendError> for SelectionError {
    fn from(error: BackendError) -> Self {
        SelectionError::Failed(error)
    }
}

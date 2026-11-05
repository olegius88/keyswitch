//! Optional Linux AT-SPI text access with bounded traversal and IPC timeouts (the Python's `atspi_context.py`).
//!
//! The Python reaches AT-SPI through libatspi's `GObject` bindings. Here the accessibility bus is spoken to directly
//! (`atspi_bus`), and this file holds what the reader decides, over the small `AtspiApi` a fake can replace in tests.

use ks_cast::To as _;
use std::cell::RefCell;
use std::time::{Duration, Instant};

use ks_backend::constants::FIELD_CONTEXT_MAX_CHARACTERS;
use ks_backend::selected_text::SOURCE_ATSPI;
use ks_backend::{
    BackendError, FieldContext, FieldRole, ManagedFieldReader, ReaderFailure, RefusalReason, SelectedText, SelectionError,
    SelectionRefused,
};
use ks_config::constants::SELECTION_COMMAND_MAX_CHARACTERS;
use ks_text::pystr;

use crate::constants::{
    ATSPI_FIELD_READ_DEADLINE_SECONDS, ATSPI_MAX_TRAVERSED_CHILDREN, ATSPI_MAX_VISITED_NODES, ATSPI_ROLE_PASSWORD_TEXT,
    ATSPI_STATE_EDITABLE, ATSPI_STATE_FOCUSED, FIELD_AFTER_CARET_MAX_CHARACTERS, PASSWORD_MASKS,
};
use crate::x11_backend::{ProcessForWindow, SelectionSource};

pub const INTERFACE_TEXT: &str = "org.a11y.atspi.Text";
pub const INTERFACE_EDITABLE_TEXT: &str = "org.a11y.atspi.EditableText";

/// An accessible object: the bus name of its application and its path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeRef {
    pub bus_name: String,
    pub path: String,
}

/// Why a call to the accessibility bus did not answer. The text is for tests; the log gets the class only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AtspiError {
    /// The bus could not be reached: libatspi's `init()` failing (the Python's `RuntimeError`).
    Unavailable(String),
    /// A call failed or ran out of time (the Python's `GLib.Error`).
    Call(String),
}

impl AtspiError {
    /// What the diagnostics say of a failure: its class and name, never its text, which may carry what the user typed.
    #[must_use]
    pub fn failure(&self) -> ReaderFailure {
        match self {
            AtspiError::Unavailable(_) => ReaderFailure { failure_type: "runtime_error", failure_name: "RuntimeError", failure_code: None, permanent: false },
            AtspiError::Call(_) => ReaderFailure { failure_type: "provider_error", failure_name: "GError", failure_code: None, permanent: false },
        }
    }
}

pub type AtspiResult<T> = Result<T, AtspiError>;

/// The calls of the accessibility bus the reader makes (`Atspi.Accessible`, `Text` and `EditableText`).
pub trait AtspiApi: Send + Sync {
    fn child_count(&self, node: Option<&NodeRef>) -> AtspiResult<usize>;
    /// The child of a node, the desktop for None.
    fn child_at(&self, node: Option<&NodeRef>, index: usize) -> AtspiResult<Option<NodeRef>>;
    fn name(&self, node: &NodeRef) -> AtspiResult<String>;
    fn process_id(&self, node: &NodeRef) -> AtspiResult<u32>;
    /// `AtspiStateSet` as a bit set: bit `n` is `AtspiStateType` `n`.
    fn states(&self, node: &NodeRef) -> AtspiResult<u64>;
    fn role(&self, node: &NodeRef) -> AtspiResult<u32>;
    /// Whether the object implements the interface (`get_text_iface`, `get_editable_text_iface`).
    fn has_interface(&self, node: &NodeRef, interface: &str) -> AtspiResult<bool>;
    fn n_selections(&self, node: &NodeRef) -> AtspiResult<i32>;
    fn caret_offset(&self, node: &NodeRef) -> AtspiResult<i32>;
    fn character_count(&self, node: &NodeRef) -> AtspiResult<i32>;
    fn get_text(&self, node: &NodeRef, start: i32, end: i32) -> AtspiResult<String>;
    fn get_selection(&self, node: &NodeRef, number: i32) -> AtspiResult<(i32, i32)>;
    fn set_selection(&self, node: &NodeRef, number: i32, start: i32, end: i32) -> AtspiResult<bool>;
    fn delete_text(&self, node: &NodeRef, start: i32, end: i32) -> AtspiResult<bool>;
    /// `length` is the byte count of the text.
    fn insert_text(&self, node: &NodeRef, position: i32, text: &str, length: i32) -> AtspiResult<bool>;
    /// `Atspi.Text.get_character_extents` in screen coordinates: x, y, width, height of the character at `offset`.
    fn character_extents(&self, _node: &NodeRef, _offset: i32) -> AtspiResult<(i32, i32, i32, i32)> {
        Err(AtspiError::Call("character extents are not offered".to_owned()))
    }
}

/// Where the last selection was read: the field and the selection's bounds and text.
#[derive(Clone, Debug)]
struct ReadSelection {
    node: NodeRef,
    start: i32,
    end: i32,
    selected: String,
}

pub struct AtspiFieldReader {
    api: Box<dyn AtspiApi>,
    process_for_window: Option<ProcessForWindow>,
    /// The selection `read_selection` found, for `write_selection` to write over.
    read: RefCell<Option<ReadSelection>>,
    /// Reads `/proc/<pid>/comm`; a test replaces it.
    process_name: Box<dyn Fn(u32) -> String + Send + Sync>,
}

fn has_state(states: u64, state: u32) -> bool {
    states >> state & 1 == 1
}

/// Whether a text shows only what GTK4 shows for a hidden entry.
fn only_password_masks(text: &str) -> bool {
    let visible = pystr::strip(text);
    !visible.is_empty() && visible.chars().all(|character| PASSWORD_MASKS.contains(&character))
}

fn read_process_name(process: u32) -> String {
    std::fs::read_to_string(format!("/proc/{process}/comm")).map(|text| pystr::strip(&text).to_owned()).unwrap_or_default()
}

fn refused(reason: RefusalReason) -> SelectionError {
    SelectionRefused::new(reason).into()
}

fn call_failed(error: AtspiError) -> SelectionError {
    // Provider text may carry what the user typed: only the class goes to the message.
    SelectionError::Failed(BackendError::Failed(match error {
        AtspiError::Unavailable(_) => "AT-SPI недоступен".to_owned(),
        AtspiError::Call(_) => "Вызов AT-SPI не удался".to_owned(),
    }))
}

impl AtspiFieldReader {
    pub fn new(api: Box<dyn AtspiApi>, process_for_window: Option<ProcessForWindow>) -> AtspiFieldReader {
        AtspiFieldReader { api, process_for_window, read: RefCell::new(None), process_name: Box::new(read_process_name) }
    }

    fn matches(&self, application: &str, node: &NodeRef) -> AtspiResult<bool> {
        let name = pystr::casefold(&self.api.name(node)?);
        let process = (self.process_name)(self.api.process_id(node).unwrap_or(0));
        let wanted = pystr::casefold(application);
        Ok(wanted == name || wanted == pystr::casefold(&process))
    }

    /// The focused node of the application's window and its field id, or None within the bounds of the walk.
    fn focused(&self, application: &str, window: i64, deadline: Instant) -> AtspiResult<Option<(NodeRef, String)>> {
        let pid = self.process_for_window.as_ref().map_or(0, |process| process(window));
        let mut stack: Vec<(NodeRef, String)> = Vec::new();
        for index in 0..self.api.child_count(None)?.min(ATSPI_MAX_TRAVERSED_CHILDREN) {
            if Instant::now() >= deadline {
                return Ok(None);
            }
            if let Some(app) = self.api.child_at(None, index)? {
                let ours = if pid != 0 { self.api.process_id(&app)? == pid } else { self.matches(application, &app)? };
                if ours {
                    stack.push((app, index.to_string()));
                }
            }
        }
        let mut visited = 0;
        while !stack.is_empty() && Instant::now() < deadline && visited < ATSPI_MAX_VISITED_NODES {
            let (node, path) = stack.pop().expect("not empty");
            visited += 1;
            if has_state(self.api.states(&node)?, ATSPI_STATE_FOCUSED) {
                return Ok(Some((node, format!("{window}:{path}"))));
            }
            for index in (0..self.api.child_count(Some(&node))?.min(ATSPI_MAX_TRAVERSED_CHILDREN)).rev() {
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                if let Some(child) = self.api.child_at(Some(&node), index)? {
                    stack.push((child, format!("{path}:{index}")));
                }
            }
        }
        Ok(None)
    }

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs_f64(ATSPI_FIELD_READ_DEADLINE_SECONDS)
    }

    fn password_field(application: &str, field_id: String) -> FieldContext {
        let mut field = FieldContext::new(application, field_id);
        field.role = FieldRole::Password;
        field.sensitive = true;
        SOURCE_ATSPI.clone_into(&mut field.source);
        field
    }

    fn plain_field(application: &str, field_id: String, selection: bool) -> FieldContext {
        let mut field = FieldContext::new(application, field_id);
        field.selection = selection;
        SOURCE_ATSPI.clone_into(&mut field.source);
        field
    }

    fn read_field(&self, application: &str, window: i64) -> AtspiResult<Option<FieldContext>> {
        let Some((node, field_id)) = self.focused(application, window, Self::deadline())? else { return Ok(None) };
        if self.api.role(&node)? == ATSPI_ROLE_PASSWORD_TEXT {
            return Ok(Some(Self::password_field(application, field_id)));
        }
        if !self.api.has_interface(&node, INTERFACE_TEXT)? {
            return Ok(None);
        }
        if self.api.n_selections(&node)? != 0 {
            return Ok(Some(Self::plain_field(application, field_id, true)));
        }
        let caret = self.api.caret_offset(&node)?;
        if caret < 0 {
            return Ok(None);
        }
        let limit = ks_cast::const_i32(FIELD_CONTEXT_MAX_CHARACTERS);
        let before = self.api.get_text(&node, (caret - limit).max(0), caret)?;
        let after = self.api.get_text(&node, caret, self.api.character_count(&node)?.min(caret + FIELD_AFTER_CARET_MAX_CHARACTERS.low::<i32>()))?;
        // GTK4 can expose a hidden Entry as ordinary TEXT while GetText returns only password masks. Treat that evidence
        // conservatively; never feed masked fields to the model.
        if only_password_masks(&format!("{before}{after}")) {
            return Ok(Some(Self::password_field(application, field_id)));
        }
        if self.api.n_selections(&node)? != 0 {
            return Ok(Some(Self::plain_field(application, field_id, true)));
        }
        if !has_state(self.api.states(&node)?, ATSPI_STATE_FOCUSED) || self.api.caret_offset(&node)? != caret {
            return Ok(Some(Self::plain_field(application, field_id, false)));
        }
        let mut field = FieldContext::new(application, field_id);
        field.before = before;
        field.after = after;
        field.role = FieldRole::Text;
        SOURCE_ATSPI.clone_into(&mut field.source);
        Ok(Some(field.bounded()))
    }

    /// The text selected in the focused field, for the selection hotkeys; a refusal when there is none to change: no field
    /// found or no text in it, a password field, a field the user cannot edit, no selection or several. The selection's
    /// bounds are passed to `get_text` as they come: its end is "the first character past the desired range"
    /// (`docs.gtk.org/atspi2/method.Text.get_text.html`).
    pub fn read_selection(&self, application: &str, window: i64) -> Result<SelectedText, SelectionError> {
        let found = self.focused(application, window, Self::deadline()).map_err(|_| refused(RefusalReason::Unsupported))?;
        let Some((node, _field_id)) = found else { return Err(refused(RefusalReason::Unsupported)) };
        let api = &*self.api;
        let unsupported = |_: AtspiError| refused(RefusalReason::Unsupported);
        if api.role(&node).map_err(unsupported)? == ATSPI_ROLE_PASSWORD_TEXT {
            return Err(refused(RefusalReason::SensitiveField));
        }
        if !api.has_interface(&node, INTERFACE_TEXT).map_err(unsupported)? {
            return Err(refused(RefusalReason::Unsupported));
        }
        if !has_state(api.states(&node).map_err(unsupported)?, ATSPI_STATE_EDITABLE) {
            return Err(refused(RefusalReason::ReadOnly));
        }
        let selections = api.n_selections(&node).map_err(unsupported)?;
        if selections > 1 {
            return Err(refused(RefusalReason::MultipleSelection));
        }
        let span = if selections != 0 { Some(api.get_selection(&node, 0).map_err(unsupported)?) } else { None };
        let Some((start, end)) = span.filter(|(start, end)| end > start) else { return Err(refused(RefusalReason::NoSelection)) };
        // One character past the bound is read, so that a longer selection is told apart.
        let stop = end.min(start + SELECTION_COMMAND_MAX_CHARACTERS.low::<i32>() + 1);
        let selected = api.get_text(&node, start, stop).map_err(unsupported)?;
        if only_password_masks(&selected) {
            return Err(refused(RefusalReason::SensitiveField));
        }
        *self.read.borrow_mut() = Some(ReadSelection { node, start, end, selected: selected.clone() });
        Ok(SelectedText { text: selected, source: SOURCE_ATSPI.to_owned() })
    }

    /// Writes `replacement` over the selection last read, through the field's `EditableText`; None without one.
    ///
    /// The program changes its own text, so any character goes in as it is (`—`, `«»`, an emoji) and no layout or key is
    /// involved. The field must still be focused and show the very selection that was read. The selected range is deleted
    /// and the replacement inserted at its start (`EditableText.delete_text` takes the first character past the range as
    /// its end; `insert_text` inserts all of a text whose byte count is within `length`, so the UTF-8 byte count is
    /// given). Says whether the result was selected again.
    pub fn write_selection(&self, selected: &SelectedText, replacement: &str) -> Result<Option<bool>, SelectionError> {
        let Some(read) = self.read.borrow().clone() else { return Err(refused(RefusalReason::FocusChanged)) };
        if read.selected != selected.text {
            return Err(refused(RefusalReason::FocusChanged));
        }
        let api = &*self.api;
        if !api.has_interface(&read.node, INTERFACE_EDITABLE_TEXT).map_err(call_failed)? {
            return Ok(None);
        }
        let span = if api.n_selections(&read.node).map_err(call_failed)? == 1 { Some(api.get_selection(&read.node, 0).map_err(call_failed)?) } else { None };
        let unchanged = match span {
            Some(span) => {
                has_state(api.states(&read.node).map_err(call_failed)?, ATSPI_STATE_FOCUSED)
                    && span == (read.start, read.end)
                    && api.get_text(&read.node, read.start, read.end).map_err(call_failed)? == read.selected
            }
            None => false,
        };
        if !unchanged {
            return Err(refused(RefusalReason::FocusChanged));
        }
        *self.read.borrow_mut() = None;
        if !api.delete_text(&read.node, read.start, read.end).map_err(call_failed)? {
            return Err(refused(RefusalReason::NotSettable));
        }
        if !api.insert_text(&read.node, read.start, replacement, replacement.len().low::<i32>()).map_err(call_failed)? {
            // The selection is gone and nothing came in its place: the selected text is put back.
            let restored = api.insert_text(&read.node, read.start, &read.selected, read.selected.len().low::<i32>()).unwrap_or(false);
            return Err(SelectionError::Failed(BackendError::Failed(format!(
                "AT-SPI не вставил новый текст; выделенный текст {}",
                if restored { "возвращён" } else { "не вернулся" }
            ))));
        }
        let length = replacement.chars().count().low::<i32>();
        Ok(Some(api.set_selection(&read.node, 0, read.start, read.start + length).unwrap_or(false)))
    }
}

impl ManagedFieldReader for AtspiFieldReader {
    fn read(&self, application: &str, window: i64) -> Result<Option<FieldContext>, ReaderFailure> {
        self.read_field(application, window).map_err(|error| error.failure())
    }

    fn close(&mut self) {
        // No retained accessible objects or per-reader native resources.
    }
}

/// The selection hotkeys' reader on Linux, X11 and Wayland alike, built on first use.
///
/// The PRIMARY selection of X11 holds whatever was selected last anywhere, and nothing tells whether that lies in the
/// focused field: text typed over a selection made in another window would land where the caret is. AT-SPI says which field
/// has the focus, whether it can be edited and what is selected in it, so it is the one source here.
pub struct AtspiSelectionReader {
    connect: Box<dyn Fn() -> AtspiResult<Box<dyn AtspiApi>> + Send>,
    process_for_window: ProcessForWindow,
    reader: Option<AtspiFieldReader>,
}

impl AtspiSelectionReader {
    pub fn new(connect: Box<dyn Fn() -> AtspiResult<Box<dyn AtspiApi>> + Send>, process_for_window: ProcessForWindow) -> Self {
        AtspiSelectionReader { connect, process_for_window, reader: None }
    }
}

impl SelectionSource for AtspiSelectionReader {
    fn read_selection(&mut self, application: &str, window: i64) -> Result<SelectedText, SelectionError> {
        if self.reader.is_none() {
            // No accessibility bus: the program's selection cannot be read.
            let api = (self.connect)().map_err(|_| refused(RefusalReason::Unsupported))?;
            self.reader = Some(AtspiFieldReader::new(api, Some(self.process_for_window.clone())));
        }
        self.reader.as_ref().expect("just built").read_selection(application, window)
    }

    fn write_selection(&mut self, selected: &SelectedText, replacement: &str) -> Result<Option<bool>, SelectionError> {
        match &self.reader {
            Some(reader) => reader.write_selection(selected, replacement),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
#[path = "atspi_context_tests.rs"]
mod tests;

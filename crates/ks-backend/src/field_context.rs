//! What a platform's accessibility reader sees of the focused field. User text is kept in RAM, never serialized.

use crate::constants::{
    FIELD_CONTEXT_APPLICATION_MAX_CHARACTERS, FIELD_CONTEXT_FIELD_ID_MAX_CHARACTERS, FIELD_CONTEXT_MAX_CHARACTERS,
    FIELD_CONTEXT_SOURCE_MAX_CHARACTERS, FIELD_INPUT_BUFFER_MAX_CHARACTERS,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FieldRole {
    #[default]
    Unknown,
    Text,
    Code,
    Search,
    Terminal,
    Password,
}

impl FieldRole {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            FieldRole::Unknown => "unknown",
            FieldRole::Text => "text",
            FieldRole::Code => "code",
            FieldRole::Search => "search",
            FieldRole::Terminal => "terminal",
            FieldRole::Password => "password",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldContext {
    pub application: String,
    pub field_id: String,
    pub before: String,
    pub after: String,
    pub role: FieldRole,
    pub selection: bool,
    pub sensitive: bool,
    pub source: String,
}

impl FieldContext {
    pub fn new(application: impl Into<String>, field_id: impl Into<String>) -> Self {
        FieldContext {
            application: application.into(),
            field_id: field_id.into(),
            before: String::new(),
            after: String::new(),
            role: FieldRole::Unknown,
            selection: false,
            sensitive: false,
            source: "observed".to_owned(),
        }
    }

    #[must_use]
    pub fn bounded(&self) -> FieldContext {
        let private = self.sensitive || self.role == FieldRole::Password;
        FieldContext {
            application: first_characters(&self.application, FIELD_CONTEXT_APPLICATION_MAX_CHARACTERS).to_owned(),
            field_id: first_characters(&self.field_id, FIELD_CONTEXT_FIELD_ID_MAX_CHARACTERS).to_owned(),
            before: if private {
                String::new()
            } else {
                last_characters(&self.before, FIELD_CONTEXT_MAX_CHARACTERS).to_owned()
            },
            after: if private {
                String::new()
            } else {
                first_characters(&self.after, FIELD_CONTEXT_MAX_CHARACTERS).to_owned()
            },
            role: self.role,
            selection: self.selection,
            sensitive: private,
            source: first_characters(&self.source, FIELD_CONTEXT_SOURCE_MAX_CHARACTERS).to_owned(),
        }
    }

    /// Whether the read shows an editor's input buffer instead of the text a word was typed into.
    ///
    /// VS Code with its screen-reader support off takes keys through an input it keeps empty and
    /// shows at most the key in flight: every read after `ghbdtn` held one character around the
    /// caret, the word was refused as typed into a changed field or into another word, and only
    /// `Pause` converted it. The keys the engine saw are then the only account of the text. A read
    /// with more of the word is the text itself, changed: `hbdtn` after `ghbdtn` lost its first
    /// letter, and the word stays refused there. A word as short as the buffer is told apart by
    /// its letters: a read that does not hold it is the buffer (`b` in `vscode b` read as the space
    /// in flight was refused), and a read that does is the text.
    #[must_use]
    pub fn input_buffer_only(&self, typed: &str) -> bool {
        let shown = format!("{}{}", self.before, self.after);
        shown.chars().count() <= FIELD_INPUT_BUFFER_MAX_CHARACTERS && !shown.contains(typed)
    }
}

/// A reader of the focused field, for one platform.
pub trait FieldReader: Send + Sync {
    fn read(&self, application: &str, window: i64) -> Option<FieldContext>;

    /// How the reader last fared; `available` lets the engine trust the text around a moved caret.
    fn status(&self) -> String {
        "not_requested".to_owned()
    }
}

/// Why a platform reader could not answer, as the diagnostics of a reader name it: the failure's class and, where the
/// provider gives one, its HRESULT. Numbers and names only: nothing a provider says may carry a character the user typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReaderFailure {
    /// `import_error`, `os_error`, `runtime_error` or `provider_error`.
    pub failure_type: &'static str,
    pub failure_name: &'static str,
    pub failure_code: Option<i64>,
    /// A missing module or a denied right needs a new process: asking again gains nothing.
    pub permanent: bool,
}

/// The reader of one platform as the reader of the focused field drives it (Python: `_ManagedReader`). It belongs to
/// the thread that built it, the engine's worker, so unlike `FieldReader` it says nothing about sharing.
pub trait ManagedFieldReader {
    /// The field, None for a field it cannot read, or the reason the provider did not answer.
    fn read(&self, application: &str, window: i64) -> Result<Option<FieldContext>, ReaderFailure>;
    fn close(&mut self);
}

fn first_characters(text: &str, count: usize) -> &str {
    match text.char_indices().nth(count) {
        Some((index, _)) => text.get(..index).unwrap_or_default(),
        None => text,
    }
}

fn last_characters(text: &str, count: usize) -> &str {
    if count == 0 {
        return "";
    }
    match text.char_indices().rev().nth(count - 1) {
        Some((index, _)) => text.get(index..).unwrap_or_default(),
        None => text,
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn bounded_cuts_by_characters_and_hides_a_password() {
        let long = "я".repeat(600);
        let mut field = FieldContext::new("a", "f");
        field.before = long.clone();
        field.after = long;
        let bounded = field.bounded();
        assert_eq!(bounded.before.chars().count(), 512);
        assert_eq!(bounded.after.chars().count(), 512);
        field.role = FieldRole::Password;
        let hidden = field.bounded();
        assert!(hidden.before.is_empty() && hidden.after.is_empty() && hidden.sensitive);
    }

    #[test]
    fn an_input_buffer_is_told_from_the_text() {
        let mut field = FieldContext::new("a", "f");
        field.before = " ".to_owned();
        assert!(field.input_buffer_only("b"));
        field.before = "b".to_owned();
        assert!(!field.input_buffer_only("b"));
        field.before = "hbdtn".to_owned();
        assert!(!field.input_buffer_only("ghbdtn"));
    }
}

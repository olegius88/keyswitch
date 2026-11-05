//! Read a bounded caret range through the macOS accessibility tree.
//!
//! Called on the engine worker, never inside the event tap: a slow answer there would have the system switch the tap off.
//! The accessibility tree is reached only through `AccessibilityAPI`, so what this module decides - which fields are
//! private, where the caret sits, how much text travels - is verified without a Mac. The attribute and role names are the
//! documented values of Apple's constants, read from `AXAttributeConstants.h` and `AXRoleConstants.h`.

use ks_cast::To as _;
use ks_backend::constants::FIELD_CONTEXT_MAX_CHARACTERS as CONTEXT_LIMIT;
use ks_backend::{
    FieldContext, FieldRole, ManagedFieldReader, ReaderFailure, RefusalReason, SelectedText, SelectionError, SelectionRefused,
};

use crate::constants::SOURCE_AX;

pub const ROLE_ATTRIBUTE: &str = "AXRole";
pub const SUBROLE_ATTRIBUTE: &str = "AXSubrole";
pub const VALUE_ATTRIBUTE: &str = "AXValue";
pub const SELECTED_RANGE_ATTRIBUTE: &str = "AXSelectedTextRange";
pub const IDENTIFIER_ATTRIBUTE: &str = "AXIdentifier";
pub const TEXT_FIELD_ROLE: &str = "AXTextField";
pub const TEXT_AREA_ROLE: &str = "AXTextArea";
pub const COMBO_BOX_ROLE: &str = "AXComboBox";
pub const SECURE_TEXT_FIELD_SUBROLE: &str = "AXSecureTextField";
pub const SEARCH_FIELD_SUBROLE: &str = "AXSearchField";
pub const SELECTED_TEXT_ATTRIBUTE: &str = "AXSelectedText";

/// What the engine calls the kind of field it is correcting in, for the roles the reader reads at all.
#[must_use]
pub fn field_role(role: &str) -> Option<FieldRole> {
    match role {
        TEXT_FIELD_ROLE | TEXT_AREA_ROLE | COMBO_BOX_ROLE => Some(FieldRole::Text),
        _ => None,
    }
}

/// The handful of accessibility calls the reader needs. An element is a reference the caller owns until `release`.
pub trait AccessibilityAPI {
    fn focused_element(&self) -> usize;
    fn string_attribute(&self, element: usize, name: &str) -> Option<String>;
    /// A range attribute as (location, length), counted in UTF-16 units.
    fn range_attribute(&self, element: usize, name: &str) -> Option<(i64, i64)>;
    fn release(&self, element: usize);
}

/// The accessibility calls the selection hotkeys need besides reading.
pub trait SelectionAccessibilityAPI: AccessibilityAPI {
    fn attribute_settable(&self, element: usize, name: &str) -> bool;
    fn set_string_attribute(&self, element: usize, name: &str, value: &str) -> bool;
}

/// Where in `text` an offset counted in UTF-16 code units falls, in characters.
///
/// `AXSelectedTextRange` counts as `NSString` does: an emoji or another character outside the basic plane is two units.
/// Slicing the string with that number would move the caret one character right for each such character before it. None
/// when the offset lies past the text or inside such a character.
#[must_use]
pub fn character_index(text: &str, units: i64) -> Option<usize> {
    let units = usize::try_from(units).ok()?;
    let mut counted = 0;
    for (index, character) in text.chars().enumerate() {
        if counted == units {
            return Some(index);
        }
        counted += character.len_utf16();
        if counted > units {
            return None;
        }
    }
    (counted == units).then(|| text.chars().count())
}

fn last_characters(text: &str, count: usize) -> &str {
    let total = text.chars().count();
    if total <= count {
        return text;
    }
    let start = text.char_indices().nth(total - count).map_or(text.len(), |(index, _)| index);
    &text[start..]
}

fn slice_characters(text: &str, start: usize, end: usize) -> &str {
    let from = text.char_indices().nth(start).map_or(text.len(), |(index, _)| index);
    let to = text.char_indices().nth(end).map_or(text.len(), |(index, _)| index);
    &text[from..to]
}

/// What stands on either side of the caret in the focused field.
pub struct MacFieldReader<A: AccessibilityAPI> {
    api: A,
}

impl<A: AccessibilityAPI> MacFieldReader<A> {
    pub fn new(api: A) -> Self {
        MacFieldReader { api }
    }

    pub fn read(&self, application: &str, _window: i64) -> Option<FieldContext> {
        let element = self.api.focused_element();
        if element == 0 {
            return None;
        }
        let context = self.read_element(application, element);
        self.api.release(element);
        context
    }

    fn read_element(&self, application: &str, element: usize) -> Option<FieldContext> {
        let role = self.api.string_attribute(element, ROLE_ATTRIBUTE).unwrap_or_default();
        // Anything else - a button, a list, a web view we cannot read - is reported as no context rather than as an empty
        // field, which the engine would take for a field that is genuinely empty.
        let kind = field_role(&role)?;
        let field_id = self.api.string_attribute(element, IDENTIFIER_ATTRIBUTE).filter(|id| !id.is_empty()).unwrap_or_else(|| role.clone());
        let subrole = self.api.string_attribute(element, SUBROLE_ATTRIBUTE).unwrap_or_default();
        if subrole == SECURE_TEXT_FIELD_SUBROLE {
            // A password field is named and then left alone: its text is never read, not even to be thrown away afterwards.
            let mut context = FieldContext::new(application, field_id);
            context.role = FieldRole::Password;
            context.sensitive = true;
            context.source = SOURCE_AX.to_owned();
            return Some(context.bounded());
        }
        let value = self.api.string_attribute(element, VALUE_ATTRIBUTE)?;
        let (location, length) = self.api.range_attribute(element, SELECTED_RANGE_ATTRIBUTE)?;
        let start = if location >= 0 && length >= 0 { character_index(&value, location) } else { None };
        let end = start.and_then(|_| character_index(&value, location + length));
        // A range the application computed against text it has since changed.
        let (start, end) = (start?, end?);
        let total = value.chars().count();
        let mut context = FieldContext::new(application, field_id);
        context.before = last_characters(slice_characters(&value, 0, start), CONTEXT_LIMIT).to_owned();
        context.after = slice_characters(&value, end, total.min(end + CONTEXT_LIMIT)).to_owned();
        context.role = if subrole == SEARCH_FIELD_SUBROLE { FieldRole::Search } else { kind };
        context.selection = length > 0;
        context.source = SOURCE_AX.to_owned();
        Some(context.bounded())
    }

    /// Nothing is kept open: every read starts from the focused element.
    pub fn close(&mut self) {}
}

impl<A: AccessibilityAPI> ManagedFieldReader for MacFieldReader<A> {
    fn read(&self, application: &str, window: i64) -> Result<Option<FieldContext>, ReaderFailure> {
        Ok(MacFieldReader::read(self, application, window))
    }

    fn close(&mut self) {
        MacFieldReader::close(self);
    }
}

/// Where the caret or the selection stands after the selection was written over (`MacSelectionAccess::replace`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionAfter {
    Kept,
    CaretAfter,
    Unknown,
}

/// Where the selection stands once `replacement` took the place of the one at `before`: still over it, a caret right after
/// it, or somewhere this cannot tell. The ranges count UTF-16 units, as `AXSelectedTextRange` does.
#[must_use]
pub fn selection_after(before: Option<(i64, i64)>, after: Option<(i64, i64)>, replacement: &str) -> SelectionAfter {
    let units = replacement.encode_utf16().count().to::<i64>();
    let (Some(before), Some(after)) = (before, after) else {
        return SelectionAfter::Unknown;
    };
    if after == (before.0, units) {
        SelectionAfter::Kept
    } else if after == (before.0 + units, 0) {
        SelectionAfter::CaretAfter
    } else {
        SelectionAfter::Unknown
    }
}

fn refused(reason: RefusalReason) -> SelectionError {
    SelectionRefused::new(reason).into()
}

/// The selection of the focused field, read and written through kAXSelectedTextAttribute.
///
/// Writing it replaces the selection inside the field itself, so no key is typed and any character the selection holds is
/// written as it is. A field that does not let it be written (Electron and Qt fields often) is refused here as not
/// settable, before anything is written, and the backend pastes instead.
pub trait SelectionAccess: Send + Sync {
    fn read(&self) -> Result<SelectedText, SelectionError>;
    /// The range of the selection, if the focused field still shows `selected` selected (else `focus_changed`).
    fn selected_range(&self, selected: &str) -> Result<Option<(i64, i64)>, SelectionError>;
    /// The selected range of the focused field now, or None where none is told.
    fn caret_range(&self) -> Option<(i64, i64)>;
    /// Write `replacement` over the selection, if the focused field still shows `selected` selected, and say where the
    /// selection stands then. A field that does not let the selection be written is refused as not settable; one that
    /// cannot be settled before writing changes nothing.
    fn replace(&self, selected: &str, replacement: &str) -> Result<SelectionAfter, SelectionError>;
}

pub struct MacSelectionAccess<A: SelectionAccessibilityAPI> {
    api: A,
}

impl<A: SelectionAccessibilityAPI> MacSelectionAccess<A> {
    pub fn new(api: A) -> Self {
        MacSelectionAccess { api }
    }

    fn with_focused<T>(&self, missing: RefusalReason, action: impl FnOnce(usize) -> Result<T, SelectionError>) -> Result<T, SelectionError> {
        let element = self.api.focused_element();
        if element == 0 {
            return Err(refused(missing));
        }
        let result = action(element);
        self.api.release(element);
        result
    }
}

impl<A: SelectionAccessibilityAPI + Send + Sync> SelectionAccess for MacSelectionAccess<A> {
    fn read(&self) -> Result<SelectedText, SelectionError> {
        self.with_focused(RefusalReason::Unsupported, |element| {
            let role = self.api.string_attribute(element, ROLE_ATTRIBUTE).unwrap_or_default();
            if field_role(&role).is_none() {
                return Err(refused(RefusalReason::Unsupported));
            }
            if self.api.string_attribute(element, SUBROLE_ATTRIBUTE).unwrap_or_default() == SECURE_TEXT_FIELD_SUBROLE {
                return Err(refused(RefusalReason::SensitiveField));
            }
            let Some(text) = self.api.string_attribute(element, SELECTED_TEXT_ATTRIBUTE) else {
                return Err(refused(RefusalReason::Unsupported));
            };
            if text.is_empty() {
                return Err(refused(RefusalReason::NoSelection));
            }
            Ok(SelectedText { text, source: SOURCE_AX.to_owned() })
        })
    }

    fn selected_range(&self, selected: &str) -> Result<Option<(i64, i64)>, SelectionError> {
        self.with_focused(RefusalReason::FocusChanged, |element| {
            if self.api.string_attribute(element, SELECTED_TEXT_ATTRIBUTE).as_deref() != Some(selected) {
                return Err(refused(RefusalReason::FocusChanged));
            }
            Ok(self.api.range_attribute(element, SELECTED_RANGE_ATTRIBUTE))
        })
    }

    fn caret_range(&self) -> Option<(i64, i64)> {
        let element = self.api.focused_element();
        if element == 0 {
            return None;
        }
        let range = self.api.range_attribute(element, SELECTED_RANGE_ATTRIBUTE);
        self.api.release(element);
        range
    }

    fn replace(&self, selected: &str, replacement: &str) -> Result<SelectionAfter, SelectionError> {
        self.with_focused(RefusalReason::FocusChanged, |element| {
            if self.api.string_attribute(element, SELECTED_TEXT_ATTRIBUTE).as_deref() != Some(selected) {
                return Err(refused(RefusalReason::FocusChanged));
            }
            let before = self.api.range_attribute(element, SELECTED_RANGE_ATTRIBUTE);
            if !self.api.attribute_settable(element, SELECTED_TEXT_ATTRIBUTE)
                || !self.api.set_string_attribute(element, SELECTED_TEXT_ATTRIBUTE, replacement)
            {
                return Err(refused(RefusalReason::NotSettable));
            }
            let after = self.api.range_attribute(element, SELECTED_RANGE_ATTRIBUTE);
            Ok(selection_after(before, after, replacement))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// One focused element with attributes, as the accessibility tree would answer.
    #[derive(Default)]
    struct FakeTree {
        focused: usize,
        strings: HashMap<String, String>,
        ranges: Mutex<Vec<(i64, i64)>>,
        settable: bool,
        released: Mutex<Vec<usize>>,
        written: Mutex<Vec<String>>,
    }

    impl AccessibilityAPI for &FakeTree {
        fn focused_element(&self) -> usize {
            self.focused
        }
        fn string_attribute(&self, _element: usize, name: &str) -> Option<String> {
            self.strings.get(name).cloned()
        }
        fn range_attribute(&self, _element: usize, _name: &str) -> Option<(i64, i64)> {
            let mut ranges = self.ranges.lock().unwrap();
            if ranges.len() > 1 { Some(ranges.remove(0)) } else { ranges.first().copied() }
        }
        fn release(&self, element: usize) {
            self.released.lock().unwrap().push(element);
        }
    }

    impl SelectionAccessibilityAPI for &FakeTree {
        fn attribute_settable(&self, _element: usize, _name: &str) -> bool {
            self.settable
        }
        fn set_string_attribute(&self, _element: usize, _name: &str, value: &str) -> bool {
            self.written.lock().unwrap().push(value.to_owned());
            true
        }
    }

    fn field(value: &str, caret: (i64, i64)) -> FakeTree {
        let mut tree = FakeTree { focused: 7, ..FakeTree::default() };
        tree.strings.insert(ROLE_ATTRIBUTE.into(), TEXT_AREA_ROLE.into());
        tree.strings.insert(VALUE_ATTRIBUTE.into(), value.into());
        *tree.ranges.lock().unwrap() = vec![caret];
        tree
    }

    #[test]
    fn utf16_offsets_become_character_indexes() {
        assert_eq!(character_index("привет", 3), Some(3));
        assert_eq!(character_index("a😀b", 3), Some(2));
        // Inside the pair, past the end.
        assert_eq!(character_index("a😀b", 2), None);
        assert_eq!(character_index("ab", 3), None);
        assert_eq!(character_index("ab", 2), Some(2));
        assert_eq!(character_index("ab", -1), None);
    }

    #[test]
    fn the_text_is_split_at_the_caret() {
        let tree = field("привет мир", (6, 0));
        let context = MacFieldReader::new(&tree).read("TextEdit", 0).unwrap();
        assert_eq!((context.before.as_str(), context.after.as_str()), ("привет", " мир"));
        assert_eq!((context.role, context.selection, context.source.as_str()), (FieldRole::Text, false, "ax"));
        assert_eq!(&*tree.released.lock().unwrap(), &[7]);
    }

    #[test]
    fn a_selection_is_marked_and_cut_out_of_both_sides() {
        let tree = field("привет мир", (7, 3));
        let context = MacFieldReader::new(&tree).read("TextEdit", 0).unwrap();
        assert_eq!((context.before.as_str(), context.after.as_str(), context.selection), ("привет ", "", true));
    }

    #[test]
    fn the_context_is_bounded_on_both_sides() {
        let text = "а".repeat(1400);
        let tree = field(&text, (700, 0));
        let context = MacFieldReader::new(&tree).read("Notes", 0).unwrap();
        assert_eq!((context.before.chars().count(), context.after.chars().count()), (CONTEXT_LIMIT, CONTEXT_LIMIT));
    }

    #[test]
    fn a_password_field_is_named_and_never_read() {
        let mut tree = field("secret", (0, 0));
        tree.strings.insert(SUBROLE_ATTRIBUTE.into(), SECURE_TEXT_FIELD_SUBROLE.into());
        let context = MacFieldReader::new(&tree).read("Safari", 0).unwrap();
        assert_eq!((context.role, context.sensitive, context.before.as_str()), (FieldRole::Password, true, ""));
    }

    #[test]
    fn a_search_field_is_told_apart() {
        let mut tree = field("abc", (3, 0));
        tree.strings.insert(SUBROLE_ATTRIBUTE.into(), SEARCH_FIELD_SUBROLE.into());
        assert_eq!(MacFieldReader::new(&tree).read("Finder", 0).unwrap().role, FieldRole::Search);
    }

    #[test]
    fn what_cannot_be_read_is_no_context() {
        let mut tree = field("abc", (3, 0));
        tree.strings.insert(ROLE_ATTRIBUTE.into(), "AXButton".into());
        assert!(MacFieldReader::new(&tree).read("X", 0).is_none());
        let nothing = FakeTree::default();
        assert!(MacFieldReader::new(&nothing).read("X", 0).is_none());
        let stale = field("abc", (9, 0));
        assert!(MacFieldReader::new(&stale).read("X", 0).is_none());
        let mut valueless = field("abc", (1, 0));
        valueless.strings.remove(VALUE_ATTRIBUTE);
        assert!(MacFieldReader::new(&valueless).read("X", 0).is_none());
    }

    #[test]
    fn the_field_id_falls_back_to_the_role() {
        let mut tree = field("abc", (3, 0));
        assert_eq!(MacFieldReader::new(&tree).read("X", 0).unwrap().field_id, TEXT_AREA_ROLE);
        tree.strings.insert(IDENTIFIER_ATTRIBUTE.into(), "note".into());
        assert_eq!(MacFieldReader::new(&tree).read("X", 0).unwrap().field_id, "note");
    }

    #[test]
    fn the_selection_after_a_write_is_told_from_the_ranges() {
        assert_eq!(selection_after(Some((2, 3)), Some((2, 2)), "ab"), SelectionAfter::Kept);
        assert_eq!(selection_after(Some((2, 3)), Some((4, 0)), "ab"), SelectionAfter::CaretAfter);
        assert_eq!(selection_after(Some((2, 3)), Some((9, 0)), "ab"), SelectionAfter::Unknown);
        assert_eq!(selection_after(None, Some((2, 2)), "ab"), SelectionAfter::Unknown);
        // The length is in UTF-16 units: an emoji is two.
        assert_eq!(selection_after(Some((0, 1)), Some((2, 0)), "😀"), SelectionAfter::CaretAfter);
    }

    #[test]
    fn reading_a_selection_names_why_it_refuses() {
        let reason = |tree: &FakeTree| match MacSelectionAccess::new(tree).read() {
            Err(SelectionError::Refused(refusal)) => refusal.reason,
            other => panic!("{other:?}"),
        };
        let mut tree = field("abc", (0, 1));
        assert_eq!(reason(&tree), RefusalReason::Unsupported);
        tree.strings.insert(SELECTED_TEXT_ATTRIBUTE.into(), String::new());
        assert_eq!(reason(&tree), RefusalReason::NoSelection);
        tree.strings.insert(SUBROLE_ATTRIBUTE.into(), SECURE_TEXT_FIELD_SUBROLE.into());
        assert_eq!(reason(&tree), RefusalReason::SensitiveField);
        tree.strings.remove(SUBROLE_ATTRIBUTE);
        tree.strings.insert(SELECTED_TEXT_ATTRIBUTE.into(), "abc".into());
        assert_eq!(MacSelectionAccess::new(&tree).read().unwrap().text, "abc");
        assert_eq!(reason(&FakeTree::default()), RefusalReason::Unsupported);
    }

    #[test]
    fn a_replacement_is_written_only_where_the_selection_is_unchanged_and_settable() {
        let mut tree = field("abc", (0, 3));
        tree.strings.insert(SELECTED_TEXT_ATTRIBUTE.into(), "abc".into());
        *tree.ranges.lock().unwrap() = vec![(0, 3), (0, 2)];
        let access = MacSelectionAccess::new(&tree);
        assert_eq!(access.replace("abc", "xy"), Err(refused(RefusalReason::NotSettable)));
        assert!(tree.written.lock().unwrap().is_empty());
        assert_eq!(access.replace("other", "xy"), Err(refused(RefusalReason::FocusChanged)));
        tree.settable = true;
        *tree.ranges.lock().unwrap() = vec![(0, 3), (0, 2)];
        assert_eq!(MacSelectionAccess::new(&tree).replace("abc", "xy"), Ok(SelectionAfter::Kept));
        assert_eq!(&*tree.written.lock().unwrap(), &["xy".to_owned()]);
    }
}

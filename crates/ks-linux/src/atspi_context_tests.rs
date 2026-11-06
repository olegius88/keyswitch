//! The reader over a fake accessibility bus: the tests of the Python's `AtspiContextTests`, and those of the selection path.

use super::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const WINDOW: i64 = 5;

fn app() -> NodeRef {
    NodeRef { bus_name: ":1.7".to_owned(), path: "/app".to_owned() }
}

fn field() -> NodeRef {
    NodeRef { bus_name: ":1.7".to_owned(), path: "/field".to_owned() }
}

/// The values a call answers in turn; the last one stays.
struct Queue<T>(VecDeque<T>);

impl<T: Clone> Queue<T> {
    fn of(values: &[T]) -> Self {
        Queue(values.iter().cloned().collect())
    }
    fn next(&mut self) -> T {
        if self.0.len() > 1 { self.0.pop_front().expect("not empty") } else { self.0.front().cloned().expect("a value") }
    }
}

struct Tree {
    desktop_children: usize,
    app_name: String,
    app_pid: u32,
    field_states: u64,
    role: u32,
    interfaces: Vec<&'static str>,
    selections: Queue<i32>,
    carets: Queue<i32>,
    character_count: i32,
    texts: Queue<String>,
    selection_span: Queue<(i32, i32)>,
    delete_ok: bool,
    insert_results: Queue<bool>,
    calls: Vec<String>,
    fail: bool,
    texts_asked: Vec<(i32, i32)>,
    inserted: Vec<String>,
    /// How many more times the field says it has the focus; None for always.
    focus_left: Option<usize>,
    fail_role: bool,
    /// The children of the application are slow to count: a walk runs out of its time there.
    slow_children: Duration,
    /// The application itself is slow to come: a walk runs out of its time before it begins.
    slow_application: Duration,
    no_application: bool,
    no_field: bool,
    /// The field holds itself as a child: a walk runs out of its nodes.
    endless: bool,
}

struct Fake(Mutex<Tree>);

impl Fake {
    fn new() -> Arc<Fake> {
        Arc::new(Fake(Mutex::new(Tree {
            desktop_children: 1,
            app_name: "editor".to_owned(),
            app_pid: 0,
            field_states: 1 << ATSPI_STATE_FOCUSED | 1 << ATSPI_STATE_EDITABLE,
            role: 0,
            interfaces: vec![INTERFACE_TEXT, INTERFACE_EDITABLE_TEXT],
            selections: Queue::of(&[0]),
            carets: Queue::of(&[40]),
            character_count: 60,
            texts: Queue::of(&["before x".to_owned(), " y".to_owned()]),
            selection_span: Queue::of(&[(3, 9)]),
            delete_ok: true,
            insert_results: Queue::of(&[true]),
            calls: Vec::new(),
            fail: false,
            texts_asked: Vec::new(),
            inserted: Vec::new(),
            focus_left: None,
            fail_role: false,
            slow_children: Duration::ZERO,
            slow_application: Duration::ZERO,
            no_application: false,
            no_field: false,
            endless: false,
        })))
    }

    fn with<R>(&self, work: impl FnOnce(&mut Tree) -> R) -> R {
        work(&mut self.0.lock().expect("fake lock"))
    }

    fn reader(self: &Arc<Self>, process: Option<ProcessForWindow>) -> AtspiFieldReader {
        let mut reader = AtspiFieldReader::new(Box::new(Shared(self.clone())), process);
        reader.process_name = Box::new(|_| "process".to_owned());
        reader
    }
}

struct Shared(Arc<Fake>);

fn check(tree: &Tree) -> AtspiResult<()> {
    if tree.fail { Err(AtspiError::Call("timed out".to_owned())) } else { Ok(()) }
}

impl AtspiApi for Shared {
    fn character_extents(&self, _node: &NodeRef, _offset: i32) -> AtspiResult<(i32, i32, i32, i32)> {
        Err(AtspiError::Call("character extents are not offered".to_owned()))
    }
    fn child_count(&self, node: Option<&NodeRef>) -> AtspiResult<usize> {
        if node == Some(&app()) {
            std::thread::sleep(self.0.with(|tree| tree.slow_children));
        }
        self.0.with(|tree| {
            check(tree)?;
            Ok(match node {
                None => tree.desktop_children,
                Some(node) if *node == app() => 1,
                Some(_) => usize::from(tree.endless),
            })
        })
    }
    fn child_at(&self, node: Option<&NodeRef>, _index: usize) -> AtspiResult<Option<NodeRef>> {
        let (slow, no_application, no_field) = self.0.with(|tree| (tree.slow_application, tree.no_application, tree.no_field));
        Ok(match node {
            None => {
                std::thread::sleep(slow);
                (!no_application).then(app)
            }
            Some(node) if *node == app() => (!no_field).then(field),
            Some(_) => Some(field()),
        })
    }
    fn name(&self, _node: &NodeRef) -> AtspiResult<String> {
        Ok(self.0.with(|tree| tree.app_name.clone()))
    }
    fn process_id(&self, _node: &NodeRef) -> AtspiResult<u32> {
        Ok(self.0.with(|tree| tree.app_pid))
    }
    fn states(&self, node: &NodeRef) -> AtspiResult<u64> {
        Ok(if *node == field() {
            self.0.with(|tree| match &mut tree.focus_left {
                Some(0) => 0,
                Some(left) => {
                    *left -= 1;
                    tree.field_states
                }
                None => tree.field_states,
            })
        } else {
            0
        })
    }
    fn role(&self, _node: &NodeRef) -> AtspiResult<u32> {
        self.0.with(|tree| if tree.fail_role { Err(AtspiError::Call("timed out".to_owned())) } else { Ok(tree.role) })
    }
    fn has_interface(&self, _node: &NodeRef, interface: &str) -> AtspiResult<bool> {
        Ok(self.0.with(|tree| tree.interfaces.contains(&interface)))
    }
    fn n_selections(&self, _node: &NodeRef) -> AtspiResult<i32> {
        Ok(self.0.with(|tree| tree.selections.next()))
    }
    fn caret_offset(&self, _node: &NodeRef) -> AtspiResult<i32> {
        Ok(self.0.with(|tree| tree.carets.next()))
    }
    fn character_count(&self, _node: &NodeRef) -> AtspiResult<i32> {
        Ok(self.0.with(|tree| tree.character_count))
    }
    fn get_text(&self, _node: &NodeRef, start: i32, end: i32) -> AtspiResult<String> {
        Ok(self.0.with(|tree| {
            tree.texts_asked.push((start, end));
            tree.texts.next()
        }))
    }
    fn get_selection(&self, _node: &NodeRef, _number: i32) -> AtspiResult<(i32, i32)> {
        Ok(self.0.with(|tree| tree.selection_span.next()))
    }
    fn set_selection(&self, _node: &NodeRef, number: i32, start: i32, end: i32) -> AtspiResult<bool> {
        self.0.with(|tree| tree.calls.push(format!("set_selection {number} {start} {end}")));
        Ok(true)
    }
    fn delete_text(&self, _node: &NodeRef, start: i32, end: i32) -> AtspiResult<bool> {
        self.0.with(|tree| {
            tree.calls.push(format!("delete {start} {end}"));
            Ok(tree.delete_ok)
        })
    }
    fn insert_text(&self, _node: &NodeRef, position: i32, text: &str, length: i32) -> AtspiResult<bool> {
        self.0.with(|tree| {
            tree.calls.push(format!("insert {position} {length}"));
            tree.inserted.push(text.to_owned());
            Ok(tree.insert_results.next())
        })
    }
}

fn read(fake: &Arc<Fake>) -> Result<Option<FieldContext>, ReaderFailure> {
    fake.reader(None).read("editor", WINDOW)
}

#[test]
fn the_text_around_the_caret_is_bounded_and_the_field_is_named_by_its_path() {
    let fake = Fake::new();
    let field = read(&fake).expect("reads").expect("a field");
    assert_eq!((field.before.as_str(), field.after.as_str(), field.field_id.as_str()), ("before x", " y", "5:0:0"));
    assert_eq!((field.role, field.source.as_str(), field.sensitive, field.selection), (FieldRole::Text, "atspi", false, false));
    // The text before the caret reaches 512 characters back, the text after 128 ahead, never past the end.
    assert_eq!(fake.with(|tree| tree.texts_asked.clone()), vec![(0, 40), (40, 60)]);
    fake.with(|tree| {
        tree.carets = Queue::of(&[600]);
        tree.character_count = 900;
        tree.texts_asked.clear();
    });
    read(&fake).expect("reads");
    assert_eq!(fake.with(|tree| tree.texts_asked.clone()), vec![(88, 600), (600, 728)]);
}

#[test]
fn a_password_a_selection_and_a_field_without_text_are_told_apart() {
    let fake = Fake::new();
    fake.with(|tree| tree.role = ATSPI_ROLE_PASSWORD_TEXT);
    let password = read(&fake).expect("reads").expect("a field");
    assert!(password.sensitive && password.role == FieldRole::Password && password.before.is_empty());
    assert!(fake.with(|tree| tree.texts_asked.is_empty()), "nothing of a password is read");
    fake.with(|tree| {
        tree.role = 0;
        tree.selections = Queue::of(&[1]);
    });
    let selected = read(&fake).expect("reads").expect("a field");
    assert!(selected.selection && selected.before.is_empty());
    assert!(fake.with(|tree| tree.texts_asked.is_empty()));
    fake.with(|tree| tree.interfaces = vec![]);
    assert_eq!(read(&fake), Ok(None));
}

#[test]
fn a_negative_caret_reads_nothing_and_a_moved_caret_reads_an_empty_field() {
    let fake = Fake::new();
    fake.with(|tree| tree.carets = Queue::of(&[-1]));
    assert_eq!(read(&fake), Ok(None));
    fake.with(|tree| tree.carets = Queue::of(&[40, 41]));
    let moved = read(&fake).expect("reads").expect("a field");
    assert!(moved.before.is_empty() && moved.after.is_empty() && moved.role == FieldRole::Unknown);
}

#[test]
fn a_masked_entry_is_a_password_and_a_selection_made_during_the_read_is_a_selection() {
    let fake = Fake::new();
    fake.with(|tree| tree.texts = Queue::of(&["••••".to_owned(), "••".to_owned()]));
    let masked = read(&fake).expect("reads").expect("a field");
    assert!(masked.sensitive && masked.before.is_empty() && masked.role == FieldRole::Password);
    fake.with(|tree| {
        tree.texts = Queue::of(&["before x".to_owned(), " y".to_owned()]);
        tree.selections = Queue::of(&[0, 1]);
    });
    let selected = read(&fake).expect("reads").expect("a field");
    assert!(selected.selection && selected.before.is_empty());
}

#[test]
fn the_application_is_matched_by_name_or_process_and_a_window_process_overrides_it() {
    let fake = Fake::new();
    assert_eq!(read_with_application(&fake, "elsewhere"), Ok(None));
    assert!(read_with_application(&fake, "Editor").expect("reads").is_some(), "case is folded");
    assert!(read_with_application(&fake, "PROCESS").expect("reads").is_some(), "the process name counts");
    fake.with(|tree| tree.app_pid = 4242);
    let matching: ProcessForWindow = Arc::new(|_| 4242);
    assert!(fake.reader(Some(matching)).read("OtherWMClass", WINDOW).expect("reads").is_some());
    let mismatched: ProcessForWindow = Arc::new(|_| 1);
    assert_eq!(fake.reader(Some(mismatched)).read("editor", WINDOW), Ok(None));
    fake.with(|tree| tree.desktop_children = 0);
    assert_eq!(read(&fake), Ok(None));
}

fn read_with_application(fake: &Arc<Fake>, application: &str) -> Result<Option<FieldContext>, ReaderFailure> {
    fake.reader(None).read(application, WINDOW)
}

#[test]
fn the_walk_stops_at_its_deadline() {
    let fake = Fake::new();
    let reader = fake.reader(None);
    assert_eq!(reader.focused("editor", WINDOW, Instant::now().checked_sub(Duration::from_secs(1)).unwrap()), Ok(None));
    assert!(reader.focused("editor", WINDOW, Instant::now() + Duration::from_secs(5)).expect("walks").is_some());
}

#[test]
fn a_failed_call_is_a_provider_failure_without_the_text_of_the_error() {
    let fake = Fake::new();
    fake.with(|tree| tree.fail = true);
    let failure = read(&fake).expect_err("fails");
    assert_eq!((failure.failure_type, failure.failure_name, failure.failure_code, failure.permanent), ("provider_error", "GError", None, false));
    let unavailable = AtspiError::Unavailable("no bus".to_owned()).failure();
    assert_eq!((unavailable.failure_type, unavailable.failure_name, unavailable.permanent), ("runtime_error", "RuntimeError", false));
}

// ---------------------------------------------------------------------------------------------------- selection

fn select(fake: &Arc<Fake>, reader: &AtspiFieldReader, text: &str) -> Result<SelectedText, SelectionError> {
    fake.with(|tree| {
        tree.selections = Queue::of(&[1]);
        tree.texts = Queue::of(&[text.to_owned()]);
    });
    reader.read_selection("editor", WINDOW)
}

fn reason(result: Result<SelectedText, SelectionError>) -> &'static str {
    match result {
        Err(SelectionError::Refused(refused)) => refused.reason.as_str(),
        other => panic!("not refused: {other:?}"),
    }
}

#[test]
fn a_selection_is_read_from_the_focused_editable_field() {
    let fake = Fake::new();
    let reader = fake.reader(None);
    let selected = select(&fake, &reader, "ghbdtn").expect("selected");
    assert_eq!((selected.text.as_str(), selected.source.as_str()), ("ghbdtn", "atspi"));
    assert_eq!(fake.with(|tree| tree.texts_asked.clone()), vec![(3, 9)], "the bounds come as they are");
}

#[test]
fn a_selection_that_cannot_be_changed_is_refused_with_its_reason() {
    let fake = Fake::new();
    let reader = fake.reader(None);
    fake.with(|tree| tree.desktop_children = 0);
    assert_eq!(reason(select(&fake, &reader, "x")), "unsupported");
    fake.with(|tree| tree.desktop_children = 1);
    fake.with(|tree| tree.role = ATSPI_ROLE_PASSWORD_TEXT);
    assert_eq!(reason(select(&fake, &reader, "x")), "sensitive_field");
    fake.with(|tree| tree.role = 0);
    fake.with(|tree| tree.interfaces = vec![]);
    assert_eq!(reason(select(&fake, &reader, "x")), "unsupported");
    fake.with(|tree| tree.interfaces = vec![INTERFACE_TEXT]);
    fake.with(|tree| tree.field_states = 1 << ATSPI_STATE_FOCUSED);
    assert_eq!(reason(select(&fake, &reader, "x")), "read_only");
    fake.with(|tree| tree.field_states = 1 << ATSPI_STATE_FOCUSED | 1 << ATSPI_STATE_EDITABLE);
    fake.with(|tree| tree.selections = Queue::of(&[2]));
    assert_eq!(reason(reader.read_selection("editor", WINDOW)), "multiple_selection");
    fake.with(|tree| tree.selections = Queue::of(&[0]));
    assert_eq!(reason(reader.read_selection("editor", WINDOW)), "no_selection");
    fake.with(|tree| tree.selection_span = Queue::of(&[(4, 4)]));
    assert_eq!(reason(select(&fake, &reader, "x")), "no_selection");
    fake.with(|tree| tree.selection_span = Queue::of(&[(3, 9)]));
    assert_eq!(reason(select(&fake, &reader, "•••")), "sensitive_field");
    fake.with(|tree| tree.fail = true);
    assert_eq!(reason(reader.read_selection("editor", WINDOW)), "unsupported", "a failed call is an unsupported field");
}

#[test]
fn a_long_selection_is_read_one_character_past_the_bound() {
    let fake = Fake::new();
    let reader = fake.reader(None);
    fake.with(|tree| tree.selection_span = Queue::of(&[(0, 5000)]));
    select(&fake, &reader, "x").expect("selected");
    assert_eq!(fake.with(|tree| tree.texts_asked.clone()), vec![(0, 1001)]);
}

#[test]
fn a_selection_is_written_over_through_the_field_and_selected_again() {
    let fake = Fake::new();
    let reader = fake.reader(None);
    let selected = select(&fake, &reader, "ghbdtn").expect("selected");
    fake.with(|tree| tree.calls.clear());
    assert_eq!(reader.write_selection(&selected, "привет"), Ok(Some(true)));
    // `length` is the UTF-8 byte count; the selection is 6 characters.
    assert_eq!(fake.with(|tree| tree.calls.clone()), ["delete 3 9", "insert 3 12", "set_selection 0 3 9"]);
    assert!(matches!(reader.write_selection(&selected, "x"), Err(SelectionError::Refused(_))), "a selection is written over once");
}

#[test]
fn a_write_needs_the_very_selection_that_was_read() {
    let fake = Fake::new();
    let reader = fake.reader(None);
    assert!(matches!(reader.write_selection(&SelectedText { text: "x".to_owned(), source: "atspi".to_owned() }, "y"), Err(SelectionError::Refused(_))));
    let selected = select(&fake, &reader, "ghbdtn").expect("selected");
    let other = SelectedText { text: "other".to_owned(), source: "atspi".to_owned() };
    assert!(matches!(reader.write_selection(&other, "y"), Err(SelectionError::Refused(_))));
    // The span moved, or the field lost the focus, or its text changed.
    for change in 0..3 {
        let fake = Fake::new();
        let reader = fake.reader(None);
        let selected = select(&fake, &reader, "ghbdtn").expect("selected");
        match change {
            0 => fake.with(|tree| tree.selection_span = Queue::of(&[(4, 9)])),
            1 => fake.with(|tree| tree.field_states = 1 << ATSPI_STATE_EDITABLE),
            _ => fake.with(|tree| tree.texts = Queue::of(&["changed".to_owned()])),
        }
        let refused = reader.write_selection(&selected, "y");
        assert!(matches!(&refused, Err(SelectionError::Refused(r)) if r.reason == RefusalReason::FocusChanged), "{change}: {refused:?}");
        assert!(fake.with(|tree| tree.calls.is_empty()));
    }
    // A field with no EditableText leaves the typing to the caller.
    fake.with(|tree| tree.interfaces = vec![INTERFACE_TEXT]);
    assert_eq!(reader.write_selection(&selected, "y"), Ok(None));
}

#[test]
fn a_field_that_refuses_the_delete_or_the_insert_is_left_as_it_was() {
    let fake = Fake::new();
    let reader = fake.reader(None);
    let selected = select(&fake, &reader, "ghbdtn").expect("selected");
    fake.with(|tree| tree.delete_ok = false);
    let refused = reader.write_selection(&selected, "y");
    assert!(matches!(&refused, Err(SelectionError::Refused(r)) if r.reason == RefusalReason::NotSettable));
    let fake = Fake::new();
    let reader = fake.reader(None);
    let selected = select(&fake, &reader, "ghbdtn").expect("selected");
    fake.with(|tree| tree.insert_results = Queue::of(&[false, true]));
    match reader.write_selection(&selected, "y") {
        Err(SelectionError::Failed(error)) => assert!(error.to_string().ends_with("возвращён")),
        other => panic!("{other:?}"),
    }
    assert_eq!(fake.with(|tree| tree.inserted.clone()), ["y", "ghbdtn"], "the selected text is put back");
}

#[test]
fn the_selection_source_connects_on_first_use_and_a_missing_bus_is_unsupported() {
    let source_fake = Fake::new();
    let shared = source_fake.clone();
    let process: ProcessForWindow = Arc::new(|_| 0);
    let mut source = AtspiSelectionReader::new(Box::new(move || Ok(Box::new(Shared(shared.clone())))), process.clone());
    assert_eq!(source.write_selection(&SelectedText { text: String::new(), source: String::new() }, "x"), Ok(None), "before any read");
    source_fake.with(|tree| {
        tree.selections = Queue::of(&[1]);
        tree.texts = Queue::of(&["ghbdtn".to_owned()]);
    });
    assert_eq!(source.read_selection("editor", WINDOW).expect("read").text, "ghbdtn");
    let mut offline = AtspiSelectionReader::new(Box::new(|| Err(AtspiError::Unavailable("no bus".to_owned()))), process);
    assert_eq!(reason(offline.read_selection("editor", WINDOW)), "unsupported");
}

#[test]
fn a_mask_is_all_a_hidden_entry_shows() {
    assert!(only_password_masks(" \u{2022}\u{2022} "));
    assert!(!only_password_masks(" "));
    assert!(!only_password_masks("\u{2022}a"));
}

#[test]
fn a_failed_call_says_only_its_class() {
    let text = |error| match call_failed(error) {
        SelectionError::Failed(error) => error.to_string(),
        SelectionError::Refused(_) => String::new(),
    };
    assert_eq!(text(AtspiError::Unavailable("what was typed".to_owned())), "AT-SPI недоступен");
    assert_eq!(text(AtspiError::Call("what was typed".to_owned())), "Вызов AT-SPI не удался");
}

#[test]
fn a_walk_stops_at_its_deadline_wherever_that_finds_it() {
    let future = || Instant::now() + Duration::from_millis(20);
    let slow = Duration::from_millis(60);
    let fake = Fake::new();
    let reader = fake.reader(None);
    fake.with(|tree| tree.slow_children = slow);
    assert_eq!(reader.focused("editor", WINDOW, future()), Ok(None));
    fake.with(|tree| {
        tree.slow_children = Duration::ZERO;
        tree.slow_application = slow;
    });
    assert_eq!(reader.focused("editor", WINDOW, future()), Ok(None));
    fake.with(|tree| {
        tree.slow_application = Duration::ZERO;
        tree.no_application = true;
    });
    assert_eq!(reader.focused("editor", WINDOW, future()), Ok(None));
    fake.with(|tree| {
        tree.no_application = false;
        tree.no_field = true;
    });
    assert_eq!(reader.focused("editor", WINDOW, future()), Ok(None));
}

#[test]
fn a_field_that_lost_the_focus_while_it_was_read_says_nothing_of_its_text() {
    let fake = Fake::new();
    let reader = fake.reader(None);
    // Found focused by the walk, not any more when read.
    fake.with(|tree| tree.focus_left = Some(1));
    let shown = reader.read("editor", WINDOW).expect("read").expect("a field");
    assert!(shown.before.is_empty());
    let fake = Fake::new();
    fake.with(|tree| tree.carets = Queue::of(&[40, 41]));
    let shown = fake.reader(None).read("editor", WINDOW).expect("read").expect("a field");
    assert!(shown.before.is_empty());
}

#[test]
fn a_selection_is_refused_when_the_field_will_not_say_what_it_is() {
    let fake = Fake::new();
    let reader = fake.reader(None);
    fake.with(|tree| tree.fail_role = true);
    assert_eq!(reason(select(&fake, &reader, "x")), "unsupported");
}

#[test]
fn a_selection_gone_before_the_write_is_a_changed_focus_and_the_text_that_did_not_come_back_is_said() {
    let fake = Fake::new();
    let reader = fake.reader(None);
    let selected = select(&fake, &reader, "ghbdtn").expect("selected");
    fake.with(|tree| tree.selections = Queue::of(&[0]));
    assert!(matches!(reader.write_selection(&selected, "y"), Err(SelectionError::Refused(refusal)) if refusal.reason == RefusalReason::FocusChanged));
    let fake = Fake::new();
    let reader = fake.reader(None);
    let selected = select(&fake, &reader, "ghbdtn").expect("selected");
    fake.with(|tree| tree.insert_results = Queue::of(&[false]));
    match reader.write_selection(&selected, "y") {
        Err(SelectionError::Failed(error)) => assert!(error.to_string().ends_with("не вернулся")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_reader_has_nothing_to_close_and_a_source_connects_once() {
    let fake = Fake::new();
    ManagedFieldReader::close(&mut fake.reader(None));
    let shared = fake.clone();
    let process: ProcessForWindow = Arc::new(|_| 0);
    let mut source = AtspiSelectionReader::new(Box::new(move || Ok(Box::new(Shared(shared.clone())))), process);
    fake.with(|tree| {
        tree.selections = Queue::of(&[1]);
        tree.texts = Queue::of(&["ghbdtn".to_owned()]);
    });
    let selected = source.read_selection("editor", WINDOW).expect("read");
    assert_eq!(source.read_selection("editor", WINDOW).expect("read again"), selected);
    assert!(source.write_selection(&selected, "y").is_ok());
}

#[test]
fn a_tree_that_never_ends_ends_the_walk() {
    let fake = Fake::new();
    let reader = fake.reader(None);
    fake.with(|tree| {
        tree.endless = true;
        tree.field_states = 0;
    });
    assert_eq!(reader.focused("editor", WINDOW, Instant::now() + Duration::from_secs(5)), Ok(None));
}

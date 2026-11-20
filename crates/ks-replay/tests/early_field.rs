//! The field an early switch keeps for the later letters of a word (`EarlyField`): a field that changed after it was kept
//! leaves the word to its later letters and its boundary, as a read at that letter did.
#![cfg(any(not(debug_assertions), coverage))]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ks_backend::constants::SHIFT_MASK;
use ks_backend::{FieldContext, FieldReader, KeyEvent};
use ks_config::constants::DETECTION_CONTEXT_READ_FIELD;
use ks_replay::harness::{Bundle, CaretReader, KEYS, Session};

/// The field as the editor holds it, under another field's name once `moved` is set: the focus went to another field
/// of the same window, as a page that moves it there does, with no click and no key of the user's.
struct MovingField {
    inner: CaretReader,
    moved: Arc<AtomicBool>,
    reads: Arc<AtomicUsize>,
}

impl FieldReader for MovingField {
    fn read(&self, application: &str, window: i64) -> Option<FieldContext> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let mut field = self.inner.read(application, window)?;
        if self.moved.load(Ordering::Relaxed) {
            "2".clone_into(&mut field.field_id);
        }
        Some(field)
    }

    fn status(&self) -> String {
        self.inner.status()
    }
}

fn press(session: &mut Session, english: char) {
    let key = KEYS.iter().find(|key| key.characters[0] == english).expect("a key of the US layout");
    let group = session.backend.group();
    let strings = key.strings();
    session.tap(KeyEvent {
        pressed: true,
        keycode: key.keycode,
        key_name: english.to_string(),
        character: strings[usize::try_from(group).expect("a layout of the two")].clone(),
        characters: strings,
        group,
        state: if key.shift { SHIFT_MASK } else { 0 },
        ..KeyEvent::default()
    });
}

#[test]
fn a_field_changed_after_it_was_kept_leaves_the_word_to_its_later_letters() {
    let mut bundle = Bundle::load(None);
    let mut session = bundle.session("firefox", 0, Vec::new());
    session.core.settings.write(DETECTION_CONTEXT_READ_FIELD, true, false).expect("in memory");
    let (moved, reads) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)));
    let backend = session.backend.clone();
    session.core.context_policy.reader = Some(Arc::new(MovingField { inner: CaretReader { backend }, moved: moved.clone(), reads }));
    // `dctu` of `dctulf` (`всегда`, which the early switch converts at its sixth letter): the first check reads the field
    // and keeps it.
    for english in "dctu".chars() {
        press(&mut session, english);
        session.idle(0.12);
    }
    assert!(session.core.early_field.is_some(), "the field the first check read");
    // The focus moves to another field of the window: the correction finds another field than the kept one, and the
    // word goes on to its boundary, which converts it.
    moved.store(true, Ordering::Relaxed);
    for english in "lf ".chars() {
        press(&mut session, english);
        session.idle(0.12);
    }
    session.idle(1.0);
    assert_eq!(session.backend.text().iter().collect::<String>(), "всегда ");
    bundle.give_back(session);
    bundle.clean();
}

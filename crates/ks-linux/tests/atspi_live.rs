//! The reader against a real GTK4 entry on a real accessibility bus. It runs only when the harness sets
//! `KS_LIVE_ATSPI_STAGE` (0: caret in text, 1: a selection, 2: a hidden entry) and a window with the focus exists;
//! otherwise it says so and passes.

use std::time::{Duration, Instant};

use ks_backend::{FieldContext, FieldRole, InputBackend, ManagedFieldReader};
use ks_linux::atspi_bus;
use ks_linux::atspi_context::AtspiFieldReader;
use ks_linux::x11_backend::X11Backend;
use ks_linux::x11_native;

const PREFIX: &str = "ранее вставлено ghbdtn";
const SUFFIX: &str = " после курсора";

fn ready(stage: u32, field: &FieldContext) -> bool {
    match stage {
        0 => field.before == PREFIX && field.after == SUFFIX,
        1 => field.selection,
        _ => field.sensitive,
    }
}

#[test]
fn the_focused_gtk_entry_is_read_through_the_accessibility_bus() {
    let Some(stage) = std::env::var("KS_LIVE_ATSPI_STAGE").ok().and_then(|text| text.parse::<u32>().ok()) else {
        eprintln!("KS_LIVE_ATSPI_STAGE is not set: no live accessibility bus to read, skipped");
        return;
    };
    let backend = X11Backend::new(Box::new(x11_native::connect), None);
    backend.open().expect("the display opens");
    let reader = AtspiFieldReader::new(atspi_bus::connect().expect("the accessibility bus connects"), Some(backend.process_for_window()));
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last = None;
    while Instant::now() < deadline {
        if let Some(focus) = backend.focused_window() {
            // Deliberately not the accessible app name: the X11 process of the focused window finds the application.
            match reader.read("DifferentWMClass", focus.window) {
                Ok(Some(field)) if ready(stage, &field) => {
                    println!("AT_SPI_STAGE_{stage}_OK role={} source={} id={}", field.role.as_str(), field.source, field.field_id);
                    println!("ACTIVE_APPLICATION {:?} window={} own={}", backend.active_application(), focus.window, focus.own);
                    assert!(stage != 1 || (field.before.is_empty() && field.after.is_empty()));
                    assert!(stage != 2 || (field.role == FieldRole::Password && field.before.is_empty() && field.after.is_empty()));
                    return;
                }
                other => last = Some(format!("{other:?}").chars().take(200).collect::<String>()),
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("no field of stage {stage} within the deadline; the last answer: {last:?}");
}

#[test]
fn a_missing_bus_is_a_runtime_failure_that_does_not_stop_the_next_attempt() {
    let Some(_) = std::env::var_os("KS_LIVE_ATSPI_UNAVAILABLE") else {
        eprintln!("KS_LIVE_ATSPI_UNAVAILABLE is not set, skipped");
        return;
    };
    // The harness points AT_SPI_BUS_ADDRESS at a socket that does not exist.
    for _ in 0..2 {
        let failure = atspi_bus::connect().err().expect("no bus").failure();
        assert_eq!((failure.failure_type, failure.failure_name, failure.permanent), ("runtime_error", "RuntimeError", false));
    }
}

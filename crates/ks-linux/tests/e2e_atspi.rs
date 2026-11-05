//! The field reader against a real GTK 4 entry on a real accessibility bus: the text around the caret, a selection, and
//! a password field, which must say nothing. Runs under `packaging/e2e.sh`, with `KS_E2E` set; otherwise it says so and
//! passes.

mod support;

use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use ks_backend::{FieldContext, FieldRole, InputBackend, ManagedFieldReader};
use ks_linux::atspi_bus;
use ks_linux::atspi_context::AtspiFieldReader;
use ks_linux::x11_backend::X11Backend;
use ks_linux::x11_native;
use support::{pump, pump_until};

const PREFIX: &str = "ранее вставлено ghbdtn";
const SUFFIX: &str = " после курсора";
const STAGES: u32 = 3;
const PASSWORD_STAGE: u32 = 2;
const SELECTION_CHARACTERS: i32 = 3;
const INITIAL_PREPARE: u64 = 600;
const PREPARE_SETTLE: u64 = 300;
const START_READ: u64 = 100;
const READ_DEADLINE: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(100);
const ANSWER_WAIT: u64 = 15_000;
const MISSING_BUS: &str = "KS_E2E_MISSING_BUS";

fn ready(stage: u32, field: &FieldContext) -> bool {
    match stage {
        0 => field.before == PREFIX && field.after == SUFFIX,
        1 => field.selection,
        _ => field.sensitive,
    }
}

/// With a missing bus the reader fails as a runtime failure that does not stop the next attempt, and the process lives.
/// Run in a process of its own, whose environment names a bus that does not exist.
fn missing_bus() -> Result<(), String> {
    for _ in 0..2 {
        let Err(error) = atspi_bus::connect() else {
            return Err("a missing bus connected".to_owned());
        };
        let failure = error.failure();
        if (failure.failure_type, failure.failure_name, failure.permanent) != ("runtime_error", "RuntimeError", false) {
            return Err(format!("a missing bus is reported as {}", failure.failure_name));
        }
    }
    Ok(())
}

fn unavailable_bus() -> Result<(), String> {
    let program = std::env::current_exe().map_err(|error| error.to_string())?;
    let status = std::process::Command::new(program)
        .env("AT_SPI_BUS_ADDRESS", "unix:path=/nonexistent/keyswitch-missing-bus")
        .env(MISSING_BUS, "1")
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("the reader with a missing bus ended with {status}"));
    }
    println!("AT_SPI_UNAVAILABLE_OK");
    Ok(())
}

fn run() -> Result<(), String> {
    unavailable_bus()?;
    gtk::init().map_err(|error| error.to_string())?;
    let backend = std::sync::Arc::new(X11Backend::new(Box::new(x11_native::connect), None));
    backend.open().map_err(|error| format!("{error:?}"))?;
    let window = gtk::Window::builder().title("KeySwitch accessibility E2E").build();
    let entry = gtk::Entry::new();
    window.set_child(Some(&entry));
    window.present();
    entry.grab_focus();
    pump(INITIAL_PREPARE);
    for stage in 0..STAGES {
        entry.grab_focus();
        entry.set_visibility(stage != PASSWORD_STAGE);
        entry.set_text(&format!("{PREFIX}{SUFFIX}"));
        // Focus and an old selection owner can queue a clear: it drains before the selection under test is made.
        pump(PREPARE_SETTLE);
        entry.set_position(PREFIX.chars().count() as i32);
        if stage == 1 {
            entry.select_region(0, SELECTION_CHARACTERS);
        }
        pump(START_READ);
        let (sender, receiver) = channel();
        let reading = backend.clone();
        std::thread::spawn(move || {
            let answer = (|| -> Result<FieldContext, String> {
                let reader = AtspiFieldReader::new(atspi_bus::connect().map_err(|error| format!("{error:?}"))?, Some(reading.process_for_window()));
                let deadline = Instant::now() + READ_DEADLINE;
                let mut last = String::new();
                while Instant::now() < deadline {
                    if let Some(focus) = reading.focused_window() {
                        // Deliberately not the accessible application's name: the X11 process of the window finds it.
                        match reader.read("DifferentWMClass", focus.window) {
                            Ok(Some(field)) if ready(stage, &field) => return Ok(field),
                            other => last = format!("{other:?}").chars().take(200).collect(),
                        }
                    }
                    // Provider notifications are asynchronous: wait for the new state, not an older valid reply.
                    std::thread::sleep(POLL);
                }
                Err(format!("no field within the deadline; the last answer: {last}"))
            })();
            let _ = sender.send(answer);
        });
        let mut answer = None;
        pump_until(ANSWER_WAIT, || {
            answer = receiver.try_recv().ok();
            answer.is_some()
        });
        let field = answer.ok_or("the reader did not answer")?.map_err(|error| format!("{error}; the entry's selection is {:?}", entry.selection_bounds()))?;
        let holds = match stage {
            0 => field.before == PREFIX && field.after == SUFFIX,
            1 => field.selection && field.before.is_empty() && field.after.is_empty(),
            _ => field.sensitive && field.role == FieldRole::Password && field.before.is_empty() && field.after.is_empty(),
        };
        if !holds {
            return Err(format!("stage {stage}: {field:?}"));
        }
        println!("AT_SPI_STAGE_{stage}_OK");
    }
    window.destroy();
    backend.close_backend();
    Ok(())
}

fn main() {
    if std::env::var_os(MISSING_BUS).is_some() {
        if let Err(error) = missing_bus() {
            println!("{error}");
            std::process::exit(1);
        }
        return;
    }
    if std::env::var_os("KS_E2E").is_none() {
        println!("KS_E2E is not set: no private accessibility bus, skipped");
        return;
    }
    match run() {
        Ok(()) => println!("AT_SPI_E2E_OK"),
        Err(error) => {
            println!("AT_SPI_FAILED: {error}");
            std::process::exit(1);
        }
    }
}

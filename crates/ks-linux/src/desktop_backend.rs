//! The keyboard backend the windows work with: X11 or Wayland, chosen by the session.

use std::sync::Arc;

use ks_backend::{BackendError, BackendProbe, FocusInfo, InputBackend, Injection, KeyFilter, KeyListener};

use crate::linux_app::new_backend;
use crate::wayland_desktop::wayland_session_active;
use crate::x11_backend::ProcessForWindow;

/// The backend and what the windows need of it besides the engine's `InputBackend`.
#[derive(Clone)]
pub struct DesktopBackend {
    pub input: Arc<dyn InputBackend>,
    pub process_for_window: ProcessForWindow,
    /// Puts the window with a native id at a screen position.
    pub position_window: Arc<dyn Fn(i64, i32, i32) -> bool + Send + Sync>,
    pub wayland: bool,
}

fn x11_backend() -> DesktopBackend {
    let backend = new_backend();
    let process_for_window = backend.process_for_window();
    let placer = backend.clone();
    DesktopBackend {
        input: backend,
        process_for_window,
        position_window: Arc::new(move |window, x, y| placer.position_window(window, x, y)),
        wayland: false,
    }
}

/// The Wayland backend; a session where it cannot be built gets the reason from `new_desktop_backend`.
fn wayland_backend() -> Result<DesktopBackend, String> {
    crate::wayland_native::new_wayland_backend(Some(crate::linux_app::selection_factory())).map(desktop_of_wayland)
}

/// The window's view of a Wayland backend.
#[must_use]
pub fn desktop_of_wayland(backend: Arc<crate::wayland_backend::WaylandBackend>) -> DesktopBackend {
    let process_for_window = backend.process_for_window();
    let placer = backend.clone();
    DesktopBackend {
        input: backend,
        process_for_window,
        position_window: Arc::new(move |window, x, y| placer.position_window(window, i64::from(x), i64::from(y))),
        wayland: true,
    }
}

/// The backend of this session. A Wayland session whose backend cannot be built gets one that reports why.
#[must_use]
pub fn new_desktop_backend() -> DesktopBackend {
    if !wayland_session_active() {
        return x11_backend();
    }
    desktop_or_reason(wayland_backend())
}

/// A Wayland session's backend, or one that says why there is none.
fn desktop_or_reason(built: Result<DesktopBackend, String>) -> DesktopBackend {
    built.unwrap_or_else(|reason| DesktopBackend {
        input: Arc::new(UnavailableBackend { reason }),
        process_for_window: Arc::new(|_| 0),
        position_window: Arc::new(|_, _, _| false),
        wayland: true,
    })
}

/// A backend that cannot start: it answers every call with the reason.
pub struct UnavailableBackend {
    pub reason: String,
}

impl UnavailableBackend {
    fn failure(&self) -> BackendError {
        BackendError::Failed(self.reason.clone())
    }
}

impl InputBackend for UnavailableBackend {
    fn start(&self, _listener: KeyListener) -> Result<(), BackendError> {
        Err(self.failure())
    }
    fn set_key_filter(&self, _predicate: Option<KeyFilter>) {}
    fn hold_input(&self) {}
    fn release_input(&self) -> Result<i32, BackendError> {
        Err(self.failure())
    }
    fn complete_action(&self, _deliver: bool) -> Result<i32, BackendError> {
        Err(self.failure())
    }
    fn stop(&self) {}
    fn close(&self) {}
    fn current_group(&self) -> Result<i32, BackendError> {
        Err(self.failure())
    }
    fn switch_group(&self, _group: i32) -> Result<(), BackendError> {
        Err(self.failure())
    }
    fn active_application(&self) -> String {
        String::new()
    }
    fn focused_window(&self) -> Option<FocusInfo> {
        None
    }
    fn probe(&self) -> BackendProbe {
        BackendProbe {
            session_type: std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "wayland".to_owned()),
            display: std::env::var("WAYLAND_DISPLAY").unwrap_or_default(),
            error: self.reason.clone(),
            current_group: -1,
            ..BackendProbe::default()
        }
    }
    fn inject_correction(&self, _injection: Injection<'_>) -> Result<i32, BackendError> {
        Err(self.failure())
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn a_session_without_its_backend_gets_one_that_says_why() {
        let desktop = desktop_or_reason(Err("why not".to_owned()));
        assert!(desktop.wayland);
        assert_eq!((desktop.process_for_window)(5), 0);
        assert!(!(desktop.position_window)(5, 1, 2));
        assert_eq!(desktop.input.probe().error, "why not");
    }

    #[test]
    fn an_unavailable_backend_says_why() {
        let backend = UnavailableBackend { reason: "нет".to_owned() };
        let probe = backend.probe();
        assert!(!probe.available && probe.error == "нет" && probe.current_group == -1);
        assert!(backend.start(Arc::new(|_| {})).is_err());
        backend.set_key_filter(None);
        backend.hold_input();
        backend.stop();
        backend.close();
        for failure in [backend.release_input().err(), backend.complete_action(true).err(), backend.current_group().err(), backend.switch_group(0).err()] {
            assert_eq!(failure.map(|error| error.to_string()).as_deref(), Some("нет"));
        }
        assert_eq!(backend.active_application(), "");
        assert_eq!(backend.focused_window(), None);
        let injection = Injection { strokes: &[], target_group: 0, boundary: None, source_group: None, late: &[], trailing: &[], kept_tail: 0, erased: None };
        assert!(backend.inject_correction(injection).is_err());
    }
}

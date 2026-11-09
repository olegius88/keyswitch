//! Cinnamon keeps the layout group itself and puts back at once any group another client locks: the X11 backend asks its
//! D-Bus service for the layout instead (`org.Cinnamon.GetInputSources` and `ActivateInputSourceIndex`).

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::blocking::connection::Builder;

use crate::constants::{
    CINNAMON_ACTIVATE_INPUT_SOURCE, CINNAMON_BUS_NAME, CINNAMON_DBUS_CALL_TIMEOUT_MS, CINNAMON_GET_INPUT_SOURCES, CINNAMON_INTERFACE,
    CINNAMON_OBJECT_PATH,
};
use crate::x11_backend::{DesktopLayouts, X11Backend, cinnamon_source_for_group, is_cinnamon_desktop};

/// One input source as `GetInputSources` gives it: the kind (`xkb`, `ibus`), the id and the index first.
type InputSource = (String, String, i32, String, String, String, String, String, String, String, i32, bool);

/// The layouts of a running Cinnamon, over the session bus opened on first use.
#[derive(Default)]
pub struct CinnamonLayouts {
    connection: Mutex<Option<Connection>>,
}

impl CinnamonLayouts {
    fn connection(&self) -> Option<Connection> {
        let mut connection = self.connection.lock().unwrap_or_else(PoisonError::into_inner);
        if connection.is_none() {
            *connection = Builder::session().and_then(|builder| builder.method_timeout(Duration::from_millis(CINNAMON_DBUS_CALL_TIMEOUT_MS)).build()).ok();
        }
        connection.clone()
    }

    fn sources(connection: &Connection) -> Option<Vec<(String, i32)>> {
        let reply = connection.call_method(Some(CINNAMON_BUS_NAME), CINNAMON_OBJECT_PATH, Some(CINNAMON_INTERFACE), CINNAMON_GET_INPUT_SOURCES, &()).ok()?;
        let (sources,): (Vec<InputSource>,) = reply.body().deserialize().ok()?;
        Some(sources.into_iter().map(|source| (source.0, source.2)).collect())
    }
}

impl DesktopLayouts for CinnamonLayouts {
    fn activate(&self, group: u8) -> bool {
        let Some(connection) = self.connection() else {
            return false;
        };
        let Some(index) = Self::sources(&connection).and_then(|sources| cinnamon_source_for_group(&sources, group)) else {
            return false;
        };
        connection.call_method(Some(CINNAMON_BUS_NAME), CINNAMON_OBJECT_PATH, Some(CINNAMON_INTERFACE), CINNAMON_ACTIVATE_INPUT_SOURCE, &(index,)).is_ok()
    }
}

/// In a Cinnamon session, the backend switches the layout through Cinnamon.
pub fn attach(backend: &X11Backend) {
    if std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| is_cinnamon_desktop(&desktop)) {
        backend.use_desktop_layouts(Arc::new(CinnamonLayouts::default()));
    }
}

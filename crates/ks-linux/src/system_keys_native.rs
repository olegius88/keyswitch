//! Where the desktop keeps its XKB options, read from the system: the X server's `_XKB_RULES_NAMES` and GNOME's settings.
//! `system_keys` makes sense of them; the live tests run this file (`packaging/coverage-native.txt`).

use gtk::gio;
use gtk::prelude::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

use crate::constants::{GNOME_INPUT_SOURCES_SCHEMA, GNOME_XKB_OPTIONS_KEY, XKB_RULES_NAMES_ATOM, XKB_RULES_NAMES_MAX_WORDS};

/// The value of `_XKB_RULES_NAMES` on the root window of the X server `DISPLAY` names; None without an X server or the
/// property.
#[must_use]
pub fn x11_rules_names() -> Option<Vec<u8>> {
    let display = std::env::var("DISPLAY").ok().filter(|display| !display.is_empty())?;
    let (connection, screen) = RustConnection::connect(Some(&display)).ok()?;
    let root = connection.setup().roots.get(screen)?.root;
    let atom = connection.intern_atom(true, XKB_RULES_NAMES_ATOM).ok()?.reply().ok()?.atom;
    let reply = connection.get_property(false, root, atom, AtomEnum::STRING, 0, XKB_RULES_NAMES_MAX_WORDS).ok()?.reply().ok()?;
    Some(reply.value)
}

/// GNOME's `xkb-options`; None where its settings are not installed.
#[must_use]
pub fn gnome_xkb_options() -> Option<Vec<String>> {
    let schema = gio::SettingsSchemaSource::default()?.lookup(GNOME_INPUT_SOURCES_SCHEMA, true)?;
    if !schema.has_key(GNOME_XKB_OPTIONS_KEY) {
        return None;
    }
    let settings = gio::Settings::new(GNOME_INPUT_SOURCES_SCHEMA);
    Some(settings.strv(GNOME_XKB_OPTIONS_KEY).iter().map(ToString::to_string).collect())
}

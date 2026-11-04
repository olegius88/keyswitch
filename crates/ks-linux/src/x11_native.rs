//! The X server reached with `x11rb`: the Python's ctypes calls into libX11, libXtst and libxkbcommon, spoken as protocol.
//!
//! Two connections are opened, as the Python opens two displays: the control connection sends requests, and the data
//! connection only receives the RECORD stream (a blocked read on it must not hold the control one). Every call that can
//! fail returns the failure as a value; nothing here panics on a server's answer.

use std::sync::{Arc, Mutex, PoisonError};

use x11rb::connection::Connection;
use x11rb::protocol::record::{self, ConnectionExt as _};
use x11rb::protocol::xkb::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self, AtomEnum, ConnectionExt as _};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use crate::constants::{X11_BUTTON_PRESS, X11_KEY_PRESS, X11_KEY_RELEASE, XKB_USE_CORE_KBD, XRECORD_ALL_CLIENTS};
use crate::x11_backend::{RecordChunk, X11Api, X11Error, XkbState};
use crate::xkb_keymap::{KeyMap, KeySyms, KeyType, TypeEntry};

/// Opens the display named by `DISPLAY`, twice, and checks that XKB answers.
pub fn connect() -> Result<Arc<dyn X11Api>, X11Error> {
    let display = std::env::var("DISPLAY").unwrap_or_default();
    if display.is_empty() {
        return Err(X11Error::Failed("Переменная DISPLAY не задана".to_owned()));
    }
    let open = || RustConnection::connect(Some(&display));
    let (Ok((control, screen)), Ok((record, _))) = (open(), open()) else {
        return Err(X11Error::Failed(format!("Не удалось открыть X11 display {display}")));
    };
    let unavailable = || X11Error::Failed("Расширение XKB недоступно или несовместимо".to_owned());
    let used = control.xkb_use_extension(1, 0).map_err(|_| unavailable())?.reply().map_err(|_| unavailable())?;
    if !used.supported {
        return Err(unavailable());
    }
    // A change of the keyboard map makes the cached one stale (libX11 refreshes its own the same way).
    let selected = control
        .xkb_select_events(
        XKB_USE_CORE_KBD,
        0u16.into(),
        xkb::EventType::NEW_KEYBOARD_NOTIFY | xkb::EventType::MAP_NOTIFY,
        0u16.into(),
        0u16.into(),
        &xkb::SelectEventsAux::default(),
    )
        .is_ok();
    if !selected {
        return Err(unavailable());
    }
    Ok(Arc::new(XcbApi {
        control,
        record,
        screen,
        xkb_version: format!("{}.{}", used.server_major, used.server_minor),
        keymap: Mutex::new(None),
        pid_atom: Mutex::new(None),
    }))
}

pub struct XcbApi {
    control: RustConnection,
    record: RustConnection,
    screen: usize,
    xkb_version: String,
    keymap: Mutex<Option<Arc<KeyMap>>>,
    pid_atom: Mutex<Option<u32>>,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl XcbApi {
    fn root_window(&self) -> u32 {
        self.control.setup().roots.get(self.screen).map_or(0, |screen| screen.root)
    }

    /// The keyboard map, read again after the server announced a change.
    fn keymap(&self) -> Option<Arc<KeyMap>> {
        let mut stale = false;
        while let Ok(Some(_event)) = self.control.poll_for_event() {
            stale = true;
        }
        let mut cached = locked(&self.keymap);
        if stale {
            *cached = None;
        }
        if cached.is_none() {
            *cached = self.read_keymap().map(Arc::new);
        }
        cached.clone()
    }

    fn read_keymap(&self) -> Option<KeyMap> {
        let setup = self.control.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let device = XKB_USE_CORE_KBD;
        let count = self
            .control
            .xkb_get_map(device, xkb::MapPart::KEY_TYPES, 0u16.into(), 0, 0, min, 0, 0, 0, 0, 0, 0u16.into(), 0, 0, 0, 0, 0, 0)
            .ok()?
            .reply()
            .ok()?
            .total_types;
        let parts = xkb::MapPart::KEY_TYPES | xkb::MapPart::KEY_SYMS;
        let reply = self
            .control
            .xkb_get_map(device, parts, 0u16.into(), 0, count, min, max - min + 1, 0, 0, 0, 0, 0u16.into(), 0, 0, 0, 0, 0, 0)
            .ok()?
            .reply()
            .ok()?;
        let types = reply
            .map
            .types_rtrn?
            .into_iter()
            .map(|key_type| KeyType {
                mods_mask: u16::from(key_type.mods_mask) as u8,
                num_levels: key_type.num_levels,
                entries: key_type
                    .map
                    .iter()
                    .map(|entry| TypeEntry { active: entry.active, mods_mask: u16::from(entry.mods_mask) as u8, level: entry.level })
                    .collect(),
            })
            .collect();
        let keys = reply
            .map
            .syms_rtrn?
            .into_iter()
            .map(|map| KeySyms { kt_index: map.kt_index, group_info: map.group_info, width: map.width, syms: map.syms })
            .collect();
        Some(KeyMap { min_keycode: min, max_keycode: max, types, keys })
    }

    fn pid_atom(&self) -> u32 {
        let mut atom = locked(&self.pid_atom);
        *atom.get_or_insert_with(|| {
            self.control
                .intern_atom(true, b"_NET_WM_PID")
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .map_or(0, |reply| reply.atom)
        })
    }
}

impl X11Api for XcbApi {
    fn xkb_version(&self) -> String {
        self.xkb_version.clone()
    }

    fn record_version(&self) -> Result<(u16, u16), String> {
        let reply = self.control.record_query_version(1, 13).map_err(|error| error.to_string())?.reply().map_err(|error| error.to_string())?;
        Ok((reply.major_version, reply.minor_version))
    }

    fn xtest_version(&self) -> Result<(u8, u16), String> {
        let reply = self.control.xtest_get_version(2, 2).map_err(|error| error.to_string())?.reply().map_err(|error| error.to_string())?;
        Ok((reply.major_version, reply.minor_version))
    }

    fn record_create_context(&self) -> Result<u32, String> {
        let context = self.control.generate_id().map_err(|error| error.to_string())?;
        let range = record::Range {
            device_events: record::Range8 { first: X11_KEY_PRESS, last: X11_BUTTON_PRESS },
            ..record::Range::default()
        };
        self.control
            .record_create_context(context, 0, &[XRECORD_ALL_CLIENTS], &[range])
            .map_err(|error| error.to_string())?
            .check()
            .map_err(|error| error.to_string())?;
        Ok(context)
    }

    fn record_enable(&self, context: u32, sink: &mut dyn FnMut(RecordChunk)) -> Result<(), String> {
        for reply in self.record.record_enable_context(context).map_err(|error| error.to_string())? {
            let reply = reply.map_err(|error| error.to_string())?;
            sink(RecordChunk { category: reply.category, client_swapped: reply.client_swapped, data: reply.data });
        }
        Ok(())
    }

    fn record_disable(&self, context: u32) {
        let _ = self.control.record_disable_context(context);
        let _ = self.control.flush();
    }

    fn record_free(&self, context: u32) {
        let _ = self.control.record_free_context(context);
        let _ = self.control.flush();
    }

    fn sync(&self) {
        if let Ok(cookie) = self.control.get_input_focus() {
            let _ = cookie.reply();
        }
    }

    fn flush(&self) {
        let _ = self.control.flush();
    }

    fn keysym(&self, keycode: u8, group: u8, level: u8) -> u32 {
        self.keymap().map_or(0, |map| map.keysym(keycode, group, level))
    }

    fn lookup_keysym(&self, keycode: u8, state: u16) -> Option<u32> {
        self.keymap()?.lookup_keysym(keycode, state)
    }

    fn keycode_for_keysym(&self, keysym: u32) -> u8 {
        self.keymap().map_or(0, |map| map.keycode_for_keysym(keysym))
    }

    fn xkb_state(&self) -> Option<XkbState> {
        let reply = self.control.xkb_get_state(XKB_USE_CORE_KBD).ok()?.reply().ok()?;
        Some(XkbState { group: u8::from(reply.group), locked_mods: u16::from(reply.locked_mods) as u8 })
    }

    fn lock_group(&self, group: u8) -> bool {
        self.control
            .xkb_latch_lock_state(XKB_USE_CORE_KBD, 0u16.into(), 0u16.into(), true, xkb::Group::from(group), 0u16.into(), false, 0)
            .is_ok()
    }

    fn fake_key(&self, pressed: bool, keycode: u8) -> bool {
        let kind = if pressed { X11_KEY_PRESS } else { X11_KEY_RELEASE };
        self.control.xtest_fake_input(kind, keycode, 0, 0, 0, 0, 0).is_ok()
    }

    fn grab_control(&self, impervious: bool) {
        let _ = self.control.xtest_grab_control(impervious);
    }

    fn input_focus(&self) -> Option<u32> {
        Some(self.control.get_input_focus().ok()?.reply().ok()?.focus)
    }

    fn pointer(&self) -> Option<(i32, i32)> {
        let reply = self.control.query_pointer(self.root_window()).ok()?.reply().ok()?;
        reply.same_screen.then_some((i32::from(reply.root_x), i32::from(reply.root_y)))
    }

    fn parent_window(&self, window: u32) -> Option<u32> {
        Some(self.control.query_tree(window).ok()?.reply().ok()?.parent)
    }

    fn window_pid(&self, window: u32) -> u32 {
        let atom = self.pid_atom();
        if atom == 0 {
            return 0;
        }
        let Some(reply) = self.control.get_property(false, window, atom, AtomEnum::CARDINAL, 0, 1).ok().and_then(|cookie| cookie.reply().ok()) else {
            return 0;
        };
        match reply.value.get(..4) {
            Some(bytes) if reply.format == 32 && reply.value_len >= 1 => u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            _ => 0,
        }
    }

    fn class_hint(&self, window: u32) -> Option<(String, String)> {
        let reply = self.control.get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256).ok()?.reply().ok()?;
        if reply.type_ != u32::from(AtomEnum::STRING) || reply.format != 8 || reply.value.is_empty() {
            return None;
        }
        let mut parts = reply.value.split(|byte| *byte == 0);
        let resource_name = String::from_utf8_lossy(parts.next().unwrap_or_default()).into_owned();
        let class_name = String::from_utf8_lossy(parts.next().unwrap_or_default()).into_owned();
        Some((resource_name, class_name))
    }

    fn set_input_focus(&self, window: u32) {
        let _ = self.control.set_input_focus(xproto::InputFocus::PARENT, window, 0u32);
    }

    fn move_raise(&self, window: u32, x: i32, y: i32) {
        let aux = xproto::ConfigureWindowAux::new().x(x).y(y).stack_mode(xproto::StackMode::ABOVE);
        let _ = self.control.configure_window(window, &aux);
    }
}

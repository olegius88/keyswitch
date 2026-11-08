//! Pure protocol framing and state parsers for Wayland desktop compositors (the Python's `wayland_desktop.py`).
//!
//! Every function here is free of live sockets and D-Bus connections so GNOME Shell, KDE Plasma 6 (`KWin`) and sway wire
//! formats are verified deterministically.

use ks_cast::To as _;
use std::collections::{HashMap, VecDeque};

use serde_json::Value;
use ks_text::pyjson::{JsonOptions, json_dumps};
use ks_text::pystr;

use crate::wayland_constants::{KDE_WINDOW_ID_CACHE_LIMIT, KDE_WINDOW_ID_START, SWAY_IPC_HEADER_BYTES};

pub const SWAY_IPC_MAGIC: &[u8; 6] = b"i3-ipc";

pub const GNOME_EXTENSION_UUID: &str = "keyswitch@io.github.olegius88.KeySwitch";
pub const GNOME_DBUS_NAME: &str = "org.gnome.Shell";
pub const GNOME_DBUS_PATH: &str = "/io/github/olegius88/KeySwitch/Gnome";
pub const GNOME_DBUS_INTERFACE: &str = "io.github.olegius88.KeySwitch.Gnome";
pub const GNOME_EXTENSIONS_BUS_NAME: &str = "org.gnome.Shell.Extensions";
pub const GNOME_EXTENSIONS_PATH: &str = "/org/gnome/Shell/Extensions";
pub const GNOME_EXTENSIONS_INTERFACE: &str = "org.gnome.Shell.Extensions";

pub const KDE_KEYBOARD_BUS_NAME: &str = "org.kde.keyboard";
pub const KDE_KEYBOARD_PATH: &str = "/Layouts";
pub const KDE_KEYBOARD_INTERFACE: &str = "org.kde.KeyboardLayouts";

pub const KWIN_BUS_NAME: &str = "org.kde.KWin";
pub const KWIN_SCRIPTING_PATH: &str = "/Scripting";
pub const KWIN_SCRIPTING_INTERFACE: &str = "org.kde.kwin.Scripting";
pub const KWIN_SCRIPT_INTERFACE: &str = "org.kde.kwin.Script";

/// A JSON text could not be read as the object a parser expects (the Python's `ValueError`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopParseError(pub String);

/// Focused top-level window metadata reported by a Wayland compositor.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DesktopWindowInfo {
    pub window_id: i64,
    pub application: String,
    pub pid: i64,
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
    /// The keyboard is in a field that hides what is typed, which the compositor knows and AT-SPI cannot tell (a password
    /// prompt of GNOME Shell).
    pub password: bool,
}

impl DesktopWindowInfo {
    #[must_use]
    pub fn center(&self) -> Option<(i64, i64)> {
        if self.width <= 0 || self.height <= 0 {
            return None;
        }
        Some((self.x + self.width.div_euclid(2), self.y + self.height.div_euclid(2)))
    }
}

/// Snapshot of layouts, active window, pointer and lock state.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DesktopSnapshot {
    pub layouts: Vec<String>,
    pub current_layout: i64,
    pub window: Option<DesktopWindowInfo>,
    pub pointer: Option<(i64, i64)>,
    pub locked: bool,
    pub isolated_layout: bool,
}

/// Whether the current Linux session runs on Wayland rather than X11, from the given environment.
pub fn wayland_session_active_in(environment: &dyn Fn(&str) -> Option<String>) -> bool {
    let get = |name: &str| environment(name).unwrap_or_default();
    let backend = pystr::casefold(pystr::strip(&get("GDK_BACKEND")));
    if backend == "x11" {
        return false;
    }
    if backend == "wayland" {
        return true;
    }
    if !pystr::strip(&get("WAYLAND_DISPLAY")).is_empty() {
        return true;
    }
    pystr::casefold(pystr::strip(&get("XDG_SESSION_TYPE"))) == "wayland" && pystr::strip(&get("DISPLAY")).is_empty()
}

#[must_use]
pub fn wayland_session_active() -> bool {
    wayland_session_active_in(&|name| std::env::var(name).ok())
}

/// The kind of compositor a Wayland session runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopKind {
    Gnome,
    Kde,
    Sway,
    Generic,
}

/// Identify the running Wayland compositor family.
///
/// A desktop's own service on the session bus outranks SWAYSOCK: sway exports it into the user's systemd environment, and a
/// GNOME or Plasma session started after a sway one inherits the dead socket. `sway_socket` is a socket that exists.
pub fn detect_desktop_kind(environment: &dyn Fn(&str) -> Option<String>, has_gnome: bool, has_kde: bool, sway_socket: &str) -> DesktopKind {
    let get = |name: &str| environment(name).unwrap_or_default();
    let desktop = pystr::casefold(&format!("{}:{}", get("XDG_CURRENT_DESKTOP"), get("DESKTOP_SESSION")));
    if has_gnome {
        return DesktopKind::Gnome;
    }
    if has_kde {
        return DesktopKind::Kde;
    }
    if !sway_socket.is_empty() {
        return DesktopKind::Sway;
    }
    if desktop.contains("gnome") || desktop.contains("ubuntu") {
        return DesktopKind::Gnome;
    }
    if desktop.contains("kde") || desktop.contains("plasma") {
        return DesktopKind::Kde;
    }
    if desktop.contains("sway") {
        return DesktopKind::Sway;
    }
    DesktopKind::Generic
}

/// Clean desktop identifiers (strip `.desktop` and Mutter's untracked `window:N`).
#[must_use]
pub fn normalize_application_id(raw: &str) -> String {
    let cleaned = pystr::strip(raw);
    if cleaned.is_empty() || cleaned.starts_with("window:") {
        return String::new();
    }
    if pystr::casefold(cleaned).ends_with(".desktop") {
        // The suffix is ASCII in any case, so its length in characters is its length in bytes.
        let keep = cleaned.chars().count() - ".desktop".len();
        return cleaned.chars().take(keep).collect();
    }
    cleaned.to_owned()
}

/// `int(value)` of a JSON number; `default` for anything else (a bool is not a number here).
fn int_field(mapping: &Value, key: &str, default: i64) -> i64 {
    match mapping.get(key) {
        Some(Value::Number(number)) => number.as_i64().or_else(|| number.as_f64().map(ks_cast::To::to::<i64>)).unwrap_or(default),
        _ => default,
    }
}

fn str_field<'a>(mapping: &'a Value, key: &str) -> &'a str {
    mapping.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Pack one `i3-ipc` request frame.
#[must_use]
pub fn encode_sway_ipc(message_type: u32, payload: &str) -> Vec<u8> {
    let body = payload.as_bytes();
    let mut frame = Vec::with_capacity(SWAY_IPC_HEADER_BYTES.to::<usize>() + body.len());
    frame.extend_from_slice(SWAY_IPC_MAGIC);
    frame.extend_from_slice(&body.len().low::<u32>().to_ne_bytes());
    frame.extend_from_slice(&message_type.to_ne_bytes());
    frame.extend_from_slice(body);
    frame
}

/// Unpack `(payload_length, message_type)` from a 14-byte `i3-ipc` header.
pub fn decode_sway_ipc_header(header: &[u8]) -> Result<(u32, u32), DesktopParseError> {
    if header.len() != SWAY_IPC_HEADER_BYTES.to::<usize>() {
        return Err(DesktopParseError("Invalid sway IPC header length".to_owned()));
    }
    if &header[..6] != SWAY_IPC_MAGIC {
        return Err(DesktopParseError("Invalid sway IPC magic".to_owned()));
    }
    let length = u32::from_ne_bytes([header[6], header[7], header[8], header[9]]);
    let message_type = u32::from_ne_bytes([header[10], header[11], header[12], header[13]]);
    Ok((length, message_type))
}

/// Extract keyboard layout names and active index from sway `GET_INPUTS`.
pub fn parse_sway_inputs(payload: &Value) -> (Vec<String>, i64) {
    let Some(items) = payload.as_array() else { return (Vec::new(), 0) };
    let mut fallback: (Vec<String>, i64) = (Vec::new(), 0);
    for item in items {
        if !item.is_object() || item.get("type").and_then(Value::as_str) != Some("keyboard") {
            continue;
        }
        let Some(raw_names) = item.get("xkb_layout_names").and_then(Value::as_array) else { continue };
        let names: Vec<String> = raw_names.iter().filter_map(Value::as_str).filter(|name| !name.is_empty()).map(str::to_owned).collect();
        if names.is_empty() {
            continue;
        }
        let mut active = int_field(item, "xkb_active_layout_index", 0);
        if !(0 <= active && active.to::<usize>() < names.len()) {
            active = 0;
        }
        if names.len() > 1 {
            return (names, active);
        }
        if fallback.0.is_empty() {
            fallback = (names, active);
        }
    }
    fallback
}

/// The children of a sway container in the order the Python walks them: `nodes` before `floating_nodes`, each in order
/// (its stack pops the last pushed first).
fn push_sway_children<'a>(node: &'a Value, stack: &mut Vec<&'a Value>) {
    for key in ["floating_nodes", "nodes"] {
        if let Some(children) = node.get(key).and_then(Value::as_array) {
            stack.extend(children.iter().rev().filter(|child| child.is_object()));
        }
    }
}

/// Find the focused container in sway `GET_TREE`.
#[must_use]
pub fn parse_sway_focused_window(tree: &Value) -> Option<DesktopWindowInfo> {
    if !tree.is_object() {
        return None;
    }
    let mut stack: Vec<&Value> = vec![tree];
    while let Some(node) = stack.pop() {
        let pid = int_field(node, "pid", 0);
        let focused = node.get("focused") == Some(&Value::Bool(true));
        if focused && pid > 0 {
            let con_id = int_field(node, "id", 0);
            if con_id <= 0 {
                return None;
            }
            let mut app_id = normalize_application_id(str_field(node, "app_id"));
            if app_id.is_empty()
                && let Some(props) = node.get("window_properties").filter(|props| props.is_object())
            {
                let class = str_field(props, "class");
                let chosen = if class.is_empty() { str_field(props, "instance") } else { class };
                app_id = normalize_application_id(chosen);
            }
            // The client area: `rect` is the container with its borders and title bar, and `window_rect` is where the client
            // sits inside it.
            let empty = Value::Null;
            let rect = node.get("rect").filter(|rect| rect.is_object()).unwrap_or(&empty);
            let inner = node.get("window_rect").filter(|inner| inner.is_object()).unwrap_or(&empty);
            let pick = |key: &str| {
                let inner_value = int_field(inner, key, 0);
                if inner_value != 0 { inner_value } else { int_field(rect, key, 0) }
            };
            return Some(DesktopWindowInfo {
                window_id: con_id,
                application: app_id,
                pid,
                x: int_field(rect, "x", 0) + int_field(inner, "x", 0),
                y: int_field(rect, "y", 0) + int_field(inner, "y", 0),
                width: pick("width"),
                height: pick("height"),
                password: false,
            });
        }
        push_sway_children(node, &mut stack);
    }
    None
}

/// Locate a sway container `con_id` owned by `pid` and, when one is named, with this `title`. Another window of the
/// program is no answer: a window not on the screen yet would have the settings window moved in its place.
#[must_use]
pub fn find_sway_window_by_pid(tree: &Value, pid: i64, title: &str) -> Option<i64> {
    if !tree.is_object() || pid <= 0 {
        return None;
    }
    let mut stack: Vec<&Value> = vec![tree];
    while let Some(node) = stack.pop() {
        let con_id = int_field(node, "id", 0);
        if int_field(node, "pid", 0) == pid && con_id > 0 && (title.is_empty() || str_field(node, "name") == title) {
            return Some(con_id);
        }
        push_sway_children(node, &mut stack);
    }
    None
}

#[must_use]
pub fn sway_popup_move_command(con_id: i64, x: i64, y: i64) -> String {
    format!("[con_id={con_id}] floating enable, border none, move position {x} {y}")
}

#[must_use]
pub fn sway_focus_command(con_id: i64) -> String {
    format!("[con_id={con_id}] focus")
}

#[must_use]
pub fn sway_switch_layout_command(index: i64) -> String {
    format!("input type:keyboard xkb_switch_layout {index}")
}

/// Decode the JSON state snapshot returned by the GNOME Shell extension.
pub fn parse_gnome_state(raw_json: &str) -> Result<DesktopSnapshot, DesktopParseError> {
    let data: Value = serde_json::from_str(raw_json).map_err(|error| DesktopParseError(error.to_string()))?;
    if !data.is_object() {
        return Err(DesktopParseError("GNOME state payload must be a JSON object".to_owned()));
    }
    let layouts: Vec<String> = data
        .get("layouts")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).filter(|item| !item.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default();
    // -1: the extension has not seen the current source applied yet; the layout is unknown till then.
    let mut current = int_field(&data, "current_layout", -1);
    if !(0 <= current && current.to::<usize>() < layouts.len()) {
        current = -1;
    }
    let mut window = None;
    if let Some(win) = data.get("window").filter(|win| win.is_object()) {
        let win_id = int_field(win, "id", 0);
        if win_id > 0 {
            let mut application = normalize_application_id(str_field(win, "wm_class"));
            if application.is_empty() {
                application = normalize_application_id(str_field(win, "app_id"));
            }
            if application.is_empty() {
                application = normalize_application_id(str_field(win, "wm_class_instance"));
            }
            window = Some(DesktopWindowInfo {
                window_id: win_id,
                application,
                pid: int_field(win, "pid", 0),
                x: int_field(win, "x", 0),
                y: int_field(win, "y", 0),
                width: int_field(win, "width", 0),
                height: int_field(win, "height", 0),
                password: win.get("password") == Some(&Value::Bool(true)),
            });
        }
    }
    Ok(DesktopSnapshot {
        layouts,
        current_layout: current,
        window,
        pointer: point_of(data.get("pointer")),
        locked: data.get("locked") == Some(&Value::Bool(true)),
        isolated_layout: data.get("per_window_layout") == Some(&Value::Bool(true)),
    })
}

fn point_of(object: Option<&Value>) -> Option<(i64, i64)> {
    let object = object.filter(|object| object.is_object())?;
    if object.get("x").is_some() && object.get("y").is_some() {
        Some((int_field(object, "x", 0), int_field(object, "y", 0)))
    } else {
        None
    }
}

/// Bounded bidirectional mapping between `KWin` `internalId` UUIDs and integer handles.
#[derive(Debug)]
pub struct KdeWindowIdMap {
    limit: usize,
    next_id: i64,
    /// Least recently used first.
    by_uuid: VecDeque<(String, i64)>,
    by_handle: HashMap<i64, String>,
}

impl Default for KdeWindowIdMap {
    fn default() -> Self {
        KdeWindowIdMap::new(KDE_WINDOW_ID_CACHE_LIMIT.to::<usize>())
    }
}

impl KdeWindowIdMap {
    #[must_use]
    pub fn new(limit: usize) -> KdeWindowIdMap {
        KdeWindowIdMap { limit, next_id: i64::from(KDE_WINDOW_ID_START), by_uuid: VecDeque::new(), by_handle: HashMap::new() }
    }

    pub fn intern(&mut self, uuid_text: &str) -> i64 {
        let key = pystr::strip(uuid_text);
        if key.is_empty() {
            return 0;
        }
        if let Some(position) = self.by_uuid.iter().position(|(uuid, _)| uuid == key) {
            let entry = self.by_uuid.remove(position).expect("the position was just found");
            let handle = entry.1;
            self.by_uuid.push_back(entry);
            return handle;
        }
        while self.by_uuid.len() >= self.limit {
            match self.by_uuid.pop_front() {
                Some((_, oldest_handle)) => {
                    self.by_handle.remove(&oldest_handle);
                }
                None => break,
            }
        }
        let handle = self.next_id;
        self.next_id += 1;
        self.by_uuid.push_back((key.to_owned(), handle));
        self.by_handle.insert(handle, key.to_owned());
        handle
    }

    #[must_use]
    pub fn resolve(&self, handle: i64) -> String {
        self.by_handle.get(&handle).cloned().unwrap_or_default()
    }
}

/// Convert `org.kde.KeyboardLayouts.getLayoutsList` `(layout, variant, label)` tuples.
#[must_use]
pub fn parse_kde_layouts(items: &[(String, String, String)]) -> Vec<String> {
    let mut result = Vec::new();
    for (layout, variant, _label) in items {
        let code = pystr::strip(layout);
        if code.is_empty() {
            continue;
        }
        let sub = pystr::strip(variant);
        result.push(if sub.is_empty() { code.to_owned() } else { format!("{code}+{sub}") });
    }
    result
}

/// The window and the pointer a `KWin` report carries.
pub type KwinReport = (Option<DesktopWindowInfo>, Option<(i64, i64)>);

/// Decode a focus/pointer JSON report pushed by the `KWin` 6 helper script.
pub fn parse_kwin_report(raw_json: &str, id_map: &mut KdeWindowIdMap) -> Result<KwinReport, DesktopParseError> {
    let data: Value = serde_json::from_str(raw_json).map_err(|error| DesktopParseError(error.to_string()))?;
    if !data.is_object() {
        return Err(DesktopParseError("KWin report must be a JSON object".to_owned()));
    }
    let mut window = None;
    if let Some(win) = data.get("window").filter(|win| win.is_object()) {
        let handle = id_map.intern(str_field(win, "id"));
        if handle > 0 {
            let mut application = normalize_application_id(str_field(win, "resourceClass"));
            if application.is_empty() {
                application = normalize_application_id(str_field(win, "desktopFileName"));
            }
            if application.is_empty() {
                application = normalize_application_id(str_field(win, "resourceName"));
            }
            window = Some(DesktopWindowInfo {
                window_id: handle,
                application,
                pid: int_field(win, "pid", 0),
                x: int_field(win, "x", 0),
                y: int_field(win, "y", 0),
                width: int_field(win, "width", 0),
                height: int_field(win, "height", 0),
                password: false,
            });
        }
    }
    Ok((window, point_of(data.get("cursor"))))
}

/// `json.dumps(text)`: a JavaScript string literal.
fn js_string(text: &str) -> String {
    json_dumps(&Value::String(text.to_owned()), &JsonOptions::default())
}

/// Generate the `KWin` 6 JavaScript listener that pushes active-window updates.
#[must_use]
pub fn build_kwin_watcher_script(bus_name: &str, object_path: &str, interface: &str) -> String {
    let (dest, path, iface) = (js_string(bus_name), js_string(object_path), js_string(interface));
    let mut script = String::new();
    for line in [
        "function ksPack(w) {",
        "    var win = null;",
        "    if (w) {",
        "        var g = w.clientGeometry || w.frameGeometry;",
        "        win = {",
        "            id: String(w.internalId || ''),",
        "            resourceClass: String(w.resourceClass || ''),",
        "            resourceName: String(w.resourceName || ''),",
        "            desktopFileName: String(w.desktopFileName || ''),",
        "            pid: Number(w.pid || 0),",
        "            x: Math.round(g.x),",
        "            y: Math.round(g.y),",
        "            width: Math.round(g.width),",
        "            height: Math.round(g.height)",
        "        };",
        "    }",
        "    var payload = JSON.stringify({",
        "        window: win,",
        "        cursor: {x: Math.round(workspace.cursorPos.x), y: Math.round(workspace.cursorPos.y)}",
        "    });",
    ] {
        script.push_str(line);
        script.push('\n');
    }
    script.push_str(&format!("    callDBus({dest}, {path}, {iface}, 'Report', payload);\n"));
    script.push_str("}\nworkspace.windowActivated.connect(ksPack);\nksPack(workspace.activeWindow);\n");
    script
}

/// What a one-shot `KWin` 6 script does: position a popup, restore focus, or query the cursor.
#[derive(Clone, Debug, Default)]
pub struct KwinAction<'a> {
    pub move_pid: i64,
    pub move_title: &'a str,
    pub x: i64,
    pub y: i64,
    pub activate_uuid: &'a str,
    pub report_target: Option<(&'a str, &'a str, &'a str)>,
}

#[must_use]
pub fn build_kwin_action_script(action: &KwinAction<'_>) -> String {
    let mut lines: Vec<String> = vec!["var list = workspace.windowList();".to_owned()];
    if action.move_pid > 0 {
        let title_json = js_string(action.move_title);
        let (pid, x, y) = (action.move_pid, action.x, action.y);
        for line in [
            format!("var targetTitle = {title_json};"),
            "var popup = null;".to_owned(),
            "for (var i = 0; i < list.length; i++) {".to_owned(),
            "    var cand = list[i];".to_owned(),
            format!("    if (Number(cand.pid || 0) === {pid} && String(cand.caption || '') === targetTitle) {{"),
            "        popup = cand;".to_owned(),
            "        break;".to_owned(),
            "    }".to_owned(),
            "}".to_owned(),
            "if (popup) {".to_owned(),
            "    popup.keepAbove = true;".to_owned(),
            "    popup.frameGeometry = {".to_owned(),
            format!("        x: {x},"),
            format!("        y: {y},"),
            "        width: popup.frameGeometry.width,".to_owned(),
            "        height: popup.frameGeometry.height".to_owned(),
            "    };".to_owned(),
            "}".to_owned(),
        ] {
            lines.push(line);
        }
    }
    if !action.activate_uuid.is_empty() {
        let uuid_json = js_string(action.activate_uuid);
        for line in [
            "for (var j = 0; j < list.length; j++) {".to_owned(),
            format!("    if (String(list[j].internalId || '') === {uuid_json}) {{"),
            "        workspace.activeWindow = list[j];".to_owned(),
            "        break;".to_owned(),
            "    }".to_owned(),
            "}".to_owned(),
        ] {
            lines.push(line);
        }
    }
    if let Some((dest, path, iface)) = action.report_target {
        let (dest, path, iface) = (js_string(dest), js_string(path), js_string(iface));
        for line in [
            "var active = workspace.activeWindow;",
            "var g = active ? (active.clientGeometry || active.frameGeometry) : null;",
            "var win = active ? {",
            "    id: String(active.internalId || ''),",
            "    resourceClass: String(active.resourceClass || ''),",
            "    resourceName: String(active.resourceName || ''),",
            "    desktopFileName: String(active.desktopFileName || ''),",
            "    pid: Number(active.pid || 0),",
            "    x: Math.round(g.x),",
            "    y: Math.round(g.y),",
            "    width: Math.round(g.width),",
            "    height: Math.round(g.height)",
            "} : null;",
            "var out = JSON.stringify({",
            "    window: win,",
            "    cursor: {x: Math.round(workspace.cursorPos.x), y: Math.round(workspace.cursorPos.y)}",
            "});",
        ] {
            lines.push(line.to_owned());
        }
        lines.push(format!("callDBus({dest}, {path}, {iface}, 'Report', out);"));
    }
    lines.join("\n") + "\n"
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    fn active(pairs: &[(&str, &str)]) -> bool {
        wayland_session_active_in(&|name| pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned()))
    }

    #[test]
    fn the_session_kind_follows_the_environment() {
        assert!(!active(&[]));
        assert!(!active(&[("GDK_BACKEND", " X11 "), ("WAYLAND_DISPLAY", "wayland-0")]));
        assert!(active(&[("GDK_BACKEND", "wayland")]));
        assert!(active(&[("WAYLAND_DISPLAY", "wayland-0")]));
        assert!(active(&[("XDG_SESSION_TYPE", "Wayland")]));
        assert!(!active(&[("XDG_SESSION_TYPE", "wayland"), ("DISPLAY", ":0")]));
        assert!(!active(&[("WAYLAND_DISPLAY", "  ")]));
    }

    use serde_json::json;

    fn info(window_id: i64, application: &str, pid: i64) -> DesktopWindowInfo {
        DesktopWindowInfo { window_id, application: application.to_owned(), pid, ..DesktopWindowInfo::default() }
    }

    fn kind(pairs: &[(&str, &str)], gnome: bool, kde: bool, sway: &str) -> DesktopKind {
        detect_desktop_kind(&|name| pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned()), gnome, kde, sway)
    }

    #[test]
    fn what_a_desktop_leaves_out_is_read_from_what_it_says_next() {
        let flat = DesktopWindowInfo { width: 10, height: 0, ..info(42, "app", 4242) };
        assert_eq!(flat.center(), None);
        assert_eq!(kind(&[("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")], false, false, ""), DesktopKind::Gnome);
        assert_eq!(kind(&[("XDG_CURRENT_DESKTOP", "ubuntu")], false, false, ""), DesktopKind::Gnome);
        assert_eq!(kind(&[("XDG_CURRENT_DESKTOP", "kde")], false, false, ""), DesktopKind::Kde);
        assert_eq!(kind(&[("XDG_CURRENT_DESKTOP", "plasma")], false, false, ""), DesktopKind::Kde);
        // An active layout past the list is the first one.
        let past = json!([{"type": "keyboard", "xkb_layout_names": ["English (US)", "Russian"], "xkb_active_layout_index": 5}]);
        assert_eq!(parse_sway_inputs(&past).1, 0);
        let negative = json!([{"type": "keyboard", "xkb_layout_names": ["English (US)", "Russian"], "xkb_active_layout_index": -1}]);
        assert_eq!(parse_sway_inputs(&negative).1, 0);
        // A focused container with no process is passed over; a class left empty gives way to the instance.
        assert_eq!(parse_sway_focused_window(&json!({"id": 5, "pid": 0, "focused": true})), None);
        let instance = json!({"id": 77, "pid": 5151, "focused": true, "window_properties": {"class": "", "instance": "xterm"}});
        assert_eq!(parse_sway_focused_window(&instance), Some(info(77, "xterm", 5151)));
        let class = json!({"id": 77, "pid": 5151, "focused": true, "window_properties": {"class": "xterm", "instance": "other"}});
        assert_eq!(parse_sway_focused_window(&class), Some(info(77, "xterm", 5151)));
        // GNOME: the class, the application, the instance, each in its turn; a pointer without both numbers is none.
        let named = |window: Value| parse_gnome_state(&json!({"window": window}).to_string()).unwrap().window.map(|window| window.application);
        assert_eq!(named(json!({"id": 1, "wm_class": "a"})).as_deref(), Some("a"));
        assert_eq!(named(json!({"id": 1, "app_id": "b"})).as_deref(), Some("b"));
        assert_eq!(named(json!({"id": 1, "wm_class_instance": "c"})).as_deref(), Some("c"));
        assert_eq!(parse_gnome_state(&json!({"pointer": {"y": 1}}).to_string()).unwrap().pointer, None);
        // KDE: the same three.
        let mut map = KdeWindowIdMap::default();
        let kde = |window: Value, map: &mut KdeWindowIdMap| parse_kwin_report(&json!({"window": window}).to_string(), map).unwrap().0.map(|window| window.application);
        assert_eq!(kde(json!({"id": "u1", "resourceClass": "a"}), &mut map).as_deref(), Some("a"));
        assert_eq!(kde(json!({"id": "u2", "desktopFileName": "b"}), &mut map).as_deref(), Some("b"));
        assert_eq!(kde(json!({"id": "u3", "resourceName": "c"}), &mut map).as_deref(), Some("c"));
        // A map that may hold nothing still answers.
        assert!(KdeWindowIdMap::new(0).intern("u") > 0);
    }

    #[test]
    fn a_window_has_a_center_and_application_ids_are_cleaned() {
        assert_eq!(info(42, "app", 4242).center(), None);
        let sized = DesktopWindowInfo { x: 100, y: -20, width: 301, height: 200, ..info(42, "app", 4242) };
        assert_eq!(sized.center(), Some((250, 80)));
        assert_eq!(normalize_application_id("   "), "");
        assert_eq!(normalize_application_id("window:12"), "");
        assert_eq!(normalize_application_id("org.gnome.TextEditor.DESKTOP"), "org.gnome.TextEditor");
        assert_eq!(normalize_application_id("kitty"), "kitty");
    }

    #[test]
    fn the_desktop_kind_follows_the_bus_services_before_the_environment() {
        assert_eq!(kind(&[], false, false, "/run/sway.sock"), DesktopKind::Sway);
        // A dead socket sway left in the environment is no sway session: the caller passes only a live one.
        assert_eq!(kind(&[("SWAYSOCK", "/run/sway.sock")], false, false, ""), DesktopKind::Generic);
        assert_eq!(kind(&[], true, false, "/run/sway.sock"), DesktopKind::Gnome);
        assert_eq!(kind(&[], false, true, "/run/sway.sock"), DesktopKind::Kde);
        assert_eq!(kind(&[("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")], false, false, ""), DesktopKind::Gnome);
        assert_eq!(kind(&[("DESKTOP_SESSION", "plasma")], false, false, ""), DesktopKind::Kde);
        assert_eq!(kind(&[("XDG_CURRENT_DESKTOP", "sway")], false, false, ""), DesktopKind::Sway);
        assert_eq!(kind(&[("XDG_CURRENT_DESKTOP", "wlroots")], false, false, ""), DesktopKind::Generic);
    }

    #[test]
    fn sway_frames_and_commands() {
        let frame = encode_sway_ipc(100, "hello");
        assert_eq!(decode_sway_ipc_header(&frame[..14]), Ok((5, 100)));
        assert_eq!(&frame[14..], b"hello");
        assert!(decode_sway_ipc_header(&[b'x'; 5]).unwrap_err().0.contains("header length"));
        let mut bad = frame[..14].to_vec();
        bad[..6].copy_from_slice(b"badipc");
        assert!(decode_sway_ipc_header(&bad).unwrap_err().0.contains("magic"));
        assert_eq!(sway_popup_move_command(42, 600, 400), "[con_id=42] floating enable, border none, move position 600 400");
        assert_eq!(sway_focus_command(42), "[con_id=42] focus");
        assert_eq!(sway_switch_layout_command(1), "input type:keyboard xkb_switch_layout 1");
    }

    #[test]
    fn sway_inputs_prefer_the_multi_layout_keyboard_and_fall_back_safely() {
        assert_eq!(parse_sway_inputs(&json!("invalid")), (Vec::new(), 0));
        let mut payload = vec![
            json!("skip"),
            json!({"type": "pointer"}),
            json!({"type": "keyboard", "xkb_layout_names": "bad"}),
            json!({"type": "keyboard", "xkb_layout_names": ["", 0]}),
            json!({"type": "keyboard", "xkb_layout_names": ["English (US)"], "xkb_active_layout_index": true}),
            json!({"type": "keyboard", "xkb_layout_names": ["Ignored single"], "xkb_active_layout_index": 0}),
        ];
        assert_eq!(parse_sway_inputs(&Value::Array(payload.clone())), (vec!["English (US)".to_owned()], 0));
        payload.push(json!({"type": "keyboard", "xkb_layout_names": ["English (US)", "Russian"], "xkb_active_layout_index": 5}));
        assert_eq!(parse_sway_inputs(&Value::Array(payload)), (vec!["English (US)".to_owned(), "Russian".to_owned()], 0));
        let valid = json!([{"type": "keyboard", "xkb_layout_names": ["English (US)", "Russian"], "xkb_active_layout_index": 1}]);
        assert_eq!(parse_sway_inputs(&valid).1, 1);
    }

    #[test]
    fn the_sway_tree_gives_the_focused_window_and_the_window_of_a_process() {
        assert_eq!(parse_sway_focused_window(&json!([])), None);
        assert_eq!(parse_sway_focused_window(&json!({"focused": true, "pid": 4242, "id": 0})), None);
        let wayland = json!({
            "id": 1,
            "nodes": ["not-a-dict", {
                "id": 42, "pid": 4242, "focused": true, "app_id": "foot.desktop",
                "rect": {"x": 12.7, "y": "bad", "width": 800, "height": 600},
                // The client sits inside its container: the window's own area is reported.
                "window_rect": {"x": 2, "y": 24, "width": 600, "height": 400},
            }],
            "floating_nodes": "not-a-list",
        });
        assert_eq!(
            parse_sway_focused_window(&wayland),
            Some(DesktopWindowInfo { window_id: 42, application: "foot".to_owned(), pid: 4242, x: 14, y: 24, width: 600, height: 400, password: false })
        );
        let xwayland = json!({"id": 1, "floating_nodes": [{"id": 77, "pid": 5151, "focused": true, "app_id": 0, "window_properties": {"instance": "xterm"}, "rect": null}]});
        assert_eq!(parse_sway_focused_window(&xwayland), Some(info(77, "xterm", 5151)));
        let no_properties = json!({"id": 99, "pid": 4242, "focused": true, "window_properties": "none"});
        assert_eq!(parse_sway_focused_window(&no_properties), Some(info(99, "", 4242)));
        assert_eq!(parse_sway_focused_window(&json!({"id": 1, "nodes": []})), None);

        assert_eq!(find_sway_window_by_pid(&json!([]), 4242, ""), None);
        assert_eq!(find_sway_window_by_pid(&json!({}), 0, ""), None);
        let tree = json!({"id": 1, "nodes": [
            "skip",
            {"id": 0, "pid": 4242, "name": "Ignored"},
            {"id": 42, "pid": 4242, "name": "Main"},
            {"id": 77, "pid": 4242, "name": "Other"},
            {"id": 99, "pid": 4242, "name": "Popup"},
        ], "floating_nodes": null});
        assert_eq!(find_sway_window_by_pid(&tree, 4242, "Popup"), Some(99));
        assert_eq!(find_sway_window_by_pid(&tree, 4242, "Missing"), None);
        assert_eq!(find_sway_window_by_pid(&tree, 4242, ""), Some(42));
    }

    #[test]
    fn the_gnome_state_is_read_whole_or_in_part() {
        assert!(parse_gnome_state("[]").unwrap_err().0.contains("JSON object"));
        assert!(parse_gnome_state("{").is_err());
        let full = json!({
            "layouts": ["us", "", "ru"], "current_layout": 5,
            "window": {"id": 42, "wm_class": "window:1", "app_id": "org.gnome.Console.desktop", "pid": 4242, "x": 100, "y": 50, "width": 800, "height": 600},
            "pointer": {"x": 640, "y": 360}, "locked": true, "per_window_layout": true,
        });
        let snapshot = parse_gnome_state(&full.to_string()).unwrap();
        assert_eq!(snapshot.layouts, vec!["us".to_owned(), "ru".to_owned()]);
        assert_eq!(snapshot.current_layout, -1);
        assert_eq!(snapshot.window, Some(DesktopWindowInfo { window_id: 42, application: "org.gnome.Console".to_owned(), pid: 4242, x: 100, y: 50, width: 800, height: 600, password: false }));
        assert_eq!((snapshot.pointer, snapshot.locked, snapshot.isolated_layout), (Some((640, 360)), true, true));
        let minimal = parse_gnome_state(&json!({"layouts": "bad", "window": {"id": 0}, "pointer": {"x": 640}}).to_string()).unwrap();
        assert_eq!(minimal, DesktopSnapshot { current_layout: -1, ..DesktopSnapshot::default() });
        let plain = parse_gnome_state(&json!({"layouts": ["us", "ru"], "current_layout": 1}).to_string()).unwrap();
        assert_eq!((plain.layouts.len(), plain.current_layout), (2, 1));
        // GNOME Shell's own password prompt has the keyboard: the window says so.
        let prompt = parse_gnome_state(&json!({"window": {"id": 42, "app_id": "org.gnome.Shell", "password": true}}).to_string()).unwrap();
        assert!(prompt.window.is_some_and(|window| window.password));
    }

    #[test]
    fn kde_handles_layouts_reports_and_scripts() {
        let mut map = KdeWindowIdMap::new(2);
        assert_eq!(map.intern("   "), 0);
        let first = map.intern("uuid-a");
        let second = map.intern("uuid-b");
        assert_eq!((first, second), (1000, 1001));
        assert_eq!(map.intern("uuid-a"), first);
        let third = map.intern("uuid-c");
        assert_eq!((map.resolve(first).as_str(), map.resolve(second).as_str(), map.resolve(third).as_str()), ("uuid-a", "", "uuid-c"));

        let layouts = [("us".to_owned(), String::new(), "English".to_owned()), (String::new(), "winkeys".to_owned(), "Blank".to_owned()), ("ru".to_owned(), "phonetic".to_owned(), "Russian".to_owned())];
        assert_eq!(parse_kde_layouts(&layouts), vec!["us".to_owned(), "ru+phonetic".to_owned()]);

        assert!(parse_kwin_report("[]", &mut map).unwrap_err().0.contains("JSON object"));
        let report = json!({
            "window": {"id": "uuid-kwrite", "resourceClass": "", "desktopFileName": "org.kde.kwrite.desktop", "resourceName": "kwrite", "pid": 4242, "x": 100, "y": 50, "width": 800, "height": 600},
            "cursor": {"x": 640, "y": 360},
        });
        let (window, pointer) = parse_kwin_report(&report.to_string(), &mut map).unwrap();
        assert_eq!(window.unwrap().application, "org.kde.kwrite");
        assert_eq!(pointer, Some((640, 360)));
        assert_eq!(parse_kwin_report(&json!({"window": {"id": ""}, "cursor": {"x": 0}}).to_string(), &mut map).unwrap(), (None, None));
        assert_eq!(parse_kwin_report("{}", &mut map).unwrap(), (None, None));

        let watcher = build_kwin_watcher_script(":1.42", "/Watcher", "io.test.Watcher");
        assert!(watcher.contains("workspace.windowActivated.connect(ksPack);"));
        assert!(watcher.contains("callDBus(\":1.42\", \"/Watcher\", \"io.test.Watcher\", 'Report', payload);\n"));
        assert_eq!(build_kwin_action_script(&KwinAction::default()), "var list = workspace.windowList();\n");
        let full = build_kwin_action_script(&KwinAction {
            move_pid: 4242,
            move_title: "Обучение KeySwitch",
            x: 600,
            y: 400,
            activate_uuid: "uuid-kwrite",
            report_target: Some((":1.42", "/Watcher", "io.test.Watcher")),
        });
        assert!(full.contains("popup.keepAbove = true;") && full.contains("\"uuid-kwrite\"") && full.contains("callDBus"));
        // json.dumps escapes non-ASCII text as the Python writes it.
        assert!(full.contains("var targetTitle = \"\\u041e\\u0431\\u0443\\u0447\\u0435\\u043d\\u0438\\u0435 KeySwitch\";"));
        assert!(full.contains("=== 4242 &&"));
        assert!(full.contains("        x: 600,\n        y: 400,\n"));
    }
}

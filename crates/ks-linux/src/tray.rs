//! `StatusNotifierItem` and `DBusMenu` integration for desktop panels (the Python's `tray.py`), over `zbus`.
//!
//! The objects are served on a connection of their own, whose executor answers the panel on its own thread: `on_action`
//! is called there, and the application moves the work to its main loop.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ks_config::indicator::{alternate_layout_action_label, alternate_layout_group, layout_icon_name, layout_label, normalize_indicator_style};
use serde_json::Value as Json;
use zbus::blocking::Connection;
use zbus::blocking::connection::Builder;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

pub const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";
pub const MENU_INTERFACE: &str = "com.canonical.dbusmenu";
pub const OBJECT_PATH: &str = "/StatusNotifierItem";
pub const MENU_PATH: &str = "/MenuBar";
const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const BUS_NAME_PREFIX: &str = "io.github.olegius88.KeySwitch.StatusNotifierItem.p";

pub const MENU_LAYOUT: i32 = 1;
pub const MENU_SETTINGS: i32 = 2;
pub const MENU_SEPARATOR_PRIMARY: i32 = 3;
pub const MENU_AUTOSWITCH: i32 = 4;
pub const MENU_SOUND: i32 = 5;
pub const MENU_NOTIFICATIONS: i32 = 6;
pub const MENU_SEPARATOR_TOOLS: i32 = 7;
pub const MENU_HISTORY: i32 = 8;
pub const MENU_EXCEPTIONS: i32 = 9;
pub const MENU_ABOUT: i32 = 10;
pub const MENU_SEPARATOR_QUIT: i32 = 11;
pub const MENU_QUIT: i32 = 12;
pub const MENU_SWITCH_LAYOUT: i32 = 13;
pub const DBUSMENU_INTERFACE_VERSION: u32 = 3;
pub const MENU_ITEM_IDS: [i32; 13] = [
    MENU_LAYOUT,
    MENU_SWITCH_LAYOUT,
    MENU_SETTINGS,
    MENU_SEPARATOR_PRIMARY,
    MENU_AUTOSWITCH,
    MENU_SOUND,
    MENU_NOTIFICATIONS,
    MENU_SEPARATOR_TOOLS,
    MENU_EXCEPTIONS,
    MENU_HISTORY,
    MENU_ABOUT,
    MENU_SEPARATOR_QUIT,
    MENU_QUIT,
];

/// What a menu line (or the middle button) asks the application to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    ShowSettings,
    ToggleEngine,
    SwitchLayout,
    ToggleSound,
    ToggleNotifications,
    ShowHistory,
    ShowExceptions,
    ShowAbout,
    Quit,
}

pub type ActionHandler = Arc<dyn Fn(TrayAction) + Send + Sync>;

fn action_of(item: i32) -> Option<TrayAction> {
    Some(match item {
        MENU_SETTINGS => TrayAction::ShowSettings,
        MENU_AUTOSWITCH => TrayAction::ToggleEngine,
        MENU_SWITCH_LAYOUT => TrayAction::SwitchLayout,
        MENU_SOUND => TrayAction::ToggleSound,
        MENU_NOTIFICATIONS => TrayAction::ToggleNotifications,
        MENU_HISTORY => TrayAction::ShowHistory,
        MENU_EXCEPTIONS => TrayAction::ShowExceptions,
        MENU_ABOUT => TrayAction::ShowAbout,
        MENU_QUIT => TrayAction::Quit,
        _ => return None,
    })
}

/// A `DBusMenu` property value, before it is a D-Bus variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Property {
    Text(String),
    Flag(bool),
    Number(i32),
}

fn text(value: &str) -> Property {
    Property::Text(value.to_owned())
}

fn variant(property: &Property) -> OwnedValue {
    let value = match property {
        Property::Text(value) => Value::from(value.clone()),
        Property::Flag(value) => Value::from(*value),
        Property::Number(value) => Value::from(*value),
    };
    value.try_to_owned().expect("a plain value has no file descriptor")
}

fn variants(properties: &[(String, Property)], requested: &[String]) -> HashMap<String, OwnedValue> {
    properties.iter().filter(|(name, _)| requested.is_empty() || requested.contains(name)).map(|(name, value)| (name.clone(), variant(value))).collect()
}

fn layout_text(group: i64) -> String {
    if group >= 0 { format!("Текущая раскладка: {}", layout_label(group)) } else { "Текущая раскладка определяется".to_owned() }
}

/// The state the menu shows.
#[derive(Clone, Debug)]
pub struct MenuState {
    pub revision: u32,
    pub enabled: bool,
    pub sound_enabled: bool,
    pub notifications_enabled: bool,
    pub group: i64,
    pub layout_icon: String,
}

impl Default for MenuState {
    fn default() -> Self {
        MenuState { revision: 1, enabled: true, sound_enabled: false, notifications_enabled: true, group: -1, layout_icon: "keyswitch".to_owned() }
    }
}

pub type Properties = Vec<(String, Property)>;

impl MenuState {
    /// `_item_properties`: None for an item that does not exist.
    #[must_use]
    pub fn item_properties(&self, item: i32) -> Option<Properties> {
        if item == 0 {
            return Some(vec![("children-display".to_owned(), text("submenu"))]);
        }
        if !MENU_ITEM_IDS.contains(&item) {
            return None;
        }
        if [MENU_SEPARATOR_PRIMARY, MENU_SEPARATOR_TOOLS, MENU_SEPARATOR_QUIT].contains(&item) {
            return Some(vec![("type".to_owned(), text("separator"))]);
        }
        let (label, icon) = match item {
            MENU_LAYOUT => (layout_text(self.group), self.layout_icon.as_str()),
            MENU_SWITCH_LAYOUT => (alternate_layout_action_label(self.group), "input-keyboard-symbolic"),
            MENU_SETTINGS => ("Настройки KeySwitch…".to_owned(), "preferences-system-symbolic"),
            MENU_AUTOSWITCH => ("Автопереключение".to_owned(), "input-keyboard-symbolic"),
            MENU_SOUND => ("Звуковые эффекты".to_owned(), "audio-volume-high-symbolic"),
            MENU_NOTIFICATIONS => ("Уведомления об исправлениях".to_owned(), "preferences-system-notifications-symbolic"),
            MENU_HISTORY => ("История исправлений…".to_owned(), "document-open-recent-symbolic"),
            MENU_EXCEPTIONS => ("Программы-исключения…".to_owned(), "application-x-executable-symbolic"),
            MENU_ABOUT => ("О программе…".to_owned(), "help-about-symbolic"),
            _ => ("Выход".to_owned(), "application-exit-symbolic"),
        };
        let mut properties = vec![("label".to_owned(), Property::Text(label)), ("icon-name".to_owned(), text(icon))];
        if item == MENU_LAYOUT {
            properties.push(("enabled".to_owned(), Property::Flag(false)));
        } else if item == MENU_SWITCH_LAYOUT {
            properties.push(("enabled".to_owned(), Property::Flag(alternate_layout_group(self.group).is_some())));
        }
        if [MENU_AUTOSWITCH, MENU_SOUND, MENU_NOTIFICATIONS].contains(&item) {
            let state = match item {
                MENU_AUTOSWITCH => self.enabled,
                MENU_SOUND => self.sound_enabled,
                _ => self.notifications_enabled,
            };
            properties.push(("toggle-type".to_owned(), text("checkmark")));
            properties.push(("toggle-state".to_owned(), Property::Number(i32::from(state))));
        }
        Some(properties)
    }

    /// The value `GetProperty` answers when the item does not carry the property: the `DBusMenu` default.
    #[must_use]
    pub fn property_default(name: &str) -> Option<Property> {
        Some(match name {
            "type" => text("standard"),
            "label" | "icon-name" | "toggle-type" | "children-display" => text(""),
            "enabled" | "visible" => Property::Flag(true),
            "toggle-state" => Property::Number(-1),
            _ => return None,
        })
    }
}

/// What the item shows.
#[derive(Clone, Debug)]
struct ItemState {
    status: String,
    icon_name: String,
    subtitle: String,
    enabled: bool,
    group: i64,
    indicator_style: &'static str,
    icon_theme_path: String,
}

struct Shared {
    item: Mutex<ItemState>,
    menu: Mutex<MenuState>,
    on_action: ActionHandler,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Item {
    shared: Arc<Shared>,
    connection: Arc<Mutex<Option<Connection>>>,
}

/// The tooltip: the icon name, no pixmaps, the title and the text.
type ToolTip = (String, Vec<(i32, i32, Vec<u8>)>, String, String);

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> String {
        "ApplicationStatus".to_owned()
    }

    #[zbus(property)]
    fn id(&self) -> String {
        "keyswitch".to_owned()
    }

    #[zbus(property)]
    fn title(&self) -> String {
        "KeySwitch".to_owned()
    }

    #[zbus(property)]
    fn status(&self) -> String {
        locked(&self.shared.item).status.clone()
    }

    #[zbus(property)]
    fn window_id(&self) -> u32 {
        0
    }

    #[zbus(property)]
    fn icon_name(&self) -> String {
        locked(&self.shared.item).icon_name.clone()
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> String {
        locked(&self.shared.item).icon_theme_path.clone()
    }

    #[zbus(property)]
    fn attention_icon_name(&self) -> String {
        "keyswitch-paused".to_owned()
    }

    #[zbus(property)]
    fn overlay_icon_name(&self) -> String {
        String::new()
    }

    #[zbus(property)]
    fn tool_tip(&self) -> ToolTip {
        let state = locked(&self.shared.item);
        (state.icon_name.clone(), Vec::new(), "KeySwitch".to_owned(), state.subtitle.clone())
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'static> {
        ObjectPath::from_static_str_unchecked(MENU_PATH)
    }

    fn activate(&self, _x: i32, _y: i32) {
        self.request_open();
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {
        (self.shared.on_action)(TrayAction::ToggleEngine);
    }

    fn context_menu(&self, _x: i32, _y: i32) {
        self.request_open();
    }

    fn scroll(&self, _delta: i32, _orientation: String) {}

    #[zbus(signal)]
    async fn new_status(emitter: &SignalEmitter<'_>, status: String) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

impl Item {
    /// `request_open`: asks the host to open the menu.
    fn request_open(&self) {
        if let Some(connection) = locked(&self.connection).as_ref() {
            let _ = connection.emit_signal(None::<&str>, MENU_PATH, MENU_INTERFACE, "ItemActivationRequested", &(0_i32, 0_u32));
        }
    }
}

struct Menu {
    shared: Arc<Shared>,
}

/// `(ia{sv}av)`: an item, its properties and its children, each child a variant that holds the same structure.
type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

impl Menu {
    fn layout(&self, item: i32, depth: i32, requested: &[String]) -> Option<Layout> {
        let state = locked(&self.shared.menu).clone();
        let properties = variants(&state.item_properties(item)?, requested);
        let mut children = Vec::new();
        if item == 0 && depth != 0 {
            for child in MENU_ITEM_IDS {
                let (id, properties, kids) = self.layout(child, 0, requested)?;
                let structure = Value::from((id, properties.into_iter().map(|(key, value)| (key, Value::from(value))).collect::<HashMap<_, _>>(), kids));
                children.push(structure.try_to_owned().ok()?);
            }
        }
        Some((item, properties, children))
    }

    fn dispatch(&self, item: i32, event: &str) -> bool {
        let known = item == 0 || MENU_ITEM_IDS.contains(&item);
        if event == "clicked"
            && let Some(action) = action_of(item)
        {
            (self.shared.on_action)(action);
        }
        known
    }
}

fn not_found(what: &str) -> zbus::fdo::Error {
    zbus::fdo::Error::InvalidArgs(what.to_owned())
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        DBUSMENU_INTERFACE_VERSION
    }

    #[zbus(property)]
    fn text_direction(&self) -> String {
        "ltr".to_owned()
    }

    #[zbus(property)]
    fn status(&self) -> String {
        "normal".to_owned()
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        vec![locked(&self.shared.item).icon_theme_path.clone()]
    }

    fn get_layout(&self, parent_id: i32, recursion_depth: i32, property_names: Vec<String>) -> zbus::fdo::Result<(u32, Layout)> {
        let layout = self.layout(parent_id, recursion_depth, &property_names).ok_or_else(|| not_found("unknown menu item"))?;
        Ok((locked(&self.shared.menu).revision, layout))
    }

    fn get_group_properties(&self, ids: Vec<i32>, property_names: Vec<String>) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let selected: Vec<i32> = if ids.is_empty() { std::iter::once(0).chain(MENU_ITEM_IDS).collect() } else { ids };
        let state = locked(&self.shared.menu).clone();
        selected.into_iter().filter_map(|item| Some((item, variants(&state.item_properties(item)?, &property_names)))).collect()
    }

    fn get_property(&self, id: i32, name: String) -> zbus::fdo::Result<OwnedValue> {
        let properties = locked(&self.shared.menu).item_properties(id).ok_or_else(|| not_found("unknown menu item"))?;
        if let Some((_, value)) = properties.iter().find(|(key, _)| *key == name) {
            return Ok(variant(value));
        }
        MenuState::property_default(&name).map(|value| variant(&value)).ok_or_else(|| zbus::fdo::Error::UnknownProperty(name))
    }

    fn event(&self, id: i32, event_id: String, _data: Value<'_>, _timestamp: u32) {
        self.dispatch(id, &event_id);
    }

    fn event_group(&self, events: Vec<(i32, String, Value<'_>, u32)>) -> Vec<i32> {
        events.into_iter().filter(|(id, event, _, _)| !self.dispatch(*id, event)).map(|(id, ..)| id).collect()
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), ids.into_iter().filter(|id| *id != 0 && !MENU_ITEM_IDS.contains(id)).collect())
    }

    #[zbus(signal)]
    async fn items_properties_updated(
        emitter: &SignalEmitter<'_>,
        updated: Vec<(i32, HashMap<String, OwnedValue>)>,
        removed: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn layout_updated(emitter: &SignalEmitter<'_>, revision: u32, parent: i32) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn item_activation_requested(emitter: &SignalEmitter<'_>, id: i32, timestamp: u32) -> zbus::Result<()>;
}

/// The icon in the panel and its menu.
pub struct StatusNotifierItem {
    connection: Connection,
    shared: Arc<Shared>,
}

fn describe(error: impl std::fmt::Display) -> String {
    error.to_string()
}

impl StatusNotifierItem {
    /// Serves the item and its menu and registers the item with the panel's watcher; an error says what is missing.
    pub fn new(on_action: ActionHandler, icon_theme_path: &Path) -> Result<StatusNotifierItem, String> {
        let shared = Arc::new(Shared {
            item: Mutex::new(ItemState {
                status: "Active".to_owned(),
                icon_name: "keyswitch".to_owned(),
                subtitle: "Раскладка определяется".to_owned(),
                enabled: true,
                group: -1,
                indicator_style: "letters",
                icon_theme_path: icon_theme_path.display().to_string(),
            }),
            menu: Mutex::new(MenuState::default()),
            on_action,
        });
        let holder = Arc::new(Mutex::new(None));
        let connection = Builder::session()
            .and_then(|builder| builder.name(format!("{BUS_NAME_PREFIX}{}", std::process::id())))
            .and_then(|builder| builder.serve_at(MENU_PATH, Menu { shared: shared.clone() }))
            .and_then(|builder| builder.serve_at(OBJECT_PATH, Item { shared: shared.clone(), connection: holder.clone() }))
            .and_then(Builder::build)
            .map_err(describe)?;
        *locked(&holder) = Some(connection.clone());
        connection
            .call_method(Some(WATCHER_NAME), WATCHER_PATH, Some(WATCHER_NAME), "RegisterStatusNotifierItem", &(OBJECT_PATH,))
            .map_err(describe)?;
        Ok(StatusNotifierItem { connection, shared })
    }

    fn emit<B>(&self, path: &str, interface: &str, name: &str, body: &B)
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let _ = self.connection.emit_signal(None::<&str>, path, interface, name, body);
    }

    pub fn set_enabled(&self, enabled: bool) {
        {
            let mut item = locked(&self.shared.item);
            if item.enabled == enabled {
                return;
            }
            item.enabled = enabled;
        }
        self.refresh();
    }

    pub fn set_layout(&self, group: i64) {
        {
            let mut item = locked(&self.shared.item);
            if item.group == group {
                return;
            }
            item.group = group;
        }
        self.refresh();
    }

    pub fn set_indicator_style(&self, style: &Json) {
        let normalized = normalize_indicator_style(style);
        {
            let mut item = locked(&self.shared.item);
            if item.indicator_style == normalized {
                return;
            }
            item.indicator_style = normalized;
        }
        self.refresh();
    }

    pub fn set_sound_enabled(&self, enabled: bool) {
        let changed = {
            let mut menu = locked(&self.shared.menu);
            std::mem::replace(&mut menu.sound_enabled, enabled) != enabled
        };
        if changed {
            self.notify(MENU_SOUND, vec![("toggle-state".to_owned(), Property::Number(i32::from(enabled)))]);
        }
    }

    pub fn set_notifications_enabled(&self, enabled: bool) {
        let changed = {
            let mut menu = locked(&self.shared.menu);
            std::mem::replace(&mut menu.notifications_enabled, enabled) != enabled
        };
        if changed {
            self.notify(MENU_NOTIFICATIONS, vec![("toggle-state".to_owned(), Property::Number(i32::from(enabled)))]);
        }
    }

    /// `StatusNotifierMenu._notify`: a new revision and the changed properties of one item.
    fn notify(&self, item: i32, properties: Properties) {
        locked(&self.shared.menu).revision += 1;
        let updated = vec![(item, variants(&properties, &[]))];
        self.emit(MENU_PATH, MENU_INTERFACE, "ItemsPropertiesUpdated", &(updated, Vec::<(i32, Vec<String>)>::new()));
    }

    /// `_refresh`: the item stays visible while paused, so it remains possible to resume from the panel.
    fn refresh(&self) {
        let (icon_name, subtitle, enabled, group) = {
            let mut item = locked(&self.shared.item);
            item.status = "Active".to_owned();
            item.icon_name = layout_icon_name(&Json::String(item.indicator_style.to_owned()), item.group).to_owned();
            let label = layout_label(item.group);
            let state = if item.enabled { "автокоррекция включена" } else { "автокоррекция на паузе" };
            item.subtitle = if label == "—" {
                let mut capital = state.chars();
                capital.next().map(|first| first.to_uppercase().chain(capital).collect()).unwrap_or_default()
            } else {
                format!("{label} · {state}")
            };
            (item.icon_name.clone(), item.subtitle.clone(), item.enabled, item.group)
        };
        let _ = subtitle;
        self.set_indicator_state(enabled, group, &icon_name);
        self.emit(OBJECT_PATH, ITEM_INTERFACE, "NewStatus", &("Active",));
        self.emit(OBJECT_PATH, ITEM_INTERFACE, "NewIcon", &());
        self.emit(OBJECT_PATH, ITEM_INTERFACE, "NewToolTip", &());
    }

    /// `StatusNotifierMenu.set_indicator_state`.
    fn set_indicator_state(&self, enabled: bool, group: i64, icon_name: &str) {
        let (group_changed, icon_changed, enabled_changed) = {
            let mut menu = locked(&self.shared.menu);
            let group_changed = menu.group != group;
            let icon_changed = menu.layout_icon != icon_name;
            let enabled_changed = menu.enabled != enabled;
            menu.group = group;
            menu.layout_icon = icon_name.to_owned();
            menu.enabled = enabled;
            (group_changed, icon_changed, enabled_changed)
        };
        if group_changed || icon_changed {
            self.notify(MENU_LAYOUT, vec![("label".to_owned(), Property::Text(layout_text(group))), ("icon-name".to_owned(), text(icon_name))]);
        }
        if group_changed {
            self.notify(
                MENU_SWITCH_LAYOUT,
                vec![
                    ("label".to_owned(), Property::Text(alternate_layout_action_label(group))),
                    ("enabled".to_owned(), Property::Flag(alternate_layout_group(group).is_some())),
                ],
            );
        }
        if enabled_changed {
            self.notify(MENU_AUTOSWITCH, vec![("toggle-state".to_owned(), Property::Number(i32::from(enabled)))]);
        }
    }

    /// Takes the item and the menu off the bus.
    pub fn close(&self) {
        let server = self.connection.object_server();
        let _ = server.remove::<Menu, _>(MENU_PATH);
        let _ = server.remove::<Item, _>(OBJECT_PATH);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(properties: &Properties, name: &str) -> Option<Property> {
        properties.iter().find(|(key, _)| key == name).map(|(_, value)| value.clone())
    }

    #[test]
    fn the_menu_has_thirteen_lines_and_toggles_carry_their_state() {
        let state = MenuState { group: 0, sound_enabled: true, ..MenuState::default() };
        assert_eq!(MENU_ITEM_IDS.len(), 13);
        let layout = state.item_properties(MENU_LAYOUT).unwrap();
        assert_eq!(find(&layout, "label"), Some(Property::Text("Текущая раскладка: EN".to_owned())));
        assert_eq!(find(&layout, "enabled"), Some(Property::Flag(false)));
        assert_eq!(find(&state.item_properties(MENU_SWITCH_LAYOUT).unwrap(), "enabled"), Some(Property::Flag(true)));
        assert_eq!(find(&state.item_properties(MENU_SOUND).unwrap(), "toggle-state"), Some(Property::Number(1)));
        assert_eq!(find(&state.item_properties(MENU_NOTIFICATIONS).unwrap(), "toggle-type"), Some(Property::Text("checkmark".to_owned())));
        assert_eq!(find(&state.item_properties(MENU_SEPARATOR_TOOLS).unwrap(), "type"), Some(Property::Text("separator".to_owned())));
        assert!(state.item_properties(99).is_none());
        assert_eq!(find(&state.item_properties(0).unwrap(), "children-display"), Some(Property::Text("submenu".to_owned())));
    }

    #[test]
    fn an_unknown_layout_cannot_be_switched() {
        let state = MenuState::default();
        assert_eq!(find(&state.item_properties(MENU_LAYOUT).unwrap(), "label"), Some(Property::Text("Текущая раскладка определяется".to_owned())));
        assert_eq!(find(&state.item_properties(MENU_SWITCH_LAYOUT).unwrap(), "enabled"), Some(Property::Flag(false)));
        assert_eq!(MenuState::property_default("toggle-state"), Some(Property::Number(-1)));
        assert_eq!(MenuState::property_default("nope"), None);
    }

    #[test]
    fn every_line_but_the_layout_and_separators_has_an_action() {
        let with_action: Vec<i32> = MENU_ITEM_IDS.iter().copied().filter(|id| action_of(*id).is_some()).collect();
        assert_eq!(with_action.len(), 9, "{with_action:?}");
        assert_eq!(action_of(MENU_QUIT), Some(TrayAction::Quit));
    }
}

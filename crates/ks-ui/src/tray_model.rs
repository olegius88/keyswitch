//! What a tray icon shows and what its menu offers (the Python's `tray_model.py`): the state, and the menu as a list of
//! lines resolved for that state. The tray window draws it; nothing here touches Windows.

use ks_config::indicator::{alternate_layout_action_label, alternate_layout_group, layout_label};

pub const CURRENT_LAYOUT_PREFIX: &str = "Текущая раскладка";
pub const SETTINGS_LABEL: &str = "Настройки KeySwitch…";
pub const AUTO_SWITCH_LABEL: &str = "Автопереключение";
pub const SOUND_LABEL: &str = "Звуковые эффекты";
pub const NOTIFICATIONS_LABEL: &str = "Уведомления об исправлениях";
pub const HISTORY_LABEL: &str = "История исправлений…";
pub const EXCLUSIONS_LABEL: &str = "Программы-исключения…";
pub const ABOUT_LABEL: &str = "О программе…";
pub const QUIT_LABEL: &str = "Выход";
/// The menu of a program that waits for macOS to let it watch the keyboard: what it waits for and where that is given.
pub const PERMISSION_WAIT_LABEL: &str = "Ждёт доступа к клавиатуре";
pub const PERMISSION_SETTINGS_LABEL: &str = "Открыть «Универсальный доступ»…";

/// `TrayActions`: what a menu line asks the application to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    ShowSettings,
    SwitchLayout,
    ToggleEngine,
    ToggleSound,
    ToggleNotifications,
    ShowHistory,
    ShowExclusions,
    ShowAbout,
    /// macOS: the pane of System Settings where the permission the program waits for is given.
    OpenPermissionSettings,
    QuitApplication,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayState {
    pub group: i64,
    pub enabled: bool,
    pub sound_enabled: bool,
    pub notifications_enabled: bool,
    pub indicator_style: &'static str,
    /// macOS: the program has no permission to watch the keyboard yet and waits for it.
    pub awaiting_permission: bool,
}

impl Default for TrayState {
    fn default() -> Self {
        TrayState { group: -1, enabled: true, sound_enabled: false, notifications_enabled: true, indicator_style: "letters", awaiting_permission: false }
    }
}

impl TrayState {
    #[must_use]
    pub fn label(&self) -> &'static str {
        layout_label(self.group)
    }

    #[must_use]
    pub fn alternate_layout_label(&self) -> String {
        alternate_layout_action_label(self.group)
    }

    #[must_use]
    pub fn can_switch_layout(&self) -> bool {
        alternate_layout_group(self.group).is_some()
    }

    /// The tooltip of the icon.
    #[must_use]
    pub fn title(&self) -> String {
        format!("KeySwitch — {}", self.label())
    }
}

/// One line of the menu. `action` None marks a line that only reports something, `checked` None a line that is not a
/// switch.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MenuEntry {
    pub label: String,
    pub action: Option<TrayAction>,
    pub checked: Option<bool>,
    pub enabled: bool,
    pub separator: bool,
    pub default: bool,
}

fn line(label: &str, action: Option<TrayAction>, checked: Option<bool>, enabled: bool, default: bool) -> MenuEntry {
    MenuEntry { label: label.to_owned(), action, checked, enabled, separator: false, default }
}

fn separator() -> MenuEntry {
    MenuEntry { separator: true, ..MenuEntry::default() }
}

/// The menu as it should look right now. While the program waits for the permission nothing else in it would work, so
/// it says so and offers the way to the switch, and a way out.
#[must_use]
pub fn menu_entries(state: &TrayState) -> Vec<MenuEntry> {
    use TrayAction::{SwitchLayout, ShowSettings, ToggleEngine, ToggleSound, ToggleNotifications, ShowHistory, ShowExclusions, ShowAbout, OpenPermissionSettings, QuitApplication};
    if state.awaiting_permission {
        return vec![
            line(PERMISSION_WAIT_LABEL, None, None, false, false),
            line(PERMISSION_SETTINGS_LABEL, Some(OpenPermissionSettings), None, true, true),
            separator(),
            line(QUIT_LABEL, Some(QuitApplication), None, true, false),
        ];
    }
    vec![
        line(&format!("{CURRENT_LAYOUT_PREFIX}: {}", state.label()), None, None, false, false),
        line(&state.alternate_layout_label(), Some(SwitchLayout), None, state.can_switch_layout(), false),
        line(SETTINGS_LABEL, Some(ShowSettings), None, true, true),
        separator(),
        line(AUTO_SWITCH_LABEL, Some(ToggleEngine), Some(state.enabled), true, false),
        line(SOUND_LABEL, Some(ToggleSound), Some(state.sound_enabled), true, false),
        line(NOTIFICATIONS_LABEL, Some(ToggleNotifications), Some(state.notifications_enabled), true, false),
        separator(),
        line(HISTORY_LABEL, Some(ShowHistory), None, true, false),
        line(EXCLUSIONS_LABEL, Some(ShowExclusions), None, true, false),
        line(ABOUT_LABEL, Some(ShowAbout), None, true, false),
        separator(),
        line(QUIT_LABEL, Some(QuitApplication), None, true, false),
    ]
}

/// `menu_activation_message`: a primary click opens the same menu as the secondary one.
#[must_use]
pub fn menu_activation_message(message: u32, primary_click: u32, menu_click: u32) -> u32 {
    if message == primary_click { menu_click } else { message }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn the_tooltip_names_the_program() {
        assert!(TrayState::default().title().starts_with("KeySwitch"));
    }

    #[test]
    fn the_menu_has_the_shape_of_the_python_one() {
        let entries = menu_entries(&TrayState { group: 1, ..TrayState::default() });
        assert_eq!(entries.len(), 13);
        assert_eq!(entries[0].label, "Текущая раскладка: RU");
        assert!(!entries[0].enabled && entries[0].action.is_none());
        assert_eq!(entries[1].label, "Переключить на английский (EN)");
        assert!(entries[2].default && entries[3].separator);
        assert_eq!(entries[4].checked, Some(true));
        let unknown = menu_entries(&TrayState::default());
        assert!(!unknown[1].enabled);
        assert_eq!(unknown[1].label, "Переключить язык");
    }

    #[test]
    fn a_program_waiting_for_the_permission_offers_its_pane_and_a_way_out() {
        let entries = menu_entries(&TrayState { group: 0, awaiting_permission: true, ..TrayState::default() });
        let labels: Vec<&str> = entries.iter().map(|entry| entry.label.as_str()).collect();
        assert_eq!(labels, [PERMISSION_WAIT_LABEL, PERMISSION_SETTINGS_LABEL, "", QUIT_LABEL]);
        assert!(!entries[0].enabled && entries[0].action.is_none());
        assert_eq!(entries[1].action, Some(TrayAction::OpenPermissionSettings));
        assert!(entries[1].enabled && entries[1].default);
        assert!(entries[2].separator);
        assert_eq!(entries[3].action, Some(TrayAction::QuitApplication));
        assert!(!TrayState::default().awaiting_permission);
    }

    #[test]
    fn a_primary_click_is_taken_for_the_menu_click() {
        assert_eq!(menu_activation_message(0x202, 0x202, 0x205), 0x205);
        assert_eq!(menu_activation_message(0x203, 0x202, 0x205), 0x203);
    }
}

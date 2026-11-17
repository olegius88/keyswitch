//! Whether the keyboard settings of the desktop answer both Shifts pressed together themselves, so that KeySwitch leaves
//! that chord to them (`detection.both_shifts_toggle`).
//!
//! The XKB options are read where each desktop keeps them: the X server's `_XKB_RULES_NAMES` (X11 sessions: GNOME, KDE,
//! Cinnamon, Xfce, MATE apply their options there, and `setxkbmap -query` reads them back), GNOME's
//! `org.gnome.desktop.input-sources xkb-options` (its Wayland session, where no X server holds them) and KDE's `kxkbrc`,
//! group `Layout`, key `Options` (its Wayland session). The options that give both Shifts to the system are those of
//! xkeyboard-config's `base.lst` that name both Shifts together.

use std::path::{Path, PathBuf};

use crate::system_keys_native::{gnome_xkb_options, x11_rules_names};

/// The XKB options that make both Shifts together do something, and what that is, as the settings window says it.
pub const BOTH_SHIFTS_OPTIONS: [(&str, &str); 4] = [
    ("grp:shifts_toggle", "переключают раскладку"),
    ("shift:both_capslock", "включают Caps Lock"),
    ("shift:both_capslock_cancel", "включают Caps Lock"),
    ("shift:both_shiftlock", "включают Shift Lock"),
];
/// The file, group and key KDE keeps its XKB options under.
const KDE_FILE_NAME: &str = "kxkbrc";
const KDE_GROUP: &str = "[Layout]";
const KDE_OPTIONS_KEY: &str = "Options";
/// The place of the options among the fields of `_XKB_RULES_NAMES`: rules, model, layout, variant, options.
const RULES_NAMES_OPTIONS_FIELD: usize = 4;

/// The options of a value of `_XKB_RULES_NAMES`: five strings, each ended by a NUL, the last of them the options joined by
/// commas.
#[must_use]
pub fn rules_names_options(property: &[u8]) -> Vec<String> {
    let field = property.split(|byte| *byte == 0).nth(RULES_NAMES_OPTIONS_FIELD).unwrap_or_default();
    split_options(&String::from_utf8_lossy(field))
}

fn split_options(text: &str) -> Vec<String> {
    text.split(',').map(str::trim).filter(|option| !option.is_empty()).map(str::to_owned).collect()
}

/// The options of a `kxkbrc`: the key `Options` of the group `Layout`.
#[must_use]
pub fn kxkbrc_options(text: &str) -> Vec<String> {
    let mut in_layout = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_layout = line == KDE_GROUP;
        } else if in_layout && let Some((key, value)) = line.split_once('=') && key.trim() == KDE_OPTIONS_KEY {
            return split_options(value);
        }
    }
    Vec::new()
}

/// KDE's settings folder: `$XDG_CONFIG_HOME`, else `~/.config`.
fn config_home(xdg: Option<PathBuf>, home: Option<PathBuf>) -> PathBuf {
    xdg.filter(|path| !path.as_os_str().is_empty()).unwrap_or_else(|| home.unwrap_or_default().join(".config"))
}

/// The options of the `kxkbrc` in `folder`; none where it is not there.
#[must_use]
pub fn kde_options(folder: &Path) -> Vec<String> {
    std::fs::read_to_string(folder.join(KDE_FILE_NAME)).map_or_default(|text| kxkbrc_options(&text))
}

/// What both Shifts together do on a desktop with these options, as the settings window says it; None when nothing.
#[must_use]
pub fn both_shifts_owner(options: &[String]) -> Option<&'static str> {
    BOTH_SHIFTS_OPTIONS.iter().find(|(option, _)| options.iter().any(|set| set == option)).map(|(_, what)| *what)
}

/// The XKB options of the desktop, wherever it keeps them.
#[must_use]
pub fn desktop_options() -> Vec<String> {
    let mut options = x11_rules_names().map_or_default(|property| rules_names_options(&property));
    options.extend(gnome_xkb_options().unwrap_or_default());
    options.extend(kde_options(&config_home(std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from), std::env::home_dir())));
    options
}

/// What both Shifts together do on this desktop; None when the system leaves them to KeySwitch.
#[must_use]
pub fn both_shifts_taken() -> Option<&'static str> {
    both_shifts_owner(&desktop_options())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn the_options_are_the_fifth_field_of_the_rules_names() {
        assert_eq!(rules_names_options(b"evdev\0pc105\0us,ru\0,\0grp:shifts_toggle,grp_led:scroll\0"), ["grp:shifts_toggle", "grp_led:scroll"]);
        assert!(rules_names_options(b"evdev\0pc105\0us\0\0\0").is_empty());
        assert!(rules_names_options(b"evdev").is_empty());
    }

    #[test]
    fn kde_keeps_its_options_under_layout() {
        let text = "[Layout]\nLayoutList=us,ru\nOptions=grp:alt_shift_toggle, shift:both_capslock\nResetOldOptions=true\n";
        assert_eq!(kxkbrc_options(text), ["grp:alt_shift_toggle", "shift:both_capslock"]);
        // An Options key of another group, or none at all.
        assert!(kxkbrc_options("[Other]\nOptions=grp:shifts_toggle\n[Layout]\nUse=true\n").is_empty());
        assert!(kxkbrc_options("").is_empty());
        let folder = std::env::temp_dir().join(format!("ks-system-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        assert!(kde_options(&folder).is_empty());
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("kxkbrc"), "[Layout]\nOptions=grp:shifts_toggle\n").unwrap();
        assert_eq!(kde_options(&folder), ["grp:shifts_toggle"]);
    }

    #[test]
    fn kde_settings_are_where_the_freedesktop_rules_put_them() {
        assert_eq!(config_home(Some(PathBuf::from("/x")), Some(PathBuf::from("/h"))), PathBuf::from("/x"));
        assert_eq!(config_home(Some(PathBuf::new()), Some(PathBuf::from("/h"))), PathBuf::from("/h/.config"));
        assert_eq!(config_home(None, None), PathBuf::from(".config"));
    }

    #[test]
    fn both_shifts_belong_to_the_system_for_these_options() {
        let options = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect::<Vec<String>>();
        assert_eq!(both_shifts_owner(&options(&["grp:alt_shift_toggle", "grp:shifts_toggle"])), Some("переключают раскладку"));
        assert_eq!(both_shifts_owner(&options(&["shift:both_capslock_cancel"])), Some("включают Caps Lock"));
        assert_eq!(both_shifts_owner(&options(&["shift:both_shiftlock"])), Some("включают Shift Lock"));
        // A single Shift switching the layout leaves the chord alone.
        assert_eq!(both_shifts_owner(&options(&["grp:lshift_toggle", "grp:rshift_toggle"])), None);
        // Whatever this desktop has, the answer is one of those.
        let found = both_shifts_taken();
        assert!(found.is_none_or(|what| BOTH_SHIFTS_OPTIONS.iter().any(|(_, said)| *said == what)));
    }
}

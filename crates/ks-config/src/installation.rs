//! A new installation told apart from one that ran before, and the keys a new installation starts with.
//!
//! A new installation works the Shift keys as the best-known switchers do (`NEW_INSTALLATION_*` in `constants`); one that
//! ran before keeps its settings exactly, whether or not it ever wrote a settings file: a user who never changed a setting
//! has none, but every start of every version writes its log into the data folder. So the program looks at its two
//! folders once, at the very start, before it writes anything there (`note_start`), and a start that found them empty is
//! the first start of a new installation.

use crate::config::{SettingsStore, config_dir};
use crate::constants::{
    DETECTION_BOTH_SHIFTS_TOGGLE, DETECTION_DOUBLE_TAP_CONVERTS, DETECTION_LAYOUT_SWITCH_KEY, NEW_INSTALLATION_BOTH_SHIFTS_TOGGLE,
    NEW_INSTALLATION_DOUBLE_TAP_CONVERTS, NEW_INSTALLATION_LAYOUT_SWITCH_KEY,
};
use crate::history::data_dir;
use std::io;
use std::path::Path;
use std::sync::OnceLock;

/// What `note_start` found.
static EARLIER_FILES: OnceLock<bool> = OnceLock::new();

/// Whether `folder` holds anything: a file or a folder. One that exists and cannot be listed is taken to hold something,
/// as a folder of an earlier installation would.
fn holds_anything(folder: &Path) -> bool {
    match std::fs::read_dir(folder) {
        Ok(mut entries) => entries.next().is_some(),
        Err(_) => folder.exists(),
    }
}

/// Whether the settings folder or the data folder holds anything an earlier start left: the settings file, the rules, the
/// history, the hints, the log, a downloaded update, a file set aside as unreadable.
#[must_use]
pub fn holds_earlier_files(config_folder: &Path, data_folder: &Path) -> bool {
    holds_anything(config_folder) || holds_anything(data_folder)
}

/// Looks at the program's folders, once per process, at the very start: before the log is opened, which writes into the
/// data folder at every start.
pub fn note_start() {
    note_start_in(&config_dir(), &data_dir());
}

/// `note_start` with the folders given.
pub fn note_start_in(config_folder: &Path, data_folder: &Path) {
    let _ = EARLIER_FILES.set(holds_earlier_files(config_folder, data_folder));
}

/// What `note_start` found: whether anything of an earlier start was there. None when the program did not look (a test
/// that builds the parts of the program itself).
#[must_use]
pub fn earlier_files_noted() -> Option<bool> {
    EARLIER_FILES.get().copied()
}

/// Writes the keys of a new installation into its settings and saves the file, so that a later change of the defaults
/// does not move them.
pub fn give_new_installation_keys(settings: &SettingsStore) -> io::Result<()> {
    const IN_MEMORY: &str = "a change not saved cannot fail";
    settings.write(DETECTION_LAYOUT_SWITCH_KEY, NEW_INSTALLATION_LAYOUT_SWITCH_KEY.to_owned(), false).expect(IN_MEMORY);
    settings.write(DETECTION_DOUBLE_TAP_CONVERTS, NEW_INSTALLATION_DOUBLE_TAP_CONVERTS, false).expect(IN_MEMORY);
    settings.write(DETECTION_BOTH_SHIFTS_TOGGLE, NEW_INSTALLATION_BOTH_SHIFTS_TOGGLE, false).expect(IN_MEMORY);
    settings.save()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::constants::{DEFAULT_BOTH_SHIFTS_TOGGLE, DEFAULT_DOUBLE_TAP_CONVERTS, DEFAULT_LAYOUT_SWITCH_KEY};
    use std::path::PathBuf;

    fn folder(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("ks-installation-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn a_folder_with_anything_in_it_is_an_earlier_start() {
        let (config, data) = (folder("config"), folder("data"));
        // Neither folder there, or both there and empty: nothing ran here before.
        assert!(!holds_earlier_files(&config, &data));
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        assert!(!holds_earlier_files(&config, &data));
        // The log of any start, or a folder of downloaded updates, is enough.
        std::fs::write(data.join("keyswitch.log"), "").unwrap();
        assert!(holds_earlier_files(&config, &data));
        std::fs::remove_file(data.join("keyswitch.log")).unwrap();
        std::fs::create_dir_all(config.join("anything")).unwrap();
        assert!(holds_earlier_files(&config, &data));
        // A file where the folder should be cannot be listed, and it is not nothing.
        let blocked = folder("blocked");
        std::fs::write(&blocked, "").unwrap();
        assert!(holds_earlier_files(&blocked, &folder("none")));
    }

    #[test]
    fn the_start_is_noted_once() {
        let (config, data) = (folder("noted-config"), folder("noted-data"));
        note_start_in(&config, &data);
        let first = earlier_files_noted();
        assert_eq!(first, Some(false));
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("keyswitch.log"), "").unwrap();
        // A second look changes nothing: the first one was made before anything was written.
        note_start();
        assert_eq!(earlier_files_noted(), first);
    }

    #[test]
    fn a_new_installation_gets_the_shift_keys_in_its_file() {
        let directory = folder("keys");
        let path = directory.join("config.json");
        let settings = SettingsStore::new(Some(path.clone()));
        assert_eq!(
            (settings.read(DETECTION_LAYOUT_SWITCH_KEY).as_deref(), settings.read(DETECTION_DOUBLE_TAP_CONVERTS), settings.read(DETECTION_BOTH_SHIFTS_TOGGLE)),
            (Some(DEFAULT_LAYOUT_SWITCH_KEY), Some(DEFAULT_DOUBLE_TAP_CONVERTS), Some(DEFAULT_BOTH_SHIFTS_TOGGLE))
        );
        give_new_installation_keys(&settings).unwrap();
        let again = SettingsStore::new(Some(path));
        assert_eq!(
            (again.read(DETECTION_LAYOUT_SWITCH_KEY).as_deref(), again.read(DETECTION_DOUBLE_TAP_CONVERTS), again.read(DETECTION_BOTH_SHIFTS_TOGGLE)),
            (Some("Shift"), Some(true), Some(true))
        );
        // A file where the folder of the settings should be: the keys stay in memory and the error says why.
        let blocked = folder("keys-blocked");
        std::fs::write(&blocked, "").unwrap();
        let unsaved = SettingsStore::new(Some(blocked.join("config.json")));
        assert!(give_new_installation_keys(&unsaved).is_err());
    }
}

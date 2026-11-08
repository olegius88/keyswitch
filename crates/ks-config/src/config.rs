//! Persistent, observable application settings.
//!
//! The file and its tree are kept by `settings_file`. A setting is named by a typed [`Key`] (`constants.rs` has one for
//! every setting): [`SettingsStore::read`] gives its value in that type and [`SettingsStore::write`] sets it, and
//! [`SettingsStore::subscribe_changes`] tells of each change as a [`SettingChange`]. The dotted-path calls next to them
//! serve the crates that are not typed yet.

use crate::constants::{SETTINGS_FILE_NAME, SettingChange, UNREADABLE_FILE_SUFFIX, UNREADABLE_FILE_TIME_FORMAT};
use crate::logsetup::{local_time, strftime, warning};
use crate::pycompat::with_name_suffix;
use crate::settings_file::Subscriber;
pub use crate::settings_file::{Key, SettingCallback, SettingsData, default_settings};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

const LOGGER_NAME: &str = "keyswitch.config";

/// What [`SettingsStore::subscribe_changes`] calls with each change.
pub type ChangeCallback = Arc<dyn Fn(&SettingChange) + Send + Sync>;
/// What [`SettingsStore::subscribe`] returns; calling it more than once is harmless.
pub type Unsubscribe = Box<dyn Fn() + Send + Sync>;

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// `Path.home()`.
pub fn home_dir() -> PathBuf {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env_nonempty(variable).map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// `config_dir()` with the environment given: `KEYSWITCH_CONFIG_DIR`, else `%APPDATA%\KeySwitch` on Windows and
/// `$XDG_CONFIG_HOME/keyswitch` (`~/.config/keyswitch`) elsewhere. macOS follows the second rule, as the Python does.
pub fn config_dir_from(windows: bool, home: &Path, env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    let get = |name: &str| env(name).filter(|value| !value.is_empty());
    if let Some(over) = get("KEYSWITCH_CONFIG_DIR") {
        return PathBuf::from(over);
    }
    if windows {
        let base = match get("APPDATA") {
            Some(roaming) => PathBuf::from(roaming),
            None => home.join("AppData").join("Roaming"),
        };
        return base.join("KeySwitch");
    }
    let base = match get("XDG_CONFIG_HOME") {
        Some(xdg) => PathBuf::from(xdg),
        None => home.join(".config"),
    };
    base.join("keyswitch")
}

#[must_use]
pub fn config_dir() -> PathBuf {
    config_dir_from(running_on_windows(), &home_dir(), &|name| std::env::var(name).ok())
}

fn running_on_windows() -> bool {
    cfg!(windows)
}

/// Move a user file that could not be read out of the way of the next save.
///
/// The application then runs on defaults, and saving them must not destroy what the user had: the bytes stay next to
/// the file under a dated name.
#[must_use]
pub fn set_aside_unreadable(path: &Path) -> Option<PathBuf> {
    let suffix = format!("{UNREADABLE_FILE_SUFFIX}{}", strftime(UNREADABLE_FILE_TIME_FORMAT, &local_time()));
    let target = with_name_suffix(path, &suffix);
    if let Ok(()) = std::fs::rename(path, &target) {
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        warning(LOGGER_NAME, &format!("{} could not be read; kept as {}, defaults in use", path.display(), name));
        Some(target)
    } else {
        warning(LOGGER_NAME, &format!("{} could not be read or moved aside", path.display()));
        None
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) struct Data {
    pub(crate) data: SettingsData,
    pub(crate) unreadable: Option<PathBuf>,
}

type Callbacks = Arc<Mutex<Vec<(u64, Subscriber)>>>;

/// Thread-safe JSON settings with dotted-path access and change callbacks.
pub struct SettingsStore {
    pub path: PathBuf,
    pub(crate) defaults: SettingsData,
    pub(crate) state: Mutex<Data>,
    pub(crate) callbacks: Callbacks,
    next_callback: Mutex<u64>,
}

impl SettingsStore {
    /// `SettingsStore(path)`: the file is read at once (`config_dir()/config.json` for `None`).
    #[must_use]
    pub fn new(path: Option<PathBuf>) -> SettingsStore {
        let defaults = default_settings(None);
        let store = SettingsStore {
            path: path.unwrap_or_else(|| config_dir().join(SETTINGS_FILE_NAME)),
            state: Mutex::new(Data { data: defaults.clone(), unreadable: None }),
            defaults,
            callbacks: Arc::new(Mutex::new(Vec::new())),
            next_callback: Mutex::new(0),
        };
        store.load();
        store
    }

    /// Where an unreadable settings file was moved by the last load, if anywhere.
    pub fn unreadable(&self) -> Option<PathBuf> {
        lock(&self.state).unreadable.clone()
    }

    /// Writes the settings: aside under a temporary name, then swapped in.
    pub fn save(&self) -> io::Result<()> {
        let state = lock(&self.state);
        self.save_locked(&state.data)
    }

    pub(crate) fn callbacks_snapshot(&self) -> Vec<Subscriber> {
        lock(&self.callbacks).iter().map(|(_, subscriber)| subscriber.clone()).collect()
    }

    fn add_subscriber(&self, subscriber: Subscriber) -> Unsubscribe {
        let id = {
            let mut next = lock(&self.next_callback);
            *next += 1;
            *next
        };
        lock(&self.callbacks).push((id, subscriber));
        let callbacks = Arc::clone(&self.callbacks);
        Box::new(move || {
            lock(&callbacks).retain(|(held, _)| *held != id);
        })
    }

    /// `subscribe(callback)`: called with the dotted path that changed (`"*"` for a reset) and its new value.
    pub fn subscribe(&self, callback: SettingCallback) -> Unsubscribe {
        self.add_subscriber(Subscriber::Tree(callback))
    }

    /// Called after each change of a setting of the table with what changed, and of a `reset` with `SettingChange::Reset`.
    pub fn subscribe_changes(&self, callback: ChangeCallback) -> Unsubscribe {
        self.add_subscriber(Subscriber::Change(callback))
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn the_folder_of_the_running_system_has_a_name() {
        let _ = config_dir();
    }

    #[test]
    fn paths_honor_overrides_and_xdg() {
        let home = Path::new("/home/u");
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
        };
        assert_eq!(config_dir_from(false, home, &env(&[("KEYSWITCH_CONFIG_DIR", "/tmp/key-conf")])), Path::new("/tmp/key-conf"));
        assert_eq!(config_dir_from(false, home, &env(&[("XDG_CONFIG_HOME", "/tmp/xdg")])), Path::new("/tmp/xdg/keyswitch"));
        assert_eq!(config_dir_from(false, home, &env(&[("XDG_CONFIG_HOME", "")])), Path::new("/home/u/.config/keyswitch"));
        assert_eq!(config_dir_from(true, home, &env(&[("APPDATA", "C:/R")])), Path::new("C:/R/KeySwitch"));
        assert_eq!(config_dir_from(true, home, &env(&[])), Path::new("/home/u/AppData/Roaming/KeySwitch"));
    }
}

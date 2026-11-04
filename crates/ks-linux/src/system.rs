//! Desktop integration helpers (the Python's `system.py`): the autostart entry of the freedesktop session.

use std::io;
use std::path::{Path, PathBuf};

use crate::constants::{APP_ID, APP_NAME};

/// The command that starts this program: the installed `keyswitch` on the search path, else a launcher beside the sources.
pub fn launcher_command() -> String {
    if let Some(installed) = find_in_path("keyswitch") {
        return shell_quote(&installed.to_string_lossy());
    }
    let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("keyswitch"));
    shell_quote(&executable.to_string_lossy())
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .filter(|directory| !directory.is_empty())
        .map(|directory| Path::new(directory).join(name))
        .find(|path| path.metadata().is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0))
}

/// `shlex.quote`.
pub fn shell_quote(text: &str) -> String {
    if text.is_empty() {
        return "''".to_owned();
    }
    let safe = text.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"@%+=:,./-_".contains(&byte));
    if safe { text.to_owned() } else { format!("'{}'", text.replace('\'', "'\"'\"'")) }
}

/// `autostart_path`: `$XDG_CONFIG_HOME/autostart/<app id>.desktop`.
pub fn autostart_path() -> PathBuf {
    let config_home = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(home) => PathBuf::from(home),
        None => std::env::home_dir().unwrap_or_default().join(".config"),
    };
    config_home.join("autostart").join(format!("{APP_ID}.desktop"))
}

pub struct AutostartManager {
    pub path: PathBuf,
}

impl AutostartManager {
    pub fn new(path: Option<PathBuf>) -> AutostartManager {
        AutostartManager { path: path.unwrap_or_else(autostart_path) }
    }

    /// Whether the entry exists and neither `Hidden` nor `X-GNOME-Autostart-enabled` turns it off.
    pub fn enabled(&self) -> bool {
        if !self.path.is_file() {
            return false;
        }
        let Ok(text) = std::fs::read_to_string(&self.path) else { return false };
        let mut fields: Vec<(String, String)> = Vec::new();
        for line in text.lines() {
            if let Some((key, value)) = line.split_once('=') {
                let key = ks_text::pystr::casefold(ks_text::pystr::strip(key));
                let value = ks_text::pystr::casefold(ks_text::pystr::strip(value));
                // A later line of the same key wins, as in a dict.
                fields.retain(|(name, _)| *name != key);
                fields.push((key, value));
            }
        }
        let field = |name: &str, default: &'static str| {
            fields.iter().find(|(key, _)| key == name).map_or(default.to_owned(), |(_, value)| value.clone())
        };
        field("hidden", "false") != "true" && field("x-gnome-autostart-enabled", "true") != "false"
    }

    pub fn set_enabled(&self, enabled: bool, start_hidden: bool) -> io::Result<()> {
        self.set_enabled_with(enabled, start_hidden, &launcher_command())
    }

    /// `set_enabled` with the launcher command given.
    pub fn set_enabled_with(&self, enabled: bool, start_hidden: bool, launcher: &str) -> io::Result<()> {
        if enabled {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let command = if start_hidden { format!("{launcher} --hidden") } else { launcher.to_owned() };
            let lines = [
                "[Desktop Entry]".to_owned(),
                "Type=Application".to_owned(),
                format!("Name={APP_NAME}"),
                "Comment=Автоматическое исправление раскладки клавиатуры".to_owned(),
                format!("Exec={command}"),
                "Icon=keyswitch".to_owned(),
                "Terminal=false".to_owned(),
                "StartupNotify=false".to_owned(),
                "Hidden=false".to_owned(),
                "X-GNOME-Autostart-enabled=true".to_owned(),
                String::new(),
            ];
            std::fs::write(&self.path, lines.join("\n"))
        } else if self.path.exists() {
            std::fs::remove_file(&self.path)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("ks-linux-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("temp dir");
        directory
    }

    #[test]
    fn quoting_is_shlex_quote() {
        assert_eq!(shell_quote("/usr/bin/keyswitch"), "/usr/bin/keyswitch");
        assert_eq!(shell_quote("/a b/it's"), "'/a b/it'\"'\"'s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn the_entry_is_written_read_and_removed() {
        let directory = temporary("autostart");
        let manager = AutostartManager::new(Some(directory.join("autostart").join("x.desktop")));
        assert!(!manager.enabled());
        manager.set_enabled_with(true, true, "/usr/bin/keyswitch").expect("write");
        let text = std::fs::read_to_string(&manager.path).expect("read");
        assert!(text.starts_with("[Desktop Entry]\nType=Application\nName=KeySwitch\n"));
        assert!(text.contains("\nExec=/usr/bin/keyswitch --hidden\n") && text.ends_with("X-GNOME-Autostart-enabled=true\n"));
        assert!(manager.enabled());
        std::fs::write(&manager.path, text.replace("Hidden=false", "Hidden=true")).expect("edit");
        assert!(!manager.enabled());
        std::fs::write(&manager.path, text.replace("enabled=true", "enabled=False")).expect("edit");
        assert!(!manager.enabled());
        manager.set_enabled_with(true, false, "/k").expect("write");
        assert!(std::fs::read_to_string(&manager.path).expect("read").contains("\nExec=/k\n"));
        manager.set_enabled_with(false, true, "/k").expect("remove");
        assert!(!manager.path.exists());
        manager.set_enabled_with(false, true, "/k").expect("removing nothing is fine");
        let _ = std::fs::remove_dir_all(directory);
    }
}

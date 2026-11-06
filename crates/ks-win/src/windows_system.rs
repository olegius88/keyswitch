//! Testable Windows profile integration and application discovery.
//!
//! The Python's launcher command also knew a source checkout, where the program is an interpreter. There is no
//! interpreter here: the command is always the program itself.

use std::collections::HashSet;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

pub use ks_backend::system_model::{Application, AutostartStatus};
use ks_text::pystr;

use crate::constants::{AUTOSTART_VALUE_NAME, STARTUP_APPROVAL_ENABLED_BYTES};

/// The name the Windows frontend uses, pointing at the shared shape.
pub type WindowsApplication = Application;

/// A failure of this module: its own (`WindowsSystemError` of the Python), the registry's (`OSError`), or a command in
/// the registry that cannot be split (`ValueError` from `shlex.split`).
#[derive(Debug)]
pub enum WindowsSystemError {
    Failed(String),
    Os(std::io::Error),
    Value(String),
}

impl fmt::Display for WindowsSystemError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowsSystemError::Failed(message) | WindowsSystemError::Value(message) => formatter.write_str(message),
            WindowsSystemError::Os(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for WindowsSystemError {}

impl From<std::io::Error> for WindowsSystemError {
    fn from(error: std::io::Error) -> Self {
        WindowsSystemError::Os(error)
    }
}

/// Starts a program with its arguments, without waiting for it.
pub type DirectoryOpener<'a> = dyn Fn(&[String]) -> std::io::Result<()> + 'a;

fn spawn_explorer(arguments: &[String]) -> std::io::Result<()> {
    if let Some((program, rest)) = arguments.split_first() {
        std::process::Command::new(program).args(rest).spawn()?;
    }
    Ok(())
}

/// Show a folder in Explorer.
///
/// Explorer reports success with a non-zero exit code, so the process is only started and never waited for; a missing
/// folder is reported here instead.
pub fn open_directory(path: &Path, spawn: Option<&DirectoryOpener<'_>>) -> Result<(), WindowsSystemError> {
    if !path.is_dir() {
        return Err(WindowsSystemError::Failed(format!("Каталог не найден: {}", path.display())));
    }
    let arguments = ["explorer".to_owned(), path.display().to_string()];
    match spawn {
        Some(spawn) => spawn(&arguments)?,
        None => spawn_explorer(&arguments)?,
    }
    Ok(())
}

pub trait WindowsRegistry: Send + Sync {
    fn read_autostart(&self, name: &str) -> std::io::Result<Option<String>>;
    fn write_autostart(&self, name: &str, command: &str) -> std::io::Result<()>;
    fn delete_autostart(&self, name: &str) -> std::io::Result<()>;
    fn read_startup_approval(&self, name: &str) -> std::io::Result<Option<Vec<u8>>>;
    fn clear_startup_approval(&self, name: &str) -> std::io::Result<()>;
    fn application_paths(&self) -> std::io::Result<Vec<(String, String)>>;
}

#[cfg(windows)]
#[cfg_attr(windows, expect(clippy::unnecessary_wraps, reason = "other systems answer with the error"))]
fn default_registry() -> Result<Arc<dyn WindowsRegistry>, WindowsSystemError> {
    Ok(Arc::new(crate::windows_registry::NativeWindowsRegistry))
}

#[cfg(not(windows))]
fn default_registry() -> Result<Arc<dyn WindowsRegistry>, WindowsSystemError> {
    Err(WindowsSystemError::Failed("Реестр Windows доступен только в Windows".to_owned()))
}

/// `os.path.isfile`: false for a path the system cannot evaluate at all - too long, or carrying a null - so a value left
/// in the registry by another install never raises here.
fn executable_exists(program: &str) -> bool {
    Path::new(program).is_file()
}

/// The packaged `KeySwitch.exe` beside the running program, when the program goes by another name.
fn installed_executable() -> Option<std::path::PathBuf> {
    let executable = std::env::current_exe().ok()?.parent()?.join("KeySwitch.exe");
    executable.is_file().then_some(executable)
}

/// Return a correctly quoted per-user startup command.
pub fn windows_launcher_command(start_hidden: bool, executable: Option<&Path>) -> Result<String, WindowsSystemError> {
    let program = match executable {
        Some(executable) => executable.to_path_buf(),
        None => std::env::current_exe()?,
    };
    let text = program.display().to_string();
    let stem = ntpath_splitext(ntpath_basename(&text)).0;
    let installed = if pystr::casefold(stem) == "keyswitch" { None } else { installed_executable() };
    let mut arguments = vec![installed.map_or(text, |path| path.display().to_string())];
    if start_hidden {
        arguments.push("--hidden".to_owned());
    }
    Ok(list2cmdline(&arguments))
}

/// `subprocess.list2cmdline`: the quoting of the MS C runtime.
#[must_use]
pub fn list2cmdline(arguments: &[String]) -> String {
    let mut result = String::new();
    for argument in arguments {
        let mut backslashes = 0;
        if !result.is_empty() {
            result.push(' ');
        }
        let needquote = argument.contains([' ', '\t']) || argument.is_empty();
        if needquote {
            result.push('"');
        }
        for character in argument.chars() {
            match character {
                '\\' => backslashes += 1,
                '"' => {
                    result.push_str(&"\\".repeat(backslashes * 2));
                    backslashes = 0;
                    result.push_str("\\\"");
                }
                _ => {
                    result.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                    result.push(character);
                }
            }
        }
        result.push_str(&"\\".repeat(backslashes));
        if needquote {
            result.push_str(&"\\".repeat(backslashes));
            result.push('"');
        }
    }
    result
}

/// `next(iter(shlex.split(command, posix=False)), "")`.
///
/// All of the command is split, as the Python's call does: a quote left open anywhere in it raises `ValueError`.
fn first_shell_word(command: &str) -> Result<String, WindowsSystemError> {
    const WHITESPACE: [char; 4] = [' ', '\t', '\r', '\n'];
    let mut first: Option<String> = None;
    let mut characters = command.chars();
    loop {
        // Between words.
        let Some(start) = characters.by_ref().find(|character| !WHITESPACE.contains(character)) else {
            return Ok(first.unwrap_or_default());
        };
        let mut word = String::from(start);
        if start == '"' || start == '\'' {
            // A quote at the start of a word runs to its partner and ends the word there.
            loop {
                let Some(character) = characters.next() else {
                    return Err(WindowsSystemError::Value("No closing quotation".to_owned()));
                };
                word.push(character);
                if character == start {
                    break;
                }
            }
        } else {
            // Anywhere else a quote is a character of the word.
            for character in characters.by_ref() {
                if WHITESPACE.contains(&character) {
                    break;
                }
                word.push(character);
            }
        }
        first.get_or_insert(word);
    }
}

pub type ExecutableCheck = Box<dyn Fn(&str) -> bool + Send + Sync>;

/// Manage the current user's `Run` value through a narrow adapter.
pub struct WindowsAutostartManager {
    registry: Arc<dyn WindowsRegistry>,
    command: Option<String>,
    /// Injected so the behaviour can be exercised off Windows, where a Windows path never resolves and every command would
    /// look like a missing target.
    exists: ExecutableCheck,
}

impl WindowsAutostartManager {
    pub fn new(
        registry: Option<Arc<dyn WindowsRegistry>>,
        command: Option<String>,
        exists: Option<ExecutableCheck>,
    ) -> Result<Self, WindowsSystemError> {
        Ok(WindowsAutostartManager {
            registry: match registry {
                Some(registry) => registry,
                None => default_registry()?,
            },
            command,
            exists: exists.unwrap_or_else(|| Box::new(executable_exists)),
        })
    }

    /// True only when the next logon really starts `KeySwitch`.
    pub fn enabled(&self) -> Result<bool, WindowsSystemError> {
        Ok(self.status()?.effective())
    }

    pub fn status(&self) -> Result<AutostartStatus, WindowsSystemError> {
        let command = self.registry.read_autostart(AUTOSTART_VALUE_NAME)?;
        let approval = self.registry.read_startup_approval(AUTOSTART_VALUE_NAME)?;
        // The first byte carries the state; see STARTUP_APPROVAL_ENABLED_BYTES.
        let blocked = approval
            .is_some_and(|approval| approval.first().is_none_or(|first| !STARTUP_APPROVAL_ENABLED_BYTES.contains(first)));
        let missing = match command.as_deref() {
            Some(command) if !command.is_empty() => !self.target_exists(command)?,
            _ => false,
        };
        Ok(AutostartStatus { command, blocked_by_windows: blocked, target_missing: missing })
    }

    fn target_exists(&self, command: &str) -> Result<bool, WindowsSystemError> {
        let word = first_shell_word(command)?;
        let program = pystr::strip_chars(&word, "\"");
        Ok(!program.is_empty() && (self.exists)(program))
    }

    /// Write or remove the value; `override_system_block` also lifts a Windows block.
    ///
    /// The startup sync at every launch must not fight a choice the user made in Windows, so only an explicit toggle in the
    /// `KeySwitch` interface clears the approval record.
    pub fn set_enabled(
        &self,
        enabled: bool,
        start_hidden: bool,
        override_system_block: bool,
    ) -> Result<(), WindowsSystemError> {
        if enabled {
            let command = match self.command.as_deref() {
                Some(command) if !command.is_empty() => command.to_owned(),
                _ => windows_launcher_command(start_hidden, None)?,
            };
            self.registry.write_autostart(AUTOSTART_VALUE_NAME, &command)?;
            if override_system_block {
                self.registry.clear_startup_approval(AUTOSTART_VALUE_NAME)?;
            }
        } else {
            self.registry.delete_autostart(AUTOSTART_VALUE_NAME)?;
        }
        Ok(())
    }
}

/// List registered executables in the form consumed by exclusions.
pub struct WindowsApplicationCatalog {
    registry: Arc<dyn WindowsRegistry>,
}

impl WindowsApplicationCatalog {
    pub fn new(registry: Option<Arc<dyn WindowsRegistry>>) -> Result<Self, WindowsSystemError> {
        Ok(WindowsApplicationCatalog {
            registry: match registry {
                Some(registry) => registry,
                None => default_registry()?,
            },
        })
    }

    pub fn installed(&self) -> Result<Vec<WindowsApplication>, WindowsSystemError> {
        let mut seen = HashSet::new();
        let mut applications = Vec::new();
        for (registered_name, executable) in self.registry.application_paths()? {
            let clean_executable = clean_windows_executable(&executable);
            let executable_name = ntpath_basename(&clean_executable);
            let registered_basename = ntpath_basename(pystr::strip(&registered_name));
            let identifier_source = if executable_name.is_empty() { registered_basename } else { executable_name };
            let identifier = pystr::casefold(ntpath_splitext(identifier_source).0);
            if identifier.is_empty() || !seen.insert(identifier.clone()) {
                continue;
            }
            let display_name = ntpath_splitext(registered_basename).0;
            applications.push(WindowsApplication {
                name: if display_name.is_empty() { identifier.clone() } else { display_name.to_owned() },
                identifier,
                executable: clean_executable,
            });
        }
        applications.sort_by_cached_key(|application| pystr::casefold(&application.name));
        Ok(applications)
    }

    #[must_use]
    pub fn from_executable(executable: &str) -> Option<WindowsApplication> {
        let clean_executable = clean_windows_executable(executable);
        let stem = ntpath_splitext(ntpath_basename(&clean_executable)).0.to_owned();
        let identifier = pystr::casefold(&stem);
        if identifier.is_empty() {
            return None;
        }
        Some(WindowsApplication { name: stem, identifier, executable: clean_executable })
    }
}

/// Remove quotes and a `DisplayIcon` index from an executable path.
#[must_use]
pub fn clean_windows_executable(value: &str) -> String {
    let text = pystr::strip(value);
    if let Some(rest) = text.strip_prefix('"')
        && let Some(closing_quote) = rest.find('"')
    {
        return rest.get(..closing_quote).unwrap_or_default().to_owned();
    }
    let mut kept = text;
    if let Some((candidate, icon_index)) = text.rsplit_once(',')
        && pystr::isdigit(pystr::lstrip_chars(pystr::strip(icon_index), "-"))
    {
        kept = candidate;
    }
    pystr::strip_chars(pystr::strip(kept), "\"").to_owned()
}

/// The drive and root of a path, and what follows: `ntpath.splitroot`, for the tail only.
fn ntpath_tail(path: &str) -> &str {
    let is_separator = |character: char| character == '\\' || character == '/';
    let mut characters = path.char_indices();
    let first = characters.next();
    let second = characters.next();
    let third = characters.next();
    match (first, second) {
        (Some((_, one)), Some((_, two))) if is_separator(one) && is_separator(two) => {
            // UNC drives, e.g. \\server\share or \\?\UNC\server\share; device drives, e.g. \\.\device or \\?\device.
            let unc = path.len() >= 8
                && path.as_bytes().iter().zip(b"\\\\?\\UNC\\").all(|(byte, wanted)| {
                    let normal = if *byte == b'/' { b'\\' } else { *byte };
                    normal.eq_ignore_ascii_case(wanted)
                });
            let start = if unc { 8 } else { 2 };
            let find_from = |from: usize| path.get(from..).and_then(|rest| rest.find(is_separator)).map(|at| at + from);
            let Some(index) = find_from(start) else {
                return "";
            };
            let Some(index2) = find_from(index + 1) else {
                return "";
            };
            path.get(index2 + 1..).unwrap_or_default()
        }
        // Relative path with root, e.g. \Windows.
        (Some((_, one)), _) if is_separator(one) => path.get(1..).unwrap_or_default(),
        (Some(_), Some((colon, ':'))) => {
            let after = colon + 1;
            // Absolute drive-letter path, e.g. X:\Windows; relative path with drive, e.g. X:Windows.
            match third {
                Some((_, three)) if is_separator(three) => path.get(after + 1..).unwrap_or_default(),
                _ => path.get(after..).unwrap_or_default(),
            }
        }
        // Relative path, e.g. Windows.
        _ => path,
    }
}

/// `ntpath.basename`.
#[must_use]
pub fn ntpath_basename(path: &str) -> &str {
    let tail = ntpath_tail(path);
    match tail.rfind(['\\', '/']) {
        Some(separator) => tail.get(separator + 1..).unwrap_or_default(),
        None => tail,
    }
}

/// `ntpath.splitext`: the extension is everything from the last dot of the last component, leading dots aside.
#[must_use]
pub fn ntpath_splitext(path: &str) -> (&str, &str) {
    let name_start = path.rfind(['\\', '/']).map_or(0, |separator| separator + 1);
    if let Some(dot) = path.rfind('.')
        && dot >= name_start
        && path.get(name_start..dot).unwrap_or_default().bytes().any(|byte| byte != b'.')
    {
        return path.split_at(dot);
    }
    (path, "")
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn an_icon_index_is_cut_only_when_it_is_a_number() {
        assert_eq!(clean_windows_executable("C:\\\\x\\\\a.exe,0"), "C:\\\\x\\\\a.exe");
        assert_eq!(clean_windows_executable("C:\\\\x\\\\a.exe,-3"), "C:\\\\x\\\\a.exe");
        assert_eq!(clean_windows_executable("C:\\\\x\\\\a,b.exe"), "C:\\\\x\\\\a,b.exe");
    }

    #[test]
    fn an_error_of_the_system_prints_its_own_words() {
        let error: WindowsSystemError = std::io::Error::other("no access").into();
        assert_eq!(error.to_string(), "no access");
    }

    #[test]
    fn explorer_is_only_started_and_nothing_starts_for_nothing() {
        assert!(spawn_explorer(&[]).is_ok());
        assert!(spawn_explorer(&["no-such-program-at-all".to_owned()]).is_err());
        assert!(open_directory(Path::new("/no/such/folder"), None).is_err());
        assert!(open_directory(&std::env::temp_dir(), None).is_ok() || cfg!(not(windows)));
    }

    #[test]
    fn the_startup_command_names_the_program_or_the_one_installed_beside_it() {
        let this = windows_launcher_command(false, None).expect("the running program");
        assert!(!this.is_empty());
        assert_eq!(windows_launcher_command(true, Some(Path::new("KeySwitch.exe"))).expect("named"), "KeySwitch.exe --hidden");
        assert_eq!(windows_launcher_command(false, Some(Path::new("C:\\x\\keyswitch.exe"))).expect("named"), "C:\\x\\keyswitch.exe");
        assert!(installed_executable().is_none(), "no such file beside a test");
    }

    #[test]
    fn paths_of_every_form_have_their_tail() {
        for (path, tail) in [
            ("//server/share/rest", "rest"),
            ("\\\\server\\share\\rest", "rest"),
            ("\\\\server", ""),
            ("\\\\server\\share", ""),
            ("//?/UNC/server/share/rest", "rest"),
            ("\\Windows", "Windows"),
            ("/Windows", "Windows"),
            ("X:\\Windows", "Windows"),
            ("X:Windows", "Windows"),
            ("X:/Windows", "Windows"),
            ("plain", "plain"),
            ("", ""),
        ] {
            assert_eq!(ntpath_tail(path), tail, "{path:?}");
        }
    }

    #[derive(Default)]
    struct FakeRegistry {
        autostart: Mutex<HashMap<String, String>>,
        startup_approval: Mutex<HashMap<String, Vec<u8>>>,
        apps: Mutex<Vec<(String, String)>>,
    }

    impl WindowsRegistry for FakeRegistry {
        fn read_autostart(&self, name: &str) -> std::io::Result<Option<String>> {
            Ok(self.autostart.lock().unwrap().get(name).cloned())
        }

        fn write_autostart(&self, name: &str, command: &str) -> std::io::Result<()> {
            self.autostart.lock().unwrap().insert(name.to_owned(), command.to_owned());
            Ok(())
        }

        fn delete_autostart(&self, name: &str) -> std::io::Result<()> {
            self.autostart.lock().unwrap().remove(name);
            Ok(())
        }

        fn read_startup_approval(&self, name: &str) -> std::io::Result<Option<Vec<u8>>> {
            Ok(self.startup_approval.lock().unwrap().get(name).cloned())
        }

        fn clear_startup_approval(&self, name: &str) -> std::io::Result<()> {
            self.startup_approval.lock().unwrap().remove(name);
            Ok(())
        }

        fn application_paths(&self) -> std::io::Result<Vec<(String, String)>> {
            Ok(self.apps.lock().unwrap().clone())
        }
    }

    const PRESENT: &str = "C:\\Programs\\KeySwitch\\KeySwitch.exe";

    fn manager(registry: &Arc<FakeRegistry>, command: &str, present: bool) -> WindowsAutostartManager {
        let exists: ExecutableCheck = Box::new(move |path| present && path == PRESENT);
        WindowsAutostartManager::new(Some(registry.clone()), Some(command.to_owned()), Some(exists))
            .unwrap()
    }

    #[test]
    fn the_autostart_manager_writes_and_removes_the_quoted_command() {
        let registry = Arc::new(FakeRegistry::default());
        let manager = manager(&registry, &format!("\"{PRESENT}\" --hidden"), true);
        assert!(!manager.enabled().unwrap());
        manager.set_enabled(true, true, false).unwrap();
        assert!(manager.enabled().unwrap());
        assert!(registry.autostart.lock().unwrap()["KeySwitch"].contains("--hidden"));
        manager.set_enabled(false, true, false).unwrap();
        assert!(!manager.enabled().unwrap());
    }

    #[test]
    fn a_program_registered_under_no_name_goes_by_its_file() {
        let registry = Arc::new(FakeRegistry::default());
        registry.apps.lock().unwrap().push((String::new(), "C:\\x\\Tool.exe".to_owned()));
        let catalog = WindowsApplicationCatalog::new(Some(registry)).unwrap();
        assert_eq!(catalog.installed().unwrap()[0].name, "tool");
    }

    #[test]
    fn an_empty_command_is_no_command() {
        let registry = Arc::new(FakeRegistry::default());
        registry.autostart.lock().unwrap().insert(AUTOSTART_VALUE_NAME.to_owned(), String::new());
        assert!(!manager(&registry, "", true).status().unwrap().target_missing);
        registry.autostart.lock().unwrap().insert(AUTOSTART_VALUE_NAME.to_owned(), "\"\"".to_owned());
        assert!(manager(&registry, "", true).status().unwrap().target_missing);
        // No command given: the program names itself.
        manager(&registry, "", true).set_enabled(true, false, false).unwrap();
        assert!(!registry.autostart.lock().unwrap()[AUTOSTART_VALUE_NAME].is_empty());
    }

    #[test]
    fn autostart_reports_what_the_next_logon_will_do() {
        let registry = Arc::new(FakeRegistry::default());
        let command = format!("\"{PRESENT}\" --hidden");
        let subject = manager(&registry, &command, true);
        subject.set_enabled(true, true, false).unwrap();
        assert_eq!(
            subject.status().unwrap(),
            AutostartStatus { command: Some(command.clone()), blocked_by_windows: false, target_missing: false }
        );
        // Task Manager's Startup tab disables the value; Windows then skips it at every logon.
        let approval = |first: &[u8]| {
            let mut value = first.to_vec();
            value.extend_from_slice(&[0; 11]);
            value
        };
        registry.startup_approval.lock().unwrap().insert("KeySwitch".to_owned(), approval(&[0x03]));
        assert!(!subject.enabled().unwrap());
        assert!(subject.status().unwrap().blocked_by_windows);
        // The automatic sync at every launch must not overrule that choice.
        subject.set_enabled(true, true, false).unwrap();
        assert!(!subject.enabled().unwrap());
        // An explicit toggle in the KeySwitch interface does lift it.
        subject.set_enabled(true, true, true).unwrap();
        assert!(subject.enabled().unwrap());
        assert!(!registry.startup_approval.lock().unwrap().contains_key("KeySwitch"));
        for (value, expected) in
            [(approval(&[0x02]), true), (approval(&[0x06]), true), (approval(&[0x01]), false), (Vec::new(), false)]
        {
            registry.startup_approval.lock().unwrap().insert("KeySwitch".to_owned(), value);
            assert_eq!(subject.enabled().unwrap(), expected);
        }
        registry.startup_approval.lock().unwrap().remove("KeySwitch");
        // A value left behind by another install location cannot start anything either.
        let stale = manager(&registry, &command, false);
        assert!(!stale.enabled().unwrap());
        assert!(stale.status().unwrap().target_missing);
    }

    #[test]
    fn a_command_with_a_quote_left_open_is_an_error_as_in_the_python() {
        let registry = Arc::new(FakeRegistry::default());
        registry.autostart.lock().unwrap().insert("KeySwitch".to_owned(), "\"unterminated".to_owned());
        let subject = manager(&registry, "x", true);
        assert!(matches!(subject.status(), Err(WindowsSystemError::Value(message)) if message == "No closing quotation"));
    }

    #[test]
    fn the_launcher_command_is_the_program_itself() {
        let path = Path::new(r"C:\Program Files\KeySwitch\KeySwitch.exe");
        assert_eq!(
            windows_launcher_command(true, Some(path)).unwrap(),
            "\"C:\\Program Files\\KeySwitch\\KeySwitch.exe\" --hidden"
        );
        assert_eq!(
            windows_launcher_command(false, Some(path)).unwrap(),
            "\"C:\\Program Files\\KeySwitch\\KeySwitch.exe\""
        );
    }

    #[test]
    fn the_default_target_check_reads_the_file_system_and_survives_a_bad_path() {
        let present = std::env::current_exe().unwrap();
        assert!(executable_exists(&present.display().to_string()));
        assert!(!executable_exists(&format!("{}/deeper", present.display())));
        for unusable in ["missing\0path".to_owned(), "x".repeat(70_000), String::new()] {
            assert!(!executable_exists(&unusable));
        }
    }

    #[test]
    fn the_catalog_deduplicates_and_parses_picker_paths() {
        let registry = Arc::new(FakeRegistry::default());
        *registry.apps.lock().unwrap() = [
            ("notepad.exe", r"C:\Windows\notepad.exe"),
            ("NOTEPAD.EXE", r"D:\Other\notepad.exe"),
            ("", ""),
            ("Браузер", r#""C:\Program Files\Browser\browser.exe",0"#),
        ]
        .map(|(name, path)| (name.to_owned(), path.to_owned()))
        .to_vec();
        let catalog = WindowsApplicationCatalog::new(Some(registry.clone())).unwrap();
        let applications = catalog.installed().unwrap();
        let identifiers: Vec<&str> = applications.iter().map(|item| item.identifier.as_str()).collect();
        assert_eq!(identifiers, ["notepad", "browser"]);
        assert_eq!(applications[1].name, "Браузер");
        assert_eq!(applications[1].executable, r"C:\Program Files\Browser\browser.exe");
        let selected = WindowsApplicationCatalog::from_executable(r#""C:\Program Files\Editor\Editor.exe""#).unwrap();
        assert_eq!(selected.identifier, "editor");
        assert_eq!(selected.name, "Editor");
        assert_eq!(clean_windows_executable(r#""C:\Program Files\Tool\tool.exe" -silent"#), r"C:\Program Files\Tool\tool.exe");
        assert_eq!(clean_windows_executable(r"C:\Tool\tool.exe,-12"), r"C:\Tool\tool.exe");
        assert_eq!(clean_windows_executable("\"unterminated.exe"), "unterminated.exe");
        assert_eq!(WindowsApplicationCatalog::from_executable(""), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn the_native_registry_is_guarded_outside_windows() {
        let manager_error = WindowsAutostartManager::new(None, None, None).err().unwrap();
        assert!(manager_error.to_string().contains("только в Windows"));
        let catalog_error = WindowsApplicationCatalog::new(None).err().unwrap();
        assert!(catalog_error.to_string().contains("только в Windows"));
    }

    #[test]
    fn open_directory_starts_explorer_and_rejects_a_missing_folder() {
        let directory = std::env::temp_dir();
        let started: Mutex<Vec<Vec<String>>> = Mutex::new(Vec::new());
        let spawn = |arguments: &[String]| {
            started.lock().unwrap().push(arguments.to_vec());
            Ok(())
        };
        open_directory(&directory, Some(&spawn)).unwrap();
        assert_eq!(started.lock().unwrap()[0], ["explorer".to_owned(), directory.display().to_string()]);
        let missing = directory.join("keyswitch-this-folder-is-not-there");
        let error = open_directory(&missing, Some(&spawn)).err().unwrap();
        assert!(error.to_string().contains("Каталог не найден"));
        assert_eq!(started.lock().unwrap().len(), 1);
    }

    /// The expected values were read from `subprocess.list2cmdline`, `CPython` 3.14.
    #[test]
    fn list2cmdline_quotes_as_the_python_does() {
        let line = |arguments: &[&str]| list2cmdline(&arguments.iter().map(|&item| item.to_owned()).collect::<Vec<_>>());
        assert_eq!(line(&[r"C:\Program Files\KeySwitch\KeySwitch.exe", "--hidden"]), r#""C:\Program Files\KeySwitch\KeySwitch.exe" --hidden"#);
        assert_eq!(line(&["C:\\a b\\\\", "x"]), "\"C:\\a b\\\\\\\\\" x");
        assert_eq!(line(&["a\"b", "c\\\"d", ""]), "a\\\"b c\\\\\\\"d \"\"");
        assert_eq!(line(&["tab\tx", "back\\slash\\\\"]), "\"tab\tx\" back\\slash\\\\");
    }

    /// The expected values were read from `shlex.split(command, posix=False)`, `CPython` 3.14.
    #[test]
    fn the_first_shell_word_is_the_python_one() {
        for (command, word) in [
            ("\"C:\\Key Switch\\KeySwitch.exe\" --hidden", "\"C:\\Key Switch\\KeySwitch.exe\""),
            ("C:\\Key Switch\\K.exe --hidden", "C:\\Key"),
            ("'a b' c", "'a b'"),
            ("x\"y z\"", "x\"y"),
            ("\"a b\"c d", "\"a b\""),
            ("  spaced\t\"q\"", "spaced"),
            ("", ""),
        ] {
            assert_eq!(first_shell_word(command).unwrap(), word, "{command}");
        }
        for command in ["\"unterminated", "a \"b"] {
            assert!(first_shell_word(command).is_err(), "{command}");
        }
    }

    /// The expected values were read from `ntpath`, `CPython` 3.14.
    #[test]
    fn the_path_functions_are_the_python_ones() {
        for (path, basename, splitext) in [
            (r"C:\Windows\notepad.exe", "notepad.exe", (r"C:\Windows\notepad", ".exe")),
            ("C:foo.txt", "foo.txt", ("C:foo", ".txt")),
            (r"\\server\share", "", (r"\\server\share", "")),
            (r"\\server\share\x.exe", "x.exe", (r"\\server\share\x", ".exe")),
            ("a/b.c/d", "d", ("a/b.c/d", "")),
            (r"C:\dir\.hidden", ".hidden", (r"C:\dir\.hidden", "")),
            (r"C:\dir\..a.b", "..a.b", (r"C:\dir\..a", ".b")),
            ("x.", "x.", ("x", ".")),
            (".", ".", (".", "")),
            (r"\\?\UNC\s\h\y.z", "y.z", (r"\\?\UNC\s\h\y", ".z")),
            ("//s/h/", "", ("//s/h/", "")),
            ("noext", "noext", ("noext", "")),
            ("", "", ("", "")),
        ] {
            assert_eq!(ntpath_basename(path), basename, "{path}");
            assert_eq!(ntpath_splitext(path), splitext, "{path}");
        }
    }
}

//! Autostart and the list of installed programs, as macOS keeps them. A launch agent is a file and the installed programs
//! are directories, so all of it is decided here and verified on any host.
use std::path::{Path, PathBuf};

use ks_backend::system_model::{Application, AutostartStatus};
use ks_text::pystr;

/// launchd reads every property list in this directory when the user logs in.
const LAUNCH_AGENTS_DIRECTORY: &str = "Library/LaunchAgents";
pub const AUTOSTART_LABEL: &str = "io.github.olegius88.KeySwitch";
const HIDDEN_ARGUMENT: &str = "--hidden";
const BUNDLE_SUFFIX: &str = ".app";
/// Where programs live on macOS: the system's own and a user's private one; all are read, none is required to exist.
const APPLICATION_DIRECTORIES: [&str; 3] = ["/Applications", "/System/Applications", "~/Applications"];
/// Apple keeps its smaller programs one level down, and installers use the same place.
const NESTED_DIRECTORIES: [&str; 1] = ["Utilities"];

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

pub fn launch_agent_path(base: Option<&Path>) -> PathBuf {
    base.map_or_else(home, Path::to_owned).join(LAUNCH_AGENTS_DIRECTORY).join(format!("{AUTOSTART_LABEL}.plist"))
}

fn escaped(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn unescaped(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// The agent's property list, in the XML form `plistlib` writes.
#[must_use]
pub fn agent_document(arguments: &[String]) -> String {
    let items: String = arguments.iter().map(|argument| format!("\t\t<string>{}</string>\n", escaped(argument))).collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>Label</key>\n\t<string>{AUTOSTART_LABEL}</string>\n\t<key>ProgramArguments</key>\n\t<array>\n{items}\t</array>\n\t<key>RunAtLoad</key>\n\t<true/>\n</dict>\n</plist>\n"
    )
}

/// What an agent will run, or nothing when it will not run at all: an agent whose `RunAtLoad` is off stays on disk and starts
/// nothing, so it is reported as no autostart.
#[must_use]
pub fn registered_arguments(document: &str) -> Vec<String> {
    let after = |key: &str| document.find(&format!("<key>{key}</key>")).map(|at| document.get(at + key.len() + 11..).unwrap_or_default());
    if !after("RunAtLoad").is_some_and(|rest| rest.trim_start().starts_with("<true/>")) {
        return Vec::new();
    }
    let Some(rest) = after("ProgramArguments") else { return Vec::new() };
    let Some(end) = rest.find("</array>") else { return Vec::new() };
    let mut arguments = Vec::new();
    let mut scan = rest.get(..end).unwrap_or_default();
    while let Some(start) = scan.find("<string>") {
        let tail = scan.get(start + 8..).unwrap_or_default();
        let Some(close) = tail.find("</string>") else { break };
        arguments.push(unescaped(tail.get(..close).unwrap_or_default()));
        scan = tail.get(close + 9..).unwrap_or_default();
    }
    arguments
}

/// One argument as `shlex.quote` writes it.
fn quoted(argument: &str) -> String {
    let safe = !argument.is_empty() && argument.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"@%+=:,./-_".contains(&byte));
    if safe { argument.to_owned() } else { format!("'{}'", argument.replace('\'', "'\"'\"'")) }
}

/// Manage the launch agent that starts `KeySwitch` when the user logs in.
pub struct MacAutostartManager {
    path: PathBuf,
    executable: PathBuf,
}

impl MacAutostartManager {
    #[must_use]
    pub fn new(path: Option<PathBuf>, executable: Option<PathBuf>) -> Self {
        MacAutostartManager {
            path: path.unwrap_or_else(|| launch_agent_path(None)),
            executable: executable.or_else(|| std::env::current_exe().ok()).unwrap_or_default(),
        }
    }

    #[must_use]
    pub fn status(&self) -> AutostartStatus {
        let arguments = std::fs::read_to_string(&self.path).map(|document| registered_arguments(&document)).unwrap_or_default();
        match arguments.first() {
            None => AutostartStatus { command: None, blocked_by_windows: false, target_missing: false },
            Some(program) => AutostartStatus {
                command: Some(arguments.iter().map(|argument| quoted(argument)).collect::<Vec<_>>().join(" ")),
                blocked_by_windows: false,
                target_missing: !Path::new(program).exists(),
            },
        }
    }

    /// Write or remove the agent. macOS keeps no separate refusal beside the agent, so removing the file is the whole of
    /// turning it off.
    pub fn set_enabled(&self, enabled: bool, start_hidden: bool) -> Result<(), String> {
        if !enabled {
            return match std::fs::remove_file(&self.path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(format!("не удалось удалить {}: {error}", self.path.display())),
            };
        }
        let mut arguments = vec![self.executable.display().to_string()];
        if start_hidden {
            arguments.push(HIDDEN_ARGUMENT.to_owned());
        }
        let written = self.path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&self.path, agent_document(&arguments)));
        written.map_err(|error| format!("не удалось записать {}: {error}", self.path.display()))
    }
}

/// List installed programs in the form the exclusion list consumes.
pub struct MacApplicationCatalog {
    directories: Vec<PathBuf>,
}

impl Default for MacApplicationCatalog {
    fn default() -> Self {
        let expand = |item: &str| item.strip_prefix("~/").map_or_else(|| PathBuf::from(item), |rest| home().join(rest));
        MacApplicationCatalog { directories: APPLICATION_DIRECTORIES.iter().map(|item| expand(item)).collect() }
    }
}

impl MacApplicationCatalog {
    #[must_use]
    pub fn with_directories(directories: Vec<PathBuf>) -> Self {
        MacApplicationCatalog { directories }
    }

    #[must_use]
    pub fn installed(&self) -> Vec<Application> {
        let mut applications: Vec<Application> = Vec::new();
        for directory in &self.directories {
            let places = std::iter::once(directory.clone()).chain(NESTED_DIRECTORIES.iter().map(|name| directory.join(name)));
            for place in places {
                let Ok(entries) = std::fs::read_dir(&place) else { continue };
                let mut names: Vec<PathBuf> = entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
                names.sort();
                for path in names.into_iter().filter(|path| path.to_string_lossy().ends_with(BUNDLE_SUFFIX)) {
                    if let Some(application) = from_executable(&path.display().to_string())
                        && !applications.iter().any(|known| known.identifier == application.identifier) {
                            applications.push(application);
                        }
                }
            }
        }
        applications.sort_by_key(|application| pystr::casefold(&application.name));
        applications
    }
}

/// Name a program by its bundle: the identifier is the bundle's name folded, because that is what the engine is told the
/// frontmost program is called.
#[must_use]
pub fn from_executable(executable: &str) -> Option<Application> {
    let path = executable.trim();
    let file = Path::new(path).file_name()?.to_string_lossy().into_owned();
    let name = file.strip_suffix(BUNDLE_SUFFIX).unwrap_or(&file).to_owned();
    let identifier = pystr::casefold(&name);
    if identifier.is_empty() {
        return None;
    }
    Some(Application { name, identifier, executable: path.to_owned() })
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn an_argument_is_quoted_only_when_it_needs_it() {
        assert_eq!(quoted("plain"), "plain");
        assert_eq!(quoted(""), "''");
        assert_eq!(quoted("a b"), "'a b'");
        assert_eq!(quoted("it's"), "'it'\"'\"'s'");
    }

    #[test]
    fn a_document_cut_short_registers_nothing_more() {
        let run = "<key>RunAtLoad</key><true/>";
        assert_eq!(registered_arguments(run), Vec::<String>::new());
        assert_eq!(registered_arguments(&format!("{run}<key>ProgramArguments</key><array><string>a</string>")), Vec::<String>::new());
        assert_eq!(registered_arguments(&format!("{run}<key>ProgramArguments</key><array><string>a</string><string>b</array>")), ["a"]);
        assert_eq!(registered_arguments("<key>ProgramArguments</key><array><string>a</string></array>"), Vec::<String>::new());
    }

    #[test]
    fn the_paths_come_from_the_home_folder_or_the_given_one() {
        assert!(launch_agent_path(None).ends_with(format!("{AUTOSTART_LABEL}.plist")));
        assert!(launch_agent_path(Some(Path::new("/base"))).starts_with("/base"));
        assert!(!MacApplicationCatalog::default().directories.is_empty());
    }

    #[test]
    fn a_name_of_nothing_is_no_program_and_programs_are_listed_once() {
        assert_eq!(from_executable(""), None);
        assert_eq!(from_executable(".app"), None);
        let folder = std::env::temp_dir().join(format!("ks-mac-apps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("a/Same.app")).expect("the folder");
        std::fs::create_dir_all(folder.join("b/same.app")).expect("the folder");
        std::fs::create_dir_all(folder.join("b/Other")).expect("the folder");
        // A bundle with no name is no program.
        std::fs::create_dir_all(folder.join("b/.app")).expect("the folder");
        let catalog = MacApplicationCatalog::with_directories(vec![folder.join("a"), folder.join("b"), folder.join("missing")]);
        let names: Vec<String> = catalog.installed().into_iter().map(|application| application.identifier).collect();
        assert_eq!(names, ["same"]);
        std::fs::remove_dir_all(&folder).expect("cleaned");
    }

    #[test]
    fn autostart_is_written_with_or_without_hiding_and_a_folder_in_its_place_is_an_error() {
        let folder = std::env::temp_dir().join(format!("ks-mac-autostart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let path = folder.join("agent.plist");
        let start = MacAutostartManager::new(Some(path.clone()), Some(std::env::current_exe().expect("this test")));
        start.set_enabled(false, false).expect("nothing to remove");
        start.set_enabled(true, false).expect("written");
        assert!(start.status().effective());
        start.set_enabled(true, true).expect("written");
        start.set_enabled(false, false).expect("removed");
        assert!(!start.status().effective());
        std::fs::create_dir_all(&path).expect("a folder in its place");
        assert!(start.set_enabled(false, false).is_err());
        std::fs::remove_dir_all(&folder).expect("cleaned");
    }

    fn temporary(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("ks-mac-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn an_agent_round_trips() {
        let directory = temporary("agent");
        let executable = directory.join("KeySwitch & Co");
        std::fs::write(&executable, "").unwrap();
        let manager = MacAutostartManager::new(Some(directory.join("a.plist")), Some(executable.clone()));
        assert!(!manager.status().effective());
        manager.set_enabled(true, true).unwrap();
        let status = manager.status();
        assert!(status.effective() && !status.target_missing);
        assert!(status.command.unwrap().ends_with(" --hidden"));
        std::fs::remove_file(&executable).unwrap();
        assert!(manager.status().target_missing);
        manager.set_enabled(false, true).unwrap();
        manager.set_enabled(false, true).unwrap();
        assert!(manager.status().command.is_none());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn an_agent_that_does_not_run_at_load_is_no_autostart() {
        let document = agent_document(&["/a".to_owned()]).replace("<true/>", "<false/>");
        assert!(registered_arguments(&document).is_empty());
        assert_eq!(registered_arguments(&agent_document(&["/a b".to_owned(), "--hidden".to_owned()])), ["/a b", "--hidden"]);
    }

    #[test]
    fn the_catalog_lists_bundles_by_their_folded_names() {
        let directory = temporary("catalog");
        for name in ["Zed.app", "Utilities/Terminal.app", "notes.txt"] {
            let path = directory.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        let found = MacApplicationCatalog::with_directories(vec![directory.clone()]).installed();
        let names: Vec<&str> = found.iter().map(|application| application.name.as_str()).collect();
        assert_eq!(names, ["Terminal", "Zed"]);
        assert_eq!(found[0].identifier, "terminal");
        std::fs::remove_dir_all(&directory).unwrap();
    }
}

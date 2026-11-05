//! The Windows application's entry (the Python's `windows_app.py` and the packaged `keyswitch_windows_entry.py`): the
//! command line, `--diagnose`, and the start of the program that `desktop_app` runs.

use std::path::{Path, PathBuf};

use ks_backend::backend::InputBackend;
use ks_config::logsetup;
use serde_json::{Map, Value, json};
use ks_text::pyjson::{Indent, JsonOptions, json_dumps};

use crate::windows_backend::WindowsBackend;
use crate::windows_system::WindowsAutostartManager;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const DIAGNOSTICS_JSON_INDENT: usize = 2;
const USAGE: &str = "usage: keyswitch [-h] [--hidden] [--diagnose] [--version]\n";
const LONG_OPTIONS: [&str; 6] = ["--help", "--hidden", "--diagnose", "--no-engine", "--smoke-ui", "--version"];

/// `build_parser`'s switches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Arguments {
    pub hidden: bool,
    pub diagnose: bool,
    pub no_engine: bool,
    pub smoke_ui: bool,
}

/// What argparse does with a command line: the switches, or text to print and a status to end with.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Arguments),
    Exit { code: i32, stdout: String, stderr: String },
}

fn error(message: &str) -> Parsed {
    Parsed::Exit { code: 2, stdout: String::new(), stderr: format!("{USAGE}keyswitch: error: {message}\n") }
}

/// `build_parser().parse_args(argv)`: long options may be shortened to any unambiguous prefix, as argparse allows.
#[must_use]
pub fn parse_arguments(arguments: &[String]) -> Parsed {
    let mut parsed = Arguments::default();
    let mut unrecognized = Vec::new();
    for argument in arguments {
        let option = if argument == "-h" {
            "--help"
        } else if argument.starts_with("--") && argument.len() > 2 {
            let (name, _) = argument.split_once('=').unwrap_or((argument, ""));
            if LONG_OPTIONS.contains(&name) {
                name
            } else {
                let matches: Vec<&'static str> = LONG_OPTIONS.iter().copied().filter(|option| option.starts_with(name)).collect();
                match matches.as_slice() {
                    [only] => *only,
                    [] => {
                        unrecognized.push(argument.as_str());
                        continue;
                    }
                    several => return error(&format!("ambiguous option: {name} could match {}", several.join(", "))),
                }
            }
        } else {
            unrecognized.push(argument.as_str());
            continue;
        };
        if argument.contains('=') && option != "--help" {
            return error(&format!("argument {option}: ignored explicit argument '{}'", argument.split_once('=').unwrap().1));
        }
        match option {
            "--help" => {
                let text = format!(
                    "{USAGE}\nАвтоматическое исправление раскладки EN/RU в Windows\n\noptions:\n  -h, --help  show this help message and exit\n  --hidden\n  --diagnose\n  --version   show program's version number and exit\n"
                );
                return Parsed::Exit { code: 0, stdout: text, stderr: String::new() };
            }
            "--version" => return Parsed::Exit { code: 0, stdout: format!("KeySwitch {VERSION}\n"), stderr: String::new() },
            "--hidden" => parsed.hidden = true,
            "--diagnose" => parsed.diagnose = true,
            "--no-engine" => parsed.no_engine = true,
            _ => parsed.smoke_ui = true,
        }
    }
    if unrecognized.is_empty() { Parsed::Run(parsed) } else { error(&format!("unrecognized arguments: {}", unrecognized.join(" "))) }
}

/// The resources folder beside the executable.
#[must_use]
pub fn resource_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent().map_or_else(|| PathBuf::from("resources"), |dir| dir.join("resources"))
}

/// What the next logon will really do, so a support report needs no registry editor.
pub(crate) fn autostart_status() -> Value {
    let status = WindowsAutostartManager::new(None, None, None).and_then(|manager| manager.status());
    match status {
        Ok(status) => json!({
            "command": status.command,
            "blocked_by_windows": status.blocked_by_windows,
            "target_missing": status.target_missing,
            "effective": status.effective(),
        }),
        Err(error) => json!({"error": format!("WindowsSystemError: {error}")}),
    }
}

#[cfg(windows)]
fn field_access() -> Value {
    let probe = crate::windows_context::probe_uia();
    let mut map = Map::new();
    map.insert("available".into(), json!(probe.available));
    if let Some(error) = probe.error {
        map.insert("error".into(), json!(error));
    }
    if let Some(value) = probe.focused_text_pattern {
        map.insert("focused_text_pattern".into(), json!(value));
    }
    if let Some(value) = probe.focused_probe_ms {
        map.insert("focused_probe_ms".into(), json!(value));
    }
    if let Some(error) = probe.focused_error {
        map.insert("focused_error".into(), json!(error));
    }
    if let Some(code) = probe.hresult {
        map.insert("hresult".into(), json!(code));
    }
    Value::Object(map)
}

#[cfg(not(windows))]
fn field_access() -> Value {
    json!({"available": false, "error": "UnsupportedPlatform"})
}

/// The report `--diagnose` prints, and whether the keyboard backend is available.
#[must_use]
pub fn diagnostics(resource_dir: &Path) -> (String, bool) {
    let resources = ks_lang::Resources::new(resource_dir);
    let models = resource_dir.join("models");
    let (backend, probe) = match WindowsBackend::new(None) {
        Ok(backend) => {
            let probe = backend.probe();
            (Some(backend), probe)
        }
        Err(failure) => (None, ks_backend::backend::BackendProbe { error: failure.to_string(), ..Default::default() }),
    };
    let (_intent_model, intent_status) = resources.intent_model();
    let (context_model, context_status) = ks_context::context_model::ContextModel::try_load(&models);
    let (prefix_available, prefix_status) = ks_context::prefix_schema::prefix_diagnostics(&models);
    let mut report = Map::new();
    report.insert("keyswitch".into(), json!(VERSION));
    report.insert("available".into(), json!(probe.available));
    report.insert("session_type".into(), json!(probe.session_type));
    report.insert("display".into(), json!(probe.display));
    report.insert("hook".into(), json!(probe.record_version));
    report.insert("injection".into(), json!(probe.xtest_version));
    report.insert("layouts".into(), json!(probe.xkb_version));
    report.insert("current_group".into(), json!(probe.current_group));
    report.insert("intent_model".into(), intent_status.as_dict());
    report.insert("context_model".into(), json!({"available": context_model.is_some(), "status": context_status}));
    report.insert("prefix_model".into(), json!({"available": prefix_available, "status": prefix_status}));
    report.insert("context_field_access".into(), field_access());
    report.insert("autostart".into(), autostart_status());
    report.insert("error".into(), json!(probe.error));
    if let Some(backend) = backend {
        backend.close();
    }
    // Keep redirected output valid even when a legacy console exposes a code page that cannot encode Russian.
    let options = JsonOptions { ensure_ascii: true, indent: Some(Indent::Spaces(DIAGNOSTICS_JSON_INDENT)), ..JsonOptions::default() };
    (json_dumps(&Value::Object(report), &options) + "\n", probe.available)
}

/// `main(argv)`: the status the process ends with.
#[must_use]
pub fn main(arguments: &[String]) -> i32 {
    let parsed = match parse_arguments(arguments) {
        Parsed::Exit { code, stdout, stderr } => {
            if !stdout.is_empty() {
                write_standard(false, &stdout);
            }
            if !stderr.is_empty() {
                write_standard(true, &stderr);
            }
            return code;
        }
        Parsed::Run(parsed) => parsed,
    };
    install_platform();
    let _ = logsetup::configure_logging(None);
    if parsed.diagnose {
        let (text, available) = diagnostics(&resource_dir());
        write_standard(false, &text);
        return i32::from(!available);
    }
    run(parsed)
}

#[cfg(windows)]
fn install_platform() {
    crate::desktop_app::install_platform();
}

#[cfg(not(windows))]
fn install_platform() {}

#[cfg(windows)]
fn run(parsed: Arguments) -> i32 {
    crate::desktop_app::run_with_instance(parsed)
}

#[cfg(not(windows))]
fn run(_parsed: Arguments) -> i32 {
    write_standard(true, "KeySwitch for Windows runs on Windows only\n");
    1
}

/// Standard output or error, also for a windowed program: redirected handles are used as they are, and a console the
/// program was started from is attached to.
#[cfg(windows)]
fn write_standard(error: bool, text: &str) {
    crate::desktop_app::write_standard(error, text);
}

#[cfg(not(windows))]
fn write_standard(error: bool, text: &str) {
    use std::io::Write;
    if error {
        let _ = std::io::stderr().write_all(text.as_bytes());
    } else {
        let _ = std::io::stdout().write_all(text.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Parsed {
        parse_arguments(&arguments.iter().map(ToString::to_string).collect::<Vec<_>>())
    }

    #[test]
    fn switches_and_prefixes() {
        assert_eq!(parse(&[]), Parsed::Run(Arguments::default()));
        assert_eq!(parse(&["--hidden", "--diag"]), Parsed::Run(Arguments { hidden: true, diagnose: true, ..Arguments::default() }));
        assert_eq!(parse(&["--smoke-ui"]), Parsed::Run(Arguments { smoke_ui: true, ..Arguments::default() }));
        assert_eq!(parse(&["--no-e"]), Parsed::Run(Arguments { no_engine: true, ..Arguments::default() }));
    }

    #[test]
    fn version_help_and_errors() {
        assert_eq!(parse(&["--version"]), Parsed::Exit { code: 0, stdout: format!("KeySwitch {VERSION}\n"), stderr: String::new() });
        assert!(matches!(parse(&["-h"]), Parsed::Exit { code: 0, .. }));
        assert!(matches!(parse(&["--h"]), Parsed::Exit { code: 2, .. }));
        assert!(matches!(parse(&["--bogus"]), Parsed::Exit { code: 2, .. }));
        assert!(matches!(parse(&["file"]), Parsed::Exit { code: 2, .. }));
    }
}

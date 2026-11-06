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
            return error(&format!("argument {option}: ignored explicit argument '{}'", argument.split_once('=').map_or("", |(_, value)| value)));
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
    field_access_of(crate::windows_context::probe_uia())
}

#[cfg(windows)]
fn field_access_of(probe: crate::windows_context::UiaProbe) -> Value {
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
#[cfg_attr(coverage_nightly, coverage(off))] // The whole program: its test is the installed one, in `e2e_windows.rs`.
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
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn off_windows_the_program_says_where_it_runs() {
        assert_eq!(main(&["--hidden".to_owned()]), 1);
    }

    /// It installs the program's logging, which goes once per process: it runs in a process of its own.
    #[test]
    fn the_diagnostic_mode_says_what_it_found_and_ends() {
        const CHILD: &str = "KS_TEST_DIAGNOSE_CHILD";
        if std::env::var_os(CHILD).is_some() {
            assert!(main(&["--diagnose".to_owned()]) <= 1);
            return;
        }
        let name = "windows_app::tests::the_diagnostic_mode_says_what_it_found_and_ends";
        let status = std::process::Command::new(std::env::current_exe().expect("this test"))
            .args(["--exact", name])
            .env(CHILD, "1")
            .env("KEYSWITCH_DATA_DIR", std::env::temp_dir().join(format!("ks-diagnose-{}", std::process::id())))
            .status()
            .expect("started");
        assert!(status.success());
        assert!(matches!(parse(&["--"]), Parsed::Exit { code: 2, .. }));
    }

    #[cfg(windows)]
    #[test]
    fn a_field_probe_reports_each_thing_it_has() {
        let probe = crate::windows_context::UiaProbe { available: true, error: Some("failed"), focused_text_pattern: Some(true), focused_probe_ms: Some(3), focused_error: Some("no focus"), hresult: Some(-1) };
        let map = field_access_of(probe);
        for name in ["available", "error", "focused_text_pattern", "focused_probe_ms", "focused_error", "hresult"] {
            assert!(map.get(name).is_some(), "{name}");
        }
        let empty = crate::windows_context::UiaProbe { available: false, error: None, focused_text_pattern: None, focused_probe_ms: None, focused_error: None, hresult: None };
        assert_eq!(field_access_of(empty).as_object().map(Map::len), Some(1));
    }

    #[test]
    fn every_way_a_command_line_can_go() {
        assert!(matches!(parse(&["--version"]), Parsed::Exit { code: 0, .. }));
        assert!(matches!(parse(&["-h"]), Parsed::Exit { code: 0, .. }));
        assert!(matches!(parse(&["--help=x"]), Parsed::Exit { code: 0, .. }));
        assert!(matches!(parse(&["--hidden=x"]), Parsed::Exit { code: 2, .. }));
        assert!(matches!(parse(&["--h"]), Parsed::Exit { code: 2, .. }));
        assert!(matches!(parse(&["--nothing"]), Parsed::Exit { code: 2, .. }));
        assert!(matches!(parse(&["plain"]), Parsed::Exit { code: 2, .. }));
        assert_eq!(parse(&["--diag", "--no-e", "--hidden"]), Parsed::Run(Arguments { hidden: true, diagnose: true, no_engine: true, smoke_ui: false }));
        assert_eq!(main(&["--version".to_owned()]), 0);
        assert_eq!(main(&["--nothing".to_owned()]), 2);
        assert!(resource_dir().ends_with("resources"));
    }

    #[test]
    fn the_report_names_the_backend_the_models_and_autostart() {
        let (report, _) = diagnostics(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources"));
        assert!(report.contains("intent_model"), "{report}");
        let (report, _) = diagnostics(Path::new("/nonexistent"));
        assert!(report.contains("intent_model"));
        assert!(autostart_status().is_object());
    }

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

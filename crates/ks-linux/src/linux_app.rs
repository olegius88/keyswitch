//! The Linux application's entry (the Python's `app.py`): the command line, `--diagnose`, and the engine run without a window.
//!
//! The windows, the tray and the rule prompt come with the GTK port; until then a run without `--headless` says so.

use ks_cast::To as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ks_backend::backend::{BackendProbe, InputBackend};
use ks_backend::{ManagedFieldReader, ReaderFailure};
use ks_config::config::SettingsStore;
use ks_config::history::{HistoryStore, data_dir, history_limit};
use ks_config::logsetup;
use ks_engine::context_access::ReaderFactory;
use ks_engine::engine::default_learning_store;
use ks_engine::engine_loader::{Build, DeferredEngine, Post};
use ks_engine::KeySwitchEngine;
use ks_text::pyjson::{Indent, JsonOptions, json_dumps};
use serde_json::{Map, Value, json};

use crate::atspi_bus;
use crate::atspi_context::{AtspiFieldReader, AtspiSelectionReader};
use crate::constants::BACKEND_LABEL;
use crate::spellcheck::hunspell_speller;
use crate::wayland_desktop::wayland_session_active;
use crate::x11_backend::{ProcessForWindow, SelectionFactory, X11Backend};
use crate::x11_native;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const LOGGER: &str = "keyswitch.app";
const DIAGNOSTICS_JSON_INDENT: usize = 2;
const USAGE: &str = "usage: keyswitch [-h] [--hidden] [--diagnose] [--enable-wayland-input] [--disable-wayland-input] [--version]\n";
const LONG_OPTIONS: [&str; 9] = [
    "--help",
    "--hidden",
    "--diagnose",
    "--no-engine",
    "--headless",
    "--wayland-input-helper",
    "--enable-wayland-input",
    "--disable-wayland-input",
    "--version",
];
const HELP: &str = "\nАвтоматическое исправление раскладки для Ubuntu (X11 и Wayland)\n\noptions:\n  -h, --help            show this help message and exit\n  --hidden              не показывать окно при запуске\n  --diagnose            проверить backend ввода и завершиться\n  --enable-wayland-input\n                        включить доступ к клавиатурам в сеансе Wayland (с правами администратора)\n  --disable-wayland-input\n                        выключить доступ к клавиатурам в сеансе Wayland (с правами администратора)\n  --version             show program's version number and exit\n";

/// `build_parser`'s switches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Arguments {
    pub hidden: bool,
    pub diagnose: bool,
    pub no_engine: bool,
    /// The engine without a window, until SIGTERM or SIGINT.
    pub headless: bool,
    pub enable_wayland_input: bool,
    pub disable_wayland_input: bool,
    /// The input helper this process is to become, with the descriptor of its socket.
    pub wayland_input_helper: Option<i32>,
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
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        index += 1;
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
        if option == "--wayland-input-helper" {
            // The helper takes a descriptor number.
            let value = argument.split_once('=').map(|(_, value)| value.to_owned()).or_else(|| {
                let next = arguments.get(index).cloned();
                if next.is_some() {
                    index += 1;
                }
                next
            });
            match value.as_deref() {
                Some(text) if text.parse::<i32>().is_ok() => {
                    parsed.wayland_input_helper = text.parse::<i32>().ok();
                    continue;
                }
                Some(text) => return error(&format!("argument --wayland-input-helper: invalid int value: '{text}'")),
                None => return error("argument --wayland-input-helper: expected one argument"),
            }
        }
        if argument.contains('=') && option != "--help" {
            return error(&format!("argument {option}: ignored explicit argument '{}'", argument.split_once('=').unwrap_or_default().1));
        }
        match option {
            "--help" => return Parsed::Exit { code: 0, stdout: format!("{USAGE}{HELP}"), stderr: String::new() },
            "--version" => return Parsed::Exit { code: 0, stdout: format!("KeySwitch {VERSION}\n"), stderr: String::new() },
            "--hidden" => parsed.hidden = true,
            "--diagnose" => parsed.diagnose = true,
            "--no-engine" => parsed.no_engine = true,
            "--headless" => parsed.headless = true,
            "--enable-wayland-input" => parsed.enable_wayland_input = true,
            _ => parsed.disable_wayland_input = true,
        }
    }
    if unrecognized.is_empty() { Parsed::Run(parsed) } else { error(&format!("unrecognized arguments: {}", unrecognized.join(" "))) }
}

/// The `resources` folder of the install: `KEYSWITCH_RESOURCES` for a build tree, else beside the executable, the bundle's
/// `keyswitch/resources` (the layout of `build-deb.sh`), then `/usr/lib/keyswitch/resources`.
pub fn resource_dir() -> PathBuf {
    resource_dir_from(std::env::var_os("KEYSWITCH_RESOURCES").map(PathBuf::from), std::env::current_exe().ok().as_deref(), &|path| path.is_dir())
}

pub fn resource_dir_from(over: Option<PathBuf>, executable: Option<&Path>, is_dir: &dyn Fn(&Path) -> bool) -> PathBuf {
    if let Some(over) = over.filter(|path| !path.as_os_str().is_empty()) {
        return over;
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(directory) = executable.and_then(Path::parent) {
        candidates.push(directory.join("resources"));
        candidates.push(directory.join("keyswitch").join("resources"));
    }
    candidates.push(PathBuf::from("/usr/lib/keyswitch/resources"));
    candidates.push(PathBuf::from("/usr/lib/keyswitch/keyswitch/resources"));
    candidates.iter().find(|path| is_dir(path)).cloned().unwrap_or_else(|| PathBuf::from("/usr/lib/keyswitch/resources"))
}

/// What a Wayland session reports: its backend's answers, or why there is no backend.
fn probe_of_wayland(built: Result<Arc<crate::wayland_backend::WaylandBackend>, String>) -> BackendProbe {
    match built {
        Ok(backend) => {
            let probe = backend.probe();
            backend.close();
            probe
        }
        Err(reason) => BackendProbe {
            session_type: std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "неизвестно".to_owned()),
            display: std::env::var("DISPLAY").unwrap_or_default(),
            error: reason,
            current_group: -1,
            ..BackendProbe::default()
        },
    }
}

/// The field reader over the accessibility bus, or why there is none.
fn reader_over(api: Result<Box<dyn crate::atspi_context::AtspiApi>, crate::atspi_context::AtspiError>, process: &ProcessForWindow) -> Result<Box<dyn ManagedFieldReader>, ReaderFailure> {
    let api = api.map_err(|error| error.failure())?;
    Ok(Box::new(AtspiFieldReader::new(api, Some(process.clone()))))
}

/// The backend with the readers of this platform: the field reader is built by the engine on its worker, the selection
/// reader on first use.
pub fn selection_factory() -> SelectionFactory {
    Box::new(|process| Box::new(AtspiSelectionReader::new(Box::new(atspi_bus::connect), process)))
}

pub fn new_backend() -> Arc<X11Backend> {
    Arc::new(X11Backend::new(Box::new(x11_native::connect), Some(selection_factory())))
}

/// The report `--diagnose` prints, and whether the keyboard backend is available.
#[must_use]
pub fn diagnostics(resource_dir: &Path) -> (String, bool) {
    let resources = ks_lang::Resources::new(resource_dir);
    let models = resource_dir.join("models");
    let probe = if wayland_session_active() {
        probe_of_wayland(crate::wayland_native::new_wayland_backend(None))
    } else {
        let backend = new_backend();
        let probe = backend.probe();
        backend.close();
        probe
    };
    let (_intent_model, intent_status) = resources.intent_model();
    let (context_model, context_status) = ks_context::context_model::ContextModel::try_load(&models);
    let (prefix_available, prefix_status) = ks_context::prefix_schema::prefix_diagnostics(&models);
    let mut report = Map::new();
    report.insert("keyswitch".into(), json!(VERSION));
    report.insert("available".into(), json!(probe.available));
    report.insert("session_type".into(), json!(probe.session_type));
    report.insert("display".into(), json!(probe.display));
    report.insert("record_version".into(), json!(probe.record_version));
    report.insert("xtest_version".into(), json!(probe.xtest_version));
    report.insert("xkb_version".into(), json!(probe.xkb_version));
    report.insert("current_group".into(), json!(probe.current_group));
    report.insert("intent_model".into(), intent_status.as_dict());
    report.insert("context_model".into(), json!({"available": context_model.is_some(), "status": context_status}));
    report.insert("prefix_model".into(), json!({"available": prefix_available, "status": prefix_status}));
    report.insert("error".into(), json!(probe.error));
    // `json.dumps(..., ensure_ascii=False, indent=2)`.
    let options = JsonOptions { ensure_ascii: false, indent: Some(Indent::Spaces(DIAGNOSTICS_JSON_INDENT)), ..JsonOptions::default() };
    (json_dumps(&Value::Object(report), &options) + "\n", probe.available)
}

fn write_standard(error: bool, text: &str) {
    use std::io::Write;
    if error {
        let _ = std::io::stderr().write_all(text.as_bytes());
    } else {
        let _ = std::io::stdout().write_all(text.as_bytes());
    }
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
    if let Some(socket) = parsed.wayland_input_helper {
        return crate::wayland_native::run_wayland_input_helper(socket);
    }
    if parsed.enable_wayland_input {
        return crate::wayland_access::enable_command();
    }
    if parsed.disable_wayland_input {
        return crate::wayland_access::disable_command();
    }
    crate::platform::install();
    let _ = logsetup::configure_logging(None);
    if parsed.diagnose {
        let (text, available) = diagnostics(&resource_dir());
        write_standard(false, &text);
        return i32::from(!available);
    }
    if parsed.headless {
        return crate::platform::run_headless(parsed.no_engine);
    }
    crate::gtk_app::run(parsed.hidden, parsed.no_engine)
}

/// Python's truthiness of a setting's value.
#[must_use]
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(flag) => *flag,
        Value::Null => false,
        Value::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

pub fn setting_bool(settings: &SettingsStore, path: &str, default: bool) -> bool {
    settings.get(path).map_or(default, |value| truthy(&value))
}

fn trim_zeros(text: &str) -> String {
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_owned() } else { text.to_owned() }
}

/// `format(value, "g")`: six significant digits, no trailing zeros.
#[must_use]
pub fn python_float(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.to_owned();
    }
    if value == 0.0 {
        return "0".to_owned();
    }
    let scientific = format!("{value:.5e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    if !(-4..6).contains(&exponent) {
        return format!("{}e{}{:02}", trim_zeros(mantissa), if exponent < 0 { '-' } else { '+' }, exponent.abs());
    }
    trim_zeros(&format!("{value:.*}", ((5 - exponent).max(0)).to::<usize>()))
}

/// The engine of this platform, built on the loader's thread: every model is read there.
pub fn engine_builder(
    settings: Arc<SettingsStore>,
    history: Arc<HistoryStore>,
    backend: Arc<dyn InputBackend>,
    process_for_window: ProcessForWindow,
    learning: Arc<ks_config::learning::LearningStore>,
) -> Build {
    let resources = ks_lang::Resources::with_speller(resource_dir(), hunspell_speller(data_dir()));
    Box::new(move || {
        let process = process_for_window;
        let factory: ReaderFactory = Arc::new(move || reader_over(atspi_bus::connect(), &process));
        KeySwitchEngine::new(
            settings,
            history,
            backend,
            Some(learning),
            BACKEND_LABEL,
            None,
            Some(factory),
            &resources,
            ks_context::input_context::monotonic_clock(),
        )
    })
}

/// The engine without a window: built, started, and stopped by SIGTERM or SIGINT. Says on standard error when it is ready.
pub fn run_headless_with(wait_for_signal: &dyn Fn(Duration) -> bool, no_engine: bool) -> i32 {
    let settings = Arc::new(SettingsStore::new(None));
    logsetup::follow_settings(&settings, None);
    let history = Arc::new(HistoryStore::new(
        None,
        history_limit(&settings.get("history.limit").unwrap_or(Value::from(ks_config::constants::DEFAULT_HISTORY_LIMIT))),
    ));
    let backend = new_backend();
    let process_for_window = backend.process_for_window();
    let learning = match default_learning_store(&settings, &history) {
        Ok(store) => Arc::new(store),
        Err(error) => {
            logsetup::error(LOGGER, &format!("Не удалось открыть правила переключения: {error}"));
            return 1;
        }
    };
    let enabled = setting_bool(&settings, "enabled", true);
    // No window: what the engine posts to "the window's thread" runs on the thread that posts it.
    let post: Post = Arc::new(|work| work());
    let engine = DeferredEngine::new(engine_builder(settings, history, backend, process_for_window, learning.clone()), learning, enabled, post, true);
    let failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    if !no_engine {
        let flag = failed.clone();
        let _ = engine.start(Arc::new(move |error: &str| {
            logsetup::error(LOGGER, &format!("Не удалось запустить клавиатурный слой\n{error}"));
            write_standard(true, &format!("keyswitch: start failed: {error}\n"));
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }));
    }
    let announced = std::sync::Once::new();
    loop {
        if wait_for_signal(Duration::from_millis(100)) {
            break;
        }
        if engine.ready() {
            announced.call_once(|| write_standard(true, "keyswitch: ready\n"));
        }
        if failed.load(std::sync::atomic::Ordering::SeqCst) {
            engine.stop();
            logsetup::uninstall_handler();
            return 1;
        }
    }
    engine.stop();
    logsetup::uninstall_handler();
    0
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    /// `format(value, "g")` as Python printed each.
    #[test]
    fn a_wayland_session_with_no_backend_reports_why() {
        let probe = probe_of_wayland(Err("why not".to_owned()));
        assert_eq!((probe.error.as_str(), probe.current_group), ("why not", -1));
    }

    #[test]
    fn a_reader_is_made_over_the_bus_or_says_why_not() {
        let process: ProcessForWindow = Arc::new(|_| 0);
        assert!(reader_over(Err(crate::atspi_context::AtspiError::Unavailable("no bus".to_owned())), &process).is_err());
        struct Nothing;
        impl crate::atspi_context::AtspiApi for Nothing {
            fn child_count(&self, _: Option<&crate::atspi_context::NodeRef>) -> crate::atspi_context::AtspiResult<usize> { Ok(0) }
            fn child_at(&self, _: Option<&crate::atspi_context::NodeRef>, _: usize) -> crate::atspi_context::AtspiResult<Option<crate::atspi_context::NodeRef>> { Ok(None) }
            fn name(&self, _: &crate::atspi_context::NodeRef) -> crate::atspi_context::AtspiResult<String> { Ok(String::new()) }
            fn process_id(&self, _: &crate::atspi_context::NodeRef) -> crate::atspi_context::AtspiResult<u32> { Ok(0) }
            fn states(&self, _: &crate::atspi_context::NodeRef) -> crate::atspi_context::AtspiResult<u64> { Ok(0) }
            fn role(&self, _: &crate::atspi_context::NodeRef) -> crate::atspi_context::AtspiResult<u32> { Ok(0) }
            fn has_interface(&self, _: &crate::atspi_context::NodeRef, _: &str) -> crate::atspi_context::AtspiResult<bool> { Ok(false) }
            fn n_selections(&self, _: &crate::atspi_context::NodeRef) -> crate::atspi_context::AtspiResult<i32> { Ok(0) }
            fn caret_offset(&self, _: &crate::atspi_context::NodeRef) -> crate::atspi_context::AtspiResult<i32> { Ok(0) }
            fn character_count(&self, _: &crate::atspi_context::NodeRef) -> crate::atspi_context::AtspiResult<i32> { Ok(0) }
            fn get_text(&self, _: &crate::atspi_context::NodeRef, _: i32, _: i32) -> crate::atspi_context::AtspiResult<String> { Ok(String::new()) }
            fn get_selection(&self, _: &crate::atspi_context::NodeRef, _: i32) -> crate::atspi_context::AtspiResult<(i32, i32)> { Ok((0, 0)) }
            fn set_selection(&self, _: &crate::atspi_context::NodeRef, _: i32, _: i32, _: i32) -> crate::atspi_context::AtspiResult<bool> { Ok(false) }
            fn delete_text(&self, _: &crate::atspi_context::NodeRef, _: i32, _: i32) -> crate::atspi_context::AtspiResult<bool> { Ok(false) }
            fn insert_text(&self, _: &crate::atspi_context::NodeRef, _: i32, _: &str, _: i32) -> crate::atspi_context::AtspiResult<bool> { Ok(false) }
            fn character_extents(&self, _: &crate::atspi_context::NodeRef, _: i32) -> crate::atspi_context::AtspiResult<(i32, i32, i32, i32)> { Ok((0, 0, 0, 0)) }
        }
        assert!(reader_over(Ok(Box::new(Nothing)), &process).is_ok());
    }

    #[test]
    fn help_takes_a_value_it_does_not_read() {
        assert!(matches!(parse(&["--help=x"]), Parsed::Exit { code: 0, .. }));
    }

    #[test]
    fn floats_read_as_python_formats_them() {
        for (value, text) in [
            (1.0, "1"), (1.5, "1.5"), (1e22, "1e+22"), (1e-5, "1e-05"), (0.0001, "0.0001"), (123_456.0, "123456"), (1_234_567.0, "1.23457e+06"), (f64::NAN, "nan"), (f64::INFINITY, "inf"),
            (f64::NEG_INFINITY, "-inf"), (0.0, "0"), (100.0, "100"), (0.5, "0.5"), (-1.5e-7, "-1.5e-07"), (2.5e10, "2.5e+10"),
        ] {
            assert_eq!(python_float(value), text);
        }
    }

    #[test]
    fn numbers_truth_and_settings_as_python_reads_them() {
        use serde_json::json;
        for (value, truth) in [(json!(null), false), (json!(0), false), (json!(1), true), (json!(""), false), (json!("x"), true), (json!([]), false), (json!([1]), true), (json!({}), false), (json!({"a": 1}), true), (json!(true), true), (json!(0.0), false)] {
            assert_eq!(truthy(&value), truth, "{value}");
        }
        assert_eq!((python_float(1.0), python_float(1.5), python_float(1e22)), ("1".to_owned(), "1.5".to_owned(), "1e+22".to_owned()));
        let settings = SettingsStore::new(None);
        assert!(setting_bool(&settings, "nothing.at.all", true));
    }

    #[test]
    fn the_report_of_the_program_and_the_command_line() {
        let (report, _) = diagnostics(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources"));
        assert!(report.contains("intent_model"), "{report}");
        assert!(matches!(parse(&["--version"]), Parsed::Exit { code: 0, .. }));
        assert!(matches!(parse(&["-h"]), Parsed::Exit { code: 0, .. }));
        assert!(matches!(parse(&["--hidden=x"]), Parsed::Exit { code: 2, .. }));
        assert!(matches!(parse(&["--h"]), Parsed::Exit { code: 2, .. }));
        assert!(matches!(parse(&["plain"]), Parsed::Exit { code: 2, .. }));
        assert_eq!(main(&["--version".to_owned()]), 0);
        assert_eq!(main(&["--nothing".to_owned()]), 2);
        let _ = selection_factory();
        let _ = resource_dir();
    }

    fn parse(arguments: &[&str]) -> Parsed {
        parse_arguments(&arguments.iter().map(ToString::to_string).collect::<Vec<_>>())
    }

    #[test]
    fn switches_and_prefixes() {
        assert_eq!(parse(&[]), Parsed::Run(Arguments::default()));
        assert_eq!(parse(&["--hidden", "--diag"]), Parsed::Run(Arguments { hidden: true, diagnose: true, ..Arguments::default() }));
        assert_eq!(parse(&["--headless"]), Parsed::Run(Arguments { headless: true, ..Arguments::default() }));
        assert_eq!(parse(&["--wayland-input-helper", "5"]), Parsed::Run(Arguments { wayland_input_helper: Some(5), ..Arguments::default() }));
        assert!(matches!(parse(&["--wayland-input-helper", "x"]), Parsed::Exit { code: 2, .. }));
    }

    #[test]
    fn version_help_and_errors() {
        assert_eq!(parse(&["--version"]), Parsed::Exit { code: 0, stdout: format!("KeySwitch {VERSION}\n"), stderr: String::new() });
        assert!(matches!(parse(&["-h"]), Parsed::Exit { code: 0, .. }));
        assert!(matches!(parse(&["--h"]), Parsed::Exit { code: 2, .. }), "--help, --hidden and --headless share the prefix");
        assert!(matches!(parse(&["--bogus"]), Parsed::Exit { code: 2, .. }));
        assert!(matches!(parse(&["file"]), Parsed::Exit { code: 2, .. }));
    }

    #[test]
    fn the_resources_are_found_where_the_package_puts_them() {
        let exe = Path::new("/usr/lib/keyswitch/keyswitch-bin");
        let only = |wanted: &'static str| move |path: &Path| path == Path::new(wanted);
        assert_eq!(resource_dir_from(None, Some(exe), &only("/usr/lib/keyswitch/resources")), Path::new("/usr/lib/keyswitch/resources"));
        assert_eq!(
            resource_dir_from(None, Some(exe), &only("/usr/lib/keyswitch/keyswitch/resources")),
            Path::new("/usr/lib/keyswitch/keyswitch/resources")
        );
        assert_eq!(resource_dir_from(None, Some(Path::new("/opt/k/keyswitch")), &only("/opt/k/resources")), Path::new("/opt/k/resources"));
        assert_eq!(resource_dir_from(Some("/x".into()), Some(exe), &|_| false), Path::new("/x"));
        assert_eq!(resource_dir_from(None, None, &|_| false), Path::new("/usr/lib/keyswitch/resources"));
    }
}

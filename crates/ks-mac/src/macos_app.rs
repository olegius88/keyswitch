//! The macOS application entry point: the command line, the permission and the diagnostics.
use crate::constants::SOURCE_AX;
use std::path::{Path, PathBuf};

use ks_backend::InputBackend;
use ks_text::{Indent, JsonOptions, json_dumps};
use serde_json::json;

use crate::macos_backend::MacBackend;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PERMISSION_REQUEST_MESSAGE: &str = "KeySwitch нужен доступ к клавиатуре. Открываю «Конфиденциальность и безопасность» → «Универсальный доступ»: включите там KeySwitch, и программа продолжит сама.";
pub const PERMISSION_GRANTED_MESSAGE: &str = "Доступ получен, KeySwitch начинает работу.";
const USAGE: &str = "usage: keyswitch [-h] [--hidden] [--diagnose] [--version]\n";

/// The `resources` folder of the bundle (`Contents/Resources/resources`), beside the executable in a build tree, or the
/// one `KEYSWITCH_RESOURCES` names.
#[must_use]
pub fn resource_dir() -> PathBuf {
    if let Some(over) = std::env::var_os("KEYSWITCH_RESOURCES").filter(|value| !value.is_empty()) {
        return PathBuf::from(over);
    }
    let executable = std::env::current_exe().unwrap_or_default();
    resource_dir_from(&executable, &|path| path.is_dir())
}

pub fn resource_dir_from(executable: &Path, is_dir: &dyn Fn(&Path) -> bool) -> PathBuf {
    let directory = executable.parent().unwrap_or(Path::new("."));
    let bundled = directory.join("../Resources/resources");
    if is_dir(&bundled) {
        return bundled;
    }
    directory.join("resources")
}

/// Everything a support report needs, without starting the engine.
#[must_use]
pub fn diagnose() -> i32 {
    let backend = match MacBackend::new(None) {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let probe = backend.probe();
    let permission = backend.permission_granted();
    let resources = ks_lang::Resources::new(resource_dir());
    let models = resource_dir().join("models");
    let (_intent, intent_status) = resources.intent_model();
    let (context_model, context_status) = ks_context::context_model::ContextModel::try_load(&models);
    let (prefix_available, prefix_status) = ks_context::prefix_schema::prefix_diagnostics(&models);
    let report = json!({
        "keyswitch": VERSION,
        "available": probe.available,
        "session_type": probe.session_type,
        "display": probe.display,
        "hook": probe.record_version,
        "injection": probe.xtest_version,
        "layouts": probe.xkb_version,
        "current_group": probe.current_group,
        "intent_model": intent_status.as_dict(),
        "context_model": {"available": context_model.is_some(), "status": context_status},
        "prefix_model": {"available": prefix_available, "status": prefix_status},
        // The accessibility tree answers both questions, so the reader is available exactly when the permission is.
        "context_field_access": {"available": permission, "source": SOURCE_AX},
        "accessibility_permission": permission,
        "error": probe.error,
    });
    let options = JsonOptions { ensure_ascii: false, indent: Some(Indent::Spaces(2)), ..JsonOptions::default() };
    println!("{}", json_dumps(&report, &options));
    backend.close();
    i32::from(!probe.available)
}

#[must_use]
pub fn main(arguments: &[String]) -> i32 {
    let (mut hidden, mut wants_diagnose, mut no_engine) = (false, false, false);
    for argument in arguments {
        match argument.as_str() {
            "--hidden" => hidden = true,
            "--diagnose" => wants_diagnose = true,
            "--no-engine" => no_engine = true,
            "--version" => {
                println!("KeySwitch {VERSION}");
                return 0;
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return 0;
            }
            other => {
                eprintln!("{USAGE}keyswitch: error: unrecognized arguments: {other}");
                return 2;
            }
        }
    }
    run(hidden, wants_diagnose, no_engine)
}

#[cfg(target_os = "macos")]
fn run(hidden: bool, wants_diagnose: bool, no_engine: bool) -> i32 {
    crate::desktop_app::install_platform();
    let _ = ks_config::logsetup::configure_logging(None);
    if wants_diagnose {
        return diagnose();
    }
    crate::desktop_app::run(hidden, no_engine)
}

#[cfg(not(target_os = "macos"))]
fn run(_hidden: bool, wants_diagnose: bool, _no_engine: bool) -> i32 {
    if wants_diagnose {
        return diagnose();
    }
    eprintln!("Эта сборка работает только в macOS");
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_folder_comes_first() {
        let executable = Path::new("/A.app/Contents/MacOS/KeySwitch");
        assert_eq!(resource_dir_from(executable, &|_| true), Path::new("/A.app/Contents/MacOS/../Resources/resources"));
        assert_eq!(resource_dir_from(executable, &|_| false), Path::new("/A.app/Contents/MacOS/resources"));
    }

    #[test]
    fn unknown_options_are_refused() {
        assert_eq!(main(&["--version".to_owned()]), 0);
        assert_eq!(main(&["--nope".to_owned()]), 2);
    }
}

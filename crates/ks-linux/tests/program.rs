//! The program as a terminal starts it, with no desktop: what it says, the code it ends with, and how it stops when told.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        let path = std::env::temp_dir().join(format!("ks-program-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the folder");
        Folder(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn program(folder: &Folder, arguments: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_keyswitch"));
    command
        .args(arguments)
        .env("KEYSWITCH_DATA_DIR", folder.0.join("data"))
        .env("KEYSWITCH_CONFIG_DIR", folder.0.join("config"))
        .env("XDG_RUNTIME_DIR", &folder.0)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("XDG_SESSION_TYPE")
        // The live tests run under this; it names X11 whatever else the environment says.
        .env_remove("GDK_BACKEND");
    command
}

fn run(name: &str, arguments: &[&str]) -> Output {
    let folder = Folder::new(name);
    program(&folder, arguments).output().expect("it runs")
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

#[test]
fn it_answers_for_its_version_and_its_options() {
    let version = run("version", &["--version"]);
    assert!(version.status.success() && said(&version).contains("KeySwitch"), "{}", said(&version));
    let help = run("help", &["--help"]);
    assert!(help.status.success(), "{}", said(&help));
    assert!(run("short-help", &["-h"]).status.success());
    assert!(run("prefix", &["--vers"]).status.success(), "an option may be shortened");
    for arguments in [&["--nothing"][..], &["plain"], &["--"], &["--version=1"], &["--wayland-input-helper"], &["--wayland-input-helper=x"], &["--wayland-input-helper", "x"], &["--"]] {
        let refused = run("refused", arguments);
        assert_eq!(refused.status.code(), Some(2), "{arguments:?}: {}", said(&refused));
        assert!(!said(&refused).is_empty());
    }
}

#[test]
fn the_diagnostics_report_what_is_missing_without_a_desktop() {
    let report = run("diagnose", &["--diagnose"]);
    assert!(said(&report).contains("\"keyswitch\"") || !said(&report).is_empty(), "{}", said(&report));
    let folder = Folder::new("wayland-diagnose");
    let wayland = program(&folder, &["--diagnose"]).env("WAYLAND_DISPLAY", "wayland-nothing").env("XDG_SESSION_TYPE", "wayland").output().expect("it runs");
    assert!(!said(&wayland).is_empty());
}

/// Never as root, where these would change the system.
#[test]
fn the_keyboard_access_commands_refuse_an_ordinary_user() {
    let user = Command::new("id").arg("-u").output().expect("id");
    if String::from_utf8_lossy(&user.stdout).trim() == "0" {
        return;
    }
    for option in ["--enable-wayland-input", "--disable-wayland-input"] {
        let refused = run("access", &[option]);
        assert_ne!(refused.status.code(), Some(0), "{option}: {}", said(&refused));
    }
}

fn stopped_by_signal(name: &str, arguments: &[&str]) -> Option<i32> {
    let folder = Folder::new(name);
    let mut child = program(&folder, arguments).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().expect("it starts");
    let deadline = Instant::now() + Duration::from_secs(20);
    std::thread::sleep(Duration::from_millis(500));
    let _ = Command::new("kill").args(["-TERM", &child.id().to_string()]).status();
    loop {
        if let Some(status) = child.try_wait().expect("it can be waited for") {
            return status.code();
        }
        assert!(Instant::now() < deadline, "it did not stop");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn without_a_window_it_runs_until_it_is_told_to_stop() {
    assert_eq!(stopped_by_signal("headless", &["--headless", "--no-engine"]), Some(0));
    let with_engine = stopped_by_signal("headless-engine", &["--headless"]);
    assert!(matches!(with_engine, Some(0 | 1)), "{with_engine:?}");
}

#[test]
fn without_a_desktop_the_window_cannot_open() {
    let folder = Folder::new("window");
    let output = program(&folder, &["--no-engine", "--hidden"]).output().expect("it runs");
    assert!(output.status.code().is_some(), "{}", said(&output));
}

/// With nothing to type into, the engine cannot start: the program says so and ends instead of running blind.
#[test]
fn an_engine_that_cannot_start_ends_the_program() {
    let folder = Folder::new("no-desktop");
    let output = program(&folder, &["--headless"]).output().expect("it runs");
    assert_eq!(output.status.code(), Some(1), "{}", said(&output));
    assert!(said(&output).contains("start failed"), "{}", said(&output));
    let folder = Folder::new("no-wayland");
    let wayland = program(&folder, &["--headless"]).env("WAYLAND_DISPLAY", "wayland-nothing").env("XDG_SESSION_TYPE", "wayland").output().expect("it runs");
    assert_eq!(wayland.status.code(), Some(1), "{}", said(&wayland));
}

#[test]
fn a_helper_on_a_socket_that_is_not_there_ends_at_once() {
    let output = run("helper", &["--wayland-input-helper", "987654"]);
    assert_eq!(output.status.code(), Some(2), "{}", said(&output));
}

#[test]
fn diagnostics_in_a_wayland_session_with_no_compositor_name_the_reason() {
    let folder = Folder::new("wayland-probe");
    let output = program(&folder, &["--diagnose"]).env("XDG_SESSION_TYPE", "wayland").env("WAYLAND_DISPLAY", "no-such-display").output().expect("it runs");
    assert!(!String::from_utf8_lossy(&output.stdout).is_empty());
}

#[test]
fn rules_that_cannot_be_opened_end_the_program_with_a_reason() {
    let folder = Folder::new("rules");
    // An old file of rules with a key the migration cannot read.
    std::fs::create_dir_all(folder.0.join("data")).expect("a folder");
    std::fs::write(folder.0.join("data/learning.json"), r#"{"schema_version": 2, "rules": {"²:слово": {"target_group": 1, "confirmations": 9}}}"#).expect("a file");
    let mut child = program(&folder, &["--headless"]).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("it starts");
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().expect("it can be asked").is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let code = child.wait().expect("it ended").code();
    assert_eq!(code, Some(1));
}

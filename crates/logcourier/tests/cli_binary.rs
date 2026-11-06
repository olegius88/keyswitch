//! The program as a terminal starts it: what it prints and the code it ends with.

use std::path::PathBuf;
use std::process::{Command, Output};

#[cfg(unix)]
const TOKEN: &str = "123456789:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        let path = std::env::temp_dir().join(format!("lc-cli-{name}-{}", std::process::id()));
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

fn run(data: &std::path::Path, arguments: &[&str], token: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_logcourier-cli"));
    command.args(arguments).env("LOGCOURIER_DATA_DIR", data).env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY").env_remove("LOGCOURIER_BOT_TOKEN");
    if let Some(token) = token {
        command.env("LOGCOURIER_BOT_TOKEN", token);
    }
    command.output().expect("it runs")
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

#[test]
fn it_answers_for_its_version_and_status() {
    let data = Folder::new("status");
    let version = run(&data.0, &["--version"], None);
    assert!(version.status.success() && said(&version).trim() == logcourier::VERSION, "{}", said(&version));
    let status = run(&data.0, &["status"], None);
    assert!(status.status.success(), "{}", said(&status));
    assert!(said(&status).contains("\"version\""), "{}", said(&status));
}

#[test]
fn it_says_what_is_missing() {
    let data = Folder::new("missing");
    let wrong = run(&data.0, &["nonsense"], None);
    assert!(!wrong.status.success() && said(&wrong).contains("invalid choice"), "{}", said(&wrong));
    let no_group = run(&data.0, &["list"], None);
    assert!(said(&no_group).contains("--chat-id"), "{}", said(&no_group));
    let bad_option = run(&data.0, &["list", "--no-such-option"], None);
    assert!(!bad_option.status.success(), "{}", said(&bad_option));
    let no_output = run(&data.0, &["fetch", "--chat-id", "-1001"], None);
    assert!(said(&no_output).contains("--output"), "{}", said(&no_output));
    let no_token = run(&data.0, &["list", "--chat-id", "-1001"], None);
    assert!(said(&no_token).contains("токена"), "{}", said(&no_token));
}

#[test]
fn a_damaged_configuration_is_reported_in_words() {
    let data = Folder::new("damaged");
    std::fs::write(data.0.join("config.json"), "not json").expect("the file");
    let broken = run(&data.0, &["status"], None);
    assert!(!broken.status.success(), "{}", said(&broken));
    let unreadable = Folder::new("unreadable");
    std::fs::create_dir_all(unreadable.0.join("config.json")).expect("a folder in the file's place");
    let failed = run(&unreadable.0, &["status"], None);
    assert!(!failed.status.success() && said(&failed).contains("не выполнена"), "{}", said(&failed));
}

/// A `curl` of its own, first on the path, so the program talks to nobody. It reads what it is told, as the real one does.
#[cfg(unix)]
fn listing_with_curl_answering(data: &Folder, name: &str, answer: &str) -> Output {
    use std::os::unix::fs::PermissionsExt as _;
    let bin = data.0.join(name);
    std::fs::create_dir_all(&bin).expect("the folder");
    let curl = bin.join("curl");
    std::fs::write(&curl, format!("#!/bin/sh\ncat > /dev/null\nprintf '%s' '{answer}'\nexit 0\n")).expect("the file");
    std::fs::set_permissions(&curl, std::fs::Permissions::from_mode(0o755)).expect("the mode");
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    Command::new(env!("CARGO_BIN_EXE_logcourier-cli"))
        .args(["list", "--chat-id", "-1001"])
        .env("LOGCOURIER_DATA_DIR", &data.0)
        .env("LOGCOURIER_BOT_TOKEN", TOKEN)
        .env("PATH", path)
        .output()
        .expect("it runs")
}

#[cfg(unix)]
#[test]
fn a_token_in_the_environment_reaches_the_server_and_a_failure_hides_it() {
    let data = Folder::new("token");
    let refused = listing_with_curl_answering(&data, "refusing", "{\"ok\":false,\"description\":\"no\"}\n401");
    assert!(!said(&refused).contains("AAAAAAAA"), "{}", said(&refused));
    assert!(!refused.status.success(), "{}", said(&refused));
    let no_token = run(&data.0, &["list", "--chat-id", "-1001"], Some("no token"));
    assert!(!no_token.status.success(), "{}", said(&no_token));
    let listed = listing_with_curl_answering(&data, "answering", "{\"ok\":true,\"result\":{\"type\":\"supergroup\"}}\n200");
    assert!(listed.status.success(), "{}", said(&listed));
}

/// Without a desktop the window cannot open, and the program says so and ends.
#[cfg(target_os = "linux")]
#[test]
fn without_a_desktop_the_window_does_not_open() {
    let data = Folder::new("window");
    let plain = run(&data.0, &[], None);
    assert!(plain.status.code().is_some(), "{}", said(&plain));
    let test = run(&data.0, &["self-test", "--output", "/nonexistent/x"], None);
    assert!(test.status.code().is_some(), "{}", said(&test));
    let gui = run(&data.0, &["gui", "--minimized"], None);
    assert!(gui.status.code().is_some(), "{}", said(&gui));
}

/// The folder of the data when nothing overrides it.
#[test]
fn the_data_folder_of_this_system() {
    let home = Folder::new("home");
    let output = Command::new(env!("CARGO_BIN_EXE_logcourier-cli"))
        .arg("status")
        .env_remove("LOGCOURIER_DATA_DIR")
        .env_remove("XDG_DATA_HOME")
        .env("HOME", &home.0)
        .env("USERPROFILE", &home.0)
        .env("LOCALAPPDATA", &home.0)
        .output()
        .expect("it runs");
    assert!(output.status.success(), "{}", said(&output));
}

/// On Windows the window is real: the self-test opens it, looks at it and closes it.
#[cfg(windows)]
#[test]
fn the_window_tests_itself() {
    let data = Folder::new("self-test");
    let output = run(&data.0, &["self-test", "--output", &data.0.join("window.txt").display().to_string()], None);
    assert!(output.status.code().is_some(), "{}", said(&output));
}

/// A Mac has no settings window: the program says so and reads its file.
#[cfg(target_os = "macos")]
#[test]
fn a_mac_has_no_window() {
    let data = Folder::new("no-window");
    let output = run(&data.0, &[], None);
    assert_eq!(output.status.code(), Some(1), "{}", said(&output));
}

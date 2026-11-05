//! Raw keyboard access for a Wayland session, given only on an administrator's explicit command (the Python's
//! `wayland_access.py`).
//!
//! In a Wayland session a program sees no keys of other windows, so `KeySwitch` reads the keyboards and types through
//! /dev/uinput itself (`wayland_helper`). The udev rule that lets the session's user do that lets every program of that
//! user do it as well: read every key typed, passwords included, and type keys of its own, as any program can on X11 and
//! Windows. The rule ships inside the application and an administrator enables it once with
//! `sudo keyswitch --enable-wayland-input`, and takes it away with `--disable-wayland-input`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::wayland_constants::WAYLAND_ACCESS_FILE_MODE;

pub const ENABLE_COMMAND: &str = "sudo keyswitch --enable-wayland-input";
pub const DISABLE_COMMAND: &str = "sudo keyswitch --disable-wayland-input";

/// What a user who meets the missing access is told, in every place that reports it.
#[must_use]
pub fn access_hint() -> String {
    format!(
        "в сеансе Wayland KeySwitch читает клавиатуру на уровне системы; доступ включает администратор один раз командой \
         «{ENABLE_COMMAND}». После этого любая программа вашего сеанса сможет читать нажатия и печатать сама, как в X11 и \
         Windows; выключить — «{DISABLE_COMMAND}»"
    )
}

/// The application's resource, where enabling puts it (relative to the root of the file system), and its bytes. The
/// bytes are part of the executable, so the rule is found whatever the installation layout is.
pub const ACCESS_FILES: [(&str, &str, &str); 2] = [
    ("60-keyswitch.rules", "etc/udev/rules.d/60-keyswitch.rules", include_str!("../resources/wayland-input/60-keyswitch.rules")),
    (
        "keyswitch.modules-load.conf",
        "etc/modules-load.d/keyswitch.conf",
        include_str!("../resources/wayland-input/keyswitch.modules-load.conf"),
    ),
];
pub const RULE_DESTINATION: &str = ACCESS_FILES[0].1;
/// The devices the rule gives access to: their ACLs stay until they are taken away (udev re-adds only tagged ones).
pub const ACL_DEVICE_PATTERNS: [&str; 2] = ["dev/uinput", "dev/input/event*"];
pub const RELOAD_COMMANDS: [&[&str]; 3] = [
    &["udevadm", "control", "--reload-rules"],
    &["udevadm", "trigger", "--subsystem-match=misc", "--sysname-match=uinput"],
    &["udevadm", "trigger", "--subsystem-match=input"],
];
pub const LOAD_UINPUT: &[&str] = &["modprobe", "uinput"];
/// udev and logind act on the triggered events after the command returns: the ACLs are stripped once they are done.
pub const SETTLE: &[&str] = &["udevadm", "settle"];

/// Runs a system command and gives its status.
pub type Runner<'a> = &'a dyn Fn(&[&str]) -> i32;

/// Run a system command; a missing program counts as a failure, not a crash.
#[must_use]
pub fn run(command: &[&str]) -> i32 {
    let Some((program, arguments)) = command.split_first() else { return 1 };
    match Command::new(program).args(arguments).stdout(Stdio::null()).stderr(Stdio::null()).status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(_) => 1,
    }
}

/// The effective user id.
#[must_use]
pub fn effective_user() -> u32 {
    // SAFETY: `geteuid` takes no arguments, touches no memory and cannot fail.
    unsafe { libc::geteuid() }
}

/// Whether an administrator has enabled the access (the rule stands in /etc/udev/rules.d).
#[must_use]
pub fn access_enabled(root: &Path) -> bool {
    root.join(RULE_DESTINATION).exists()
}

/// Install the rule and the module list, load uinput and apply the rule to the devices already present.
pub fn enable_wayland_input(root: &Path, run: Runner<'_>, euid: &dyn Fn() -> u32, out: &mut dyn FnMut(&str)) -> i32 {
    if euid() != 0 {
        out(&format!("Нужны права администратора: {ENABLE_COMMAND}"));
        return 1;
    }
    for (_, destination, content) in ACCESS_FILES {
        let target = root.join(destination);
        let written = target
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(&target, content))
            .and_then(|()| fs::set_permissions(&target, fs::Permissions::from_mode(WAYLAND_ACCESS_FILE_MODE)));
        if let Err(error) = written {
            out(&format!("{}: {error}", target.display()));
            return 1;
        }
    }
    let failed: Vec<&[&str]> = std::iter::once(LOAD_UINPUT).chain(RELOAD_COMMANDS).filter(|command| run(command) != 0).collect();
    out(&format!(
        "Доступ KeySwitch к клавиатурам в сеансе Wayland включён. Теперь любая программа сеанса может читать нажатия и \
         печатать сама, как в X11 и Windows. Выключить: {DISABLE_COMMAND}"
    ));
    for command in failed {
        out(&format!("Команда не выполнилась: {}; доступ появится после перезагрузки", command.join(" ")));
    }
    0
}

/// The devices of `ACL_DEVICE_PATTERNS` that exist under `root`, sorted.
fn acl_devices(root: &Path) -> Vec<PathBuf> {
    let mut devices = Vec::new();
    for pattern in ACL_DEVICE_PATTERNS {
        if let Some(prefix) = pattern.strip_suffix('*') {
            let (directory, name) = prefix.rsplit_once('/').unwrap_or(("", prefix));
            if let Ok(entries) = fs::read_dir(root.join(directory)) {
                devices.extend(
                    entries.flatten().filter(|entry| entry.file_name().to_string_lossy().starts_with(name)).map(|entry| entry.path()),
                );
            }
        } else {
            let path = root.join(pattern);
            if path.exists() {
                devices.push(path);
            }
        }
    }
    devices.sort();
    devices
}

/// Remove the rule and the module list and take the access away from the devices at once.
///
/// udev applies a removed rule to no device, and the ACLs it granted stay until they are stripped. Stripped before the
/// devices are triggered, the ACL of /dev/uinput came back with the triggered event (Ubuntu 26.04), so the events are
/// triggered and settled first; a device systemd grants on its own (a joystick) has its grant given back at the next
/// event or login.
pub fn disable_wayland_input(root: &Path, run: Runner<'_>, euid: &dyn Fn() -> u32, out: &mut dyn FnMut(&str)) -> i32 {
    if euid() != 0 {
        out(&format!("Нужны права администратора: {DISABLE_COMMAND}"));
        return 1;
    }
    for (_, destination, _) in ACCESS_FILES {
        // A file that is not there is already what is asked for.
        let _ = fs::remove_file(root.join(destination));
    }
    let failed = RELOAD_COMMANDS.iter().copied().chain(std::iter::once(SETTLE)).filter(|command| run(command) != 0).count() > 0;
    let devices = acl_devices(root);
    let names: Vec<String> = devices.iter().map(|device| device.to_string_lossy().into_owned()).collect();
    let mut command: Vec<&str> = vec!["setfacl", "-b"];
    command.extend(names.iter().map(String::as_str));
    let stripped = !devices.is_empty() && run(&command) == 0;
    out("Доступ KeySwitch к клавиатурам в сеансе Wayland выключен.");
    if !stripped || failed {
        out("Права на устройства снимутся после перезагрузки.");
    }
    0
}

/// `keyswitch --enable-wayland-input` and `--disable-wayland-input` as the binary runs them: the real root, the real
/// commands, the report on the standard output.
#[must_use]
pub fn enable_command() -> i32 {
    enable_wayland_input(Path::new("/"), &run, &effective_user, &mut |line| println!("{line}"))
}

#[must_use]
pub fn disable_command() -> i32 {
    disable_wayland_input(Path::new("/"), &run, &effective_user, &mut |line| println!("{line}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::os::unix::fs::MetadataExt;

    struct Recorder {
        commands: RefCell<Vec<Vec<String>>>,
        failing: Vec<&'static str>,
    }

    impl Recorder {
        fn new(failing: &[&'static str]) -> Recorder {
            Recorder { commands: RefCell::new(Vec::new()), failing: failing.to_vec() }
        }
        fn call(&self, command: &[&str]) -> i32 {
            self.commands.borrow_mut().push(command.iter().map(|part| (*part).to_owned()).collect());
            i32::from(self.failing.contains(&command[0]))
        }
        fn seen(&self) -> Vec<Vec<String>> {
            self.commands.borrow().clone()
        }
    }

    fn strings(command: &[&str]) -> Vec<String> {
        command.iter().map(|part| (*part).to_owned()).collect()
    }

    fn temporary_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("keyswitch-access-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn the_rule_ships_inside_the_application_and_nothing_enables_it() {
        let rule = ACCESS_FILES[0].2;
        assert!(rule.contains("TAG+=\"uaccess\""));
        assert!(rule.contains("--enable-wayland-input"));
        let root = temporary_root("ships");
        assert!(!access_enabled(&root));
        assert!(access_hint().contains(ENABLE_COMMAND) && access_hint().contains(DISABLE_COMMAND));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn without_root_nothing_is_touched() {
        let root = temporary_root("user");
        let recorder = Recorder::new(&[]);
        let mut lines: Vec<String> = Vec::new();
        assert_eq!(enable_wayland_input(&root, &|c| recorder.call(c), &|| 1000, &mut |line| lines.push(line.to_owned())), 1);
        assert!(lines[0].contains(ENABLE_COMMAND));
        lines.clear();
        assert_eq!(disable_wayland_input(&root, &|c| recorder.call(c), &|| 1000, &mut |line| lines.push(line.to_owned())), 1);
        assert!(lines[0].contains(DISABLE_COMMAND));
        assert!(recorder.seen().is_empty());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn enabling_installs_the_files_loads_uinput_and_applies_the_rule() {
        let root = temporary_root("enable");
        let recorder = Recorder::new(&[]);
        let mut lines: Vec<String> = Vec::new();
        assert_eq!(enable_wayland_input(&root, &|c| recorder.call(c), &|| 0, &mut |line| lines.push(line.to_owned())), 0);
        for (_, destination, content) in ACCESS_FILES {
            let target = root.join(destination);
            assert_eq!(fs::read_to_string(&target).unwrap(), content);
            assert_eq!(fs::metadata(&target).unwrap().mode() & 0o7777, WAYLAND_ACCESS_FILE_MODE);
        }
        assert!(access_enabled(&root));
        let mut expected = vec![strings(LOAD_UINPUT)];
        expected.extend(RELOAD_COMMANDS.iter().map(|command| strings(command)));
        assert_eq!(recorder.seen(), expected);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains(DISABLE_COMMAND));
        // A command that fails is named, and the access comes with the next boot.
        let mut lines: Vec<String> = Vec::new();
        let failing = Recorder::new(&["modprobe"]);
        assert_eq!(enable_wayland_input(&root, &|c| failing.call(c), &|| 0, &mut |line| lines.push(line.to_owned())), 0);
        assert!(lines[1].contains(&LOAD_UINPUT.join(" ")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn disabling_removes_the_files_and_strips_the_device_acls_after_udev_ran_again() {
        let root = temporary_root("disable");
        let mut lines: Vec<String> = Vec::new();
        enable_wayland_input(&root, &|_| 0, &|| 0, &mut |line| lines.push(line.to_owned()));
        let devices = [root.join("dev/uinput"), root.join("dev/input/event0")];
        for device in &devices {
            fs::create_dir_all(device.parent().unwrap()).unwrap();
            fs::write(device, "").unwrap();
        }
        let recorder = Recorder::new(&[]);
        let mut lines: Vec<String> = Vec::new();
        assert_eq!(disable_wayland_input(&root, &|c| recorder.call(c), &|| 0, &mut |line| lines.push(line.to_owned())), 0);
        assert!(!access_enabled(&root));
        assert!(ACCESS_FILES.iter().all(|(_, destination, _)| !root.join(destination).exists()));
        let mut expected: Vec<Vec<String>> = RELOAD_COMMANDS.iter().map(|command| strings(command)).collect();
        expected.push(strings(SETTLE));
        let mut strip = strings(&["setfacl", "-b"]);
        let mut names: Vec<String> = devices.iter().map(|device| device.to_string_lossy().into_owned()).collect();
        names.sort();
        strip.extend(names);
        expected.push(strip);
        assert_eq!(recorder.seen(), expected);
        assert_eq!(lines.len(), 1);
        // Without setfacl, or with a failing udevadm, the access goes at the next boot.
        for failing in [&["setfacl"][..], &["udevadm"][..]] {
            let recorder = Recorder::new(failing);
            let mut lines: Vec<String> = Vec::new();
            assert_eq!(disable_wayland_input(&root, &|c| recorder.call(c), &|| 0, &mut |line| lines.push(line.to_owned())), 0);
            assert!(lines[0].contains("выключен"));
            assert!(lines.last().unwrap().contains("после перезагрузки"));
        }
        for device in &devices {
            fs::remove_file(device).unwrap();
        }
        let recorder = Recorder::new(&[]);
        let mut lines: Vec<String> = Vec::new();
        disable_wayland_input(&root, &|c| recorder.call(c), &|| 0, &mut |line| lines.push(line.to_owned()));
        assert_eq!(recorder.seen().len(), RELOAD_COMMANDS.len() + 1);
        assert!(lines.last().unwrap().contains("после перезагрузки"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_missing_program_is_a_failed_command_and_the_user_is_the_systems() {
        assert_ne!(run(&["keyswitch-no-such-program"]), 0);
        assert_eq!(run(&["true"]), 0);
        // SAFETY: `geteuid` takes no arguments and cannot fail.
        assert_eq!(effective_user(), unsafe { libc::geteuid() });
    }
}

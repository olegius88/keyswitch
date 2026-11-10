//! Installing an update on macOS. Everything is decided from files and from what the tools of the system answer, so the
//! module compiles and is tested on any host: each tool runs through [`Commands`].
//!
//! The archive of the new version is unpacked beside the application (the same volume, so the swap is two renames) and
//! checked three ways: against the designated requirement of the running program, which is what macOS keeps the
//! Accessibility permission by (read from the running process, never written by hand), by the version its property list
//! names, and by a start with `--version`. Then the engine stops, since it reads files of the bundle after its own start,
//! the old bundle is set aside and the new one takes its place. A helper started from the set-aside old bundle, in a
//! process group of its own so that launchd does not end it together with the program, waits for the old copy to end,
//! starts the new one and waits for it to hold the lock of a running copy. If it does not, the old bundle goes back and
//! starts again, and that version is not installed by an automatic check again.
//!
//! When the user cannot write the folder of the application or the bundle (another user installed it, an account without
//! administrator rights), the swap runs as root through the system's password dialog (`do shell script ... with
//! administrator privileges`). Root runs only tools of the system: it copies the checked bundle into a folder it makes
//! beside the application that only root can enter, checks that copy again and moves it into place from there, so what
//! root moves is never something the user could change after it was checked. The old bundle is then handed to the user
//! (with the right to delete its folder), and the helper removes it once the new version has started.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ks_config::logsetup;
use ks_ui::windows_ui_model::{SettingSpec, update_settings};
use ks_updates::{AutomaticInstallGate, SemanticVersion, UpdateError, UpdateRelease};

use crate::constants::{
    ARCHIVE_BUNDLE_NAME, BUNDLE_EXECUTABLE, BUNDLE_INFO_PLIST, CODESIGN_TOOL, DITTO_TOOL, FINISH_RIGHTS_ADMINISTRATOR, FINISH_RIGHTS_USER,
    FINISH_UPDATE_ARGUMENT, FINISH_WINDOW_HIDDEN, FINISH_WINDOW_SHOWN, ID_TOOL, INSTANCE_LOCK_FILE, KILL_TOOL, LSOF_TOOL, OPEN_TOOL, OSASCRIPT_TOOL,
    OWNER_WRITE_PERMISSION, PLIST_BUDDY_TOOL, TRANSLOCATION_MARKER, UPDATE_DOWNLOADS_FOLDER, UPDATE_FAILED_FOLDER, UPDATE_FAILED_VERSIONS_FILE,
    UPDATE_NEW_FOLDER, UPDATE_OLD_COPY_EXIT_TIMEOUT_SECONDS, UPDATE_PENDING_FILE, UPDATE_PENDING_STALE_SECONDS, UPDATE_POLL_INTERVAL_MS,
    UPDATE_PREVIOUS_FOLDER, UPDATE_RESULT_FILE, UPDATE_SETTLE_SECONDS, UPDATE_START_TIMEOUT_SECONDS, UPDATE_WORK_PREFIX,
};

const LOGGER: &str = "keyswitch.update";
const INSTALL_SETTING: &str = "updates.install_automatically";
const INSTALL_DESCRIPTION: &str = "Скачать проверенный архив, заменить им приложение и перезапустить KeySwitch. Если новая версия не запустится, вернётся прежняя. Когда для замены нужен пароль администратора, обновление ждёт кнопки «Установить сейчас».";
const TRANSLOCATED: &str = "macOS запустила KeySwitch из временной копии только для чтения: перенесите приложение в папку «Программы» и запустите его оттуда, тогда обновление установится";
const NOT_A_BUNDLE: &str = "KeySwitch запущен не из приложения KeySwitch.app, обновление ставится вручную";
const READ_ONLY_VOLUME: &str = "Приложение лежит на томе только для чтения (например, в открытом образе диска): перенесите KeySwitch в папку «Программы»";
const UNSIGNED: &str = "Эта сборка KeySwitch не подписана разработчиком, и новую версию не с чем сверить: обновление ставится вручную";
const SIGNATURE_MISMATCH: &str = "Подпись новой версии не совпала с подписью установленной, обновление не установлено";
const NO_BUNDLE_IN_ARCHIVE: &str = "В архиве обновления нет приложения KeySwitch.app";
const NOT_STARTED: &str = "Новая версия не запускается на этом Mac, обновление не установлено";
const CANCELLED: &str = "Пароль администратора не введён, обновление не установлено";
/// What `osascript` says when the user closes the password dialog: AppleScript's «User canceled» error.
const USER_CANCELED_CODE: &str = "(-128)";
const RESULT_INSTALLED: &str = "installed";
const RESULT_FAILED: &str = "failed";

/// What a program printed and whether it succeeded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// The system as an update uses it: running its tools, starting the helper, waiting, the clock.
pub trait Commands: Send + Sync {
    /// Run a program to its end.
    fn run(&self, program: &str, arguments: &[String]) -> io::Result<Output>;
    /// Start a program in a process group of its own, so that it outlives this one, without waiting for it.
    fn start_detached(&self, program: &Path, arguments: &[String]) -> io::Result<()>;
    fn pause(&self, duration: Duration);
    /// Seconds since 1970.
    fn now_seconds(&self) -> u64;
}

/// The real system.
pub struct SystemCommands;

impl Commands for SystemCommands {
    fn run(&self, program: &str, arguments: &[String]) -> io::Result<Output> {
        let output = Command::new(program).args(arguments).stdin(Stdio::null()).output()?;
        Ok(Output {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn start_detached(&self, program: &Path, arguments: &[String]) -> io::Result<()> {
        // launchd ends every process of a job's group when the job ends (launchd.plist(5)), and the helper has to outlive
        // the program it replaces.
        let mut child = Command::new(program).args(arguments).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0).spawn()?;
        // Reaped here if the program is still running when the helper ends; otherwise launchd adopts it.
        std::thread::spawn(move || child.wait());
        Ok(())
    }

    fn pause(&self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn now_seconds(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
    }
}

fn texts(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn failed(message: impl Into<String>) -> UpdateError {
    UpdateError::new(message)
}

fn io_failed(error: io::Error) -> UpdateError {
    failed(format!("Не удалось установить обновление: {error}"))
}

/// Remove a folder and all it holds; one that is not there is fine.
fn remove_tree(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// The application that runs, and the folder it lies in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    pub path: PathBuf,
    pub parent: PathBuf,
}

/// The bundle of the running executable (`<name>.app/Contents/MacOS/<executable>`), which the caller has resolved to its
/// real path. A program macOS translocated runs from a read-only copy that it throws away: it is refused with what to do.
pub fn running_bundle(executable: &Path) -> Result<Bundle, UpdateError> {
    if path_text(executable).contains(TRANSLOCATION_MARKER) {
        return Err(failed(TRANSLOCATED));
    }
    let named = |path: Option<&Path>, name: &str| path.and_then(Path::file_name).is_some_and(|found| found == name);
    let folder = executable.parent();
    let contents = folder.and_then(Path::parent);
    let bundle = contents.and_then(Path::parent).filter(|bundle| bundle.extension().is_some_and(|extension| extension == "app"));
    match bundle {
        Some(bundle) if named(folder, "MacOS") && named(contents, "Contents") => {
            Ok(Bundle { path: bundle.to_path_buf(), parent: bundle.parent().unwrap_or(Path::new("/")).to_path_buf() })
        }
        _ => Err(failed(NOT_A_BUNDLE)),
    }
}

/// The designated requirement of a running process, as macOS checks it: a new version that satisfies it keeps the
/// Accessibility permission (Apple's TN3127). An unsigned or ad-hoc signed build has none worth the name.
pub fn designated_requirement(commands: &dyn Commands, process: u32) -> Result<String, UpdateError> {
    let output = commands.run(CODESIGN_TOOL, &texts(&["-d", "-r-", &process.to_string()])).map_err(io_failed)?;
    let requirement = output.stdout.lines().find_map(|line| line.strip_prefix("designated => ")).map(str::trim);
    match requirement {
        Some(requirement) if output.success && !requirement.is_empty() && !requirement.starts_with("cdhash") => Ok(requirement.to_owned()),
        _ => Err(failed(UNSIGNED)),
    }
}

/// Where the swap runs: in the user's own rights, or as root after the administrator's password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    InPlace,
    NeedsPassword,
}

/// Every entry of the tree belongs to `owner`, and the owner may write every folder in it: the old bundle can be moved
/// and removed without other rights.
fn owned_tree(path: &Path, owner: u32) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else { return false };
    if metadata.uid() != owner {
        return false;
    }
    if !metadata.is_dir() {
        return true;
    }
    if metadata.mode() & OWNER_WRITE_PERMISSION == 0 {
        return false;
    }
    let Ok(entries) = fs::read_dir(path) else { return false };
    entries.into_iter().all(|entry| entry.is_ok_and(|entry| owned_tree(&entry.path(), owner)))
}

fn work_folder(base: &Path, process: u32) -> PathBuf {
    base.join(format!("{UPDATE_WORK_PREFIX}{process}"))
}

/// A new work folder in `base`, after what an earlier attempt of a process with the same number left there.
fn create_work(base: &Path, process: u32) -> io::Result<PathBuf> {
    let work = work_folder(base, process);
    remove_tree(&work)?;
    fs::create_dir(&work)?;
    Ok(work)
}

/// A work folder beside the application could not be made: a read-only volume is refused, missing rights call for the
/// administrator's password.
fn password_or_refusal(error: &io::Error) -> Result<(), UpdateError> {
    let kind = error.kind();
    if kind == io::ErrorKind::ReadOnlyFilesystem {
        Err(failed(READ_ONLY_VOLUME))
    } else if kind == io::ErrorKind::PermissionDenied {
        Ok(())
    } else {
        Err(failed(format!("Не удалось подготовить обновление рядом с приложением: {error}")))
    }
}

/// Whether the swap can run in the user's own rights: a folder can be made beside the bundle, and the bundle is all the
/// user's. The trial folder is removed at once.
pub fn placement(bundle: &Bundle, process: u32) -> Result<Placement, UpdateError> {
    match create_work(&bundle.parent, process) {
        Ok(work) => {
            let owner = fs::metadata(&work).map(|metadata| metadata.uid());
            let _ = fs::remove_dir(&work);
            Ok(if owner.is_ok_and(|owner| owned_tree(&bundle.path, owner)) { Placement::InPlace } else { Placement::NeedsPassword })
        }
        Err(error) => password_or_refusal(&error).map(|()| Placement::NeedsPassword),
    }
}

/// The version an archive of a release is named for: `KeySwitch-<version>-macos-<processor>.zip`.
pub fn archive_version(archive: &Path) -> Result<SemanticVersion, UpdateError> {
    let name = archive.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let version = name.strip_prefix("KeySwitch-").and_then(|rest| rest.split_once("-macos-")).map(|(version, _)| version);
    version.ok_or_else(|| failed(format!("Архив обновления назван не как выпуск KeySwitch: {name}"))).and_then(SemanticVersion::parse)
}

/// A bundle carries the signature the running program requires and the version it is installed as.
pub fn check_bundle(commands: &dyn Commands, bundle: &Path, requirement: &str, version: &SemanticVersion) -> Result<(), UpdateError> {
    let signature = commands
        .run(CODESIGN_TOOL, &texts(&["--verify", "--strict", "--deep", "-R", &format!("={requirement}"), &path_text(bundle)]))
        .map_err(io_failed)?;
    if !signature.success {
        return Err(failed(format!("{SIGNATURE_MISMATCH}: {}", signature.stderr.trim())));
    }
    let plist = path_text(&bundle.join(BUNDLE_INFO_PLIST));
    let named = commands.run(PLIST_BUDDY_TOOL, &texts(&["-c", "Print :CFBundleShortVersionString", &plist])).map_err(io_failed)?;
    if !named.success || named.stdout.trim() != version.to_string() {
        return Err(failed(format!("Новая версия называет себя «{}», а не {version}: обновление не установлено", named.stdout.trim())));
    }
    Ok(())
}

/// The new version, unpacked and checked, and where the swap will run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub bundle: Bundle,
    pub placement: Placement,
    /// The work folder: beside the application when the swap runs in the user's rights, in the folder of the downloads
    /// when root makes its own.
    pub work: PathBuf,
    pub new_bundle: PathBuf,
    pub requirement: String,
    pub version: SemanticVersion,
}

fn unpack_and_check(commands: &dyn Commands, archive: &Path, prepared: &Prepared) -> Result<(), UpdateError> {
    let target = prepared.work.join(UPDATE_NEW_FOLDER);
    let unpacked = commands.run(DITTO_TOOL, &texts(&["-x", "-k", &path_text(archive), &path_text(&target)])).map_err(io_failed)?;
    if !unpacked.success {
        return Err(failed(format!("Архив обновления не распакован: {}", unpacked.stderr.trim())));
    }
    if !fs::symlink_metadata(&prepared.new_bundle).is_ok_and(|metadata| metadata.is_dir()) {
        return Err(failed(NO_BUNDLE_IN_ARCHIVE));
    }
    check_bundle(commands, &prepared.new_bundle, &prepared.requirement, &prepared.version)?;
    // An archive for another processor, or a program that cannot load, stops here while the old version still runs.
    let started = commands.run(&path_text(&prepared.new_bundle.join(BUNDLE_EXECUTABLE)), &texts(&["--version"])).map_err(io_failed)?;
    if !started.success || started.stdout.trim() != format!("KeySwitch {}", prepared.version) {
        return Err(failed(NOT_STARTED));
    }
    Ok(())
}

/// Unpack the downloaded archive and check the new version, before anything of the running program is touched.
pub fn prepare(commands: &dyn Commands, archive: &Path, bundle: &Bundle, downloads: &Path, process: u32) -> Result<Prepared, UpdateError> {
    let version = archive_version(archive)?;
    let requirement = designated_requirement(commands, process)?;
    let (work, placement) = match create_work(&bundle.parent, process) {
        Ok(work) => {
            let owner = fs::metadata(&work).map_err(io_failed)?.uid();
            if owned_tree(&bundle.path, owner) {
                (work, Placement::InPlace)
            } else {
                let _ = fs::remove_dir(&work);
                (create_work(downloads, process).map_err(io_failed)?, Placement::NeedsPassword)
            }
        }
        Err(error) => {
            password_or_refusal(&error)?;
            (create_work(downloads, process).map_err(io_failed)?, Placement::NeedsPassword)
        }
    };
    let new_bundle = work.join(UPDATE_NEW_FOLDER).join(ARCHIVE_BUNDLE_NAME);
    let prepared = Prepared { bundle: bundle.clone(), placement, work, new_bundle, requirement, version };
    if let Err(error) = unpack_and_check(commands, archive, &prepared) {
        let _ = remove_tree(&prepared.work);
        return Err(error);
    }
    Ok(prepared)
}

/// The swap in the user's own rights: the old bundle into the work folder, the new one into its place; if the second
/// step fails, the first is undone. Returns where the old bundle now is.
pub fn swap_in_place(prepared: &Prepared) -> Result<PathBuf, UpdateError> {
    let previous = prepared.work.join(UPDATE_PREVIOUS_FOLDER);
    fs::rename(&prepared.bundle.path, &previous).map_err(io_failed)?;
    if let Err(error) = fs::rename(&prepared.new_bundle, &prepared.bundle.path) {
        let _ = fs::rename(&previous, &prepared.bundle.path);
        return Err(io_failed(error));
    }
    Ok(previous)
}

/// Put the old bundle back in place of the new one, which goes to `failed`; if that fails, the new one stays.
fn put_back(bundle: &Path, previous: &Path, failed: &Path) -> io::Result<()> {
    fs::rename(bundle, failed)?;
    if let Err(error) = fs::rename(previous, bundle) {
        let _ = fs::rename(failed, bundle);
        return Err(error);
    }
    Ok(())
}

/// The swap root runs, a shell script of the system's tools with its values as arguments:
/// `bundle source requirement version user keep`. It copies `source` into a folder only root can enter, beside the
/// bundle, gives the copy the owner of the bundle, checks the copy's signature and version, and swaps it in for the
/// bundle by two renames. With `keep` the old bundle stays in that folder, which goes to `user` with the right to delete
/// it, and the folder's path is printed; otherwise the folder goes with the old bundle in it.
#[must_use]
pub fn root_swap_script() -> String {
    format!(
        r#"set -eu
umask 022
bundle=$1 source=$2 requirement=$3 version=$4 user=$5 keep=$6
work=$(/usr/bin/mktemp -d "$(/usr/bin/dirname "$bundle")/{UPDATE_WORK_PREFIX}XXXXXX")
trap '/bin/rm -rf "$work"' EXIT
/bin/mkdir "$work/{UPDATE_NEW_FOLDER}"
{DITTO_TOOL} "$source" "$work/{UPDATE_NEW_FOLDER}/{ARCHIVE_BUNDLE_NAME}"
/usr/sbin/chown -R "$(/usr/bin/stat -f %u:%g "$bundle")" "$work/{UPDATE_NEW_FOLDER}"
{CODESIGN_TOOL} --verify --strict --deep -R "=$requirement" "$work/{UPDATE_NEW_FOLDER}/{ARCHIVE_BUNDLE_NAME}"
test "$({PLIST_BUDDY_TOOL} -c 'Print :CFBundleShortVersionString' "$work/{UPDATE_NEW_FOLDER}/{ARCHIVE_BUNDLE_NAME}/{BUNDLE_INFO_PLIST}")" = "$version"
/bin/mv "$bundle" "$work/{UPDATE_PREVIOUS_FOLDER}"
if ! /bin/mv "$work/{UPDATE_NEW_FOLDER}/{ARCHIVE_BUNDLE_NAME}" "$bundle"; then /bin/mv "$work/{UPDATE_PREVIOUS_FOLDER}" "$bundle"; exit 1; fi
if [ "$keep" = keep ]; then
  /bin/rmdir "$work/{UPDATE_NEW_FOLDER}"
  /usr/sbin/chown -R "$user" "$work"
  /bin/chmod +a "$user allow delete" "$work"
  trap - EXIT
  echo "$work"
fi
"#
    )
}

/// One argument of `sh`, quoted.
fn shell_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The swap as root: the system asks for the administrator's password with `prompt`, then runs [`root_swap_script`].
/// With `keep`, returns the folder root left the old bundle in.
pub fn privileged_swap(
    commands: &dyn Commands,
    bundle: &Path,
    source: &Path,
    requirement: &str,
    version: &str,
    keep: bool,
    prompt: &str,
) -> Result<Option<PathBuf>, UpdateError> {
    let user = commands.run(ID_TOOL, &texts(&["-un"])).map_err(io_failed)?.stdout.trim().to_owned();
    if user.is_empty() {
        return Err(failed("Не удалось узнать имя пользователя"));
    }
    let script = root_swap_script();
    let keep_word = if keep { "keep" } else { "discard" };
    let words = ["/bin/sh", "-c", script.as_str(), "keyswitch-update", &path_text(bundle), &path_text(source), requirement, version, &user, keep_word];
    let line: Vec<String> = words.iter().map(|word| shell_quoted(word)).collect();
    let arguments = texts(&[
        "-e",
        "on run argv",
        "-e",
        "do shell script (item 1 of argv) with prompt (item 2 of argv) with administrator privileges",
        "-e",
        "end run",
        &line.join(" "),
        prompt,
    ]);
    let output = commands.run(OSASCRIPT_TOOL, &arguments).map_err(io_failed)?;
    if !output.success {
        if output.stderr.contains(USER_CANCELED_CODE) {
            return Err(failed(CANCELLED));
        }
        return Err(failed(format!("Замена приложения с правами администратора не удалась: {}", output.stderr.trim())));
    }
    if !keep {
        return Ok(None);
    }
    let work = PathBuf::from(output.stdout.trim());
    let beside = work.parent() == bundle.parent();
    let named = work.file_name().is_some_and(|name| name.to_string_lossy().starts_with(UPDATE_WORK_PREFIX));
    if !beside || !named {
        return Err(failed(format!("Замена с правами администратора вернула неожиданный путь: {}", work.display())));
    }
    Ok(Some(work))
}

/// The records of updates, in the folder of the downloads, and the lock file of a running copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateFiles {
    pub downloads: PathBuf,
    pub lock: PathBuf,
}

impl UpdateFiles {
    #[must_use]
    pub fn new(data: &Path) -> Self {
        Self { downloads: data.join(UPDATE_DOWNLOADS_FOLDER), lock: data.join(INSTANCE_LOCK_FILE) }
    }

    fn record(&self, name: &str) -> PathBuf {
        self.downloads.join(name)
    }

    /// The versions that did not start: an automatic check never installs them again.
    #[must_use]
    pub fn failed_versions(&self) -> Vec<String> {
        let text = fs::read_to_string(self.record(UPDATE_FAILED_VERSIONS_FILE)).unwrap_or_default();
        text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect()
    }

    fn add_failed_version(&self, version: &str) {
        let mut versions = self.failed_versions();
        if !versions.iter().any(|known| known == version) {
            versions.push(version.to_owned());
        }
        let _ = fs::create_dir_all(&self.downloads);
        let _ = fs::write(self.record(UPDATE_FAILED_VERSIONS_FILE), format!("{}\n", versions.join("\n")));
    }

    fn write_result(&self, kind: &str, new_version: &str, old_version: &str) {
        let _ = fs::create_dir_all(&self.downloads);
        let _ = fs::write(self.record(UPDATE_RESULT_FILE), format!("{kind} {new_version} {old_version}\n"));
    }

    /// The outcome of the last update, read once.
    fn take_result(&self) -> Option<String> {
        let path = self.record(UPDATE_RESULT_FILE);
        let text = fs::read_to_string(&path).ok()?;
        let _ = fs::remove_file(&path);
        Some(text)
    }

    fn mark_pending(&self, now: u64) {
        let _ = fs::create_dir_all(&self.downloads);
        let _ = fs::write(self.record(UPDATE_PENDING_FILE), format!("{now}\n"));
    }

    fn clear_pending(&self) {
        let _ = fs::remove_file(self.record(UPDATE_PENDING_FILE));
    }

    fn pending_since(&self) -> Option<u64> {
        fs::read_to_string(self.record(UPDATE_PENDING_FILE)).ok()?.trim().parse().ok()
    }
}

/// What to tell the user of the last update, from its record.
#[must_use]
pub fn result_notice(record: &str) -> Option<(String, String)> {
    let words: Vec<&str> = record.split_whitespace().collect();
    match words.as_slice() {
        [RESULT_INSTALLED, new, _] => Some(("KeySwitch обновлён".to_owned(), format!("Установлена версия {new}."))),
        [RESULT_FAILED, new, old] => Some((
            "Обновление KeySwitch не установлено".to_owned(),
            format!("Версия {new} не запустилась, и вернулась версия {old}. Сама она больше не установится, «Установить сейчас» в настройках поставит её снова."),
        )),
        _ => None,
    }
}

/// Remove the work folders an update left in `base`; one that cannot be removed stays.
fn remove_work_folders(base: &Path) {
    let Ok(entries) = fs::read_dir(base) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(UPDATE_WORK_PREFIX) {
            let _ = remove_tree(&entry.path());
        }
    }
}

/// At the start of the program: unless an update is under way, remove what updates left (their work folders beside the
/// application and in the folder of the downloads, the downloaded archives); and say how the last update ended, once.
#[must_use]
pub fn startup(files: &UpdateFiles, bundle: Option<&Bundle>, now: u64) -> Option<(String, String)> {
    let under_way = files.pending_since().is_some_and(|since| now.saturating_sub(since) < UPDATE_PENDING_STALE_SECONDS);
    if !under_way {
        files.clear_pending();
        if let Some(bundle) = bundle {
            remove_work_folders(&bundle.parent);
        }
        remove_work_folders(&files.downloads);
        let _ = ks_updates::remove_previous_installers(&files.downloads, "");
    }
    files.take_result().as_deref().and_then(result_notice)
}

/// What an automatic check asks before it installs a release by itself: a version that did not start before, a program
/// that cannot be updated where it lies, and a swap that needs the administrator's password wait for the user.
#[must_use]
pub fn automatic_install_gate(files: UpdateFiles, bundle: Result<Bundle, UpdateError>, process: u32) -> AutomaticInstallGate {
    Arc::new(move |release: &UpdateRelease| {
        let version = release.version.to_string();
        if files.failed_versions().contains(&version) {
            return Some(format!("Доступна версия {version}. Она не запустилась при прошлой установке, поэтому сама не ставится: «Установить сейчас» поставит её снова."));
        }
        let placement = bundle.as_ref().map_err(Clone::clone).and_then(|bundle| placement(bundle, process));
        match placement {
            Ok(Placement::InPlace) => None,
            Ok(Placement::NeedsPassword) => Some(format!("Доступна версия {version}. Для её установки нужен пароль администратора: нажмите «Установить сейчас».")),
            Err(error) => Some(format!("Доступна версия {version}. {error}.")),
        }
    })
}

/// The settings of the Updates page that work on macOS, described as they work here.
#[must_use]
pub fn mac_update_settings() -> Vec<SettingSpec> {
    update_settings()
        .into_iter()
        .map(|mut spec| {
            if spec.path == INSTALL_SETTING {
                INSTALL_DESCRIPTION.clone_into(&mut spec.description);
            }
            spec
        })
        .collect()
}

/// What the helper is told: the bundle, where the old one is, the two versions, whether the settings window was open,
/// and whether the swap ran as root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finish {
    pub bundle: PathBuf,
    pub previous: PathBuf,
    pub old_version: String,
    pub new_version: String,
    pub show_window: bool,
    pub privileged: bool,
}

impl Finish {
    /// The command line of the helper.
    #[must_use]
    pub fn arguments(&self) -> Vec<String> {
        vec![
            FINISH_UPDATE_ARGUMENT.to_owned(),
            path_text(&self.bundle),
            path_text(&self.previous),
            self.old_version.clone(),
            self.new_version.clone(),
            (if self.show_window { FINISH_WINDOW_SHOWN } else { FINISH_WINDOW_HIDDEN }).to_owned(),
            (if self.privileged { FINISH_RIGHTS_ADMINISTRATOR } else { FINISH_RIGHTS_USER }).to_owned(),
        ]
    }

    /// The command line after [`FINISH_UPDATE_ARGUMENT`].
    #[must_use]
    pub fn parse(arguments: &[String]) -> Option<Self> {
        let [bundle, previous, old_version, new_version, window, rights] = arguments else { return None };
        let show_window = [FINISH_WINDOW_SHOWN, FINISH_WINDOW_HIDDEN].iter().position(|word| word == window)? == 0;
        let privileged = [FINISH_RIGHTS_ADMINISTRATOR, FINISH_RIGHTS_USER].iter().position(|word| word == rights)? == 0;
        Some(Self {
            bundle: PathBuf::from(bundle),
            previous: PathBuf::from(previous),
            old_version: old_version.clone(),
            new_version: new_version.clone(),
            show_window,
            privileged,
        })
    }

    fn work(&self) -> PathBuf {
        self.previous.parent().map_or_else(|| self.previous.clone(), Path::to_path_buf)
    }
}

/// The processes that have a file open, as `lsof` lists them (the lock of a running copy, or an executable a process
/// runs); none when it cannot tell.
fn lock_holders(commands: &dyn Commands, lock: &Path) -> Vec<u32> {
    commands
        .run(LSOF_TOOL, &texts(&["-t", "--", &path_text(lock)]))
        .map(|output| output.stdout.lines().filter_map(|line| line.trim().parse().ok()).collect())
        .unwrap_or_default()
}

/// Look until `condition` holds, for at most `timeout`.
fn wait_for(commands: &dyn Commands, timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let poll = Duration::from_millis(UPDATE_POLL_INTERVAL_MS);
    let mut waited = Duration::ZERO;
    loop {
        if condition() {
            return true;
        }
        if waited >= timeout {
            return false;
        }
        commands.pause(poll);
        waited += poll;
    }
}

/// Start the program from `bundle`: in the background with its window hidden, or with the window when it was open.
fn open_program(commands: &dyn Commands, bundle: &Path, show_window: bool) {
    let bundle = path_text(bundle);
    let arguments = if show_window { texts(&[&bundle]) } else { texts(&["-g", &bundle, "--args", "--hidden"]) };
    match commands.run(OPEN_TOOL, &arguments) {
        Ok(output) if output.success => {}
        Ok(output) => logsetup::error(LOGGER, &format!("KeySwitch не запустился: {}", output.stderr.trim())),
        Err(error) => logsetup::error(LOGGER, &format!("KeySwitch не запустился: {error}")),
    }
}

/// The new copy took the lock within its time and kept it; the process that took it, if one did.
fn watch_new_copy(commands: &dyn Commands, lock: &Path) -> (Option<u32>, bool) {
    let mut holder = None;
    let took = wait_for(commands, Duration::from_secs(UPDATE_START_TIMEOUT_SECONDS), || {
        holder = lock_holders(commands, lock).first().copied();
        holder.is_some()
    });
    let Some(process) = holder.filter(|_| took) else { return (None, false) };
    let lost = wait_for(commands, Duration::from_secs(UPDATE_SETTLE_SECONDS), || !lock_holders(commands, lock).contains(&process));
    (Some(process), !lost)
}

/// Put the old version back: by two renames, or as root after the password again. The work folder goes either way.
fn restore(commands: &dyn Commands, finish: &Finish, process: u32) -> bool {
    let work = finish.work();
    let restored = if finish.privileged {
        let prompt = format!(
            "Версия {} не запустилась. Чтобы вернуть версию {}, нужен пароль администратора.",
            finish.new_version, finish.old_version
        );
        designated_requirement(commands, process)
            .and_then(|requirement: String| privileged_swap(commands, &finish.bundle, &finish.previous, &requirement, &finish.old_version, false, &prompt))
            .map_err(|error| logsetup::error(LOGGER, &error.to_string()))
            .is_ok()
    } else {
        put_back(&finish.bundle, &finish.previous, &work.join(UPDATE_FAILED_FOLDER)).map_err(|error| logsetup::error(LOGGER, &error.to_string())).is_ok()
    };
    if restored {
        let _ = remove_tree(&work);
    }
    restored
}

/// The helper's work, from the set-aside old bundle once the program has quit: start the new version and see that it
/// runs, or put the old one back. True when the new version runs.
pub fn finish_update(commands: &dyn Commands, files: &UpdateFiles, finish: &Finish, process: u32) -> bool {
    let ended = wait_for(commands, Duration::from_secs(UPDATE_OLD_COPY_EXIT_TIMEOUT_SECONDS), || lock_holders(commands, &files.lock).is_empty());
    if !ended {
        // The old copy still runs, from its memory: its bundle goes back under it, and the new version is not to blame.
        logsetup::error(LOGGER, "Прежняя копия KeySwitch не закрылась, обновление отменено");
        restore(commands, finish, process);
        files.clear_pending();
        return false;
    }
    open_program(commands, &finish.bundle, finish.show_window);
    let (holder, started) = watch_new_copy(commands, &files.lock);
    if started {
        let _ = remove_tree(&finish.work());
        files.write_result(RESULT_INSTALLED, &finish.new_version, &finish.old_version);
        files.clear_pending();
        logsetup::info(LOGGER, &format!("KeySwitch обновлён с версии {} до {}", finish.old_version, finish.new_version));
        return true;
    }
    logsetup::error(LOGGER, &format!("Версия {} не запустилась, возвращаем {}", finish.new_version, finish.old_version));
    // Whatever still runs of the new version goes: a copy that hangs without the lock would otherwise be the one macOS
    // brings forward when the old version is opened, since only one copy of the application may run.
    let mut running = lock_holders(commands, &finish.bundle.join(BUNDLE_EXECUTABLE));
    running.extend(holder.filter(|holder| !running.contains(holder)));
    for stray in running {
        let _ = commands.run(KILL_TOOL, &texts(&[&stray.to_string()]));
    }
    files.add_failed_version(&finish.new_version);
    if restore(commands, finish, process) {
        open_program(commands, &finish.bundle, finish.show_window);
    }
    files.write_result(RESULT_FAILED, &finish.new_version, &finish.old_version);
    files.clear_pending();
    false
}

/// The helper's command line, after [`FINISH_UPDATE_ARGUMENT`]: its exit status.
pub fn finish_main(commands: &dyn Commands, files: &UpdateFiles, arguments: &[String], process: u32) -> i32 {
    let Some(finish) = Finish::parse(arguments) else {
        eprintln!("usage: KeySwitch {FINISH_UPDATE_ARGUMENT} <bundle> <previous> <old version> <new version> shown|hidden administrator|user");
        return 2;
    };
    i32::from(!finish_update(commands, files, &finish, process))
}

/// What the program does around the swap, which only it can do: its engine, its window, its end.
pub trait InstallHost: Send + Sync {
    fn stop_engine(&self);
    fn start_engine(&self);
    fn window_shown(&self) -> bool;
    fn quit(&self);
}

/// Installs a downloaded archive: the update manager's install request on macOS.
pub struct Installer {
    pub commands: Arc<dyn Commands>,
    pub host: Arc<dyn InstallHost>,
    pub files: UpdateFiles,
    /// The running executable, resolved to its real path.
    pub executable: PathBuf,
    pub process: u32,
    pub current_version: String,
}

impl Installer {
    /// Check the new version, swap it in, start the helper and quit; on a failure before the program would quit, the old
    /// version carries on and the error says why.
    pub fn install(&self, archive: &Path) -> Result<(), UpdateError> {
        let commands = self.commands.as_ref();
        let bundle = running_bundle(&self.executable)?;
        let prepared = prepare(commands, archive, &bundle, &self.files.downloads, self.process)?;
        let show_window = self.host.window_shown();
        self.host.stop_engine();
        let privileged = prepared.placement == Placement::NeedsPassword;
        let swapped = if privileged {
            let prompt = format!(
                "KeySwitch устанавливает версию {}. Чтобы заменить приложение в папке «{}», нужен пароль администратора.",
                prepared.version,
                bundle.parent.display()
            );
            let swapped = privileged_swap(commands, &bundle.path, &prepared.new_bundle, &prepared.requirement, &prepared.version.to_string(), true, &prompt);
            // Root has made its own copy: the one in the downloads is no longer needed.
            let _ = remove_tree(&prepared.work);
            swapped.map(|work| work.unwrap_or_default().join(UPDATE_PREVIOUS_FOLDER))
        } else {
            swap_in_place(&prepared)
        };
        let previous = match swapped {
            Ok(previous) => previous,
            Err(error) => {
                let _ = remove_tree(&prepared.work);
                self.host.start_engine();
                return Err(error);
            }
        };
        self.files.mark_pending(commands.now_seconds());
        let finish = Finish {
            bundle: bundle.path.clone(),
            previous,
            old_version: self.current_version.clone(),
            new_version: prepared.version.to_string(),
            show_window,
            privileged,
        };
        if let Err(error) = commands.start_detached(&finish.previous.join(BUNDLE_EXECUTABLE), &finish.arguments()) {
            self.files.clear_pending();
            // Without the helper nothing would start the new version: in the user's rights the old one goes back and
            // carries on; as root it cannot without the password again, and the new version starts with the next start.
            let message = if privileged || restore(commands, &finish, self.process) {
                format!("Не удалось запустить помощника обновления: {error}")
            } else {
                format!("Не удалось запустить помощника обновления, и прежняя версия не вернулась на место: {error}")
            };
            self.host.start_engine();
            return Err(failed(message));
        }
        self.host.quit();
        Ok(())
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
#[path = "macos_update_tests.rs"]
mod tests;

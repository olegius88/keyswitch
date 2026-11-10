//! Installing an update on macOS. Everything is decided from files and from what the tools of the system answer, so the
//! module compiles and is tested on any host: each tool runs through [`Commands`].
//!
//! The archive of the new version is unpacked beside the application (the same volume, so the swap is two renames) and
//! checked three ways: against the designated requirement of the running program, which is what macOS keeps the
//! Accessibility permission by (read from the running process, never written by hand), by the version its property list
//! names, and by a start with `--version`. Then the engine stops, since it reads files of the bundle after its own start,
//! the old bundle is set aside and the new one takes its place. A helper started from the set-aside old bundle, in a
//! process group of its own so that launchd does not end it together with the program, waits for the old copy to end,
//! starts the new one and waits for it to say it runs (it marks its start in the folder of the downloads; a version from
//! before that mark is known by the lock of a running copy, held by a process of the new executable). If it does not, the
//! old bundle goes back and starts again, and that version is not installed by an automatic check again.
//!
//! When the user cannot write the folder of the application or the bundle (another user installed it, an account without
//! administrator rights), the swap runs as root through the system's password dialog (`do shell script ... with
//! administrator privileges`, from an empty environment). Root runs only tools of the system, by their full paths: it
//! copies the checked bundle into a folder it makes beside the application that only root can enter and works there by
//! relative paths, so renaming the folder from outside changes nothing; it takes from the copy every right the user
//! could have given it (access lists, write bits of group and others, set-id bits), checks the copy's signature and
//! version and moves that copy into place. The old bundle, checked against the same signature, is handed to the user
//! (its folders and the work folder, with the right to delete it), and the helper removes it once the new version runs.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ks_config::logsetup;
use ks_ui::windows_ui_model::{SettingSpec, update_settings};
use ks_updates::{AutomaticInstallGate, SemanticVersion, UpdateError, UpdateRelease};

use crate::constants::{
    ARCHIVE_BUNDLE_NAME, BUNDLE_EXECUTABLE, BUNDLE_EXTENSION, BUNDLE_INFO_PLIST, CHMOD_TOOL, CHOWN_TOOL, CLEAN_PATH, CODESIGN_TOOL, DIRNAME_TOOL,
    DITTO_TOOL, FIND_TOOL, FINISH_RIGHTS_ADMINISTRATOR, FINISH_RIGHTS_USER, FINISH_UPDATE_ARGUMENT, FINISH_WINDOW_HIDDEN, FINISH_WINDOW_SHOWN, ID_TOOL,
    INSTANCE_LOCK_FILE, KILL_TOOL, LIMITED_RUN_POLL_MS, LSOF_TIME_LIMIT_MS, LSOF_TOOL, MKDIR_TOOL, MKTEMP_TOOL, MV_TOOL, OPEN_ATTEMPTS, OPEN_RETRY_MS, OPEN_TOOL,
    OSASCRIPT_TOOL, OWNER_WRITE_PERMISSION, PLIST_BUDDY_TOOL, RMDIR_TOOL, RM_TOOL, STAT_TOOL, TRANSLOCATION_MARKER, UPDATE_DOWNLOADS_FOLDER,
    UPDATE_FAILED_FOLDER, UPDATE_FAILED_VERSIONS_FILE, UPDATE_NEW_FOLDER, UPDATE_OLD_COPY_EXIT_TIMEOUT_SECONDS, UPDATE_PENDING_FILE,
    UPDATE_PENDING_STALE_SECONDS, UPDATE_POLL_INTERVAL_MS, UPDATE_PREVIOUS_FOLDER, UPDATE_RESULT_FILE, UPDATE_SETTLE_SECONDS, UPDATE_STARTED_FILE,
    UPDATE_START_TIMEOUT_SECONDS, UPDATE_STOP_WAIT_SECONDS, UPDATE_STRANDED_FILE, UPDATE_WORK_PREFIX,
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
/// The outcomes of an update the next start tells: the old version came back, or it could not.
const RESULT_FAILED: &str = "failed";
const RESULT_STRANDED: &str = "stranded";

/// What a program printed and whether it succeeded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Output {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// The system as an update uses it: running its tools, starting the helper, waiting, the clocks.
pub trait Commands: Send + Sync {
    /// Run a program to its end.
    fn run(&self, program: &str, arguments: &[String]) -> io::Result<Output>;
    /// Run a program with an empty environment but for a fixed `PATH`: what runs as root takes nothing from ours.
    fn run_clean(&self, program: &str, arguments: &[String]) -> io::Result<Output>;
    /// Run a program, and stop it if it has not ended within `limit`.
    fn run_limited(&self, program: &str, arguments: &[String], limit: Duration) -> io::Result<Output>;
    /// Start a program in a process group of its own, so that it outlives this one, without waiting for it.
    fn start_detached(&self, program: &Path, arguments: &[String]) -> io::Result<()>;
    fn pause(&self, duration: Duration);
    /// Seconds since 1970.
    fn now_seconds(&self) -> u64;
    /// A clock that only goes forward, for every wait.
    fn monotonic(&self) -> Duration;
}

/// The real system.
pub struct SystemCommands;

fn output_of(output: &std::process::Output) -> Output {
    Output {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

impl Commands for SystemCommands {
    fn run(&self, program: &str, arguments: &[String]) -> io::Result<Output> {
        Command::new(program).args(arguments).stdin(Stdio::null()).output().map(|output| output_of(&output))
    }

    fn run_clean(&self, program: &str, arguments: &[String]) -> io::Result<Output> {
        // `do shell script` runs its command in the environment of the process that asks (Apple's TN2065).
        Command::new(program).args(arguments).env_clear().env("PATH", CLEAN_PATH).stdin(Stdio::null()).output().map(|output| output_of(&output))
    }

    fn run_limited(&self, program: &str, arguments: &[String], limit: Duration) -> io::Result<Output> {
        let mut child = Command::new(program).args(arguments).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        let deadline = Instant::now() + limit;
        while child.try_wait()?.is_none() {
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(io::ErrorKind::TimedOut, format!("{program} did not end in time")));
            }
            std::thread::sleep(Duration::from_millis(LIMITED_RUN_POLL_MS));
        }
        child.wait_with_output().map(|output| output_of(&output))
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

    fn monotonic(&self) -> Duration {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        ORIGIN.get_or_init(Instant::now).elapsed()
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
    let bundle = contents.and_then(Path::parent).filter(|bundle| bundle.extension().is_some_and(|extension| extension == BUNDLE_EXTENSION));
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

/// The tools the swap as root runs, by their full paths: the system's own, or stand-ins in a test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootTools {
    pub mktemp: String,
    pub dirname: String,
    pub mkdir: String,
    pub ditto: String,
    pub chown: String,
    pub stat: String,
    pub chmod: String,
    pub codesign: String,
    pub plist_buddy: String,
    pub mv: String,
    pub rm: String,
    pub rmdir: String,
    pub find: String,
}

impl RootTools {
    /// The tools of macOS.
    #[must_use]
    pub fn system() -> Self {
        let tool = |path: &str| path.to_owned();
        Self {
            mktemp: tool(MKTEMP_TOOL),
            dirname: tool(DIRNAME_TOOL),
            mkdir: tool(MKDIR_TOOL),
            ditto: tool(DITTO_TOOL),
            chown: tool(CHOWN_TOOL),
            stat: tool(STAT_TOOL),
            chmod: tool(CHMOD_TOOL),
            codesign: tool(CODESIGN_TOOL),
            plist_buddy: tool(PLIST_BUDDY_TOOL),
            mv: tool(MV_TOOL),
            rm: tool(RM_TOOL),
            rmdir: tool(RMDIR_TOOL),
            find: tool(FIND_TOOL),
        }
    }
}

/// The swap root runs, a shell script of the tools with its values as arguments:
/// `bundle source requirement version user keep`.
///
/// It makes a folder only root can enter beside the bundle and works inside it by relative paths, so the folder renamed
/// or replaced from outside changes nothing. It copies `source` there, gives the copy the owner of the bundle and takes
/// from it whatever rights the user could have given it (access lists, write bits of group and others, set-id and sticky
/// bits: `ditto` keeps them), checks the copy's signature and version, and swaps it in for the bundle by two renames; if
/// the second fails, the first is undone, and if that fails too, the old bundle stays in the folder and the error says
/// where. From the swap on nothing fails the script. With `keep` the old bundle, checked against the same signature or
/// else removed, stays in the folder: its folders and the folder itself go to `user`, with the right to delete the folder,
/// and the folder's path is printed. Otherwise the old bundle and the folder go.
#[must_use]
pub fn root_swap_script(tools: &RootTools) -> String {
    format!(
        r#"set -eu
umask 022
bundle=$1 source=$2 requirement=$3 version=$4 user=$5 keep=$6
work=$({mktemp} -d "$({dirname} "$bundle")/{prefix}XXXXXX")
cd "$work"
trap '{rm} -rf ./{new}; cd /; {rmdir} "$work" 2>/dev/null || :' EXIT
{mkdir} ./{new}
{ditto} "$source" ./{new}/{app}
{chown} -R "$({stat} -f %u:%g "$bundle")" ./{new}
{chmod} -R -N ./{new}
{chmod} -R go-w,a-st ./{new}
{codesign} --verify --strict --deep -R "=$requirement" ./{new}/{app}
test "$({plist_buddy} -c 'Print :CFBundleShortVersionString' ./{new}/{app}/{plist})" = "$version"
{mv} "$bundle" ./{previous}
if ! {mv} ./{new}/{app} "$bundle"; then
  {mv} ./{previous} "$bundle" || {{ echo "the old version stays in $work/{previous}" >&2; exit 1; }}
  exit 1
fi
{rmdir} ./{new} || :
if [ "$keep" != keep ]; then
  {rm} -rf ./{previous}
  exit 0
fi
if {codesign} --verify --strict --deep -R "=$requirement" ./{previous}; then
  {find} ./{previous} -type d -exec {chown} "$user" {{}} + || :
else
  {rm} -rf ./{previous} || :
fi
{chown} "$user" . || :
{chmod} +a "user:$user allow delete" . || :
trap - EXIT
echo "$work"
"#,
        mktemp = tools.mktemp,
        dirname = tools.dirname,
        mkdir = tools.mkdir,
        ditto = tools.ditto,
        chown = tools.chown,
        stat = tools.stat,
        chmod = tools.chmod,
        codesign = tools.codesign,
        plist_buddy = tools.plist_buddy,
        mv = tools.mv,
        rm = tools.rm,
        rmdir = tools.rmdir,
        find = tools.find,
        prefix = UPDATE_WORK_PREFIX,
        new = UPDATE_NEW_FOLDER,
        previous = UPDATE_PREVIOUS_FOLDER,
        app = ARCHIVE_BUNDLE_NAME,
        plist = BUNDLE_INFO_PLIST,
    )
}

/// One argument of `sh`, quoted.
fn shell_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The command line of `sh` that runs the swap as root with its values.
fn root_swap_command(tools: &RootTools, bundle: &Path, source: &Path, requirement: &str, version: &str, user: &str, keep: bool) -> String {
    let script = root_swap_script(tools);
    let keep_word = if keep { "keep" } else { "discard" };
    let words = ["/bin/sh", "-c", script.as_str(), "keyswitch-update", &path_text(bundle), &path_text(source), requirement, version, user, keep_word];
    words.iter().map(|word| shell_quoted(word)).collect::<Vec<String>>().join(" ")
}

/// The swap as root: the system asks for the administrator's password with `prompt`, then runs [`root_swap_script`] with
/// the tools of macOS. With `keep`, returns the folder root left the old bundle in.
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
    let line = root_swap_command(&RootTools::system(), bundle, source, requirement, version, &user, keep);
    let arguments = texts(&[
        "-e",
        "on run argv",
        "-e",
        "do shell script (item 1 of argv) with prompt (item 2 of argv) with administrator privileges",
        "-e",
        "end run",
        &line,
        prompt,
    ]);
    let output = commands.run_clean(OSASCRIPT_TOOL, &arguments).map_err(io_failed)?;
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

    fn write(&self, name: &str, text: &str) {
        let _ = fs::create_dir_all(&self.downloads);
        let _ = fs::write(self.record(name), text);
    }

    fn read(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.record(name)).ok()
    }

    fn remove(&self, name: &str) {
        let _ = fs::remove_file(self.record(name));
    }

    /// The versions that did not start: an automatic check never installs them again.
    #[must_use]
    pub fn failed_versions(&self) -> Vec<String> {
        let text = self.read(UPDATE_FAILED_VERSIONS_FILE).unwrap_or_default();
        text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect()
    }

    fn add_failed_version(&self, version: &str) {
        let mut versions = self.failed_versions();
        if !versions.iter().any(|known| known == version) {
            versions.push(version.to_owned());
        }
        self.write(UPDATE_FAILED_VERSIONS_FILE, &format!("{}\n", versions.join("\n")));
    }

    /// The outcome of an update for the next start to tell: `kind new old`, and where the old version stays when it
    /// could not go back.
    fn write_result(&self, kind: &str, new_version: &str, old_version: &str, place: Option<&Path>) {
        let place = place.map(|place| format!(" {}", path_text(place))).unwrap_or_default();
        self.write(UPDATE_RESULT_FILE, &format!("{kind} {new_version} {old_version}{place}\n"));
    }

    /// The outcome of the last update, read once.
    fn take_result(&self) -> Option<String> {
        let text = self.read(UPDATE_RESULT_FILE)?;
        self.remove(UPDATE_RESULT_FILE);
        Some(text)
    }

    /// An update is under way from `old` to `new` since `now`.
    fn mark_pending(&self, now: u64, old_version: &str, new_version: &str) {
        self.write(UPDATE_PENDING_FILE, &format!("{now} {old_version} {new_version}\n"));
    }

    fn clear_pending(&self) {
        self.remove(UPDATE_PENDING_FILE);
    }

    /// The update under way: since when, from which version, to which.
    fn pending(&self) -> Option<(u64, String, String)> {
        let text = self.read(UPDATE_PENDING_FILE)?;
        let words: Vec<&str> = text.split_whitespace().collect();
        let [since, old_version, new_version] = words.as_slice() else { return None };
        Some((since.parse().ok()?, (*old_version).to_owned(), (*new_version).to_owned()))
    }

    /// The new copy of an update says it runs: its version and its process.
    fn write_started(&self, version: &str, process: u32) {
        self.write(UPDATE_STARTED_FILE, &format!("{version} {process}\n"));
    }

    /// The process of `version` that said it runs.
    fn started(&self, version: &str) -> Option<u32> {
        let text = self.read(UPDATE_STARTED_FILE)?;
        let (named, process) = text.trim().split_once(' ')?;
        if named != version {
            return None;
        }
        process.parse().ok()
    }

    fn clear_started(&self) {
        self.remove(UPDATE_STARTED_FILE);
    }

    /// The old version could not go back and stays at `previous`: no start removes it.
    fn strand(&self, previous: &Path) {
        self.write(UPDATE_STRANDED_FILE, &format!("{}\n", path_text(previous)));
    }

    /// Where an old version that could not go back stays, while it is still there.
    fn stranded(&self) -> Option<PathBuf> {
        let place = PathBuf::from(self.read(UPDATE_STRANDED_FILE)?.trim_end_matches('\n'));
        if place.exists() {
            return Some(place);
        }
        self.remove(UPDATE_STRANDED_FILE);
        None
    }
}

/// What to tell the user of the last update, from its record.
#[must_use]
pub fn result_notice(record: &str) -> Option<(String, String)> {
    let mut words = record.trim_end_matches('\n').splitn(4, ' ');
    match (words.next(), words.next(), words.next(), words.next()) {
        (Some(RESULT_FAILED), Some(new), Some(old), None) => Some((
            "Обновление KeySwitch не установлено".to_owned(),
            format!("Версия {new} не запустилась, и вернулась версия {old}. Сама она больше не установится, «Установить сейчас» в настройках поставит её снова."),
        )),
        (Some(RESULT_STRANDED), Some(new), Some(old), Some(place)) => Some((
            "Обновление KeySwitch не установлено".to_owned(),
            format!("Версия {new} не запустилась, а вернуть версию {old} на место не удалось: она осталась в папке «{place}». Установите KeySwitch заново со страницы выпусков."),
        )),
        _ => None,
    }
}

/// The start of the new copy of an update, while the update is under way: it marks that it runs, which the helper waits
/// for, and says so to the user at once.
#[must_use]
pub fn mark_started(files: &UpdateFiles, version: &str, process: u32) -> Option<(String, String)> {
    let (_, old_version, new_version) = files.pending()?;
    if new_version != version {
        return None;
    }
    files.write_started(version, process);
    Some(("KeySwitch обновлён".to_owned(), format!("Установлена версия {version} вместо {old_version}.")))
}

/// Remove the work folders an update left in `base`, but `keep`; one that cannot be removed stays.
fn remove_work_folders(base: &Path, keep: Option<&Path>) {
    let Ok(entries) = fs::read_dir(base) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with(UPDATE_WORK_PREFIX) && Some(path.as_path()) != keep {
            let _ = remove_tree(&path);
        }
    }
}

/// At the start of the program: unless an update is under way, remove what updates left (their work folders beside the
/// application and in the folder of the downloads, the downloaded archives), but the folder of an old version that could
/// not go back; and say how the last update ended, once.
#[must_use]
pub fn startup(files: &UpdateFiles, bundle: Option<&Bundle>, now: u64) -> Option<(String, String)> {
    let under_way = files.pending().is_some_and(|(since, _, _)| now.saturating_sub(since) < UPDATE_PENDING_STALE_SECONDS);
    if !under_way {
        files.clear_pending();
        files.clear_started();
        let stranded = files.stranded();
        let keep = stranded.as_deref().and_then(Path::parent);
        if let Some(bundle) = bundle {
            remove_work_folders(&bundle.parent, keep);
        }
        remove_work_folders(&files.downloads, keep);
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
/// whether the swap ran as root, and the process of the old copy, whose end it waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finish {
    pub bundle: PathBuf,
    pub previous: PathBuf,
    pub old_version: String,
    pub new_version: String,
    pub show_window: bool,
    pub privileged: bool,
    pub parent: u32,
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
            self.parent.to_string(),
        ]
    }

    /// The command line after [`FINISH_UPDATE_ARGUMENT`].
    #[must_use]
    pub fn parse(arguments: &[String]) -> Option<Self> {
        let [bundle, previous, old_version, new_version, window, rights, parent] = arguments else { return None };
        let show_window = [FINISH_WINDOW_SHOWN, FINISH_WINDOW_HIDDEN].iter().position(|word| word == window)? == 0;
        let privileged = [FINISH_RIGHTS_ADMINISTRATOR, FINISH_RIGHTS_USER].iter().position(|word| word == rights)? == 0;
        Some(Self {
            bundle: PathBuf::from(bundle),
            previous: PathBuf::from(previous),
            old_version: old_version.clone(),
            new_version: new_version.clone(),
            show_window,
            privileged,
            parent: parent.parse().ok()?,
        })
    }

    fn work(&self) -> PathBuf {
        self.previous.parent().map_or_else(|| self.previous.clone(), Path::to_path_buf)
    }

    fn executable(&self) -> PathBuf {
        self.bundle.join(BUNDLE_EXECUTABLE)
    }
}

/// Whether a process runs; a question `kill` cannot answer counts as yes.
fn alive(commands: &dyn Commands, process: u32) -> bool {
    commands.run(KILL_TOOL, &texts(&["-0", &process.to_string()])).map_or(true, |output| output.success)
}

/// The processes that have a file open (the lock of a running copy, or the executable a process runs), as `lsof` lists
/// them within its time; `None` when it cannot tell. `lsof` ends with 1 both when nothing has the file open and when it
/// failed, and only a failure says why.
fn processes_with(commands: &dyn Commands, file: &Path) -> Option<Vec<u32>> {
    let output = commands.run_limited(LSOF_TOOL, &texts(&["-t", "--", &path_text(file)]), Duration::from_millis(LSOF_TIME_LIMIT_MS)).ok()?;
    let found: Vec<u32> = output.stdout.lines().filter_map(|line| line.trim().parse().ok()).collect();
    if found.is_empty() && !output.success && !output.stderr.trim().is_empty() {
        return None;
    }
    Some(found)
}

/// Look until `condition` holds, for at most `timeout` of the monotonic clock.
fn wait_for(commands: &dyn Commands, timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = commands.monotonic() + timeout;
    loop {
        if condition() {
            return true;
        }
        if commands.monotonic() >= deadline {
            return false;
        }
        commands.pause(Duration::from_millis(UPDATE_POLL_INTERVAL_MS));
    }
}

/// Start the program from `bundle`, in the background with its window hidden or with the window when it was open; a
/// failed start is tried again.
fn open_program(commands: &dyn Commands, bundle: &Path, show_window: bool) -> bool {
    let bundle = path_text(bundle);
    let arguments = if show_window { texts(&[&bundle]) } else { texts(&["-g", &bundle, "--args", "--hidden"]) };
    for _ in 0..OPEN_ATTEMPTS {
        match commands.run(OPEN_TOOL, &arguments) {
            Ok(output) if output.success => return true,
            Ok(output) => logsetup::error(LOGGER, &format!("KeySwitch не запустился: {}", output.stderr.trim())),
            Err(error) => logsetup::error(LOGGER, &format!("KeySwitch не запустился: {error}")),
        }
        commands.pause(Duration::from_millis(OPEN_RETRY_MS));
    }
    false
}

/// The new copy, once it says it runs: by its own mark, or, for a version from before the mark, by the lock of a running
/// copy held by a process that runs the new executable.
fn new_copy(commands: &dyn Commands, files: &UpdateFiles, finish: &Finish) -> Option<u32> {
    if let Some(process) = files.started(&finish.new_version) {
        return Some(process);
    }
    let holders = processes_with(commands, &files.lock)?;
    let runners = processes_with(commands, &finish.executable())?;
    holders.into_iter().find(|holder| runners.contains(holder))
}

/// The new copy said it runs within its time and kept running; its process, if it said so.
fn watch_new_copy(commands: &dyn Commands, files: &UpdateFiles, finish: &Finish) -> (Option<u32>, bool) {
    let mut found = None;
    wait_for(commands, Duration::from_secs(UPDATE_START_TIMEOUT_SECONDS), || {
        found = new_copy(commands, files, finish);
        found.is_some()
    });
    let Some(process) = found else { return (None, false) };
    let lost = wait_for(commands, Duration::from_secs(UPDATE_SETTLE_SECONDS), || !alive(commands, process));
    (Some(process), !lost)
}

/// Stop the processes and wait for them to end; those that do not end in time are killed.
fn stop_processes(commands: &dyn Commands, processes: &[u32]) {
    let ended = || processes.iter().all(|process| !alive(commands, *process));
    for signal in ["-TERM", "-KILL"] {
        for process in processes {
            let _ = commands.run(KILL_TOOL, &texts(&[signal, &process.to_string()]));
        }
        if wait_for(commands, Duration::from_secs(UPDATE_STOP_WAIT_SECONDS), ended) {
            return;
        }
    }
}

/// Put the old version back: by two renames, or as root after the password again. The work folder goes once it is back.
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
    let ended = wait_for(commands, Duration::from_secs(UPDATE_OLD_COPY_EXIT_TIMEOUT_SECONDS), || !alive(commands, finish.parent));
    if !ended {
        // The old copy still runs, from its memory: its bundle goes back under it, and the new version is not to blame.
        logsetup::error(LOGGER, "Прежняя копия KeySwitch не закрылась, обновление отменено");
        if !restore(commands, finish, process) {
            files.strand(&finish.previous);
        }
        files.clear_pending();
        return false;
    }
    let (holder, started) = if open_program(commands, &finish.bundle, finish.show_window) { watch_new_copy(commands, files, finish) } else { (None, false) };
    if started {
        let _ = remove_tree(&finish.work());
        files.clear_pending();
        files.clear_started();
        logsetup::info(LOGGER, &format!("KeySwitch обновлён с версии {} до {}", finish.old_version, finish.new_version));
        return true;
    }
    logsetup::error(LOGGER, &format!("Версия {} не запустилась, возвращаем {}", finish.new_version, finish.old_version));
    // Whatever still runs of the new version goes: a copy that hangs without saying it runs would otherwise be the one
    // macOS brings forward when the old version is opened, since only one copy of the application may run.
    let mut running = processes_with(commands, &finish.executable()).unwrap_or_default();
    running.extend(holder.filter(|holder| !running.contains(holder)));
    stop_processes(commands, &running);
    files.add_failed_version(&finish.new_version);
    files.clear_pending();
    files.clear_started();
    if restore(commands, finish, process) {
        // Written before the old version starts, which reads it once, at its start.
        files.write_result(RESULT_FAILED, &finish.new_version, &finish.old_version, None);
        open_program(commands, &finish.bundle, finish.show_window);
    } else {
        files.strand(&finish.previous);
        files.write_result(RESULT_STRANDED, &finish.new_version, &finish.old_version, Some(finish.previous.as_path()));
    }
    false
}

/// The helper's command line, after [`FINISH_UPDATE_ARGUMENT`]: its exit status.
pub fn finish_main(commands: &dyn Commands, files: &UpdateFiles, arguments: &[String], process: u32) -> i32 {
    let Some(finish) = Finish::parse(arguments) else {
        eprintln!("usage: KeySwitch {FINISH_UPDATE_ARGUMENT} <bundle> <previous> <old version> <new version> shown|hidden administrator|user <old process>");
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
        let new_version = prepared.version.to_string();
        self.files.clear_started();
        self.files.mark_pending(commands.now_seconds(), &self.current_version, &new_version);
        let finish = Finish {
            bundle: bundle.path.clone(),
            previous,
            old_version: self.current_version.clone(),
            new_version,
            show_window,
            privileged,
            parent: self.process,
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

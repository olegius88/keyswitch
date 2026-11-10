//! The update of the macOS build against a fake of the system's tools: `ditto` unpacks a made-up bundle, `codesign`,
//! `PlistBuddy`, `osascript` and `lsof` answer as each case needs, and the clock only counts.
use super::*;

use std::os::unix::fs::PermissionsExt as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use ks_updates::ReleaseAsset;

const REQUIREMENT: &str = "identifier \"io.github.olegius88.KeySwitch\" and anchor apple generic";

type Handler = Box<dyn Fn(&str, &[String]) -> io::Result<Output> + Send + Sync>;
type Starter = Box<dyn Fn(&Path, &[String]) -> io::Result<()> + Send + Sync>;

struct Fake {
    handler: Handler,
    starter: Starter,
    calls: Mutex<Vec<(String, Vec<String>)>>,
    started: Mutex<Vec<(PathBuf, Vec<String>)>>,
    paused: AtomicU64,
}

impl Fake {
    fn new(handler: impl Fn(&str, &[String]) -> io::Result<Output> + Send + Sync + 'static) -> Self {
        Self { handler: Box::new(handler), starter: Box::new(|_, _| Ok(())), calls: Mutex::default(), started: Mutex::default(), paused: AtomicU64::new(0) }
    }

    fn starting(mut self, starter: impl Fn(&Path, &[String]) -> io::Result<()> + Send + Sync + 'static) -> Self {
        self.starter = Box::new(starter);
        self
    }

    fn ran(&self, program: &str) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().iter().filter(|(name, _)| name == program).map(|(_, arguments)| arguments.clone()).collect()
    }
}

impl Commands for Fake {
    fn run(&self, program: &str, arguments: &[String]) -> io::Result<Output> {
        self.calls.lock().unwrap().push((program.to_owned(), arguments.to_vec()));
        (self.handler)(program, arguments)
    }

    fn start_detached(&self, program: &Path, arguments: &[String]) -> io::Result<()> {
        self.started.lock().unwrap().push((program.to_path_buf(), arguments.to_vec()));
        (self.starter)(program, arguments)
    }

    fn pause(&self, duration: Duration) {
        self.paused.fetch_add(u64::try_from(duration.as_millis()).unwrap(), Ordering::SeqCst);
    }

    fn now_seconds(&self) -> u64 {
        1_000_000
    }
}

#[expect(clippy::unnecessary_wraps, reason = "the answer a fake tool gives")]
fn answer(success: bool, stdout: &str) -> io::Result<Output> {
    Ok(Output { success, stdout: stdout.to_owned(), stderr: if success { String::new() } else { "it failed\n".to_owned() } })
}

fn temporary(name: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!("ks-mac-update-{}-{name}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst)));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

/// Make a bundle with its executable, as an installed one or as `ditto` unpacks one.
fn make_bundle(path: &Path, content: &str) {
    fs::create_dir_all(path.join("Contents/MacOS")).unwrap();
    fs::write(path.join(BUNDLE_EXECUTABLE), content).unwrap();
}

fn mode(path: &Path, bits: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(bits)).unwrap();
}

/// A place with an installed bundle, its folder, the folder of the downloads and an archive of version 2.0.0.
struct Place {
    root: PathBuf,
    bundle: Bundle,
    files: UpdateFiles,
    archive: PathBuf,
}

impl Place {
    fn new(name: &str) -> Self {
        let root = temporary(name);
        let parent = root.join("Applications");
        let path = parent.join("KeySwitch.app");
        make_bundle(&path, "old");
        let files = UpdateFiles::new(&root.join("data"));
        fs::create_dir_all(&files.downloads).unwrap();
        let archive = files.downloads.join("KeySwitch-2.0.0-macos-x86_64.zip");
        fs::write(&archive, "zip").unwrap();
        Self { root, bundle: Bundle { path, parent }, files, archive }
    }

    fn executable(&self) -> PathBuf {
        self.bundle.path.join(BUNDLE_EXECUTABLE)
    }

    fn content(&self) -> String {
        fs::read_to_string(self.executable()).unwrap_or_default()
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        // Folders a case made read-only become removable again first.
        for folder in [self.bundle.parent.clone(), self.bundle.path.join("Contents")] {
            let _ = fs::set_permissions(&folder, fs::Permissions::from_mode(0o755));
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// The tools of a good update to version 2.0.0, with `lsof` as given for the lock file; no process runs an executable.
fn tools(lsof: impl Fn() -> String + Send + Sync + 'static) -> impl Fn(&str, &[String]) -> io::Result<Output> + Send + Sync + 'static {
    tools_running(lsof, String::new)
}

/// The tools of a good update, with `lsof` as given for the lock file and for the executable of a bundle.
fn tools_running(
    lsof: impl Fn() -> String + Send + Sync + 'static,
    running: impl Fn() -> String + Send + Sync + 'static,
) -> impl Fn(&str, &[String]) -> io::Result<Output> + Send + Sync + 'static {
    move |program: &str, arguments: &[String]| match program {
        CODESIGN_TOOL if arguments[0] == "-d" => answer(true, &format!("Executable=/x\ndesignated => {REQUIREMENT}\n")),
        CODESIGN_TOOL => answer(arguments[4] == format!("={REQUIREMENT}"), ""),
        DITTO_TOOL => {
            make_bundle(&Path::new(&arguments[3]).join(ARCHIVE_BUNDLE_NAME), "new");
            answer(true, "")
        }
        PLIST_BUDDY_TOOL => answer(true, "2.0.0\n"),
        LSOF_TOOL if arguments[2].ends_with(BUNDLE_EXECUTABLE) => answer(true, &running()),
        LSOF_TOOL => answer(true, &lsof()),
        ID_TOOL => answer(true, "someone\n"),
        OPEN_TOOL | KILL_TOOL | OSASCRIPT_TOOL => answer(true, ""),
        program if program.ends_with(BUNDLE_EXECUTABLE) => answer(true, "KeySwitch 2.0.0\n"),
        _ => answer(false, ""),
    }
}

fn good_tools() -> impl Fn(&str, &[String]) -> io::Result<Output> + Send + Sync + 'static {
    tools(String::new)
}

// ------------------------------------------------------------------------------------------------- the system itself

#[test]
fn the_system_runs_programs_and_starts_the_helper_apart() {
    let output = SystemCommands.run("/bin/sh", &texts(&["-c", "echo out; echo err >&2; exit 3"])).unwrap();
    assert_eq!(output, Output { success: false, stdout: "out\n".to_owned(), stderr: "err\n".to_owned() });
    assert!(SystemCommands.run("/no/such/program", &[]).is_err());
    SystemCommands.start_detached(Path::new("/bin/sh"), &texts(&["-c", "exit 0"])).unwrap();
    assert!(SystemCommands.start_detached(Path::new("/no/such/program"), &[]).is_err());
    SystemCommands.pause(Duration::ZERO);
    assert!(SystemCommands.now_seconds() > 0);
}

// ------------------------------------------------------------------------------------------------------- the bundle

#[test]
fn the_bundle_is_the_application_the_executable_lies_in() {
    let bundle = running_bundle(Path::new("/Applications/KeySwitch.app/Contents/MacOS/KeySwitch")).unwrap();
    assert_eq!(bundle, Bundle { path: PathBuf::from("/Applications/KeySwitch.app"), parent: PathBuf::from("/Applications") });
    assert_eq!(running_bundle(Path::new("/K.app/Contents/MacOS/K")).unwrap().parent, PathBuf::from("/"));
    for path in [
        "/x/KeySwitch.app/Contents/Other/KeySwitch",
        "/x/KeySwitch.app/Other/MacOS/KeySwitch",
        "/x/KeySwitch/Contents/MacOS/KeySwitch",
        "/Contents/MacOS/KeySwitch",
        "KeySwitch",
    ] {
        assert_eq!(running_bundle(Path::new(path)).unwrap_err().0, NOT_A_BUNDLE, "{path}");
    }
    let translocated = "/private/var/folders/x/AppTranslocation/1234/d/KeySwitch.app/Contents/MacOS/KeySwitch";
    assert_eq!(running_bundle(Path::new(translocated)).unwrap_err().0, TRANSLOCATED);
}

#[test]
fn the_requirement_comes_from_the_running_process_and_must_be_a_real_one() {
    let with = |success: bool, stdout: &'static str| Fake::new(move |_, _| answer(success, stdout));
    let fake = with(true, "Executable=/a\ndesignated => identifier \"x\" and anchor apple generic\n");
    assert_eq!(designated_requirement(&fake, 42).unwrap(), "identifier \"x\" and anchor apple generic");
    assert_eq!(fake.ran(CODESIGN_TOOL), [texts(&["-d", "-r-", "42"])]);
    for (success, stdout) in [(true, "nothing\n"), (true, "designated => \n"), (true, "designated => cdhash H\"ab\"\n"), (false, "designated => identifier \"x\"\n")] {
        assert_eq!(designated_requirement(&with(success, stdout), 1).unwrap_err().0, UNSIGNED, "{stdout}");
    }
    let broken = Fake::new(|_, _| Err(io::Error::other("no codesign")));
    assert!(designated_requirement(&broken, 1).unwrap_err().0.contains("no codesign"));
}

#[test]
fn a_tree_is_the_users_when_every_entry_is_and_every_folder_is_writable() {
    let root = temporary("owned");
    let owner = fs::metadata(&root).unwrap().uid();
    make_bundle(&root.join("A.app"), "x");
    assert!(owned_tree(&root.join("A.app"), owner));
    assert!(!owned_tree(&root.join("A.app"), owner + 1));
    assert!(!owned_tree(&root.join("missing"), owner));
    mode(&root.join("A.app/Contents"), 0o555);
    assert!(!owned_tree(&root.join("A.app"), owner));
    mode(&root.join("A.app/Contents"), 0o300);
    assert!(!owned_tree(&root.join("A.app"), owner));
    mode(&root.join("A.app/Contents"), 0o755);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_folder_that_cannot_be_made_calls_for_the_password_or_is_refused() {
    assert!(password_or_refusal(&io::Error::from(io::ErrorKind::PermissionDenied)).is_ok());
    assert_eq!(password_or_refusal(&io::Error::from(io::ErrorKind::ReadOnlyFilesystem)).unwrap_err().0, READ_ONLY_VOLUME);
    assert!(password_or_refusal(&io::Error::from(io::ErrorKind::NotFound)).unwrap_err().0.contains("рядом с приложением"));
}

#[test]
fn the_swap_runs_in_the_users_rights_only_where_the_user_owns_everything() {
    let place = Place::new("placement");
    assert_eq!(placement(&place.bundle, 7).unwrap(), Placement::InPlace);
    assert!(!work_folder(&place.bundle.parent, 7).exists(), "the trial folder is removed");
    mode(&place.bundle.path.join("Contents"), 0o555);
    assert_eq!(placement(&place.bundle, 7).unwrap(), Placement::NeedsPassword);
    mode(&place.bundle.path.join("Contents"), 0o755);
    mode(&place.bundle.parent, 0o555);
    assert_eq!(placement(&place.bundle, 7).unwrap(), Placement::NeedsPassword);
    mode(&place.bundle.parent, 0o755);
    let nowhere = Bundle { path: place.root.join("gone/K.app"), parent: place.root.join("gone") };
    assert!(placement(&nowhere, 7).is_err());
}

#[test]
fn an_archive_is_named_for_its_version() {
    assert_eq!(archive_version(Path::new("/d/KeySwitch-0.71.2-macos-x86_64.zip")).unwrap().to_string(), "0.71.2");
    assert!(archive_version(Path::new("/d/Other-0.71.2-macos-x86_64.zip")).is_err());
    assert!(archive_version(Path::new("/d/KeySwitch-0.71-macos-arm64.zip")).is_err());
    assert!(archive_version(Path::new("/")).is_err());
}

#[test]
fn a_bundle_must_carry_the_required_signature_and_its_version() {
    let version = SemanticVersion::parse("2.0.0").unwrap();
    let bundle = Path::new("/new/KeySwitch.app");
    let fake = Fake::new(good_tools());
    check_bundle(&fake, bundle, REQUIREMENT, &version).unwrap();
    assert_eq!(fake.ran(CODESIGN_TOOL), [texts(&["--verify", "--strict", "--deep", "-R", &format!("={REQUIREMENT}"), "/new/KeySwitch.app"])]);
    assert_eq!(fake.ran(PLIST_BUDDY_TOOL), [texts(&["-c", "Print :CFBundleShortVersionString", "/new/KeySwitch.app/Contents/Info.plist"])]);
    assert!(check_bundle(&fake, bundle, "another", &version).unwrap_err().0.starts_with(SIGNATURE_MISMATCH));
    let older = SemanticVersion::parse("1.9.0").unwrap();
    assert!(check_bundle(&fake, bundle, REQUIREMENT, &older).unwrap_err().0.contains("«2.0.0», а не 1.9.0"));
    let unreadable = Fake::new(|program, _| if program == PLIST_BUDDY_TOOL { answer(false, "") } else { answer(true, "") });
    assert!(check_bundle(&unreadable, bundle, REQUIREMENT, &version).is_err());
    let broken = Fake::new(|_, _| Err(io::Error::other("gone")));
    assert!(check_bundle(&broken, bundle, REQUIREMENT, &version).is_err());
    let no_plist_tool = Fake::new(|program, _| if program == CODESIGN_TOOL { answer(true, "") } else { Err(io::Error::other("gone")) });
    assert!(check_bundle(&no_plist_tool, bundle, REQUIREMENT, &version).is_err());
}

// --------------------------------------------------------------------------------------------------------- preparing

#[test]
fn an_archive_is_unpacked_beside_the_application_and_checked() {
    let place = Place::new("prepare");
    let fake = Fake::new(good_tools());
    let prepared = prepare(&fake, &place.archive, &place.bundle, &place.files.downloads, 7).unwrap();
    assert_eq!(prepared.placement, Placement::InPlace);
    assert_eq!(prepared.work, work_folder(&place.bundle.parent, 7));
    assert_eq!(prepared.new_bundle, prepared.work.join("new/KeySwitch.app"));
    assert_eq!(prepared.requirement, REQUIREMENT);
    assert_eq!(prepared.version.to_string(), "2.0.0");
    let started = fake.ran(&path_text(&prepared.new_bundle.join(BUNDLE_EXECUTABLE)));
    assert_eq!(started, [texts(&["--version"])]);
    assert_eq!(fake.ran(DITTO_TOOL)[0][..2], texts(&["-x", "-k"]));
}

#[test]
fn without_rights_beside_the_application_the_archive_is_unpacked_in_the_downloads() {
    let place = Place::new("prepare-password");
    mode(&place.bundle.parent, 0o555);
    let fake = Fake::new(good_tools());
    let prepared = prepare(&fake, &place.archive, &place.bundle, &place.files.downloads, 7).unwrap();
    assert_eq!(prepared.placement, Placement::NeedsPassword);
    assert_eq!(prepared.work, work_folder(&place.files.downloads, 7));
    mode(&place.bundle.parent, 0o755);
    // A bundle the user does not own entirely: the trial folder beside it goes, and the work moves to the downloads.
    mode(&place.bundle.path.join("Contents"), 0o555);
    let prepared = prepare(&fake, &place.archive, &place.bundle, &place.files.downloads, 8).unwrap();
    assert_eq!(prepared.placement, Placement::NeedsPassword);
    assert!(!work_folder(&place.bundle.parent, 8).exists());
    // No folder of the downloads to unpack in.
    let gone = place.root.join("no-downloads");
    assert!(prepare(&fake, &place.archive, &place.bundle, &gone, 9).is_err());
    mode(&place.bundle.path.join("Contents"), 0o755);
    mode(&place.bundle.parent, 0o555);
    assert!(prepare(&fake, &place.archive, &place.bundle, &gone, 9).is_err());
    mode(&place.bundle.parent, 0o755);
}

#[test]
fn every_failed_check_leaves_nothing_behind() {
    let place = Place::new("prepare-failures");
    let failing = |failure: &'static str| {
        let good = good_tools();
        Fake::new(move |program: &str, arguments: &[String]| match (failure, program) {
            ("ditto", DITTO_TOOL) => answer(false, ""),
            ("empty", DITTO_TOOL) => answer(true, ""),
            ("signature", CODESIGN_TOOL) if arguments[0] == "--verify" => answer(false, ""),
            ("start", program) if program.ends_with(BUNDLE_EXECUTABLE) => answer(false, ""),
            ("version", program) if program.ends_with(BUNDLE_EXECUTABLE) => answer(true, "KeySwitch 1.0.0\n"),
            ("silent", program) if program.ends_with(BUNDLE_EXECUTABLE) => Err(io::Error::other("no program")),
            _ => good(program, arguments),
        })
    };
    for (failure, text) in [
        ("ditto", "не распакован"),
        ("empty", NO_BUNDLE_IN_ARCHIVE),
        ("signature", SIGNATURE_MISMATCH),
        ("start", NOT_STARTED),
        ("version", NOT_STARTED),
        ("silent", "no program"),
    ] {
        let error = prepare(&failing(failure), &place.archive, &place.bundle, &place.files.downloads, 7).unwrap_err();
        assert!(error.0.contains(text), "{failure}: {error}");
        assert!(!work_folder(&place.bundle.parent, 7).exists(), "{failure}");
    }
    let unsigned = Fake::new(|_, _| answer(false, ""));
    assert_eq!(prepare(&unsigned, &place.archive, &place.bundle, &place.files.downloads, 7).unwrap_err().0, UNSIGNED);
    let fake = Fake::new(good_tools());
    assert!(prepare(&fake, Path::new("/d/archive.zip"), &place.bundle, &place.files.downloads, 7).is_err());
    let nowhere = Bundle { path: place.root.join("gone/K.app"), parent: place.root.join("gone") };
    assert!(prepare(&fake, &place.archive, &nowhere, &place.files.downloads, 7).is_err());
}

// ---------------------------------------------------------------------------------------------------------- swapping

#[test]
fn the_swap_is_two_renames_and_a_failed_second_undoes_the_first() {
    let place = Place::new("swap");
    let fake = Fake::new(good_tools());
    let prepared = prepare(&fake, &place.archive, &place.bundle, &place.files.downloads, 7).unwrap();
    let previous = swap_in_place(&prepared).unwrap();
    assert_eq!(place.content(), "new");
    assert_eq!(fs::read_to_string(previous.join(BUNDLE_EXECUTABLE)).unwrap(), "old");

    let again = prepare(&fake, &place.archive, &place.bundle, &place.files.downloads, 8).unwrap();
    fs::remove_dir_all(&again.new_bundle).unwrap();
    assert!(swap_in_place(&again).is_err());
    assert_eq!(place.content(), "new", "the first rename was undone");
    fs::remove_dir_all(&place.bundle.path).unwrap();
    assert!(swap_in_place(&again).is_err());
}

#[test]
fn putting_back_keeps_the_new_bundle_when_the_old_one_is_gone() {
    let root = temporary("put-back");
    make_bundle(&root.join("K.app"), "new");
    make_bundle(&root.join("previous"), "old");
    put_back(&root.join("K.app"), &root.join("previous"), &root.join("failed")).unwrap();
    assert_eq!(fs::read_to_string(root.join("K.app").join(BUNDLE_EXECUTABLE)).unwrap(), "old");
    assert!(put_back(&root.join("K.app"), &root.join("previous"), &root.join("failed")).is_err());
    assert_eq!(fs::read_to_string(root.join("K.app").join(BUNDLE_EXECUTABLE)).unwrap(), "old");
    assert!(put_back(&root.join("missing.app"), &root.join("previous"), &root.join("other")).is_err());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_root_step_runs_only_the_systems_tools_with_its_values_as_arguments() {
    let script = root_swap_script();
    for part in [
        "set -eu",
        "/usr/bin/mktemp -d \"$(/usr/bin/dirname \"$bundle\")/.KeySwitch-update.XXXXXX\"",
        "/usr/bin/ditto \"$source\" \"$work/new/KeySwitch.app\"",
        "/usr/bin/codesign --verify --strict --deep -R \"=$requirement\" \"$work/new/KeySwitch.app\"",
        "Print :CFBundleShortVersionString' \"$work/new/KeySwitch.app/Contents/Info.plist\")\" = \"$version\"",
        "/bin/chmod +a \"$user allow delete\" \"$work\"",
    ] {
        assert!(script.contains(part), "{part}");
    }
    assert_eq!(shell_quoted("it's"), r"'it'\''s'");
}

#[test]
fn the_root_step_asks_for_the_password_and_reports_where_it_left_the_old_bundle() {
    let place = Place::new("privileged");
    let work = place.bundle.parent.join(".KeySwitch-update.Ab12Cd");
    let reported = path_text(&work);
    let fake = Fake::new(move |program, _| match program {
        ID_TOOL => answer(true, "someone\n"),
        _ => answer(true, &format!("{reported}\n")),
    });
    let swapped = privileged_swap(&fake, &place.bundle.path, Path::new("/staged/K.app"), REQUIREMENT, "2.0.0", true, "Пароль").unwrap();
    assert_eq!(swapped, Some(work));
    let call = &fake.ran(OSASCRIPT_TOOL)[0];
    assert_eq!(call[3], "do shell script (item 1 of argv) with prompt (item 2 of argv) with administrator privileges");
    assert_eq!(call[7], "Пароль");
    assert!(call[6].starts_with("'/bin/sh' '-c' 'set -eu"), "{}", call[6]);
    assert!(call[6].ends_with(&format!("'keyswitch-update' '{}' '/staged/K.app' '{REQUIREMENT}' '2.0.0' 'someone' 'keep'", place.bundle.path.display())), "{}", call[6]);
    assert_eq!(privileged_swap(&fake, &place.bundle.path, Path::new("/s"), REQUIREMENT, "1.0.0", false, "p").unwrap(), None);
    assert!(fake.ran(OSASCRIPT_TOOL)[1][6].ends_with("'discard'"));

    for stdout in ["/elsewhere/.KeySwitch-update.Ab12Cd\n", &format!("{}/other\n", place.bundle.parent.display())] {
        let stdout = stdout.to_owned();
        let odd = Fake::new(move |_, _| answer(true, &stdout));
        assert!(privileged_swap(&odd, &place.bundle.path, Path::new("/s"), REQUIREMENT, "2.0.0", true, "p").unwrap_err().0.contains("неожиданный путь"));
    }
    let cancelled = Fake::new(|program, _| match program {
        OSASCRIPT_TOOL => Ok(Output { success: false, stdout: String::new(), stderr: "execution error: User canceled. (-128)\n".to_owned() }),
        _ => answer(true, "someone"),
    });
    assert_eq!(privileged_swap(&cancelled, &place.bundle.path, Path::new("/s"), REQUIREMENT, "2.0.0", true, "p").unwrap_err().0, CANCELLED);
    let refused = Fake::new(|program, _| answer(program == ID_TOOL, "someone"));
    assert!(privileged_swap(&refused, &place.bundle.path, Path::new("/s"), REQUIREMENT, "2.0.0", true, "p").unwrap_err().0.contains("не удалась"));
    let nameless = Fake::new(|_, _| answer(true, " \n"));
    assert!(privileged_swap(&nameless, &place.bundle.path, Path::new("/s"), REQUIREMENT, "2.0.0", true, "p").unwrap_err().0.contains("имя пользователя"));
    let silent = Fake::new(|program, _| if program == ID_TOOL { answer(true, "someone") } else { Err(io::Error::other("no osascript")) });
    assert!(privileged_swap(&silent, &place.bundle.path, Path::new("/s"), REQUIREMENT, "2.0.0", true, "p").is_err());
    let anonymous = Fake::new(|_, _| Err(io::Error::other("no id")));
    assert!(privileged_swap(&anonymous, &place.bundle.path, Path::new("/s"), REQUIREMENT, "2.0.0", true, "p").is_err());
}

// --------------------------------------------------------------------------------------------- records and the start

#[test]
fn the_records_keep_failed_versions_once_and_tell_the_last_outcome_once() {
    let place = Place::new("records");
    let files = &place.files;
    assert!(files.failed_versions().is_empty());
    files.add_failed_version("2.0.0");
    files.add_failed_version("2.0.0");
    files.add_failed_version("2.1.0");
    assert_eq!(files.failed_versions(), ["2.0.0", "2.1.0"]);
    files.write_result(RESULT_INSTALLED, "2.0.0", "1.0.0");
    assert_eq!(files.take_result().as_deref(), Some("installed 2.0.0 1.0.0\n"));
    assert_eq!(files.take_result(), None);
    files.mark_pending(5);
    assert_eq!(files.pending_since(), Some(5));
    files.clear_pending();
    assert_eq!(files.pending_since(), None);
}

#[test]
fn the_notice_names_the_versions() {
    let (title, message) = result_notice("installed 2.0.0 1.0.0\n").unwrap();
    assert_eq!((title.as_str(), message.as_str()), ("KeySwitch обновлён", "Установлена версия 2.0.0."));
    let (_, message) = result_notice("failed 2.0.0 1.0.0").unwrap();
    assert!(message.starts_with("Версия 2.0.0 не запустилась, и вернулась версия 1.0.0."));
    assert_eq!(result_notice("something else"), None);
}

#[test]
fn the_start_removes_leftovers_unless_an_update_is_under_way() {
    let place = Place::new("startup");
    let files = &place.files;
    let beside = work_folder(&place.bundle.parent, 3);
    let staged = work_folder(&files.downloads, 4);
    for folder in [&beside, &staged] {
        make_bundle(&folder.join("previous"), "old");
    }
    fs::write(place.bundle.parent.join("Other.app"), "x").unwrap();
    files.mark_pending(1_000);
    files.write_result(RESULT_INSTALLED, "2.0.0", "1.0.0");
    let notice = startup(files, Some(&place.bundle), 1_000 + UPDATE_PENDING_STALE_SECONDS - 1);
    assert_eq!(notice.unwrap().1, "Установлена версия 2.0.0.");
    assert!(beside.exists() && staged.exists() && place.archive.exists(), "an update under way keeps its folders");

    assert_eq!(startup(files, Some(&place.bundle), 1_000 + UPDATE_PENDING_STALE_SECONDS), None);
    assert!(!beside.exists() && !staged.exists() && !place.archive.exists());
    assert!(place.bundle.parent.join("Other.app").exists() && place.bundle.path.exists());
    assert_eq!(files.pending_since(), None);
    make_bundle(&staged, "x");
    assert_eq!(startup(files, None, 0), None);
    assert!(!staged.exists());
    let nothing = UpdateFiles::new(&place.root.join("nothing"));
    assert_eq!(startup(&nothing, None, 0), None);
}

#[test]
fn an_automatic_install_waits_for_the_user_when_it_cannot_go_by_itself() {
    let place = Place::new("gate");
    let release = |version: &str| UpdateRelease {
        version: SemanticVersion::parse(version).unwrap(),
        tag: format!("v{version}"),
        page_url: String::new(),
        notes: String::new(),
        asset: ReleaseAsset { name: String::new(), url: String::new(), size: 1, sha256: String::new() },
    };
    let gate = automatic_install_gate(place.files.clone(), Ok(place.bundle.clone()), 7);
    assert_eq!(gate(&release("2.0.0")), None);
    place.files.add_failed_version("2.0.0");
    assert!(gate(&release("2.0.0")).unwrap().contains("не запустилась при прошлой установке"));
    mode(&place.bundle.parent, 0o555);
    assert!(gate(&release("2.1.0")).unwrap().contains("нужен пароль администратора"));
    mode(&place.bundle.parent, 0o755);
    let gone = automatic_install_gate(place.files.clone(), Ok(Bundle { path: place.root.join("gone/K.app"), parent: place.root.join("gone") }), 7);
    assert!(gone(&release("2.1.0")).unwrap().contains("рядом с приложением"));
    let translocated = automatic_install_gate(place.files.clone(), Err(failed(TRANSLOCATED)), 7);
    assert!(translocated(&release("2.1.0")).unwrap().contains("временной копии"));
}

#[test]
fn the_install_setting_is_described_as_it_works_on_a_mac() {
    let specs = mac_update_settings();
    assert_eq!(specs.len(), update_settings().len());
    assert_eq!(specs[0].description, update_settings()[0].description);
    let install = specs.iter().find(|spec| spec.path == INSTALL_SETTING).unwrap();
    assert_eq!(install.description, INSTALL_DESCRIPTION);
}

// -------------------------------------------------------------------------------------------------------- the helper

fn finish_of(place: &Place, previous: &Path, privileged: bool) -> Finish {
    Finish {
        bundle: place.bundle.path.clone(),
        previous: previous.to_path_buf(),
        old_version: "1.0.0".to_owned(),
        new_version: "2.0.0".to_owned(),
        show_window: false,
        privileged,
    }
}

#[test]
fn the_helpers_command_line_round_trips() {
    for (show_window, privileged) in [(false, false), (true, true)] {
        let finish = Finish { bundle: PathBuf::from("/A/K.app"), previous: PathBuf::from("/A/w/previous"), old_version: "1.0.0".to_owned(), new_version: "2.0.0".to_owned(), show_window, privileged };
        let arguments = finish.arguments();
        assert_eq!(arguments[0], FINISH_UPDATE_ARGUMENT);
        assert_eq!(Finish::parse(&arguments[1..]), Some(finish));
    }
    let good = texts(&["/a", "/b", "1", "2", "hidden", "user"]);
    assert!(Finish::parse(&good).is_some());
    assert_eq!(Finish::parse(&good[..5]), None);
    let mut window = good.clone();
    window[4] = "maybe".to_owned();
    assert_eq!(Finish::parse(&window), None);
    let mut rights = good;
    rights[5] = "root".to_owned();
    assert_eq!(Finish::parse(&rights), None);
    let rootless = Finish { previous: PathBuf::new(), ..Finish::parse(&texts(&["/a", "/b", "1", "2", "hidden", "user"])).unwrap() };
    assert_eq!(rootless.work(), PathBuf::new());
}

#[test]
fn waiting_looks_until_the_condition_holds_or_the_time_is_up() {
    let fake = Fake::new(good_tools());
    assert!(wait_for(&fake, Duration::from_secs(1), || true));
    assert_eq!(fake.paused.load(Ordering::SeqCst), 0);
    assert!(!wait_for(&fake, Duration::from_secs(1), || false));
    assert_eq!(fake.paused.load(Ordering::SeqCst), 1_000);
    let lsof = Fake::new(|_, _| answer(false, "12\nnot a number\n 34 \n"));
    assert_eq!(lock_holders(&lsof, Path::new("/lock")), [12, 34]);
    assert_eq!(lsof.ran(LSOF_TOOL), [texts(&["-t", "--", "/lock"])]);
    let broken = Fake::new(|_, _| Err(io::Error::other("no lsof")));
    assert!(lock_holders(&broken, Path::new("/lock")).is_empty());
}

#[test]
fn the_program_starts_hidden_unless_its_window_was_open() {
    for (show_window, success) in [(false, true), (true, false)] {
        let fake = Fake::new(move |_, _| answer(success, ""));
        open_program(&fake, Path::new("/A/K.app"), show_window);
        let expected = if show_window { texts(&["/A/K.app"]) } else { texts(&["-g", "/A/K.app", "--args", "--hidden"]) };
        assert_eq!(fake.ran(OPEN_TOOL), [expected]);
    }
    open_program(&Fake::new(|_, _| Err(io::Error::other("no open"))), Path::new("/A/K.app"), false);
}

/// `lsof` answers from a list, one answer a call, the last one ever after.
fn lsof_answers(answers: &[&str]) -> impl Fn() -> String + Send + Sync + 'static {
    let answers: Vec<String> = answers.iter().map(|answer| (*answer).to_owned()).collect();
    let calls = AtomicUsize::new(0);
    move || answers[calls.fetch_add(1, Ordering::SeqCst).min(answers.len() - 1)].clone()
}

#[test]
fn the_new_copy_counts_as_started_once_it_holds_the_lock_and_keeps_it() {
    let lock = Path::new("/lock");
    let never = Fake::new(tools(String::new));
    assert_eq!(watch_new_copy(&never, lock), (None, false));
    let kept = Fake::new(tools(lsof_answers(&["", "", "77\n"])));
    assert_eq!(watch_new_copy(&kept, lock), (Some(77), true));
    let lost = Fake::new(tools(lsof_answers(&["77\n", "77\n", ""])));
    assert_eq!(watch_new_copy(&lost, lock), (Some(77), false));
}

#[test]
fn a_finished_update_removes_the_old_bundle_and_records_the_version() {
    let place = Place::new("finish-good");
    let work = work_folder(&place.bundle.parent, 5);
    make_bundle(&work.join("previous"), "old");
    place.files.mark_pending(1);
    let fake = Fake::new(tools(lsof_answers(&["9\n", "", "", "88\n"])));
    assert!(finish_update(&fake, &place.files, &finish_of(&place, &work.join("previous"), false), 9));
    assert!(!work.exists());
    assert_eq!(fake.ran(OPEN_TOOL), [texts(&["-g", &path_text(&place.bundle.path), "--args", "--hidden"])]);
    assert_eq!(place.files.take_result().as_deref(), Some("installed 2.0.0 1.0.0\n"));
    assert_eq!(place.files.pending_since(), None);
    assert!(place.files.failed_versions().is_empty());
}

#[test]
fn a_version_that_does_not_start_is_replaced_by_the_old_one_and_not_installed_again() {
    let place = Place::new("finish-bad");
    fs::write(place.executable(), "new").unwrap();
    let work = work_folder(&place.bundle.parent, 5);
    make_bundle(&work.join("previous"), "old");
    // It takes the lock and loses it: it is stopped with whatever else runs its executable, and the old bundle goes back
    // and starts.
    let fake = Fake::new(tools_running(lsof_answers(&["", "66\n", ""]), || "55\n66\n".to_owned()));
    assert!(!finish_update(&fake, &place.files, &finish_of(&place, &work.join("previous"), false), 9));
    assert_eq!(place.content(), "old");
    assert!(!work.exists());
    assert_eq!(fake.ran(KILL_TOOL), [texts(&["55"]), texts(&["66"])]);
    assert_eq!(fake.ran(LSOF_TOOL).last().unwrap()[2], path_text(&place.executable()));
    assert_eq!(fake.ran(OPEN_TOOL).len(), 2, "the new version, then the old one");
    assert_eq!(place.files.failed_versions(), ["2.0.0"]);
    assert_eq!(place.files.take_result().as_deref(), Some("failed 2.0.0 1.0.0\n"));

    // It never takes the lock, and the old bundle cannot go back: nothing is started again.
    let fake = Fake::new(tools(String::new));
    assert!(!finish_update(&fake, &place.files, &finish_of(&place, &place.root.join("missing/previous"), false), 9));
    assert!(fake.ran(KILL_TOOL).is_empty());
    assert_eq!(fake.ran(OPEN_TOOL).len(), 1);
    assert_eq!(place.content(), "old");
}

#[test]
fn an_old_copy_that_does_not_end_keeps_its_bundle() {
    let place = Place::new("finish-stuck");
    fs::write(place.executable(), "new").unwrap();
    let work = work_folder(&place.bundle.parent, 5);
    make_bundle(&work.join("previous"), "old");
    let fake = Fake::new(tools(|| "9\n".to_owned()));
    assert!(!finish_update(&fake, &place.files, &finish_of(&place, &work.join("previous"), false), 9));
    assert_eq!(place.content(), "old");
    assert!(fake.ran(OPEN_TOOL).is_empty());
    assert!(place.files.failed_versions().is_empty(), "the new version is not to blame");
    assert_eq!(fake.paused.load(Ordering::SeqCst), UPDATE_OLD_COPY_EXIT_TIMEOUT_SECONDS * 1_000);
}

#[test]
fn as_root_the_old_version_comes_back_through_the_password_again() {
    let place = Place::new("finish-root");
    let work = work_folder(&place.bundle.parent, 5);
    make_bundle(&work.join("previous"), "old");
    let fake = Fake::new(tools(String::new));
    assert!(!finish_update(&fake, &place.files, &finish_of(&place, &work.join("previous"), true), 9));
    let call = &fake.ran(OSASCRIPT_TOOL)[0];
    assert!(call[6].ends_with(&format!("'{}' '{REQUIREMENT}' '1.0.0' 'someone' 'discard'", work.join("previous").display())), "{}", call[6]);
    assert!(call[7].starts_with("Версия 2.0.0 не запустилась."));
    assert!(!work.exists());
    assert_eq!(fake.ran(CODESIGN_TOOL), [texts(&["-d", "-r-", "9"])]);
    // A helper without a signature cannot ask root to check the old bundle.
    let unsigned = Fake::new(|program, arguments| if program == CODESIGN_TOOL { answer(false, "") } else { tools(String::new)(program, arguments) });
    make_bundle(&work.join("previous"), "old");
    assert!(!finish_update(&unsigned, &place.files, &finish_of(&place, &work.join("previous"), true), 9));
    assert!(unsigned.ran(OSASCRIPT_TOOL).is_empty() && work.exists());
}

#[test]
fn the_helpers_main_reads_its_command_line() {
    let place = Place::new("finish-main");
    let fake = Fake::new(tools(lsof_answers(&["", "88\n"])));
    assert_eq!(finish_main(&fake, &place.files, &texts(&["bad"]), 1), 2);
    let work = work_folder(&place.bundle.parent, 5);
    make_bundle(&work.join("previous"), "old");
    let arguments = finish_of(&place, &work.join("previous"), false).arguments();
    assert_eq!(finish_main(&fake, &place.files, &arguments[1..], 1), 0);
    let failing = Fake::new(tools(String::new));
    assert_eq!(finish_main(&failing, &place.files, &arguments[1..], 1), 1);
}

// ----------------------------------------------------------------------------------------------------- the installer

#[derive(Default)]
struct Host {
    stopped: AtomicUsize,
    started: AtomicUsize,
    quit: AtomicBool,
    window: bool,
    on_stop: Option<Box<dyn Fn() + Send + Sync>>,
}

impl InstallHost for Host {
    fn stop_engine(&self) {
        self.stopped.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = &self.on_stop {
            hook();
        }
    }

    fn start_engine(&self) {
        self.started.fetch_add(1, Ordering::SeqCst);
    }

    fn window_shown(&self) -> bool {
        self.window
    }

    fn quit(&self) {
        self.quit.store(true, Ordering::SeqCst);
    }
}

fn new_installer(place: &Place, fake: Fake, host: Host) -> (Installer, Arc<Fake>, Arc<Host>) {
    let (fake, host) = (Arc::new(fake), Arc::new(host));
    let installer = Installer {
        commands: fake.clone(),
        host: host.clone(),
        files: place.files.clone(),
        executable: place.executable(),
        process: 7,
        current_version: "1.0.0".to_owned(),
    };
    (installer, fake, host)
}

#[test]
fn an_install_swaps_the_bundle_starts_the_helper_and_quits() {
    let place = Place::new("install");
    let (installer, fake, host) = new_installer(&place, Fake::new(good_tools()), Host { window: true, ..Host::default() });
    installer.install(&place.archive).unwrap();
    assert_eq!(place.content(), "new");
    assert_eq!(host.stopped.load(Ordering::SeqCst), 1);
    assert!(host.quit.load(Ordering::SeqCst));
    assert_eq!(host.started.load(Ordering::SeqCst), 0);
    let previous = work_folder(&place.bundle.parent, 7).join("previous");
    let started = fake.started.lock().unwrap().clone();
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].0, previous.join(BUNDLE_EXECUTABLE));
    let finish = Finish::parse(&started[0].1[1..]).unwrap();
    assert_eq!(finish, Finish { show_window: true, ..finish_of(&place, &previous, false) });
    assert_eq!(place.files.pending_since(), Some(1_000_000));
}

#[test]
fn an_install_that_cannot_begin_leaves_the_program_running() {
    let place = Place::new("install-refused");
    let (refused, _, host) = new_installer(&place, Fake::new(|_, _| answer(false, "")), Host::default());
    assert_eq!(refused.install(&place.archive).unwrap_err().0, UNSIGNED);
    assert_eq!(host.stopped.load(Ordering::SeqCst), 0);
    let (mut elsewhere, _, _) = new_installer(&place, Fake::new(good_tools()), Host::default());
    elsewhere.executable = PathBuf::from("/usr/local/bin/keyswitch");
    assert_eq!(elsewhere.install(&place.archive).unwrap_err().0, NOT_A_BUNDLE);
}

#[test]
fn a_failed_swap_restarts_the_engine() {
    let place = Place::new("install-swap-fails");
    let new_bundle = work_folder(&place.bundle.parent, 7).join("new/KeySwitch.app");
    let host = Host { on_stop: Some(Box::new(move || fs::remove_dir_all(&new_bundle).unwrap())), ..Host::default() };
    let (installer, fake, host) = new_installer(&place, Fake::new(good_tools()), host);
    assert!(installer.install(&place.archive).is_err());
    assert_eq!(place.content(), "old");
    assert_eq!(host.started.load(Ordering::SeqCst), 1);
    assert!(!host.quit.load(Ordering::SeqCst));
    assert!(fake.started.lock().unwrap().is_empty());
    assert!(!work_folder(&place.bundle.parent, 7).exists());
}

#[test]
fn without_rights_the_install_runs_the_root_step_and_the_helper_from_roots_folder() {
    let place = Place::new("install-root");
    mode(&place.bundle.parent, 0o555);
    let work = place.bundle.parent.join(".KeySwitch-update.Xy12Zw");
    let reported = path_text(&work);
    let good = good_tools();
    let fake = Fake::new(move |program, arguments| if program == OSASCRIPT_TOOL { answer(true, &reported) } else { good(program, arguments) });
    let (installer, fake, host) = new_installer(&place, fake, Host::default());
    installer.install(&place.archive).unwrap();
    let call = &fake.ran(OSASCRIPT_TOOL)[0];
    assert!(call[7].starts_with("KeySwitch устанавливает версию 2.0.0."), "{}", call[7]);
    assert!(call[6].contains(&format!("'{}'", work_folder(&place.files.downloads, 7).join("new/KeySwitch.app").display())));
    assert!(!work_folder(&place.files.downloads, 7).exists(), "root made its own copy");
    let started = fake.started.lock().unwrap().clone();
    assert_eq!(started[0].0, work.join("previous").join(BUNDLE_EXECUTABLE));
    assert_eq!(started[0].1.last().map(String::as_str), Some(FINISH_RIGHTS_ADMINISTRATOR));
    assert!(host.quit.load(Ordering::SeqCst));

    let good = good_tools();
    let cancelled = Fake::new(move |program, arguments| {
        if program == OSASCRIPT_TOOL {
            return Ok(Output { success: false, stdout: String::new(), stderr: "User canceled. (-128)".to_owned() });
        }
        good(program, arguments)
    });
    let (installer, _, host) = new_installer(&place, cancelled, Host::default());
    assert_eq!(installer.install(&place.archive).unwrap_err().0, CANCELLED);
    assert_eq!(host.started.load(Ordering::SeqCst), 1);
    assert!(!work_folder(&place.files.downloads, 7).exists());
    mode(&place.bundle.parent, 0o755);
}

#[test]
fn a_helper_that_does_not_start_leaves_the_old_version_running() {
    let place = Place::new("install-no-helper");
    let fake = Fake::new(good_tools()).starting(|_, _| Err(io::Error::other("cannot start")));
    let (installer, _, host) = new_installer(&place, fake, Host::default());
    let error = installer.install(&place.archive).unwrap_err();
    assert_eq!(error.0, "Не удалось запустить помощника обновления: cannot start");
    assert_eq!(place.content(), "old");
    assert_eq!(host.started.load(Ordering::SeqCst), 1);
    assert!(!host.quit.load(Ordering::SeqCst));
    assert_eq!(place.files.pending_since(), None);

    // The old bundle is gone from where it was set aside: the new one stays.
    let previous = work_folder(&place.bundle.parent, 7).join("previous");
    let fake = Fake::new(good_tools()).starting(move |_, _| {
        fs::remove_dir_all(&previous).unwrap();
        Err(io::Error::other("cannot start"))
    });
    let (installer, _, _) = new_installer(&place, fake, Host::default());
    assert!(installer.install(&place.archive).unwrap_err().0.contains("прежняя версия не вернулась"));
    assert_eq!(place.content(), "new");

    // As root, nothing goes back without the password: the new version stays for the next start.
    fs::write(place.executable(), "old").unwrap();
    mode(&place.bundle.parent, 0o555);
    let good = good_tools();
    let reported = path_text(&place.bundle.parent.join(".KeySwitch-update.Xy12Zw"));
    let fake = Fake::new(move |program, arguments| if program == OSASCRIPT_TOOL { answer(true, &reported) } else { good(program, arguments) })
        .starting(|_, _| Err(io::Error::other("cannot start")));
    let (installer, fake, host) = new_installer(&place, fake, Host::default());
    assert_eq!(installer.install(&place.archive).unwrap_err().0, "Не удалось запустить помощника обновления: cannot start");
    assert_eq!(fake.ran(OSASCRIPT_TOOL).len(), 1);
    assert_eq!(host.started.load(Ordering::SeqCst), 1);
    mode(&place.bundle.parent, 0o755);
}

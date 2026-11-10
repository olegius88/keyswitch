//! The update of the macOS build against a fake of the system's tools: `ditto` unpacks a made-up bundle, `codesign`,
//! `PlistBuddy`, `osascript`, `lsof` and `kill` answer as each case needs, and the clocks only count the waits. The swap
//! as root runs in a real `/bin/sh`, with stand-ins for the tools of macOS.
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
    /// The programs run with a clean environment.
    clean: Mutex<Vec<String>>,
    started: Mutex<Vec<(PathBuf, Vec<String>)>>,
    paused: AtomicU64,
}

impl Fake {
    fn new(handler: impl Fn(&str, &[String]) -> io::Result<Output> + Send + Sync + 'static) -> Self {
        Self {
            handler: Box::new(handler),
            starter: Box::new(|_, _| Ok(())),
            calls: Mutex::default(),
            clean: Mutex::default(),
            started: Mutex::default(),
            paused: AtomicU64::new(0),
        }
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

    fn run_clean(&self, program: &str, arguments: &[String]) -> io::Result<Output> {
        self.clean.lock().unwrap().push(program.to_owned());
        self.run(program, arguments)
    }

    fn run_limited(&self, program: &str, arguments: &[String], limit: Duration) -> io::Result<Output> {
        assert_eq!(limit, Duration::from_millis(LSOF_TIME_LIMIT_MS));
        self.run(program, arguments)
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

    fn monotonic(&self) -> Duration {
        Duration::from_millis(self.paused.load(Ordering::SeqCst))
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

/// What `lsof` answers: the processes, nothing (it ends with 1 and says nothing), or a failure (it says why).
#[expect(clippy::unnecessary_wraps, reason = "the answer a fake tool gives")]
fn lsof_answer(answer: Option<&str>) -> io::Result<Output> {
    Ok(match answer {
        Some("") => Output::default(),
        Some(processes) => Output { success: true, stdout: processes.to_owned(), stderr: String::new() },
        None => Output { success: false, stdout: String::new(), stderr: "lsof: WARNING: can't stat()\n".to_owned() },
    })
}

/// The tools of a good update to version 2.0.0. `lock` and `runners` answer for `lsof` on the lock file and on the
/// executable of a bundle, and `alive` for `kill -0`; a signal is always sent.
fn system(
    lock: impl Fn() -> Option<String> + Send + Sync + 'static,
    runners: impl Fn() -> Option<String> + Send + Sync + 'static,
    alive: impl Fn(u32) -> bool + Send + Sync + 'static,
) -> impl Fn(&str, &[String]) -> io::Result<Output> + Send + Sync + 'static {
    move |program: &str, arguments: &[String]| match program {
        CODESIGN_TOOL if matches!(arguments[0].as_str(), "-d") => answer(true, &format!("Executable=/x\ndesignated => {REQUIREMENT}\n")),
        CODESIGN_TOOL => answer(arguments[4] == format!("={REQUIREMENT}"), ""),
        DITTO_TOOL => {
            make_bundle(&Path::new(&arguments[3]).join(ARCHIVE_BUNDLE_NAME), "new");
            answer(true, "")
        }
        PLIST_BUDDY_TOOL => answer(true, "2.0.0\n"),
        LSOF_TOOL if arguments[2].ends_with(BUNDLE_EXECUTABLE) => lsof_answer(runners().as_deref()),
        LSOF_TOOL => lsof_answer(lock().as_deref()),
        KILL_TOOL if matches!(arguments[0].as_str(), "-0") => answer(alive(arguments[1].parse().unwrap()), ""),
        ID_TOOL => answer(true, "someone\n"),
        OPEN_TOOL | KILL_TOOL | OSASCRIPT_TOOL => answer(true, ""),
        program if program.ends_with(BUNDLE_EXECUTABLE) => answer(true, "KeySwitch 2.0.0\n"),
        _ => answer(false, ""),
    }
}

/// No process holds anything, and none runs.
fn good_tools() -> impl Fn(&str, &[String]) -> io::Result<Output> + Send + Sync + 'static {
    system(|| Some(String::new()), || Some(String::new()), |_| false)
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
    let before = SystemCommands.monotonic();
    assert!(SystemCommands.monotonic() >= before);
    // What the password dialog runs gets no variable of ours but a fixed PATH.
    let clean = SystemCommands.run_clean("/usr/bin/env", &[]).unwrap();
    assert_eq!(clean.stdout, format!("PATH={CLEAN_PATH}\n"));
    let quick = SystemCommands.run_limited("/bin/sh", &texts(&["-c", "sleep 0.2; echo done"]), Duration::from_secs(5)).unwrap();
    assert_eq!(quick.stdout, "done\n");
    let slow = SystemCommands.run_limited("/bin/sh", &texts(&["-c", "sleep 5"]), Duration::from_millis(100)).unwrap_err();
    assert_eq!(slow.kind(), io::ErrorKind::TimedOut);
    assert!(SystemCommands.run_limited("/no/such/program", &[], Duration::from_secs(1)).is_err());
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

// ------------------------------------------------------------------------------- the swap as root, in a real shell

/// Stand-ins for the tools of macOS the swap as root runs. Each logs its call and fails when `FAIL` names it (`mv2`: the
/// second `mv`; `codesign-previous`: the check of the old bundle; `chown-user` and `chmod-acl`: what hands the old bundle
/// over); otherwise it does what the tool would do here (`ditto` copies keeping the modes, `chmod` changes them but for
/// the access lists, which only macOS has).
const STAND_IN: &str = r#"#!/bin/sh
name=$(basename "$0")
{ printf '%s' "$name"; for argument in "$@"; do printf ' [%s]' "$argument"; done; echo; } >> "$LOG"
case " $FAIL " in *" $name "*) exit 1 ;; esac
for last in "$@"; do :; done
case "$name" in
  ditto) exec /bin/cp -R -p "$1" "$2" ;;
  stat) echo 0:0 ;;
  chown) case " $FAIL " in *" chown-user "*) [ "$1" = someone ] && exit 1 ;; esac ;;
  chmod)
    case " $* " in
      *" +a "*) case " $FAIL " in *" chmod-acl "*) exit 1 ;; esac; exit 0 ;;
      *" -N "*) exit 0 ;;
    esac
    exec /bin/chmod "$@" ;;
  codesign) case "$last" in *previous) case " $FAIL " in *" codesign-previous "*) exit 1 ;; esac ;; esac ;;
  PlistBuddy) cat "$last" ;;
  mv)
    count=$(( $(cat "$COUNTER" 2>/dev/null || echo 0) + 1 )); echo "$count" > "$COUNTER"
    case " $FAIL " in *" mv$count "*) exit 1 ;; esac
    exec /bin/mv "$@" ;;
esac
exit 0
"#;

/// A place for the swap as root: an installed bundle in a folder whose name holds a space, quotes, `$` and a backtick,
/// a staged new version, and the stand-ins.
struct RootPlace {
    root: PathBuf,
    bundle: PathBuf,
    source: PathBuf,
    tools: RootTools,
}

impl RootPlace {
    fn new(name: &str) -> Self {
        let root = temporary(name);
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        for tool in ["ditto", "chown", "stat", "chmod", "codesign", "PlistBuddy", "mv"] {
            fs::write(bin.join(tool), STAND_IN).unwrap();
            mode(&bin.join(tool), 0o755);
        }
        let stand_in = |tool: &str| path_text(&bin.join(tool));
        let tools = RootTools {
            ditto: stand_in("ditto"),
            chown: stand_in("chown"),
            stat: stand_in("stat"),
            chmod: stand_in("chmod"),
            codesign: stand_in("codesign"),
            plist_buddy: stand_in("PlistBuddy"),
            mv: stand_in("mv"),
            ..RootTools::system()
        };
        let parent = root.join("Prog's $HOME `date` \"x\" dir");
        let bundle = parent.join("KeySwitch.app");
        make_bundle(&bundle, "old");
        let source = root.join("staged $x").join("KeySwitch.app");
        make_bundle(&source, "new");
        fs::write(source.join(BUNDLE_INFO_PLIST), "2.0.0\n").unwrap();
        Self { root, bundle, source, tools }
    }

    /// Run the swap as root would, with `keep` or not, the stand-ins failing as `fail` says.
    fn run(&self, version: &str, keep: bool, fail: &str) -> std::process::Output {
        let _ = fs::remove_file(self.root.join("counter"));
        Command::new("/bin/sh")
            .arg("-c")
            .arg(root_swap_script(&self.tools))
            .arg("keyswitch-update")
            .args([path_text(&self.bundle), path_text(&self.source), REQUIREMENT.to_owned(), version.to_owned(), "someone".to_owned()])
            .arg(if keep { "keep" } else { "discard" })
            .env("LOG", self.root.join("log"))
            .env("COUNTER", self.root.join("counter"))
            .env("FAIL", fail)
            .output()
            .unwrap()
    }

    fn content(&self) -> String {
        fs::read_to_string(self.bundle.join(BUNDLE_EXECUTABLE)).unwrap_or_default()
    }

    /// The work folders the swap left beside the bundle.
    fn work_folders(&self) -> Vec<PathBuf> {
        let parent = self.bundle.parent().unwrap();
        let mut found: Vec<PathBuf> = fs::read_dir(parent)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.file_name().unwrap().to_string_lossy().starts_with(UPDATE_WORK_PREFIX))
            .collect();
        found.sort();
        found
    }

    fn log(&self) -> String {
        fs::read_to_string(self.root.join("log")).unwrap_or_default()
    }
}

impl Drop for RootPlace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn as_root_a_copy_without_the_users_rights_takes_the_place_and_the_old_bundle_is_handed_over() {
    let place = RootPlace::new("root-keep");
    // Rights the user could give the staged copy: a folder anyone may write, an executable that runs as its owner.
    mode(&place.source.join("Contents/MacOS"), 0o777);
    mode(&place.source.join(BUNDLE_EXECUTABLE), 0o4777);
    let output = place.run("2.0.0", true, "");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(place.content(), "new");
    let installed = fs::metadata(place.bundle.join("Contents/MacOS")).unwrap().mode();
    assert_eq!(installed & 0o022, 0, "{installed:o}");
    assert_eq!(fs::metadata(place.bundle.join(BUNDLE_EXECUTABLE)).unwrap().mode() & 0o7000, 0);
    let work = place.work_folders();
    assert_eq!(work.len(), 1);
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), path_text(&work[0]));
    assert_eq!(fs::read_to_string(work[0].join("previous").join(BUNDLE_EXECUTABLE)).unwrap(), "old");
    assert!(!work[0].join("new").exists());
    let log = place.log();
    for call in ["chmod [-R] [-N] [./new]", "chmod [-R] [go-w,a-st] [./new]", "chown [someone] [./previous]", "chown [someone] [.]", "chmod [+a] [user:someone allow delete] [.]"] {
        assert!(log.contains(call), "{call}\n{log}");
    }
    // Only folders of the old bundle go to the user: a file there may be a link to anything.
    assert!(!log.contains("[./previous/Contents/MacOS/KeySwitch]"), "{log}");
    assert!(log.contains(&format!("codesign [--verify] [--strict] [--deep] [-R] [={REQUIREMENT}] [./previous]")), "{log}");
}

#[test]
fn as_root_a_rollback_takes_the_old_bundle_away_with_its_folder() {
    let place = RootPlace::new("root-discard");
    let output = place.run("2.0.0", false, "");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(place.content(), "new");
    assert!(place.work_folders().is_empty());
    assert!(output.stdout.is_empty());
}

#[test]
fn as_root_every_failure_before_the_swap_leaves_the_bundle_and_no_folder() {
    let place = RootPlace::new("root-before");
    for (version, fail) in [("2.0.0", "ditto"), ("2.0.0", "chown"), ("2.0.0", "chmod"), ("2.0.0", "codesign"), ("2.1.0", ""), ("2.0.0", "mv1"), ("2.0.0", "mv2")] {
        let output = place.run(version, true, fail);
        assert!(!output.status.success(), "{fail} {version}");
        assert_eq!(place.content(), "old", "{fail} {version}");
        assert!(place.work_folders().is_empty(), "{fail} {version}");
    }
}

#[test]
fn as_root_the_old_version_is_never_lost() {
    // Neither the new bundle nor the old one goes back into place: the old one stays in the work folder, and the error
    // says where.
    let place = RootPlace::new("root-stuck");
    let output = place.run("2.0.0", true, "mv2 mv3");
    assert!(!output.status.success());
    assert!(!place.bundle.exists());
    let work = place.work_folders();
    assert_eq!(work.len(), 1);
    assert_eq!(fs::read_to_string(work[0].join("previous").join(BUNDLE_EXECUTABLE)).unwrap(), "old");
    assert!(!work[0].join("new").exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains(&format!("the old version stays in {}/previous", work[0].display())));
}

#[test]
fn as_root_nothing_after_the_swap_fails_it() {
    let place = RootPlace::new("root-after");
    let output = place.run("2.0.0", true, "chown-user chmod-acl");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(place.content(), "new");
    let work = place.work_folders();
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), path_text(&work[0]));
    assert!(work[0].join("previous").exists());
}

#[test]
fn as_root_an_old_bundle_without_the_signature_is_removed_not_handed_over() {
    let place = RootPlace::new("root-tampered");
    let output = place.run("2.0.0", true, "codesign-previous");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(place.content(), "new");
    let work = place.work_folders();
    assert!(!work[0].join("previous").exists());
    assert!(!place.log().contains("chown [someone] [./previous]"));
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
    assert!(call[6].contains("/usr/bin/ditto \"$source\" ./new/KeySwitch.app"), "{}", call[6]);
    assert_eq!(*fake.clean.lock().unwrap(), [OSASCRIPT_TOOL]);
    assert_eq!(shell_quoted("it's"), r"'it'\''s'");
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
    files.write_result(RESULT_FAILED, "2.0.0", "1.0.0", None);
    assert_eq!(files.take_result().as_deref(), Some("failed 2.0.0 1.0.0\n"));
    assert_eq!(files.take_result(), None);
    files.write_result(RESULT_STRANDED, "2.0.0", "1.0.0", Some(Path::new("/A b/.w/previous")));
    assert_eq!(files.take_result().as_deref(), Some("stranded 2.0.0 1.0.0 /A b/.w/previous\n"));
    files.mark_pending(5, "1.0.0", "2.0.0");
    assert_eq!(files.pending(), Some((5, "1.0.0".to_owned(), "2.0.0".to_owned())));
    fs::write(files.record(UPDATE_PENDING_FILE), "5 1.0.0\n").unwrap();
    assert_eq!(files.pending(), None);
    fs::write(files.record(UPDATE_PENDING_FILE), "soon 1.0.0 2.0.0\n").unwrap();
    assert_eq!(files.pending(), None);
    files.clear_pending();
    assert_eq!(files.pending(), None);
    files.write_started("2.0.0", 88);
    assert_eq!(files.started("2.0.0"), Some(88));
    assert_eq!(files.started("2.1.0"), None);
    fs::write(files.record(UPDATE_STARTED_FILE), "2.0.0\n").unwrap();
    assert_eq!(files.started("2.0.0"), None);
    files.clear_started();
    assert_eq!(files.started("2.0.0"), None);
    files.strand(&place.bundle.path);
    assert_eq!(files.stranded(), Some(place.bundle.path.clone()));
    files.strand(&place.root.join("gone"));
    assert_eq!(files.stranded(), None);
    assert!(!files.record(UPDATE_STRANDED_FILE).exists(), "a place that is gone is forgotten");
}

#[test]
fn the_notice_names_the_versions_and_where_the_old_one_stays() {
    let (title, message) = result_notice("failed 2.0.0 1.0.0\n").unwrap();
    assert_eq!(title, "Обновление KeySwitch не установлено");
    assert!(message.starts_with("Версия 2.0.0 не запустилась, и вернулась версия 1.0.0."));
    let (_, message) = result_notice("stranded 2.0.0 1.0.0 /A b/.w/previous\n").unwrap();
    assert!(message.contains("вернуть версию 1.0.0 на место не удалось: она осталась в папке «/A b/.w/previous»"), "{message}");
    for record in ["installed 2.0.0 1.0.0", "failed 2.0.0 1.0.0 extra", "stranded 2.0.0 1.0.0", "something else"] {
        assert_eq!(result_notice(record), None, "{record}");
    }
}

#[test]
fn the_new_copy_of_an_update_marks_that_it_runs_and_says_so() {
    let place = Place::new("mark");
    let files = &place.files;
    assert_eq!(mark_started(files, "2.0.0", 88), None);
    files.mark_pending(1, "1.0.0", "2.0.0");
    assert_eq!(mark_started(files, "1.0.0", 77), None, "the old version marks nothing");
    assert_eq!(files.started("1.0.0"), None);
    let (title, message) = mark_started(files, "2.0.0", 88).unwrap();
    assert_eq!((title.as_str(), message.as_str()), ("KeySwitch обновлён", "Установлена версия 2.0.0 вместо 1.0.0."));
    assert_eq!(files.started("2.0.0"), Some(88));
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
    files.mark_pending(1_000, "1.0.0", "2.0.0");
    files.write_started("2.0.0", 88);
    files.write_result(RESULT_FAILED, "2.0.0", "1.0.0", None);
    let notice = startup(files, Some(&place.bundle), 1_000 + UPDATE_PENDING_STALE_SECONDS - 1);
    assert!(notice.unwrap().1.starts_with("Версия 2.0.0 не запустилась"));
    assert!(beside.exists() && staged.exists() && place.archive.exists(), "an update under way keeps its folders");
    assert_eq!(files.started("2.0.0"), Some(88));

    assert_eq!(startup(files, Some(&place.bundle), 1_000 + UPDATE_PENDING_STALE_SECONDS), None);
    assert!(!beside.exists() && !staged.exists() && !place.archive.exists());
    assert!(place.bundle.parent.join("Other.app").exists() && place.bundle.path.exists());
    assert_eq!(files.pending(), None);
    assert_eq!(files.started("2.0.0"), None);
    make_bundle(&staged, "x");
    assert_eq!(startup(files, None, 0), None);
    assert!(!staged.exists());
    let nothing = UpdateFiles::new(&place.root.join("nothing"));
    assert_eq!(startup(&nothing, None, 0), None);
}

#[test]
fn the_start_keeps_an_old_version_that_could_not_go_back() {
    let place = Place::new("startup-stranded");
    let files = &place.files;
    let stranded = work_folder(&place.bundle.parent, 3);
    let other = work_folder(&place.bundle.parent, 4);
    make_bundle(&stranded.join("previous"), "old");
    make_bundle(&other.join("previous"), "old");
    files.strand(&stranded.join("previous"));
    assert_eq!(startup(files, Some(&place.bundle), 0), None);
    assert!(stranded.join("previous").exists(), "the old version that did not go back stays");
    assert!(!other.exists());
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
        parent: 7,
    }
}

#[test]
fn the_helpers_command_line_round_trips() {
    for (show_window, privileged) in [(false, false), (true, true)] {
        let finish = Finish {
            bundle: PathBuf::from("/A/K.app"),
            previous: PathBuf::from("/A/w/previous"),
            old_version: "1.0.0".to_owned(),
            new_version: "2.0.0".to_owned(),
            show_window,
            privileged,
            parent: 42,
        };
        let arguments = finish.arguments();
        assert_eq!(arguments[0], FINISH_UPDATE_ARGUMENT);
        assert_eq!(Finish::parse(&arguments[1..]), Some(finish));
    }
    let good = texts(&["/a", "/b", "1", "2", "hidden", "user", "42"]);
    assert!(Finish::parse(&good).is_some());
    assert_eq!(Finish::parse(&good[..6]), None);
    for (index, word) in [(4, "maybe"), (5, "root"), (6, "me")] {
        let mut changed = good.clone();
        changed[index] = word.to_owned();
        assert_eq!(Finish::parse(&changed), None, "{word}");
    }
    let rootless = Finish { previous: PathBuf::new(), ..Finish::parse(&good).unwrap() };
    assert_eq!(rootless.work(), PathBuf::new());
}

#[test]
fn waiting_looks_until_the_condition_holds_or_the_monotonic_time_is_up() {
    let fake = Fake::new(good_tools());
    assert!(wait_for(&fake, Duration::from_secs(1), || true));
    assert_eq!(fake.paused.load(Ordering::SeqCst), 0);
    assert!(!wait_for(&fake, Duration::from_secs(1), || false));
    assert_eq!(fake.paused.load(Ordering::SeqCst), 1_000);
}

#[test]
fn lsof_tells_who_holds_a_file_or_that_it_cannot_tell() {
    let with = |output: Output| Fake::new(move |_, _| Ok(output.clone()));
    let some = |stdout: &str| Output { success: true, stdout: stdout.to_owned(), stderr: String::new() };
    assert_eq!(processes_with(&with(some("12\nnot a number\n 34 \n")), Path::new("/lock")), Some(vec![12, 34]));
    assert_eq!(processes_with(&with(Output::default()), Path::new("/lock")), Some(Vec::new()));
    assert_eq!(processes_with(&with(Output { success: true, ..Output::default() }), Path::new("/lock")), Some(Vec::new()));
    let warned = Output { success: false, stdout: "12\n".to_owned(), stderr: "lsof: WARNING\n".to_owned() };
    assert_eq!(processes_with(&with(warned), Path::new("/lock")), Some(vec![12]));
    let failed = Output { success: false, stdout: String::new(), stderr: "lsof: WARNING\n".to_owned() };
    assert_eq!(processes_with(&with(failed), Path::new("/lock")), None);
    let broken = Fake::new(|_, _| Err(io::Error::from(io::ErrorKind::TimedOut)));
    assert_eq!(processes_with(&broken, Path::new("/lock")), None);
    assert_eq!(broken.ran(LSOF_TOOL), [texts(&["-t", "--", "/lock"])]);
    // A process `kill` cannot ask about counts as running.
    assert!(alive(&broken, 5));
}

#[test]
fn the_program_starts_hidden_unless_its_window_was_open_and_a_failed_start_is_tried_again() {
    for show_window in [false, true] {
        let fake = Fake::new(good_tools());
        assert!(open_program(&fake, Path::new("/A/K.app"), show_window));
        let expected = if show_window { texts(&["/A/K.app"]) } else { texts(&["-g", "/A/K.app", "--args", "--hidden"]) };
        assert_eq!(fake.ran(OPEN_TOOL), [expected]);
    }
    let calls = AtomicUsize::new(0);
    let second = Fake::new(move |_, _| answer(calls.fetch_add(1, Ordering::SeqCst) > 0, ""));
    assert!(open_program(&second, Path::new("/A/K.app"), false));
    assert_eq!(second.ran(OPEN_TOOL).len(), 2);
    let never = Fake::new(|_, _| Err(io::Error::other("no open")));
    assert!(!open_program(&never, Path::new("/A/K.app"), false));
    assert_eq!(never.ran(OPEN_TOOL).len(), usize::try_from(OPEN_ATTEMPTS).unwrap());
}

/// Answers from a list, one a call, the last one ever after; `None` is a failure of `lsof`.
fn answers(list: &[Option<&str>]) -> impl Fn() -> Option<String> + Send + Sync + 'static {
    let list: Vec<Option<String>> = list.iter().map(|answer| answer.map(str::to_owned)).collect();
    let calls = AtomicUsize::new(0);
    move || list[calls.fetch_add(1, Ordering::SeqCst).min(list.len() - 1)].clone()
}

/// `kill -0` answers yes for `process` its first `times` times, and no after.
fn alive_for(process: u32, times: usize) -> impl Fn(u32) -> bool + Send + Sync + 'static {
    let calls = AtomicUsize::new(0);
    move |asked| asked == process && calls.fetch_add(1, Ordering::SeqCst) < times
}

#[test]
fn the_new_copy_is_known_by_its_mark_or_by_the_lock_held_by_its_executable() {
    let place = Place::new("new-copy");
    let finish = finish_of(&place, &place.root.join("w/previous"), false);
    let files = &place.files;
    // Its own mark.
    files.write_started("2.0.0", 88);
    let marked = Fake::new(system(|| None, || None, |_| true));
    assert_eq!(watch_new_copy(&marked, files, &finish), (Some(88), true));
    files.clear_started();
    // A version from before the mark: the lock, held by a process of the new executable.
    let held = Fake::new(system(answers(&[Some(""), Some("88\n")]), || Some("88\n".to_owned()), |_| true));
    assert_eq!(watch_new_copy(&held, files, &finish), (Some(88), true));
    // Another program holds the lock, or lsof cannot tell: no new copy.
    for (lock, runners) in [(Some("77\n"), Some("88\n")), (None, Some("88\n")), (Some("88\n"), None)] {
        let other = Fake::new(system(move || lock.map(str::to_owned), move || runners.map(str::to_owned), |_| true));
        assert_eq!(watch_new_copy(&other, files, &finish), (None, false), "{lock:?} {runners:?}");
    }
    // It says it runs and ends within the settle time.
    files.write_started("2.0.0", 88);
    let ended = Fake::new(system(|| None, || None, alive_for(88, 3)));
    assert_eq!(watch_new_copy(&ended, files, &finish), (Some(88), false));
}

#[test]
fn stopping_ends_the_processes_and_kills_those_that_stay() {
    let quiet = Fake::new(good_tools());
    stop_processes(&quiet, &[]);
    assert!(quiet.ran(KILL_TOOL).is_empty());
    let polite = Fake::new(system(|| None, || None, alive_for(5, 1)));
    stop_processes(&polite, &[5]);
    assert_eq!(polite.ran(KILL_TOOL), [texts(&["-TERM", "5"]), texts(&["-0", "5"]), texts(&["-0", "5"])]);
    let stubborn = Fake::new(system(|| None, || None, |_| true));
    stop_processes(&stubborn, &[5]);
    let signals: Vec<String> = stubborn.ran(KILL_TOOL).into_iter().map(|call| call[0].clone()).filter(|signal| signal != "-0").collect();
    assert_eq!(signals, ["-TERM", "-KILL"]);
}

#[test]
fn a_finished_update_removes_the_old_bundle() {
    let place = Place::new("finish-good");
    let work = work_folder(&place.bundle.parent, 5);
    make_bundle(&work.join("previous"), "old");
    place.files.mark_pending(1, "1.0.0", "2.0.0");
    let files = place.files.clone();
    let tools = system(|| None, || None, |process| process == 88);
    // The old copy ends; the new one, started, marks that it runs.
    let fake = Fake::new(move |program, arguments| {
        if program == OPEN_TOOL {
            files.write_started("2.0.0", 88);
        }
        tools(program, arguments)
    });
    assert!(finish_update(&fake, &place.files, &finish_of(&place, &work.join("previous"), false), 9));
    assert!(!work.exists());
    assert_eq!(fake.ran(OPEN_TOOL), [texts(&["-g", &path_text(&place.bundle.path), "--args", "--hidden"])]);
    assert_eq!(place.files.take_result(), None, "the new copy has said it itself");
    assert_eq!(place.files.pending(), None);
    assert_eq!(place.files.started("2.0.0"), None);
    assert!(place.files.failed_versions().is_empty());
    assert_eq!(fake.ran(KILL_TOOL)[0], texts(&["-0", "7"]), "the end of the old copy is waited for first");
}

#[test]
fn a_version_that_does_not_start_is_replaced_by_the_old_one_and_not_installed_again() {
    let place = Place::new("finish-bad");
    fs::write(place.executable(), "new").unwrap();
    let work = work_folder(&place.bundle.parent, 5);
    make_bundle(&work.join("previous"), "old");
    // It holds the lock and then ends: it and whatever runs its executable are stopped, the old bundle goes back, the
    // outcome is written, and only then the old version starts.
    let files = place.files.clone();
    let tools = system(answers(&[Some("66\n")]), || Some("55\n66\n".to_owned()), alive_for(66, 2));
    let fake = Fake::new(move |program, arguments| {
        if program == OPEN_TOOL && !files.failed_versions().is_empty() {
            assert!(files.take_result().unwrap().starts_with("failed 2.0.0 1.0.0"), "the outcome comes before the start");
        }
        tools(program, arguments)
    });
    assert!(!finish_update(&fake, &place.files, &finish_of(&place, &work.join("previous"), false), 9));
    assert_eq!(place.content(), "old");
    assert!(!work.exists());
    let signals: Vec<Vec<String>> = fake.ran(KILL_TOOL).into_iter().filter(|call| call[0] == "-TERM").collect();
    assert_eq!(signals, [texts(&["-TERM", "55"]), texts(&["-TERM", "66"])]);
    assert_eq!(fake.ran(OPEN_TOOL).len(), 2, "the new version, then the old one");
    assert_eq!(place.files.failed_versions(), ["2.0.0"]);
    assert_eq!(place.files.stranded(), None);
}

#[test]
fn an_old_version_that_cannot_go_back_is_kept_and_named() {
    let place = Place::new("finish-stranded");
    fs::write(place.executable(), "new").unwrap();
    // It never says it runs, nothing can be started, and the old bundle cannot go back where it was.
    let previous = place.root.join("missing/previous");
    let fake = Fake::new(system(|| Some(String::new()), || Some(String::new()), |_| false));
    let failing_open = Fake::new(move |program, arguments| if program == OPEN_TOOL { answer(false, "") } else { fake.run(program, arguments) });
    assert!(!finish_update(&failing_open, &place.files, &finish_of(&place, &previous, false), 9));
    assert_eq!(failing_open.ran(OPEN_TOOL).len(), usize::try_from(OPEN_ATTEMPTS).unwrap(), "the old version is not started");
    assert_eq!(place.content(), "new");
    let record = place.files.take_result().unwrap();
    assert_eq!(record, format!("stranded 2.0.0 1.0.0 {}\n", previous.display()));
    // Kept by the record while the place exists; forgotten when it is gone.
    assert_eq!(place.files.stranded(), None);
    assert_eq!(place.files.failed_versions(), ["2.0.0"]);
}

#[test]
fn an_old_copy_that_does_not_end_keeps_its_bundle() {
    let place = Place::new("finish-stuck");
    fs::write(place.executable(), "new").unwrap();
    let work = work_folder(&place.bundle.parent, 5);
    make_bundle(&work.join("previous"), "old");
    let fake = Fake::new(system(|| None, || None, |process| process == 7));
    assert!(!finish_update(&fake, &place.files, &finish_of(&place, &work.join("previous"), false), 9));
    assert_eq!(place.content(), "old");
    assert!(fake.ran(OPEN_TOOL).is_empty());
    assert!(place.files.failed_versions().is_empty(), "the new version is not to blame");
    assert!(fake.paused.load(Ordering::SeqCst) >= UPDATE_OLD_COPY_EXIT_TIMEOUT_SECONDS * 1_000);
    // And when its bundle cannot go back either, the old version is kept where it is.
    let gone = place.root.join("gone/previous");
    assert!(!finish_update(&fake, &place.files, &finish_of(&place, &gone, false), 9));
    assert_eq!(fs::read_to_string(place.files.record(UPDATE_STRANDED_FILE)).unwrap(), format!("{}\n", gone.display()));
}

#[test]
fn as_root_the_old_version_comes_back_through_the_password_again() {
    let place = Place::new("finish-root");
    let work = work_folder(&place.bundle.parent, 5);
    make_bundle(&work.join("previous"), "old");
    let fake = Fake::new(good_tools());
    assert!(!finish_update(&fake, &place.files, &finish_of(&place, &work.join("previous"), true), 9));
    let call = &fake.ran(OSASCRIPT_TOOL)[0];
    assert!(call[6].ends_with(&format!("'{}' '{REQUIREMENT}' '1.0.0' 'someone' 'discard'", work.join("previous").display())), "{}", call[6]);
    assert!(call[7].starts_with("Версия 2.0.0 не запустилась."));
    assert!(!work.exists());
    assert_eq!(fake.ran(CODESIGN_TOOL), [texts(&["-d", "-r-", "9"])]);
    assert_eq!(*fake.clean.lock().unwrap(), [OSASCRIPT_TOOL]);
    // A helper without a signature cannot ask root to check the old bundle: it stays, named.
    let unsigned = Fake::new(|program, arguments| if program == CODESIGN_TOOL { answer(false, "") } else { good_tools()(program, arguments) });
    make_bundle(&work.join("previous"), "old");
    assert!(!finish_update(&unsigned, &place.files, &finish_of(&place, &work.join("previous"), true), 9));
    assert!(unsigned.ran(OSASCRIPT_TOOL).is_empty() && work.exists());
    assert_eq!(place.files.stranded(), Some(work.join("previous")));
}

#[test]
fn the_helpers_main_reads_its_command_line() {
    let place = Place::new("finish-main");
    let fake = Fake::new(system(answers(&[Some("88\n")]), || Some("88\n".to_owned()), |process| process == 88));
    assert_eq!(finish_main(&fake, &place.files, &texts(&["bad"]), 1), 2);
    let work = work_folder(&place.bundle.parent, 5);
    make_bundle(&work.join("previous"), "old");
    let arguments = finish_of(&place, &work.join("previous"), false).arguments();
    assert_eq!(finish_main(&fake, &place.files, &arguments[1..], 1), 0);
    let failing = Fake::new(good_tools());
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
    assert_eq!(place.files.pending(), Some((1_000_000, "1.0.0".to_owned(), "2.0.0".to_owned())));
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
    assert_eq!(started[0].1[6], FINISH_RIGHTS_ADMINISTRATOR);
    assert_eq!(started[0].1[7], "7", "the helper waits for the end of this process");
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
    assert_eq!(place.files.pending(), None);

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

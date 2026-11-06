//! One command that turns a prepared working tree into a published release: the version into every file that spells it,
//! the changelog section closed, the checks, the commit, the tag and the push, the release workflow waited for, the
//! published files checked. It writes no prose: the changelog's Unreleased entries and `RELEASE_NOTES.md` are the author's,
//! and it refuses to run without them.
//!
//!     ks-release --version X.Y.Z [--candidate N] [--dry-run] [--skip-checks] [--skip-ci] [--remote origin] [--branch main]
//!                [--field-root <tree>] [--field-released <folder>] [--message-file <file>]
//!
//! With --candidate the tag is vX.Y.Z-rcN and the release a pre-release: the update check of an installed program does not
//! see it, and the APT repository stays as it is. Without it the tag replaces the published version for everybody.
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
use ks_cast::To as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const UNRELEASED: &str = "## Unreleased";
const WORKFLOW: &str = "CI and release";
const CI_TIMEOUT_SECONDS: u64 = 3 * 60 * 60;
const CI_POLL_SECONDS: u64 = 60;
const SECONDS_PER_DAY: i64 = 86_400;
const ASSETS: [&str; 9] = [
    "KeySwitch-Setup-{version}-x64.exe",
    "KeySwitch-Setup-{version}-arm64.exe",
    "KeySwitch-Setup-{version}-x86.exe",
    "KeySwitch-{version}-windows-x64.zip",
    "KeySwitch-{version}-windows-arm64.zip",
    "KeySwitch-{version}-windows-x86.zip",
    "keyswitch_{version}_amd64.deb",
    "KeySwitch-{version}-macos-arm64.zip",
    "KeySwitch-{version}-macos-x86_64.zip",
];
const TESTED_CRATES: [&str; 8] = ["ks-text", "ks-lang", "ks-context", "ks-engine", "ks-config", "ks-replay", "ks-train", "logcourier"];

#[derive(Default)]
struct Options {
    version: String,
    candidate: Option<u32>,
    dry_run: bool,
    skip_checks: bool,
    skip_ci: bool,
    remote: String,
    branch: String,
    field_root: Option<PathBuf>,
    field_released: Option<PathBuf>,
    message_file: Option<PathBuf>,
    poll_seconds: u64,
    timeout_seconds: u64,
}

fn fail(message: impl Into<String>) -> String {
    message.into()
}

fn output(root: &Path, program: &str, arguments: &[&str]) -> Result<String, String> {
    let done = Command::new(program).current_dir(root).args(arguments).output().map_err(|error| format!("{program}: {error}"))?;
    if !done.status.success() {
        return Err(format!("{program} {}: {}", arguments.join(" "), String::from_utf8_lossy(&done.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&done.stdout).trim().to_owned())
}

fn run(root: &Path, label: &str, program: &str, arguments: &[&str]) -> Result<(), String> {
    println!("    $ {label}");
    let status = Command::new(program).current_dir(root).args(arguments).status().map_err(|error| format!("{program}: {error}"))?;
    if status.success() { Ok(()) } else { Err(format!("{label} failed ({status})")) }
}

/// The files that spell the version, each by the pattern that finds the version it holds: the program's own in
/// `Cargo.toml`, the last published one in the man page and the READMEs.
const VERSION_SITES: [(&str, &str, bool); 4] = [
    ("Cargo.toml", r#"(?m)^version = "([0-9]+\.[0-9]+\.[0-9]+)"$"#, false),
    ("packaging/keyswitch.1", r#""KeySwitch ([0-9]+\.[0-9]+\.[0-9]+)""#, false),
    ("README.md", r"releases/tag/v([0-9]+\.[0-9]+\.[0-9]+)", true),
    ("README.en.md", r"releases/tag/v([0-9]+\.[0-9]+\.[0-9]+)", true),
];

fn apply_version(root: &Path, options: &Options) -> Result<(), String> {
    for (file, pattern, everywhere) in VERSION_SITES {
        let path = root.join(file);
        let text = std::fs::read_to_string(&path).map_err(|error| format!("{file}: {error}"))?;
        let expression = regex::Regex::new(pattern).expect("a valid pattern");
        let Some(previous) = expression.captures(&text).map(|found| found[1].to_owned()) else {
            return Err(format!("{file} does not mention a version in the expected form ({pattern})"));
        };
        if previous == options.version {
            println!("    {file} already declares {previous}");
            continue;
        }
        // A README names the version in the files it lists too; elsewhere only the match itself is the version.
        let updated = if everywhere { text.replace(&previous, &options.version) } else { expression.replace(&text, |found: &regex::Captures<'_>| found[0].replace(&previous, &options.version)).into_owned() };
        println!("    {} {previous} in {file}", if options.dry_run { "would rewrite" } else { "rewrote" });
        if !options.dry_run {
            std::fs::write(&path, updated).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// `changelog_sections`: the entries under each `## ` heading, in order.
fn changelog_sections(text: &str) -> Vec<(String, Vec<String>)> {
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            sections.push((heading.to_owned(), Vec::new()));
        } else if let Some(section) = sections.last_mut()
            && !line.trim().is_empty()
        {
            section.1.push(line.to_owned());
        }
    }
    sections
}

/// Today in the UTC calendar: days since the epoch to a date (Howard Hinnant's `civil_from_days`).
fn today() -> String {
    let seconds = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())).to::<i64>();
    let days = seconds / SECONDS_PER_DAY + 719_468;
    let era = days.div_euclid(146_097);
    let of_era = days.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * of_year + 2) / 153;
    let day = of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 { shifted + 3 } else { shifted - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn close_changelog(root: &Path, options: &Options) -> Result<(), String> {
    let path = root.join("CHANGELOG.md");
    let text = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let sections = changelog_sections(&text);
    if sections.iter().any(|(heading, _)| heading != "Unreleased" && heading.split(' ').next() == Some(options.version.as_str())) {
        println!("    CHANGELOG.md already has a section for {}", options.version);
        return Ok(());
    }
    let entries = sections.iter().find(|(heading, _)| heading == "Unreleased").map_or(0, |(_, entries)| entries.len());
    if entries == 0 {
        return Err(format!("CHANGELOG.md has neither a section for {} nor entries under {UNRELEASED}; describe the release there first", options.version));
    }
    let heading = format!("## {} — {}", options.version, today());
    if options.dry_run {
        println!("    would move {entries} Unreleased entries under {heading}");
        return Ok(());
    }
    std::fs::write(&path, text.replacen(&format!("{UNRELEASED}\n"), &format!("{UNRELEASED}\n\n{heading}\n"), 1)).map_err(|error| error.to_string())?;
    println!("    moved {entries} Unreleased entries under {heading}");
    Ok(())
}

/// The numbers of the lines that continue the paragraph or list item above them; code blocks are left alone.
fn wrapped_lines(text: &str) -> Vec<usize> {
    let block_start = |line: &str| {
        let trimmed = line.trim_start();
        trimmed.starts_with(['#', '|', '<', '>', '-', '*', '+']) || trimmed.split_once(". ").is_some_and(|(number, _)| !number.is_empty() && number.chars().all(|character| character.is_ascii_digit()))
    };
    let (mut wrapped, mut fenced, mut continued) = (Vec::new(), false, false);
    for (index, line) in text.split('\n').enumerate() {
        if line.starts_with("```") {
            fenced = !fenced;
            continued = false;
        } else if fenced || line.trim().is_empty() {
            continued = false;
        } else if continued && !block_start(line) {
            wrapped.push(index + 1);
        } else {
            continued = !line.starts_with(['#', '|', '<']);
        }
    }
    wrapped
}

fn release_tag(options: &Options) -> String {
    options.candidate.map_or_else(|| format!("v{}", options.version), |candidate| format!("v{}-rc{candidate}", options.version))
}

fn assets(version: &str) -> Vec<String> {
    ASSETS.iter().map(|asset| asset.replace("{version}", version)).collect()
}

fn check_release_notes(root: &Path, version: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(root.join("RELEASE_NOTES.md")).map_err(|error| error.to_string())?;
    let found = text.strip_prefix("# KeySwitch ").and_then(|rest| rest.split_whitespace().next());
    if found != Some(version) {
        return Err(format!("RELEASE_NOTES.md describes {}, not {version}; write the notes for this release first", found.unwrap_or("nothing")));
    }
    let missing: Vec<String> = assets(version).into_iter().filter(|asset| !text.contains(asset.as_str())).collect();
    if !missing.is_empty() {
        return Err(format!("RELEASE_NOTES.md does not name the published files: {}", missing.join(", ")));
    }
    let wrapped = wrapped_lines(&text);
    if !wrapped.is_empty() {
        return Err(format!("RELEASE_NOTES.md continues a paragraph or list item on lines {wrapped:?}; GitHub and the update notice show every line break, so write each paragraph and item on one line"));
    }
    println!("    the release notes describe this version");
    Ok(())
}

/// Every check a release holds to, stopping at the first that does not.
fn run_checks(root: &Path, options: &Options) -> Result<(), String> {
    let tree = root.to_string_lossy().into_owned();
    let mut arguments = vec!["test", "--profile", "parity"];
    for name in TESTED_CRATES {
        arguments.extend(["-p", name]);
    }
    run(root, "the tests of the engine, the trainer and LogCourier", "cargo", &arguments)?;
    run(root, "the trainer", "cargo", &["build", "--profile", "train", "-p", "ks-train"])?;
    let trainer = root.join("target/train/ks-train");
    let trainer = trainer.to_string_lossy().into_owned();
    run(root, "the evidence of every installed and rejected model", &trainer, &["gates", "--tree", &tree])?;
    run(root, "the boundary model reproduces", &trainer, &["boundary", "--tree", &tree, "--verify"])?;
    run(root, "the orthotactic model reproduces", &trainer, &["ortho", "--tree", &tree, "--verify"])?;
    run(root, "the inputs of the orthotactic model reproduce", &trainer, &["ortho", "--tree", &tree, "--inputs"])?;
    run(root, "the layout-intent model reproduces", &trainer, &["intent", "--tree", &tree, "--verify"])?;
    run(root, "the strict evaluation of the layout-intent model", &trainer, &["intent", "--tree", &tree, "--evaluate", "--strict", "--output", "/dev/null"])?;
    run(root, "the files of retired families", &trainer, &["archive", "--tree", &tree])?;
    for family in ["boundary-v1", "context-v2", "ortho-v2", "prefix-v1"] {
        run(root, "a rejected or retired model replays", &trainer, &["retired", family, "--tree", &tree, "--replay"])?;
    }
    run(root, "the boundary regressions", "dbus-run-session", &["--", &trainer, "engine", "boundary", "--tree", &tree, "--refresh-runtime"])?;
    run(root, "the prefix replay", "dbus-run-session", &["--", &trainer, "engine", "prefix", "--tree", &tree, "--refresh-runtime"])?;
    if let (Some(field_root), Some(released)) = (&options.field_root, &options.field_released) {
        let (field_root, released) = (field_root.to_string_lossy().into_owned(), released.to_string_lossy().into_owned());
        run(root, "the owner's typing against the last release", "cargo", &["run", "--profile", "parity", "--bin", "field-gate", "--", "--root", &field_root, "--released", &released])?;
    } else {
        return Err(fail("--field-root and --field-released name the owner's typing and the last release's reports; a release does not skip them"));
    }
    run(root, "the Debian package", "bash", &["packaging/build-rust-deb.sh"])?;
    let architecture = output(root, "dpkg", &["--print-architecture"])?;
    let package = format!("dist/keyswitch_{}_{architecture}.deb", options.version);
    run(root, "what the package holds", &trainer, &["deb-check", "--tree", &tree, &package])?;
    run(root, "the real tests in a private X server, and the package", "bash", &["packaging/e2e.sh", &package])
}

fn commit_tag_push(root: &Path, options: &Options) -> Result<String, String> {
    let tag = release_tag(options);
    if !output(root, "git", &["status", "--porcelain"])?.is_empty() {
        let message = if let Some(path) = &options.message_file { std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))? } else {
            let text = std::fs::read_to_string(root.join("CHANGELOG.md")).map_err(|error| error.to_string())?;
            let sections = changelog_sections(&text);
            let entries = sections.iter().find(|(heading, _)| heading.split(' ').next() == Some(options.version.as_str())).map(|(_, entries)| entries.join("\n")).filter(|entries| !entries.is_empty());
            format!("Release KeySwitch {}\n\n{}\n", options.version, entries.ok_or_else(|| format!("CHANGELOG.md has no entries under {}; nothing to describe", options.version))?)
        };
        output(root, "git", &["add", "-A"])?;
        output(root, "git", &["commit", "-m", &message])?;
        println!("    committed {}", output(root, "git", &["rev-parse", "--short", "HEAD"])?);
    }
    if output(root, "git", &["tag", "--list", &tag])?.is_empty() {
        output(root, "git", &["tag", "-a", &tag, "-m", &format!("KeySwitch {}", options.version)])?;
        println!("    created {tag}");
    }
    output(root, "git", &["push", &options.remote, &options.branch, "--follow-tags"])?;
    println!("    pushed {} and {tag} to {}", options.branch, options.remote);
    Ok(tag)
}

/// The workflow run of this tag, by polling: `completed` with `success`, or the reason it is not.
fn wait_for_workflow(root: &Path, tag: &str, options: &Options) -> Result<(), String> {
    let started = std::time::Instant::now();
    loop {
        let listed = output(root, "gh", &["run", "list", "--workflow", WORKFLOW, "--branch", tag, "--limit", "1", "--json", "status,conclusion,url"])?;
        let runs: serde_json::Value = serde_json::from_str(&listed).map_err(|error| error.to_string())?;
        if let Some(run) = runs.as_array().and_then(|runs| runs.first())
            && run["status"] == "completed"
        {
            if run["conclusion"] == "success" {
                println!("    {}", run["url"].as_str().unwrap_or(""));
                return Ok(());
            }
            return Err(format!("the release workflow ended with {}: {}", run["conclusion"], run["url"]));
        }
        if started.elapsed().as_secs() >= options.timeout_seconds {
            return Err(format!("the release workflow of {tag} did not finish in time"));
        }
        std::thread::sleep(std::time::Duration::from_secs(options.poll_seconds));
    }
}

fn verify_published(root: &Path, options: &Options, tag: &str) -> Result<String, String> {
    let release: serde_json::Value = serde_json::from_str(&output(root, "gh", &["release", "view", tag, "--json", "url,assets"])?).map_err(|error| error.to_string())?;
    let published: Vec<&str> = release["assets"].as_array().map_or_else(Vec::new, |assets| assets.iter().filter_map(|asset| asset["name"].as_str()).collect());
    let missing: Vec<String> = assets(&options.version).into_iter().filter(|asset| !published.contains(&asset.as_str())).collect();
    if !missing.is_empty() {
        return Err(format!("the release {tag} is missing: {}", missing.join(", ")));
    }
    Ok(release["url"].as_str().unwrap_or("").to_owned())
}

fn check_preconditions(root: &Path, options: &Options) -> Result<(), String> {
    let parts: Vec<&str> = options.version.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty() || !part.chars().all(|character| character.is_ascii_digit())) {
        return Err(format!("{} is no version of the form X.Y.Z", options.version));
    }
    let branch = output(root, "git", &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if branch != options.branch {
        return Err(format!("the tree is on {branch}, not on {}", options.branch));
    }
    let tag = release_tag(options);
    if !output(root, "git", &["ls-remote", "--tags", &options.remote, &tag])?.is_empty() && !output(root, "git", &["status", "--porcelain"])?.is_empty() {
        return Err(format!("{tag} is already published and the tree has changes; choose another version"));
    }
    Ok(())
}

fn release(root: &Path, options: &Options) -> Result<(), String> {
    println!("[1] checking the tree for KeySwitch {}", options.version);
    check_preconditions(root, options)?;
    println!("[2] propagating the version");
    apply_version(root, options)?;
    println!("[3] closing the changelog section");
    close_changelog(root, options)?;
    println!("[4] checking the release notes");
    check_release_notes(root, &options.version)?;
    if options.dry_run {
        println!("    dry run: nothing was written, stopping before the checks");
        return Ok(());
    }
    if options.skip_checks {
        println!("[5] skipping the checks (--skip-checks)");
    } else {
        println!("[5] running the checks");
        run_checks(root, options)?;
    }
    println!("[6] committing, tagging and pushing");
    let tag = commit_tag_push(root, options)?;
    if options.skip_ci {
        println!("    skipping the release workflow (--skip-ci)");
        return Ok(());
    }
    println!("[7] waiting for the release workflow");
    wait_for_workflow(root, &tag, options)?;
    println!("[8] verifying the published release");
    let url = verify_published(root, options, &tag)?;
    println!("\nKeySwitch {tag} is released: {url}");
    Ok(())
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| arguments.iter().position(|argument| argument == name).and_then(|place| arguments.get(place + 1)).cloned();
    let flag = |name: &str| arguments.iter().any(|argument| argument == name);
    let Some(version) = value("--version") else {
        eprintln!("usage: ks-release --version X.Y.Z [--candidate N] [--dry-run] [--skip-checks] [--skip-ci] [--remote origin] [--branch main] [--field-root <tree>] [--field-released <folder>] [--message-file <file>]");
        std::process::exit(2);
    };
    let candidate = match value("--candidate").map(|candidate| candidate.parse::<u32>()).transpose() {
        Ok(candidate) => candidate,
        Err(error) => {
            eprintln!("--candidate takes a number: {error}");
            std::process::exit(2);
        }
    };
    let options = Options {
        version,
        candidate,
        dry_run: flag("--dry-run"),
        skip_checks: flag("--skip-checks"),
        skip_ci: flag("--skip-ci"),
        remote: value("--remote").unwrap_or_else(|| "origin".to_owned()),
        branch: value("--branch").unwrap_or_else(|| "main".to_owned()),
        field_root: value("--field-root").map(PathBuf::from),
        field_released: value("--field-released").map(PathBuf::from),
        message_file: value("--message-file").map(PathBuf::from),
        poll_seconds: value("--poll-seconds").and_then(|seconds| seconds.parse().ok()).unwrap_or(CI_POLL_SECONDS),
        timeout_seconds: value("--timeout-seconds").and_then(|seconds| seconds.parse().ok()).unwrap_or(CI_TIMEOUT_SECONDS),
    };
    let root = match output(Path::new("."), "git", &["rev-parse", "--show-toplevel"]) {
        Ok(root) => PathBuf::from(root),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = release(&root, &options) {
        eprintln!("\nrelease stopped: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    fn options(version: &str, dry_run: bool) -> Options {
        Options { version: version.to_owned(), dry_run, ..Options::default() }
    }

    struct Tree(PathBuf);

    impl Tree {
        fn new(label: &str) -> Tree {
            let path = std::env::temp_dir().join(format!("keyswitch-release-test-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("packaging")).unwrap();
            Tree(path)
        }

        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.0.join(name), text).unwrap();
        }

        fn read(&self, name: &str) -> String {
            std::fs::read_to_string(self.0.join(name)).unwrap()
        }

        fn with_version_sites(self) -> Tree {
            self.write("Cargo.toml", "[workspace.package]\nversion = \"1.2.3\"\nrust-version = \"1.2.3\"\n");
            self.write("packaging/keyswitch.1", ".TH KEYSWITCH 1 \"2026\" \"KeySwitch 1.2.3\"\n");
            self.write("README.md", "[скачать](https://example.org/releases/tag/v1.2.3) KeySwitch-Setup-1.2.3.exe\n");
            self.write("README.en.md", "[download](https://example.org/releases/tag/v1.2.3) KeySwitch-Setup-1.2.3.exe\n");
            self
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_site_is_rewritten_in_its_own_way() {
        let tree = Tree::new("sites").with_version_sites();
        apply_version(&tree.0, &options("1.3.0", false)).unwrap();
        // In Cargo.toml only the match is the version; a README names it in the files it lists too.
        assert_eq!(tree.read("Cargo.toml"), "[workspace.package]\nversion = \"1.3.0\"\nrust-version = \"1.2.3\"\n");
        assert!(tree.read("packaging/keyswitch.1").contains("KeySwitch 1.3.0"));
        assert_eq!(tree.read("README.md"), "[скачать](https://example.org/releases/tag/v1.3.0) KeySwitch-Setup-1.3.0.exe\n");
        assert_eq!(tree.read("README.en.md"), "[download](https://example.org/releases/tag/v1.3.0) KeySwitch-Setup-1.3.0.exe\n");
    }

    #[test]
    fn a_dry_run_writes_nothing() {
        let tree = Tree::new("dry").with_version_sites();
        let before = tree.read("Cargo.toml");
        apply_version(&tree.0, &options("1.3.0", true)).unwrap();
        assert_eq!(tree.read("Cargo.toml"), before);
    }

    #[test]
    fn a_site_without_a_version_is_an_error() {
        let tree = Tree::new("missing").with_version_sites();
        tree.write("README.md", "no version here\n");
        assert!(apply_version(&tree.0, &options("1.3.0", false)).unwrap_err().contains("README.md"));
    }

    #[test]
    fn the_sites_exist_in_the_repository() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (file, pattern, _) in VERSION_SITES {
            let text = std::fs::read_to_string(root.join(file)).unwrap();
            assert!(regex::Regex::new(pattern).unwrap().is_match(&text), "{file}");
        }
    }

    #[test]
    fn unreleased_entries_move_under_a_dated_heading() {
        let tree = Tree::new("changelog");
        tree.write("CHANGELOG.md", "# Changes\n\n## Unreleased\n\n- a change\n\n## 1.2.3 — 2026-01-01\n\n- old\n");
        close_changelog(&tree.0, &options("1.3.0", false)).unwrap();
        let text = tree.read("CHANGELOG.md");
        let sections = changelog_sections(&text);
        assert_eq!(sections[0], ("Unreleased".to_owned(), Vec::new()));
        assert!(sections[1].0.starts_with("1.3.0 — "));
        assert_eq!(sections[1].1, vec!["- a change".to_owned()]);
    }

    #[test]
    fn an_empty_unreleased_section_stops_the_release() {
        let tree = Tree::new("empty");
        tree.write("CHANGELOG.md", "# Changes\n\n## Unreleased\n\n## 1.2.3 — 2026-01-01\n\n- old\n");
        assert!(close_changelog(&tree.0, &options("1.3.0", false)).is_err());
    }

    #[test]
    fn a_section_already_written_is_left_alone() {
        let tree = Tree::new("written");
        let text = "# Changes\n\n## Unreleased\n\n## 1.3.0 — 2026-01-01\n\n- new\n";
        tree.write("CHANGELOG.md", text);
        close_changelog(&tree.0, &options("1.3.0", false)).unwrap();
        assert_eq!(tree.read("CHANGELOG.md"), text);
    }

    #[test]
    fn notes_are_checked_for_the_version_the_files_and_the_paragraphs() {
        let tree = Tree::new("notes");
        let files: String = assets("1.3.0").iter().map(|name| format!("`{name}`, ")).collect();
        tree.write("RELEASE_NOTES.md", &format!("# KeySwitch 1.3.0\n\nAll on one line: {files}done.\n"));
        check_release_notes(&tree.0, "1.3.0").unwrap();
        assert!(check_release_notes(&tree.0, "1.4.0").is_err(), "notes for another version");
        tree.write("RELEASE_NOTES.md", "# KeySwitch 1.3.0\n\nNo file named at all.\n");
        assert!(check_release_notes(&tree.0, "1.3.0").is_err(), "notes without the published files");
        tree.write("RELEASE_NOTES.md", &format!("# KeySwitch 1.3.0\n\nOver two lines: {files}\nand on.\n"));
        assert!(check_release_notes(&tree.0, "1.3.0").is_err(), "a paragraph over several lines");
    }

    #[test]
    fn a_bad_version_is_refused_before_anything_is_written() {
        for version in ["1.2", "1.2.3.4", "1.2.x", "", "v1.2.3"] {
            let tree = Tree::new("version").with_version_sites();
            assert!(check_preconditions(&tree.0, &options(version, false)).unwrap_err().contains("no version"), "{version}");
        }
    }

    #[test]
    fn a_candidate_has_its_own_tag() {
        let plain = Options { version: "1.3.0".to_owned(), ..Options::default() };
        assert_eq!(release_tag(&plain), "v1.3.0");
        let candidate = Options { version: "1.3.0".to_owned(), candidate: Some(2), ..Options::default() };
        assert_eq!(release_tag(&candidate), "v1.3.0-rc2");
    }

    #[test]
    fn wrapped_lines_are_found() {
        assert_eq!(wrapped_lines("# KeySwitch 1\n\nOne paragraph\ncontinued\n\n- item\n- item\n\n```\nfree\ntext\n```\n"), vec![4]);
    }

    #[test]
    fn changelog_entries_stay_under_their_heading() {
        let sections = changelog_sections("# Changes\n\n## Unreleased\n\n- a\n- b\n\n## 1.0.0 — 2026-01-01\n\n- c\n");
        assert_eq!(sections, vec![("Unreleased".to_owned(), vec!["- a".to_owned(), "- b".to_owned()]), ("1.0.0 — 2026-01-01".to_owned(), vec!["- c".to_owned()])]);
    }
}

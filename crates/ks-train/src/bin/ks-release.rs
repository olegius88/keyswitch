//! One command that turns a prepared working tree into a published release: the version into every file that spells it,
//! the changelog section closed, the checks, the commit, the tag and the push, the release workflow waited for, the
//! published files checked. It writes no prose: the changelog's Unreleased entries and `RELEASE_NOTES.md` are the author's,
//! and it refuses to run without them.
//!
//!     ks-release --version X.Y.Z [--candidate N] [--dry-run] [--skip-checks] [--skip-ci] [--remote origin] [--branch main]
//!                [--field-root <tree>] [--field-released <folder>] [--field-released-speller <folder>] [--message-file <file>]
//!
//! With --candidate the tag is vX.Y.Z-rcN and the release a pre-release: the update check of an installed program does not
//! see it, and the APT repository stays as it is. Without it the tag replaces the published version for everybody.
//!
//! No check the workflow makes is made here as well. What only this machine can check is checked here on every run: the
//! engine reports the release commits, and the owner's typing against the last release. Everything else the workflow
//! run of the tag makes on every system, so a candidate leaves it to that run, which the command waits for; a failed run
//! costs a candidate number. A release on the very commit of a candidate that passed leaves it to the run that candidate
//! had, and the run of its own tag publishes the candidate's files; any other release makes every check here first,
//! since its tag cannot be taken back. Either way the workflow must already have checked the commit the release starts
//! from.
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
use ks_cast::To as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const UNRELEASED: &str = "## Unreleased";
const WORKFLOW: &str = "CI and release";
/// The job that ends the checks of a workflow run. It passes only in a run that checked and built everything: a run that
/// publishes the files of a candidate, or that stands aside for the run of a tag on the same commit, skips it.
const CHECKS_JOB: &str = "Every line and branch of the program ran";
/// What `gh` says of a run or a job that has ended, and of one that ended well or was not needed.
const COMPLETED: &str = "completed";
const SUCCESS: &str = "success";
const SKIPPED: &str = "skipped";
/// How many runs of one commit are read (a branch, its candidates and its release), and of one tag.
const RUNS_OF_A_COMMIT: &str = "20";
const RUNS_OF_A_TAG: &str = "1";
/// How many answers in a row `gh` may fail to give before a wait is given up: a network that is down for half an hour
/// comes back, a `gh` that is not signed in does not.
const UNREAD_ANSWERS: u32 = 30;
const CI_TIMEOUT_SECONDS: u64 = 3 * 60 * 60;
const CI_POLL_SECONDS: u64 = 60;
const SECONDS_PER_DAY: i64 = 86_400;
const DATE_PATTERN: &str = r"^\d{4}-\d{2}-\d{2}$";
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
    /// The last release's reports replayed with the Hunspell dictionaries, as the installed program runs.
    field_released_speller: Option<PathBuf>,
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

/// A run of the workflow, as `gh run list --json` gives it.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Run {
    database_id: u64,
    status: String,
    conclusion: String,
    /// The branch or the tag the run is for.
    head_branch: String,
    url: String,
}

/// A job of a run, as `gh run view --json jobs` gives it.
#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct Job {
    name: String,
    status: String,
    conclusion: String,
}

#[derive(serde::Deserialize)]
struct Jobs {
    jobs: Vec<Job>,
}

/// A published release, as `gh release view --json url,assets` gives it.
#[derive(serde::Deserialize)]
struct Published {
    url: String,
    assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
struct Asset {
    name: String,
}

/// A run that ended in success, and whether it checked and built everything.
struct Passed {
    run: Run,
    whole: bool,
}

/// What the runs of a tag or of a commit say so far.
enum Runs {
    Passed(Vec<Passed>),
    Failed(String),
    Going,
}

/// The answer of `gh`, read as what it is asked for.
fn gh<T: serde::de::DeserializeOwned>(root: &Path, arguments: &[&str]) -> Result<T, String> {
    ks_text::json::loads(output(root, "gh", arguments)?).map_err(|error| format!("gh {} gave no answer to read: {error}", arguments.join(" ")))
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

/// What a release writes into the tree besides the files that spell the version: the changelog, the lockfile cargo rewrites
/// for a new version, and the two engine reports its checks replay for this program.
const RELEASE_WRITES: [&str; 4] = ["CHANGELOG.md", "Cargo.lock", "model/boundary_v1/engine-regression.json", "model/prefix_v1/engine-report.json"];
/// Where a replay keeps the report it replaced: the only folders a release adds files to.
const RELEASE_HISTORIES: [&str; 2] = ["model/boundary_v1/engine-history", "model/prefix_v1/engine-history"];

/// The changes in the tree that the release does not make itself; they would ride into the release commit unseen.
fn foreign_changes(root: &Path) -> Result<Vec<String>, String> {
    let changed = output(root, "git", &["diff", "--name-only", "HEAD"])?;
    let added = output(root, "git", &["ls-files", "--others", "--exclude-standard"])?;
    let written = |name: &str| RELEASE_WRITES.contains(&name) || VERSION_SITES.iter().any(|(site, _, _)| *site == name);
    let kept = |name: &str| RELEASE_HISTORIES.iter().any(|folder| Path::new(name).parent() == Some(Path::new(folder)));
    Ok(changed.lines().filter(|name| !written(name)).chain(added.lines().filter(|name| !kept(name))).map(str::to_owned).collect())
}

fn refuse_foreign_changes(root: &Path, what: &str) -> Result<(), String> {
    let foreign = foreign_changes(root)?;
    if foreign.is_empty() {
        return Ok(());
    }
    Err(format!("{what} files a release does not write: {}; commit them or move them out of the tree", foreign.join(", ")))
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

/// The changelog with the entries under Unreleased put at the top of the section `heading`, when that section is the one
/// right below them.
fn unreleased_into_section(text: &str, heading: &str) -> Option<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let unreleased = lines.iter().position(|line| *line == UNRELEASED)?;
    let (head, below) = lines.split_at(unreleased + 1);
    let (entries, section) = below.split_at(below.iter().position(|line| line.starts_with("## "))?);
    let (title, older) = section.split_first()?;
    if title.strip_prefix("## ") != Some(heading) {
        return None;
    }
    let entries = entries.iter().filter(|line| !line.trim().is_empty());
    let older = older.iter().skip_while(|line| line.trim().is_empty());
    Some(head.iter().chain(&["", *title, ""]).chain(entries).chain(older).copied().collect::<Vec<&str>>().join("\n"))
}

/// Today in the local calendar, as the Python tool dated a release (`date.today()`): the answer of `date +%F`, or the UTC
/// date when it gives none.
fn today() -> String {
    let local = Command::new("date").arg("+%F").output().ok().filter(|done| done.status.success()).map(|done| String::from_utf8_lossy(&done.stdout).trim().to_owned());
    local_date(local, utc_today())
}

/// The answer of `date` when it is a date, else the UTC date.
fn local_date(answer: Option<String>, utc: String) -> String {
    answer.filter(|date| regex::Regex::new(DATE_PATTERN).is_ok_and(|pattern| pattern.is_match(date))).unwrap_or(utc)
}

/// Today in the UTC calendar.
fn utc_today() -> String {
    date_of((std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())).to::<i64>())
}

/// The UTC date of a moment, in seconds since the epoch: days since the epoch to a date (Howard Hinnant's `civil_from_days`).
fn date_of(seconds: i64) -> String {
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
    let entries = sections.iter().find(|(heading, _)| heading == "Unreleased").map_or(0, |(_, entries)| entries.len());
    if let Some((heading, _)) = sections.iter().find(|(heading, _)| heading != "Unreleased" && heading.split(' ').next() == Some(options.version.as_str())) {
        // The section keeps the day its candidate closed it: a release on the candidate's commit publishes the
        // candidate's files, and a new date would be a new commit that nothing has checked.
        if entries == 0 {
            println!("    CHANGELOG.md already has a section for {}", options.version);
            return Ok(());
        }
        // Written after the section was closed, these are still changes of this release.
        let moved = unreleased_into_section(&text, heading).ok_or_else(|| format!("CHANGELOG.md has entries under {UNRELEASED} and a section for {} that is not the one right below them; move the entries by hand", options.version))?;
        if options.dry_run {
            println!("    would move {entries} Unreleased entries into the section {heading}");
            return Ok(());
        }
        std::fs::write(&path, moved).map_err(|error| error.to_string())?;
        println!("    moved {entries} Unreleased entries into the section {heading}");
        return Ok(());
    }
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

/// What only this machine can check, stopping at the first check that does not hold: the engine reports replayed for this
/// program, which the release commits, and the owner's typing against the last release, which never leaves this machine.
/// The reports come first, so every check after them holds the tree that is committed.
fn own_checks(root: &Path, options: &Options) -> Result<(), String> {
    let (Some(field_root), Some(released)) = (&options.field_root, &options.field_released) else {
        return Err(fail("--field-root and --field-released name the owner's typing and the last release's reports; a release does not skip them"));
    };
    let tree = root.to_string_lossy().into_owned();
    run(root, "the trainer", "cargo", &["build", "--profile", "train", "-p", "ks-train"])?;
    let trainer = root.join("target/train/ks-train").to_string_lossy().into_owned();
    run(root, "the boundary regressions", "dbus-run-session", &["--", &trainer, "engine", "boundary", "--tree", &tree, "--refresh-runtime"])?;
    run(root, "the prefix replay", "dbus-run-session", &["--", &trainer, "engine", "prefix", "--tree", &tree, "--refresh-runtime"])?;
    let (field_root, released) = (field_root.to_string_lossy().into_owned(), released.to_string_lossy().into_owned());
    // The gate runs the replay beside it: both are built, so neither is left from an older tree.
    run(root, "the replay tools", "cargo", &["build", "--profile", "parity", "-p", "ks-replay", "--bins"])?;
    let gate = root.join("target/parity/field-gate").to_string_lossy().into_owned();
    run(root, "the owner's typing against the last release", &gate, &["--root", &field_root, "--released", &released])?;
    if let Some(spelled) = &options.field_released_speller {
        run(root, "the owner's typing with Hunspell against the last release", &gate, &["--root", &field_root, "--released", &spelled.to_string_lossy(), "--speller"])?;
    }
    Ok(())
}

/// The checks the workflow run of a tag repeats on every system, made here for a release whose tag no run has checked
/// yet: a tag that is pushed and then fails cannot be taken back.
fn repeated_checks(root: &Path, options: &Options) -> Result<(), String> {
    let tree = root.to_string_lossy().into_owned();
    let trainer = root.join("target/train/ks-train").to_string_lossy().into_owned();
    let mut arguments = vec!["test", "--profile", "parity"];
    for name in TESTED_CRATES {
        arguments.extend(["-p", name]);
    }
    run(root, "the tests of the engine, the trainer and LogCourier", "cargo", &arguments)?;
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
    run(root, "the Debian package", "bash", &["packaging/build-rust-deb.sh"])?;
    let architecture = output(root, "dpkg", &["--print-architecture"])?;
    let package = format!("dist/keyswitch_{}_{architecture}.deb", options.version);
    run(root, "what the package holds", &trainer, &["deb-check", "--tree", &tree, &package])?;
    run(root, "the real tests in a private X server, and the package", "bash", &["packaging/e2e.sh", &package])
}

fn commit_tag_push(root: &Path, options: &Options) -> Result<String, String> {
    let tag = release_tag(options);
    refuse_foreign_changes(root, "the checks changed")?;
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

/// What the push runs of the workflow that `selector` names say now (`--branch <tag>` or `--commit <sha>`). A job that
/// has failed is said at once, while the jobs beside it go on: the run can no longer end well.
fn runs_so_far(root: &Path, selector: [&str; 2], limit: &str) -> Result<Runs, String> {
    let listed: Vec<Run> = gh(root, &["run", "list", "--workflow", WORKFLOW, selector[0], selector[1], "--event", "push", "--limit", limit, "--json", "databaseId,status,conclusion,headBranch,url"])?;
    let mut passed = Vec::new();
    let mut going = false;
    for run in listed {
        let jobs = gh::<Jobs>(root, &["run", "view", &run.database_id.to_string(), "--json", "jobs"])?.jobs;
        if let Some(job) = jobs.iter().find(|job| job.status == COMPLETED && job.conclusion != SUCCESS && job.conclusion != SKIPPED) {
            return Ok(Runs::Failed(format!("the job «{}» ended with {}: {}", job.name, job.conclusion, run.url)));
        }
        if run.status != COMPLETED {
            going = true;
        } else if run.conclusion == SUCCESS {
            passed.push(Passed { whole: jobs.iter().any(|job| job.name == CHECKS_JOB && job.conclusion == SUCCESS), run });
        } else {
            return Ok(Runs::Failed(format!("the run ended with {}: {}", run.conclusion, run.url)));
        }
    }
    Ok(if going { Runs::Going } else { Runs::Passed(passed) })
}

/// The runs that `selector` names, once all of them have ended well; `first` waits for a run to appear, as the run of a
/// tag just pushed does. An answer `gh` fails to give is asked for again, since one lost call must not end a release
/// whose tag is already pushed; the wait is given up when the time is up, or when `gh` has given no answer too many times
/// in a row.
fn settled_runs(root: &Path, selector: [&str; 2], limit: &str, first: bool, options: &Options) -> Result<Vec<Passed>, String> {
    let started = std::time::Instant::now();
    let (mut said, mut unread_in_a_row, mut appeared) = (String::new(), 0_u32, false);
    loop {
        let unread = match runs_so_far(root, selector, limit) {
            Ok(Runs::Passed(passed)) if !(first && passed.is_empty()) => return Ok(passed),
            Ok(Runs::Failed(why)) => return Err(why),
            Ok(found) => {
                appeared |= matches!(found, Runs::Going);
                unread_in_a_row = 0;
                None
            }
            Err(error) => {
                unread_in_a_row += 1;
                Some(error)
            }
        };
        // Said once, not at every asking: whoever watches sees what is wrong.
        if let Some(error) = unread.as_ref().filter(|error| **error != said) {
            println!("    asking again: {error}");
            said.clone_from(error);
        }
        let late = started.elapsed().as_secs() >= options.timeout_seconds;
        if let Some(error) = unread.filter(|_| late || unread_in_a_row >= UNREAD_ANSWERS) {
            return Err(format!("the runs could not be read: {error}"));
        }
        if late {
            return Err(fail(if appeared { "the run did not end in time" } else { "no run appeared in time" }));
        }
        std::thread::sleep(std::time::Duration::from_secs(options.poll_seconds));
    }
}

/// The runs that passed on a commit hold one that checked and built everything: the checks made here are not all there
/// are.
fn with_a_whole_run(passed: Vec<Passed>, head: &str) -> Result<Vec<Passed>, String> {
    if passed.iter().any(|run| run.whole) {
        return Ok(passed);
    }
    Err(format!("no run of the workflow has checked the commit {head} whole; push the branch, let its run end and start again"))
}

/// The runs that passed on the commit the release starts from. With `wait` the runs that still go on are waited for.
/// Without it only what is known already is said, before the minutes of the checks made here: a run that failed, or no
/// run that checked the commit.
fn checked_commit(root: &Path, options: &Options, wait: bool) -> Result<Vec<Passed>, String> {
    let head = output(root, "git", &["rev-parse", "HEAD"])?;
    let on_commit = |why: String| format!("the workflow on the commit {head}: {why}");
    if wait {
        println!("    $ the workflow has checked the commit this release starts from");
        return with_a_whole_run(settled_runs(root, ["--commit", &head], RUNS_OF_A_COMMIT, false, options).map_err(on_commit)?, &head);
    }
    match runs_so_far(root, ["--commit", &head], RUNS_OF_A_COMMIT) {
        Ok(Runs::Failed(why)) => Err(on_commit(why)),
        Ok(Runs::Passed(passed)) => with_a_whole_run(passed, &head),
        // Still going, or not read this once: the wait after the checks made here settles it.
        Ok(Runs::Going) | Err(_) => Ok(Vec::new()),
    }
}

/// The candidate whose files a release publishes: a candidate of this version whose run checked this very commit whole
/// and passed, when the tree holds nothing more to commit. The workflow run of the release's tag then publishes those
/// files instead of building others.
fn published_candidate(root: &Path, options: &Options, passed: &[Passed]) -> Result<Option<String>, String> {
    if !output(root, "git", &["status", "--porcelain"])?.is_empty() {
        return Ok(None);
    }
    let prefix = format!("v{}-rc", options.version);
    Ok(passed.iter().filter(|run| run.whole).map(|run| &run.run.head_branch).find(|tag| tag.strip_prefix(&prefix).is_some_and(|number| number.parse::<u32>().is_ok())).cloned())
}

/// Every check a release holds to. What only this machine can check is checked on every run; the rest is repeated here
/// only when no workflow run has made it, or will make it before the tag is final. Gives the candidate whose files the
/// run of the release's tag is to publish, when there is one.
fn run_checks(root: &Path, options: &Options) -> Result<Option<String>, String> {
    checked_commit(root, options, false)?;
    own_checks(root, options)?;
    let passed = checked_commit(root, options, true)?;
    if options.candidate.is_some() {
        println!("    the workflow run of the candidate's tag makes every other check");
        return Ok(None);
    }
    let candidate = published_candidate(root, options, &passed)?;
    match &candidate {
        Some(candidate) => println!("    {candidate} passed every other check on this commit: none is repeated here, and the run of the release's tag is to publish its files"),
        None => repeated_checks(root, options)?,
    }
    Ok(candidate)
}

/// The workflow run of this tag, waited for until it ends in success. Says whether the run checked and built everything,
/// as a run that publishes the files of a candidate does not.
fn wait_for_workflow(root: &Path, tag: &str, options: &Options) -> Result<bool, String> {
    let passed = settled_runs(root, ["--branch", tag], RUNS_OF_A_TAG, true, options).map_err(|why| format!("the release workflow of {tag}: {why}"))?;
    for run in &passed {
        println!("    {}", run.run.url);
    }
    Ok(passed.iter().any(|run| run.whole))
}

fn verify_published(root: &Path, options: &Options, tag: &str) -> Result<String, String> {
    let release: Published = gh(root, &["release", "view", tag, "--json", "url,assets"])?;
    let missing: Vec<String> = assets(&options.version).into_iter().filter(|asset| !release.assets.iter().any(|published| published.name == *asset)).collect();
    if !missing.is_empty() {
        return Err(format!("the release {tag} is missing: {}", missing.join(", ")));
    }
    Ok(release.url)
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
    refuse_foreign_changes(root, "the tree has changed")
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
    let candidate = if options.skip_checks {
        println!("[5] skipping the checks (--skip-checks)");
        None
    } else {
        println!("[5] running the checks");
        run_checks(root, options)?
    };
    println!("[6] committing, tagging and pushing");
    let tag = commit_tag_push(root, options)?;
    if options.skip_ci {
        println!("    skipping the release workflow (--skip-ci)");
        return Ok(());
    }
    println!("[7] waiting for the release workflow");
    // What the run did with the candidate is its own decision (packaging/ci-plan.sh), so it is read, not assumed.
    match (candidate, wait_for_workflow(root, &tag, options)?) {
        (Some(candidate), false) => println!("    the run published the files of {candidate}"),
        (Some(candidate), true) => println!("    the run checked and built everything anew instead of publishing the files of {candidate}: its plan job says why"),
        (None, _) => {}
    }
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
        eprintln!("usage: ks-release --version X.Y.Z [--candidate N] [--dry-run] [--skip-checks] [--skip-ci] [--remote origin] [--branch main] [--field-root <tree>] [--field-released <folder>] [--field-released-speller <folder>] [--message-file <file>]");
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
        field_released_speller: value("--field-released-speller").map(PathBuf::from),
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
    fn a_candidate_leaves_a_section_already_written_alone() {
        let tree = Tree::new("written");
        let text = "# Changes\n\n## Unreleased\n\n## 1.3.0 — 2026-01-01\n\n- new\n";
        tree.write("CHANGELOG.md", text);
        close_changelog(&tree.0, &Options { candidate: Some(2), ..options("1.3.0", false) }).unwrap();
        assert_eq!(tree.read("CHANGELOG.md"), text);
    }

    #[test]
    fn a_release_keeps_the_day_its_candidate_closed_the_section() {
        // A new date would be a new commit, and the release publishes the files of the candidate's commit.
        let tree = Tree::new("kept-date");
        let text = "# Changes\n\n## Unreleased\n\n## 1.3.0 — 2026-01-01\n\n- new\n\n## 1.2.0 — 2026-01-01\n\n- old\n";
        tree.write("CHANGELOG.md", text);
        close_changelog(&tree.0, &options("1.3.0", false)).unwrap();
        assert_eq!(tree.read("CHANGELOG.md"), text);
    }

    #[test]
    fn entries_written_after_the_section_was_closed_join_it() {
        let tree = Tree::new("late-entries");
        let text = "# Changes\n\n## Unreleased\n\n- late\n- later\n\n## 1.3.0 — 2026-01-01\n\n- new\n\n## 1.2.0 — 2026-01-01\n\n- old\n";
        tree.write("CHANGELOG.md", text);
        close_changelog(&tree.0, &options("1.3.0", true)).unwrap();
        assert_eq!(tree.read("CHANGELOG.md"), text, "a dry run writes nothing");
        for candidate in [Some(2), None] {
            tree.write("CHANGELOG.md", text);
            close_changelog(&tree.0, &Options { candidate, ..options("1.3.0", false) }).unwrap();
            assert_eq!(tree.read("CHANGELOG.md"), "# Changes\n\n## Unreleased\n\n## 1.3.0 — 2026-01-01\n\n- late\n- later\n- new\n\n## 1.2.0 — 2026-01-01\n\n- old\n");
        }
        // Another section between them: which entries belong to which release is the author's to say.
        tree.write("CHANGELOG.md", "# Changes\n\n## Unreleased\n\n- late\n\n## 1.4.0 — 2026-02-01\n\n- other\n\n## 1.3.0 — 2026-01-01\n\n- new\n");
        assert!(close_changelog(&tree.0, &options("1.3.0", false)).unwrap_err().contains("by hand"));
        assert_eq!(unreleased_into_section("# Changes\n\n## 1.3.0 — 2026-01-01\n", "1.3.0 — 2026-01-01"), None, "no Unreleased heading");
        assert_eq!(unreleased_into_section("## Unreleased\n\n- late\n", "1.3.0 — 2026-01-01"), None, "no section below");
    }

    #[test]
    fn the_workflow_has_the_names_the_command_asks_for() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let workflow = std::fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
        assert!(workflow.starts_with(&format!("name: {WORKFLOW}\n")), "the workflow is asked for by its name");
        assert!(workflow.contains(&format!("    name: {CHECKS_JOB}\n")), "the job that ends the checks is asked for by its name");
        // The workflow's own plan publishes a candidate's files only when it finds every file a release has.
        let plan = std::fs::read_to_string(root.join("packaging/ci-plan.sh")).unwrap().replace("${version}", "$version");
        for asset in ASSETS {
            assert!(plan.contains(&format!("\"{}\"", asset.replace("{version}", "$version"))), "{asset}");
        }
        assert!(plan.contains(&format!("checks_job=\"{CHECKS_JOB}\"")), "the plan reads the same job");
        assert!(workflow.contains(&format!("    name: {}\n", plan.split("apt_job=\"").nth(1).unwrap().split('"').next().unwrap())), "the plan names the job of the APT repository");
    }

    #[test]
    fn the_stable_tests_of_a_tag_are_the_tests_a_release_runs_here() {
        // A candidate leaves these tests to the workflow run of its tag: a crate tested here and not there would go untested.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let script = std::fs::read_to_string(root.join("packaging/stable-tests.sh")).unwrap();
        let named = |line: &str| line.split(" -p ").skip(1).map(|rest| rest.split([' ', '"']).next().unwrap().to_owned()).collect::<Vec<String>>();
        let whole = script.lines().find(|line| line.trim_start().starts_with("all) ")).unwrap();
        assert_eq!(named(whole), TESTED_CRATES, "a whole run");
        let rest = script.lines().find(|line| line.contains(" -p ks-text ") && !line.trim_start().starts_with("all) ")).unwrap();
        let mut in_parts = named(rest);
        in_parts.push("ks-train".to_owned());
        in_parts.sort();
        let mut tested = TESTED_CRATES.map(str::to_owned).to_vec();
        tested.sort();
        assert_eq!(in_parts, tested, "the parts: every crate but ks-train in `rest`, ks-train dealt out by its tests");
    }

    #[test]
    fn the_workflow_runs_every_part_the_scripts_deal_out() {
        // A part a script knows and no runner takes would leave its tests unrun with every job green.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let workflow = std::fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
        let parts = |script: &str| {
            let text = std::fs::read_to_string(root.join("packaging").join(script)).unwrap();
            let usage = text.lines().find_map(|line| line.strip_prefix("usage='usage: ")).unwrap().to_owned();
            let listed = usage.split('[').next_back().unwrap().trim_end_matches("]'").to_owned();
            listed.split('|').filter(|part| *part != "all").map(str::to_owned).collect::<Vec<String>>()
        };
        let measured = parts("coverage.sh");
        assert_eq!(measured.len(), 8);
        for part in &measured {
            assert_eq!(workflow.matches(&format!("            part: {part}\n")).count(), 1, "{part}");
        }
        assert_eq!(workflow.matches("          - os: ubuntu-26.04\n            part: ").count(), measured.len(), "a part of the workflow no script knows");
        assert!(workflow.contains(&format!("        part: [{}]\n", parts("stable-tests.sh").join(", "))), "the parts of the tests on the stable compiler");
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
    fn a_version_with_an_empty_part_is_refused() {
        let tree = Tree::new("empty-part").with_version_sites();
        assert!(check_preconditions(&tree.0, &options("1..3", false)).unwrap_err().contains("no version"));
    }

    #[test]
    fn a_date_is_that_of_the_utc_calendar_in_every_month() {
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(951_782_400), "2000-02-29");
        assert_eq!(date_of(1_700_000_000), "2023-11-14");
        assert!(regex::Regex::new(DATE_PATTERN).unwrap().is_match(&today()));
        assert!(regex::Regex::new(DATE_PATTERN).unwrap().is_match(&utc_today()));
    }

    #[test]
    fn the_local_date_is_the_answer_of_date_when_it_is_one() {
        assert_eq!(local_date(Some("2026-11-08".to_owned()), "2026-11-07".to_owned()), "2026-11-08");
        assert_eq!(local_date(Some("Sun Nov  8".to_owned()), "2026-11-07".to_owned()), "2026-11-07");
        assert_eq!(local_date(None, "2026-11-07".to_owned()), "2026-11-07");
    }

    #[test]
    fn a_numbered_item_starts_a_block_and_a_dotted_word_does_not() {
        assert_eq!(wrapped_lines("1. one\n2. two\n\nWords\nand. more\n. dotted\n12a. not a number\n"), vec![5, 6, 7]);
    }

    #[test]
    fn changelog_entries_stay_under_their_heading() {
        let sections = changelog_sections("# Changes\n\n## Unreleased\n\n- a\n- b\n\n## 1.0.0 — 2026-01-01\n\n- c\n");
        assert_eq!(sections, vec![("Unreleased".to_owned(), vec!["- a".to_owned(), "- b".to_owned()]), ("1.0.0 — 2026-01-01".to_owned(), vec!["- c".to_owned()])]);
    }
}

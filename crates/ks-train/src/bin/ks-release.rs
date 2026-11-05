//! One command that turns a prepared working tree into a published release: the version into every file that spells it,
//! the changelog section closed, the checks, the commit, the tag and the push, the release workflow waited for, the
//! published files checked. It writes no prose: the changelog's Unreleased entries and RELEASE_NOTES.md are the author's,
//! and it refuses to run without them.
//!
//!     ks-release --version X.Y.Z [--dry-run] [--skip-checks] [--skip-ci] [--remote origin] [--branch rust]
//!                [--field-root <tree>] [--field-released <folder>] [--message-file <file>]
use std::path::{Path, PathBuf};
use std::process::Command;

const UNRELEASED: &str = "## Unreleased";
const WORKFLOW: &str = "Rust";
const CI_TIMEOUT_SECONDS: u64 = 3 * 60 * 60;
const CI_POLL_SECONDS: u64 = 60;
const SECONDS_PER_DAY: i64 = 86_400;
const ASSETS: [&str; 6] = ["KeySwitch-Setup-{version}-x64.exe", "KeySwitch-Setup-{version}-arm64.exe", "KeySwitch-Setup-{version}-x86.exe", "keyswitch_{version}_amd64.deb", "KeySwitch-{version}-macos-arm64.zip", "KeySwitch-{version}-macos-x86_64.zip"];
const TESTED_CRATES: [&str; 8] = ["ks-text", "ks-lang", "ks-context", "ks-engine", "ks-config", "ks-replay", "ks-train", "logcourier"];

struct Options {
    version: String,
    dry_run: bool,
    skip_checks: bool,
    skip_ci: bool,
    remote: String,
    branch: String,
    field_root: Option<PathBuf>,
    field_released: Option<PathBuf>,
    message_file: Option<PathBuf>,
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
    let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs()) as i64;
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
    run(root, "the receipt of the installed pair", &trainer, &["verify", "--tree", &tree])?;
    run(root, "the boundary model reproduces", &trainer, &["boundary", "--tree", &tree, "--verify"])?;
    run(root, "the boundary regressions", "dbus-run-session", &["--", &trainer, "engine", "boundary", "--tree", &tree, "--refresh-runtime"])?;
    run(root, "the prefix replay", "dbus-run-session", &["--", &trainer, "engine", "prefix", "--tree", &tree, "--refresh-runtime"])?;
    if let (Some(field_root), Some(released)) = (&options.field_root, &options.field_released) {
        let (field_root, released) = (field_root.to_string_lossy().into_owned(), released.to_string_lossy().into_owned());
        run(root, "the owner's typing against the last release", "cargo", &["run", "--profile", "parity", "--bin", "field-gate", "--", "--root", &field_root, "--released", &released])?;
    } else {
        return Err(fail("--field-root and --field-released name the owner's typing and the last release's reports; a release does not skip them"));
    }
    run(root, "the Debian package", "bash", &["packaging/build-rust-deb.sh"])
}

fn commit_tag_push(root: &Path, options: &Options) -> Result<String, String> {
    let tag = format!("v{}", options.version);
    if !output(root, "git", &["status", "--porcelain"])?.is_empty() {
        let message = match &options.message_file {
            Some(path) => std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?,
            None => {
                let text = std::fs::read_to_string(root.join("CHANGELOG.md")).map_err(|error| error.to_string())?;
                let sections = changelog_sections(&text);
                let entries = sections.iter().find(|(heading, _)| heading.split(' ').next() == Some(options.version.as_str())).map(|(_, entries)| entries.join("\n")).filter(|entries| !entries.is_empty());
                format!("Release KeySwitch {}\n\n{}\n", options.version, entries.ok_or_else(|| format!("CHANGELOG.md has no entries under {}; nothing to describe", options.version))?)
            }
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
fn wait_for_workflow(root: &Path, tag: &str) -> Result<(), String> {
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
        if started.elapsed().as_secs() > CI_TIMEOUT_SECONDS {
            return Err(format!("the release workflow of {tag} did not finish in time"));
        }
        std::thread::sleep(std::time::Duration::from_secs(CI_POLL_SECONDS));
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
    let tag = format!("v{}", options.version);
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
    wait_for_workflow(root, &tag)?;
    println!("[8] verifying the published release");
    let url = verify_published(root, options, &tag)?;
    println!("\nKeySwitch {} is released: {url}", options.version);
    Ok(())
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| arguments.iter().position(|argument| argument == name).and_then(|place| arguments.get(place + 1)).cloned();
    let flag = |name: &str| arguments.iter().any(|argument| argument == name);
    let Some(version) = value("--version") else {
        eprintln!("usage: ks-release --version X.Y.Z [--dry-run] [--skip-checks] [--skip-ci] [--remote origin] [--branch rust] [--field-root <tree>] [--field-released <folder>] [--message-file <file>]");
        std::process::exit(2);
    };
    let options = Options {
        version,
        dry_run: flag("--dry-run"),
        skip_checks: flag("--skip-checks"),
        skip_ci: flag("--skip-ci"),
        remote: value("--remote").unwrap_or_else(|| "origin".to_owned()),
        branch: value("--branch").unwrap_or_else(|| "rust".to_owned()),
        field_root: value("--field-root").map(PathBuf::from),
        field_released: value("--field-released").map(PathBuf::from),
        message_file: value("--message-file").map(PathBuf::from),
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
mod tests {
    use super::*;

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

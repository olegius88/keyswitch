//! The check that every line and branch of the program ran. It reads the reports `cargo llvm-cov --json --branch` wrote on
//! each system: code for one system is measured there, code for all of them counts as covered when any of them ran it.
//! The files in the list of native code are left out, as they were in the Python: they call the operating system, and the
//! failures they handle cannot be produced. The live tests run them.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

/// What the reports say of one source file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FileCoverage {
    /// Line, and whether any report saw it run.
    pub lines: BTreeMap<u64, bool>,
    /// Where a branch starts, with the times it went each way.
    pub branches: BTreeMap<(u64, u64, u64, u64), (u64, u64)>,
}

/// One uncovered place.
#[derive(Debug, PartialEq, Eq)]
pub enum Gap {
    Line(u64),
    /// A branch that went only one way, or none.
    Branch { line: u64, column: u64, taken: bool, not_taken: bool },
}

fn number(value: &Value) -> u64 {
    value.as_u64().unwrap_or(0)
}

/// The lines of a file with a count, from its segments, as `llvm-cov` reads them: a line's count is the largest of the
/// region running into it and the regions starting on it.
fn line_counts(segments: &[Value]) -> BTreeMap<u64, u64> {
    let mut counts = BTreeMap::new();
    let Some(last) = segments.iter().map(|segment| number(&segment[0])).max() else { return counts };
    let mut next = segments.iter().peekable();
    let mut wrapped: Option<&Value> = None;
    let first = number(&segments[0][0]);
    for line in first..=last {
        let mut starting = Vec::new();
        while let Some(segment) = next.next_if(|segment| number(&segment[0]) == line) {
            starting.push(segment);
        }
        let skipped = starting.first().is_some_and(|segment| segment[3] == false && segment[4] == true);
        let regions = starting.iter().filter(|segment| segment[3] == true && segment[4] == true && segment[5] == false);
        let running = wrapped.filter(|segment| segment[3] == true).map(|segment| number(&segment[2]));
        let entered = regions.map(|segment| number(&segment[2])).max();
        if !skipped && let Some(count) = running.into_iter().chain(entered).max() {
            counts.insert(line, count);
        }
        if let Some(segment) = starting.last() {
            wrapped = Some(segment);
        }
    }
    counts
}

/// The path of a file below `crates/`, the same on every system.
fn relative(name: &str) -> Option<String> {
    let name = name.replace('\\', "/");
    name.split_once("/crates/").map(|(_, rest)| format!("crates/{rest}"))
}

/// Adds one report to what the others said.
pub fn merge(files: &mut BTreeMap<String, FileCoverage>, report: &Value) {
    let reported = report["data"][0]["files"].as_array().into_iter().flatten();
    for file in reported {
        let Some(name) = file["filename"].as_str().and_then(relative) else { continue };
        let entry = files.entry(name).or_default();
        let segments = file["segments"].as_array().map_or(&[][..], Vec::as_slice);
        for (line, count) in line_counts(segments) {
            *entry.lines.entry(line).or_default() |= count > 0;
        }
        for branch in file["branches"].as_array().into_iter().flatten() {
            let key = (number(&branch[0]), number(&branch[1]), number(&branch[2]), number(&branch[3]));
            let counts = entry.branches.entry(key).or_default();
            counts.0 += number(&branch[4]);
            counts.1 += number(&branch[5]);
        }
    }
}

/// What did not run in a file, in order of its lines.
#[must_use]
pub fn gaps(file: &FileCoverage) -> Vec<Gap> {
    let mut found: Vec<Gap> = file.lines.iter().filter(|(_, ran)| !**ran).map(|(line, _)| Gap::Line(*line)).collect();
    for (&(line, column, _, _), &(taken, not_taken)) in &file.branches {
        if taken == 0 || not_taken == 0 {
            found.push(Gap::Branch { line, column, taken: taken > 0, not_taken: not_taken > 0 });
        }
    }
    found
}

/// The list of native files: one path a line; `#` starts a comment.
#[must_use]
pub fn native_files(text: &str) -> BTreeSet<String> {
    text.lines().map(|line| line.split('#').next().unwrap_or_default().trim()).filter(|line| !line.is_empty()).map(str::to_owned).collect()
}

/// Whether a file is on the list: by its path, or under a folder, which ends in `/`.
fn listed(native: &BTreeSet<String>, name: &str) -> bool {
    native.contains(name) || native.iter().any(|path| path.ends_with('/') && name.starts_with(path.as_str()))
}

/// Reads the reports and says what is missing. `Ok` holds the number of files checked.
pub fn check(root: &Path, reports: &[String], native_list: &Path) -> Result<usize, String> {
    let native = native_files(&std::fs::read_to_string(native_list).map_err(|error| format!("{}: {error}", native_list.display()))?);
    let gone: Vec<&String> = native.iter().filter(|path| !root.join(path).exists()).collect();
    if !gone.is_empty() {
        return Err(format!("the list of native files names files that are gone: {gone:?}"));
    }
    let mut files = BTreeMap::new();
    for report in reports {
        let text = std::fs::read_to_string(report).map_err(|error| format!("{report}: {error}"))?;
        let value: Value = serde_json::from_str(&text).map_err(|error| format!("{report}: {error}"))?;
        merge(&mut files, &value);
    }
    let mut failures = Vec::new();
    for (name, file) in &files {
        if listed(&native, name) {
            continue;
        }
        let found = gaps(file);
        if !found.is_empty() {
            let shown: Vec<String> = found
                .iter()
                .map(|gap| match gap {
                    Gap::Line(line) => line.to_string(),
                    Gap::Branch { line, column, taken, not_taken } => {
                        format!("{line}:{column} branch ({})", if *taken { "never false" } else if *not_taken { "never true" } else { "never reached" })
                    }
                })
                .collect();
            failures.push(format!("{name}: {}", shown.join(", ")));
        }
    }
    if failures.is_empty() {
        return Ok(files.keys().filter(|name| !listed(&native, name)).count());
    }
    Err(format!("{} files have code no test ran:\n{}", failures.len(), failures.join("\n")))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use serde_json::json;

    use super::*;

    fn report(name: &str, segments: Value, branches: Value) -> Value {
        json!({"data": [{"files": [{"filename": name, "segments": segments, "branches": branches}]}]})
    }

    #[test]
    fn a_line_runs_when_any_report_saw_it_and_a_branch_adds_up() {
        let mut files = BTreeMap::new();
        let segments = |count: u64| json!([[1, 1, count, true, true, false], [1, 9, 0, false, false, false]]);
        merge(&mut files, &report("/a/crates/x/src/lib.rs", segments(0), json!([[1, 4, 1, 9, 0, 3, 0, 0, 4]])));
        let file = &files["crates/x/src/lib.rs"];
        assert_eq!(gaps(file), [Gap::Line(1), Gap::Branch { line: 1, column: 4, taken: false, not_taken: true }]);
        merge(&mut files, &report("C:\\b\\crates\\x\\src\\lib.rs", segments(2), json!([[1, 4, 1, 9, 5, 0, 0, 0, 4]])));
        assert!(gaps(&files["crates/x/src/lib.rs"]).is_empty());
    }

    #[test]
    fn a_region_running_through_a_line_counts_and_files_elsewhere_do_not() {
        let mut files = BTreeMap::new();
        let segments = json!([[1, 1, 4, true, true, false], [3, 1, 0, false, false, false]]);
        merge(&mut files, &report("/a/crates/x/src/lib.rs", segments, json!([])));
        merge(&mut files, &report("/rustc/library/core/src/lib.rs", json!([[1, 1, 0, true, true, false]]), json!([])));
        assert_eq!(files.len(), 1);
        let file = &files["crates/x/src/lib.rs"];
        assert_eq!(file.lines.len(), 3);
        assert!(gaps(file).is_empty());
        assert!(line_counts(&[]).is_empty());
        let skipped = [json!([1, 1, 0, false, true, false])];
        assert!(line_counts(&skipped).is_empty());
    }

    #[test]
    fn a_count_that_enters_no_region_or_fills_a_gap_is_not_a_line_count() {
        // On line 1 a count that is no region's entry, a gap region and a region; line 2 follows a segment without a count.
        let segments = [json!([1, 1, 5, true, false, false]), json!([1, 3, 7, true, true, true]), json!([1, 5, 2, true, true, false]), json!([1, 9, 0, false, false, false]), json!([3, 1, 1, true, true, false])];
        assert_eq!(line_counts(&segments), BTreeMap::from([(1, 2), (3, 1)]));
        let native = native_files("crates/x/\n");
        assert!(listed(&native, "crates/x/a.rs") && !listed(&native, "crates/y/a.rs"));
    }

    #[test]
    fn the_list_of_native_files_and_the_check() {
        assert_eq!(native_files("# all\n a.rs # why\n\n").into_iter().collect::<Vec<_>>(), ["a.rs"]);
        let folder = std::env::temp_dir().join(format!("ks-coverage-gate-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("crates/x")).unwrap();
        std::fs::write(folder.join("crates/x/native.rs"), "").unwrap();
        let list = folder.join("native.txt");
        std::fs::write(&list, "crates/x/native.rs\n").unwrap();
        let path = folder.join("report.json");
        let reports = [path.display().to_string()];
        let write = |name: &str, count: u64, branches: Value| {
            let segments = json!([[1, 1, count, true, true, false], [1, 9, 0, false, false, false]]);
            std::fs::write(&path, report(name, segments, branches).to_string()).unwrap();
        };
        write("/a/crates/x/native.rs", 0, json!([]));
        assert_eq!(check(&folder, &reports, &list), Ok(0));
        std::fs::write(&list, "crates/x/\n").unwrap();
        assert_eq!(check(&folder, &reports, &list), Ok(0));
        std::fs::write(&list, "crates/x/native.rs\n").unwrap();
        write("/a/crates/x/lib.rs", 1, json!([]));
        assert_eq!(check(&folder, &reports, &list), Ok(1));
        write("/a/crates/x/lib.rs", 0, json!([[1, 4, 1, 9, 0, 3, 0, 0, 4], [1, 5, 1, 9, 3, 0, 0, 0, 4], [1, 6, 1, 9, 0, 0, 0, 0, 4]]));
        let failure = check(&folder, &reports, &list).unwrap_err();
        assert!(failure.contains("crates/x/lib.rs: 1, 1:4 branch (never true), 1:5 branch (never false), 1:6 branch (never reached)"), "{failure}");
        std::fs::write(&list, "crates/x/gone.rs\n").unwrap();
        assert!(check(&folder, &reports, &list).unwrap_err().contains("gone"));
        assert!(check(&folder, &reports, &folder.join("nothing")).is_err());
        std::fs::write(&list, "").unwrap();
        assert!(check(&folder, &["nothing".to_owned()], &list).is_err());
        std::fs::write(&path, "not json").unwrap();
        assert!(check(&folder, &reports, &list).is_err());
    }
}

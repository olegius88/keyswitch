//! No tracked file may change its bytes on checkout: seals, receipts and manifests pin them, and a Windows checkout that
//! converted one would fail there alone.
use std::path::Path;
use std::process::Command;

#[test]
fn nothing_tracked_changes_its_line_endings() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let Ok(output) = Command::new("git").current_dir(&root).args(["ls-files", "--eol"]).output() else {
        eprintln!("no git here, skipped");
        return;
    };
    if !output.status.success() {
        eprintln!("not a git checkout, skipped");
        return;
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    let mut files = 0;
    for line in listing.lines() {
        let (columns, path) = line.split_once('\t').expect("a listing line");
        files += 1;
        let parts: Vec<&str> = columns.split_whitespace().collect();
        let [index, _, attributes @ ..] = parts.as_slice() else { panic!("an unexpected listing line: {line}") };
        assert!(!matches!(*index, "i/crlf" | "i/mixed"), "{path} has carriage returns in the index");
        let attributes = attributes.join(" ");
        assert!(attributes.contains("eol=lf") || attributes.contains("-text"), "{path} may change on checkout: {attributes}");
    }
    assert!(files > 0, "no tracked files");
}

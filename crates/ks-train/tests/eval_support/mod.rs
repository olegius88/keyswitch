//! Trees of links to this repository: nothing in one is written until `own` makes a file its own, so a test may change
//! a file of the evidence without touching the repository.
#![allow(dead_code, reason = "each test program uses what it needs of this")]
use serde_json::Value;
use std::path::{Path, PathBuf};

/// A tree of a link for each entry of the repository.
pub(crate) fn linked(name: &str) -> PathBuf {
    let folder = crate::support::scratch(name);
    for entry in std::fs::read_dir(crate::support::tree()).expect("the repository") {
        let entry = entry.expect("an entry");
        let file = entry.file_name();
        if matches!(file.to_str(), Some(".git" | ".t" | "target" | "build" | "dist")) {
            continue;
        }
        std::os::unix::fs::symlink(entry.path(), folder.join(&file)).expect("a link");
    }
    folder
}

/// `relative` as a file of the tree's own: every folder above it real with links for its other entries, and the file a
/// copy, so that writing it never writes through a link into the repository.
pub(crate) fn own(root: &Path, relative: &str) -> PathBuf {
    let mut at = root.to_path_buf();
    for part in Path::new(relative).components() {
        let next = at.join(part);
        if next.is_symlink() {
            let target = std::fs::read_link(&next).expect("a link");
            std::fs::remove_file(&next).expect("the link goes");
            if target.is_dir() {
                std::fs::create_dir(&next).expect("a folder");
                for entry in std::fs::read_dir(&target).expect("a folder") {
                    let entry = entry.expect("an entry");
                    std::os::unix::fs::symlink(entry.path(), next.join(entry.file_name())).expect("a link");
                }
            } else {
                std::fs::copy(&target, &next).expect("a copy");
            }
        }
        at = next;
    }
    at
}

pub(crate) fn read(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("a file")).expect("json")
}

/// A JSON file of the tree changed in place, as a file of its own.
pub(crate) fn edit(root: &Path, relative: &str, change: impl FnOnce(&mut Value)) {
    let mut value = read(&root.join(relative));
    change(&mut value);
    std::fs::write(own(root, relative), serde_json::to_vec(&value).expect("json")).expect("a file");
}

pub(crate) fn sha(root: &Path, relative: &str) -> String {
    crate::support::digest(&std::fs::read(root.join(relative)).expect("a file"))
}

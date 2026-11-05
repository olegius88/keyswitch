//! The licences of the crates compiled into the programs. Rust links them statically, so a package has to carry their
//! notices. The lists are generated from the lockfile for each system and kept in the tree; `--verify` says when they
//! fall behind.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

const FOLDER: &str = "packaging/third-party-licenses";
/// The programs of each system and what they are compiled for: a crate of any of them is listed.
const SYSTEMS: [(&str, &[&str], &[&str]); 3] = [
    ("linux", &["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"], &["ks-linux", "logcourier"]),
    ("windows", &["x86_64-pc-windows-msvc", "i686-pc-windows-msvc", "aarch64-pc-windows-msvc"], &["ks-win", "logcourier"]),
    ("macos", &["x86_64-apple-darwin", "aarch64-apple-darwin"], &["ks-mac"]),
];
const NOTICE_PREFIXES: [&str; 5] = ["license", "licence", "copying", "notice", "authors"];

fn metadata(root: &Path, target: &str) -> Result<Value, String> {
    let output = Command::new("cargo").current_dir(root).args(["metadata", "--format-version", "1", "--locked", "--filter-platform", target]).output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())
}

/// The packages reached from these programs through normal dependencies: what is compiled into them.
fn reached(meta: &Value, roots: &[&str]) -> Result<BTreeSet<String>, String> {
    let nodes = meta["resolve"]["nodes"].as_array().ok_or("cargo metadata without a resolve graph")?;
    let packages = meta["packages"].as_array().ok_or("cargo metadata without packages")?;
    let by_id: BTreeMap<&str, &Value> = nodes.iter().filter_map(|node| node["id"].as_str().map(|id| (id, node))).collect();
    let mut pending: Vec<String> = packages.iter().filter(|package| package["name"].as_str().is_some_and(|name| roots.contains(&name))).filter_map(|package| package["id"].as_str().map(str::to_owned)).collect();
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(node) = by_id.get(id.as_str()) else { continue };
        for dependency in node["deps"].as_array().into_iter().flatten() {
            let normal = dependency["dep_kinds"].as_array().into_iter().flatten().any(|kind| kind["kind"].is_null());
            if normal {
                if let Some(next) = dependency["pkg"].as_str() {
                    pending.push(next.to_owned());
                }
            }
        }
    }
    Ok(seen)
}

fn notice_files(directory: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.file_name().and_then(|name| name.to_str()).is_some_and(|name| { let lowered = name.to_ascii_lowercase(); NOTICE_PREFIXES.iter().any(|prefix| lowered.starts_with(prefix)) }))
        .collect();
    found.sort();
    found
}

fn generate(root: &Path, targets: &[&str], programs: &[&str]) -> Result<String, String> {
    let mut crates: BTreeMap<(String, String), (String, String, Vec<PathBuf>)> = BTreeMap::new();
    for target in targets {
        let meta = metadata(root, target)?;
        let ids = reached(&meta, programs)?;
        for package in meta["packages"].as_array().into_iter().flatten() {
            let (Some(id), Some(name), Some(version)) = (package["id"].as_str(), package["name"].as_str(), package["version"].as_str()) else { continue };
            // The workspace's own crates are under the project's licence.
            if !ids.contains(id) || package["source"].is_null() {
                continue;
            }
            let manifest = PathBuf::from(package["manifest_path"].as_str().unwrap_or_default());
            let directory = manifest.parent().ok_or("a manifest outside a folder")?;
            let licence = package["license"].as_str().unwrap_or("").to_owned();
            let home = package["repository"].as_str().unwrap_or("").to_owned();
            crates.insert((name.to_owned(), version.to_owned()), (licence, home, notice_files(directory)));
        }
    }
    let mut text = String::from("The components compiled into this program, with the notices their authors ask to keep.\n");
    for ((name, version), (licence, home, files)) in &crates {
        text.push_str(&format!("\n================================================================================\n{name} {version}\nLicense: {licence}\nSource: {home}\n"));
        for file in files {
            let content = std::fs::read(file).map_err(|error| error.to_string())?;
            text.push_str(&format!("\n--- {} ---\n{}\n", file.file_name().and_then(|name| name.to_str()).unwrap_or_default(), String::from_utf8_lossy(&content).replace("\r\n", "\n").trim_end()));
        }
    }
    Ok(text)
}

/// Writes the lists, or with `verify` compares them with what the lockfile gives.
pub fn command(root: &Path, verify: bool) -> Result<(), String> {
    let folder = root.join(FOLDER);
    for (system, targets, programs) in SYSTEMS {
        let text = generate(root, targets, programs)?;
        let path = folder.join(format!("{system}.txt"));
        if verify {
            let known = std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            if known != text {
                return Err(format!("{} fell behind the lockfile: run ks-train licenses --tree .", path.display()));
            }
        } else {
            std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
            std::fs::write(&path, &text).map_err(|error| error.to_string())?;
        }
        println!("{system}: {} crates, {} bytes", text.matches("\n================").count(), text.len());
    }
    Ok(())
}

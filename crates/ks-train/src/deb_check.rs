//! What a Debian package has to hold before anyone installs it: the exact frozen models, the licences, the repository
//! source and its public key, nothing enabled behind the user's back, a program that links and starts and loads the very
//! model it carries.
use crate::corpus::checksum;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const INTENT_MODEL_MAX_BYTES: usize = 14 * 1024 * 1024;
const INTENT_MANIFEST_MAX_BYTES: usize = 1024 * 1024;
const INTENT_PAYLOAD_MAX_BYTES: usize = 12 * 1024 * 1024;
const INTENT_FINGERPRINT_MAX_COUNT: u64 = 1 << 20;
const KSLM_HEADER_BYTES: usize = 52;
const KSLM_MANIFEST_LENGTH_AT: usize = 8;
const KSLM_PAYLOAD_LENGTH_AT: usize = 12;
const FINGERPRINT_BYTES: u64 = 8;
const WEIGHT_BYTES: u64 = 2;
const GNOME_EXTENSION: &str = "keyswitch@io.github.olegius88.KeySwitch";
const DIAGNOSE_SECONDS: &str = "30s";
const ONBOARD_NOTICES: [&str; 4] = ["Files: models/*", "Copyright: 2013, 2014, marmuta <marmvta@gmail.com>", "  2011, 2012, Francesco Fumanti <francesco.fumanti@gmx.net>", "License: GPL-3+"];

fn run(program: &str, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new(program).args(arguments).stderr(Stdio::inherit()).output().map_err(|error| format!("{program}: {error}"))?;
    if !output.status.success() {
        return Err(format!("{program} {arguments:?} ended with {}", output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn same(left: &Path, right: &Path) -> Result<(), String> {
    let read = |path: &Path| std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()));
    if read(left)? != read(right)? {
        return Err(format!("{} differs from {}", right.display(), left.display()));
    }
    Ok(())
}

/// The container's bounds, and that its payload has the shape its own manifest declares. Returns the model's version.
fn kslm_version(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > INTENT_MODEL_MAX_BYTES {
        return Err("KSLM container exceeds its packaging bound".to_owned());
    }
    if bytes.len() < KSLM_HEADER_BYTES || &bytes[..4] != b"KSLM" {
        return Err("KSLM header is truncated".to_owned());
    }
    let field = |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
    let (manifest_length, payload_length) = (field(KSLM_MANIFEST_LENGTH_AT), field(KSLM_PAYLOAD_LENGTH_AT));
    if !(2..=INTENT_MANIFEST_MAX_BYTES).contains(&manifest_length) || payload_length > INTENT_PAYLOAD_MAX_BYTES || bytes.len() != KSLM_HEADER_BYTES + manifest_length + payload_length {
        return Err("KSLM container has invalid bounds".to_owned());
    }
    let manifest: Value = serde_json::from_slice(&bytes[KSLM_HEADER_BYTES..KSLM_HEADER_BYTES + manifest_length]).map_err(|error| error.to_string())?;
    let (fingerprints, dimension) = (manifest["supported_fingerprint_count"].as_u64().unwrap_or(u64::MAX), manifest["dimension"].as_u64().unwrap_or(0));
    if fingerprints > INTENT_FINGERPRINT_MAX_COUNT || dimension == 0 || payload_length as u64 != dimension * WEIGHT_BYTES + fingerprints * FINGERPRINT_BYTES {
        return Err("KSLM payload shape does not match its embedded manifest".to_owned());
    }
    manifest["model_version"].as_str().map(str::to_owned).ok_or_else(|| "a manifest without a version".to_owned())
}

fn is_elf(path: &Path) -> bool {
    std::fs::File::open(path).ok().is_some_and(|mut file| {
        let mut magic = [0_u8; 4];
        std::io::Read::read_exact(&mut file, &mut magic).is_ok() && &magic == b"\x7fELF"
    })
}

fn walk(folder: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, found);
        } else {
            found.push(path);
        }
    }
}

pub fn verify(root: &Path, package: &Path) -> Result<String, String> {
    let deb = package.to_str().ok_or("a package path that is no text")?;
    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|error| error.to_string())?;
    let version = cargo.lines().find_map(|line| line.strip_prefix("version = \"").and_then(|rest| rest.strip_suffix('"'))).ok_or("no version in Cargo.toml")?;
    let architecture = run("dpkg", &["--print-architecture"])?;
    for (field, wanted) in [("Package", "keyswitch"), ("Version", version), ("Architecture", architecture.as_str())] {
        let found = run("dpkg-deb", &["--field", deb, field])?;
        if found != wanted {
            return Err(format!("the package's {field} is {found}, not {wanted}"));
        }
    }
    let depends = run("dpkg-deb", &["--field", deb, "Depends"])?;
    if depends.split([',', ' ', ':']).any(|word| word == "python3") {
        return Err("the package depends on the Python interpreter".to_owned());
    }
    let staging = std::env::temp_dir().join(format!("keyswitch-deb-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
    let result = check_extracted(root, deb, &staging, version);
    let _ = std::fs::remove_dir_all(&staging);
    result
}

fn check_extracted(root: &Path, deb: &str, staging: &Path, version: &str) -> Result<String, String> {
    let unpacked = staging.join("root");
    let control = staging.join("control");
    let as_text = |path: &Path| path.to_str().map(str::to_owned).ok_or_else(|| "a path that is no text".to_owned());
    run("dpkg-deb", &["--extract", deb, &as_text(&unpacked)?])?;
    run("dpkg-deb", &["--control", deb, &as_text(&control)?])?;
    let (binary, resources) = (unpacked.join("usr/bin/keyswitch"), unpacked.join("usr/lib/keyswitch/resources"));
    let frozen = root.join("model/intent_v1/sources");
    let intent = resources.join("models/layout_intent_v1.ksm");
    let version_of_model = kslm_version(&std::fs::read(&intent).map_err(|error| error.to_string())?)?;
    for (known, packed) in [
        (root.join("resources/models/layout_intent_v1.ksm"), intent.clone()),
        (root.join("resources/models/context_policy_v1.json"), resources.join("models/context_policy_v1.json")),
        (root.join("resources/models/prefix_policy_v1.json"), resources.join("models/prefix_policy_v1.json")),
        (root.join("resources/models/boundary-v2.json"), resources.join("models/boundary-v2.json")),
        (root.join("resources/models/ortho_v1.json"), resources.join("models/ortho_v1.json")),
        (frozen.join("en_US.lm"), resources.join("models/en_US.lm")),
        (frozen.join("ru_RU.lm"), resources.join("models/ru_RU.lm")),
        (frozen.join("COPYRIGHT.onboard-data"), unpacked.join("usr/share/doc/keyswitch/licenses/copyright.onboard-data")),
        (root.join("packaging/third-party-licenses/linux.txt"), unpacked.join("usr/share/doc/keyswitch/licenses/third-party-rust.txt")),
        (root.join("resources/wayland-input/60-keyswitch.rules"), resources.join("wayland-input/60-keyswitch.rules")),
        (root.join("resources/wayland-input/keyswitch.modules-load.conf"), resources.join("wayland-input/keyswitch.modules-load.conf")),
        (root.join(format!("packaging/gnome-shell/{GNOME_EXTENSION}/metadata.json")), unpacked.join(format!("usr/share/gnome-shell/extensions/{GNOME_EXTENSION}/metadata.json"))),
        (root.join(format!("packaging/gnome-shell/{GNOME_EXTENSION}/extension.js")), unpacked.join(format!("usr/share/gnome-shell/extensions/{GNOME_EXTENSION}/extension.js"))),
    ] {
        same(&known, &packed)?;
    }
    let onboard = std::fs::read_to_string(unpacked.join("usr/share/doc/keyswitch/licenses/copyright.onboard-data")).map_err(|error| error.to_string())?;
    for line in ONBOARD_NOTICES {
        if !onboard.lines().any(|found| found == line) {
            return Err(format!("the word lists' notice lost a line: {line}"));
        }
    }
    // The Wayland input rule ships for `keyswitch --enable-wayland-input`; nothing enables it on install.
    for absent in [control.join("postinst"), unpacked.join("usr/lib/udev/rules.d/60-keyswitch.rules"), unpacked.join("usr/lib/modules-load.d/keyswitch.conf")] {
        if absent.exists() {
            return Err(format!("the package installs what it must only offer: {}", absent.display()));
        }
    }
    // An installed KeySwitch keeps itself upgradable through APT: the source as a conffile and the public key it trusts.
    let sources = std::fs::read_to_string(root.join("packaging/debian/keyswitch.sources")).map_err(|error| error.to_string())?;
    let field = |name: &str| sources.lines().find_map(|line| line.strip_prefix(name)).map(|rest| rest.trim().to_owned()).ok_or_else(|| format!("the APT source has no {name}"));
    let (uri, keyring) = (field("URIs:")?, field("Signed-By:")?);
    let source_path = "/etc/apt/sources.list.d/keyswitch.sources";
    let conffiles = std::fs::read_to_string(control.join("conffiles")).map_err(|error| error.to_string())?;
    for path in [source_path, keyring.as_str()] {
        if !conffiles.lines().any(|line| line == path) {
            return Err(format!("{path} is not a conffile"));
        }
    }
    same(&root.join("packaging/debian/keyswitch.sources"), &unpacked.join(source_path.trim_start_matches('/')))?;
    same(&root.join("packaging/keyswitch-archive-keyring.asc"), &unpacked.join(keyring.trim_start_matches('/')))?;
    if !uri.starts_with("https://") || !uri.ends_with('/') {
        return Err(format!("the packaged APT source does not serve HTTPS from a directory: {uri}"));
    }
    let key = std::fs::read_to_string(unpacked.join(keyring.trim_start_matches('/'))).map_err(|error| error.to_string())?;
    if !key.contains("BEGIN PGP PUBLIC KEY BLOCK") || key.contains("PRIVATE KEY BLOCK") {
        return Err("the packaged APT keyring must hold the public key alone".to_owned());
    }
    if !is_elf(&binary) {
        return Err("the packaged program is no ELF executable".to_owned());
    }
    let mut files = Vec::new();
    walk(&unpacked, &mut files);
    if let Some(script) = files.iter().find(|path| path.extension().is_some_and(|extension| extension == "py" || extension == "pyc" || extension == "pyo")) {
        return Err(format!("the package contains Python: {}", script.display()));
    }
    let program = as_text(&binary)?;
    let announced = run(&program, &["--version"])?;
    if announced != format!("KeySwitch {version}") {
        return Err(format!("the program says {announced}"));
    }
    let mut linked = 0;
    for file in files.iter().filter(|path| is_elf(path)) {
        linked += 1;
        let libraries = run("ldd", &[&as_text(file)?])?;
        if libraries.contains("not found") {
            return Err(format!("{} has unresolved shared libraries:\n{libraries}", file.display()));
        }
    }
    // The diagnostic reports the keyboard's absence by its status: the model is what this reads.
    let diagnostic = Command::new("timeout")
        .args([DIAGNOSE_SECONDS, &program, "--diagnose"])
        .env("KEYSWITCH_DATA_DIR", staging.join("diagnostic-data"))
        .env("KEYSWITCH_RESOURCES", &resources)
        .env("KEYSWITCH_INTENT_MODEL_PATH", "")
        .output()
        .map_err(|error| error.to_string())?;
    if !matches!(diagnostic.status.code(), Some(0 | 1)) {
        return Err(format!("the packaged diagnostic ended with {}: {}", diagnostic.status, String::from_utf8_lossy(&diagnostic.stderr)));
    }
    let report: Value = serde_json::from_slice(&diagnostic.stdout).map_err(|error| format!("the diagnostic: {error}"))?;
    let model = &report["intent_model"];
    let path = model["path"].as_str().map(Path::new).ok_or("the diagnostic names no model")?;
    if model["available"] != Value::Bool(true) || path.canonicalize().ok() != intent.canonicalize().ok() || model["checksum"].as_str() != Some(checksum(&intent)?.as_str()) || model["version"].as_str() != Some(version_of_model.as_str()) {
        return Err(format!("the packaged diagnostic did not load the exact bundled intent model: {model}"));
    }
    Ok(format!("NATIVE_DEB_OK package={deb} elf_files={linked} intent_model={version_of_model} apt_source={uri}"))
}

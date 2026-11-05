//! The behaviour contract of the updater, over a fake opener (the cases of the Python tests that matter).

#![expect(clippy::unwrap_used, reason = "a test or a probe stops on what it did not expect")]
use ks_cast::To as _;
use std::collections::VecDeque;
use std::fmt::Display;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ks_updates::constants::MAX_ASSET_BYTES;
use ks_updates::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const DOWNLOAD_URL: &str = "https://release-assets.githubusercontent.com/github-production-release-asset/1/file";

struct FakeResponse {
    data: Vec<u8>,
    position: usize,
    url: String,
    read_error: bool,
    closed: Arc<AtomicUsize>,
}

impl FakeResponse {
    fn new(data: &[u8], url: &str) -> (Self, Arc<AtomicUsize>) {
        let closed = Arc::new(AtomicUsize::new(0));
        let response = Self { data: data.to_vec(), position: 0, url: url.into(), read_error: false, closed: closed.clone() };
        (response, closed)
    }
}

impl Drop for FakeResponse {
    fn drop(&mut self) {
        self.closed.fetch_add(1, Ordering::SeqCst);
    }
}

impl UpdateResponse for FakeResponse {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.read_error {
            return Err(io::Error::other("connection reset"));
        }
        // Small reads, so that the limits and progress see several chunks.
        let count = buffer.len().min(4).min(self.data.len() - self.position);
        buffer[..count].copy_from_slice(&self.data[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }

    fn url(&self) -> String {
        self.url.clone()
    }
}

#[derive(Default)]
struct FakeOpener {
    queue: Mutex<VecDeque<io::Result<FakeResponse>>>,
    requests: Mutex<Vec<UpdateRequest>>,
}

impl FakeOpener {
    fn with(results: Vec<io::Result<FakeResponse>>) -> Arc<Self> {
        Arc::new(Self { queue: Mutex::new(results.into()), requests: Mutex::default() })
    }

    fn opened(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl ResponseOpener for FakeOpener {
    fn open(&self, request: &UpdateRequest, _timeout: Duration) -> io::Result<Box<dyn UpdateResponse>> {
        self.requests.lock().unwrap().push(request.clone());
        match self.queue.lock().unwrap().pop_front() {
            Some(Ok(response)) => Ok(Box::new(response)),
            Some(Err(error)) => Err(error),
            None => Err(io::Error::other("no response queued")),
        }
    }
}

fn sha_hex(content: &[u8]) -> String {
    Sha256::digest(content).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn release_payload(version: &str, target: UpdateTarget, content: &[u8]) -> Value {
    let parsed = SemanticVersion::parse(version).unwrap();
    let name = target.asset_name(parsed);
    json!({
        "tag_name": format!("v{version}"),
        "draft": false,
        "prerelease": false,
        "html_url": format!("https://github.com/olegius88/keyswitch/releases/tag/v{version}"),
        "body": "notes",
        "assets": [{
            "name": name,
            "state": "uploaded",
            "size": content.len(),
            "digest": format!("sha256:{}", sha_hex(content)),
            "browser_download_url": format!("https://github.com/olegius88/keyswitch/releases/download/v{version}/{name}"),
        }],
    })
}

#[expect(clippy::unnecessary_wraps, reason = "the signature a fake response has")]
fn json_response(value: &Value) -> io::Result<FakeResponse> {
    Ok(FakeResponse::new(value.to_string().as_bytes(), LATEST_RELEASE_URL).0)
}

fn latest(payload: &Value) -> Result<UpdateRelease, UpdateError> {
    let opener = FakeOpener::with(vec![json_response(payload)]);
    GitHubReleaseClient::new("0.4.0", opener).latest_release(UpdateTarget::WindowsX64)
}

fn temp_dir(label: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!("ks-updates-{}-{label}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn checked_release(version: &str, target: UpdateTarget, content: &[u8]) -> UpdateRelease {
    latest_for(&release_payload(version, target, content), target).unwrap()
}

fn latest_for(payload: &Value, target: UpdateTarget) -> Result<UpdateRelease, UpdateError> {
    let opener = FakeOpener::with(vec![json_response(payload)]);
    GitHubReleaseClient::new("0.4.0", opener).latest_release(target)
}

#[test]
fn versions_targets_and_platform_detection() {
    assert_eq!(SemanticVersion::parse("v1.2.3").unwrap().to_string(), "1.2.3");
    assert!(SemanticVersion::parse("0.9.9").unwrap() < SemanticVersion::parse("1.0.0").unwrap());
    assert!(SemanticVersion::parse("0.70.0").unwrap() > SemanticVersion::parse("0.66.1").unwrap());
    for invalid in ["", "1.2", "01.2.3", "1.2.3-beta", "1.2.3.4", "vv1.2.3", "1.2.x", "+1.2.3", "１.2.3"] {
        assert!(SemanticVersion::parse(invalid).is_err(), "{invalid}");
    }
    let version = SemanticVersion::parse("2.3.4").unwrap();
    assert_eq!(UpdateTarget::WindowsX64.asset_name(version), "KeySwitch-Setup-2.3.4-x64.exe");
    assert_eq!(UpdateTarget::WindowsArm64.asset_name(version), "KeySwitch-Setup-2.3.4-arm64.exe");
    assert_eq!(UpdateTarget::WindowsX86.asset_name(version), "KeySwitch-Setup-2.3.4-x86.exe");
    assert_eq!(UpdateTarget::UbuntuAmd64.asset_name(version), "keyswitch_2.3.4_amd64.deb");
    assert!(UpdateTarget::WindowsX64.windows_installer() && !UpdateTarget::UbuntuAmd64.windows_installer());
    for (platform, target) in [
        ("win-amd64", UpdateTarget::WindowsX64),
        ("win-arm64", UpdateTarget::WindowsArm64),
        ("win32", UpdateTarget::WindowsX86),
    ] {
        for machine in ["AMD64", "ARM64", "x86"] {
            assert_eq!(detect_update_target("Windows", machine, platform), Some(target));
        }
    }
    assert_eq!(detect_update_target("Windows", "ARM", "win-arm32"), None);
    assert_eq!(detect_update_target("Linux", "x86_64", ""), Some(UpdateTarget::UbuntuAmd64));
    assert_eq!(detect_update_target("Linux", "amd64", ""), Some(UpdateTarget::UbuntuAmd64));
    assert_eq!(detect_update_target("Linux", "aarch64", ""), None);
    assert_eq!(detect_update_target("Darwin", "x86_64", ""), None);
}

#[test]
fn success_validates_request_and_platform_assets() {
    let content = b"package";
    for target in [UpdateTarget::WindowsX64, UpdateTarget::WindowsArm64, UpdateTarget::WindowsX86, UpdateTarget::UbuntuAmd64] {
        let opener = FakeOpener::with(vec![json_response(&release_payload("1.0.0", target, content))]);
        let release = GitHubReleaseClient::new("0.4.0", opener.clone()).latest_release(target).unwrap();
        assert_eq!(release.version.to_string(), "1.0.0");
        assert_eq!(release.asset.name, target.asset_name(release.version));
        assert_eq!(release.asset.size, content.len().to::<u64>());
        assert_eq!(release.asset.sha256, sha_hex(content));
        let requests = opener.requests.lock().unwrap();
        assert_eq!(requests[0].url, LATEST_RELEASE_URL);
        assert!(requests[0].headers.contains(&("User-Agent".into(), "KeySwitch/0.4.0".into())));
        assert!(requests[0].headers.contains(&("X-GitHub-Api-Version".into(), GITHUB_API_VERSION.into())));
    }
}

#[test]
fn a_release_without_the_asset_of_this_architecture_is_reported() {
    let payload = release_payload("1.0.0", UpdateTarget::WindowsX64, b"x");
    let error = latest_for(&payload, UpdateTarget::WindowsArm64).unwrap_err();
    assert_eq!(error.0, "В выпуске отсутствует единственный файл KeySwitch-Setup-1.0.0-arm64.exe");
    let mut doubled = payload.clone();
    let asset = doubled["assets"][0].clone();
    doubled["assets"].as_array_mut().unwrap().push(asset);
    assert!(latest(&doubled).is_err());
}

#[test]
fn transport_encoding_and_response_limits_fail_closed() {
    let wrong_url = FakeResponse::new(release_payload("1.0.0", UpdateTarget::WindowsX64, b"x").to_string().as_bytes(), "https://example.invalid/latest").0;
    let oversized = vec![b'x'; constants::MAX_RELEASE_JSON_BYTES + 1];
    let cases: Vec<(io::Result<FakeResponse>, bool)> = vec![
        (Err(io::Error::other("offline")), false),
        (Ok(FakeResponse::new(b"\xff", LATEST_RELEASE_URL).0), true),
        (Ok(FakeResponse::new(b"not-json", LATEST_RELEASE_URL).0), true),
        (Ok(FakeResponse::new(&oversized, LATEST_RELEASE_URL).0), true),
        (Ok(wrong_url), true),
    ];
    for (result, has_response) in cases {
        let closed = result.as_ref().ok().map(|response| response.closed.clone());
        let opener = FakeOpener::with(vec![result]);
        let outcome = GitHubReleaseClient::new("0.4.0", opener).latest_release(UpdateTarget::WindowsX64);
        assert!(outcome.is_err());
        if has_response {
            assert_eq!(closed.unwrap().load(Ordering::SeqCst), 1);
        }
    }
}

#[test]
fn release_and_asset_schema_rejections() {
    let base = release_payload("1.0.0", UpdateTarget::WindowsX64, b"package");
    assert!(latest(&base).is_ok());
    let edits: Vec<(&str, Box<dyn Fn(&mut Value)>)> = vec![
        ("not an object", Box::new(|v| *v = json!([]))),
        ("draft", Box::new(|v| v["draft"] = json!(true))),
        ("draft missing", Box::new(|v| { v.as_object_mut().unwrap().remove("draft"); })),
        ("prerelease", Box::new(|v| v["prerelease"] = json!(true))),
        ("tag without v", Box::new(|v| v["tag_name"] = json!("1.0.0"))),
        ("tag with suffix", Box::new(|v| v["tag_name"] = json!("v1.0.0-rc1"))),
        ("tag not a string", Box::new(|v| v["tag_name"] = json!(7))),
        ("page of another repository", Box::new(|v| v["html_url"] = json!("https://github.com/evil/keyswitch/releases/tag/v1.0.0"))),
        ("assets not a list", Box::new(|v| v["assets"] = json!({}))),
        ("asset not an object", Box::new(|v| v["assets"] = json!([1]))),
        ("not uploaded", Box::new(|v| v["assets"][0]["state"] = json!("starter"))),
        ("foreign download address", Box::new(|v| v["assets"][0]["browser_download_url"] = json!("https://example.invalid/a.exe"))),
        ("no digest", Box::new(|v| { v["assets"][0].as_object_mut().unwrap().remove("digest"); })),
        ("short digest", Box::new(|v| v["assets"][0]["digest"] = json!("sha256:abc"))),
        ("upper-case digest", Box::new(|v| v["assets"][0]["digest"] = json!(format!("sha256:{}", "A".repeat(64))))),
        ("sha1 digest", Box::new(|v| v["assets"][0]["digest"] = json!(format!("sha1:{}", "a".repeat(64))))),
        ("size true", Box::new(|v| v["assets"][0]["size"] = json!(true))),
        ("size text", Box::new(|v| v["assets"][0]["size"] = json!("3"))),
        ("size zero", Box::new(|v| v["assets"][0]["size"] = json!(0))),
        ("size float", Box::new(|v| v["assets"][0]["size"] = json!(3.0))),
        ("size over the limit", Box::new(|v| v["assets"][0]["size"] = json!(MAX_ASSET_BYTES + 1))),
    ];
    for (label, edit) in edits {
        let mut payload = base.clone();
        edit(&mut payload);
        assert!(latest(&payload).is_err(), "{label}");
    }
    let mut long_notes = base.clone();
    long_notes["body"] = json!("я".repeat(5000));
    assert_eq!(latest(&long_notes).unwrap().notes.chars().count(), 4000);
    let mut no_notes = base;
    no_notes["body"] = json!(5);
    assert_eq!(latest(&no_notes).unwrap().notes, "");
}

#[test]
fn download_success_and_progress() {
    let content = b"verified package";
    let release = checked_release("1.0.0", UpdateTarget::WindowsX64, content);
    let directory = temp_dir("ok");
    let partial = directory.join(format!("{}.part", release.asset.name));
    std::fs::write(&partial, b"stale").unwrap();
    let (response, closed) = FakeResponse::new(content, DOWNLOAD_URL);
    let client = GitHubReleaseClient::new("0.4.0", FakeOpener::with(vec![Ok(response)]));
    let mut progress = Vec::new();
    let installed = client.download(&release, &directory, &mut |done, total| progress.push((done, total))).unwrap();
    assert_eq!(std::fs::read(&installed).unwrap(), content);
    assert!(!partial.exists());
    assert_eq!(*progress.last().unwrap(), (content.len().to::<u64>(), content.len().to::<u64>()));
    assert_eq!(closed.load(Ordering::SeqCst), 1);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn every_integrity_failure_deletes_the_file_and_reports_an_error() {
    let content = b"verified package";
    let release = checked_release("1.0.0", UpdateTarget::WindowsX64, content);
    let directory = temp_dir("bad");
    let partial = directory.join(format!("{}.part", release.asset.name));
    let installer = directory.join(&release.asset.name);
    let with = |size: u64, sha: &str| {
        let mut changed = release.clone();
        changed.asset.size = size;
        changed.asset.sha256 = sha.to_string();
        changed
    };
    let length = content.len().to::<u64>();
    let zeros = "0".repeat(64);
    let mut read_error = FakeResponse::new(content, DOWNLOAD_URL).0;
    read_error.read_error = true;
    let failures: Vec<(UpdateRelease, FakeResponse, &str)> = vec![
        (with(3, &release.asset.sha256), FakeResponse::new(content, DOWNLOAD_URL).0, "превышает объявленный"),
        (with(length + 1, &release.asset.sha256), FakeResponse::new(content, DOWNLOAD_URL).0, "не совпадает с объявленным"),
        (with(length, &zeros), FakeResponse::new(content, DOWNLOAD_URL).0, "SHA-256 не совпала"),
        (release.clone(), FakeResponse::new(content, "https://example.invalid/file").0, "недоверенный адрес"),
        (release.clone(), read_error, "connection reset"),
    ];
    for (failed, response, text) in failures {
        let closed = response.closed.clone();
        let client = GitHubReleaseClient::new("0.4.0", FakeOpener::with(vec![Ok(response)]));
        let error = client.download(&failed, &directory, &mut |_, _| {}).unwrap_err();
        assert!(error.0.contains(text), "{error}");
        assert!(!partial.exists() && !installer.exists());
        assert_eq!(closed.load(Ordering::SeqCst), 1);
    }
    let offline = GitHubReleaseClient::new("0.4.0", FakeOpener::with(vec![Err(io::Error::other("offline"))]));
    assert!(offline.download(&release, &directory, &mut |_, _| {}).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_download_removes_the_installers_of_earlier_updates() {
    let content = b"new";
    let release = checked_release("1.0.0", UpdateTarget::WindowsX64, content);
    let directory = temp_dir("old");
    let doomed = ["KeySwitch-Setup-0.9.0-x64.exe", "KeySwitch-Setup-0.9.0-arm64.exe", "KeySwitch-Setup-10.20.30-x86.exe"];
    let kept = ["notes.txt", "KeySwitch-Setup-0.9.0-x64.exe.part", "KeySwitch-Setup-09.0.0-x64.exe", "KeySwitch-Setup-0.9.0-x64.EXE", "KeySwitch-Setup-0.9-x64.exe", "other.exe"];
    for name in doomed.iter().chain(&kept) {
        std::fs::write(directory.join(name), b"old").unwrap();
    }
    let (response, _) = FakeResponse::new(content, DOWNLOAD_URL);
    let client = GitHubReleaseClient::new("0.4.0", FakeOpener::with(vec![Ok(response)]));
    client.download(&release, &directory, &mut |_, _| {}).unwrap();
    for name in doomed {
        assert!(!directory.join(name).exists(), "{name}");
    }
    for name in kept {
        assert!(directory.join(name).exists(), "{name}");
    }
    assert!(directory.join(&release.asset.name).exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn response_urls_are_validated() {
    // Through the client: only the exact API address and the release-assets host pass.
    let content = b"x";
    let release = checked_release("1.0.0", UpdateTarget::WindowsX64, content);
    let directory = temp_dir("urls");
    for (url, accepted) in [
        (DOWNLOAD_URL, true),
        ("HTTPS://Release-Assets.GitHubUserContent.com/github-production-release-asset/1/f", true),
        ("https://release-assets.githubusercontent.com:443/github-production-release-asset/1/f?x=1", true),
        ("http://release-assets.githubusercontent.com/github-production-release-asset/1/f", false),
        ("https://example.invalid/github-production-release-asset/1/file", false),
        ("https://release-assets.githubusercontent.com/untrusted/file", false),
        ("https://release-assets.githubusercontent.com.evil.test/github-production-release-asset/1/f", false),
        ("", false),
    ] {
        let (response, _) = FakeResponse::new(content, url);
        let client = GitHubReleaseClient::new("0.4.0", FakeOpener::with(vec![Ok(response)]));
        assert_eq!(client.download(&release, &directory, &mut |_, _| {}).is_ok(), accepted, "{url}");
    }
    std::fs::remove_dir_all(directory).unwrap();
}

// ---- the manager ----

struct FakeReleaseClient {
    release: Mutex<UpdateRelease>,
    latest_targets: Mutex<Vec<UpdateTarget>>,
    downloads: AtomicUsize,
    latest_error: Mutex<Option<String>>,
    download_error: Mutex<Option<String>>,
    download_hook: Mutex<Option<Box<dyn Fn() + Send>>>,
}

impl FakeReleaseClient {
    fn new(release: UpdateRelease) -> Arc<Self> {
        Arc::new(Self {
            release: Mutex::new(release),
            latest_targets: Mutex::default(),
            downloads: AtomicUsize::new(0),
            latest_error: Mutex::default(),
            download_error: Mutex::default(),
            download_hook: Mutex::default(),
        })
    }
}

impl ReleaseClient for FakeReleaseClient {
    fn latest_release(&self, target: UpdateTarget) -> Result<UpdateRelease, UpdateError> {
        self.latest_targets.lock().unwrap().push(target);
        if let Some(message) = self.latest_error.lock().unwrap().clone() {
            return Err(UpdateError::new(message));
        }
        Ok(self.release.lock().unwrap().clone())
    }

    fn download(&self, release: &UpdateRelease, directory: &Path, progress: &mut dyn FnMut(u64, u64)) -> Result<PathBuf, UpdateError> {
        self.downloads.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = self.download_hook.lock().unwrap().as_ref() {
            hook();
        }
        if let Some(message) = self.download_error.lock().unwrap().clone() {
            return Err(UpdateError::new(message));
        }
        progress(release.asset.size / 2, release.asset.size);
        progress(release.asset.size, release.asset.size);
        Ok(directory.join(&release.asset.name))
    }
}

fn immediate() -> BackgroundRunner {
    Arc::new(|job| {
        job();
        Ok(())
    })
}

fn manager(current: &str, client: Arc<dyn ReleaseClient>, target: UpdateTarget) -> UpdateManager {
    UpdateManager::new(current, PathBuf::from("updates"), client)
        .unwrap()
        .with_target(Some(target))
        .with_runner(immediate())
}

fn recording_installer() -> (InstallRequest, Arc<Mutex<Vec<PathBuf>>>) {
    let installed = Arc::new(Mutex::new(Vec::new()));
    let sink = installed.clone();
    let request: InstallRequest = Arc::new(move |path| {
        sink.lock().unwrap().push(path.to_path_buf());
        Ok(())
    });
    (request, installed)
}

fn available(version: &str) -> UpdateRelease {
    checked_release(version, UpdateTarget::WindowsX64, b"package")
}

#[test]
fn current_available_and_observer_isolation() {
    let client = FakeReleaseClient::new(checked_release("0.4.0", UpdateTarget::UbuntuAmd64, b"deb"));
    let manager = manager("0.4.0", client.clone(), UpdateTarget::UbuntuAmd64);
    let states = Arc::new(Mutex::new(Vec::<UpdateSnapshot>::new()));
    let sink = states.clone();
    manager.subscribe(Arc::new(move |snapshot| sink.lock().unwrap().push(snapshot.clone())));
    let failed = Arc::new(AtomicUsize::new(0));
    let flag = failed.clone();
    manager.subscribe(Arc::new(move |snapshot| {
        assert!(!(snapshot.phase != UpdatePhase::Idle && flag.fetch_add(1, Ordering::SeqCst) == 0), "observer");
    }));
    assert!(manager.check(false, false));
    assert!(failed.load(Ordering::SeqCst) > 0);
    assert_eq!(manager.snapshot().phase, UpdatePhase::Current);
    assert_eq!(states.lock().unwrap()[0].phase, UpdatePhase::Idle);
    assert_eq!(*client.latest_targets.lock().unwrap(), [UpdateTarget::UbuntuAmd64]);

    *client.release.lock().unwrap() = checked_release("1.0.0", UpdateTarget::UbuntuAmd64, b"deb");
    assert!(manager.check(true, true));
    assert_eq!(manager.snapshot().phase, UpdatePhase::Available);
    assert_eq!(manager.snapshot().available_version, "1.0.0");
    // APT updates the package: no installer download, no install.
    assert_eq!(client.downloads.load(Ordering::SeqCst), 0);
    assert!(!manager.install_available());
}

#[test]
fn a_newer_installed_version_is_never_downgraded_and_nothing_is_downloaded() {
    // The installed program is 0.70.0, the latest published release is 0.66.1.
    let content = b"old installer";
    let payload = release_payload("0.66.1", UpdateTarget::WindowsX64, content);
    let (asset, _) = FakeResponse::new(content, DOWNLOAD_URL);
    let opener = FakeOpener::with(vec![json_response(&payload), Ok(asset)]);
    let directory = temp_dir("downgrade");
    let (request, installed) = recording_installer();
    let manager = UpdateManager::with_opener("0.70.0", directory.clone(), opener.clone())
        .unwrap()
        .with_target(Some(UpdateTarget::WindowsX64))
        .with_runner(immediate())
        .with_install_request(request);
    assert!(manager.check(true, true));
    let snapshot = manager.snapshot();
    assert_eq!(snapshot.phase, UpdatePhase::Current);
    assert_eq!(snapshot.message, "Установлена актуальная версия 0.70.0");
    assert_eq!(snapshot.available_version, "");
    assert_eq!(opener.opened(), 1, "only the release feed is read");
    assert!(!manager.install_available());
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
    assert!(installed.lock().unwrap().is_empty());
    // The same version is current as well.
    let same = FakeReleaseClient::new(available("0.70.0"));
    let equal = manager_for(same.clone());
    assert!(equal.check(true, true));
    assert_eq!(equal.snapshot().phase, UpdatePhase::Current);
    assert_eq!(same.downloads.load(Ordering::SeqCst), 0);
    std::fs::remove_dir_all(directory).unwrap();
}

fn manager_for(client: Arc<FakeReleaseClient>) -> UpdateManager {
    manager("0.70.0", client, UpdateTarget::WindowsX64)
}

#[test]
fn a_missing_asset_is_an_error_snapshot() {
    let payload = release_payload("1.0.0", UpdateTarget::WindowsX64, b"x");
    let opener = FakeOpener::with(vec![json_response(&payload)]);
    let manager = UpdateManager::with_opener("0.4.0", PathBuf::from("updates"), opener)
        .unwrap()
        .with_target(Some(UpdateTarget::WindowsArm64))
        .with_runner(immediate());
    assert!(manager.check(true, true));
    let snapshot = manager.snapshot();
    assert_eq!(snapshot.phase, UpdatePhase::Error);
    assert_eq!(snapshot.message, "Обновление не выполнено: В выпуске отсутствует единственный файл KeySwitch-Setup-1.0.0-arm64.exe");
}

#[test]
fn automatic_and_manual_windows_install_paths() {
    let client = FakeReleaseClient::new(available("1.0.0"));
    let (request, installed) = recording_installer();
    let automatic = manager("0.4.0", client.clone(), UpdateTarget::WindowsX64).with_install_request(request.clone());
    assert!(automatic.check(true, true));
    assert_eq!(automatic.snapshot().phase, UpdatePhase::Installing);
    assert_eq!(automatic.snapshot().progress, 100);
    assert_eq!(client.downloads.load(Ordering::SeqCst), 1);
    assert_eq!(installed.lock().unwrap().len(), 1);

    let manual = manager("0.4.0", client.clone(), UpdateTarget::WindowsX64).with_install_request(request);
    assert!(manual.check(false, true));
    assert_eq!(manual.snapshot().phase, UpdatePhase::Available);
    assert!(manual.install_available());
    assert_eq!(manual.snapshot().phase, UpdatePhase::Installing);
    assert_eq!(client.downloads.load(Ordering::SeqCst), 2);
}

#[test]
fn the_arm64_and_x86_builds_install_their_own_updates() {
    for target in [UpdateTarget::WindowsArm64, UpdateTarget::WindowsX86] {
        let client = FakeReleaseClient::new(checked_release("1.0.0", target, b"p"));
        let (request, _) = recording_installer();
        let manager = manager("0.4.0", client.clone(), target).with_install_request(request);
        assert!(manager.check(true, true));
        assert_eq!(manager.snapshot().phase, UpdatePhase::Installing);
        assert_eq!(*client.latest_targets.lock().unwrap().last().unwrap(), target);
    }
}

#[test]
fn busy_runner_and_client_errors() {
    let client = FakeReleaseClient::new(available("1.0.0"));
    let queue = Arc::new(Mutex::new(Vec::<Job>::new()));
    let sink = queue.clone();
    let queued: BackgroundRunner = Arc::new(move |job| {
        sink.lock().unwrap().push(job);
        Ok(())
    });
    let busy = manager("0.4.0", client.clone(), UpdateTarget::WindowsX64).with_runner(queued);
    assert!(busy.check(false, false));
    assert!(!busy.check(false, false));
    assert!(!busy.install_available());
    let job = queue.lock().unwrap().pop().unwrap();
    job();
    assert_eq!(busy.snapshot().phase, UpdatePhase::Available);

    let broken: BackgroundRunner = Arc::new(|_| Err(io::Error::other("thread unavailable")));
    let runner_failure = manager("0.4.0", client.clone(), UpdateTarget::WindowsX64).with_runner(broken.clone());
    assert!(!runner_failure.check(false, false));
    assert_eq!(runner_failure.snapshot().phase, UpdatePhase::Error);

    *client.latest_error.lock().unwrap() = Some("feed failed".into());
    let feed_failure = manager("0.4.0", client.clone(), UpdateTarget::WindowsX64);
    assert!(feed_failure.check(false, false));
    assert!(feed_failure.snapshot().message.contains("feed failed"));
    *client.latest_error.lock().unwrap() = None;

    // The runner fails when a manual install is queued: error, and the manager is free again.
    let late = Arc::new(Mutex::new(false));
    let switch = late.clone();
    let flaky: BackgroundRunner = Arc::new(move |job| {
        if *switch.lock().unwrap() {
            return Err(io::Error::other("thread unavailable"));
        }
        job();
        Ok(())
    });
    let manual = manager("0.4.0", client, UpdateTarget::WindowsX64).with_runner(flaky);
    assert!(manual.check(false, false));
    *late.lock().unwrap() = true;
    assert!(!manual.install_available());
    assert_eq!(manual.snapshot().phase, UpdatePhase::Error);
}

#[test]
fn unsupported_closed_and_close_during_download() {
    let client = FakeReleaseClient::new(available("1.0.0"));
    let unsupported = UpdateManager::new("0.4.0", PathBuf::from("u"), client.clone()).unwrap().with_target(None).with_runner(immediate());
    assert!(unsupported.check(false, false));
    assert!(unsupported.snapshot().message.contains("архитектуру"));

    let (request, installed) = recording_installer();
    let closed = manager("0.4.0", client.clone(), UpdateTarget::WindowsX64).with_install_request(request.clone());
    closed.close();
    closed.subscribe(Arc::new(|_| panic!("a closed manager has no subscribers")));
    assert!(!closed.check(false, false));
    assert!(!closed.install_available());
    closed.installation_failed(&"ignored");
    assert_eq!(closed.snapshot().phase, UpdatePhase::Idle);

    let closing = manager("0.4.0", client.clone(), UpdateTarget::WindowsX64).with_install_request(request.clone());
    let handle = closing.clone();
    *client.download_hook.lock().unwrap() = Some(Box::new(move || handle.close()));
    assert!(closing.check(true, true));
    assert!(installed.lock().unwrap().is_empty());
    *client.download_hook.lock().unwrap() = None;

    let closing_check = manager("0.4.0", client.clone(), UpdateTarget::WindowsX64).with_install_request(request);
    let before = client.downloads.load(Ordering::SeqCst);
    assert!(closing_check.check(true, true));
    assert_eq!(client.downloads.load(Ordering::SeqCst), before + 1);
}

#[test]
fn download_and_install_failures() {
    let client = FakeReleaseClient::new(available("1.0.0"));
    let (request, _) = recording_installer();
    *client.download_error.lock().unwrap() = Some("disk full".into());
    let download_failure = manager("0.4.0", client.clone(), UpdateTarget::WindowsX64).with_install_request(request);
    assert!(download_failure.check(true, true));
    assert!(download_failure.snapshot().message.contains("disk full"));
    *client.download_error.lock().unwrap() = None;

    let missing = manager("0.4.0", client.clone(), UpdateTarget::WindowsX64);
    assert!(missing.check(true, true));
    assert!(missing.snapshot().message.contains("Не настроен"));

    let failing: InstallRequest = Arc::new(|_| Err(UpdateError::new("cannot execute")));
    let install_failure = manager("0.4.0", client, UpdateTarget::WindowsX64).with_install_request(failing);
    assert!(install_failure.check(true, true));
    assert!(install_failure.snapshot().message.contains("cannot execute"));
    install_failure.installation_failed(&"reported failure");
    assert!(install_failure.snapshot().message.contains("reported failure"));
}

#[test]
fn progress_is_published_in_whole_percent() {
    let client = FakeReleaseClient::new(available("1.0.0"));
    let (request, _) = recording_installer();
    let manager = manager("0.4.0", client, UpdateTarget::WindowsX64).with_install_request(request);
    let messages = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = messages.clone();
    manager.subscribe(Arc::new(move |snapshot| sink.lock().unwrap().push(snapshot.message.clone())));
    manager.check(true, true);
    let messages = messages.lock().unwrap();
    for expected in ["Загружаем версию 1.0.0: 0%", "Загружаем версию 1.0.0: 42%", "Загружаем версию 1.0.0: 100%", "Запускаем установку версии 1.0.0"] {
        assert!(messages.iter().any(|message| message == expected), "{expected}");
    }
}

#[test]
fn the_default_runner_uses_a_named_thread() {
    let (sender, receiver) = std::sync::mpsc::channel();
    default_runner()(Box::new(move || {
        sender.send(std::thread::current().name().map(str::to_string)).unwrap();
    }))
    .unwrap();
    assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap().as_deref(), Some("keyswitch-update"));
}

// ---- the installer launch and the schedule ----

#[test]
fn installer_path_and_arguments_are_strict() {
    let directory = temp_dir("launch");
    let spawned: Mutex<Vec<(PathBuf, Vec<String>)>> = Mutex::default();
    let spawn = |path: &Path, arguments: &[&str]| -> io::Result<()> {
        spawned.lock().unwrap().push((path.to_path_buf(), arguments.iter().map(ToString::to_string).collect()));
        Ok(())
    };
    let missing = directory.join("KeySwitch-Setup-1.0.0-x64.exe");
    assert!(launch_windows_installer(&missing, &spawn).unwrap_err().0.contains("не найден"));
    for name in ["setup.exe", "KeySwitch-Setup-1.0.0-x64.exe.part", "KeySwitch-Setup-1.0.0-ia64.exe", "KeySwitch-Setup-01.0.0-x64.exe"] {
        let path = directory.join(name);
        std::fs::write(&path, b"x").unwrap();
        assert!(launch_windows_installer(&path, &spawn).unwrap_err().0.contains("не прошло проверку"), "{name}");
    }
    assert!(spawned.lock().unwrap().is_empty());
    std::fs::write(&missing, b"x").unwrap();
    launch_windows_installer(&missing, &spawn).unwrap();
    let calls = spawned.lock().unwrap();
    assert_eq!(calls[0].0, missing);
    assert_eq!(
        calls[0].1,
        ["/SP-", "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS", "/NORESTARTAPPLICATIONS", "/KEYSWITCHUPDATE=1"]
    );
    let failing = |_: &Path, _: &[&str]| -> io::Result<()> { Err(io::Error::other("denied")) };
    assert!(launch_windows_installer(&missing, &failing).is_err());
    std::fs::remove_dir_all(&directory).unwrap();
}

#[derive(Default)]
struct CountingController {
    checks: Mutex<Vec<(bool, bool)>>,
}

impl UpdateController for CountingController {
    fn snapshot(&self) -> UpdateSnapshot {
        UpdateSnapshot::new(UpdatePhase::Idle, "", "0.0.0")
    }
    fn subscribe(&self, _callback: UpdateObserver) {}
    fn check(&self, automatic: bool, install_automatically: bool) -> bool {
        self.checks.lock().unwrap().push((automatic, install_automatically));
        true
    }
    fn install_available(&self) -> bool {
        false
    }
    fn installation_failed(&self, _error: &dyn Display) {}
}

#[test]
fn automatic_checks_follow_the_setting_and_can_be_stopped() {
    let controller = Arc::new(CountingController::default());
    assert!(UpdateChecks::start_with(controller.clone(), || false, Duration::from_millis(1), Duration::from_millis(1)).is_none());
    let checks = UpdateChecks::start_with(controller.clone(), || true, Duration::from_millis(20), Duration::from_millis(20)).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    checks.stop();
    let count = controller.checks.lock().unwrap().len();
    assert!(count >= 2, "{count}");
    assert!(controller.checks.lock().unwrap().iter().all(|call| *call == (true, false)));
    std::thread::sleep(Duration::from_millis(100));
    assert!(controller.checks.lock().unwrap().len() <= count + 1);

    // Stopped before the first delay ends: no check at all.
    let quiet = Arc::new(CountingController::default());
    let early = UpdateChecks::start_with(quiet.clone(), || true, Duration::from_secs(60), Duration::from_secs(60)).unwrap();
    drop(early);
    std::thread::sleep(Duration::from_millis(100));
    assert!(quiet.checks.lock().unwrap().is_empty());
    assert_eq!(constants::UPDATE_CHECK_INITIAL_DELAY, Duration::from_secs(30));
    assert_eq!(constants::UPDATE_CHECK_INTERVAL, Duration::from_secs(21_600));
}

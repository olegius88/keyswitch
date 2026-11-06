//! Strict GitHub Releases update discovery and verified package download.

use ks_cast::To as _;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::constants::{
    DOWNLOAD_CHUNK_BYTES, MAX_ASSET_BYTES, MAX_RELEASE_JSON_BYTES, MAX_RELEASE_NOTES_CHARACTERS,
    PROGRESS_COMPLETE_PERCENT, UPDATE_CHECK_INITIAL_DELAY, UPDATE_CHECK_INTERVAL, UPDATE_REQUEST_TIMEOUT,
};

pub const REPOSITORY: &str = "olegius88/keyswitch";
pub const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/olegius88/keyswitch/releases/latest";
pub const RELEASE_PAGE_PREFIX: &str = "https://github.com/olegius88/keyswitch/releases/tag/";
pub const RELEASE_DOWNLOAD_PREFIX: &str = "https://github.com/olegius88/keyswitch/releases/download/";
pub const GITHUB_API_VERSION: &str = "2026-03-10";
/// The arguments of the update installer; the last one tells the installer to relaunch `KeySwitch` afterwards.
pub const WINDOWS_INSTALLER_ARGUMENTS: [&str; 7] = [
    "/SP-",
    "/VERYSILENT",
    "/SUPPRESSMSGBOXES",
    "/NORESTART",
    "/CLOSEAPPLICATIONS",
    "/NORESTARTAPPLICATIONS",
    "/KEYSWITCHUPDATE=1",
];

/// An update feed, download or installer failed a required check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateError(pub String);

impl UpdateError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UpdateError {}

fn fail<T>(message: impl Into<String>) -> Result<T, UpdateError> {
    Err(UpdateError::new(message))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemanticVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl SemanticVersion {
    /// Exactly three numbers without leading zeros, with an optional `v` in front.
    pub fn parse(value: &str) -> Result<Self, UpdateError> {
        let digits = value.strip_prefix('v').unwrap_or(value);
        match parse_numbers(digits) {
            Some(version) => Ok(version),
            None => fail(format!("Некорректная версия выпуска: {value:?}")),
        }
    }
}

fn parse_numbers(text: &str) -> Option<SemanticVersion> {
    let mut numbers = [0u64; 3];
    let mut parts = text.split('.');
    for slot in &mut numbers {
        let part = parts.next()?;
        let valid = !part.is_empty()
            && part.bytes().all(|byte| byte.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'));
        if !valid {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(SemanticVersion { major: numbers[0], minor: numbers[1], patch: numbers[2] })
}

impl fmt::Display for SemanticVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpdateTarget {
    WindowsX64,
    WindowsArm64,
    WindowsX86,
    UbuntuAmd64,
}

impl UpdateTarget {
    #[must_use]
    pub fn value(self) -> &'static str {
        match self {
            Self::WindowsX64 => "windows-x64",
            Self::WindowsArm64 => "windows-arm64",
            Self::WindowsX86 => "windows-x86",
            Self::UbuntuAmd64 => "ubuntu-amd64",
        }
    }

    /// `KeySwitch` downloads and runs this target's installer itself; APT updates the package.
    #[must_use]
    pub fn windows_installer(self) -> bool {
        self != Self::UbuntuAmd64
    }

    #[must_use]
    pub fn asset_name(self, version: SemanticVersion) -> String {
        let architecture = match self {
            Self::UbuntuAmd64 => return format!("keyswitch_{version}_amd64.deb"),
            Self::WindowsX64 => "x64",
            Self::WindowsArm64 => "arm64",
            Self::WindowsX86 => "x86",
        };
        format!("KeySwitch-Setup-{version}-{architecture}.exe")
    }
}

/// A Windows build updates with the installer of its own architecture, which the build script names after the
/// platform the program was built for: an x86 build under WOW64 stays x86, an x64 one on an Arm PC stays x64. The
/// machine is not asked, so builds of two architectures never share an install folder.
#[must_use]
pub fn detect_update_target(system: &str, machine: &str, build_platform: &str) -> Option<UpdateTarget> {
    let operating_system = system.to_lowercase();
    if operating_system == "windows" {
        return match build_platform {
            "win-amd64" => Some(UpdateTarget::WindowsX64),
            "win-arm64" => Some(UpdateTarget::WindowsArm64),
            "win32" => Some(UpdateTarget::WindowsX86),
            _ => None,
        };
    }
    let architecture = machine.to_lowercase();
    if operating_system == "linux" && (architecture == "amd64" || architecture == "x86_64") {
        return Some(UpdateTarget::UbuntuAmd64);
    }
    None
}

/// The target of this very build.
#[must_use]
pub fn detect_build_update_target() -> Option<UpdateTarget> {
    detect_update_target(std::env::consts::OS, std::env::consts::ARCH, build_platform_of(std::env::consts::ARCH))
}

/// The name of the Windows build of a processor.
fn build_platform_of(architecture: &str) -> &'static str {
    match architecture {
        "x86_64" => "win-amd64",
        "aarch64" => "win-arm64",
        "x86" => "win32",
        _ => "",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRelease {
    pub version: SemanticVersion,
    pub tag: String,
    pub page_url: String,
    pub notes: String,
    pub asset: ReleaseAsset,
}

/// One GET request: the URL and its headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

/// An open response; dropping it closes it. `url` is the address after redirects.
pub trait UpdateResponse: Send {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize>;
    fn url(&self) -> String;
}

/// The HTTP transport. A status of 400 and above, or no connection, is an `Err`.
pub trait ResponseOpener: Send + Sync {
    fn open(&self, request: &UpdateRequest, timeout: Duration) -> io::Result<Box<dyn UpdateResponse>>;
}

fn object_mapping<'a>(value: &'a Value, label: &str) -> Result<&'a Map<String, Value>, UpdateError> {
    match value {
        Value::Object(map) => Ok(map),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => fail(format!("GitHub вернул некорректное поле {label}")),
    }
}

fn required_string(mapping: &Map<String, Value>, key: &str) -> Result<String, UpdateError> {
    match mapping.get(key) {
        Some(Value::String(value)) if !value.is_empty() => Ok(value.clone()),
        _ => fail(format!("GitHub не вернул строковое поле {key}")),
    }
}

fn required_size(mapping: &Map<String, Value>) -> Result<u64, UpdateError> {
    let Some(Value::Number(number)) = mapping.get("size") else {
        return fail("GitHub не вернул размер файла обновления");
    };
    if number.is_f64() {
        return fail("GitHub не вернул размер файла обновления");
    }
    match number.as_u64() {
        Some(value) if value > 0 && value <= MAX_ASSET_BYTES => Ok(value),
        _ => fail(format!("Недопустимый размер файла обновления: {number}")),
    }
}

/// Read at most `maximum` bytes; one more means the response is too large.
fn read_limited(response: &mut dyn UpdateResponse, maximum: usize) -> io::Result<Result<Vec<u8>, UpdateError>> {
    let mut payload = Vec::new();
    let mut chunk = vec![0u8; DOWNLOAD_CHUNK_BYTES];
    while payload.len() <= maximum {
        let wanted = chunk.len().min(maximum + 1 - payload.len());
        let count = response.read(&mut chunk[..wanted])?;
        if count == 0 {
            break;
        }
        payload.extend_from_slice(&chunk[..count]);
    }
    if payload.len() > maximum {
        return Ok(fail("Ответ сервера обновлений превышает допустимый размер"));
    }
    Ok(Ok(payload))
}

fn validate_api_response_url(value: &str) -> Result<(), UpdateError> {
    if value != LATEST_RELEASE_URL {
        return fail("GitHub API перенаправил запрос на недоверенный адрес");
    }
    Ok(())
}

/// The scheme, the host (lower case, without user and port) and the path of an absolute URL.
fn split_url(value: &str) -> Option<(String, String, &str)> {
    let value = value.trim_start_matches(|c: char| c <= ' ');
    let (scheme, rest) = value.split_once(':')?;
    let rest = rest.strip_prefix("//")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let host = match host_port.rsplit_once(':') {
        Some((host, port)) if port.bytes().all(|byte| byte.is_ascii_digit()) && !host.ends_with(']') => host,
        _ => host_port,
    };
    let path_end = tail.find(['?', '#']).unwrap_or(tail.len());
    Some((scheme.to_lowercase(), host.to_lowercase(), tail.get(..path_end).unwrap_or_default()))
}

fn validate_download_response_url(value: &str) -> Result<(), UpdateError> {
    let trusted = match split_url(value) {
        Some((scheme, host, path)) => {
            scheme == "https"
                && host == "release-assets.githubusercontent.com"
                && path.starts_with("/github-production-release-asset/")
        }
        None => false,
    };
    if !trusted {
        return fail("GitHub перенаправил загрузку на недоверенный адрес");
    }
    Ok(())
}

/// `urllib.parse.quote(value, safe='')`.
fn quote(value: &str) -> String {
    let mut result = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"_.-~".contains(&byte) {
            result.push(char::from(byte));
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}

fn is_digest(value: &str) -> Option<&str> {
    let hex = value.strip_prefix("sha256:")?;
    let valid = hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    valid.then_some(hex)
}

/// Whether `name` is exactly the name of a `KeySwitch` Windows installer.
fn is_windows_installer_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("KeySwitch-Setup-").and_then(|rest| rest.strip_suffix(".exe")) else {
        return false;
    };
    match rest.rsplit_once('-') {
        Some((version, architecture)) => {
            matches!(architecture, "x64" | "arm64" | "x86") && parse_numbers(version).is_some()
        }
        None => false,
    }
}

pub trait ReleaseClient: Send + Sync {
    fn latest_release(&self, target: UpdateTarget) -> Result<UpdateRelease, UpdateError>;

    fn download(
        &self,
        release: &UpdateRelease,
        directory: &Path,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<PathBuf, UpdateError>;
}

/// Read and validate one stable release from the public GitHub API.
pub struct GitHubReleaseClient {
    pub current_version: String,
    pub opener: Arc<dyn ResponseOpener>,
    pub timeout: Duration,
}

impl GitHubReleaseClient {
    pub fn new(current_version: &str, opener: Arc<dyn ResponseOpener>) -> Self {
        Self { current_version: current_version.to_string(), opener, timeout: UPDATE_REQUEST_TIMEOUT }
    }

    fn fetch_release_json(&self) -> Result<Value, UpdateError> {
        let request = UpdateRequest {
            url: LATEST_RELEASE_URL.to_string(),
            headers: vec![
                ("Accept".into(), "application/vnd.github+json".into()),
                ("X-GitHub-Api-Version".into(), GITHUB_API_VERSION.into()),
                ("User-Agent".into(), format!("KeySwitch/{}", self.current_version)),
            ],
        };
        let read_failed = |error: &dyn fmt::Display| UpdateError::new(format!("Не удалось прочитать GitHub Releases: {error}"));
        let mut response = self.opener.open(&request, self.timeout).map_err(|error| read_failed(&error))?;
        validate_api_response_url(&response.url())?;
        let raw = read_limited(response.as_mut(), MAX_RELEASE_JSON_BYTES).map_err(|error| read_failed(&error))??;
        drop(response);
        let text = String::from_utf8(raw).map_err(|error| read_failed(&error))?;
        serde_json::from_str(&text).map_err(|error| read_failed(&error))
    }

    fn download_to(
        &self,
        release: &UpdateRelease,
        partial_path: &Path,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<(), UpdateError> {
        let request = UpdateRequest {
            url: release.asset.url.clone(),
            headers: vec![("User-Agent".into(), format!("KeySwitch/{}", self.current_version))],
        };
        let io_failed = |error: io::Error| UpdateError::new(format!("Не удалось загрузить обновление: {error}"));
        let mut response = self.opener.open(&request, self.timeout).map_err(io_failed)?;
        validate_download_response_url(&response.url())?;
        let mut hasher = Sha256::new();
        let mut downloaded: u64 = 0;
        let mut handle = fs::File::create(partial_path).map_err(io_failed)?;
        let mut chunk = vec![0u8; DOWNLOAD_CHUNK_BYTES];
        loop {
            let count = response.read(&mut chunk).map_err(io_failed)?;
            if count == 0 {
                break;
            }
            downloaded += count.to::<u64>();
            if downloaded > release.asset.size {
                return fail("Размер загруженного обновления превышает объявленный");
            }
            io::Write::write_all(&mut handle, &chunk[..count]).map_err(io_failed)?;
            hasher.update(&chunk[..count]);
            progress(downloaded, release.asset.size);
        }
        drop(response);
        drop(handle);
        if downloaded != release.asset.size {
            return fail("Размер загруженного обновления не совпадает с объявленным");
        }
        let digest: String = hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
        if digest != release.asset.sha256 {
            return fail("Контрольная сумма обновления SHA-256 не совпала");
        }
        Ok(())
    }
}

impl ReleaseClient for GitHubReleaseClient {
    fn latest_release(&self, target: UpdateTarget) -> Result<UpdateRelease, UpdateError> {
        let decoded = self.fetch_release_json()?;
        let payload = object_mapping(&decoded, "release")?;
        if payload.get("draft") != Some(&Value::Bool(false)) || payload.get("prerelease") != Some(&Value::Bool(false)) {
            return fail("GitHub вернул черновик или предварительный выпуск");
        }
        let tag = required_string(payload, "tag_name")?;
        let version = SemanticVersion::parse(&tag)?;
        if tag != format!("v{version}") {
            return fail(format!("Некорректный тег выпуска: {tag}"));
        }
        let page_url = required_string(payload, "html_url")?;
        if page_url != format!("{RELEASE_PAGE_PREFIX}{}", quote(&tag)) {
            return fail("Страница выпуска не относится к репозиторию KeySwitch");
        }

        let expected_name = target.asset_name(version);
        let Some(Value::Array(assets)) = payload.get("assets") else {
            return fail("GitHub не вернул список файлов выпуска");
        };
        let mut matching = Vec::new();
        for item in assets {
            let asset = object_mapping(item, "asset")?;
            if asset.get("name") == Some(&Value::String(expected_name.clone())) {
                matching.push(asset);
            }
        }
        if matching.len() != 1 {
            return fail(format!("В выпуске отсутствует единственный файл {expected_name}"));
        }

        let asset_payload = matching[0];
        if asset_payload.get("state") != Some(&Value::String("uploaded".into())) {
            return fail("Файл обновления ещё не опубликован полностью");
        }
        let asset_url = required_string(asset_payload, "browser_download_url")?;
        let expected_url = format!("{RELEASE_DOWNLOAD_PREFIX}{}/{}", quote(&tag), quote(&expected_name));
        if asset_url != expected_url {
            return fail("Файл обновления ссылается на недоверенный адрес");
        }
        let digest = required_string(asset_payload, "digest")?;
        let Some(sha256) = is_digest(&digest) else {
            return fail("GitHub не вернул обязательную контрольную сумму SHA-256");
        };
        let notes = match payload.get("body") {
            Some(Value::String(body)) => body.chars().take(MAX_RELEASE_NOTES_CHARACTERS).collect(),
            _ => String::new(),
        };
        let size = required_size(asset_payload)?;
        Ok(UpdateRelease {
            version,
            tag,
            page_url,
            notes,
            asset: ReleaseAsset { name: expected_name, url: asset_url, size, sha256: sha256.to_string() },
        })
    }

    fn download(
        &self,
        release: &UpdateRelease,
        directory: &Path,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<PathBuf, UpdateError> {
        let io_failed = |error: io::Error| UpdateError::new(format!("Не удалось загрузить обновление: {error}"));
        fs::create_dir_all(directory).map_err(io_failed)?;
        let destination = directory.join(&release.asset.name);
        let partial_path = directory.join(format!("{}.part", release.asset.name));
        unlink_if_exists(&partial_path).map_err(io_failed)?;
        remove_previous_installers(directory, &release.asset.name).map_err(io_failed)?;
        // The installer is never reachable under its final name before the size and the digest agree.
        let result = self.download_to(release, &partial_path, progress).and_then(|()| {
            fs::rename(&partial_path, &destination).map_err(io_failed)
        });
        match result {
            Ok(()) => Ok(destination),
            Err(error) => {
                let _ = unlink_if_exists(&partial_path);
                Err(error)
            }
        }
    }
}

/// Delete the installers of earlier updates; each one is tens of megabytes.
///
/// Only files named exactly as a `KeySwitch` installer are touched. One that is still open is left for the next
/// download to try again.
pub fn remove_previous_installers(directory: &Path, keep: &str) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name != keep && is_windows_installer_name(name) {
            let _ = fs::remove_file(entry.path());
        }
    }
    Ok(())
}

pub fn unlink_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatePhase {
    Idle,
    Checking,
    Current,
    Available,
    Downloading,
    Installing,
    Error,
}

impl UpdatePhase {
    #[must_use]
    pub fn value(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Checking => "checking",
            Self::Current => "current",
            Self::Available => "available",
            Self::Downloading => "downloading",
            Self::Installing => "installing",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSnapshot {
    pub phase: UpdatePhase,
    pub message: String,
    pub current_version: String,
    pub available_version: String,
    pub release_url: String,
    pub progress: u64,
}

impl UpdateSnapshot {
    pub fn new(phase: UpdatePhase, message: impl Into<String>, current_version: impl Into<String>) -> Self {
        Self {
            phase,
            message: message.into(),
            current_version: current_version.into(),
            available_version: String::new(),
            release_url: String::new(),
            progress: 0,
        }
    }
}

pub type UpdateObserver = Arc<dyn Fn(&UpdateSnapshot) + Send + Sync>;
pub type Job = Box<dyn FnOnce() + Send>;
/// Runs a job away from the caller; `Err` when that was impossible (no thread).
pub type BackgroundRunner = Arc<dyn Fn(Job) -> io::Result<()> + Send + Sync>;
/// Starts the downloaded installer.
pub type InstallRequest = Arc<dyn Fn(&Path) -> Result<(), UpdateError> + Send + Sync>;

pub fn run_in_background(action: Job) -> io::Result<()> {
    thread::Builder::new().name("keyswitch-update".into()).spawn(action).map(|_| ())
}

pub fn default_runner() -> BackgroundRunner {
    Arc::new(run_in_background)
}

pub trait UpdateController: Send + Sync {
    fn snapshot(&self) -> UpdateSnapshot;
    fn subscribe(&self, callback: UpdateObserver);
    fn check(&self, automatic: bool, install_automatically: bool) -> bool;
    /// Download and run the installer of the release found by the last check (Python `install_available`).
    fn install_available(&self) -> bool;
    fn installation_failed(&self, error: &dyn fmt::Display);
}

struct State {
    observers: Vec<UpdateObserver>,
    release: Option<UpdateRelease>,
    busy: bool,
    closed: bool,
    snapshot: UpdateSnapshot,
}

struct Inner {
    current: SemanticVersion,
    directory: PathBuf,
    target: Option<UpdateTarget>,
    client: Arc<dyn ReleaseClient>,
    runner: BackgroundRunner,
    install_request: Option<InstallRequest>,
    state: Mutex<State>,
}

/// Serialize background checks/downloads and publish immutable UI state.
#[derive(Clone)]
pub struct UpdateManager {
    inner: Arc<Inner>,
}

impl UpdateManager {
    /// `target` is `detect_build_update_target()` unless set with [`UpdateManager::with_target`].
    pub fn new(current_version: &str, directory: PathBuf, client: Arc<dyn ReleaseClient>) -> Result<Self, UpdateError> {
        let current = SemanticVersion::parse(current_version)?;
        let snapshot = UpdateSnapshot::new(UpdatePhase::Idle, "Обновления ещё не проверялись", current.to_string());
        Ok(Self {
            inner: Arc::new(Inner {
                current,
                directory,
                target: detect_build_update_target(),
                client,
                runner: default_runner(),
                install_request: None,
                state: Mutex::new(State { observers: Vec::new(), release: None, busy: false, closed: false, snapshot }),
            }),
        })
    }

    /// The manager with the GitHub client over `opener`.
    pub fn with_opener(current_version: &str, directory: PathBuf, opener: Arc<dyn ResponseOpener>) -> Result<Self, UpdateError> {
        Self::new(current_version, directory, Arc::new(GitHubReleaseClient::new(current_version, opener)))
    }

    #[expect(clippy::panic, reason = "builders run before the manager is shared: this is a mistake in the program, not in what it reads")]
    fn rebuilt(self, change: impl FnOnce(&mut Inner)) -> Self {
        // Builders run before the manager is shared, so the Arc is unique.
        let mut inner = Arc::try_unwrap(self.inner).unwrap_or_else(|_| panic!("builder used on a shared manager"));
        change(&mut inner);
        Self { inner: Arc::new(inner) }
    }

    #[must_use]
    pub fn with_target(self, target: Option<UpdateTarget>) -> Self {
        self.rebuilt(|inner| inner.target = target)
    }

    #[must_use]
    pub fn with_runner(self, runner: BackgroundRunner) -> Self {
        self.rebuilt(|inner| inner.runner = runner)
    }

    #[must_use]
    pub fn with_install_request(self, request: InstallRequest) -> Self {
        self.rebuilt(|inner| inner.install_request = Some(request))
    }

    #[must_use]
    pub fn target(&self) -> Option<UpdateTarget> {
        self.inner.target
    }

    pub fn close(&self) {
        let mut state = self.inner.lock();
        state.closed = true;
        state.busy = false;
        state.observers.clear();
    }
}

impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn notify(observers: &[UpdateObserver], snapshot: &UpdateSnapshot) {
        for observer in observers {
            // One failing subscriber must not stop the others.
            let call = std::panic::AssertUnwindSafe(|| observer(snapshot));
            if std::panic::catch_unwind(call).is_err() {
                eprintln!("Ошибка подписчика состояния обновлений");
            }
        }
    }

    fn transition(&self, snapshot: UpdateSnapshot, busy: Option<bool>) -> bool {
        let observers = {
            let mut state = self.lock();
            if state.closed {
                return false;
            }
            if let Some(busy) = busy {
                state.busy = busy;
            }
            state.snapshot = snapshot.clone();
            state.observers.clone()
        };
        Self::notify(&observers, &snapshot);
        true
    }

    fn release_snapshot(&self, phase: UpdatePhase, message: String, release: &UpdateRelease, progress: u64) -> UpdateSnapshot {
        UpdateSnapshot {
            phase,
            message,
            current_version: self.current.to_string(),
            available_version: release.version.to_string(),
            release_url: release.page_url.clone(),
            progress,
        }
    }

    fn set_busy(&self, value: bool) {
        let mut state = self.lock();
        if !state.closed {
            state.busy = value;
        }
    }

    fn fail(&self, error: &dyn fmt::Display) {
        self.transition(
            UpdateSnapshot::new(UpdatePhase::Error, format!("Обновление не выполнено: {error}"), self.current.to_string()),
            Some(false),
        );
    }

    fn check_worker(&self, automatic: bool, install_automatically: bool) {
        let Some(target) = self.target else {
            self.fail(&UpdateError::new("Автообновление не поддерживает эту архитектуру"));
            return;
        };
        let release = match self.client.latest_release(target) {
            Ok(release) => release,
            Err(error) => {
                self.fail(&error);
                return;
            }
        };
        // A release that is not newer, a downgrade included, is never offered or downloaded.
        if release.version <= self.current {
            self.lock().release = None;
            self.transition(
                UpdateSnapshot::new(
                    UpdatePhase::Current,
                    format!("Установлена актуальная версия {}", self.current),
                    self.current.to_string(),
                ),
                Some(false),
            );
            return;
        }
        self.lock().release = Some(release.clone());
        let available = self.release_snapshot(UpdatePhase::Available, format!("Доступна версия {}", release.version), &release, 0);
        if !self.transition(available, None) {
            return;
        }
        if automatic && install_automatically && target.windows_installer() {
            self.download_worker(&release);
            return;
        }
        self.set_busy(false);
    }

    fn download_worker(&self, release: &UpdateRelease) {
        self.transition(
            self.release_snapshot(UpdatePhase::Downloading, format!("Загружаем версию {}: 0%", release.version), release, 0),
            None,
        );
        let mut progress = |downloaded: u64, total: u64| self.download_progress(release, downloaded, total);
        let path = match self.client.download(release, &self.directory, &mut progress) {
            Ok(path) => path,
            Err(error) => {
                self.fail(&error);
                return;
            }
        };
        let installing = self.release_snapshot(
            UpdatePhase::Installing,
            format!("Запускаем установку версии {}", release.version),
            release,
            PROGRESS_COMPLETE_PERCENT,
        );
        if !self.transition(installing, Some(false)) {
            return;
        }
        let Some(request) = self.install_request.clone() else {
            self.fail(&UpdateError::new("Не настроен запуск установщика обновления"));
            return;
        };
        if let Err(error) = request(&path) {
            self.fail(&error);
        }
    }

    fn download_progress(&self, release: &UpdateRelease, downloaded: u64, total: u64) {
        let percentage = (downloaded.saturating_mul(PROGRESS_COMPLETE_PERCENT) / total.max(1)).min(PROGRESS_COMPLETE_PERCENT);
        self.transition(
            self.release_snapshot(
                UpdatePhase::Downloading,
                format!("Загружаем версию {}: {percentage}%", release.version),
                release,
                percentage,
            ),
            None,
        );
    }
}

impl UpdateController for UpdateManager {
    fn snapshot(&self) -> UpdateSnapshot {
        self.inner.lock().snapshot.clone()
    }

    fn subscribe(&self, callback: UpdateObserver) {
        let snapshot = {
            let mut state = self.inner.lock();
            if state.closed {
                return;
            }
            state.observers.push(callback.clone());
            state.snapshot.clone()
        };
        callback(&snapshot);
    }

    fn check(&self, automatic: bool, install_automatically: bool) -> bool {
        let checking = UpdateSnapshot::new(UpdatePhase::Checking, "Проверяем GitHub Releases…", self.inner.current.to_string());
        let observers = {
            let mut state = self.inner.lock();
            if state.closed || state.busy {
                return false;
            }
            state.busy = true;
            state.snapshot = checking.clone();
            state.observers.clone()
        };
        Inner::notify(&observers, &checking);
        let inner = self.inner.clone();
        if let Err(error) = (self.inner.runner)(Box::new(move || inner.check_worker(automatic, install_automatically))) {
            self.inner.fail(&error);
            return false;
        }
        true
    }

    fn install_available(&self) -> bool {
        let release = {
            let mut state = self.inner.lock();
            let installable = self.inner.target.is_some_and(UpdateTarget::windows_installer);
            match &state.release {
                Some(release) if !state.closed && !state.busy && installable => {
                    let release = release.clone();
                    state.busy = true;
                    release
                }
                _ => return false,
            }
        };
        let downloading = self.inner.release_snapshot(
            UpdatePhase::Downloading,
            format!("Загружаем версию {}: 0%", release.version),
            &release,
            0,
        );
        self.inner.transition(downloading, None);
        let inner = self.inner.clone();
        if let Err(error) = (self.inner.runner)(Box::new(move || inner.download_worker(&release))) {
            self.inner.fail(&error);
            return false;
        }
        true
    }

    fn installation_failed(&self, error: &dyn fmt::Display) {
        self.inner.fail(error);
    }
}

/// Check the name and existence of the installer, then start it with the arguments of an update.
pub fn launch_windows_installer(
    path: &Path,
    spawn: &dyn Fn(&Path, &[&str]) -> io::Result<()>,
) -> Result<(), UpdateError> {
    if !path.is_file() {
        return fail(format!("Файл установщика не найден: {}", path.display()));
    }
    let named = path.file_name().and_then(|name| name.to_str()).is_some_and(is_windows_installer_name);
    if !named {
        return fail("Имя установщика обновления не прошло проверку");
    }
    spawn(path, &WINDOWS_INSTALLER_ARGUMENTS).map_err(|error| UpdateError::new(error.to_string()))
}

/// The automatic checks: the first one after [`UPDATE_CHECK_INITIAL_DELAY`], then one every
/// [`UPDATE_CHECK_INTERVAL`] while the setting stays on. Dropping it, or [`UpdateChecks::stop`], cancels them.
pub struct UpdateChecks {
    cancel: Arc<(Mutex<bool>, Condvar)>,
}

impl UpdateChecks {
    pub fn start(controller: Arc<dyn UpdateController>, check_automatically: impl Fn() -> bool + Send + 'static) -> Option<Self> {
        Self::start_with(controller, check_automatically, UPDATE_CHECK_INITIAL_DELAY, UPDATE_CHECK_INTERVAL)
    }

    /// `None` when the setting is off; the caller starts again when it turns on.
    pub fn start_with(
        controller: Arc<dyn UpdateController>,
        check_automatically: impl Fn() -> bool + Send + 'static,
        initial_delay: Duration,
        interval: Duration,
    ) -> Option<Self> {
        if !check_automatically() {
            return None;
        }
        let cancel = Arc::new((Mutex::new(false), Condvar::new()));
        let signal = cancel.clone();
        let spawned = thread::Builder::new().name("keyswitch-update-timer".into()).spawn(move || {
            let wait = |delay: Duration| -> bool {
                let (flag, condvar) = &*signal;
                let guard = flag.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                let (guard, _) = condvar
                    .wait_timeout_while(guard, delay, |cancelled| !*cancelled)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                !*guard
            };
            if !wait(initial_delay) {
                return;
            }
            controller.check(true, false);
            if !check_automatically() {
                return;
            }
            while wait(interval) {
                controller.check(true, false);
            }
        });
        spawned.ok().map(|_| Self { cancel })
    }

    pub fn stop(&self) {
        let (flag, condvar) = &*self.cancel;
        *flag.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        condvar.notify_all();
    }
}

impl Drop for UpdateChecks {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn every_processor_has_its_build_name() {
        for (architecture, name) in [("x86_64", "win-amd64"), ("aarch64", "win-arm64"), ("x86", "win32"), ("riscv64", "")] {
            assert_eq!(build_platform_of(architecture), name);
        }
    }

    #[test]
    fn a_host_is_read_without_user_and_port() {
        let host = |url: &str| split_url(url).map(|(_, host, _)| host);
        assert_eq!(host("https://user@GitHub.com:443/x").as_deref(), Some("github.com"));
        assert_eq!(host("https://github.com:x/y").as_deref(), Some("github.com:x"));
        assert_eq!(host("https://[::1]/y").as_deref(), Some("[::1]"));
        assert_eq!(host("https://[::1]:1/y").as_deref(), Some("[::1]:1"));
        assert_eq!(host("https://a]:1/y").as_deref(), Some("a]:1"));
        assert_eq!(host("nothing"), None);
        assert_eq!(host("https:/one-slash"), None);
    }

    #[test]
    fn a_name_is_quoted_as_python_does() {
        assert_eq!(quote("v1.0-rc_1~a b/c\u{e9}"), "v1.0-rc_1~a%20b%2Fc%C3%A9");
    }
}

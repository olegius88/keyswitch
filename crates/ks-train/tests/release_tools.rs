//! The tools around a release, over stubs of the system programs: the APT repository built and read back, the Debian package checked.
//! This program is its own packaged binary as well: asked as `keyswitch --version` or `--diagnose`, it answers as the program would.
#![expect(clippy::unwrap_used, clippy::expect_used, reason = "a test or a probe stops on what it did not expect")]
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]

mod support;

use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use support::{digest, recipe, scratch, small_tree, trainer, tree};

const CONTROL: &str = "Package: keyswitch\nVersion: 9.9.9\nArchitecture: amd64\nDepends: libc6, libgtk-4-1\nDescription: the keyboard\n";

fn executable(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The system programs the tools call, answering by the environment.
fn stubs(folder: &Path) -> std::ffi::OsString {
    let bin = folder.join("stub-bin");
    std::fs::create_dir_all(&bin).unwrap();
    executable(&bin.join("dpkg"), "#!/bin/sh\n[ -z \"$KS_NO_DPKG\" ] || exit 3\necho amd64\n");
    executable(
        &bin.join("dpkg-deb"),
        "#!/bin/sh\ncontrol=\"$2.control\"\ncase \"$1\" in\n  --field)\n    if [ -z \"$3\" ]; then cat \"$control\"; else sed -n \"s/^$3: //p\" \"$control\"; fi ;;\n  --extract) cp -r \"$2.root/.\" \"$3\" ;;\n  --control) cp -r \"$2.meta/.\" \"$3\" ;;\nesac\n",
    );
    executable(&bin.join("gpg"), "#!/bin/sh\n[ -z \"$KS_NO_GPG\" ] || exit 2\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in --output) output=\"$2\"; shift ;; esac\n  last=\"$1\"\n  shift\ndone\ncp \"$last\" \"$output\"\n");
    executable(&bin.join("gpgv"), "#!/bin/sh\n[ -z \"$KS_NO_GPGV\" ]\n");
    executable(&bin.join("apt-get"), "#!/bin/sh\n[ -z \"$KS_NO_APT\" ]\n");
    executable(&bin.join("apt-cache"), "#!/bin/sh\n[ -n \"$KS_NO_CANDIDATE\" ] || echo \"  Candidate: ${KS_CANDIDATE:-9.9.9}\"\n");
    let mut path = bin.into_os_string();
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap());
    path
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

/// A package the stub of `dpkg-deb` reads.
fn plain_package(folder: &Path, control: &str) -> PathBuf {
    let package = folder.join("keyswitch_9.9.9_amd64.deb");
    std::fs::write(&package, b"a package").unwrap();
    std::fs::write(folder.join("keyswitch_9.9.9_amd64.deb.control"), control).unwrap();
    package
}

struct Apt {
    tree: PathBuf,
    folder: PathBuf,
    path: std::ffi::OsString,
}

impl Apt {
    fn build(&self, repository: &Path, package: Option<&Path>, passphrase: bool) -> Output {
        let mut command = trainer();
        command.arg("apt-build").arg("--tree").arg(&self.tree).arg("--repository").arg(repository).env("PATH", &self.path);
        if let Some(package) = package {
            command.arg("--package").arg(package);
        }
        if passphrase {
            command.arg("--passphrase-file").arg(self.folder.join("passphrase"));
        }
        command.output().unwrap()
    }

    fn verify(&self, repository: &Path) -> Command {
        let mut command = trainer();
        command.arg("apt-verify").arg("--tree").arg(&self.tree).arg("--repository").arg(repository).args(["--expect-version", "9.9.9"]).env("PATH", &self.path);
        command
    }
}

fn set_version(tree: &Path, version: &str) {
    let path = tree.join("Cargo.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<String> = text.lines().map(|line| if line.starts_with("version = \"") { format!("version = \"{version}\"") } else { line.to_owned() }).collect();
    std::fs::write(path, lines.join("\n") + "\n").unwrap();
}

fn apt_repository() {
    let folder = scratch("apt");
    let tree = small_tree("apt-tree", &recipe());
    set_version(&tree, "9.9.9");
    let apt = Apt { tree: tree.clone(), path: stubs(&folder), folder: folder.clone() };
    let package = plain_package(&folder, CONTROL);
    let repository = folder.join("repository");
    let built = apt.build(&repository, Some(&package), true);
    assert!(built.status.success(), "{}", said(&built));
    assert!(repository.join("dists/stable/InRelease").is_file() && repository.join("index.html").is_file());
    let read = apt.verify(&repository).output().unwrap();
    assert!(read.status.success() && said(&read).contains("APT_REPOSITORY_OK"), "{}", said(&read));
    assert!(apt.verify(&repository).arg("--public-key").arg(tree.join("packaging/keyswitch-archive-keyring.asc")).output().unwrap().status.success());
    // What it refuses to build.
    let refused = |package: Option<&Path>, words: &str| {
        let answer = apt.build(&folder.join("refused"), package, false);
        assert!(!answer.status.success() && said(&answer).contains(words), "{words}: {}", said(&answer));
    };
    refused(None, "at least one package");
    let other_folder = folder.join("other-architecture");
    std::fs::create_dir_all(&other_folder).unwrap();
    refused(Some(&plain_package(&other_folder, &CONTROL.replace("amd64", "arm64"))), "built for arm64");
    let other_folder = folder.join("other-name");
    std::fs::create_dir_all(&other_folder).unwrap();
    refused(Some(&plain_package(&other_folder, &CONTROL.replace("Package: keyswitch", "Package: other"))), "is not a keyswitch package");
    let other_folder = folder.join("computed");
    std::fs::create_dir_all(&other_folder).unwrap();
    refused(Some(&plain_package(&other_folder, &format!("{CONTROL}Size: 1\n"))), "carries a Size");
    refused(Some(Path::new("/nonexistent/package.deb")), "");
    let signing = trainer().arg("apt-build").arg("--tree").arg(&tree).arg("--repository").arg(folder.join("unsigned")).arg("--package").arg(&package).env("PATH", &apt.path).env("KS_NO_GPG", "1").output().unwrap();
    assert!(!signing.status.success());
    // The stanza that says where the repository is.
    let stanza = tree.join("packaging/debian/keyswitch.sources");
    let good = std::fs::read_to_string(&stanza).unwrap();
    for (changed, words) in [
        (good.replace("URIs: https://olegius88.github.io/keyswitch/", "URIs: http://example.com/"), "must serve HTTPS"),
        (good.replace("URIs: https://olegius88.github.io/keyswitch/", "URIs: https://example.com"), "must serve HTTPS"),
        (good.replace("Types: deb", "Types: deb-src"), "declare a binary repository"),
        (good.replace("Suites: stable", "Suites: stable testing"), "exactly one Suites"),
        (good.replace("Components: main", ""), "missing the Components"),
    ] {
        std::fs::write(&stanza, changed).unwrap();
        refused(Some(&package), words);
    }
    std::fs::write(&stanza, &good).unwrap();
    // A landing page with a hole, a manifest with no version.
    let cargo = tree.join("Cargo.toml");
    let manifest = std::fs::read_to_string(&cargo).unwrap();
    std::fs::write(&cargo, "[package]\n").unwrap();
    refused(Some(&package), "");
    std::fs::write(&cargo, manifest).unwrap();
    // What it refuses to read back.
    let refuses = |environment: &[(&str, &str)], words: &str| {
        let answer = apt.verify(&repository).envs(environment.iter().copied()).output().unwrap();
        assert!(!answer.status.success() && said(&answer).contains(words), "{words}: {}", said(&answer));
    };
    refuses(&[("KS_NO_GPG", "1")], "");
    refuses(&[("KS_NO_GPGV", "1")], "");
    refuses(&[("KS_NO_APT", "1")], "");
    refuses(&[("KS_NO_CANDIDATE", "1")], "no candidate");
    refuses(&[("KS_CANDIDATE", "1.0.0")], "APT offers 1.0.0");
    let dists = repository.join("dists/stable");
    let release = std::fs::read_to_string(dists.join("Release")).unwrap();
    let inline = std::fs::read_to_string(dists.join("InRelease")).unwrap();
    let with = |text: &str, words: &str| {
        std::fs::write(dists.join("Release"), text).unwrap();
        std::fs::write(dists.join("InRelease"), text).unwrap();
        refuses(&[], words);
    };
    std::fs::write(dists.join("InRelease"), "something else").unwrap();
    refuses(&[], "does not cover");
    let first_line = release.lines().find(|line| line.starts_with(' ')).unwrap().to_owned();
    with(&format!(" {first_line}\n{release}"), "before naming it");
    with(&release.replace(&first_line, " one two"), "malformed digest");
    with(&release.replace(&first_line, &format!(" {} 1 {}", "0".repeat(32), first_line.split_whitespace().last().unwrap())), "misstates");
    with(&release.replace(&first_line, &format!(" {} 1 {}", first_line.split_whitespace().next().unwrap(), first_line.split_whitespace().last().unwrap())), "misstates");
    let mut fewer = String::new();
    for line in release.lines() {
        if line != first_line {
            fewer.push_str(line);
            fewer.push('\n');
        }
    }
    with(&fewer, "does not cover both");
    std::fs::write(dists.join("Release"), &release).unwrap();
    std::fs::write(dists.join("InRelease"), &inline).unwrap();
    assert!(apt.verify(&repository).output().unwrap().status.success());
    assert!(!apt.verify(&folder.join("gone")).output().unwrap().status.success());
}

const PACKED: [(&str, &str); 13] = [
    ("resources/models/layout_intent_v1.ksm", "usr/lib/keyswitch/resources/models/layout_intent_v1.ksm"),
    ("resources/models/context_policy_v1.json", "usr/lib/keyswitch/resources/models/context_policy_v1.json"),
    ("resources/models/prefix_policy_v1.json", "usr/lib/keyswitch/resources/models/prefix_policy_v1.json"),
    ("resources/models/boundary-v2.json", "usr/lib/keyswitch/resources/models/boundary-v2.json"),
    ("resources/models/ortho_v1.json", "usr/lib/keyswitch/resources/models/ortho_v1.json"),
    ("model/intent_v1/sources/en_US.lm", "usr/lib/keyswitch/resources/models/en_US.lm"),
    ("model/intent_v1/sources/ru_RU.lm", "usr/lib/keyswitch/resources/models/ru_RU.lm"),
    ("model/intent_v1/sources/COPYRIGHT.onboard-data", "usr/share/doc/keyswitch/licenses/copyright.onboard-data"),
    ("packaging/third-party-licenses/linux.txt", "usr/share/doc/keyswitch/licenses/third-party-rust.txt"),
    ("resources/wayland-input/60-keyswitch.rules", "usr/lib/keyswitch/resources/wayland-input/60-keyswitch.rules"),
    ("resources/wayland-input/keyswitch.modules-load.conf", "usr/lib/keyswitch/resources/wayland-input/keyswitch.modules-load.conf"),
    ("packaging/gnome-shell/keyswitch@io.github.olegius88.KeySwitch/metadata.json", "usr/share/gnome-shell/extensions/keyswitch@io.github.olegius88.KeySwitch/metadata.json"),
    ("packaging/gnome-shell/keyswitch@io.github.olegius88.KeySwitch/extension.js", "usr/share/gnome-shell/extensions/keyswitch@io.github.olegius88.KeySwitch/extension.js"),
];

/// A package whose files are those of the tree, with this very program as its binary.
fn whole_package(folder: &Path, tree: &Path) -> PathBuf {
    let package = plain_package(folder, CONTROL);
    let root = folder.join("keyswitch_9.9.9_amd64.deb.root");
    for (known, packed) in PACKED {
        let target = root.join(packed);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::copy(tree.join(known), target).unwrap();
    }
    for (known, packed) in [("packaging/debian/keyswitch.sources", "etc/apt/sources.list.d/keyswitch.sources"), ("packaging/keyswitch-archive-keyring.asc", "etc/apt/keyrings/keyswitch-archive-keyring.asc")] {
        let target = root.join(packed);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::copy(tree.join(known), target).unwrap();
    }
    let binary = root.join("usr/bin/keyswitch");
    std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
    std::fs::copy(std::env::current_exe().unwrap(), &binary).unwrap();
    let meta = folder.join("keyswitch_9.9.9_amd64.deb.meta");
    std::fs::create_dir_all(&meta).unwrap();
    std::fs::write(meta.join("conffiles"), "/etc/apt/sources.list.d/keyswitch.sources\n/etc/apt/keyrings/keyswitch-archive-keyring.asc\n").unwrap();
    package
}

fn deb_check_of(tree: &Path, package: &Path, path: &std::ffi::OsString, environment: &[(&str, &str)]) -> Output {
    trainer().arg("deb-check").arg("--tree").arg(tree).arg(package).env("PATH", path).envs(environment.iter().copied()).output().unwrap()
}

fn debian_package() {
    let folder = scratch("deb");
    let tree = small_tree("deb-tree", &recipe());
    set_version(&tree, "9.9.9");
    let path = stubs(&folder);
    let package = whole_package(&folder, &tree);
    let root = folder.join("keyswitch_9.9.9_amd64.deb.root");
    let meta = folder.join("keyswitch_9.9.9_amd64.deb.meta");
    let good = deb_check_of(&tree, &package, &path, &[]);
    assert!(good.status.success() && said(&good).contains("NATIVE_DEB_OK"), "{}", said(&good));
    let refused = |words: &str, environment: &[(&str, &str)]| {
        let answer = deb_check_of(&tree, &package, &path, environment);
        assert!(!answer.status.success() && said(&answer).contains(words), "{words}: {}", said(&answer));
    };
    // Each file changed, or gone.
    let changing = |relative: &str, bytes: &[u8], words: &str| {
        let target = root.join(relative);
        let before = std::fs::read(&target).unwrap();
        std::fs::write(&target, bytes).unwrap();
        refused(words, &[]);
        std::fs::write(&target, before).unwrap();
    };
    changing("usr/lib/keyswitch/resources/models/boundary-v2.json", b"{}", "differs from");
    changing("usr/share/doc/keyswitch/licenses/copyright.onboard-data", b"changed", "differs from");
    changing("etc/apt/sources.list.d/keyswitch.sources", b"changed", "differs from");
    // The model container.
    let model = "usr/lib/keyswitch/resources/models/layout_intent_v1.ksm";
    changing(model, &vec![0; 15 << 20], "exceeds its packaging bound");
    changing(model, b"short", "header is truncated");
    changing(model, &[b'K'; 60], "header is truncated");
    let real = std::fs::read(root.join(model)).unwrap();
    let mut bounds = real.clone();
    bounds[8..12].copy_from_slice(&0_u32.to_le_bytes());
    changing(model, &bounds, "invalid bounds");
    let mut payload = real.clone();
    payload[12..16].copy_from_slice(&(13_u32 << 20).to_le_bytes());
    changing(model, &payload, "invalid bounds");
    let mut extra = real.clone();
    extra.push(0);
    changing(model, &extra, "invalid bounds");
    // What the program does with its files.
    for absent in ["usr/lib/udev/rules.d/60-keyswitch.rules", "usr/lib/modules-load.d/keyswitch.conf"] {
        let target = root.join(absent);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"x").unwrap();
        refused("must only offer", &[]);
        std::fs::remove_file(&target).unwrap();
    }
    std::fs::write(meta.join("postinst"), b"x").unwrap();
    refused("must only offer", &[]);
    std::fs::remove_file(meta.join("postinst")).unwrap();
    let conffiles = std::fs::read(meta.join("conffiles")).unwrap();
    std::fs::write(meta.join("conffiles"), "/etc/apt/keyrings/keyswitch-archive-keyring.asc\n").unwrap();
    refused("is not a conffile", &[]);
    std::fs::write(meta.join("conffiles"), &conffiles).unwrap();
    // The APT source and its key, changed in the tree and the package alike.
    let stanza = tree.join("packaging/debian/keyswitch.sources");
    let packed_stanza = root.join("etc/apt/sources.list.d/keyswitch.sources");
    let stanza_text = std::fs::read_to_string(&stanza).unwrap();
    for (changed, words) in [
        (stanza_text.replace("https://olegius88.github.io/keyswitch/", "http://example.com/"), "does not serve HTTPS"),
        (stanza_text.replace("https://olegius88.github.io/keyswitch/", "https://example.com"), "does not serve HTTPS"),
        (stanza_text.replace("Signed-By:", "Signed:"), "has no Signed-By"),
    ] {
        std::fs::write(&stanza, &changed).unwrap();
        std::fs::write(&packed_stanza, &changed).unwrap();
        refused(words, &[]);
    }
    std::fs::write(&stanza, &stanza_text).unwrap();
    std::fs::write(&packed_stanza, &stanza_text).unwrap();
    let key = tree.join("packaging/keyswitch-archive-keyring.asc");
    let packed_key = root.join("etc/apt/keyrings/keyswitch-archive-keyring.asc");
    let real_key = std::fs::read_to_string(&packed_key).unwrap();
    for changed in ["no block at all", "-----BEGIN PGP PUBLIC KEY BLOCK-----\n-----BEGIN PGP PRIVATE KEY BLOCK-----\n"] {
        std::fs::write(&key, changed).unwrap();
        std::fs::write(&packed_key, changed).unwrap();
        refused("the public key alone", &[]);
    }
    std::fs::write(&key, &real_key).unwrap();
    std::fs::write(&packed_key, &real_key).unwrap();
    // The binary.
    let binary = root.join("usr/bin/keyswitch");
    let program = std::fs::read(&binary).unwrap();
    std::fs::write(&binary, b"#!/bin/sh\n").unwrap();
    refused("no ELF executable", &[]);
    std::fs::write(&binary, &program).unwrap();
    let python = root.join("usr/lib/keyswitch/left.py");
    std::fs::write(&python, b"").unwrap();
    refused("contains Python", &[]);
    std::fs::remove_file(&python).unwrap();
    refused("the program says", &[("KS_FAKE_VERSION", "KeySwitch 1.0")]);
    refused("ended with", &[("KS_FAKE_DIAGNOSE", "9")]);
    refused("the diagnostic:", &[("KS_FAKE_DIAGNOSE", "garbage")]);
    refused("names no model", &[("KS_FAKE_DIAGNOSE", "nomodel")]);
    refused("did not load the exact", &[("KS_FAKE_DIAGNOSE", "other")]);
    // The fields of the package.
    for (changed, words) in [
        (CONTROL.replace("Package: keyswitch", "Package: other"), "Package is other"),
        (CONTROL.replace("9.9.9", "1.0.0"), "Version is 1.0.0"),
        (CONTROL.replace("amd64", "arm64"), "Architecture is arm64"),
        (CONTROL.replace("libgtk-4-1", "python3:any"), "depends on the Python"),
    ] {
        std::fs::write(folder.join("keyswitch_9.9.9_amd64.deb.control"), &changed).unwrap();
        refused(words, &[]);
    }
    std::fs::write(folder.join("keyswitch_9.9.9_amd64.deb.control"), CONTROL).unwrap();
    refused("", &[("KS_NO_DPKG", "1")]);
    let cargo = tree.join("Cargo.toml");
    let manifest = std::fs::read(&cargo).unwrap();
    std::fs::write(&cargo, "[package]\n").unwrap();
    refused("no version", &[]);
    std::fs::write(&cargo, manifest).unwrap();
    let answer = trainer().arg("deb-check").arg("--tree").arg("/nonexistent").arg(&package).env("PATH", &path).output().unwrap();
    assert!(!answer.status.success());
}

/// A repository of the shape a release reads, with a bare remote, and the programs around a release as stubs.
struct Project {
    root: PathBuf,
    path: std::ffi::OsString,
    gh_runs: PathBuf,
    gh_jobs: PathBuf,
    gh_release: PathBuf,
}

fn in_git(root: &Path, arguments: &[&str]) -> String {
    let done = Command::new("git").current_dir(root).args(arguments).output().unwrap();
    assert!(done.status.success(), "git {arguments:?}: {}", said(&done));
    String::from_utf8_lossy(&done.stdout).trim().to_owned()
}

fn notes(version: &str) -> String {
    let files = [
        format!("KeySwitch-Setup-{version}-x64.exe"),
        format!("KeySwitch-Setup-{version}-arm64.exe"),
        format!("KeySwitch-Setup-{version}-x86.exe"),
        format!("KeySwitch-{version}-windows-x64.zip"),
        format!("KeySwitch-{version}-windows-arm64.zip"),
        format!("KeySwitch-{version}-windows-x86.zip"),
        format!("keyswitch_{version}_amd64.deb"),
        format!("KeySwitch-{version}-macos-arm64.zip"),
        format!("KeySwitch-{version}-macos-x86_64.zip"),
    ];
    format!("# KeySwitch {version}\n\nThe files: {}.\n", files.join(", "))
}

fn project(name: &str) -> Project {
    let folder = scratch(name);
    let remote = folder.join("remote.git");
    std::fs::create_dir_all(&remote).unwrap();
    in_git(&remote, &["init", "--bare", "-q", "-b", "main"]);
    let root = folder.join("tree");
    std::fs::create_dir_all(root.join("packaging")).unwrap();
    std::fs::create_dir_all(root.join("target/train")).unwrap();
    in_git(&root, &["init", "-q", "-b", "main"]);
    in_git(&root, &["config", "user.email", "test@example.org"]);
    in_git(&root, &["config", "user.name", "Test"]);
    in_git(&root, &["remote", "add", "origin", remote.to_str().unwrap()]);
    std::fs::write(root.join("Cargo.toml"), "[workspace.package]\nversion = \"1.2.3\"\n").unwrap();
    std::fs::write(root.join("packaging/keyswitch.1"), ".TH KEYSWITCH 1 \"2026\" \"KeySwitch 1.2.3\"\n").unwrap();
    std::fs::write(root.join("README.md"), "[скачать](https://example.org/releases/tag/v1.2.3)\n").unwrap();
    std::fs::write(root.join("README.en.md"), "[download](https://example.org/releases/tag/v1.2.3)\n").unwrap();
    std::fs::write(root.join("CHANGELOG.md"), "# Changes\n\n## Unreleased\n\n- a change\n\n## 1.2.3 — 2026-01-01\n\n- old\n").unwrap();
    std::fs::write(root.join("RELEASE_NOTES.md"), notes("1.3.0")).unwrap();
    std::fs::write(root.join(".gitignore"), "target\n").unwrap();
    in_git(&root, &["add", "-A"]);
    in_git(&root, &["commit", "-q", "-m", "start"]);
    in_git(&root, &["push", "-q", "origin", "main"]);
    let bin = folder.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let trainer_stub = root.join("target/train/ks-train");
    // A replay keeps the report it replaced in the history, as the real one does; a step may also leave a stray file.
    executable(&trainer_stub, "#!/bin/sh\nif [ \"$1\" = engine ] && [ -z \"$KS_SAME_REPORT\" ]; then mkdir -p model/boundary_v1/engine-history && : > model/boundary_v1/engine-history/0.json; fi\nif [ \"$1\" = \"$KS_STRAY_STEP\" ]; then : > stray; fi\n[ \"$1\" != \"$KS_FAIL_STEP\" ]\n");
    executable(&bin.join("cargo"), "#!/bin/sh\n[ \"$1\" != \"$KS_FAIL_CARGO\" ]\n");
    std::fs::create_dir_all(root.join("target/parity")).unwrap();
    executable(&root.join("target/parity/field-gate"), "#!/bin/sh\ncase \"$*\" in\n  *--speller*) [ \"$KS_FAIL_GATE\" != speller ] ;;\n  *) [ \"$KS_FAIL_GATE\" != plain ] ;;\nesac\n");
    executable(&bin.join("dbus-run-session"), "#!/bin/sh\nshift\nexec \"$@\"\n");
    executable(&bin.join("bash"), "#!/bin/sh\nexit 0\n");
    executable(&bin.join("dpkg"), "#!/bin/sh\necho amd64\n");
    // `gh run list` gives the next line of the answers, and fails where the line says so; `gh run view <id>` the jobs
    // written for that run, or none.
    let (gh_runs, gh_jobs, gh_release) = (folder.join("runs.jsonl"), folder.join("jobs.json"), folder.join("release.json"));
    executable(
        &bin.join("gh"),
        &format!(
            "#!/bin/sh\nif [ \"$1 $2\" = \"run list\" ]; then\n  line=\"$(head -n 1 {runs})\"\n  sed -i 1d {runs}\n  [ \"$line\" != fail ] || exit 1\n  printf '%s\\n' \"$line\"\nelif [ \"$1 $2\" = \"run view\" ]; then\n  if [ -f {jobs}.$3 ]; then cat {jobs}.$3; else echo '{{\"jobs\": []}}'; fi\nelse\n  cat {release}\nfi\n",
            runs = gh_runs.display(),
            jobs = gh_jobs.display(),
            release = gh_release.display()
        ),
    );
    let mut path = bin.into_os_string();
    path.push(":");
    path.push(std::env::var_os("PATH").unwrap());
    let project = Project { root, path, gh_runs, gh_jobs, gh_release };
    // The workflow has checked the commit a release starts from, unless a test says otherwise.
    let checked = runs(&[run(7, "completed", "success", "main")]);
    project.answers(&[&checked, &checked]);
    project.jobs(7, CHECKED);
    project
}

/// A run as `gh run list` gives it.
fn run(id: u64, status: &str, conclusion: &str, branch: &str) -> String {
    format!(r#"{{"databaseId": {id}, "status": "{status}", "conclusion": "{conclusion}", "headBranch": "{branch}", "url": "https://example.org/run/{id}"}}"#)
}

/// One answer of `gh run list`.
fn runs(listed: &[String]) -> String {
    format!("[{}]", listed.join(", "))
}

/// The jobs of a run that checked and built everything, and of one that had nothing to check.
const CHECKED: &str = r#"{"jobs": [{"name": "Tests", "status": "completed", "conclusion": "success"}, {"name": "Every line and branch of the program ran", "status": "completed", "conclusion": "success"}]}"#;
const STOOD_ASIDE: &str = r#"{"jobs": [{"name": "Every line and branch of the program ran", "status": "completed", "conclusion": "skipped"}]}"#;

impl Project {
    fn release(&self, arguments: &[&str]) -> Output {
        self.release_with(arguments, &[])
    }

    fn release_with(&self, arguments: &[&str], environment: &[(&str, &str)]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ks-release")).current_dir(&self.root).args(arguments).env("PATH", &self.path).envs(environment.iter().copied()).output().unwrap()
    }

    /// What `gh run list` answers, call by call.
    fn answers(&self, lines: &[&str]) {
        std::fs::write(&self.gh_runs, lines.join("\n") + "\n").unwrap();
    }

    /// The jobs `gh run view` gives for a run.
    fn jobs(&self, id: u64, text: &str) {
        std::fs::write(format!("{}.{id}", self.gh_jobs.display()), text).unwrap();
    }
}

fn the_release_command() {
    let refused = |output: &Output, words: &str| assert!(!output.status.success() && said(output).contains(words), "{words}: {}", said(output));
    let release = Command::new(env!("CARGO_BIN_EXE_ks-release"));
    let mut bare = release;
    refused(&bare.output().unwrap(), "usage");
    let outside = scratch("no-git");
    refused(&Command::new(env!("CARGO_BIN_EXE_ks-release")).current_dir(&outside).args(["--version", "1.3.0"]).env("GIT_CEILING_DIRECTORIES", outside.parent().unwrap()).output().unwrap(), "git");
    // A dry run writes nothing.
    let tree = project("release-dry");
    let dry = tree.release(&["--version", "1.3.0", "--dry-run"]);
    assert!(dry.status.success() && said(&dry).contains("dry run"), "{}", said(&dry));
    assert_eq!(in_git(&tree.root, &["status", "--porcelain"]), "");
    refused(&tree.release(&["--version", "1.3.0", "--candidate", "many"]), "takes a number");
    refused(&tree.release(&["--version", "1.3.0", "--branch", "other", "--dry-run"]), "not on other");
    refused(&tree.release(&["--version", "1.3"]), "no version");
    refused(&tree.release(&["--version", "1.4.0", "--skip-checks", "--skip-ci"]), "notes");
    // Committed, tagged, pushed; then nothing left to do but to say so.
    let tree = project("release-push");
    let done = tree.release(&["--version", "1.3.0", "--skip-checks", "--skip-ci"]);
    assert!(done.status.success() && said(&done).contains("pushed"), "{}", said(&done));
    assert!(in_git(&tree.root, &["log", "-1", "--format=%s"]).contains("Release KeySwitch 1.3.0"));
    assert_eq!(in_git(&tree.root, &["tag", "--list", "v1.3.0"]), "v1.3.0");
    let again = tree.release(&["--version", "1.3.0", "--skip-checks", "--skip-ci"]);
    assert!(again.status.success(), "{}", said(&again));
    // Published, and the tree has moved on.
    std::fs::write(tree.root.join("new"), b"").unwrap();
    refused(&tree.release(&["--version", "1.3.0", "--skip-checks", "--skip-ci"]), "already published");
    // A message of its own, a candidate's own tag.
    let tree = project("release-message");
    let message = tree.root.parent().unwrap().join("message.txt");
    std::fs::write(&message, "Its own words\n").unwrap();
    let done = tree.release(&["--version", "1.3.0", "--candidate", "1", "--skip-checks", "--skip-ci", "--message-file", message.to_str().unwrap()]);
    assert!(done.status.success(), "{}", said(&done));
    assert_eq!(in_git(&tree.root, &["log", "-1", "--format=%s"]), "Its own words");
    assert_eq!(in_git(&tree.root, &["tag", "--list", "v1.3.0-rc1"]), "v1.3.0-rc1");
    let tree = project("release-gone-message");
    refused(&tree.release(&["--version", "1.3.0", "--skip-checks", "--skip-ci", "--message-file", "/nonexistent"]), "/nonexistent");
    // The whole way of a release no candidate went before: every check here, the workflow polled until it ends, the assets.
    let names: Vec<Value> = notes("1.3.0").split(": ").nth(1).unwrap().trim_end().trim_end_matches('.').split(", ").map(|name| json!({"name": name})).collect();
    let tree = project("release-whole");
    let whole_arguments = ["--version", "1.3.0", "--field-root", ".", "--field-released", ".", "--poll-seconds", "0", "--timeout-seconds", "60"];
    // The commit it starts from is still being checked before the checks made here and after them, then is; the run of
    // the tag is not there yet, cannot be read twice (which is said once), goes on, and ends.
    tree.answers(&[
        &runs(&[run(7, "in_progress", "", "main")]),
        &runs(&[run(7, "in_progress", "", "main")]),
        &runs(&[run(7, "completed", "success", "main")]),
        "[]",
        "fail",
        "fail",
        &runs(&[run(8, "in_progress", "", "v1.3.0")]),
        &runs(&[run(8, "completed", "success", "v1.3.0")]),
    ]);
    let published = json!({"url": "https://example.org/release", "assets": names}).to_string();
    std::fs::write(&tree.gh_release, &published).unwrap();
    let whole = tree.release(&whole_arguments);
    assert!(whole.status.success() && said(&whole).contains("is released") && said(&whole).contains("https://example.org/run/8"), "{}", said(&whole));
    assert!(said(&whole).contains("the tests of the engine"), "no candidate went before: {}", said(&whole));
    assert_eq!(said(&whole).matches("asking again").count(), 1, "{}", said(&whole));
    assert!(!said(&whole).contains("the run published") && !said(&whole).contains("anew"), "no candidate, nothing to say of one: {}", said(&whole));
    // Each way the workflow or the release can disappoint.
    tree.answers(&[&runs(&[run(8, "completed", "failure", "v1.3.0")])]);
    refused(&tree.release(&["--version", "1.3.0", "--skip-checks", "--poll-seconds", "0"]), "the release workflow of v1.3.0: the run ended with failure");
    // A job that failed is said while the others go on.
    tree.answers(&[&runs(&[run(9, "in_progress", "", "v1.3.0")])]);
    tree.jobs(9, r#"{"jobs": [{"name": "Tests", "status": "in_progress", "conclusion": ""}, {"name": "Windows installer (arm64)", "status": "completed", "conclusion": "failure"}]}"#);
    refused(&tree.release(&["--version", "1.3.0", "--skip-checks", "--poll-seconds", "0"]), "the job «Windows installer (arm64)» ended with failure: https://example.org/run/9");
    tree.answers(&[&runs(&[run(8, "queued", "", "v1.3.0")]), &runs(&[run(8, "queued", "", "v1.3.0")])]);
    refused(&tree.release(&["--version", "1.3.0", "--skip-checks", "--poll-seconds", "0", "--timeout-seconds", "0"]), "the run did not end in time");
    tree.answers(&["[]"]);
    refused(&tree.release(&["--version", "1.3.0", "--skip-checks", "--poll-seconds", "0", "--timeout-seconds", "0"]), "no run appeared in time");
    // An answer that never comes ends the wait long before the time is up.
    tree.answers(&["fail"; 30]);
    refused(&tree.release(&["--version", "1.3.0", "--skip-checks", "--poll-seconds", "0", "--timeout-seconds", "3600"]), "the runs could not be read");
    // What cannot be read is asked for again, until the time is up.
    tree.answers(&["not json"]);
    refused(&tree.release(&["--version", "1.3.0", "--skip-checks", "--poll-seconds", "0", "--timeout-seconds", "0"]), "the runs could not be read");
    tree.answers(&["fail"]);
    refused(&tree.release(&["--version", "1.3.0", "--skip-checks", "--poll-seconds", "0", "--timeout-seconds", "0"]), "the runs could not be read");
    tree.answers(&[&runs(&[run(10, "in_progress", "", "v1.3.0")])]);
    tree.jobs(10, "not json");
    refused(&tree.release(&["--version", "1.3.0", "--skip-checks", "--poll-seconds", "0", "--timeout-seconds", "0"]), "the runs could not be read");
    tree.answers(&[&runs(&[run(8, "completed", "success", "v1.3.0")])]);
    std::fs::write(&tree.gh_release, json!({"url": "https://example.org/release", "assets": []}).to_string()).unwrap();
    refused(&tree.release(&["--version", "1.3.0", "--skip-checks", "--poll-seconds", "0"]), "is missing");
    tree.answers(&[&runs(&[run(8, "completed", "success", "v1.3.0")])]);
    std::fs::write(&tree.gh_release, "not json").unwrap();
    refused(&tree.release(&["--version", "1.3.0", "--skip-checks", "--poll-seconds", "0"]), "gave no answer to read");
    // The commit a release starts from has been checked by the workflow, whole and with no run that failed. What is
    // known already is said before the checks made here take their minutes.
    let unchecked = |name: &str, answers: &[&str], jobs: &str, words: &str, at_once: bool| {
        let tree = project(&format!("release-unchecked-{name}"));
        tree.answers(answers);
        tree.jobs(7, jobs);
        let answer = tree.release(&whole_arguments);
        refused(&answer, words);
        assert_eq!(!said(&answer).contains("$ the trainer"), at_once, "{}", said(&answer));
    };
    let (going, passed, failed) = (runs(&[run(7, "in_progress", "", "main")]), runs(&[run(7, "completed", "success", "main")]), runs(&[run(7, "completed", "failure", "main")]));
    unchecked("none", &["[]"], CHECKED, "no run of the workflow has checked the commit", true);
    unchecked("aside", &[&passed], STOOD_ASIDE, "no run of the workflow has checked the commit", true);
    unchecked("failed", &[&failed], CHECKED, "the workflow on the commit", true);
    // What a run still going, or an answer not read, could not say before them is said after them.
    unchecked("failed-later", &[&going, &failed], CHECKED, "the run ended with failure", false);
    unchecked("aside-later", &["fail", &passed], STOOD_ASIDE, "no run of the workflow has checked the commit", false);
    // Each check that fails stops it; the owner's typing is not skipped, and is asked for before anything runs.
    let tree = project("release-checks");
    refused(&tree.release(&["--version", "1.3.0"]), "a release does not skip them");
    let failing = |variable: &str, value: &str, words: &str| {
        let tree = project(&format!("release-check-{value}"));
        refused(&tree.release_with(&whole_arguments, &[(variable, value)]), words);
    };
    failing("KS_FAIL_CARGO", "test", "the tests of the engine");
    failing("KS_FAIL_CARGO", "build", "the trainer failed");
    failing("KS_FAIL_GATE", "plain", "the owner's typing against");
    // With the reports of the last release replayed with Hunspell, that gate runs too.
    let tree = project("release-check-speller");
    let mut spelled: Vec<&str> = whole_arguments.to_vec();
    spelled.extend(["--field-released-speller", "."]);
    refused(&tree.release_with(&spelled, &[("KS_FAIL_GATE", "speller")]), "with Hunspell");
    failing("KS_FAIL_STEP", "gates", "the evidence");
    failing("KS_FAIL_STEP", "retired", "rejected or retired");
    failing("KS_FAIL_STEP", "engine", "the boundary regressions");
    // A candidate checks here what only this machine can, and leaves the rest to the workflow run of its tag: a test that
    // would fail here is not run here.
    let tree = project("release-candidate");
    let mut candidate: Vec<&str> = whole_arguments.to_vec();
    candidate.extend(["--candidate", "1", "--skip-ci"]);
    let done = tree.release_with(&candidate, &[("KS_FAIL_CARGO", "test"), ("KS_FAIL_STEP", "gates")]);
    assert!(done.status.success() && said(&done).contains("makes every other check") && !said(&done).contains("the tests of the engine"), "{}", said(&done));
    assert_eq!(in_git(&tree.root, &["tag", "--list", "v1.3.0-rc1"]), "v1.3.0-rc1");
    refused(&project("release-candidate-gate").release_with(&candidate, &[("KS_FAIL_GATE", "plain")]), "the owner's typing against");
    refused(&project("release-candidate-engine").release_with(&candidate, &[("KS_FAIL_STEP", "engine")]), "the boundary regressions");
    // A release on the commit of that candidate, once the run of the candidate's tag has checked it whole: the files of the
    // candidate are published, and nothing is checked a second time. The tag is on the candidate's commit.
    let mut release: Vec<&str> = whole_arguments.to_vec();
    release.push("--skip-ci");
    let both = runs(&[run(5, "completed", "success", "main"), run(6, "completed", "success", "v1.3.0-rc1")]);
    let of_the_tag = runs(&[run(9, "completed", "success", "v1.3.0")]);
    tree.answers(&[&both, &both, &of_the_tag]);
    tree.jobs(5, STOOD_ASIDE);
    tree.jobs(6, CHECKED);
    tree.jobs(9, STOOD_ASIDE);
    std::fs::write(&tree.gh_release, &published).unwrap();
    let done = tree.release_with(&whole_arguments, &[("KS_FAIL_CARGO", "test"), ("KS_FAIL_STEP", "gates"), ("KS_SAME_REPORT", "1")]);
    assert!(done.status.success() && said(&done).contains("v1.3.0-rc1 passed every other check on this commit"), "{}", said(&done));
    assert!(said(&done).contains("the run published the files of v1.3.0-rc1") && said(&done).contains("is released"), "{}", said(&done));
    assert_eq!(in_git(&tree.root, &["rev-parse", "v1.3.0^{commit}"]), in_git(&tree.root, &["rev-parse", "v1.3.0-rc1^{commit}"]));
    // The run of the tag may still build everything anew (the files its candidate's run built are kept for a time only):
    // what it did is read from the run, and said.
    tree.answers(&[&both, &both, &of_the_tag]);
    tree.jobs(9, CHECKED);
    let done = tree.release_with(&whole_arguments, &[("KS_FAIL_CARGO", "test"), ("KS_SAME_REPORT", "1")]);
    assert!(done.status.success() && said(&done).contains("built everything anew instead of publishing the files of v1.3.0-rc1"), "{}", said(&done));
    // Not that release: the reports changed after the candidate, the run that passed was of a tag that is no candidate of
    // this version, or the candidate's run had nothing to check. Every check is then made here.
    let other = |name: &str, answer: &str, same_report: &str| {
        let tree = project(&format!("release-other-{name}"));
        assert!(tree.release(&["--version", "1.3.0", "--candidate", "1", "--skip-checks", "--skip-ci"]).status.success());
        tree.answers(&[answer, answer]);
        tree.jobs(5, CHECKED);
        tree.jobs(6, CHECKED);
        refused(&tree.release_with(&release, &[("KS_FAIL_CARGO", "test"), ("KS_SAME_REPORT", same_report)]), "the tests of the engine");
    };
    other("changed", &both, "");
    other("version", &runs(&[run(5, "completed", "success", "main"), run(6, "completed", "success", "v1.2.9-rc1")]), "1");
    other("number", &runs(&[run(5, "completed", "success", "main"), run(6, "completed", "success", "v1.3.0-rcx")]), "1");
    let tree = project("release-other-aside");
    assert!(tree.release(&["--version", "1.3.0", "--candidate", "1", "--skip-checks", "--skip-ci"]).status.success());
    tree.answers(&[&both, &both]);
    tree.jobs(5, CHECKED);
    tree.jobs(6, STOOD_ASIDE);
    refused(&tree.release_with(&release, &[("KS_FAIL_CARGO", "test"), ("KS_SAME_REPORT", "1")]), "the tests of the engine");
    // A file the release does not write stops it: one a check leaves before the commit, one in the tree before it starts.
    failing("KS_STRAY_STEP", "gates", "the checks changed files a release does not write: stray");
    let tree = project("release-foreign");
    std::fs::write(tree.root.join(".gitignore"), "target\nother\n").unwrap();
    refused(&tree.release(&whole_arguments), "the tree has changed files a release does not write: .gitignore");
    // Nothing to describe.
    let tree = project("release-no-entries");
    std::fs::write(tree.root.join("CHANGELOG.md"), "# Changes\n\n## Unreleased\n\n## 1.3.0 — 2026-01-01\n\n").unwrap();
    refused(&tree.release(&["--version", "1.3.0", "--skip-checks", "--skip-ci"]), "nothing to describe");
}

fn answer_as_the_program(arguments: &[String]) -> bool {
    match arguments.first().map(String::as_str) {
        Some("--version") => {
            println!("{}", std::env::var("KS_FAKE_VERSION").unwrap_or_else(|_| "KeySwitch 9.9.9".to_owned()));
            true
        }
        Some("--diagnose") => {
            let kind = std::env::var("KS_FAKE_DIAGNOSE").unwrap_or_default();
            if kind == "9" {
                std::process::exit(9);
            }
            if kind == "garbage" {
                println!("not json");
                return true;
            }
            if kind == "nomodel" {
                println!("{}", json!({"intent_model": {}}));
                return true;
            }
            let resources = PathBuf::from(std::env::var("KEYSWITCH_RESOURCES").unwrap());
            let intent = resources.join("models/layout_intent_v1.ksm");
            let bytes = std::fs::read(&intent).unwrap();
            let manifest_length = usize::try_from(u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]])).unwrap();
            let manifest: Value = serde_json::from_slice(&bytes[52..52 + manifest_length]).unwrap();
            let checksum = if kind == "other" { "0".to_owned() } else { digest(&bytes) };
            println!("{}", json!({"intent_model": {"available": true, "path": intent, "checksum": checksum, "version": manifest["model_version"]}}));
            true
        }
        _ => false,
    }
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if answer_with_another_model(&arguments) || answer_as_the_program(&arguments) {
        return;
    }
    let _ = tree();
    the_release_command();
    apt_repository();
    apt_landing_page();
    debian_package();
    more_of_the_debian_package();
    println!("the release tools answer as they should");
}

/// A tree of the files a package check reads: the repository's, with the notice of the word lists and the manifest copied.
fn package_tree(name: &str) -> PathBuf {
    let root = scratch(name);
    let linked = PACKED.iter().map(|(known, _)| *known).chain(["packaging/debian/keyswitch.sources", "packaging/keyswitch-archive-keyring.asc"]);
    for relative in linked {
        let target = root.join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        if relative.ends_with("COPYRIGHT.onboard-data") {
            std::fs::copy(tree().join(relative), target).unwrap();
        } else {
            std::os::unix::fs::symlink(tree().join(relative), target).unwrap();
        }
    }
    std::fs::copy(tree().join("Cargo.toml"), root.join("Cargo.toml")).unwrap();
    set_version(&root, "9.9.9");
    root
}

/// A model container of this manifest and a payload of so many bytes.
fn container(manifest: &[u8], payload: usize) -> Vec<u8> {
    let mut bytes = b"KSLM\x04\0\0\0".to_vec();
    bytes.extend_from_slice(&u32::try_from(manifest.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(payload).unwrap().to_le_bytes());
    bytes.resize(52, 0);
    bytes.extend_from_slice(manifest);
    bytes.resize(bytes.len() + payload, 0);
    bytes
}

fn more_of_the_debian_package() {
    let folder = scratch("deb-more");
    let tree = package_tree("deb-more-tree");
    let path = stubs(&folder);
    let package = whole_package(&folder, &tree);
    let root = folder.join("keyswitch_9.9.9_amd64.deb.root");
    let check = |path: &std::ffi::OsString, environment: &[(&str, &str)]| deb_check_of(&tree, &package, path, environment);
    let refused = |words: &str, environment: &[(&str, &str)]| {
        let answer = check(&path, environment);
        assert!(!answer.status.success() && said(&answer).contains(words), "{words}: {}", said(&answer));
    };
    let passed = |path: &std::ffi::OsString, environment: &[(&str, &str)]| {
        let answer = check(path, environment);
        assert!(answer.status.success() && said(&answer).contains("NATIVE_DEB_OK"), "{}", said(&answer));
    };
    // A file too short to be a program is not one; a diagnostic that ends with 1 has still reported.
    std::fs::write(root.join("usr/lib/keyswitch/two"), b"ab").unwrap();
    passed(&path, &[]);
    passed(&path, &[("KS_OTHER_MODEL", "status-one")]);
    // The diagnostic names another model than the one the package carries.
    refused("did not load the exact", &[("KS_OTHER_MODEL", "unavailable")]);
    refused("did not load the exact", &[("KS_OTHER_MODEL", "elsewhere")]);
    refused("did not load the exact", &[("KS_OTHER_MODEL", "another-version")]);
    // Python in any of its forms.
    for name in ["left.pyc", "left.pyo"] {
        let python = root.join("usr/lib/keyswitch").join(name);
        std::fs::write(&python, b"").unwrap();
        refused("contains Python", &[]);
        std::fs::remove_file(&python).unwrap();
    }
    // A program whose libraries are not all there, and a linker that cannot tell.
    let linker = folder.join("linker-bin");
    std::fs::create_dir_all(&linker).unwrap();
    executable(&linker.join("ldd"), "#!/bin/sh\n[ -z \"$KS_LDD_FAILS\" ] || exit 1\necho '\tlibmissing.so.1 => not found'\n");
    let mut with_linker = linker.into_os_string();
    with_linker.push(":");
    with_linker.push(&path);
    let answer = check(&with_linker, &[]);
    assert!(!answer.status.success() && said(&answer).contains("unresolved shared libraries"), "{}", said(&answer));
    let answer = check(&with_linker, &[("KS_LDD_FAILS", "1")]);
    assert!(!answer.status.success() && said(&answer).contains("ldd"), "{}", said(&answer));
    // A system without `timeout` cannot run the diagnostic at all.
    let bare = folder.join("bare-bin");
    std::fs::create_dir_all(&bare).unwrap();
    for program in ["sed", "cat", "cp"] {
        let found = std::env::split_paths(&std::env::var_os("PATH").unwrap()).map(|folder| folder.join(program)).find(|candidate| candidate.is_file()).unwrap();
        std::os::unix::fs::symlink(found, bare.join(program)).unwrap();
    }
    executable(&bare.join("ldd"), "#!/bin/sh\necho '\tlinux-vdso.so.1'\n");
    let mut without_timeout = folder.join("stub-bin").into_os_string();
    without_timeout.push(":");
    without_timeout.push(&bare);
    let answer = check(&without_timeout, &[]);
    assert!(!answer.status.success() && said(&answer).lines().any(|line| line == "No such file or directory (os error 2)"), "{}", said(&answer));
    // The notice of the word lists, short of a line in the tree and the package alike.
    let notice = tree.join("model/intent_v1/sources/COPYRIGHT.onboard-data");
    let packed_notice = root.join("usr/share/doc/keyswitch/licenses/copyright.onboard-data");
    let full = std::fs::read_to_string(&notice).unwrap();
    let short: String = full.lines().filter(|line| *line != "License: GPL-3+").map(|line| format!("{line}\n")).collect();
    std::fs::write(&notice, &short).unwrap();
    std::fs::write(&packed_notice, &short).unwrap();
    refused("notice lost a line: License: GPL-3+", &[]);
    std::fs::write(&notice, &full).unwrap();
    std::fs::write(&packed_notice, &full).unwrap();
    // The packaged key alone changed.
    let packed_key = root.join("etc/apt/keyrings/keyswitch-archive-keyring.asc");
    let key = std::fs::read(&packed_key).unwrap();
    std::fs::write(&packed_key, b"another key").unwrap();
    refused("differs from", &[]);
    std::fs::write(&packed_key, &key).unwrap();
    // Containers whose manifest does not describe their payload.
    let model = root.join("usr/lib/keyswitch/resources/models/layout_intent_v1.ksm");
    let real = std::fs::read(&model).unwrap();
    for (manifest, payload, words) in [
        (json!({"supported_fingerprint_count": (1 << 20) + 1, "dimension": 1}), 10, "payload shape"),
        (json!({"dimension": 1}), 10, "payload shape"),
        (json!({"supported_fingerprint_count": 1, "dimension": 0}), 8, "payload shape"),
        (json!({"supported_fingerprint_count": 1}), 8, "payload shape"),
        (json!({"supported_fingerprint_count": 1, "dimension": 1}), 9, "payload shape"),
        (json!({"supported_fingerprint_count": 1, "dimension": 1}), 10, "a manifest without a version"),
    ] {
        std::fs::write(&model, container(manifest.to_string().as_bytes(), payload)).unwrap();
        refused(words, &[]);
    }
    std::fs::write(&model, container(b"{x", 0)).unwrap();
    refused("", &[]);
    std::fs::write(&model, &real).unwrap();
    passed(&path, &[]);
}

/// More answers of the program: its diagnostic naming a model other than the one it carries, or ending with 1.
fn answer_with_another_model(arguments: &[String]) -> bool {
    let kind = std::env::var("KS_OTHER_MODEL").unwrap_or_default();
    if kind.is_empty() || arguments.first().map(String::as_str) != Some("--diagnose") {
        return false;
    }
    let resources = PathBuf::from(std::env::var("KEYSWITCH_RESOURCES").unwrap());
    let intent = resources.join("models/layout_intent_v1.ksm");
    let bytes = std::fs::read(&intent).unwrap();
    let manifest_length = usize::try_from(u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]])).unwrap();
    let manifest: Value = serde_json::from_slice(&bytes[52..52 + manifest_length]).unwrap();
    let mut model = json!({"available": true, "path": intent, "checksum": digest(&bytes), "version": manifest["model_version"]});
    match kind.as_str() {
        "unavailable" => model["available"] = json!(false),
        "elsewhere" => model["path"] = json!(resources.join("models/boundary-v2.json")),
        "another-version" => model["version"] = json!("intent-v1-another"),
        _ => {}
    }
    println!("{}", json!({"intent_model": model}));
    if kind == "status-one" {
        std::process::exit(1);
    }
    true
}

/// A landing page with a placeholder the build does not know is not published.
fn apt_landing_page() {
    let folder = scratch("apt-page");
    let root = folder.join("tree");
    for relative in ["packaging/debian/keyswitch.sources", "packaging/keyswitch-archive-keyring.asc", "packaging/site/img"] {
        let target = root.join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(tree().join(relative), target).unwrap();
    }
    std::fs::copy(tree().join("Cargo.toml"), root.join("Cargo.toml")).unwrap();
    std::fs::write(root.join("packaging/site/index.html"), "<p>KeySwitch @VERSION@ from @URI@, signed by @UNHEARD_OF@</p>\n").unwrap();
    let apt = Apt { tree: root, path: stubs(&folder), folder: folder.clone() };
    let package = plain_package(&folder, CONTROL);
    let answer = apt.build(&folder.join("repository"), Some(&package), false);
    assert!(!answer.status.success() && said(&answer).contains("keeps @UNHEARD_OF@ unfilled"), "{}", said(&answer));
}

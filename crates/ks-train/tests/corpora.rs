//! The corpora made from their public sources, on small samples of each format: treebanks, package indexes and sentence exports,
//! through the base, its reconciliation, the technical corpus, their merge, a holdout and a fitting extension.
mod support;

use ks_train::base_corpus::BASE_PINS;
use ks_train::freeze::PINS;
use serde_json::{Value, json};
use sha1::Digest as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::LazyLock;
use support::{digest, scratch, trainer, tree};

const RUSSIAN: [&str; 16] = ["ба", "ве", "ги", "до", "ку", "ла", "ме", "но", "пу", "ро", "са", "ти", "фу", "хо", "це", "чу"];
const ENGLISH: [&str; 16] = ["ba", "ve", "gi", "do", "ku", "la", "me", "no", "pu", "ro", "sa", "ti", "fu", "ho", "ce", "chu"];

/// A word no dictionary holds, the same on every machine.
fn word(syllables: &[&str; 16], number: usize) -> String {
    format!("{}{}{}", syllables[number % 16], syllables[(number / 16) % 16], syllables[(number * 7) % 16])
}

fn row(id: &str, form: &str, misc: &str) -> String {
    format!("{id}\t{form}\t{}\tNOUN\t_\t_\t0\troot\t_\t{misc}\n", form.to_lowercase())
}

/// One treebank file of many documents: sentences with text, without it, with text that does not match, with words of several parts.
fn treebank(syllables: &[&str; 16], seed: usize, documents: usize) -> String {
    let mut text = String::new();
    let mut number = seed;
    for document in 0..documents {
        if document % 7 == 6 {
            text.push_str("# newdoc\n");
        } else {
            text.push_str(&format!("# newdoc id = doc{seed}-{document}\n"));
        }
        for sentence in 0..3 {
            let words: Vec<String> = (0..4 + sentence).map(|place| word(syllables, number + place * 3)).collect();
            number += 11;
            text.push_str(&format!("# sent_id = s{seed}-{document}-{sentence}\n"));
            match (document + sentence) % 5 {
                0 => {
                    text.push_str(&format!("# text = {}.\n", words.join(" ")));
                    for (place, form) in words.iter().enumerate() {
                        text.push_str(&row(&(place + 1).to_string(), form, "_"));
                    }
                    text.push_str(&row(&(words.len() + 1).to_string(), ".", "_"));
                }
                1 => {
                    for (place, form) in words.iter().enumerate() {
                        text.push_str(&row(&(place + 1).to_string(), form, if place == 0 { "SpaceAfter=No" } else if place == 1 { "SpacesAfter=\\s\\n" } else { "_" }));
                    }
                }
                2 => {
                    text.push_str(&format!("# text = {} совсем не то\n", words[0]));
                    for (place, form) in words.iter().enumerate() {
                        text.push_str(&row(&(place + 1).to_string(), form, "_"));
                    }
                }
                3 => {
                    let joined = format!("{}{}", words[0], words[1]);
                    text.push_str(&format!("# text = {joined} {}\n", words[2]));
                    text.push_str(&format!("1-2\t{joined}\t_\t_\t_\t_\t_\t_\t_\t_\n"));
                    text.push_str(&row("1", &words[0], "_"));
                    text.push_str("2\t_\t_\tNOUN\t_\t_\t0\troot\t_\t_\n");
                    text.push_str(&row("2.1", "пусто", "_"));
                    text.push_str(&row("3", &words[2], "_"));
                }
                _ => {
                    text.push_str(&format!("# text = {} C++ 42 смесьmix {}\n", words[0], words[1]));
                    text.push_str(&row("1", &words[0], "_"));
                    text.push_str(&row("2", "C++", "_"));
                    text.push_str(&row("3", "42", "_"));
                    text.push_str(&row("4", "смесьmix", "_"));
                    text.push_str(&row("5", &words[1], "_"));
                }
            }
            text.push('\n');
        }
    }
    text
}

fn blob_sha(content: &[u8]) -> String {
    let mut blob = sha1::Sha1::new();
    blob.update(format!("blob {}\0", content.len()).as_bytes());
    blob.update(content);
    blob.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

fn pinned_treebank(root: &Path, source: &str, commit: &str, syllables: &[&str; 16], seed: usize, documents: usize) {
    let folder = root.join(source);
    std::fs::create_dir_all(&folder).expect("a folder");
    let content = treebank(syllables, seed, documents);
    let name = format!("{}-ud-test.conllu", source.to_lowercase());
    std::fs::write(folder.join(&name), &content).expect("a file");
    let readme = b"# a treebank";
    std::fs::write(folder.join("README.md"), readme).expect("a file");
    let file = |path: &str, bytes: &[u8]| json!({"path": path, "sha256": digest(bytes), "sha": blob_sha(bytes)});
    let pin = json!({"commit": commit, "repository": format!("https://github.com/UniversalDependencies/{source}"), "files": [file(&name, content.as_bytes()), file("README.md", readme)]});
    std::fs::write(folder.join("pin.json"), serde_json::to_vec(&pin).expect("json")).expect("a pin");
}

fn gz(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).expect("in memory");
    encoder.finish().expect("in memory")
}

/// A tar archive of plain files, as the indexes are packed.
fn tar(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut output = Vec::new();
    for (name, content) in files {
        let mut header = [0_u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        let size = format!("{:011o}\0", content.len());
        header[124..136].copy_from_slice(size.as_bytes());
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        output.extend_from_slice(&header);
        output.extend_from_slice(content);
        output.resize(output.len().div_ceil(512) * 512, 0);
    }
    output.resize(output.len() + 1024, 0);
    output
}

fn commands(prefix: &str, count: usize) -> Vec<String> {
    (0..count).map(|number| format!("{prefix}{}", word(&ENGLISH, number * 5))).collect()
}

/// A Debian-style index with its receipt and release file.
fn deb_source(folder: &Path, contents_url: &str, release_url: &str, listed: &str, prefix: &str, signed: bool) {
    std::fs::create_dir_all(folder).expect("a folder");
    let mut text = String::from("usr/share/doc/thing                     docs/thing\nnot-a-row\n");
    for (number, name) in commands(prefix, 60).iter().enumerate() {
        let owners = if number % 9 == 0 { format!("utils/{name},admin/both") } else { format!("universe/utils/pkg{}", number / 3) };
        let path = if number % 2 == 0 { format!("usr/bin/{name}") } else { format!("usr/sbin/{name}") };
        text.push_str(&format!("{path}          {owners}\n"));
    }
    text.push_str("bin/XY                         utils/upper\n");
    let contents = gz(text.as_bytes());
    std::fs::write(folder.join("Contents-amd64.gz"), &contents).expect("a file");
    let release = format!("Origin: test\nSHA256:\n {} {} {listed}\nAcquire-By-Hash: yes\n", digest(&contents), contents.len());
    std::fs::write(folder.join("InRelease"), &release).expect("a file");
    let receipt = json!({
        "contents": {"url": contents_url, "status": 200}, "release": {"url": release_url, "status": 200}, "tls_certificate_verification": true,
        "contents_sha256": digest(&contents), "contents_bytes": contents.len(), "release_sha256": digest(release.as_bytes()), "signature": {"verified": signed},
    });
    std::fs::write(folder.join("source-receipt.json"), serde_json::to_vec(&receipt).expect("json")).expect("a receipt");
}

fn arch_source(folder: &Path) {
    std::fs::create_dir_all(folder).expect("a folder");
    let mut databases = json!({});
    for repository in ["core", "extra"] {
        let mut files = Vec::new();
        for (number, name) in commands(&format!("{}a", &repository[..1]), 20).iter().enumerate() {
            let directory = format!("pkg{number}-1.0-1");
            files.push((format!("{directory}/desc"), format!("%FILENAME%\nx\n\n%NAME%\npkg{number}\n").into_bytes()));
            files.push((format!("{directory}/files"), format!("%FILES%\nusr/\nusr/bin/\nusr/bin/{name}\nusr/share/doc\n").into_bytes()));
        }
        let packed = gz(&tar(&files));
        let file = format!("{repository}.files");
        std::fs::write(folder.join(&file), &packed).expect("a file");
        databases[repository] = json!({"download": {"status": 200, "url": format!("https://geo.mirror.pkgbuild.com/{repository}/os/x86_64/{file}")}, "file": file, "sha256": digest(&packed), "bytes": packed.len()});
    }
    let receipt = json!({"tls_certificate_verification": true, "mirror": "https://geo.mirror.pkgbuild.com/", "databases": databases});
    std::fs::write(folder.join("source-receipt.json"), serde_json::to_vec(&receipt).expect("json")).expect("a receipt");
}

fn alpine_source(folder: &Path) {
    std::fs::create_dir_all(folder).expect("a folder");
    let mut index = String::from("C:x\np:cmd:orphan\n\n");
    for (number, name) in commands("alp", 30).iter().enumerate() {
        index.push_str(&format!("P:pkg{number}\nV:1\np:so:libx.so=1 cmd:{name}=1.0 cmd:X\n\n"));
    }
    let packed = gz(&tar(&[("APKINDEX".to_owned(), index.into_bytes())]));
    std::fs::write(folder.join("APKINDEX.tar.gz"), &packed).expect("a file");
    let base = "https://dl-cdn.alpinelinux.org/alpine/v3.21/main/x86_64/";
    let receipt = json!({"base_url": base, "tls_certificate_verification": true, "archive": {"status": 200, "url": format!("{base}APKINDEX.tar.gz"), "sha256": digest(&packed), "bytes": packed.len()}});
    std::fs::write(folder.join("source-receipt.json"), serde_json::to_vec(&receipt).expect("json")).expect("a receipt");
}

fn rpm_source(folder: &Path, algorithm: Option<&str>) {
    use sha2::Sha512;
    std::fs::create_dir_all(folder).expect("a folder");
    let mut primary = String::from("<metadata>\n");
    for (number, name) in commands("rpm", 30).iter().enumerate() {
        primary.push_str(&format!("<package type=\"rpm\"><name>pkg{number}</name><format><file>/usr/share/doc/x</file><file type=\"dir\">/usr/bin</file><file>/usr/bin/{name}</file></format></package>\n"));
    }
    primary.push_str("<package type=\"rpm\"><name>open</name><file>/usr/bin/never</package>\n</metadata>\n");
    let packed = ruzstd::encoding::compress_to_vec(primary.as_bytes(), ruzstd::encoding::CompressionLevel::Fastest);
    std::fs::write(folder.join("primary.xml.zst"), &packed).expect("a file");
    let declared = if algorithm == Some("sha512") { Sha512::digest(&packed).iter().map(|byte| format!("{byte:02x}")).collect() } else { digest(&packed) };
    let repomd = format!("<repomd><data type=\"primary\"><checksum>{declared}</checksum><location href=\"repodata/primary.xml.zst\"/></data></repomd>");
    std::fs::write(folder.join("repomd.xml"), &repomd).expect("a file");
    let mut described = json!({"sha256": digest(&packed), "href": "repodata/primary.xml.zst"});
    if let Some(name) = algorithm {
        described["checksum_type"] = json!(name);
        described["checksum"] = json!(declared);
    }
    let receipt = json!({"base_url": "https://dl.fedoraproject.org/pub/fedora/linux/releases/43/Everything/x86_64/os/", "tls_certificate_verification": true, "repomd": {"status": 200}, "repomd_sha256": digest(repomd.as_bytes()), "primary": described});
    std::fs::write(folder.join("source-receipt.json"), serde_json::to_vec(&receipt).expect("json")).expect("a receipt");
}

fn tatoeba_source(folder: &Path) {
    std::fs::create_dir_all(folder).expect("a folder");
    let base = "https://downloads.tatoeba.org/exports/per_language/";
    let mut files = json!({});
    for language in ["rus", "eng"] {
        let name = format!("{language}_sentences.tsv.bz2");
        let bytes = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tatoeba-many").join(&name)).expect("a sample");
        std::fs::write(folder.join(&name), &bytes).expect("a file");
        files[&name] = json!({"status": 200, "language": language, "url": format!("{base}{language}/{name}"), "sha256": digest(&bytes), "bytes": bytes.len()});
    }
    let receipt = json!({"licence": "CC BY 2.0 FR", "tls_certificate_verification": true, "base_url": base, "files": files});
    std::fs::write(folder.join("source-receipt.json"), serde_json::to_vec(&receipt).expect("json")).expect("a receipt");
}

struct Chain {
    folder: PathBuf,
    base: PathBuf,
    reconciled: PathBuf,
    technical: PathBuf,
    merged: PathBuf,
    prefix: PathBuf,
    holdout_sources: PathBuf,
}

fn run(command: &mut Command) -> Output {
    command.output().expect("the trainer starts")
}

fn said(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn base(action: &str) -> Command {
    let mut command = trainer();
    command.arg("base").arg(action).arg("--tree").arg(tree());
    command
}

fn done(output: &Output) -> String {
    assert!(output.status.success(), "{}", said(output));
    said(output)
}

static CHAIN: LazyLock<Chain> = LazyLock::new(|| {
    let folder = scratch("corpora");
    let sources = folder.join("sources");
    for (index, (name, commit)) in BASE_PINS.iter().enumerate() {
        let syllables = if name.contains("Russian") { &RUSSIAN } else { &ENGLISH };
        pinned_treebank(&sources, name, commit, syllables, 1000 * (index + 1), 40);
    }
    let (corpus, reconciled, technical, merged, prefix) = (folder.join("base"), folder.join("reconciled"), folder.join("technical"), folder.join("merged"), folder.join("prefix-inventory.json"));
    done(&run(base("freeze").arg("--source-root").arg(&sources).arg("--output").arg(&corpus).args(["--namespace", "small:base", "--max-documents", "60", "--max-sentences-per-document", "3"])));
    done(&run(base("reconcile").arg("--corpus").arg(&corpus).arg("--source-root").arg(&sources).arg("--report").arg(folder.join("reconcile.json")).arg("--output").arg(&reconciled)));
    let trixie = folder.join("trixie");
    deb_source(&trixie, "https://deb.debian.org/debian/dists/trixie/main/Contents-amd64.gz", "https://deb.debian.org/debian/dists/trixie/InRelease", "main/Contents-amd64.gz", "tx", true);
    done(&run(base("technical").arg("--source-directory").arg(&trixie).arg("--ud-corpus").arg(&reconciled).arg("--preview-report").arg(folder.join("technical-preview.json")).arg("--output").arg(&technical)));
    done(&run(base("merge").arg("--base").arg(&reconciled).arg("--technical").arg(&technical).arg("--output").arg(&merged)));
    done(&run(base("inventory").arg("--output").arg(&prefix)));
    let holdout_sources = folder.join("holdout-sources");
    for (index, (name, commit)) in PINS.iter().enumerate() {
        let syllables = if name.contains("Russian") { &RUSSIAN } else { &ENGLISH };
        pinned_treebank(&holdout_sources, name, commit, syllables, 5000 + 700 * index, 12);
    }
    Chain { folder, base: corpus, reconciled, technical, merged, prefix, holdout_sources }
});

#[test]
fn each_step_of_the_chain_writes_its_corpus() {
    let chain = &*CHAIN;
    for folder in [&chain.base, &chain.reconciled, &chain.technical, &chain.merged] {
        let manifest: Value = serde_json::from_slice(&std::fs::read(folder.join("manifest.json")).expect("a manifest")).expect("json");
        assert!(manifest["splits"].is_object(), "{}", folder.display());
    }
    assert!(chain.prefix.is_file());
}

fn holdout(chain: &Chain, namespace: &str, output: &Path) -> Command {
    let mut command = trainer();
    command
        .arg("holdout")
        .arg("--tree")
        .arg(tree())
        .arg("--base")
        .arg(&chain.merged)
        .arg("--ud-origin")
        .arg(&chain.reconciled)
        .arg("--technical-origin")
        .arg(&chain.technical)
        .arg("--prefix-inventory")
        .arg(&chain.prefix)
        .arg("--ledger")
        .arg(chain.folder.join("no-ledger"))
        .args(["--namespace", namespace])
        .arg("--source-root")
        .arg(&chain.holdout_sources)
        .arg("--output")
        .arg(output)
        .arg("--report")
        .arg(chain.folder.join(format!("{namespace}.report.json")));
    command
}

#[test]
fn a_holdout_is_made_from_each_kind_of_package_index() {
    let chain = &*CHAIN;
    let sid = chain.folder.join("sid");
    deb_source(&sid, "https://deb.debian.org/debian/dists/sid/main/Contents-amd64.gz", "https://deb.debian.org/debian/dists/sid/InRelease", "main/Contents-amd64.gz", "sid", false);
    done(&run(holdout(chain, "small-sid", &chain.folder.join("h-sid")).arg("--sid-directory").arg(&sid)));
    let arch = chain.folder.join("arch");
    arch_source(&arch);
    done(&run(holdout(chain, "small-arch", &chain.folder.join("h-arch")).arg("--arch-directory").arg(&arch).args(["--treebank", "UD_Russian-GSD", "--treebank", "UD_English-GUM"])));
    let alpine = chain.folder.join("alpine");
    alpine_source(&alpine);
    done(&run(holdout(chain, "small-alpine", &chain.folder.join("h-alpine")).arg("--alpine-directory").arg(&alpine).args(["--treebank", "none"])));
    let rpm = chain.folder.join("rpm");
    rpm_source(&rpm, None);
    done(&run(holdout(chain, "small-rpm", &chain.folder.join("h-rpm")).arg("--rpm-directory").arg(&rpm).args(["--treebank", "none"])));
    rpm_source(&rpm, Some("sha512"));
    done(&run(holdout(chain, "small-rpm-512", &chain.folder.join("h-rpm-512")).arg("--rpm-directory").arg(&rpm).args(["--treebank", "none"])));
    // With Tatoeba's sentences as well.
    let tatoeba = chain.folder.join("tatoeba");
    tatoeba_source(&tatoeba);
    done(&run(holdout(chain, "small-tatoeba", &chain.folder.join("h-tatoeba")).arg("--sid-directory").arg(&sid).args(["--treebank", "none", "--tatoeba-language", "rus", "--tatoeba-language", "eng"]).arg("--tatoeba-directory").arg(&tatoeba)));
    // No output: only the report.
    let mut preview = trainer();
    preview.arg("holdout").arg("--tree").arg(tree()).arg("--base").arg(&chain.merged).arg("--ud-origin").arg(&chain.reconciled).arg("--technical-origin").arg(&chain.technical).arg("--prefix-inventory").arg(&chain.prefix).arg("--ledger").arg(chain.folder.join("no-ledger")).args(["--namespace", "preview", "--treebank", "none"]).arg("--source-root").arg(&chain.holdout_sources).arg("--sid-directory").arg(&sid);
    done(&run(&mut preview));
}

fn fitting(chain: &Chain, namespace: &str, output: &Path, base: &Path, tatoeba: &Path) -> Command {
    let mut command = trainer();
    command
        .arg("fitting")
        .arg("--tree")
        .arg(tree())
        .arg("--base")
        .arg(base)
        .arg("--ud-origin")
        .arg(&chain.reconciled)
        .arg("--technical-origin")
        .arg(&chain.technical)
        .arg("--prefix-inventory")
        .arg(&chain.prefix)
        .arg("--ledger")
        .arg(chain.folder.join("no-ledger"))
        .args(["--namespace", namespace, "--tatoeba-language", "rus", "--tatoeba-language", "eng"])
        .arg("--tatoeba-directory")
        .arg(tatoeba)
        .arg("--output")
        .arg(output)
        .arg("--report")
        .arg(chain.folder.join(format!("{namespace}.report.json")));
    command
}

#[test]
fn a_fitting_extension_adds_to_the_splits_of_its_base() {
    let chain = &*CHAIN;
    let tatoeba = chain.folder.join("tatoeba-fit");
    tatoeba_source(&tatoeba);
    let output = chain.folder.join("fitting");
    // Over a corpus whose test names its words, as a holdout's does.
    let sid = chain.folder.join("sid-fit");
    deb_source(&sid, "https://deb.debian.org/debian/dists/sid/main/Contents-amd64.gz", "https://deb.debian.org/debian/dists/sid/InRelease", "main/Contents-amd64.gz", "fit", true);
    let over = chain.folder.join("h-fit");
    done(&run(holdout(chain, "small-over", &over).arg("--sid-directory").arg(&sid)));
    done(&run(&mut fitting(chain, "small-fitting", &output, &over, &tatoeba)));
    assert!(said(&run(&mut fitting(chain, "small-no-words", &chain.folder.join("fitting-2"), &chain.merged, &tatoeba))).contains("names no words"));
    let manifest: Value = serde_json::from_slice(&std::fs::read(output.join("manifest.json")).expect("a manifest")).expect("json");
    assert_eq!(manifest["kind"], json!("fitting-extension"));
}

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
        for (number, name) in commands(&repository.replace(['o', 'r', 'e', 'x', 't'], "a"), 20).iter().enumerate() {
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
    let prefix_corpus = folder.join("prefix-corpus");
    std::fs::create_dir_all(&prefix_corpus).expect("a folder");
    for split in ["train", "development", "calibration"] {
        std::fs::write(prefix_corpus.join(format!("{split}.jsonl.gz")), gz(format!("{}\n", json!({"text": format!("{split}слово"), "family": "семья"})).as_bytes())).expect("a split");
    }
    std::fs::write(prefix_corpus.join("corpus.json"), b"{}").expect("a receipt");
    done(&run(base("inventory").arg("--corpus").arg(&prefix_corpus).arg("--output").arg(&prefix)));
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

/// A file of a corpus changed with what it has to say about it, and put back.
fn edited(path: &Path, edit: &dyn Fn(&mut Value), then: &dyn Fn()) {
    let bytes = std::fs::read(path).expect("a file");
    let mut value: Value = serde_json::from_slice(&bytes).expect("json");
    edit(&mut value);
    std::fs::write(path, serde_json::to_vec(&value).expect("json")).expect("a file");
    then();
    std::fs::write(path, bytes).expect("a file");
}

fn refused(output: &Output, words: &str) {
    assert!(!output.status.success() && said(output).contains(words), "{words}: {}", said(output));
}

#[test]
fn a_receipt_that_does_not_hold_stops_a_holdout_before_anything_long() {
    let chain = &*CHAIN;
    let holdout_of = |flag: &str, directory: &Path, words: &str| {
        let answer = run(holdout(chain, "small-refused", &chain.folder.join("h-refused")).arg(flag).arg(directory).args(["--treebank", "none"]));
        refused(&answer, words);
    };
    // A Contents index.
    let sid = chain.folder.join("refused-sid");
    deb_source(&sid, "https://deb.debian.org/debian/dists/sid/main/Contents-amd64.gz", "https://deb.debian.org/debian/dists/sid/InRelease", "main/Contents-amd64.gz", "rs", true);
    let receipt = sid.join("source-receipt.json");
    let known = "does not identify verified TLS downloads of a known";
    let sums = "Contents index checksum mismatch";
    let edits: [(&dyn Fn(&mut Value), &str); 8] = [
        (&|value| value["contents"]["url"] = json!("https://example.org/"), known),
        (&|value| value["release"]["url"] = json!("https://example.org/"), known),
        (&|value| value["contents"]["status"] = json!(404), known),
        (&|value| value["release"]["status"] = json!(404), known),
        (&|value| value["tls_certificate_verification"] = json!(false), known),
        (&|value| value["contents_sha256"] = json!("0"), sums),
        (&|value| value["contents_bytes"] = json!(1), sums),
        (&|value| value["release_sha256"] = json!("0"), sums),
    ];
    for (edit, words) in edits {
        edited(&receipt, edit, &|| holdout_of("--sid-directory", &sid, words));
    }
    // A release file that names another checksum: the receipt still holds, and the section ends without it.
    let release = sid.join("InRelease");
    let text = std::fs::read_to_string(&release).expect("a file");
    let other = "Origin: test\nSHA256:\n other 1 main/Contents-amd64.gz\n\nAcquire-By-Hash: yes\n".to_owned();
    std::fs::write(&release, &other).expect("a file");
    edited(&receipt, &|value| value["release_sha256"] = json!(digest(other.as_bytes())), &|| holdout_of("--sid-directory", &sid, "absent from InRelease"));
    std::fs::write(&release, &text).expect("a file");
    // Who owns what, in the index itself.
    let contents = sid.join("Contents-amd64.gz");
    let packed = gz(b"usr/bin/badowner     Not/Valid\n");
    let original = std::fs::read(&contents).expect("a file");
    std::fs::write(&contents, &packed).expect("a file");
    let listed = format!("SHA256:\n {} {} main/Contents-amd64.gz\n", digest(&packed), packed.len());
    std::fs::write(&release, &listed).expect("a file");
    edited(&receipt, &|value| {
        value["contents_sha256"] = json!(digest(&packed));
        value["contents_bytes"] = json!(packed.len());
        value["release_sha256"] = json!(digest(listed.as_bytes()));
    }, &|| holdout_of("--sid-directory", &sid, "invalid package ownership"));
    std::fs::write(&contents, original).expect("a file");
    std::fs::write(&release, text).expect("a file");
    // A database of Arch.
    let arch = chain.folder.join("refused-arch");
    arch_source(&arch);
    let receipt = arch.join("source-receipt.json");
    let edits: [(&dyn Fn(&mut Value), &str); 7] = [
        (&|value| value["tls_certificate_verification"] = json!(false), "verified TLS Arch"),
        (&|value| value["mirror"] = json!("https://example.org/"), "verified TLS Arch"),
        (&|value| value["databases"]["core"]["download"]["status"] = json!(404), "files database download"),
        (&|value| value["databases"]["core"]["download"]["url"] = json!("x"), "files database download"),
        (&|value| value["databases"]["core"]["file"] = json!("x"), "files database download"),
        (&|value| value["databases"]["core"]["sha256"] = json!("0"), "checksum mismatch"),
        (&|value| value["databases"]["extra"]["bytes"] = json!(1), "checksum mismatch"),
    ];
    for (edit, words) in edits {
        edited(&receipt, edit, &|| holdout_of("--arch-directory", &arch, words));
    }
    // Packages that say nothing of themselves.
    let database = arch.join("core.files");
    let original = std::fs::read(&database).expect("a file");
    for (files, words) in [
        (vec![("pkg-1/files".to_owned(), b"usr/bin/thing\n".to_vec())], "without a description"),
        (vec![("pkg-1/desc".to_owned(), b"%OTHER%\nx\n".to_vec()), ("pkg-1/files".to_owned(), b"usr/bin/thing\n".to_vec())], "package descri"),
    ] {
        let packed = gz(&tar(&files));
        std::fs::write(&database, &packed).expect("a file");
        edited(&receipt, &|value| {
            value["databases"]["core"]["sha256"] = json!(digest(&packed));
            value["databases"]["core"]["bytes"] = json!(packed.len());
        }, &|| holdout_of("--arch-directory", &arch, words));
    }
    std::fs::write(&database, original).expect("a file");
    // An index of Alpine.
    let alpine = chain.folder.join("refused-alpine");
    alpine_source(&alpine);
    let receipt = alpine.join("source-receipt.json");
    let edits: [(&dyn Fn(&mut Value), &str); 6] = [
        (&|value| value["base_url"] = json!("https://example.org/"), "verified TLS Alpine"),
        (&|value| value["archive"]["status"] = json!(404), "verified TLS Alpine"),
        (&|value| value["tls_certificate_verification"] = json!(false), "verified TLS Alpine"),
        (&|value| value["archive"]["url"] = json!("x"), "verified TLS Alpine"),
        (&|value| value["archive"]["sha256"] = json!("0"), "index checksum mismatch"),
        (&|value| value["archive"]["bytes"] = json!(1), "index checksum mismatch"),
    ];
    for (edit, words) in edits {
        edited(&receipt, edit, &|| holdout_of("--alpine-directory", &alpine, words));
    }
    let archive = alpine.join("APKINDEX.tar.gz");
    let packed = gz(&tar(&[("OTHER".to_owned(), b"x".to_vec())]));
    std::fs::write(&archive, &packed).expect("a file");
    edited(&receipt, &|value| {
        value["archive"]["sha256"] = json!(digest(&packed));
        value["archive"]["bytes"] = json!(packed.len());
    }, &|| holdout_of("--alpine-directory", &alpine, "no APKINDEX member"));
    // An index of RPM packages.
    let rpm = chain.folder.join("refused-rpm");
    rpm_source(&rpm, Some("sha512"));
    let receipt = rpm.join("source-receipt.json");
    let edits: [(&dyn Fn(&mut Value), &str); 9] = [
        (&|value| value["base_url"] = json!("https://example.org/"), "verified TLS RPM"),
        (&|value| value["tls_certificate_verification"] = json!(false), "verified TLS RPM"),
        (&|value| value["repomd"]["status"] = json!(404), "verified TLS RPM"),
        (&|value| value["repomd_sha256"] = json!("0"), "RPM index checksum mismatch"),
        (&|value| value["primary"]["sha256"] = json!("0"), "RPM index checksum mismatch"),
        (&|value| value["primary"]["checksum_type"] = json!("md5"), "unknown checksum"),
        (&|value| value["primary"]["checksum"] = json!("0"), "does not match the checksum"),
        (&|value| value["primary"]["href"] = json!("x"), "not the one repomd.xml names"),
        (&|value| { value["primary"].as_object_mut().expect("a table").remove("checksum"); value["primary"]["checksum_type"] = json!("sha256"); }, "not the one repomd.xml names"),
    ];
    for (edit, words) in edits {
        edited(&receipt, edit, &|| holdout_of("--rpm-directory", &rpm, words));
    }
    // A receipt of Tatoeba.
    let tatoeba = chain.folder.join("refused-tatoeba");
    tatoeba_source(&tatoeba);
    let receipt = tatoeba.join("source-receipt.json");
    let with_tatoeba = |directory: &Path, words: &str, languages: &[&str]| {
        let mut command = holdout(chain, "small-refused", &chain.folder.join("h-refused"));
        command.arg("--sid-directory").arg(&sid).args(["--treebank", "none"]).arg("--tatoeba-directory").arg(directory);
        for language in languages {
            command.args(["--tatoeba-language", language]);
        }
        refused(&run(&mut command), words);
    };
    let identifies = "does not identify verified TLS Tatoeba";
    let mismatch = "receipt mismatch";
    let edits: [(&dyn Fn(&mut Value), &str); 9] = [
        (&|value| value["licence"] = json!("All rights reserved"), identifies),
        (&|value| value["files"] = json!(1), identifies),
        (&|value| value["tls_certificate_verification"] = json!(false), identifies),
        (&|value| value["base_url"] = json!("x"), identifies),
        (&|value| value["files"]["rus_sentences.tsv.bz2"]["status"] = json!(404), mismatch),
        (&|value| value["files"]["rus_sentences.tsv.bz2"]["language"] = json!("eng"), mismatch),
        (&|value| value["files"]["rus_sentences.tsv.bz2"]["url"] = json!("x"), mismatch),
        (&|value| value["files"]["rus_sentences.tsv.bz2"]["sha256"] = json!("0"), mismatch),
        (&|value| value["files"]["rus_sentences.tsv.bz2"]["bytes"] = json!(1), mismatch),
    ];
    for (edit, words) in edits {
        edited(&receipt, edit, &|| with_tatoeba(&tatoeba, words, &["rus"]));
    }
    with_tatoeba(&tatoeba, "at least one --tatoeba-language", &[]);
    refused(&run(holdout(chain, "small-refused", &chain.folder.join("h-refused")).arg("--sid-directory").arg(&sid).args(["--treebank", "none", "--tatoeba-language", "rus"])), "needs --tatoeba-directory");
    // Exports that are not what they say.
    for (fixture, words) in [("tatoeba-bad", "must be id, language, text"), ("tatoeba-mislabelled", "holds a eng sentence")] {
        let bad = chain.folder.join(fixture);
        std::fs::create_dir_all(&bad).expect("a folder");
        let bytes = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(fixture).join("rus_sentences.tsv.bz2")).expect("a sample");
        std::fs::write(bad.join("rus_sentences.tsv.bz2"), &bytes).expect("a file");
        let base = "https://downloads.tatoeba.org/exports/per_language/";
        std::fs::write(bad.join("source-receipt.json"), serde_json::to_vec(&json!({"licence": "CC BY", "tls_certificate_verification": true, "base_url": base, "files": {"rus_sentences.tsv.bz2": {"status": 200, "language": "rus", "url": format!("{base}rus/rus_sentences.tsv.bz2"), "sha256": digest(&bytes), "bytes": bytes.len()}}})).expect("json")).expect("a receipt");
        with_tatoeba(&bad, words, &["rus"]);
    }
    // The treebanks a holdout names.
    refused(&run(holdout(chain, "small-refused", &chain.folder.join("h-refused")).arg("--sid-directory").arg(&sid).args(["--treebank", "UD_Nothing"])), "unpinned treebank");
    let answer = run(holdout(chain, "small-refused", &chain.folder.join("h-refused")).args(["--treebank", "none"]));
    refused(&answer, "exactly one technical source");
    let two = run(holdout(chain, "small-refused", &chain.folder.join("h-refused")).arg("--sid-directory").arg(&sid).arg("--arch-directory").arg(&arch));
    refused(&two, "exactly one technical source");
}

#[test]
fn a_pinned_treebank_is_exactly_what_was_pinned() {
    let chain = &*CHAIN;
    let sources = chain.folder.join("refused-sources");
    let (name, commit) = PINS[0];
    pinned_treebank(&sources, name, commit, &RUSSIAN, 9000, 2);
    let sid = chain.folder.join("treebank-sid");
    deb_source(&sid, "https://deb.debian.org/debian/dists/sid/main/Contents-amd64.gz", "https://deb.debian.org/debian/dists/sid/InRelease", "main/Contents-amd64.gz", "ts", true);
    let check = |words: &str| {
        let mut command = trainer();
        command.arg("holdout").arg("--tree").arg(tree()).arg("--base").arg(&chain.merged).arg("--ud-origin").arg(&chain.reconciled).arg("--technical-origin").arg(&chain.technical).arg("--prefix-inventory").arg(&chain.prefix).arg("--ledger").arg(chain.folder.join("no-ledger")).args(["--namespace", "n", "--treebank", name]).arg("--source-root").arg(&sources).arg("--sid-directory").arg(&sid);
        refused(&run(&mut command), words);
    };
    let pin = sources.join(name).join("pin.json");
    let edits: [(&dyn Fn(&mut Value), &str); 6] = [
        (&|value| value["commit"] = json!("0"), "pinned commit differs"),
        (&|value| value["repository"] = json!("x"), "pinned commit differs"),
        (&|value| value["files"] = json!([]), "invalid pinned holdout inventory"),
        (&|value| value["files"][0]["path"] = json!("thing.exe"), "invalid pinned holdout filename"),
        (&|value| value["files"][0]["sha256"] = json!("0"), "pinned holdout checksum differs"),
        (&|value| value["files"][0]["sha"] = json!("0"), "disagrees with pinned Git tree blob"),
    ];
    for (edit, words) in edits {
        edited(&pin, edit, &|| check(words));
    }
    // A list of a note and a text is fine; a treebank that breaks its format is not.
    let file = sources.join(name).join(format!("{}-ud-test.conllu", name.to_lowercase()));
    let original = std::fs::read(&file).expect("a file");
    let broken = |content: &str, words: &str| {
        std::fs::write(&file, content).expect("a file");
        let bytes = content.as_bytes();
        edited(&pin, &|value| {
            value["files"][0]["sha256"] = json!(digest(bytes));
            value["files"][0]["sha"] = json!(blob_sha(bytes));
        }, &|| check(words));
    };
    broken("1\tслово\n", "must have ten columns");
    broken("2-1\tслово\t_\t_\t_\t_\t_\t_\t_\t_\n", "invalid or overlapping");
    broken("1-2\tслово\t_\t_\t_\t_\t_\t_\t_\t_\n1\tсло\t_\t_\t_\t_\t_\t_\t_\t_\n", "invalid or overlapping");
    broken("1-2\tслово\t_\t_\t_\t_\t_\t_\t_\t_\n1\tсло\t_\t_\t_\t_\t_\t_\t_\t_\n2\tво\t_\t_\t_\t_\t_\t_\t_\t_\n1-2\tснова\t_\t_\t_\t_\t_\t_\t_\t_\n", "invalid or overlapping");
    broken("# sent_id = a\n1\tслово\t_\t_\t_\t_\t_\t_\t_\t_\n\n# sent_id = a\n1\tслово\t_\t_\t_\t_\t_\t_\t_\t_\n", "duplicate holdout sentence identity");
    std::fs::write(&file, original).expect("a file");
}

#[test]
fn what_stands_in_the_way_of_a_new_corpus_says_so() {
    let chain = &*CHAIN;
    let sid = chain.folder.join("way-sid");
    deb_source(&sid, "https://deb.debian.org/debian/dists/sid/main/Contents-amd64.gz", "https://deb.debian.org/debian/dists/sid/InRelease", "main/Contents-amd64.gz", "wy", true);
    let plain = |output: &Path| {
        let mut command = holdout(chain, "small-way", output);
        command.arg("--sid-directory").arg(&sid).args(["--treebank", "none"]);
        run(&mut command)
    };
    // Into the base, over itself, over what is there.
    refused(&plain(&chain.merged), "refusing to overwrite");
    refused(&plain(&chain.merged.join("inside")), "must not be inside the base");
    let there = chain.folder.join("there");
    std::fs::create_dir_all(&there).expect("a folder");
    refused(&plain(&there), "refusing to overwrite");
    // A base that no longer holds what its manifest says.
    let copy = chain.folder.join("changed-base");
    std::fs::create_dir_all(&copy).expect("a folder");
    for entry in std::fs::read_dir(&chain.merged).expect("the corpus").flatten() {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), copy.join(entry.file_name())).expect("a copy");
        }
    }
    let with_base = |output: &Path, words: &str| {
        let mut command = trainer();
        command.arg("holdout").arg("--tree").arg(tree()).arg("--base").arg(&copy).arg("--ud-origin").arg(&chain.reconciled).arg("--technical-origin").arg(&chain.technical).arg("--prefix-inventory").arg(&chain.prefix).arg("--ledger").arg(chain.folder.join("no-ledger")).args(["--namespace", "n", "--treebank", "none"]).arg("--source-root").arg(&chain.holdout_sources).arg("--sid-directory").arg(&sid).arg("--output").arg(output);
        refused(&run(&mut command), words);
    };
    let manifest = copy.join("manifest.json");
    edited(&manifest, &|value| value["test_membership_sha256"] = json!("0"), &|| with_base(&chain.folder.join("o1"), "differs from its manifest"));
    edited(&manifest, &|value| value["splits"]["train"]["sha256"] = json!("0"), &|| with_base(&chain.folder.join("o2"), "base split checksum mismatch"));
    edited(&manifest, &|value| value["splits"]["quarantine"]["sha256"] = json!("0"), &|| with_base(&chain.folder.join("o3"), "base quarantine checksum mismatch"));
    edited(&manifest, &|value| value["splits"]["quarantine"]["rows"] = json!(1_000_000), &|| with_base(&chain.folder.join("o4"), "row count mismatch"));
    // An exposure file that is not there, or not of its form.
    let mut extra = holdout(chain, "small-way", &chain.folder.join("o5"));
    extra.arg("--sid-directory").arg(&sid).args(["--treebank", "none", "--extra-exposure", "/nonexistent/exposure.json"]);
    refused(&run(&mut extra), "exposure input is absent");
    let exposure = chain.folder.join("exposure.tsv.gz");
    std::fs::write(&exposure, gz(b"only\ttwo\n")).expect("a file");
    let mut extra = holdout(chain, "small-way", &chain.folder.join("o6"));
    extra.arg("--sid-directory").arg(&sid).args(["--treebank", "none", "--extra-exposure"]).arg(&exposure);
    refused(&run(&mut extra), "invalid prior context TSV");
}

/// A test read before, as a ledger holds it.
fn ledger_of(folder: &Path, membership: &Value) -> PathBuf {
    std::fs::create_dir_all(folder).expect("a folder");
    std::fs::write(folder.join("read.access.json"), serde_json::to_vec(&json!({"test_membership": membership})).expect("json")).expect("a record");
    std::fs::write(folder.join("other.access.json"), b"{}").expect("a record");
    std::fs::write(folder.join("notes.txt"), b"").expect("a file");
    folder.to_path_buf()
}

#[test]
fn a_test_read_before_takes_its_rows_out_of_a_new_one() {
    let chain = &*CHAIN;
    let sid = chain.folder.join("ledger-sid");
    deb_source(&sid, "https://deb.debian.org/debian/dists/sid/main/Contents-amd64.gz", "https://deb.debian.org/debian/dists/sid/InRelease", "main/Contents-amd64.gz", "lg", true);
    let tatoeba = chain.folder.join("ledger-tatoeba");
    tatoeba_source(&tatoeba);
    let first = chain.folder.join("ledger-first");
    done(&run(holdout(chain, "small-ledger", &first).arg("--sid-directory").arg(&sid).args(["--treebank", "UD_Russian-GSD"])));
    let membership: Value = serde_json::from_slice(&std::fs::read(first.join("test-membership.json")).expect("a membership")).expect("json");
    let whole = ledger_of(&chain.folder.join("ledger-whole"), &membership);
    let run_with = |ledger: &Path, name: &str| {
        let mut command = trainer();
        command.arg("holdout").arg("--tree").arg(tree()).arg("--base").arg(&chain.merged).arg("--ud-origin").arg(&chain.reconciled).arg("--technical-origin").arg(&chain.technical).arg("--prefix-inventory").arg(&chain.prefix).arg("--ledger").arg(ledger).args(["--namespace", "small-ledger", "--treebank", "UD_Russian-GSD"]).arg("--source-root").arg(&chain.holdout_sources).arg("--sid-directory").arg(&sid).arg("--output").arg(chain.folder.join(name));
        done(&run(&mut command))
    };
    run_with(&whole, "ledger-second");
    // Only the rows, or only the documents: the families were not known then.
    let rows = ledger_of(&chain.folder.join("ledger-rows"), &json!({"row_ids_sha256": membership["row_ids_sha256"]}));
    run_with(&rows, "ledger-third");
    let documents = ledger_of(&chain.folder.join("ledger-documents"), &json!({"document_ids_sha256": membership["document_ids_sha256"]}));
    run_with(&documents, "ledger-fourth");
    // The fitting extension over the same.
    let over = first;
    let fit = |ledger: &Path, name: &str| {
        let mut command = fitting(chain, name, &chain.folder.join(name), &over, &tatoeba);
        command.arg("--ledger").arg(ledger);
        command
    };
    // The option above is read first: a ledger of its own goes with its own call.
    let mut own = trainer();
    own.arg("fitting").arg("--tree").arg(tree()).arg("--base").arg(&over).arg("--ud-origin").arg(&chain.reconciled).arg("--technical-origin").arg(&chain.technical).arg("--prefix-inventory").arg(&chain.prefix).arg("--ledger").arg(&whole).args(["--namespace", "fit-ledger", "--tatoeba-language", "rus", "--tatoeba-language", "eng"]).arg("--tatoeba-directory").arg(&tatoeba).arg("--output").arg(chain.folder.join("fit-ledger"));
    done(&run(&mut own));
    let _ = fit;
    // A fitting extension needs somewhere to write, and something to read.
    let mut none = trainer();
    none.arg("fitting").arg("--tree").arg(tree()).arg("--base").arg(&over).arg("--ud-origin").arg(&chain.reconciled).arg("--technical-origin").arg(&chain.technical).arg("--prefix-inventory").arg(&chain.prefix).args(["--namespace", "n"]);
    refused(&run(&mut none), "needs an output");
    let mut no_exports = trainer();
    no_exports.arg("fitting").arg("--tree").arg(tree()).arg("--base").arg(&over).arg("--ud-origin").arg(&chain.reconciled).arg("--technical-origin").arg(&chain.technical).arg("--prefix-inventory").arg(&chain.prefix).args(["--namespace", "n"]).arg("--output").arg(chain.folder.join("fit-none"));
    refused(&run(&mut no_exports), "needs the Tatoeba exports");
}

fn copy_of(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("a folder");
    for entry in std::fs::read_dir(from).expect("a corpus").flatten() {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), to.join(entry.file_name())).expect("a copy");
        }
    }
}

#[test]
fn a_merge_checks_both_corpora_to_the_byte() {
    let chain = &*CHAIN;
    let (base_copy, technical_copy) = (chain.folder.join("merge-base"), chain.folder.join("merge-technical"));
    copy_of(&chain.reconciled, &base_copy);
    copy_of(&chain.technical, &technical_copy);
    let output = chain.folder.join("merge-output");
    let merge = |words: &str| refused(&run(base("merge").arg("--base").arg(&base_copy).arg("--technical").arg(&technical_copy).arg("--output").arg(&output)), words);
    let existing = chain.folder.join("merge-there");
    std::fs::create_dir_all(&existing).expect("a folder");
    refused(&run(base("merge").arg("--base").arg(&base_copy).arg("--technical").arg(&technical_copy).arg("--output").arg(&existing)), "refusing to overwrite combined");
    let manifest = base_copy.join("manifest.json");
    let edits: [(&dyn Fn(&mut Value), &str); 15] = [
        (&|value| value["namespace"] = json!(""), "invalid corpus manifest identity"),
        (&|value| value["schema_version"] = json!(2), "invalid corpus manifest identity"),
        (&|value| value["label"] = json!("other"), "invalid corpus manifest identity"),
        (&|value| value["splits"]["train"]["path"] = json!("x"), "unexpected source split filename"),
        (&|value| value["splits"]["train"]["sha256"] = json!("0"), "compressed checksum mismatch"),
        (&|value| value["splits"]["train"]["content_sha256"] = json!("0"), "raw content checksum mismatch"),
        (&|value| value["splits"]["train"]["rows"] = json!(1), "streamed row count mismatch"),
        (&|value| value["generator_sha256"] = json!("0"), "origin generator checksum mismatch"),
        (&|value| value["reconciliation"]["source_sha256"] = json!("0"), "reconciliation generator checksum mismatch"),
        (&|value| value["test_membership_sha256"] = json!("0"), "test membership checksum mismatch"),
        (&|value| value["reconciliation"]["closure_sha256"] = json!("0"), "base physical sidecar checksum mismatch"),
        (&|value| value["namespace"] = json!("other"), "namespace mismatch"),
        (&|value| value["generator_source"] = json!("gone.rs"), ""),
        (&|value| value["splits"]["train"] = json!({}), "unexpected source split filename"),
        (&|value| value["splits"] = json!({}), "unexpected source split filename"),
    ];
    for (edit, words) in edits {
        edited(&manifest, edit, &|| merge(words));
    }
    edited(&technical_copy.join("manifest.json"), &|value| value["family_aliases_sha256"] = json!("0"), &|| merge("technical family sidecar checksum mismatch"));
    // The membership of a test, with the manifest following it.
    let pinned = |folder: &Path, edit: &dyn Fn(&mut Value), words: &str| {
        let path = folder.join("test-membership.json");
        let bytes = std::fs::read(&path).expect("a file");
        let mut value: Value = serde_json::from_slice(&bytes).expect("json");
        edit(&mut value);
        let changed = serde_json::to_vec(&value).expect("json");
        std::fs::write(&path, &changed).expect("a file");
        edited(&folder.join("manifest.json"), &|manifest| manifest["test_membership_sha256"] = json!(digest(&changed)), &|| merge(words));
        std::fs::write(&path, bytes).expect("a file");
    };
    pinned(&base_copy, &|value| value["row_ids_sha256"] = json!([]), "row count mismatch");
    pinned(&base_copy, &|value| value["family_ids_sha256"] = json!([]), "requires family and document membership");
    pinned(&base_copy, &|value| { let list = value["row_ids_sha256"].as_array_mut().expect("a list"); let first = list[0].clone(); list.push(first); }, "duplicate hashes");
    pinned(&technical_copy, &|value| value["row_ids_sha256"] = json!(["0".repeat(64)]), "row count mismatch");
    let membership: Value = serde_json::from_slice(&std::fs::read(chain.reconciled.join("test-membership.json")).expect("a file")).expect("json");
    pinned(&technical_copy, &|value| value["family_ids_sha256"][0] = membership["family_ids_sha256"][0].clone(), "");
    // The sidecars, with the manifests following them.
    let sidecar = technical_copy.join("family-aliases.json");
    let original = std::fs::read(&sidecar).expect("a file");
    let good: Value = serde_json::from_slice(&original).expect("json");
    let family = good["families"].as_object().expect("families").keys().next().expect("a family").clone();
    let closure: Value = serde_json::from_slice(&std::fs::read(base_copy.join("physical-family-closure.json")).expect("a file")).expect("json");
    let known_alias = closure["alias_sha256_to_closure_sha256"].as_object().expect("aliases").keys().next().expect("an alias").clone();
    let with_sidecar = |edit: &dyn Fn(&mut Value), words: &str| {
        let mut changed = good.clone();
        edit(&mut changed);
        let bytes = serde_json::to_vec(&changed).expect("json");
        std::fs::write(&sidecar, &bytes).expect("a file");
        edited(&technical_copy.join("manifest.json"), &|manifest| manifest["family_aliases_sha256"] = json!(digest(&bytes)), &|| merge(words));
    };
    with_sidecar(&|value| value["schema_version"] = json!(2), "unsupported technical family sidecar");
    with_sidecar(&|value| value["families"] = json!(1), "expected corpus metadata object");
    with_sidecar(&|value| value["families"][&family]["split"] = json!("nowhere"), "invalid retained split");
    with_sidecar(&|value| value["families"][&family]["aliases_sha256"] = json!([]), "requires aliases and documents");
    with_sidecar(&|value| value["families"][&family]["document_ids_sha256"] = json!([]), "requires aliases and documents");
    with_sidecar(&|value| value["families"][&family]["aliases_sha256"] = json!([known_alias]), "overlaps base physical alias");
    with_sidecar(&|value| value["families"]["another"] = value["families"][&family].clone(), "multiple families or splits");
    with_sidecar(
        &|value| {
            let mut other = value["families"][&family].clone();
            other["aliases_sha256"] = json!(["a".repeat(64)]);
            other["split"] = json!(if other["split"] == json!("train") { "development" } else { "train" });
            value["families"]["another"] = other;
        },
        "document spans multiple splits",
    );
    with_sidecar(
        &|value| {
            let test = value["families"].as_object_mut().expect("families").iter_mut().find(|(_, record)| record["split"] == json!("test"));
            if let Some((_, record)) = test {
                record["split"] = json!("train");
            }
        },
        "",
    );
    std::fs::write(&sidecar, original).expect("a file");
    // Files that are not whole lines, or no gzip at all.
    let split = base_copy.join("train.jsonl.gz");
    let bytes = std::fs::read(&split).expect("a file");
    for content in [gz(b"no newline at the end"), gz(b"\n"), b"not gzip".to_vec()] {
        std::fs::write(&split, content).expect("a file");
        merge("");
    }
    std::fs::write(&split, bytes).expect("a file");
    refused(&run(base("merge").arg("--base").arg("/nonexistent").arg("--technical").arg(&technical_copy).arg("--output").arg(&output)), "");
    assert!(!output.exists());
}

#[test]
fn what_changed_since_a_base_was_frozen_stops_its_reconciliation() {
    let chain = &*CHAIN;
    let sources = chain.folder.join("sources");
    let copy = chain.folder.join("reconcile-copy");
    copy_of(&chain.base, &copy);
    let report = chain.folder.join("deep/er/reconcile.json");
    let reconcile = |words: &str| refused(&run(base("reconcile").arg("--corpus").arg(&copy).arg("--source-root").arg(&sources).arg("--report").arg(&report)), words);
    let manifest = copy.join("manifest.json");
    let edits: [(&dyn Fn(&mut Value), &str); 6] = [
        (&|value| value["namespace"] = json!(1), "a namespace"),
        (&|value| value["sampling"]["max_documents_per_source"] = json!("many"), "sampling.max_documents_per_source"),
        (&|value| value["sources"] = json!([]), "pinned source inventory differs"),
        (&|value| value["sampling"]["sources"] = json!({}), "source sampling changed"),
        (&|value| value["sampling"]["max_sentences_per_document"] = json!(1), "source sampling changed"),
        (&|value| value["exposure_inputs"] = json!(1), "exposure inputs"),
    ];
    for (edit, words) in edits {
        edited(&manifest, edit, &|| reconcile(words));
    }
    let edits: [(&dyn Fn(&mut Value), &str); 9] = [
        (&|value| value["exposed_membership_sha256"] = json!("0"), "historical exposure reconstruction differs"),
        (&|value| value["partitioning"] = json!({}), "original partitioning changed"),
        (&|value| value["splits"]["train"]["path"] = json!("x"), "original compressed split changed"),
        (&|value| value["splits"]["train"]["sha256"] = json!("0"), "original compressed split changed"),
        (&|value| value["splits"]["train"]["rows"] = json!(1), "reconstructed original split differs"),
        (&|value| value["splits"]["train"]["content_sha256"] = json!("0"), "reconstructed original split differs"),
        (&|value| value["test_membership_sha256"] = json!("0"), "original test membership changed"),
        (&|value| value["splits"]["development"]["content_sha256"] = json!("0"), "reconstructed original split differs"),
        (&|value| value["exposure_inputs"] = json!({}), "historical exposure input changed"),
    ];
    for (edit, words) in edits {
        edited(&manifest, edit, &|| reconcile(words));
    }
    // Over a folder that is there.
    let there = chain.folder.join("reconcile-there");
    std::fs::create_dir_all(&there).expect("a folder");
    refused(&run(base("reconcile").arg("--corpus").arg(&copy).arg("--source-root").arg(&sources).arg("--report").arg(&report).arg("--output").arg(&there)), "refusing to overwrite");
    assert!(report.is_file());
}

#[test]
fn a_new_base_and_its_technical_corpus_have_their_own_refusals() {
    let chain = &*CHAIN;
    let sources = chain.folder.join("sources");
    let freeze = |output: &Path, extra: &[&str]| run(base("freeze").arg("--source-root").arg(&sources).arg("--output").arg(output).args(["--namespace", "small:base"]).args(extra));
    refused(&freeze(&chain.base, &[]), "refusing to overwrite a frozen corpus");
    refused(&freeze(&chain.folder.join("b1"), &["--max-sentences-per-document", "0"]), "must be positive");
    // The technical corpus, without writing anything, and its checks.
    let trixie = chain.folder.join("refused-trixie");
    deb_source(&trixie, "https://deb.debian.org/debian/dists/trixie/main/Contents-amd64.gz", "https://deb.debian.org/debian/dists/trixie/InRelease", "main/Contents-amd64.gz", "rt", true);
    let preview = chain.folder.join("preview.json");
    let technical = |corpus: &Path, words: &str| refused(&run(base("technical").arg("--source-directory").arg(&trixie).arg("--ud-corpus").arg(corpus).arg("--preview-report").arg(&preview)), words);
    let receipt = trixie.join("source-receipt.json");
    let identifies = "does not identify verified TLS Debian";
    let sums = "Debian source checksum mismatch";
    let edits: [(&dyn Fn(&mut Value), &str); 8] = [
        (&|value| value["contents"]["status"] = json!(404), identifies),
        (&|value| value["release"]["status"] = json!(404), identifies),
        (&|value| value["tls_certificate_verification"] = json!(false), identifies),
        (&|value| value["contents"]["url"] = json!("x"), identifies),
        (&|value| value["release"]["url"] = json!("x"), identifies),
        (&|value| value["contents_sha256"] = json!("0"), sums),
        (&|value| value["contents_bytes"] = json!(1), sums),
        (&|value| value["release_sha256"] = json!("0"), sums),
    ];
    for (edit, words) in edits {
        edited(&receipt, edit, &|| technical(&chain.reconciled, words));
    }
    let release = trixie.join("InRelease");
    let text = std::fs::read_to_string(&release).expect("a file");
    let other = "SHA256:\n other 1 main/Contents-amd64.gz\n\nAcquire-By-Hash: yes\n".to_owned();
    std::fs::write(&release, &other).expect("a file");
    edited(&receipt, &|value| value["release_sha256"] = json!(digest(other.as_bytes())), &|| technical(&chain.reconciled, "absent from InRelease"));
    std::fs::write(&release, text).expect("a file");
    let copy = chain.folder.join("technical-copy");
    copy_of(&chain.reconciled, &copy);
    edited(&copy.join("manifest.json"), &|value| value["reconciliation"]["closure_sha256"] = json!("0"), &|| technical(&copy, "UD physical sidecar checksum mismatch"));
    // The preview alone.
    let done_preview = run(base("technical").arg("--source-directory").arg(&trixie).arg("--ud-corpus").arg(&chain.reconciled).arg("--preview-report").arg(&preview));
    assert!(done_preview.status.success(), "{}", said(&done_preview));
    refused(&run(base("technical").arg("--source-directory").arg(&trixie).arg("--ud-corpus").arg(&chain.reconciled).arg("--preview-report").arg(&preview).arg("--output").arg(&chain.technical)), "");
    // The inventory, over what is there.
    refused(&run(base("inventory").arg("--output").arg(&chain.prefix)), "refusing to overwrite");
    let empty = chain.folder.join("inventory-gone");
    refused(&run(base("inventory").arg("--corpus").arg(&empty).arg("--output").arg(chain.folder.join("i1.json"))), "");
    std::fs::create_dir_all(&empty).expect("a folder");
    for split in ["train", "development", "calibration"] {
        std::fs::write(empty.join(format!("{split}.jsonl.gz")), gz(b"{\"text\": \"\"}\n")).expect("a split");
    }
    refused(&run(base("inventory").arg("--corpus").arg(&empty).arg("--output").arg(chain.folder.join("i2.json"))), "prefix row without a text");
}

#[test]
fn a_corpus_without_rows_or_aliases_is_no_origin_of_a_merge() {
    let chain = &*CHAIN;
    // A base needs documents to sample.
    let sources = chain.folder.join("sources");
    refused(&run(base("freeze").arg("--source-root").arg(&sources).arg("--output").arg(chain.folder.join("b0")).args(["--namespace", "small:base", "--max-documents", "0"])), "must be positive");
    let merge = |base_copy: &Path, words: &str| refused(&run(base("merge").arg("--base").arg(base_copy).arg("--technical").arg(&chain.technical).arg("--output").arg(chain.folder.join("merge-refused"))), words);
    // Every split empty.
    let empty = chain.folder.join("merge-empty");
    copy_of(&chain.reconciled, &empty);
    let manifest_path = empty.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(&manifest_path).expect("a manifest")).expect("json");
    for split in ["train", "development", "calibration", "test", "quarantine"] {
        let packed = gz(b"");
        std::fs::write(empty.join(format!("{split}.jsonl.gz")), &packed).expect("a split");
        manifest["splits"][split] = json!({"path": format!("{split}.jsonl.gz"), "sha256": digest(&packed), "content_sha256": digest(b""), "rows": 0});
    }
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).expect("json")).expect("a manifest");
    merge(&empty, "empty source corpus");
    // A base whose closure names no alias.
    let bare = chain.folder.join("merge-bare");
    copy_of(&chain.reconciled, &bare);
    let closure = bare.join("physical-family-closure.json");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&closure).expect("a sidecar")).expect("json");
    value["alias_sha256_to_closure_sha256"] = json!({});
    let bytes = serde_json::to_vec(&value).expect("json");
    std::fs::write(&closure, &bytes).expect("a sidecar");
    edited(&bare.join("manifest.json"), &|manifest| manifest["reconciliation"]["closure_sha256"] = json!(digest(&bytes)), &|| merge(&bare, "empty base physical alias sidecar"));
}

/// The rows of a split file.
fn rows_of(path: &Path) -> Vec<Value> {
    let mut text = String::new();
    std::io::Read::read_to_string(&mut flate2::read::MultiGzDecoder::new(std::fs::File::open(path).expect("a split")), &mut text).expect("text");
    text.lines().map(|line| serde_json::from_str(line).expect("a row")).collect()
}

/// A row more in a split of a corpus, its manifest following.
fn append_row(folder: &Path, split: &str, row: &Value) {
    let path = folder.join(format!("{split}.jsonl.gz"));
    let mut raw = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::MultiGzDecoder::new(std::fs::File::open(&path).expect("a split")), &mut raw).expect("bytes");
    raw.extend_from_slice(serde_json::to_string(row).expect("json").as_bytes());
    raw.push(b'\n');
    let packed = gz(&raw);
    std::fs::write(&path, &packed).expect("a split");
    let manifest_path = folder.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(&manifest_path).expect("a manifest")).expect("json");
    let rows = manifest["splits"][split]["rows"].as_u64().expect("a count") + 1;
    manifest["splits"][split] = json!({"path": format!("{split}.jsonl.gz"), "sha256": digest(&packed), "content_sha256": digest(&raw), "rows": rows});
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).expect("json")).expect("a manifest");
}

#[test]
fn a_fitting_extension_takes_out_what_a_test_saw_and_what_its_base_splits_two_ways() {
    let chain = &*CHAIN;
    let sid = chain.folder.join("contest-sid");
    deb_source(&sid, "https://deb.debian.org/debian/dists/sid/main/Contents-amd64.gz", "https://deb.debian.org/debian/dists/sid/InRelease", "main/Contents-amd64.gz", "ct", true);
    let over = chain.folder.join("contest-over");
    done(&run(holdout(chain, "small-contest", &over).arg("--sid-directory").arg(&sid).args(["--treebank", "UD_Russian-GSD"])));
    let tatoeba = chain.folder.join("contest-tatoeba");
    tatoeba_source(&tatoeba);
    let first = chain.folder.join("contest-first");
    done(&run(&mut fitting(chain, "small-contest-fit", &first, &over, &tatoeba)));
    let added: Vec<Value> = rows_of(&first.join("train.jsonl.gz")).into_iter().filter(|row| row["source"].as_str().is_some_and(|source| source.starts_with("Tatoeba"))).collect();
    assert!(added.len() > 2, "{}", added.len());
    // A base whose train and development both hold a word of the first extension.
    let contested = chain.folder.join("contest-base");
    copy_of(&over, &contested);
    let word = added[0]["original"].as_str().expect("a word").to_owned();
    for split in ["train", "development"] {
        let mut template = rows_of(&contested.join(format!("{split}.jsonl.gz")))[0].clone();
        template["original"] = json!(word);
        template["lemma"] = json!(word);
        template["identifier"] = json!(format!("contest:{split}"));
        append_row(&contested, split, &template);
    }
    // A test read before, of a row, a document and a family the first extension added.
    let text = |row: &Value, field: &str| row[field].as_str().expect("a field").to_owned();
    let membership = json!({"row_ids_sha256": [digest(text(&added[1], "identifier").as_bytes())], "document_ids_sha256": [digest(text(&added[2], "document").as_bytes())], "family_ids_sha256": [text(&added[added.len() - 1], "family")]});
    let ledger = ledger_of(&chain.folder.join("contest-ledger"), &membership);
    let report = chain.folder.join("contest-second.report.json");
    let mut second = trainer();
    second.arg("fitting").arg("--tree").arg(tree()).arg("--base").arg(&contested).arg("--ud-origin").arg(&chain.reconciled).arg("--technical-origin").arg(&chain.technical).arg("--prefix-inventory").arg(&chain.prefix).arg("--ledger").arg(&ledger).args(["--namespace", "small-contest-fit", "--tatoeba-language", "rus", "--tatoeba-language", "eng"]).arg("--tatoeba-directory").arg(&tatoeba).arg("--output").arg(chain.folder.join("contest-second")).arg("--report").arg(&report);
    done(&run(&mut second));
    let reasons: Value = serde_json::from_slice(&std::fs::read(&report).expect("a report")).expect("json");
    for reason in ["base-split-conflict", "prior-accessed-test-row", "prior-accessed-test-document", "prior-accessed-test-family"] {
        assert!(reasons["fitting"]["quarantine_reasons"].get(reason).is_some(), "{reason}: {}", reasons["fitting"]["quarantine_reasons"]);
    }
    // A base whose test or quarantine is not the file its manifest names.
    for split in ["test", "quarantine"] {
        let changed = chain.folder.join(format!("contest-{split}"));
        copy_of(&over, &changed);
        edited(&changed.join("manifest.json"), &|manifest| manifest["splits"][split]["sha256"] = json!("0"), &|| {
            refused(&run(&mut fitting(chain, "small-contest-fit", &chain.folder.join(format!("contest-{split}-out")), &changed, &tatoeba)), &format!("base split checksum mismatch: {split}"));
        });
    }
}

#[test]
fn a_reconciliation_writes_its_report_alone_and_refuses_a_membership_rebuilt_otherwise() {
    let chain = &*CHAIN;
    let sources = chain.folder.join("sources");
    let reconcile = |corpus: &Path, report: &Path| run(base("reconcile").arg("--corpus").arg(corpus).arg("--source-root").arg(&sources).arg("--report").arg(report));
    let alone = chain.folder.join("reconcile-alone.json");
    done(&reconcile(&chain.base, &alone));
    assert!(alone.is_file());
    refused(&reconcile(&chain.base, Path::new("/")), "");
    let copy = chain.folder.join("reconcile-membership");
    copy_of(&chain.base, &copy);
    let path = copy.join("test-membership.json");
    let mut membership: Value = serde_json::from_slice(&std::fs::read(&path).expect("a membership")).expect("json");
    membership["scope"] = json!("another scope");
    let bytes = serde_json::to_vec(&membership).expect("json");
    std::fs::write(&path, &bytes).expect("a membership");
    edited(&copy.join("manifest.json"), &|manifest| manifest["test_membership_sha256"] = json!(digest(&bytes)), &|| refused(&reconcile(&copy, &chain.folder.join("reconcile-membership.json")), "reconstructed test membership differs"));
}

#[test]
fn a_merge_refuses_a_technical_test_whose_documents_or_rows_are_not_its_own() {
    let chain = &*CHAIN;
    let technical = chain.folder.join("merge-membership");
    copy_of(&chain.technical, &technical);
    let merge = |words: &str| refused(&run(base("merge").arg("--base").arg(&chain.reconciled).arg("--technical").arg(&technical).arg("--output").arg(chain.folder.join("merge-membership-out"))), words);
    let path = technical.join("test-membership.json");
    let original = std::fs::read(&path).expect("a membership");
    let good: Value = serde_json::from_slice(&original).expect("json");
    let pinned = |edit: &dyn Fn(&mut Value), words: &str| {
        let mut value = good.clone();
        edit(&mut value);
        let bytes = serde_json::to_vec(&value).expect("json");
        std::fs::write(&path, &bytes).expect("a membership");
        edited(&technical.join("manifest.json"), &|manifest| manifest["test_membership_sha256"] = json!(digest(&bytes)), &|| merge(words));
        std::fs::write(&path, &original).expect("a membership");
    };
    pinned(&|value| value["document_ids_sha256"] = json!(["a".repeat(64)]), "test document membership mismatch");
    let base_membership: Value = serde_json::from_slice(&std::fs::read(chain.reconciled.join("test-membership.json")).expect("a membership")).expect("json");
    pinned(&|value| value["row_ids_sha256"][0] = base_membership["row_ids_sha256"][0].clone(), "origin test membership overlap");
    // A technical corpus whose test is empty: its sidecar still names test families.
    let packed = gz(b"");
    std::fs::write(technical.join("test.jsonl.gz"), &packed).expect("a split");
    edited(&technical.join("manifest.json"), &|manifest| manifest["splits"]["test"] = json!({"path": "test.jsonl.gz", "sha256": digest(&packed), "content_sha256": digest(b""), "rows": 0}), &|| {
        pinned(&|value| {
            for field in ["row_ids_sha256", "family_ids_sha256", "document_ids_sha256"] {
                value[field] = json!([]);
            }
        }, "test family membership mismatch");
    });
}

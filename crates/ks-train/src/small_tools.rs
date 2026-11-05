//! The short tools of a model cycle and a release: the base of a fit that refits some heads (`carry_context_heads`), what
//! a machine can run (`training_doctor`), the truncated forms dropped from the Russian supplement
//! (`filter_lexicon_supplement`), and the signed APT repository (`apt_repository`).
use crate::artifact::canonical;
use crate::corpus::{checksum, read_json};
use crate::heads::HEAD_PREFIXES;
use crate::physical::hex;
use flate2::Compression;
use flate2::write::GzEncoder;
use ks_context::context_model::ContextModel;
use ks_lang::SpellChecker;
use ks_lang::language_model::locale_fallbacks;
use ks_lang::LanguageModel;
use ks_text::{Indent, JsonOptions, json_dumps, pystr};
use md5::Md5;
use serde_json::{Map, Value, json};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const VERSION_HASH_CHARACTERS: usize = 12;

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn io(error: std::io::Error) -> String {
    error.to_string()
}

/// `carried`: the payload of `pair` without the weights of the heads named, with its own checksum and version.
pub fn carried(pair: &Value, refit: &[String]) -> Result<Value, String> {
    if refit.is_empty() {
        return Err("name a head to fit again".to_owned());
    }
    let prefixes: Vec<&str> = refit.iter().map(|name| HEAD_PREFIXES.iter().find(|head| head.0 == name).map(|head| head.1).ok_or_else(|| format!("no head {name}"))).collect::<Result<_, _>>()?;
    let weights = pair["weights"].as_object().ok_or("a pair without weights")?;
    if prefixes.iter().any(|prefix| !weights.keys().any(|name| name.starts_with(prefix))) {
        return Err("the pair has no weights for a head named to fit again".to_owned());
    }
    let kept: Map<String, Value> = weights.iter().filter(|(name, _)| !prefixes.iter().any(|prefix| name.starts_with(prefix))).map(|(name, values)| (name.clone(), values.clone())).collect();
    let raw = canonical(&Value::Object(kept.clone()));
    let digest = sha256_hex(&raw[..raw.len() - 1]);
    let mut output = pair.as_object().ok_or("a pair")?.clone();
    output.insert("weights".to_owned(), Value::Object(kept));
    output.insert("weights_sha256".to_owned(), json!(digest));
    output.insert("version".to_owned(), json!(format!("context-v3-{}", &digest[..VERSION_HASH_CHARACTERS])));
    Ok(Value::Object(output))
}

/// `carry`: writes the base and says what it is.
pub fn carry(root: &Path, pair_path: &Path, refit: &[String], output: &Path) -> Result<Value, String> {
    let models = root.join("resources/models");
    let source = ContextModel::load_artifact(pair_path, &models).map_err(|error| error.name.to_string())?;
    let base = carried(&read_json(pair_path)?, refit)?;
    std::fs::write(output, canonical(&base)).map_err(io)?;
    let loaded = ContextModel::load_artifact(output, &models).map_err(|error| error.name.to_string())?;
    Ok(json!({"artifact": output.to_string_lossy(), "sha256": checksum(output)?, "model_version": loaded.version, "pair": source.version, "refit": refit, "weights": base["weights"].as_object().map_or(0, Map::len)}))
}

struct Check {
    name: &'static str,
    passed: bool,
    detail: String,
}

fn check_reference_models(root: &Path) -> Check {
    let resources = ks_lang::Resources::with_speller(root.join("resources"), ks_hunspell::hunspell_speller(ks_config::history::data_dir()));
    match crate::tools::reference_models(root, &resources, true) {
        Ok(_) => Check { name: "reference_models", passed: true, detail: "pinned lexicons and Hunspell load".to_owned() },
        Err(error) => Check { name: "reference_models", passed: false, detail: format!("{error} (apt-get install hunspell-en-us hunspell-ru; or KEYSWITCH_HUNSPELL_PATH=model/intent_v1/sources/hunspell)") },
    }
}

/// `training_doctor`: which stages can run on this machine; `required` stages fail the exit status when blocked.
pub fn doctor(root: &Path, corpus: &Path, ledger: &Path, required: &[String]) -> i32 {
    let prefix_missing: Vec<String> = ["train", "development", "calibration"].iter().map(|split| format!("model/prefix_v1/{split}.jsonl.gz")).filter(|path| !root.join(path).is_file()).collect();
    let manifest = corpus.join("manifest.json");
    let checks = [
        check_reference_models(root),
        Check { name: "prefix_v1_data", passed: prefix_missing.is_empty(), detail: if prefix_missing.is_empty() { "prefix-v1 splits present".to_owned() } else { format!("missing {}", prefix_missing.join(", ")) } },
        Check { name: "context_action_corpus", passed: manifest.is_file(), detail: if manifest.is_file() { format!("manifest sha256 {}", checksum(&manifest).unwrap_or_default()) } else { format!("no {}: the frozen corpus is private, copy it from the author's machine", manifest.display()) } },
        Check { name: "context_action_ledger", passed: ledger.is_dir(), detail: if ledger.is_dir() { "test ledger present".to_owned() } else { format!("no {}: the test access ledger is private and must travel with the corpus", ledger.display()) } },
    ];
    println!("Checks:");
    for check in &checks {
        println!("  [{}] {}: {}", if check.passed { "ok" } else { "NO" }, check.name, check.detail);
    }
    let stages: [(&str, &str, &[&str], &str); 5] = [
        ("replay-boundary-v2", "replay of the shipped boundary-v2 seal", &["reference_models"], "ks-train boundary --tree . --verify"),
        ("train-prefix-v2", "new prefix-v2 candidate from model/prefix_v2/recipe.json", &["reference_models", "prefix_v1_data"], "ks-train prefix --tree . --output <folder>"),
        ("train-context-v3", "new context-v3 candidate from model/context_v3/recipe.json", &["reference_models", "context_action_corpus"], "ks-train fit --tree . --corpus <corpus> --output <folder>"),
        ("evaluate-context-v3", "sequence evaluation of a context-v3 + prefix-v2 pair", &["reference_models", "context_action_corpus", "context_action_ledger"], "ks-train evaluate --tree . --candidate ... --seal ... --prefix-candidate ... --prefix-seal ... --corpus ... --output ..."),
        ("promote-pair", "the checks, the sealed test, the install and the receipt", &["reference_models", "context_action_corpus", "context_action_ledger"], "ks-train promote --tree . --candidate ... --prefix-candidate ... --corpus ... --report ... --released ... --devchat ..."),
    ];
    println!("Stages:");
    let mut blocked = BTreeSet::new();
    for (name, description, requires, command) in stages {
        let missing: Vec<&str> = requires.iter().copied().filter(|needed| !checks.iter().any(|check| check.name == *needed && check.passed)).collect();
        if missing.is_empty() {
            println!("  [ready]   {name}: {description}\n            {command}");
        } else {
            blocked.insert(name);
            println!("  [blocked] {name}: {description}; needs {}", missing.join(", "));
        }
    }
    let failed: Vec<&str> = required.iter().map(String::as_str).filter(|name| blocked.contains(name)).collect();
    if failed.is_empty() {
        0
    } else {
        eprintln!("Blocked requested stages: {}", failed.join(", "));
        1
    }
}

const LEXICON_LOCALE: &str = "ru_RU";
const SUPPLEMENT: &str = "resources/lexicon-supplement-ru_RU.json";
const SUPPLEMENT_RECEIPT: &str = "resources/lexicon-supplement-ru_RU.receipt.json";
const HUNSPELL_ROOT: &str = "model/intent_v1/sources/hunspell";
const SUPPLEMENT_NAME: &str = "opensubtitles-2018-ru-full-min10-outside-onboard-v4";
const FILTER_RULE: &str = "drop a form when the reference Hunspell ru_RU dictionary accepts neither the form nor its capitalised spelling and the form is a proper prefix of a word of the onboard ru_RU lexicon (ru_RU.lm unigrams plus the built-in fallback words); keep every other form";
const SCOPE_ADDITION: &str = " Filtered by the supplement filter: forms the reference Hunspell dictionary does not accept (neither as written nor capitalised) that are proper prefixes of an onboard word are dropped, as subtitle line breaks cut words there; the receipt beside this file lists them.";

fn words_digest(words: &[String]) -> String {
    let mut sorted: Vec<&str> = words.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    let joined: String = sorted.iter().map(|word| format!("{word}\n")).collect();
    sha256_hex(joined.as_bytes())
}

fn accepted(speller: &dyn SpellChecker, form: &str) -> bool {
    speller.check(form) || speller.check(&format!("{}{}", pystr::upper(pystr::slice(form, Some(0), Some(1))), pystr::slice(form, Some(1), None)))
}

/// `prefix_owner`: the first onboard word (sorted) that the form begins without being it.
fn prefix_owner<'a>(onboard: &'a [String], form: &str) -> Option<&'a str> {
    let index = onboard.partition_point(|word| word.as_str() <= form);
    onboard.get(index).map(String::as_str).filter(|word| word.starts_with(form) && *word != form)
}

fn split_forms(forms: &[String], onboard: &[String], speller: &dyn SpellChecker) -> (Vec<String>, Vec<(String, String)>) {
    let (mut kept, mut dropped) = (Vec::new(), Vec::new());
    for form in forms {
        match prefix_owner(onboard, form) {
            Some(owner) if !accepted(speller, form) => dropped.push((form.clone(), owner.to_owned())),
            _ => kept.push(form.clone()),
        }
    }
    (kept, dropped)
}

/// `reference_inputs`: the onboard vocabulary and the reference dictionary, each checked against the intent pins.
fn reference_inputs(root: &Path) -> Result<(Vec<String>, Box<dyn SpellChecker>, Value), String> {
    let config = read_json(&root.join("model/intent_v1/config.json"))?;
    let (languages, hunspell) = (&config["sources"]["languages"][LEXICON_LOCALE], &config["external_evaluation"]["hunspell"][LEXICON_LOCALE]);
    let relative = languages["path"].as_str().ok_or("a lexicon without a path")?;
    let lexicon = root.join(relative);
    let (dictionary, affix) = (format!("{HUNSPELL_ROOT}/{LEXICON_LOCALE}.dic"), format!("{HUNSPELL_ROOT}/{LEXICON_LOCALE}.aff"));
    let pins = json!({"onboard_lexicon": {"path": relative, "sha256": checksum(&lexicon)?}, "hunspell_dictionary": {"path": dictionary, "sha256": checksum(&root.join(&dictionary))?}, "hunspell_affix": {"path": affix, "sha256": checksum(&root.join(&affix))?}});
    if pins["onboard_lexicon"]["sha256"] != languages["sha256"] || pins["hunspell_dictionary"]["sha256"] != hunspell["dictionary_sha256"] || pins["hunspell_affix"]["sha256"] != hunspell["affix_sha256"] {
        return Err("reference lexicon or dictionary differs from the intent configuration".to_owned());
    }
    let speller = ks_hunspell::HunspellDictionary::new(LEXICON_LOCALE, &ks_config::history::data_dir());
    if !speller.available() || Path::new(speller.source()).canonicalize().map_err(io)? != root.join(&dictionary).canonicalize().map_err(io)? {
        return Err(format!("the reference Hunspell dictionary is not the one loaded; run with KEYSWITCH_HUNSPELL_PATH={}", root.join(HUNSPELL_ROOT).display()));
    }
    let (frequencies, _) = LanguageModel::_read_arpa(&lexicon);
    let mut onboard: BTreeSet<String> = frequencies.iter().map(|(word, _)| word.to_owned()).collect();
    onboard.extend(locale_fallbacks(LEXICON_LOCALE).iter().map(|word| LanguageModel::normalize(word)));
    Ok((onboard.into_iter().collect(), Box::new(speller), pins))
}

fn strings(value: &Value) -> Vec<String> {
    value.as_array().map_or_else(Vec::new, |items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect())
}

/// `verify`: the packaged supplement is the recorded output, and the recorded split follows the rule.
pub fn verify_supplement(root: &Path) -> Result<Value, String> {
    let record = read_json(&root.join(SUPPLEMENT_RECEIPT))?;
    let raw = std::fs::read(root.join(SUPPLEMENT)).map_err(io)?;
    if record["output"]["sha256"].as_str() != Some(sha256_hex(&raw).as_str()) {
        return Err("the packaged supplement is not the recorded output".to_owned());
    }
    let (onboard, speller, pins) = reference_inputs(root)?;
    if record["references"] != pins {
        return Err("the reference lexicon or dictionary changed since the filter ran".to_owned());
    }
    let kept = strings(&serde_json::from_slice::<Value>(&raw).map_err(|error| error.to_string())?["words"]);
    let forms = strings(&record["dropped"]["forms"]);
    let together: Vec<String> = kept.iter().chain(forms.iter()).cloned().collect();
    if record["input"]["words_sha256"].as_str() != Some(words_digest(&together).as_str()) || record["input"]["words"].as_u64() != Some(together.len() as u64) {
        return Err("kept and dropped forms are not the recorded input".to_owned());
    }
    let (retained, removed) = split_forms(&kept, &onboard, speller.as_ref());
    if let Some((form, _)) = removed.first() {
        return Err(format!("a kept form meets the drop rule: {form}"));
    }
    if split_forms(&forms, &onboard, speller.as_ref()).1.len() != forms.len() {
        return Err("a dropped form does not meet the rule".to_owned());
    }
    Ok(json!({"verified": true, "kept": retained.len(), "dropped": forms.len()}))
}

/// `main` of the filter: the output, its receipt and the dropped forms' list.
pub fn filter_supplement(root: &Path, input: &Path, output: &Path, receipt_path: &Path, removed_list: Option<&Path>) -> Result<Value, String> {
    let input_raw = std::fs::read(input).map_err(io)?;
    let payload: Value = serde_json::from_slice(&input_raw).map_err(|error| error.to_string())?;
    let words = strings(&payload["words"]);
    let mut sorted = words.clone();
    sorted.sort();
    sorted.dedup();
    if words != sorted || payload["locale"].as_str() != Some(LEXICON_LOCALE) {
        return Err("the input is not a sorted Russian supplement".to_owned());
    }
    if payload["name"].as_str() == Some(SUPPLEMENT_NAME) {
        return Err("the input is already filtered".to_owned());
    }
    let (onboard, speller, pins) = reference_inputs(root)?;
    let (kept, dropped) = split_forms(&words, &onboard, speller.as_ref());
    let mut built = payload.as_object().ok_or("a supplement")?.clone();
    let mut selection = payload["selection"].as_object().cloned().unwrap_or_default();
    selection.insert("dropped_truncations".to_owned(), json!(dropped.len()));
    selection.insert("selected".to_owned(), json!(kept.len()));
    built.insert("name".to_owned(), json!(SUPPLEMENT_NAME));
    built.insert("scope".to_owned(), json!(format!("{}{SCOPE_ADDITION}", payload["scope"].as_str().unwrap_or(""))));
    built.insert("selection".to_owned(), Value::Object(selection));
    built.insert("words".to_owned(), json!(kept));
    let output_raw = canonical(&Value::Object(built));
    std::fs::write(output, &output_raw).map_err(io)?;
    let mut forms: Vec<&str> = dropped.iter().map(|entry| entry.0.as_str()).collect();
    forms.sort_unstable();
    let receipt = json!({"schema_version": 1, "tool": "crates/ks-train/src/small_tools.rs", "rule": FILTER_RULE,
        "input": {"sha256": sha256_hex(&input_raw), "words": words.len(), "words_sha256": words_digest(&words)},
        "output": {"path": SUPPLEMENT, "sha256": sha256_hex(&output_raw), "words": kept.len(), "words_sha256": words_digest(&kept)},
        "references": pins, "dropped": {"count": dropped.len(), "forms": forms}});
    let readable = JsonOptions { ensure_ascii: false, sort_keys: true, indent: Some(Indent::Spaces(2)), ..JsonOptions::default() };
    std::fs::write(receipt_path, json_dumps(&receipt, &readable) + "\n").map_err(io)?;
    if let Some(path) = removed_list {
        let mut pairs = dropped.clone();
        pairs.sort();
        std::fs::write(path, pairs.iter().map(|(form, owner)| format!("{form}\t{owner}\n")).collect::<String>()).map_err(io)?;
    }
    Ok(json!({"input": words.len(), "kept": kept.len(), "dropped": dropped.len(), "output_sha256": sha256_hex(&output_raw)}))
}

const SOURCES_STANZA: &str = "packaging/debian/keyswitch.sources";
const PUBLIC_KEY: &str = "packaging/keyswitch-archive-keyring.asc";
const SITE: &str = "packaging/site";
const PACKAGE_NAME: &str = "keyswitch";
const RELEASE_DESCRIPTION: &str = "KeySwitch packages for Ubuntu";
const KEYRING_FILE_NAME: &str = "keyswitch-archive-keyring.asc";
const COMPUTED_INDEX_FIELDS: [&str; 5] = ["Filename", "Size", "MD5sum", "SHA1", "SHA256"];
const APT_INDEX_FILE_VARIANTS: usize = 2;
const SECONDS_PER_DAY: i64 = 86_400;

struct Sources {
    uri: String,
    suite: String,
    component: String,
    architecture: String,
    keyring_path: String,
}

impl Sources {
    fn dists(&self) -> PathBuf {
        Path::new("dists").join(&self.suite)
    }

    fn index_relative(&self) -> String {
        format!("{}/binary-{}", self.component, self.architecture)
    }

    fn pool(&self) -> String {
        format!("pool/{}/{}/{PACKAGE_NAME}", self.component, &PACKAGE_NAME[..1])
    }
}

fn stanza_field(stanza: &str, name: &str) -> Result<String, String> {
    let prefix = format!("{name}:");
    for line in stanza.lines() {
        if line.starts_with('#') || !line.starts_with(&prefix) {
            continue;
        }
        let values: Vec<&str> = line[prefix.len()..].split_whitespace().collect();
        let [value] = values.as_slice() else { return Err(format!("{SOURCES_STANZA} must give exactly one {name} value")) };
        return Ok((*value).to_owned());
    }
    Err(format!("{SOURCES_STANZA} is missing the {name} field"))
}

fn read_sources(root: &Path) -> Result<Sources, String> {
    let stanza = std::fs::read_to_string(root.join(SOURCES_STANZA)).map_err(io)?;
    let uri = stanza_field(&stanza, "URIs")?;
    if !uri.starts_with("https://") || !uri.ends_with('/') {
        return Err(format!("{SOURCES_STANZA} must serve HTTPS from a directory"));
    }
    if stanza_field(&stanza, "Types")? != "deb" {
        return Err(format!("{SOURCES_STANZA} must declare a binary repository"));
    }
    Ok(Sources { uri, suite: stanza_field(&stanza, "Suites")?, component: stanza_field(&stanza, "Components")?, architecture: stanza_field(&stanza, "Architectures")?, keyring_path: stanza_field(&stanza, "Signed-By")? })
}

fn run(command: &mut Command) -> Result<String, String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let done = command.output().map_err(|error| format!("{program}: {error}"))?;
    if !done.status.success() {
        return Err(format!("{program} failed ({}): {}{}", done.status, String::from_utf8_lossy(&done.stdout), String::from_utf8_lossy(&done.stderr)));
    }
    Ok(String::from_utf8_lossy(&done.stdout).into_owned())
}

fn digests(payload: &[u8]) -> (String, String, String) {
    (hex(&Md5::digest(payload)), hex(&Sha1::digest(payload)), hex(&Sha256::digest(payload)))
}

fn deb_field(package: &Path, name: &str) -> Result<String, String> {
    Ok(run(Command::new("dpkg-deb").arg("--field").arg(package).arg(name))?.trim().to_owned())
}

fn index_entry(package: &Path, pool_path: &str) -> Result<String, String> {
    let control = run(Command::new("dpkg-deb").arg("--field").arg(package))?;
    let control = control.trim_end_matches('\n');
    for field in COMPUTED_INDEX_FIELDS {
        if control.lines().any(|line| line.starts_with(&format!("{field}:"))) {
            return Err(format!("{} carries a {field} control field of its own", package.display()));
        }
    }
    let payload = std::fs::read(package).map_err(io)?;
    let (md5, sha1, sha256) = digests(&payload);
    Ok(format!("{control}\nFilename: {pool_path}\nSize: {}\nMD5sum: {md5}\nSHA1: {sha1}\nSHA256: {sha256}", payload.len()))
}

/// `%a, %d %b %Y %H:%M:%S UTC` of the current moment.
fn release_date() -> String {
    let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs()) as i64;
    let (days, rest) = (seconds.div_euclid(SECONDS_PER_DAY), seconds.rem_euclid(SECONDS_PER_DAY));
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let of_era = shifted.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let cycle = (5 * of_year + 2) / 153;
    let day = of_year - (153 * cycle + 2) / 5 + 1;
    let month = if cycle < 10 { cycle + 3 } else { cycle - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{}, {day:02} {} {year:04} {:02}:{:02}:{:02} UTC", WEEKDAYS[days.rem_euclid(7) as usize], MONTHS[month as usize - 1], rest / 3600, rest / 60 % 60, rest % 60)
}

fn release_stanza(sources: &Sources, indices: &[(String, Vec<u8>)]) -> String {
    let mut lines = vec!["Origin: KeySwitch".to_owned(), "Label: KeySwitch".to_owned(), format!("Suite: {}", sources.suite), format!("Codename: {}", sources.suite), format!("Architectures: {}", sources.architecture), format!("Components: {}", sources.component), format!("Date: {}", release_date()), format!("Description: {RELEASE_DESCRIPTION}"), "Acquire-By-Hash: no".to_owned()];
    let mut ordered: Vec<&(String, Vec<u8>)> = indices.iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));
    lines.push("MD5Sum:".to_owned());
    lines.extend(ordered.iter().map(|(name, payload)| format!(" {} {} {name}", digests(payload).0, payload.len())));
    lines.push("SHA256:".to_owned());
    lines.extend(ordered.iter().map(|(name, payload)| format!(" {} {} {name}", digests(payload).2, payload.len())));
    format!("{}\n", lines.join("\n"))
}

fn landing_page(root: &Path, sources: &Sources) -> Result<String, String> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).map_err(io)?;
    let version = manifest.lines().find_map(|line| line.strip_prefix("version = \"")).map(|rest| rest.trim_end_matches('"')).ok_or("Cargo.toml names no version")?;
    let mut page = std::fs::read_to_string(root.join(SITE).join("index.html")).map_err(io)?;
    for (placeholder, value) in [("@VERSION@", version), ("@URI@", &sources.uri), ("@SUITE@", &sources.suite), ("@COMPONENT@", &sources.component), ("@ARCHITECTURE@", &sources.architecture), ("@KEYRING_PATH@", &sources.keyring_path), ("@KEYRING_FILE@", KEYRING_FILE_NAME)] {
        page = page.replace(placeholder, value);
    }
    if let Some(found) = regex::Regex::new("@[A-Z_]+@").expect("a pattern").find(&page) {
        return Err(format!("{SITE}/index.html keeps {} unfilled", found.as_str()));
    }
    Ok(page)
}

fn gpg_common(passphrase: Option<&Path>) -> Command {
    let mut command = Command::new("gpg");
    command.args(["--batch", "--yes", "--armor"]);
    if let Some(file) = passphrase {
        command.args(["--pinentry-mode", "loopback", "--passphrase-file"]).arg(file);
    }
    command
}

fn copy_tree(from: &Path, to: &Path, skip: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(from).map_err(io)?.flatten() {
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            std::fs::create_dir_all(&target).map_err(io)?;
            copy_tree(&path, &target, skip)?;
        } else if path != skip {
            std::fs::copy(&path, &target).map_err(io)?;
        }
    }
    Ok(())
}

/// `build`: the complete repository, written and signed with the key in the ambient GnuPG home.
pub fn apt_build(root: &Path, repository: &Path, packages: &[PathBuf], passphrase: Option<&Path>) -> Result<String, String> {
    let sources = read_sources(root)?;
    if packages.is_empty() {
        return Err("The repository needs at least one package".to_owned());
    }
    let (pool, index_relative) = (sources.pool(), sources.index_relative());
    let index_directory = repository.join(sources.dists()).join(&index_relative);
    std::fs::create_dir_all(repository.join(&pool)).map_err(io)?;
    std::fs::create_dir_all(&index_directory).map_err(io)?;
    let mut ordered: Vec<&PathBuf> = packages.iter().collect();
    ordered.sort_by_key(|path| path.file_name().map(|name| name.to_os_string()));
    let mut entries = Vec::new();
    for package in ordered {
        let architecture = deb_field(package, "Architecture")?;
        if architecture != sources.architecture {
            return Err(format!("{} is built for {architecture}, not {}", package.display(), sources.architecture));
        }
        if deb_field(package, "Package")? != PACKAGE_NAME {
            return Err(format!("{} is not a {PACKAGE_NAME} package", package.display()));
        }
        let pool_path = format!("{pool}/{}", package.file_name().ok_or("a package without a name")?.to_string_lossy());
        std::fs::copy(package, repository.join(&pool_path)).map_err(io)?;
        entries.push(index_entry(package, &pool_path)?);
    }
    let index = format!("{}\n", entries.join("\n\n")).into_bytes();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(9));
    encoder.write_all(&index).map_err(io)?;
    let compressed = encoder.finish().map_err(io)?;
    std::fs::write(index_directory.join("Packages"), &index).map_err(io)?;
    std::fs::write(index_directory.join("Packages.gz"), &compressed).map_err(io)?;
    let release = release_stanza(&sources, &[(format!("{index_relative}/Packages"), index), (format!("{index_relative}/Packages.gz"), compressed)]);
    let dists = repository.join(sources.dists());
    let release_path = dists.join("Release");
    std::fs::write(&release_path, release).map_err(io)?;
    run(gpg_common(passphrase).arg("--output").arg(dists.join("InRelease")).arg("--clearsign").arg(&release_path))?;
    run(gpg_common(passphrase).arg("--output").arg(dists.join("Release.gpg")).arg("--detach-sign").arg(&release_path))?;
    std::fs::copy(root.join(PUBLIC_KEY), repository.join(KEYRING_FILE_NAME)).map_err(io)?;
    let site = root.join(SITE);
    copy_tree(&site, repository, &site.join("index.html"))?;
    std::fs::write(repository.join("index.html"), landing_page(root, &sources)?).map_err(io)?;
    Ok(format!("Built {}/{} for {} in {}", sources.suite, sources.component, sources.architecture, repository.display()))
}

/// `verify`: the published repository, the way an installed system reads it.
pub fn apt_verify(root: &Path, repository: &Path, expected_version: &str, public_key: Option<&Path>) -> Result<String, String> {
    let sources = read_sources(root)?;
    let key = public_key.map_or_else(|| root.join(PUBLIC_KEY), Path::to_path_buf);
    let dists = repository.join(sources.dists());
    let scratch = std::env::temp_dir().join(format!("keyswitch-apt-verify-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).map_err(io)?;
    let keyring = scratch.join("trusted.gpg");
    run(Command::new("gpg").env("GNUPGHOME", &scratch).args(["--batch", "--yes", "--output"]).arg(&keyring).arg("--dearmor").arg(&key))?;
    run(Command::new("gpgv").arg("--keyring").arg(&keyring).arg(dists.join("Release.gpg")).arg(dists.join("Release")))?;
    run(Command::new("gpgv").arg("--keyring").arg(&keyring).arg(dists.join("InRelease")))?;
    let release = std::fs::read_to_string(dists.join("Release")).map_err(io)?;
    if !std::fs::read_to_string(dists.join("InRelease")).map_err(io)?.contains(release.trim()) {
        return Err("The inline signature does not cover the release file".to_owned());
    }
    let (mut checked, mut algorithm) = (0, "");
    for line in release.lines() {
        if line == "MD5Sum:" {
            algorithm = "md5";
        } else if line == "SHA256:" {
            algorithm = "sha256";
        }
        if !line.starts_with(' ') {
            continue;
        }
        if algorithm.is_empty() {
            return Err("The release file lists a digest before naming it".to_owned());
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        let [digest, size, name] = parts.as_slice() else { return Err("A malformed digest line".to_owned()) };
        let payload = std::fs::read(dists.join(name)).map_err(io)?;
        let (md5, _, sha256) = digests(&payload);
        if (if algorithm == "md5" { md5 } else { sha256 }) != *digest || size.parse::<usize>().ok() != Some(payload.len()) {
            return Err(format!("The release file misstates {name}"));
        }
        checked += 1;
    }
    if checked != 2 * APT_INDEX_FILE_VARIANTS {
        return Err("The release file does not cover both indices".to_owned());
    }
    let state = scratch.join("state");
    for relative in ["lists/partial", "cache/archives/partial", "etc"] {
        std::fs::create_dir_all(state.join(relative)).map_err(io)?;
    }
    std::fs::write(state.join("status"), "").map_err(io)?;
    let source_file = state.join("etc/keyswitch.sources");
    let repository_uri = repository.canonicalize().map_err(io)?;
    std::fs::write(&source_file, format!("Types: deb\nURIs: file://{}/\nSuites: {}\nComponents: {}\nArchitectures: {}\nSigned-By: {}\n", repository_uri.display(), sources.suite, sources.component, sources.architecture, key.display())).map_err(io)?;
    let options = |command: &mut Command| {
        for setting in [format!("Dir::Etc::sourcelist={}", source_file.display()), "Dir::Etc::sourceparts=/dev/null".to_owned(), "Dir::Etc::trusted=/dev/null".to_owned(), "Dir::Etc::trustedparts=/dev/null".to_owned(), format!("Dir::State::lists={}", state.join("lists").display()), format!("Dir::State::status={}", state.join("status").display()), format!("Dir::Cache={}", state.join("cache").display()), format!("APT::Architecture={}", sources.architecture), format!("APT::Architectures::={}", sources.architecture), "APT::Get::List-Cleanup=0".to_owned()] {
            command.arg("-o").arg(setting);
        }
    };
    let mut update = Command::new("apt-get");
    options(&mut update);
    run(update.arg("update"))?;
    let mut policy = Command::new("apt-cache");
    options(&mut policy);
    let listed = run(policy.args(["policy", PACKAGE_NAME]))?;
    let _ = std::fs::remove_dir_all(&scratch);
    let candidate = listed.lines().find_map(|line| line.trim().strip_prefix("Candidate:")).map(|value| value.trim().to_owned()).ok_or("APT found no candidate version in the published repository")?;
    if candidate != expected_version {
        return Err(format!("APT offers {candidate} where the release publishes {expected_version}"));
    }
    Ok(format!("APT_REPOSITORY_OK uri={} suite={} component={} architecture={} candidate={candidate}", sources.uri, sources.suite, sources.component, sources.architecture))
}

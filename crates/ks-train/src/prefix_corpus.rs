//! Public, incremental input situations for the prefix task (`prefix_corpus`): ground truth comes from an intended whole
//! word and an authored setting, never from the early-switch threshold. No private logs. `--verify` rebuilds every split
//! and compares its content with the frozen file; `--freeze` writes a new generation into an empty folder.
use ks_cast::To as _;
use crate::corpus::{checksum, read_json};
use crate::ortho_inputs::load_phrases;
use crate::physical::{hex, hex_digest, pair};
use crate::tools::{Lexicons, reference_models};
use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use ks_backend::field_context::{FieldContext, FieldRole};
use ks_context::prefix_model::{PrefixInput, PrefixSide, features};
use ks_lang::Resources;
use ks_lang::constants::PREFIX_INDEX_DICTIONARY_MAX_BYTES;
use ks_lang::early_switch::{EarlySwitchPolicy, PrefixIndex, PrefixScorer, dictionary_stems, early_switch_decision};
use ks_text::pystr;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

const SPLITS: [&str; 4] = ["train", "development", "calibration", "test"];
const PROFILES: [&str; 2] = ["portable", "reference_hunspell"];
const BEFORE_CHARACTERS: usize = 160;
const FAMILY_KEY_CHARACTERS: usize = 3;
const SHARED_WAIT_MAX_CHARACTERS: usize = 2;
const SPLIT_BUCKET_COUNT: u64 = 10;
const TRAIN_SPLIT_BUCKETS: u64 = 7;
const CHOICE_HEX_DIGITS: usize = 8;
const DELETION_INDEX: usize = 3;
const KEEP: i64 = 0;
const CONVERT: i64 = 1;
const WAIT: i64 = 2;
const CONFIG: &str = "model/prefix_v1/config.json";

fn family(word: &str, group: i64) -> String {
    let physical = if group == 1 { pair().translate(word, "ru", "us").expect("layouts") } else { word.to_owned() };
    pystr::casefold(&physical).replace('`', "t").chars().take(FAMILY_KEY_CHARACTERS).collect()
}

fn split_for(signature: &str, namespace: &str) -> &'static str {
    let digest = hex_digest(&format!("{namespace}:{signature}"));
    let bucket = u64::from_str_radix(digest.get(..CHOICE_HEX_DIGITS).unwrap_or_default(), 16).expect("a digest") % SPLIT_BUCKET_COUNT;
    if bucket < TRAIN_SPLIT_BUCKETS { "train" } else { SPLITS[(bucket - TRAIN_SPLIT_BUCKETS + 1).to::<usize>()] }
}

/// `WORDS.finditer`: `[^\W_]+(?:['’\-][^\W_]+)*`, as (first character, word).
fn word_matches(characters: &[char]) -> Vec<(usize, String)> {
    let letter = |character: char| character != '_' && pystr::isalnum(character.encode_utf8(&mut [0; 4]));
    let mut matches = Vec::new();
    let mut index = 0;
    while index < characters.len() {
        if !letter(characters[index]) {
            index += 1;
            continue;
        }
        let start = index;
        while index < characters.len() && letter(characters[index]) {
            index += 1;
        }
        while index + 1 < characters.len() && matches!(characters[index], '\'' | '’' | '-') && letter(characters[index + 1]) {
            index += 1;
            while index < characters.len() && letter(characters[index]) {
                index += 1;
            }
        }
        matches.push((start, characters[start..index].iter().collect()));
    }
    matches
}

struct Situation {
    signature: String,
    split: &'static str,
    source: i64,
    text: String,
    before: String,
    role: FieldRole,
    application: &'static str,
    category: &'static str,
}

/// `situations`.
fn situations(root: &Path, config: &Value) -> Result<Vec<Situation>, String> {
    let number = |name: &str| config[name].as_u64().map(ks_cast::To::to::<usize>).ok_or_else(|| format!("config.{name}"));
    let (minimum, maximum, per_language, per_family) = (number("minimum_word_length")?, number("maximum_word_length")?, number("maximum_words_per_language")?, number("maximum_words_per_family")?);
    let namespace = config["namespace"].as_str().ok_or("a namespace")?;
    let mut words: BTreeMap<(String, i64), i64> = BTreeMap::new();
    let mut preceding: HashMap<(String, i64), String> = HashMap::new();
    for phrase in load_phrases(root)? {
        let characters: Vec<char> = phrase.text.chars().collect();
        for (start, found) in word_matches(&characters) {
            let word = pystr::casefold(&found);
            if !(minimum..=maximum).contains(&word.chars().count()) {
                continue;
            }
            let russian = word.chars().all(|character| ('а'..='я').contains(&character) || character == 'ё');
            let english = word.chars().all(|character| character.is_ascii_lowercase());
            if !(russian || english) {
                continue;
            }
            let key = (word, i64::from(russian));
            *words.entry(key.clone()).or_insert(0) += 1;
            preceding.entry(key).or_insert_with(|| characters[start.saturating_sub(BEFORE_CHARACTERS)..start].iter().collect());
        }
    }
    let mut ranked: Vec<((String, i64), i64)> = words.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let (mut counts, mut families): (HashMap<i64, usize>, HashMap<String, usize>) = Default::default();
    let mut output = Vec::new();
    for ((word, target), _) in ranked {
        let signature = family(&word, target);
        if *counts.get(&target).unwrap_or(&0) >= per_language || *families.get(&signature).unwrap_or(&0) >= per_family {
            continue;
        }
        *counts.entry(target).or_insert(0) += 1;
        *families.entry(signature.clone()).or_insert(0) += 1;
        let split = split_for(&signature, namespace);
        let wrong = if target != 0 { pair().translate(&word, "ru", "us") } else { pair().translate(&word, "us", "ru") }.expect("layouts");
        let before = preceding[&(word.clone(), target)].clone();
        let letters: Vec<char> = word.chars().collect();
        let typo: String = letters.iter().enumerate().filter(|(index, _)| *index != DELETION_INDEX).map(|(_, character)| *character).collect();
        let physical = if target != 0 { wrong.clone() } else { word.clone() };
        let other = 1 - target;
        let mut rows: Vec<(String, i64, String, &'static str, FieldRole, &'static str)> = vec![
            (word.clone(), target, before.clone(), "Telegram", FieldRole::Text, "correct"),
            (wrong.clone(), other, before.clone(), "Telegram", FieldRole::Text, "wrong"),
            (wrong.clone(), other, String::new(), "TestEditor", FieldRole::Unknown, "wrong_empty"),
            (wrong.clone(), other, if target != 0 { "// пояснение " } else { "// explanation " }.to_owned(), "Code", FieldRole::Code, "comment"),
            (wrong.clone(), other, if target != 0 { "# описание " } else { "# description " }.to_owned(), "WindowsTerminal", FieldRole::Terminal, "shell_comment"),
            (typo, target, before.clone(), "Telegram", FieldRole::Text, "typo"),
            (format!("{physical}_value"), 0, "const value = ".to_owned(), "Code", FieldRole::Code, "identifier"),
            (physical, 0, "$ ".to_owned(), "WindowsTerminal", FieldRole::Terminal, "command"),
        ];
        if target == 0 {
            rows.push((word.clone(), 0, "в сообщении написано ".to_owned(), "Telegram", FieldRole::Text, "mixed_english"));
        }
        for (text, source, before, application, role, category) in rows {
            output.push(Situation { signature: signature.clone(), split, source, text, before, role, application, category });
        }
    }
    Ok(output)
}

struct Profile {
    models: Lexicons,
    indexes: Vec<PrefixIndex>,
}

/// `lexicon`: the models and prefix indexes the engine builds, from the pinned inputs. Indexes hold the plain onboard words
/// and, with Hunspell, the stems of its dictionary; they never hold the supplement.
fn profile(root: &Path, resources: &Resources, spelling: bool) -> Result<Profile, String> {
    let models = reference_models(root, resources, spelling)?;
    let mut indexes = Vec::new();
    for (group, locale) in ["en_US", "ru_RU"].into_iter().enumerate() {
        let onboard = ks_lang::LanguageModel::load(locale, std::iter::empty::<&str>(), &[root.join("model/intent_v1/sources")], ks_lang::LanguageModel::disabled_speller());
        if spelling {
            let stems = dictionary_stems(Path::new(models[group].speller.source()), PREFIX_INDEX_DICTIONARY_MAX_BYTES);
            indexes.push(PrefixIndex::new(onboard.frequencies.words().iter().chain(stems.iter()), onboard.frequencies.clone()));
        } else {
            indexes.push(PrefixIndex::new(onboard.frequencies.words().iter(), onboard.frequencies.clone()));
        }
    }
    Ok(Profile { models, indexes })
}

fn role_name(role: FieldRole) -> &'static str {
    role.as_str()
}

/// The rows of every split, each row a canonical line.
fn build(root: &Path, resources: &Arc<Resources>, config: &Value) -> Result<(BTreeMap<&'static str, Vec<u8>>, BTreeMap<&'static str, i64>, BTreeMap<&'static str, BTreeSet<String>>), String> {
    let profiles = [profile(root, resources, false)?, profile(root, resources, true)?];
    let maximum_prefix = (config["maximum_prefix_length"].as_u64().ok_or("config.maximum_prefix_length")?).to::<usize>();
    let mut content: BTreeMap<&'static str, Vec<u8>> = SPLITS.iter().map(|split| (*split, Vec::new())).collect();
    let mut counts: BTreeMap<&'static str, i64> = SPLITS.iter().map(|split| (*split, 0)).collect();
    let mut groups: BTreeMap<&'static str, BTreeSet<String>> = SPLITS.iter().map(|split| (*split, BTreeSet::new())).collect();
    let policy = EarlySwitchPolicy::default();
    for (sequence, situation) in situations(root, config)?.into_iter().enumerate() {
        groups.get_mut(situation.split).expect("a split").insert(situation.signature.clone());
        let letters: Vec<char> = situation.text.chars().collect();
        let (from, to) = if situation.source != 0 { ("ru", "us") } else { ("us", "ru") };
        let desired = matches!(situation.category, "wrong" | "wrong_empty" | "comment" | "shell_comment");
        for length in 1..=letters.len().min(maximum_prefix) {
            let original: String = letters[..length].iter().collect();
            let alternate = pair().translate(&original, from, to).expect("layouts");
            let ambiguous = profiles[0].indexes[situation.source.to::<usize>()].completions(&original).completions > 0;
            let label = if length <= SHARED_WAIT_MAX_CHARACTERS || (desired && ambiguous) { WAIT } else if desired { CONVERT } else { KEEP };
            for (name, lexicon) in PROFILES.iter().zip(&profiles) {
                let item = PrefixInput { original: original.clone(), alternative: alternate.clone(), source_group: situation.source, field: FieldContext { role: situation.role, before: situation.before.clone(), ..FieldContext::new(situation.application, "public-prefix") } };
                let sides = [PrefixSide { index: &lexicon.indexes[0], model: &lexicon.models[0] }, PrefixSide { index: &lexicon.indexes[1], model: &lexicon.models[1] }];
                let values: Map<String, Value> = features(&item, [&sides[0], &sides[1]]).into_iter().map(|(feature, value)| (feature, json!(value))).collect();
                let indexes: Vec<(i64, &PrefixIndex)> = lexicon.indexes.iter().enumerate().map(|(group, index)| (group.to::<i64>(), index)).collect();
                let scorers: Vec<(i64, &dyn PrefixScorer)> = lexicon.models.iter().enumerate().map(|(group, model)| -> (i64, &dyn PrefixScorer) { (group.to::<i64>(), &**model) }).collect();
                let baseline = early_switch_decision(&indexes, &scorers, &original, &[(1 - situation.source, alternate.as_str())], situation.source, &policy);
                let row = json!({"sequence": sequence, "family": situation.signature, "source": situation.source, "text": situation.text, "before": situation.before, "application": situation.application, "role": role_name(situation.role), "category": situation.category, "length": length, "profile": name, "label": label, "desired": desired, "legacy_convert": baseline.should_switch, "features": values});
                content.get_mut(situation.split).expect("a split").extend(crate::artifact::canonical(&row));
                *counts.get_mut(situation.split).expect("a split") += 1;
            }
        }
    }
    for left in SPLITS {
        for right in SPLITS {
            if left != right && groups[left].intersection(&groups[right]).next().is_some() {
                return Err("physical prefix families overlap".to_owned());
            }
        }
    }
    Ok((content, counts, groups))
}

/// The command: `verify` against the frozen files, or `freeze` into `output`.
pub fn command(root: &Path, resources: &Arc<Resources>, output: Option<&Path>) -> Result<(), String> {
    let config = read_json(&root.join(CONFIG))?;
    let (content, counts, groups) = build(root, resources, &config)?;
    let Some(output) = output else {
        let directory = root.join("model/prefix_v1");
        let receipt = read_json(&directory.join("corpus.json"))?;
        let mut differences = 0;
        for split in SPLITS {
            let mut raw = Vec::new();
            MultiGzDecoder::new(std::fs::File::open(directory.join(format!("{split}.jsonl.gz"))).map_err(|error| error.to_string())?).read_to_end(&mut raw).map_err(|error| error.to_string())?;
            let same = raw == content[split] && receipt["sha256"][split].as_str() == Some(&checksum(&directory.join(format!("{split}.jsonl.gz")))?) && receipt["rows"][split] == json!(counts[split]) && receipt["families"][split] == json!(groups[split].len());
            println!("{split}: {}", if same { "same" } else { "DIFFERS" });
            differences += usize::from(!same);
        }
        return if differences == 0 { Ok(()) } else { Err(format!("{differences} splits of the prefix corpus differ")) };
    };
    std::fs::create_dir_all(output).map_err(|error| error.to_string())?;
    let mut digests = Map::new();
    for split in SPLITS {
        let path = output.join(format!("{split}.jsonl.gz"));
        if path.exists() {
            return Err("refusing to overwrite frozen prefix examples".to_owned());
        }
        let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(&content[split]).map_err(|error| error.to_string())?;
        std::fs::write(&path, encoder.finish().map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
        digests.insert(split.to_owned(), json!(hex(&Sha256::digest(std::fs::read(&path).map_err(|error| error.to_string())?))));
    }
    let families: Map<String, Value> = groups.iter().map(|(split, set)| ((*split).to_owned(), json!(set.len()))).collect();
    let mut provenance = Map::new();
    for file in [CONFIG, "model/context_v2/sources/tatoeba-cc0-en-ru.tsv.gz", "crates/ks-train/src/prefix_corpus.rs", "crates/ks-context/src/prefix_model.rs", "crates/ks-lang/src/early_switch.rs", "crates/ks-lang/src/language_model.rs", "crates/ks-lang/src/spellcheck.rs", "crates/ks-text/src/layouts.rs", "crates/ks-lang/src/lexicon_supplement.rs", "resources/lexicon-supplement-ru_RU.json"] {
        provenance.insert(file.to_owned(), json!(checksum(&root.join(file))?));
    }
    let receipt = json!({"schema_version": 1, "provenance": provenance, "sha256": digests, "rows": counts, "families": families, "family_overlap": 0, "profiles": PROFILES, "scope": config["evidence_scope"]});
    std::fs::write(output.join("corpus.json"), crate::artifact::canonical(&receipt)).map_err(|error| error.to_string())
}

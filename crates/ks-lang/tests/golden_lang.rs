//! Parity of `ks-lang` with the Python, against golden files dumped by `gen_golden_lang.py`.
//!
//! The files live in `$KS_GOLDEN_DIR/lang` (default: the `golden` folder of the private work tree), the resources in
//! `$KS_RESOURCES` (default: those of the Python tree); a test skips with a message when they are absent. Floats are
//! compared by their bits. The first word of `words.txt` is the empty string, so the list is split on `\n` alone.

#![expect(clippy::unwrap_used, reason = "a test or a probe stops on what it did not expect")]
use ks_cast::To as _;
use flate2::read::GzDecoder;
use ks_lang::boundary_model::{self, BoundaryModel, Features};
use ks_lang::boundary_policy::{self, BoundaryPolicy};
use ks_lang::early_switch::{self, EarlySwitchDecision, EarlySwitchPolicy, PrefixEvidence, PrefixIndex, PrefixScorer};
use ks_lang::identifier_lexicon::IdentifierLexicon;
use ks_lang::language_model::{LanguageModel, WordScore};
use ks_lang::ortho_model::{self as ortho, Identifier, KnownWord, OrthoEvidence, OrthoModel, OrthoScore};
use ks_lang::packed_grams::PackedGrams;
use ks_lang::{lexicon_supplement, FnSpellChecker, Resources, WordTable};
use ks_text::layouts::LayoutPair;
use ks_text::pystr;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const THREADS: usize = 4;
const BATCH: usize = 8192;

fn golden(name: &str) -> Option<PathBuf> {
    let root = std::env::var("KS_GOLDEN_DIR").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../.t/golden").to_string());
    let path = PathBuf::from(root).join("lang").join(name);
    // The answers on made-up files are kept in the repository; those built on real typing are not.
    let kept = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/lang")).join(name);
    if path.exists() {
        Some(path)
    } else if kept.exists() {
        Some(kept)
    } else {
        eprintln!("SKIPPED: golden file {} is absent", path.display());
        None
    }
}

fn resource_dir() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var("KS_RESOURCES").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources").to_string()));
    if path.join("models/ortho_v1.json").exists() {
        Some(path)
    } else {
        eprintln!("SKIPPED: no resources at {}", path.display());
        None
    }
}

fn reader(name: &str) -> Option<BufReader<GzDecoder<std::fs::File>>> {
    golden(name).map(|path| BufReader::new(GzDecoder::new(std::fs::File::open(path).expect("open"))))
}

fn word_lines(name: &str) -> Option<Vec<String>> {
    let mut text = String::new();
    reader(name)?.read_to_string(&mut text).expect("gz text");
    let mut words: Vec<String> = text.split('\n').map(str::to_string).collect();
    words.pop();
    Some(words)
}

fn hex(x: f64) -> String {
    format!("{:016x}", x.to_bits())
}

fn hex_of(value: &Value) -> &str {
    value.as_str().expect("hex")
}

#[derive(Default)]
struct Failures {
    count: AtomicUsize,
    total: AtomicUsize,
    shown: Mutex<Vec<String>>,
}

impl Failures {
    fn check<T: PartialEq + std::fmt::Debug>(&self, what: &str, input: &str, got: T, want: T) {
        self.total.fetch_add(1, Ordering::Relaxed);
        if got != want {
            self.count.fetch_add(1, Ordering::Relaxed);
            let mut shown = self.shown.lock().unwrap();
            if shown.len() < 25 {
                shown.push(format!("{what}({input:?}): got {got:?}, want {want:?}"));
            }
        }
    }

    fn finish(self, name: &str) {
        let (count, total) = (self.count.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed));
        eprintln!("{name}: {total} checks, {count} failures");
        assert!(total > 0, "{name}: nothing was compared");
        assert_eq!(count, 0, "{name}:\n{}", self.shown.lock().unwrap().join("\n"));
    }
}

/// Calls `visit(index, row)` for every row of a file, on several threads, a batch at a time.
fn for_each_row<F: Fn(usize, &Value) + Sync>(name: &str, skip: usize, visit: F) -> bool {
    let Some(file) = reader(name) else {
        return false;
    };
    let mut lines = file.lines().skip(skip);
    let mut base = 0;
    loop {
        let batch: Vec<String> = lines.by_ref().take(BATCH).map(|line| line.expect("line")).collect();
        if batch.is_empty() {
            return true;
        }
        let visit = &visit;
        std::thread::scope(|scope| {
            for (part, chunk) in batch.chunks(BATCH.div_ceil(THREADS)).enumerate() {
                let first = base + part * BATCH.div_ceil(THREADS);
                scope.spawn(move || {
                    for (offset, line) in chunk.iter().enumerate() {
                        visit(first + offset, &serde_json::from_str(line).expect("json row"));
                    }
                });
            }
        });
        base += batch.len();
    }
}

struct Fixture {
    resources: Arc<Resources>,
    en: Arc<LanguageModel>,
    ru: Arc<LanguageModel>,
    words: Vec<String>,
    prefixes: Vec<String>,
}

fn fixture() -> Option<&'static Fixture> {
    static FIXTURE: OnceLock<Option<Fixture>> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let (dir, words, prefixes) = (resource_dir()?, word_lines("words.txt.gz")?, word_lines("prefixes.txt.gz")?);
            let resources = Resources::new(dir);
            let en = resources.engine_language_model("en_US").expect("en");
            let ru = resources.engine_language_model("ru_RU").expect("ru");
            Some(Fixture { resources, en, ru, words, prefixes })
        })
        .as_ref()
}

fn check_score(fail: &Failures, what: &str, word: &str, got: &WordScore, want: &Value) {
    let flags = i64::from(got.known) | i64::from(got.exact) << 1 | i64::from(got.spell_known) << 2;
    let row = serde_json::json!([hex(got.value), flags, got.frequency, hex(got.gram_ratio), hex(got.ngram_score), hex(got.invalid_ratio), hex(got.raw_ngram_score)]);
    fail.check(what, word, &row, want);
}

fn check_language_row(fail: &Failures, word: &str, row: &Value, en: &LanguageModel, ru: &LanguageModel) {
    let normalized = LanguageModel::normalize(word);
    fail.check("normalize", word, normalized.as_str(), row[0].as_str().unwrap());
    for (slot, model) in [(1, en), (2, ru)] {
        let want = &row[slot];
        fail.check("_raw_ngram_score", word, hex(model.raw_ngram_score(word)).as_str(), hex_of(&want[0]));
        fail.check("ngram_score", word, hex(model.ngram_score(word)).as_str(), hex_of(&want[1]));
        fail.check("frequency", word, model.frequencies.get_or_zero(&normalized), want[2].as_i64().unwrap());
        check_score(fail, "score", word, &model.score(word), &want[3]);
        check_score(fail, "best_single_deletion", word, &model.best_single_deletion(word), &want[4]);
    }
}

#[test]
fn language_tables() {
    let (Some(f), Some(file)) = (fixture(), reader("language_tables.jsonl.gz")) else {
        return;
    };
    let fail = Failures::default();
    let plain_en = f.resources.language_model("en_US", std::iter::empty::<&str>());
    let plain_ru = f.resources.language_model("ru_RU", std::iter::empty::<&str>());
    enum Target<'a> {
        Model(&'a LanguageModel),
        Arpa(WordTable, WordTable),
    }
    let mut target = Target::Model(&f.en);
    for line in file.lines() {
        let row: Value = serde_json::from_str(&line.expect("line")).expect("json");
        match (&row, &target) {
            (Value::Object(header), _) if header["kind"] == "model" => {
                let name = header["name"].as_str().unwrap();
                let model = match name {
                    "en_US+supplement" => &*f.en,
                    "ru_RU+supplement" => &*f.ru,
                    "en_US" => &plain_en,
                    _ => &plain_ru,
                };
                fail.check("locale", name, model.locale.as_str(), header["locale"].as_str().unwrap());
                fail.check("source", name, model.source.as_str(), header["source"].as_str().unwrap());
                fail.check("words", name, model.frequencies.len().to::<u64>(), header["words"].as_u64().unwrap());
                fail.check("bigrams", name, model.bigrams.len().to::<u64>(), header["bigrams"].as_u64().unwrap());
                fail.check("maximum", name, model.maximum, header["maximum"].as_i64().unwrap());
                fail.check("maximum_bigram", name, model.maximum_bigram, header["maximum_bigram"].as_i64().unwrap());
                fail.check("_ngram_mean", name, hex(model.ngram_mean).as_str(), header["mean"].as_str().unwrap());
                fail.check("_ngram_deviation", name, hex(model.ngram_deviation).as_str(), header["deviation"].as_str().unwrap());
                for (slot, order) in [2, 3, 4].into_iter().enumerate() {
                    let (size, total) = model.gram_size(order);
                    fail.check("gram size", name, size.to::<u64>(), header["gram_sizes"][slot].as_u64().unwrap());
                    fail.check("gram total", name, total, header["gram_totals"][slot].as_u64().unwrap());
                }
                target = Target::Model(model);
            }
            (Value::Object(header), _) => {
                let locale = header["locale"].as_str().unwrap();
                let (unigrams, bigrams) = LanguageModel::_read_arpa(Path::new(&format!("/usr/share/onboard/models/{locale}.lm")));
                fail.check("arpa words", locale, unigrams.len().to::<u64>(), header["words"].as_u64().unwrap());
                fail.check("arpa bigrams", locale, bigrams.len().to::<u64>(), header["bigrams"].as_u64().unwrap());
                target = Target::Arpa(unigrams, bigrams);
            }
            (_, Target::Model(model)) => match row[0].as_str().unwrap() {
                "f" => fail.check("frequencies", row[1].as_str().unwrap(), model.frequencies.get(row[1].as_str().unwrap()), row[2].as_i64()),
                "b" => fail.check("bigrams", row[1].as_str().unwrap(), model.bigram(row[1].as_str().unwrap(), row[2].as_str().unwrap()), row[3].as_i64().unwrap()),
                _ => {
                    let gram = row[1].as_str().unwrap();
                    fail.check("gram", gram, i64::from(model.gram_count(gram.chars().count(), gram)), row[2].as_i64().unwrap());
                    if gram.chars().count() == 3 {
                        fail.check("_grams", gram, model.has_gram(gram), true);
                    }
                }
            },
            (_, Target::Arpa(unigrams, bigrams)) => match row[0].as_str().unwrap() {
                "f" => fail.check("arpa word", row[1].as_str().unwrap(), unigrams.get(row[1].as_str().unwrap()), row[2].as_i64()),
                _ => fail.check("arpa bigram", row[1].as_str().unwrap(), bigrams.get(&format!("{}\0{}", row[1].as_str().unwrap(), row[2].as_str().unwrap())), row[3].as_i64()),
            },
        }
    }
    fail.finish("language_tables");
}

#[test]
fn language_words() {
    let Some(f) = fixture() else {
        return;
    };
    let fail = Failures::default();
    if for_each_row("language_words.jsonl.gz", 0, |index, row| check_language_row(&fail, &f.words[index], row, &f.en, &f.ru)) {
        fail.finish("language_words");
    }
}

#[test]
fn language_pairs() {
    let Some(f) = fixture() else {
        return;
    };
    let fail = Failures::default();
    if for_each_row("language_pairs.jsonl.gz", 0, |_index, row| {
        let (left, right) = (row[0].as_str().unwrap(), row[1].as_str().unwrap());
        fail.check("context_score en", left, hex(f.en.context_score(left, right)).as_str(), hex_of(&row[2]));
        fail.check("context_score ru", left, hex(f.ru.context_score(left, right)).as_str(), hex_of(&row[3]));
    }) {
        fail.finish("language_pairs");
    }
}

/// The same words with the answers Hunspell gave in the Python, through a `FnSpellChecker`.
#[test]
fn language_words_with_replayed_hunspell() {
    let (Some(f), Some(mut live)) = (fixture(), reader("language_words_live.jsonl.gz")) else {
        return;
    };
    let Some(answers_en) = word_lines("hunspell-answers-en_US.txt.gz") else {
        return;
    };
    let Some(answers_ru) = word_lines("hunspell-answers-ru_RU.txt.gz") else {
        return;
    };
    let mut header = String::new();
    live.read_line(&mut header).expect("header");
    let header: Value = serde_json::from_str(&header).expect("json");
    let table = |lines: &[String]| {
        Arc::new(WordTable::from_pairs(lines.iter().map(|line| {
            let (word, answer) = line.rsplit_once('\t').expect("word and answer");
            (word.to_string(), i64::from(answer == "1"))
        })))
    };
    let (table_en, table_ru) = (table(&answers_en), table(&answers_ru));
    let missing = Arc::new(AtomicUsize::new(0));
    let resources = {
        let (table_en, table_ru, header, missing) = (table_en.clone(), table_ru.clone(), header.clone(), missing.clone());
        Resources::with_speller(
            f.resources.resource_dir().to_path_buf(),
            Box::new(move |locale| {
                let table = if locale == "en_US" { table_en.clone() } else { table_ru.clone() };
                let missing = missing.clone();
                Box::new(FnSpellChecker::new(header[locale]["speller_source"].as_str().unwrap(), move |word: &str| {
                    table.get(word).unwrap_or_else(|| {
                        missing.fetch_add(1, Ordering::Relaxed);
                        0
                    }) == 1
                }))
            }),
        )
    };
    let en = resources.engine_language_model("en_US").unwrap();
    let ru = resources.engine_language_model("ru_RU").unwrap();
    let fail = Failures::default();
    fail.check("source", "en", en.source.as_str(), header["en_US"]["source"].as_str().unwrap());
    fail.check("source", "ru", ru.source.as_str(), header["ru_RU"]["source"].as_str().unwrap());
    if for_each_row("language_words_live.jsonl.gz", 1, |index, row| check_language_row(&fail, &f.words[index], row, &en, &ru)) {
        fail.check("words the dump does not hold", "", missing.load(Ordering::Relaxed), 0);
        fail.finish("language_words_live");
    }
}

fn check_ortho(fail: &Failures, what: &str, input: &str, got: &OrthoScore, want: &Value) {
    let row = serde_json::json!([hex(got.ratio), hex(got.shape_ratio), hex(got.total), hex(got.source_logprob), hex(got.target_logprob), i64::from(got.supported), hex(got.features), got.reading]);
    fail.check(what, input, &row, want);
}

fn strings(value: &Value) -> Vec<&str> {
    value.as_array().unwrap().iter().map(|item| item.as_str().unwrap()).collect()
}

#[test]
fn ortho_words() {
    let Some(f) = fixture() else {
        return;
    };
    let (model, status) = f.resources.ortho_model();
    let model = model.expect("the artifact loads");
    let Some(mut file) = reader("ortho_words.jsonl.gz") else {
        return;
    };
    let mut header = String::new();
    file.read_line(&mut header).expect("header");
    let meta: Value = serde_json::from_str(&header).expect("json");
    let fail = Failures::default();
    fail.check("status", "", status.as_str(), meta["status"].as_str().unwrap());
    fail.check("order", "", model.order.to::<u64>(), meta["order"].as_u64().unwrap());
    fail.check("version", "", model.version.as_str(), meta["version"].as_str().unwrap());
    fail.check("minimum_length", "", model.minimum_length, meta["minimum_length"].as_i64().unwrap());
    fail.check("collapse", "", model.collapse, meta["collapse"].as_i64());
    fail.check("stretch", "", model.stretch, meta["stretch"].as_i64());
    fail.check("hyphens", "", model.hyphens, meta["hyphens"].as_bool().unwrap());
    fail.check("unscored", "", model.unscored_shapes.len(), meta["unscored"].as_array().unwrap().len());
    fail.check("gated", "", model.gated.clone(), strings(&meta["gated"]));
    fail.check("compound en", "", model.compound_min_letters.en, meta["compound_min_letters"]["en"].as_i64());
    fail.check("compound ru", "", model.compound_min_letters.ru, meta["compound_min_letters"]["ru"].as_i64());
    fail.check("target_centres", "", model.target_centres.is_some(), !meta["target_centres"].as_object().unwrap().is_empty());
    for script in ortho::SCRIPTS {
        fail.check("threshold", script, hex(*model.thresholds.get(script)).as_str(), hex_of(&meta["thresholds"][script]));
        fail.check("centre", script, hex(*model.centres.as_ref().unwrap().get(script)).as_str(), hex_of(&meta["centres"][script]));
        let weights = model.features.as_ref().unwrap().get(script);
        for (position, (name, weight)) in weights.iter().enumerate() {
            fail.check("feature name", script, *name, meta["features"][script][position][0].as_str().unwrap());
            fail.check("feature weight", name, hex(*weight).as_str(), hex_of(&meta["features"][script][position][1]));
        }
        fail.check("features", script, weights.len(), meta["features"][script].as_array().unwrap().len());
        for (slot, shape) in ortho::SHAPES.iter().enumerate() {
            fail.check("prose", shape, hex(model.prose_shape.get(script)[slot]).as_str(), hex_of(&meta["prose"][script][shape]));
            fail.check("acronym", shape, hex(model.acronym_shape.get(script)[slot]).as_str(), hex_of(&meta["acronym"][script][shape]));
        }
    }
    fail.check("uniform", "en", hex(model.channels.en.uniform).as_str(), hex_of(&meta["uniform"]["en"]));
    fail.check("uniform", "ru", hex(model.channels.ru.uniform).as_str(), hex_of(&meta["uniform"]["ru"]));
    fail.check("uniform", "source_ru", hex(model.source_channels.ru.as_ref().unwrap().uniform).as_str(), hex_of(&meta["uniform"]["source_ru"]));
    let lexicon = f.resources.identifier_lexicon().0.expect("the lexicon loads");
    let known = |script: &str, word: &str| if script == "en" { f.en.score(word).known } else { f.ru.score(word).known };
    if for_each_row("ortho_words.jsonl.gz", 1, |index, row| {
        let word = f.words[index].as_str();
        fail.check("readings", word, ortho::readings(word, "en").1.as_str(), row[0].as_str().unwrap());
        fail.check("readings ru", word, ortho::readings(word, "ru").0.as_str(), row[0].as_str().unwrap());
        fail.check("reduplicated", word, i64::from(ortho::reduplicated(word)), row[1].as_i64().unwrap());
        for (slot, limit) in [(2, 2), (3, 1), (4, 0)] {
            fail.check("collapse_runs", word, ortho::collapse_runs(word, limit).as_str(), row[slot].as_str().unwrap());
        }
        for (slot, minimum) in [(5, 3), (6, 2)] {
            fail.check("read_once", word, ortho::read_once(word, minimum).as_str(), row[slot].as_str().unwrap());
        }
        fail.check("drop_hyphen_stretches", word, ortho::drop_hyphen_stretches(word).as_str(), row[7].as_str().unwrap());
        for (slot, (collapse, stretch, hyphens)) in [(8, (Some(2), Some(3), true)), (9, (Some(2), Some(3), false)), (10, (None, None, false)), (11, (None, Some(2), true))] {
            let got = ortho::stretch_readings(word, collapse, stretch, hyphens);
            fail.check("stretch_readings", word, got.iter().map(String::as_str).collect::<Vec<_>>(), strings(&row[slot]));
        }
        fail.check("shape_of first", word, ortho::shape_of(word, true), row[12].as_str().unwrap());
        let shape = ortho::shape_of(word, false);
        fail.check("shape_of", word, shape, row[13].as_str().unwrap());
        for (slot, script) in [(14, "en"), (15, "ru")] {
            let want = &row[slot];
            let identifier = |token: &str| lexicon.contains(token);
            let features = ortho::token_features(word, script, &identifier, &known, shape, true, &model.compound_min_letters);
            let values: Vec<i64> = features.as_pairs().iter().map(|pair| pair.1).collect();
            let wanted: Vec<i64> = want[0].as_array().unwrap().iter().map(|value| value.as_i64().unwrap()).collect();
            fail.check("token_features", word, values, wanted);
            fail.check("one_key_extra", word, i64::from(ortho::one_key_extra(word, script, &known)), want[1].as_i64().unwrap());
            check_ortho(&fail, "score", word, &model.score(&OrthoEvidence { keys: word, shape, source_script: script }, None), &want[2]);
        }
        if index % 16 == 0 {
            for (offset, shape) in ["lower", "initial", "inner", "upper", "weird"].into_iter().enumerate() {
                check_ortho(&fail, "score by shape", word, &model.score(&OrthoEvidence { keys: word, shape, source_script: "en" }, None), &row[16 + offset]);
            }
            check_ortho(&fail, "score de", word, &model.score(&OrthoEvidence { keys: word, shape: "lower", source_script: "de" }, None), &row[21]);
            let odd = |_script: &str, text: &str| pystr::len(text).is_multiple_of(2);
            check_ortho(&fail, "score known", word, &model.score(&OrthoEvidence { keys: word, shape: "lower", source_script: "ru" }, Some(&odd)), &row[22]);
            fail.check("channel en", word, hex(model.channels.en.score(word)).as_str(), hex_of(&row[23][0]));
            fail.check("channel ru", word, hex(model.channels.ru.score(word)).as_str(), hex_of(&row[23][1]));
            fail.check("channel source", word, hex(model.source_channels.ru.as_ref().unwrap().score(word)).as_str(), hex_of(&row[23][2]));
        }
    }) {
        fail.finish("ortho_words");
    }
}

#[test]
fn packed_tables() {
    let Some(f) = fixture() else {
        return;
    };
    let (model, _status) = f.resources.ortho_model();
    let model = model.expect("the artifact loads");
    let Some(file) = reader("packed_tables.jsonl.gz") else {
        return;
    };
    let fail = Failures::default();
    let mut entries: Option<Box<dyn Iterator<Item = (String, f64)> + '_>> = None;
    for line in file.lines() {
        let row: Value = serde_json::from_str(&line.expect("line")).expect("json");
        if let Value::Object(header) = &row {
            let channel = match header["channel"].as_str().unwrap() {
                "en" => &model.channels.en,
                "ru" => &model.channels.ru,
                _ => model.source_channels.ru.as_ref().unwrap(),
            };
            let table = if header["table"] == "logprob" { &channel.logprob } else { &channel.backoff };
            let packed = table.packed().expect("the real tables pack");
            fail.check("len", "", packed.len().to::<u64>(), header["len"].as_u64().unwrap());
            entries = Some(Box::new(packed.iter()));
        } else {
            let (gram, value) = entries.as_mut().unwrap().next().expect("an entry for each row");
            fail.check("iter", &gram, gram.as_str(), row[0].as_str().unwrap());
            fail.check("value", &gram, hex(value).as_str(), hex_of(&row[1]));
        }
    }
    fail.finish("packed_tables");
}

#[test]
fn packed_cases() {
    let Some(file) = reader("packed_cases.jsonl.gz") else {
        return;
    };
    let fail = Failures::default();
    for line in file.lines() {
        let row: Value = serde_json::from_str(&line.expect("line")).expect("json");
        let name = row["name"].as_str().unwrap();
        let table: Vec<(&str, f64)> = row["table"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| (entry[0].as_str().unwrap(), f64::from_bits(u64::from_str_radix(entry[1].as_str().unwrap(), 16).unwrap())))
            .collect();
        let packed = PackedGrams::pack(&table);
        fail.check("pack", name, packed.is_some(), row["packed"].as_bool().unwrap());
        if let Some(packed) = packed {
            fail.check("len", name, packed.len().to::<u64>(), row["len"].as_u64().unwrap());
            fail.check("iter", name, packed.iter().map(|entry| entry.0).collect::<Vec<_>>(), strings(&row["iter"]).iter().map(ToString::to_string).collect::<Vec<_>>());
            for probe in row["probes"].as_array().unwrap() {
                fail.check("get", probe[0].as_str().unwrap(), packed.get(probe[0].as_str().unwrap()).map(hex), probe[1].as_str().map(str::to_string));
            }
        }
    }
    fail.finish("packed_cases");
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ks-lang-golden-{}-{name}", std::process::id()))
}

#[test]
fn ortho_tiny_artifacts() {
    let Some(file) = reader("ortho_tiny.jsonl.gz") else {
        return;
    };
    let identifiers = ["htop", "ls", "ghbdtn"];
    let known_en = ["hello", "ab", "the", "r", "ut", "d", "dist", "ghb"];
    let known_ru = ["привет", "ну", "в", "к", "то", "я", "ура", "приве"];
    let known = move |script: &str, word: &str| if script == "en" { known_en.contains(&word) } else { known_ru.contains(&word) };
    let identifier: Identifier = Arc::new(move |token| identifiers.contains(&token));
    let known_word: KnownWord = Arc::new(known);
    let fail = Failures::default();
    let path = temp_path("tiny.json");
    for line in file.lines() {
        let row: Value = serde_json::from_str(&line.expect("line")).expect("json");
        std::fs::write(&path, row["artifact"].to_string()).unwrap();
        let model = OrthoModel::load(&path, Some(identifier.clone()), Some(known_word.clone()), None).expect("a tiny artifact loads");
        for case in row["scores"].as_array().unwrap() {
            let (keys, shape, script) = (case[0].as_str().unwrap(), case[1].as_str().unwrap(), case[2].as_str().unwrap());
            check_ortho(&fail, "score", &format!("{} {keys} {shape} {script}", row["schema"]), &model.score(&OrthoEvidence { keys, shape, source_script: script }, Some(&known)), &case[3]);
        }
    }
    std::fs::remove_file(&path).unwrap();
    fail.finish("ortho_tiny");
}

#[test]
fn ortho_load_errors() {
    let Some(file) = reader("ortho_errors.jsonl.gz") else {
        return;
    };
    let fail = Failures::default();
    let path = temp_path("errors.json");
    let directory = temp_path("errors-dir");
    std::fs::create_dir_all(&directory).unwrap();
    for line in file.lines() {
        let row: Value = serde_json::from_str(&line.expect("line")).expect("json");
        let want = row[1].as_str().unwrap();
        let status = match row[0].as_str() {
            None => OrthoModel::try_load(&temp_path("absent.json"), None).1,
            Some("<directory>") => OrthoModel::try_load(&directory, None).1,
            Some(text) => {
                std::fs::write(&path, text).unwrap();
                OrthoModel::try_load(&path, None).1
            }
        };
        let shown = row[0].as_str().unwrap_or("").chars().take(4000).collect::<String>();
        // The artifact without its channels, which only make the message long.
        let shown = shown.split("\"models\"").next().unwrap_or("").to_string() + row[0].as_str().unwrap_or("").rsplit("\"thresholds\"").next().unwrap_or("");
        fail.check("try_load", &shown, status.as_str(), want);
    }
    let _ = std::fs::remove_file(&path);
    std::fs::remove_dir_all(&directory).unwrap();
    fail.finish("ortho_errors");
}

fn digest_lines<'a, I: Iterator<Item = &'a str>>(lines: I) -> String {
    let mut hasher = Sha256::new();
    for (index, line) in lines.enumerate() {
        if index > 0 {
            hasher.update(b"\n");
        }
        hasher.update(line.as_bytes());
    }
    let mut text = String::new();
    for byte in hasher.finalize() {
        write!(text, "{byte:02x}").unwrap();
    }
    text
}

#[test]
fn lexicons() {
    let (Some(f), Some(file)) = (fixture(), reader("lexicons.jsonl.gz")) else {
        return;
    };
    let fail = Failures::default();
    for line in file.lines() {
        let row: Value = serde_json::from_str(&line.expect("line")).expect("json");
        if row["kind"] == "identifiers" {
            let (lexicon, status) = f.resources.identifier_lexicon();
            let lexicon = lexicon.expect("the lexicon loads");
            fail.check("name", "", lexicon.name.as_str(), row["name"].as_str().unwrap());
            fail.check("version", "", lexicon.version.as_str(), row["version"].as_str().unwrap());
            fail.check("status", "", status.as_str(), row["status"].as_str().unwrap());
            fail.check("len", "", lexicon.identifiers.len().to::<u64>(), row["len"].as_u64().unwrap());
            fail.check("digest", "", digest_lines(lexicon.identifiers.iter()).as_str(), row["digest"].as_str().unwrap());
            for (word, want) in f.words.iter().zip(row["contains"].as_str().unwrap().chars()) {
                fail.check("contains", word, lexicon.contains(word), want == '1');
            }
            let normalized: Vec<String> = f.words.iter().map(|word| IdentifierLexicon::normalize(word)).collect();
            fail.check("normalize", "", digest_lines(normalized.iter().map(String::as_str)).as_str(), row["normalize"].as_str().unwrap());
        } else {
            let locale = row["locale"].as_str().unwrap();
            let words = f.resources.supplement_words(locale).expect("the supplement loads");
            fail.check("supplement len", locale, words.len().to::<u64>(), row["len"].as_u64().unwrap());
            fail.check("supplement", locale, digest_lines(words.iter()).as_str(), row["digest"].as_str().unwrap());
        }
    }
    fail.finish("lexicons");
}

#[test]
fn lexicon_load_errors() {
    let Some(file) = reader("lexicon_errors.jsonl.gz") else {
        return;
    };
    let fail = Failures::default();
    let root = temp_path("lexicons");
    std::fs::create_dir_all(&root).unwrap();
    let name_of = |error: ks_lang::LoadError| error.name.to_string();
    for line in file.lines() {
        let row: Value = serde_json::from_str(&line.expect("line")).expect("json");
        let kind = row[0].as_str().unwrap();
        if kind == "supplement" {
            let (locale, text) = (row[1].as_str().unwrap(), row[2].as_str().unwrap());
            let path = root.join(format!("lexicon-supplement-{locale}.json"));
            std::fs::write(&path, text).unwrap();
            let got = lexicon_supplement::supplement_words(&root, locale);
            std::fs::remove_file(&path).unwrap();
            match got {
                Ok(words) => {
                    fail.check("supplement", text, "ok", row[3].as_str().unwrap());
                    fail.check("supplement words", text, words.iter().collect::<Vec<_>>(), strings(&row[4]));
                }
                Err(error) => fail.check("supplement", text, name_of(error).as_str(), row[3].as_str().unwrap()),
            }
            continue;
        }
        let text = row[1].as_str().unwrap();
        // JSON has no NaN: serde_json refuses the document where the Python refuses the weight. Both are refusals.
        if text.contains("NaN") {
            continue;
        }
        let path = root.join("file.json");
        std::fs::write(&path, text).unwrap();
        let got = match kind {
            "identifier" => IdentifierLexicon::load(&path).map(|_| ()),
            "policy" => BoundaryPolicy::load(&path).map(|_| ()),
            _ => BoundaryModel::load(&path).map(|_| ()),
        };
        fail.check(kind, text, got.map_or_else(name_of, |()| "ok".to_string()).as_str(), row[2].as_str().unwrap());
    }
    std::fs::remove_dir_all(&root).unwrap();
    fail.finish("lexicon_errors");
}

fn evidence_row(evidence: &PrefixEvidence) -> Value {
    serde_json::json!([evidence.completions, evidence.maximum_frequency, i64::from(evidence.known)])
}

fn check_decision(fail: &Failures, input: &str, got: &EarlySwitchDecision, want: &Value) {
    let row = serde_json::json!([
        i64::from(got.should_switch), got.source_group, got.target_group, got.original, got.replacement, got.reason,
        got.source_evidence.as_ref().map(evidence_row), got.target_evidence.as_ref().map(evidence_row)
    ]);
    fail.check("decision", input, &row, &Value::Array(want.as_array().unwrap()[..8].to_vec()));
    let dict: Value = serde_json::from_str(want[8].as_str().unwrap()).expect("json");
    fail.check("as_dict", input, &got.as_dict(), &dict);
}

#[test]
fn early_switch_on_prefixes() {
    let Some(f) = fixture() else {
        return;
    };
    let production = [f.resources.prefix_index(&f.en), f.resources.prefix_index(&f.ru)];
    let reference = [PrefixIndex::from_lexicon(f.en.frequencies.clone(), "reference"), PrefixIndex::from_lexicon(f.ru.frequencies.clone(), "reference")];
    let Some(mut file) = reader("early_prefixes.jsonl.gz") else {
        return;
    };
    let mut header = String::new();
    file.read_line(&mut header).expect("header");
    let header: Value = serde_json::from_str(&header).expect("json");
    let fail = Failures::default();
    for group in 0..2 {
        fail.check("production len", "", production[group].len().to::<u64>(), header["production_len"][group].as_u64().unwrap());
        fail.check("reference len", "", reference[group].len().to::<u64>(), header["reference_len"][group].as_u64().unwrap());
    }
    let completed = for_each_row("early_prefixes.jsonl.gz", 1, |index, row| {
        let prefix = f.prefixes[index].as_str();
        fail.check("prefix", prefix, prefix, row[0].as_str().unwrap());
        for (slot, index_of) in [(1, &*production[0]), (2, &*production[1]), (3, &reference[0]), (4, &reference[1])] {
            fail.check("completions", prefix, evidence_row(&index_of.completions(prefix)), row[slot].clone());
        }
        if index % 64 == 0 {
            fail.check("completions limit", prefix, evidence_row(&production[1].completions_limit(prefix, 3)), row[5].clone());
        }
    });
    let indexes: [(i64, &PrefixIndex); 2] = [(0, &production[0]), (1, &production[1])];
    let gentle = EarlySwitchPolicy { minimum_length: 2, minimum_completions: 2, minimum_frequency: 100, dominant_frequency: 5000 };
    let pair = LayoutPair::default();
    let completed = completed
        && for_each_row("early_decisions.jsonl.gz", 0, |index, row| {
            let prefix = f.prefixes[index].as_str();
            let scorers: [(i64, &dyn PrefixScorer); 2] = [(0, &*f.en), (1, &*f.ru)];
            let mut slot = 1;
            for (group, (source, target)) in [(0i64, ("us", "ru")), (1, ("ru", "us"))] {
                let replacement = pair.translate(prefix, source, target).unwrap();
                let alternatives = [(1 - group, replacement.as_str())];
                check_decision(&fail, prefix, &early_switch::early_switch_decision(&indexes, &scorers, prefix, &alternatives, group, &EarlySwitchPolicy::default()), &row[slot]);
                slot += 1;
                if index % 8 == 0 {
                    check_decision(&fail, prefix, &early_switch::early_switch_decision(&indexes, &scorers, prefix, &alternatives, group, &gentle), &row[slot]);
                    slot += 1;
                }
            }
        });
    if completed {
        fail.finish("early_switch");
    }
}

#[test]
fn early_switch_misc() {
    let (Some(f), Some(file)) = (fixture(), reader("early_misc.jsonl.gz")) else {
        return;
    };
    let fail = Failures::default();
    let mut index = None;
    for line in file.lines() {
        let row: Value = serde_json::from_str(&line.expect("line")).expect("json");
        if let Some(locale) = row.get("stems").and_then(Value::as_str) {
            let path = PathBuf::from(row["path"].as_str().unwrap());
            if !path.is_file() {
                eprintln!("SKIPPED: no dictionary {}", path.display());
                index = None;
                continue;
            }
            let stems = early_switch::dictionary_stems(&path, ks_lang::constants::PREFIX_INDEX_DICTIONARY_MAX_BYTES);
            fail.check("stems", locale, stems.len().to::<u64>(), row["len"].as_u64().unwrap());
            fail.check("stems digest", locale, digest_lines(stems.iter()).as_str(), row["digest"].as_str().unwrap());
            fail.check("stems limited", locale, early_switch::dictionary_stems(&path, 10).len().to::<u64>(), row["limited"].as_u64().unwrap());
            let source = format!("x; Hunspell: {}", path.display());
            fail.check("dictionary path", locale, early_switch::hunspell_dictionary_path(&source), Some(path));
            let plain = f.resources.language_model(locale, std::iter::empty::<&str>()).frequencies.clone();
            index = Some(PrefixIndex::from_lexicon(plain, &source));
        } else if let Some(locale) = row.get("stems_index").and_then(Value::as_str) {
            if let Some(index) = &index {
                fail.check("stems index len", locale, index.len().to::<u64>(), row["len"].as_u64().unwrap());
            }
        } else if row[1].is_array() {
            if let Some(index) = &index {
                fail.check("stems completions", row[0].as_str().unwrap(), evidence_row(&index.completions(row[0].as_str().unwrap())), row[1].clone());
            }
        } else {
            let word = row[0].as_str().unwrap();
            fail.check("abbreviation", word, i64::from(early_switch::looks_like_abbreviation(word)), row[1].as_i64().unwrap());
        }
    }
    fail.finish("early_misc");
}

fn features_digest(values: &Features) -> String {
    let mut hasher = Sha256::new();
    for (name, value) in values {
        hasher.update(format!("{name}={};", hex(*value)).as_bytes());
    }
    let mut text = String::new();
    for byte in &hasher.finalize()[..8] {
        write!(text, "{byte:02x}").unwrap();
    }
    text
}

fn hexes(values: &[f64]) -> Vec<String> {
    values.iter().map(|value| hex(*value)).collect()
}

fn texts(value: &Value) -> Vec<String> {
    strings(value).iter().map(ToString::to_string).collect()
}

#[test]
fn boundary_words() {
    let Some(f) = fixture() else {
        return;
    };
    let Some(mut file) = reader("boundary_words.jsonl.gz") else {
        return;
    };
    let mut header = String::new();
    file.read_line(&mut header).expect("header");
    let header: Value = serde_json::from_str(&header).expect("json");
    let fail = Failures::default();
    let policy = f.resources.boundary_policy().expect("the policy loads");
    fail.check("policy threshold", "", hex(policy.threshold).as_str(), header["threshold"].as_str().unwrap());
    fail.check("policy version", "", policy.version.as_str(), header["version"].as_str().unwrap());
    fail.check("policy feature version", "", policy.feature_version, header["feature_version"].as_i64().unwrap());
    fail.check("policy weights", "", policy.weights.len().to::<u64>(), header["weights"].as_u64().unwrap());
    fail.check("policy default", "", true, header["policy_default"].as_bool().unwrap());
    fail.check("model default", "", f.resources.boundary_model().is_some(), header["model_default"].as_bool().unwrap());
    let Some(artifact) = golden("boundary-v1-test.json") else {
        return;
    };
    let legacy = BoundaryModel::load(&artifact).expect("the test artifact loads");
    let pair = LayoutPair::default();
    let punctuation = ks_lang::constants::BOUNDARY_AMBIGUOUS_PUNCTUATION;
    if for_each_row("boundary_words.jsonl.gz", 1, |index, row| {
        let word = row[0].as_str().unwrap();
        fail.check("misspelled en", word, i64::from(boundary_policy::misspelled(word, &f.en)), row[1].as_i64().unwrap());
        fail.check("misspelled ru", word, i64::from(boundary_policy::misspelled(word, &f.ru)), row[2].as_i64().unwrap());
        for (slot, (source, target, from, to)) in [(3, (&*f.en, &*f.ru, "us", "ru")), (4, (&*f.ru, &*f.en, "ru", "us"))] {
            let part = row[slot].as_array().unwrap();
            let alternative = pair.translate(word, from, to).unwrap();
            fail.check("alternative", word, alternative.as_str(), part[0].as_str().unwrap());
            let length = pystr::len(word).to::<i64>();
            let tail = length - (pystr::len(pystr::rstrip_chars(word, punctuation))).to::<i64>();
            let count = tail.min(ks_lang::constants::BOUNDARY_MAX_LITERAL_SUFFIX_CHARACTERS).min((length - 1).max(0)) + 1;
            let candidates: Vec<Features> = (0..count).map(|suffix| policy.extract(word, &alternative, suffix, source, target)).collect();
            fail.check("features", word, candidates.iter().map(features_digest).collect::<Vec<_>>(), texts(&part[1]));
            fail.check("probabilities", word, hexes(&policy.probabilities(&candidates).unwrap()), texts(&part[2]));
            let predicted = policy.predict(&candidates).unwrap();
            fail.check("suffix_length", word, predicted.suffix_length.map(ks_cast::To::to::<u64>), part[3].as_u64());
            fail.check("probability", word, hex(predicted.probability).as_str(), part[4].as_str().unwrap());
            fail.check("prediction version", word, predicted.version.as_str(), part[5].as_str().unwrap());
            let mut next = 6;
            if index % 8 == 0 {
                let want = &part[next];
                next += 1;
                let v2: Vec<Features> = (0..count).map(|suffix| boundary_policy::features(word, &alternative, suffix, source, target, 2)).collect();
                let old: Vec<Features> = (0..count).map(|suffix| boundary_model::features(word, &alternative, suffix, source, target)).collect();
                fail.check("features v2", word, v2.iter().map(features_digest).collect::<Vec<_>>(), texts(&want[0]));
                fail.check("features v1", word, old.iter().map(features_digest).collect::<Vec<_>>(), texts(&want[1]));
                fail.check("probabilities v1", word, hexes(&legacy.probabilities(&old).unwrap()), texts(&want[2]));
                let predicted = legacy.predict(&old).unwrap();
                fail.check("suffix_length v1", word, predicted.suffix_length.map(ks_cast::To::to::<u64>), want[3].as_u64());
                fail.check("probability v1", word, hex(predicted.probability).as_str(), want[4].as_str().unwrap());
            }
            if index % 64 == 0 {
                let want = &part[next];
                next += 1;
                let dumped: Vec<Vec<(String, String)>> = candidates.iter().map(|values| values.iter().map(|(name, value)| (name.clone(), hex(*value))).collect()).collect();
                let wanted: Vec<Vec<(String, String)>> = want
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|values| values.as_array().unwrap().iter().map(|pair| (pair[0].as_str().unwrap().to_string(), pair[1].as_str().unwrap().to_string())).collect())
                    .collect();
                fail.check("features whole", word, dumped, wanted);
            }
            if index % 16 == 0 {
                let want = &part[next];
                let extra = length + 3;
                fail.check("features past the start", word, features_digest(&policy.extract(word, &alternative, extra, source, target)).as_str(), want[0].as_str().unwrap());
                fail.check("legacy past the start", word, features_digest(&boundary_model::features(word, &alternative, extra, source, target)).as_str(), want[1].as_str().unwrap());
            }
        }
    }) {
        fail.finish("boundary_words");
    }
}

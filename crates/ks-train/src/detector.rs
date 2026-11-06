//! The reproducible precision and recall of the EN/RU detector (`evaluate_detector`): the most frequent words of each
//! lexicon, a deterministic sample of each Hunspell dictionary, and the authored cases.
use ks_cast::To as _;
use crate::physical::pair;
use ks_lang::detector::{DecideOptions, DetectionDecision, IntentClassifier, LanguageDetector, LanguageScorer};
use ks_lang::language_model::locale_fallbacks;
use ks_lang::{LanguageModel, Resources};
use ks_text::pynum::py_round_ndigits;
use ks_text::{py_round, pystr};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

const MIN_WORD_CHARACTERS: usize = 3;
const MAX_WORD_CHARACTERS: usize = 18;
const METRIC_DECIMALS: i64 = 6;
const MAX_REPORTED_MISSES: usize = 20;
const MIN_SAMPLE_SIZE: usize = 100;
const GATE_MIN_PRECISION: f64 = 0.999;
const GATE_MIN_SPECIFICITY: f64 = 0.999;
const GATE_MIN_RECALL: f64 = 0.985;
const GATE_MIN_DICTIONARY_RECALL: f64 = 0.90;
const AFFIX_HEADER_SCAN_LINES: usize = 40;
const LOCALES: [&str; 2] = ["en_US", "ru_RU"];
const POSITIVE_TARGETS: [(&str, i64); 12] = [("превет", 1), ("база", 1), ("общих", 1), ("скажи", 1), ("штуку", 1), ("переключал", 1), ("тестируемый", 1), ("документацию", 1), ("helllo", 0), ("worlld", 0), ("debugging", 0), ("reconnected", 0)];
const NEGATIVE_WORDS: [&str; 13] = ["here", "руку", "foobar", "docker", "kubectl", "xfce", "API", "camelCase", "user_name", "abc123", "localhost", "github", "qwerty"];

#[derive(Default)]
struct Metrics {
    language: String,
    samples: usize,
    true_positive: i64,
    false_negative: i64,
    true_negative: i64,
    false_positive: i64,
}

fn ratio(good: i64, other: i64) -> f64 {
    let denominator = good + other;
    if denominator == 0 { 1.0 } else { good.to::<f64>() / denominator.to::<f64>() }
}

impl Metrics {
    fn precision(&self) -> f64 {
        ratio(self.true_positive, self.false_positive)
    }

    fn recall(&self) -> f64 {
        ratio(self.true_positive, self.false_negative)
    }

    fn specificity(&self) -> f64 {
        ratio(self.true_negative, self.false_positive)
    }

    fn payload(&self) -> Value {
        json!({"language": self.language, "samples": self.samples, "true_positive": self.true_positive, "false_negative": self.false_negative, "true_negative": self.true_negative, "false_positive": self.false_positive,
            "precision": py_round_ndigits(self.precision(), METRIC_DECIMALS), "recall": py_round_ndigits(self.recall(), METRIC_DECIMALS), "specificity": py_round_ndigits(self.specificity(), METRIC_DECIMALS)})
    }
}

fn converted(word: &str, group: i64) -> String {
    let (source, target) = if group == 0 { ("us", "ru") } else { ("ru", "us") };
    pair().translate(word, source, target).expect("known layouts")
}

struct Harness {
    detector: LanguageDetector,
}

impl Harness {
    fn decide(&self, original: &str, group: i64, target_group: i64, alternative: String) -> Result<DetectionDecision, String> {
        self.detector.decide(original, &[(target_group, alternative)], group, &DecideOptions::default()).map_err(|error| format!("{error:?}"))
    }

    fn evaluate_words(&self, group: i64, label: String, words: &[String]) -> Result<(Metrics, Vec<Value>), String> {
        let mut metrics = Metrics { language: label, samples: words.len(), ..Metrics::default() };
        let mut misses = Vec::new();
        for word in words {
            let other = converted(word, group);
            let negative = self.decide(word, group, 1 - group, other.clone())?;
            if negative.should_convert {
                metrics.false_positive += 1;
                if misses.len() < MAX_REPORTED_MISSES {
                    misses.push(json!({"kind": "false_positive", "source": word, "replacement": negative.replacement, "reason": negative.reason}));
                }
            } else {
                metrics.true_negative += 1;
            }
            let positive = self.decide(&other, 1 - group, group, word.clone())?;
            if positive.should_convert {
                metrics.true_positive += 1;
            } else {
                metrics.false_negative += 1;
                if misses.len() < MAX_REPORTED_MISSES {
                    misses.push(json!({"kind": "false_negative", "source": other, "replacement": word, "reason": positive.reason}));
                }
            }
        }
        Ok((metrics, misses))
    }
}

fn word_fits(word: &str) -> bool {
    (MIN_WORD_CHARACTERS..=MAX_WORD_CHARACTERS).contains(&word.chars().count()) && pystr::isalpha(word)
}

/// The words of a model in the order its table was filled: the lexicon's lines, then the fallback words it lacks.
fn table_order(model: &LanguageModel, locale: &str) -> Vec<(String, i64)> {
    let mut seen = std::collections::HashSet::new();
    let mut ordered = Vec::new();
    if let Ok(content) = std::fs::read(model.source.split(';').next().unwrap_or("")) {
        let text = String::from_utf8_lossy(&content);
        let mut inside = false;
        for line in text.lines() {
            if line == r"\1-grams:" {
                inside = true;
            } else if line.starts_with('\\') {
                inside = false;
            } else if inside
                && let Some((_, payload)) = line.split_once(' ') {
                    let word = LanguageModel::normalize(payload);
                    if let Some(frequency) = model.frequencies.get(&word)
                        && seen.insert(word.clone())
                    {
                        ordered.push((word, frequency));
                    }
                }
        }
    }
    for word in locale_fallbacks(locale) {
        let word = LanguageModel::normalize(word);
        if let Some(frequency) = model.frequencies.get(&word)
            && seen.insert(word.clone())
        {
            ordered.push((word, frequency));
        }
    }
    let mut rest: Vec<(String, i64)> = model.frequencies.iter().filter(|(word, _)| !seen.contains(*word)).map(|(word, frequency)| (word.to_owned(), frequency)).collect();
    ordered.append(&mut rest);
    ordered
}

/// `hunspell_words`: a deterministic broad-word sample from the active dictionary.
#[must_use]
pub fn hunspell_words(source: &str, sample: usize) -> Vec<String> {
    let dictionary = Path::new(source);
    let Ok(affix) = std::fs::read(dictionary.with_extension("aff")) else { return Vec::new() };
    let header = String::from_utf8_lossy(&affix);
    let encoding = header.lines().take(AFFIX_HEADER_SCAN_LINES).find_map(|line| line.strip_prefix("SET ")).unwrap_or("utf-8").trim().to_owned();
    if !encoding.eq_ignore_ascii_case("utf-8") && !encoding.eq_ignore_ascii_case("utf8") {
        eprintln!("{source}: the dictionary is in {encoding}, read as UTF-8 with replacement");
    }
    let Ok(bytes) = std::fs::read(dictionary) else { return Vec::new() };
    let text = String::from_utf8_lossy(&bytes);
    let candidates: BTreeSet<String> = pystr::splitlines(&text, false).into_iter().skip(1).map(|line| pystr::casefold(pystr::strip(line.split('/').next().unwrap_or("")))).collect();
    let words: Vec<String> = candidates.into_iter().filter(|word| word_fits(word)).collect();
    if words.len() <= sample {
        return words;
    }
    (0..sample).map(|index| words[(py_round(index.to::<f64>() * (words.len() - 1).to::<f64>() / (sample - 1).to::<f64>())).to::<usize>()].clone()).collect()
}

/// The command: the report on standard output; `strict` fails the exit status when a gate does not hold.
pub fn command(resources: &Arc<Resources>, sample: usize, dictionary_sample: usize, strict: bool) -> Result<i32, String> {
    let models: Vec<Arc<LanguageModel>> = LOCALES.iter().map(|locale| Arc::new(resources.language_model(locale, std::iter::empty::<&str>()))).collect();
    let scorers: Vec<(i64, Arc<dyn LanguageScorer>)> = models.iter().enumerate().map(|(group, model)| -> (i64, Arc<dyn LanguageScorer>) { (group.to::<i64>(), model.clone()) }).collect();
    let classifier: Option<Arc<dyn IntentClassifier>> = None;
    let protected = resources.protected_tokens();
    let harness = Harness { detector: LanguageDetector::new(scorers, classifier, protected.clone()).map_err(|error| error.name.to_string())? };
    let (mut directions, mut dictionary_directions, mut misses) = (Vec::new(), Vec::new(), Vec::new());
    for (group, model) in models.iter().enumerate() {
        let locale = LOCALES[group];
        let mut ordered: Vec<(String, i64)> = table_order(model, locale);
        ordered.sort_by_key(|a| std::cmp::Reverse(a.1));
        let words: Vec<String> = ordered.into_iter().map(|entry| entry.0).filter(|word| word_fits(word)).take(sample.max(MIN_SAMPLE_SIZE)).collect();
        let (metrics, found) = harness.evaluate_words(group.to::<i64>(), locale.to_owned(), &words)?;
        directions.push(metrics);
        misses.extend(found);
        if model.speller.available() {
            let sampled = hunspell_words(model.speller.source(), dictionary_sample.max(MIN_SAMPLE_SIZE));
            if !sampled.is_empty() {
                let (metrics, found) = harness.evaluate_words(group.to::<i64>(), format!("{locale} Hunspell"), &sampled)?;
                dictionary_directions.push(metrics);
                misses.extend(found);
            }
        }
    }
    let mut curated_failures = Vec::new();
    for (target, group) in POSITIVE_TARGETS {
        let wrong = converted(target, group);
        let decision = harness.decide(&wrong, 1 - group, group, target.to_owned())?;
        if !decision.should_convert {
            curated_failures.push(json!({"kind": "curated_false_negative", "source": wrong, "replacement": target, "reason": decision.reason}));
        }
    }
    let mut negative: BTreeSet<String> = NEGATIVE_WORDS.iter().map(|word| (*word).to_owned()).collect();
    negative.extend(protected.iter().filter(|token| token.chars().count() >= MIN_WORD_CHARACTERS).map(str::to_owned));
    for word in &negative {
        let group = i64::from(!word.chars().next().is_some_and(|first| first.is_ascii()));
        let decision = harness.decide(word, group, 1 - group, converted(word, group))?;
        if decision.should_convert {
            curated_failures.push(json!({"kind": "curated_false_positive", "source": word, "replacement": decision.replacement, "reason": decision.reason}));
        }
    }
    let counts: HashMap<&str, usize> = HashMap::from([("positive", POSITIVE_TARGETS.len()), ("negative", negative.len())]);
    let describe: serde_json::Map<String, Value> = models.iter().enumerate().map(|(group, model)| (group.to_string(), json!({"locale": LOCALES[group], "words": model.frequencies.len(), "bigrams": model.bigrams.len(), "hunspell": model.speller.available(), "source": model.source}))).collect();
    let payload = json!({"models": describe, "directions": directions.iter().map(Metrics::payload).collect::<Vec<Value>>(), "dictionary_directions": dictionary_directions.iter().map(Metrics::payload).collect::<Vec<Value>>(), "curated_samples": counts["positive"] + counts["negative"], "curated_failures": curated_failures, "sample_failures": misses});
    println!("{}", ks_text::json_dumps(&payload, &ks_text::JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(2)), ..ks_text::JsonOptions::default() }));
    if !strict {
        return Ok(0);
    }
    let holds = |metrics: &Metrics, recall: f64| metrics.precision() >= GATE_MIN_PRECISION && metrics.specificity() >= GATE_MIN_SPECIFICITY && metrics.recall() >= recall;
    let passed = directions.iter().all(|metrics| holds(metrics, GATE_MIN_RECALL)) && dictionary_directions.len() == models.len() && dictionary_directions.iter().all(|metrics| holds(metrics, GATE_MIN_DICTIONARY_RECALL)) && curated_failures.is_empty();
    Ok(i32::from(!passed))
}

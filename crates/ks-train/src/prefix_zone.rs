//! The words of a list in the zone where a prefix model of schema four decides alone (`ks-train prefix-zone-words`):
//! their first `ZONE_KEYS` keys begin no word of the index of their own layout and read as the beginning of a word of the
//! other layout's index, while the word itself is no word of its lexicon, no term of the lists and, read in the other
//! layout, no word there. A model that never saw such a word typed as meant converts it there; the rows these words give,
//! typed as meant only, are what a derived corpus may add to training (`prefix-derive --add`). Only counts are printed.
use ks_cast::To as _;
use crate::prefix::prefix_terms_of;
use crate::prefix_acceptance::listed_counts;
use crate::prefix_words::{Candidate, Class, ClassedCorpus, WordRow, compact, gzip};
use ks_lang::Resources;
use ks_lang::early_switch::PrefixLexicon;
use ks_text::pystr;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

/// The keys the zone is read at: the first key the engine asks the prefix model about.
pub const ZONE_KEYS: usize = 4;
/// The longest token of the short-term class (`Class::ShortTerm`): a shorter one is a term.
const SHORT_TERM_MAXIMUM_CHARACTERS: usize = 4;

/// What `prefix-zone-words` reads.
pub struct ZoneRequest<'a> {
    pub root: &'a Path,
    /// The word corpus whose classes and situations the rows take, and the base it was drawn from.
    pub corpus: &'a Path,
    pub base: &'a Path,
    /// A list of words with their counts (`listed_counts`), the band of counts taken, both ends in, and the layout of its
    /// words: 0 Latin, 1 Cyrillic.
    pub list: &'a Path,
    pub minimum: u64,
    pub maximum: u64,
    pub group: i64,
    pub source: &'a str,
    pub output: &'a Path,
}

/// What the list gave, in numbers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZoneCounts {
    /// The words of the band, those the corpus could hold (letters of the layout, of its lengths, of `ZONE_KEYS` at
    /// least), and of them those no lexicon, list or reading in the other layout knows, and those in the zone.
    pub listed: usize,
    pub admitted: usize,
    pub unknown: usize,
    pub zone: usize,
    /// Latin words: terms and short terms; Cyrillic words: classed as the corpus classes its own.
    pub terms: usize,
    pub short_terms: usize,
    pub rows: usize,
}

/// `prefix-zone-words`: the rows of the zone's words, written once, each typed as meant only.
pub fn zone_words(request: &ZoneRequest<'_>) -> Result<ZoneCounts, String> {
    if request.output.exists() {
        return Err("refusing to overwrite the rows of the zone".to_owned());
    }
    let resources = Resources::new(request.root.join("resources"));
    let classed = ClassedCorpus::read(request.root, &resources, request.corpus, request.base)?;
    let models = classed.classes.models();
    let terms = prefix_terms_of(resources.resource_dir(), models)?;
    // The index a model of schema four runs with: every word of the language models, the supplement's among them.
    let indexes = [resources.prefix_index(&models[0], PrefixLexicon::Held), resources.prefix_index(&models[1], PrefixLexicon::Held)];
    let (own, other) = (request.group.to::<usize>(), (1 - request.group).to::<usize>());
    let (from, to) = [("us", "ru"), ("ru", "us")][own];
    // A word of one layout's letters always has a reading in the other.
    let other_reading = |text: &str| pystr::casefold(&crate::physical::pair().translate(text, from, to).expect("the pair of layouts"));
    let mut counts = ZoneCounts::default();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut candidates: Vec<Candidate> = Vec::new();
    for (word, count) in listed_counts(request.list)? {
        let text = pystr::casefold(&word);
        if !(request.minimum..=request.maximum).contains(&count) || !seen.insert(text.clone()) {
            continue;
        }
        counts.listed += 1;
        if !classed.admits(&text, request.group) || text.chars().count() < ZONE_KEYS {
            continue;
        }
        counts.admitted += 1;
        if classed.classes.known(&text, request.group) || terms.lookup(&text, request.group).exact.is_some() || classed.classes.known(&other_reading(&text), other.to::<i64>()) {
            continue;
        }
        counts.unknown += 1;
        let keys: String = text.chars().take(ZONE_KEYS).collect();
        if indexes[own].completions(&keys).completions != 0 || indexes[other].completions(&other_reading(&keys)).completions == 0 {
            continue;
        }
        counts.zone += 1;
        let class = if own == 0 { [Class::Term, Class::ShortTerm][usize::from(text.chars().count() <= SHORT_TERM_MAXIMUM_CHARACTERS)] } else { classed.classes.class_of(&text, 1) };
        counts.terms += usize::from(class == Class::Term);
        counts.short_terms += usize::from(class == Class::ShortTerm);
        candidates.push(Candidate { text, group: request.group, class, source: request.source.to_owned(), natural: None, hard: class.hard(request.group) });
    }
    // Typed as meant only: a Cyrillic word of the zone typed in the other layout is the very prefix the model should not
    // convert for it.
    let rows: Vec<WordRow> = classed.rows_of(&candidates).into_iter().map(|row| WordRow { paired: false, ..row }).collect();
    counts.rows = rows.len();
    let lines: String = rows.iter().map(|row| ks_text::json::dumps(row, &compact()) + "\n").collect();
    std::fs::create_dir_all(request.output.parent().unwrap_or(Path::new("."))).map_err(|error| error.to_string())?;
    std::fs::write(request.output, gzip(lines.as_bytes())?).map_err(|error| format!("{}: {error}", request.output.display()))?;
    Ok(counts)
}

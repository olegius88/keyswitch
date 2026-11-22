//! The term lists the program loads, read as evidence for the prefix model (schema four): the identifier lexicon, the
//! protected tokens and the two columns of the term table, with the counts the table has.
//!
//! A list holds tokens typed as meant; a token whose keys read as a word of the other layout is that word typed in the
//! wrong layout as often as a term (`ghbdtn`, `tcnm` in the Latin column), and a list that kept it would teach the model to
//! keep the commonest layout mistakes. Such tokens are left out, the same way where the model is trained and where it runs.
use crate::constants::PREFIX_TERM_MIN_CHARACTERS;
use crate::flat_table::CountTable;
use ks_text::layouts::LayoutPair;
use ks_text::pystr;

/// What the lists say of a term.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TermEvidence {
    /// The count of the term table; none in a list without counts.
    pub count: u64,
    /// In the identifier lexicon: names of commands and packages, without counts.
    pub identifier: bool,
    /// A protected token.
    pub protected: bool,
}

impl TermEvidence {
    fn merge(&mut self, other: TermEvidence) {
        self.count = self.count.max(other.count);
        self.identifier |= other.identifier;
        self.protected |= other.protected;
    }
}

/// What the lists say of a reading of the keys: of the term it is, and of the longer terms it begins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TermMatch {
    pub exact: Option<TermEvidence>,
    /// Over every longer term the reading begins: the largest count, and which lists hold one.
    pub begins: Option<TermEvidence>,
}

/// The terms of each script, sorted, each once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrefixTerms {
    latin: Vec<(String, TermEvidence)>,
    cyrillic: Vec<(String, TermEvidence)>,
}

fn sorted(entries: Vec<(String, TermEvidence)>) -> Vec<(String, TermEvidence)> {
    let mut merged: std::collections::BTreeMap<String, TermEvidence> = std::collections::BTreeMap::new();
    for (term, evidence) in entries {
        merged.entry(term).or_default().merge(evidence);
    }
    merged.into_iter().collect()
}

impl PrefixTerms {
    /// The lists, case-folded, without the tokens whose keys read as a word of the other layout's lexicon
    /// (`known_word(reading, group)`).
    pub fn build<'a>(
        identifiers: impl Iterator<Item = &'a str>, protected: impl Iterator<Item = &'a str>, latin: &CountTable, cyrillic: &CountTable, known_word: impl Fn(&str, i64) -> bool,
    ) -> PrefixTerms {
        let pair = LayoutPair::default();
        let read_as = |term: &str, from: &str, to: &str| pair.translate(term, from, to).unwrap_or_default();
        let mut latin_terms: Vec<(String, TermEvidence)> = Vec::new();
        latin_terms.extend(identifiers.map(|term| (pystr::casefold(term), TermEvidence { identifier: true, ..TermEvidence::default() })));
        latin_terms.extend(protected.map(|term| (pystr::casefold(term), TermEvidence { protected: true, ..TermEvidence::default() })));
        latin_terms.extend(latin.words().map(|term| (pystr::casefold(term), TermEvidence { count: latin.count(term), ..TermEvidence::default() })));
        latin_terms.retain(|(term, _)| !known_word(&read_as(term, "us", "ru"), 1));
        let mut cyrillic_terms: Vec<(String, TermEvidence)> = cyrillic.words().map(|term| (pystr::casefold(term), TermEvidence { count: cyrillic.count(term), ..TermEvidence::default() })).collect();
        cyrillic_terms.retain(|(term, _)| !known_word(&read_as(term, "ru", "us"), 0));
        PrefixTerms { latin: sorted(latin_terms), cyrillic: sorted(cyrillic_terms) }
    }

    /// The lists without these tokens, given case-folded: what a model reads of a token no list holds.
    #[must_use]
    pub fn without(&self, tokens: &std::collections::BTreeSet<String>) -> PrefixTerms {
        let kept = |terms: &[(String, TermEvidence)]| terms.iter().filter(|(term, _)| !tokens.contains(term)).cloned().collect();
        PrefixTerms { latin: kept(&self.latin), cyrillic: kept(&self.cyrillic) }
    }

    /// How many terms each script holds.
    #[must_use]
    pub fn len(&self) -> [usize; 2] {
        [self.latin.len(), self.cyrillic.len()]
    }

    /// What the lists of a reading's script say of it, from `PREFIX_TERM_MIN_CHARACTERS` letters: shorter readings begin
    /// too many terms to say anything.
    #[must_use]
    pub fn lookup(&self, reading: &str, group: i64) -> TermMatch {
        let key = pystr::casefold(reading);
        if key.chars().count() < PREFIX_TERM_MIN_CHARACTERS {
            return TermMatch::default();
        }
        let terms = if group == 1 { &self.cyrillic } else { &self.latin };
        let start = terms.partition_point(|(term, _)| term.as_str() < key.as_str());
        let mut found = TermMatch::default();
        for (term, evidence) in terms[start..].iter().take_while(|(term, _)| term.starts_with(&key)) {
            if *term == key {
                found.exact = Some(*evidence);
            } else {
                found.begins.get_or_insert_with(TermEvidence::default).merge(*evidence);
            }
        }
        found
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn evidence_of_several_lists_is_merged() {
        let mut evidence = TermEvidence { count: 3, identifier: false, protected: true };
        evidence.merge(TermEvidence { count: 2, identifier: true, protected: false });
        assert_eq!(evidence, TermEvidence { count: 3, identifier: true, protected: true });
        let merged = sorted(vec![("b".to_owned(), TermEvidence::default()), ("a".to_owned(), evidence), ("b".to_owned(), TermEvidence { count: 1, ..TermEvidence::default() })]);
        assert_eq!(merged.iter().map(|(term, item)| (term.as_str(), item.count)).collect::<Vec<_>>(), [("a", 3), ("b", 1)]);
    }
}

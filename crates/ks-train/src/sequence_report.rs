//! The report of a sequence evaluation and the public blocks of a receipt, as types. A report names its pairs, profiles and
//! counts, and a case says what was typed and what came of it; every field is read and written by name, and the JSON that
//! `ks_text::json::dumps` writes of these types is the bytes the report has always had (keys are sorted when it is written).
use crate::artifact::typed;
use crate::evaluate::Earliness;
use crate::replay::{Outcome, Plan};
use ks_cast::To as _;
use serde::{Deserialize, Serialize};
use ks_text::json::Kept;
use std::collections::BTreeMap;

/// One correction the editor received, as a case lists it. From protocol 5 on it names the kind of plan that made it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrectionRecord {
    pub event: usize,
    pub before: String,
    pub after: String,
    pub source_group: i64,
    pub target_group: i64,
    pub actual_group: i64,
    pub caret_before: usize,
    pub caret_after: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// The wrong key a plan typed on purpose: where, what it was meant to be and what the layout made of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intervention {
    pub event: i64,
    pub expected: String,
    pub observed: String,
    pub source_group: i64,
    pub target_group: i64,
}

/// One plan typed through the engine, and how it came out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub identifier: String,
    pub document: String,
    pub group: Option<i64>,
    pub initially_wrong: bool,
    pub expected: String,
    pub trimmed_left: i64,
    pub trimmed_right: i64,
    /// The letters of the word a first plan types first; absent from the cases of protocol 4's plans.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_letters: Option<i64>,
    pub mode: String,
    pub actual: String,
    pub exact: bool,
    pub length_mismatch: bool,
    pub final_group: i64,
    pub expected_final_group: i64,
    pub final_layout_matches: bool,
    pub corrections: Vec<CorrectionRecord>,
    pub intervention: Option<Intervention>,
    pub error: Option<String>,
}

impl Case {
    /// The case of a plan and its outcome.
    #[must_use]
    pub fn new(plan: &Plan, outcome: &Outcome) -> Case {
        Case {
            identifier: plan.identifier.clone(),
            document: plan.document.clone(),
            group: plan.group,
            initially_wrong: plan.initially_wrong,
            expected: plan.expected.clone(),
            trimmed_left: plan.trimmed_left.to::<i64>(),
            trimmed_right: plan.trimmed_right.to::<i64>(),
            focus_letters: plan.focus_letters.map(ks_cast::To::to::<i64>),
            mode: outcome.mode.as_str().to_owned(),
            actual: outcome.actual.clone(),
            exact: outcome.exact,
            length_mismatch: outcome.length_mismatch,
            final_group: outcome.final_group,
            expected_final_group: outcome.expected_final_group,
            final_layout_matches: outcome.final_layout_matches,
            corrections: outcome
                .corrections
                .iter()
                .map(|item| CorrectionRecord {
                    event: item.event,
                    before: item.before.clone(),
                    after: item.after.clone(),
                    source_group: i64::from(item.source_group),
                    target_group: i64::from(item.target_group),
                    actual_group: i64::from(item.actual_group),
                    caret_before: item.caret_before,
                    caret_after: item.caret_after,
                    kind: Some(item.kind.to_owned()),
                })
                .collect(),
            intervention: outcome.intervention.clone(),
            error: outcome.error.clone(),
        }
    }
}

/// The counts of one pair's replay of plans.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairCounts {
    pub rows: i64,
    pub initially_correct: i64,
    pub initially_wrong: i64,
    pub preserved_correct: i64,
    pub exactly_restored: i64,
    pub correct_text_corruptions: i64,
    pub length_mismatches: i64,
    pub injections: i64,
    pub execution_errors: i64,
    pub correction_layout_mismatches: i64,
    pub final_layout_mismatches: i64,
}

impl PairCounts {
    /// Each count under its name, in the order a report lists them.
    #[must_use]
    pub fn entries(&self) -> [(&'static str, i64); 11] {
        [
            ("rows", self.rows),
            ("initially_correct", self.initially_correct),
            ("initially_wrong", self.initially_wrong),
            ("preserved_correct", self.preserved_correct),
            ("exactly_restored", self.exactly_restored),
            ("correct_text_corruptions", self.correct_text_corruptions),
            ("length_mismatches", self.length_mismatches),
            ("injections", self.injections),
            ("execution_errors", self.execution_errors),
            ("correction_layout_mismatches", self.correction_layout_mismatches),
            ("final_layout_mismatches", self.final_layout_mismatches),
        ]
    }

    /// The counts a list of cases gives, as the check recomputes them.
    #[must_use]
    pub fn of_cases(cases: &[Case]) -> PairCounts {
        let count = |test: &dyn Fn(&Case) -> bool| cases.iter().filter(|case| test(case)).count().to::<i64>();
        PairCounts {
            rows: cases.len().to::<i64>(),
            initially_correct: count(&|case| !case.initially_wrong),
            initially_wrong: count(&|case| case.initially_wrong),
            preserved_correct: count(&|case| !case.initially_wrong && case.exact),
            exactly_restored: count(&|case| case.initially_wrong && case.exact),
            correct_text_corruptions: count(&|case| !case.initially_wrong && !case.exact),
            length_mismatches: count(&|case| case.length_mismatch),
            injections: cases.iter().map(|case| case.corrections.len().to::<i64>()).sum(),
            execution_errors: count(&|case| case.error.is_some()),
            correction_layout_mismatches: cases.iter().map(|case| case.corrections.iter().filter(|item| item.actual_group != item.target_group).count().to::<i64>()).sum(),
            final_layout_mismatches: count(&|case| !case.final_layout_matches),
        }
    }
}

/// The gates of a block of protocol 4's plans, each held or not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gates {
    pub correct_text_preserved: bool,
    pub length_preserved: bool,
    pub net_restorations_at_least_baseline: bool,
    pub enough_documents_us: bool,
    pub enough_documents_ru: bool,
    pub execution_succeeded: bool,
    pub correction_layouts_match: bool,
}

impl Gates {
    /// Whether every gate holds.
    #[must_use]
    pub fn all_held(&self) -> bool {
        [
            self.correct_text_preserved,
            self.length_preserved,
            self.net_restorations_at_least_baseline,
            self.enough_documents_us,
            self.enough_documents_ru,
            self.execution_succeeded,
            self.correction_layouts_match,
        ]
        .into_iter()
        .all(|held| held)
    }
}

/// The gates of the first plans (protocol 5): restorations net of corruptions at least the baseline's, no more
/// corruptions than the baseline's, no length changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirstGates {
    pub net_restorations_at_least_baseline: bool,
    pub correct_text_preserved: bool,
    pub length_preserved: bool,
}

impl FirstGates {
    /// The gates from the counts of the candidate pair and of the baseline pair.
    #[must_use]
    pub fn of(candidate: &PairCounts, baseline: &PairCounts) -> FirstGates {
        let net = |counts: &PairCounts| counts.exactly_restored - counts.correct_text_corruptions;
        FirstGates {
            net_restorations_at_least_baseline: net(candidate) >= net(baseline),
            correct_text_preserved: candidate.correct_text_corruptions <= baseline.correct_text_corruptions,
            length_preserved: candidate.length_mismatches == 0,
        }
    }

    /// Whether every gate holds.
    #[must_use]
    pub fn all_held(&self) -> bool {
        self.net_restorations_at_least_baseline && self.correct_text_preserved && self.length_preserved
    }
}

/// A value of each of the two pairs a block is gated by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gated<T> {
    pub baseline: T,
    pub candidate: T,
}

/// A value of each of the pairs the first plans of a report name: the two gated and the installed reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ByPair<T> {
    pub baseline: T,
    pub candidate: T,
    pub installed: T,
}

impl<T> ByPair<T> {
    /// Each pair's value under its name.
    #[must_use]
    pub fn each(&self) -> [(&'static str, &T); 3] {
        [("baseline", &self.baseline), ("candidate", &self.candidate), ("installed", &self.installed)]
    }
}

/// A value of each of the pairs the first plans of a report hold: the three a receipt publishes, and the candidate's context
/// model with the baseline's prefix model, which no gate and no receipt names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllPairs<T> {
    pub baseline: T,
    pub candidate: T,
    pub candidate_context_baseline_prefix: T,
    pub installed: T,
}

impl<T> AllPairs<T> {
    /// A value made for each pair, by its name.
    pub fn build<E>(mut make: impl FnMut(&str) -> Result<T, E>) -> Result<AllPairs<T>, E> {
        Ok(AllPairs { baseline: make("baseline")?, candidate: make("candidate")?, candidate_context_baseline_prefix: make("candidate_context_baseline_prefix")?, installed: make("installed")? })
    }

    /// The value of a pair by its name.
    #[must_use]
    pub fn named(&self, pair: &str) -> Option<&T> {
        match pair {
            "baseline" => Some(&self.baseline),
            "candidate" => Some(&self.candidate),
            "candidate_context_baseline_prefix" => Some(&self.candidate_context_baseline_prefix),
            "installed" => Some(&self.installed),
            _ => None,
        }
    }

    /// A value made of each pair's value.
    pub fn map<U>(&self, make: impl Fn(&T) -> U) -> AllPairs<U> {
        AllPairs { baseline: make(&self.baseline), candidate: make(&self.candidate), candidate_context_baseline_prefix: make(&self.candidate_context_baseline_prefix), installed: make(&self.installed) }
    }

    /// The pairs a receipt publishes, each under its name.
    #[must_use]
    pub fn published(&self) -> [(&'static str, &T); 3] {
        [("baseline", &self.baseline), ("candidate", &self.candidate), ("installed", &self.installed)]
    }

    /// The pairs a receipt publishes, each under its name, to change.
    pub fn published_mut(&mut self) -> [(&'static str, &mut T); 3] {
        [("baseline", &mut self.baseline), ("candidate", &mut self.candidate), ("installed", &mut self.installed)]
    }
}

impl<T: Clone> AllPairs<T> {
    /// What a receipt publishes of it: the three pairs it names.
    #[must_use]
    pub fn public(&self) -> ByPair<T> {
        ByPair { baseline: self.baseline.clone(), candidate: self.candidate.clone(), installed: self.installed.clone() }
    }
}

/// A pair's counts and the cases they were counted from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Block {
    pub counts: PairCounts,
    pub cases: Vec<Case>,
}

/// The first plans of a profile (protocol 5), in the default mode: the counts, the earliness and the cases of each pair,
/// and the gates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirstBlock {
    pub counts: AllPairs<PairCounts>,
    pub earliness: AllPairs<Earliness>,
    pub cases: AllPairs<Vec<Case>>,
    pub gates: FirstGates,
}

/// What a receipt publishes of the first plans: no case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicFirst {
    pub counts: ByPair<PairCounts>,
    pub earliness: ByPair<Earliness>,
    pub gates: FirstGates,
}

/// The control without early switching: the gated pairs' counts and cases, and the gates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EarlyOff {
    pub counts: Gated<PairCounts>,
    pub cases: Gated<Vec<Case>>,
    pub gates: Gates,
}

/// One lexical profile, in the default mode, with its control.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub counts: Gated<PairCounts>,
    pub cases: Gated<Vec<Case>>,
    pub gates: Gates,
    /// The pairs that are in no gate but are measured: the candidate's context model with the baseline's prefix model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ablation: Option<BTreeMap<String, Block>>,
    /// The tree's installed pair, replayed as a reference (protocol 5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<Block>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<FirstBlock>,
    pub early_off: EarlyOff,
}

impl Profile {
    /// The protocol-4 cases of a pair in the default mode: those the earliness of its first plans counts false early switches
    /// in. None for a pair the profile has no cases of.
    #[must_use]
    pub fn sequence_cases(&self, pair: &str) -> Option<&[Case]> {
        match pair {
            "installed" => self.installed.as_ref().map(|block| block.cases.as_slice()),
            "baseline" => Some(&self.cases.baseline),
            "candidate" => Some(&self.cases.candidate),
            other => self.ablation.as_ref().and_then(|pairs| pairs.get(other)).map(|block| block.cases.as_slice()),
        }
    }
}

/// The profiles of a report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profiles {
    pub portable: Profile,
    pub reference_hunspell: Profile,
}

impl Profiles {
    /// Each profile under its name, in the order of `PROFILES`.
    #[must_use]
    pub fn each(&self) -> [(&'static str, &Profile); 2] {
        [("portable", &self.portable), ("reference_hunspell", &self.reference_hunspell)]
    }

    /// Each profile under its name, to change.
    pub fn each_mut(&mut self) -> [(&'static str, &mut Profile); 2] {
        [("portable", &mut self.portable), ("reference_hunspell", &mut self.reference_hunspell)]
    }
}

/// How many distinct documents each group of plans came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Documents {
    #[serde(rename = "0")]
    pub us: i64,
    #[serde(rename = "1")]
    pub ru: i64,
    #[serde(rename = "None")]
    pub none: i64,
}

/// A row protocol 4 could not type: its identifier and the glyphs that had no key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unsupported {
    pub identifier: String,
    pub codepoints: Vec<String>,
}

/// What a sequence evaluation selected to replay.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub source_ids: Vec<String>,
    pub selected_rows: i64,
    pub replayed_documents: Documents,
    pub trimmed_rows: i64,
    pub unsupported: Vec<Unsupported>,
    /// The rows protocol 4 types whose focus opens otherwise than with a letter of its layout: no first plans (protocol 5).
    #[serde(default)]
    pub first_ineligible: Vec<String>,
}

/// The installed pair a report replayed as its reference, by the digests of its two models.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledReference {
    pub context_sha256: String,
    pub prefix_sha256: String,
}

/// The report of a sequence evaluation, as it is written. A report of a replay that did not run holds an
/// `execution_error` and no selection or profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema_version: i64,
    pub split: String,
    pub identity: Kept,
    pub protocol: Kept,
    pub gate_policy: Kept,
    pub promotion_passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_reference: Option<InstalledReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<Selection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profiles: Option<Profiles>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// The one thing of a protocol block that is read as its type: the version it names.
#[derive(Deserialize)]
struct Versioned {
    version: i64,
}

/// The version a protocol block names, when it names one.
#[must_use]
pub fn protocol_version<S: Serialize + ?Sized>(protocol: &S) -> Option<i64> {
    typed::<Versioned, _>(protocol).ok().map(|named| named.version)
}

/// Whether a value, as a document of any shape, is the very one the report carries.
fn carries<S: Serialize + ?Sized>(kept: &Kept, value: &S) -> bool {
    typed::<Kept, _>(value).is_ok_and(|other| other == *kept)
}

impl Report {
    /// A document the report carries as it is, from the tree or the record that holds it.
    pub fn carry<S: Serialize + ?Sized>(value: &S) -> Result<Kept, String> {
        typed(value)
    }

    /// The version of the sequence protocol the report names, when it names one.
    #[must_use]
    pub fn protocol_version(&self) -> Option<i64> {
        protocol_version(&self.protocol)
    }
}

/// What a report says first: which identity, protocol and gate policy it was run against, and that it passed. It is read
/// apart from the rest, which is checked before it is read as its type.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ReportHead {
    pub schema_version: i64,
    pub split: String,
    pub identity: Kept,
    pub protocol: Kept,
    pub gate_policy: Kept,
    pub promotion_passed: bool,
    /// What a report of a replay that did not run says; a report that passed has none.
    #[serde(default)]
    pub execution_error: Option<Kept>,
}

impl ReportHead {
    /// The version of the sequence protocol the report names, when it names one.
    #[must_use]
    pub fn protocol_version(&self) -> Option<i64> {
        protocol_version(&self.protocol)
    }

    /// Whether the report names this identity, this protocol and this gate policy: what its test was run against.
    pub fn answers<I: Serialize + ?Sized, P: Serialize + ?Sized, G: Serialize + ?Sized>(&self, identity: &I, protocol: &P, gate_policy: &G) -> bool {
        carries(&self.identity, identity) && carries(&self.protocol, protocol) && carries(&self.gate_policy, gate_policy)
    }
}

/// The `test` block of a receipt: what was selected, and each profile's counts, gates and, from protocol 5, first plans.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicTest {
    pub documents: Documents,
    pub selected_rows: i64,
    pub trimmed_rows: i64,
    pub unsupported_rows: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_ineligible_rows: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_reference: Option<InstalledReference>,
    pub profiles: PublicProfiles,
}

/// A profile of a receipt's test: the default mode's counts and gates, its control's, and the first plans of protocol 5.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicProfile {
    pub counts: Gated<PairCounts>,
    pub gates: Gates,
    pub early_off: PublicEarlyOff,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<PublicFirst>,
}

/// A receipt's control without early switching.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicEarlyOff {
    pub counts: Gated<PairCounts>,
    pub gates: Gates,
}

/// The profiles of a receipt's test.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicProfiles {
    pub portable: PublicProfile,
    pub reference_hunspell: PublicProfile,
}

impl PublicProfiles {
    /// A profile by its name.
    #[must_use]
    pub fn named(&self, name: &str) -> Option<&PublicProfile> {
        match name {
            "portable" => Some(&self.portable),
            "reference_hunspell" => Some(&self.reference_hunspell),
            _ => None,
        }
    }

    /// Each profile under its name, to change.
    pub fn each_mut(&mut self) -> [(&'static str, &mut PublicProfile); 2] {
        [("portable", &mut self.portable), ("reference_hunspell", &mut self.reference_hunspell)]
    }
}

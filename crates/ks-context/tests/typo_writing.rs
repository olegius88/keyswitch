//! The written list confirms the subtitle list's choice: a word written text counts often enough, chosen as clearly there.

use ks_context::context_model::TermTables;
use ks_context::typo_correction::{
    Edit, Operation, Reason, TypoChoice, TypoCorrector, TypoJudgement, TypoPolicy, WrittenCandidate, confirm_in_writing, written_evidence, written_section,
};
use std::path::PathBuf;

fn resources() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources"))
}

fn slip(operation: Operation) -> Edit {
    Edit { operation, position: 0, letter: None }
}

fn rival(word: &str, count: u64, operation: Operation) -> WrittenCandidate {
    WrittenCandidate { word: word.to_owned(), count, edits: vec![slip(operation)] }
}

/// A judgement of the subtitle list that chose `word`.
fn chose(word: &str) -> TypoJudgement {
    TypoJudgement {
        reason: Reason::Corrected,
        choice: Some(TypoChoice { word: word.to_owned(), edit: slip(Operation::Substitution), weight: 80, candidates: 2, margin: 2.0 }),
        candidates: 2,
        written: None,
    }
}

#[test]
fn each_language_has_its_section_of_the_written_list() {
    assert_eq!(written_section("ru_RU"), Some("russian"));
    assert_eq!(written_section("en_US"), Some("english"));
    assert_eq!(written_section("de_DE"), None);
}

#[test]
fn written_text_confirms_a_choice_only_when_it_counts_it_and_prefers_it_clearly() {
    let policy = TypoPolicy::default();
    // A judgement that corrects nothing is not asked about.
    let left = TypoJudgement { reason: Reason::Ambiguous, choice: None, candidates: 3, written: None };
    assert_eq!(confirm_in_writing(&policy, left.clone(), &[]), left);
    // Not counted at all, or too seldom: no word of written text.
    let none = confirm_in_writing(&policy, chose("слово"), &[rival("слава", 1000, Operation::Substitution)]);
    assert_eq!(none.reason, Reason::Unwritten);
    assert_eq!(none.written.map(|written| written.count), Some(0));
    let rare = confirm_in_writing(&policy, chose("Слово"), &[rival("слово", policy.written_min_count - 1, Operation::Substitution)]);
    assert_eq!(rare.reason, Reason::Unwritten, "the choice is compared as written, in lowercase");
    // Another word one slip away that written text counts far more often: the lists disagree.
    let contested = confirm_in_writing(&policy, chose("слово"), &[rival("слово", 100, Operation::Substitution), rival("слава", 1000, Operation::Substitution)]);
    assert_eq!(contested.reason, Reason::Contested);
    assert!(contested.written.is_some_and(|written| written.margin < 0.0));
    // Preferred, but not clearly: a near tie.
    let near = confirm_in_writing(&policy, chose("слово"), &[rival("слово", 200, Operation::Substitution), rival("слава", 100, Operation::Substitution)]);
    assert_eq!(near.reason, Reason::Contested);
    // Clearly, or alone.
    let clear = confirm_in_writing(&policy, chose("слово"), &[rival("слово", 1000, Operation::Substitution), rival("слава", 10, Operation::Substitution)]);
    assert_eq!(clear.reason, Reason::Corrected);
    let alone = written_evidence(&policy, "слово", &[rival("слово", 50, Operation::Deletion)]);
    assert!(alone.margin.is_infinite() && alone.margin > 0.0 && alone.count == 50);
}

#[test]
fn the_written_list_is_asked_about_every_word_one_slip_away_in_both_spellings_of_yo() {
    let corrector = TypoCorrector::packaged(&resources(), &["ru_RU"]).expect("the packaged list");
    let counted = |form: &str| match form {
        "еще" => 7,
        _ => 0,
    };
    let rivals = corrector.written_candidates("ещ", "ru_RU", &counted);
    let count = |word: &str| rivals.iter().find(|rival| rival.word == word).map(|rival| rival.count);
    assert_eq!(count("ещё"), Some(7), "written as `еще`");
    assert_eq!(count("еще"), Some(7));
    assert_eq!(count("ещь"), None, "a form written text does not count is no rival");
}

#[test]
fn a_word_is_judged_by_the_subtitle_list_and_then_by_written_text() {
    let root = resources();
    let corrector = TypoCorrector::packaged(&root, &["en_US", "ru_RU"]).expect("the packaged lists");
    let terms = TermTables::load(&root.join("models")).expect("the term tables");
    let written = |form: &str| terms.term("russian").count(form);
    let known = |_: &str| false;
    let judge = |word: &str| corrector.judge(word, "ru_RU", &[], false, &known, &written);
    assert_eq!(judge("мржно").reason, Reason::Corrected);
    assert_eq!(judge("формар").reason, Reason::Unwritten, "`форман` is a name films say");
    assert_eq!(judge("сьрока").reason, Reason::Contested, "written text says `строка`, films `сорока`");
    assert_eq!(judge("можно").reason, Reason::Occurs, "a word the subtitle list holds is left before written text is asked");
    assert_eq!(corrector.judge_counted("формар", "ru_RU", &[], false, &known).reason, Reason::Corrected);
}

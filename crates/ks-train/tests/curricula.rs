//! The curricula of a fit one by one, with every setting a recipe can give them, on rows of a small corpus.
#![expect(clippy::disallowed_types, clippy::disallowed_macros, clippy::disallowed_methods, reason = "temporary: these tests still build and read their JSON as untyped trees; step 7 of the strict-json plan types them and removes this")]
mod support;

use ks_context::context_model::ContextAction;
use ks_lang::Resources;
use ks_train::corpus::{CorpusRow, load_rows};
use ks_train::curricula::{self as c};
use ks_train::heads::{HEAD_PREFIXES, OWN_EPOCH_HEADS, own_curriculum};
use ks_train::lookahead;
use ks_train::rows::{ActionRow, public_field};
use ks_train::tools::{Lexicons, Tools, reference_models};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};
use support::{recipe, row, scratch, tree, write_split};

/// Rows of a small corpus in the shapes the curricula look for.
const RUSSIAN: &[&[&str]] = &[
    &["сегодня", "хорошая", "погода", "и", "дом", "стоит", "на", "горе"],
    &["он", "а", "мы", "в", "лесу", "гуляем", "весело", "вечером"],
    &["я", "пишу", "письмо", "с", "другом", "у", "моря", "о", "жизни"],
    &["Москва", "большой", "город", "к", "нему", "едут", "люди"],
    &["работа", "программа", "система", "решение", "задача", "результат", "время", "дорога"],
];
const ENGLISH: &[&[&str]] = &[
    &["the", "weather", "is", "good", "today", "and", "people", "are", "working"],
    &["I", "think", "a", "house", "stands", "on", "the", "hill", "before", "water"],
    &["she", "said", "that", "letters", "should", "come", "through", "here"],
];

fn rows() -> Vec<CorpusRow> {
    let folder = scratch("curricula-corpus");
    let mut made = Vec::new();
    let mut index = 0;
    for (group, sentences) in [(1, RUSSIAN), (0, ENGLISH)] {
        for sentence in sentences {
            for at in 0..sentence.len() {
                made.push(row("train", index, group, sentence, at));
                index += 1;
            }
        }
    }
    // Those a curriculum has no use for: no group, no representation, words of the other script.
    let mut odd = row("train", index, 1, &["word", "слово"], 0);
    odd["layout_representable"] = json!(false);
    made.push(odd);
    let mut no_group = row("train", index + 1, 0, &["слово", "дело"], 0);
    no_group["group"] = Value::Null;
    made.push(no_group);
    made.push(row("train", index + 2, 1, &["word", "слово"], 0));
    made.push(row("train", index + 3, 1, &["word", "слово"], 1));
    made.push(row("train", index + 4, 1, &["word,", "сло-во"], 1));
    // Tokens joined by a dash, in each script, of one word twice, more than once and of no script; and words with signs between.
    let mut next = index + 5;
    let more: [(i64, &[&str], usize); 16] = [
        (0, &["the", "go-on", "people", "are"], 3),
        (0, &["the", "people", "well-known", "are", "here"], 4),
        (1, &["это", "очень", "кто-то", "важно", "сегодня"], 0),
        (1, &["это", "вечером", "кто-то", "ушёл", "домой"], 0),
        (1, &["там", "сидит", "мама-мама", "и", "ждёт"], 0),
        (0, &["the", "people", "well-known", "are", "here"], 0),
        (0, &["many", "people", "mongo-db", "are", "here"], 0),
        (1, &["это", "большая", "работа", "ьщтпщ", "сегодня"], 3),
        (1, &["это", "большая", "работа", "цуферук", "сегодня"], 3),
        (1, &["дом,", "стоит", "на", "горе", "а", "ветер", "дует"], 5),
        (0, &["house,", "stands", "on", "1", "hill", "a", "wind", "blows"], 6),
        (0, &["we", "said", "a", "1", "wonderfullylongwordwithouthyphenswonderfullylongwordwithouthyphens", "here"], 4),
        (0, &["we", "said", "a", "wonderfullylongwordwithouthyphenswonderfullylongwordwithouthyphens", "here"], 4),
        (1, &["там", "ñandu-ñandu", "сидит", "и", "ждёт"], 0),
        (1, &["мы", "и", "дом", "и", "сад"], 1),
        (1, &["это", "color-дом", "factory-дом", "очень", "важно"], 4),
    ];
    for (group, words, at) in more {
        made.push(row("train", next, group, words, at));
        next += 1;
    }
    // One token in one document twice.
    for at in [0, 1] {
        let mut twice = row("train", next, 1, &["кто-то", "сказал", "ещё"], at);
        twice["document"] = json!("one document");
        made.push(twice);
        next += 1;
    }
    let mut manifest = json!({"splits": {}});
    write_split(&folder, "train", &made, &mut manifest);
    std::fs::write(folder.join("manifest.json"), serde_json::to_vec(&manifest).expect("json")).expect("a manifest");
    load_rows(&folder, "train").expect("the rows")
}

static TOOLS: LazyLock<Tools> = LazyLock::new(|| {
    let root = tree();
    let resources: Arc<Resources> = Resources::new(root.join("resources"));
    let portable = reference_models(&root, &resources, false).expect("the lexicons");
    let refused: BTreeSet<String> = ["work", "ьфьф", "ctujlyz"].iter().map(|word| (*word).to_owned()).collect();
    Tools::new(&root, resources, portable, refused).expect("the tools")
});

static ROWS: LazyLock<Vec<CorpusRow>> = LazyLock::new(rows);

fn options(name: &str) -> Value {
    recipe()[name].clone()
}

fn reported(curriculum: &(Vec<ActionRow>, Value)) -> usize {
    assert!(curriculum.1.is_object(), "{}", curriculum.1);
    curriculum.0.len()
}

fn with(mut value: Value, changes: &[(&str, Value)]) -> Value {
    for (name, change) in changes {
        value[*name] = change.clone();
    }
    value
}

const SPLITS: [&str; 3] = ["train", "development", "calibration"];

#[test]
fn a_curriculum_set_to_nothing_or_given_nothing_gives_no_frames() {
    let (tools, rows) = (&*TOOLS, &*ROWS);
    let models = &tools.portable;
    let none: &[CorpusRow] = &[];
    assert_eq!(reported(&c::capital_citation_curriculum(tools, none, &options("capital_citation_curriculum"), models)), 0);
    assert_eq!(reported(&c::english_capital_curriculum(tools, rows, &with(options("english_capital_curriculum"), &[("frames_per_word", json!(0))]), models)), 0);
    assert_eq!(reported(&c::abbreviation_curriculum(tools, "train", rows, &with(options("abbreviation_curriculum"), &[("frames_per_form", json!(0))]))), 0);
    assert_eq!(reported(&c::latin_abbreviation_curriculum(tools, "train", rows, &with(options("latin_abbreviation_curriculum"), &[("frames_per_form", json!(0))]))), 0);
    assert_eq!(reported(&c::unknown_token_curriculum(tools, "train", rows, &with(options("unknown_token_curriculum"), &[("item_modulus", json!(0))]))), 0);
    assert_eq!(reported(&c::joined_token_curriculum(tools, "train", rows, &with(options("joined_token_curriculum"), &[("item_modulus", json!(0))]))), 0);
    assert_eq!(reported(&c::term_curriculum(tools, "train", rows, &with(options("term_curriculum"), &[("item_modulus", json!(0))]))), 0);
    assert_eq!(reported(&c::first_curriculum(tools, "train", &with(options("first_curriculum"), &[("enabled", json!(false))]))), 0);
    assert_eq!(reported(&c::first_curriculum(tools, "train", &json!({}))), 0);
    assert_eq!(reported(&c::brief_curriculum(tools, "train", &with(options("brief_curriculum"), &[("minimum_count", json!(0))]))), 0);
    assert_eq!(reported(&c::kept_neighbour_curriculum(tools, rows, &with(options("kept_neighbour_curriculum"), &[("natural_frames", json!(0)), ("term_frames", json!(0)), ("abbreviation_frames", json!(0)), ("typo_frames", json!(0))]), models)), 0);
    assert_eq!(reported(&c::counted_token_curriculum(tools, none, &with(options("counted_token_curriculum"), &[("term_frames", json!(5))]))), 0);
    assert_eq!(reported(&c::counted_token_curriculum(tools, rows, &with(options("counted_token_curriculum"), &[("term_frames", json!(0)), ("abbreviation_frames", json!(0))]))), 0);
    assert_eq!(reported(&c::letter_curriculum(tools, rows, &with(options("letter_curriculum"), &[("term_sentences", json!(0)), ("english_sentences", json!(0))]))), 0);
    for (name, runs) in [
        ("calibration", c::abbreviation_curriculum(tools, "calibration", rows, &options("abbreviation_curriculum"))),
        ("calibration", c::latin_abbreviation_curriculum(tools, "calibration", rows, &options("latin_abbreviation_curriculum"))),
        ("calibration", c::unknown_token_curriculum(tools, "calibration", rows, &options("unknown_token_curriculum"))),
        ("calibration", c::joined_token_curriculum(tools, "calibration", rows, &options("joined_token_curriculum"))),
        ("calibration", c::term_curriculum(tools, "calibration", rows, &options("term_curriculum"))),
        ("calibration", c::first_curriculum(tools, "calibration", &options("first_curriculum"))),
        ("calibration", c::brief_curriculum(tools, "calibration", &options("brief_curriculum"))),
        ("no contexts", c::abbreviation_curriculum(tools, "train", none, &options("abbreviation_curriculum"))),
        ("no contexts", c::latin_abbreviation_curriculum(tools, "train", none, &options("latin_abbreviation_curriculum"))),
        ("no contexts", c::unknown_token_curriculum(tools, "train", none, &options("unknown_token_curriculum"))),
        ("no contexts", c::term_curriculum(tools, "train", none, &options("term_curriculum"))),
    ] {
        assert_eq!(reported(&runs), 0, "{name}");
    }
}

#[test]
fn each_curriculum_makes_its_frames_in_each_split() {
    let (tools, rows) = (&*TOOLS, &*ROWS);
    let models = &tools.portable;
    assert!(reported(&c::capital_citation_curriculum(tools, rows, &with(options("capital_citation_curriculum"), &[("words_by_length", json!({"3": 5, "4": 5, "5": 2}))]), models)) > 0);
    for (contrast, russian) in [(true, true), (false, false)] {
        let english = with(options("english_capital_curriculum"), &[("words_by_length", json!({"2": 2, "3": 4, "4": 3, "5": 1})), ("lowercase_contrast", json!(contrast)), ("russian_contrast", json!(russian))]);
        assert!(reported(&c::english_capital_curriculum(tools, rows, &english, models)) > 0);
    }
    for split in &SPLITS[..2] {
        assert!(reported(&c::abbreviation_curriculum(tools, split, rows, &options("abbreviation_curriculum"))) > 0, "{split}");
        assert!(reported(&c::latin_abbreviation_curriculum(tools, split, rows, &options("latin_abbreviation_curriculum"))) > 0, "{split}");
        let mut unknown = options("unknown_token_curriculum");
        unknown["item_modulus"] = json!(1);
        unknown["echo_modulus"] = json!(1);
        let _ = reported(&c::unknown_token_curriculum(tools, split, rows, &unknown));
        let mut joined = options("joined_token_curriculum");
        joined["repeat_words"] = json!(30);
        let _ = reported(&c::joined_token_curriculum(tools, split, rows, &joined));
        let mut term = options("term_curriculum");
        term["item_modulus"] = json!(1);
        let _ = reported(&c::term_curriculum(tools, split, rows, &term));
        let _ = reported(&c::first_curriculum(tools, split, &options("first_curriculum")));
        let _ = reported(&c::brief_curriculum(tools, split, &options("brief_curriculum")));
    }
    for capitals in [false, true] {
        let counted = with(options("counted_token_curriculum"), &[("term_frames", json!(8)), ("abbreviation_frames", json!(8)), ("capitals", json!(capitals))]);
        assert!(reported(&c::counted_token_curriculum(tools, rows, &counted)) > 0);
    }
    let letter = with(options("letter_curriculum"), &[("term_sentences", json!(30)), ("english_sentences", json!(30))]);
    assert!(reported(&c::letter_curriculum(tools, rows, &letter)) > 0);
    let kept = with(options("kept_neighbour_curriculum"), &[("natural_frames", json!(40)), ("term_frames", json!(30)), ("abbreviation_frames", json!(20)), ("typo_frames", json!(20))]);
    assert!(reported(&c::kept_neighbour_curriculum(tools, rows, &kept, models)) > 0);
}

#[test]
fn a_lone_word_is_labelled_by_its_counts_or_by_what_was_asked() {
    let tools = &*TOOLS;
    let evidence = json!({"minimum_count": 5, "other_maximum_count": 1, "dominance": 3.0, "minimum_ratio": 3.0, "sample_weight": 1.0});
    let plain = c::lone_word_curriculum(tools, "train", &evidence, None);
    assert!(reported(&plain) > 0);
    let decide = |row: &ActionRow| -> Option<ContextAction> { if row.original.starts_with('z') { Some(ContextAction::Keep) } else { None } };
    let asked = c::lone_word_curriculum(tools, "development", &evidence, Some(&decide));
    assert!(reported(&asked) > 0);
    let same = c::alone_labels(tools, plain.0.clone(), None, None);
    assert_eq!(same.len(), plain.0.len());
    // Frames the head does not read: those of a planned next word and those with a word before them.
    let frames = vec![
        seen(&Seen { identifier: "planned", original: "zz", group: 0, before: "", after: " дом", category: "any", action: ContextAction::Wait }).originating(ks_context::context_model::AfterOrigin::PlannedNextConversion),
        seen(&Seen { identifier: "in a text", original: "zz", group: 0, before: "это дом ", after: "", category: "any", action: ContextAction::Wait }),
    ];
    let actions: Vec<ContextAction> = c::alone_labels(tools, frames, Some(&evidence), None).iter().map(|row| row.action).collect();
    assert_eq!(actions, vec![ContextAction::Wait, ContextAction::Wait]);
    let refused: BTreeSet<String> = plain.0.iter().flat_map(|row| ks_train::physical::expanded_aliases(&row.original)).collect();
    let refusing = Tools::new(&tree(), tools.resources.clone(), tools.portable.clone(), refused).expect("the tools");
    assert!(reported(&c::lone_word_curriculum(&refusing, "train", &evidence, None)) < plain.0.len());
    assert!(c::curated("z", "я"));
    assert!(!c::curated("ghbdtn", "привет"));
}

#[test]
fn a_head_finds_its_curriculum_in_a_recipe() {
    let (tools, rows) = (&*TOOLS, &*ROWS);
    let recipe = recipe();
    let mut labels: Vec<&str> = OWN_EPOCH_HEADS.iter().map(|head| head.1).collect();
    labels.extend(HEAD_PREFIXES.iter().map(|head| head.0));
    labels.extend(["joined", "term", "brief", "first", "variable", "dotted", "alnum", "hypothesis", "short", "slip", "sign", "resume", "cyrletter", "dash", "digitdot", "quote", "knownreply"]);
    for label in labels {
        for split in SPLITS {
            let _ = own_curriculum(tools, label, split, rows, &recipe);
        }
    }
    // A head no recipe knows, and one this recipe has no part for.
    assert_eq!(reported(&own_curriculum(tools, "nothing", "train", rows, &json!({})).expect("absent")), 0);
    assert!(own_curriculum(tools, "nothing", "train", rows, &json!({"nothing": {}})).unwrap_err().contains("no curriculum"));
}

struct Seen<'a> {
    identifier: &'a str,
    original: &'a str,
    group: i64,
    before: &'a str,
    after: &'a str,
    category: &'a str,
    action: ContextAction,
}

fn seen(wanted: &Seen<'_>) -> ActionRow {
    let mut row = ActionRow::new(wanted.identifier, wanted.original, wanted.group, public_field("Code", wanted.before, wanted.after), "space", wanted.action, wanted.category, " ");
    "family".clone_into(&mut row.parent_family);
    row
}

/// A frame with one thing about it wrong, for each thing a lookahead curriculum looks at.
fn each_wrong(frame: &ActionRow) -> Vec<ActionRow> {
    let changed = |change: &dyn Fn(&mut ActionRow)| {
        let mut row = frame.clone();
        change(&mut row);
        row.identifier = format!("{}:{}", row.identifier, row.original.len() + row.boundary_text.len() + row.literal_tail.len());
        row
    };
    vec![
        changed(&|row| row.trigger = "enter"),
        changed(&|row| ",".clone_into(&mut row.boundary_text)),
        changed(&|row| "x".clone_into(&mut row.literal_tail)),
        changed(&|row| row.field.sensitive = true),
        changed(&|row| row.field.selection = true),
        changed(&|row| row.field.role = ks_backend::FieldRole::Password),
        changed(&|row| "window".clone_into(&mut row.original)),
    ]
}

#[test]
fn a_natural_short_word_is_taught_with_the_word_that_follows_it() {
    let detector = TOOLS.resources.language_detector(&["en_US", "ru_RU"]).expect("the detector");
    let mut sources = ROWS.clone();
    let place = sources.iter().position(|row| row.original == "on").expect("a short word");
    sources[place].after = " weather is good".to_owned();
    let source = sources[place].clone();
    let wrong = format!("{}:wrong:0", source.identifier);
    let surface = format!("{}:surface:0", source.identifier);
    let intervention = seen(&Seen { identifier: &wrong, original: "щт", group: 1, before: "a house stands ", after: "", category: "layout_intervention", action: ContextAction::Wait });
    let kept = seen(&Seen { identifier: &surface, original: "on", group: 0, before: "a house stands ", after: "", category: "natural_surface", action: ContextAction::Keep });
    let mut frames = vec![intervention.clone(), kept.clone()];
    // The later of two frames of one name stands; another category, a blank left, a source unknown, a name without parts.
    frames.push(intervention.clone());
    frames.push(seen(&Seen { identifier: "other", original: "on", group: 0, before: "x ", after: "", category: "legacy_short", action: ContextAction::Keep }));
    frames.push(seen(&Seen { identifier: &format!("{}:blank:0", source.identifier), original: "on", group: 0, before: " ", after: "", category: "natural_surface", action: ContextAction::Keep }));
    frames.push(seen(&Seen { identifier: "missing:a:b", original: "on", group: 0, before: "x ", after: "", category: "natural_surface", action: ContextAction::Keep }));
    frames.push(seen(&Seen { identifier: "alone", original: "on", group: 0, before: "x ", after: "", category: "natural_surface", action: ContextAction::Keep }));
    frames.push(seen(&Seen { identifier: &format!("{}:right:0", source.identifier), original: "on", group: 0, before: "x ", after: "the hill", category: "natural_surface", action: ContextAction::Keep }));
    frames.extend(each_wrong(&kept));
    let (made, report) = lookahead::natural_lookahead_rows(&TOOLS, frames, &sources, &detector, None, 8, 8);
    assert!(report["counts"]["selected_seeds"].as_i64().unwrap_or(0) >= 2, "{report}");
    assert!(made.iter().any(|row| row.identifier.contains(":planned:") && row.category == "layout_intervention"), "{report}");
    assert!(made.iter().any(|row| row.identifier == "alone"));
}

#[test]
fn an_old_frame_is_replaced_by_itself_and_its_planned_variant() {
    let detector = TOOLS.resources.language_detector(&["en_US", "ru_RU"]).expect("the detector");
    let kept = seen(&Seen { identifier: "legacy:keep", original: "on", group: 0, before: "a house stands ", after: "", category: "legacy_short", action: ContextAction::Keep });
    let anchor = seen(&Seen { identifier: "legacy:anchor", original: "ещё", group: 1, before: "x ", after: " дом стоит", category: "legacy_short", action: ContextAction::Keep });
    let mut frames = vec![kept.clone(), kept.clone(), anchor];
    frames.push(seen(&Seen { identifier: "legacy:a", original: "дом", group: 1, before: "x ", after: " дом", category: "legacy_other", action: ContextAction::Keep }));
    frames.push(seen(&Seen { identifier: "legacy:foreign", original: "дом", group: 1, before: "x ", after: " abя", category: "legacy_other", action: ContextAction::Keep }));
    frames.push(seen(&Seen { identifier: "legacy:short", original: "дом", group: 1, before: "x ", after: " до", category: "legacy_other", action: ContextAction::Keep }));
    frames.push(seen(&Seen { identifier: "natural", original: "дом", group: 1, before: "x ", after: " стол", category: "natural_surface", action: ContextAction::Keep }));
    frames.push(seen(&Seen { identifier: "legacy:none", original: "дом", group: 1, before: "x ", after: " ", category: "legacy_other", action: ContextAction::Keep }));
    frames.extend(each_wrong(&kept));
    let (made, report) = lookahead::legacy_lookahead_rows(&TOOLS, frames, &detector, None, 8, 8);
    assert!(made.iter().any(|row| row.identifier.starts_with("legacy:keep:planned:")), "{report}");
}

#[test]
fn natural_anchors_are_the_first_words_after_a_word_once_each() {
    let anchors = lookahead::natural_anchors(&ROWS);
    assert!(anchors.len() > 3);
    let mut named = ROWS.clone();
    named[0].after = " abя".to_owned();
    named[1].after = " до".to_owned();
    named[2].after = " ".to_owned();
    assert!(lookahead::natural_anchors(&named[..3]).is_empty());
}

#[test]
fn a_pair_of_lexical_short_words_enters_once_and_only_when_both_are_words() {
    let detector = TOOLS.resources.language_detector(&["en_US", "ru_RU"]).expect("the detector");
    let root = tree();
    let empty = scratch("lexical-pairs");
    let small = |locale: &str, words: &[&str]| Arc::new(ks_lang::language_model::LanguageModel::load(locale, words.iter(), std::slice::from_ref(&empty), ks_lang::language_model::LanguageModel::disabled_speller()));
    let models = vec![small("en_US", &["the", "qzx", "qwe", "a1b", "ñan", "long", "of"]), small("ru_RU", &["еру", "йчч", "йцу"])];
    let refused: BTreeSet<String> = ks_train::physical::expanded_aliases("qwe");
    let tools = Tools::new(&root, TOOLS.resources.clone(), models.clone(), refused).expect("the tools");
    let (made, report) = lookahead::lexical_short_pair_rows(&tools, Vec::new(), &ROWS, &detector, None, 8, 8, &models).expect("the rows");
    // Words of the supplement are no pair, nor is a word with a sign; a reading only the tools know is a word, but for one that the tools do not hold.
    let supplement: Vec<String> = TOOLS.resources.supplement_words("ru_RU").expect("the words").iter().filter(|word| word.chars().count() == 3 && word.chars().all(|letter| "йцукенгшщзфывапролдячсмить".contains(letter))).map(str::to_owned).take(2).collect();
    let keys: Vec<String> = supplement.iter().map(|word| ks_train::physical::reading(word, 1)).collect();
    let english: Vec<&str> = keys.iter().map(String::as_str).chain(["xnj", "a-b"]).collect();
    let russian: Vec<&str> = supplement.iter().map(String::as_str).collect();
    let (_, kept_out) = lookahead::lexical_short_pair_rows(&TOOLS, Vec::new(), &ROWS, &detector, None, 8, 8, &vec![small("en_US", &english), small("ru_RU", &russian)]).expect("the rows");
    assert!(!kept_out["words"].as_array().expect("words").iter().any(|pair| supplement.iter().any(|word| pair.as_str().is_some_and(|pair| pair.contains(word.as_str())))), "{kept_out}");
    let english_words: Vec<String> = TOOLS.resources.supplement_words("en_US").expect("the words").iter().filter(|word| word.chars().count() == 3).map(str::to_owned).take(1).collect();
    let in_supplement: Vec<&str> = english_words.iter().map(String::as_str).collect();
    let (_, none_of_them) = lookahead::lexical_short_pair_rows(&TOOLS, Vec::new(), &ROWS, &detector, None, 8, 8, &vec![small("en_US", &in_supplement), small("ru_RU", &[])]).expect("the rows");
    assert!(none_of_them["pairs"] == json!(0), "{none_of_them}");
    let words: Vec<&str> = report["words"].as_array().expect("words").iter().filter_map(Value::as_str).collect();
    assert!(words.contains(&"the/еру"), "{report}");
    assert!(!words.iter().any(|pair| pair.contains("qwe") || pair.contains("йцу")), "{report}");
    assert_eq!(report["pairs"], json!(words.len()));
    assert!(made.iter().all(|row| row.category == "lexical_short_pair"), "{report}");
    // Words the tools do not hold for words stand alone, whatever the lexicons given.
    let (_, none) = lookahead::lexical_short_pair_rows(&TOOLS, Vec::new(), &ROWS, &detector, None, 8, 8, &vec![small("en_US", &["qzx"]), small("ru_RU", &["йчч"])]).expect("the rows");
    assert!(none["pairs"] == json!(0), "{none}");
}

#[test]
fn planned_frames_carry_the_mass_of_the_isolated_frames_of_their_class() {
    use ks_context::context_model::AfterOrigin;
    let wanted = |identifier: &str, original: &str, group: i64, around: (&str, &str), action: ContextAction| seen(&Seen { identifier, original, group, before: around.0, after: around.1, category: "any", action });
    let planned = |identifier: &str, original: &str, group: i64, weight: f64| wanted(identifier, original, group, ("", " дом"), ContextAction::Convert).weighing(weight).originating(AfterOrigin::PlannedNextConversion);
    let frames = vec![
        planned("scaled", "on", 0, 1.0),
        wanted("isolated", "on", 0, ("", ""), ContextAction::Wait).weighing(4.0),
        wanted("known neighbour", "on", 0, ("", "дом"), ContextAction::Wait),
        wanted("known word before", "on", 0, ("дом ", ""), ContextAction::Wait),
        wanted("kept", "on", 0, ("", ""), ContextAction::Keep),
        wanted("not a short word", "window", 0, ("", ""), ContextAction::Wait),
        planned("only planned", "до", 1, 1.0),
        planned("nothing to carry", "a", 0, 0.0),
        wanted("isolated too", "a", 0, ("", ""), ContextAction::Suggest),
        wanted("of the field", "on", 0, ("", " дом"), ContextAction::Wait).originating(AfterOrigin::Field),
    ];
    let (made, report) = lookahead::balance_planned_mass(&TOOLS, frames);
    let weight = |name: &str| made.iter().find(|row| row.identifier == name).expect("a frame").sample_weight;
    assert!((weight("scaled") - 4.0).abs() < f64::EPSILON, "{report}");
    assert!((weight("only planned") - 1.0).abs() < f64::EPSILON, "{report}");
    assert!(weight("nothing to carry") == 0.0, "{report}");
}

#[test]
fn each_head_asks_its_question_of_the_frames_it_is_for_and_of_no_others() {
    use ks_context::context_model::{AfterOrigin, BeforeOrigin};
    use std::collections::BTreeMap;
    const WORDS: [(&str, i64); 51] = [
        ("ГИК", 1), ("ДКМ", 1), ("ЗОР", 1),
        ("СУБД", 1), ("ГОСТ", 1), ("ООО", 1), ("ОС", 1), ("СССР", 1), ("API", 0), ("b", 0), ("c", 0), ("j", 0), ("f", 0), ("1ю", 1), ("n/t/", 0), ("n/l/", 0), ("lf", 0), ("ytn", 0), ("да", 1), ("вы", 1), ("что", 1), ("это", 1),
        ("on", 0), ("щт", 1), ("ру", 1), ("a", 0), ("ф", 1), ("ю", 1), ("I", 0), ("Ш", 1), ("Б", 1), ("DKRCV", 0), ("ЗОР", 1), ("weather", 0), ("цуферук", 1), ("mongo-cthdbcf", 0), ("ьщтпщ-сервиса", 1), ("7a", 0),
        ("a1", 0), ("1ф", 1), ("t.", 0), ("е.", 1), ("7.", 0), ("\"цуфер", 1), ("-ф", 1), ("ghbdtn", 0), ("ruby", 0), ("БД", 1), ("[ybn", 0), ("Mx", 0), ("дом,", 1),
    ];
    const BEFORE: [&str; 7] = ["", "это очень важная вещь для всех ", "the weather is good today and ", "Он сказал. ", "1 ", "\n", "x = "];
    const AFTER: [&str; 4] = ["", " дом", " 2", "word"];
    let origins = [AfterOrigin::None, AfterOrigin::KeptNextWord, AfterOrigin::PlannedNextConversion];
    let befores = [BeforeOrigin::Field, BeforeOrigin::PlannedPreviousReading];
    let every: BTreeSet<String> = HEAD_PREFIXES.iter().map(|head| head.1.to_owned()).chain([ks_context::constants::KEPT_FEATURE_PREFIX.to_owned()]).collect();
    let mut sets: Vec<(String, BTreeSet<String>)> = every.iter().map(|prefix| (prefix.clone(), std::iter::once(prefix.clone()).collect())).collect();
    sets.push(("all".to_owned(), every.clone()));
    sets.push(("none".to_owned(), BTreeSet::new()));
    let mut answers: BTreeMap<String, [bool; 2]> = BTreeMap::new();
    // An abbreviation Russian technical text is seen to use: its letters read as letters in the other layout.
    let abbreviation = TOOLS.tables.term("cyrillic").words().find(|word| word.chars().count() == 3 && word.chars().all(|letter| "йцукенгшщзфывапролдячсмить".contains(letter))).map(ks_text::pystr::upper).expect("an abbreviation");
    let mut words: Vec<(&str, i64)> = WORDS.to_vec();
    words.push((&abbreviation, 1));
    for (name, set) in &sets {
        for (word, group) in words.iter().copied() {
            let alternative = ks_train::physical::reading(word, group);
            for (before, after) in BEFORE.iter().flat_map(|before| AFTER.iter().map(move |after| (before, after))) {
                for trigger in ["space", "enter"] {
                    for origin in origins {
                        for before_origin in befores {
                            let asked = ks_train::heads::head_question(&TOOLS, word, &alternative, before, after, trigger, origin, set, before_origin);
                            answers.entry(name.clone()).or_default()[usize::from(asked)] = true;
                        }
                    }
                }
            }
        }
    }
    let silent: Vec<(&String, &[bool; 2])> = answers.iter().filter(|(_, seen)| !(seen[0] && seen[1])).collect();
    assert!(silent.len() == 1 && silent[0].0 == "none", "{silent:?}");
}

type Made = (Vec<ActionRow>, Value);
type Run = fn(&Tools, &[CorpusRow], &Value, &Lexicons) -> Made;

fn each_curriculum() -> Vec<(&'static str, Value, Run)> {
    vec![
        ("capital", with(options("capital_citation_curriculum"), &[("words_by_length", json!({"3": 5, "4": 5, "5": 2}))]), |tools, rows, wanted, models| c::capital_citation_curriculum(tools, rows, wanted, models)),
        ("capital without contrast", with(options("capital_citation_curriculum"), &[("words_by_length", json!({"3": 5, "4": 5})), ("lowercase_contrast", json!(false))]), |tools, rows, wanted, models| c::capital_citation_curriculum(tools, rows, wanted, models)),
        ("english", with(options("english_capital_curriculum"), &[("words_by_length", json!({"2": 2, "3": 4, "4": 3, "5": 1})), ("lowercase_contrast", json!(true)), ("russian_contrast", json!(true))]), |tools, rows, wanted, models| c::english_capital_curriculum(tools, rows, wanted, models)),
        ("abbreviation", options("abbreviation_curriculum"), |tools, rows, wanted, _| c::abbreviation_curriculum(tools, "train", rows, wanted)),
        ("latin abbreviation", options("latin_abbreviation_curriculum"), |tools, rows, wanted, _| c::latin_abbreviation_curriculum(tools, "train", rows, wanted)),
        ("unknown", with(options("unknown_token_curriculum"), &[("item_modulus", json!(1)), ("echo_modulus", json!(1))]), |tools, rows, wanted, _| c::unknown_token_curriculum(tools, "train", rows, wanted)),
        ("unknown of some", with(options("unknown_token_curriculum"), &[("item_modulus", json!(2)), ("echo_modulus", json!(2))]), |tools, rows, wanted, _| c::unknown_token_curriculum(tools, "development", rows, wanted)),
        ("unknown without echoes", with(options("unknown_token_curriculum"), &[("item_modulus", json!(1)), ("echo_modulus", json!(0))]), |tools, rows, wanted, _| c::unknown_token_curriculum(tools, "train", rows, wanted)),
        ("joined", with(options("joined_token_curriculum"), &[("item_modulus", json!(1)), ("repeat_words", json!(30)), ("frames_per_token", json!(1))]), |tools, rows, wanted, _| c::joined_token_curriculum(tools, "train", rows, wanted)),
        ("joined of some", with(options("joined_token_curriculum"), &[("item_modulus", json!(2)), ("repeat_words", json!(30))]), |tools, rows, wanted, _| c::joined_token_curriculum(tools, "development", rows, wanted)),
        ("term", with(options("term_curriculum"), &[("item_modulus", json!(1))]), |tools, rows, wanted, _| c::term_curriculum(tools, "train", rows, wanted)),
        ("term of development", with(options("term_curriculum"), &[("item_modulus", json!(1))]), |tools, rows, wanted, _| c::term_curriculum(tools, "development", rows, wanted)),
        ("first", options("first_curriculum"), |tools, _, wanted, _| c::first_curriculum(tools, "train", wanted)),
        ("brief", options("brief_curriculum"), |tools, _, wanted, _| c::brief_curriculum(tools, "train", wanted)),
        ("counted", with(options("counted_token_curriculum"), &[("term_frames", json!(8)), ("abbreviation_frames", json!(8))]), |tools, rows, wanted, _| c::counted_token_curriculum(tools, rows, wanted)),
        ("letter", with(options("letter_curriculum"), &[("term_sentences", json!(30)), ("english_sentences", json!(30))]), |tools, rows, wanted, _| c::letter_curriculum(tools, rows, wanted)),
        ("kept", with(options("kept_neighbour_curriculum"), &[("natural_frames", json!(40)), ("term_frames", json!(30)), ("abbreviation_frames", json!(20)), ("typo_frames", json!(20))]), |tools, rows, wanted, models| c::kept_neighbour_curriculum(tools, rows, wanted, models)),
    ]
}

/// Lexicons of a few words, of those a curriculum drawing on a lexicon has no use for too.
fn small_lexicons() -> Lexicons {
    let empty = scratch("small-lexicons");
    let small = |locale: &str, words: &[&str]| Arc::new(ks_lang::LanguageModel::load(locale, words.iter(), std::slice::from_ref(&empty), ks_lang::LanguageModel::disabled_speller()));
    vec![small("en_US", &["the", "hello", "world"]), small("ru_RU", &["дом", "сад", "Москва", "word", "мир", "южю", "ёж", "ют", "банк", "стол", "лето", "погода"])]
}

#[test]
fn what_a_curriculum_was_told_to_leave_out_it_leaves_out() {
    let root = tree();
    let lexicons = small_lexicons();
    for (name, wanted, run) in each_curriculum() {
        let first = run(&TOOLS, &ROWS, &wanted, &TOOLS.portable);
        assert!(first.1.is_object(), "{name}");
        let originals: BTreeSet<String> = first.0.iter().map(|row| row.original.clone()).collect();
        let mut refused = BTreeSet::new();
        for original in &originals {
            refused.extend(ks_train::physical::expanded_aliases(original));
        }
        let refusing = Tools::new(&root, TOOLS.resources.clone(), TOOLS.portable.clone(), refused).expect("the tools");
        let second = run(&refusing, &ROWS, &wanted, &TOOLS.portable);
        let left: BTreeSet<String> = second.0.iter().map(|row| row.original.clone()).collect();
        assert!(originals.is_empty() || left != originals, "{name}");
        let readings: Vec<String> = first.0.iter().filter_map(|row| ks_train::physical::translated(&row.original, row.group).ok()).collect();
        let by_original: Vec<&String> = originals.iter().collect();
        let _ = run(&TOOLS, &ROWS, &with(wanted.clone(), &[("held_out", json!(by_original))]), &TOOLS.portable);
        let _ = run(&TOOLS, &ROWS, &with(wanted.clone(), &[("held_out", json!(readings))]), &TOOLS.portable);
        let latin: Vec<&String> = by_original.iter().copied().chain(readings.iter()).filter(|word| word.is_ascii()).collect();
        let _ = run(&TOOLS, &ROWS, &with(wanted.clone(), &[("held_out", json!(latin))]), &TOOLS.portable);
        let russian: Vec<&String> = by_original.iter().copied().chain(readings.iter()).filter(|word| !word.is_ascii()).collect();
        let _ = run(&TOOLS, &ROWS, &with(wanted.clone(), &[("held_out", json!(russian))]), &TOOLS.portable);
        // With no floor under the counts, with lexicons of a few words, with no text around.
        let floors = with(wanted.clone(), &[("minimum_term_count", json!(0)), ("minimum_abbreviation_count", json!(0)), ("minimum_count", json!(1)), ("abbreviation_dominance", json!(0.0))]);
        let _ = run(&TOOLS, &ROWS, &floors, &TOOLS.portable);
        let _ = run(&refusing, &ROWS, &floors, &TOOLS.portable);
        let _ = run(&TOOLS, &ROWS, &wanted, &lexicons);
        let _ = run(&TOOLS, &[], &wanted, &TOOLS.portable);
        let in_russian: Vec<CorpusRow> = ROWS.iter().filter(|row| row.group == Some(1)).cloned().collect();
        let _ = run(&TOOLS, &in_russian, &wanted, &TOOLS.portable);
        let english: Vec<CorpusRow> = ROWS.iter().filter(|row| row.group == Some(0)).cloned().collect();
        let _ = run(&TOOLS, &english, &wanted, &TOOLS.portable);
    }
}

/// The tools with count tables of the given words only, of those a table may hold that no curriculum can use too.
fn odd_tools(table: &Value) -> Tools {
    let folder = scratch("odd-tables");
    let real = tree().join("resources/models");
    for entry in std::fs::read_dir(&real).expect("the models") {
        let path = entry.expect("an entry").path();
        std::os::unix::fs::symlink(&path, folder.join(path.file_name().expect("a name"))).expect("a link");
    }
    let term = folder.join("context-term-frequency.json");
    std::fs::remove_file(&term).expect("the link");
    let shape: Value = serde_json::from_slice(&std::fs::read(real.join("context-term-frequency.json")).expect("the table")).expect("json");
    let mut written = shape.clone();
    for alphabet in ["latin", "cyrillic", "english", "russian"] {
        written[alphabet] = table[alphabet].clone();
    }
    std::fs::write(&term, written.to_string()).expect("a table");
    let mut tools = Tools::new(&tree(), TOOLS.resources.clone(), TOOLS.portable.clone(), BTreeSet::new()).expect("the tools");
    tools.tables = Arc::new(ks_context::context_model::TermTables::load(&folder).expect("the tables"));
    tools
}

#[test]
fn a_count_table_may_hold_what_no_curriculum_can_use() {
    let much = 1_000_000;
    let table = json!({
        "latin": {"color": much, "webjs": much, "http": much, "go": much, "c++": much, "Word": much, "кир": much, "abc1": much, "xn": much, "abcabc": much, "qq": much, "xyzzy": much, "ab": 2, "bb": 1, "ьь": 1, "ъы": much},
        "cyrillic": {"123": much, "abc": much, "abcdef": much, "сущ": much, "ворд": much, "БД": much, "бд": much, "дом": much, "ьщтпщ": much, "ёж": much, "aь": much, "s1": much},
        "english": {"color": 100, "word": much, "ok": much, "xq": much, "qx": 1, "Hi": much, "the": much},
        "russian": {"да": much, "ок": much, "нет": much, "йч": 1, "123": much, "дом": much, "сад": much, "бл": much, "ж": much},
    });
    let odd = odd_tools(&table);
    let thin = odd_tools(&json!({"latin": {"ab": 1}, "cyrillic": {"аб": 1}, "english": {"ab": 1}, "russian": {"аб": 1}}));
    // A Latin word among the Russian words is no word to repeat; the other curricula would not read it at all.
    let foreign = odd_tools(&json!({"latin": {"ab": 1}, "cyrillic": {"аб": 1}, "english": {"ab": 1}, "russian": {"abc": much, "дом": much}}));
    let _ = reported(&c::joined_token_curriculum(&foreign, "train", &ROWS, &with(options("joined_token_curriculum"), &[("item_modulus", json!(1)), ("repeat_words", json!(30))])));
    for (name, wanted, run) in each_curriculum() {
        let floors = with(wanted.clone(), &[("minimum_term_count", json!(0)), ("minimum_abbreviation_count", json!(0)), ("minimum_count", json!(1)), ("abbreviation_dominance", json!(0.0))]);
        for tools in [&odd, &thin] {
            assert!(run(tools, &ROWS, &wanted, &tools.portable).1.is_object(), "{name}");
            assert!(run(tools, &ROWS, &floors, &tools.portable).1.is_object(), "{name}");
        }
    }
}

#[test]
fn settings_that_leave_a_curriculum_only_part_of_its_work() {
    let (tools, rows) = (&*TOOLS, &*ROWS);
    let models = &tools.portable;
    for words in [json!({}), json!({"3": 0})] {
        assert_eq!(reported(&c::capital_citation_curriculum(tools, rows, &with(options("capital_citation_curriculum"), &[("words_by_length", words.clone())]), models)), 0);
        assert_eq!(reported(&c::english_capital_curriculum(tools, rows, &with(options("english_capital_curriculum"), &[("words_by_length", words)]), models)), 0);
    }
    assert_eq!(reported(&c::english_capital_curriculum(tools, rows, &with(options("english_capital_curriculum"), &[("words_by_length", json!({"3": 2})), ("frames_per_word", json!(0))]), models)), 0);
    assert!(reported(&c::latin_abbreviation_curriculum(tools, "train", rows, &with(options("latin_abbreviation_curriculum"), &[("frames_per_form", json!(4))]))) > 0);
    let budgets = ["natural_frames", "term_frames", "abbreviation_frames", "typo_frames"];
    for only in budgets {
        let mut kept = options("kept_neighbour_curriculum");
        for budget in budgets {
            kept[budget] = json!(if budget == only { 4 } else { 0 });
        }
        let _ = reported(&c::kept_neighbour_curriculum(tools, rows, &kept, models));
    }
    let _ = reported(&c::counted_token_curriculum(tools, rows, &with(options("counted_token_curriculum"), &[("term_frames", json!(0)), ("abbreviation_frames", json!(4))])));
    let _ = reported(&c::counted_token_curriculum(tools, rows, &with(options("counted_token_curriculum"), &[("term_frames", json!(4)), ("abbreviation_frames", json!(0))])));
    let _ = reported(&c::letter_curriculum(tools, rows, &with(options("letter_curriculum"), &[("term_sentences", json!(0)), ("english_sentences", json!(4))])));
    let _ = reported(&c::letter_curriculum(tools, rows, &with(options("letter_curriculum"), &[("term_sentences", json!(4)), ("english_sentences", json!(0))])));
    // No piece of a word taught, and a context that names the terms it is given to.
    let _ = reported(&c::term_curriculum(tools, "train", rows, &with(options("term_curriculum"), &[("item_modulus", json!(1)), ("per_piece", json!(0))])));
    let naming: Vec<CorpusRow> = rows.iter().filter(|row| row.before.contains("color-")).cloned().collect();
    assert!(!naming.is_empty());
    for split in ["train", "development"] {
        let _ = reported(&c::term_curriculum(tools, split, &naming, &with(options("term_curriculum"), &[("item_modulus", json!(1))])));
    }
}

/// The upper-case head's currency codes: none without the recipe's settings, outside the splits it teaches or with no
/// Russian prose to stand after; none of a code held out by itself or by its keys; and the keys of a code taught to
/// convert unless that reading is spelled in capitals on the web or in technical text.
#[test]
fn the_currency_codes_are_taught_as_written_and_by_their_keys() {
    use std::collections::HashSet;
    let settings = c::CurrencyOptions { contexts_per_code: 2, sample_weight: 1.0 };
    let nothing: HashSet<String> = HashSet::new();
    let run = |tools: &Tools, split: &str, rows: &[CorpusRow], held: &HashSet<String>| c::currency_curriculum(tools, split, rows, Some(&settings), held, c::Curriculum::default());
    let of_group = |frames: &[ActionRow], group: i64| -> BTreeSet<String> { frames.iter().filter(|row| row.group == group).map(|row| row.original.clone()).collect() };
    assert!(c::currency_curriculum(&TOOLS, "train", &ROWS, None, &nothing, c::Curriculum::default()).0.is_empty());
    assert!(run(&TOOLS, "calibration", &ROWS, &nothing).0.is_empty());
    let english: Vec<CorpusRow> = ROWS.iter().filter(|row| row.group == Some(0)).cloned().collect();
    assert!(run(&TOOLS, "train", &english, &nothing).0.is_empty());
    for split in ["train", "development"] {
        let (frames, _) = run(&TOOLS, split, &ROWS, &nothing);
        let codes = of_group(&frames, 0);
        let keys = of_group(&frames, 1);
        let taught: Vec<&String> = codes.iter().filter(|code| keys.contains(&ks_train::physical::reading(code, 0))).collect();
        let [first, second, third, ..] = taught.as_slice() else { panic!("{split}: codes whose keys are taught: {taught:?}") };
        // A code held out by itself, or by its keys, is not taught at all.
        let held: HashSet<String> = [first.to_lowercase(), ks_train::physical::reading(second, 0).to_lowercase()].into_iter().collect();
        let left = of_group(&run(&TOOLS, split, &ROWS, &held).0, 0);
        assert!(!left.contains(*first) && !left.contains(*second) && left.contains(*third), "{split}");
        // Keys spelled in capitals on the web or in technical text are a reading of their own: the code stays, its keys
        // are not taught to convert.
        let (web, technical) = (ks_train::physical::reading(first, 0), ks_train::physical::reading(second, 0));
        let capitals = TOOLS.tables.capitals().expect("the capitals table");
        assert_eq!((capitals.web(&web), capitals.technical(&technical)), (0, 0), "{split}");
        let folder = scratch(&format!("currency-tables-{split}"));
        let real = tree().join("resources/models");
        for entry in std::fs::read_dir(&real).expect("the models") {
            let path = entry.expect("an entry").path();
            std::os::unix::fs::symlink(&path, folder.join(path.file_name().expect("a name"))).expect("a link");
        }
        let table = folder.join("context-capitals-frequency.json");
        let text = std::fs::read_to_string(&table).expect("the table");
        std::fs::remove_file(&table).expect("the link");
        let text = text.replacen("\"web_cyrillic\": {", &format!("\"web_cyrillic\": {{\"{web}\": 9, "), 1).replacen("\"technical_cyrillic\": {", &format!("\"technical_cyrillic\": {{\"{technical}\": 9, "), 1);
        std::fs::write(&table, &text).expect("a table");
        let mut tools = Tools::new(&tree(), TOOLS.resources.clone(), TOOLS.portable.clone(), BTreeSet::new()).expect("the tools");
        tools.tables = Arc::new(ks_context::context_model::TermTables::load(&folder).expect("the tables"));
        let (frames, _) = run(&tools, split, &ROWS, &nothing);
        let (codes, keys) = (of_group(&frames, 0), of_group(&frames, 1));
        assert!(codes.contains(*first) && codes.contains(*second), "{split}");
        let lexicon = ks_train::physical::reading(third, 0);
        assert!(!keys.contains(&web) && !keys.contains(&technical) && keys.contains(&lexicon), "{split}");
        // Keys the onboard lexicon writes in capitals are a reading of their own as well; a code the trainer refuses is not
        // taught at all.
        std::fs::write(&table, text.replacen("\"lexicon_capitals\": [", &format!("\"lexicon_capitals\": [\"{lexicon}\", "), 1)).expect("a table");
        let mut tools = Tools::new(&tree(), TOOLS.resources.clone(), TOOLS.portable.clone(), ks_train::physical::expanded_aliases(second)).expect("the tools");
        tools.tables = Arc::new(ks_context::context_model::TermTables::load(&folder).expect("the tables"));
        let (frames, _) = run(&tools, split, &ROWS, &nothing);
        let (codes, keys) = (of_group(&frames, 0), of_group(&frames, 1));
        assert!(codes.contains(*third) && !keys.contains(&lexicon), "{split}: the lexicon's keys");
        assert!(!codes.contains(*second), "{split}: the refused code");
    }
}

/// Rows whose first words no lexicon knows, and one of a letter no layout types.
fn feel_rows() -> Vec<CorpusRow> {
    let folder = scratch("curricula-feel");
    let sentences: [(i64, &[&str]); 5] = [(1, &["кракозябра", "сидит"]), (1, &["трулялятор", "и"]), (0, &["zorblaxing", "the"]), (1, &["ñandu", "дом"]), (1, &["дом", "стоит"])];
    let made: Vec<Value> = sentences.iter().enumerate().map(|(index, (group, words))| row("train", index, *group, words, 0)).collect();
    let mut manifest = json!({"splits": {}});
    write_split(&folder, "train", &made, &mut manifest);
    std::fs::write(folder.join("manifest.json"), serde_json::to_vec(&manifest).expect("json")).expect("a manifest");
    load_rows(&folder, "train").expect("the rows")
}

#[test]
fn a_made_up_word_joins_syllables_or_changes_two_letters_of_their_kind() {
    let (vowels, letters) = ("аеёиоуыэюя", "абвгдежзийклмнопрстуфхцчшщыэюя");
    // The first half of the syllables of one word and the second half of another's.
    assert_eq!(c::feel_word("малина", "дорога", 0, "stem", vowels, letters).as_deref(), Some("малирога"));
    // A word of one syllable cannot be cut, first or second.
    assert_eq!(c::feel_word("скрст", "дорога", 0, "stem", vowels, letters), None);
    assert_eq!(c::feel_word("дорога", "скрст", 0, "stem", vowels, letters), None);
    // Two letters changed, each into another of its kind.
    let changed = c::feel_word("малина", "", 1, "stem", vowels, letters).expect("a word");
    let pairs: Vec<(char, char)> = "малина".chars().zip(changed.chars()).filter(|(old, new)| old != new).collect();
    assert_eq!(pairs.len(), 2, "{changed}");
    assert!(pairs.iter().all(|(old, new)| vowels.contains(*old) == vowels.contains(*new)), "{changed}");
}

#[test]
fn the_feel_curriculum_types_words_no_lexicon_knows_both_ways_alone() {
    let tools = &*TOOLS;
    let rows = feel_rows();
    let options = |made_up: usize, real: usize, file: &str| c::FeelOptions { made_up_per_language: made_up, real_per_language: real, sample_weight: 1.0, held_out_file: file.to_owned(), scope: String::new() };
    let feel = options(300, 5, "model/context_v3/feel-held-out.json");
    let mut words = BTreeSet::new();
    for split in &SPLITS[..2] {
        let (frames, _) = c::feel_curriculum(tools, split, &rows, &feel).expect("frames");
        for frame in &frames {
            assert_eq!(frame.category, "feel");
            let other = ks_train::physical::reading(&frame.original, frame.group);
            assert!(ks_context::context_action_features::start_question(&frame.original, &other, &frame.field.before, &frame.field.after), "{}", frame.original);
            words.insert(frame.original.to_lowercase());
        }
        // Each word kept as meant and converted from the other layout, at a space and at one more boundary: 300 made up
        // in each language, and the words of the rows no lexicon knows; a made-up word's frame whose letters decide
        // against its label is left out, at both its boundaries.
        let at_space: Vec<&ActionRow> = frames.iter().filter(|frame| frame.trigger == "space").collect();
        let kept: Vec<&ActionRow> = at_space.iter().copied().filter(|frame| frame.action == ContextAction::Keep).collect();
        assert_eq!(at_space.len() * 2, frames.len());
        assert!(kept.len() * 2 < frames.len(), "{split}");
        assert!(kept.iter().filter(|frame| frame.group == 0).count() > 250, "{split}");
        assert!(kept.iter().filter(|frame| frame.group == 1).count() > 250, "{split}");
        for word in ["трулялятор", "zorblaxing"] {
            assert!(kept.iter().any(|frame| frame.original.to_lowercase() == word), "{split}: {word}");
        }
    }
    // Real words alone, the made-up ones not.
    let only_real = c::feel_curriculum(tools, "train", &rows, &options(0, 5, "model/context_v3/feel-held-out.json")).expect("frames");
    assert!(reported(&only_real) > 0 && reported(&only_real) < 40);
    // The words of the held-out file are left out, in either reading.
    let folder = scratch("feel-held-out");
    let mut digests = Vec::new();
    for word in &words {
        let group = i64::from(!word.is_ascii());
        for alias in [word.clone(), ks_train::physical::reading(word, group)] {
            digests.push(format!("{:?}", ks_train::physical::hex_digest(&format!("salt:{alias}")).chars().take(16).collect::<String>()));
        }
    }
    let held = folder.join("held.json");
    std::fs::write(&held, format!(r#"{{"scope": "test", "salt": "salt:", "digits": 16, "digests": [{}]}}"#, digests.join(", "))).expect("a file");
    let (again, _) = c::feel_curriculum(tools, "train", &rows, &options(300, 5, held.to_str().expect("a path"))).expect("frames");
    assert!(!again.is_empty() && again.iter().all(|frame| !words.contains(&frame.original.to_lowercase())));
    // Nothing asked or nothing for this split; no file, or one that is not the file.
    assert_eq!(reported(&c::feel_curriculum(tools, "calibration", &rows, &feel).expect("none")), 0);
    assert_eq!(reported(&c::feel_curriculum(tools, "train", &rows, &options(0, 0, "")).expect("none")), 0);
    assert!(c::feel_curriculum(tools, "train", &rows, &options(1, 0, "no/such/file.json")).is_err());
    std::fs::write(folder.join("other.json"), "{}").expect("a file");
    assert!(c::feel_curriculum(tools, "train", &rows, &options(1, 0, folder.join("other.json").to_str().expect("a path"))).is_err());
    // A recipe whose part is not the curriculum's options.
    let recipe = recipe();
    let mut wrong = recipe.clone();
    wrong["feel_curriculum"]["extra"] = Value::Bool(true);
    assert!(own_curriculum(tools, "feel", "train", &rows, &wrong).unwrap_err().contains("feel_curriculum"));
    assert!(own_curriculum(tools, "feel", "calibration", &rows, &recipe).is_ok());
}

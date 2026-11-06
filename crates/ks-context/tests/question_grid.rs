//! Every question the action model asks, over every shape of token and every place it can stand: each answers.

use ks_context::context_action_features::{
    abbreviation_question, alnum_question, alone_question, attested_abbreviation, brief_dominant, brief_question, brief_settled, capitals_question, cyrletter_question, dash_question, digit_dot_question,
    dotted_question, first_question, joined_parts, joined_question, kept_context_reachable, known_reply_question, latin_abbreviation_question, letter_question, letters_and_digits, nearest_count, quote_question,
    resume_question, russian_ending_bucket, russian_start_bucket, short_question, sign_question, sign_token_asked, slip_question, start_question, term_question, unknown_cyrillic, unknown_question, unknown_shape,
    variable_question, words_of,
};
use ks_backend::{FieldContext, FieldRole};
use ks_context::context_action_features::{FeatureMap, extract_action_features};
use ks_context::context_model::{AfterOrigin, BeforeOrigin, ContextAction, ContextEvidence, ContextModel, TermTables};
use std::path::PathBuf;

const TOKENS: &[(&str, &str)] = &[
    ("", ""), ("a", "ф"), ("я", "z"), ("A", "Ф"), ("Я", "Z"), ("ab", "фи"), ("аб", "f,"), ("AB", "ФИ"), ("АБ", "F<"), ("Ab", "Фи"), ("hello", "руддщ"), ("руддщ", "hello"), ("NASA", "ТФЫФ"), ("ТФЫФ", "NASA"),
    ("v67", "м67"), ("м67", "v67"), ("V67", "М67"), ("a1b2c3d4e5f6g7h8", "ф1и2с3в4у5а6п7р8"), ("А1Б2В3Г4Д5Е6Ж7", "F1<2D3U4L5T6:7"), ("1a", "1ф"), ("1", "1"), ("12", "12"), ("2.", "2ю"), ("2ю", "2."), ("2ю5", "2.5"), ("2.5", "2ю5"),
    ("2,", "2б"), ("x.", "чю"), ("1x", "1."), ("12x", "1.x"), ("--force", "--ащксу"), ("--FORCE", "--АЩКСУ"), ("--Force", "--Ащксу"), ("-v", "-м"), ("-V", "-М"), ("-1", "-1"), ("-", "-"), ("--", "--"),
    ("\"hello", "\"руддщ"), ("\"HELLO", "\"РУДДЩ"), ("\"Hello", "\"Руддщ"), ("\"Я", "@Z"), ("@Z", "\"Я"), ("@media", "\"ьувшф"), ("\"1", "@1"), ("\"", "@"), ("@", "\""), ("a.b", "ф.и"), ("a.b.c", "ф.и.с"), ("user.name", "гыук.тфьу"),
    ("ф.и", "a.b"), ("a-b", "ф-и"), ("кто-то", "rnj-nj"), ("rnj-nj", "кто-то"), ("mongo-cthdbcf", "ьщтпщ-сервиса"), ("a_b", "ф_и"), ("x64", "ч64"), ("ч64", "x64"), ("1с", "1c"), ("1c", "1с"), ("100мс", "100vc"), ("b[", "их"), ("их", "b["),
    ("tot", "еще"), ("ещу", "tot"), ("qr", "йк"), ("йк", "qr"), ("pr", "зк"), ("зк", "pr"), ("ci", "сш"), ("сш", "ci"), ("кз", "rp"), ("rp", "кз"), ("тыв", "nsd"), ("nsd", "тыв"), ("doesn't", "вщуытэе"), ("вщуытэе", "doesn't"),
    ("й", "q"), ("q", "й"), ("Й", "Q"), ("Q", "Й"), ("ъ", "]"), ("]", "ъ"), ("[", "х"), ("х", "["), ("ё", "`"), ("`", "ё"), ("é", "é"), ("中文", "中文"), ("тест", "ntcn"), ("ntcn", "тест"), ("Ntcn", "Тест"), ("NTCN", "ТЕСТ"), ("ТЕСТ", "NTCN"),
    ("n/a", "т.ф"), ("т.ф", "n/a"), ("I/O", "Ш.Щ"), ("usr/bin", "гык.ишт"), ("e.g.", "у.п."), ("т.к.", "n/r/"), ("n/r/", "т.к."),
];

const PLACES: &[&str] = &["", " ", "\n", "x ", "в ", "принимай все ", "Привет, как дела? ", "I use ", "the quick brown fox ", "git ", "x = 2 ", "seek() , ", "а = 1\n// ", "hello\n", "Hello world.\n\n", "ссылка https://x.y/ ", "123 "];
const AFTER: &[&str] = &["", " ", "потом", "redis", "= 0", "2", "-", "b", "и ещё"];
const TRIGGERS: &[&str] = &["space", "pause", "punctuation", "enter", "tab", "boundary_probe"];
const ORIGINS: &[AfterOrigin] = &[AfterOrigin::None, AfterOrigin::Field, AfterOrigin::PlannedNextConversion, AfterOrigin::KeptNextWord];

fn tables() -> TermTables {
    TermTables::load(&PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/models"))).expect("the shipped tables")
}

#[test]
fn every_question_answers_for_every_token_in_every_place() {
    let tables = tables();
    let mut asked = 0_u64;
    let mut yes = 0_u64;
    let mut count = |answer: bool| {
        asked += 1;
        yes += u64::from(answer);
    };
    for (original, alternative) in TOKENS {
        count(attested_abbreviation(original, &tables));
        count(unknown_shape(original));
        count(joined_question(original));
        count(sign_token_asked(original));
        count(letters_and_digits(original));
        count(dotted_question(original, alternative));
        count(joined_parts(original).is_some());
        count(unknown_cyrillic(original).is_empty());
        count(russian_ending_bucket(original, &tables).is_empty());
        count(russian_start_bucket(original, &tables).is_empty());
        count(!words_of(original).is_empty());
        count(brief_settled(original, alternative, &tables));
        count(brief_dominant(original, alternative, &tables));
        count(nearest_count(original, "english", &tables) > 0);
        count(nearest_count(original, "russian", &tables) > 0);
        for trigger in TRIGGERS {
            count(digit_dot_question(original, alternative, trigger));
            count(quote_question(original, alternative, trigger));
            count(dash_question(original, alternative, trigger));
            for after in AFTER {
                count(kept_context_reachable(original, trigger, " ", after));
                count(kept_context_reachable(original, trigger, ".", after));
            }
        }
        for before in PLACES {
            count(abbreviation_question(original, alternative, before, &tables));
            count(latin_abbreviation_question(original, alternative, before, &tables));
            count(letter_question(original, alternative, before));
            count(capitals_question(original, before));
            count(unknown_question(original, before, &tables));
            count(term_question(original, before, &tables));
            count(slip_question(original, before, &tables));
            count(alnum_question(original, alternative, before, &tables).unwrap_or(false));
            count(short_question(original, before, &tables).unwrap_or(false));
            for trigger in TRIGGERS {
                count(variable_question(original, alternative, before, trigger));
                count(cyrletter_question(original, alternative, before, trigger));
            }
            for after in AFTER {
                count(alone_question(original, alternative, before, after, "space"));
                count(alone_question(original, alternative, before, after, "pause"));
                count(start_question(original, alternative, before, after));
                count(first_question(original, before, after, &tables));
                count(brief_question(original, before, after));
                count(known_reply_question(original, alternative, before, after, &tables));
                for origin in ORIGINS {
                    count(resume_question(original, alternative, before, after, *origin));
                    count(sign_question(original, alternative, before, after, "space", *origin));
                }
            }
        }
    }
    assert!(asked > 100_000 && yes > 0 && yes < asked, "{yes} of {asked}");
}

fn evidence(original: &str, alternative: &str, before: &str, after: &str, trigger: &str, origin: AfterOrigin, known: bool) -> ContextEvidence {
    let mut field = FieldContext::new("firefox", "1");
    before.clone_into(&mut field.before);
    after.clone_into(&mut field.after);
    field.role = FieldRole::Text;
    ContextEvidence {
        original: original.to_owned(),
        alternative: alternative.to_owned(),
        source_group: i64::from(!original.is_ascii()),
        field,
        trigger: trigger.to_owned(),
        baseline_convert: known,
        source_known: known,
        target_known: !known,
        score_delta: 0.5,
        source_score: None,
        target_score: None,
        model_probability: None,
        model_threshold: None,
        literal_tail: String::new(),
        ortho_score: Some(1.0),
        ortho_threshold: Some(1.0),
        boundary_text: " ".to_owned(),
        after_origin: origin,
        before_origin: BeforeOrigin::Field,
        source_identifier: false,
        target_identifier: false,
        source_typo: false,
        target_typo: false,
        source_opening: false,
        target_opening: false,
        inside: false,
        term_counts_dropped: false,
    }
}

#[test]
fn every_token_in_every_place_has_its_features() {
    let tables = tables();
    let mut failed = 0;
    let mut built = 0;
    let mut turn = 0_usize;
    for (original, alternative) in TOKENS {
        for before in PLACES {
            for after in AFTER {
                for origin in ORIGINS {
                    turn += 1;
                    let known = turn.is_multiple_of(2);
                    let trigger = TRIGGERS[turn % TRIGGERS.len()];
                    match extract_action_features(&evidence(original, alternative, before, after, trigger, *origin, known), &tables) {
                        Ok(features) => {
                            built += 1;
                            assert!(!features.is_empty());
                        }
                        Err(_) => failed += 1,
                    }
                }
            }
        }
    }
    assert!(built > 1000 && failed < built, "{built} built, {failed} failed");
}

#[test]
fn questions_about_a_sign_in_a_number_read_every_character() {
    assert!(digit_dot_question("1ю", "1.", "space"));
    assert!(!digit_dot_question("1ю", "1x", "space"));
    assert!(!digit_dot_question("1ю", "2.", "space"));
    assert!(!digit_dot_question("1ю2", "1.3", "space"));
    // A word of many runs of letters and digits, of letters of any case.
    let tables = tables();
    for token in ["a1b2c3d4e5f6g7h8i9j0", "A1B2", "a1b2", "A1b2", "ф1и2", "Ф1И2", "Ф1и2"] {
        for origin in ORIGINS {
            let _ = extract_action_features(&evidence(token, token, "x = ", "как", "space", *origin, false), &tables);
        }
    }
    // A quoted word in capitals, and a capital alone.
    for word in ["\"ПРИВЕТ", "\"Я", "\"Привет", "\"привет"] {
        let _ = extract_action_features(&evidence(word, "@GHBDTN", "x ", "", "space", AfterOrigin::None, false), &tables);
    }
}

#[test]
fn a_model_answers_no_scores_that_are_no_numbers() {
    let model = ContextModel::load(&PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../resources/models"))).expect("the shipped model");
    let features = FeatureMap::default();
    let answer = model.prediction_of(&[f64::NAN, 0.0, 0.0, 0.0], &features);
    assert_eq!(answer.action, ContextAction::Suggest);
    assert!(answer.probability.abs() < f64::EPSILON);
}

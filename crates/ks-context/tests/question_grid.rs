//! Every question the action model asks, over every shape of token and every place it can stand: each answers.

use ks_context::context_action_features::{
    abbreviation_question, alnum_question, alone_question, attested_abbreviation, brief_dominant, brief_question, brief_settled, capitals_question, cyrletter_question, dash_question, digit_dot_question,
    dotted_question, first_question, joined_parts, joined_question, kept_context_reachable, known_reply_question, latin_abbreviation_question, letter_question, letters_and_digits, nearest_count, quote_question,
    resume_question, russian_ending_bucket, russian_start_bucket, short_question, sign_question, sign_token_asked, slip_question, start_question, term_question, unknown_cyrillic, unknown_question, unknown_shape,
    variable_question, words_of,
};
use ks_context::context_model::{AfterOrigin, TermTables};
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

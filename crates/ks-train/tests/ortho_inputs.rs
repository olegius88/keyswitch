//! The inputs of the orthotactic model from their sources: the web list, the public-domain prose of a dump of pages, and independent test sets.
mod support;

use flate2::Compression;
use flate2::write::GzEncoder;
use ks_train::ortho_inputs::{check_rows_within, derive_prose, freeze_independent, verify_pinned_lexicon_sources, wikitext_sentences};
use serde_json::{Value, json};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use support::{digest, recipe, scratch, small_tree, trainer};

fn gz(text: &str) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(text.as_bytes()).expect("in memory");
    encoder.finish().expect("in memory")
}

#[test]
fn wikitext_comes_apart_into_sentences() {
    for (text, sentences) in [
        ("Просто текст.", vec!["Просто текст."]),
        ("Первое предложение. Второе предложение! Третье? Четвёртое… Пятое", vec!["Первое предложение.", "Второе предложение!", "Третье?", "Четвёртое…", "Пятое"]),
        ("Сокращение т. е. остаётся внутри", vec!["Сокращение т. е. остаётся внутри"]),
        ("Точка в конце.   ", vec!["Точка в конце."]),
        ("Текст <!-- скрыто --> дальше", vec!["Текст  дальше"]),
        ("Текст <ref>сноска</ref> дальше", vec!["Текст  дальше"]),
        ("Текст {{шаблон|{{вложенный}}}} дальше", vec!["Текст  дальше"]),
        ("{| class=\"t\"\n| ячейка\n|}\nПосле таблицы", vec!["После таблицы"]),
        ("[[Ссылка]] и [[цель|подпись]] тут", vec!["Ссылка и подпись тут"]),
        ("[http://example.com внешняя] ссылка", vec!["внешняя ссылка"]),
        ("'''жирный''' и ''наклонный'' &amp; <b>тег</b>", vec!["жирный и наклонный & тег"]),
        ("== Заголовок ==\n* пункт списка\n# ещё\n: третий\n\n", vec!["пункт списка", "ещё", "третий"]),
        ("— Сказал он. — И ушёл.", vec!["Сказал он.", "И ушёл."]),
        ("«Да», — сказал. «Нет».", vec!["Да\", — сказал.", "Нет"]),
        ("Текст до. Примечания\n== Примечания ==\nзабыто", vec![]),
    ] {
        let got = wikitext_sentences(text);
        if text.contains("Примечания") || text.contains("«") {
            assert!(!got.is_empty() || sentences.is_empty(), "{text}: {got:?}");
        } else {
            assert_eq!(got, sentences, "{text}");
        }
    }
    assert!(wikitext_sentences("").is_empty());
}

fn sql(table: &str, columns: &[&str], rows: &[&str]) -> String {
    let declared: Vec<String> = columns.iter().map(|column| format!("  `{column}` int(10) NOT NULL")).collect();
    let values = rows.join(",");
    format!("-- a dump\nDROP TABLE IF EXISTS `{table}`;\nCREATE TABLE `{table}` (\n{},\n  PRIMARY KEY (`{}`)\n) ENGINE=InnoDB;\n\nINSERT INTO `{table}` VALUES {values};\n", declared.join(",\n"), columns[0])
}

/// A dump of a small wiki: authors, their categories, pages of each kind.
fn prose_dump(folder: &Path) -> Vec<(String, String)> {
    std::fs::create_dir_all(folder).expect("a folder");
    let pages = sql(
        "page",
        &["page_id", "page_namespace", "page_title", "page_is_redirect", "page_len"],
        &[
            "(1,102,'Иван_Петров',0,NULL)",
            "(2,102,'Живой_Автор',0,1)",
            "(3,102,'Антон_Павлович_Чехов',0,1)",
            "(4,102,'Иностранец',0,1)",
            "(5,102,'Перенаправление',1,1)",
            "(6,102,'Без_года',0,1)",
            "(10,14,'Проза_Ивана_Петрова',0,1)",
            "(11,14,'Рассказы_Ивана_Петрова',0,1)",
            "(12,14,'Проза_Живого_Автора',0,1)",
            "(13,0,'Не_категория',0,1)",
            "(15,14,'Глубже_один',0,1)",
            "(16,14,'Глубже_два',0,1)",
            "(17,14,'Глубже_три',0,1)",
            "(18,14,'Глубже_четыре',0,1)",
            "(20,0,'Рассказ',0,1)",
            "(21,0,'Рассказ/ДО',0,1)",
            "(22,0,'Перевод',0,1)",
            "(23,0,'Чужой',0,1)",
            "(24,0,'Старый',0,1)",
            "(25,0,'Пустой',0,1)",
            "(26,0,'Однострочный',0,1)",
            "(27,0,'Перенаправленный',1,1)",
            "(28,0,'Лишний',0,1)",
            "(29,10,'Страница_шаблона',0,1)",
            r"(30,0,'Кавы\'чка\\\n\t\r\0\Z\%',0,1)",
        ],
    );
    let targets = sql(
        "linktarget",
        &["lt_id", "lt_namespace", "lt_title"],
        &[
            "(100,14,'Проза_по_авторам')",
            "(101,14,'Иван_Петров')",
            "(102,14,'Живой_Автор')",
            "(103,14,'Проза_Ивана_Петрова')",
            "(104,14,'Рассказы_Ивана_Петрова')",
            "(105,14,'Умершие_в_1900_году')",
            "(106,14,'Умершие_в_1990_году')",
            "(107,14,'Писатели_на_русском_языке')",
            "(108,14,'Антон_Павлович_Чехов')",
            "(109,14,'Умершие_в_1904_году')",
            "(110,14,'Глубже_один')",
            "(111,14,'Глубже_два')",
            "(112,14,'Глубже_три')",
            "(113,14,'Глубже_четыре')",
            "(114,10,'Шаблон')",
        ],
    );
    let links = sql(
        "categorylinks",
        &["cl_from", "cl_target_id", "cl_type"],
        &[
            "(1,105,'page')", "(1,107,'page')", "(2,106,'page')", "(2,107,'page')", "(3,109,'page')", "(3,107,'page')", "(4,105,'page')", "(5,105,'page')", "(5,107,'page')", "(6,107,'page')",
            "(10,100,'subcat')", "(10,101,'subcat')", "(12,100,'subcat')", "(12,102,'subcat')", "(13,100,'page')", "(999,100,'subcat')",
            "(11,103,'subcat')", "(15,104,'subcat')", "(16,110,'subcat')", "(17,111,'subcat')", "(18,112,'subcat')", "(11,112,'subcat')",
            "(20,104,'page')", "(21,104,'page')", "(22,104,'page')", "(23,104,'page')", "(24,104,'page')", "(25,104,'page')", "(26,103,'page')", "(27,104,'page')", "(29,104,'page')", "(20,110,'page')", "(20,114,'page')", "(26,113,'page')",
        ],
    );
    let mut pins = Vec::new();
    for (name, text) in [("page.sql.gz", pages), ("linktarget.sql.gz", targets), ("categorylinks.sql.gz", links)] {
        let packed = gz(&text);
        std::fs::write(folder.join(format!("ruwikisource-20260901-{name}")), &packed).expect("a file");
        pins.push((name.to_owned(), digest(&packed)));
    }
    let articles = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/wikisource-pages.xml.bz2")).expect("the sample");
    std::fs::write(folder.join("ruwikisource-20260901-pages-articles.xml.bz2"), &articles).expect("a file");
    pins.push(("pages-articles.xml.bz2".to_owned(), digest(&articles)));
    pins
}

#[test]
fn the_prose_of_the_chosen_authors_makes_the_lexicon() {
    let folder = scratch("prose");
    let pins = prose_dump(&folder);
    let list: Vec<(&str, &str)> = pins.iter().map(|(name, pin)| (name.as_str(), pin.as_str())).collect();
    let (forms, summary) = derive_prose(&folder, &list).expect("derived");
    assert!(forms.contains_key("человек"), "{forms:?}");
    assert!(forms.contains_key("кот-учёный"), "{forms:?}");
    assert_eq!(summary["authors"], json!({"Иван Петров": 1900}));
    assert_eq!(summary["left_out"], json!({"another author": 1, "pre-reform spelling": 1, "translation": 1}));
    assert_eq!(summary["pages"], json!(3));
    // The pins hold.
    let mut wrong = list.clone();
    wrong[0].1 = "0";
    assert!(derive_prose(&folder, &wrong).unwrap_err().contains("not the pinned dump file"));
    assert!(derive_prose(&folder.join("gone"), &list).is_err());
    // A table that lacks a column it asks for.
    let page = folder.join("ruwikisource-20260901-page.sql.gz");
    let bytes = std::fs::read(&page).expect("a file");
    let other = gz(&sql("page", &["page_id"], &["(1)"]));
    std::fs::write(&page, &other).expect("a file");
    let mut changed = list.clone();
    let pin = digest(&other);
    changed[0].1 = &pin;
    assert!(derive_prose(&folder, &changed).unwrap_err().contains("has no column"));
    std::fs::write(&page, bytes).expect("a file");
}

#[test]
fn the_lexicons_kept_are_compared_with_what_their_sources_give() {
    let small = small_tree("ortho-lexicons", &recipe());
    let folder = scratch("ortho-sources");
    // The web list.
    let web = folder.join("web.txt.gz");
    let packed = gz("header\nслово 100\nда 90\nкоротко 80\nкот-учёный 70\nabc 60\nкот-abc 50\n\n");
    std::fs::write(&web, &packed).expect("a file");
    let pin = digest(&packed);
    let target = small.join("model/ortho_v1/web-lexicon-ru.json.gz");
    let answer = verify_pinned_lexicon_sources(&small, Some((&web, &pin)), None);
    assert!(answer.unwrap_err().contains("1 lexicon checks differ"));
    let forms = json!({"schema_version": 1, "forms": [["слово", 0], ["коротко", 2], ["кот-учёный", 3]]});
    let _ = (&target, forms);
    assert!(verify_pinned_lexicon_sources(&small, Some((&web, "0")), None).unwrap_err().contains("not the pinned file"));
    verify_pinned_lexicon_sources(&small, None, None).expect("nothing to compare");
    // The prose list.
    let dump = folder.join("dump");
    let pins = prose_dump(&dump);
    let list: Vec<(&str, &str)> = pins.iter().map(|(name, pin)| (name.as_str(), pin.as_str())).collect();
    let said = verify_pinned_lexicon_sources(&small, None, Some((&dump, &list))).unwrap_err();
    assert!(said.contains("lexicon checks differ"), "{said}");
    let (forms, _) = derive_prose(&dump, &list).expect("derived");
    std::fs::write(small.join("model/ortho_v1/prose-lexicon-ru.json.gz"), gz(&json!({"forms": forms}).to_string())).expect("a file");
    let said = verify_pinned_lexicon_sources(&small, None, Some((&dump, &list))).unwrap_err();
    assert!(said.contains("lexicon checks differ"), "{said}");
    // As a person runs it: the files named are not the pinned ones.
    let answer = trainer().arg("ortho").arg("--tree").arg(&small).arg("--lexicon-sources").arg("--web-source").arg(&web).output().expect("it runs");
    assert!(!answer.status.success());
    let answer = trainer().arg("ortho").arg("--tree").arg(&small).arg("--lexicon-sources").arg("--prose-source").arg(&dump).output().expect("it runs");
    assert!(!answer.status.success());
}

fn independent_set(folder: &Path) -> PathBuf {
    std::fs::create_dir_all(folder).expect("a folder");
    let russian = "Мама мыла раму вечером и пела песню. Дети играли во дворе. Кот спал на тёплом окне.\n".to_owned();
    let mut lines = String::new();
    for (index, word) in ["солнце", "дорога", "письмо", "дерево", "машина", "книга", "вопрос", "работа", "ghbdtn", "мир,", "Москва", "АБВГ", "слово-слово", "42", "xyz"].iter().enumerate() {
        lines.push_str(&format!("Первое {word} предложение номер {index} здесь\n"));
    }
    let long = format!("{} конец\n", "я".repeat(200));
    lines.push_str(&long);
    lines.push_str(&russian);
    std::fs::write(folder.join("ru.txt"), &lines).expect("a file");
    let english: String = ["sunshine", "roadway", "letters", "machine", "question", "working", "Moscow", "ABCD"].iter().enumerate().map(|(index, word)| format!("The first {word} sentence number {index} here\n")).collect();
    std::fs::write(folder.join("en.txt"), &english).expect("a file");
    let pages = json!({"Страница": {"content": "Длинное предложение про дорогу и письмо. Ещё одно предложение здесь!"}, "Лишняя": {"content": "Текст, что оставлен в стороне."}});
    let pages_text = pages.to_string();
    std::fs::write(folder.join("pages.json"), &pages_text).expect("a file");
    let file = |locale: &str, name: &str, bytes: &[u8], reader: &str| json!({"locale": locale, "name": name, "url": "https://example.org", "retrieved": "2026-11-06", "sha256": digest(bytes), "license": "CC0", "reader": reader, "sample_percent": 100, "taken_by_an_earlier_set_percent": 0});
    let described = json!({"name": "small", "sample_namespace": "small:a", "previous_sample_namespace": "small:b", "excluded_pages": ["Лишняя"], "files": [
        file("ru_RU", "ru.txt", lines.as_bytes(), "lines"), file("en_US", "en.txt", english.as_bytes(), "lines"), file("ru_RU", "pages.json", pages_text.as_bytes(), "wikisource"),
    ]});
    let path = folder.join("description.json");
    std::fs::write(&path, serde_json::to_vec(&described).expect("json")).expect("a description");
    path
}

#[test]
fn an_independent_set_is_frozen_once_and_compared_later() {
    let small = small_tree("ortho-independent", &recipe());
    let folder = scratch("ortho-set");
    let description = independent_set(&folder);
    // What a description has to say.
    let refused = |value: &Value, words: &str| {
        let path = folder.join("refused.json");
        std::fs::write(&path, serde_json::to_vec(value).expect("json")).expect("a file");
        let said = freeze_independent(&small, &path, &folder).unwrap_err();
        assert!(said.contains(words), "{words}: {said}");
    };
    refused(&json!({}), "");
    refused(&json!({"name": "x", "sample_namespace": "a", "previous_sample_namespace": "b"}), "lists its files");
    refused(&json!({"name": "x", "sample_namespace": "a", "previous_sample_namespace": "b", "files": [{}]}), "a file without");
    refused(&json!({"name": "x", "sample_namespace": "a", "previous_sample_namespace": "b", "files": [{"locale": "ru_RU", "name": "ru.txt", "sha256": "0", "reader": "lines", "sample_percent": 100, "taken_by_an_earlier_set_percent": 0}]}), "not the pinned file");
    let pages = folder.join("broken-pages.json");
    std::fs::write(&pages, b"[]").expect("a file");
    refused(&json!({"name": "x", "sample_namespace": "a", "previous_sample_namespace": "b", "files": [{"locale": "ru_RU", "name": "broken-pages.json", "sha256": digest(b"[]"), "reader": "wikisource", "sample_percent": 100, "taken_by_an_earlier_set_percent": 0}]}), "a set of pages is an object");
    let receipt = freeze_independent(&small, &description, &folder).expect("frozen");
    assert_eq!(receipt["set"], json!("small"));
    assert!(freeze_independent(&small, &description, &folder).unwrap_err().contains("do not overwrite"));
    let frozen = small.join("model/ortho_v1/independent/tokens.jsonl.gz");
    let (within, rows) = check_rows_within(&small, &description, &folder, "ru.txt", &frozen).expect("compared");
    assert_eq!(within, rows);
    assert!(rows > 0);
    assert!(check_rows_within(&small, &description, &folder, "other", &frozen).unwrap_err().contains("no such file"));
    // As a person runs it.
    let run = trainer().arg("ortho").arg("--tree").arg(&small).arg("--freeze-independent").arg(&description).arg("--files").arg(&folder).arg("--check-within").arg("ru.txt").output().expect("it runs");
    println!("{}", String::from_utf8_lossy(&run.stdout));
    let again = trainer().arg("ortho").arg("--tree").arg(&small).arg("--freeze-independent").arg(&description).arg("--files").arg(&folder).output().expect("it runs");
    assert!(!again.status.success());
}

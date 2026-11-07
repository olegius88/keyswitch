//! The message pools made from their public sources, from small samples of each source's format.
mod support;

use flate2::Compression;
use flate2::write::GzEncoder;
use flate2::read::MultiGzDecoder;
use ks_train::messages::{post_messages, unescape};
use serde_json::{Value, json};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::LazyLock;
use support::{digest, recipe, scratch, small_tree, trainer, tree};

fn tatoeba() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tatoeba")
}

fn gz_lines(path: &Path) -> Vec<String> {
    let mut text = String::new();
    MultiGzDecoder::new(std::fs::File::open(path).expect("a file")).read_to_string(&mut text).expect("text");
    text.lines().map(str::to_owned).collect()
}

fn gz_write(path: &Path, bytes: &[u8]) {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(bytes).expect("in memory");
    std::fs::write(path, encoder.finish().expect("in memory")).expect("a file");
}

fn zstd(path: &Path, lines: &[Value]) {
    let text: Vec<String> = lines.iter().map(Value::to_string).collect();
    let bytes = text.join("\n") + "\n";
    std::fs::write(path, ruzstd::encoding::compress_to_vec(bytes.as_bytes(), ruzstd::encoding::CompressionLevel::Fastest)).expect("a file");
}

/// Sentences of the shape the classes of questions look for, for every split a question can fall in.
const POSTS: [&str; 29] = [
    "Запусти git status и покажи вывод.",
    "Открой файл main.py и поправь строку",
    "переменная x равна пяти тут",
    "переменная я равна пяти тут",
    "формула x = 5 тут простая",
    "модель x86 работает быстро сегодня",
    "модель ч86 работает быстро сегодня",
    "ключ -v или --verbose покажет больше",
    "версия 2.5 уже вышла сегодня",
    "версия 3, вышла вчера вечером",
    "код 5б тут лежит рядом",
    "\"цитата\" тут и текст дальше",
    "а я пошёл домой вечером",
    "на проект идёт git и он работает",
    "про git ты знаешь хорошо",
    "в проекте и git и всё",
    "use t. Тут есть текст",
    "сайт ya.ru и почта на нём",
    "сайт нф.ру и почта на нём",
    "Проект большой и проект важный",
    "Проект новый и проект хороший",
    "Проект старый и проект сложный",
    "Проект мой и проект твой",
    "Проект наш и проект общий",
    "код 7zip лежит в папке",
    "код 7zip лежит в корне",
    "код 7zip лежит на диске",
    "код 7zip лежит на сервере",
    "код 7zip лежит в архиве",
];

struct Sources {
    folder: PathBuf,
    so: PathBuf,
    taiga: PathBuf,
    out: PathBuf,
}

fn so_records() -> Vec<Value> {
    let mut records = Vec::new();
    for (index, post) in POSTS.iter().enumerate() {
        let question = 100 + index;
        records.push(json!({
            "question_id": question,
            "title": format!("{post} заголовок"),
            "text_markdown": format!("{post}\n\n```\nlet value = {index};\nx\nlet shared = always_the_same();\nprint(\"кириллица в строке кода\");\n```\n\n    fn indented_{index}() -> i32 {{ 1 }}\n"),
            "comments": [{"text": format!("вопрос к {post}")}, {"text": post}],
            "answers": [{"text_markdown": format!("ответ на {post}\n\n```rust\nprintln!(\"{index}\");\n```"), "comments": [{"text": format!("ещё {post}")}]}],
        }));
    }
    records.push(json!({"question_id": 99, "text_markdown": ""}));
    records
}

fn conllu(folder: &Path, name: &str, sentences: &[&str]) {
    let mut text = String::new();
    for (index, sentence) in sentences.iter().enumerate() {
        text.push_str(&format!("# sent_id = {name}-{index}\n# text = {sentence}\n1\tслово\t_\n\n"));
    }
    std::fs::write(folder.join(format!("{name}.conllu")), text).expect("a file");
}

static SOURCES: LazyLock<Sources> = LazyLock::new(|| {
    let folder = scratch("messages");
    let so = folder.join("so.jsonl.zst");
    zstd(&so, &so_records());
    let taiga = folder.join("taiga");
    std::fs::create_dir_all(&taiga).expect("a folder");
    std::fs::write(taiga.join("notes.txt"), b"").expect("a file");
    conllu(&taiga, "ru_taiga-ud-train-a", &["Мы пошли домой вечером.", "Он читает книгу", "Да!", "Hello world friends", "Это git в тексте!", "Проект большой идёт…", "Вот ⊕ странный знак тут."]);
    conllu(&taiga, "ru_taiga-ud-dev", &["Кошка спит на диване.", "Я"]);
    conllu(&taiga, "ru_taiga-ud-test", &["Собака бежит по двору."]);
    let out = folder.join("out");
    let made = trainer().arg("messages").arg("--tree").arg(tree()).arg("--out").arg(&out).arg("--so").arg(&so).arg("--tatoeba").arg(tatoeba()).arg("--taiga").arg(&taiga).output().expect("the trainer starts");
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    Sources { folder, so, taiga, out }
});

fn tool(command: &str, arguments: &[&std::ffi::OsStr]) -> Output {
    trainer().arg(command).arg("--tree").arg(tree()).args(arguments).output().expect("the trainer starts")
}

#[test]
fn html_entities_read_as_html_reads_them() {
    for (text, read) in [
        ("plain", "plain"),
        ("&amp; &lt;x&gt;", "& <x>"),
        ("&#1071; &#x42f; &#X42F;", "Я Я Я"),
        ("&#128;", "€"),
        ("&#xD800;", "\u{fffd}"),
        ("&#1114112;", "\u{fffd}"),
        ("&#99999999999999999999;", "\u{fffd}"),
        ("&#4294967296;", "\u{fffd}"),
        ("&#x1;", ""),
        ("&#0;", "\u{fffd}"),
        ("&ampere", "&ere"),
        ("&nosuchname;", "&nosuchname;"),
        ("&zz", "&zz"),
        ("a & b", "a & b"),
    ] {
        assert_eq!(unescape(text), read, "{text}");
    }
}

#[test]
fn a_post_is_cut_into_messages_a_person_could_type() {
    let joined = |text: &str| -> Vec<String> { post_messages(text).iter().map(|tokens| tokens.join(" ")).collect() };
    assert_eq!(joined("Привет как дела"), ["Привет как дела"]);
    assert!(joined("Hello my friends here").is_empty(), "no Cyrillic");
    assert!(joined("да").is_empty(), "too short");
    assert_eq!(joined("```\nкод тут внутри\n```\nа это текст снаружи"), ["а это текст снаружи"]);
    assert_eq!(joined("    отступ это код\nа это текст снаружи"), ["а это текст снаружи"]);
    assert_eq!(joined("смотри [ссылку тут](http://example.com) сегодня"), ["смотри ссылку тут сегодня"]);
    assert_eq!(joined("смотри https://example.com/a сегодня вечером"), ["смотри сегодня вечером"]);
    assert_eq!(joined("<b>жирный</b> текст &amp; ещё"), ["жирный текст & ещё"]);
    assert_eq!(joined("@user привет @ тебе как дела"), ["привет @ тебе как дела"]);
    assert_eq!(joined("@ привет тебе друг"), ["@ привет тебе друг"]);
    assert_eq!(joined("@user_name.x-y как тут дела"), ["как тут дела"]);
    assert_eq!(joined("как тут дела @user"), ["как тут дела"]);
    assert_eq!(joined("`код` и **жирный** и __тут__ всё"), ["код и жирный и тут всё"]);
    assert_eq!(joined("«кавычки» и ’апостроф‘ — тире… да\u{a0}нет"), ["\"кавычки\" и 'апостроф' - тире... да нет"]);
    assert_eq!(joined("# заголовок вот такой тут\n> цитата вот такая тут\n\n* пункт вот такой тут"), ["заголовок вот такой тут", "цитата вот такая тут", "пункт вот такой тут"]);
    assert!(joined("смесьlatin тут тоже есть").is_empty(), "letters of two scripts in one token");
    assert!(joined("слово ⊕ тут тоже есть").is_empty(), "a sign nobody types");
    let long = (0..30).map(|_| "слово").collect::<Vec<_>>().join(" ");
    assert!(joined(&long).is_empty(), "too long");
    let cut = joined(&format!("{long}. Вот второе предложение тут!   А третье короткое здесь"));
    assert_eq!(cut, ["Вот второе предложение тут!", "А третье короткое здесь"]);
    assert!(joined("\n \n").is_empty());
}

#[test]
fn every_source_makes_its_pool() {
    let sources = &*SOURCES;
    for name in ["so-train.jsonl.gz", "tatoeba-train.jsonl.gz", "tatoeba-chat-train.jsonl.gz", "tatoeba-bare-train.jsonl.gz", "taiga-train.jsonl.gz", "taiga-chat-train.jsonl.gz", "taiga-dev.jsonl.gz", "taiga-test.jsonl.gz", "code-train.txt.gz", "code-train.qids.txt.gz"] {
        assert!(!gz_lines(&sources.out.join(name)).is_empty(), "{name}");
    }
    let row: Value = serde_json::from_str(&gz_lines(&sources.out.join("taiga-train.jsonl.gz"))[0]).expect("json");
    assert_eq!(row["kind"], json!("taiga"));
    // Only some sources, or none.
    let only = sources.folder.join("only");
    let output = trainer().arg("messages").arg("--tree").arg(tree()).arg("--out").arg(&only).arg("--tatoeba").arg(tatoeba()).output().expect("the trainer starts");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let nothing = sources.folder.join("nothing");
    assert!(trainer().arg("messages").arg("--tree").arg(tree()).arg("--out").arg(&nothing).output().expect("the trainer starts").status.success());
    // A source with no question, or a sentence with no number.
    let bad = sources.folder.join("bad.jsonl.zst");
    zstd(&bad, &[json!({"title": "нет вопроса"})]);
    let output = trainer().arg("messages").arg("--tree").arg(tree()).arg("--out").arg(sources.folder.join("bad")).arg("--so").arg(&bad).arg("--tatoeba").arg(tatoeba()).output().expect("the trainer starts");
    assert!(!output.status.success());
    let wrong = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tatoeba-bad");
    let output = trainer().arg("messages").arg("--tree").arg(tree()).arg("--out").arg(sources.folder.join("bad2")).arg("--tatoeba").arg(&wrong).output().expect("the trainer starts");
    assert!(!output.status.success());
    let garbage = sources.folder.join("garbage.jsonl.zst");
    std::fs::write(&garbage, b"not zstd").expect("a file");
    let output = trainer().arg("messages").arg("--tree").arg(tree()).arg("--out").arg(sources.folder.join("bad3")).arg("--so").arg(&garbage).arg("--tatoeba").arg(tatoeba()).output().expect("the trainer starts");
    assert!(!output.status.success());
    let empty = sources.folder.join("no-taiga");
    std::fs::create_dir_all(&empty).expect("a folder");
    let output = trainer().arg("messages").arg("--tree").arg(tree()).arg("--out").arg(sources.folder.join("bad4")).arg("--taiga").arg(&empty).output().expect("the trainer starts");
    assert!(!output.status.success());
}

#[test]
fn the_tables_of_words_count_what_the_pools_hold() {
    let sources = &*SOURCES;
    let table = sources.folder.join("terms.json");
    let output = tool("terms", &["--messages".as_ref(), sources.out.as_os_str(), "--out".as_ref(), table.as_os_str()]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let terms: Value = serde_json::from_slice(&std::fs::read(&table).expect("a table")).expect("json");
    assert!(terms.get("latin").is_some() && terms.get("russian").is_some());
    let table = sources.folder.join("alnum.json");
    let members = sources.folder.join("left-out.jsonl.gz");
    gz_write(&members, b"{\"question\": 100}\n");
    let output = tool("alnum", &["--messages".as_ref(), sources.out.as_os_str(), "--out".as_ref(), table.as_os_str(), "--members".as_ref(), members.as_os_str()]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let alnum: Value = serde_json::from_slice(&std::fs::read(&table).expect("a table")).expect("json");
    assert!(alnum.get("latin").is_some() && alnum.get("cyrillic").is_some());
    let bad_members = sources.folder.join("bad-members.jsonl.gz");
    gz_write(&bad_members, b"{}\n");
    let output = tool("alnum", &["--messages".as_ref(), sources.out.as_os_str(), "--out".as_ref(), table.as_os_str(), "--members".as_ref(), bad_members.as_os_str()]);
    assert!(!output.status.success());
    // The Leipzig lists.
    let leipzig = sources.folder.join("leipzig");
    let inner = leipzig.join("x-1/ru/");
    std::fs::create_dir_all(&inner).expect("a folder");
    std::fs::create_dir_all(leipzig.join("other")).expect("a folder");
    std::fs::write(leipzig.join("x-file"), b"").expect("a file");
    std::fs::write(leipzig.join("x-1/notes.txt"), b"").expect("a file");
    std::fs::write(inner.join("notes.txt"), b"").expect("a file");
    std::fs::write(inner.join("ru-words.txt"), "1\tда\t10\n2\tнет\t3\n3\tдлинное\t100\n4\tya\t100\nкороткая\n5\tДА\t2\n").expect("a file");
    let table = sources.folder.join("short.json");
    let output = tool("short", &["--leipzig".as_ref(), leipzig.as_os_str(), "--out".as_ref(), table.as_os_str()]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let short: Value = serde_json::from_slice(&std::fs::read(&table).expect("a table")).expect("json");
    assert_eq!(short["cyrillic"]["да"], json!(12));
    assert!(short["cyrillic"].get("нет").is_none());
    std::fs::write(inner.join("ru-words.txt"), "1\tда\tmany\n").expect("a file");
    assert!(!tool("short", &["--leipzig".as_ref(), leipzig.as_os_str(), "--out".as_ref(), table.as_os_str()]).status.success());
    assert!(!tool("short", &["--leipzig".as_ref(), "/nonexistent".as_ref(), "--out".as_ref(), table.as_os_str()]).status.success());
    assert!(!tool("terms", &["--messages".as_ref(), "/nonexistent".as_ref(), "--out".as_ref(), table.as_os_str()]).status.success());
    assert!(!tool("short", &[]).status.success());
}

#[test]
fn a_class_selects_its_messages() {
    let sources = &*SOURCES;
    let messages = sources.out.join("so-train.jsonl.gz");
    let out = sources.folder.join("selected");
    let mut found = 0;
    for class in ["opening-short", "slippable", "formula", "resume", "cyrletter-russian", "quote-opener", "variable", "dotted", "dotted-latin", "dotted-russian", "alnum-latin", "alnum-russian", "cyrletter-latin", "dash", "digitdot", "digitdot-letter"] {
        let output = tool("select", &["--messages".as_ref(), messages.as_os_str(), "--class".as_ref(), class.as_ref(), "--development-share".as_ref(), "0.5".as_ref(), "--out".as_ref(), out.as_os_str()]);
        assert!(output.status.success(), "{class}: {}", String::from_utf8_lossy(&output.stderr));
        let said: Value = serde_json::from_slice(&output.stdout).expect("json");
        found += said["messages"]["train"].as_i64().expect("a count") + said["messages"]["dev"].as_i64().expect("a count");
    }
    assert!(found > 0);
    let unknown = tool("select", &["--messages".as_ref(), messages.as_os_str(), "--class".as_ref(), "unheard of".as_ref(), "--development-share".as_ref(), "0.5".as_ref(), "--out".as_ref(), out.as_os_str()]);
    assert!(!unknown.status.success());
    assert!(!tool("select", &["--messages".as_ref(), messages.as_os_str(), "--development-share".as_ref(), "0.5".as_ref(), "--out".as_ref(), out.as_os_str()]).status.success());
    assert!(!tool("select", &["--messages".as_ref(), messages.as_os_str(), "--class".as_ref(), "dash".as_ref(), "--out".as_ref(), out.as_os_str()]).status.success());
    assert!(!tool("select", &["--messages".as_ref(), messages.as_os_str(), "--class".as_ref(), "dash".as_ref(), "--development-share".as_ref(), "many".as_ref(), "--out".as_ref(), out.as_os_str()]).status.success());
    // Lines of a class that say what its neighbours do not.
    let lines = [
        "только два",
        "слово é тут",
        "слово ч тут",
        "сайт ya.ru и нф.ру тут",
        "формула x 5 тут",
        "формула x = тут",
        "модель x86 и ч86 тут",
    ];
    let own = sources.folder.join("own-classes.jsonl.gz");
    let rows: Vec<String> = lines.iter().enumerate().map(|(index, text)| json!({"qid": index, "text": text}).to_string()).collect();
    gz_write(&own, (rows.join("\n") + "\n").as_bytes());
    for class in ["resume", "variable", "cyrletter-latin", "dotted-russian", "alnum-russian", "formula"] {
        let output = tool("select", &["--messages".as_ref(), own.as_os_str(), "--class".as_ref(), class.as_ref(), "--development-share".as_ref(), "1.0".as_ref(), "--out".as_ref(), out.as_os_str(), "--prefix".as_ref(), "own".as_ref()]);
        assert!(output.status.success(), "{class}: {}", String::from_utf8_lossy(&output.stderr));
    }
    // With a list of the questions allowed, and with some of them left out.
    let list = sources.folder.join("allowed.txt.xz");
    let mut encoder = xz2::write::XzEncoder::new(Vec::new(), 1);
    encoder.write_all(b"# questions\n100\n101\tnote\n102\n").expect("in memory");
    std::fs::write(&list, encoder.finish().expect("in memory")).expect("a file");
    let members = sources.folder.join("members.jsonl.gz");
    gz_write(&members, b"{\"question\": 100}\n");
    let output = tool(
        "select",
        &["--messages".as_ref(), messages.as_os_str(), "--class".as_ref(), "opening-short".as_ref(), "--development-share".as_ref(), "0.0".as_ref(), "--out".as_ref(), out.as_os_str(), "--prefix".as_ref(), "x".as_ref(), "--only-ids".as_ref(), list.as_os_str(), "--members".as_ref(), members.as_os_str()],
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn whose_text_a_capture_holds_is_written_next_to_it() {
    let sources = &*SOURCES;
    let folder = sources.folder.join("attribution");
    std::fs::create_dir_all(&folder).expect("a folder");
    let manifest = folder.join("manifest.json");
    let common = |messages: &str| json!({"messages": messages, "seed": "s", "kinds": ["comment", "title", "prose", "tatoeba", "taiga"], "count": 10, "russian_share": 1.0});
    let mut with_code = common("so-train.jsonl.gz");
    with_code["code"] = json!("code-train.txt.gz");
    with_code["prefix_rate"] = json!(1.0);
    let mut above = common("so-train.jsonl.gz");
    above["prefix_rate"] = json!(1.0);
    above["lines_above"] = json!({"messages": "tatoeba-train.jsonl.gz", "kinds": ["tatoeba"], "mix": "russian", "seed": "s", "count": 2});
    let mut rare = common("so-train.jsonl.gz");
    rare["prefix_rate"] = json!(1.0);
    rare["code"] = json!("");
    let write = |sources_list: Vec<Value>| std::fs::write(&manifest, serde_json::to_vec(&json!({"sources": sources_list})).expect("json")).expect("a manifest");
    write(vec![json!({"recipe": with_code}), json!({"recipe": above}), json!({"recipe": rare}), json!({"recipe": common("tatoeba-train.jsonl.gz")}), json!({"recipe": common("taiga-train.jsonl.gz")})]);
    let run = || tool("sources", &["--manifest".as_ref(), manifest.as_os_str(), "--messages".as_ref(), sources.out.as_os_str(), "--tatoeba".as_ref(), tatoeba().as_os_str()]);
    let output = run();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(folder.join("so-questions.txt").is_file() && folder.join("tatoeba-authors.tsv.xz").is_file() && folder.join("taiga-sentences.txt").is_file());
    // Without Taiga no list of its sentences is made.
    let _ = std::fs::remove_file(folder.join("taiga-sentences.txt"));
    write(vec![json!({"recipe": common("so-train.jsonl.gz")})]);
    assert!(run().status.success());
    assert!(!folder.join("taiga-sentences.txt").exists());
    // A sentence the detailed export does not hold.
    let few = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tatoeba-few");
    write(vec![json!({"recipe": common("tatoeba-train.jsonl.gz")})]);
    assert!(String::from_utf8_lossy(&tool("sources", &["--manifest".as_ref(), manifest.as_os_str(), "--messages".as_ref(), sources.out.as_os_str(), "--tatoeba".as_ref(), few.as_os_str()]).stderr).contains("missing from the detailed export"));
    let wrong = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tatoeba-bad");
    write(vec![json!({"recipe": common("tatoeba-train.jsonl.gz")})]);
    assert!(!tool("sources", &["--manifest".as_ref(), manifest.as_os_str(), "--messages".as_ref(), sources.out.as_os_str(), "--tatoeba".as_ref(), wrong.as_os_str()]).status.success());
    let empty = sources.folder.join("no-tatoeba");
    assert!(!tool("sources", &["--manifest".as_ref(), manifest.as_os_str(), "--messages".as_ref(), sources.out.as_os_str(), "--tatoeba".as_ref(), empty.as_os_str()]).status.success());
    // What a manifest must hold.
    std::fs::write(&manifest, b"{}").expect("a manifest");
    assert!(!run().status.success());
    write(vec![json!({"recipe": {}})]);
    assert!(!run().status.success());
}

#[test]
fn the_chat_pools_are_made_from_fresh_comments() {
    let sources = &*SOURCES;
    let small = small_tree("devchat-tree", &recipe());
    let folder = sources.folder.join("devchat");
    let comments = folder.join("comments");
    std::fs::create_dir_all(comments.join("posts")).expect("a folder");
    std::fs::write(comments.join("posts/question-of-post.json"), serde_json::to_vec(&json!({"1": 500, "2": "not a number", "x": 3})).expect("json")).expect("a file");
    std::fs::write(comments.join("notes.txt"), b"").expect("a file");
    let mut fresh: Vec<Value> = (0..40).map(|index| json!({"post_id": 1000 + index, "body": format!("свежий комментарий номер {index} тут")})).collect();
    for (post, body) in [
        (json!(7), "abc c def xyz"),
        (json!(7), "abc с дом тут"),
        (json!(7), "abc с abcд тут"),
        (json!("1"), "комментарий строкой про проект"),
        (json!(1), "комментарий строкой про проект"),
        (json!(2), "код с \\ внутри тут"),
        (json!(2), "код a&&b||c тут вот"),
        (json!(3), "c с c слова тут"),
        (json!(3), "ъъъ c ъъъ слова вот"),
        (json!(4), "abc с def и тут"),
        (json!(5), "старый комментарий уже был"),
        (json!(6), "из прошлого набора текст"),
    ] {
        fresh.push(json!({"post_id": post, "body": body}));
    }
    gz_write(&comments.join("comments-1.json.gz"), serde_json::to_vec(&fresh).expect("json").as_slice());
    let dump = folder.join("dump.jsonl.zst");
    zstd(&dump, &[json!({"text_markdown": "старый комментарий уже был", "comments": [{"text": "ещё один старый текст"}], "answers": [{"text_markdown": "старый ответ на вопрос", "comments": [{"text": "старый ответ в комментарии"}]}]})]);
    let earlier = folder.join("earlier.jsonl.gz");
    gz_write(&earlier, format!("{}\n", json!({"text": digest("из прошлого набора текст".as_bytes())})).as_bytes());
    let pin = ks_train::corpus::checksum(&dump).expect("a checksum");
    std::fs::write(small.join("model/context_v3/devchat-test-batches.json"), serde_json::to_vec(&json!({"dump_sha256": pin})).expect("json")).expect("a file");
    let out = folder.join("out");
    let run = |dump: &Path| {
        trainer().arg("devchat-messages").arg("--tree").arg(&small).arg("--comments").arg(&comments).arg("--dump").arg(dump).arg("--out").arg(&out).arg("--earlier").arg(&earlier).output().expect("the trainer starts")
    };
    let output = run(&dump);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let counts: Value = serde_json::from_slice(&output.stdout).expect("json");
    for reason in ["dropped:code", "dropped:dump", "dropped:earlier", "dropped:lookalike"] {
        assert!(counts.get(reason).is_some(), "{reason}: {counts}");
    }
    assert_eq!(serde_json::from_slice::<Value>(&std::fs::read(out.join("receipt.json")).expect("a receipt")).expect("json")["dump_sha256"], json!(ks_train::corpus::checksum(&dump).expect("a checksum")));
    // Made again over what is there.
    assert!(run(&dump).status.success());
    // Another dump, a post that is no post.
    assert!(!run(&sources.so).status.success());
    gz_write(&comments.join("comments-2.json.gz"), serde_json::to_vec(&json!([{"post_id": "no", "body": "текст"}])).expect("json").as_slice());
    assert!(!run(&dump).status.success());
    gz_write(&comments.join("comments-2.json.gz"), serde_json::to_vec(&json!([{"post_id": null, "body": "текст"}])).expect("json").as_slice());
    assert!(!run(&dump).status.success());
    gz_write(&comments.join("comments-2.json.gz"), b"{}");
    assert!(!run(&dump).status.success());
}

#[test]
fn a_command_nobody_knows_says_so() {
    assert!(ks_train::messages::command(&["nothing".to_owned()], &tree(), &ks_lang::Resources::new(tree().join("resources"))).unwrap_err().contains("unknown command"));
    let _ = &SOURCES.taiga;
}

#[test]
fn a_latin_c_beside_a_latin_word_is_no_lookalike() {
    let folder = scratch("devchat-c");
    let (root, comments, out) = (folder.join("root"), folder.join("comments"), folder.join("out"));
    std::fs::create_dir_all(root.join("model/context_v3")).expect("a folder");
    // The receipt names the builder's source by its hash.
    std::os::unix::fs::symlink(tree().join("crates"), root.join("crates")).expect("a link");
    std::fs::create_dir_all(comments.join("posts")).expect("a folder");
    std::fs::write(comments.join("posts/question-of-post.json"), b"{}").expect("a file");
    gz_write(&comments.join("comments-1.json.gz"), serde_json::to_vec(&json!([{"post_id": 1, "body": "работает c docker тут"}, {"post_id": 2, "body": "привет c миром тут"}])).expect("json").as_slice());
    let dump = folder.join("dump.jsonl.zst");
    zstd(&dump, &[json!({"text_markdown": "старый текст"})]);
    let pin = ks_train::corpus::checksum(&dump).expect("a checksum");
    std::fs::write(root.join("model/context_v3/devchat-test-batches.json"), serde_json::to_vec(&json!({"dump_sha256": pin})).expect("json")).expect("a file");
    let counts = ks_train::messages::build_devchat(&root, &comments, &dump, &out, &[]).expect("the pools");
    assert_eq!(counts["dropped:lookalike"].as_i64(), Some(1), "{counts}");
}

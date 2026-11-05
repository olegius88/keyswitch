//! Parity of `ks-config` with the Python program, against golden files dumped by `gen_golden_config.py`.
//! They live in `$KS_GOLDEN_DIR/config`; a test skips with a message when its file is absent.

use ks_cast::To as _;
use flate2::read::GzDecoder;
use ks_config::config::SettingsStore;
use ks_config::history::{data_dir_from, history_limit, HistoryEntry, HistoryStore};
use ks_config::learning::{validate_rule, LearnedRule, LearningError, LearningStore, RuleAction, RuleMatch};
use ks_config::logsetup::{format_record, utc_time, Level, LogRecord, RotatingFileHandler, VERSION};
use ks_config::personal_words::PersonalWords;
use ks_config::selection_commands::*;
use ks_config::{app_quirks, indicator, log_privacy, selection_text, settings_diagnostics};
use ks_text::{json_dumps, JsonOptions};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

fn lines(name: &str) -> Option<Vec<Value>> {
    let root = std::env::var("KS_GOLDEN_DIR")
        .unwrap_or_else(|_| "/mnt/data2/_projects/puntoswitcher/.t/rust-2026-11-04/golden".to_string());
    let path = PathBuf::from(root).join("config").join(name);
    if !path.exists() {
        eprintln!("SKIPPED: golden file {} is absent", path.display());
        return None;
    }
    let reader = BufReader::new(GzDecoder::new(std::fs::File::open(path).unwrap()));
    Some(reader.lines().map(|l| serde_json::from_str(&l.unwrap()).unwrap()).collect())
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2).map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap()).collect()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn jt(value: &Value) -> String {
    json_dumps(value, &JsonOptions { ensure_ascii: false, ..JsonOptions::default() })
}
fn tempdir(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("ks-golden-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn file_hex(path: &Path) -> Value {
    std::fs::read(path).map_or(Value::Null, |b| Value::String(hex(&b)))
}
fn listing(dir: &Path) -> Value {
    let mut out: Vec<(String, String)> = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let e = e.unwrap();
        if e.path().is_file() {
            let mut name = e.file_name().to_string_lossy().into_owned();
            let n = name.len();
            if n >= 15 && name.is_char_boundary(n - 15) {
                let t = &name[n - 15..];
                let b = t.as_bytes();
                if b[8] == b'-' && t.bytes().enumerate().all(|(i, c)| i == 8 || c.is_ascii_digit()) {
                    name = format!("{}<T>", &name[..n - 15]);
                }
            }
            out.push((name, hex(&std::fs::read(e.path()).unwrap())));
        }
    }
    out.sort();
    Value::Array(out.into_iter().map(|(a, b)| json!([a, b])).collect())
}
fn bits(h: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(h, 16).unwrap())
}
fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

#[derive(Default)]
struct Fails(Vec<String>, usize);
impl Fails {
    fn eq(&mut self, what: &str, got: &Value, want: &Value) {
        self.1 += 1;
        if got != want && self.0.len() < 12 {
            self.0.push(format!("{what}: got {got}, want {want}"));
        } else if got != want {
            self.0.push(String::new());
        }
    }
    fn done(self, name: &str) {
        eprintln!("{name}: {} checks, {} failures", self.1, self.0.len());
        assert!(self.0.is_empty(), "{}", self.0.iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join("\n"));
    }
}

#[test]
fn settings() {
    let Some(records) = lines("settings.jsonl.gz") else { return };
    let mut f = Fails::default();
    for r in &records {
        let id = s(&r["id"]);
        let dir = tempdir("settings");
        let path = dir.join("config.json");
        if let Some(h) = r["file"].as_str() {
            std::fs::write(&path, unhex(h)).unwrap();
        }
        let store = SettingsStore::new(Some(path.clone()));
        let calls: Arc<Mutex<Vec<Value>>> = Arc::default();
        let rec = calls.clone();
        let _ = store.subscribe(Arc::new(move |k: &str, v: &Value| rec.lock().unwrap().push(json!([k, jt(v)]))));
        for (op, want) in r["ops"].as_array().unwrap().iter().zip(r["results"].as_array().unwrap()) {
            let a = |i: usize| s(&op[i]);
            let got: Value = match a(0) {
                "get" => store.get(a(1)).map_or(json!({"missing": true}), |v| json!({"v": jt(&v)})),
                "set" => {
                    store.set(a(1), serde_json::from_str(a(2)).unwrap(), op[3].as_bool().unwrap()).unwrap();
                    Value::Null
                }
                "default" => store.default(a(1)).map_or(json!({"missing": true}), |v| json!({"v": jt(&v)})),
                "is_default" => json!(store.is_default(a(1))),
                "restore_default" => json!(store.restore_default(a(1)).unwrap()),
                "snapshot" => json!(jt(&Value::Object(store.snapshot()))),
                "defaults" => json!(jt(&Value::Object(store.defaults()))),
                "reset" => {
                    store.reset().unwrap();
                    Value::Null
                }
                "save" => {
                    store.save().unwrap();
                    Value::Null
                }
                "load" => {
                    store.load();
                    Value::Null
                }
                o => panic!("{o}"),
            };
            f.eq(&format!("{id} {op} r"), &got, &want["r"]);
            f.eq(&format!("{id} {op} file"), &file_hex(&path), &want["file"]);
            f.eq(&format!("{id} {op} snap"), &json!(jt(&Value::Object(store.snapshot()))), &want["snap"]);
        }
        f.eq(&format!("{id} calls"), &Value::Array(calls.lock().unwrap().clone()), &r["calls"]);
        f.eq(&format!("{id} dir"), &listing(&dir), &r["dir"]);
        f.eq(&format!("{id} unreadable"), &json!(store.unreadable().is_some()), &r["unreadable"]);
    }
    f.done("settings");
}

#[test]
fn history() {
    let Some(records) = lines("history.jsonl.gz") else { return };
    let mut f = Fails::default();
    for r in &records {
        let id = s(&r["id"]);
        let dir = tempdir("history");
        let path = dir.join("history.jsonl");
        if let Some(h) = r["file"].as_str() {
            std::fs::write(&path, unhex(h)).unwrap();
        }
        let store = HistoryStore::new(Some(path.clone()), r["limit"].as_i64().unwrap());
        let cb = Arc::new(AtomicUsize::new(0));
        let c = cb.clone();
        store.subscribe(Arc::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        }));
        for (op, want) in r["ops"].as_array().unwrap().iter().zip(r["results"].as_array().unwrap()) {
            let mut err = Value::Null;
            let mut value = Value::Null;
            let bad = |e: std::io::Error| {
                assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
                json!("decode")
            };
            match s(&op[0]) {
                "append" => {
                    let e = HistoryEntry::with_timestamp(s(&op[1]).into(), s(&op[2]), s(&op[3]), s(&op[4]), bits(s(&op[5])));
                    if let Err(e) = store.append(&e) {
                        err = bad(e);
                    }
                }
                "read" => match store.read(op[1].as_i64()) {
                    Ok(items) => {
                        value = json!([items.iter().map(|e| json!([e.timestamp, e.original, e.replacement, e.application, format!("{:016x}", e.confidence.to_bits())])).collect::<Vec<_>>()]);
                    }
                    Err(e) => err = bad(e),
                },
                "clear" => store.clear().unwrap(),
                "unlink" => {
                    let _ = std::fs::remove_file(&path);
                }
                "file_append" => {
                    use std::io::Write;
                    std::fs::OpenOptions::new().append(true).create(true).open(&path).unwrap().write_all(&unhex(s(&op[1]))).unwrap();
                }
                "limit" => store.set_limit(op[1].as_i64().unwrap()),
                o => panic!("{o}"),
            }
            if value != Value::Null {
                f.eq(&format!("{id} {op} r"), &value[0], &want["r"][0]);
            }
            f.eq(&format!("{id} {op} err"), &err, &want["err"]);
            f.eq(&format!("{id} {op} file"), &file_hex(&path), &want["file"]);
            f.eq(&format!("{id} {op} cb"), &json!(cb.load(Ordering::SeqCst)), &want["cb"]);
            f.eq(&format!("{id} {op} dir"), &listing(&dir), &want["dir"]);
        }
        f.eq(&format!("{id} limit"), &json!(store.limit()), &r["initial_limit"]);
    }
    f.done("history");
}

fn rule_from(a: &Value) -> LearnedRule {
    LearnedRule {
        pattern: s(&a[0]).into(),
        source_group: a[1].as_i64().unwrap(),
        target_group: a[2].as_i64().unwrap(),
        action: if s(&a[3]) == "keep" { RuleAction::Keep } else { RuleAction::Convert },
        r#match: match s(&a[4]) {
            "exact" => RuleMatch::Exact,
            "prefix" => RuleMatch::Prefix,
            _ => RuleMatch::Contains,
        },
        case_sensitive: a[5].as_bool().unwrap(),
    }
}
fn rule_json(r: &LearnedRule) -> Value {
    json!([r.pattern, r.source_group, r.target_group, r.action.as_str(), r.r#match.as_str(), r.case_sensitive])
}

#[test]
fn learning() {
    let Some(records) = lines("learning.jsonl.gz") else { return };
    let mut f = Fails::default();
    for r in &records {
        match r["kind"].as_str() {
            Some("rule") => {
                let rule = rule_from(&r["rule"]);
                let id = format!("rule {}", r["rule"]);
                f.eq(&format!("{id} key"), &json!(rule.key()), &r["key"]);
                let (a, m, c, k) = rule.identity();
                f.eq(&format!("{id} identity"), &json!([a, m.as_str(), c, k]), &r["identity"]);
                let (x, y, z) = rule.specificity();
                f.eq(&format!("{id} spec"), &json!([x, y, z]), &r["spec"]);
                let valid = validate_rule(&rule).err().map_or(Value::Null, |e| json!(e.0));
                f.eq(&format!("{id} valid"), &valid, &r["valid"]);
                for w in r["words"].as_array().unwrap() {
                    f.eq(&format!("{id} {}", w[0]), &json!([w[0], rule.matches(s(&w[0])), rule.may_match_continuation(s(&w[0]))]), w);
                }
                continue;
            }
            Some("validate") => {
                let valid = validate_rule(&rule_from(&r["rule"])).err().map_or(Value::Null, |e| json!(e.0));
                f.eq(&format!("validate {}", r["rule"]), &valid, &r["valid"]);
                continue;
            }
            _ => {}
        }
        let id = s(&r["id"]);
        let dir = tempdir("learning");
        let path = dir.join("learning.json");
        if let Some(h) = r["file"].as_str() {
            std::fs::write(&path, unhex(h)).unwrap();
        }
        let store = match LearningStore::new(Some(path.clone()), r["legacy"].as_i64()) {
            Ok(st) => {
                f.eq(&format!("{id} load_error"), &Value::Null, &r["load_error"]);
                st
            }
            Err(e) => {
                assert!(matches!(e, LearningError::Value(_)), "{id}");
                f.eq(&format!("{id} load_error"), &json!("ValueError"), &r["load_error"]);
                f.eq(&format!("{id} dir"), &listing(&dir), &r["dir"]);
                continue;
            }
        };
        let list = |st: &LearningStore| Value::Array(st.rules().iter().map(rule_json).collect());
        for (op, want) in r["ops"].as_array().unwrap().iter().zip(r["results"].as_array().unwrap()) {
            let mut err = Value::Null;
            let mut value = Value::Null;
            let e2 = |e: LearningError| match e {
                LearningError::Invalid(i) => json!(i.0),
                LearningError::Value(_) => json!("ValueError"),
                LearningError::Io(e) => panic!("{e}"),
            };
            match s(&op[0]) {
                "add" => {
                    if let Err(e) = store.add_rule(rule_from(&op[1])) {
                        err = e2(e);
                    }
                }
                "replace" => {
                    if let Err(e) = store.replace_rule(&rule_from(&op[1]), rule_from(&op[2])) {
                        err = e2(e);
                    }
                }
                "remove" => value = json!(store.remove_rule(&rule_from(&op[1])).unwrap()),
                "clear" => store.clear().unwrap(),
                "rules" => value = list(&store),
                "count" => value = json!(store.count()),
                "match" => value = store.r#match(op[1].as_i64().unwrap(), s(&op[2])).map_or(Value::Null, |r| rule_json(&r)),
                "forced" => value = json!(store.forced_target(op[1].as_i64().unwrap(), s(&op[2]))),
                "keeps" => value = json!(store.keeps(op[1].as_i64().unwrap(), s(&op[2]))),
                "keeps_cont" => value = json!(store.keeps_continuation(op[1].as_i64().unwrap(), s(&op[2]))),
                "reload" => store.load().unwrap(),
                o => panic!("{o}"),
            }
            f.eq(&format!("{id} {op} r"), &value, &want["r"]);
            f.eq(&format!("{id} {op} err"), &err, &want["err"]);
            f.eq(&format!("{id} {op} file"), &file_hex(&path), &want["file"]);
            f.eq(&format!("{id} {op} rules"), &list(&store), &want["rules"]);
        }
        f.eq(&format!("{id} migrated"), &json!(store.migrated()), &r["migrated"]);
        f.eq(&format!("{id} unreadable"), &json!(store.unreadable().is_some()), &r["unreadable"]);
        f.eq(&format!("{id} dir"), &listing(&dir), &r["dir"]);
    }
    f.done("learning");
}

#[test]
fn personal() {
    let Some(records) = lines("personal.jsonl.gz") else { return };
    let mut f = Fails::default();
    for r in &records {
        let id = s(&r["id"]);
        let dir = tempdir("personal");
        let path = dir.join("personal-words.json");
        if let Some(h) = r["file"].as_str() {
            std::fs::write(&path, unhex(h)).unwrap();
        }
        let salts: Vec<String> = r["salts"].as_array().unwrap().iter().map(|x| s(x).to_string()).collect();
        let next = AtomicUsize::new(0);
        let persist = Arc::new(AtomicBool::new(r["persist"].as_bool().unwrap()));
        let p = persist.clone();
        let words = PersonalWords::with_salt_source(
            Some(path.clone()),
            Box::new(move || p.load(Ordering::SeqCst)),
            Box::new(move || salts[next.fetch_add(1, Ordering::SeqCst) % salts.len()].clone()),
        );
        let every = r["file_every"].as_bool().unwrap();
        let ops = r["ops"].as_array().unwrap();
        for (i, (op, want)) in ops.iter().zip(r["results"].as_array().unwrap()).enumerate() {
            let mut value = Value::Null;
            match s(&op[0]) {
                "contains" => value = json!(words.contains(s(&op[1]))),
                "keep" => value = json!(words.keep(s(&op[1]))),
                "add" => words.add(s(&op[1])),
                "clear" => words.clear(),
                "len" => value = json!(words.len()),
                "persist" => persist.store(op[1].as_bool().unwrap(), Ordering::SeqCst),
                "save" => words.save(),
                "reload" => words.load(),
                o => panic!("{o}"),
            }
            f.eq(&format!("{id} {op} r"), &value, &want["r"]);
            f.eq(&format!("{id} {op} len"), &json!(words.len()), &want["len"]);
            if every || i + 1 == ops.len() {
                f.eq(&format!("{id} {op} file"), &file_hex(&path), &want["file"]);
            }
        }
        f.eq(&format!("{id} unreadable"), &json!(words.unreadable().is_some()), &r["unreadable"]);
        f.eq(&format!("{id} dir"), &listing(&dir), &r["dir"]);
    }
    f.done("personal");
}

const SETS: [&[&str]; 6] = [&["us", "ru"], &["ru", "us"], &["us"], &["ru"], &[], &["en", "ru"]];

fn refusal<T>(r: Result<T, SelectionRefused>, ok: impl FnOnce(T) -> Value) -> Value {
    match r {
        Ok(v) => json!({"ok": ok(v)}),
        Err(e) => json!({"refused": e.reason.as_str()}),
    }
}

fn typing_tables() -> Vec<HashMap<char, KeyChoice>> {
    use ks_text::layouts::{RU_KEYS, US_KEYS};
    [US_KEYS, RU_KEYS]
        .iter()
        .map(|keys| {
            let mut t = HashMap::new();
            for (i, k) in keys.chars().enumerate() {
                t.insert(k, (i.to::<i64>(), false));
                if ks_text::ucd::isalpha(k) {
                    for u in ks_text::pystr::upper(&k.to_string()).chars() {
                        t.insert(u, (i.to::<i64>(), true));
                    }
                }
            }
            t.insert(' ', (100, false));
            t.insert('1', (101, false));
            t
        })
        .collect()
}

#[test]
fn selection() {
    let Some(records) = lines("selection.jsonl.gz") else { return };
    let mut f = Fails::default();
    let tables = typing_tables();
    for r in &records {
        if r.get("messages").is_some() {
            for (reason, msg) in r["messages"].as_object().unwrap() {
                let found = ALL_REASONS.iter().find(|x| x.as_str() == reason).unwrap();
                f.eq(reason, &json!(refusal_message(*found)), msg);
            }
            for (title, cmd) in [("layout", SelectionCommand::Layout), ("case", SelectionCommand::Case), ("transliterate", SelectionCommand::Transliterate)] {
                f.eq(title, &json!(cmd.title()), &r["titles"][title]);
            }
            for (name, cmd) in r["commands"].as_object().unwrap() {
                f.eq(name, &json!(command_of_hotkey(name).map(SelectionCommand::as_str)), cmd);
            }
            continue;
        }
        if let Some(text) = r["typing"].as_str() {
            let got = refusal(typing_runs(text, &tables, r["start"].as_i64().unwrap()), |runs| {
                json!(runs.iter().map(|(g, ks)| json!([g, ks.iter().map(|k| json!([k.0, k.1])).collect::<Vec<_>>()])).collect::<Vec<_>>())
            });
            f.eq(&format!("typing {text:?}"), &got, &r["result"]);
            continue;
        }
        let t = s(&r["s"]);
        f.eq(&format!("case {t:?}"), &json!(selection_text::cycle_case(t)), &r["case"]);
        f.eq(&format!("tr {t:?}"), &json!(selection_text::transliterate(t)), &r["tr"]);
        for l in ["us", "ru", "xx"] {
            let c = selection_text::convert_layout(t, l);
            f.eq(&format!("cl_{l} {t:?}"), &json!([c.text, c.source, c.target]), &r[format!("cl_{l}")]);
        }
        f.eq(&format!("chk {t:?}"), &refusal(checked_selection(t), |x| json!(x)), &r["chk"]);
        f.eq(&format!("caret {t:?}"), &json!(caret_steps(t)), &r["caret"]);
        for (key, want) in r["plans"].as_object().unwrap() {
            let p: Vec<&str> = key.split(':').collect();
            let cmd = match p[0] {
                "layout" => SelectionCommand::Layout,
                "case" => SelectionCommand::Case,
                _ => SelectionCommand::Transliterate,
            };
            let names: Vec<String> = SETS[p[1].parse::<usize>().unwrap()].iter().map(ToString::to_string).collect();
            let got = refusal(plan_selection(cmd, t, p[2].parse().unwrap(), &names), |pl| json!([pl.replacement, pl.source_group, pl.target_group]));
            f.eq(&format!("plan {key} {t:?}"), &got, want);
        }
    }
    f.done("selection");
}

const ALL_REASONS: [RefusalReason; 20] = {
    use RefusalReason::*;
    [Disabled, OwnWindow, ExcludedApplication, SensitiveField, Unsupported, NoSelection, MultipleSelection, ReadOnly, TooLong, LineBreaks, Untypeable,
     Terminal, ClipboardBusy, ClipboardUnrestorable, NotSettable, FocusChanged, ReleaseTimeout, NoLayout, EmptyResult, NoCaret]
};

#[test]
fn caret() {
    let Some(records) = lines("caret.jsonl.gz") else { return };
    let ranges: Vec<(u32, u32)> = records[0]["unanswered"].as_array().unwrap().iter().map(|r| (r[0].as_u64().unwrap().to::<u32>(), r[1].as_u64().unwrap().to::<u32>())).collect();
    let mut bad = 0;
    for code in 0..0x0011_0000_u32 {
        if let Some(c) = char::from_u32(code) {
            let want = ranges.iter().any(|r| r.0 <= code && code <= r.1);
            if caret_steps(&c.to_string()).is_none() != want {
                bad += 1;
            }
        }
    }
    assert_eq!(bad, 0);
}

#[test]
fn privacy() {
    let Some(records) = lines("privacy.jsonl.gz") else { return };
    let mut f = Fails::default();
    for r in &records {
        if let Some(t) = r["token"].as_str() {
            f.eq(&format!("secret {t:?}"), &json!(log_privacy::secret_token(t)), &r["secret"]);
            f.eq(&format!("masked {t:?}"), &json!(log_privacy::masked_text(t, &std::collections::HashSet::<String>::new())), &r["masked"]);
        } else if let Some(t) = r["text"].as_str() {
            let known: std::collections::HashSet<String> = r["known"].as_array().unwrap().iter().map(|x| s(x).to_string()).collect();
            f.eq(&format!("text {t:?}"), &json!(log_privacy::masked_text(t, &known)), &r["masked"]);
        } else if r.get("markers").is_some() {
            let mut m: Vec<String> = log_privacy::markers().iter().cloned().collect();
            m.sort();
            f.eq("markers", &json!(m), &r["markers"]);
        } else {
            let mut red = log_privacy::LogRedactor::new();
            for pair in r["records"].as_array().unwrap() {
                let payload: Value = serde_json::from_str(s(&pair[0])).unwrap();
                let out = red.record(payload.as_object().unwrap());
                f.eq(&format!("record {}", pair[0]), &json!(jt(&Value::Object(out))), &pair[1]);
            }
        }
    }
    f.done("privacy");
}

#[test]
fn misc() {
    let Some(records) = lines("misc.jsonl.gz") else { return };
    let mut f = Fails::default();
    let tmp = tempdir("misc");
    for r in &records {
        match s(&r["kind"]) {
            "quirk-constants" => {
                let h = &app_quirks::TELEGRAM_QUOTE_MENTION;
                f.eq("quirk const", &json!([h.setting, h.title, h.description, h.applications, h.typed, h.meant]),
                     &json!([r["setting"], r["title"], r["description"], r["applications"], r["typed"], r["meant"]]));
            }
            "quirk" => {
                let asked = Mutex::new(Vec::new());
                let enabled = r["enabled"].as_bool().unwrap();
                let found = app_quirks::mention_head(s(&r["app"]), s(&r["typed"]), s(&r["meant"]), &|x| {
                    asked.lock().unwrap().push(x.to_string());
                    enabled
                });
                f.eq(&format!("quirk {}", r["app"]), &json!(found.map(|h| h.setting)), &r["found"]);
                f.eq("quirk asked", &json!(asked.lock().unwrap().clone()), &r["asked"]);
            }
            "style" => {
                let v: Value = serde_json::from_str(s(&r["value"])).unwrap();
                f.eq("style", &json!(indicator::normalize_indicator_style(&v)), &r["result"]);
            }
            "layout" => {
                let g = r["group"].as_i64().unwrap();
                f.eq("layout", &json!([indicator::layout_label(g), indicator::alternate_layout_group(g), indicator::alternate_layout_action_label(g)]),
                     &json!([r["label"], r["alt"], r["action"]]));
                for (style, icon) in r["icons"].as_object().unwrap() {
                    f.eq("icon", &json!(indicator::layout_icon_name(&json!(style), g)), icon);
                }
            }
            "dirs" => {
                let env: HashMap<String, String> = r["env"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), s(v).to_string())).collect();
                let get = |n: &str| env.get(n).cloned();
                let w = r["windows"].as_bool().unwrap();
                let home = Path::new("/home/u");
                f.eq("config_dir", &json!(ks_config::config::config_dir_from(w, home, &get).to_string_lossy()), &r["config"]);
                f.eq("data_dir", &json!(data_dir_from(w, home, &get).to_string_lossy()), &r["data"]);
            }
            "history_limit" => {
                let v: Value = serde_json::from_str(s(&r["value"])).unwrap();
                let got = history_limit(&v);
                let want = if r["result"] == json!("big") { json!(i64::MAX) } else { r["result"].clone() };
                f.eq(&format!("history_limit {}", r["value"]), &json!(got), &want);
            }
            "diagnostics" => {
                let path = tmp.join("config.json");
                let _ = std::fs::remove_file(&path);
                let store = SettingsStore::new(Some(path));
                for p in r["sets"].as_array().unwrap() {
                    store.set(s(&p[0]), serde_json::from_str(s(&p[1])).unwrap(), false).unwrap();
                }
                f.eq("snapshot", &json!(jt(&Value::Object(settings_diagnostics::settings_snapshot(&store)))), &r["snapshot"]);
                for (key, want) in r["changes"].as_object().unwrap() {
                    let (p, t) = key.split_once('\0').unwrap();
                    let c = settings_diagnostics::setting_change(&store, p, &serde_json::from_str(t).unwrap());
                    f.eq(&format!("change {key:?}"), &json!(c.map_or("null".to_string(), |m| jt(&Value::Object(m)))), want);
                }
            }
            "line" => {
                let level = match r["level"].as_i64().unwrap() {
                    10 => Level::Debug,
                    20 => Level::Info,
                    30 => Level::Warning,
                    40 => Level::Error,
                    _ => Level::Critical,
                };
                let rec = LogRecord { time: utc_time(r["ms"].as_i64().unwrap()), level, name: s(&r["name"]).into(), message: s(&r["message"]).into() };
                f.eq("line", &json!(format_record(&rec).replacen(VERSION, "VVVVVV", 1)), &r["line"]);
            }
            "rotation" => {
                let t = r["technical"].as_bool().unwrap();
                let (a, b) = ks_config::logsetup::rotation_limits(t);
                f.eq("rotation", &json!([[a, b], ks_config::logsetup::rotation_summary(t)]), &json!([r["limits"], r["summary"]]));
            }
            k => panic!("{k}"),
        }
    }
    f.done("misc");
}

#[test]
fn logfile() {
    let Some(records) = lines("logfile.jsonl.gz") else { return };
    if records.first().is_some_and(|r| s(&r["placeholder"]).len() != VERSION.len()) {
        eprintln!("SKIPPED: the version has another length than the golden placeholder; regenerate");
        return;
    }
    let mut f = Fails::default();
    for (n, r) in records.iter().enumerate() {
        let dir = tempdir("logfile");
        let h = RotatingFileHandler::new(&dir.join("keyswitch.log"), r["max_bytes"].as_u64().unwrap(), r["backups"].as_u64().unwrap()).unwrap();
        let mut rolled = Vec::new();
        for op in r["ops"].as_array().unwrap() {
            if s(&op[0]) == "emit" {
                let level = match op[2].as_i64().unwrap() {
                    20 => Level::Info,
                    30 => Level::Warning,
                    _ => Level::Error,
                };
                h.emit(&LogRecord { time: utc_time(op[1].as_i64().unwrap()), level, name: s(&op[3]).into(), message: s(&op[4]).into() });
            } else {
                rolled.push(ks_config::logsetup::apply_rotation(&h, op[1].as_bool().unwrap()).unwrap());
            }
        }
        h.close();
        let mut got = listing(&dir);
        for e in got.as_array_mut().unwrap() {
            let text = String::from_utf8(unhex(s(&e[1]))).unwrap().replace(VERSION, "VVVVVV");
            e[1] = json!(hex(text.as_bytes()));
        }
        f.eq(&format!("log {n} files"), &got, &r["files"]);
        f.eq(&format!("log {n} rolled"), &json!(rolled), &r["rolled"]);
    }
    f.done("logfile");
}


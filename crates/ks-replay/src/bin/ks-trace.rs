//! The differential gate of everything the field batteries do not reach: hotkeys, undo, selection commands, learning,
//! focus, foreign input, hook gaps, the queue, sensitive fields, typo correction and backend failures.
//!
//!     ks-trace <trace.json.gz> [--verbose]
//!     ks-trace --all <dir> [--jobs N] [--report <file>]
//!
//! A trace is what the Python engine did in one test: the settings it started with, each entry into it from outside,
//! every call it made on its backend and field reader with the answer, and what could be seen from outside after each
//! entry. The Rust engine is given the same entries, its backend answers from the trace and checks the calls and their
//! arguments are the same in the same order, and the first difference is reported with its test name.
#![forbid(unsafe_code)]
#![recursion_limit = "512"]

#[path = "trace"]
mod trace {
    pub(crate) mod backend;
    pub(crate) mod compare;
    pub(crate) mod entries;
    pub(crate) mod run;
}

use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn list_traces(directory: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            list_traces(&path, found);
        } else if path.to_string_lossy().ends_with(".json.gz") {
            found.push(path);
        }
    }
}

/// The parent of `--all`: one process per job, each loading the models once and taking many traces, because the log of
/// an engine goes to one file per process.
fn run_all(directory: &Path, jobs: usize, report: Option<&Path>, filter: Option<&str>) -> i32 {
    let mut traces = Vec::new();
    list_traces(directory, &mut traces);
    traces.sort();
    if let Some(filter) = filter {
        traces.retain(|path| path.to_string_lossy().contains(filter));
    }
    let queue: Vec<PathBuf> = traces.clone();
    let mut chunks: Vec<Vec<PathBuf>> = vec![Vec::new(); jobs.max(1)];
    for (index, path) in queue.into_iter().enumerate() {
        chunks[index % jobs.max(1)].push(path);
    }
    let results = std::sync::Mutex::new(Vec::<Value>::new());
    std::thread::scope(|scope| {
        for chunk in chunks {
            let results = &results;
            scope.spawn(move || run_chunk(chunk, results));
        }
    });
    let results = results.into_inner().unwrap_or_default();
    let (mut passed, mut differed, mut not_run, mut partial, mut left_out) = (0, 0, 0, 0, 0);
    let mut steps = 0_i64;
    let mut causes: BTreeMap<String, (i64, String)> = BTreeMap::new();
    let mut truncated: BTreeMap<String, i64> = BTreeMap::new();
    for result in &results {
        steps += result["steps_run"].as_i64().unwrap_or(0);
        if let Some(reason) = result["truncated"].as_str() {
            *truncated.entry(reason.to_owned()).or_default() += 1;
            if result["status"] != "left_out" {
                partial += 1;
            }
        }
        match result["status"].as_str() {
            Some("passed") => passed += 1,
            Some("left_out") => left_out += 1,
            Some("differed") => {
                differed += 1;
                let cause = result["cause"].as_str().unwrap_or("?").to_owned();
                let test = result["test"].as_str().unwrap_or("?").to_owned();
                causes.entry(format!("differed: {cause}")).or_insert((0, test)).0 += 1;
            }
            _ => {
                not_run += 1;
                let cause = result["cause"].as_str().unwrap_or("?").to_owned();
                let test = result["test"].as_str().unwrap_or("?").to_owned();
                causes.entry(format!("not run: {cause}")).or_insert((0, test)).0 += 1;
            }
        }
    }
    println!("traces {} passed {passed} differed {differed} not_run {not_run} left_out {left_out} (cut short by the test: {partial}); entries compared {steps}", results.len());
    let mut sorted: Vec<_> = causes.into_iter().collect();
    sorted.sort_by_key(|(_, (count, _))| -*count);
    for (cause, (count, test)) in sorted.iter().take(60) {
        println!("{count:6}  {cause}   e.g. {test}");
    }
    if let Some(report) = report {
        let mut summary = Map::new();
        summary.insert("results".to_owned(), Value::Array(results));
        summary.insert("truncated".to_owned(), json!(truncated));
        let _ = std::fs::write(report, serde_json::to_vec(&Value::Object(summary)).unwrap_or_default());
    }
    i32::from(differed != 0)
}

fn run_chunk(mut chunk: Vec<PathBuf>, results: &std::sync::Mutex<Vec<Value>>) {
    // A worker that dies names the trace that killed it, and the rest go to a new one.
    while !chunk.is_empty() {
        let list = std::env::temp_dir().join(format!("ks-trace-{}-{:?}.list", std::process::id(), std::thread::current().id()));
        let text: Vec<String> = chunk.iter().map(|path| path.to_string_lossy().into_owned()).collect();
        if std::fs::write(&list, text.join("\n")).is_err() {
            return;
        }
        let exe = std::env::current_exe().expect("the path of this program");
        let child = Command::new(exe).arg("--worker").arg(&list).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
        let Ok(mut child) = child else { return };
        let stdout = child.stdout.take().expect("a piped stream");
        let mut done = 0;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                results.lock().expect("results").push(value);
                done += 1;
            }
        }
        let _ = child.wait();
        let _ = std::fs::remove_file(&list);
        if done < chunk.len() {
            let dead = chunk[done].to_string_lossy().into_owned();
            results.lock().expect("results").push(json!({"file": dead, "test": dead, "status": "not_run", "cause": "the worker died on this trace"}));
            chunk.drain(..=done);
        } else {
            chunk.clear();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let option = |name: &str| args.iter().position(|arg| arg == name).and_then(|index| args.get(index + 1)).cloned();
    if args.first().map(String::as_str) == Some("--worker") {
        let text = std::fs::read_to_string(&args[1]).expect("the list of traces");
        let mut worker = trace::run::Worker::new();
        let stdout = std::io::stdout();
        for path in text.lines() {
            let result = worker.run_file(Path::new(path), false);
            let mut sink = stdout.lock();
            let _ = writeln!(sink, "{result}");
            let _ = sink.flush();
        }
        worker.clean();
        return;
    }
    if args.first().map(String::as_str) == Some("--all") {
        let directory = args.get(1).expect("the folder of the traces");
        let jobs = option("--jobs").and_then(|text| text.parse().ok()).unwrap_or(4);
        let report = option("--report").map(PathBuf::from);
        let filter = option("--filter");
        std::process::exit(run_all(Path::new(directory), jobs, report.as_deref(), filter.as_deref()));
    }
    let Some(path) = args.first() else {
        eprintln!("usage: ks-trace <trace.json.gz> | --all <dir> [--jobs N] [--report file] [--filter text]");
        std::process::exit(2);
    };
    let verbose = args.iter().any(|arg| arg == "--verbose");
    let mut worker = trace::run::Worker::new();
    let result = worker.run_file(Path::new(path), verbose);
    worker.clean();
    println!("{}", serde_json::to_string_pretty(&result).unwrap_or_default());
    std::process::exit(i32::from(result["status"] != "passed"));
}

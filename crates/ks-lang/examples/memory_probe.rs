//! Loads all the language models together, as the engine does, and prints what that cost:
//! `/usr/bin/time -v memory_probe <resources>`; the steady RSS is the value printed after loading.

use ks_lang::ortho_model::OrthoEvidence;
use ks_lang::Resources;

fn status(name: &str) -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| text.lines().find(|line| line.starts_with(name)).map(|line| line[name.len() + 1..].trim().to_string()))
        .unwrap_or_default()
}

fn main() {
    let dir = std::env::args().nth(1).expect("the resources folder");
    let started = std::time::Instant::now();
    let resources = Resources::new(dir);
    let en = resources.engine_language_model("en_US").expect("en");
    let ru = resources.engine_language_model("ru_RU").expect("ru");
    let indexes = (resources.prefix_index(&en), resources.prefix_index(&ru));
    let (ortho, status_text) = resources.ortho_model();
    let ortho = ortho.expect("ortho");
    let lexicon = resources.identifier_lexicon().0.expect("identifiers");
    let policy = resources.boundary_policy().expect("policy");
    let score = ortho.score(&OrthoEvidence { keys: "ghbdtn", shape: "lower", source_script: "en" }, None);
    println!(
        "loaded in {:?}: en {} ru {} words, index {} {}, ortho {status_text} total {}, identifiers {}, boundary {}",
        started.elapsed(),
        en.frequencies.len(),
        ru.frequencies.len(),
        indexes.0.len(),
        indexes.1.len(),
        score.total,
        lexicon.identifiers.len(),
        policy.version
    );
    println!("VmRSS {}  VmHWM {}", status("VmRSS:"), status("VmHWM:"));
}

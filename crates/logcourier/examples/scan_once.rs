//! One scan with the settings of a data folder, printing what the queue then holds: a check against files of other versions.
use logcourier::collector::Collector;
use logcourier::config::load_config;
use logcourier::store::Store;

fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("data folder"));
    let config = load_config(&root).expect("settings");
    let store = Store::open(&root).expect("store");
    let (count, errors) = Collector::new(&store).scan(&config).expect("scan");
    let rows = store.queue(&config.destination()).expect("queue");
    println!("new {count} errors {errors:?} queue {}", rows.len());
    if let Some(output) = std::env::args().nth(2) {
        let last = rows.iter().rev().find(|row| row.name.starts_with("lc-2")).expect("a fragment");
        std::fs::write(output, last.payload.as_ref().expect("payload")).expect("write");
    }
}

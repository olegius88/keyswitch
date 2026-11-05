//! Coalesce unsent legacy fragments transactionally, without changing their bytes.
use ks_cast::To as _;
use serde_json::{Value, json};
use sqlite::params;

use crate::config::Config;
use crate::constants::*;
use crate::error::{Error, Result};
use crate::store::{Bundle, Store, dumps};
use crate::timeutil::{iso_now, new_id, sha256_hex};
use crate::versions::MARKER_KIND;
use crate::zipfile::ZipWriter;

pub fn compact(store: &Store, config: &Config) -> Result<usize> {
    compact_with(store, config, MAX_DOWNLOAD, MAX_FRAGMENTS_PER_BATCH)
}

pub fn compact_with(store: &Store, config: &Config, limit: usize, max_fragments: usize) -> Result<usize> {
    let destination = config.destination();
    let rows: Vec<Bundle> = {
        let mut statement = store.db.prepare("SELECT * FROM bundles WHERE destination=? AND indexed=0 AND file_id IS NULL ORDER BY created,id")?;
        let found = statement.query_map([&destination], Bundle::from_row)?;
        found.collect::<sqlite::Result<_>>()?
    };
    struct Group {
        key: (Value, Value),
        selected: Vec<usize>,
        estimated: usize,
        full: bool,
    }
    let mut groups: Vec<Group> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let meta = row.metadata();
        if meta["batch"].as_bool().unwrap_or(false) || meta["kind"].as_str() == Some(MARKER_KIND) {
            continue;
        }
        let version = meta["keyswitch_version"].clone();
        let truthy = version.as_str().is_some_and(|text| !text.is_empty());
        let key = (if truthy { meta["source_id"].clone() } else { Value::Null }, version);
        let position = match groups.iter().position(|group| group.key == key) {
            Some(position) => position,
            None => {
                groups.push(Group { key, selected: Vec::new(), estimated: ZIP_MANIFEST_OVERHEAD_BYTES, full: false });
                groups.len() - 1
            }
        };
        let group = &mut groups[position];
        if group.full {
            continue;
        }
        // ZIP_STORED: payload lengths plus conservative per-entry/manifest overhead.
        let cost = row.payload.as_ref().map_or(0, Vec::len) + row.meta.len() + ZIP_ENTRY_OVERHEAD_BYTES;
        if group.estimated + cost > limit || group.selected.len() >= max_fragments {
            group.full = true;
            continue;
        }
        group.selected.push(index);
        group.estimated += cost;
    }
    let Some(chosen) = groups.iter().find(|group| group.selected.len() >= MIN_FRAGMENTS_TO_COMPACT) else {
        return Ok(0);
    };
    let selected: Vec<&Bundle> = chosen.selected.iter().map(|index| &rows[*index]).collect();
    let mut archive = ZipWriter::new();
    let mut manifest = Vec::new();
    for row in &selected {
        let mut meta = row.metadata();
        let name = format!("fragments/{}.zip", row.id);
        archive.write(&name, row.payload.as_deref().unwrap_or_default(), false);
        meta["archive"] = json!(name);
        manifest.push(meta);
    }
    archive.write("manifest.json", dumps(&json!({"schema": 1, "fragments": manifest}), false).as_bytes(), false);
    let payload = archive.finish();
    if payload.len() > limit {
        return Err(Error::value("Пакет превысил допустимый размер; исходная очередь сохранена."));
    }
    let identifier = new_id();
    let mut meta = json!({
        "schema": BATCH_SCHEMA_VERSION, "batch": true, "bundle_id": identifier, "device_id": config.device_id, "device_name": config.device_name,
        "source_label": format!("Пакет из {} фрагментов", selected.len()), "fragments": selected.len(), "created_at": iso_now(),
        "sha256": sha256_hex(&payload), "size": payload.len(),
    });
    let first = selected[0].metadata();
    meta["keyswitch_version"] = first["keyswitch_version"].clone();
    if first["keyswitch_version"].as_str().is_some_and(|text| !text.is_empty()) {
        meta["source_id"] = first["source_id"].clone();
    }
    let transaction = store.db.unchecked_transaction()?;
    let before: usize = selected.iter().map(|row| row.payload.as_ref().map_or(0, Vec::len)).sum();
    if store.queue_bytes()? - before.to::<i64>() + payload.len().to::<i64>() > store.capacity {
        return Err(Error::QueueFull("Для упаковки не хватает места в очереди. Фрагменты сохранены.".to_owned()));
    }
    for row in &selected {
        if store.db.execute("DELETE FROM bundles WHERE id=? AND file_id IS NULL AND indexed=0", [&row.id])? != 1 {
            return Err(Error::Runtime("Очередь изменилась во время упаковки; упаковка отменена.".to_owned()));
        }
    }
    store.db.execute(
        "INSERT INTO bundles(id,destination,created,name,meta,payload) VALUES (?,?,?,?,?,?)",
        params![identifier, destination, selected[0].created, format!("lc-batch-{identifier}.zip"), dumps(&meta, false), payload],
    )?;
    transaction.commit()?;
    Ok(selected.len())
}

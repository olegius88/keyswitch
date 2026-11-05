//! `KeySwitch` provenance from log headers, never from installed binaries or message text.
use std::io::{Read, Seek, SeekFrom};

use serde_json::{Map, Value, json};

use crate::config::{Config, Source};
use crate::constants::{PROBE_BYTES, HEADER_BYTES, JSON_INDENT_SPACES};
use crate::error::Result;
use crate::store::Store;
use crate::timeutil::{iso_now, new_id, sha256_hex};

pub const MARKER_KIND: &str = "keyswitch.version";
const TIMESTAMP_SHAPE: &[u8] = b"0000-00-00 00:00:00,000 ";
const LEVELS: [&[u8]; 5] = [b"DEBUG", b"INFO", b"WARNING", b"ERROR", b"CRITICAL"];

/// `[0-9]{1,6}\.[0-9]{1,6}\.[0-9]{1,6}(?:[-+][a-zA-Z0-9.-]{1,40})?` over the whole text.
#[must_use]
pub fn is_version(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0;
    for part in 0..3 {
        let digits = bytes[index..].iter().take_while(|byte| byte.is_ascii_digit()).count();
        if !(1..=6).contains(&digits) {
            return false;
        }
        index += digits;
        if part < 2 {
            if bytes.get(index) != Some(&b'.') {
                return false;
            }
            index += 1;
        }
    }
    if index == bytes.len() {
        return true;
    }
    if !matches!(bytes[index], b'-' | b'+') {
        return false;
    }
    let tail = &bytes[index + 1..];
    (1..=40).contains(&tail.len()) && tail.iter().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

/// Whether a record's time stamp stands at the start of these bytes.
fn record_start(data: &[u8]) -> bool {
    data.len() >= TIMESTAMP_SHAPE.len()
        && data.iter().zip(TIMESTAMP_SHAPE).all(|(byte, shape)| if *shape == b'0' { byte.is_ascii_digit() } else { byte == shape })
}

/// The version of a log record at the start of these bytes, when they are a `KeySwitch` record.
#[must_use]
pub fn header_version(data: &[u8]) -> Option<String> {
    if !record_start(data) {
        return None;
    }
    let rest = &data[TIMESTAMP_SHAPE.len()..];
    let length = rest.iter().take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-')).count();
    if length == 0 || rest.get(length) != Some(&b' ') {
        return None;
    }
    let after = &rest[length + 1..];
    let level = LEVELS.iter().find(|level| after.starts_with(level))?;
    let mut logger = after[level.len()..].strip_prefix(b" keyswitch")?;
    while let Some(tail) = logger.strip_prefix(b".") {
        let part = tail.iter().take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_').count();
        if part == 0 {
            return None;
        }
        logger = &tail[part..];
    }
    if !logger.starts_with(b": ") {
        return None;
    }
    let version = std::str::from_utf8(&rest[..length]).ok()?;
    is_version(version).then(|| version.to_owned())
}

/// Bounded tail probe; an unrecognized last record is not assigned an old version.
pub fn latest_version<S: Read + Seek>(stream: &mut S) -> Result<Option<String>> {
    let size = stream.seek(SeekFrom::End(0))?;
    let start = size.saturating_sub(PROBE_BYTES);
    stream.seek(SeekFrom::Start(start))?;
    let mut data = Vec::new();
    stream.by_ref().take(PROBE_BYTES).read_to_end(&mut data)?;
    if start > 0 {
        // The first bytes may be the middle of an arbitrarily long log message.
        data = match data.iter().position(|byte| *byte == b'\n') {
            Some(newline) => data[newline + 1..].to_vec(),
            None => Vec::new(),
        };
    }
    let mut last = None;
    let mut position = 0;
    loop {
        if record_start(&data[position..]) {
            last = Some(position);
        }
        match data[position..].iter().position(|byte| *byte == b'\n') {
            Some(newline) => position += newline + 1,
            None => break,
        }
    }
    Ok(last.and_then(|at| header_version(&data[at..(at + HEADER_BYTES).min(data.len())])))
}

/// How many leading bytes hold one version, and that version; the bytes are kept exactly, partial UTF-8 included.
///
/// Lookahead only identifies the bounded header; it is not consumed. Cursor context survives a chunk or restart in the
/// middle of a record or traceback. An unfinished header waits for more bytes instead of inheriting an old version.
#[must_use]
pub fn take_fragment(data: &[u8], previous: Option<String>, mut line_start: bool, lookahead: &[u8]) -> (usize, Option<String>) {
    let combined: Vec<u8> = [data, lookahead].concat();
    let mut version = previous;
    let mut offset = 0;
    while offset < data.len() {
        if line_start {
            let prefix = &combined[offset..(offset + HEADER_BYTES).min(combined.len())];
            let found = header_version(prefix);
            if found.is_none() && !prefix.contains(&b'\n') && prefix.len() < HEADER_BYTES {
                // Even a single digit can be the start of the next version's timestamp. Do not consume it before the
                // writer finishes the header.
                let shaped = prefix.iter().zip(TIMESTAMP_SHAPE).all(|(byte, shape)| if *shape == b'0' { byte.is_ascii_digit() } else { byte == shape });
                if shaped {
                    return (offset, version);
                }
            }
            let candidate = if found.is_some() || record_start(prefix) { found } else { version.clone() };
            if offset > 0 && candidate != version {
                return (offset, version);
            }
            version = candidate;
        }
        match data[offset..].iter().position(|byte| *byte == b'\n') {
            Some(newline) => {
                offset += newline + 1;
                line_start = true;
            }
            None => break,
        }
    }
    (data.len(), version)
}

pub fn current_versions(store: &Store, config: &Config) -> Map<String, Value> {
    let values = store.get(&format!("keyswitch_versions:{}", config.destination())).unwrap_or_else(|| json!({}));
    // Removed sources and a previous collector's sources are not current inputs.
    config.sources.iter().filter_map(|source| values.get(&source.id).map(|state| (source.id.clone(), state.clone()))).collect()
}

pub fn observe_version(store: &Store, config: &Config, source: &Source, version: &str) -> Result<()> {
    let key = format!("keyswitch_versions:{}", config.destination());
    let mut values = current_versions(store, config);
    let previous = values.get(&source.id).and_then(|state| state.get("version")).cloned().unwrap_or(Value::Null);
    if previous.as_str() == Some(version) {
        return Ok(());
    }
    let identifier = new_id();
    let created = iso_now();
    let state = json!({"version": version, "previous_version": previous, "marker_id": identifier, "detected_at": created, "source_label": source.label});
    let mut metadata = json!({
        "schema": 1, "kind": MARKER_KIND, "bundle_id": identifier, "device_id": config.device_id, "device_name": config.device_name,
        "source_id": source.id, "source_label": source.label, "created_at": created, "keyswitch_version": version, "previous_version": previous,
    });
    let options = ks_text::JsonOptions { ensure_ascii: false, indent: Some(ks_text::Indent::Spaces(JSON_INDENT_SPACES)), ..ks_text::JsonOptions::default() };
    let payload = ks_text::json_dumps(&metadata, &options).into_bytes();
    metadata["size"] = json!(payload.len());
    metadata["sha256"] = json!(sha256_hex(&payload));
    values.insert(source.id.clone(), state);
    // Both the marker and current pointer commit together, even when the queue fills.
    store.enqueue_event(&config.destination(), &metadata, &payload, &format!("lc-version-{identifier}.json"), &[(key, Value::Object(values))])
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".to_owned(),
        other @ Value::Bool(_) | other @ Value::Number(_) | other @ Value::Array(_) | other @ Value::Object(_) => other.to_string(),
    }
}

#[must_use]
pub fn caption(metadata: &Value) -> String {
    let version = &metadata["keyswitch_version"];
    let truthy = |value: &Value| value.as_str().is_some_and(|text| !text.is_empty());
    if metadata["kind"].as_str() == Some(MARKER_KIND) {
        let previous = &metadata["previous_version"];
        let title = if truthy(previous) {
            format!("KeySwitch: смена версии {} → {}", text(previous), text(version))
        } else {
            format!("KeySwitch: определена версия {}", text(version))
        };
        return format!(
            "МАРКЕР ВЕРСИИ · {title}\n{} · {}\nДля анализа — только архивы с этой версией. Старая очередь может прийти позже; используйте фильтр версии, а не положение сообщения в чате.\n{}\nID: {}",
            text(&metadata["device_name"]), text(&metadata["source_label"]), text(&metadata["created_at"]), text(&metadata["bundle_id"]),
        );
    }
    let label = if truthy(version) { format!("KeySwitch {}", text(version)) } else { "версия KeySwitch не определена".to_owned() };
    format!(
        "LogCourier · {} · {}\n{label}\n{}\nID: {}",
        text(&metadata["device_name"]), text(&metadata["source_label"]), text(&metadata["created_at"]), text(&metadata["bundle_id"]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &[u8] = b"2026-11-04 12:00:00,123 0.66.1 INFO keyswitch.engine: x\n";
    const NEXT: &[u8] = b"2026-11-04 12:00:01,000 0.70.0 INFO keyswitch: y\n";

    #[test]
    fn versions_and_headers_are_read_as_the_patterns_say() {
        assert!(is_version("0.66.1") && is_version("1.2.3-rc.1") && !is_version("1.2") && !is_version("1.2.3-"));
        assert_eq!(header_version(LINE).as_deref(), Some("0.66.1"));
        assert_eq!(header_version(b"2026-11-04 12:00:00,123 0.66.1 INFO other: x\n"), None);
        assert_eq!(header_version(b"not a record"), None);
    }

    #[test]
    fn a_fragment_ends_where_the_version_changes() {
        let data = [LINE, NEXT].concat();
        assert_eq!(take_fragment(&data, None, true, b""), (LINE.len(), Some("0.66.1".to_owned())));
        assert_eq!(take_fragment(NEXT, Some("0.66.1".to_owned()), true, b""), (NEXT.len(), Some("0.70.0".to_owned())));
    }

    #[test]
    fn a_continuation_keeps_the_version_and_an_unfinished_header_waits() {
        assert_eq!(take_fragment(b"Traceback\n", Some("0.66.1".to_owned()), true, b""), (10, Some("0.66.1".to_owned())));
        assert_eq!(take_fragment(b"2026-1", Some("0.66.1".to_owned()), true, b""), (0, Some("0.66.1".to_owned())));
    }

    #[test]
    fn the_last_record_names_the_version() {
        let mut stream = std::io::Cursor::new([LINE, NEXT, b"continuation\n"].concat());
        assert_eq!(latest_version(&mut stream).expect("probe").as_deref(), Some("0.70.0"));
    }
}

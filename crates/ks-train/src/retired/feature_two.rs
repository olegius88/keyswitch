//! Whether feature schema 2 still computes what the code that fitted context-v2 computed: declarations aside, its softmax,
//! its features, its loader and its predictions, on the pinned probe plan. The answers were written down once by that code
//! (`model/context_v2/compatibility/feature-two-observations.json.gz`), and this compares with them to the bit.
use ks_cast::To as _;
use crate::physical::hex;
use flate2::read::MultiGzDecoder;
use ks_context::context_action_features::extract_baseline_features;
use ks_context::context_model::{ContextEvidence, ContextModel, FieldContext, softmax};
use ks_context::input_context::FieldRole;
use ks_text::pynum::float_hex;
use ks_text::{JsonOptions, json_dumps};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

const OBSERVATIONS: &str = "model/context_v2/compatibility/feature-two-observations.json.gz";
const PLAN_SHA256: &str = "d990630577f8e6eb3622ac0b77fd7665184cff1f028021f6d4c6a36d6111e740";
const BASELINE: &str = "model/context_v2/baseline-context-v1.json";
const CANDIDATE: &str = "model/context_v2/candidate.json";
/// The plan probes a role spelled so, which no typed field can hold; those records are not compared.
const UNTYPED_ROLE: &str = "Unknown";

fn parse_hex(text: &str) -> Result<f64, String> {
    ks_text::pynum::float_fromhex(text).ok_or_else(|| format!("not a hexadecimal float: {text}"))
}

fn role_of(name: &str) -> FieldRole {
    match name {
        "text" => FieldRole::Text,
        "code" => FieldRole::Code,
        "search" => FieldRole::Search,
        "terminal" => FieldRole::Terminal,
        "password" => FieldRole::Password,
        _ => FieldRole::Unknown,
    }
}

fn evidence_of(record: &Value) -> Result<ContextEvidence, String> {
    let text = |name: &str| record[name].as_str().map(str::to_owned).ok_or_else(|| format!("a record without {name}"));
    let mut field = FieldContext::new(text("application")?, text("field_id")?);
    field.before = text("before")?;
    field.after = text("after")?;
    field.role = role_of(&text("role")?);
    let mut item = ContextEvidence::new(&text("original")?, &text("alternative")?, record["source_group"].as_i64().ok_or("a record without a group")?, field);
    item.trigger = text("trigger")?;
    item.baseline_convert = record["baseline_convert"].as_bool().unwrap_or(false);
    item.source_known = record["source_known"].as_bool().unwrap_or(false);
    item.target_known = record["target_known"].as_bool().unwrap_or(false);
    item.score_delta = parse_hex(&text("score_delta")?)?;
    Ok(item.finished())
}

/// The model's weights as the archive describes them: names in the file's order, then every weight's bits.
fn describe(model: &ContextModel, payload: &Value) -> Option<Value> {
    let names: Vec<&String> = payload.get("weights")?.as_object()?.keys().collect();
    let mut digest = Sha256::new();
    let joined = names.iter().map(|name| name.as_str()).collect::<Vec<&str>>().join("\0");
    digest.update(joined.as_bytes());
    for name in names {
        for weight in model.weights.get(name)? {
            digest.update(weight.to_le_bytes());
        }
    }
    Some(json!({"weights": hex(&digest.finalize()), "version": model.version, "threshold": float_hex(model.conversion_threshold)}))
}

fn compact(sort_keys: bool) -> JsonOptions {
    JsonOptions { ensure_ascii: false, separators: Some((",".to_owned(), ":".to_owned())), sort_keys, ..JsonOptions::default() }
}

fn signature(weights: &Value) -> String {
    hex(&Sha256::digest(json_dumps(weights, &compact(true)).as_bytes()))
}

/// The artifacts too large to store with the plan, rebuilt the way the plan built them; their digests are recorded.
fn bulky_content(label: &str, base: &[u8], baseline: &[u8]) -> Result<Vec<u8>, String> {
    if label.starts_with("baseline ") {
        let mut historical: Value = serde_json::from_slice(baseline).map_err(|error| error.to_string())?;
        let table = historical["weights"].as_object_mut().ok_or("a baseline without weights")?;
        let first = table.keys().next().ok_or("empty baseline weights")?.clone();
        let row = table[&first].as_array_mut().ok_or("a baseline row")?;
        row[0] = json!(row[0].as_f64().ok_or("a baseline weight")?.next_up());
        if label == "baseline resigned" {
            let digest = signature(&historical["weights"]);
            historical["weights_sha256"] = json!(digest);
        }
        let spaced = JsonOptions { ensure_ascii: false, ..JsonOptions::default() };
        return Ok(json_dumps(&historical, &spaced).into_bytes());
    }
    if let Some(count) = label.strip_prefix("features ") {
        let count: usize = count.parse().map_err(|_| "a feature count")?;
        let mut value: Value = serde_json::from_slice(base).map_err(|error| error.to_string())?;
        let width = value["actions"].as_array().map_or(0, Vec::len);
        let zeros = vec![0.0_f64; width];
        let weights: Map<String, Value> = (0..count).map(|index| (format!("f{index}"), json!(zeros))).collect();
        value["weights"] = Value::Object(weights);
        let digest = signature(&value["weights"]);
        value["weights_sha256"] = json!(digest);
        return Ok(json_dumps(&value, &compact(false)).into_bytes());
    }
    if label.starts_with("bytes ") {
        return Ok(base.to_vec());
    }
    Err(format!("an unknown large probe: {label}"))
}

/// Whether the loader of today agrees with the archived one on a probe.
fn compare_load(index: usize, load: &Value, content: &[u8], folder: &Path, models: &Path) -> Result<(), String> {
    let path = folder.join(format!("payload-{index}.json"));
    let padding = (load["padding"].as_u64().unwrap_or(0)).to::<usize>();
    let mut bytes = content.to_vec();
    bytes.resize(content.len() + padding, b' ');
    std::fs::write(&path, &bytes).map_err(|error| error.to_string())?;
    let found = ContextModel::load_artifact(&path, models).ok();
    let label = load["label"].as_str().unwrap_or("");
    let declares = load["feature_two"].as_bool().unwrap_or(false);
    let found = found.filter(|model| declares || model.feature_version == 2);
    let answer = match &found {
        Some(model) => {
            let text = content.strip_prefix(b"\xef\xbb\xbf").unwrap_or(content);
            let payload: Value = serde_json::from_slice(text).map_err(|error| error.to_string())?;
            describe(model, &payload).unwrap_or(Value::Null)
        }
        None => Value::Null,
    };
    if declares {
        if answer != load["loaded"] {
            return Err(format!("feature-2 loader changed: {label}: {answer} against {}", load["loaded"]));
        }
    } else if !answer.is_null() && answer != load["loaded"] {
        return Err(format!("feature-2 loader loads what the archive did not: {label}"));
    }
    Ok(())
}

pub fn verify(root: &Path) -> Result<(), String> {
    let mut raw = Vec::new();
    MultiGzDecoder::new(std::fs::File::open(root.join(OBSERVATIONS)).map_err(|error| error.to_string())?).read_to_end(&mut raw).map_err(|error| error.to_string())?;
    let observed: Value = serde_json::from_slice(&raw).map_err(|error| error.to_string())?;
    if observed["plan_sha256"].as_str() != Some(PLAN_SHA256) {
        return Err("feature-2 probe plan changed".to_owned());
    }
    for (index, row) in observed["softmax"].as_array().ok_or("no softmax probes")?.iter().enumerate() {
        let scores: Vec<f64> = row["scores"].as_array().ok_or("a softmax probe")?.iter().map(|value| parse_hex(value.as_str().unwrap_or(""))).collect::<Result<_, _>>()?;
        let result = softmax(&scores);
        let expected = row["result"].as_array().ok_or("a softmax answer")?;
        let same = expected.iter().zip(result.iter()).all(|(wanted, value)| wanted.as_str() == Some(float_hex(*value).as_str()));
        if !same || expected.len() != scores.len() {
            return Err(format!("feature-2 numerical behaviour changed: softmax #{index}"));
        }
    }
    let records = observed["records"].as_array().ok_or("no records")?;
    let evidence: Vec<Option<ContextEvidence>> = records.iter().map(|record| if record["role"] == UNTYPED_ROLE { Ok(None) } else { evidence_of(record).map(Some) }).collect::<Result<_, _>>()?;
    let digests = observed["features"].as_array().ok_or("no feature digests")?;
    for (index, (item, digest)) in evidence.iter().zip(digests).enumerate() {
        let Some(item) = item else { continue };
        let mut text = String::new();
        for (name, value) in extract_baseline_features(item) {
            text.push_str(&format!("{name}={}\n", float_hex(value)));
        }
        if digest.as_str() != Some(hex(&Sha256::digest(text.as_bytes())).as_str()) {
            return Err(format!("feature-2 numerical behaviour changed: extract_context_features #{index}"));
        }
    }
    let models = root.join("resources/models");
    for (label, relative) in [("baseline-context-v1.json", BASELINE), ("candidate.json", CANDIDATE)] {
        let path = root.join(relative);
        let model = ContextModel::load_artifact(&path, &models).map_err(|error| format!("historical feature-2 artifact cannot be loaded: {label}: {error}"))?;
        let payload: Value = serde_json::from_slice(&std::fs::read(&path).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
        let expected = &observed["predictions"][label];
        if describe(&model, &payload).as_ref() != Some(&expected["model"]) {
            return Err(format!("feature-2 numerical behaviour changed: load {label}"));
        }
        let mut rows = Vec::with_capacity(evidence.len());
        for item in evidence.iter().flatten() {
            let answer = model.predict(item);
            let mut fields = vec![answer.action.as_str().to_owned(), float_hex(answer.probability)];
            fields.extend(answer.probabilities.iter().map(|value| float_hex(*value)));
            fields.push(if answer.supported { "True" } else { "False" }.to_owned());
            rows.push(fields.join("|"));
        }
        if expected["digest"].as_str() != Some(hex(&Sha256::digest(rows.join("\n").as_bytes())).as_str()) {
            return Err(format!("feature-2 numerical behaviour changed: {label} predict"));
        }
    }
    let baseline = std::fs::read(root.join(BASELINE)).map_err(|error| error.to_string())?;
    let loads = observed["loads"].as_array().ok_or("no loader probes")?;
    let folder = std::env::temp_dir().join(format!("keyswitch-feature-two-{}", std::process::id()));
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let base = decode(loads.first().and_then(|load| load["content"].as_str()).ok_or("no base probe")?)?;
    let result = (|| -> Result<(), String> {
        for (index, load) in loads.iter().enumerate() {
            let label = load["label"].as_str().unwrap_or("");
            let content = match load["content"].as_str() {
                Some(text) => decode(text)?,
                None => bulky_content(label, &base, &baseline)?,
            };
            if load["sha256"].as_str() != Some(hex(&Sha256::digest(&content)).as_str()) {
                return Err(format!("feature-2 probe differs from the plan: {label}"));
            }
            compare_load(index, load, &content, &folder, &models)?;
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&folder);
    result
}

/// Standard base64: the probes hold bytes that are no JSON text at all.
fn decode(text: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    let (mut buffer, mut bits) = (0_u32, 0_u32);
    for character in text.bytes().filter(|byte| *byte != b'=') {
        let value = match character {
            b'A'..=b'Z' => character - b'A',
            b'a'..=b'z' => character - b'a' + 26,
            b'0'..=b'9' => character - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return Err("invalid base64".to_owned()),
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits).to::<u8>());
            buffer &= (1 << bits) - 1;
        }
    }
    Ok(bytes)
}

//! Every stage of a fit written down, one line per frame, floats as C99 hex: what a comparison with another trainer reads.
use flate2::Compression;
use flate2::write::GzEncoder;
use ks_backend::FieldContext;
use ks_context::context_model::ContextEvidence;
use ks_train::fit::{Columns, Frame, Stages};
use ks_train::rows::ActionRow;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Writer {
    folder: PathBuf,
}

impl Writer {
    pub fn new(folder: &Path) -> Writer {
        std::fs::create_dir_all(folder).expect("a folder for the stages");
        Writer { folder: folder.to_path_buf() }
    }

    fn write(&self, name: &str, lines: impl Iterator<Item = String>) {
        let file = std::fs::File::create(self.folder.join(format!("{name}.gz"))).expect("a stage file");
        let mut stream = GzEncoder::new(file, Compression::new(3));
        let mut hasher = Sha256::new();
        let mut count = 0;
        for line in lines {
            hasher.update(line.as_bytes());
            hasher.update(b"\n");
            stream.write_all(line.as_bytes()).expect("a stage file");
            stream.write_all(b"\n").expect("a stage file");
            count += 1;
        }
        stream.finish().expect("a stage file");
        println!("STAGE {name} {count} {}", ks_train::physical::hex(&hasher.finalize()));
    }
}

fn dumped(value: &Value) -> String {
    // `json.dumps(value, ensure_ascii=False)`: ", " and ": " between items.
    ks_text::json_dumps(value, &ks_text::JsonOptions { ensure_ascii: false, ..ks_text::JsonOptions::default() })
}

/// `float.hex()`.
fn hex_of(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.to_owned();
    }
    let sign = if value.is_sign_negative() { "-" } else { "" };
    let bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i64;
    let mantissa = bits & ((1 << 52) - 1);
    if exponent == 0 && mantissa == 0 {
        return format!("{sign}0x0.0p+0");
    }
    let (lead, power) = if exponent == 0 { (0, -1022) } else { (1, exponent - 1023) };
    format!("{sign}0x{lead}.{mantissa:013x}p{power:+}")
}

fn field_list(field: &FieldContext) -> Vec<Value> {
    vec![json!(field.application), json!(field.field_id), json!(field.before), json!(field.after), json!(field.role.as_str()), json!(field.selection), json!(field.sensitive), json!(field.source)]
}

fn row_line(row: &ActionRow) -> String {
    let mut items = vec![json!(row.identifier), json!(row.original), json!(row.group)];
    items.extend(field_list(&row.field));
    items.extend([
        json!(row.trigger), json!(row.literal_tail), json!(row.action.as_str()), json!(row.category), json!(row.boundary_text), json!(hex_of(row.sample_weight)),
        json!(row.after_origin.as_str()), json!(row.parent_family), json!(row.revisit), json!(row.before_origin.as_str()),
    ]);
    dumped(&Value::Array(items))
}

fn optional_hex(value: Option<f64>) -> Value {
    value.map_or(Value::Null, |value| json!(hex_of(value)))
}

fn evidence_list(item: &ContextEvidence) -> Vec<Value> {
    let mut items = vec![json!(item.original), json!(item.alternative), json!(item.source_group)];
    items.extend(field_list(&item.field));
    items.extend([
        json!(item.trigger), json!(item.baseline_convert), json!(item.source_known), json!(item.target_known), json!(hex_of(item.score_delta)), optional_hex(item.model_probability),
        optional_hex(item.model_threshold), json!(item.literal_tail), optional_hex(item.ortho_score), optional_hex(item.ortho_threshold), json!(item.boundary_text),
        json!(item.after_origin.as_str()), json!(item.before_origin.as_str()), json!(item.source_identifier), json!(item.target_identifier), json!(item.source_typo), json!(item.target_typo),
        json!(item.source_opening), json!(item.target_opening), json!(item.inside), json!(item.term_counts_dropped),
    ]);
    items
}

fn frame_line(frame: &Frame) -> String {
    match frame {
        Frame::Row(row) => row_line(row),
        Frame::Span(span) => {
            let mut items = vec![json!("span"), json!(span.sequence_id), json!(span.parent_family), json!(span.source_identifier), json!(span.action.as_str()), json!(hex_of(span.sample_weight))];
            items.extend(evidence_list(&span.evidence));
            dumped(&Value::Array(items))
        }
    }
}

impl Stages for Writer {
    fn rows(&mut self, name: &str, rows: &[ActionRow]) {
        self.write(name, rows.iter().map(row_line));
    }

    fn frames(&mut self, name: &str, frames: &[Frame]) {
        self.write(name, frames.iter().map(frame_line));
    }

    fn features(&mut self, name: &str, columns: &Columns, names: &[String]) {
        let mut position = 0;
        let lines: Vec<String> = columns
            .counts
            .iter()
            .enumerate()
            .map(|(row, count)| {
                let end = position + *count as usize;
                let features: Vec<String> = (position..end).map(|entry| format!("{}={}", names[columns.ids[entry] as usize], hex_of(columns.values[entry]))).collect();
                position = end;
                format!("{}|{}|{}|{}", columns.labels[row], hex_of(columns.importance[row]), hex_of(columns.weights[row]), features.join(";"))
            })
            .collect();
        self.write(name, lines.into_iter());
    }
}

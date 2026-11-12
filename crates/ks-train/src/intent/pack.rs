//! The layout-intent model as the program is packaged with it.
//!
//! The fit writes the sealed model to `SEALED_ARTIFACT`: the file the candidate's identity, the manifest and the test report
//! name. It holds a table of fingerprints that gives only the `coverage` of a prediction, which the evaluation of the trainer
//! reports and no decision of the program reads, and the table is two thirds of the file. The program is packaged with
//! `PACKAGED_ARTIFACT`: the sealed model without that table (`pack_model`), every weight and parameter the same to the byte.
//! `PACKAGING_RECORD` names both files by their digests; the gate of a package, the replay and the strict evaluation hold
//! the packaged file to the sealed one.
//!
//! This module is no part of the toolchain a seal pins (`seal::TOOLCHAIN_FILES`): it writes nothing a seal describes.
use ks_cast::To as _;
use super::seal::{self, KSLM_HEADER_BYTES, still};
use crate::artifact::canonical;
use crate::physical::hex;
use ks_lang::intent_model::LinearNgramModel;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The sealed model, as the fit wrote it: evidence, never packaged.
pub const SEALED_ARTIFACT: &str = "model/intent_v1/layout_intent_v1.ksm";
/// The model the program loads: `pack_model` of the sealed one.
pub const PACKAGED_ARTIFACT: &str = "resources/models/layout_intent_v1.ksm";
/// Which file is packaged from which, by their digests.
pub const PACKAGING_RECORD: &str = "model/intent_v1/packaged-model.json";
const KSLM_MANIFEST_LENGTH_AT: usize = 8;
const KSLM_PAYLOAD_LENGTH_AT: usize = 12;
const KSLM_WEIGHT_BYTES: usize = 2;
const KSLM_FINGERPRINT_BYTES: usize = 8;
const RECORD_SCHEMA_VERSION: i64 = 1;

/// A length of the header of a container that loads: four bytes, little-endian.
fn header_length(container: &[u8], at: usize) -> usize {
    u32::from_le_bytes([container[at], container[at + 1], container[at + 2], container[at + 3]]).to::<usize>()
}

/// What a decision reads of a model: every runtime parameter but the digest of the payload, which covers the
/// fingerprints too, and the weights themselves.
fn decision_parameters(model: &LinearNgramModel) -> (Value, Vec<i16>, Value) {
    let mut parameters = seal::runtime_parameters(model);
    if let Some(table) = parameters.as_object_mut() {
        table.remove("payload_sha256");
    }
    let weights = (0..model.dimension.to::<usize>()).map(|bucket| model.weight(bucket)).collect();
    (parameters, weights, model.metadata())
}

/// The sealed model as the program is packaged with it: the same weights, bias, calibration, thresholds, veto, seeds,
/// version and signed metadata, byte for byte, and no table of fingerprints. The manifest inside changes in two fields
/// only: the count of fingerprints, now zero, and the digest of the payload, now of the weights alone. A sealed manifest
/// this writer would write otherwise is refused: rewriting it would move a byte of what was signed.
pub fn pack_model(sealed: &[u8]) -> Result<Vec<u8>, String> {
    let model = LinearNgramModel::from_bytes(sealed.to_vec(), None).map_err(|error| format!("the sealed model does not load: {}", error.name))?;
    // A file that loads holds its header, its manifest and a payload of the lengths the header names.
    let payload_start = KSLM_HEADER_BYTES + header_length(sealed, KSLM_MANIFEST_LENGTH_AT);
    let weights = &sealed[payload_start..payload_start + model.dimension.to::<usize>() * KSLM_WEIGHT_BYTES];
    let manifest = packed_manifest(&sealed[KSLM_HEADER_BYTES..payload_start], weights)?;
    let packed = seal::container(&manifest, weights);
    let loaded = LinearNgramModel::from_bytes(packed.clone(), None).map_err(|error| format!("the packed model does not load: {}", error.name))?;
    still(decision_parameters(&loaded) == decision_parameters(&model) && loaded.model_version == model.model_version, "the packed model decides with other parameters than the sealed one")?;
    Ok(packed)
}

/// The manifest of a packed container, from the bytes of the sealed one and the weights it keeps: no fingerprints, and the
/// digest of the weights alone. The sealed manifest is written again whole, so it must be in the form this writer writes.
fn packed_manifest(sealed: &[u8], weights: &[u8]) -> Result<Value, String> {
    let mut manifest: Value = serde_json::from_slice(sealed).map_err(|error| format!("the sealed manifest: {error}"))?;
    let mut written_again = canonical(&manifest);
    written_again.pop();
    still(written_again == sealed, "the sealed manifest is not in the form this writer writes: packing it would rewrite what was signed")?;
    manifest["payload_sha256"] = json!(hex(&Sha256::digest(weights)));
    manifest["supported_fingerprint_count"] = json!(0);
    Ok(manifest)
}

/// The record of a packaging: the sealed file and the packaged one by size and digest, the version they share and how
/// many fingerprints the packaged one is without. Written beside the evidence, so that the bytes a package holds are
/// named in the repository and a change of them is a change someone reads.
pub fn packaging_record(sealed: &[u8], packed: &[u8]) -> Result<Vec<u8>, String> {
    let load = |bytes: &[u8], what: &str| LinearNgramModel::from_bytes(bytes.to_vec(), None).map_err(|error| format!("the {what} model does not load: {}", error.name));
    let (_, model) = (load(sealed, "sealed")?, load(packed, "packed")?);
    // The payload of the packed model is the weights; the sealed one holds its fingerprints after them.
    let left_out = header_length(sealed, KSLM_PAYLOAD_LENGTH_AT).saturating_sub(header_length(packed, KSLM_PAYLOAD_LENGTH_AT)) / KSLM_FINGERPRINT_BYTES;
    let file = |path: &str, bytes: &[u8]| json!({"path": path, "bytes": bytes.len(), "sha256": hex(&Sha256::digest(bytes))});
    let record = json!({
        "schema_version": RECORD_SCHEMA_VERSION,
        "model_version": model.model_version,
        "sealed": file(SEALED_ARTIFACT, sealed),
        "packaged": file(PACKAGED_ARTIFACT, packed),
        "fingerprints_left_out": left_out,
        "scope": "the packaged model is the sealed one without its table of fingerprints, which gives only the coverage of a prediction; every weight and parameter a decision reads is the same to the byte",
    });
    Ok((super::pretty(&record) + "\n").into_bytes())
}

/// What `--pack` and a fit publish beside the sealed model: the packaged model and its record.
pub fn packaged_outputs(root: &Path, sealed: &[u8]) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let packed = pack_model(sealed)?;
    let record = packaging_record(sealed, &packed)?;
    Ok(vec![(root.join(PACKAGED_ARTIFACT), packed), (root.join(PACKAGING_RECORD), record)])
}

/// The packaged model of a tree against a sealed model: the packaged file must be `pack_model` of the sealed bytes and the
/// record must name both, to the byte. Answers the SHA-256 of the sealed bytes and of the packaged file.
pub fn packaged_from(root: &Path, sealed: &[u8]) -> Result<(String, String), String> {
    let read = |name: &str| std::fs::read(root.join(name)).map_err(|error| format!("{name}: {error}"));
    let packed = pack_model(sealed)?;
    if read(PACKAGED_ARTIFACT)? != packed {
        return Err(format!("{PACKAGED_ARTIFACT} is not {SEALED_ARTIFACT} without its fingerprints: run `ks-train intent --pack`"));
    }
    if read(PACKAGING_RECORD)? != packaging_record(sealed, &packed)? {
        return Err(format!("{PACKAGING_RECORD} does not name the sealed and the packaged model of this tree: run `ks-train intent --pack`"));
    }
    Ok((hex(&Sha256::digest(sealed)), hex(&Sha256::digest(&packed))))
}

/// `packaged_from` with the sealed model of the tree.
pub fn packaged_from_sealed(root: &Path) -> Result<(String, String), String> {
    let sealed = std::fs::read(root.join(SEALED_ARTIFACT)).map_err(|error| format!("{SEALED_ARTIFACT}: {error}"))?;
    packaged_from(root, &sealed)
}

/// Whether a path names the sealed model of a tree: the same file, however the path is written.
#[must_use]
pub fn is_sealed_artifact(root: &Path, artifact: &Path) -> bool {
    let sealed = root.join(SEALED_ARTIFACT);
    match (artifact.canonicalize(), sealed.canonicalize()) {
        (Ok(given), Ok(sealed)) => given == sealed,
        _ => artifact == sealed,
    }
}

/// The check of a strict evaluation: the packaged file of the tree is the cut of the artifact that is evaluated, with its
/// record. What it says goes into the report.
#[must_use]
pub fn packaged_check(root: &Path, artifact: &Path) -> (bool, String) {
    let held = std::fs::read(artifact).map_err(|error| format!("{}: {error}", artifact.display())).and_then(|sealed| packaged_from(root, &sealed));
    match held {
        Ok((_, packaged)) => (true, format!("packaged={packaged}: the evaluated model without its fingerprints")),
        Err(error) => (false, error),
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::intent::fit::{Confusion, Platt, Quantized, Selection, Selections, Veto};
    use ks_lang::intent_model::TRIGGERS;
    use std::collections::BTreeSet;

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("ks-train-pack-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a folder");
        folder
    }

    fn sealed_model() -> (Vec<u8>, usize) {
        let quantized = Quantized { dimension: 8, scale: 0.5, values: vec![3, 0, -2, 0, 0, 1, 0, 0], support: vec![0, 2, 5], maximum_absolute_error: 0.0 };
        let selection = Selection { logits: [0.5, 0.25], metrics: Confusion::default(), typo_metrics: Confusion::default(), margin: 0.0 };
        let selections: Selections = TRIGGERS.iter().map(|trigger| (trigger.as_str(), (*trigger, selection))).collect();
        let platt = Platt { slope: 1.5, intercept: -0.25, sample_count: 2, positive_count: 1 };
        let veto = Veto { raw_logit: -1.0, positive_samples: 1, vetoed_positive_samples: 0, false_negative_rate: 0.0 };
        let metadata = json!({"note": "small", "share": 0.1, "seed": 18_446_744_073_709_551_615_u64});
        let fingerprints: BTreeSet<u64> = [5, 9, 12].into_iter().collect();
        let file = seal::ModelFile { version: "intent-v1-small", quantized: &quantized, fingerprints: &fingerprints, selections: &selections, veto: &veto, bias: 0.125, calibration: &[platt, platt], fnv_seed: 1, membership_seed: 2, metadata: &metadata };
        (seal::encode_model(&file).expect("a model"), fingerprints.len())
    }

    #[test]
    fn a_packed_model_is_the_sealed_one_without_its_fingerprints() {
        let (sealed, fingerprints) = sealed_model();
        let packed = pack_model(&sealed).expect("a cut");
        // Three fingerprints of eight bytes less; the count of them is one digit either way.
        assert_eq!(packed.len() + fingerprints * KSLM_FINGERPRINT_BYTES, sealed.len());
        let (whole, cut) = (LinearNgramModel::from_bytes(sealed.clone(), None).expect("it loads"), LinearNgramModel::from_bytes(packed.clone(), None).expect("it loads"));
        assert_eq!(decision_parameters(&cut), decision_parameters(&whole));
        assert_eq!(cut.model_version, whole.model_version);
        assert_ne!(cut.payload_sha256, whole.payload_sha256);
        assert_ne!(seal::runtime_parameters(&cut), seal::runtime_parameters(&whole));
        // A cut of the cut is the cut: nothing is left to take away.
        assert_eq!(pack_model(&packed).expect("a cut"), packed);
        // What is no model, and a manifest another writer wrote.
        assert!(pack_model(b"KSLM").unwrap_err().contains("the sealed model does not load"));
        assert!(packaging_record(&sealed, b"KSLM").unwrap_err().contains("the packed model does not load"));
        assert!(packaging_record(b"KSLM", &packed).unwrap_err().contains("the sealed model does not load"));
        assert!(packed_manifest(b"{", &[]).unwrap_err().contains("the sealed manifest"));
        assert!(packed_manifest(b"{\"b\": 1, \"a\": 2}", &[]).unwrap_err().contains("not in the form this writer writes"));
        let record: Value = serde_json::from_slice(&packaging_record(&sealed, &packed).expect("a record")).expect("json");
        assert_eq!((record["fingerprints_left_out"].as_u64(), record["model_version"].as_str()), (Some(3), Some("intent-v1-small")));
    }

    #[test]
    fn a_tree_holds_its_packaged_model_to_its_sealed_one() {
        let (sealed, _) = sealed_model();
        let root = folder("tree");
        let write = |name: &str, bytes: &[u8]| {
            std::fs::create_dir_all(root.join(name).parent().expect("a folder")).expect("a folder");
            std::fs::write(root.join(name), bytes).expect("a file");
        };
        write(SEALED_ARTIFACT, &sealed);
        for (path, bytes) in packaged_outputs(&root, &sealed).expect("what is published") {
            write(path.strip_prefix(&root).expect("a path of the tree").to_str().expect("a name"), &bytes);
        }
        let packed = pack_model(&sealed).expect("a cut");
        let digest = |bytes: &[u8]| hex(&Sha256::digest(bytes));
        assert_eq!(packaged_from_sealed(&root).expect("the cut"), (digest(&sealed), digest(&packed)));
        let artifact = root.join(SEALED_ARTIFACT);
        assert!(packaged_check(&root, &artifact).0);
        // The sealed model by another spelling of its path is the sealed model; another file is not.
        assert!(is_sealed_artifact(&root, &root.join("model/intent_v1/../intent_v1/layout_intent_v1.ksm")));
        assert!(!is_sealed_artifact(&root, &root.join(PACKAGED_ARTIFACT)));
        assert!(!is_sealed_artifact(&root, Path::new("/nonexistent/model.ksm")));
        assert!(is_sealed_artifact(Path::new("/nonexistent"), &Path::new("/nonexistent").join(SEALED_ARTIFACT)));
        // A record of other files, the whole model in the place of the cut, no packaged model, no sealed one.
        write(PACKAGING_RECORD, b"{}");
        assert!(packaged_from_sealed(&root).unwrap_err().contains(PACKAGING_RECORD));
        write(PACKAGED_ARTIFACT, &sealed);
        let (passed, said) = packaged_check(&root, &artifact);
        assert!(!passed && said.contains("without its fingerprints"), "{said}");
        std::fs::remove_file(root.join(PACKAGED_ARTIFACT)).expect("removed");
        assert!(packaged_from_sealed(&root).unwrap_err().contains(PACKAGED_ARTIFACT));
        std::fs::remove_file(&artifact).expect("removed");
        assert!(packaged_from_sealed(&root).unwrap_err().contains(SEALED_ARTIFACT));
        assert!(!packaged_check(&root, &artifact).0);
    }
}

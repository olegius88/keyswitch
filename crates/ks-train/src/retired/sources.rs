//! What the evidence of a rejected generation pins. Its seal, receipts and reports name the files that produced them by
//! SHA-256. Those files were Python and are not in this tree; `frozen` folders are the generation's own evidence and are
//! read in place, and every other pin has to carry the reviewed digest from these tables, so that nothing is compared
//! with a live file that merely has the same name.
use crate::corpus::checksum;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::path::Path;

pub struct SourceArchive {
    pub name: &'static str,
    pub frozen: &'static [&'static str],
    pub sources: &'static [(&'static str, &'static str)],
}

impl SourceArchive {
    fn reviewed(&self, relative: &str) -> Option<&'static str> {
        self.sources.iter().find(|(name, _)| *name == relative).map(|(_, digest)| *digest)
    }

    fn check(&self, relative: &str, digest: &str, root: &Path) -> Result<(), String> {
        if self.frozen.iter().any(|folder| relative.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'))) {
            if checksum(&root.join(relative))? != digest {
                return Err(format!("historical source changed: {relative}"));
            }
            return Ok(());
        }
        match self.reviewed(relative) {
            None => Err(format!("{}: historical source is not archived: {relative}", self.name)),
            Some(reviewed) if reviewed != digest => Err(format!("unapproved historical source identity: {relative}")),
            Some(_) => Ok(()),
        }
    }

    /// Every pin holds, whatever separator the recording platform used; the pins come back normalized.
    pub fn verify(&self, pins: &Value, root: &Path) -> Result<Map<String, Value>, String> {
        let table = pins.as_object().filter(|table| !table.is_empty()).ok_or("missing historical source pins")?;
        let mut normalized = Map::new();
        for (key, digest) in table {
            let text = digest.as_str().filter(|text| is_digest(text)).ok_or("invalid historical source pin")?;
            let name = key.replace('\\', "/");
            let bad = name.starts_with('/') || name.contains(':') || name.split('/').any(|part| part == ".." || part == "." || part.is_empty());
            if bad || normalized.contains_key(&name) {
                return Err("invalid or duplicate historical source path".to_owned());
            }
            self.check(&name, text, root)?;
            normalized.insert(name, json!(text));
        }
        Ok(normalized)
    }

    /// Pins a replay puts back into the evidence it reproduces: exactly these paths, all verified.
    pub fn recorded(&self, pins: &Value, paths: &[&str], root: &Path) -> Result<Map<String, Value>, String> {
        let normalized = self.verify(pins, root)?;
        let found: BTreeSet<&str> = normalized.keys().map(String::as_str).collect();
        if found != paths.iter().copied().collect::<BTreeSet<&str>>() {
            return Err(format!("{}: recorded provenance names other files than its tool records", self.name));
        }
        Ok(normalized)
    }

    #[must_use]
    pub fn all_paths(&self) -> Vec<&'static str> {
        self.sources.iter().map(|(name, _)| *name).collect()
    }

    /// One recorded tool digest (a receipt's generator), verified the same way.
    pub fn recorded_digest(&self, relative: &str, digest: &Value, root: &Path) -> Result<String, String> {
        let verified = self.verify(&json!({relative: digest}), root)?;
        Ok(verified[relative].as_str().unwrap_or_default().to_owned())
    }
}

fn is_digest(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The candidate seal, the lexical receipt, the corpus receipt and the engine report of context-v2.
pub const CONTEXT_V2: SourceArchive = SourceArchive {
    name: "context-v2",
    frozen: &["model/context_v2"],
    sources: &[
        ("model/intent_v1/config.json", "76fde35bec793c2c1d6a168753e28cb1cb0c62ac63fa1da40a6d36c85c7fd532"),
        ("src/keyswitch/boundary_model.py", "95fe89d56cbf5c3afaefd1da1158677229e787849e98149f6cd4821c0b3a766a"),
        ("src/keyswitch/boundary_policy.py", "f8a9e8779133585684d927d068907e02a0869dafc73c6aa71e2f4c8686e0e0bf"),
        ("src/keyswitch/context_model.py", "760c0d10ef73167ee8c601b806a75262e2ad3a118fde425e252e11cfde5e0491"),
        ("src/keyswitch/context_policy.py", "c77bc72cf0f4e6da13d260cf426e2af68ee404d42556c8f2239da80f15ad8ae2"),
        ("src/keyswitch/detector.py", "2da2f507f1e25f6f3574af4388758631fdad847166a1bdf56a5ba8f7c21c21b2"),
        ("src/keyswitch/engine.py", "25eda8ad37def9160492245d8a5c55d0924a0fbbdc81829d55e629437ea3dbf9"),
        ("src/keyswitch/input_context.py", "09431aac715a22aee4044519cf04a92cfba5f7a3d71c96c0716d01dc38de7087"),
        ("src/keyswitch/language_model.py", "328b6def30ce1694cecb6d5d9654827671f4bb550052ae2d1e2f314043ea2901"),
        ("src/keyswitch/resources/models/boundary-v2.json", "6685e02734f0451a91278fa577b37c7a25a2aa692c7a91c9a979fdbb045b7cf5"),
        ("src/keyswitch/resources/models/layout_intent_v1.ksm", "47f86818c4c1243daeabfafd50d03dd9884aa3973bb546191afe02c4092a3f4d"),
        ("src/keyswitch/resources/models/ortho_v1.json", "02cd2bb5aacf2c88085028a14c6af551a588f01f2ec0163d645376fd11e3c198"),
        ("src/keyswitch/spellcheck.py", "0bdd97f092e16a444846be9279a6f6bfdca555c83aefc318bed34c13dc7a2f94"),
        ("tests/test_input_integrity.py", "9b3a0b5fc5fbd84d229f14f07a1bc24512c230d0af323b6cd313c9c00ec46c79"),
        ("tools/context_corpus.py", "4c71b1ec1d213b2301899856b10066d5551e44a56c1e5a352d1742969e83806a"),
        ("tools/context_evidence.py", "cd74258aa6f9094ce8b860bd7bfae3c50b8cc8c771f799c796b51cdb40e65d37"),
        ("tools/context_frames.py", "b7d8f6019ca668ef7956278e9ea5253a13c33b49880e97798ec062939f8bdf9f"),
        ("tools/context_optimizer.c", "f523aa5652a70a897c1251985dfdc78676471ef338bfd9d8efe44f74efeb4b0c"),
        ("tools/context_optimizer.py", "45cf97c58c9a7c8518ba93b65a7b8d24960bc3583fad983eefdbe7e2ff22995b"),
        ("tools/evaluate_context_engine.py", "42b2d5baf56b7668349c5a6b1aa1d81d991184dbf46321cfb3fef2433419d981"),
        ("tools/train_context_v2.py", "28fc267ec70fe85fcd48d6881b971f10224b48b9c2357f0f93332d9b1994a34c"),
    ],
};

/// The seal of boundary-v1.
pub const BOUNDARY_V1: SourceArchive = SourceArchive {
    name: "boundary-v1",
    frozen: &["model/boundary_v1"],
    sources: &[
        ("src/keyswitch/boundary_model.py", "95fe89d56cbf5c3afaefd1da1158677229e787849e98149f6cd4821c0b3a766a"),
        ("src/keyswitch/language_model.py", "328b6def30ce1694cecb6d5d9654827671f4bb550052ae2d1e2f314043ea2901"),
        ("src/keyswitch/layouts.py", "374e8913fe51c72370ae8f775875847e47c2a1db7c8deaa2fc919944dd45597c"),
        ("tools/context_corpus.py", "4c71b1ec1d213b2301899856b10066d5551e44a56c1e5a352d1742969e83806a"),
        ("tools/context_evidence.py", "cd74258aa6f9094ce8b860bd7bfae3c50b8cc8c771f799c796b51cdb40e65d37"),
        ("tools/train_boundary_model.py", "4151400e9a5e55714e8261ac40ce87678192a42a3505ecb9a68b158bdfe3ab58"),
    ],
};

/// The seal, the corpus receipt and the generator digests of the two dictionary receipts of ortho-v2. It also read the
/// frozen phrase snapshot of context-v2 and the frozen Onboard lexicons of the intent model in place.
pub const ORTHO_V2: SourceArchive = SourceArchive {
    name: "ortho-v2",
    frozen: &["model/ortho_v2", "model/context_v2", "model/intent_v1/sources"],
    sources: &[
        ("model/ortho_v1/candidate.json", "02cd2bb5aacf2c88085028a14c6af551a588f01f2ec0163d645376fd11e3c198"),
        ("model/ortho_v1/tokens.jsonl.gz", "fab2fc577471b912278d4364cc192f06918f4f835a1f5366eea40e539e87be9e"),
        ("src/keyswitch/layouts.py", "374e8913fe51c72370ae8f775875847e47c2a1db7c8deaa2fc919944dd45597c"),
        ("src/keyswitch/ortho_model.py", "998b1b228ff1e37ddcb6aa3678d2b199e1b83489d85dca6ef40a5c1ee3dc5f38"),
        ("tools/context_corpus.py", "4c71b1ec1d213b2301899856b10066d5551e44a56c1e5a352d1742969e83806a"),
        ("tools/ortho_v2_corpus.py", "ce9f7357a86d49cf52057fe0c1fed7be8015ec239c6c54ea090170bdf663386a"),
        ("tools/ortho_v2_known.py", "f3f02e0385ee2077d83bd84ef5d4e1bf7d093343efa00bf027d1709590b8e75f"),
        ("tools/ortho_v2_verified.py", "4f18b5e15fcdfbc6bc3933ec9e8b84153b25ffb4323a151cb83294f2920affe0"),
        ("tools/train_ortho_v2.py", "326ec3784b1128d5bd0c287ed4a8538c93fbdc6fe02973e6263184b66e51b5eb"),
    ],
};

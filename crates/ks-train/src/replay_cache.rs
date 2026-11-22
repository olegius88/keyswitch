//! Caches of the work a development measure repeats: the cases a pair gave on a development sample (`evaluate --cache`),
//! keyed by everything such a replay reads. A prefix experiment changes one pair of the four an evaluation replays; the
//! three it does not change are read back instead of being typed again. A sealed test never reads a cache.
use crate::corpus::checksum;
use crate::physical::hex;
use crate::sequence_report::{Case, PairCounts};
use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The file that pins the reference lexicons and their Hunspell dictionaries, which every replay reads.
const LEXICON_PINS: &str = "model/intent_v1/config.json";

/// The SHA-256 of the program that runs: a replay is the engine's code as much as its models, and a rebuilt program is
/// another engine.
pub fn program_digest() -> Result<String, String> {
    let path = std::env::current_exe().map_err(|error| format!("the running program: {error}"))?;
    checksum(&path)
}

/// The digest of every file under a folder, each by its path inside it, in the order of the paths.
pub fn folder_digest(folder: &Path) -> Result<String, String> {
    fn walk(folder: &Path, found: &mut Vec<PathBuf>) -> Result<(), String> {
        for entry in std::fs::read_dir(folder).map_err(|error| format!("{}: {error}", folder.display()))? {
            let path = entry.map_err(|error| error.to_string())?.path();
            if path.is_dir() {
                walk(&path, found)?;
            } else {
                found.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(folder, &mut files)?;
    let mut named: Vec<(String, PathBuf)> = files.into_iter().map(|path| (path.strip_prefix(folder).unwrap_or(&path).display().to_string(), path)).collect();
    named.sort();
    let mut hasher = Sha256::new();
    for (name, path) in named {
        hasher.update(format!("{name}\0{}\n", checksum(&path)?).as_bytes());
    }
    Ok(hex(&hasher.finalize()))
}

/// What one cached replay holds.
#[derive(Serialize, Deserialize)]
struct Entry {
    counts: PairCounts,
    cases: Vec<Case>,
}

/// A folder of replays of one development sample, under the digest of what every replay of it reads besides its two
/// models: the program, the tree's resources, the pinned lexicons, the protocols and the sample's rows.
pub struct ReplayCache {
    folder: PathBuf,
    identity: String,
}

impl ReplayCache {
    pub fn open(folder: &Path, root: &Path, resource_dir: &Path, rows: &Path) -> Result<ReplayCache, String> {
        let parts = [
            ("program", program_digest()?),
            ("resources", folder_digest(resource_dir)?),
            ("lexicons", checksum(&root.join(LEXICON_PINS))?),
            ("protocols", checksum(&root.join(crate::evaluate::PROTOCOLS))?),
            ("rows", checksum(rows)?),
        ];
        let text: String = parts.iter().map(|(name, digest)| format!("{name} {digest}\n")).collect();
        std::fs::create_dir_all(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
        Ok(ReplayCache { folder: folder.to_path_buf(), identity: hex(&Sha256::digest(text.as_bytes())) })
    }

    /// The name of one replay: a pair's two models, the plans it typed, the lexical profile and the settings mode.
    #[must_use]
    pub fn key(&self, context_sha256: &str, prefix_sha256: &str, part: &str, profile: &str, mode: &str) -> String {
        let text = format!("{}\ncontext {context_sha256}\nprefix {prefix_sha256}\npart {part}\nprofile {profile}\nmode {mode}\n", self.identity);
        hex(&Sha256::digest(text.as_bytes()))
    }

    fn path(&self, key: &str) -> PathBuf {
        self.folder.join(format!("{key}.json.gz"))
    }

    /// The cached replay of a key; none when it was never written or cannot be read.
    #[must_use]
    pub fn read(&self, key: &str) -> Option<(PairCounts, Vec<Case>)> {
        let file = std::fs::File::open(self.path(key)).ok()?;
        let entry: Entry = ks_text::json::loads(std::io::read_to_string(MultiGzDecoder::new(file)).ok()?).ok()?;
        Some((entry.counts, entry.cases))
    }

    /// Keeps a replay under its key; a reader never sees half a file.
    pub fn write(&self, key: &str, counts: &PairCounts, cases: &[Case]) -> Result<(), String> {
        let entry = Entry { counts: counts.clone(), cases: cases.to_vec() };
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&crate::artifact::canonical_of(&entry)).map_err(|error| error.to_string())?;
        let bytes = encoder.finish().map_err(|error| error.to_string())?;
        let partial = self.folder.join(format!("{key}.{}.partial", std::process::id()));
        std::fs::write(&partial, bytes).map_err(|error| format!("{}: {error}", partial.display()))?;
        std::fs::rename(&partial, self.path(key)).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    #[test]
    fn a_folder_digest_names_each_file_by_its_path_and_bytes() {
        let folder = std::env::temp_dir().join(format!("ks-train-folder-digest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("inner")).unwrap();
        std::fs::write(folder.join("a.txt"), "a").unwrap();
        std::fs::write(folder.join("inner/b.txt"), "b").unwrap();
        let first = folder_digest(&folder).unwrap();
        assert_eq!(folder_digest(&folder).unwrap(), first);
        std::fs::write(folder.join("inner/b.txt"), "c").unwrap();
        assert_ne!(folder_digest(&folder).unwrap(), first);
        std::fs::rename(folder.join("inner/b.txt"), folder.join("inner/d.txt")).unwrap();
        let moved = folder_digest(&folder).unwrap();
        std::fs::write(folder.join("inner/d.txt"), "b").unwrap();
        assert_ne!(folder_digest(&folder).unwrap(), moved);
        assert!(folder_digest(&folder.join("missing")).is_err());
    }
}

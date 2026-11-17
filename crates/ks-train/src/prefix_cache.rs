//! The frames of a prefix fit kept on disk (`prefix --frames-cache`): a split's frames depend on its parents, the rules
//! that label and measure them, the feature schema, the tree's resources and lexicons and the program, and nothing a
//! recipe changes between two experiments on one corpus (epochs, rates, importance, the selection). Experiments that
//! differ only there read the frames back instead of computing a million of them again.
use crate::corpus::checksum;
use crate::physical::hex;
use crate::prefix::{ParentWord, PrefixFrame, SplitFrames};
use crate::replay_cache::{folder_digest, program_digest};
use flate2::Compression;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use ks_cast::To as _;
use sha2::{Digest, Sha256};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

/// The first bytes of a frames file, with the version of its layout.
const MAGIC: &[u8; 8] = b"KSPFRM1\n";
/// The file that pins the reference lexicons and their Hunspell dictionaries, which every frame reads.
const LEXICON_PINS: &str = "model/intent_v1/config.json";

/// A folder of frames, under the digest of what every split's frames read besides their parents and rules.
pub struct FramesCache {
    folder: PathBuf,
    identity: String,
}

impl FramesCache {
    pub fn open(folder: &Path, root: &Path, resource_dir: &Path) -> Result<FramesCache, String> {
        let text = format!("program {}\nresources {}\nlexicons {}\n", program_digest()?, folder_digest(resource_dir)?, checksum(&root.join(LEXICON_PINS))?);
        std::fs::create_dir_all(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
        Ok(FramesCache { folder: folder.to_path_buf(), identity: hex(&Sha256::digest(text.as_bytes())) })
    }

    /// The name of a split's frames: the settings that make them, as the fit words them, and every field of every parent.
    #[must_use]
    pub fn key(&self, settings: &str, parents: &[ParentWord]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(format!("{}\n{settings}\n", self.identity).as_bytes());
        for parent in parents {
            let fields = [
                parent.original.as_str(),
                &parent.group.to_string(),
                &parent.family,
                &parent.before,
                &parent.identifier,
                &parent.category,
                &parent.application,
                parent.role.as_str(),
                &u8::from(parent.paired).to_string(),
                &u8::from(parent.own_context).to_string(),
                &u8::from(parent.hard).to_string(),
            ];
            for field in fields {
                hasher.update(field.as_bytes());
                hasher.update([0]);
            }
            hasher.update(b"\n");
        }
        hex(&hasher.finalize())
    }

    fn path(&self, key: &str) -> PathBuf {
        self.folder.join(format!("{key}.frames.gz"))
    }

    /// The frames kept under a key; none when they were never written or cannot be read.
    #[must_use]
    pub fn read(&self, key: &str) -> Option<SplitFrames> {
        let file = std::fs::File::open(self.path(key)).ok()?;
        decode(&mut BufReader::new(MultiGzDecoder::new(file))).ok()
    }

    /// Keeps a split's frames under a key; a reader never sees half a file.
    pub fn write(&self, key: &str, frames: &SplitFrames) -> Result<(), String> {
        let partial = self.folder.join(format!("{key}.{}.partial", std::process::id()));
        let file = std::fs::File::create(&partial).map_err(|error| format!("{}: {error}", partial.display()))?;
        let mut writer = BufWriter::new(GzEncoder::new(file, Compression::fast()));
        encode(frames, &mut writer).map_err(|error| error.to_string())?;
        writer.into_inner().map_err(|error| error.to_string())?.finish().map_err(|error| error.to_string())?;
        std::fs::rename(&partial, self.path(key)).map_err(|error| error.to_string())
    }
}

fn put_texts(texts: &[String], out: &mut impl Write) -> std::io::Result<()> {
    out.write_all(&texts.len().to::<u64>().to_le_bytes())?;
    for text in texts {
        out.write_all(&text.len().to::<u64>().to_le_bytes())?;
        out.write_all(text.as_bytes())?;
    }
    Ok(())
}

/// A split's frames as bytes: every field, every float by its bits.
pub fn encode(frames: &SplitFrames, out: &mut impl Write) -> std::io::Result<()> {
    out.write_all(MAGIC)?;
    for texts in [&frames.names, &frames.families, &frames.classes, &frames.typed] {
        put_texts(texts, out)?;
    }
    out.write_all(&frames.pairs.to::<u64>().to_le_bytes())?;
    out.write_all(&frames.frames.len().to::<u64>().to_le_bytes())?;
    for frame in &frames.frames {
        out.write_all(&[frame.label, frame.profile, u8::from(frame.desired), u8::from(frame.supported), frame.source, u8::from(frame.hard)])?;
        out.write_all(&frame.class.to_le_bytes())?;
        for value in [frame.parent_family, frame.sequence, frame.length, frame.full_length, frame.pair] {
            out.write_all(&value.to_le_bytes())?;
        }
        out.write_all(&frame.sample_weight.to_bits().to_le_bytes())?;
        out.write_all(&frame.features.len().to::<u32>().to_le_bytes())?;
        for (id, value) in &frame.features {
            out.write_all(&id.to_le_bytes())?;
            out.write_all(&value.to_bits().to_le_bytes())?;
        }
    }
    Ok(())
}

fn bytes<const N: usize>(input: &mut impl Read) -> std::io::Result<[u8; N]> {
    let mut buffer = [0_u8; N];
    input.read_exact(&mut buffer)?;
    Ok(buffer)
}

fn number(input: &mut impl Read) -> std::io::Result<usize> {
    usize::try_from(u64::from_le_bytes(bytes(input)?)).map_err(|_| std::io::Error::other("a count beyond memory"))
}

fn small(input: &mut impl Read) -> std::io::Result<u32> {
    Ok(u32::from_le_bytes(bytes(input)?))
}

fn float(input: &mut impl Read) -> std::io::Result<f64> {
    Ok(f64::from_bits(u64::from_le_bytes(bytes(input)?)))
}

fn get_texts(input: &mut impl Read) -> std::io::Result<Vec<String>> {
    let count = number(input)?;
    let mut texts = Vec::new();
    for _ in 0..count {
        let mut buffer = vec![0_u8; number(input)?];
        input.read_exact(&mut buffer)?;
        texts.push(String::from_utf8(buffer).map_err(std::io::Error::other)?);
    }
    Ok(texts)
}

/// The frames `encode` wrote.
pub fn decode(input: &mut impl Read) -> std::io::Result<SplitFrames> {
    if &bytes::<8>(input)? != MAGIC {
        return Err(std::io::Error::other("not a frames file of this layout"));
    }
    let (names, families, classes, typed) = (get_texts(input)?, get_texts(input)?, get_texts(input)?, get_texts(input)?);
    let pairs = number(input)?;
    let count = number(input)?;
    let mut frames = Vec::with_capacity(count);
    for _ in 0..count {
        let [label, profile, desired, supported, source, hard] = bytes::<6>(input)?;
        let class = u16::from_le_bytes(bytes(input)?);
        let (parent_family, sequence, length, full_length, pair) = (small(input)?, small(input)?, small(input)?, small(input)?, small(input)?);
        let sample_weight = float(input)?;
        let features = (0..small(input)?).map(|_| Ok((small(input)?, float(input)?))).collect::<std::io::Result<Vec<(u32, f64)>>>()?;
        frames.push(PrefixFrame { label, class, parent_family, sequence, length, full_length, desired: desired != 0, profile, sample_weight, supported: supported != 0, pair, source, hard: hard != 0, features });
    }
    Ok(SplitFrames { frames, names, families, pairs, classes, typed })
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;

    fn frames() -> SplitFrames {
        let frame = |label: u8, features: Vec<(u32, f64)>| PrefixFrame {
            label,
            class: 1,
            parent_family: 2,
            sequence: 3,
            length: 4,
            full_length: 6,
            desired: label != 0,
            profile: 1,
            sample_weight: 1.0 / 3.0,
            supported: true,
            pair: 7,
            source: 1,
            hard: true,
            features,
        };
        SplitFrames {
            frames: vec![frame(0, vec![(0, 1.0), (1, -0.1)]), frame(2, Vec::new())],
            names: vec!["bias".to_owned(), "ortho:sum:4".to_owned()],
            families: vec!["ghb".to_owned()],
            pairs: 2,
            classes: vec!["rare:0".to_owned(), "name:1".to_owned()],
            typed: vec!["привет".to_owned()],
        }
    }

    fn same(left: &SplitFrames, right: &SplitFrames) -> bool {
        let fields = |frames: &SplitFrames| (frames.names.clone(), frames.families.clone(), frames.pairs, frames.classes.clone(), frames.typed.clone());
        let frame = |item: &PrefixFrame| {
            let numbers = [item.class.to::<u32>(), item.parent_family, item.sequence, item.length, item.full_length, item.pair];
            let flags = [item.label, item.profile, u8::from(item.desired), u8::from(item.supported), item.source, u8::from(item.hard)];
            let features: Vec<(u32, u64)> = item.features.iter().map(|(id, value)| (*id, value.to_bits())).collect();
            (numbers, flags, item.sample_weight.to_bits(), features)
        };
        fields(left) == fields(right) && left.frames.iter().map(frame).eq(right.frames.iter().map(frame))
    }

    #[test]
    fn frames_come_back_as_they_were_written() {
        let mut bytes = Vec::new();
        encode(&frames(), &mut bytes).unwrap();
        let read = decode(&mut bytes.as_slice()).unwrap();
        assert!(same(&read, &frames()));
        assert!(decode(&mut &bytes[..bytes.len() - 1]).is_err());
        let mut other = bytes.clone();
        other[0] = b'X';
        assert!(decode(&mut other.as_slice()).is_err());
    }

    #[test]
    fn a_cache_keeps_frames_under_their_parents_and_settings() {
        let folder = std::env::temp_dir().join(format!("ks-train-frames-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let cache = FramesCache::open(&folder, &root, &root.join("resources")).unwrap();
        let parent = ParentWord {
            original: "ghbdtn".to_owned(),
            group: 1,
            family: "ghb".to_owned(),
            before: String::new(),
            identifier: "p".to_owned(),
            category: "rare".to_owned(),
            application: "Editor".to_owned(),
            role: ks_backend::FieldRole::Unknown,
            paired: true,
            own_context: true,
            hard: false,
        };
        let key = cache.key("settings", std::slice::from_ref(&parent));
        assert_ne!(key, cache.key("other settings", std::slice::from_ref(&parent)));
        assert_ne!(key, cache.key("settings", &[ParentWord { hard: true, ..parent.clone() }]));
        assert!(cache.read(&key).is_none());
        cache.write(&key, &frames()).unwrap();
        assert!(same(&cache.read(&key).unwrap(), &frames()));
    }
}

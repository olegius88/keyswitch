//! Exact US/RU key pairs shared by context training and physical replay (`context_physical_keys`), the hashes the
//! curricula draw with, and the physical identity of a word (`physical`, `expanded_aliases`).
use ks_cast::To as _;
use ks_text::layouts::LayoutPair;
use ks_text::{normalize, pystr};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

const PHYSICAL_KEY_KEYCODE_BASE: u32 = 20;
const PLAIN_US: &str = "`1234567890-=qwertyuiop[]\\asdfghjkl;'zxcvbnm,./ ";
const PLAIN_RU: &str = "ё1234567890-=йцукенгшщзхъ\\фывапролджэячсмитьбю. ";
const SHIFT_US: &str = "~!@#$%^&*()_+QWERTYUIOP{}|ASDFGHJKL:\"ZXCVBNM<>? ";
const SHIFT_RU: &str = "Ё!\"№;%:?*()_+ЙЦУКЕНГШЩЗХЪ/ФЫВАПРОЛДЖЭЯЧСМИТЬБЮ, ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhysicalKey {
    pub characters: [char; 2],
    pub shift: bool,
    pub keycode: u32,
    pub explicit_group: Option<i32>,
}

pub static KEYS: LazyLock<Vec<PhysicalKey>> = LazyLock::new(|| {
    let mut keys = Vec::new();
    for (shift, us, ru) in [(false, PLAIN_US, PLAIN_RU), (true, SHIFT_US, SHIFT_RU)] {
        for (index, (left, right)) in us.chars().zip(ru.chars()).enumerate() {
            keys.push(PhysicalKey { characters: [left, right], shift, keycode: index.to::<u32>() + PHYSICAL_KEY_KEYCODE_BASE, explicit_group: None });
        }
    }
    keys
});

/// For each layout, the first key that types a character there.
static BY_CHARACTER: LazyLock<[HashMap<char, usize>; 2]> = LazyLock::new(|| {
    let mut maps = [HashMap::new(), HashMap::new()];
    for (index, key) in KEYS.iter().enumerate() {
        for group in 0..2 {
            maps[group].entry(key.characters[group]).or_insert(index);
        }
    }
    maps
});

/// `translated`: identical key presses read in the other layout; an unknown glyph is an error.
pub fn translated(text: &str, group: i64) -> Result<String, char> {
    let group = usize::try_from(group).ok().filter(|group| *group < 2).expect("physical translation requires group 0 or 1");
    let mut result = String::with_capacity(text.len());
    for character in text.chars() {
        let key = BY_CHARACTER[group].get(&character).ok_or(character)?;
        result.push(KEYS[*key].characters[1 - group]);
    }
    Ok(result)
}

/// `translated` where the Python lets the exception stop the fit.
pub fn reading(text: &str, group: i64) -> String {
    translated(text, group).unwrap_or_else(|glyph| panic!("unsupported physical glyph U+{:04X} in group {group}", u32::from(glyph)))
}

/// The first key that types a character in this layout.
pub fn key_of(character: char, group: usize) -> Option<&'static PhysicalKey> {
    BY_CHARACTER[group].get(&character).map(|index| &KEYS[*index])
}

pub fn sha256(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

/// `hashlib.sha256(text.encode()).hexdigest()`.
pub fn hex_digest(text: &str) -> String {
    hex(&sha256(text))
}

pub fn hex(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push_str(&format!("{byte:02x}"));
    }
    result
}

/// `variant_choice`: a number below `count` from the first eight hex digits of the hash of `purpose:identifier`.
pub fn variant_choice(identifier: &str, purpose: &str, count: usize) -> usize {
    let mut hasher = Sha256::new();
    hasher.update(purpose.as_bytes());
    hasher.update(b":");
    hasher.update(identifier.as_bytes());
    let digest = hasher.finalize();
    u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]).to::<usize>() % count
}

/// `_ring_point`: the first sixteen hex digits of the hash.
pub fn ring_point(text: &str) -> u64 {
    let digest = sha256(text);
    u64::from_be_bytes([digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7]])
}

static PAIR: LazyLock<LayoutPair> = LazyLock::new(LayoutPair::default);

pub fn pair() -> &'static LayoutPair {
    &PAIR
}

/// `physical`: the word's keys, whichever layout wrote it.
pub fn physical(token: &str) -> String {
    let normalized = pystr::casefold(&normalize::nfc(token)).replace('’', "'");
    PAIR.translate(&normalized, "ru", "us").expect("the pair of layouts")
}

/// `physical_aliases`: the hash of every supported whole-layout or script-guided physical reading.
pub fn physical_aliases(form: &str, aliases: &mut BTreeSet<String>) {
    let text = pystr::casefold(&normalize::nfc(form)).replace('’', "'");
    for initial in 0..2_usize {
        for guided in [false, true] {
            let mut group = initial;
            let mut positions: Vec<String> = Vec::new();
            let mut whole = true;
            for character in text.chars() {
                if guided {
                    if character.is_ascii_lowercase() {
                        group = 0;
                    } else if ('а'..='я').contains(&character) || character == 'ё' {
                        group = 1;
                    }
                    if key_of(character, group).is_none() && key_of(character, 1 - group).is_some() {
                        group = 1 - group;
                    }
                }
                match key_of(character, group) {
                    Some(key) => positions.push(key.keycode.to_string()),
                    None => {
                        whole = false;
                        break;
                    }
                }
            }
            if whole {
                aliases.insert(hex_digest(&format!("physical-key-positions-v1:{}", positions.join(","))));
            }
        }
    }
    aliases.insert(hex_digest(&format!("normalized-literal-v1:{text}")));
}

/// `expanded_aliases`: the aliases of a word and of its readings in both layouts.
pub fn expanded_aliases(form: &str) -> BTreeSet<String> {
    let mut aliases = BTreeSet::new();
    physical_aliases(form, &mut aliases);
    physical_aliases(&PAIR.translate(form, "us", "ru").expect("the pair of layouts"), &mut aliases);
    physical_aliases(&PAIR.translate(form, "ru", "us").expect("the pair of layouts"), &mut aliases);
    for group in 0..2 {
        if let Ok(other) = translated(form, group) {
            physical_aliases(&other, &mut aliases);
        }
    }
    aliases
}

/// `repr(str)`: a Python string as its source text.
pub fn py_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') { '"' } else { '\'' };
    let mut result = String::with_capacity(text.len() + 2);
    result.push(quote);
    for character in text.chars() {
        match character {
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            _ if character == quote => {
                result.push('\\');
                result.push(character);
            }
            _ => {
                let code = u32::from(character);
                let mut buffer = [0_u8; 4];
                if code < 0x20 || code == 0x7f {
                    result.push_str(&format!("\\x{code:02x}"));
                } else if code < 0x7f || pystr::isprintable(character.encode_utf8(&mut buffer)) {
                    result.push(character);
                } else if code <= 0xff {
                    result.push_str(&format!("\\x{code:02x}"));
                } else if code <= 0xffff {
                    result.push_str(&format!("\\u{code:04x}"));
                } else {
                    result.push_str(&format!("\\U{code:08x}"));
                }
            }
        }
    }
    result.push(quote);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_read_in_the_other_layout() {
        assert_eq!(reading("ghbdtn", 0), "привет");
        assert_eq!(reading("привет", 1), "ghbdtn");
        assert_eq!(translated("«", 0), Err('«'));
    }

    #[test]
    fn repr_as_python_writes_it() {
        assert_eq!(py_repr("a"), "'a'");
        assert_eq!(py_repr("it's"), "\"it's\"");
        assert_eq!(py_repr("\n"), "'\\n'");
        assert_eq!(py_repr("а\u{a0}б"), "'а\\xa0б'");
        assert_eq!(py_repr("\u{2028}"), "'\\u2028'");
    }
}

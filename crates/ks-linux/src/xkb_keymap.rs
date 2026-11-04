//! The client side of the XKB keyboard map: what libX11 answers to `XkbKeycodeToKeysym` and `XkbLookupKeySym`.
//!
//! The map is read from the server once (key types and key symbols) and kept until the server says it changed.

/// One entry of a key type's map: the modifiers that select a level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeEntry {
    pub active: bool,
    pub mods_mask: u8,
    pub level: u8,
}

/// `XkbKeyTypeRec`: which modifiers count, and the level each combination gives.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct KeyType {
    pub mods_mask: u8,
    pub num_levels: u8,
    pub entries: Vec<TypeEntry>,
}

/// The symbols of one key: `width` levels for each of its groups.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct KeySyms {
    pub kt_index: [u8; 4],
    pub group_info: u8,
    pub width: u8,
    pub syms: Vec<u32>,
}

impl KeySyms {
    fn groups(&self) -> usize {
        usize::from(self.group_info & 0x0f)
    }
}

/// `XkbDescRec` with the parts the backend reads.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct KeyMap {
    pub min_keycode: u8,
    pub max_keycode: u8,
    pub types: Vec<KeyType>,
    /// Indexed from `min_keycode`.
    pub keys: Vec<KeySyms>,
}

const OUT_OF_RANGE_CLAMP: u8 = 0x40;
const OUT_OF_RANGE_REDIRECT: u8 = 0x80;
const OUT_OF_RANGE_MASK: u8 = 0xc0;
const REDIRECT_GROUP_MASK: u8 = 0x30;
const REDIRECT_GROUP_SHIFT: u32 = 4;

impl KeyMap {
    fn key(&self, keycode: u8) -> Option<&KeySyms> {
        if keycode < self.min_keycode || keycode > self.max_keycode {
            return None;
        }
        self.keys.get(usize::from(keycode - self.min_keycode))
    }

    fn key_type(&self, key: &KeySyms, group: usize) -> Option<&KeyType> {
        self.types.get(usize::from(*key.kt_index.get(group)?))
    }

    /// `XkbKeycodeToKeysym`: 0 for NoSymbol.
    pub fn keysym(&self, keycode: u8, group: u8, level: u8) -> u32 {
        let Some(key) = self.key(keycode) else { return 0 };
        let group = usize::from(group);
        if group >= key.groups() {
            return 0;
        }
        let width = self.key_type(key, group).map_or(0, |key_type| key_type.num_levels);
        if level >= width {
            // libX11 answers the second level of a one-level key of the first two groups with its first level (the core
            // protocol's four columns: two levels of two groups); every other level beyond the type is NoSymbol. Compared
            // with libX11 on the keymaps of `us,ru`, `ru,us`, `us,ru,de,fr`, `us,il,ara,th`, `us,ru,jp,kr` and `us,ru,ua,by`.
            return if group < 2 && level == 1 && width == 1 { key.syms.get(group * usize::from(key.width)).copied().unwrap_or(0) } else { 0 };
        }
        key.syms.get(group * usize::from(key.width) + usize::from(level)).copied().unwrap_or(0)
    }

    /// `XkbLookupKeySym` (`XkbTranslateKeyCode`): the symbol a key gives in a core state, with the group in bits 13-14.
    pub fn lookup_keysym(&self, keycode: u8, state: u16) -> Option<u32> {
        let key = self.key(keycode)?;
        let groups = key.groups();
        if groups == 0 {
            return None;
        }
        let mut group = usize::from((state >> 13) & 0x3);
        if group >= groups {
            group = match key.group_info & OUT_OF_RANGE_MASK {
                OUT_OF_RANGE_CLAMP => groups - 1,
                OUT_OF_RANGE_REDIRECT => {
                    let redirected = usize::from((key.group_info & REDIRECT_GROUP_MASK) >> REDIRECT_GROUP_SHIFT);
                    if redirected >= groups { 0 } else { redirected }
                }
                _ => group % groups,
            };
        }
        let mut column = group * usize::from(key.width);
        if let Some(key_type) = self.key_type(key, group) {
            let active = (state & 0xff) as u8 & key_type.mods_mask;
            if let Some(entry) = key_type.entries.iter().find(|entry| entry.active && entry.mods_mask == active) {
                column += usize::from(entry.level);
            }
        }
        key.syms.get(column).copied().filter(|keysym| *keysym != 0)
    }

    /// `XKeysymToKeycode`: the first key that carries the keysym in one of the first four columns (two levels of two
    /// groups), the columns in order.
    pub fn keycode_for_keysym(&self, keysym: u32) -> u8 {
        for column in 0u8..4 {
            for keycode in self.min_keycode..=self.max_keycode {
                if self.keysym(keycode, column / 2, column % 2) == keysym {
                    return keycode;
                }
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Key 10: `a`/`A` in group 0, `ф`/`Ф` in group 1 (two-level alphabetic type); key 11: one group, one level.
    fn map() -> KeyMap {
        let alphabetic = KeyType {
            mods_mask: 0x3,
            num_levels: 2,
            entries: vec![
                TypeEntry { active: true, mods_mask: 0x1, level: 1 },
                TypeEntry { active: true, mods_mask: 0x2, level: 1 },
            ],
        };
        let one_level = KeyType { mods_mask: 0, num_levels: 1, entries: vec![] };
        KeyMap {
            min_keycode: 10,
            max_keycode: 11,
            types: vec![alphabetic, one_level],
            keys: vec![
                KeySyms { kt_index: [0, 0, 0, 0], group_info: 2, width: 2, syms: vec![0x61, 0x41, 0x6c6, 0x6e6] },
                KeySyms { kt_index: [1, 0, 0, 0], group_info: 1, width: 1, syms: vec![0x20] },
            ],
        }
    }

    #[test]
    fn a_key_in_each_group_and_level() {
        let map = map();
        assert_eq!(map.keysym(10, 0, 0), 0x61);
        assert_eq!(map.keysym(10, 0, 1), 0x41);
        assert_eq!(map.keysym(10, 1, 0), 0x6c6);
        assert_eq!(map.keysym(10, 2, 0), 0);
        assert_eq!(map.keysym(11, 0, 0), 0x20);
        assert_eq!(map.keysym(11, 1, 0), 0, "a key with one group has nothing in the second");
        assert_eq!(map.keysym(11, 0, 1), 0x20, "the second level of a one-level key of the first groups is its first");
        assert_eq!(map.keysym(11, 0, 2), 0);
        assert_eq!(map.keysym(9, 0, 0), 0, "out of the key range");
    }

    #[test]
    fn the_lookup_applies_the_group_fallback_and_the_modifiers() {
        let map = map();
        assert_eq!(map.lookup_keysym(10, 1 << 13), Some(0x6c6));
        assert_eq!(map.lookup_keysym(10, (1 << 13) | 0x1), Some(0x6e6), "shift");
        assert_eq!(map.lookup_keysym(10, 0x2), Some(0x41), "lock turns a letter over");
        assert_eq!(map.lookup_keysym(10, 0x1 | 0x2), Some(0x61), "both modifiers match no entry: the first level");
        assert_eq!(map.lookup_keysym(10, 0x4), Some(0x61), "control is not in the type's mask");
        assert_eq!(map.lookup_keysym(11, 1 << 13), Some(0x20), "the second group wraps to the only one");
        assert_eq!(map.lookup_keysym(12, 0), None);
    }

    #[test]
    fn the_out_of_range_actions() {
        let mut map = map();
        map.keys[1].group_info = 1 | OUT_OF_RANGE_CLAMP;
        assert_eq!(map.lookup_keysym(11, 3 << 13), Some(0x20));
        map.keys[1].group_info = 1 | OUT_OF_RANGE_REDIRECT | (3 << REDIRECT_GROUP_SHIFT);
        assert_eq!(map.lookup_keysym(11, 1 << 13), Some(0x20), "a redirect outside the key's groups goes to the first");
    }

    #[test]
    fn a_keysym_is_found_by_column_before_keycode() {
        let map = map();
        assert_eq!(map.keycode_for_keysym(0x41), 10);
        assert_eq!(map.keycode_for_keysym(0x6c6), 10);
        assert_eq!(map.keycode_for_keysym(0x20), 11);
        assert_eq!(map.keycode_for_keysym(0x7a), 0);
    }
}

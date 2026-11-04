//! Numbers of Russian grammar used when a count is written into a text of either window or a message.

/// Russian pluralisation of a count (1 запись / 2 записи / 5 записей): the standard "count % 10, with
/// a % 100 teens exception" rule.
pub const RUSSIAN_PLURAL_MOD_TEN: u32 = 10;
pub const RUSSIAN_PLURAL_MOD_HUNDRED: u32 = 100;
pub const RUSSIAN_PLURAL_ELEVEN_EXCEPTION: u32 = 11;
pub const RUSSIAN_PLURAL_FEW_LAST_DIGITS: [u32; 3] = [2, 3, 4];
pub const RUSSIAN_PLURAL_TEEN_EXCEPTIONS: [u32; 3] = [12, 13, 14];

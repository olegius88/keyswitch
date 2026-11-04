//! Optional Hunspell integration used as morphological dictionary evidence (the Python's `spellcheck.py`).
//!
//! The application talks to the stable Hunspell C API, loaded at run time as the Python loads it with ctypes: nothing is
//! linked, so the program runs on a machine without Hunspell and then answers as `DisabledSpellChecker` does. The
//! dictionary roots, the file names tried and the argument handed to the library are `ks_lang::spellcheck`'s.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::{Mutex, OnceLock, PoisonError};

use ks_lang::SpellChecker;
use ks_lang::resources::SpellerFactory;
use ks_lang::spellcheck::{check_argument, find_dictionary};

use crate::constants::HUNSPELL_LIBRARY_NAMES;

type Create = unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_void;
type Destroy = unsafe extern "C" fn(*mut c_void);
type Spell = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
type DictionaryEncoding = unsafe extern "C" fn(*mut c_void) -> *const c_char;

/// The four functions of the library the program uses. The library stays loaded for the life of the process.
struct Library {
    _library: libloading::Library,
    create: Create,
    destroy: Destroy,
    spell: Spell,
    dictionary_encoding: DictionaryEncoding,
}

static LIBRARY: OnceLock<Option<Library>> = OnceLock::new();

fn library() -> Option<&'static Library> {
    LIBRARY
        .get_or_init(|| {
            for name in HUNSPELL_LIBRARY_NAMES {
                // SAFETY: loading a shared library runs its initialisers; Hunspell's are those of an ordinary C++ library.
                let Ok(library) = (unsafe { libloading::Library::new(name) }) else { continue };
                // SAFETY: the symbols are looked up by the names and signatures of Hunspell's C API (hunspell/hunspell.h):
                // `Hunspell_create`, `Hunspell_destroy`, `Hunspell_spell`, `Hunspell_get_dic_encoding`. The function
                // pointers are copied out and `_library` keeps the code mapped for as long as they live.
                unsafe {
                    let create = *library.get::<Create>(b"Hunspell_create\0").ok()?;
                    let destroy = *library.get::<Destroy>(b"Hunspell_destroy\0").ok()?;
                    let spell = *library.get::<Spell>(b"Hunspell_spell\0").ok()?;
                    let dictionary_encoding = *library.get::<DictionaryEncoding>(b"Hunspell_get_dic_encoding\0").ok()?;
                    return Some(Library { _library: library, create, destroy, spell, dictionary_encoding });
                }
            }
            None
        })
        .as_ref()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoding {
    Utf8,
    Latin1,
    /// A code page the Python knows and this port does not encode: every word is unknown, none is mis-encoded.
    Unsupported,
}

/// What Python's `codecs.lookup` would make of the name Hunspell reports, for the code pages dictionaries use.
fn encoding_of(name: &str) -> Encoding {
    let normalized = name.to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "utf8" | "utf_8" => Encoding::Utf8,
        "iso8859_1" | "iso_8859_1" | "latin1" | "latin_1" | "l1" | "8859" | "cp819" => Encoding::Latin1,
        _ if ["iso8859", "iso_8859", "koi8", "cp12", "windows_12", "latin", "cp8", "cp1", "mac"].iter().any(|prefix| normalized.starts_with(prefix)) => {
            Encoding::Unsupported
        }
        // A name Python does not know falls back to UTF-8.
        _ => Encoding::Utf8,
    }
}

struct Handle(*mut c_void);

// SAFETY: a Hunspell handle is used by one thread at a time: every call takes `HunspellDictionary::handle`'s mutex, as the
// Python guards its handle with a lock, and the pointer is never copied out of the struct.
unsafe impl Send for Handle {}

/// Small, thread-safe owner of one Hunspell dictionary handle.
pub struct HunspellDictionary {
    source: String,
    encoding: Encoding,
    handle: Mutex<Option<Handle>>,
}

impl HunspellDictionary {
    pub fn new(locale: &str, data_dir: &Path) -> HunspellDictionary {
        let closed = HunspellDictionary { source: String::new(), encoding: Encoding::Utf8, handle: Mutex::new(None) };
        let Some(library) = library() else { return closed };
        let Some((affix_path, dictionary_path)) = find_dictionary(locale, data_dir) else { return closed };
        let (Ok(affix), Ok(dictionary)) = (CString::new(affix_path.as_os_str().as_bytes()), CString::new(dictionary_path.as_os_str().as_bytes()))
        else {
            return closed;
        };
        // SAFETY: both arguments are NUL-terminated strings that outlive the call; the library copies what it keeps.
        let handle = unsafe { (library.create)(affix.as_ptr(), dictionary.as_ptr()) };
        if handle.is_null() {
            return closed;
        }
        // SAFETY: `handle` was just made by the library; the string it returns lives as long as the handle and is copied here.
        let reported = unsafe { (library.dictionary_encoding)(handle) };
        let encoding = if reported.is_null() {
            Encoding::Utf8
        } else {
            // SAFETY: the library returns a NUL-terminated string.
            encoding_of(&unsafe { CStr::from_ptr(reported) }.to_string_lossy())
        };
        HunspellDictionary { source: dictionary_path.to_string_lossy().into_owned(), encoding, handle: Mutex::new(Some(Handle(handle))) }
    }

    fn encode(&self, word: &str) -> Option<CString> {
        let bytes: Vec<u8> = match self.encoding {
            Encoding::Utf8 => word.as_bytes().to_vec(),
            Encoding::Latin1 => word.chars().map(|character| u8::try_from(u32::from(character)).ok()).collect::<Option<_>>()?,
            Encoding::Unsupported => return None,
        };
        CString::new(bytes).ok()
    }
}

impl SpellChecker for HunspellDictionary {
    fn available(&self) -> bool {
        self.handle.lock().unwrap_or_else(PoisonError::into_inner).is_some()
    }

    fn source(&self) -> &str {
        &self.source
    }

    fn check(&self, word: &str) -> bool {
        let Some(normalized) = check_argument(word) else { return false };
        let Some(encoded) = self.encode(normalized) else { return false };
        let (Some(library), guard) = (library(), self.handle.lock().unwrap_or_else(PoisonError::into_inner)) else { return false };
        match guard.as_ref() {
            // SAFETY: the handle is live (it is dropped only with the struct) and no other thread uses it, as the guard is held;
            // the word is a NUL-terminated string that outlives the call.
            Some(handle) => unsafe { (library.spell)(handle.0, encoded.as_ptr()) != 0 },
            None => false,
        }
    }
}

impl Drop for HunspellDictionary {
    fn drop(&mut self) {
        let handle = self.handle.get_mut().unwrap_or_else(PoisonError::into_inner).take();
        if let (Some(handle), Some(library)) = (handle, library()) {
            // SAFETY: the handle came from `Hunspell_create` and is destroyed once, here.
            unsafe { (library.destroy)(handle.0) };
        }
    }
}

/// The speller of a locale for `Resources::with_speller`: `HunspellDictionary(locale)`.
pub fn hunspell_speller(data_dir: std::path::PathBuf) -> SpellerFactory {
    Box::new(move |locale| Box::new(HunspellDictionary::new(locale, &data_dir)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_pages_are_told_as_python_tells_them() {
        assert_eq!(encoding_of("UTF-8"), Encoding::Utf8);
        assert_eq!(encoding_of("ISO8859-1"), Encoding::Latin1);
        assert_eq!(encoding_of("KOI8-R"), Encoding::Unsupported);
        assert_eq!(encoding_of("microsoft-cp1251"), Encoding::Utf8, "Python has no codec of that name");
        assert_eq!(encoding_of("ISO8859-15"), Encoding::Unsupported);
    }

    #[test]
    fn without_a_dictionary_the_speller_is_closed() {
        let missing = HunspellDictionary::new("xx_XX", Path::new("/nonexistent"));
        assert!(!missing.available() && missing.source().is_empty() && !missing.check("word"));
    }

    #[test]
    fn a_latin1_dictionary_cannot_be_asked_about_russian() {
        let dictionary = HunspellDictionary { source: String::new(), encoding: Encoding::Latin1, handle: Mutex::new(None) };
        assert_eq!(dictionary.encode("café").map(|word| word.into_bytes()), Some(vec![b'c', b'a', b'f', 0xe9]));
        assert!(dictionary.encode("слово").is_none());
    }

    /// The answers of the Python's `HunspellDictionary` for the same words on the same dictionaries, taken from it.
    #[test]
    fn the_real_library_answers_as_the_python_does() {
        let data = ks_config::history::data_dir();
        let english = HunspellDictionary::new("en_US", &data);
        let russian = HunspellDictionary::new("ru_RU", &data);
        if !english.available() || !russian.available() {
            eprintln!("Hunspell or its en_US/ru_RU dictionaries are not installed: skipped");
            return;
        }
        let long = "a".repeat(129);
        let words = [
            "hello", "Hello", "helo", "running", "ghbdtn", "  hello  ", "", long.as_str(), "привет", "приветствие", "руддщ",
            "компьютерами", "Москва", "hello,", "x",
        ];
        let want_english = [true, true, false, true, false, true, false, false, false, false, false, false, false, false, true];
        let want_russian = [false, false, false, false, false, false, false, false, true, true, false, true, true, false, false];
        for (index, word) in words.iter().enumerate() {
            assert_eq!(english.check(word), want_english[index], "en_US {word:?}");
            assert_eq!(russian.check(word), want_russian[index], "ru_RU {word:?}");
        }
        assert!(english.source().ends_with("en_US.dic") && russian.source().ends_with("ru_RU.dic"));
    }
}

//! Per-application input conventions that a language model cannot learn.
//!
//! One physical key carries `@` in the English layout and the quote in the Russian one, and in a chat client a word
//! that starts with it is usually a mention: `@name`. That is a fact about the application, not about either language,
//! so it lives here.
//!
//! A lone quote is shown as `@` the moment its key comes up, because the application offers its list of people only
//! after a real `@` and the list is how a mention is usually made. The layout stays as it was. Whatever is typed next
//! writes the quote back, and from there the engine analyses the word that follows on its own - `ощрт` is nothing in
//! Russian while `john` is a name, `привет` is an ordinary Russian word - and the head follows that decision: it
//! becomes `@` again when the word turns out to be the other layout, and stays a quote when the word is left alone. A
//! quotation therefore survives, and so does a quote typed after the user selected the layout by hand or moved the
//! caret, because those already stop the engine from touching the word.
//!
//! Every convention is one setting the user can turn off, and every condition is observable in the engine's own state:
//! which application has focus, which symbol is pending and which word grew after it.

use ks_text::pystr::casefold;

/// A pending symbol that may be the start of a mention in this application.
#[derive(Debug, PartialEq, Eq)]
pub struct MentionHead {
    pub setting: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub applications: &'static [&'static str],
    pub typed: &'static str,
    pub meant: &'static str,
}

impl MentionHead {
    pub fn applies(&self, application: &str, typed: &str, meant: &str) -> bool {
        if typed != self.typed || meant != self.meant {
            return false;
        }
        let name = casefold(application);
        self.applications.iter().any(|candidate| name.contains(candidate))
    }
}

pub static TELEGRAM_QUOTE_MENTION: MentionHead = MentionHead {
    setting: "applications.telegram_quote_mention",
    title: "Telegram: кавычка в начале слова — это @",
    description: "Кавычка, набранная в русской раскладке в начале слова, сразу показывается как «@», \
чтобы открылся список участников; раскладка не меняется. Если после неё набрать текст, \
«@» снова становится кавычкой и дальше решает слово: «ощрт» → «@john», а русское \
слово остаётся цитатой. Pause сразу после «@» возвращает кавычку.",
    applications: &["telegram", "kotatogram", "ayugram", "unigram", "forkgram"],
    typed: "\"",
    meant: "@",
};

pub static MENTION_HEADS: [&MentionHead; 1] = [&TELEGRAM_QUOTE_MENTION];

/// The convention that explains this pending symbol, or `None`.
pub fn mention_head(
    application: &str,
    typed: &str,
    meant: &str,
    enabled: &dyn Fn(&str) -> bool,
) -> Option<&'static MentionHead> {
    MENTION_HEADS.iter().copied().find(|head| head.applies(application, typed, meant) && enabled(head.setting))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_quote() {
        let on = |_: &str| true;
        let off = |_: &str| false;
        assert!(mention_head("Telegram Desktop", "\"", "@", &on).is_some());
        assert!(mention_head("ПРИЛОЖЕНИЕ AyuGram", "\"", "@", &on).is_some());
        assert!(mention_head("Telegram", "\"", "@", &off).is_none());
        assert!(mention_head("Telegram", "'", "@", &on).is_none());
        assert!(mention_head("Firefox", "\"", "@", &on).is_none());
        assert_eq!(TELEGRAM_QUOTE_MENTION.setting, "applications.telegram_quote_mention");
    }
}

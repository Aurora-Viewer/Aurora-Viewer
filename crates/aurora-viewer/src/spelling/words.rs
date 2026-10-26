//! The words of a text as the spell checker sees them. Port of the word
//! scan of LLLineEditor::draw / LLTextBase::drawMisspelled
//! (indra/llui/lllineeditor.cpp, lltextbase.cpp, originally LGPL 2.1): a
//! word is a run of letters, digits and `_`, an apostrophe inside it (a
//! contraction, "don't") included; words under 3 characters are never
//! checked (LLSpellChecker::checkSpelling).
//!
//! Deliberate differences, so that no squiggle lands where no dictionary
//! can help: a word holding a digit or a `_` ("x3", "2nd", "user_name") is
//! left alone, and so is a web or `secondlife://` address or an e-mail
//! address (the Legacy Viewer checked the words of a link one by one), and
//! an emoji code right after a `:`. The typographic apostrophe (’) counts
//! like the straight one.

use std::ops::Range;

/// The shortest word checked (LLSpellChecker::checkSpelling).
pub const MIN_CHARS: usize = 3;

/// One word to check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word {
    /// Its characters in the text (egui's cursors count characters).
    pub chars: Range<usize>,
    /// Its bytes in the text.
    pub bytes: Range<usize>,
}

fn is_part(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_apostrophe(c: char) -> bool {
    c == '\'' || c == '\u{2019}'
}

/// A run of text without white space that is an address rather than words.
fn is_address(chunk: &str) -> bool {
    let lower = chunk.trim_start_matches(|c: char| !c.is_alphanumeric()).to_ascii_lowercase();
    chunk.contains("://")
        || lower.starts_with("www.")
        || chunk
            .split_once('@')
            .is_some_and(|(user, host)| !user.is_empty() && host.contains('.') && !host.ends_with('.'))
}

/// The words of `text` worth checking, in order.
pub fn words(text: &str) -> Vec<Word> {
    let mut out = Vec::new();
    // char index of every byte offset visited, walked once
    let mut chars = 0;
    let mut iter = text.char_indices().peekable();
    // the end (in bytes) of the address being skipped
    let mut skip_until = 0;
    while let Some((at, c)) = iter.next() {
        if at < skip_until || !is_part(c) {
            chars += 1;
            continue;
        }
        // the chunk up to the next white space, for addresses
        let chunk_start = text[..at]
            .rfind(char::is_whitespace)
            .map_or(0, |i| i + text[i..].chars().next().map_or(1, char::len_utf8));
        let chunk_end = text[at..].find(char::is_whitespace).map_or(text.len(), |i| at + i);
        if is_address(&text[chunk_start..chunk_end]) {
            skip_until = chunk_end;
            chars += 1;
            continue;
        }
        let (start_byte, start_char) = (at, chars);
        let mut end_byte = at + c.len_utf8();
        chars += 1;
        let mut plain = c.is_alphabetic();
        let mut prev = c;
        while let Some(&(i, next)) = iter.peek() {
            if is_part(next) {
                plain &= next.is_alphabetic();
            } else if is_apostrophe(next) && prev.is_alphanumeric() {
                // an apostrophe joins two halves of one word only
                let after = text[i + next.len_utf8()..].chars().next();
                if !after.is_some_and(char::is_alphanumeric) {
                    break;
                }
            } else {
                break;
            }
            prev = next;
            end_byte = i + next.len_utf8();
            chars += 1;
            iter.next();
        }
        // an emoji code (":smile:", or ":hea" being typed, `ui/emoji/codes.rs`)
        let code = text[..start_byte].ends_with(':');
        if plain && !code && chars - start_char >= MIN_CHARS {
            out.push(Word {
                chars: start_char..chars,
                bytes: start_byte..end_byte,
            });
        }
    }
    out
}

/// The word of `words` under the character cursor `at` (its end
/// included, as LLLineEditor::getMisspelledWord).
pub fn at(words: &[(Range<usize>, String)], at: usize) -> Option<&(Range<usize>, String)> {
    words.iter().find(|(range, _)| range.start <= at && at <= range.end)
}

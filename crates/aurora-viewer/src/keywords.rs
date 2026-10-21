//! Keyword alerts in chat and IMs: a list of words that colours the
//! messages holding one and plays a sound when they arrive. Port of
//! FSKeywords (indra/newview/fskeywords.cpp), the keyword colour of
//! LLViewerChat::getChatColor (indra/newview/llviewerchat.cpp) and the
//! FSKeyword* / UISndFSKeywordSound settings (settings_per_account.xml),
//! originally LGPL 2.1, Copyright (C) The Phoenix Firestorm Project, Inc.
//!
//! The settings live here and are installed with [`set`] when they change;
//! the world asks [`matches`] when a line arrives (sound, system
//! notification) and the chat asks [`color`] when it draws one.
//!
//! Deliberate differences: an empty item of the list ("a,,b") matches
//! nothing (the Legacy Viewer's empty token matched every message), and
//! "whole words" means no letter, digit or `_` right before or after the
//! keyword, accents included (the Legacy `\b` treated é as a word end, and
//! never matched a keyword ending in punctuation, such as "c++").

use crate::settings::Rgba;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

/// UISndFSKeywordSound's default: the nearby chat sound.
pub const DEFAULT_SOUND: Uuid = Uuid::from_u128(0xa3f48b85_c29f_1f97_ebb6_644b7c053512);

/// Preferences › Chat › Keyword alerts, with the Legacy Viewer's defaults
/// (all off, an orange colour).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeywordSettings {
    /// FSKeywordOn.
    pub on: bool,
    /// FSKeywordInChat: nearby chat, objects' messages and the radar's and
    /// friends' notices in it.
    pub in_chat: bool,
    /// FSKeywordInIM: private messages, group chats and conferences.
    pub in_im: bool,
    /// FSKeywordSpeakersName: the sender's name counts too.
    pub speakers_name: bool,
    /// FSKeywordCaseSensitive.
    pub case_sensitive: bool,
    /// FSKeywordMatchWholeWords.
    pub whole_words: bool,
    /// FSKeywords: separated by commas, each trimmed.
    pub words: String,
    /// FSKeywordChangeColor and FSKeywordColor.
    pub change_color: bool,
    pub color: Rgba,
    /// PlayModeUISndFSKeywordSound and UISndFSKeywordSound (nil: silent).
    pub play_sound: bool,
    pub sound: Uuid,
}

impl Default for KeywordSettings {
    fn default() -> Self {
        KeywordSettings {
            on: false,
            in_chat: false,
            in_im: false,
            speakers_name: false,
            case_sensitive: false,
            whole_words: false,
            words: String::new(),
            change_color: false,
            // FSKeywordColor 1.0, 0.6, 0.0
            color: [255, 153, 0, 255],
            play_sound: false,
            sound: DEFAULT_SOUND,
        }
    }
}

/// The settings ready to test lines (FSKeywords::updateKeywords).
#[derive(Debug, Default)]
pub struct Matcher {
    words: Vec<String>,
    in_chat: bool,
    in_im: bool,
    speakers_name: bool,
    case_sensitive: bool,
    whole_words: bool,
    color: Option<Rgba>,
}

impl Matcher {
    pub fn new(s: &KeywordSettings) -> Matcher {
        let words = if s.on {
            s.words
                .split(',')
                .map(str::trim)
                .filter(|w| !w.is_empty())
                .map(|w| if s.case_sensitive { w.to_owned() } else { w.to_lowercase() })
                .collect()
        } else {
            Vec::new()
        };
        Matcher {
            words,
            in_chat: s.in_chat,
            in_im: s.in_im,
            speakers_name: s.speakers_name,
            case_sensitive: s.case_sensitive,
            whole_words: s.whole_words,
            color: s.change_color.then_some(s.color),
        }
    }

    /// A line holds a keyword (FSKeywords::chatContainsKeyword). `local`:
    /// nearby chat (objects' IMs and the notices shown there included),
    /// otherwise a conversation. The caller leaves out our own lines and the
    /// debug channel.
    pub fn matches(&self, from: &str, text: &str, local: bool) -> bool {
        if self.words.is_empty() || (local && !self.in_chat) || (!local && !self.in_im) {
            return false;
        }
        let joined;
        let source = if self.speakers_name {
            joined = format!("{from} {text}");
            joined.as_str()
        } else {
            text
        };
        let lowered;
        let source = if self.case_sensitive {
            source
        } else {
            lowered = source.to_lowercase();
            lowered.as_str()
        };
        self.words.iter().any(|w| {
            if self.whole_words {
                has_word(source, w)
            } else {
                source.contains(w.as_str())
            }
        })
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `word` stands in `text` with no word character right before or after it.
fn has_word(text: &str, word: &str) -> bool {
    let mut from = 0;
    while let Some(i) = text[from..].find(word) {
        let at = from + i;
        let end = at + word.len();
        let before = text[..at].chars().next_back().is_some_and(is_word);
        let after = text[end..].chars().next().is_some_and(is_word);
        if !before && !after {
            return true;
        }
        // the next occurrence may start inside this one
        from = at + text[at..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

static GLOBAL: RwLock<Option<Arc<Matcher>>> = RwLock::new(None);

/// Installs the current settings (at start and when they change).
pub fn set(s: &KeywordSettings) {
    if let Ok(mut g) = GLOBAL.write() {
        *g = Some(Arc::new(Matcher::new(s)));
    }
}

fn current() -> Option<Arc<Matcher>> {
    GLOBAL.read().ok().and_then(|g| g.clone())
}

/// A line arriving holds a keyword (its sound and its notification).
pub fn matches(from: &str, text: &str, local: bool) -> bool {
    current().is_some_and(|m| m.matches(from, text, local))
}

/// The colour of a line that holds a keyword, while "Change the colour" is
/// on (LLViewerChat::getChatColor, after the sender's colour).
pub fn color(from: &str, text: &str, local: bool) -> Option<Rgba> {
    let m = current()?;
    let color = m.color?;
    m.matches(from, text, local).then_some(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(words: &str) -> KeywordSettings {
        KeywordSettings {
            on: true,
            in_chat: true,
            in_im: true,
            words: words.into(),
            ..Default::default()
        }
    }

    #[test]
    fn legacy_defaults_are_off() {
        let d = KeywordSettings::default();
        assert!(!d.on && !d.in_chat && !d.in_im && !d.change_color && !d.play_sound);
        assert_eq!(d.sound, DEFAULT_SOUND);
        assert!(!Matcher::new(&KeywordSettings { words: "nova".into(), ..d }).matches("", "nova", true));
    }

    #[test]
    fn the_list_is_split_on_commas_trimmed_and_without_empty_items() {
        let m = Matcher::new(&on(" Party , ,builders,"));
        assert_eq!(m.words, vec!["party", "builders"]);
        assert!(m.matches("Tess", "A PARTY tonight", true));
        assert!(!m.matches("Tess", "nothing here", true), "an empty item matches nothing");
    }

    #[test]
    fn chat_and_ims_have_their_own_switch() {
        let mut s = on("party");
        s.in_im = false;
        let m = Matcher::new(&s);
        assert!(m.matches("", "party", true));
        assert!(!m.matches("", "party", false));
        s.in_im = true;
        s.in_chat = false;
        let m = Matcher::new(&s);
        assert!(!m.matches("", "party", true));
        assert!(m.matches("", "party", false));
        s.on = false;
        assert!(!Matcher::new(&s).matches("", "party", false));
    }

    #[test]
    fn case_and_names() {
        let mut s = on("Nova");
        assert!(Matcher::new(&s).matches("", "hi NOVA", true));
        s.case_sensitive = true;
        let m = Matcher::new(&s);
        assert!(!m.matches("", "hi nova", true));
        assert!(m.matches("", "hi Nova", true));
        assert!(!m.matches("Nova Exemple", "hello", true));
        s.speakers_name = true;
        assert!(Matcher::new(&s).matches("Nova Exemple", "hello", true));
    }

    #[test]
    fn whole_words_need_no_word_character_around() {
        let mut s = on("art, c++, café");
        assert!(Matcher::new(&s).matches("", "a party", true), "a part of a word matches by default");
        s.whole_words = true;
        let m = Matcher::new(&s);
        assert!(!m.matches("", "a party", true));
        assert!(m.matches("", "art!", true));
        assert!(m.matches("", "(art)", true));
        assert!(m.matches("", "I like c++ a lot", true));
        assert!(!m.matches("", "cafés", true));
        assert!(m.matches("", "un café, merci", true));
        assert!(!m.matches("", "smart_art", true));
        // a later occurrence still counts
        assert!(m.matches("", "party art", true));
    }

    #[test]
    fn has_word_finds_an_occurrence_inside_an_earlier_one() {
        // "ba-a" fails at 1; "a-a" at 3 overlaps it
        assert!(has_word("ba-a-a", "a-a"));
        assert!(has_word("aaa aa", "aa"));
        assert!(!has_word("aaa", "aa"));
    }
}

//! Which language a message is written in, without the model.

use crate::lang::{Lang, lang};
use whatlang::Lang as W;

/// A detected language and how sure whatlang is (0 to 1).
#[derive(Debug)]
pub struct Detected {
    pub lang: &'static Lang,
    pub confidence: f64,
}

/// The language of `text`, when whatlang knows it and it is one of `LANGS`.
pub fn detect(text: &str) -> Option<Detected> {
    let info = whatlang::detect(text)?;
    let code = match info.lang() {
        W::Eng => "en",
        W::Spa => "es",
        W::Fra => "fr",
        W::Deu => "de",
        W::Ita => "it",
        W::Por => "pt",
        W::Nld => "nl",
        W::Pol => "pl",
        W::Rus => "ru",
        W::Ukr => "uk",
        W::Tur => "tr",
        W::Ara => "ar",
        W::Heb => "he",
        W::Pes => "fa",
        W::Hin => "hi",
        W::Ben => "bn",
        W::Ind => "id",
        W::Vie => "vi",
        W::Tha => "th",
        W::Cmn => "zh",
        W::Jpn => "ja",
        W::Kor => "ko",
        W::Swe => "sv",
        W::Dan => "da",
        W::Nob => "no",
        W::Fin => "fi",
        W::Ces => "cs",
        W::Ell => "el",
        W::Ron => "ro",
        W::Hun => "hu",
        _ => return None,
    };
    Some(Detected {
        lang: lang(code)?,
        confidence: info.confidence(),
    })
}

/// Text the model has nothing to do with: only links, emoji, digits and
/// punctuation, or a single letter, once the links are taken out. Two-letter
/// words ("ok", "lol" is three) are kept as they are too: the same in most
/// languages.
pub fn nothing_to_translate(text: &str) -> bool {
    let letters: usize = text
        .split_whitespace()
        .filter(|w| !(w.contains("://") || w.starts_with("www.")))
        .map(|w| w.chars().filter(|c| c.is_alphabetic()).count())
        .sum();
    letters <= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_spanish() {
        let d = detect("¿Dónde estás? Te estoy buscando en la tienda.").expect("detected");
        assert_eq!(d.lang.code, "es");
        assert!(d.confidence >= 0.9, "{}", d.confidence);
    }

    #[test]
    fn nothing_to_translate_cases() {
        for t in [
            "https://example.com/x",
            "secondlife:///app/agent/0000/about",
            "😀👍",
            "42!",
            "ok",
            "  ",
        ] {
            assert!(nothing_to_translate(t), "{t}");
        }
        assert!(!nothing_to_translate("hola amigo"));
        assert!(!nothing_to_translate("see https://example.com please"));
    }
}

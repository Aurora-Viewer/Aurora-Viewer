//! The text given to the model and the cleanup of what it answers.
//!
//! TranslateGemma's own chat template takes structured content
//! (`source_lang_code`, `target_lang_code`) that llama.cpp's template engine
//! does not handle, so the turn is written here, with the wording of the
//! official template (https://huggingface.co/google/translategemma-4b-it).

use crate::lang::{LANGS, Lang};

/// The prompt format of a catalogue model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    TranslateGemma,
}

/// The whole prompt: one user turn asking for the translation of `text`,
/// then the opening of the model's turn.
pub fn prompt(kind: PromptKind, text: &str, from: &Lang, to: &Lang) -> String {
    match kind {
        PromptKind::TranslateGemma => {
            let (s, sc, t, tc) = (from.english, from.code, to.english, to.code);
            format!(
                "<start_of_turn>user\nYou are a professional {s} ({sc}) to {t} ({tc}) translator. \
                 Your goal is to accurately convey the meaning and nuances of the original {s} text \
                 while adhering to {t} grammar, vocabulary, and cultural sensitivities.\n\
                 Produce only the {t} translation, without any additional explanations or commentary. \
                 Please translate the following {s} text into {t}:\n\n\n\
                 {text}<end_of_turn>\n<start_of_turn>model\n"
            )
        }
    }
}

/// Why an answer of the model is not used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    Empty,
    /// More than four times the length of the message: the model rambled.
    TooLong,
    /// The model talked instead of translating.
    Chatter,
}

const CHATTER: [&str; 4] = ["sure", "here is", "here's", "certainly"];

/// The translation in the model's answer: trimmed, without the end-of-turn
/// marker, a label such as "Translation:" or "Spanish:", nor one pair of
/// quotes around it.
pub fn clean_output(raw: &str, input: &str) -> Result<String, Rejected> {
    let mut out = raw.split("<end_of_turn>").next().unwrap_or("").trim();
    if let Some((label, rest)) = out.split_once(':') {
        let label = label.trim().to_lowercase();
        let named = label == "translation"
            || LANGS
                .iter()
                .any(|l| l.english.to_lowercase() == label || l.native.to_lowercase() == label);
        if named {
            out = rest.trim();
        }
    }
    for (open, close) in [('"', '"'), ('“', '”'), ('«', '»'), ('\'', '\'')] {
        if out.chars().count() >= 2 && out.starts_with(open) && out.ends_with(close) {
            out = out[open.len_utf8()..out.len() - close.len_utf8()].trim();
            break;
        }
    }
    if out.is_empty() {
        return Err(Rejected::Empty);
    }
    let lower = out.to_lowercase();
    if CHATTER.iter().any(|c| lower.starts_with(c)) {
        return Err(Rejected::Chatter);
    }
    if out.chars().count() > 4 * input.chars().count().max(1) {
        return Err(Rejected::TooLong);
    }
    Ok(out.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::lang;

    fn en() -> &'static crate::Lang {
        lang("en").expect("en")
    }
    fn es() -> &'static crate::Lang {
        lang("es").expect("es")
    }

    #[test]
    fn prompt_is_one_gemma_turn_ending_with_the_text() {
        let p = prompt(PromptKind::TranslateGemma, "Hello there", en(), es());
        assert!(p.starts_with("<start_of_turn>user\n"));
        assert!(p.ends_with("Hello there<end_of_turn>\n<start_of_turn>model\n"));
        assert!(p.contains("English (en)") && p.contains("Spanish (es)"));
    }

    #[test]
    fn clean_strips_quotes_and_prefixes() {
        assert_eq!(clean_output("\"Hola\"", "Hello"), Ok("Hola".into()));
        assert_eq!(clean_output("Translation: Hola", "Hello"), Ok("Hola".into()));
        assert_eq!(clean_output("Spanish: Hola\n", "Hello"), Ok("Hola".into()));
        assert_eq!(clean_output("«Hola»<end_of_turn>", "Hello"), Ok("Hola".into()));
        // a colon inside the message stays
        assert_eq!(clean_output("Nota: ven", "Note: come"), Ok("Nota: ven".into()));
    }

    #[test]
    fn clean_rejects() {
        assert_eq!(clean_output("", "Hello"), Err(Rejected::Empty));
        assert_eq!(clean_output("   ", "Hello"), Err(Rejected::Empty));
        assert_eq!(clean_output(&"a".repeat(41), "0123456789"), Err(Rejected::TooLong));
        assert_eq!(clean_output("Sure, here is the translation: Hola", "Hello"), Err(Rejected::Chatter));
        assert_eq!(clean_output("Here's the translation", "Hello world"), Err(Rejected::Chatter));
    }
}

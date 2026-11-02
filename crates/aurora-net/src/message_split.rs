//! Long chat lines and IMs sent as several messages. The simulator keeps
//! the first 1023 bytes of a ChatFromViewer or ImprovedInstantMessage text
//! (MAX_MSG_BUF_SIZE - 1, indra/llmessage/lldbstrings.h), so a longer one
//! leaves in parts, cut at the last space within the limit, else at a
//! character boundary. Port of the split loop of LLIMModel::sendMessage
//! (indra/newview/llimview.cpp) and FSNearbyChat::sendChatFromViewer
//! (indra/newview/fsnearbychathub.cpp), originally LGPL 2.1. The Legacy
//! Viewer sends the space it cut at as the first character of the next
//! part; here it is dropped, the recipient sees no stray leading space.

/// Largest text of one chat or IM message, in bytes.
pub const MAX_MESSAGE_BYTES: usize = 1023;

/// The parts `text` is sent as, in order; none for an empty text.
pub fn message_parts(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = text;
    while rest.len() > MAX_MESSAGE_BYTES {
        let bytes = rest.as_bytes();
        // a space right after the limit cuts there too, as in the Legacy loop
        let cut = match (1..=MAX_MESSAGE_BYTES).rev().find(|&i| bytes[i] == b' ') {
            Some(space) => {
                parts.push(&rest[..space]);
                space + 1
            }
            None => {
                let mut end = MAX_MESSAGE_BYTES;
                while !rest.is_char_boundary(end) {
                    end -= 1;
                }
                parts.push(&rest[..end]);
                end
            }
        };
        rest = &rest[cut..];
    }
    if !rest.is_empty() {
        parts.push(rest);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_and_empty_texts() {
        assert_eq!(message_parts("hello"), ["hello"]);
        assert_eq!(message_parts(&"a".repeat(MAX_MESSAGE_BYTES)), ["a".repeat(MAX_MESSAGE_BYTES)]);
        assert!(message_parts("").is_empty());
    }

    #[test]
    fn words_are_cut_at_the_last_space() {
        // 2,600 bytes of text: three parts, no word cut in two
        let words = "word ".repeat(520);
        let parts = message_parts(words.trim_end());
        assert_eq!(parts.len(), 3);
        assert!(
            parts
                .iter()
                .all(|p| p.len() <= MAX_MESSAGE_BYTES && !p.starts_with(' ') && p.ends_with("word"))
        );
        assert_eq!(parts.join(" "), words.trim_end());
    }

    #[test]
    fn a_space_just_past_the_limit_cuts_there() {
        let text = format!("{} next", "x".repeat(MAX_MESSAGE_BYTES));
        assert_eq!(message_parts(&text), ["x".repeat(MAX_MESSAGE_BYTES), "next".into()]);
    }

    #[test]
    fn no_space_cuts_at_a_character_boundary() {
        let text = "é".repeat(1500);
        let parts = message_parts(&text);
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.len() <= MAX_MESSAGE_BYTES));
        assert_eq!(parts.concat(), text);
    }
}

//! Notecard import/export following LLNotecard and LLViewerTextEditor
//! (Firestorm indra/llinventory/llnotecard.cpp and indra/newview/llviewertexteditor.cpp,
//! originally LGPL 2.1). Inventory records stay opaque so all permissions survive.

use std::collections::{HashMap, HashSet};

pub const MAX_TEXT_BYTES: usize = 65_536;
pub const FIRST_EMBEDDED_CHAR: u32 = 0x100000;

#[derive(Clone, Debug)]
struct EmbeddedItem {
    name: String,
    record: String,
}

#[derive(Clone, Debug, Default)]
pub struct Notecard {
    pub text: String,
    items: Vec<EmbeddedItem>,
}

fn invalid() -> String {
    aurora_i18n::tr!("world-inventory-preview-invalid-notecard").into()
}

fn line<'a>(data: &'a [u8], cursor: &mut usize) -> Result<&'a str, String> {
    let rest = data.get(*cursor..).ok_or_else(invalid)?;
    let end = rest.iter().position(|b| *b == b'\n').ok_or_else(invalid)?;
    let value = std::str::from_utf8(&rest[..end]).map_err(|_| invalid())?;
    *cursor += end + 1;
    Ok(value.trim())
}

fn require(data: &[u8], cursor: &mut usize, value: &str) -> Result<(), String> {
    if line(data, cursor)? != value {
        return Err(invalid());
    }
    Ok(())
}

impl Notecard {
    pub fn decode(data: &[u8]) -> Result<Self, String> {
        // Empty assets are newly created cards; plain text is also accepted by
        // the existing inventory preview, without interpreting it as a header.
        if !data.starts_with(b"Linden text version") {
            let text = std::str::from_utf8(data).map_err(|_| invalid())?.to_owned();
            Self::validate_text(&text)?;
            return Ok(Self { text, items: Vec::new() });
        }
        let mut cursor = 0;
        let version = match line(data, &mut cursor)? {
            "Linden text version 1" => 1,
            "Linden text version 2" => 2,
            _ => return Err(invalid()),
        };
        require(data, &mut cursor, "{")?;
        require(data, &mut cursor, "LLEmbeddedItems version 1")?;
        require(data, &mut cursor, "{")?;
        let count: usize = line(data, &mut cursor)?
            .strip_prefix("count ")
            .and_then(|n| n.trim().parse().ok())
            .filter(|n| *n <= MAX_TEXT_BYTES && *n <= data.len())
            .ok_or_else(invalid)?;
        let mut items = Vec::new();
        for index in 0..count {
            require(data, &mut cursor, "{")?;
            let item_index: usize = line(data, &mut cursor)?
                .strip_prefix("ext char index ")
                .and_then(|n| n.trim().parse().ok())
                .ok_or_else(invalid)?;
            if item_index != index {
                return Err(invalid());
            }
            let start = cursor;
            let mut depth = 1usize;
            let mut end = start;
            let mut name = String::new();
            while depth > 0 {
                end = cursor;
                let value = line(data, &mut cursor)?;
                match value {
                    "{" => depth += 1,
                    "}" => depth -= 1,
                    _ if depth == 2 => {
                        if let Some(value) = value.strip_prefix("name").and_then(|s| s.strip_prefix([' ', '\t'])) {
                            name = value.trim_start().trim_end_matches('|').into();
                        }
                    }
                    _ => {}
                }
            }
            let record = std::str::from_utf8(&data[start..end]).map_err(|_| invalid())?;
            if !record.split_whitespace().take(2).eq(["inv_item", "0"]) {
                return Err(invalid());
            }
            items.push(EmbeddedItem {
                name,
                record: record.into(),
            });
        }
        require(data, &mut cursor, "}")?;
        let len: usize = line(data, &mut cursor)?
            .strip_prefix("Text length ")
            .and_then(|n| n.trim().parse().ok())
            .filter(|n| *n <= MAX_TEXT_BYTES)
            .ok_or_else(invalid)?;
        let end = cursor.checked_add(len).ok_or_else(invalid)?;
        let body = data.get(cursor..end).ok_or_else(invalid)?;
        let footer = data.get(end..).ok_or_else(invalid)?;
        if footer.iter().copied().filter(|b| !b.is_ascii_whitespace() && *b != 0).ne(*b"}") {
            return Err(invalid());
        }
        let text = if version == 1 {
            body.iter()
                .map(|b| {
                    if *b >= 0x80 {
                        // This range is always a valid Unicode scalar.
                        char::from_u32(FIRST_EMBEDDED_CHAR + u32::from(*b - 0x80)).unwrap_or('\u{fffd}')
                    } else {
                        char::from(*b)
                    }
                })
                .collect()
        } else {
            std::str::from_utf8(body).map_err(|_| invalid())?.to_owned()
        };
        let card = Self { text, items };
        card.validate_references(&card.text)?;
        Ok(card)
    }

    pub fn validate_text(text: &str) -> Result<(), String> {
        if text.len() > MAX_TEXT_BYTES {
            return Err(aurora_i18n::tr!("notecard-text-too-large").into());
        }
        if text.contains('\0') {
            return Err(invalid());
        }
        Ok(())
    }

    fn validate_references(&self, text: &str) -> Result<(), String> {
        Self::validate_text(text)?;
        if text
            .chars()
            .any(|c| c as u32 >= FIRST_EMBEDDED_CHAR && (c as u32 - FIRST_EMBEDDED_CHAR) as usize >= self.items.len())
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn attachment_names(&self, text: &str) -> Vec<&str> {
        let referenced: HashSet<_> = text.chars().filter_map(|c| (c as u32).checked_sub(FIRST_EMBEDDED_CHAR)).collect();
        self.items
            .iter()
            .enumerate()
            .filter(|(index, _)| referenced.contains(&(*index as u32)))
            .map(|(_, item)| item.name.as_str())
            .collect()
    }

    pub fn encode(&self, text: &str) -> Result<Vec<u8>, String> {
        self.validate_references(text)?;
        // Firestorm exports only referenced items and renumbers the text and
        // records together. Removing a marker removes that embedded item.
        let mut used = HashMap::new();
        for c in text.chars().filter(|c| *c as u32 >= FIRST_EMBEDDED_CHAR) {
            used.insert(c as u32 - FIRST_EMBEDDED_CHAR, 0u32);
        }
        let mut embedded = format!("LLEmbeddedItems version 1\n{{\ncount {}\n", used.len());
        let mut next = 0;
        for (index, item) in self.items.iter().enumerate() {
            if let Some(mapped) = used.get_mut(&(index as u32)) {
                *mapped = next;
                embedded.push_str(&format!("{{\next char index {next}\n{}}}\n", item.record));
                next += 1;
            }
        }
        embedded.push_str("}\n");
        let text: String = text
            .chars()
            .map(|c| {
                if c as u32 >= FIRST_EMBEDDED_CHAR {
                    char::from_u32(FIRST_EMBEDDED_CHAR + used[&(c as u32 - FIRST_EMBEDDED_CHAR)]).unwrap_or('\u{fffd}')
                } else {
                    c
                }
            })
            .collect();
        Ok(format!("Linden text version 2\n{{\n{embedded}Text length {}\n{text}}}\n", text.len()).into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn embedded(text: &str) -> Vec<u8> {
        format!("Linden text version 2\n{{\nLLEmbeddedItems version 1\n{{\ncount 2\n{{\next char index 0\ninv_item 0\n{{\npermissions 0\n{{\nowner_mask 0000a000\n}}\nname Photo|\n}}\n}}\n{{\next char index 1\ninv_item 0\n{{\nname Repère|\n}}\n}}\n}}\nText length {}\n{text}}}\n", text.len()).into_bytes()
    }

    #[test]
    fn utf8_size_empty_cards_and_exact_limit_round_trip() {
        for text in [
            String::new(),
            "Été 🐺\nText length 99\ncount 0\n}".into(),
            "é".repeat(MAX_TEXT_BYTES / 2),
        ] {
            let bytes = Notecard::default().encode(&text).expect("encode");
            assert_eq!(Notecard::decode(&bytes).expect("decode").text, text);
        }
        assert!(Notecard::default().encode(&"é".repeat(MAX_TEXT_BYTES / 2 + 1)).is_err());
        assert!(Notecard::default().encode("a\0b").is_err());
    }

    #[test]
    fn attachments_keep_permissions_and_follow_deleted_markers() {
        let text = "Photo \u{100000}\nRepère \u{100001}";
        let card = Notecard::decode(&embedded(text)).expect("embedded card");
        assert_eq!(card.attachment_names(text), ["Photo", "Repère"]);
        let bytes = card.encode(&format!("Ajout é\n{text}")).expect("save");
        let saved = Notecard::decode(&bytes).expect("saved card");
        assert_eq!(saved.items[0].record, card.items[0].record);
        assert!(saved.items[0].record.contains("owner_mask 0000a000"));
        let saved = Notecard::decode(&card.encode("Repère \u{100001}").expect("delete photo")).expect("renumbered");
        assert_eq!(saved.text, "Repère \u{100000}");
        assert_eq!(saved.attachment_names(&saved.text), ["Repère"]);
        assert!(card.encode("\u{100002}").is_err());
    }

    #[test]
    fn invalid_headers_counts_lengths_and_utf8_do_not_panic() {
        let bytes = embedded("é");
        let text = std::str::from_utf8(&bytes).expect("fixture");
        for invalid in [
            text.replace("version 2", "version 3"),
            text.replace("count 2", "count -1"),
            text.replace("count 2", "count 999999999"),
            text.replace("ext char index 1", "ext char index 0"),
            text.replace("Text length 2", "Text length 1"),
            text.replace("Text length 2", "Text length 65537"),
            text.replace("Text length 2", &format!("Text length {}", usize::MAX)),
        ] {
            assert!(Notecard::decode(invalid.as_bytes()).is_err());
        }
        for len in b"Linden text version".len()..bytes.len() - 2 {
            assert!(Notecard::decode(&bytes[..len]).is_err());
        }
    }

    #[test]
    fn legacy_ascii_attachment_markers_are_exported_as_utf8() {
        let mut bytes = embedded("x");
        bytes = String::from_utf8(bytes)
            .expect("fixture")
            .replace("version 2", "version 1")
            .into_bytes();
        let at = bytes.iter().rposition(|b| *b == b'x').expect("text");
        bytes[at] = 0x81;
        let card = Notecard::decode(&bytes).expect("v1");
        assert_eq!(card.text, "\u{100001}");
        let saved = Notecard::decode(&card.encode(&card.text).expect("v2 export")).expect("v2");
        assert_eq!(saved.text, "\u{100000}");
        assert_eq!(saved.attachment_names(&saved.text), ["Repère"]);
    }
}

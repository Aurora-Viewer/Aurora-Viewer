//! Notecard import/export following LLNotecard and LLViewerTextEditor
//! (Firestorm indra/llinventory/llnotecard.cpp and indra/newview/llviewertexteditor.cpp,
//! originally LGPL 2.1). Inventory records stay opaque so all permissions survive.

use super::InvItem;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub const MAX_TEXT_BYTES: usize = 65_536;
pub const FIRST_EMBEDDED_CHAR: u32 = 0x100000;

#[derive(Clone, Debug)]
pub struct EmbeddedItem {
    pub item: InvItem,
    /// Newly dropped items must be saved before copying them from the card.
    pub saved: bool,
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
            while depth > 0 {
                end = cursor;
                let value = line(data, &mut cursor)?;
                match value {
                    "{" => depth += 1,
                    "}" => depth -= 1,
                    _ => {}
                }
            }
            let record = std::str::from_utf8(&data[start..end]).map_err(|_| invalid())?;
            if !record.split_whitespace().take(2).eq(["inv_item", "0"]) {
                return Err(invalid());
            }
            let item = crate::task_inventory::parse_task_inventory(record)
                .first()
                .and_then(crate::build::TaskItem::to_inv_item)
                .ok_or_else(invalid)?;
            items.push(EmbeddedItem {
                item,
                saved: true,
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
            .map(|(_, item)| item.item.name.as_str())
            .collect()
    }

    pub fn attachment(&self, marker: char) -> Option<&EmbeddedItem> {
        self.items.get((marker as u32).checked_sub(FIRST_EMBEDDED_CHAR)? as usize)
    }

    /// Clipboard references are local to an editor. Remap them together with
    /// their opaque records instead of interpreting another card's index here.
    pub fn paste(&mut self, text: &str, source: &Self) -> Result<String, String> {
        let mut mapped = HashMap::new();
        let mut pasted = String::new();
        let mut additions = Vec::new();
        for c in text.chars() {
            if (c as u32) < FIRST_EMBEDDED_CHAR {
                pasted.push(c);
                continue;
            }
            let attachment = source.attachment(c).ok_or_else(invalid)?;
            let marker = if let Some(marker) = mapped.get(&c) {
                *marker
            } else {
                let marker = char::from_u32(FIRST_EMBEDDED_CHAR + (self.items.len() + additions.len()) as u32).ok_or_else(invalid)?;
                let mut attachment = attachment.clone();
                attachment.saved = false;
                additions.push(attachment);
                mapped.insert(c, marker);
                marker
            };
            pasted.push(marker);
        }
        self.items.extend(additions);
        Ok(pasted)
    }

    /// LLViewerTextEditor::handleDragAndDrop: inventory items, unrestricted
    /// next-owner permissions, supported cargo types (folders and links excluded).
    pub fn can_embed(item: &InvItem) -> Result<(), String> {
        if !matches!(item.asset_type, 0 | 1 | 2 | 3 | 5 | 6 | 7 | 10 | 13 | 20 | 21 | 49 | 56 | 57) {
            return Err(aurora_i18n::tr!("notecard-drop-type").into());
        }
        if item.next_owner_mask & 0xe000 != 0xe000 {
            return Err(aurora_i18n::tr!("notecard-drop-permissions").into());
        }
        if item.id.is_nil() {
            return Err(invalid());
        }
        Ok(())
    }

    pub fn insert_attachment(&mut self, text: &mut String, position: usize, item: &InvItem) -> Result<char, String> {
        Self::can_embed(item)?;
        self.validate_references(text)?;
        let marker = char::from_u32(FIRST_EMBEDDED_CHAR + self.items.len() as u32).ok_or_else(invalid)?;
        if text.len() + marker.len_utf8() > MAX_TEXT_BYTES {
            return Err(aurora_i18n::tr!("notecard-text-too-large").into());
        }
        let byte = text.char_indices().nth(position).map_or(text.len(), |(i, _)| i);
        self.items.push(EmbeddedItem {
            item: item.clone(),
            saved: false,
            record: legacy_record(item),
        });
        text.insert(byte, marker);
        Ok(marker)
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

/// LLInventoryItem::exportLegacyStream and LLPermissions::exportLegacyStream.
/// Existing records remain byte-for-byte intact; only new inventory drops use this.
fn legacy_record(item: &InvItem) -> String {
    let asset_type = match item.asset_type {
        0 => "texture",
        1 => "sound",
        2 => "callcard",
        3 => "landmark",
        5 => "clothing",
        6 => "object",
        7 => "notecard",
        10 => "lsltext",
        13 => "bodypart",
        20 => "animatn",
        21 => "gesture",
        49 => "mesh",
        56 => "settings",
        57 => "material",
        _ => "none",
    };
    let inv_type = match item.inv_type {
        0 => "texture",
        1 => "sound",
        2 => "callcard",
        3 => "landmark",
        6 => "object",
        7 => "notecard",
        10 => "script",
        15 => "snapshot",
        17 => "attach",
        18 => "wearable",
        19 => "animation",
        20 => "gesture",
        22 => "mesh",
        25 => "settings",
        26 => "material",
        _ => "none",
    };
    let (asset_key, asset) = if item.base_mask & 0xe000 == 0xe000 || item.asset_id.is_nil() {
        ("asset_id", item.asset_id)
    } else {
        (
            "shadow_id",
            Uuid::from_u128(item.asset_id.as_u128() ^ 0x3c115e51_04f4_523c_9fa6_98aff1034730),
        )
    };
    let clean = |s: &str| {
        s.chars()
            .map(|c| if c == '|' || c.is_control() { ' ' } else { c })
            .collect::<String>()
    };
    let sale = match item.sale_type {
        1 => "orig",
        2 => "copy",
        3 => "cntn",
        _ => "not",
    };
    let metadata = if item.thumbnail.is_nil() {
        String::new()
    } else {
        let xml = aurora_llsd::to_xml_string(&aurora_llsd::llsd_map! {
            "thumbnail" => aurora_llsd::llsd_map! { "asset_id" => item.thumbnail }
        });
        // LLInventoryItem reads inline <llsd> metadata, without an XML prolog.
        let xml = xml.find("<llsd>").map_or(xml.as_str(), |start| &xml[start..]);
        format!("\t\tmetadata\t{xml}|\n")
    };
    let group_owned = if item.group_owned { "\t\tgroup_owned\t1\n" } else { "" };
    // LLNotecard matches "inv_item\t0\n" byte-for-byte; name and desc use
    // scanf "%s%[\t]%[^|]". Spaces are not interchangeable with tabs here.
    format!(
        "\tinv_item\t0\n\t{{\n\t\titem_id\t{}\n\t\tparent_id\t{}\n\tpermissions 0\n\t{{\n\t\tbase_mask\t{:08x}\n\t\towner_mask\t{:08x}\n\t\tgroup_mask\t{:08x}\n\t\teveryone_mask\t{:08x}\n\t\tnext_owner_mask\t{:08x}\n\t\tcreator_id\t{}\n\t\towner_id\t{}\n\t\tlast_owner_id\t{}\n\t\tgroup_id\t{}\n{group_owned}\t}}\n{metadata}\t\t{asset_key}\t{asset}\n\t\ttype\t{asset_type}\n\t\tinv_type\t{inv_type}\n\t\tflags\t{:08x}\n\tsale_info\t0\n\t{{\n\t\tsale_type\t{sale}\n\t\tsale_price\t{}\n\t}}\n\t\tname\t{}|\n\t\tdesc\t{}|\n\t\tcreation_date\t{}\n\t}}\n",
        item.id,
        item.parent,
        item.base_mask,
        item.owner_mask,
        item.group_mask,
        item.everyone_mask,
        item.next_owner_mask,
        item.creator,
        item.owner,
        item.last_owner,
        item.group_id,
        item.flags,
        item.sale_price,
        clean(&item.name),
        clean(&item.desc),
        item.created_at
    )
}

/// copy_inventory_from_notecard (llviewerinventory.cpp): hyphenated fields
/// are protocol names; a nil folder lets the simulator choose the type folder.
pub fn copy_body(notecard: Uuid, object: Uuid, item: Uuid, callback: u32) -> aurora_llsd::Llsd {
    aurora_llsd::llsd_map! { "notecard-id" => notecard, "object-id" => object, "item-id" => item,
    "folder-id" => Uuid::nil(), "callback-id" => callback as i32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: i32, inv_type: i32) -> InvItem {
        InvItem {
            id: Uuid::from_u128(1),
            parent: Uuid::from_u128(2),
            asset_id: Uuid::from_u128(3),
            creator: Uuid::from_u128(4),
            owner: Uuid::from_u128(5),
            last_owner: Uuid::from_u128(6),
            group_id: Uuid::from_u128(7),
            group_owned: true,
            base_mask: 0x7fffffff,
            owner_mask: 0x7fffffff,
            next_owner_mask: 0xe000,
            group_mask: 0x8000,
            everyone_mask: 0x2000,
            asset_type: kind,
            inv_type,
            name: "Été 🐺".into(),
            desc: "Description".into(),
            flags: 42,
            created_at: 1234,
            sale_type: 2,
            sale_price: 12,
            ..Default::default()
        }
    }

    #[test]
    fn new_item_records_use_firestorms_exact_header_and_tab_delimited_strings() {
        let mut card = Notecard::default();
        let mut text = String::new();
        let mut item = item(0, 0);
        item.thumbnail = Uuid::from_u128(70);
        let marker = card.insert_attachment(&mut text, 0, &item).expect("drop");
        let record = &card.attachment(marker).expect("item").record;
        // Literal comparison in LLNotecard, and scanf "%s%[\t]%[^|]"
        // for name/desc in LLInventoryItem::importLegacyStream.
        assert!(record.trim_start().starts_with("inv_item\t0\n"));
        assert!(record.contains("\n\t\tname\tÉté 🐺|\n"));
        assert!(record.contains("\n\t\tdesc\tDescription|\n"));
        let metadata = record
            .lines()
            .find_map(|line| line.trim_start().strip_prefix("metadata\t"))
            .expect("metadata");
        assert!(
            metadata.starts_with("<llsd>"),
            "legacy inventory imports XML only when the token starts with <llsd>"
        );
        let parsed = aurora_llsd::from_xml(metadata.trim_end_matches('|').as_bytes()).expect("thumbnail XML");
        assert_eq!(parsed["thumbnail"]["asset_id"].as_uuid(), item.thumbnail);
    }

    #[test]
    fn inventory_drops_preserve_types_assets_permissions_and_utf8_positions() {
        for (kind, inv_type) in [
            (0, 0),
            (0, 15),
            (1, 1),
            (2, 2),
            (3, 3),
            (5, 18),
            (6, 6),
            (7, 7),
            (10, 10),
            (13, 18),
            (20, 19),
            (21, 20),
            (49, 22),
            (56, 25),
            (57, 26),
        ] {
            let item = item(kind, inv_type);
            let mut card = Notecard::default();
            let mut text = "été après".to_owned();
            let marker = card.insert_attachment(&mut text, 3, &item).expect("drop");
            assert_eq!(text, "été\u{100000} après");
            assert!(!card.attachment(marker).expect("new item").saved);
            let saved = Notecard::decode(&card.encode(&text).expect("save")).expect("read");
            let embedded = saved.attachment(marker).expect("item");
            assert!(embedded.saved);
            let actual = &embedded.item;
            assert_eq!(
                (actual.asset_type, actual.inv_type, actual.asset_id),
                (kind, inv_type, item.asset_id)
            );
            assert_eq!(
                (actual.name.as_str(), actual.desc.as_str()),
                (item.name.as_str(), item.desc.as_str())
            );
            assert_eq!(
                (
                    actual.base_mask,
                    actual.owner_mask,
                    actual.next_owner_mask,
                    actual.group_mask,
                    actual.everyone_mask
                ),
                (
                    item.base_mask,
                    item.owner_mask,
                    item.next_owner_mask,
                    item.group_mask,
                    item.everyone_mask
                )
            );
            assert_eq!(
                (actual.creator, actual.owner, actual.last_owner, actual.group_id, actual.group_owned),
                (item.creator, item.owner, item.last_owner, item.group_id, item.group_owned)
            );
            assert_eq!(
                (actual.flags, actual.created_at, actual.sale_type, actual.sale_price),
                (42, 1234, 2, 12)
            );
        }
    }

    #[test]
    fn restricted_unsupported_and_oversized_drops_leave_the_card_unchanged() {
        let mut card = Notecard::default();
        let mut text = "avant".to_owned();
        for bit in [0x8000, 0x4000, 0x2000] {
            let mut item = item(0, 0);
            item.next_owner_mask &= !bit;
            assert!(card.insert_attachment(&mut text, 0, &item).is_err());
        }
        for kind in [8, 24, 25, -1] {
            assert!(card.insert_attachment(&mut text, 0, &item(kind, kind)).is_err());
        }
        assert_eq!(text, "avant");
        assert!(card.items.is_empty());
        let mut text = "a".repeat(MAX_TEXT_BYTES - 3);
        assert!(card.insert_attachment(&mut text, 0, &item(0, 0)).is_err());
        assert!(card.items.is_empty());
    }

    #[test]
    fn clipboard_remaps_colliding_markers_and_keeps_opaque_permissions() {
        let mut a = Notecard::default();
        let mut a_text = String::new();
        a.insert_attachment(&mut a_text, 0, &item(3, 3)).expect("landmark");
        let mut b = Notecard::default();
        let mut b_text = String::new();
        b.insert_attachment(&mut b_text, 0, &item(0, 0)).expect("texture");
        let pasted = b.paste(&a_text, &a).expect("paste");
        assert_eq!(pasted, "\u{100001}");
        b_text.push_str(&pasted);
        let saved = Notecard::decode(&b.encode(&b_text).expect("save")).expect("decode");
        assert_eq!(saved.attachment('\u{100000}').expect("texture").item.asset_type, 0);
        assert_eq!(saved.attachment('\u{100001}').expect("landmark").item.asset_type, 3);
        assert!(b.paste("\u{100002}", &a).is_err());
    }

    #[test]
    fn copy_request_uses_simulator_item_ids_and_exact_capability_fields() {
        let body = copy_body(Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), 42);
        let round_trip = aurora_llsd::from_xml(&aurora_llsd::to_xml(&body)).expect("LLSD");
        assert_eq!(round_trip["notecard-id"].as_uuid(), Uuid::from_u128(1));
        assert_eq!(round_trip["object-id"].as_uuid(), Uuid::from_u128(2));
        assert_eq!(round_trip["item-id"].as_uuid(), Uuid::from_u128(3));
        assert!(round_trip["folder-id"].as_uuid().is_nil());
        assert_eq!(round_trip["callback-id"].as_i32(), 42);
        assert_eq!(round_trip.as_map().expect("map").len(), 5);
        assert!(crate::caps::REQUESTED_CAPS.contains(&"CopyInventoryFromNotecard"));
    }

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

//! "Reopen conversations with residents at login" (FSRestoreOpenIMs): the
//! private conversations open at the end of a session, kept per account
//! like the Legacy Viewer's FSLastOpenIMs (FSFloaterIMContainer::saveOpenIMs
//! / restoreOpenIMs, fsfloaterimcontainer.cpp, originally LGPL 2.1). Only
//! one-to-one conversations are kept; group chats and conferences are not.
//! The list is always written; the setting decides whether it is reopened.

use aurora_llsd::Llsd;
use std::path::Path;
use uuid::Uuid;

/// Per account file, the same LLSD array as FSLastOpenIMs.
pub const FILE: &str = "open_ims.xml";

/// The residents of the conversations to reopen, in their order.
pub fn load(path: &Path) -> Vec<Uuid> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("open conversations not read: {e}");
            }
            return Vec::new();
        }
    };
    match aurora_llsd::from_xml(&bytes) {
        Ok(data) => parse(&data),
        Err(e) => {
            log::warn!("open conversations unreadable: {e:?}");
            Vec::new()
        }
    }
}

/// restoreOpenIMs: entries that are not a map, or without a resident, are
/// skipped.
fn parse(data: &Llsd) -> Vec<Uuid> {
    let mut out: Vec<Uuid> = Vec::new();
    for entry in data.as_array() {
        if !entry.is_map() {
            continue;
        }
        let id = entry.get("other_participant_id").as_uuid();
        if !id.is_nil() && !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

pub fn save(path: &Path, others: &[Uuid]) {
    let list: Vec<Llsd> = others
        .iter()
        .map(|id| {
            let mut entry = aurora_llsd::Map::new();
            entry.insert("other_participant_id".into(), Llsd::from(*id));
            Llsd::Map(entry)
        })
        .collect();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, aurora_llsd::to_xml(&Llsd::Array(list))) {
        log::warn!("open conversations not saved: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_comes_back_in_its_order_without_bad_entries() {
        let dir = std::env::temp_dir().join(format!("aurora-open-ims-{}", Uuid::new_v4()));
        let path = dir.join(FILE);
        assert!(load(&path).is_empty(), "no file yet");
        let (a, b) = (Uuid::from_u128(2), Uuid::from_u128(1));
        save(&path, &[a, b]);
        assert_eq!(load(&path), vec![a, b]);
        // a nil resident or a stray value is skipped
        let mut nil = aurora_llsd::Map::new();
        nil.insert("other_participant_id".into(), Llsd::from(Uuid::nil()));
        let mut good = aurora_llsd::Map::new();
        good.insert("other_participant_id".into(), Llsd::from(a));
        let data = Llsd::Array(vec![Llsd::Map(nil), Llsd::from(3), Llsd::Map(good.clone()), Llsd::Map(good)]);
        assert_eq!(parse(&data), vec![a]);
        let _ = std::fs::remove_dir_all(dir);
    }
}

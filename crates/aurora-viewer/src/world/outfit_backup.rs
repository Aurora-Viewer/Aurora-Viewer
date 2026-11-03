//! Portable outfit recipes, not asset copies. This Aurora addition preserves
//! the links/order used by LLAppearanceMgr::updateCOF / makeNewOutfitLinks
//! (indra/newview/llappearancemgr.cpp); imports use the ordinary outfit mutation.
use super::{appearance, inventory::Inventory};
use aurora_net::outfits::OutfitLink;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ITEMS: usize = 1024;
/// Inventory descriptions hold at most 127 bytes on the server
/// (DB_INV_ITEM_DESC_STR_LEN, indra/llmessage/lldbstrings.h).
const MAX_DESC_BYTES: usize = 127;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Backup {
    format: String,
    version: u32,
    pub name: String,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub item_id: Uuid,
    pub name: String,
    description: String,
    pub asset_type: i32,
    inventory_type: i32,
    wearable_type: Option<u8>,
    attachment_point: Option<u8>,
}

pub struct Resolved {
    pub links: Vec<OutfitLink>,
    pub missing: Vec<Item>,
}

impl Backup {
    pub fn capture(inv: &Inventory, folder: Uuid, worn: &HashMap<Uuid, u8>) -> Result<Self, String> {
        if !appearance::complete(inv, folder) {
            return Err(aurora_i18n::tr!("world-appearance-this-item-has-not-loaded-yet").into());
        }
        let current = Some(folder) == appearance::cof(inv);
        let named = if current { appearance::base(inv).unwrap_or(folder) } else { folder };
        let name = inv.folders.get(&named).map(|f| f.info.name.clone()).unwrap_or_default();
        let mut links = appearance::folder_links(inv, folder);
        if current {
            // Only ordinary inventory-backed attachments; temporary objects
            // cannot be restored by inventory UUID and are deliberately absent.
            for id in worn.keys() {
                if !links.iter().any(|link| link.target == *id) {
                    let item = inv
                        .items
                        .get(id)
                        .ok_or(aurora_i18n::tr!("world-appearance-a-worn-object-is-still-loading"))?;
                    links.push(appearance::item_link(item));
                }
            }
        }
        let mut items = Vec::new();
        for link in links.into_iter().filter(|l| !l.folder) {
            let original = inv.items.get(&link.target);
            // Retain known broken links too, so a subsequent import reports
            // them rather than silently pretending the backup is complete.
            if original.is_some_and(|it| !matches!(it.asset_type, 5 | 6 | 13 | 21)) {
                continue;
            }
            let kind = original.map_or_else(
                || match link.inv_type {
                    18 => 5,
                    21 => 21,
                    _ => 6,
                },
                |it| it.asset_type,
            );
            items.push(Item {
                item_id: link.target,
                name: link.name,
                description: link.desc,
                asset_type: kind,
                inventory_type: link.inv_type,
                wearable_type: original
                    .filter(|it| matches!(it.asset_type, 5 | 13))
                    .map(|it| (it.flags & 0xff) as u8),
                attachment_point: (kind == 6).then(|| {
                    worn.get(&link.target)
                        .filter(|_| current)
                        .copied()
                        .unwrap_or_else(|| original.map_or(0, |it| (it.flags & 0xff) as u8))
                }),
            });
        }
        // UUID order makes the JSON stable; clothing order is explicitly held
        // in each link description, just as it is in the saved outfit.
        items.sort_by_key(|item| item.item_id);
        let backup = Self {
            format: "aurora-outfit".into(),
            version: 1,
            name,
            items,
        };
        backup.validate()?;
        Ok(backup)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let invalid = || aurora_i18n::tr!("outfit-json-invalid").to_owned();
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid());
        }
        let backup: Self = serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes)).map_err(|_| invalid())?;
        backup.validate()?;
        Ok(backup)
    }

    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|_| aurora_i18n::tr!("outfit-json-failed").to_owned())?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_BYTES {
            return Err(aurora_i18n::tr!("outfit-json-invalid").into());
        }
        Ok(bytes)
    }

    fn validate(&self) -> Result<(), String> {
        let mut ids = HashSet::new();
        if self.format != "aurora-outfit"
            || self.version != 1
            || !valid_name(&self.name)
            || self.items.len() > MAX_ITEMS
            || self.items.iter().any(|it| {
                it.item_id.is_nil()
                    || !ids.insert(it.item_id)
                    || it.name.len() > 1024
                    || it.description.len() > 4096
                    || !matches!(it.asset_type, 5 | 6 | 13 | 21)
                    || !matches!(it.inventory_type, 6 | 18 | 20 | 21)
                    || it.wearable_type.is_some_and(|kind| kind > 16)
                    || it.attachment_point.is_some_and(|point| point > 55)
            })
        {
            return Err(aurora_i18n::tr!("outfit-json-invalid").into());
        }
        Ok(())
    }

    pub fn unresolved(&self, inv: &Inventory) -> Vec<Uuid> {
        self.items
            .iter()
            .filter(|it| !inv.items.contains_key(&it.item_id) && !inv.broken(&it.item_id))
            .map(|it| it.item_id)
            .collect()
    }

    pub fn resolve(&self, inv: &Inventory, agent: Uuid) -> Resolved {
        let mut found = Resolved {
            links: Vec::new(),
            missing: Vec::new(),
        };
        for saved in &self.items {
            let item = inv.items.get(&saved.item_id).filter(|it| {
                it.asset_type == saved.asset_type
                    && (it.owner.is_nil() || it.owner == agent)
                    && !super::inventory::actions::library(inv, it.id)
                    && !super::inventory::actions::in_type(inv, it.id, 14)
                    && saved.wearable_type.is_none_or(|kind| it.flags & 0xff == u32::from(kind))
            });
            if let Some(item) = item {
                found.links.push(OutfitLink {
                    target: item.id,
                    name: item.name.clone(),
                    desc: link_description(&saved.description),
                    folder: false,
                    inv_type: item.inv_type,
                });
            } else {
                found.missing.push(saved.clone());
            }
        }
        found
    }

    pub fn points(&self) -> Vec<appearance::WearRequest> {
        self.items
            .iter()
            .filter(|it| it.asset_type == 6)
            .map(|it| appearance::WearRequest {
                item: it.item_id,
                replace: false,
                point: it.attachment_point.unwrap_or(0),
            })
            .collect()
    }
}

/// The JSON may hold a longer, hand-edited description: cut it on a UTF-8
/// character boundary so the link the server stores is never refused.
fn link_description(desc: &str) -> String {
    desc[..desc.floor_char_boundary(MAX_DESC_BYTES)].to_owned()
}

pub fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.trim().chars().count() <= 63 && !name.chars().any(char::is_control)
}

/// Apply the strictest common filename rules on every OS, so a backup
/// proposed on Linux is also a usable Windows/macOS filename.
pub fn file_name(name: &str) -> String {
    let mut stem: String = name
        .chars()
        .map(|ch| if ch.is_control() || "<>:\"/\\|?*".contains(ch) { '_' } else { ch })
        .collect();
    stem = stem.trim().trim_end_matches(['.', ' ']).to_owned();
    // Bound UTF-8 bytes and UTF-16 units, leaving room for the extension.
    while stem.len() > 180 || stem.encode_utf16().count() > 180 {
        stem.pop();
    }
    let device = stem.split('.').next().unwrap_or_default().to_ascii_uppercase();
    if matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$")
        || ["COM", "LPT"].iter().any(|prefix| {
            device
                .strip_prefix(prefix)
                .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"))
        })
    {
        stem.insert(0, '_');
    }
    if stem.is_empty() {
        stem = "outfit".into();
    }
    format!("{stem}.json")
}

#[cfg(test)]
mod tests;

//! Compact presentation of the owner and next-owner masks, shared by the
//! inventory and both object-contents windows. The same C/M/T bits drive
//! Firestorm LLItemBridge::getLabelSuffix and LLTaskInvFVBridge::getDisplayName
//! (indra/newview, originally LGPL 2.1). Aurora also shows granted rights.

use crate::theme::Palette;
use crate::world::inventory::{Inventory, actions};
use aurora_net::build::perm;
use egui::{FontId, Stroke, TextFormat, text::LayoutJob};
use uuid::Uuid;

#[derive(Clone, Copy, Default)]
pub struct Permissions {
    owner: Option<u32>,
    next: Option<u32>,
}

impl Permissions {
    pub fn new(owner: u32, next: u32) -> Self {
        Self {
            owner: Some(owner),
            next: Some(next),
        }
    }

    pub fn inventory(inv: &Inventory, id: Uuid) -> Option<Self> {
        let item = inv.items.get(&id)?;
        // Calling cards and folder links have no useful item permissions.
        if matches!(item.asset_type, 2 | 25) {
            return None;
        }
        Some(
            actions::original(inv, id)
                .and_then(|id| inv.items.get(&id))
                .map_or_else(Self::default, |item| Self::new(item.owner_mask, item.next_owner_mask)),
        )
    }

    #[cfg(test)]
    pub fn indicators(self, p: &Palette) -> LayoutJob {
        let mut job = LayoutJob::default();
        self.append_to(&mut job, p);
        job
    }

    pub fn append_to(self, job: &mut LayoutJob, p: &Palette) {
        let font = FontId::proportional(12.0);
        for (index, (bit, letter)) in [(perm::COPY, "C"), (perm::MODIFY, "M"), (perm::TRANSFER, "T")]
            .into_iter()
            .enumerate()
        {
            if index != 0 {
                job.append("·", 0.0, TextFormat::simple(font.clone(), p.muted_dim));
            }
            let allowed = self.owner.map(|mask| mask & bit != 0);
            let color = match allowed {
                Some(true) => p.success,
                Some(false) => p.danger,
                None => p.muted,
            };
            job.append(
                letter,
                0.0,
                TextFormat {
                    strikethrough: if allowed == Some(false) {
                        Stroke::new(1.0, color)
                    } else {
                        Stroke::NONE
                    },
                    ..TextFormat::simple(font.clone(), color)
                },
            );
            if allowed.is_none() {
                job.append("?", 0.0, TextFormat::simple(font.clone(), p.muted));
            }
        }
    }

    pub fn append_suffix(self, job: &mut LayoutJob, p: &Palette) {
        job.append(" ", 0.0, TextFormat::simple(FontId::proportional(12.0), p.muted));
        self.append_to(job, p);
    }

    pub fn tooltip(self) -> String {
        let (Some(owner), Some(next)) = (self.owner, self.next) else {
            return aurora_i18n::tr!("inventory-permissions-unavailable").into();
        };
        let mut text = String::new();
        for (title, mask) in [
            (aurora_i18n::tr!("inventory-permissions-current-owner"), owner),
            (aurora_i18n::tr!("inventory-permissions-next-owner"), next),
        ] {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(title);
            for (bit, label) in [
                (perm::COPY, aurora_i18n::tr!("inventory-permissions-copy")),
                (perm::MODIFY, aurora_i18n::tr!("inventory-permissions-modify")),
                (perm::TRANSFER, aurora_i18n::tr!("inventory-permissions-transfer")),
            ] {
                text.push('\n');
                text.push_str(&aurora_i18n::tr!(
                    "inventory-permissions-detail",
                    right = label,
                    state = if mask & bit != 0 {
                        aurora_i18n::tr!("inventory-permissions-allowed")
                    } else {
                        aurora_i18n::tr!("inventory-permissions-denied")
                    }
                ));
            }
        }
        text
    }

    /// Same suffix, hover and selection as the user's inventory. Long names
    /// enlarge the scrollable list rather than the containing window.
    pub fn row(self, ui: &mut egui::Ui, p: &Palette, name: &str, selected: bool) -> egui::Response {
        let mut label = super::super::widgets::name_with_suffix(name, "", FontId::proportional(13.0), p.ink);
        self.append_suffix(&mut label, p);
        super::super::widgets::inventory_item_label(ui, label, selected, egui::Sense::click_and_drag(), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::inventory::InvItem;

    #[test]
    fn every_combination_marks_only_denied_owner_rights() {
        let p = crate::theme::Theme::default().palette();
        for rights in 0..8 {
            let mask = ((rights & 1) * perm::COPY) | (((rights >> 1) & 1) * perm::MODIFY) | (((rights >> 2) & 1) * perm::TRANSFER);
            let job = Permissions::new(mask, 0).indicators(&p);
            assert_eq!(job.text, "C·M·T");
            for (index, bit) in [perm::COPY, perm::MODIFY, perm::TRANSFER].into_iter().enumerate() {
                assert_eq!(job.sections[index * 2].format.strikethrough == Stroke::NONE, mask & bit != 0);
                assert_eq!(
                    job.sections[index * 2].format.color,
                    if mask & bit != 0 { p.success } else { p.danger }
                );
            }
        }
    }

    #[test]
    fn links_use_original_masks_and_missing_or_cyclic_links_remain_unknown() {
        let original = Uuid::from_u128(1);
        let link = Uuid::from_u128(2);
        let mut inv = Inventory::default();
        inv.items.insert(
            original,
            InvItem {
                id: original,
                asset_type: 6,
                owner_mask: perm::COPY,
                next_owner_mask: perm::TRANSFER,
                ..Default::default()
            },
        );
        inv.items.insert(
            link,
            InvItem {
                id: link,
                asset_type: 24,
                asset_id: original,
                owner_mask: u32::MAX,
                ..Default::default()
            },
        );
        let rights = Permissions::inventory(&inv, link).expect("item link");
        assert_eq!(rights.owner, Some(perm::COPY));
        assert_eq!(rights.next, Some(perm::TRANSFER));
        inv.items.remove(&original);
        assert!(Permissions::inventory(&inv, link).expect("broken link").owner.is_none());
        inv.items.get_mut(&link).expect("link").asset_id = link;
        let unknown = Permissions::inventory(&inv, link).expect("cyclic link");
        let job = unknown.indicators(&crate::theme::Theme::default().palette());
        assert_eq!(job.text, "C?·M?·T?");
        assert!(job.sections.iter().all(|s| s.format.strikethrough == Stroke::NONE));
    }
}

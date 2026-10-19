//! Compact presentation of the owner and next-owner masks, shared by the
//! inventory and both object-contents windows. The same C/M/T bits drive
//! Legacy LLItemBridge::getLabelSuffix and LLTaskInvFVBridge::getDisplayName
//! (indra/newview, originally LGPL 2.1). Aurora also shows granted rights.
//! Every list draws the letters alike: those of Aurora's inventory rows.

use crate::theme::Palette;
use crate::world::inventory::{Inventory, actions};
use aurora_net::build::perm;
use egui::{Color32, FontFamily, FontId, Stroke, TextFormat, text::LayoutJob};
use uuid::Uuid;

/// The size of the letters, as in Aurora's rights column.
const LETTER_SIZE: f32 = 11.0;
/// The room between two letters of the inline suffix.
const LETTER_GAP: f32 = 5.0;

/// The colour of a letter: allowed, refused or unknown (a broken link).
pub fn letter_ink(p: &Palette, allowed: Option<bool>) -> Color32 {
    match allowed {
        Some(true) => p.muted,
        Some(false) => p.muted_dim.gamma_multiply(0.45),
        None => p.muted_dim,
    }
}

/// Which rights the rows of Aurora's inventory show (its "CMT" menu): yours
/// by default, the next owner's (what a resident you give the item to gets),
/// both, or none. Kept with the inventory preferences.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Rights {
    Hidden,
    #[default]
    Mine,
    Next,
    Both,
}

impl Rights {
    pub fn shows_mine(self) -> bool {
        matches!(self, Self::Mine | Self::Both)
    }

    pub fn shows_next(self) -> bool {
        matches!(self, Self::Next | Self::Both)
    }
}

#[derive(Clone, Copy, Default)]
pub struct Permissions {
    owner: Option<u32>,
    next: Option<u32>,
    show_next: bool,
}

impl Permissions {
    /// The owner's mask, unknown for a broken link.
    pub fn owner(self) -> Option<u32> {
        self.owner
    }

    /// The next owner's mask, unknown for a broken link.
    pub fn next(self) -> Option<u32> {
        self.next
    }

    pub fn new(owner: u32, next: u32) -> Self {
        Self {
            owner: Some(owner),
            next: Some(next),
            show_next: false,
        }
    }

    /// Presentation only: retain the owner mask and optionally append the
    /// original item's next-owner mask, including unknown link data.
    pub fn with_next_owner(mut self, show: bool) -> Self {
        self.show_next = show;
        self
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
        self.append_to(&mut job, p, &FontFamily::Proportional);
        job
    }

    /// The letters of `aurora::rights`, as text: semi-bold, grey when
    /// allowed, dimmed and struck through when refused, the next owner's
    /// after an arrow.
    pub fn append_to(self, job: &mut LayoutJob, p: &Palette, family: &FontFamily) {
        Self::append_mask(job, p, family, self.owner);
        if self.show_next {
            job.append("→", LETTER_GAP, TextFormat::simple(FontId::proportional(LETTER_SIZE), p.muted));
            Self::append_mask(job, p, family, self.next);
        }
    }

    fn append_mask(job: &mut LayoutJob, p: &Palette, family: &FontFamily, mask: Option<u32>) {
        for (bit, letter) in [(perm::COPY, "C"), (perm::MODIFY, "M"), (perm::TRANSFER, "T")] {
            let allowed = mask.map(|mask| mask & bit != 0);
            let color = letter_ink(p, allowed);
            let mut format = crate::ui::fonts::semibold_format(family, LETTER_SIZE, color);
            if allowed == Some(false) {
                format.strikethrough = Stroke::new(1.0, color);
            }
            let gap = if job.is_empty() { 0.0 } else { LETTER_GAP };
            job.append(letter, gap, format);
        }
    }

    pub fn append_suffix(self, ui: &egui::Ui, job: &mut LayoutJob, p: &Palette) {
        let family = crate::ui::fonts::semibold(ui.ctx());
        job.append(" ", 0.0, TextFormat::simple(FontId::proportional(12.0), p.muted));
        self.append_to(job, p, &family);
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
        self.append_suffix(ui, &mut label, p);
        super::super::widgets::inventory_item_label(ui, label, selected, egui::Sense::click_and_drag(), None)
    }
}

/// Shared presentation control; each owning window supplies its own flag.
pub fn next_owner_toggle(ui: &mut egui::Ui, p: &Palette, show: &mut bool) -> bool {
    use crate::ui::widgets;
    ui.push_id("next_owner_permissions", |ui| {
        ui.horizontal(|ui| {
            let tip = if *show {
                aurora_i18n::tr!("inventory-permissions-hide-next")
            } else {
                aurora_i18n::tr!("inventory-permissions-show-next")
            };
            let width = (ui.available_width() - widgets::SWITCH_SIZE.x - ui.spacing().item_spacing.x).max(40.0);
            let clicked = widgets::action_button(
                ui,
                p,
                if *show { "eye-filled" } else { "eye-off" },
                aurora_i18n::tr!("inventory-permissions-next-label"),
                widgets::ActionKind::Flat,
                egui::vec2(width, 26.0),
            )
            .on_hover_text(tip)
            .clicked();
            if clicked {
                *show = !*show;
            }
            widgets::switch(ui, p, show).on_hover_text(tip).changed() || clicked
        })
        .inner
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::inventory::InvItem;

    #[test]
    fn next_owner_suffix_keeps_both_masks_independent_for_every_combination() {
        let p = crate::theme::Theme::default().palette();
        let mask = |bits: u32| ((bits & 1) * perm::COPY) | (((bits >> 1) & 1) * perm::MODIFY) | (((bits >> 2) & 1) * perm::TRANSFER);
        for owner in 0..8 {
            for next in 0..8 {
                let permissions = Permissions::new(mask(owner), mask(next));
                let current = permissions.indicators(&p);
                let both = permissions.with_next_owner(true).indicators(&p);
                assert_eq!(current.text, "CMT");
                assert_eq!(both.text, "CMT→CMT");
                for index in 0..3 {
                    assert_eq!(&both.sections[index].format, &current.sections[index].format);
                    let next_section = &both.sections[4 + index].format;
                    assert_eq!(next_section.color, letter_ink(&p, Some(next & (1 << index) != 0)));
                    assert_eq!(next_section.strikethrough == Stroke::NONE, next & (1 << index) != 0);
                }
                assert_eq!(
                    permissions.with_next_owner(true).with_next_owner(false).indicators(&p).text,
                    current.text
                );
            }
        }
        let unknown = Permissions::default().with_next_owner(true).indicators(&p);
        assert_eq!(unknown.text, "CMT→CMT");
        assert!(unknown.sections.iter().all(|section| section.format.strikethrough == Stroke::NONE));
    }

    #[test]
    fn every_combination_marks_only_denied_owner_rights() {
        let p = crate::theme::Theme::default().palette();
        for rights in 0..8 {
            let mask = ((rights & 1) * perm::COPY) | (((rights >> 1) & 1) * perm::MODIFY) | (((rights >> 2) & 1) * perm::TRANSFER);
            let job = Permissions::new(mask, 0).indicators(&p);
            assert_eq!(job.text, "CMT");
            for (index, bit) in [perm::COPY, perm::MODIFY, perm::TRANSFER].into_iter().enumerate() {
                assert_eq!(job.sections[index].format.strikethrough == Stroke::NONE, mask & bit != 0);
                assert_eq!(job.sections[index].format.color, letter_ink(&p, Some(mask & bit != 0)));
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
        let p = crate::theme::Theme::default().palette();
        let both = rights.with_next_owner(true).indicators(&p);
        assert_eq!(both.text, "CMT→CMT");
        assert_eq!(both.sections[0].format.color, letter_ink(&p, Some(true)));
        assert_eq!(both.sections[4].format.color, letter_ink(&p, Some(false)));
        assert_eq!(both.sections[6].format.color, letter_ink(&p, Some(true)));
        inv.items.remove(&original);
        assert!(Permissions::inventory(&inv, link).expect("broken link").owner.is_none());
        inv.items.get_mut(&link).expect("link").asset_id = link;
        let unknown = Permissions::inventory(&inv, link).expect("cyclic link");
        let p = crate::theme::Theme::default().palette();
        let job = unknown.indicators(&p);
        assert_eq!(job.text, "CMT");
        assert!(job.sections.iter().all(|s| s.format.color == letter_ink(&p, None)));
        assert!(job.sections.iter().all(|s| s.format.strikethrough == Stroke::NONE));
    }
}

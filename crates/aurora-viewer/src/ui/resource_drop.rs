//! LLTextureCtrl::handleDragAndDrop (indra/newview/lltexturectrl.cpp): one
//! inventory texture/snapshot or material on an editable preview. Inventory
//! rows and resource inspectors carry the same item, including its rights.
use crate::{theme::Palette, world::inventory::Inventory};
use aurora_net::inventory::InvItem;
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct Target {
    rect: egui::Rect,
    clip: egui::Rect,
    layer: egui::LayerId,
    radius: egui::CornerRadius,
    pass: u64,
    materials: bool,
    enabled: bool,
}

pub(crate) struct Resource<'a> {
    pub item: &'a InvItem,
    pub asset: Uuid,
}

impl Target {
    /// Keep only the hovered, visible target. Some editors resolve the item
    /// later in this same pass, when their picker has access to inventory.
    pub fn hovered(ui: &egui::Ui, rect: egui::Rect, materials: bool, enabled: bool) -> Option<Self> {
        (ui.rect_contains_pointer(rect) && super::inventory::drag::carried_count(ui.ctx()).is_some()).then(|| Self {
            rect,
            clip: ui.clip_rect(),
            layer: ui.layer_id(),
            radius: ui.visuals().widgets.inactive.corner_radius,
            pass: ui.ctx().cumulative_pass_nr(),
            materials,
            enabled: enabled && ui.is_enabled(),
        })
    }

    pub fn receive<'a>(
        &self,
        ctx: &egui::Context,
        p: &Palette,
        inv: &'a Inventory,
        accepts: impl FnOnce(&InvItem) -> bool,
    ) -> Option<Resource<'a>> {
        if self.pass != ctx.cumulative_pass_nr() {
            return None;
        }
        let ids = super::inventory::drag::carried_ids(ctx)?;
        let resource = match ids.as_slice() {
            [id] => crate::world::inventory::actions::original(inv, *id)
                .and_then(|id| inv.items.get(&id))
                .filter(|item| {
                    if self.materials {
                        item.asset_type == 57
                    } else {
                        matches!(item.asset_type, 0 | 15)
                    }
                })
                .filter(|item| self.materials || !item.asset_id.is_nil())
                .map(|item| Resource {
                    item,
                    asset: if self.materials && item.asset_id.is_nil() {
                        super::texture_library::catalogue::BLANK_MATERIAL
                    } else {
                        item.asset_id
                    },
                }),
            _ => None,
        };
        let allowed = self.enabled && resource.as_ref().is_some_and(|r| accepts(r.item));
        super::inventory::drag::feedback(
            ctx,
            if allowed {
                egui::CursorIcon::Copy
            } else {
                egui::CursorIcon::NotAllowed
            },
        );
        ctx.layer_painter(self.layer).with_clip_rect(self.clip).rect_stroke(
            self.rect,
            self.radius,
            egui::Stroke::new(2.0, if allowed { p.violet_light } else { p.muted_dim }),
            egui::StrokeKind::Inside,
        );
        if !ctx.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
            return None;
        }
        // A refused drop belongs to this widget too: never let it reach the
        // object behind the floater or another overlapping target.
        egui::DragAndDrop::clear_payload(ctx);
        resource.filter(|_| allowed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_drops_resolve_links_and_filter_type_rights_and_disabled_targets() {
        for (kind, materials, enabled, permitted, linked, blank, accepted) in [
            (0, false, true, true, false, false, true),
            (15, false, true, true, true, false, true),
            (57, true, true, true, true, false, true),
            (57, true, true, true, false, true, true),
            (57, false, true, true, false, false, false),
            (0, true, true, true, false, false, false),
            (0, false, false, true, false, false, false),
            (0, false, true, false, false, false, false),
            (0, false, true, true, false, true, false),
        ] {
            let ctx = egui::Context::default();
            let palette = crate::theme::Theme::default().palette();
            let mut inv = Inventory::default();
            let item = Uuid::from_u128(1);
            let asset = if blank { Uuid::nil() } else { Uuid::from_u128(2) };
            inv.items.insert(
                item,
                InvItem {
                    id: item,
                    asset_id: asset,
                    asset_type: kind,
                    ..Default::default()
                },
            );
            let cargo = if linked {
                let id = Uuid::from_u128(3);
                inv.items.insert(
                    id,
                    InvItem {
                        id,
                        asset_id: item,
                        asset_type: 24,
                        ..Default::default()
                    },
                );
                id
            } else {
                item
            };
            let pos = egui::pos2(40.0, 40.0);
            let mut received = None;
            for frame in 0..4 {
                if frame == 2 {
                    egui::DragAndDrop::set_payload(&ctx, super::super::inventory::InvDrag(cargo));
                }
                let mut events = vec![egui::Event::PointerMoved(pos)];
                if frame == 3 {
                    events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    });
                }
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(100.0, 100.0), egui::Sense::click());
                        if let Some(target) = Target::hovered(ui, rect, materials, enabled)
                            && let Some(resource) = target.receive(&ctx, &palette, &inv, |_| permitted)
                        {
                            received = Some((resource.item.id, resource.asset));
                        }
                    },
                );
                output.textures_delta.clear();
                if frame < 3 {
                    assert!(received.is_none(), "hover must never apply the asset");
                }
            }
            let expected = accepted.then_some((
                item,
                if blank {
                    super::super::texture_library::catalogue::BLANK_MATERIAL
                } else {
                    asset
                },
            ));
            assert_eq!(
                received, expected,
                "kind={kind}, materials={materials}, enabled={enabled}, permitted={permitted}, linked={linked}"
            );
            assert!(!egui::DragAndDrop::has_payload_of_type::<super::super::inventory::InvDrag>(&ctx));
        }
    }
}

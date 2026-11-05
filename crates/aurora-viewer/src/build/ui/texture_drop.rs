//! An inventory texture or material dropped on a slot of the Texture tab.
//! Port of LLTextureCtrl::handleDragAndDrop / allowDrop
//! (indra/newview/lltexturectrl.cpp) with the filters and the drag callbacks
//! of FSPanelFace (onDragTexture, onDragPbr, indra/newview/fspanelface.cpp),
//! originally LGPL 2.1. The choice is applied like the picker's, to every
//! selected face.

use super::{PickTarget, Sel};
use crate::build::BuildTool;
use crate::build::contents_drop;
use crate::theme::Palette;
use crate::world::World;
use crate::world::inventory::actions as rules;
use aurora_net::inventory::InvItem;
use std::collections::BTreeSet;

const AT_MATERIAL: i32 = 57;

/// Why a slot refuses the item held over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Refusal {
    /// Not what the slot takes (allow_dnd: a material on the material, a
    /// texture elsewhere), or the slot is disabled.
    Cargo,
    /// TooltipTextureRestrictedDrop: on a worn object, only a copy and
    /// transfer texture (FSPanelFace's immediate filter of attachments).
    Restricted,
    /// An object of the selection does not take the item
    /// (isInventoryDropAcceptable, willObjectAcceptInventory).
    Object,
}

/// Whether `item` may be dropped on `target` for the faces `sel`, in
/// allowDrop's order: the cargo, the immediate permission filter, then the
/// drag callback over the selection's roots. The PBR maps have neither.
pub(super) fn allowed(world: &World, sel: &[Sel], target: PickTarget, item: &InvItem) -> Result<(), Refusal> {
    let cargo = match target {
        PickTarget::PbrMaterial => item.asset_type == AT_MATERIAL,
        _ => matches!(item.asset_type, 0 | 15),
    };
    if !cargo {
        return Err(Refusal::Cargo);
    }
    if matches!(target, PickTarget::PbrMap(_)) {
        return Ok(());
    }
    let roots: BTreeSet<(usize, bool)> = sel.iter().filter_map(|s| contents_drop::linkset(world, s.idx)).collect();
    // attachments are in world and in inventory: the server does not
    // support changing permissions in such a case
    let both = rules::COPY | rules::TRANSFER;
    if target != PickTarget::PbrMaterial
        && roots.iter().any(|&(_, worn)| worn)
        && contents_drop::agent_mask(item, world.agent_id) & both != both
    {
        return Err(Refusal::Restricted);
    }
    if roots.is_empty() || roots.iter().any(|&(root, _)| contents_drop::accepts(world, root, item).is_err()) {
        return Err(Refusal::Object);
    }
    Ok(())
}

/// The drop over one slot: the copy cursor and outline while the item held
/// is accepted, the refusal and its reason otherwise. On release an accepted
/// item becomes the slot's choice, applied by the tab before it draws.
pub(in crate::build::ui) fn slot(
    ui: &egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &World,
    rect: egui::Rect,
    target: PickTarget,
    enabled: bool,
) {
    if let Some(drop) = crate::ui::resource_drop::Target::hovered(ui, rect, target == PickTarget::PbrMaterial, enabled)
        && let Some(resource) = drop.receive(ui.ctx(), p, &world.inventory, |item| {
            let verdict = allowed(world, &super::selected(tool, world), target, item);
            if verdict == Err(Refusal::Restricted) {
                egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), ui.scope_id().with("texture-drop-refused"), rect).show(|ui| {
                    ui.label(aurora_i18n::tr!("build-ui-texture-drop-restricted"));
                });
            }
            verdict.is_ok()
        })
    {
        // The direct drop is the latest choice; a previous picker cannot
        // subsequently restore the value captured before it.
        tool.dismiss_texture_picker();
        tool.ui.slot_drop = Some((target, resource.asset));
    }
}

#[cfg(test)]
#[path = "texture_drop_tests.rs"]
mod tests;

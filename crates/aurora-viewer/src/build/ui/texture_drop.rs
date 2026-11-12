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
use crate::ui::context::flags::OBJECT_MODIFY;
use crate::world::World;
use crate::world::inventory::actions as rules;
use aurora_net::inventory::InvItem;
use std::collections::BTreeSet;

const AT_MATERIAL: i32 = 57;

/// Why a slot refuses the item held over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Refusal {
    /// Not what the slot takes (allow_dnd: a material on the material, a
    /// texture elsewhere), or several entries at once.
    Cargo,
    /// TooltipTextureRestrictedDrop: on a worn object, only a copy and
    /// transfer texture (FSPanelFace's immediate filter of attachments).
    Restricted,
    /// An object of the selection does not take the item
    /// (isInventoryDropAcceptable, willObjectAcceptInventory), or a worn one
    /// cannot be modified.
    Object,
    /// The item would do, but the slot is disabled for this selection.
    Disabled,
}

impl Refusal {
    /// What the tooltip over the slot says: every refusal has a reason.
    fn text(self, target: PickTarget) -> &'static str {
        match self {
            Self::Cargo if target == PickTarget::PbrMaterial => aurora_i18n::tr!("build-ui-texture-drop-single-material"),
            Self::Cargo => aurora_i18n::tr!("build-ui-texture-drop-single-texture"),
            Self::Restricted => aurora_i18n::tr!("build-ui-texture-drop-restricted"),
            Self::Object => aurora_i18n::tr!("build-ui-texture-drop-object"),
            Self::Disabled => aurora_i18n::tr!("build-ui-texture-drop-disabled"),
        }
    }
}

fn image(target: PickTarget) -> bool {
    !matches!(target, PickTarget::PbrMaterial | PickTarget::PbrMap(_))
}

/// Whether `item` may be dropped on `target` for the faces `sel`: the cargo,
/// then the immediate permission filter, then each root of the selection.
/// A worn root refuses an image its immediate filter refuses
/// (`worn_image_filter`, with its reason), then what
/// `contents_drop::worn_refused` refuses, and takes an image with its own
/// modify right; any other case is the
/// Legacy Viewer's drag callback. The PBR maps and the sculpt map take only a
/// copy and transfer item, the filter of their controls
/// (`texture_copy::map_slot_takes`).
pub(super) fn allowed(world: &World, sel: &[Sel], target: PickTarget, item: &InvItem) -> Result<(), Refusal> {
    let cargo = match target {
        PickTarget::PbrMaterial => item.asset_type == AT_MATERIAL,
        _ => matches!(item.asset_type, 0 | 15),
    };
    if !cargo {
        return Err(Refusal::Cargo);
    }
    if matches!(target, PickTarget::PbrMap(_) | PickTarget::Sculpt) && !crate::build::texture_copy::map_slot_takes(item, world.agent_id) {
        return Err(Refusal::Restricted);
    }
    if matches!(target, PickTarget::PbrMap(_)) {
        return Ok(());
    }
    let roots: BTreeSet<(usize, bool)> = sel.iter().filter_map(|s| contents_drop::linkset(world, s.idx)).collect();
    let worn_image = |worn: bool| worn && image(target);
    if roots.iter().any(|&(_, worn)| worn_image(worn)) && !contents_drop::worn_image_filter(item, world.agent_id) {
        return Err(Refusal::Restricted);
    }
    let takes = |&(root, worn): &(usize, bool)| {
        if contents_drop::worn_refused(world, root, item) {
            false
        } else if worn_image(worn) {
            world.objects.get(root).is_some_and(|o| o.update_flags & OBJECT_MODIFY != 0)
        } else {
            contents_drop::accepts(world, root, item).is_ok()
        }
    };
    if roots.is_empty() || !roots.iter().all(takes) {
        return Err(Refusal::Object);
    }
    Ok(())
}

/// The rights an item needs in the picker of `target` for the faces `sel`:
/// LLTextureCtrl's immediate filter, which hides what a worn object refuses,
/// and always copy and transfer for the PBR maps and the sculpt map
/// (FSPanelFace::postBuild, LLPanelObject::postBuild).
pub(in crate::build::ui) fn picker_permissions(world: &World, sel: &[Sel], target: PickTarget) -> u32 {
    if matches!(target, PickTarget::PbrMap(_) | PickTarget::Sculpt) {
        return rules::COPY | rules::TRANSFER;
    }
    let worn = sel
        .iter()
        .filter_map(|s| contents_drop::linkset(world, s.idx))
        .any(|(_, worn)| worn);
    if worn && image(target) { rules::COPY | rules::TRANSFER } else { 0 }
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
    let Some(drop) = crate::ui::resource_drop::Target::hovered(ui, rect, target == PickTarget::PbrMaterial, enabled) else {
        return;
    };
    // Stays when `receive` finds no single item of the slot's kind.
    let mut refusal = Some(Refusal::Cargo);
    let resource = drop.receive(ui.ctx(), p, &world.inventory, |item| {
        let verdict = allowed(world, &super::selected(tool, world), target, item);
        refusal = verdict.err().or((!drop.enabled()).then_some(Refusal::Disabled));
        verdict.is_ok()
    });
    // Nothing editable selected: the tab says so already.
    let told = refusal == Some(Refusal::Object) && super::selected(tool, world).is_empty();
    if let Some(refusal) = refusal.filter(|_| !told) {
        egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), ui.scope_id().with("texture-drop-refused"), rect).show(|ui| {
            ui.label(refusal.text(target));
        });
    }
    if let Some(resource) = resource {
        // The direct drop is the latest choice; a previous picker cannot
        // subsequently restore the value captured before it.
        tool.dismiss_texture_picker();
        tool.ui.slot_drop = Some((target, resource.asset, resource.item.id));
    }
}

#[cfg(test)]
#[path = "texture_drop_tests.rs"]
mod tests;

//! Skin, hair, eyes and system clothing editing. Behaviour follows
//! LLPanelEditWearable / LLViewerWearable (indra/newview): edit a worn
//! original, preserve its complete asset and acknowledge saves before rewearing.
mod bake;
pub mod catalog;
mod masks;
pub mod preview;
#[cfg(test)]
pub(crate) mod tests;
use crate::{shape_editor::Draft, world::World};
use aurora_assets::wearable::Wearable;
use aurora_net::{NetCommand, inventory::InvItem};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

pub fn supported(item: &InvItem) -> bool {
    matches!((item.asset_type, item.flags & 0xff), (13, 1..=3) | (5, 4..=14 | 16))
}

pub fn worn(world: &World, item: Uuid) -> bool {
    crate::world::appearance::cof(&world.inventory).is_some_and(|cof| {
        crate::world::appearance::folder_links(&world.inventory, cof)
            .iter()
            .any(|link| !link.folder && link.target == item)
    })
}

pub fn slots(kind: u8) -> &'static [u32] {
    match kind {
        1 => &[0, 5, 6],
        2 => &[4],
        3 => &[3],
        4 => &[1],
        5 => &[2],
        6 => &[7],
        7 => &[12],
        8 => &[13, 14],
        9 => &[15],
        10 => &[16],
        11 => &[17],
        12 => &[18],
        13 => &[21, 22, 23, 24, 25],
        14 => &[26, 27, 28],
        16 => &[29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39],
        _ => &[],
    }
}

pub fn color_params(kind: u8) -> Option<[i32; 3]> {
    // LLAvatarAppearance::teToColorParams. A jacket, tattoo or universal
    // layer shares one tint across its texture slots.
    Some(match kind {
        4 => [803, 804, 805],
        5 => [806, 807, 808],
        6 => [812, 813, 817],
        7 => [818, 819, 820],
        8 => [834, 835, 836],
        9 => [827, 829, 830],
        10 => [821, 822, 823],
        11 => [824, 825, 826],
        12 => [921, 922, 923],
        14 => [1071, 1072, 1073],
        16 => [1238, 1239, 1240],
        _ => return None,
    })
}

fn texture_allowed(world: &World, draft: &Draft, texture: Uuid) -> bool {
    let kind = draft.edited.kind;
    let required = crate::world::inventory::actions::COPY
        | if draft.item.base_mask & crate::world::inventory::actions::TRANSFER != 0 {
            crate::world::inventory::actions::TRANSFER
        } else {
            0
        };
    !world.local_assets.contains_key(&texture)
        && (texture == crate::world::inventory::wearable::default_texture(kind)
            || texture.is_nil() && matches!(kind, 1 | 13 | 14 | 16)
            || texture == crate::scene::avatar::IMG_INVISIBLE && kind == 13
            || world
                .inventory
                .items
                .values()
                .any(|item| matches!(item.asset_type, 0 | 15) && item.asset_id == texture && item.owner_mask & required == required))
}

/// The texture an inventory entry dragged onto a texture swatch carries: a
/// texture or a snapshot, through its link (LLTextureCtrl::handleDragAndDrop,
/// indra/newview/lltexturectrl.cpp, takes DAD_TEXTURE cargo). Folders and
/// other assets give nothing.
pub fn dragged_texture(world: &World, item: Uuid) -> Option<Uuid> {
    let id = crate::world::inventory::actions::original(&world.inventory, item)?;
    world
        .inventory
        .items
        .get(&id)
        .filter(|item| matches!(item.asset_type, 0 | 15))
        .map(|item| item.asset_id)
}

/// Whether a swatch accepts `texture` now: the same rules as the picker
/// (LLTextureCtrl::allowDrop refuses a disabled control), so the drop
/// highlight never promises what `State::set_texture` then refuses.
pub fn accepts_texture(world: &World, draft: &Draft, slot: u32, texture: Uuid) -> bool {
    draft.pending.is_none()
        && draft.can_modify(world)
        && slots(draft.edited.kind).contains(&slot)
        && (world.local_assets.get(&texture).is_some_and(|(_, material)| !material) || texture_allowed(world, draft, texture))
}

pub fn has_local_textures(world: &World, draft: &Draft) -> bool {
    draft
        .edited
        .textures
        .values()
        .any(|texture| world.local_assets.contains_key(texture))
}

pub fn slot_label(slot: u32) -> &'static str {
    match slot {
        0 | 23 | 26 | 29 => aurora_i18n::tr!("picker-bom-head"),
        1 | 5 | 13 | 15 | 16 | 22 | 27 | 30 => aurora_i18n::tr!("picker-bom-upper"),
        2 | 6 | 7 | 12 | 14 | 17 | 21 | 28 | 31 => aurora_i18n::tr!("picker-bom-lower"),
        3 | 24 | 34 => aurora_i18n::tr!("picker-bom-eyes"),
        4 | 25 | 33 => aurora_i18n::tr!("picker-bom-hair"),
        18 | 32 => aurora_i18n::tr!("picker-bom-skirt"),
        35 => aurora_i18n::tr!("picker-bom-leftarm"),
        36 => aurora_i18n::tr!("picker-bom-leftleg"),
        37 => aurora_i18n::tr!("picker-bom-aux1"),
        38 => aurora_i18n::tr!("picker-bom-aux2"),
        39 => aurora_i18n::tr!("picker-bom-aux3"),
        _ => aurora_i18n::tr!("common-texture"),
    }
}

pub fn slot_bake(slot: u32) -> Option<usize> {
    Some(match slot {
        0 | 23 | 26 | 29 => 8,
        1 | 5 | 13 | 15 | 16 | 22 | 27 | 30 => 9,
        2 | 6 | 7 | 12 | 14 | 17 | 21 | 28 | 31 => 10,
        3 | 24 | 34 => 11,
        18 | 32 => 19,
        4 | 25 | 33 => 20,
        35 => 40,
        36 => 41,
        37 => 42,
        38 => 43,
        39 => 44,
        _ => return None,
    })
}

pub fn affected_bakes(kind: u8) -> &'static [usize] {
    match kind {
        1 | 14 => &[8, 9, 10],
        2 => &[8, 20],
        3 => &[11],
        4 | 9 | 10 => &[9],
        5 | 6 | 7 | 11 => &[10],
        8 => &[9, 10],
        12 => &[19],
        13 => &[8, 9, 10, 11, 20],
        16 => &[8, 9, 10, 11, 19, 20, 40, 41, 42, 43, 44],
        _ => &[],
    }
}

#[derive(Default)]
pub struct State {
    pub open: bool,
    pub draft: Option<Draft>,
    pub loading: Option<(Uuid, Uuid)>,
    pub message: String,
    pub copy_name: Option<String>,
    pub close_confirmation: bool,
    pub close_after_save: bool,
    pub picker: crate::ui::texture_picker::TexturePicker,
    pub picker_slot: Option<u32>,
    picker_before: Option<TextureSnapshot>,
    pub wanted_images: HashSet<Uuid>,
    pub group: String,
    pub camera_pending: bool,
    pub alpha_previous: HashMap<u32, Uuid>,
    local_textures: HashSet<Uuid>,
    pub preview: preview::Preview,
}

struct TextureSnapshot {
    item: Uuid,
    asset: Uuid,
    texture: Option<Uuid>,
    local: bool,
}

impl State {
    pub fn open_item(&mut self, world: &mut World, item: Uuid) -> Option<NetCommand> {
        self.dismiss_picker();
        if self.draft.as_ref().is_some_and(|draft| draft.pending.is_some() || draft.dirty()) {
            self.open = true;
            self.close_confirmation = true;
            return None;
        }
        self.close(world);
        self.open = true;
        self.camera_pending = true;
        let Some(item) = world
            .inventory
            .items
            .get(&item)
            .filter(|item| supported(item) && worn(world, item.id))
            .cloned()
        else {
            self.message = aurora_i18n::tr!("wearable-not-worn").into();
            return None;
        };
        self.message.clear();
        self.loading = Some((item.id, item.asset_id));
        Some(NetCommand::PreviewInventoryItem(item))
    }

    pub fn loaded(&mut self, world: &mut World, item: Uuid, asset: Uuid, result: &Result<Vec<u8>, String>) -> bool {
        self.preview.loaded(item, asset, result);
        if self.loading != Some((item, asset)) {
            return false;
        }
        self.loading = None;
        let Some(item) = world
            .inventory
            .items
            .get(&item)
            .filter(|item| item.asset_id == asset && supported(item) && worn(world, item.id))
            .cloned()
        else {
            self.message = aurora_i18n::tr!("wearable-changed").into();
            return true;
        };
        match result
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|bytes| Wearable::parse(bytes).map_err(|_| aurora_i18n::tr!("wearable-error").to_owned()))
        {
            Ok(mut original) if u32::from(original.kind) == item.flags & 0xff => {
                self.preview.cache(item.id, asset, original.clone());
                for param in catalog::PARAMS.iter().filter(|param| param.kind == original.kind) {
                    original.params.entry(param.id).or_insert(param.default);
                }
                if let Some(channels) = color_params(original.kind) {
                    for channel in channels {
                        original.params.entry(channel).or_insert(1.0);
                    }
                }
                self.group = catalog::PARAMS
                    .iter()
                    .find(|param| param.kind == original.kind)
                    .map_or(String::new(), |param| param.group.into());
                self.draft = Some(Draft {
                    item,
                    edited: original.clone(),
                    original,
                    pending: None,
                });
                self.camera_pending = true;
                self.changed(world);
            }
            Ok(_) => self.message = aurora_i18n::tr!("wearable-error").into(),
            Err(reason) => self.message = reason,
        }
        true
    }

    pub fn changed(&mut self, world: &mut World) {
        self.preview.invalidate();
        if let Some(draft) = &self.draft {
            // Only this wearable's editable controls may affect the avatar.
            // Preserve every other parameter and unknown asset field on disk.
            let values: BTreeMap<_, _> = catalog::PARAMS
                .iter()
                .filter(|param| param.kind == draft.edited.kind)
                .filter_map(|param| draft.edited.params.get(&param.id).map(|value| (param.id, *value)))
                .collect();
            let preview = world.avatar_lib.shape_params.edit_values(world.server_visual_params(), &values);
            world.preview_shape(Some(preview));
        }
    }

    pub fn camera_group(&self) -> usize {
        let kind = self.draft.as_ref().map(|draft| draft.edited.kind);
        match (kind, self.group.as_str()) {
            (Some(1), "skin_bodydetail") => 0,
            (Some(1 | 2), _) => 1,
            (Some(3), _) => 2,
            (Some(4 | 8 | 9 | 10), _) => 7,
            (Some(5 | 6 | 7 | 11 | 12), _) => 8,
            _ => 0,
        }
    }

    pub fn close(&mut self, world: &mut World) {
        self.preview.cancel(world);
        self.open = false;
        self.loading = None;
        self.draft = None;
        self.close_confirmation = false;
        self.close_after_save = false;
        self.copy_name = None;
        self.picker.open = false;
        self.picker_slot = None;
        self.picker_before = None;
        self.wanted_images.clear();
        self.alpha_previous.clear();
        self.local_textures.clear();
        world.preview_shape(None);
    }

    pub fn update(&mut self, world: &mut World) {
        let mut changed = false;
        if let Some(draft) = &mut self.draft {
            // LLLocalBitmap::updateUserLayers replaces removed local maps.
            // Restore this slot's original instead of fetching a private ID.
            draft.edited.textures.retain(|slot, texture| {
                if self.local_textures.contains(texture) && !world.local_assets.contains_key(texture) {
                    changed = true;
                    let Some(original) = draft.original.textures.get(slot) else {
                        return false;
                    };
                    *texture = *original;
                }
                true
            });
            self.alpha_previous.retain(|slot, texture| {
                if self.local_textures.contains(texture) && !world.local_assets.contains_key(texture) {
                    let Some(original) = draft.original.textures.get(slot) else {
                        return false;
                    };
                    *texture = *original;
                }
                true
            });
            self.local_textures.retain(|id| {
                draft
                    .edited
                    .textures
                    .values()
                    .chain(self.alpha_previous.values())
                    .any(|texture| texture == id)
            });
        }
        if changed {
            self.changed(world);
        }
        if self.open
            && self.draft.as_ref().is_some_and(|draft| {
                draft.pending.is_none()
                    && (!worn(world, draft.item.id)
                        || world
                            .inventory
                            .items
                            .get(&draft.item.id)
                            .is_none_or(|item| item.asset_id != draft.item.asset_id))
            })
        {
            self.close(world);
            self.message = aurora_i18n::tr!("wearable-changed").into();
            self.open = true;
        }
    }

    pub fn set_texture(&mut self, world: &World, slot: u32, texture: Uuid) -> bool {
        let Some(draft) = self
            .draft
            .as_mut()
            .filter(|draft| draft.pending.is_none() && draft.can_modify(world))
        else {
            return false;
        };
        if !slots(draft.edited.kind).contains(&slot) {
            return false;
        }
        if !accepts_texture(world, draft, slot, texture) {
            self.message = aurora_i18n::tr!("wearable-texture-permissions").into();
            return false;
        }
        draft.edited.textures.insert(slot, texture);
        if world.local_assets.contains_key(&texture) {
            self.local_textures.insert(texture);
        }
        self.message.clear();
        true
    }

    /// Each choice previews; closing or switching slots keeps it in the
    /// unsaved draft. Only the picker's explicit Cancel restores the slot.
    pub fn open_picker(&mut self, world: &World, slot: u32) {
        self.dismiss_picker();
        let Some(draft) = &self.draft else { return };
        if draft.pending.is_some() || !draft.can_modify(world) || !slots(draft.edited.kind).contains(&slot) {
            return;
        }
        let texture = draft.edited.textures.get(&slot).copied();
        self.picker_before = Some(TextureSnapshot {
            item: draft.item.id,
            asset: draft.item.asset_id,
            texture,
            local: texture.is_some_and(|id| world.local_assets.contains_key(&id)),
        });
        self.picker_slot = Some(slot);
        self.picker.open_with_defaults(
            aurora_i18n::tr!("wearable-choose-texture"),
            texture.unwrap_or_default(),
            None,
            matches!(draft.edited.kind, 1 | 13 | 14 | 16),
        );
        self.picker.allow_local = true;
    }

    pub fn dismiss_picker(&mut self) {
        self.picker.open = false;
        self.picker_slot = None;
        self.picker_before = None;
    }

    pub fn preview_picker(&mut self, world: &World, texture: Uuid) -> bool {
        self.picker_slot.is_some_and(|slot| self.set_texture(world, slot, texture))
    }

    pub fn finish_picker(&mut self, world: &World, accepted: Option<Uuid>) -> bool {
        self.picker.open = false;
        let before = self.picker_before.take();
        let Some(slot) = self.picker_slot.take() else { return false };
        if let Some(texture) = accepted {
            let previous = self.draft.as_ref().and_then(|draft| draft.edited.textures.get(&slot)).copied();
            if self.set_texture(world, slot, texture) {
                return previous != Some(texture);
            }
        } else {
            self.message.clear();
        }
        let Some(before) = before else { return false };
        let Some(draft) = self
            .draft
            .as_mut()
            .filter(|draft| draft.item.id == before.item && draft.item.asset_id == before.asset)
        else {
            return false;
        };
        let texture = if before.local && before.texture.is_some_and(|id| !world.local_assets.contains_key(&id)) {
            draft.original.textures.get(&slot).copied()
        } else {
            before.texture
        };
        let changed = draft.edited.textures.get(&slot).copied() != texture;
        if let Some(texture) = texture {
            draft.edited.textures.insert(slot, texture);
            if world.local_assets.contains_key(&texture) {
                self.local_textures.insert(texture);
            }
        } else {
            draft.edited.textures.remove(&slot);
        }
        changed
    }

    pub fn save(&mut self, world: &World, name: Option<String>) -> Option<NetCommand> {
        if self.picker_slot.is_some() {
            return None;
        }
        let draft = self.draft.as_mut()?;
        let valid = if name.is_some() {
            draft.can_copy(world) && (draft.can_modify(world) || !draft.dirty())
        } else {
            draft.can_modify(world)
        };
        if draft.pending.is_some() {
            return None;
        }
        if has_local_textures(world, draft) || draft.edited.textures.values().any(|texture| self.local_textures.contains(texture)) {
            self.message = aurora_i18n::tr!("wearable-local-preview").into();
            return None;
        }
        let current = world.inventory.items.get(&draft.item.id)?;
        if !valid || current.asset_id != draft.item.asset_id || !worn(world, current.id) {
            self.message = aurora_i18n::tr!("wearable-save-denied").into();
            return None;
        }
        if draft
            .edited
            .textures
            .iter()
            .any(|(slot, texture)| draft.original.textures.get(slot) != Some(texture) && !texture_allowed(world, draft, *texture))
        {
            self.message = aurora_i18n::tr!("wearable-texture-permissions").into();
            return None;
        }
        let data = match draft.edited.encode(name.as_deref().unwrap_or(&current.name)) {
            Ok(data) => data,
            Err(_) => {
                self.message = aurora_i18n::tr!("wearable-error").into();
                return None;
            }
        };
        let request = Uuid::new_v4();
        draft.pending = Some(request);
        self.message.clear();
        Some(NetCommand::SaveWearable {
            request,
            item: Box::new(current.clone()),
            data,
            copy_name: name,
        })
    }

    pub fn saved(&mut self, world: &mut World, request: Uuid, result: Result<InvItem, String>) -> Option<(Uuid, Uuid)> {
        let draft = self.draft.as_mut().filter(|draft| draft.pending == Some(request))?;
        draft.pending = None;
        match result {
            Ok(item) => {
                if item.asset_type != draft.item.asset_type
                    || item.flags & 0xff != u32::from(draft.edited.kind)
                    || item.owner != draft.item.owner
                {
                    self.message = aurora_i18n::tr!("wearable-changed").into();
                    return None;
                }
                let old = draft.item.id;
                let new = item.id;
                draft.item = item.clone();
                draft.original = draft.edited.clone();
                world.inventory.add_items(vec![item]);
                self.copy_name = None;
                self.message = aurora_i18n::tr!("wearable-saved").into();
                if old != new || self.close_after_save {
                    self.close(world);
                }
                Some((old, new))
            }
            Err(reason) => {
                self.message = reason;
                self.close_after_save = false;
                None
            }
        }
    }
}

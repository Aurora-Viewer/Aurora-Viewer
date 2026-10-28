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
    draft.pending.is_none() && draft.can_modify(world) && slots(draft.edited.kind).contains(&slot) && texture_allowed(world, draft, texture)
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
    pub wanted_images: HashSet<Uuid>,
    pub group: String,
    pub camera_pending: bool,
    pub alpha_previous: HashMap<u32, Uuid>,
    pub preview: preview::Preview,
}

impl State {
    pub fn open_item(&mut self, world: &mut World, item: Uuid) -> Option<NetCommand> {
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
        self.wanted_images.clear();
        self.alpha_previous.clear();
        world.preview_shape(None);
    }

    pub fn update(&mut self, world: &mut World) {
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
        if !texture_allowed(world, draft, texture) {
            self.message = aurora_i18n::tr!("wearable-texture-permissions").into();
            return false;
        }
        draft.edited.textures.insert(slot, texture);
        self.message.clear();
        true
    }

    pub fn save(&mut self, world: &World, name: Option<String>) -> Option<NetCommand> {
        let draft = self.draft.as_mut()?;
        let valid = if name.is_some() {
            draft.can_copy(world) && (draft.can_modify(world) || !draft.dirty())
        } else {
            draft.can_modify(world)
        };
        if draft.pending.is_some() {
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

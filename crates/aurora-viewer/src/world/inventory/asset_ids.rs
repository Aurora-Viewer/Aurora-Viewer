//! One asset id display rule for the Dev Toolbox and the explorers, in bulk:
//! the rule of `actions::asset_uuid_shown` (an inventory item holding the
//! asset with copy, modify and transfer, LLViewerInventoryItem::
//! getProtectedAssetUUID, indra/newview/llviewerinventory.cpp, and the UUID
//! field of LLFloaterTexturePicker, indra/newview/lltexturectrl.cpp), plus
//! the built-in assets, the local textures and demo mode. Any other id is
//! written `hidden-` and a short tag: the same asset keeps its tag for the
//! session, so a report still tells two textures apart, and the tag cannot
//! be turned back into the id.

use super::Inventory;
use super::actions::FULL;
use std::collections::HashSet;
use std::hash::{BuildHasher, RandomState};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use uuid::Uuid;

/// How a hidden id starts in a text (`dev_toolbox::legend` reads it back).
pub const HIDDEN: &str = "hidden-";
/// Hex digits of the tag after [`HIDDEN`].
pub const TAG_LEN: usize = 8;

/// The full-permission assets of the inventory, built again when the
/// inventory changes.
#[derive(Default)]
pub(crate) struct Cache {
    snapshot: Mutex<Option<((u64, usize), Arc<HashSet<Uuid>>)>>,
}

impl Inventory {
    /// The assets an item of the inventory holds with full permissions:
    /// one scan per inventory change, then a lookup per id.
    pub fn full_permission_assets(&self) -> Arc<HashSet<Uuid>> {
        let key = (self.generation, self.items.len());
        let mut cached = self.full_assets.snapshot.lock().unwrap_or_else(PoisonError::into_inner);
        match cached.as_ref() {
            Some((k, set)) if *k == key => set.clone(),
            _ => {
                let set: Arc<HashSet<Uuid>> = Arc::new(
                    self.items
                        .values()
                        .filter(|it| it.owner_mask & FULL == FULL && !it.asset_id.is_nil())
                        .map(|it| it.asset_id)
                        .collect(),
                );
                *cached = Some((key, set.clone()));
                set
            }
        }
    }
}

/// Ids every viewer knows: the null id, the built-in textures and blank
/// material, the glTF override's "removed" marker, the bake channels, the
/// collision sounds and the built-in animations (gUserAnimStates).
pub fn built_in(id: &Uuid) -> bool {
    use aurora_assets::material::OVERRIDE_NULL_UUID;
    use aurora_prim::te::{BLANK_TEXTURE, DEFAULT_TEXTURE, TRANSPARENT_TEXTURE};
    id.is_nil()
        || [
            BLANK_TEXTURE,
            DEFAULT_TEXTURE,
            TRANSPARENT_TEXTURE,
            crate::scene::meshes::BLANK_MATERIAL,
            OVERRIDE_NULL_UUID,
        ]
        .contains(id)
        || crate::scene::avatar::bake_slot_for(id).is_some()
        || crate::scene::sounds::is_collision(id)
        || crate::gestures::builtins::ANIMATIONS.iter().any(|(_, a)| a == id)
}

/// Demo mode (`AURORA_DEMO`, offline: no grid behind it).
fn demo_mode() -> bool {
    static DEMO: OnceLock<bool> = OnceLock::new();
    *DEMO.get_or_init(|| std::env::var_os("AURORA_DEMO").is_some())
}

/// The tag of a hidden id: a keyed hash whose key is drawn at each launch.
fn tag(id: &Uuid) -> u32 {
    static KEY: OnceLock<RandomState> = OnceLock::new();
    KEY.get_or_init(RandomState::new).hash_one(id) as u32
}

/// An asset id as a text writes it: in full, or hidden.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AssetId {
    Shown(Uuid),
    Hidden(u32),
}

impl std::fmt::Display for AssetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetId::Shown(id) => write!(f, "{id}"),
            AssetId::Hidden(tag) => write!(f, "{HIDDEN}{tag:08x}"),
        }
    }
}

/// Like `Display`, so that `{:?}` of a list or an `Option` of them reads as
/// the ids would.
impl std::fmt::Debug for AssetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

/// Which asset ids may be written out, taken once for a text or a frame.
#[derive(Clone)]
pub struct AssetIds {
    full: Arc<HashSet<Uuid>>,
    local: HashSet<Uuid>,
    every: bool,
    /// RLVa @viewtexture (RLV_BHVR_VIEWTEXTURE): built-in ids only.
    restricted: bool,
}

impl AssetIds {
    pub fn new(inv: &Inventory, local: impl IntoIterator<Item = Uuid>, demo: bool) -> Self {
        Self {
            full: inv.full_permission_assets(),
            local: local.into_iter().collect(),
            every: demo,
            restricted: false,
        }
    }

    /// The rule for the windows: the inventory, the local textures and demo
    /// mode of `world`.
    pub fn of(world: &crate::world::World) -> Self {
        Self::new(&world.inventory, world.local_assets.keys().copied(), demo_mode())
    }

    /// The rule for the Dev Toolbox's texts, which are mostly textures: as
    /// [`Self::of`], and built-in ids only while RLVa @viewtexture holds.
    pub fn diagnostics(world: &crate::world::World) -> Self {
        Self {
            restricted: !world.inventory.can_preview(0),
            ..Self::of(world)
        }
    }

    /// Whether `id` may be written out, copied or offered.
    pub fn shown(&self, id: &Uuid) -> bool {
        built_in(id) || self.every || (!self.restricted && (self.full.contains(id) || self.local.contains(id)))
    }

    /// `id` as a text writes it.
    pub fn id(&self, id: Uuid) -> AssetId {
        if self.shown(&id) {
            AssetId::Shown(id)
        } else {
            AssetId::Hidden(tag(&id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::actions::{COPY, MODIFY, TRANSFER};
    use aurora_net::inventory::InvItem;

    fn item(inv: &mut Inventory, n: u128, asset: Uuid, mask: u32) {
        inv.items.insert(
            Uuid::from_u128(n),
            InvItem {
                id: Uuid::from_u128(n),
                asset_id: asset,
                owner_mask: mask,
                ..Default::default()
            },
        );
        inv.generation += 1;
    }

    #[test]
    fn an_asset_id_is_written_for_full_permissions_local_built_in_or_demo() {
        let mut inv = Inventory::default();
        let (asset, local) = (Uuid::from_u128(0x51), Uuid::from_u128(0x52));
        let ids = AssetIds::new(&inv, [local], false);
        assert!(!ids.shown(&asset), "not in the inventory");
        assert!(ids.shown(&local), "a local texture");
        assert!(ids.shown(&aurora_prim::te::BLANK_TEXTURE) && ids.shown(&Uuid::nil()));
        assert!(ids.shown(&crate::scene::avatar::BAKES[0].2), "a bake channel");
        assert!(AssetIds::new(&inv, [], true).shown(&asset), "demo mode");
        for missing in [COPY, MODIFY, TRANSFER] {
            item(&mut inv, 1, asset, FULL & !missing);
            assert!(!AssetIds::new(&inv, [], false).shown(&asset), "missing {missing:x}");
        }
        item(&mut inv, 2, asset, FULL);
        assert!(AssetIds::new(&inv, [], false).shown(&asset), "a second item with full permissions");
        // the same answer as the inventory windows' rule
        assert!(crate::world::inventory::actions::asset_uuid_shown(&inv, asset));
    }

    #[test]
    fn the_full_permission_assets_follow_the_inventory() {
        let mut inv = Inventory::default();
        let asset = Uuid::from_u128(0x61);
        item(&mut inv, 1, asset, FULL);
        let before = inv.full_permission_assets();
        assert!(Arc::ptr_eq(&before, &inv.full_permission_assets()), "built once per change");
        inv.items.remove(&Uuid::from_u128(1));
        inv.generation += 1;
        assert!(!inv.full_permission_assets().contains(&asset), "gone with its item");
    }

    #[test]
    fn a_hidden_id_keeps_its_tag_and_leaves_nothing_of_the_id() {
        let inv = Inventory::default();
        let ids = AssetIds::new(&inv, [], false);
        let (a, b) = (Uuid::from_u128(0x2bd25806_c894_2b00_807e_495a276ddcd5), Uuid::from_u128(0x71));
        let text = ids.id(a).to_string();
        assert!(text.starts_with(HIDDEN) && text.len() == HIDDEN.len() + TAG_LEN, "{text}");
        assert_eq!(text, ids.id(a).to_string(), "the same tag for the same asset");
        assert_ne!(text, ids.id(b).to_string());
        let hex = a.simple().to_string();
        assert!(!hex.contains(&text[HIDDEN.len()..]), "{text}");
        assert_eq!(format!("{:?}", [ids.id(a)]), format!("[{text}]"));
        assert_eq!(ids.id(Uuid::nil()).to_string(), Uuid::nil().to_string());
    }

    #[test]
    fn the_texture_view_restriction_leaves_built_in_ids_only() {
        let mut inv = Inventory::default();
        let asset = Uuid::from_u128(0x81);
        item(&mut inv, 1, asset, FULL);
        let ids = AssetIds {
            restricted: true,
            ..AssetIds::new(&inv, [Uuid::from_u128(0x82)], false)
        };
        assert!(!ids.shown(&asset) && !ids.shown(&Uuid::from_u128(0x82)));
        assert!(ids.shown(&aurora_prim::te::TRANSPARENT_TEXTURE));
    }
}

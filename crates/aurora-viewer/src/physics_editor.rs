//! Editing the worn physics layer (wearable type 15) and "New physics".
//! LLPanelEditWearable's physics panel (indra/newview/llpaneleditwearable.cpp,
//! skins/default/xui/en/panel_edit_physics.xml) and
//! LLAgentWearables::createWearable / wear_and_edit_cb
//! (indra/newview/llagentwearables.cpp), originally LGPL 2.1.
//!
//! The draft's weights are previewed as our avatar's visual parameters
//! (`World::preview_shape`), from which the scene builds the avatar's body
//! physics (`scene/body_physics.rs`): a slider changes the bounce at once,
//! and nothing reaches the simulator until the layer is saved.

use crate::{shape_editor::Draft, world::World};
use aurora_assets::wearable::Wearable;
use aurora_net::{NetCommand, inventory::InvItem};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Wearable type of a physics layer (LLWearableType::WT_PHYSICS).
pub const KIND: u8 = 15;
/// Clothing folder, where "New physics" is created
/// (LLFolderType::assetTypeToFolderType(AT_CLOTHING)).
const FT_CLOTHING: i32 = 5;
/// Sex parameter of the shape (above 0.5: male).
const PARAM_MALE: i32 = 80;

/// One slider: a tweakable physics parameter of avatar_lad.xml.
#[derive(Debug, PartialEq)]
pub struct Param {
    pub id: i32,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// `sex="female"`: hidden on a male avatar (getSortedParams).
    pub female: bool,
}

const fn param(id: i32, min: f32, max: f32, default: f32, female: bool) -> Param {
    Param {
        id,
        min,
        max,
        default,
        female,
    }
}

/// The physics parameters, in avatar_lad.xml's order (their display order).
pub const PARAMS: [Param; 33] = [
    param(10000, 0.1, 1.0, 0.1, true),
    param(10001, 0.0, 30.0, 0.0, true),
    param(10002, 0.0, 10.0, 1.0, true),
    param(10003, 0.0, 3.0, 0.0, true),
    param(10004, 0.0, 100.0, 10.0, true),
    param(10005, 1.0, 100.0, 10.0, true),
    param(10006, 0.0, 1.0, 0.2, true),
    param(10007, 0.0, 3.0, 0.0, true),
    param(10008, 0.0, 100.0, 10.0, true),
    param(10009, 1.0, 100.0, 10.0, true),
    param(10010, 0.0, 1.0, 0.2, true),
    param(10011, 0.1, 1.0, 0.1, false),
    param(10012, 0.0, 30.0, 0.0, false),
    param(10013, 0.0, 10.0, 1.0, false),
    param(10014, 0.0, 3.0, 0.0, false),
    param(10015, 0.0, 100.0, 10.0, false),
    param(10016, 1.0, 100.0, 10.0, false),
    param(10017, 0.0, 1.0, 0.2, false),
    param(10018, 0.1, 1.0, 0.1, false),
    param(10019, 0.0, 30.0, 0.0, false),
    param(10020, 0.0, 10.0, 1.0, false),
    param(10021, 0.0, 3.0, 0.0, false),
    param(10022, 0.0, 100.0, 10.0, false),
    param(10023, 1.0, 100.0, 10.0, false),
    param(10024, 0.0, 1.0, 0.2, false),
    param(10025, 0.0, 3.0, 0.0, false),
    param(10026, 0.0, 100.0, 10.0, false),
    param(10027, 1.0, 100.0, 10.0, false),
    param(10028, 0.0, 1.0, 0.2, false),
    param(10029, 0.0, 3.0, 0.0, true),
    param(10030, 0.0, 100.0, 10.0, true),
    param(10031, 1.0, 100.0, 10.0, true),
    param(10032, 0.0, 1.0, 0.2, true),
];

impl Param {
    pub fn label(&self) -> &'static str {
        match self.id {
            10000 => aurora_i18n::tr!("physics-breast-mass"),
            10001 => aurora_i18n::tr!("physics-breast-gravity"),
            10002 => aurora_i18n::tr!("physics-breast-drag"),
            10011 => aurora_i18n::tr!("physics-belly-mass"),
            10012 => aurora_i18n::tr!("physics-belly-gravity"),
            10013 => aurora_i18n::tr!("physics-belly-drag"),
            10018 => aurora_i18n::tr!("physics-butt-mass"),
            10019 => aurora_i18n::tr!("physics-butt-gravity"),
            10020 => aurora_i18n::tr!("physics-butt-drag"),
            // each motion: Max Effect, Spring, Gain, Damping
            _ => match GROUPS[..6].iter().find_map(|g| g.params.iter().position(|id| *id == self.id)) {
                Some(0) => aurora_i18n::tr!("physics-max-effect"),
                Some(1) => aurora_i18n::tr!("physics-spring"),
                Some(2) => aurora_i18n::tr!("physics-gain"),
                _ => aurora_i18n::tr!("physics-damping"),
            },
        }
    }
}

pub fn param_by_id(id: i32) -> Option<&'static Param> {
    PARAMS.iter().find(|p| p.id == id)
}

/// One accordion tab of panel_edit_physics.xml (a SUBPART_PHYSICS_* entry):
/// the parameters of its edit_group, in display order.
pub struct Group {
    /// SEX_FEMALE subpart: the tab is hidden on a male avatar.
    pub female: bool,
    pub params: &'static [i32],
}

pub const GROUPS: [Group; 7] = [
    // physics_breasts_updown
    Group {
        female: true,
        params: &[10003, 10004, 10005, 10006],
    },
    // physics_breasts_inout
    Group {
        female: true,
        params: &[10007, 10008, 10009, 10010],
    },
    // physics_breasts_leftright
    Group {
        female: true,
        params: &[10029, 10030, 10031, 10032],
    },
    // physics_belly_updown
    Group {
        female: false,
        params: &[10014, 10015, 10016, 10017],
    },
    // physics_butt_updown
    Group {
        female: false,
        params: &[10021, 10022, 10023, 10024],
    },
    // physics_butt_leftright
    Group {
        female: false,
        params: &[10025, 10026, 10027, 10028],
    },
    // physics_advanced
    Group {
        female: false,
        params: &[10000, 10001, 10002, 10011, 10012, 10013, 10018, 10019, 10020],
    },
];

pub fn group_label(index: usize) -> &'static str {
    match index {
        0 => aurora_i18n::tr!("physics-group-breasts-bounce"),
        1 => aurora_i18n::tr!("physics-group-breasts-cleavage"),
        2 => aurora_i18n::tr!("physics-group-breasts-sway"),
        3 => aurora_i18n::tr!("physics-group-belly-bounce"),
        4 => aurora_i18n::tr!("physics-group-butt-bounce"),
        5 => aurora_i18n::tr!("physics-group-butt-sway"),
        _ => aurora_i18n::tr!("physics-group-advanced"),
    }
}

/// The tabs shown, by index into [`GROUPS`]: the breasts' only on a female
/// avatar (LLPanelEditWearable::showWearable).
pub fn visible_groups(male: bool) -> Vec<usize> {
    (0..GROUPS.len()).filter(|k| !(male && GROUPS[*k].female)).collect()
}

/// The sliders of a tab (getSortedParams: the avatar's sex).
pub fn visible_params(group: usize, male: bool) -> Vec<&'static Param> {
    GROUPS
        .get(group)
        .into_iter()
        .flat_map(|g| g.params)
        .filter_map(|id| param_by_id(*id))
        .filter(|p| !(male && p.female))
        .collect()
}

pub fn is_physics(it: &InvItem) -> bool {
    it.asset_type == 5 && it.flags & 0xff == u32::from(KIND)
}

/// The physics layer of the current outfit (one at most: not multiwear).
pub fn worn_physics(world: &World) -> Option<Uuid> {
    crate::world::appearance::cof(&world.inventory)
        .into_iter()
        .flat_map(|cof| crate::world::appearance::folder_links(&world.inventory, cof))
        .filter(|l| !l.folder)
        .map(|l| l.target)
        .find(|id| world.inventory.items.get(id).is_some_and(is_physics))
}

/// Our avatar is male, by its shape as currently shown.
pub fn male(world: &World) -> bool {
    let values = world.visual_params(&world.agent_id).unwrap_or(&[]);
    world
        .avatar_lib
        .shape_params
        .weights(values)
        .get(&PARAM_MALE)
        .is_some_and(|w| *w > 0.5)
}

pub fn value(draft: &Draft, param: &Param) -> f32 {
    draft
        .edited
        .params
        .get(&param.id)
        .copied()
        .unwrap_or(param.default)
        .clamp(param.min, param.max)
}

pub fn set(draft: &mut Draft, param: &Param, value: f32) {
    if value.is_finite() {
        draft.edited.params.insert(param.id, value.clamp(param.min, param.max));
    }
}

/// The physics weights of a layer: its catalogued parameters only, so a
/// stray id in an asset never previews another wearable's parameter.
fn physics_weights(wearable: &Wearable) -> BTreeMap<i32, f32> {
    wearable
        .params
        .iter()
        .filter(|(id, _)| param_by_id(**id).is_some())
        .map(|(id, w)| (*id, *w))
        .collect()
}

/// "New physics" waiting for the server: created in `parent` under `name`,
/// then worn and edited (wear_and_edit_cb).
#[derive(Debug, Clone, PartialEq)]
struct Creating {
    parent: Uuid,
    name: String,
}

#[derive(Default)]
pub struct State {
    pub open: bool,
    /// The open tab (single expansion accordion), by index into [`GROUPS`].
    pub group: Option<usize>,
    pub draft: Option<Draft>,
    pub loading: Option<(Uuid, Uuid)>,
    pub message: String,
    pub copy_name: Option<String>,
    pub close_confirmation: bool,
    pub close_after_save: bool,
    creating: Option<Creating>,
    /// A new layer to edit once the outfit wears it
    /// (requestEditingWearable).
    edit_when_worn: Option<Uuid>,
    /// AURORA_DEMO_PHYSICS_EDIT=drag: when the sliders start moving.
    pub(crate) demo_drag: Option<std::time::Instant>,
}

impl State {
    pub fn open_item(&mut self, world: &mut World, item: Uuid) -> Option<NetCommand> {
        if self.draft.as_ref().is_some_and(|d| d.pending.is_some() || d.dirty()) {
            self.open = true;
            return None;
        }
        self.open = true;
        self.message.clear();
        self.draft = None;
        self.copy_name = None;
        self.close_confirmation = false;
        self.close_after_save = false;
        self.edit_when_worn = None;
        if self.group.is_none() {
            self.group = visible_groups(male(world)).first().copied();
        }
        world.preview_shape(None);
        let Some(it) = world
            .inventory
            .items
            .get(&item)
            .filter(|it| Some(it.id) == worn_physics(world))
            .cloned()
        else {
            self.message = aurora_i18n::tr!("physics-no-physics").into();
            return None;
        };
        self.loading = Some((it.id, it.asset_id));
        Some(NetCommand::PreviewInventoryItem(it))
    }

    pub fn loaded(&mut self, world: &mut World, item: Uuid, asset: Uuid, result: &Result<Vec<u8>, String>) -> bool {
        if self.loading != Some((item, asset)) {
            return false;
        }
        self.loading = None;
        let Some(info) = world
            .inventory
            .items
            .get(&item)
            .filter(|i| i.asset_id == asset && Some(item) == worn_physics(world))
            .cloned()
        else {
            self.message = aurora_i18n::tr!("physics-changed").into();
            return true;
        };
        match result
            .as_ref()
            .map_err(|e| e.clone())
            .and_then(|bytes| Wearable::parse(bytes).map_err(|_| aurora_i18n::tr!("physics-error").into()))
        {
            Ok(mut original) if original.kind == KIND => {
                // a parameter the asset lacks has its default
                // (LLViewerWearable::setParamsToDefaults)
                for param in &PARAMS {
                    original.params.entry(param.id).or_insert(param.default);
                }
                self.draft = Some(Draft {
                    item: info,
                    edited: original.clone(),
                    original,
                    pending: None,
                });
                self.changed(world);
            }
            Ok(_) => self.message = aurora_i18n::tr!("physics-error").into(),
            Err(reason) => self.message = reason,
        }
        true
    }

    /// Show the draft on our avatar: its body physics follow at once.
    pub fn changed(&mut self, world: &mut World) {
        if let Some(draft) = &self.draft {
            let values = world
                .avatar_lib
                .shape_params
                .edit_values(world.server_visual_params(), &physics_weights(&draft.edited));
            world.preview_shape(Some(values));
        }
    }

    pub fn revert(&mut self, world: &mut World) {
        if let Some(draft) = &mut self.draft {
            draft.edited = draft.original.clone();
        }
        self.changed(world);
    }

    pub fn close(&mut self, world: &mut World) {
        self.open = false;
        self.loading = None;
        self.draft = None;
        self.close_confirmation = false;
        self.copy_name = None;
        self.close_after_save = false;
        self.demo_drag = None;
        world.preview_shape(None);
    }

    pub fn save(&mut self, world: &World, copy_name: Option<String>) -> Option<NetCommand> {
        let draft = self.draft.as_mut()?;
        if draft.pending.is_some() {
            return None;
        }
        let valid = if copy_name.is_some() {
            draft.can_copy(world) && (draft.can_modify(world) || !draft.dirty())
        } else {
            draft.can_modify(world)
        };
        let current = world.inventory.items.get(&draft.item.id)?;
        if !valid || current.asset_id != draft.item.asset_id || Some(current.id) != worn_physics(world) {
            self.message = aurora_i18n::tr!("physics-save-denied").into();
            return None;
        }
        let name = copy_name.as_deref().unwrap_or(&current.name);
        let Ok(data) = draft.edited.encode(name) else {
            self.message = aurora_i18n::tr!("physics-error").into();
            return None;
        };
        let request = Uuid::new_v4();
        draft.pending = Some(request);
        self.message.clear();
        Some(NetCommand::SaveWearable {
            request,
            item: Box::new(current.clone()),
            data,
            copy_name,
        })
    }

    /// The save of `request` is done: the saved (or copied) item, to wear
    /// again so the server bakes the new layer into the appearance.
    pub fn saved(&mut self, world: &mut World, request: Uuid, result: Result<InvItem, String>) -> Option<Uuid> {
        let draft = self.draft.as_mut().filter(|d| d.pending == Some(request))?;
        draft.pending = None;
        match result {
            Ok(item) => {
                let copied = item.id != draft.item.id;
                let id = item.id;
                draft.item = item.clone();
                draft.original = draft.edited.clone();
                world.inventory.add_items(vec![item]);
                self.message = aurora_i18n::tr!("physics-saved").into();
                self.copy_name = None;
                if copied || self.close_after_save {
                    self.close(world);
                }
                Some(id)
            }
            Err(reason) => {
                self.message = reason;
                self.close_after_save = false;
                None
            }
        }
    }

    /// The editor follows the outfit: it closes when its layer is no longer
    /// the worn one, and a new layer opens once it is worn. Returns the
    /// item to open.
    pub fn update(&mut self, world: &mut World) -> Option<Uuid> {
        if self.open
            && let Some(draft) = &self.draft
            && draft.pending.is_none()
            && (Some(draft.item.id) != worn_physics(world)
                || world
                    .inventory
                    .items
                    .get(&draft.item.id)
                    .is_none_or(|i| i.asset_id != draft.item.asset_id))
        {
            self.close(world);
            self.message = aurora_i18n::tr!("physics-changed").into();
            self.open = true;
        }
        let item = self.edit_when_worn?;
        (worn_physics(world) == Some(item)).then(|| {
            self.edit_when_worn = None;
            item
        })
    }

    /// "New physics": where to create it and its name (createWearable with
    /// `wear`: the Clothing folder, LLWearableType's default name).
    pub fn begin_create(&mut self, world: &World) -> Option<(Uuid, String)> {
        if self.creating.is_some() {
            return None;
        }
        let parent = crate::world::appearance::system_folder(&world.inventory, FT_CLOTHING).unwrap_or(world.inventory.root);
        let name = aurora_i18n::tr!("physics-new-name").to_owned();
        self.creating = Some(Creating {
            parent,
            name: name.clone(),
        });
        self.message = aurora_i18n::tr!("physics-creating").into();
        Some((parent, name))
    }

    /// An item the server created: true when it is the new layer, which is
    /// then edited once worn.
    pub fn created(&mut self, item: &InvItem) -> bool {
        let ours = self
            .creating
            .as_ref()
            .is_some_and(|c| is_physics(item) && item.parent == c.parent && item.name == c.name);
        if ours {
            self.creating = None;
            self.message.clear();
            self.edit_once_worn(item.id);
        }
        ours
    }

    /// Open the editor on `item` once the outfit wears it.
    pub fn edit_once_worn(&mut self, item: Uuid) {
        self.edit_when_worn = Some(item);
    }

    /// The creation failed: "New physics" can be chosen again. True when
    /// one was waiting.
    pub fn creation_failed(&mut self) -> bool {
        let waiting = self.creating.take().is_some();
        if waiting {
            self.message.clear();
        }
        waiting
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::scene::{avatar::AvatarLibrary, body_physics};
    use crate::world::appearance::{Action, demo_mutate, plan};
    use std::{collections::HashMap, sync::Arc};

    fn wear(world: &mut World, item: Uuid) {
        let (change, _) = plan(
            &world.inventory,
            Action::WearItem {
                item,
                replace: true,
                point: 0,
            },
            &HashMap::new(),
        )
        .expect("wear plan");
        demo_mutate(&mut world.inventory, world.agent_id, &change).expect("worn");
    }

    /// A demo outfit wearing a new physics layer, its editor loaded.
    pub(crate) fn fixture() -> (World, State, Uuid) {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(42);
        world.inventory.root = Uuid::from_u128(43);
        crate::world::appearance::seed_demo(&mut world.inventory, world.agent_id);
        let mut state = State::default();
        let (parent, name) = state.begin_create(&world).expect("create");
        let item = InvItem {
            id: Uuid::from_u128(900),
            parent,
            name,
            asset_type: 5,
            inv_type: 18,
            flags: 15,
            owner: world.agent_id,
            creator: world.agent_id,
            base_mask: 0x7fff_ffff,
            owner_mask: 0x7fff_ffff,
            ..Default::default()
        };
        assert!(state.created(&item));
        world.inventory.add_items(vec![item.clone()]);
        assert_eq!(state.update(&mut world), None, "not worn yet");
        wear(&mut world, item.id);
        assert_eq!(state.update(&mut world), Some(item.id), "worn: edit it");
        assert!(state.open_item(&mut world, item.id).is_some());
        let data = crate::world::inventory::wearable::create(KIND, &item.name, world.agent_id);
        assert!(state.loaded(&mut world, item.id, item.asset_id, &data));
        (world, state, item.id)
    }

    fn body_physics_of(world: &World, male: bool) -> Option<body_physics::Params> {
        let lib = &world.avatar_lib;
        let rig = body_physics::PhysicsRig::new(&lib.shape_params, &lib.rig).expect("physics rig");
        let weights = lib
            .shape_params
            .effective_weights(world.visual_params(&world.agent_id).unwrap_or(&[]));
        body_physics::Params::from_weights(&rig, &weights, male)
    }

    #[test]
    fn the_catalogue_is_avatar_lads_physics_parameters() {
        let lad = include_bytes!("../assets/character/avatar_lad.xml");
        let root = aurora_llsd::xml::parse(lad).expect("avatar_lad.xml");
        let mut found = Vec::new();
        let mut stack = vec![&root];
        while let Some(e) = stack.pop() {
            if e.name == "param" && e.attr("wearable") == Some("physics") && e.attr("group") == Some("0") {
                let f = |name| e.attr(name).and_then(|v| v.trim().parse::<f32>().ok()).expect("number");
                let id = e.attr("id").and_then(|v| v.trim().parse::<i32>().ok()).expect("id");
                found.push((
                    id,
                    e.attr("edit_group").expect("edit group").to_owned(),
                    e.attr("sex") == Some("female"),
                ));
                assert_eq!(
                    param_by_id(id),
                    Some(&param(
                        id,
                        f("value_min"),
                        f("value_max"),
                        f("value_default"),
                        e.attr("sex") == Some("female")
                    )),
                    "{id}"
                );
            }
            stack.extend(e.elements());
        }
        assert_eq!(found.len(), PARAMS.len());
        let edit_groups = [
            "physics_breasts_updown",
            "physics_breasts_inout",
            "physics_breasts_leftright",
            "physics_belly_updown",
            "physics_butt_updown",
            "physics_butt_leftright",
            "physics_advanced",
        ];
        for (group, name) in GROUPS.iter().zip(edit_groups) {
            let mut ids: Vec<i32> = found.iter().filter(|f| f.1 == name).map(|f| f.0).collect();
            ids.sort_unstable();
            assert_eq!(group.params, ids, "{name}: every parameter, in display order");
        }
        assert!(GROUPS[..3].iter().all(|g| g.female) && GROUPS[3..].iter().all(|g| !g.female));
    }

    #[test]
    fn a_male_avatar_sees_no_breast_tab_nor_breast_parameter() {
        assert_eq!(visible_groups(false), (0..7).collect::<Vec<_>>());
        assert_eq!(visible_groups(true), vec![3, 4, 5, 6]);
        assert_eq!(visible_params(6, false).len(), 9);
        let advanced: Vec<i32> = visible_params(6, true).iter().map(|p| p.id).collect();
        assert_eq!(advanced, vec![10011, 10012, 10013, 10018, 10019, 10020]);
        assert!(visible_params(7, false).is_empty());
        let labels: Vec<_> = visible_params(4, false).iter().map(|p| p.label()).collect();
        assert_eq!(
            labels,
            vec![
                aurora_i18n::tr!("physics-max-effect"),
                aurora_i18n::tr!("physics-spring"),
                aurora_i18n::tr!("physics-gain"),
                aurora_i18n::tr!("physics-damping"),
            ]
        );
    }

    #[test]
    fn a_new_physics_wearable_serializes_its_defaults_and_round_trips_edits() {
        let owner = Uuid::from_u128(5);
        let bytes = crate::world::inventory::wearable::create(KIND, "New Physics", owner).expect("asset");
        let text = String::from_utf8(bytes.clone()).expect("UTF-8");
        assert!(text.starts_with("LLWearable version 22\nNew Physics\n"));
        assert!(text.contains("\ntype 15\nparameters 33\n"));
        assert!(text.ends_with("textures 0\n"), "a physics layer has no texture");
        assert!(text.contains(&format!("creator_id {owner}")));
        let mut wearable = Wearable::parse(&bytes).expect("parse");
        assert_eq!(wearable.kind, KIND);
        for p in &PARAMS {
            assert_eq!(wearable.params.get(&p.id), Some(&p.default), "{}", p.id);
        }
        wearable.params.insert(10003, 2.25);
        let copy = Wearable::parse(&wearable.encode("Bouncy").expect("encode")).expect("round trip");
        assert_eq!(copy.params[&10003], 2.25);
        assert_eq!(copy.params.len(), 33);
        assert_eq!(copy.kind, KIND);
    }

    #[test]
    fn sliders_drive_the_local_body_physics_until_the_editor_closes() {
        let (mut world, mut state, _) = fixture();
        // a new layer: every Max Effect at 0, nothing moves
        assert_eq!(body_physics_of(&world, false), None);
        let draft = state.draft.as_mut().expect("draft");
        assert!(!draft.dirty());
        let breast = param_by_id(10003).expect("breast bounce max effect");
        set(draft, breast, 2.0);
        state.changed(&mut world);
        assert!(state.draft.as_ref().expect("draft").dirty());
        let preview = body_physics_of(&world, false).expect("the breasts bounce");
        assert_eq!(body_physics_of(&world, true), None, "breasts only: nothing on a male body");
        // a male body still bounces its belly
        set(state.draft.as_mut().expect("draft"), param_by_id(10014).expect("belly"), 1.0);
        state.changed(&mut world);
        assert!(body_physics_of(&world, true).is_some());
        assert_ne!(body_physics_of(&world, false), Some(preview));
        state.revert(&mut world);
        assert_eq!(body_physics_of(&world, false), None, "revert: back to the saved layer");
        set(state.draft.as_mut().expect("draft"), breast, 2.0);
        state.changed(&mut world);
        state.close(&mut world);
        assert_eq!(world.visual_params(&world.agent_id).unwrap_or(&[]), world.server_visual_params());
        assert_eq!(body_physics_of(&world, false), None, "closed: the appearance again");
    }

    #[test]
    fn values_are_clamped_and_stray_parameters_are_not_previewed() {
        let (mut world, mut state, _) = fixture();
        let draft = state.draft.as_mut().expect("draft");
        let spring = param_by_id(10004).expect("spring");
        set(draft, spring, 500.0);
        assert_eq!(value(draft, spring), 100.0);
        set(draft, spring, f32::NAN);
        assert_eq!(value(draft, spring), 100.0);
        set(draft, spring, -1.0);
        assert_eq!(value(draft, spring), 0.0);
        // a shape parameter hidden in a physics asset (height)
        draft.edited.params.insert(33, 3.0);
        let before = world.server_visual_params().to_vec();
        state.changed(&mut world);
        let height = world
            .avatar_lib
            .shape_params
            .weights(world.visual_params(&world.agent_id).expect("preview"))[&33];
        assert_eq!(height, world.avatar_lib.shape_params.weights(&before)[&33]);
    }

    #[test]
    fn save_uploads_the_edited_layer_and_save_as_needs_copy() {
        let (mut world, mut state, item) = fixture();
        set(state.draft.as_mut().expect("draft"), param_by_id(10021).expect("butt"), 1.5);
        let Some(NetCommand::SaveWearable {
            request,
            item: saved,
            data,
            copy_name,
        }) = state.save(&world, None)
        else {
            panic!("save command");
        };
        assert!(saved.id == item && copy_name.is_none());
        assert!(is_physics(&saved));
        let asset = Wearable::parse(&data).expect("asset");
        assert_eq!((asset.kind, asset.params[&10021]), (KIND, 1.5));
        assert!(state.save(&world, None).is_none(), "one save at a time");
        let mut done = (*saved).clone();
        done.asset_id = Uuid::from_u128(77);
        assert_eq!(state.saved(&mut world, request, Ok(done)), Some(item));
        assert!(!state.draft.as_ref().expect("draft").dirty());
        assert_eq!(state.message, aurora_i18n::tr!("physics-saved"));
        // no copy permission: no Save As
        world.inventory.items.get_mut(&item).expect("item").owner_mask = 0x4000;
        state.draft.as_mut().expect("draft").item.asset_id = Uuid::from_u128(77);
        assert!(state.save(&world, Some("Copy".into())).is_none());
        assert_eq!(state.message, aurora_i18n::tr!("physics-save-denied"));
    }

    #[test]
    fn the_editor_closes_when_its_layer_is_taken_off_and_ignores_other_creations() {
        let (mut world, mut state, item) = fixture();
        let (change, _) = plan(&world.inventory, Action::Remove(item), &HashMap::new()).expect("take off");
        demo_mutate(&mut world.inventory, world.agent_id, &change).expect("removed");
        assert_eq!(state.update(&mut world), None);
        assert!(state.draft.is_none() && state.open);
        assert_eq!(state.message, aurora_i18n::tr!("physics-changed"));
        // a shirt or another name is not the layer being created
        let (parent, name) = state.begin_create(&world).expect("create");
        assert!(state.begin_create(&world).is_none(), "one at a time");
        let mut other = InvItem {
            id: Uuid::from_u128(901),
            parent,
            name: name.clone(),
            asset_type: 5,
            flags: 4,
            ..Default::default()
        };
        assert!(!state.created(&other));
        other.flags = 15;
        other.name = "Elsewhere".into();
        assert!(!state.created(&other));
        state.creation_failed();
        assert!(state.begin_create(&world).is_some());
    }
}

//! "Inspect Objects" (ui/inspect.rs): which prims the window lists
//! and where their figures come from — the ObjectProperties the simulator
//! sends for selected prims, the scene for geometry and textures. Follows
//! LLFloaterInspect::onOpen / refresh (indra/newview/llfloaterinspect.cpp).
use super::*;
use crate::ui::inspect::{Row, Totals};
use crate::world::names::NameScope;
use aurora_net::RegionHandle;
use aurora_net::build::BuildCmd;
use std::collections::HashMap;
use uuid::Uuid;

impl App {
    /// "Inspect" of a right-click menu on object `id`: its whole
    /// linkset, as the menu's selection in Firestorm.
    pub(super) fn inspect_object(&mut self, id: Uuid) {
        self.inspect_objects(&[id]);
    }

    /// "Inspect" of the Area Search menu: the linksets of every selected
    /// row, each once (selectObjectAndFamily, then Floater "inspect").
    pub(super) fn inspect_objects(&mut self, ids: &[Uuid]) {
        let mut roots = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for id in ids {
            if let Some(root) = self
                .world
                .objects
                .index_of_uuid(id)
                .map(|idx| crate::build::root_of(&self.world, idx))
                .and_then(|root| self.world.objects.get(root))
                .map(|o| o.full_id)
                && seen.insert(root)
            {
                roots.push(root);
            }
        }
        if roots.is_empty() {
            return;
        }
        self.inspect_wire(false);
        self.inspect_ui.inspect(roots);
        // names, owners and dates come with the selection of each prim
        self.inspect_wire(true);
    }

    /// "Inspect" of an avatar's menu, "Inspect everything worn" of a worn
    /// object's: every object it wears, one linkset each, in the window
    /// LLFloaterInspect gives a selection. Not in the Legacy Viewer, whose
    /// avatar menu has no Inspect and whose worn object's Inspect shows that
    /// object alone (menu_attachment_other.xml).
    pub(super) fn inspect_avatar(&mut self, id: Uuid) {
        let roots = worn_roots(&self.world, &id);
        if roots.is_empty() {
            return;
        }
        self.inspect_wire(false);
        self.inspect_ui.inspect(roots);
        self.inspect_wire(true);
    }

    /// The window was closed: its prims are no longer selected.
    pub(super) fn inspect_closed(&mut self) {
        self.inspect_wire(false);
        self.inspect_ui.roots.clear();
    }

    /// Prims of the inspected objects, root first: (index, is the root).
    /// An avatar sitting on one is not part of it.
    pub(super) fn inspect_prims(&self) -> Vec<(usize, bool)> {
        let w = &self.world;
        self.inspect_ui
            .roots
            .iter()
            .filter_map(|id| w.objects.index_of_uuid(id))
            .flat_map(|root| {
                crate::build::family(w, root)
                    .into_iter()
                    .filter(|&i| w.objects.get(i).is_some_and(|o| !o.is_avatar()))
                    .map(move |i| (i, i == root))
            })
            .collect()
    }

    /// ObjectSelect / ObjectDeselect of the inspected prims. What the build
    /// tools have selected themselves stays selected.
    fn inspect_wire(&mut self, select: bool) {
        let kept: HashSet<usize> = if select {
            HashSet::new()
        } else {
            self.build.highlighted(&self.world).into_iter().map(|(i, _)| i).collect()
        };
        let mut by_region: HashMap<RegionHandle, Vec<u32>> = HashMap::new();
        for (idx, _) in self.inspect_prims() {
            if let Some(o) = self.world.objects.get(idx).filter(|_| !kept.contains(&idx)) {
                by_region.entry(o.key.region).or_default().push(o.key.local_id);
            }
        }
        for (handle, local_ids) in by_region {
            self.send(NetCommand::Build(if select {
                BuildCmd::Select { handle, local_ids }
            } else {
                BuildCmd::Deselect { handle, local_ids }
            }));
        }
    }

    /// The inspected prims to outline in the world: the highlighted row in
    /// the root's color, the others in the children's (Firestorm marks it
    /// with sHighlightInspectColor). HUDs keep their own selection.
    pub(super) fn inspect_outline(&self) -> Vec<(usize, bool)> {
        if !self.inspect_ui.open {
            return Vec::new();
        }
        let current = self.inspect_ui.selected;
        self.inspect_prims()
            .into_iter()
            .filter(|(i, _)| self.scene.gpu.get(*i).is_some_and(|g| !g.hud))
            .filter_map(|(i, _)| Some((i, Some(self.world.objects.get(i)?.full_id) == current)))
            .collect()
    }

    /// Rows and totals of the window, this frame (LLFloaterInspect::refresh).
    pub(super) fn inspect_rows(&mut self) -> (Vec<Row>, Totals) {
        // objects that are gone (detached, derezzed) leave the list
        let world = &self.world;
        self.inspect_ui.roots.retain(|id| world.objects.index_of_uuid(id).is_some());
        let prims = self.inspect_prims();
        let mut totals = Totals {
            objects: self.inspect_ui.roots.len() as u32,
            prims: prims.len() as u32,
            ..Totals::default()
        };
        // a linkset at a time, for its complexity
        let mut linkset: Vec<usize> = Vec::new();
        for (i, (idx, root)) in prims.iter().enumerate() {
            if *root && !linkset.is_empty() {
                totals.complexity += self.scene.linkset_complexity(&self.world, &linkset);
                linkset.clear();
            }
            linkset.push(*idx);
            if i + 1 == prims.len() {
                totals.complexity += self.scene.linkset_complexity(&self.world, &linkset);
            }
        }
        let mut textures: HashSet<Uuid> = HashSet::new();
        let mut rows = Vec::with_capacity(prims.len());
        let loading = aurora_i18n::tr!("common-loading-2");
        for (idx, root) in prims {
            let Some(id) = self.world.objects.get(idx).map(|o| o.full_id) else {
                continue;
            };
            let stats = self.scene.prim_stats(&self.world, idx);
            let mut row = Row {
                id,
                root,
                name: loading.into(),
                faces: stats.faces,
                vertices: stats.vertices,
                triangles: stats.triangles,
                ..Row::default()
            };
            for texture in &stats.textures {
                let (memory, vram) = self.scene.texture_memory(texture);
                row.texture_memory += memory;
                row.vram += vram;
                if textures.insert(*texture) {
                    totals.texture_memory += memory;
                    totals.vram += vram;
                }
            }
            totals.faces += stats.faces;
            totals.visible_faces += stats.visible_faces;
            totals.vertices += stats.vertices;
            totals.triangles += stats.triangles;
            // the full properties of a selected prim; the family subset the
            // hover asks for has neither creator nor date
            if let Some(props) = self.build.props.get(&id).filter(|props| props.full).cloned() {
                row.name = props.name;
                row.description = props.description;
                row.created = props.creation_date / 1_000_000;
                // LLPermissions::isGroupOwned
                let group_owned = !props.group_id.is_nil() && (props.owner_id.is_nil() || props.owner_id == props.group_id);
                if group_owned {
                    let group = self
                        .world
                        .groups
                        .group(&props.group_id)
                        .map(|g| g.name.clone())
                        .unwrap_or_else(|| aurora_i18n::tr!("common-group").into());
                    row.owner_name = aurora_i18n::tr!("inspect-group-owner", name = group);
                } else if !props.owner_id.is_nil() {
                    self.world.social.want_name(props.owner_id);
                    (row.owner_name, row.owner) = owner_shown(&self.world, props.owner_id);
                }
                if !props.creator_id.is_nil() {
                    self.world.social.want_name(props.creator_id);
                    (row.creator_name, row.creator) = creator_shown(&self.world, props.owner_id, props.creator_id);
                    row.creator_named_here_alone = row.creator.is_some() && !self.world.rlva_shows_name(NameScope::World, props.creator_id);
                }
            }
            rows.push(row);
        }
        totals.textures = textures.len() as u32;
        (rows, totals)
    }
}

/// The resident who owns a prim as its row names them, and whose profile
/// "See Owner Profile" opens. Under RLVa's `@shownames` the owner is their
/// stand-in and the button is off (LLFloaterInspect::refresh,
/// onSelectObject and onClickOwnerProfile, indra/newview/llfloaterinspect.cpp).
fn owner_shown(world: &crate::world::World, owner: Uuid) -> (String, Option<Uuid>) {
    let scope = NameScope::Owner;
    (
        world.social.name_in(scope, &owner),
        world.rlva_shows_name(scope, owner).then_some(owner),
    )
}

/// `owner_shown` for the creator of a prim `owner` owns. The same sources:
/// the creator is hidden only when they also own the object or stand
/// around, and "See Creator Profile" follows the name, so it opens the
/// profile of a creator the world hides (`button creator` is enabled by
/// `(idOwner != idCreator && !isNearbyAgent(idCreator)) || canShowName`).
fn creator_shown(world: &crate::world::World, owner: Uuid, creator: Uuid) -> (String, Option<Uuid>) {
    let scope = NameScope::Creator { owner };
    (
        world.social.name_in(scope, &creator),
        world.rlva_shows_name(scope, creator).then_some(creator),
    )
}

/// The root prims of what avatar `id` wears, as the world has them.
pub(super) fn worn_roots(world: &crate::world::World, id: &Uuid) -> Vec<Uuid> {
    let Some(avatar) = world.objects.index_of_uuid(id).and_then(|i| world.objects.get(i)) else {
        return Vec::new();
    };
    world
        .objects
        .children_of(&avatar.key)
        .iter()
        .filter_map(|&i| world.objects.get(i))
        .filter(|o| !o.is_avatar())
        .map(|o| o.full_id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LLFloaterInspect under `@shownames`: the owner anonymous and without
    /// a profile; the creator named, with their profile, unless they own
    /// the object or stand around.
    #[test]
    fn shownames_names_the_creator_who_is_away_and_opens_their_profile() {
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let near = world
            .objects
            .avatars()
            .find(|(_, o)| o.full_id != world.agent_id)
            .map(|(_, o)| o.full_id)
            .unwrap();
        let (maker, owner) = (Uuid::from_u128(0xF3), Uuid::from_u128(0xF1));
        for (id, first) in [(near, "Near"), (maker, "Maker"), (owner, "Owner")] {
            world.social.avatar_names.on_legacy(id, first, "Tester");
        }
        // no restriction: both named, both profiles
        assert_eq!(owner_shown(&world, owner), ("Owner Tester".into(), Some(owner)));
        assert_eq!(creator_shown(&world, owner, near), ("Near Tester".into(), Some(near)));
        world.rlva.configure(true);
        assert!(world.rlva.receive(&aurora_net::ChatMessage {
            from_name: "Inspect fixture".into(),
            source_id: Uuid::from_u128(4),
            owner_id: world.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            region: None,
            position: glam::Vec3::ZERO,
            message: "@shownames=n".into(),
        }));
        world.refresh_rlva_privacy();
        let (name, profile) = owner_shown(&world, owner);
        assert!(!name.contains("Tester") && profile.is_none(), "{name}");
        // away, on someone else's object
        assert_eq!(creator_shown(&world, owner, maker), ("Maker Tester".into(), Some(maker)));
        assert!(!world.rlva_shows_name(NameScope::World, maker), "the world still hides them");
        // their own object, or around us
        for (owner, creator) in [(maker, maker), (owner, near)] {
            let (name, profile) = creator_shown(&world, owner, creator);
            assert!(!name.contains("Tester") && profile.is_none(), "{name}");
        }
        // ourselves, always
        let me = world.agent_id;
        assert_eq!(owner_shown(&world, me).1, Some(me));
        assert_eq!(creator_shown(&world, me, me).1, Some(me));
    }
}

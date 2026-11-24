//! "Inspect Objects" (ui/inspect.rs): which prims the window lists
//! and where their figures come from — the ObjectProperties the simulator
//! sends for selected prims, the scene for geometry and textures. Follows
//! LLFloaterInspect::onOpen / refresh (indra/newview/llfloaterinspect.cpp).
use super::*;
use crate::object_stage::{Group, Place};
use crate::ui::inspect::{Action, Person, Row, Subject, Totals};
use crate::world::names::NameScope;
use aurora_net::RegionHandle;
use aurora_net::build::BuildCmd;
use std::collections::HashMap;
use uuid::Uuid;

/// Land impacts asked for in one request, and for one window.
const COST_BATCH: usize = 64;
const COST_MOST: usize = 512;

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
        self.inspect_ui.inspect(roots, Subject::Objects);
        // names, owners and dates come with the selection of each prim
        self.inspect_wire(true);
    }

    /// "Inspect" of an avatar's menu: every object it wears, one linkset
    /// each, in the window LLFloaterInspect gives the selection of one
    /// worn object (menu_attachment_other.xml).
    pub(super) fn inspect_avatar(&mut self, id: Uuid) {
        let roots = worn_roots(&self.world, &id);
        if roots.is_empty() {
            return;
        }
        self.inspect_wire(false);
        self.inspect_ui.inspect(roots, Subject::Avatar(id));
        self.inspect_wire(true);
    }

    /// The window was closed: its prims are no longer selected.
    pub(super) fn inspect_closed(&mut self) {
        self.inspect_wire(false);
        self.inspect_ui.roots.clear();
        self.inspect_ui.stage.stage.clear(&self.scene);
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
        // a linkset at a time, for its complexity, kept on its root's row
        let mut complexity: HashMap<usize, u32> = HashMap::new();
        let mut linkset: Vec<usize> = Vec::new();
        for (i, (idx, root)) in prims.iter().enumerate() {
            if *root && !linkset.is_empty() {
                let value = self.scene.linkset_complexity(&self.world, &linkset);
                complexity.insert(linkset[0], value);
                totals.complexity += value;
                linkset.clear();
            }
            linkset.push(*idx);
            if i + 1 == prims.len() {
                let value = self.scene.linkset_complexity(&self.world, &linkset);
                complexity.insert(linkset[0], value);
                totals.complexity += value;
            }
        }
        let mut textures: HashSet<Uuid> = HashSet::new();
        let mut rows = Vec::with_capacity(prims.len());
        let loading = aurora_i18n::tr!("common-loading-2");
        let mut point = 0;
        for (idx, root) in prims {
            if root {
                // the attachment point of the linkset (0: in the world)
                point = self
                    .world
                    .objects
                    .get(idx)
                    .filter(|o| o.is_attachment())
                    .map_or(0, |o| o.attachment_point());
            }
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
                point,
                complexity: complexity.get(&idx).copied().unwrap_or(0),
                land_impact: (root && point == 0)
                    .then(|| self.build.costs.objects.get(&id).map(|c| c.linked_set))
                    .flatten(),
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
                    row.owner_group = Some(props.group_id);
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

impl App {
    /// The Inspect window this frame, and what it asks for.
    pub(super) fn inspect_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if !self.inspect_ui.open {
            return;
        }
        let (rows, totals) = self.inspect_rows();
        // (a demo's view is never kept in the settings)
        let forced = self.inspect_ui.forced_wide;
        let mut wide = forced.unwrap_or(self.settings.inspect_wide);
        let board = self.inspect_ui.is_wide(ctx, wide);
        let objects = self.inspect_ui.subject == Subject::Objects;
        let people = if board && !objects { self.inspect_people() } else { Vec::new() };
        let parcel = (board && objects).then(|| self.inspect_parcel(&rows)).flatten();
        if board {
            self.inspect_stage(ctx);
            if objects {
                self.inspect_costs();
            }
        } else {
            self.inspect_ui.stage.stage.clear(&self.scene);
        }
        let (name, username, profile) = match self.inspect_ui.subject {
            Subject::Avatar(id) => {
                self.world.social.want_name(id);
                let shown = self.world.rlva_shows_name(NameScope::World, id);
                (
                    self.world.social.display_name_in(NameScope::World, &id),
                    shown
                        .then(|| crate::ui::profile::web_username(&self.world, NameScope::World, &id))
                        .flatten(),
                    shown.then_some(id),
                )
            }
            Subject::Objects => (String::new(), None, None),
        };
        let mut columns = if objects {
            self.settings.inspect_columns
        } else {
            self.settings.inspect_avatar_columns
        };
        let mut view = self.settings.inspect_view;
        let actions = self.inspect_ui.show(
            ctx,
            p,
            crate::ui::inspect::Frame {
                rows: &rows,
                totals: &totals,
                subject_name: &name,
                subject_username: username.as_deref(),
                subject_profile: profile,
                pics: &self.avatar_pics,
                point_names: &self.world.avatar_lib.attach_names,
                max_complexity: self.settings.max_complexity,
                people: &people,
                parcel,
                columns: &mut columns,
                view: &mut view,
            },
            &mut wide,
        );
        let kept = if objects {
            &mut self.settings.inspect_columns
        } else {
            &mut self.settings.inspect_avatar_columns
        };
        if columns != *kept || view != self.settings.inspect_view || (forced.is_none() && wide != self.settings.inspect_wide) {
            *kept = columns;
            self.settings.inspect_view = view;
            if forced.is_none() {
                self.settings.inspect_wide = wide;
            }
            self.settings.save();
        }
        for action in actions {
            match action {
                Action::Profile {
                    who,
                    named_here_alone: false,
                } => self.profile_ui.open(&mut self.world, who),
                Action::Profile {
                    who,
                    named_here_alone: true,
                } => {
                    crate::ui::profile::request_open_in(ctx, NameScope::Roster, who);
                }
                Action::LookAt(id) => self.inspect_look_at(id),
                Action::Group(id) => self.group_profile_ui.open(&mut self.world, id),
            }
        }
        if !self.inspect_ui.open {
            self.inspect_closed();
        }
    }

    /// Keep the 3D view of the wide window up to date with what is inspected.
    fn inspect_stage(&mut self, ctx: &egui::Context) {
        let w = &self.world;
        let place = match self.inspect_ui.subject {
            Subject::Avatar(id) => w.objects.index_of_uuid(&id).map_or(Place::World, Place::Worn),
            Subject::Objects => Place::World,
        };
        let group = |root: &Uuid| stage_group(w, root);
        let now = std::time::Instant::now();
        let ui = &mut self.inspect_ui;
        ui.stage.stage.update(w, &self.scene, &ui.roots, group, place, ctx, now);
    }

    /// The people here and their complexity, as the world names them.
    fn inspect_people(&mut self) -> Vec<Person> {
        let own = self.world.agent_id;
        let seen = self.seen_avatars();
        // RLVa `@shownearby`: no list of the people around (as Near me)
        let alone = self.world.rlva.has("shownearby");
        let subject = match self.inspect_ui.subject {
            Subject::Avatar(id) => Some(id),
            Subject::Objects => None,
        };
        seen.into_iter()
            .filter(|s| s.weight > 0 && (!alone || s.id == own || Some(s.id) == subject))
            .map(|s| {
                self.world.social.want_name(s.id);
                let shown = self.world.rlva_shows_name(NameScope::World, s.id);
                Person {
                    id: s.id,
                    pic: shown.then_some(s.id),
                    name: self.world.social.display_name_in(NameScope::World, &s.id),
                    complexity: s.weight,
                    you: s.id == own,
                    friend: shown && self.world.social.is_friend(&s.id),
                    drawn: !s.too_complex,
                }
            })
            .collect()
    }

    /// Land impact used and allowed on our parcel, when every object listed
    /// stands on it.
    fn inspect_parcel(&self, rows: &[Row]) -> Option<(i32, i32)> {
        let parcel = self.world.parcel.as_ref().filter(|p| !p.bitmap.is_empty())?;
        let origin = self.world.main_origin()?;
        let now = std::time::Instant::now();
        let here = rows.iter().filter(|r| r.root).all(|r| {
            self.world
                .objects
                .index_of_uuid(&r.id)
                .and_then(|idx| crate::scene::Scene::object_transform(&self.world, idx, now, 0))
                .is_some_and(|xf| {
                    let (x, y) = crate::ui::minimap::to_global(&self.world, xf.0);
                    crate::world::landmarks::in_parcel(&parcel.bitmap, origin, glam::DVec3::new(x, y, f64::from(xf.0.z)))
                })
        });
        let max_tasks = self
            .world
            .main_region
            .and_then(|h| self.world.regions.get(&h))
            .map_or(0, |r| r.max_tasks);
        let usage = crate::world::land::PrimUsage::of(parcel, max_tasks);
        here.then_some((usage.used, usage.max))
    }

    /// Ask for the land impact of the inspected objects not known yet, once
    /// each (GetObjectCost, as LLSelectMgr asks for its selection).
    fn inspect_costs(&mut self) {
        let mut by_region: HashMap<RegionHandle, Vec<Uuid>> = HashMap::new();
        for id in self.inspect_ui.roots.iter().take(COST_MOST) {
            let Some(o) = self.world.objects.index_of_uuid(id).and_then(|i| self.world.objects.get(i)) else {
                continue;
            };
            if !o.is_attachment() && self.build.costs.ask_once(o.full_id) {
                by_region.entry(o.key.region).or_default().push(o.full_id);
            }
        }
        for (handle, ids) in by_region {
            for batch in ids.chunks(COST_BATCH) {
                self.build.ask_costs(handle, batch.to_vec());
            }
        }
    }

    /// AURORA_DEMO_INSPECT=wolf|self|objects[:options], the options separated
    /// by commas (`InspectUi::demo`: wide, small, open, creators, details, view…):
    /// Inspect on what Violet Wolf or the demo agent wears, or on objects of
    /// the plaza, once they are in view.
    pub(super) fn demo_inspect(&mut self) {
        if !self.demo || self.inspect_ui.demo_done {
            return;
        }
        let Ok(scenario) = std::env::var("AURORA_DEMO_INSPECT") else {
            self.inspect_ui.demo_done = true;
            return;
        };
        let (who, option) = scenario.split_once(':').unwrap_or((scenario.as_str(), ""));
        match who {
            "objects" | "pbr" => {
                // (`pbr`: those with a PBR material)
                let ids: Vec<Uuid> = self
                    .world
                    .objects
                    .iter()
                    .filter(|(_, o)| !o.is_avatar() && !o.is_attachment() && o.parent_id == 0)
                    .filter(|(_, o)| who == "objects" || !o.extra.render_materials.is_empty())
                    .map(|(_, o)| o.full_id)
                    .take(6)
                    .collect();
                if ids.len() < if who == "objects" { 6 } else { 1 } {
                    return;
                }
                self.inspect_objects(&ids);
            }
            "market" => {
                let ids: Vec<Uuid> = crate::demo::market::some()
                    .into_iter()
                    .filter_map(|local| self.world.objects.iter().find(|(_, o)| o.key.local_id == local))
                    .map(|(_, o)| o.full_id)
                    .collect();
                if ids.is_empty() {
                    return;
                }
                self.inspect_objects(&ids);
            }
            // someone of AURORA_DEMO_CROWD: many rigged meshes
            "crowd" => {
                let known = [crate::demo::DEMO_AGENT, crate::demo::DEMO_LOUP];
                let who = self
                    .world
                    .objects
                    .avatars()
                    .map(|(_, o)| o.full_id)
                    .find(|id| !known.contains(id) && worn_roots(&self.world, id).len() > 1);
                let Some(id) = who else { return };
                self.inspect_avatar(id);
            }
            who => {
                let id = if who == "self" {
                    crate::demo::DEMO_AGENT
                } else {
                    crate::demo::DEMO_LOUP
                };
                if worn_roots(&self.world, &id).len() < 2 {
                    return;
                }
                self.inspect_avatar(id);
            }
        }
        option.split(',').for_each(|o| self.inspect_ui.demo(o));
        self.inspect_ui.demo_done = true;
    }

    /// Look at: the menu's Zoom on the object, the camera following it (an
    /// attachment: its avatar), as Area Search's Zoom.
    fn inspect_look_at(&mut self, id: Uuid) {
        let now = std::time::Instant::now();
        let Some(position) = self
            .world
            .objects
            .index_of_uuid(&id)
            .and_then(|idx| crate::scene::Scene::object_transform(&self.world, idx, now, 0))
            .map(|xf| xf.0)
        else {
            return;
        };
        self.on_ctx_action(crate::ui::context::CtxAction::Zoom(position, Some(id)));
    }
}

/// An object and its prims, for the 3D views of Inspect and Area Search.
pub(super) fn stage_group(world: &crate::world::World, root: &Uuid) -> Option<Group> {
    let idx = world.objects.index_of_uuid(root)?;
    let prims = crate::build::family(world, idx)
        .into_iter()
        .filter(|i| world.objects.get(*i).is_some_and(|o| !o.is_avatar()))
        .collect();
    Some(Group { root: *root, prims })
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
        // (not the copies drawn from the cache, world/avatar_cache)
        .filter(|o| !o.is_avatar() && !o.is_client_preview())
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

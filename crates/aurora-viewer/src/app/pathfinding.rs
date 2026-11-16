//! The pathfinding windows in the app: opening them (Build › Pathfinding,
//! the object menu's Show in…), their requests and answers, the owner names,
//! the enable rules from the in-world selection, the list selection made
//! the in-world one, the actions, and Show beacon. Port of
//! LLFloaterPathfindingObjects (requestGetObjects, selectScrollListItemsInWorld,
//! updateStateOnActionControls, the Take / Return / Delete / Teleport
//! handlers, the beacons of draw()) and LLFloaterPathfindingLinksets
//! (requestSetLinksets), indra/newview, originally LGPL 2.1.
use super::*;
use crate::ui::pathfinding::{Action, Context, Facts};
use aurora_net::pathfinding::Kind;
use std::collections::HashMap;
use uuid::Uuid;

/// What the app remembers between frames for the two windows.
#[derive(Default)]
pub(super) struct PathfindingFeed {
    /// The list selection last made the in-world one, per window.
    synced: [Vec<Uuid>; 2],
    /// The facts and the selection they were worked out for.
    facts: [Facts; 2],
    facts_for: [Vec<Uuid>; 2],
    facts_at: Option<Instant>,
    /// The last look for owner names still loading.
    names_at: Option<Instant>,
    /// The region the lists were asked of (onRegionBoundaryCrossed).
    region: Option<aurora_net::RegionHandle>,
}

fn slot(kind: Kind) -> usize {
    match kind {
        Kind::Objects => 0,
        Kind::Characters => 1,
    }
}

impl App {
    /// showFloaterWithSelectionObjects: open a window, `select` chosen in it.
    pub(super) fn open_pathfinding(&mut self, kind: Kind, select: Vec<Uuid>) {
        let window = match kind {
            Kind::Objects => ui::pathfinding::LINKSETS_WINDOW,
            Kind::Characters => ui::pathfinding::CHARACTERS_WINDOW,
        };
        if self.pathfinding.open(kind, select) {
            self.refresh_pathfinding(kind);
        }
        ui::widgets::bring_floater_front(&self.egui_ctx, window);
    }

    /// Build › Pathfinding: the window opens on the in-world selection.
    pub(super) fn open_pathfinding_from_selection(&mut self, kind: Kind) {
        let select = self
            .build
            .roots(&self.world)
            .into_iter()
            .filter_map(|i| self.world.objects.get(i).map(|o| o.full_id))
            .collect();
        self.open_pathfinding(kind, select);
    }

    /// requestGetObjects: the agent's region; the terrain row too for a god
    /// or an estate manager (isAllowViewTerrainProperties).
    fn refresh_pathfinding(&mut self, kind: Kind) {
        let region = self.world.main_region.unwrap_or_default();
        self.pathfinding_feed.region = self.world.main_region;
        let terrain = kind == Kind::Objects && self.can_manage_estate();
        let request = self.pathfinding.list_mut(kind).start_get(region);
        let command = NetCommand::FetchPathfinding {
            request,
            region,
            kind,
            terrain,
        };
        if self.demo {
            self.demo_pathfinding_answer(command);
        } else {
            self.send(command);
        }
    }

    /// The lists' answers (NetEvent::Pathfinding, NetEvent::LinksetsUpdated).
    pub(super) fn on_pathfinding_event(&mut self, ev: NetEvent) -> Option<NetEvent> {
        match ev {
            NetEvent::Pathfinding {
                request,
                region,
                kind,
                result,
            } => {
                // the View tab's own list answers to its own request
                if kind == Kind::Objects {
                    self.pathfinding.overlay_list.on_list(request, region, result.clone());
                }
                self.pathfinding.list_mut(kind).on_list(request, region, result);
            }
            NetEvent::LinksetsUpdated { request, region, result } => {
                self.pathfinding.linksets.on_update(request, region, result);
                self.overlay_list_stale();
            }
            other => return Some(other),
        }
        None
    }

    /// The windows, once a frame.
    pub(super) fn pathfinding_windows(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if !self.pathfinding.linksets_open && !self.pathfinding.characters_open {
            return;
        }
        let now = Instant::now();
        // onRegionBoundaryCrossed: a new list of the new region
        if self.pathfinding_feed.region != self.world.main_region {
            for kind in [Kind::Objects, Kind::Characters] {
                if self.pathfinding.is_open(kind) {
                    self.pathfinding.list_mut(kind).clear();
                    self.refresh_pathfinding(kind);
                }
            }
        }
        self.prepare_pathfinding_lists(now);
        let facts = [
            self.pathfinding_facts(Kind::Objects, now),
            self.pathfinding_facts(Kind::Characters, now),
        ];
        self.pathfinding_feed.facts_at = Some(now);
        for action in self.pathfinding.show(ctx, p, facts) {
            self.pathfinding_action(action, now);
        }
        for kind in [Kind::Objects, Kind::Characters] {
            if self.pathfinding.is_open(kind) {
                self.sync_pathfinding_selection(kind);
            }
        }
    }

    /// The cells, with the owners' names (asked once, filled in as they
    /// arrive: the Legacy "[Loading]").
    fn prepare_pathfinding_lists(&mut self, now: Instant) {
        let names_due = self
            .pathfinding_feed
            .names_at
            .is_none_or(|at| now.duration_since(at) >= Duration::from_secs(1));
        for kind in [Kind::Objects, Kind::Characters] {
            if !self.pathfinding.is_open(kind) {
                continue;
            }
            let list = self.pathfinding.list(kind);
            if names_due && list.owners_loading() {
                self.pathfinding.list_mut(kind).mark_dirty();
            }
            let owners: Vec<(Uuid, bool)> = {
                let mut seen = std::collections::HashSet::new();
                self.pathfinding.list(kind).owners().filter(|(id, _)| seen.insert(*id)).collect()
            };
            let mut names: HashMap<Uuid, Option<String>> = HashMap::with_capacity(owners.len());
            for (id, group) in owners {
                let name = if id.is_nil() {
                    None
                } else if group {
                    self.world.land.group_name(&id, &self.world.groups.groups)
                } else if !self.world.rlva_can_name(id) {
                    Some(aurora_i18n::tr!("scene-object-hidden-person").into())
                } else {
                    self.world.social.want_name(id);
                    self.world.person_name(&id)
                };
                names.insert(id, name);
            }
            let p = self.world.agent.position;
            let cx = Context {
                owners: &names,
                agent: [f64::from(p.x), f64::from(p.y), f64::from(p.z)],
            };
            self.pathfinding.list_mut(kind).prepare(&cx);
        }
        if names_due {
            self.pathfinding_feed.names_at = Some(now);
        }
    }

    /// updateStateOnActionControls over the selected objects the viewer
    /// has (Take any, Take copy all, Return any, Delete any, as the object
    /// menus' "multiple" entries), with RLVa. Worked out again when the
    /// selection changes, else four times a second (flags can change).
    fn pathfinding_facts(&mut self, kind: Kind, now: Instant) -> Facts {
        let i = slot(kind);
        let ids = &self.pathfinding.list(kind).selection.ids;
        let fresh = self
            .pathfinding_feed
            .facts_at
            .is_some_and(|at| now.duration_since(at) < Duration::from_millis(250));
        if fresh && self.pathfinding_feed.facts_for[i] == *ids {
            return self.pathfinding_feed.facts[i];
        }
        let w = &self.world;
        let roots: Vec<usize> = ids
            .iter()
            .filter_map(|id| w.objects.index_of_uuid(id))
            .map(|idx| context_menu::linkset_root(w, idx))
            .collect();
        let flags_of = |i: usize| w.objects.get(i).map_or(0, |o| o.update_flags);
        let each: Vec<ui::context::ObjectFacts> = roots
            .iter()
            .map(|&r| {
                let family: Vec<u32> = crate::build::family(w, r).into_iter().map(flags_of).collect();
                ui::context::ObjectFacts::from_flags(flags_of(r), 0, flags_of(r), &family, false, false)
            })
            .collect();
        let rlva = &w.rlva;
        let touch_blocked = ["fartouch", "touchall", "touchworld", "touchme", "touchthis", "interact"]
            .iter()
            .any(|b| rlva.has(b));
        let facts = Facts {
            take: each.iter().any(|e| e.take),
            take_copy: !each.is_empty() && each.iter().all(|e| e.take_copy),
            // Object.EnableReturn: any object that is not worn
            return_: roots.iter().any(|&r| ui::context::wearer(w, r).is_none()),
            delete: each.iter().any(|e| e.delete),
            rlva_blocked: rlva.enabled() && (touch_blocked || !w.rlva_can_delete_or_return_all(&roots)),
            rlva_no_teleport: rlva.enabled() && rlva.has("tplocal"),
        };
        self.pathfinding_feed.facts_for[i].clone_from(ids);
        self.pathfinding_feed.facts[i] = facts;
        facts
    }

    /// selectScrollListItemsInWorld: the list's selection becomes the
    /// in-world one (objects the viewer does not have are left out).
    fn sync_pathfinding_selection(&mut self, kind: Kind) {
        let i = slot(kind);
        let ids = &self.pathfinding.list(kind).selection.ids;
        if self.pathfinding_feed.synced[i] == *ids {
            return;
        }
        self.pathfinding_feed.synced[i].clone_from(ids);
        let idxs: Vec<usize> = ids.iter().filter_map(|id| self.world.objects.index_of_uuid(id)).collect();
        self.build.deselect_all(&self.world);
        if !idxs.is_empty() {
            self.build.select_many(&mut self.world, &self.settings.build, &idxs);
        }
        self.flush_build();
    }

    /// handle_take / handle_take_copy on the selection; Take also asks the
    /// open lists again (onTakeClicked: the taken linksets leave them).
    fn take_pathfinding(&mut self, ids: &[Uuid], copy: bool) {
        let idxs: Vec<usize> = ids.iter().filter_map(|id| self.world.objects.index_of_uuid(id)).collect();
        self.build.select_many(&mut self.world, &self.settings.build, &idxs);
        self.build.take(&mut self.world, copy);
        self.flush_build();
        if !copy {
            for kind in [Kind::Objects, Kind::Characters] {
                if self.pathfinding.is_open(kind) {
                    self.refresh_pathfinding(kind);
                }
            }
        }
    }

    fn pathfinding_action(&mut self, action: Action, now: Instant) {
        match action {
            Action::Refresh(kind) => self.refresh_pathfinding(kind),
            Action::Take(ids) => self.take_pathfinding(&ids, false),
            Action::TakeCopy(ids) => self.take_pathfinding(&ids, true),
            Action::Return(ids) => self.ask_return_objects(&ids),
            Action::Delete(ids) => self.ask_delete_objects(ids),
            Action::Teleport { id, listed } => {
                let live = self
                    .world
                    .objects
                    .index_of_uuid(&id)
                    .and_then(|idx| crate::scene::Scene::object_transform(&self.world, idx, now, 0))
                    .map(|xf| xf.0);
                let at = live.unwrap_or(Vec3::new(listed[0] as f32, listed[1] as f32, listed[2] as f32));
                self.teleport_render_pos(at);
            }
            Action::Apply(apply) => {
                let request = self.pathfinding.linksets.start_set();
                let region = self.pathfinding.linksets.region;
                let command = NetCommand::SetLinksets {
                    request,
                    region,
                    objects: apply.objects,
                    terrain: apply.terrain,
                };
                if self.demo {
                    self.demo_pathfinding_answer(command);
                } else {
                    self.send(command);
                }
            }
            // onClose: the floater's in-world selection is released
            Action::Closed(kind) => {
                let i = slot(kind);
                if !std::mem::take(&mut self.pathfinding_feed.synced[i]).is_empty() && !self.build.open {
                    self.build.deselect_all(&self.world);
                    self.flush_build();
                }
            }
        }
    }

    /// Show physics capsule: the one selected character, where the viewer
    /// has it (isPhysicsCapsuleEnabled / getCapsuleRenderData).
    pub(super) fn pathfinding_capsule(&self, now: Instant) -> Option<ui::pathfinding::capsule::Capsule> {
        let id = self.pathfinding.capsule_character()?;
        let row = self.pathfinding.characters.row(&id)?;
        let aurora_net::pathfinding::Detail::Character {
            length,
            radius,
            horizontal,
            ..
        } = row.detail
        else {
            return None;
        };
        let idx = self.world.objects.index_of_uuid(&id)?;
        let (center, rotation, _) = crate::scene::Scene::object_transform(&self.world, idx, now, 0)?;
        Some(ui::pathfinding::capsule::Capsule {
            center,
            rotation,
            length: length as f32,
            radius: radius as f32,
            horizontal,
        })
    }

    /// Show beacon: each selected object the viewer has, with its name
    /// (addDebugBeacon in LLFloaterPathfindingObjects::draw). Capped to the
    /// nearest `MAX_BEACONS`: Select all on a large region would draw
    /// thousands, unreadable and costly (a deliberate difference).
    pub(super) fn pathfinding_beacons(&self, now: Instant, p: &crate::theme::Palette) -> Vec<(Vec3, egui::Color32, String)> {
        const MAX_BEACONS: usize = 256;
        let mut beacons = Vec::new();
        for (kind, color) in [(Kind::Objects, p.indigo_light), (Kind::Characters, p.danger)] {
            if !self.pathfinding.is_open(kind) || !self.pathfinding.beacons(kind) {
                continue;
            }
            let list = self.pathfinding.list(kind);
            for row in list.selected_rows() {
                if let Some(at) = self
                    .world
                    .objects
                    .index_of_uuid(&row.id)
                    .and_then(|idx| crate::scene::Scene::object_transform(&self.world, idx, now, 0))
                    .map(|xf| xf.0)
                {
                    beacons.push((at, color, row.name.clone()));
                }
            }
        }
        let agent = self.world.agent.position;
        if beacons.len() > MAX_BEACONS {
            beacons.sort_by(|a, b| a.0.distance_squared(agent).total_cmp(&b.0.distance_squared(agent)));
            beacons.truncate(MAX_BEACONS);
        }
        beacons
    }
}

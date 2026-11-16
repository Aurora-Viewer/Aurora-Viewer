//! AURORA_DEMO_PATHFINDING: the pathfinding windows on made-up lists, no
//! grid. `linksets`, `linksets-selected`, `linksets-beacon` (selected, with
//! Show beacon ticked), `linksets:<count>` (a list of that many linksets, for
//! the scale check), `characters`, `characters-selected`, `characters-beacon`.
//! The demo answers the list and edit requests itself, like the region:
//! the plaza's objects first, then numbered linksets of every kind; an edit
//! comes back with the changed linksets.
//! `rebake`: the navmesh has pending changes and the agent may rebake (the
//! rebake icon, Build › Pathfinding › Rebake region; a rebake builds for
//! `REBAKE_TIME`, then the navmesh is up to date). `rebake-disabled`: an up to
//! date navmesh in a region without dynamic pathfinding (its icon).
use super::*;
use aurora_llsd::{Llsd, llsd_map};
use aurora_net::pathfinding::navmesh::{State, Status};
use aurora_net::pathfinding::{self, Kind};
use uuid::Uuid;

/// Synthetic rows' owner, so the owner column has a name to wait for.
const DEMO_OWNER: Uuid = Uuid::from_u128(0x5afe_0000_0000_0000_0000_0000_0000_0042);
/// How long the demo region takes to rebake its navmesh.
const REBAKE_TIME: Duration = Duration::from_secs(6);

#[derive(Default)]
pub(super) struct PathfindingDemo {
    /// How many linksets the demo region holds.
    linksets: usize,
    /// A pathfinding scenario runs: the region acts as a pathfinding one.
    active: bool,
    /// When the demo's rebake finishes.
    rebake_done: Option<Instant>,
}

impl PathfindingDemo {
    /// The demo region has pathfinding (the disabled icon may show).
    pub(super) fn region(&self) -> bool {
        self.active
    }
}

fn vector(at: [f64; 3]) -> Llsd {
    Llsd::Array(at.iter().map(|&n| Llsd::Real(n)).collect())
}

fn demo_status(state: State) -> Option<Status> {
    Some(Status {
        region: Uuid::nil(),
        version: 1,
        state,
    })
}

impl App {
    pub(super) fn demo_pathfinding(&mut self, mode: &str) {
        let (name, count) = mode.split_once(':').unwrap_or((mode, ""));
        self.pathfinding_demo.linksets = count.parse().unwrap_or(40).clamp(1, 20_000);
        self.pathfinding_demo.active = true;
        self.panels.chat = false;
        self.panels.people = false;
        self.panels.minimap = false;
        if name.starts_with("view") {
            // the console's View tab with every shape and the water plane;
            // `view-xray`, `view-world-off`, `view-movables` add one option
            let console = &mut self.pathfinding.console;
            console.open = true;
            console.tab = crate::ui::pathfinding::console::Tab::View;
            let view = &mut console.view;
            view.walkables = true;
            view.material_volumes = true;
            view.static_obstacles = true;
            view.exclusion_volumes = true;
            view.water_plane = true;
            view.xray = name == "view-xray";
            view.world = name != "view-world-off";
            view.movables_only = name == "view-movables";
            let region = self.world.main_region.unwrap_or_default();
            self.pathfinding.rebake.region_changed(Some(region));
            self.pathfinding
                .rebake
                .on_status(region, demo_status(State::Complete), Instant::now());
            self.open_pathfinding_console();
            return;
        }
        if name.starts_with("path") {
            // the console's Test path tab with both ends chosen near the
            // avatar; `path-hole` starts in the demo navmesh's hole
            let console = &mut self.pathfinding.console;
            console.open = true;
            console.tab = crate::ui::pathfinding::console::Tab::Test;
            let feet = self.world.agent.position - glam::Vec3::Z * 0.9;
            let start = if name == "path-hole" {
                let [x, y] = crate::world::bridge::demo::HOLE;
                glam::Vec3::new(x, y, feet.z)
            } else {
                feet + glam::Vec3::new(-3.0, 4.0, 0.0)
            };
            console.test.set_point(false, start);
            console.test.set_point(true, feet + glam::Vec3::new(8.0, -6.0, 0.0));
            // an up-to-date navmesh (the simulator status block)
            let region = self.world.main_region.unwrap_or_default();
            if let Some(r) = self.world.regions.get_mut(&region) {
                r.dynamic_pathfinding = true;
            }
            self.pathfinding.rebake.region_changed(Some(region));
            self.pathfinding
                .rebake
                .on_status(region, demo_status(State::Complete), Instant::now());
            self.open_pathfinding_console();
            self.ask_path();
            return;
        }
        if name.starts_with("rebake") {
            let disabled = name == "rebake-disabled";
            let region = self.world.main_region.unwrap_or_default();
            if let Some(r) = self.world.regions.get_mut(&region) {
                r.dynamic_pathfinding = !disabled;
            }
            let rebake = &mut self.pathfinding.rebake;
            rebake.region_changed(Some(region));
            let state = if disabled { State::Complete } else { State::Pending };
            rebake.on_status(region, demo_status(state), Instant::now());
            rebake.on_agent_state(region, true);
            return;
        }
        let kind = if name.starts_with("characters") {
            Kind::Characters
        } else {
            Kind::Objects
        };
        let beacon = name.ends_with("-beacon");
        let select = if name.ends_with("-selected") || beacon {
            self.world
                .objects
                .iter()
                .filter(|(_, o)| o.parent_id == 0 && !o.is_avatar())
                .map(|(_, o)| o.full_id)
                .take(2)
                .collect()
        } else {
            Vec::new()
        };
        self.open_pathfinding(kind, select);
        match kind {
            Kind::Objects => self.pathfinding.linkset_beacons = beacon,
            Kind::Characters => self.pathfinding.character_beacons = beacon,
        }
    }

    /// The region's answer to a pathfinding request, made up locally.
    pub(super) fn demo_pathfinding_answer(&mut self, command: NetCommand) {
        match command {
            NetCommand::FetchPathfinding {
                request,
                region,
                kind,
                terrain,
            } => {
                let data = self.demo_pathfinding_list(kind);
                let mut result = pathfinding::parse(&data, kind);
                if terrain && let Ok(rows) = &mut result {
                    let ground = llsd_map! { "navmesh_category" => 0, "A" => 100, "B" => 100, "C" => 100, "D" => 100 };
                    if let Ok(row) = pathfinding::parse_terrain(&ground) {
                        rows.insert(0, row);
                    }
                }
                self.pathfinding.list_mut(kind).on_list(request, region, result);
            }
            NetCommand::SetLinksets {
                request,
                region,
                objects,
                terrain,
            } => {
                let list = &self.pathfinding.linksets;
                let edited = |id: Uuid, edit: pathfinding::linkset::Edit| {
                    let mut row = list.row(&id)?.clone();
                    if let pathfinding::Detail::Object { linkset, .. } = &mut row.detail {
                        let phantom = edit.phantom.unwrap_or(linkset.use_.is_phantom());
                        let category = edit.category.unwrap_or(linkset.use_.category());
                        linkset.use_ = pathfinding::linkset::Use::new(phantom, category);
                        for (slot, value) in linkset.walkability.iter_mut().zip(edit.walkability) {
                            if let Some(value) = value {
                                *slot = value;
                            }
                        }
                    }
                    Some(row)
                };
                let mut rows: Vec<_> = objects.into_iter().filter_map(|(id, edit)| edited(id, edit)).collect();
                if let Some(edit) = terrain
                    && let Some(row) = edited(Uuid::nil(), edit)
                {
                    rows.push(row);
                }
                self.pathfinding.linksets.on_update(request, region, Ok(rows));
            }
            _ => {}
        }
    }

    /// RegionObjects / CharacterProperties as the demo region would send them.
    /// The View tab's RegionObjects, answered like the windows' one.
    pub(super) fn demo_overlay_answer(&mut self, command: NetCommand) {
        if let NetCommand::FetchPathfinding { request, region, .. } = command {
            let data = self.demo_pathfinding_list(Kind::Objects);
            let result = pathfinding::parse(&data, Kind::Objects);
            self.pathfinding.overlay_list.on_list(request, region, result);
        }
    }

    fn demo_pathfinding_list(&self, kind: Kind) -> Llsd {
        let mut data = Llsd::new_map();
        let plaza: Vec<_> = self
            .world
            .objects
            .iter()
            .filter(|(_, o)| o.parent_id == 0 && !o.is_avatar())
            // the plaza's objects belong to no one: the demo avatar owns them here
            .map(|(_, o)| {
                let owner = if o.owner_id.is_nil() { self.world.agent_id } else { o.owner_id };
                (o.full_id, o.text.clone(), owner, o.position)
            })
            .collect();
        match kind {
            Kind::Characters => {
                for (n, (id, text, owner, at)) in plaza.iter().take(2).enumerate() {
                    let name = if text.is_empty() {
                        format!("{} {}", aurora_i18n::tr!("common-object"), n + 1)
                    } else {
                        text.clone()
                    };
                    data.insert(
                        id.to_string(),
                        llsd_map! {
                            "name" => name, "description" => "", "owner" => *owner, "owner_is_group" => false,
                            "position" => vector([f64::from(at.x), f64::from(at.y), f64::from(at.z)]),
                            "cpu_time" => 40.0 + 25.0 * n as f64, "length" => 1.5, "radius" => 0.4, "horizontal" => n == 1
                        },
                    );
                }
            }
            Kind::Objects => {
                // every kind of linkset, in turn: (category, phantom, modifiable, can_be_volume)
                const KINDS: [(i32, bool, bool, bool); 6] = [
                    (0, false, true, true),
                    (1, false, true, true),
                    (2, false, true, false),
                    (0, true, true, true),
                    (1, true, false, true),
                    (2, false, false, true),
                ];
                for n in 0..self.pathfinding_demo.linksets {
                    let (category, phantom, modifiable, can_be_volume) = KINDS[n % KINDS.len()];
                    let (id, name, owner, at) = match plaza.get(n) {
                        Some((id, text, owner, at)) => (
                            *id,
                            if text.is_empty() {
                                format!("{} {}", aurora_i18n::tr!("common-object"), n + 1)
                            } else {
                                text.clone()
                            },
                            *owner,
                            [f64::from(at.x), f64::from(at.y), f64::from(at.z)],
                        ),
                        None => (
                            Uuid::from_u128(0xd3_0000 + n as u128),
                            format!("{} {}", aurora_i18n::tr!("common-object"), n + 1),
                            DEMO_OWNER,
                            [(n * 7 % 256) as f64, (n * 13 % 256) as f64, 21.0 + (n % 5) as f64],
                        ),
                    };
                    let walk = |k: usize| ((n * (k + 3) * 17) % 101) as i32;
                    data.insert(
                        id.to_string(),
                        llsd_map! {
                            "name" => name, "description" => if n % 3 == 0 { "" } else { "Demo linkset" }, "owner" => owner,
                            "owner_is_group" => n % 11 == 5, "position" => vector(at), "landimpact" => (n % 9 + 1) as i32,
                            "navmesh_category" => category, "phantom" => phantom, "modifiable" => modifiable,
                            "can_be_volume" => can_be_volume, "is_scripted" => n % 4 == 0,
                            "A" => if n % 2 == 0 { 100 } else { walk(0) }, "B" => walk(1), "C" => walk(2), "D" => walk(3)
                        },
                    );
                }
            }
        }
        data
    }
}

impl App {
    /// The demo region accepts the rebake and builds (navMeshRebakeCoro's
    /// answer, then NavMeshStatusUpdate "building").
    pub(super) fn demo_rebake_answer(&mut self, region: aurora_net::RegionHandle) {
        let now = Instant::now();
        let rebake = &mut self.pathfinding.rebake;
        rebake.on_response(region, true, now);
        rebake.on_status(region, demo_status(State::Building), now);
        self.pathfinding_demo.rebake_done = Some(now + REBAKE_TIME);
    }

    /// The demo's rebake ends: NavMeshStatusUpdate "complete".
    pub(super) fn demo_rebake_frame(&mut self) {
        let now = Instant::now();
        if self.pathfinding_demo.rebake_done.is_some_and(|at| now >= at) {
            self.pathfinding_demo.rebake_done = None;
            if let Some(region) = self.world.main_region {
                self.pathfinding.rebake.on_status(region, demo_status(State::Complete), now);
            }
        }
    }
}

//! Finds the prim water volumes around our avatar (#2008) without looking
//! at every prim each frame. Every two seconds, the phantom prims within
//! 64 m are listed from the scene's pick index; the names of those not
//! known yet are asked once with RequestObjectPropertiesFamily, as a hover
//! does, at most a few a sweep. Each frame only tests the few volumes that
//! matched. Names that reach the viewer another way (hover, build, Area
//! Search) are kept too.
use crate::movement::water_volume::{self, Volume, Water};
use crate::scene::Scene;
use crate::world::World;
use aurora_net::NetCommand;
use aurora_net::build::ObjectProps;
use glam::Vec3;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

const RANGE: f32 = 64.0;
const SWEEP: Duration = Duration::from_secs(2);
/// Names asked per sweep, nearest first: about ten a second at most.
const ASKS: usize = 20;
/// A name that never came back is asked again after this.
const ASK_AGAIN: Duration = Duration::from_secs(60);
/// Floors are probed again when the avatar moved this far or after FLOOR_AGE.
const FLOOR_MOVE: f32 = 0.5;
const FLOOR_AGE: Duration = Duration::from_millis(500);
const FLOOR_DEPTH: f32 = 128.0;

#[derive(Default)]
pub(super) struct Detector {
    /// Name and description of the candidates whose properties arrived.
    names: HashMap<Uuid, (String, String)>,
    asked: HashMap<Uuid, Instant>,
    matched: Vec<Uuid>,
    /// The list as typed, and its words.
    words: (String, Vec<String>),
    next_sweep: Option<Instant>,
    floor: Option<(Vec3, Instant, Option<f32>)>,
    active: bool,
}

impl Detector {
    /// Called every frame; the requests to send. Off, it forgets everything.
    pub(super) fn update(
        &mut self,
        settings: &water_volume::Settings,
        swimming: bool,
        world: &World,
        scene: &Scene,
        now: Instant,
    ) -> Vec<NetCommand> {
        self.active = settings.enabled && swimming;
        if !self.active {
            if self.next_sweep.is_some() {
                *self = Self::default();
            }
            return Vec::new();
        }
        if self.words.0 != settings.words {
            self.words = (settings.words.clone(), water_volume::words(&settings.words));
            self.next_sweep = None;
        }
        if self.next_sweep.is_some_and(|at| now < at) {
            return Vec::new();
        }
        self.next_sweep = Some(now + SWEEP);
        let position = world.agent.position;
        let mut candidates = Vec::new();
        scene.water_candidates(world, position, RANGE, &mut candidates);
        let mut unknown = Vec::new();
        self.matched.clear();
        for idx in candidates {
            let Some(o) = world.objects.get(idx) else { continue };
            let Some(volume) = scene.water_volume(world, idx) else { continue };
            match self.names.get(&o.full_id) {
                Some((name, description)) => {
                    if water_volume::matches(&self.words.1, name, description) {
                        self.matched.push(o.full_id);
                    }
                }
                None if self.asked.get(&o.full_id).is_none_or(|at| now.duration_since(*at) >= ASK_AGAIN) => {
                    unknown.push((volume.center.distance_squared(position), o.full_id, o.key.region));
                }
                None => {}
            }
        }
        unknown.sort_by(|a, b| a.0.total_cmp(&b.0));
        let requests = unknown
            .into_iter()
            .take(ASKS)
            .map(|(_, object, handle)| {
                self.asked.insert(object, now);
                NetCommand::RequestObjectProperties { handle, object }
            })
            .collect();
        self.names.retain(|id, _| world.objects.index_of_uuid(id).is_some());
        self.asked
            .retain(|id, at| world.objects.index_of_uuid(id).is_some() && now.duration_since(*at) < ASK_AGAIN);
        requests
    }

    /// Keep the names of phantom prims (asked or not); a match counts at once.
    pub(super) fn on_properties(&mut self, world: &World, props: &[ObjectProps]) {
        if !self.active {
            return;
        }
        for p in props {
            let phantom = world
                .objects
                .index_of_uuid(&p.object_id)
                .and_then(|idx| world.objects.get(idx))
                .is_some_and(|o| o.update_flags & crate::build::edits::FLAGS_PHANTOM != 0);
            if !phantom && !self.asked.contains_key(&p.object_id) {
                continue;
            }
            self.asked.remove(&p.object_id);
            if phantom && water_volume::matches(&self.words.1, &p.name, &p.description) && !self.matched.contains(&p.object_id) {
                self.matched.push(p.object_id);
            }
            self.names.insert(p.object_id, (p.name.clone(), p.description.clone()));
        }
    }

    /// Names known, names asked, volumes matched (the demo's log).
    pub(super) fn counts(&self) -> (usize, usize, usize) {
        (self.names.len(), self.asked.len(), self.matched.len())
    }

    /// The water of the volume around `position`, if any.
    pub(super) fn water(&mut self, world: &World, scene: &Scene, position: Vec3, height: f32, now: Instant) -> Option<Water> {
        if !self.active || self.matched.is_empty() {
            return None;
        }
        let volumes: Vec<Volume> = self
            .matched
            .iter()
            .filter_map(|id| world.objects.index_of_uuid(id))
            .filter_map(|idx| scene.water_volume(world, idx))
            .collect();
        let Some(top) = volumes.iter().filter(|v| v.over(position)).map(Volume::top).max_by(f32::total_cmp) else {
            self.floor = None;
            return None;
        };
        let stale = self
            .floor
            .is_none_or(|(at, when, _)| at.distance(position) > FLOOR_MOVE || now.duration_since(when) > FLOOR_AGE);
        if stale {
            let origin = position.with_z(position.z.min(top - 0.01));
            let floor = [scene.water_floor(world, origin, FLOOR_DEPTH), world.ground_height(position)]
                .into_iter()
                .flatten()
                .filter(|f| *f <= top)
                .max_by(f32::total_cmp);
            self.floor = Some((position, now, floor));
        }
        water_volume::water_at(&volumes, position, height, self.floor.and_then(|f| f.2))
    }
}

impl super::App {
    /// AURORA_DEMO_PRIM_WATER: the pool, a swimming AO and the avatar in the
    /// water at frame 360, then the movement state in the log each second.
    pub(super) fn prim_water_demo(&mut self) {
        let Some(mode) = self.demo.then(crate::demo::prim_water::mode).flatten() else {
            return;
        };
        if self.frame_count == 360 {
            crate::ao::demo::assets(&mut self.scene);
            let selected = crate::ao::demo::seed_movement_design(&mut self.world.inventory, self.world.agent_id);
            self.settings.ao.accounts.insert(
                self.world.agent_id,
                crate::ao::Account {
                    enabled: true,
                    selected,
                    ..Default::default()
                },
            );
            self.settings.water_volumes = crate::demo::prim_water::settings(&mode);
            for event in crate::demo::prim_water::events(&mode) {
                self.world.apply(event);
            }
            let center = crate::demo::prim_water::CENTER;
            let position = center.with_z(crate::demo::prim_water::surface() - 0.6);
            self.world
                .agent
                .on_server_update_at(position, Vec3::ZERO, Vec3::ZERO, false, true, Instant::now());
            self.world.agent.position = position;
            self.world.agent.flying = true;
            let target = center.with_z(crate::demo::prim_water::surface());
            self.camera.set_view(
                target + Vec3::new(-7.0, -7.0, 4.0),
                target,
                &mut self.world.agent,
                &self.settings.camera,
            );
            self.panels.ao = true;
            self.ao_ui.tab = 1;
            self.panels.perf = false;
            self.panels.inventory = false;
        }
        // The seeded set loads over a few frames; swimming waits for it.
        let selected = self.settings.ao.accounts.get(&self.world.agent_id).map(|a| a.selected);
        if self.frame_count > 360 && self.frame_count < 900 && selected.is_some_and(|s| self.ao_catalog.swimming_ready(s)) {
            self.settings.movements.entry(self.world.agent_id).or_default().swimming = true;
        }
        // No prim collision nor bridge motor offline: a scripted loop at the
        // surface, under water, above the volume (out of it), then back.
        if self.frame_count > 360 {
            let phase = (self.frame_count - 360) % 720;
            let surface = crate::demo::prim_water::surface();
            let z = match phase {
                0..180 => surface - 0.6,
                180..360 => surface - 1.6,
                360..540 => surface + 3.0,
                _ => surface - 0.6,
            };
            self.world.agent.position = crate::demo::prim_water::CENTER.with_z(z);
            self.world.agent.velocity = Vec3::ZERO;
            if phase % 180 == 170 {
                log::info!(
                    target: "aurora_viewer::prim_water",
                    "mode {mode:?}, phase {}: swimming {}, names/asked/matched {:?}, state {:?}, water {:?}, z {:.2}, bridge {}",
                    phase / 180,
                    self.settings.movements.get(&self.world.agent_id).is_some_and(|p| p.swimming),
                    self.water_volumes.counts(),
                    self.movement_runtime.state,
                    self.movement_water,
                    self.world.agent.position.z,
                    self.bridge_runtime.available(),
                );
            }
        }
    }
}

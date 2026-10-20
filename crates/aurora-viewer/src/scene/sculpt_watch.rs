//! Opt-in attachment diagnostics for one avatar, without changing rendering.
//! Identifiers come from the launch environment; never record names, locations,
//! texture contents, credentials or service URLs.

use super::textures::TextureStreamer;
use super::{ObjGpu, Scene};
use crate::world::World;
use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub(super) struct SculptWatch {
    target: Uuid,
    last_poll: Option<Instant>,
    found: Option<bool>,
    previous: BTreeMap<Uuid, String>,
}

impl SculptWatch {
    pub(super) fn from_env() -> Option<Self> {
        let value = std::env::var("AURORA_SCULPT_WATCH").ok()?;
        let target = match Uuid::parse_str(&value).ok().filter(|id| !id.is_nil()) {
            Some(id) => id,
            None => {
                log::warn!("sculpt watch: AURORA_SCULPT_WATCH requires a non-null avatar UUID");
                return None;
            }
        };
        Some(Self::new(target))
    }

    /// The watch of one avatar (also the Dev Toolbox's O9, on its target).
    pub(super) fn new(target: Uuid) -> Self {
        log::info!("sculpt watch: enabled for the configured avatar; attachment states are logged only when changed");
        Self {
            target,
            last_poll: None,
            found: None,
            previous: BTreeMap::new(),
        }
    }

    pub(super) fn target(&self) -> Uuid {
        self.target
    }

    pub(super) fn poll(&mut self, world: &World, gpu: &[ObjGpu], textures: &TextureStreamer, now: Instant) {
        if self
            .last_poll
            .is_some_and(|last| now.duration_since(last) < Duration::from_millis(500))
        {
            return;
        }
        self.last_poll = Some(now);
        let avatar = world
            .objects
            .index_of_uuid(&self.target)
            .filter(|&i| world.objects.get(i).is_some_and(|o| o.is_avatar()));
        let found = avatar.is_some();
        if self.found != Some(found) {
            log::info!("sculpt watch: target avatar {}", if found { "present" } else { "not yet present" });
            self.found = Some(found);
        }
        let mut next = BTreeMap::new();
        let mut todo: Vec<usize> = avatar.into_iter().collect();
        let mut visited = HashSet::new();
        while let Some(idx) = todo.pop() {
            if !visited.insert(idx) {
                continue;
            }
            let Some(o) = world.objects.get(idx) else { continue };
            todo.extend(world.objects.children_of(&o.key));
            if o.is_avatar() || o.pcode != aurora_prim::params::LL_PCODE_VOLUME {
                continue;
            }
            let volume = o.visual_volume();
            let class = if volume.is_mesh() {
                "mesh"
            } else if volume.is_sculpt() {
                "sculpt"
            } else {
                "prim"
            };
            let map = volume.sculpt.map(|s| s.texture);
            let cpu_map = map
                .and_then(|id| textures.sculpt_map(&id))
                .map(|m| (m.width, m.height, m.components, m.pixels.len()));
            let texture = map.map(|id| textures.sculpt_watch_status(&id));
            let bound = gpu.get(idx).and_then(|g| g.geom);
            let wanted = gpu.get(idx).and_then(|g| g.wanted_geom);
            let wait = gpu.get(idx).and_then(|g| g.sculpt_wait);
            let state = format!(
                "object={} local={} parent={} class={class} path={:#04x} profile={:#04x} extra_sculpt={:?} volume_sculpt={:?} visual_sculpt={:?} expected={:?} wanted={wanted:?} bound={bound:?} wait={wait:?} cpu_map={cpu_map:?} texture={texture:?}",
                o.full_id,
                o.key.local_id,
                o.parent_id,
                volume.path.curve_type,
                volume.profile.curve_type,
                o.extra.sculpt,
                o.volume.sculpt,
                volume.sculpt,
                Scene::object_geom_key(o, gpu.get(idx).map_or(3, |g| g.lod)),
            );
            if self.previous.get(&o.full_id) != Some(&state) {
                log::info!("sculpt watch: {state}");
            }
            next.insert(o.full_id, state);
        }
        for id in self.previous.keys().filter(|id| !next.contains_key(id)) {
            log::info!("sculpt watch: object={id} no longer attached to the target");
        }
        self.previous = next;
    }
}

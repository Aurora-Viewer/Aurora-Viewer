//! AURORA_STILL_CHECK: what changed during each second while the picture
//! should stand still (camera locked on an object, nothing touched). One log
//! line per second: the camera, our avatar, the objects whose faces were
//! rebuilt (and why), and the placed objects whose model matrix changed.
//! Tells the cause of a trembling picture apart: view, geometry, placement
//! or avatar. Costs nothing when the variable is not set.

use glam::Vec3;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Examples kept per second and per kind of change.
const EXAMPLES: usize = 6;

/// Why `Scene::sync_object` rebuilt the faces of an object.
#[derive(Debug, Clone, Copy, Default)]
pub struct RebuildWhy {
    pub material: bool,
    pub skeleton: bool,
    pub rigged: bool,
    pub shape: bool,
    pub faces: bool,
    pub geometry: bool,
    pub material_generation: bool,
}

impl RebuildWhy {
    pub fn any(&self) -> bool {
        self.material || self.skeleton || self.rigged || self.shape || self.faces || self.geometry || self.material_generation
    }

    fn names(&self) -> Vec<&'static str> {
        [
            (self.material, "material"),
            (self.skeleton, "skeleton"),
            (self.rigged, "rigged"),
            (self.shape, "shape"),
            (self.faces, "faces"),
            (self.geometry, "geometry"),
            (self.material_generation, "material-generation"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect()
    }
}

/// An object in an example line: what it is, its detail level, and the
/// motion the simulator reported for it.
pub fn describe(o: &crate::world::objects::Object, lod: u8) -> String {
    let kind = if o.volume.is_mesh() {
        "mesh"
    } else if o.is_flexible() {
        "flexible"
    } else if o.volume.is_sculpt() {
        "sculpt"
    } else {
        "prim"
    };
    let link = if o.parent_id == 0 { "root" } else { "child" };
    let mut s = format!("{kind} {link} lod {lod} scale {:.2?}", o.scale.to_array());
    if o.has_motion() {
        s.push_str(&format!(
            ", motion v {:?} a {:?} w {:?}",
            o.velocity.to_array(),
            o.acceleration.to_array(),
            o.angular_velocity.to_array()
        ));
    }
    s
}

/// Changes of a position over the second: in how many frames, how far at most.
#[derive(Default, Clone, Copy)]
struct Moves {
    frames: u32,
    max: f32,
}

impl Moves {
    fn note(&mut self, from: Vec3, to: Vec3) {
        if from != to {
            self.frames += 1;
            self.max = self.max.max(from.distance(to));
        }
    }
}

impl std::fmt::Display for Moves {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} frames (max {:.3} mm)", self.frames, self.max * 1000.0)
    }
}

/// What the app sees of a frame.
#[derive(Clone, Copy)]
pub struct View {
    pub camera: Vec3,
    pub target: Vec3,
    pub fov_y: f32,
    pub agent: Vec3,
}

pub struct StillCheck {
    since: Instant,
    frames: u32,
    last: Option<View>,
    camera: Moves,
    target: Moves,
    fov_changes: u32,
    agent: Moves,
    rebuilds: u32,
    rebuild_reasons: BTreeMap<&'static str, u32>,
    rebuild_examples: Vec<String>,
    placed: u32,
    placed_max: f32,
    placed_examples: Vec<String>,
    worn: u32,
}

impl StillCheck {
    /// The check, when AURORA_STILL_CHECK asks for it.
    pub fn from_env() -> Option<StillCheck> {
        std::env::var_os("AURORA_STILL_CHECK")
            .filter(|v| v != "0")
            .map(|_| StillCheck::start())
    }

    /// The check started now (also the Dev Toolbox's P4).
    pub fn start() -> StillCheck {
        StillCheck {
            since: Instant::now(),
            frames: 0,
            last: None,
            camera: Moves::default(),
            target: Moves::default(),
            fov_changes: 0,
            agent: Moves::default(),
            rebuilds: 0,
            rebuild_reasons: BTreeMap::new(),
            rebuild_examples: Vec::new(),
            placed: 0,
            placed_max: 0.0,
            placed_examples: Vec::new(),
            worn: 0,
        }
    }

    /// The faces of an object were rebuilt. `detail` describes the object
    /// (only formatted for the examples kept).
    pub fn rebuilt(&mut self, id: Uuid, why: RebuildWhy, detail: impl FnOnce() -> String) {
        self.rebuilds += 1;
        let names = why.names();
        for name in &names {
            *self.rebuild_reasons.entry(name).or_default() += 1;
        }
        if self.rebuild_examples.len() < EXAMPLES {
            self.rebuild_examples.push(format!("{id} [{}] {}", names.join("+"), detail()));
        }
    }

    /// The model matrix of an object's faces changed by `distance` (its
    /// translation). `worn`: an attachment, which follows its avatar.
    pub fn moved(&mut self, id: Uuid, worn: bool, distance: f32, detail: impl FnOnce() -> String) {
        if worn {
            self.worn += 1;
            return;
        }
        self.placed += 1;
        self.placed_max = self.placed_max.max(distance);
        if self.placed_examples.len() < EXAMPLES {
            self.placed_examples
                .push(format!("{id} moved {:.3} mm {}", distance * 1000.0, detail()));
        }
    }

    /// Once per frame; writes the second's line when it is over.
    pub fn frame(&mut self, view: View) {
        self.frames += 1;
        if let Some(last) = self.last {
            self.camera.note(last.camera, view.camera);
            self.target.note(last.target, view.target);
            self.agent.note(last.agent, view.agent);
            if last.fov_y != view.fov_y {
                self.fov_changes += 1;
            }
        }
        self.last = Some(view);
        if self.since.elapsed() < Duration::from_secs(1) {
            return;
        }
        let reasons: Vec<String> = self.rebuild_reasons.iter().map(|(name, n)| format!("{name} {n}")).collect();
        log::info!(
            "still check: {} frames | camera moved in {}, target in {}, fov changed in {} frames | agent moved in {} | \
             faces rebuilt {} [{}] | placed objects moved {} (max {:.3} mm), worn {}",
            self.frames,
            self.camera,
            self.target,
            self.fov_changes,
            self.agent,
            self.rebuilds,
            reasons.join(", "),
            self.placed,
            self.placed_max * 1000.0,
            self.worn,
        );
        for e in &self.rebuild_examples {
            log::info!("still check:   rebuilt {e}");
        }
        for e in &self.placed_examples {
            log::info!("still check:   placed {e}");
        }
        self.since = Instant::now();
        self.frames = 0;
        self.camera = Moves::default();
        self.target = Moves::default();
        self.fov_changes = 0;
        self.agent = Moves::default();
        self.rebuilds = 0;
        self.rebuild_reasons.clear();
        self.rebuild_examples.clear();
        self.placed = 0;
        self.placed_max = 0.0;
        self.placed_examples.clear();
        self.worn = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_are_named_in_order() {
        let why = RebuildWhy {
            geometry: true,
            material: true,
            ..Default::default()
        };
        assert_eq!(why.names(), vec!["material", "geometry"]);
        assert!(RebuildWhy::default().names().is_empty());
    }

    #[test]
    fn moves_count_changed_frames_and_keep_the_largest() {
        let mut m = Moves::default();
        m.note(Vec3::ZERO, Vec3::ZERO);
        m.note(Vec3::ZERO, Vec3::X * 0.001);
        m.note(Vec3::X * 0.001, Vec3::X * 0.004);
        assert_eq!(m.frames, 2);
        assert!((m.max - 0.003).abs() < 1e-6);
    }
}

//! The 3D view of Inspect and Area Search: the inspected objects alone, made
//! from what the viewer already holds of them (meshes, prims, sculpts and
//! textures), turned by the user and pulled apart into their items or prims.
//! Aurora's own: the Legacy Viewer has no such view.
//!
//! The appearance and the worn geometry are resolved as the Shape editor
//! resolves them (`shape_editor`). Nothing is asked of the network. The view
//! draws only: it shows no asset id, saves and exports nothing, and is closed
//! wherever RLVa limits what may be seen (`RlvaState::second_camera_blocked`).
//!
//! Every prim owns its bones, rigid ones included, so that moving, pulling
//! apart and hiding are a new palette (`StageModel::pose`): the geometry is
//! uploaded once.

mod build;
pub mod layout;
#[cfg(test)]
mod tests;
pub use build::{Input, StageModel, Standing};

/// A standing as a key: the same to the eye is the same pose.
pub mod build_standing {
    pub type Key = (i32, Option<(usize, i32)>);

    pub fn key(s: super::Standing) -> Key {
        let step = |v: f32| (v * 2000.0).round() as i32;
        (step(s.apart), s.focus.map(|(g, f)| (g, step(f))))
    }
}

use crate::scene::Scene;
use crate::world::World;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// An object to show: its root prim, and its prims by index, the root first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub root: Uuid,
    pub prims: Vec<usize>,
}

/// Where the objects are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Worn by the avatar of that index: shown on a neutral stance.
    Worn(usize),
    /// In the world: side by side.
    World,
}

/// How often what is held of the objects is looked at again (textures and
/// meshes that arrive).
const LOOK_AGAIN: Duration = Duration::from_millis(500);

/// Objects shown at most.
const MOST_OBJECTS: usize = 64;

/// Why there is no 3D view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Absent {
    /// The first model is being made.
    Loading,
    /// Nothing of the objects can be drawn (nothing held, or HUDs).
    Nothing,
    /// RLVa limits what may be seen.
    Refused,
}

/// The models of one window: asked for, made on the background pool, kept.
pub struct Stage {
    tx: mpsc::Sender<(u64, Option<StageModel>)>,
    rx: mpsc::Receiver<(u64, Option<StageModel>)>,
    /// What the shown model was made from, and the one under way.
    key: u64,
    asked: Option<u64>,
    looked: Option<Instant>,
    /// The groups last asked for: other objects never show the old model.
    groups: Vec<Uuid>,
    model: Option<Arc<StageModel>>,
    /// Counts the models: what was made for one is not used for the next.
    serial: u64,
    absent: Absent,
}

impl Default for Stage {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Stage {
            tx,
            rx,
            key: 0,
            asked: None,
            looked: None,
            groups: Vec::new(),
            model: None,
            serial: 0,
            absent: Absent::Loading,
        }
    }
}

impl Stage {
    /// The model to draw, or why there is none.
    pub fn model(&self) -> Result<&Arc<StageModel>, Absent> {
        self.model.as_ref().ok_or(self.absent)
    }

    /// Which model it is.
    pub fn serial(&self) -> u64 {
        self.serial
    }

    /// Forget everything (the window closed): the model is freed off the
    /// main thread.
    pub fn clear(&mut self, scene: &Scene) {
        if let Some(model) = self.model.take() {
            scene.jobs.discard(model);
        }
        self.key = 0;
        self.asked = None;
        self.looked = None;
        self.groups.clear();
        self.absent = Absent::Loading;
    }

    /// Keep the model of the objects `roots` up to date while the window shows
    /// it; `group` tells their prims, asked only when they are looked at.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        world: &World,
        scene: &Scene,
        roots: &[Uuid],
        group: impl Fn(&Uuid) -> Option<Group>,
        place: Place,
        ctx: &egui::Context,
        now: Instant,
    ) {
        if world.rlva.second_camera_blocked() {
            if self.model.is_some() || self.absent != Absent::Refused {
                self.clear(scene);
                self.absent = Absent::Refused;
            }
            return;
        }
        if self.absent == Absent::Refused {
            self.absent = Absent::Loading;
        }
        while let Ok((key, model)) = self.rx.try_recv() {
            if self.asked != Some(key) {
                // other objects were asked for since
                if let Some(model) = model {
                    scene.jobs.discard(model);
                }
                continue;
            }
            self.asked = None;
            self.key = key;
            self.absent = Absent::Nothing;
            self.serial += 1;
            if let Some(old) = std::mem::replace(&mut self.model, model.map(Arc::new)) {
                scene.jobs.discard(old);
            }
        }
        // (more than these would not be told apart)
        let roots = &roots[..roots.len().min(MOST_OBJECTS)];
        if self.groups != roots {
            self.clear(scene);
            self.groups = roots.to_vec();
        }
        if roots.is_empty() {
            self.absent = Absent::Nothing;
            return;
        }
        if self.asked.is_some() || self.looked.is_some_and(|at| now.duration_since(at) < LOOK_AGAIN) {
            ctx.request_repaint_after(LOOK_AGAIN);
            return;
        }
        self.looked = Some(now);
        ctx.request_repaint_after(LOOK_AGAIN);
        let groups: Vec<Group> = roots.iter().filter_map(group).collect();
        let input = Input::capture(world, scene, &groups, place);
        if input.key == self.key {
            return;
        }
        let (key, tx, ctx) = (input.key, self.tx.clone(), ctx.clone());
        let (lib, previous) = (world.avatar_lib.clone(), self.model.clone());
        self.asked = Some(key);
        scene.jobs.run(move || {
            let model = crate::scene::jobs::caught(|| input.prepare(&lib, previous.as_deref())).flatten();
            let _ = tx.send((key, model));
            ctx.request_repaint();
        });
    }
}

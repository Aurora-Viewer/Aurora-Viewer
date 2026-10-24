//! Native animation creation shares IK Edit's pose tools, while keeping its
//! runtime associations separate. Expensive encoding and file dialogs are jobs.
use super::*;
use crate::scene::{anim::BoundAnim, jobs::Jobs};
use aurora_assets::Animation;
use std::{collections::HashSet, sync::Arc};
pub(super) mod document;
pub(super) mod files;
#[cfg(test)]
mod tests;
mod ui;
use document::{Document, FPS, LAST};
pub use files::Prepared;

#[derive(Clone, Copy)]
enum Capture {
    Bone(usize),
    Modified,
    Pose,
}
struct Preview {
    revision: u64,
    start: u32,
    end: u32,
    bound: Arc<BoundAnim>,
}
pub(super) struct State {
    pub active: bool,
    rig: Option<Arc<Rig>>,
    document: Option<Arc<Document>>,
    history: Vec<Arc<Document>>,
    hold_source: bool,
    frame: u32,
    cursor: f64,
    playing: bool,
    at: Option<Instant>,
    revision: u64,
    preview_revision: Option<u64>,
    building: bool,
    preview: Option<Preview>,
    capture: Option<Capture>,
    modified: HashSet<usize>,
    translations: HashSet<usize>,
    clear_after: Option<u64>,
    clear_bones: Vec<usize>,
    saved_goals: Vec<Goal>,
    saved_authoring: authoring::State,
    saved_disabled: Vec<usize>,
    tab: u8,
    export: Option<files::Action>,
    busy: bool,
    error: Option<String>,
    message: Option<String>,
    tx: crossbeam_channel::Sender<files::Output>,
    rx: crossbeam_channel::Receiver<files::Output>,
    pub prepared: Vec<Prepared>,
}
impl Default for State {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            active: false,
            rig: None,
            document: None,
            history: Vec::new(),
            hold_source: false,
            frame: 0,
            cursor: 0.0,
            playing: false,
            at: None,
            revision: 0,
            preview_revision: None,
            building: false,
            preview: None,
            capture: None,
            modified: HashSet::new(),
            translations: HashSet::new(),
            clear_after: None,
            clear_bones: Vec::new(),
            saved_goals: Vec::new(),
            saved_authoring: authoring::State::default(),
            saved_disabled: Vec::new(),
            tab: 0,
            export: None,
            busy: false,
            error: None,
            message: None,
            tx,
            rx,
            prepared: Vec::new(),
        }
    }
}
impl State {
    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.error = None;
        self.message = None;
        if self.clear_after.is_some() {
            self.clear_after = Some(self.revision);
        }
    }
    fn seek(&mut self, frame: u32) {
        self.frame = frame.min(LAST);
        self.cursor = f64::from(self.frame);
        self.playing = false;
        self.capture = None;
        self.clear_after = None;
        self.modified.clear();
        self.translations.clear();
    }
    pub(super) fn mark(&mut self, goal: &Goal) {
        self.playing = false;
        self.clear_bones.retain(|b| *b != goal.bone);
        if goal.mode == Mode::Direct {
            self.modified.insert(goal.bone);
            self.translations.insert(goal.bone);
        } else if let Some(path) = self.rig.as_ref().and_then(|r| ik::chain(r, goal.bone, goal.root)) {
            self.modified.extend(path);
        } else {
            self.modified.insert(goal.bone);
        }
    }
    pub(super) fn waiting_source(&self) -> bool {
        self.active && self.hold_source && self.preview_revision != Some(self.revision)
    }
    pub fn sample(&self) -> Vec<ik::animations::Sample> {
        if !self.active || self.waiting_source() {
            return Vec::new();
        }
        self.preview.as_ref().map_or_else(Vec::new, |p| {
            vec![ik::animations::Sample {
                animation: p.bound.clone(),
                frame: self.frame.clamp(p.start, p.end) - p.start,
            }]
        })
    }
    fn tick(&mut self, jobs: &Jobs, ctx: &egui::Context, rig: Arc<Rig>) -> Vec<usize> {
        let mut clear = Vec::new();
        while let Ok(output) = self.rx.try_recv() {
            match output {
                files::Output::Preview {
                    revision,
                    start,
                    end,
                    result,
                } => {
                    self.building = false;
                    if revision == self.revision {
                        self.preview_revision = Some(revision);
                        match result {
                            Ok(bound) => {
                                self.hold_source = false;
                                self.preview = Some(Preview {
                                    revision,
                                    start,
                                    end,
                                    bound,
                                });
                                if self.clear_after == Some(revision) {
                                    clear = std::mem::take(&mut self.clear_bones);
                                    self.clear_after = None;
                                    self.modified.retain(|b| !clear.contains(b));
                                    self.translations.retain(|b| !clear.contains(b));
                                }
                            }
                            Err(key) => self.error = Some(aurora_i18n::text(key).to_owned()),
                        }
                    }
                }
                files::Output::Prepared(value) => {
                    self.busy = false;
                    self.prepared.push(value);
                }
                files::Output::Saved(path) => {
                    self.busy = false;
                    self.message = Some(aurora_i18n::tr!("anim-edit-saved", path = path.display().to_string()).to_string());
                }
                files::Output::Cancelled => self.busy = false,
                files::Output::Failed(error) => {
                    self.busy = false;
                    self.error = Some(error);
                }
            }
        }
        if let Some(document) = &self.document {
            if !self.building && self.preview_revision != Some(self.revision) {
                self.building = true;
                let revision = self.revision;
                let document = document.clone();
                let tx = self.tx.clone();
                let ctx = ctx.clone();
                jobs.execute(move || {
                    let result = document.preview().map(|a| Arc::new(BoundAnim::bind(a, &rig)));
                    let _ = tx.send(files::Output::Preview {
                        revision,
                        start: document.start,
                        end: document.end,
                        result,
                    });
                    ctx.request_repaint();
                });
            }
            if let Some(action) = self.export.take() {
                files::start(jobs, ctx, self.tx.clone(), document.clone(), action);
            }
            let now = Instant::now();
            let dt = self.at.map_or(0.0, |at| now.saturating_duration_since(at).as_secs_f64().min(0.1));
            self.at = Some(now);
            if self.active && self.playing {
                self.cursor += dt * f64::from(FPS);
                let end = f64::from(document.end);
                if document.meta.looping {
                    let a = f64::from(document.start) + f64::from(document.meta.loop_in) * f64::from(FPS);
                    let b = f64::from(document.start) + f64::from(document.meta.loop_out) * f64::from(FPS);
                    if b > a && self.cursor >= b {
                        self.cursor = a + (self.cursor - a).rem_euclid(b - a);
                    }
                } else if self.cursor >= end {
                    self.cursor = end;
                    self.playing = false;
                }
                self.frame = (self.cursor.round() as u32).clamp(document.start, document.end);
            }
        }
        clear
    }
}
impl Editor {
    pub(super) fn clip_demo(&mut self, world: &World, frame: &Frame, mode: &str) {
        self.enter_clip(world, frame);
        let rig = &world.avatar_lib.rig;
        let head = rig.names["mHead"];
        let pelvis = rig.pelvis;
        if let Some(document) = &mut self.clip.document {
            let document = Arc::make_mut(document);
            for (at, angle, height) in [(30, 0.7, 0.06), (60, -0.4, 0.0)] {
                let mut sample = Frame {
                    model: frame.model,
                    joints: frame.joints.clone(),
                    at: frame.at,
                };
                let (scale, q, p) = sample.joints[head].to_scale_rotation_translation();
                let model_q = frame.model.to_scale_rotation_translation().1;
                sample.joints[head] =
                    Mat4::from_scale_rotation_translation(scale, model_q * Quat::from_rotation_z(angle) * model_q.inverse() * q, p);
                sample.joints[pelvis].w_axis += frame.model.transform_vector3(Vec3::Z * height).extend(0.0);
                let _ = document.record(rig, &sample, at, &[head, pelvis], &[]);
            }
            for track in &mut document.tracks {
                if track.bone == head {
                    track.motion.priority = 5;
                }
                if track.bone == pelvis {
                    track.motion.priority = 4;
                }
            }
            document.meta.loop_in = 0.2;
            document.meta.loop_out = 1.8;
            document.meta.ease_in = 0.2;
            document.meta.ease_out = 0.3;
            if mode == "clip-crop" {
                document.range(15, 45);
            }
        }
        self.clip.changed();
        self.clip.frame = if mode == "clip-frame" { 45 } else { 15 };
        self.clip.cursor = f64::from(self.clip.frame);
        self.clip.tab = if mode == "clip-priority" {
            1
        } else if ["clip-settings", "clip-crop"].contains(&mode) {
            2
        } else {
            0
        };
        self.select(head, rig, frame);
        self.tab = 2;
        self.visibility.body();
        if mode == "clip-import" {
            self.clip.export = Some(files::Action::Import);
            self.clip.busy = true;
        }
    }
    pub fn take_clip_imports(&mut self) -> Vec<Prepared> {
        let prepared = std::mem::take(&mut self.clip.prepared);
        if !prepared.is_empty() {
            self.poser.active = false;
            self.leave_clip();
            self.open = false;
            self.cancel_link_pick();
        }
        prepared
    }
    pub fn tick_clip(&mut self, jobs: &Jobs, ctx: &egui::Context, rig: Arc<Rig>) {
        let clear = self.clip.tick(jobs, ctx, rig);
        if self.clip.active && !clear.is_empty() {
            let baked = self.authoring.baked(&clear);
            self.goals.retain(|g| !clear.contains(&g.bone) && !baked.contains(&g.bone));
            self.refresh_clip_marks();
            self.refresh = true;
        }
    }
    fn new_clip(&mut self, world: &World, frame: &Frame) {
        if self.clip.history.len() >= 15 {
            return;
        }
        let mut document = Document::new();
        if let Err(key) = document.record(
            &world.avatar_lib.rig,
            frame,
            0,
            &(0..world.avatar_lib.rig.len()).collect::<Vec<_>>(),
            &[],
        ) {
            self.clip.error = Some(aurora_i18n::text(key).to_owned());
            return;
        }
        if let Some(old) = self.clip.document.replace(Arc::new(document)) {
            self.clip.history.push(old);
        }
        self.clip.seek(0);
        self.clip.changed();
        self.enter_clip(world, frame);
    }
    fn capture_ik_clip(&mut self, world: &World, frame: &Frame) {
        let Some(document) = self.clip.document.as_mut() else { return };
        let translations: Vec<_> = self.goals.iter().filter(|g| g.mode == Mode::Direct).map(|g| g.bone).collect();
        let bones: Vec<_> = (0..world.avatar_lib.rig.len()).collect();
        match Arc::make_mut(document).record(&world.avatar_lib.rig, frame, self.clip.frame, &bones, &translations) {
            Ok(()) => {
                self.clip.changed();
                self.enter_clip(world, frame);
            }
            Err(key) => self.clip.error = Some(aurora_i18n::text(key).to_owned()),
        }
    }
    fn enter_clip(&mut self, world: &World, frame: &Frame) {
        let rig = &world.avatar_lib.rig;
        self.clip.rig = Some(rig.clone());
        if self.clip.document.is_none() {
            let mut document = Document::new();
            let bones: Vec<_> = (0..rig.len()).collect();
            if let Err(key) = document.record(rig, frame, 0, &bones, &[]) {
                self.clip.error = Some(aurora_i18n::text(key).to_owned());
                return;
            }
            self.clip.document = Some(Arc::new(document));
            self.clip.changed();
        }
        let previous = self.request();
        self.clip.saved_goals = self.goals.clone();
        self.clip.saved_authoring = self.authoring.clone();
        self.clip.saved_disabled = self.disabled.clone();
        self.goals = previous.goals;
        self.goals
            .extend(self.clip.saved_goals.iter().filter(|g| self.authoring.owns(g.bone)).cloned());
        self.clip.hold_source = true;
        self.clip.active = true;
        self.clip.playing = false;
        self.clip.modified.clear();
        self.clip.translations.clear();
        self.clip.clear_after = Some(self.clip.revision);
        self.clip.clear_bones = (0..rig.len()).collect();
        if self.clip.preview.as_ref().is_some_and(|p| p.revision == self.clip.revision) {
            self.goals.clear();
            self.authoring = authoring::State::default();
            self.clip.clear_after = None;
            self.clip.hold_source = false;
        }
        self.goals.retain(|g| !self.disabled.contains(&g.bone));
        self.disabled.clear();
        self.graph.selected = None;
        self.graph.pause();
        self.cancel_link_pick();
        self.drag = None;
        self.gizmo.cancel();
        self.weight = 1.0;
        self.tab = 2;
        self.refresh = true;
    }
    pub(super) fn leave_clip(&mut self) {
        if !self.clip.active {
            return;
        }
        self.clip.active = false;
        self.clip.playing = false;
        self.clip.capture = None;
        self.clip.clear_after = None;
        self.clip.clear_bones.clear();
        self.goals = std::mem::take(&mut self.clip.saved_goals);
        self.authoring = std::mem::take(&mut self.clip.saved_authoring);
        self.disabled = std::mem::take(&mut self.clip.saved_disabled);
        self.drag = None;
        self.gizmo.cancel();
        self.refresh = true;
        self.tab = 2;
    }
    pub(super) fn capture_clip(&mut self, world: &World, frame: &Frame) {
        let Some(capture) = self.clip.capture.take() else { return };
        if !self.clip.active {
            return;
        }
        let rig = &world.avatar_lib.rig;
        let bones: Vec<_> = match capture {
            Capture::Bone(b) => {
                let mut selected = vec![b];
                if self.symmetry
                    && let Some(other) = self.mirrors.counterpart(b)
                {
                    selected.push(other);
                }
                let mut result = Vec::new();
                for bone in selected {
                    if self.authoring.owns(bone) {
                        result.extend(self.authoring.bones(bone));
                    } else if let Some(goal) = self.goals.iter().find(|g| g.bone == bone && g.mode == Mode::Ik)
                        && let Some(chain) = ik::chain(rig, bone, goal.root)
                    {
                        result.extend(chain);
                    } else {
                        result.push(bone);
                    }
                }
                result
            }
            Capture::Modified => self.clip.modified.iter().copied().collect(),
            Capture::Pose => (0..rig.len()).collect(),
        };
        let translations: Vec<_> = self.clip.translations.iter().copied().collect();
        let Some(document) = self.clip.document.as_mut() else { return };
        match Arc::make_mut(document).record(rig, frame, self.clip.frame, &bones, &translations) {
            Ok(()) => {
                self.clip.changed();
                self.clip.clear_after = Some(self.clip.revision);
                self.clip.clear_bones = bones;
            }
            Err(key) => self.clip.error = Some(aurora_i18n::text(key).to_owned()),
        }
    }
    pub(super) fn sync_clip_selection(&mut self, frame: &Frame) {
        if !self.clip.active || self.selected.is_some_and(|b| self.goals.iter().any(|g| g.bone == b)) {
            return;
        }
        if let Some(bone) = self.selected
            && let Some(matrix) = frame.joints.get(bone)
        {
            let local = frame.model.inverse() * *matrix;
            self.position = frame.model.inverse().transform_point3(matrix.transform_point3(self.contact));
            self.set_rotation(local.to_scale_rotation_translation().1);
        }
    }
    pub(super) fn refresh_clip_marks(&mut self) {
        if !self.clip.active {
            return;
        }
        self.clip.modified.clear();
        self.clip.translations.clear();
        for goal in &self.goals {
            if self.authoring.owns(goal.bone) {
                for joint in self.authoring.request(std::slice::from_ref(goal), &self.disabled) {
                    self.clip.modified.insert(joint.bone);
                    if joint.translation.is_some() {
                        self.clip.translations.insert(joint.bone);
                    }
                }
            } else {
                self.clip.mark(goal);
            }
        }
    }
}

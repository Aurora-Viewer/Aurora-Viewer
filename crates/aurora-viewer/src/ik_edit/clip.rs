//! Native animation creation shares IK Edit's pose tools, while keeping its
//! runtime associations separate. Expensive encoding and file dialogs are jobs.
use super::*;
use crate::scene::{anim::BoundAnim, jobs::Jobs};
use aurora_assets::Animation;
use std::{collections::HashSet, sync::Arc};
pub(super) mod document;
pub(super) mod files;
#[cfg(test)]
mod key_tests;
#[cfg(test)]
mod tests;
mod timeline;
mod ui;
use document::{Document, FPS, MAX_FRAME};
pub use files::Prepared;

#[derive(Clone, Copy)]
pub(super) enum Capture {
    Bone(usize),
    Modified,
    AutoKeyModified,
    Pose,
}
struct Preview {
    revision: u64,
    start: u32,
    end: u32,
    bound: Arc<BoundAnim>,
}
pub(super) struct TimelineTrack {
    pub bone: usize,
    pub name: String,
    pub enabled: bool,
    pub authored: bool,
    pub keys: Vec<u32>,
}

fn wave_demo_bones(rig: &Rig) -> Option<[usize; 8]> {
    Some([
        *rig.names.get("mShoulderRight")?,
        *rig.names.get("mElbowRight")?,
        *rig.names.get("mWristRight")?,
        *rig.names.get("mHandIndex1Right")?,
        *rig.names.get("mHandMiddle1Right")?,
        *rig.names.get("mHandRing1Right")?,
        *rig.names.get("mHandPinky1Right")?,
        *rig.names.get("mHandThumb1Right")?,
    ])
}

pub(super) struct State {
    pub active: bool,
    rig: Option<Arc<Rig>>,
    pub(super) document: Option<Arc<Document>>,
    history: Vec<Arc<Document>>,
    undo: Vec<Arc<Document>>,
    redo: Vec<Arc<Document>>,
    clipboard: Vec<document::Keyframe>,
    drag_key: Option<(usize, u32, u32)>,
    timeline_zoom: f32,
    timeline_start: u32,
    pub(super) auto_key_pending: bool,
    pub(super) auto_key_retime_from: Option<f32>,
    pub(super) timeline_looping: bool,
    pub(super) timeline_loop_last_key: bool,
    pub(super) timeline_open: bool,
    auto_key_advance_to: Option<u32>,
    hold_source: bool,
    frame: u32,
    cursor: f64,
    playing: bool,
    at: Option<Instant>,
    revision: u64,
    timeline_tracks: Option<(u64, Arc<Vec<TimelineTrack>>)>,
    preview_revision: Option<u64>,
    selection_sync: Option<(usize, u64, u64, Option<u64>)>,
    building: bool,
    preview: Option<Preview>,
    pub(super) capture: Option<Capture>,
    pub(super) modified: HashSet<usize>,
    pub(super) translations: HashSet<usize>,
    clear_after: Option<u64>,
    clear_bones: Vec<usize>,
    saved_goals: Vec<Goal>,
    saved_authoring: authoring::State,
    saved_disabled: Vec<usize>,
    saved_pose_transform: Option<usize>,
    tab: u8,
    export: Option<files::Action>,
    busy: bool,
    error: Option<String>,
    message: Option<String>,
    tx: crossbeam_channel::Sender<files::Output>,
    rx: crossbeam_channel::Receiver<files::Output>,
    pub prepared: Vec<Prepared>,
    from_avatar_studio: bool,
    pub(super) return_after_upload: bool,
}
impl Default for State {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            active: false,
            rig: None,
            document: None,
            history: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            clipboard: Vec::new(),
            drag_key: None,
            timeline_zoom: 1.0,
            timeline_start: 0,
            auto_key_pending: false,
            auto_key_retime_from: None,
            timeline_looping: false,
            timeline_loop_last_key: false,
            timeline_open: true,
            auto_key_advance_to: None,
            hold_source: false,
            frame: 0,
            cursor: 0.0,
            playing: false,
            at: None,
            revision: 0,
            timeline_tracks: None,
            preview_revision: None,
            selection_sync: None,
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
            saved_pose_transform: None,
            tab: 0,
            export: None,
            busy: false,
            error: None,
            message: None,
            tx,
            rx,
            prepared: Vec::new(),
            from_avatar_studio: false,
            return_after_upload: false,
        }
    }
}
impl State {
    pub(super) fn is_playing(&self) -> bool {
        self.playing
    }

    pub(super) fn restrict_pose_transform(&mut self, transform: &mut usize) -> bool {
        if self.active && *transform > 1 {
            self.saved_pose_transform.get_or_insert(*transform);
            *transform = 0;
            true
        } else {
            false
        }
    }
    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.error = None;
        self.message = None;
        if self.clear_after.is_some() {
            self.clear_after = Some(self.revision);
        }
    }
    pub(super) fn timeline_tracks(&mut self) -> Option<Arc<Vec<TimelineTrack>>> {
        if let Some((revision, tracks)) = &self.timeline_tracks
            && *revision == self.revision
        {
            return Some(tracks.clone());
        }
        let document = self.document.as_ref()?;
        let fps = document.fps as f32;
        let tracks: Vec<TimelineTrack> = document
            .tracks
            .iter()
            .map(|track| {
                let mut keys: Vec<_> = track
                    .motion
                    .rot_keys
                    .iter()
                    .map(|key| (key.time * fps).round() as u32)
                    .chain(track.motion.pos_keys.iter().map(|key| (key.time * fps).round() as u32))
                    .collect();
                keys.sort_unstable();
                keys.dedup();
                TimelineTrack {
                    bone: track.bone,
                    name: track.motion.joint_name.clone(),
                    enabled: track.enabled,
                    authored: track.authored,
                    keys,
                }
            })
            .collect();
        let tracks = Arc::new(tracks);
        self.timeline_tracks = Some((self.revision, tracks.clone()));
        Some(tracks)
    }
    pub(super) fn has_key(&mut self, bone: Option<usize>, frame: u32) -> bool {
        self.timeline_tracks().is_some_and(|tracks| {
            tracks
                .iter()
                .any(|track| bone.is_none_or(|bone| track.bone == bone) && track.keys.binary_search(&frame).is_ok())
        })
    }
    fn checkpoint(&mut self) {
        if let Some(document) = self.document.as_ref() {
            self.undo.push(document.clone());
            if self.undo.len() > 100 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }
    fn clear_undo_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
    fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else { return false };
        if let Some(current) = self.document.replace(previous) {
            self.redo.push(current);
        }
        self.changed();
        true
    }
    fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        if let Some(current) = self.document.replace(next) {
            self.undo.push(current);
        }
        self.changed();
        true
    }
    fn seek(&mut self, frame: u32) {
        self.frame = frame.min(MAX_FRAME);
        self.cursor = f64::from(self.frame);
        self.playing = false;
        self.capture = None;
        self.auto_key_pending = false;
        self.auto_key_advance_to = None;
        self.clear_after = None;
        self.modified.clear();
        self.translations.clear();
        self.selection_sync = None;
    }
    pub(super) fn finish_auto_key_on_release(&mut self, pointer_down: bool) {
        if self.auto_key_pending && !pointer_down {
            self.auto_key_pending = false;
            if !self.modified.is_empty() {
                self.capture = Some(Capture::AutoKeyModified);
            }
        }
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
        self.auto_key_pending |= self.document.as_ref().is_some_and(|document| document.auto_keyframe);
    }
    pub(super) fn waiting_source(&self) -> bool {
        self.active && self.hold_source && self.preview_revision != Some(self.revision)
    }
    pub fn sample(&self) -> Vec<ik::animations::Sample> {
        if !self.active || self.waiting_source() {
            return Vec::new();
        }
        self.preview.as_ref().map_or_else(Vec::new, |p| {
            let timeline_fps = self.document.as_ref().map_or(FPS, |document| document.fps as f32);
            let playhead = self.cursor.clamp(f64::from(p.start), f64::from(p.end));
            let time_seconds = ((playhead - f64::from(p.start)) / f64::from(timeline_fps)) as f32;
            vec![ik::animations::Sample {
                animation: p.bound.clone(),
                frame: (time_seconds * ik::animations::FPS).round() as u32,
                time_seconds: Some(time_seconds),
            }]
        })
    }
    fn tick(&mut self, jobs: &Jobs, ctx: &egui::Context, rig: Arc<Rig>, skeleton: Option<&aurora_assets::Skeleton>) -> Vec<usize> {
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
                                    if let Some(frame) = self.auto_key_advance_to.take() {
                                        self.frame = frame;
                                        self.cursor = f64::from(frame);
                                    }
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
                files::Output::Loaded(document) => {
                    self.busy = false;
                    if let Some(current) = self.document.replace(Arc::new(document))
                        && self.history.len() < 15
                    {
                        self.history.push(current);
                    }
                    let start = self.document.as_ref().map_or(0, |document| document.start);
                    self.frame = start;
                    self.cursor = f64::from(start);
                    self.playing = false;
                    self.clear_undo_history();
                    self.changed();
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
                let preview_rig = rig.clone();
                jobs.execute(move || {
                    let result = document.preview().map(|a| Arc::new(BoundAnim::bind(a, &preview_rig)));
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
                let import_skeleton = matches!(action, files::Action::Load)
                    .then(|| skeleton.cloned().map(Arc::new))
                    .flatten();
                files::start(jobs, ctx, self.tx.clone(), document.clone(), action, rig.clone(), import_skeleton);
            }
            let now = Instant::now();
            let dt = self.at.map_or(0.0, |at| now.saturating_duration_since(at).as_secs_f64());
            self.at = Some(now);
            if self.active && self.playing {
                (self.cursor, self.playing) =
                    advance_playhead(self.cursor, dt, document, self.timeline_looping, self.timeline_loop_last_key);
                self.frame = (self.cursor.round() as u32).clamp(document.start, document.end);
            }
        }
        clear
    }
}

fn advance_playhead(
    mut cursor: f64,
    elapsed: f64,
    document: &Document,
    timeline_looping: bool,
    timeline_loop_last_key: bool,
) -> (f64, bool) {
    cursor += elapsed * f64::from(document.fps);
    if timeline_looping {
        let start = f64::from(document.start);
        let loop_end = if timeline_loop_last_key {
            document
                .last_authored_keyframe()
                .map_or(document.end, |last| last.saturating_add(1))
        } else {
            document.end
        };
        let end = f64::from(loop_end).max(start + 1.0);
        if end > start && cursor >= end {
            cursor = start + (cursor - end).rem_euclid(end - start);
        }
        (cursor, true)
    } else if cursor >= f64::from(document.end) {
        (f64::from(document.end), false)
    } else {
        (cursor, true)
    }
}

fn next_auto_key_frame(document: &mut Document, frame: u32) -> Option<u32> {
    let step = (document.auto_key_advance_seconds * document.fps as f32).round().max(1.0) as u32;
    let next = frame.saturating_add(step).min(document.max_frame());
    if next <= frame {
        return None;
    }
    if next > document.end {
        document.range(document.start, next);
    }
    Some(next)
}

impl Editor {
    pub(in crate::ik_edit::clip) fn queue_studio_file_action(&mut self, action: files::Action) {
        self.clip.export = Some(action);
        self.clip.busy = true;
        self.clip.error = None;
        self.clip.message = None;
    }

    pub(super) fn studio_clip_undo(&mut self) {
        if self.clip.undo() {
            let frame = self.clip.frame;
            self.clip.seek(frame);
            self.goals.clear();
            self.authoring = authoring::State::default();
        }
    }

    pub(super) fn studio_toggle_clip_playback(&mut self) {
        let Some(document) = self.clip.document.as_ref() else { return };
        let (start, end) = (document.start, document.end);
        self.goals.clear();
        self.authoring = authoring::State::default();
        self.clip.modified.clear();
        self.clip.translations.clear();
        self.clip.clear_after = None;
        self.clip.auto_key_pending = false;
        self.clip.auto_key_advance_to = None;
        self.clip.playing = !self.clip.playing;
        if self.clip.playing && self.clip.cursor >= f64::from(end) {
            self.clip.frame = start;
            self.clip.cursor = f64::from(start);
        }
        self.refresh = true;
    }

    pub(super) fn reset_animation_from_studio(&mut self, world: &World) {
        let rig = &world.avatar_lib.rig;
        let Some(current) = Frame::new(world) else { return };
        let mut document = Document::new();
        let base = Frame {
            model: current.model,
            joints: rig
                .world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale)
                .iter()
                .map(|joint| current.model * *joint)
                .collect(),
            at: current.at,
        };
        let bones: Vec<_> = (0..rig.len()).collect();
        if let Err(key) = document.record(rig, &base, 0, &bones, &[]) {
            self.clip.error = Some(aurora_i18n::text(key).to_owned());
            return;
        }
        self.clip.checkpoint();
        self.clip.document = Some(Arc::new(document));
        self.clip.auto_key_retime_from = None;
        self.clip.seek(0);
        self.clip.playing = false;
        self.clip.changed();
    }

    pub(super) fn studio_clip_redo(&mut self) {
        if self.clip.redo() {
            let frame = self.clip.frame;
            self.clip.seek(frame);
            self.goals.clear();
            self.authoring = authoring::State::default();
        }
    }

    pub(super) fn studio_copy_clip_keyframe(&mut self) {
        if let Some(document) = &self.clip.document {
            self.clip.clipboard = document.copy_keyframe(self.clip.frame);
        }
    }

    pub(super) fn studio_paste_clip_keyframe(&mut self) {
        if self.clip.clipboard.is_empty() {
            return;
        }
        self.clip.checkpoint();
        if let Some(document) = self.clip.document.as_mut() {
            Arc::make_mut(document).paste_keyframe(self.clip.frame, &self.clip.clipboard);
            self.clip.changed();
        }
    }

    pub(super) fn studio_delete_clip_keyframe(&mut self) {
        self.clip.checkpoint();
        if let Some(document) = self.clip.document.as_mut() {
            Arc::make_mut(document).remove_keyframe(self.clip.frame);
            self.clip.changed();
        }
    }

    pub(super) fn studio_delete_selected_clip_bone_keyframe(&mut self) {
        let Some(bone) = self.selected else { return };
        self.clip.checkpoint();
        if let Some(document) = self.clip.document.as_mut() {
            Arc::make_mut(document).remove_track_keyframe(bone, self.clip.frame);
            self.clip.changed();
        }
    }

    pub(super) fn open_from_avatar_studio(&mut self, world: &World) {
        self.open = true;
        self.tab = 2;
        self.clip.tab = 1;
        self.clip.from_avatar_studio = true;
        self.clip.timeline_open = true;
        // The clip editor has its own keyed pose controls. Leaving the Poser
        // session active makes Editor::show return through show_poser before
        // clip capture runs, so manual and automatic keying silently miss edits.
        self.poser.active = false;
        if self.clip.active {
            return;
        }
        if let Some(frame) = Frame::new(world) {
            if self.clip.document.is_some() {
                self.enter_clip(world, &frame);
            } else {
                self.new_clip(world, &frame);
            }
        }
    }

    pub(super) fn wave_demo_from_avatar_studio(&mut self, world: &World) {
        let Some(frame) = Frame::new(world) else { return };
        self.open_from_avatar_studio(world);
        let rig = &world.avatar_lib.rig;
        let Some([shoulder, elbow, wrist, index, middle, ring, pinky, thumb]) = wave_demo_bones(rig) else {
            return;
        };
        let model_rotation = frame.model.to_scale_rotation_translation().1;
        let mut samples = Vec::new();
        for (at, shoulder_angle, elbow_angle, wrist_angle, finger_curl) in [
            (0, 0.0, 0.0, 0.0, 0.0),
            (30, 1.15, -0.25, 0.0, 0.25),
            (60, 1.15, -0.25, 0.65, 0.25),
            (90, 1.15, -0.25, -0.65, 0.25),
            (120, 1.15, -0.25, 0.65, 0.25),
            (150, 0.0, 0.0, 0.0, 0.0),
        ] {
            let mut sample = Frame {
                model: frame.model,
                joints: frame.joints.clone(),
                at: frame.at,
            };
            for (bone, axis, angle) in [
                (shoulder, Vec3::X, shoulder_angle),
                (elbow, Vec3::X, elbow_angle),
                (wrist, Vec3::Z, wrist_angle),
                (index, Vec3::X, -finger_curl),
                (middle, Vec3::X, -finger_curl * 0.9),
                (ring, Vec3::X, -finger_curl * 0.8),
                (pinky, Vec3::X, -finger_curl * 0.7),
                (thumb, Vec3::Y, finger_curl * 0.45),
            ] {
                let (scale, rotation, position) = sample.joints[bone].to_scale_rotation_translation();
                let delta = model_rotation * Quat::from_axis_angle(axis, angle) * model_rotation.inverse();
                sample.joints[bone] = Mat4::from_scale_rotation_translation(scale, delta * rotation, position);
            }
            samples.push((at, sample));
        }
        if let Some(document) = self.clip.document.as_mut() {
            let document = Arc::make_mut(document);
            for (at, sample) in samples {
                let _ = document.record(rig, &sample, at, &[shoulder, elbow, wrist, index, middle, ring, pinky, thumb], &[]);
            }
            document.range(0, 150);
            document.meta.ease_in = 0.12;
            document.meta.ease_out = 0.12;
            document.timing();
        }
        self.clip.changed();
        self.clip.seek(0);
        self.clip.playing = true;
        self.select(wrist, rig, &frame);
        self.tab = 2;
        self.clip.tab = 0;
    }

    pub(super) fn jump_demo_from_avatar_studio(&mut self, world: &World, frame: &Frame) {
        self.open_from_avatar_studio(world);
        let rig = &world.avatar_lib.rig;
        let pelvis = rig.pelvis;
        if let Some(document) = self.clip.document.as_mut() {
            let document = Arc::make_mut(document);
            document.range(0, 90);
            document.meta.looping = false;
            for (at, height) in [(15, 0.65), (30, 1.0), (45, 0.65), (60, 0.0)] {
                let mut sample = Frame {
                    model: frame.model,
                    joints: frame.joints.clone(),
                    at: frame.at,
                };
                sample.joints[pelvis].w_axis += frame.model.transform_vector3(Vec3::Z * height).extend(0.0);
                let _ = document.record(rig, &sample, at, &[pelvis], &[pelvis]);
            }
        }
        self.clip.changed();
        self.clip.seek(30);
        self.select(pelvis, rig, frame);
        self.tab = 0;
        self.clip.tab = 0;
        self.clip.timeline_open = false;
    }

    pub(super) fn close_from_avatar_studio(&mut self) {
        self.leave_clip();
        self.open = false;
        self.clip.from_avatar_studio = false;
        self.clip.return_after_upload = false;
    }

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
        self.clip.tab = if mode == "clip-priority" || mode == "animation-editor" {
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
            self.queue_studio_file_action(files::Action::Import { last_key_only: false });
        }
    }
    pub fn take_clip_imports(&mut self) -> Vec<Prepared> {
        let prepared = std::mem::take(&mut self.clip.prepared);
        if !prepared.is_empty() {
            self.clip.return_after_upload = self.clip.from_avatar_studio;
            self.poser.active = false;
            self.leave_clip();
            self.open = false;
            self.cancel_link_pick();
        }
        prepared
    }
    pub fn resume_after_upload(&mut self, world: &World) -> bool {
        if !self.clip.return_after_upload || !self.clip.from_avatar_studio {
            self.clip.return_after_upload = false;
            return false;
        }
        self.clip.return_after_upload = false;
        self.open_from_avatar_studio(world);
        self.clip.active
    }
    pub fn tick_clip(&mut self, jobs: &Jobs, ctx: &egui::Context, world: &World) {
        let clear = self
            .clip
            .tick(jobs, ctx, world.avatar_lib.rig.clone(), world.avatar_lib.skeleton.as_ref());
        if self.clip.active && !clear.is_empty() {
            let baked = self.authoring.baked(&clear);
            self.goals.retain(|g| !clear.contains(&g.bone) && !baked.contains(&g.bone));
            self.refresh_clip_marks();
            self.refresh = true;
        }
    }
    /// Rebuild an authored frame from the rest skeleton, local pose edits and
    /// contacts, never from the displayed animation. Keep the shape so later
    /// keys do not read it as a translation.
    fn authored_frame(&self, rig: &Rig, frame: &Frame) -> Option<Frame> {
        let (shaped, scales) = authoring::local_pose(rig, frame)?;
        let mut pose = ik::LocalPose {
            rotations: rig.local_rot.clone(),
            positions: shaped.positions,
        };
        pose.positions[rig.pelvis] = rig.local_pos[rig.pelvis];
        for joint in self.authoring.request(&self.goals, &self.disabled) {
            if joint.bone >= rig.len() || !joint.weight.is_finite() || !(0.0..=1.0).contains(&joint.weight) {
                continue;
            }
            if let Some(rotation) = joint
                .rotation
                .filter(|rotation| rotation.is_finite() && (0.9..=1.1).contains(&rotation.length_squared()))
            {
                pose.rotations[joint.bone] = pose.rotations[joint.bone].slerp(rotation.normalize(), joint.weight).normalize();
            }
            if let Some(translation) = joint.translation.filter(|translation| translation.is_finite()) {
                pose.positions[joint.bone] += translation * joint.weight;
            }
        }
        for goal in &self.request().goals {
            ik::apply_goal(rig, &mut pose, Vec3::ZERO, &scales, goal);
        }
        Some(Frame {
            model: frame.model,
            joints: rig
                .world_with(&pose.rotations, Vec3::ZERO, &pose.positions, &rig.scale)
                .iter()
                .map(|m| frame.model * *m)
                .collect(),
            at: frame.at,
        })
    }
    fn new_clip(&mut self, world: &World, frame: &Frame) {
        if self.clip.history.len() >= 15 {
            return;
        }
        let rig = &world.avatar_lib.rig;
        let mut document = Document::new();
        let recorded = self.authored_frame(rig, frame).map_or(Err("anim-edit-invalid"), |authored| {
            document.record(rig, &authored, 0, &(0..rig.len()).collect::<Vec<_>>(), &[])
        });
        if let Err(key) = recorded {
            self.clip.error = Some(aurora_i18n::text(key).to_owned());
            return;
        }
        for track in &mut document.tracks {
            track.authored = false;
        }
        if let Some(old) = self.clip.document.replace(Arc::new(document)) {
            self.clip.history.push(old);
        }
        self.clip.clear_undo_history();
        self.clip.seek(0);
        self.clip.changed();
        self.enter_clip(world, frame);
    }
    fn capture_ik_clip(&mut self, world: &World, frame: &Frame) {
        let translations: Vec<_> = self.goals.iter().filter(|g| g.mode == Mode::Direct).map(|g| g.bone).collect();
        let rig = &world.avatar_lib.rig;
        let bones: Vec<_> = (0..rig.len()).collect();
        let at = self.clip.frame;
        let authored = self.authored_frame(rig, frame);
        self.clip.checkpoint();
        let Some(document) = self.clip.document.as_mut() else { return };
        let recorded = authored.map_or(Err("anim-edit-invalid"), |authored| {
            Arc::make_mut(document).record(rig, &authored, at, &bones, &translations)
        });
        match recorded {
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
            let recorded = self
                .authored_frame(rig, frame)
                .map_or(Err("anim-edit-invalid"), |authored| document.record(rig, &authored, 0, &bones, &[]));
            if let Err(key) = recorded {
                self.clip.error = Some(aurora_i18n::text(key).to_owned());
                return;
            }
            for track in &mut document.tracks {
                track.authored = false;
            }
            self.clip.document = Some(Arc::new(document));
            self.clip.auto_key_retime_from = None;
            self.clip.changed();
        }
        if !self.clip.active {
            self.clip.saved_pose_transform = Some(self.poser.transform);
            if self.poser.transform == 2 {
                self.poser.transform = 0;
                self.tool = Tool::Rotate;
                self.poser.cancel_scale_gizmo();
            }
        }
        let previous = self.request();
        self.clip.saved_goals = self.goals.clone();
        self.clip.saved_authoring = self.authoring.clone();
        self.clip.saved_disabled = self.disabled.clone();
        self.clip.capture = None;
        self.clip.auto_key_pending = false;
        self.clip.auto_key_advance_to = None;
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
        self.clip.auto_key_pending = false;
        self.clip.modified.clear();
        self.clip.translations.clear();
        self.clip.clear_after = None;
        self.clip.clear_bones.clear();
        self.goals = std::mem::take(&mut self.clip.saved_goals);
        self.authoring = std::mem::take(&mut self.clip.saved_authoring);
        self.disabled = std::mem::take(&mut self.clip.saved_disabled);
        if let Some(transform) = self.clip.saved_pose_transform.take() {
            self.poser.transform = transform.min(2);
            self.tool = if self.poser.transform == 0 { Tool::Rotate } else { Tool::Move };
        }
        self.drag = None;
        self.gizmo.cancel();
        self.refresh = true;
        self.tab = 2;
    }
    pub(super) fn capture_clip(&mut self, world: &World, frame: &Frame) {
        // the held source is the displayed pose, which is never recorded
        if self.clip.waiting_source() {
            return;
        }
        let Some(capture) = self.clip.capture.take() else { return };
        if !self.clip.active {
            return;
        }
        let rig = &world.avatar_lib.rig;
        let advance_after_auto_key = matches!(capture, Capture::AutoKeyModified);
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
            Capture::Modified | Capture::AutoKeyModified => self.clip.modified.iter().copied().collect(),
            Capture::Pose => (0..rig.len()).collect(),
        };
        let translations: Vec<_> = self.clip.translations.iter().copied().collect();
        let Some(authored) = self.authored_frame(rig, frame) else {
            self.clip.error = Some(aurora_i18n::text("anim-edit-invalid").to_owned());
            return;
        };
        self.clip.checkpoint();
        let Some(document) = self.clip.document.as_mut() else { return };
        match Arc::make_mut(document).record(rig, &authored, self.clip.frame, &bones, &translations) {
            Ok(()) => {
                if advance_after_auto_key && let Some(document) = self.clip.document.as_mut() {
                    let document = Arc::make_mut(document);
                    self.clip.auto_key_advance_to = next_auto_key_frame(document, self.clip.frame);
                }
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
        let Some(bone) = self.selected else { return };
        let sync_key = (bone, self.clip.cursor.to_bits(), self.clip.revision, self.clip.preview_revision);
        if !self.clip.playing && self.clip.selection_sync == Some(sync_key) {
            return;
        }
        if let Some(matrix) = frame.joints.get(bone) {
            let local = frame.model.inverse() * *matrix;
            self.position = frame.model.inverse().transform_point3(matrix.transform_point3(self.contact));
            self.set_rotation(local.to_scale_rotation_translation().1);
            self.clip.selection_sync = Some(sync_key);
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

//! Couple mode: a second person animated beside the first, in the same
//! timeline. Second Life plays one animation per avatar, so a couple pose is
//! two animations started together, the furniture placing each avatar: each
//! person keeps a document of their own, keyed in their own avatar's space,
//! and exports to a file of their own, with a note of where B stands from A.
//!
//! The editor works on one person at a time, with all its tools. The other
//! person's whole editing state waits here (`Person`) and is still shown:
//! its own preview at the shared playhead, with the edits not keyed yet.
//! Switching swaps the two, once the edited person's preview is current, so
//! that the person set aside always shows what was keyed.
use super::*;

/// One person's editing state while the other is edited.
#[derive(Default)]
pub(in crate::ik_edit) struct Person {
    document: Option<Arc<Document>>,
    undo: Vec<Arc<Document>>,
    redo: Vec<Arc<Document>>,
    selected_keys: BTreeSet<document::KeyPick>,
    /// Current for `document` (switching waits for it).
    preview: Option<Preview>,
    modified: HashSet<usize>,
    translations: HashSet<usize>,
    together: Vec<usize>,
    goals: Vec<Goal>,
    authoring: authoring::State,
    disabled: Vec<usize>,
    selected: Option<usize>,
    /// The lanes of `document`, built once while it waits.
    lanes: Option<Arc<Vec<TimelineTrack>>>,
}

/// Who Person B is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ik_edit) enum Partner {
    /// A resident nearby who accepted to be posed (`poser_consent`).
    Avatar(Uuid),
    /// A mirror of our own avatar (`World::spawn_mirror`), to animate a
    /// couple alone: no one to ask.
    Mirror(Uuid),
}

impl Partner {
    pub(in crate::ik_edit) fn id(self) -> Uuid {
        match self {
            Self::Avatar(id) | Self::Mirror(id) => id,
        }
    }
}

pub(in crate::ik_edit) struct Couple {
    pub(in crate::ik_edit) partner: Partner,
    /// B's name, for the timeline.
    pub(in crate::ik_edit) partner_name: String,
    /// The editor works on Person B (otherwise on A, our own avatar).
    pub(in crate::ik_edit) editing_b: bool,
    other: Person,
    /// A switch asked for while the edited person's preview was building.
    pending_switch: bool,
    /// The joint to select once the switch is done (a click on the other
    /// person's joint).
    select_after: Option<usize>,
    /// Where B is moved on screen: a shift in the region (metres) and a
    /// turn about their pelvis (radians), never part of their animation.
    pub(in crate::ik_edit) place: (Vec3, f32),
}

impl Couple {
    /// The avatar the editor works on.
    pub(in crate::ik_edit) fn edited(&self, agent: Uuid) -> Uuid {
        if self.editing_b { self.partner.id() } else { agent }
    }
    /// The avatar set aside.
    pub(in crate::ik_edit) fn waiting(&self, agent: Uuid) -> Uuid {
        if self.editing_b { agent } else { self.partner.id() }
    }
    /// The person set aside's lanes, built from their document.
    pub(in crate::ik_edit) fn waiting_lanes(&mut self) -> Option<Arc<Vec<TimelineTrack>>> {
        if self.other.lanes.is_none() {
            let document = self.other.document.as_ref()?;
            self.other.lanes = Some(Arc::new(lanes_of(document)));
        }
        self.other.lanes.clone()
    }
}

/// A document's lanes: each track with its keys as frames.
pub(super) fn lanes_of(document: &Document) -> Vec<TimelineTrack> {
    let fps = document.fps as f32;
    document
        .tracks
        .iter()
        .map(|track| {
            let frames = |times: &mut dyn Iterator<Item = f32>| {
                let mut frames: Vec<u32> = times.map(|time| (time * fps).round() as u32).collect();
                frames.sort_unstable();
                frames.dedup();
                frames
            };
            let rotation = frames(&mut track.motion.rot_keys.iter().map(|key| key.time));
            let location = frames(&mut track.motion.pos_keys.iter().map(|key| key.time));
            let mut keys: Vec<u32> = rotation.iter().chain(&location).copied().collect();
            keys.sort_unstable();
            keys.dedup();
            TimelineTrack {
                bone: track.bone,
                name: track.motion.joint_name.clone(),
                enabled: track.enabled,
                authored: track.authored,
                keys,
                rotation,
                location,
            }
        })
        .collect()
}

/// The sample of `preview` at the playhead `cursor` (timeline frames).
pub(super) fn sample_at(preview: &Preview, fps: f32, cursor: f64) -> ik::animations::Sample {
    let playhead = cursor.clamp(f64::from(preview.start), f64::from(preview.end));
    let time_seconds = ((playhead - f64::from(preview.start)) / f64::from(fps)) as f32;
    ik::animations::Sample {
        animation: preview.bound.clone(),
        frame: (time_seconds * ik::animations::FPS).round() as u32,
        time_seconds: Some(time_seconds),
    }
}

/// The way a skeleton model faces in the region (radians about up).
fn yaw_of(model: Mat4) -> f32 {
    let forward = model.transform_vector3(Vec3::X);
    forward.y.atan2(forward.x)
}

/// The way our avatar (Person A) faces.
fn a_facing(world: &World) -> f32 {
    world.avatar_models.get(&world.agent_id).map_or(0.0, |model| yaw_of(*model))
}

/// Where B stands from A: B's position in A's avatar space (metres) and
/// B's turn about A's up axis (degrees), for the furniture's sit targets.
pub(in crate::ik_edit) fn placement(a: Mat4, b: Mat4) -> Option<(Vec3, f32)> {
    let relative = a.inverse() * b;
    let (_, rotation, position) = relative.to_scale_rotation_translation();
    let forward = rotation * Vec3::X;
    let yaw = forward.y.atan2(forward.x).to_degrees();
    (position.is_finite() && yaw.is_finite()).then_some((position, yaw))
}

/// The placement note written beside the two exported files.
pub(in crate::ik_edit) fn placement_note(name: &str, position: Vec3, yaw: f32) -> String {
    format!(
        "{name}: couple animation\n\
         Person A: {name} A\n\
         Person B: {name} B\n\
         Person B from Person A, in A's avatar space:\n\
         position <{:.3}, {:.3}, {:.3}> m\n\
         rotation <0, 0, {:.1}> degrees\n",
        position.x, position.y, position.z, yaw
    )
}

impl State {
    /// The edited person's preview shows what is keyed, and no edit or key
    /// drag is on its way to the document: the two can be swapped.
    pub(super) fn can_switch_person(&self) -> bool {
        self.active
            && !self.hold_source
            && self.clear_after.is_none()
            && self.capture.is_none()
            && self.drag_key.is_none()
            && self.document.is_some()
            && self.preview_revision == Some(self.revision)
            && self.preview.as_ref().is_some_and(|p| p.revision == self.revision)
    }
    /// The person set aside's pose at the playhead.
    pub(in crate::ik_edit) fn waiting_sample(&self) -> Vec<ik::animations::Sample> {
        let Some(couple) = self.couple.as_ref() else { return Vec::new() };
        let Some(preview) = couple.other.preview.as_ref() else {
            return Vec::new();
        };
        let fps = couple.other.document.as_ref().map_or(FPS, |document| document.fps as f32);
        vec![sample_at(preview, fps, self.cursor)]
    }
}

impl Editor {
    /// Person B's avatar while couple mode runs: the world keeps it drawn
    /// if they leave (`World::set_couple_partner`).
    pub fn couple_partner(&self) -> Option<Uuid> {
        self.clip
            .couple
            .as_ref()
            .filter(|_| self.clip.active)
            .map(|couple| couple.partner.id())
    }
    /// The avatar the animation editor poses (Person B in couple mode).
    pub fn clip_edited_subject(&self, world: &World) -> Uuid {
        self.edited_subject(world)
    }
    pub(in crate::ik_edit) fn edited_subject(&self, world: &World) -> Uuid {
        match self.clip.couple.as_ref() {
            Some(couple) if self.clip.active => couple.edited(world.agent_id),
            _ => world.agent_id,
        }
    }
    /// What the scene shows on the person set aside: their keyed preview
    /// at the playhead, with their edits not keyed yet.
    pub fn waiting_request(&self, world: &World) -> Option<(Uuid, ik::EditRequest)> {
        let couple = self.clip.couple.as_ref().filter(|_| self.clip.active && !self.poser.active)?;
        let other = &couple.other;
        let request = ik::EditRequest {
            active: true,
            animations: self.clip.waiting_sample(),
            suppress_live: true,
            goals: other
                .goals
                .iter()
                .filter(|g| !other.disabled.contains(&g.bone) && !other.authoring.owns(g.bone))
                .cloned()
                .collect(),
            joints: other.authoring.request(&other.goals, &other.disabled),
        };
        Some((couple.waiting(world.agent_id), request))
    }
    /// Start couple mode with `partner` as Person B. B starts from their
    /// rest pose, with A's range, rate and settings, and is edited first.
    pub(in crate::ik_edit) fn start_couple(&mut self, world: &World, partner: Partner) -> bool {
        let partner_name = match partner {
            Partner::Avatar(id) => crate::world::poser_consent::name(world, id),
            Partner::Mirror(_) => aurora_i18n::text("anim-couple-mirror-name").to_owned(),
        };
        if !self.clip.active || self.clip.couple.is_some() || !self.clip.can_switch_person() {
            return false;
        }
        let Some(frame) = Frame::subject(world, partner.id()) else {
            return false;
        };
        let Some(a) = self.clip.document.clone() else { return false };
        self.clip.couple = Some(Couple {
            partner,
            partner_name,
            editing_b: false,
            other: Person::default(),
            pending_switch: false,
            select_after: None,
            place: (Vec3::ZERO, 0.0),
        });
        // A is set aside, B's state starts empty
        self.swap_person();
        let rig = &world.avatar_lib.rig;
        let mut document = Document::new();
        document.name = format!("{} B", a.name);
        document.meta = a.meta.clone();
        document.fps = a.fps;
        document.start = a.start;
        document.end = a.end;
        document.auto_keyframe = a.auto_keyframe;
        document.auto_key_advance_seconds = a.auto_key_advance_seconds;
        let bones: Vec<_> = (0..rig.len()).collect();
        let recorded = self
            .authored_frame(rig, &frame, world)
            .map_or(Err("anim-edit-invalid"), |authored| document.record(rig, &authored, 0, &bones, &[]));
        if let Err(key) = recorded {
            // back to A alone
            self.swap_person();
            self.clip.couple = None;
            self.clip.error = Some(aurora_i18n::text(key).to_owned());
            return false;
        }
        for track in &mut document.tracks {
            track.authored = false;
        }
        self.clip.document = Some(Arc::new(document));
        self.clip.changed();
        true
    }
    /// AURORA_DEMO_COUPLE's scripted modes (`clip`, `hover`), for screenshots with no clicks: ask `peer`, then take them
    /// as Person B once they accept, standing face to face, with the Clip
    /// page open. True once couple mode runs.
    pub fn demo_couple_step(&mut self, world: &World, peer: Uuid) -> bool {
        if !self.clip.active {
            return false;
        }
        if self.clip.couple.is_some() {
            return true;
        }
        use crate::world::poser_consent::Status;
        match world.poser_consent.status(peer) {
            Status::Ask => {
                self.push_poser_request(poser::Request::Ask(peer));
                false
            }
            Status::Accepted if self.start_couple(world, Partner::Avatar(peer)) => {
                self.couple_face_to_face(world);
                self.clip.tab = ClipTab::Clip;
                self.clip.timeline_open = true;
                true
            }
            _ => false,
        }
    }
    /// AURORA_DEMO_COUPLE=mirror: press "Mirror yourself" once the editor
    /// is open. True once asked.
    pub fn demo_mirror_step(&mut self) -> bool {
        if !self.clip.active || self.clip.couple.is_some() {
            return false;
        }
        self.clip.mirror_wanted.get_or_insert_with(std::time::Instant::now);
        true
    }
    /// "Mirror yourself" was pressed: the app makes the mirror
    /// (`World::spawn_mirror`) and hands it over with `start_mirror_couple`.
    pub fn mirror_wanted(&self) -> bool {
        self.clip.mirror_wanted.is_some() && self.clip.active && self.clip.couple.is_none()
    }
    /// Take the mirror `id` as Person B, once the scene has posed it (its
    /// first frame). A mirror that never shows (or none, while our avatar
    /// is not in the world) gives up after a few
    /// seconds, with a message. True once couple mode runs.
    pub fn start_mirror_couple(&mut self, world: &World, id: Option<Uuid>) -> bool {
        let Some(since) = self.clip.mirror_wanted else { return false };
        if let Some(id) = id
            && self.start_couple(world, Partner::Mirror(id))
        {
            self.clip.mirror_wanted = None;
            self.clip.tab = ClipTab::Clip;
            return true;
        }
        if since.elapsed() > std::time::Duration::from_secs(5) {
            self.clip.mirror_wanted = None;
            if self.clip.error.is_none() {
                self.clip.error = Some(aurora_i18n::text("anim-couple-wait").to_owned());
            }
        }
        false
    }
    /// Leave couple mode: A is edited alone again, B's animation stays
    /// among the drafts.
    pub(in crate::ik_edit) fn end_couple(&mut self) {
        let Some(couple) = self.clip.couple.as_ref() else { return };
        if couple.editing_b {
            // nothing of B's is lost: the swap does not wait for a preview
            self.swap_person();
        }
        if let Some(couple) = self.clip.couple.take()
            && let Some(document) = couple.other.document
        {
            self.clip.shelve(document);
        }
        self.clip.changed();
    }
    /// A save or an export queued in couple mode writes both people: their
    /// documents, A's first, and where B stands from A as the avatars
    /// stand now. The upload stays the edited person's own.
    pub(super) fn prepare_couple_files(&mut self, world: &World) {
        let pair = matches!(self.clip.export, Some(files::Action::Save { .. } | files::Action::ExportBvh { .. }));
        let Some(couple) = self.clip.couple.as_ref().filter(|_| pair) else {
            self.clip.couple_files = None;
            return;
        };
        let (Some(edited), Some(waiting)) = (self.clip.document.clone(), couple.other.document.clone()) else {
            return;
        };
        let (a, b) = if couple.editing_b { (waiting, edited) } else { (edited, waiting) };
        // the avatars' models as the scene places them: Avatar Studio's
        // display lift of ours stays out of the note
        let models = world
            .avatar_models
            .get(&world.agent_id)
            .zip(world.avatar_models.get(&couple.partner.id()));
        // B where the editor moved them
        let models = models.map(|(a, b)| (*a, world.couple_move(couple.partner.id(), *b).map_or(*b, |shift| shift * *b)));
        let Some((position, yaw)) = models.and_then(|(a, b)| placement(a, b)) else {
            self.clip.export = None;
            self.clip.busy = false;
            self.clip.error = Some(aurora_i18n::text("anim-couple-partner-left").to_owned());
            return;
        };
        let note = placement_note(&files::basename(&a.name), position, yaw);
        self.clip.couple_files = Some(files::CoupleFiles { a, b, note });
    }
    /// Move Person B on screen: `local` is the shift in A's facing (forward,
    /// left, up, metres), `turn` B's turn (radians). Kept within 20 m.
    pub(in crate::ik_edit) fn set_couple_place(&mut self, world: &World, local: Vec3, turn: f32) {
        let Some(couple) = self.clip.couple.as_mut() else { return };
        let facing = a_facing(world);
        let local = local.clamp(Vec3::splat(-20.0), Vec3::splat(20.0));
        couple.place = (Quat::from_rotation_z(facing) * local, turn);
    }
    /// B's shift in A's facing, and B's turn, for the fields.
    pub(in crate::ik_edit) fn couple_place_local(&self, world: &World) -> (Vec3, f32) {
        let (offset, turn) = self.couple_place();
        (Quat::from_rotation_z(-a_facing(world)) * offset, turn)
    }
    /// Stand B half a metre in front of A, facing A.
    pub(in crate::ik_edit) fn couple_face_to_face(&mut self, world: &World) {
        let Some(couple) = self.clip.couple.as_ref() else { return };
        let (Some(a), Some(b)) = (
            world.avatar_models.get(&world.agent_id).copied(),
            world.couple_base(couple.partner.id()),
        ) else {
            return;
        };
        let pelvis = world.avatar_lib.pelvis;
        let facing = a_facing(world);
        let target = a.transform_point3(pelvis) + Quat::from_rotation_z(facing) * Vec3::new(0.5, 0.0, 0.0);
        let shift = target - b.transform_point3(pelvis);
        let b_facing = yaw_of(b);
        let turn = facing + std::f32::consts::PI - b_facing;
        let turn = (turn + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        if let Some(couple) = self.clip.couple.as_mut() {
            couple.place = (shift, turn);
        }
    }
    /// Where Person B is moved on screen, for `World::couple_place`.
    pub fn couple_place(&self) -> (Vec3, f32) {
        self.clip
            .couple
            .as_ref()
            .filter(|_| self.clip.active)
            .map_or((Vec3::ZERO, 0.0), |couple| couple.place)
    }
    /// A click on a joint of the person set aside: edit them, with that
    /// joint selected.
    pub(in crate::ik_edit) fn switch_to_joint(&mut self, bone: usize) {
        if let Some(couple) = self.clip.couple.as_mut() {
            couple.select_after = Some(bone);
        }
        self.switch_person();
    }
    /// The joints of the person set aside, in their colour, over their body:
    /// both people can be picked from the scene, a click edits the other.
    pub(in crate::ik_edit) fn couple_waiting_handles(
        &mut self,
        ctx: &egui::Context,
        palette: &Palette,
        world: &World,
        proj: &Projector,
        blocked: bool,
    ) {
        let Some(couple) = self.clip.couple.as_ref().filter(|_| self.clip.active && !self.poser.active) else {
            return;
        };
        let b = !couple.editing_b;
        let Some(frame) = Frame::subject(world, couple.waiting(world.agent_id)) else {
            return;
        };
        let color = super::timeline::person_color(palette, b);
        let rig = &world.avatar_lib.rig;
        let mut picked = None;
        for (joint, name) in rig.canonical_names.iter().enumerate() {
            if !self.visibility.visible(name) {
                continue;
            }
            let Some(matrix) = frame.joints.get(joint) else { continue };
            let Some((at, _)) = proj.project(matrix.w_axis.truncate()) else {
                continue;
            };
            let half = 6.0;
            let response = egui::Area::new(egui::Id::unique(("ik-couple-joint", joint)))
                .fixed_pos(at - egui::vec2(half, half))
                .order(egui::Order::Background)
                .movable(false)
                .interactable(!blocked)
                .show(ctx, |ui| {
                    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(half * 2.0), egui::Sense::click());
                    let hovered = response.hovered();
                    ui.painter().circle_filled(
                        rect.center(),
                        if hovered { 4.0 } else { 2.5 },
                        color.gamma_multiply(if hovered { 1.0 } else { 0.7 }),
                    );
                    response
                })
                .inner;
            self.manipulator_hovered |= response.contains_pointer() && !blocked;
            if response.clicked() {
                picked = Some(joint);
            }
            if response.hovered() {
                response.on_hover_text(aurora_i18n::tr!("anim-couple-switch"));
            }
        }
        if let Some(joint) = picked {
            self.switch_to_joint(joint);
            self.select_after_switch(world);
        }
    }
    /// The joint clicked on the other person, selected once the switch to
    /// them is done, as any pick: its binding, its followers, the tool.
    fn select_after_switch(&mut self, world: &World) {
        let Some(couple) = self.clip.couple.as_mut().filter(|couple| !couple.pending_switch) else {
            return;
        };
        let Some(bone) = couple.select_after.take() else { return };
        match Frame::subject(world, couple.edited(world.agent_id)) {
            Some(frame) => self.select(bone, &world.avatar_lib.rig, &frame),
            // no body to read yet: the joint is picked all the same
            None => self.selected = Some(bone),
        }
    }
    /// Edit the other person: now, or once the edited person's preview
    /// shows what was just keyed.
    pub(in crate::ik_edit) fn switch_person(&mut self) {
        if self.clip.couple.is_none() {
            return;
        }
        if self.clip.can_switch_person() {
            self.swap_person();
        } else if let Some(couple) = self.clip.couple.as_mut() {
            couple.pending_switch = true;
        }
    }
    /// A switch asked for while a preview was building, and the partner
    /// gone (they withdrew their consent, or are no longer there).
    pub(in crate::ik_edit) fn tick_couple(&mut self, world: &World) {
        let Some(couple) = self.clip.couple.as_ref() else { return };
        if !self.clip.active {
            self.clip.couple = None;
            return;
        }
        let partner = couple.partner;
        let pending_switch = couple.pending_switch;
        let present = match partner {
            Partner::Avatar(id) => world.poser_consent.can_pose(world, id),
            Partner::Mirror(id) => world.is_mirror(&id) && world.objects.index_of_uuid(&id).is_some(),
        };
        if !present {
            self.end_couple();
            self.clip.error = Some(aurora_i18n::text("anim-couple-partner-left").to_owned());
            return;
        }
        // a name still loading when B was chosen shows once it arrives
        if let Partner::Avatar(id) = partner {
            let name = crate::world::poser_consent::name(world, id);
            if let Some(couple) = self.clip.couple.as_mut()
                && couple.partner_name != name
            {
                couple.partner_name = name;
            }
        }
        self.sync_couple_timing(&world.avatar_lib.rig);
        if pending_switch && self.clip.can_switch_person() {
            if let Some(couple) = self.clip.couple.as_mut() {
                couple.pending_switch = false;
            }
            self.swap_person();
            self.select_after_switch(world);
        }
    }
    /// The two animations play together: the range, frame rate, loop and
    /// ease set on the edited person carry over to the one set aside (their
    /// keys stay), whose preview is rebuilt at once so they keep in step.
    fn sync_couple_timing(&mut self, rig: &Rig) {
        let Some(edited) = self.clip.document.clone() else { return };
        let Some(couple) = self.clip.couple.as_mut() else { return };
        let Some(other) = couple.other.document.as_ref() else { return };
        let timing = |d: &Document| {
            (
                d.fps,
                d.start,
                d.end,
                d.meta.looping,
                d.meta.loop_in,
                d.meta.loop_out,
                d.meta.ease_in,
                d.meta.ease_out,
            )
        };
        if timing(&edited) == timing(other) {
            return;
        }
        let mut document = (**other).clone();
        document.set_fps(edited.fps);
        document.range(edited.start, edited.end);
        document.meta.looping = edited.meta.looping;
        document.meta.loop_in = edited.meta.loop_in;
        document.meta.loop_out = edited.meta.loop_out;
        document.meta.ease_in = edited.meta.ease_in;
        document.meta.ease_out = edited.meta.ease_out;
        document.timing();
        let person = &mut couple.other;
        match document.preview() {
            Ok(animation) => {
                let revision = person.preview.as_ref().map_or(0, |preview| preview.revision);
                person.preview = Some(Preview {
                    revision,
                    start: document.start,
                    end: document.end,
                    bound: Arc::new(BoundAnim::bind(animation, rig)),
                });
            }
            // nothing to play (no track yet): nothing stale either
            Err(_) => person.preview = None,
        }
        person.lanes = None;
        person.document = Some(Arc::new(document));
    }
    /// Swap the edited person's whole state with the one set aside.
    fn swap_person(&mut self) {
        let Some(mut couple) = self.clip.couple.take() else { return };
        let incoming = std::mem::take(&mut couple.other);
        let current = self.clip.preview_revision == Some(self.clip.revision);
        let preview = self.clip.preview.take().filter(|_| current);
        couple.other = Person {
            document: std::mem::replace(&mut self.clip.document, incoming.document),
            undo: std::mem::replace(&mut self.clip.undo, incoming.undo),
            redo: std::mem::replace(&mut self.clip.redo, incoming.redo),
            selected_keys: std::mem::replace(&mut self.clip.selected_keys, incoming.selected_keys),
            preview,
            modified: std::mem::replace(&mut self.clip.modified, incoming.modified),
            translations: std::mem::replace(&mut self.clip.translations, incoming.translations),
            together: std::mem::replace(&mut self.clip.together, incoming.together),
            goals: std::mem::replace(&mut self.goals, incoming.goals),
            authoring: std::mem::replace(&mut self.authoring, incoming.authoring),
            disabled: std::mem::replace(&mut self.disabled, incoming.disabled),
            selected: std::mem::replace(&mut self.selected, incoming.selected),
            lanes: None,
        };
        couple.editing_b = !couple.editing_b;
        self.clip.couple = Some(couple);
        // the incoming preview is current for its document: it keeps
        // showing under the revision that now names that document
        self.clip.changed();
        let revision = self.clip.revision;
        self.clip.preview = incoming.preview.map(|mut preview| {
            preview.revision = revision;
            preview
        });
        self.clip.preview_revision = self.clip.preview.as_ref().map(|_| revision);
        self.clip.hold_source = false;
        self.clip.clear_after = None;
        self.clip.clear_bones.clear();
        self.clip.capture = None;
        self.clip.drag_key = None;
        self.clip.key_box = None;
        self.clip.auto_key_pending = false;
        self.clip.auto_key_advance_to = None;
        self.clip.selection_sync = None;
        self.clip.authored_sync = None;
        self.drag = None;
        self.gizmo.cancel();
        self.pole = None;
        self.pole_orbit = None;
        self.refresh = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;

    fn rest_frame(lib: &AvatarLibrary, turn: f32) -> Frame {
        let rig = &lib.rig;
        let mut rotations = rig.local_rot.clone();
        let head = rig.names["mHead"];
        rotations[head] = Quat::from_rotation_z(turn) * rig.local_rot[head];
        Frame {
            model: Mat4::IDENTITY,
            joints: rig.world_with(&rotations, Vec3::ZERO, &rig.local_pos, &rig.scale),
            at: Instant::now(),
        }
    }

    /// A document keying the head at `turn`, with its built preview.
    fn keyed(lib: &AvatarLibrary, name: &str, turn: f32) -> (Arc<Document>, Preview) {
        let rig = &lib.rig;
        let mut document = Document::new();
        document.name = name.into();
        document
            .record(rig, &rest_frame(lib, turn), 0, &[rig.names["mHead"]], &[])
            .expect("head key");
        let bound = Arc::new(BoundAnim::bind(document.preview().expect("preview"), rig));
        let preview = Preview {
            revision: 0,
            start: document.start,
            end: document.end,
            bound,
        };
        (Arc::new(document), preview)
    }

    /// An editor in the animation editor on A, its preview current, with B
    /// set aside as a couple would hold them.
    fn couple_editor(lib: &AvatarLibrary) -> Editor {
        let (a, a_preview) = keyed(lib, "Hug", 0.3);
        let (b, b_preview) = keyed(lib, "Hug B", -0.6);
        let mut editor = Editor::default();
        editor.clip.active = true;
        editor.clip.document = Some(a);
        editor.clip.revision = 7;
        editor.clip.preview = Some(Preview { revision: 7, ..a_preview });
        editor.clip.preview_revision = Some(7);
        editor.clip.couple = Some(Couple {
            partner: Partner::Avatar(Uuid::from_u128(42)),
            partner_name: "Nova".into(),
            editing_b: false,
            other: Person {
                document: Some(b),
                preview: Some(b_preview),
                ..Default::default()
            },
            pending_switch: false,
            select_after: None,
            place: (Vec3::ZERO, 0.0),
        });
        editor
    }

    #[test]
    fn range_rate_loop_and_ease_carry_over_to_the_person_set_aside() {
        let lib = AvatarLibrary::load();
        let mut editor = couple_editor(&lib);
        let document = Arc::make_mut(editor.clip.document.as_mut().expect("A"));
        document.set_fps(24);
        document.range(6, 54);
        document.meta.looping = true;
        document.meta.loop_in = 0.5;
        document.meta.loop_out = 1.5;
        document.meta.ease_in = 0.25;
        document.timing();
        let a = editor.clip.document.clone().expect("A");
        editor.sync_couple_timing(&lib.rig);
        let couple = editor.clip.couple.as_ref().expect("couple");
        let b = couple.other.document.as_ref().expect("B");
        assert_eq!((b.fps, b.start, b.end), (a.fps, a.start, a.end));
        assert!(b.meta.looping);
        assert_eq!((b.meta.loop_in, b.meta.loop_out, b.meta.ease_in), (0.5, 1.5, a.meta.ease_in));
        assert_eq!(b.name, "Hug B", "B keeps their own animation");
        let preview = couple.other.preview.as_ref().expect("B's preview");
        assert_eq!((preview.start, preview.end), (a.start, a.end), "B plays the new range at once");
    }

    #[test]
    fn switching_person_swaps_every_part_of_the_editing_state() {
        let lib = AvatarLibrary::load();
        let rig = &lib.rig;
        let mut editor = couple_editor(&lib);
        let elbow = rig.names["mElbowLeft"];
        editor.goals = vec![Goal {
            weight: 1.0,
            bone: elbow,
            root: elbow,
            position: Vec3::ZERO,
            rotation: Some(Quat::from_rotation_y(0.6)),
            contact: Vec3::ZERO,
            pole: None,
            mode: Mode::Direct,
        }];
        editor.selected = Some(elbow);
        editor.clip.selected_keys.insert((elbow, document::Channel::Both, 0));
        editor.clip.undo.push(editor.clip.document.clone().expect("A"));

        editor.switch_person();
        let couple = editor.clip.couple.as_ref().expect("couple");
        assert!(couple.editing_b);
        assert_eq!(editor.clip.document.as_ref().map(|d| d.name.as_str()), Some("Hug B"));
        assert!(editor.goals.is_empty() && editor.selected.is_none());
        assert!(editor.clip.selected_keys.is_empty() && editor.clip.undo.is_empty());
        // B's preview shows at once, under the revision that names B now
        assert_eq!(editor.clip.preview_revision, Some(editor.clip.revision));
        assert!(editor.clip.can_switch_person());
        // A waits with all of it
        assert_eq!(couple.other.document.as_ref().map(|d| d.name.as_str()), Some("Hug"));
        assert_eq!(couple.other.goals.len(), 1);
        assert_eq!(couple.other.selected, Some(elbow));
        assert!(couple.other.preview.is_some());

        editor.switch_person();
        assert!(!editor.clip.couple.as_ref().expect("couple").editing_b);
        assert_eq!(editor.clip.document.as_ref().map(|d| d.name.as_str()), Some("Hug"));
        assert_eq!(editor.goals.len(), 1);
        assert_eq!(editor.selected, Some(elbow));
        assert!(editor.clip.selected_keys.contains(&(elbow, document::Channel::Both, 0)));
        assert_eq!(editor.clip.undo.len(), 1);
    }

    #[test]
    fn dragging_the_ruler_in_couple_mode_stops_between_keys() {
        let lib = Arc::new(AvatarLibrary::load());
        let rig = &lib.rig;
        let mut world = World::new(lib.clone());
        world.agent_id = Uuid::from_u128(1);
        let span = |turn: f32, name: &str| {
            let mut document = Document::new();
            document.name = name.into();
            document.range(0, 30);
            for key in [0, 30] {
                document
                    .record(rig, &rest_frame(&lib, turn * key as f32), key, &[rig.names["mHead"]], &[])
                    .expect("head key");
            }
            let bound = Arc::new(BoundAnim::bind(document.preview().expect("preview"), rig));
            let preview = Preview {
                revision: 0,
                start: document.start,
                end: document.end,
                bound,
            };
            (Arc::new(document), preview)
        };
        let mut editor = couple_editor(&lib);
        editor.open = true;
        let (a, a_preview) = span(0.01, "Hug");
        let (b, b_preview) = span(-0.02, "Hug B");
        editor.clip.document = Some(a);
        editor.clip.preview = Some(Preview { revision: 7, ..a_preview });
        let couple = editor.clip.couple.as_mut().expect("couple");
        couple.other.document = Some(b);
        couple.other.preview = Some(b_preview);
        for _ in 0..2 {
            let reached = super::super::tests::ruler_drag_frames(&mut editor, &world);
            assert!(reached.windows(2).all(|w| w[0] < w[1]), "{reached:?}");
            assert!(reached.iter().any(|f| *f != 0 && *f != 30), "{reached:?}");
            editor.clip_seek(0);
            editor.switch_person();
        }
    }

    #[test]
    fn a_click_on_the_other_person_s_joint_edits_them_with_that_joint() {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        world.agent_id = Uuid::from_u128(1);
        let world = world;
        let elbow = lib.rig.names["mElbowLeft"];
        let mut editor = couple_editor(&lib);
        editor.switch_to_joint(elbow);
        editor.select_after_switch(&world);
        assert!(editor.clip.couple.as_ref().expect("couple").editing_b);
        assert_eq!(editor.selected, Some(elbow));

        // a click while A's keys are still building: B gets it after
        let mut editor = couple_editor(&lib);
        editor.clip.changed();
        editor.switch_to_joint(elbow);
        editor.select_after_switch(&world);
        assert!(!editor.clip.couple.as_ref().expect("couple").editing_b);
        assert_eq!(editor.selected, None);
        let preview = editor.clip.preview.take().map(|preview| Preview {
            revision: editor.clip.revision,
            ..preview
        });
        editor.clip.preview = preview;
        editor.clip.preview_revision = Some(editor.clip.revision);
        // what `tick_couple` does once the preview is current
        assert!(editor.clip.can_switch_person());
        editor.clip.couple.as_mut().expect("couple").pending_switch = false;
        editor.swap_person();
        editor.select_after_switch(&world);
        assert!(editor.clip.couple.as_ref().expect("couple").editing_b);
        assert_eq!(editor.selected, Some(elbow));
    }

    #[test]
    fn the_timeline_names_person_b_without_their_resident_name() {
        use crate::ui::widgets::elide_tests::Frames;
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        world.agent_id = Uuid::from_u128(1);
        let mut editor = couple_editor(&lib);
        editor.open = true;
        editor.clip.couple.as_mut().expect("couple").partner_name = "A Very Long Resident Name Indeed".into();
        let palette = crate::theme::Theme::default().palette();
        let mut frames = Frames::new();
        let mut draw = |ui: &mut egui::Ui| editor.studio_timeline_overlay(ui.ctx(), &palette, &world);
        frames.run(Vec::new(), &mut draw);
        let texts = frames.run(Vec::new(), &mut draw);
        assert!(texts.iter().any(|t| t.source == aurora_i18n::text("anim-couple-person-b")));
        assert!(!texts.iter().any(|t| t.source.contains("Very Long Resident")));
    }

    #[test]
    fn a_switch_waits_while_the_edited_person_s_preview_is_building() {
        let lib = AvatarLibrary::load();
        let mut editor = couple_editor(&lib);
        // a key was just recorded: the preview no longer shows the document
        editor.clip.changed();
        assert!(!editor.clip.can_switch_person());
        editor.switch_person();
        let couple = editor.clip.couple.as_ref().expect("couple");
        assert!(!couple.editing_b && couple.pending_switch);
        assert_eq!(editor.clip.document.as_ref().map(|d| d.name.as_str()), Some("Hug"));
    }

    #[test]
    fn the_person_set_aside_shows_their_own_keys_at_the_shared_playhead() {
        let lib = AvatarLibrary::load();
        let mut editor = couple_editor(&lib);
        editor.clip.cursor = 0.0;
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(1);
        let (id, request) = editor.waiting_request(&world).expect("B is shown");
        assert_eq!(id, Uuid::from_u128(42));
        assert!(request.active && request.suppress_live);
        let sample = request.animations.first().expect("B's sample");
        let b = editor
            .clip
            .couple
            .as_ref()
            .and_then(|c| c.other.preview.as_ref())
            .expect("B's preview");
        assert!(Arc::ptr_eq(&sample.animation, &b.bound));
        // editing B, A is the one set aside
        editor.switch_person();
        assert_eq!(editor.waiting_request(&world).map(|(id, _)| id), Some(world.agent_id));
        assert_eq!(editor.edited_subject(&world), Uuid::from_u128(42));
    }

    #[test]
    fn ending_couple_mode_returns_to_a_and_keeps_b_among_the_drafts() {
        let lib = AvatarLibrary::load();
        let mut editor = couple_editor(&lib);
        editor.switch_person();
        editor.end_couple();
        assert!(editor.clip.couple.is_none());
        assert_eq!(editor.clip.document.as_ref().map(|d| d.name.as_str()), Some("Hug"));
        assert!(editor.clip.history.iter().any(|d| d.name == "Hug B"));
    }

    #[test]
    fn the_placement_note_says_where_b_stands_from_a() {
        let a = Mat4::from_rotation_translation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), Vec3::new(10.0, 20.0, 30.0));
        // one metre in front of A (A faces +Y in the region), turned to face A
        let b = Mat4::from_rotation_translation(Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2), Vec3::new(10.0, 21.0, 30.0));
        let (position, yaw) = placement(a, b).expect("placement");
        assert!(position.abs_diff_eq(Vec3::new(1.0, 0.0, 0.0), 1e-5), "{position}");
        assert!((yaw.abs() - 180.0).abs() < 1e-3, "{yaw}");
        let note = placement_note("Hug", position, yaw);
        assert!(note.contains("Hug A") && note.contains("Hug B"));
        assert!(note.contains("position <1.000, 0.000, 0.000> m"));
    }

    #[test]
    fn lanes_show_each_track_s_keys_as_frames() {
        let lib = AvatarLibrary::load();
        let (b, _) = keyed(&lib, "Hug B", 0.2);
        let lanes = lanes_of(&b);
        let head = lanes.iter().find(|lane| lane.name == "mHead").expect("head lane");
        assert_eq!(head.keys, vec![0]);
    }
}

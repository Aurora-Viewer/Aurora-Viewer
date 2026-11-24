//! LiveMesh's Fit lab: a row of mannequins in front of us, each a
//! client-only copy of our avatar and of what it wears (the mesh body, head
//! and hands, the LiveMesh preview garment), in a shape of its own, to see
//! how a garment fits several bodies at once.
//!
//! The copies are made like the animation editor's mirror (mirror.rs):
//! `objects::PREVIEW_REGION`, ids of their own, so that the scene gives each
//! mannequin its own skeleton, palette and pose, while the mesh geometry
//! and skin binds stay shared with our worn objects. Their bakes are ours
//! (`appearance_of`); their shape is ours with the body sliders of their
//! `Body` changed. A worn object that changes (a new LiveMesh revision, an
//! alpha cut from a body's HUD) is copied again on the next frame.
//!
//! Nothing about them is sent to the region and nothing is saved: no one
//! else sees them. The Legacy Viewer has no such tool. Rule chosen by the
//! human (2026-11-23): any body the agent wears may dress the mannequins,
//! the same right it needs to wear it; nothing leaves this viewer.

use super::*;
use objects::PREVIEW_REGION;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

/// The first local id of the first mannequin, clear of the mirror's
/// (0xFFF0_0000) and of the avatar cache's copies (0xFFE0_0000…).
const FIRST_LOCAL: u32 = 0xFFF1_0000;
/// Local ids per mannequin: its avatar and up to 4095 worn prims.
const LOCALS_EACH: u32 = 0x1000;
/// How far in front of us the row stands (metres).
const DISTANCE: f32 = 2.2;
/// Between two mannequins of the row (metres).
const SPACING: f32 = 1.0;

/// A mannequin's body: our shape with these body sliders changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Body {
    /// Our own shape, unchanged.
    Me,
    Slim,
    Curvy,
    Full,
    Athletic,
    Tall,
    Petite,
}

impl Body {
    pub const ALL: [Body; 7] = [
        Body::Me,
        Body::Slim,
        Body::Curvy,
        Body::Full,
        Body::Athletic,
        Body::Tall,
        Body::Petite,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Body::Me => aurora_i18n::tr!("fit-lab-body-me"),
            Body::Slim => aurora_i18n::tr!("fit-lab-body-slim"),
            Body::Curvy => aurora_i18n::tr!("fit-lab-body-curvy"),
            Body::Full => aurora_i18n::tr!("fit-lab-body-full"),
            Body::Athletic => aurora_i18n::tr!("fit-lab-body-athletic"),
            Body::Tall => aurora_i18n::tr!("fit-lab-body-tall"),
            Body::Petite => aurora_i18n::tr!("fit-lab-body-petite"),
        }
    }

    /// The sliders it sets, as (avatar_lad.xml id, position of the slider
    /// from 0 to 1), the sliders of the shape editor's Body, Torso and Legs.
    /// The fitted-mesh collision volumes follow them, as on a resident with
    /// that shape.
    fn sliders(self) -> &'static [(i32, f32)] {
        // 33 height, 34 body thickness, 637 body fat; 36 shoulders,
        // 105 breast size, 507 breast buoyancy, 157 belly size,
        // 649 torso muscles, 756 love handles; 37 hip width, 652 leg
        // muscles, 692 leg length, 753 saddle bags, 795 butt size
        match self {
            Body::Me => &[],
            Body::Slim => &[
                (34, 0.25),
                (637, 0.0),
                (649, 0.35),
                (652, 0.35),
                (105, 0.3),
                (795, 0.15),
                (37, 0.35),
                (753, 0.2),
                (157, 0.0),
                (756, 0.2),
            ],
            Body::Curvy => &[
                (105, 0.75),
                (507, 0.6),
                (795, 0.75),
                (37, 0.7),
                (753, 0.45),
                (756, 0.45),
                (637, 0.3),
                (34, 0.5),
                (652, 0.55),
                (157, 0.1),
            ],
            Body::Full => &[
                (637, 0.85),
                (34, 0.8),
                (157, 0.6),
                (756, 0.75),
                (753, 0.6),
                (795, 0.65),
                (105, 0.7),
                (649, 0.6),
                (652, 0.7),
                (37, 0.65),
            ],
            Body::Athletic => &[(649, 0.9), (652, 0.85), (36, 0.7), (637, 0.05), (34, 0.5), (157, 0.0), (756, 0.15)],
            Body::Tall => &[(33, 0.9), (692, 0.7)],
            Body::Petite => &[(33, 0.15), (692, 0.35), (36, 0.35)],
        }
    }
}

/// What the mannequins play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Motion {
    #[default]
    Stand,
    Walk,
    Sit,
    /// The range of motion (fit_poses.rs): a pose held, or all in turn.
    Range(Option<super::fit_poses::Pose>),
    /// What our avatar plays (its AO, a pose, a dance).
    LikeMe,
}

impl Motion {
    /// The choices, a range of motion as all its poses in turn.
    pub const ALL: [Motion; 5] = [Motion::Stand, Motion::Walk, Motion::Sit, Motion::Range(None), Motion::LikeMe];

    pub fn label(self) -> &'static str {
        match self {
            Motion::Stand => aurora_i18n::tr!("fit-lab-motion-stand"),
            Motion::Walk => aurora_i18n::tr!("fit-lab-motion-walk"),
            Motion::Sit => aurora_i18n::tr!("fit-lab-motion-sit"),
            Motion::Range(_) => aurora_i18n::tr!("fit-lab-motion-range"),
            Motion::LikeMe => aurora_i18n::tr!("fit-lab-motion-like-me"),
        }
    }

    /// The stock animation it plays; `None`: ours.
    fn animation(self) -> Option<Uuid> {
        match self {
            Motion::Stand => Some(crate::own_motion::ANIM_STAND),
            Motion::Walk => Some(crate::own_motion::ANIM_WALK),
            Motion::Sit => Some(super::body::ANIM_SIT_GROUND_CONSTRAINED),
            Motion::Range(pose) => Some(super::fit_poses::animation_id(pose)),
            Motion::LikeMe => None,
        }
    }
}

/// One mannequin and its copies.
#[derive(Debug, Clone)]
struct Mannequin {
    id: Uuid,
    body: Body,
    /// Its first local id: its avatar; its copies follow.
    base: u32,
    next: u32,
    /// Its AvatarAppearance values: ours with its sliders changed.
    params: Vec<u8>,
    /// Moves when `params` change (the scene rebuilds its skeleton).
    generation: u64,
    /// Our worn object → its copy's local id and the source's signature
    /// when it was copied.
    copies: HashMap<ObjKey, (u32, u64)>,
    animations: Vec<PlayingAnimation>,
}

/// The Fit lab's mannequins.
#[derive(Debug, Default)]
pub struct FitLab {
    mannequins: Vec<Mannequin>,
    motion: Motion,
    /// The region they were placed in: a region change takes them away.
    region: Option<RegionHandle>,
    /// Our appearance generation their params were made from.
    from_generation: Option<u64>,
    /// The motion their animations were set for.
    motion_set: Option<Motion>,
}

impl FitLab {
    fn get(&self, id: &Uuid) -> Option<&Mannequin> {
        self.mannequins.iter().find(|m| m.id == *id)
    }

    /// The mannequins' bodies, in their order in the row.
    pub fn bodies(&self) -> Vec<Body> {
        self.mannequins.iter().map(|m| m.body).collect()
    }
}

/// A mannequin's avatar id: fixed for the agent and its place in the row,
/// so that one made again finds the scene's state of the last one.
pub fn mannequin_id(agent: Uuid, index: usize) -> Uuid {
    Uuid::from_u128(agent.as_u128() ^ 0x6669_746c_6162_0000_0000_0000_0000_0000 ^ (index as u128 + 1))
}

/// The signature of a worn object: it is copied again when it changes.
fn signature(world: &World, object: &objects::Object) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    object.full_id.hash(&mut h);
    object.full_update_serial.hash(&mut h);
    object.parent_key().hash(&mut h);
    object.state.hash(&mut h);
    object.te.as_ref().map(|te| Arc::as_ptr(te) as usize).hash(&mut h);
    for v in [object.position.to_array(), object.scale.to_array()] {
        v.map(f32::to_bits).hash(&mut h);
    }
    object.rotation.to_array().map(f32::to_bits).hash(&mut h);
    world
        .gltf_overrides
        .get(&object.key)
        .map(|sides| Arc::as_ptr(sides) as *const u8 as usize)
        .hash(&mut h);
    h.finish()
}

impl World {
    pub fn is_mannequin(&self, id: &Uuid) -> bool {
        self.fit_lab.get(id).is_some()
    }

    /// A client-only copy of our avatar: the mirror or a mannequin. Lists of
    /// residents (radar, minimap, contact markers…) leave them out.
    pub fn is_local_copy(&self, id: &Uuid) -> bool {
        self.is_mirror(id) || self.is_mannequin(id)
    }

    /// A mannequin's AvatarAppearance values, while it has them.
    pub(super) fn mannequin_params(&self, id: &Uuid) -> Option<&[u8]> {
        self.fit_lab.get(id).map(|m| m.params.as_slice()).filter(|p| !p.is_empty())
    }

    pub(super) fn mannequin_generation(&self, id: &Uuid) -> Option<u64> {
        self.fit_lab.get(id).map(|m| m.generation)
    }

    /// What a mannequin plays: its stock animation, or ours, over the
    /// pelvis anchor (`fit_poses::anchor`).
    pub(super) fn mannequin_animations(&self, id: &Uuid) -> Option<&[PlayingAnimation]> {
        self.fit_lab.get(id).map(|m| m.animations.as_slice())
    }

    /// Make the row match `bodies` (in that order) and `motion`, then keep
    /// each mannequin dressed as we are. Called every frame while the Fit
    /// lab is on; an empty list takes the mannequins away. `place` puts the
    /// row in front of us again.
    pub fn update_fit_lab(&mut self, bodies: &[Body], motion: Motion, place: bool, now: Instant) {
        if self.fit_lab.region.is_some() && self.fit_lab.region != self.main_region {
            self.remove_fit_lab();
        }
        if bodies.is_empty() {
            self.remove_fit_lab();
            return;
        }
        let Some(own) = self.objects.index_of_uuid(&self.agent_id) else {
            self.remove_fit_lab();
            return;
        };
        // a new row when the bodies change; the same ids come back, so
        // the scene finds its states again
        if self.fit_lab.bodies() != bodies {
            self.remove_fit_lab();
            self.fit_lab.region = self.main_region;
            for (index, &body) in bodies.iter().enumerate() {
                let base = FIRST_LOCAL + index as u32 * LOCALS_EACH;
                self.fit_lab.mannequins.push(Mannequin {
                    id: mannequin_id(self.agent_id, index),
                    body,
                    base,
                    next: base + 1,
                    params: Vec::new(),
                    generation: 1,
                    copies: HashMap::new(),
                    animations: Vec::new(),
                });
            }
            self.fit_lab.from_generation = None;
            self.fit_lab.motion_set = None;
            self.place_fit_lab(own);
            log::info!("LiveMesh Fit lab: {} mannequins dressed as our avatar", bodies.len());
        } else if place {
            self.place_fit_lab(own);
        }
        self.fit_lab.motion = motion;
        // ours, again each frame while they play what we play
        if self.fit_lab.motion_set != Some(motion) || motion == Motion::LikeMe {
            let first = self.fit_lab.motion_set.is_none();
            self.fit_lab.motion_set = Some(motion);
            let anchor = self.fit_lab.mannequins.first().and_then(|m| {
                m.animations
                    .iter()
                    .find(|a| a.id == super::fit_poses::ANCHOR)
                    .copied()
                    .filter(|_| !first)
            });
            let mut playing: Vec<_> = match motion.animation() {
                Some(id) => vec![PlayingAnimation::from_signal(id, 1, now, None)],
                None => self.animations_of(&self.agent_id).to_vec(),
            };
            playing.push(anchor.unwrap_or_else(|| PlayingAnimation::from_signal(super::fit_poses::ANCHOR, 1, now, None)));
            for m in &mut self.fit_lab.mannequins {
                m.animations.clone_from(&playing);
            }
        }
        self.shape_mannequins();
        self.dress_mannequins(own);
    }

    /// Stand the row in front of us, facing us.
    fn place_fit_lab(&mut self, own: usize) {
        let Some(avatar) = self.objects.get(own) else {
            return;
        };
        let facing = self.agent.body_rotation();
        let count = self.fit_lab.mannequins.len();
        let roots: Vec<_> = self
            .fit_lab
            .mannequins
            .iter()
            .enumerate()
            .map(|(index, m)| {
                let side = (index as f32 - (count as f32 - 1.0) / 2.0) * SPACING;
                let mut root = super::mirror::copy_of(avatar, m.id, m.base, 0);
                root.position = self.agent.position + facing * Vec3::new(DISTANCE, -side, 0.0);
                root.rotation = facing * glam::Quat::from_rotation_z(std::f32::consts::PI);
                root
            })
            .collect();
        for root in roots {
            self.objects.upsert_preview(None, root);
        }
    }

    /// Give each mannequin our shape with its sliders, again when ours
    /// changes (an edit in the shape editor shows on them too).
    fn shape_mannequins(&mut self) {
        let generation = self.appearance_generation(&self.agent_id);
        if self.fit_lab.from_generation == Some(generation) {
            return;
        }
        let shape = &self.avatar_lib.shape_params;
        // (before our appearance comes, ours is drawn in the default shape:
        // theirs start from it too)
        let ours = self
            .visual_params(&self.agent_id)
            .map_or_else(|| shape.values_with(&[]), <[u8]>::to_vec);
        self.fit_lab.from_generation = Some(generation);
        for m in &mut self.fit_lab.mannequins {
            let changed: BTreeMap<i32, f32> = m
                .body
                .sliders()
                .iter()
                .filter_map(|&(id, at)| shape.range(id).map(|(min, max, _)| (id, min + at.clamp(0.0, 1.0) * (max - min))))
                .collect();
            m.params = if changed.is_empty() {
                ours.clone()
            } else {
                shape.edit_values(&ours, &changed)
            };
            m.generation += 1;
        }
    }

    /// Copy to each mannequin what we wear and changed since its last copy,
    /// parents before their children, and take away the copies of what we
    /// no longer wear. Not the HUDs (they would show twice on the screen),
    /// nor the avatar cache's copies (they go when the region's come).
    fn dress_mannequins(&mut self, own: usize) {
        let Some(own_key) = self.objects.get(own).map(|o| o.key) else {
            return;
        };
        let worn: Vec<(usize, ObjKey, u64)> = self
            .objects
            .subtree(own)
            .into_iter()
            .skip(1)
            .filter_map(|idx| {
                let object = self.objects.get(idx)?;
                if super::avatar_cache::is_copy(object.key) {
                    return None;
                }
                let parent = object.parent_key()?;
                if parent == own_key && (object.is_avatar() || (31..=38).contains(&object.attachment_point())) {
                    return None;
                }
                Some((idx, object.key, signature(self, object)))
            })
            .collect();
        let mut updates = Vec::new();
        let mut gone = Vec::new();
        let mut overrides = Vec::new();
        for m in &mut self.fit_lab.mannequins {
            let mut kept = HashSet::new();
            for &(idx, key, sig) in &worn {
                let Some(object) = self.objects.get(idx) else {
                    continue;
                };
                let Some(parent_key) = object.parent_key() else {
                    continue;
                };
                let parent = if parent_key == own_key {
                    m.base
                } else if let Some(&(local, _)) = m.copies.get(&parent_key) {
                    local
                } else {
                    // its parent was left out: so is it
                    continue;
                };
                kept.insert(key);
                let local = match m.copies.get(&key) {
                    Some(&(_, before)) if before == sig => continue,
                    Some(&(local, _)) => local,
                    None if m.next < m.base + LOCALS_EACH => {
                        m.next += 1;
                        m.next - 1
                    }
                    None => continue,
                };
                m.copies.insert(key, (local, sig));
                let full_id = Uuid::from_u128(m.id.as_u128() ^ object.full_id.as_u128().rotate_left(1));
                let copy_key = ObjKey {
                    region: PREVIEW_REGION,
                    local_id: local,
                };
                overrides.push((copy_key, self.gltf_overrides.get(&key).cloned()));
                updates.push((
                    ObjKey {
                        region: PREVIEW_REGION,
                        local_id: parent,
                    },
                    super::mirror::copy_of(object, full_id, local, parent),
                ));
            }
            m.copies.retain(|key, (local, _)| {
                let keep = kept.contains(key);
                if !keep {
                    gone.push(*local);
                }
                keep
            });
        }
        for (parent, update) in updates {
            self.objects.upsert_preview(Some(parent), update);
        }
        for (key, sides) in overrides {
            match sides {
                Some(sides) => self.gltf_overrides.insert(key, sides),
                None => self.gltf_overrides.remove(&key),
            };
        }
        if !gone.is_empty() {
            for &local in &gone {
                self.gltf_overrides.remove(&ObjKey {
                    region: PREVIEW_REGION,
                    local_id: local,
                });
            }
            self.kill_objects(PREVIEW_REGION, &gone);
        }
    }

    /// The clipping map's avatars, ours and the mannequins, and the
    /// garments on them: `garments` (the LiveMesh objects we wear) and
    /// their copies (object indices).
    pub fn fit_clip_targets(&self, garments: &[Uuid]) -> (Vec<usize>, Vec<usize>) {
        let avatars = std::iter::once(self.agent_id)
            .chain(self.fit_lab.mannequins.iter().map(|m| m.id))
            .filter_map(|id| self.objects.index_of_uuid(&id))
            .collect();
        let mut worn = Vec::new();
        for idx in garments.iter().filter_map(|id| self.objects.index_of_uuid(id)) {
            worn.push(idx);
            let Some(key) = self.objects.get(idx).map(|o| o.key) else {
                continue;
            };
            for m in &self.fit_lab.mannequins {
                if let Some(&(local, _)) = m.copies.get(&key)
                    && let Some(copy) = self.objects.index_of(&ObjKey {
                        region: PREVIEW_REGION,
                        local_id: local,
                    })
                {
                    worn.push(copy);
                }
            }
        }
        (avatars, worn)
    }

    /// Take the mannequins away, with what they wear.
    pub fn remove_fit_lab(&mut self) {
        let mannequins = std::mem::take(&mut self.fit_lab.mannequins);
        self.fit_lab.region = None;
        self.fit_lab.from_generation = None;
        if mannequins.is_empty() {
            return;
        }
        for m in &mannequins {
            for (local, _) in m.copies.values() {
                self.gltf_overrides.remove(&ObjKey {
                    region: PREVIEW_REGION,
                    local_id: *local,
                });
            }
        }
        let roots: Vec<u32> = mannequins.iter().map(|m| m.base).collect();
        self.kill_objects(PREVIEW_REGION, &roots);
        log::info!("LiveMesh Fit lab: mannequins removed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HAT: u32 = 70_001;

    /// Our avatar with a hat (skull) and a HUD, and our shape.
    fn world_with_avatar() -> (World, RegionHandle) {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let me = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0001);
        world.agent_id = me;
        let (handle, avatar) = crate::demo::events()
            .into_iter()
            .find_map(|ev| match ev {
                aurora_net::NetEvent::ObjectUpdates { handle, objects } => {
                    objects.into_iter().find(|o| o.full_id == me).map(|o| (handle, o))
                }
                _ => None,
            })
            .expect("demo contains our avatar");
        world.main_region = Some(handle);
        world.ensure_region(handle, (256, 256));
        let worn = |local_id: u32, point: u8| {
            let mut prim = avatar.clone();
            prim.pcode = 9;
            prim.local_id = local_id;
            prim.full_id = Uuid::from_u128(local_id as u128);
            prim.parent_id = avatar.local_id;
            prim.state = (point & 0x0F) << 4 | (point >> 4);
            prim.position = Vec3::ZERO;
            prim
        };
        let objects = vec![avatar.clone(), worn(HAT, 2), worn(70_002, 35)];
        world.apply(aurora_net::NetEvent::ObjectUpdates { handle, objects });
        world.shape_preview = Some(world.avatar_lib.shape_params.values_with(&[]));
        world.shape_preview_generation = 1;
        (world, handle)
    }

    fn worn_points(world: &World, id: &Uuid) -> Vec<u8> {
        let idx = world.objects.index_of_uuid(id).expect("drawn");
        let key = world.objects.get(idx).expect("mannequin").key;
        world
            .objects
            .children_of(&key)
            .iter()
            .filter_map(|&i| world.objects.get(i))
            .map(|o| o.attachment_point())
            .collect()
    }

    fn thickness(world: &World, id: &Uuid) -> f32 {
        let values = world.visual_params(id).expect("a shape");
        world.avatar_lib.shape_params.effective_weights(values)[&34]
    }

    #[test]
    fn the_fit_lab_dresses_each_mannequin_as_us_in_its_own_shape() {
        let (mut world, _) = world_with_avatar();
        let now = Instant::now();
        world.update_fit_lab(&[Body::Me, Body::Slim, Body::Full], Motion::Stand, false, now);
        let ids: Vec<Uuid> = (0..3).map(|i| mannequin_id(world.agent_id, i)).collect();
        for id in &ids {
            assert!(world.is_mannequin(id) && world.is_local_copy(id));
            let idx = world.objects.index_of_uuid(id).expect("drawn");
            let o = world.objects.get(idx).expect("mannequin");
            assert!(o.is_avatar() && o.is_client_preview() && o.parent_id == 0);
            assert_eq!(worn_points(&world, id), vec![2], "the hat, not the HUD");
            assert_eq!(world.appearance_of(id), world.agent_id, "our bakes");
            let playing: Vec<Uuid> = world.animations_of(id).iter().map(|a| a.id).collect();
            assert_eq!(
                playing,
                vec![crate::own_motion::ANIM_STAND, super::super::fit_poses::ANCHOR],
                "a stand of its own, over the pelvis anchor"
            );
        }
        let me = world.agent_id;
        assert_eq!(world.visual_params(&ids[0]), world.visual_params(&me), "Me: our shape");
        assert!(thickness(&world, &ids[1]) < thickness(&world, &me), "Slim is thinner");
        assert!(thickness(&world, &ids[2]) > thickness(&world, &me), "Full figure is thicker");
        // a change of our shape reaches them, with a new skeleton
        let before = world.appearance_generation(&ids[1]);
        world.shape_preview_generation += 1;
        world.update_fit_lab(&[Body::Me, Body::Slim, Body::Full], Motion::Stand, false, now);
        assert!(world.appearance_generation(&ids[1]) > before + 1);
        // Like me: they play what we play, over the anchor that brings the
        // pelvis back after a sit
        world.update_fit_lab(&[Body::Me, Body::Slim, Body::Full], Motion::LikeMe, false, now);
        let ours = world.animations_of(&me).to_vec();
        let theirs = world.animations_of(&ids[1]);
        assert_eq!(&theirs[..ours.len()], ours.as_slice());
        assert_eq!(theirs.last().map(|a| a.id), Some(super::super::fit_poses::ANCHOR));
        // none: they go, with what they wear
        world.update_fit_lab(&[], Motion::Stand, false, now);
        assert!(
            ids.iter()
                .all(|id| world.objects.index_of_uuid(id).is_none() && !world.is_mannequin(id))
        );
        assert_eq!(world.objects.avatars().count(), 1);
        assert!(world.objects.iter().all(|(_, o)| !o.is_client_preview()));
    }

    #[test]
    fn the_clipping_map_takes_our_garment_and_its_copies_on_us_and_the_mannequins() {
        let (mut world, handle) = world_with_avatar();
        world.update_fit_lab(&[Body::Slim, Body::Full], Motion::Stand, false, Instant::now());
        let hat = world
            .objects
            .index_of(&ObjKey {
                region: handle,
                local_id: HAT,
            })
            .expect("hat");
        let garment = world.objects.get(hat).expect("hat").full_id;
        let (avatars, garments) = world.fit_clip_targets(&[garment]);
        let ours = world.objects.index_of_uuid(&world.agent_id).expect("us");
        assert_eq!(avatars.len(), 3);
        assert_eq!(avatars[0], ours);
        assert_eq!(garments.len(), 3, "the hat and its two copies");
        assert_eq!(garments[0], hat);
        for (&copy, &avatar) in garments[1..].iter().zip(&avatars[1..]) {
            let parent = world.objects.get(copy).and_then(|o| o.parent_key());
            assert_eq!(parent, world.objects.get(avatar).map(|o| o.key), "worn by its mannequin");
        }
        // without mannequins: our avatar and our garment alone
        world.update_fit_lab(&[], Motion::Stand, false, Instant::now());
        assert_eq!(world.fit_clip_targets(&[garment]), (vec![ours], vec![hat]));
    }

    #[test]
    fn a_worn_object_that_changes_is_copied_again_and_one_taken_off_goes() {
        let (mut world, handle) = world_with_avatar();
        let now = Instant::now();
        world.update_fit_lab(&[Body::Curvy], Motion::Stand, false, now);
        let id = mannequin_id(world.agent_id, 0);
        let hat = world
            .objects
            .index_of(&ObjKey {
                region: handle,
                local_id: HAT,
            })
            .expect("hat");
        // an alpha cut from a HUD: a new texture entry on the worn object
        let mut te = world
            .objects
            .get(hat)
            .and_then(|o| o.te.as_deref().cloned())
            .expect("a texture entry");
        te.faces.truncate(1);
        let te = Arc::new(te);
        world.objects.get_mut(hat).expect("hat").te = Some(te.clone());
        world.update_fit_lab(&[Body::Curvy], Motion::Stand, false, now);
        let mannequin = world
            .objects
            .index_of_uuid(&id)
            .and_then(|i| world.objects.get(i))
            .expect("drawn")
            .key;
        let copy = world.objects.children_of(&mannequin)[0];
        assert_eq!(world.objects.get(copy).and_then(|o| o.te.clone()), Some(te), "the new alpha cut");
        // the hat taken off: its copy goes, the mannequin stays
        world.kill_objects(handle, &[HAT]);
        world.update_fit_lab(&[Body::Curvy], Motion::Stand, false, now);
        assert!(worn_points(&world, &id).is_empty());
        assert!(world.is_mannequin(&id));
    }
}

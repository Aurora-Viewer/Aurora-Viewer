//! Cloth physics for worn rigged meshes (Aurora's own; the Legacy Viewer has
//! none): skirts, long hair, capes and tails swing with the avatar's motion.
//!
//! Nothing comes from the creator. Each avatar's rigged meshes are sorted by
//! kind (hair, tails, clothing, close-fitting clothing that only sways at
//! its hems) and the meshes of one kind are simulated as one cloth: pieces
//! of a garment (a dress in segments, a mermaid's tail and fins) meet at
//! their seams, welded there, and do not part when it swings. For each
//! rigged mesh worn:
//!
//! 1. **Rest** (a background job, once per mesh): decoded as the GPU holds it
//!    (`jobs::mesh_to_faces`, so the vertex order is the GPU's) and placed on
//!    a default avatar as the shader skins it at rest. A mesh that hugs the
//!    body there is *body-like*: a mesh body or one of its segments, a head,
//!    hands, feet, fitted clothes. It never swings.
//! 2. **Fit** (once per outfit, per avatar): the body-like meshes the avatar
//!    wears fit its collision volumes to the body actually worn (a Reborn's
//!    hips, a slim body): `weights::fit_scales`.
//! 3. **Weigh** (a background job, once per group of meshes and fit): each
//!    vertex's freedom against that body (weights.rs). Again when the group
//!    or the outfit's body changes; the cloth keeps its old weights while
//!    only the body changes.
//!
//! At each frame the cloth is simulated in world space (sim.rs) from the
//! skeleton as posed for the GPU, against the fitted volumes; each vertex's
//! offset from its skinned place, taken back to the mesh's bind space, goes
//! into palette slots reserved for it, and the record of each face points at
//! them (`DrawRecord::identity` y: first vector, z: the face's first vertex,
//! w: its vertex count). The vertex shader adds the offset before skinning
//! (`cloth_delta`, object.wgsl), in every pass.
//!
//! Prototype, on the CPU: `AURORA_CLOTH=1` our own avatar, `AURORA_CLOTH=all`
//! also the `MAX_OTHERS` nearest avatars within `OTHERS_RANGE`.

mod coarse;
mod sim;
mod weights;

use super::anim::{ANCHOR_BASE, PALETTE_JOINTS};
use super::{GeomKey, Scene};
use crate::world::World;
use aurora_render::MeshAlloc;
use crossbeam_channel::{Receiver, Sender};
use glam::{Mat4, Vec3};
use sim::{BindData, ClothMesh, ClothState, NodeSkin, Volume};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Off,
    /// Our own avatar.
    Own,
    /// Ours and the nearest others.
    All,
}

/// `AURORA_CLOTH=1` (or `own`): cloth physics on our own avatar's rigged
/// attachments; `all`: on the nearest avatars' too; `0`: none. On, ours,
/// with its demo scenario (`AURORA_DEMO_CLOTH`). Over the preference.
fn env_mode() -> Option<Mode> {
    static MODE: std::sync::OnceLock<Option<Mode>> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_CLOTH").as_deref() {
        Ok("all") => Some(Mode::All),
        Ok("1" | "own") => Some(Mode::Own),
        Ok("0" | "off") => Some(Mode::Off),
        _ if std::env::var("AURORA_DEMO_CLOTH").is_ok() => Some(Mode::Own),
        _ => None,
    })
}

/// What swings, from the preferences (`Settings::cloth_*`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Config {
    pub mode: Mode,
    pub hair: bool,
    pub clothing: bool,
    pub tails: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Off,
            hair: true,
            clothing: true,
            tails: true,
        }
    }
}

impl Config {
    /// `enabled`, `nearby`: the preferences (`AURORA_CLOTH` over them).
    pub fn new(enabled: bool, nearby: bool, hair: bool, clothing: bool, tails: bool) -> Self {
        let mode = env_mode().unwrap_or(match (enabled, nearby) {
            (false, _) => Mode::Off,
            (true, false) => Mode::Own,
            (true, true) => Mode::All,
        });
        Self {
            mode,
            hair,
            clothing,
            tails,
        }
    }
}

/// Other avatars simulated at most (the nearest), and how near they must be.
const MAX_OTHERS: usize = 8;
const OTHERS_RANGE: f32 = 30.0;
/// The budget of the other avatars' cloth (`budget`): simulated points at
/// most per avatar and in all, given to the nearest first. Ours is never
/// cut. (With a dress of 21 000 points and two hairs of 4 000, the cloth of
/// a busy place took 3 to 20 ms a frame before it went on every core.)
const AVATAR_POINTS: usize = 12_000;
const OTHERS_POINTS: usize = 30_000;
/// A cloth swinging already counts as this much nearer than it is: walking
/// past someone at the budget's edge does not switch it on and off.
const BUDGET_KEEP: f32 = 3.0;

/// A group the budget weighs (`budget`).
#[derive(Clone, Copy, Debug)]
struct Candidate {
    slot: GroupSlot,
    /// Worn by our own avatar.
    own: bool,
    /// Its wearer's distance from ours (m).
    distance: f32,
    /// Its simulated points (0 while not weighed yet).
    points: usize,
    /// Swinging now.
    active: bool,
}

/// The groups that may swing: all of ours, then the other avatars' nearest
/// first (a swinging one `BUDGET_KEEP` nearer), within `AVATAR_POINTS` each
/// and `OTHERS_POINTS` in all. A group that does not fit is drawn rigid,
/// others of the same avatar may still swing (its hair when its dress is
/// too big). Avatars too complex to be drawn have no cloth at all
/// (`update_cloth_avatars`).
fn budget(candidates: &[Candidate]) -> HashSet<GroupSlot> {
    let near = |c: &Candidate| c.distance - if c.active { BUDGET_KEEP } else { 0.0 };
    let mut order: Vec<&Candidate> = candidates.iter().collect();
    order.sort_by(|a, b| b.own.cmp(&a.own).then(near(a).total_cmp(&near(b))).then(a.slot.cmp(&b.slot)));
    let mut out = HashSet::new();
    let mut total = 0;
    let mut per: HashMap<Uuid, usize> = HashMap::new();
    for c in order {
        if !c.own {
            let mine = per.entry(c.slot.0).or_default();
            if *mine + c.points > AVATAR_POINTS || total + c.points > OTHERS_POINTS {
                continue;
            }
            *mine += c.points;
            total += c.points;
        }
        out.insert(c.slot);
    }
    out
}

/// Furthest a cloth may hang from the body at rest, and the longest it may
/// be: past these the mesh is not worn cloth (a vehicle, a scenery piece
/// rigged to the avatar, a broken upload) and stays rigid.
const MAX_BODY_GAP: f32 = 1.0;
const MAX_LENGTH: f32 = 3.0;
/// Points of the worn body kept for the fit (evenly thinned beyond).
const FIT_POINTS: usize = 40_000;
/// Simulated points a mesh may have on the CPU: a denser one stays rigid
/// (a dress of hundreds of thousands of vertices cost seconds a frame).
const MAX_NODES: usize = 60_000;
/// The lattice a cloth is simulated on (`coarse`): cells about this wide.
/// Finer than the folds a dress or a lock of hair swings in, coarse enough
/// that a dense dress is a few thousand points.
const CELL: f32 = 0.03;
/// How near the body (at rest) a vertex gets the backstop.
const BACKSTOP_RANGE: f32 = 0.15;

/// Skin joints whose vertices never swing: fingers, face, eyes, feet (and
/// the fitted-mesh volumes of hands and feet), the Bento wings and hind
/// limbs: struts and legs that animations move (dragged, a demon wing's
/// struts bent and crumpled).
fn never_swings(name: &str) -> bool {
    [
        "mHand",
        "mFace",
        "mEye",
        "mToe",
        "mFoot",
        "L_HAND",
        "R_HAND",
        "L_FOOT",
        "R_FOOT",
        "mWing",
        "mHindLimb",
    ]
    .iter()
    .any(|p| name.starts_with(p))
}

/// The Bento tail: animations sway it already. Simulated freely, the cloth
/// tugged against them and the tail shivered; its vertices only drag a
/// little behind the animation instead (a tuft), at most this free, on a
/// stiff spring. A tail rigged to the pelvis alone, never animated, swings.
fn animated(name: &str) -> bool {
    name.starts_with("mTail")
}
const ANIMATED_FREEDOM: f32 = 0.3;

/// A mesh that is never cloth, whatever it looks like: most of its
/// vertices skinned mostly to joints that never swing (`rigid`, per face
/// and vertex: wings flapping from their own animation, a pet or a prop
/// riding a Bento joint, gloves, shoes), or too few to be a garment (a
/// deformer: a few triangles that only move the skeleton).
fn never_cloth(rigid: &[Vec<bool>]) -> bool {
    let total: usize = rigid.iter().map(Vec::len).sum();
    let fixed = rigid.iter().flatten().filter(|&&r| r).count();
    total < MIN_VERTICES || fixed as f32 > NEVER_SHARE * total as f32
}
/// Fewer vertices than this: a deformer, not a garment.
const MIN_VERTICES: usize = 24;
/// Share of a mesh's vertices on joints that never swing past which it is
/// never cloth.
const NEVER_SHARE: f32 = 0.5;

/// Whether a vertex is skinned mostly (at least half its weight) to joints
/// of `list` (per joint). Rigid only then: a sweater's cuffs, partly on the
/// hands, were held whole and its sleeves never moved; a glove or a lock on
/// the face is mostly on them.
fn mostly_on(list: &[bool], s: &NodeSkin) -> bool {
    let (mut on, mut sum) = (0.0, 0.0);
    for k in 0..4 {
        let w = s.weights[k].max(0.0);
        sum += w;
        if list.get(s.joints[k] as usize).copied().unwrap_or(false) {
            on += w;
        }
    }
    sum > 1e-4 && on >= 0.5 * sum
}

/// The share of a vertex's skin weight on the tail's joints (`moved`: per
/// joint, whether it is one).
fn tail_share(moved: &[bool], s: &NodeSkin) -> f32 {
    let (mut on, mut sum) = (0.0, 0.0);
    for k in 0..4 {
        let w = s.weights[k].max(0.0);
        sum += w;
        if moved.get(s.joints[k] as usize).copied().unwrap_or(false) {
            on += w;
        }
    }
    if sum <= 1e-4 { 0.0 } else { (on / sum).clamp(0.0, 1.0) }
}
/// Tail joints still this long (relative to the pelvis) are not animated:
/// the tail swings free until an animation moves them again.
const STILL_FOR: f32 = 2.0;
/// Share of its vertices on those joints past which a whole mesh is held.
const ANIMATED_SHARE: f32 = 0.2;

/// Joints of the arms and legs (and their fitted-mesh volumes): hair is
/// never skinned to them, bodies and clothes are. (Not the collarbones: hair
/// lying on the shoulders may be.)
fn limb(name: &str) -> bool {
    if name.ends_with("_CLAVICLE") {
        return false;
    }
    [
        "mShoulder",
        "mElbow",
        "mWrist",
        "mHand",
        "mHip",
        "mKnee",
        "mAnkle",
        "mFoot",
        "mToe",
        "L_",
        "R_",
    ]
    .iter()
    .any(|p| name.starts_with(p))
}
/// Hair hangs at least this far below its top: a mesh head and its neck do
/// not.
const HAIR_LENGTH: f32 = 0.4;
/// Open-edge vertices a body-like mesh needs standing off the body to be
/// weighed for a sway.
const SWAY_MIN_EDGE: usize = 12;
/// Tails and clothes: separate pieces this near each other are stitched
/// (`weights::Params::stitch`): a mermaid's fins came apart from its tail.
const STITCH: f32 = 0.03;
/// Hair's top is at least this far above the neck joint (on the head), however
/// it is rigged (to the head, the neck, the spine, face joints...).
const HAIR_TOP: f32 = 0.1;
/// Hair is pinned only where it touches the body this far above the neck
/// joint: the head.
const SCALP: f32 = 0.05;
/// Hair (and headwear taken for it) never moves where it covers the crown:
/// within this of the head's volume, above its middle. Hair's cap does not
/// swing; a pelt's head worn as a headdress bounced and swelled on the head.
/// (Down to the neck, it held the hair beside the face too, and the hair
/// over the shoulders swung less.)
const HEAD_SHELL: f32 = 0.06;

type Key = (Uuid, u8);

/// A mesh as worn on a default avatar (step 1).
struct RestMesh {
    /// Per face, per GPU vertex: its rest place, its triangles, whether it
    /// is skinned to a joint that never swings.
    positions: Vec<Vec<Vec3>>,
    indices: Vec<Vec<u32>>,
    rigid: Vec<Vec<bool>>,
    /// Skinned to an animated appendage (`animated`).
    animated: Vec<Vec<bool>>,
    /// Per face, per GPU vertex: the share of its skin weight on the tail's
    /// joints (`animated`), 0..1.
    on_tail: Vec<Vec<f32>>,
    bind: BindData,
    /// Hugs the default body: part of the body worn.
    body_like: bool,
    /// Largely a Bento tail (`ANIMATED_SHARE`).
    tail: bool,
    /// Never cloth (`never_cloth`): wings, a pet or a prop on joints that
    /// never swing, a near-empty deformer.
    never: bool,
    /// Body-like, with open edges standing off the default body (a flared
    /// hem, a cuff): weighed to sway there (`weights::Params::sway`).
    sway: bool,
    /// Long hair: skinned to the head, not to any limb, hanging past the
    /// neck. It may lie on the body and still swing.
    hair: bool,
    /// Hair: the height (at rest) only its scalp reaches.
    scalp: f32,
    /// Its skin's joints (its matrices in a group's).
    joints: usize,
}

/// What an avatar's rigged mesh is to the cloth: the meshes of one kind
/// are simulated together.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
enum Kind {
    Hair,
    Tail,
    Clothing,
    /// Body-like, with hems standing off the body: sways there only.
    Fitted,
}

impl Kind {
    /// None: part of the body, or never cloth (`RestMesh::never`).
    fn of(rest: &RestMesh) -> Option<Kind> {
        if rest.never {
            return None;
        }
        if rest.body_like {
            return rest.sway.then_some(Kind::Fitted);
        }
        Some(if rest.hair {
            Kind::Hair
        } else if rest.tail {
            Kind::Tail
        } else {
            Kind::Clothing
        })
    }

    fn wanted(self, config: &Config) -> bool {
        match self {
            Kind::Hair => config.hair,
            Kind::Tail => config.tails,
            Kind::Clothing | Kind::Fitted => config.clothing,
        }
    }
}

enum Rest {
    Pending,
    Failed,
    Ready(Arc<RestMesh>),
}

enum Weighed {
    Pending,
    Rigid,
    Cloth(Arc<GroupMesh>),
}

/// The meshes of a group simulated as one (step 3).
struct GroupMesh {
    cloth: ClothMesh,
    /// Per member: its first vector in the group's offsets
    /// (`ClothState::deltas`' order) and how many.
    spans: Vec<(usize, usize)>,
}

/// An avatar's collision volumes fitted to the body it wears (step 2).
#[derive(Clone, Default)]
struct Fit {
    /// Its body-like meshes, sorted (what it was fitted from; their level
    /// of detail aside: a body piece changing level as the camera moves is
    /// the same body, and refitting each time re-weighed every cloth).
    body: Vec<Uuid>,
    /// Per collision volume (`Scene::cloth_volumes` order), its scale.
    scales: Arc<Vec<Vec3>>,
    /// 0: the default volumes.
    signature: u64,
    /// The body worn's surface (its points at rest).
    surface: Option<Arc<weights::Surface>>,
}

/// One worn object's place in the palettes (its vertices', whatever group
/// it is simulated in).
struct Worn {
    id: Uuid,
    owner: Uuid,
    key: Key,
    kind: Kind,
    /// Palette slots reserved for its offsets (consecutive).
    slots: std::ops::Range<u32>,
    /// Per face, its first vector in the palette buffer, and its vertices.
    faces: Vec<u32>,
    counts: Vec<u32>,
}

/// An avatar's meshes of one kind, simulated as one cloth.
struct Group {
    /// Its worn objects and their meshes, by object index.
    members: Vec<(usize, Key)>,
    /// The fit it was weighed for.
    signature: u64,
    mesh: Arc<GroupMesh>,
    state: ClothState,
    /// This frame's skin matrices (each member's after the one before),
    /// then offsets (stepped in parallel).
    mats: Vec<Mat4>,
    out: Vec<[f32; 4]>,
}

/// A group's weighing: its members' meshes (hashed) and the fit.
type GroupKey = (u64, u64);

/// A group: its avatar, its kind and, for close-fitting clothes, its one
/// object (each such mesh only sways at its own hems: grouped, an avatar's
/// forty body pieces made one cloth of 40 000 points weighed in a second).
type GroupSlot = (Uuid, Kind, usize);

fn group_slot(owner: Uuid, kind: Kind, idx: usize) -> GroupSlot {
    (owner, kind, if kind == Kind::Fitted { idx } else { usize::MAX })
}

type RestResult = (Key, Option<RestMesh>);
type WeighResult = (GroupKey, Option<GroupMesh>);

pub struct Cloth {
    config: Config,
    /// The config changed: every cloth recorded again.
    config_changed: bool,
    /// Per avatar: its tail joints relative to its pelvis last frame, and
    /// when they last moved.
    tails: HashMap<Uuid, (Vec<Mat4>, Instant)>,
    rest: HashMap<Key, Rest>,
    weighed: HashMap<GroupKey, Weighed>,
    rest_tx: Sender<RestResult>,
    rest_rx: Receiver<RestResult>,
    weigh_tx: Sender<WeighResult>,
    weigh_rx: Receiver<WeighResult>,
    /// Every rigged mesh seen worn by a simulated avatar, by object index:
    /// its object, its avatar, its mesh (the outfits the fits come from).
    outfit: HashMap<usize, (Uuid, Uuid, Key)>,
    fits: HashMap<Uuid, Fit>,
    /// The avatars simulated this frame.
    simulated: HashSet<Uuid>,
    /// By object index.
    worn: HashMap<usize, Worn>,
    /// By avatar, kind and `group_slot`.
    groups: HashMap<GroupSlot, Group>,
    /// Objects to record again when their mesh's rest or weights arrive.
    waiting: HashMap<Key, Vec<usize>>,
    last: Option<Instant>,
    /// The last `cloth motion` log line.
    logged: Option<Instant>,
    /// Groups the budget holds rigid (`budget`; diagnostics).
    held: usize,
}

impl Default for Cloth {
    fn default() -> Self {
        let (rest_tx, rest_rx) = crossbeam_channel::unbounded();
        let (weigh_tx, weigh_rx) = crossbeam_channel::unbounded();
        Self {
            rest: HashMap::new(),
            weighed: HashMap::new(),
            rest_tx,
            rest_rx,
            weigh_tx,
            weigh_rx,
            outfit: HashMap::new(),
            fits: HashMap::new(),
            simulated: HashSet::new(),
            worn: HashMap::new(),
            groups: HashMap::new(),
            waiting: HashMap::new(),
            last: None,
            logged: None,
            held: 0,
            tails: HashMap::new(),
            config: Config::default(),
            config_changed: false,
        }
    }
}

impl Cloth {
    /// The scene's palettes start again (logout, device lost): every slot
    /// reserved is gone. The meshes' rest and weights stay.
    pub fn clear(&mut self) {
        self.worn.clear();
        self.groups.clear();
        self.waiting.clear();
        self.outfit.clear();
        self.fits.clear();
        self.simulated.clear();
        self.last = None;
    }
}

/// Step 1, a background job: the mesh's level `lod` as the GPU holds it, on
/// a default avatar (`body`: its collision volumes at rest).
fn rest_job(
    key: Key,
    view: super::meshes::MeshView,
    lod: u8,
    skin: Arc<aurora_assets::SkinInfo>,
    rig: Arc<super::anim::Rig>,
    body: Vec<Mat4>,
) -> Option<RestMesh> {
    let actual = view.header.actual_lod(lod as usize)?;
    let range = view.header.lods.get(actual).copied().flatten()?;
    let section = view.bytes.section(range)?;
    let faces = aurora_assets::decode_mesh_lod(section).ok()?;
    let map: Vec<u8> = (0..skin.joint_names.len()).map(|i| i.min(255) as u8).collect();
    let (geoms, _, _) = super::jobs::mesh_to_faces(&faces, Some(skin.bind_shape), Some(&map));
    let fixed: Vec<bool> = skin.joint_names.iter().map(|n| never_swings(n)).collect();
    // hair is often rigged to the face's root (the head, for Bento): only
    // the face's own joints keep it still
    let fixed_hair: Vec<bool> = skin.joint_names.iter().map(|n| never_swings(n) && n != "mFaceRoot").collect();
    let mut rigid_hair: Vec<Vec<bool>> = Vec::with_capacity(geoms.len());
    let moved: Vec<bool> = skin.joint_names.iter().map(|n| animated(n)).collect();
    // The mesh as it stands on a default avatar: its bind space can be
    // anywhere (centimetres, metres away) and its inverse binds bring it on
    // the skeleton, as the shader's skinning does. Everything geometric (the
    // body, heights, lengths) is worked out there.
    let rest: Vec<Mat4> = skin
        .joint_names
        .iter()
        .zip(&skin.inverse_bind)
        .map(|(name, inv)| {
            let (joint, inv) = rig.skin_bind(name, *inv);
            rig.default_world_inv.get(joint).map_or(Mat4::IDENTITY, |m| m.inverse()) * inv
        })
        .collect();
    let rest_matrix = |s: &NodeSkin| {
        let mut m = Mat4::ZERO;
        let mut sum = 0.0;
        for k in 0..4 {
            if s.weights[k] > 0.0 {
                m += *rest.get(s.joints[k] as usize).unwrap_or(&Mat4::IDENTITY) * s.weights[k];
                sum += s.weights[k];
            }
        }
        if sum <= 1e-4 { Mat4::IDENTITY } else { m * (1.0 / sum) }
    };
    let body = weights::Body::new(body);
    let mut out = RestMesh {
        positions: Vec::with_capacity(geoms.len()),
        indices: Vec::with_capacity(geoms.len()),
        rigid: Vec::with_capacity(geoms.len()),
        animated: Vec::with_capacity(geoms.len()),
        on_tail: Vec::with_capacity(geoms.len()),
        bind: BindData::default(),
        body_like: false,
        tail: false,
        never: false,
        sway: false,
        hair: false,
        scalp: 0.0,
        joints: skin.joint_names.len(),
    };
    let limbs: Vec<bool> = skin.joint_names.iter().map(|n| limb(n)).collect();
    let mut on_limb = false;
    let neck = rig
        .names
        .get("mNeck")
        .and_then(|&j| rig.default_world_inv.get(j))
        .map_or(1.5, |m| m.inverse().w_axis.z);
    // (diagnostics: per joint, its vertices)
    let mut joint_count = vec![0usize; skin.joint_names.len()];
    // (diagnostics: per limb joint, its vertices)
    let mut limb_count = vec![0usize; skin.joint_names.len()];
    for g in &geoms {
        let Some(g) = g else {
            // (a face the GPU does not hold either: no vertex)
            out.positions.push(Vec::new());
            out.indices.push(Vec::new());
            out.rigid.push(Vec::new());
            rigid_hair.push(Vec::new());
            out.animated.push(Vec::new());
            out.on_tail.push(Vec::new());
            out.bind.skin.push(Vec::new());
            out.bind.positions.push(Vec::new());
            out.bind.normals.push(Vec::new());
            continue;
        };
        out.indices.push(g.indices.iter().map(|&i| i as u32).collect());
        let sv = g.skin.as_deref().unwrap_or(&[]);
        let node_skin: Vec<NodeSkin> = (0..g.vertices.len())
            .map(|i| {
                let s = sv.get(i).copied().unwrap_or_default();
                NodeSkin {
                    joints: s.joints.map(u16::from),
                    weights: s.weights.map(|w| w as f32 / 65535.0),
                }
            })
            .collect();
        let on =
            |list: &[bool], s: &NodeSkin| (0..4).any(|k| s.weights[k] > 0.1 && list.get(s.joints[k] as usize).copied().unwrap_or(false));
        on_limb |= node_skin.iter().any(|s| on(&limbs, s));
        for s in &node_skin {
            for k in 0..4 {
                let j = s.joints[k] as usize;
                if s.weights[k] > 0.1 {
                    if limbs.get(j).copied().unwrap_or(false) {
                        limb_count[j] += 1;
                    }
                    if let Some(c) = joint_count.get_mut(j) {
                        *c += 1;
                    }
                }
            }
        }
        out.rigid.push(node_skin.iter().map(|s| mostly_on(&fixed, s)).collect());
        rigid_hair.push(node_skin.iter().map(|s| mostly_on(&fixed_hair, s)).collect());
        out.animated.push(node_skin.iter().map(|s| on(&moved, s)).collect());
        out.on_tail.push(node_skin.iter().map(|s| tail_share(&moved, s)).collect());
        // rest places; in bind space for the skinning, the backstop's
        // normals turned away from the body (judged at rest, whichever way
        // the creator wound the faces, sim.rs), only near the body: it keeps
        // cloth out of the wearer, and turned "away from the body" round a
        // tail out in the air, half its normals pointed into it and let it
        // flatten
        let mut rest_pos = Vec::with_capacity(g.vertices.len());
        let mut bind_normal = Vec::with_capacity(g.vertices.len());
        for (v, s) in g.vertices.iter().zip(&node_skin) {
            let m = rest_matrix(s);
            let p = m.transform_point3(Vec3::from_array(v.pos));
            let n = Vec3::new(v.normal[0] as f32, v.normal[1] as f32, v.normal[2] as f32).normalize_or_zero();
            rest_pos.push(p);
            bind_normal.push(if body.distance(p) > BACKSTOP_RANGE {
                Vec3::ZERO
            } else if m.transform_vector3(n).dot(body.outward(p)) < 0.0 {
                -n
            } else {
                n
            });
        }
        out.positions.push(rest_pos);
        out.bind
            .positions
            .push(g.vertices.iter().map(|v| Vec3::from_array(v.pos)).collect());
        out.bind.normals.push(bind_normal);
        out.bind.skin.push(node_skin);
    }
    let all: Vec<Vec3> = out.positions.iter().flatten().copied().collect();
    let (top, bottom) = all.iter().fold((f32::MIN, f32::MAX), |(t, b), p| (t.max(p.z), b.min(p.z)));
    out.hair = !on_limb && top > neck + HAIR_TOP && top - bottom > HAIR_LENGTH;
    out.scalp = neck + SCALP;
    if out.hair {
        let head_m = rig
            .names
            .get("HEAD")
            .and_then(|&j| rig.default_world_inv.get(j))
            .map(|m| m.inverse());
        let crown = head_m.map_or(f32::MAX, |m| m.w_axis.z);
        let head = head_m.map(|m| weights::Body::new([m]));
        let mut shell = 0usize;
        for (f, flags) in rigid_hair.iter_mut().enumerate() {
            for (v, flag) in flags.iter_mut().enumerate() {
                let p = out.positions[f][v];
                if !*flag && p.z > crown && head.as_ref().is_some_and(|h| h.distance(p) < HEAD_SHELL) {
                    *flag = true;
                    shell += 1;
                }
            }
        }
        log::info!("cloth rest: {key:?} hair, {shell} vertices on the head held");
        out.rigid = rigid_hair;
    }
    out.never = !out.hair && never_cloth(&out.rigid);
    let (hugs, wrap) = weights::hugs(&body, &all, &weights::Params::default());
    out.body_like = hugs && !out.hair;
    let total = out.animated.iter().map(Vec::len).sum::<usize>().max(1);
    out.tail = out.animated.iter().flatten().filter(|&&a| a).count() as f32 > ANIMATED_SHARE * total as f32;
    if out.body_like {
        // open edges (of one triangle) standing off the body: worth weighing
        let params = weights::Params::default();
        let mut standing = 0usize;
        for (f, idx) in out.indices.iter().enumerate() {
            let mut uses: HashMap<(u32, u32), u32> = HashMap::new();
            for t in idx.as_chunks::<3>().0 {
                for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    *uses.entry((a.min(b), a.max(b))).or_default() += 1;
                }
            }
            for (&(a, b), &c) in &uses {
                if c == 1 {
                    for v in [a, b] {
                        if out.positions[f]
                            .get(v as usize)
                            .is_some_and(|&p| body.distance(p) > params.sway_from)
                        {
                            standing += 1;
                        }
                    }
                }
            }
        }
        out.sway = standing >= SWAY_MIN_EDGE;
    }
    let mut limb_list: Vec<(usize, &str)> = limb_count
        .iter()
        .zip(&skin.joint_names)
        .filter(|(c, _)| **c > 0)
        .map(|(c, n)| (*c, n.as_str()))
        .collect();
    limb_list.sort_by_key(|a| std::cmp::Reverse(a.0));
    let mut joint_list: Vec<(usize, &str)> = joint_count
        .iter()
        .zip(&skin.joint_names)
        .filter(|(c, _)| **c > 0)
        .map(|(c, n)| (*c, n.as_str()))
        .collect();
    joint_list.sort_by_key(|a| std::cmp::Reverse(a.0));
    joint_list.truncate(6);
    log::info!(
        "cloth rest: {key:?} {} vertices, body-like {} (wrap {wrap:.2}), hair {} (top {:.2} m, neck {neck:.2} m, length {:.2} m, limbs {:?}, joints {:?})",
        all.len(),
        out.body_like,
        out.hair,
        top,
        top - bottom,
        limb_list,
        joint_list
    );
    Some(out)
}

/// Step 3, a background job: a group's freedom against a body (`body`: the
/// avatar's collision volumes at rest, fitted; `surface`: the body worn;
/// `exclude`: the members' own places among the body's meshes, a body-like
/// one being measured against the others). Its members' faces one after the
/// other, welded where they meet: the seams of a garment in pieces hold.
fn weigh_group(
    kind: Kind,
    rests: &[Arc<RestMesh>],
    body: Vec<Mat4>,
    surface: Option<Arc<weights::Surface>>,
    exclude: Vec<u32>,
    signature: u64,
) -> Option<GroupMesh> {
    let mut inputs: Vec<weights::FaceInput> = Vec::new();
    let mut members: Vec<u32> = Vec::new();
    for (m, rest) in rests.iter().enumerate() {
        for f in 0..rest.positions.len() {
            members.push(m as u32);
            inputs.push(weights::FaceInput {
                positions: &rest.positions[f],
                indices: &rest.indices[f],
                rigid: &rest.rigid[f],
            });
        }
    }
    let started = Instant::now();
    let hair_scalp = rests.iter().find(|r| r.hair).map(|r| r.scalp);
    let params = weights::Params {
        drapes: kind == Kind::Hair,
        sway: kind == Kind::Fitted,
        stitch: if matches!(kind, Kind::Tail | Kind::Clothing) { STITCH } else { 0.0 },
        pin_above: if kind == Kind::Hair { hair_scalp } else { None },
        ..Default::default()
    };
    let mut w = weights::compute_members(
        &inputs,
        &members,
        Some(&weights::Body::new(body).with_surface(surface).excluding(exclude)),
        &params,
    );
    if kind == Kind::Tail {
        free_by_tail_weight(&mut w, rests);
    }
    let free_still = w.node_free.clone();
    // a Bento tail only drags behind its animation; a mesh that is largely
    // one (a tail whose base is on the pelvis) is held whole: held in part,
    // it pinched where its held part met its free one
    let mut face = 0;
    let mut on_animated = 0;
    for rest in rests {
        let total = rest.animated.iter().map(Vec::len).sum::<usize>().max(1);
        let animated = rest.animated.iter().flatten().filter(|&&a| a).count();
        on_animated += animated;
        let whole = animated as f32 > ANIMATED_SHARE * total as f32;
        for f in 0..rest.positions.len() {
            let Some(map) = w.node_of.get(face + f) else { continue };
            for (v, &node) in map.iter().enumerate() {
                let on = whole || rest.animated.get(f).and_then(|a| a.get(v)).copied().unwrap_or(false);
                if on && w.node_free[node as usize] > ANIMATED_FREEDOM {
                    w.node_free[node as usize] = ANIMATED_FREEDOM;
                }
            }
        }
        face += rest.positions.len();
    }
    // the members' skins, each's joints after the one before's
    let mut bind = BindData::default();
    let mut spans = Vec::with_capacity(rests.len());
    let (mut base, mut vectors) = (0u16, 0usize);
    for rest in rests {
        let count: usize = rest.positions.iter().map(Vec::len).sum();
        spans.push((vectors, count));
        vectors += count;
        for f in 0..rest.positions.len() {
            bind.skin.push(
                rest.bind.skin[f]
                    .iter()
                    .map(|s| NodeSkin {
                        joints: s.joints.map(|j| j + base),
                        weights: s.weights,
                    })
                    .collect(),
            );
            bind.positions.push(rest.bind.positions[f].clone());
            bind.normals.push(rest.bind.normals[f].clone());
        }
        base = base.saturating_add(rest.joints.min(u16::MAX as usize) as u16);
    }
    // only what moves is simulated, on a lattice (`coarse`); its points are
    // counted against `MAX_NODES`
    let kept = w.prune();
    let free_still: Vec<f32> = kept.iter().map(|&u| free_still.get(u as usize).copied().unwrap_or(0.0)).collect();
    let far = w.stats.body_gap.is_some_and(|g| g > MAX_BODY_GAP) || w.stats.max_distance > MAX_LENGTH;
    let rigid = w.is_rigid();
    let stats = w.stats.clone();
    let moving = w.nodes.len();
    let cloth = (!rigid && !far).then(|| {
        let mut cloth = ClothMesh::coarse(w, &bind, free_still, CELL);
        cloth.floats = kind != Kind::Tail;
        cloth
    });
    let points = cloth.as_ref().map_or(0, |c| c.nodes.len());
    let dense = points > MAX_NODES;
    log::info!(
        "cloth weights: {kind:?}, {} meshes, {vectors} vertices, {} nodes ({moving} kept, {points} simulated), {} islands ({} floating, {} detached, {} stitches), {} anchors, {on_animated} on Bento tail joints, body gap {:?}, longest {:.3} m, hugging {} (loose {:.2}, swaying {}), not worn {far}, dense {dense}, rigid {rigid}, fit {signature:x}, {:?}",
        rests.len(),
        stats.nodes,
        stats.islands,
        stats.floating_islands,
        stats.detached,
        stats.stitches,
        stats.anchors,
        stats.body_gap,
        stats.max_distance,
        stats.hugging,
        stats.loose,
        stats.swaying,
        started.elapsed()
    );
    for (m, ms) in stats.members.iter().enumerate() {
        log::info!(
            "cloth member: {kind:?} {m}/{}, {} nodes, z {:.2}..{:.2} m, touching {} near {} loose {}, hugs {} tight {}, anchors {} rigid {} unreached {}, free mean {:.2} max {:.2} hem {:.2}, fit {signature:x}",
            stats.members.len(),
            ms.nodes,
            ms.low,
            ms.high,
            ms.touching,
            ms.near,
            ms.loose,
            ms.hugs,
            ms.tight,
            ms.anchors,
            ms.rigid,
            ms.unreached,
            ms.mean_free,
            ms.max_free,
            ms.hem_free
        );
    }
    cloth.filter(|_| !dense).map(|cloth| GroupMesh { cloth, spans })
}

/// A tail moves as far as it is rigged to the tail: each node's freedom
/// scaled by its vertices' share of skin weight on the tail's joints (the
/// most of them, where meshes meet). A mermaid's tail wraps the hips loosely
/// and is rigged there to the pelvis: freed by its distance from the body
/// alone, its top ballooned in the air drag when flying; it now follows the
/// hips, and its fins (on the tail's joints) swing as before. (Clothes are
/// not scaled: a dress rigged to the pelvis and legs must swing.)
fn free_by_tail_weight(w: &mut weights::Weights, rests: &[Arc<RestMesh>]) {
    let mut share = vec![0.0f32; w.node_free.len()];
    let mut face = 0;
    for rest in rests {
        for f in 0..rest.positions.len() {
            if let Some(map) = w.node_of.get(face + f) {
                for (v, &node) in map.iter().enumerate() {
                    let s = rest.on_tail.get(f).and_then(|t| t.get(v)).copied().unwrap_or(0.0);
                    if let Some(x) = share.get_mut(node as usize) {
                        *x = x.max(s);
                    }
                }
            }
        }
        face += rest.positions.len();
    }
    for (free, s) in w.node_free.iter_mut().zip(&share) {
        *free *= s;
    }
    for (out, map) in w.faces.iter_mut().zip(&w.node_of) {
        for (f, &node) in out.iter_mut().zip(map) {
            *f = w.node_free.get(node as usize).copied().unwrap_or(0.0);
        }
    }
}

/// A group's members, hashed (with their meshes and levels).
fn members_hash(members: &[(usize, Key)]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (_, key) in members {
        key.hash(&mut h);
    }
    h.finish()
}

impl Scene {
    /// The preferences' cloth (app.rs, each frame).
    pub fn set_cloth_config(&mut self, config: Config) {
        if self.cloth.config != config {
            self.cloth.config = config;
            self.cloth.config_changed = true;
        }
    }

    /// The collision volumes of the avatar skeleton (rig indices).
    fn cloth_volumes(&self) -> Vec<usize> {
        self.avatar_lib
            .skeleton
            .as_ref()
            .map(|sk| {
                (0..self.avatar_lib.rig.len())
                    .filter(|&j| sk.joints.get(j).is_some_and(|x| x.is_collision_volume))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The collision volumes at rest, scaled by a fit.
    fn cloth_rest_volumes(&self, scales: &[Vec3]) -> Vec<Mat4> {
        self.cloth_volumes()
            .into_iter()
            .enumerate()
            .map(|(k, j)| {
                self.avatar_lib.rig.default_world_inv[j].inverse() * Mat4::from_scale(scales.get(k).copied().unwrap_or(Vec3::ONE))
            })
            .collect()
    }

    /// For a face of a rigged mesh being recorded: where its cloth offsets
    /// are (`DrawRecord::identity` y, z, w), or None (not simulated, not
    /// cloth, not ready: its job is started and the object recorded again
    /// once done). `geom`: the object's geometry (its GPU entry is out while
    /// recorded).
    pub(super) fn cloth_face(
        &mut self,
        world: &World,
        idx: usize,
        geom: Option<GeomKey>,
        owner: Uuid,
        fi: usize,
        alloc: &MeshAlloc,
    ) -> Option<[u32; 3]> {
        let mode = self.cloth.config.mode;
        if mode == Mode::Off || (owner != world.agent_id && mode != Mode::All) {
            return None;
        }
        let Some(GeomKey::Mesh { id, lod }) = geom else {
            return None;
        };
        let object = world.objects.get(idx)?;
        // an avatar's skeleton, not an animesh's
        world
            .objects
            .index_of_uuid(&owner)
            .and_then(|i| world.objects.get(i))
            .filter(|o| o.is_avatar())?;
        let key = (id, lod);
        self.cloth.outfit.insert(idx, (object.full_id, owner, key));
        if !self.cloth.simulated.contains(&owner) && owner != world.agent_id {
            self.release_cloth(idx);
            return None;
        }
        // step 1
        let rest = match self.cloth.rest.get(&key) {
            Some(Rest::Ready(rest)) => rest.clone(),
            Some(Rest::Failed) => return None,
            Some(Rest::Pending) => {
                self.cloth_wait(key, idx);
                return None;
            }
            None => {
                let view = self.meshes.view(&id)?;
                let skin = view.skin.clone()?;
                let body = self.cloth_rest_volumes(&[]);
                let rig = self.avatar_lib.rig.clone();
                let tx = self.cloth.rest_tx.clone();
                self.cloth.rest.insert(key, Rest::Pending);
                self.cloth_wait(key, idx);
                self.jobs.execute(move || {
                    let _ = tx.send((key, rest_job(key, view, lod, skin, rig, body)));
                });
                return None;
            }
        };
        // what it is, and whether the preferences let it swing
        let Some(kind) = Kind::of(&rest).filter(|k| k.wanted(&self.cloth.config)) else {
            self.release_cloth(idx);
            return None;
        };
        // its palette slots: one vector per vertex, four per palette matrix,
        // in whole consecutive slots taken at the end (whatever its group)
        let keep = self
            .cloth
            .worn
            .get(&idx)
            .is_some_and(|w| w.key == key && w.id == object.full_id && w.owner == owner && w.kind == kind);
        if !keep {
            self.release_cloth(idx);
            let counts: Vec<u32> = rest.positions.iter().map(|f| f.len() as u32).collect();
            let vectors: u32 = counts.iter().sum();
            let slot_vectors = PALETTE_JOINTS as u32 * 4;
            let count = vectors.div_ceil(slot_vectors).max(1);
            let first = self.palette_count;
            self.palette_count += count;
            let mut faces = Vec::with_capacity(counts.len());
            let mut at = first * slot_vectors;
            for c in &counts {
                faces.push(at);
                at += c;
            }
            log::info!("cloth worn: object {idx} of {owner}, {kind:?}, mesh {id} level {lod}");
            self.cloth.worn.insert(
                idx,
                Worn {
                    id: object.full_id,
                    owner,
                    key,
                    kind,
                    slots: first..first + count,
                    faces,
                    counts,
                },
            );
        }
        // simulated once its group is (update_cloth records it again then)
        let group = self.cloth.groups.get(&group_slot(owner, kind, idx))?;
        if !group.members.contains(&(idx, key)) {
            return None;
        }
        let worn = self.cloth.worn.get(&idx)?;
        // the GPU's face must be the one weighed
        let count = *worn.counts.get(fi)?;
        if count != alloc.vertex_count {
            log::info!(
                "cloth face: {key:?} face {fi}: {count} vertices weighed, the GPU holds {}",
                alloc.vertex_count
            );
        }
        (count == alloc.vertex_count && count > 0).then(|| [worn.faces[fi], alloc.vertex_offset, alloc.vertex_count])
    }

    fn cloth_wait(&mut self, key: Key, idx: usize) {
        let w = self.cloth.waiting.entry(key).or_default();
        if !w.contains(&idx) {
            w.push(idx);
        }
    }

    /// The object no longer wears that cloth: its slots go back.
    fn release_cloth(&mut self, idx: usize) {
        if let Some(w) = self.cloth.worn.remove(&idx) {
            self.palette_free.extend(w.slots);
        }
    }

    /// Record an object again (its cloth changed).
    fn cloth_rerecord(&mut self, world: &mut World, idx: usize) {
        // (a worn rigged mesh hangs from its anchor: the sync visits it
        // again only for a change of its own, as anchors.rs asks)
        if let Some(o) = world.objects.get_mut_untracked(idx) {
            o.render.needs_records = true;
            o.material_dirty = true;
        }
        self.sets.dirty.insert(idx);
    }

    /// Step 2: each avatar's volumes fitted to the body-like meshes it wears,
    /// again when they change; its other meshes weighed again then.
    fn update_cloth_fits(&mut self, world: &mut World) {
        let mut bodies: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for &(_, owner, key) in self.cloth.outfit.values() {
            let entry = bodies.entry(owner).or_default();
            if matches!(self.cloth.rest.get(&key), Some(Rest::Ready(r)) if r.body_like) && !entry.contains(&key.0) {
                entry.push(key.0);
            }
        }
        self.cloth.fits.retain(|owner, _| bodies.contains_key(owner));
        let defaults = self.cloth_rest_volumes(&[]);
        for (owner, mut body) in bodies {
            body.sort();
            if self.cloth.fits.get(&owner).is_some_and(|f| f.body == body) {
                continue;
            }
            // each body mesh at the finest level held
            let mut points: Vec<Vec3> = Vec::new();
            let mut owners: Vec<u32> = Vec::new();
            for (k, id) in body.iter().enumerate() {
                if let Some(r) = (0..4u8).rev().find_map(|lod| match self.cloth.rest.get(&(*id, lod)) {
                    Some(Rest::Ready(r)) => Some(r),
                    _ => None,
                }) {
                    points.extend(r.positions.iter().flatten());
                    owners.resize(points.len(), k as u32);
                }
            }
            if points.len() > FIT_POINTS {
                let step = points.len().div_ceil(FIT_POINTS);
                points = points.into_iter().step_by(step).collect();
                owners = owners.into_iter().step_by(step).collect();
            }
            let started = Instant::now();
            let scales = if points.is_empty() {
                Vec::new()
            } else {
                weights::fit_scales(&defaults, &points)
            };
            let signature = if body.is_empty() {
                0
            } else {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                body.hash(&mut h);
                h.finish() | 1
            };
            let (lo, hi) = scales
                .iter()
                .fold((f32::MAX, 0.0f32), |(lo, hi), s| (lo.min(s.min_element()), hi.max(s.max_element())));
            log::info!(
                "cloth fit: avatar {owner}, {} body meshes, {} points, scales {lo:.2}..{hi:.2}, fit {signature:x}, {:?}",
                body.len(),
                points.len(),
                started.elapsed()
            );
            self.cloth.fits.insert(
                owner,
                Fit {
                    body,
                    scales: Arc::new(scales),
                    signature,
                    surface: (!points.is_empty()).then(|| Arc::new(weights::Surface::new(&points, &owners))),
                },
            );
            // its cloth weighed again for the body it now wears
            let objects: Vec<usize> = self
                .cloth
                .outfit
                .iter()
                .filter(|(_, (_, o, _))| *o == owner)
                .map(|(idx, _)| *idx)
                .collect();
            for idx in objects {
                self.cloth_rerecord(world, idx);
            }
        }
    }

    /// The avatars simulated: ours, and with `all` the nearest others in
    /// range. One leaving gives its slots back and is drawn rigid again.
    fn update_cloth_avatars(&mut self, world: &mut World) {
        let mut next: HashSet<Uuid> = HashSet::new();
        next.insert(world.agent_id);
        if self.cloth.config.mode == Mode::All {
            let here = world.agent.position;
            let mut others: Vec<(f32, Uuid)> = self
                .cloth
                .outfit
                .values()
                .map(|(_, owner, _)| *owner)
                .filter(|o| *o != world.agent_id)
                .collect::<HashSet<_>>()
                .into_iter()
                .filter_map(|o| {
                    let i = world.objects.index_of_uuid(&o)?;
                    // shown as a silhouette (too complex): no cloth
                    if self.too_complex.contains(&i) {
                        return None;
                    }
                    let obj = world.objects.get(i)?;
                    let d = obj.position.distance(here);
                    (d <= OTHERS_RANGE).then_some((d, o))
                })
                .collect();
            others.sort_by(|a, b| a.0.total_cmp(&b.0));
            next.extend(others.into_iter().take(MAX_OTHERS).map(|(_, o)| o));
        }
        if next == self.cloth.simulated {
            return;
        }
        let changed: HashSet<Uuid> = next.symmetric_difference(&self.cloth.simulated).copied().collect();
        self.cloth.simulated = next;
        let objects: Vec<usize> = self
            .cloth
            .outfit
            .iter()
            .filter(|(_, (_, o, _))| changed.contains(o))
            .map(|(idx, _)| *idx)
            .collect();
        for idx in objects {
            if !self.cloth.simulated.contains(&self.cloth.outfit[&idx].1) {
                self.release_cloth(idx);
            }
            self.cloth_rerecord(world, idx);
        }
    }

    /// Each frame, after the skeletons are posed: finished jobs, the avatars
    /// and fits, then every worn cloth stepped and its offsets written to its
    /// palette slots.
    pub(super) fn update_cloth(&mut self, world: &mut World, now: Instant, moved: &mut Vec<std::ops::Range<usize>>) {
        if std::mem::take(&mut self.cloth.config_changed) {
            // every worn mesh recorded again under the new preferences: those
            // the cloth knows, and every other one (turned on, it knew none:
            // nothing already recorded swung until it changed)
            let mut all: Vec<usize> = self.cloth.worn.keys().chain(self.cloth.outfit.keys()).copied().collect();
            all.extend(
                self.gpu
                    .iter()
                    .enumerate()
                    .filter(|(_, g)| g.owner_avatar.is_some() && matches!(g.geom, Some(GeomKey::Mesh { .. })))
                    .map(|(idx, _)| idx),
            );
            all.sort_unstable();
            all.dedup();
            let worn: Vec<usize> = self.cloth.worn.keys().copied().collect();
            for idx in worn {
                self.release_cloth(idx);
            }
            self.cloth.groups.clear();
            if self.cloth.config.mode == Mode::Off {
                self.cloth.outfit.clear();
                self.cloth.simulated.clear();
            }
            for idx in all {
                self.cloth_rerecord(world, idx);
            }
        }
        if self.cloth.config.mode == Mode::Off {
            return;
        }
        let mut arrived: Vec<Key> = Vec::new();
        while let Ok((key, rest)) = self.cloth.rest_rx.try_recv() {
            self.cloth.rest.insert(key, rest.map_or(Rest::Failed, |r| Rest::Ready(Arc::new(r))));
            arrived.push(key);
        }
        while let Ok((group, mesh)) = self.cloth.weigh_rx.try_recv() {
            self.cloth
                .weighed
                .insert(group, mesh.map_or(Weighed::Rigid, |m| Weighed::Cloth(Arc::new(m))));
        }
        for key in arrived {
            for idx in self.cloth.waiting.remove(&key).unwrap_or_default() {
                self.cloth_rerecord(world, idx);
            }
        }
        // gone or changed: forgotten
        let gone = |idx: usize, id: Uuid, key: Key, gpu: &[super::ObjGpu]| {
            world.objects.get(idx).is_none_or(|o| {
                o.full_id != id || !matches!(gpu.get(idx).and_then(|g| g.geom), Some(GeomKey::Mesh { id, lod }) if (id, lod) == key)
            })
        };
        let stale: Vec<usize> = self
            .cloth
            .worn
            .iter()
            .filter(|(idx, w)| gone(**idx, w.id, w.key, &self.gpu))
            .map(|(idx, _)| *idx)
            .collect();
        for idx in stale {
            self.release_cloth(idx);
        }
        let gpu = &self.gpu;
        self.cloth.outfit.retain(|idx, (id, _, key)| !gone(*idx, *id, *key, gpu));
        self.update_cloth_avatars(world);
        self.update_cloth_fits(world);

        self.update_cloth_groups(world);

        let dt = self.cloth.last.map_or(0.0, |l| now.saturating_duration_since(l).as_secs_f32());
        self.cloth.last = Some(now);
        let cvs = self.cloth_volumes();
        let binds = self.skin_binds.list();
        let log_now = self
            .cloth
            .logged
            .is_none_or(|l| now.saturating_duration_since(l).as_secs_f32() >= 2.0);
        let started = Instant::now();
        // per avatar: its skeleton as posed, its fitted volumes, which way its
        // body's rest down now points (its pelvis turned: a handstand), its
        // water line
        let pelvis = self.avatar_lib.rig.pelvis;
        #[allow(clippy::type_complexity)]
        let mut posed: HashMap<Uuid, (Vec<Mat4>, Vec<Volume>, Vec3, Option<f32>)> = HashMap::new();
        for (&(owner, _, _), g) in self.cloth.groups.iter_mut() {
            g.mats.clear();
            if let std::collections::hash_map::Entry::Vacant(vacant) = posed.entry(owner) {
                let Some(&slot) = self.palette_slots.get(&owner) else { continue };
                let pbase = slot as usize * PALETTE_JOINTS;
                // (a copy of the skeleton's slot: the offsets are written to
                // the same palettes below)
                let Some(pal) = self.palettes.get(pbase..pbase + PALETTE_JOINTS) else {
                    continue;
                };
                let pal: Vec<Mat4> = pal.iter().map(Mat4::from_cols_array_2d).collect();
                let anchor = pal[ANCHOR_BASE];
                let scales = self.cloth.fits.get(&owner).map(|f| f.scales.clone()).unwrap_or_default();
                let volumes = cvs
                    .iter()
                    .enumerate()
                    .filter_map(|(k, &j)| {
                        let scale = scales.get(k).copied().unwrap_or(Vec3::ONE);
                        Volume::new(anchor * pal.get(j).copied().unwrap_or(Mat4::IDENTITY) * Mat4::from_scale(scale))
                    })
                    .collect();
                let body_down = (anchor * pal.get(pelvis).copied().unwrap_or(Mat4::IDENTITY)).transform_vector3(Vec3::NEG_Z);
                // its tail animated? (its joints moving against the pelvis)
                let rig = &self.avatar_lib.rig;
                let pelvis_inv = pal.get(pelvis).copied().unwrap_or(Mat4::IDENTITY).inverse();
                let tail: Vec<Mat4> = (1..=6)
                    .filter_map(|k| rig.names.get(&format!("mTail{k}")))
                    .filter_map(|&j| pal.get(j))
                    .map(|m| pelvis_inv * *m)
                    .collect();
                let entry = self.cloth.tails.entry(owner).or_insert((Vec::new(), now));
                let moved = entry.0.len() != tail.len()
                    || entry
                        .0
                        .iter()
                        .zip(&tail)
                        .any(|(a, b)| (a.to_cols_array().iter().zip(b.to_cols_array().iter())).any(|(x, y)| (x - y).abs() > 1e-3));
                if moved {
                    entry.1 = now;
                }
                entry.0 = tail;
                // the water line of its region, in the same space (the
                // anchors place it as the flexis are: region offset added)
                let water = world
                    .objects
                    .index_of_uuid(&owner)
                    .and_then(|i| world.objects.get(i))
                    .and_then(|o| {
                        let height = world.regions.get(&o.key.region)?.info.as_ref()?.water_height;
                        Some(height + world.region_offset(o.key.region).unwrap_or(Vec3::ZERO).z)
                    });
                vacant.insert((pal, volumes, body_down, water));
            }
            let Some((pal, _, _, _)) = posed.get(&owner) else { continue };
            let anchor = pal[ANCHOR_BASE];
            // each member's matrices after the one before's (its skin's
            // joints were numbered so, `weigh_group`)
            let mut complete = true;
            for (_, key) in &g.members {
                let Some(bbase) = self.skin_binds.base_of(&key.0) else {
                    complete = false;
                    break;
                };
                let joints = self.meshes.view(&key.0).and_then(|v| v.skin).map_or(0, |s| s.joint_names.len());
                g.mats.extend((0..joints).map(|j| match binds.get(bbase as usize + j) {
                    Some(b) => {
                        anchor * pal.get(b.joint[0] as usize).copied().unwrap_or(Mat4::IDENTITY) * Mat4::from_cols_array_2d(&b.inverse_bind)
                    }
                    None => anchor,
                }));
            }
            if !complete {
                g.mats.clear();
            }
        }
        // each group on its own core
        {
            use rayon::prelude::*;
            let posed = &posed;
            let wind = self.weather_wind.unwrap_or(Vec3::ZERO);
            let tails = &self.cloth.tails;
            self.cloth.groups.par_iter_mut().for_each(|(&(owner, _, _), g)| {
                g.out.clear();
                g.state.wind = wind;
                g.state.held = tails
                    .get(&owner)
                    .is_none_or(|(_, at)| now.saturating_duration_since(*at).as_secs_f32() < STILL_FOR);
                let Some((pal, volumes, body_down, water)) = posed.get(&owner).filter(|_| !g.mats.is_empty()) else {
                    return;
                };
                g.state.water = *water;
                g.state.step(&g.mesh.cloth, &g.mats, volumes, pal[ANCHOR_BASE], *body_down, dt);
                g.state.deltas(&g.mesh.cloth, &mut g.out);
            });
        }
        // each member's share of its group's offsets into its own slots
        let mut largest = 0.0f32;
        for g in self.cloth.groups.values() {
            if g.out.is_empty() {
                continue;
            }
            if log_now {
                largest = largest.max(g.state.largest_offset());
            }
            for (m, (idx, key)) in g.members.iter().enumerate() {
                let (Some(worn), Some(&(start, count))) = (self.cloth.worn.get(idx), g.mesh.spans.get(m)) else {
                    continue;
                };
                if worn.key != *key {
                    continue;
                }
                let Some(part) = g.out.get(start..start + count) else { continue };
                let first = worn.slots.start as usize * PALETTE_JOINTS;
                let mats_needed = count.div_ceil(4);
                let Some(region) = self.palettes.get_mut(first..first + mats_needed) else {
                    continue;
                };
                for (k, d) in part.iter().enumerate() {
                    region[k / 4][k % 4] = *d;
                }
                moved.push(first..first + mats_needed);
            }
        }
        if log_now && !self.cloth.groups.is_empty() {
            self.cloth.logged = Some(now);
            log::info!(
                "cloth motion: {} groups of {} meshes on {} avatars ({} held rigid by the budget), {} points, largest offset {largest:.3} m, step {:?}",
                self.cloth.groups.len(),
                self.cloth.groups.values().map(|g| g.members.len()).sum::<usize>(),
                posed.len(),
                self.cloth.held,
                self.cloth.groups.values().map(|g| g.mesh.cloth.nodes.len()).sum::<usize>(),
                started.elapsed()
            );
        }
    }

    /// The groups: per simulated avatar and kind, the worn objects of that
    /// kind, simulated together once weighed together. A group whose members
    /// change is weighed again (its meshes are drawn rigid meanwhile); one
    /// whose fit changes keeps its old weights until the new ones are in.
    fn update_cloth_groups(&mut self, world: &mut World) {
        let mut wanted: HashMap<GroupSlot, Vec<(usize, Key)>> = HashMap::new();
        for (&idx, w) in &self.cloth.worn {
            if self.cloth.simulated.contains(&w.owner) || w.owner == world.agent_id {
                wanted.entry(group_slot(w.owner, w.kind, idx)).or_default().push((idx, w.key));
            }
        }
        for members in wanted.values_mut() {
            members.sort();
        }
        // the budget: the groups that do not fit are drawn rigid
        let here = world.agent.position;
        let candidates: Vec<Candidate> = wanted
            .iter()
            .map(|(slot, members)| {
                let owner = slot.0;
                let signature = self.cloth.fits.get(&owner).map_or(0, |f| f.signature);
                let points = match self.cloth.weighed.get(&(members_hash(members), signature)) {
                    Some(Weighed::Cloth(m)) => m.cloth.nodes.len(),
                    _ => 0,
                };
                let distance = if owner == world.agent_id {
                    0.0
                } else {
                    world
                        .objects
                        .index_of_uuid(&owner)
                        .and_then(|i| world.objects.get(i))
                        .map_or(f32::INFINITY, |o| o.position.distance(here))
                };
                Candidate {
                    slot: *slot,
                    own: owner == world.agent_id,
                    distance,
                    points,
                    active: self.cloth.groups.contains_key(slot),
                }
            })
            .collect();
        let allowed = budget(&candidates);
        wanted.retain(|slot, _| allowed.contains(slot));
        self.cloth.held = candidates.len() - allowed.len();
        // groups no longer worn
        let gone: Vec<GroupSlot> = self.cloth.groups.keys().filter(|k| !wanted.contains_key(k)).copied().collect();
        for k in gone {
            if let Some(g) = self.cloth.groups.remove(&k) {
                for (idx, _) in g.members {
                    self.cloth_rerecord(world, idx);
                }
            }
        }
        for (slot, members) in wanted {
            let (owner, kind, _) = slot;
            let fit = self.cloth.fits.get(&owner).cloned().unwrap_or_default();
            let key: GroupKey = (members_hash(&members), fit.signature);
            let same = self.cloth.groups.get(&slot).is_some_and(|g| g.members == members);
            if same && self.cloth.groups.get(&slot).is_some_and(|g| g.signature == fit.signature) {
                continue;
            }
            match self.cloth.weighed.get(&key) {
                Some(Weighed::Cloth(mesh)) => {
                    let mesh = mesh.clone();
                    match self.cloth.groups.get_mut(&slot) {
                        // the same meshes for the body worn now: the motion
                        // goes on when the nodes are the same
                        Some(g) if same => {
                            if g.mesh.cloth.nodes.len() != mesh.cloth.nodes.len() {
                                g.state = ClothState::default();
                            }
                            g.mesh = mesh;
                            g.signature = fit.signature;
                        }
                        _ => {
                            log::info!(
                                "cloth group: avatar {owner}, {kind:?}, {} meshes, {} nodes",
                                members.len(),
                                mesh.cloth.nodes.len()
                            );
                            let old = self.cloth.groups.insert(
                                slot,
                                Group {
                                    members: members.clone(),
                                    signature: fit.signature,
                                    mesh,
                                    state: ClothState::default(),
                                    mats: Vec::new(),
                                    out: Vec::new(),
                                },
                            );
                            for (idx, _) in old.map(|g| g.members).unwrap_or_default().into_iter().chain(members) {
                                self.cloth_rerecord(world, idx);
                            }
                        }
                    }
                }
                Some(Weighed::Rigid) => {
                    if let Some(g) = self.cloth.groups.remove(&slot) {
                        for (idx, _) in g.members {
                            self.cloth_rerecord(world, idx);
                        }
                    }
                }
                Some(Weighed::Pending) => {
                    // other members: the old group's mapping no longer holds
                    if !same && let Some(g) = self.cloth.groups.remove(&slot) {
                        for (idx, _) in g.members {
                            self.cloth_rerecord(world, idx);
                        }
                    }
                }
                None => {
                    let rests: Option<Vec<Arc<RestMesh>>> = members
                        .iter()
                        .map(|(_, k)| match self.cloth.rest.get(k) {
                            Some(Rest::Ready(r)) => Some(r.clone()),
                            _ => None,
                        })
                        .collect();
                    let Some(rests) = rests else { continue };
                    let body = self.cloth_rest_volumes(&fit.scales);
                    let surface = fit.surface.clone();
                    let exclude: Vec<u32> = members
                        .iter()
                        .filter_map(|(_, k)| fit.body.iter().position(|b| *b == k.0).map(|p| p as u32))
                        .collect();
                    let tx = self.cloth.weigh_tx.clone();
                    let signature = fit.signature;
                    self.cloth.weighed.insert(key, Weighed::Pending);
                    self.jobs.execute(move || {
                        let _ = tx.send((key, weigh_group(kind, &rests, body, surface, exclude, signature)));
                    });
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "group_tests.rs"]
mod group_tests;

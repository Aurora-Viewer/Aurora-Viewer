//! Ray-traced sun shadows of the main view, and with them the local
//! lights' shadows, the indirect light and the reflections: experimental,
//! Preferences › Graphics › Ray tracing (`RenderSettings::ray_tracing`,
//! off by default; devices opened with ray queries).
//!
//! Through wgpu's ray queries (`EXPERIMENTAL_RAY_QUERY`): a bottom level per
//! face geometry of the arena, a top level of the faces that stand still in
//! the world, built again only when their set changes (`StillTracker`; the
//! few that move go to the top level built each frame), and one compute pass after
//! the contact shadows that traces a ray per pixel of the depth prepass
//! towards the sun (shaders/rt_shadows.wgsl). Its mask takes the contact
//! shadows' place in the frame group, the darker of the two, so the shading
//! reads one texture as before.
//!
//! What is traced: the opaque and alpha-masked faces of objects placed in
//! the world, the terrain, and the skeletons (avatars with what they wear,
//! animated objects): their faces are posed on the GPU (shaders/rt_pose.wgsl,
//! the vertex shaders' skinning) into a bottom level each, built again only
//! when the skeleton moved, in a top level of their own. Every instance
//! carries its record, so a hit finds its surface (shaders/rt_common.wgsl):
//! an alpha-masked face lets the ray through under its cutoff. Not yet:
//! blended faces. The shadow maps still shade those passes the mask does
//! not cover (blended faces, reflections).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, OnceLock};

use bytemuck::{Pod, Zeroable};

use crate::arena::RecordStore;
use crate::gpu_cull::{CullFace, CullObject, CullTables, GpuTable, NO_OBJECT, object_flags, pass};
use crate::pipeline_cache::CachedPipelines;
use crate::types::{DrawRecord, flags};

/// The device feature ray tracing needs.
pub const FEATURES: wgpu::Features = wgpu::Features::EXPERIMENTAL_RAY_QUERY;

/// Which faces a ray passes (`AURORA_RT_SHADOWS=front` or `back`): none,
/// those it meets from their front, those it meets from behind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cull {
    #[default]
    None,
    Front,
    Back,
}

impl Cull {
    /// The ray flags of WGSL's `RayDesc` (Vulkan's `gl_RayFlags*`).
    fn ray_flags(self) -> u32 {
        match self {
            Cull::None => 0,
            Cull::Back => 0x10,
            Cull::Front => 0x20,
        }
    }
}

/// `AURORA_RT_SHADOWS`, `AURORA_RT_LIGHTS`, `AURORA_RT_GI` and
/// `AURORA_RT_REFLECTIONS`: the options of Preferences › Graphics › Ray
/// tracing forced for a launch (a comparison, an automated run), whatever
/// the settings and without changing them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Override {
    shadows: Option<bool>,
    lights: Option<bool>,
    indirect: Option<bool>,
    reflections: Option<bool>,
    cull: Cull,
}

impl Override {
    /// From the four variables' values: `0` or `off` forces an option off,
    /// anything else on (`front` and `back`, the sun shadows with that
    /// culling); unset or empty leaves it to the settings.
    fn parse(shadows: Option<&str>, lights: Option<&str>, indirect: Option<&str>, reflections: Option<&str>) -> Self {
        let forced = |value: Option<&str>| match value?.trim() {
            "" => None,
            "0" | "off" => Some(false),
            _ => Some(true),
        };
        Override {
            shadows: forced(shadows),
            lights: forced(lights),
            indirect: forced(indirect),
            reflections: forced(reflections),
            cull: match shadows.map(str::trim) {
                Some("front") => Cull::Front,
                Some("back") => Cull::Back,
                _ => Cull::None,
            },
        }
    }

    fn from_env() -> Self {
        static FORCED: OnceLock<Override> = OnceLock::new();
        let var = |name: &str| std::env::var(name).ok();
        *FORCED.get_or_init(|| {
            Override::parse(
                var("AURORA_RT_SHADOWS").as_deref(),
                var("AURORA_RT_LIGHTS").as_deref(),
                var("AURORA_RT_GI").as_deref(),
                var("AURORA_RT_REFLECTIONS").as_deref(),
            )
        })
    }

    /// What is traced when the settings ask `asked`: nothing without the
    /// sun's shadows, whose scene the others trace.
    fn applied(self, asked: crate::RayTracing) -> crate::RayTracing {
        let shadows = self.shadows.unwrap_or(asked.shadows);
        crate::RayTracing {
            shadows,
            lights: shadows && self.lights.unwrap_or(asked.lights),
            indirect: shadows && self.indirect.unwrap_or(asked.indirect),
            reflections: shadows && self.reflections.unwrap_or(asked.reflections),
        }
    }
}

/// Whether a device is to be opened with [`FEATURES`]: the settings ask for
/// ray tracing (`asked`) or a launch variable forces it.
pub(crate) fn wanted_at_open(asked: bool) -> bool {
    Override::from_env()
        .applied(crate::RayTracing {
            shadows: asked,
            ..Default::default()
        })
        .shadows
}

/// What `device` traces when the settings ask `asked`: nothing on a device
/// opened without ray queries.
pub(crate) fn traced(device: &wgpu::Device, asked: crate::RayTracing) -> crate::RayTracing {
    if device.features().contains(FEATURES) {
        Override::from_env().applied(asked)
    } else {
        crate::RayTracing::default()
    }
}

/// A face to trace: its record, its geometry in the arena, where it stands,
/// and whether it is opaque (an alpha-masked face is tested at each hit).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instance {
    pub record: u32,
    pub base_vertex: i32,
    pub first_index: u32,
    pub index_count: u32,
    pub opaque: bool,
    pub model: [[f32; 4]; 4],
}

/// A rigged or worn face: its world positions exist only once posed, so the
/// pose pass (shaders/rt_pose.wgsl) writes them on the frames its skeleton's
/// bottom level is built again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Posed {
    pub record: u32,
    pub base_vertex: u32,
    pub first_index: u32,
    pub index_count: u32,
    pub opaque: bool,
}

/// A skeleton's faces (an avatar with what it wears, or an animated
/// object), keyed by its palette's first entry: one bottom level each, built
/// again on the frames `rebuild` says, from `Faces::posed[first..first + count]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PosedGroup {
    pub key: u32,
    pub first: u32,
    pub count: u32,
    /// Posed vertices of all its faces (one per index).
    pub vertices: u32,
    /// A hash of its faces: another set makes its bottom level again.
    pub faces: u64,
    pub rebuild: bool,
}

/// The small still faces of a cell of the world ([`CELL`]), merged into one
/// bottom level: the pose pass writes them where they stand, as it does a
/// skeleton's, once, and the level is made again when `version` changes.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    /// Its place ([`cell_of`]) and which part of the cell's faces it holds
    /// ([`PART_FACES`], [`PART_VERTICES`]).
    pub key: u64,
    /// A hash of its faces, their geometry and their places.
    pub version: u64,
    pub faces: Arc<Vec<Posed>>,
    /// Posed vertices of all its faces (one per index).
    pub vertices: u32,
}

/// What the frame traces: faces placed by their model matrix, and the
/// skeletons' faces, posed on the GPU.
#[derive(Clone, Debug, Default)]
pub struct Faces {
    /// The placed faces that keep their place, which the scene's top level
    /// holds ([`StillTracker`]): the large ones each by its own instance,
    /// the small ones merged by cell. Their set's number: the level built
    /// from a set stands until a frame brings another number. (Every frame
    /// carries the set: the renderer does not draw every frame it is sent.)
    pub still: Arc<Vec<Instance>>,
    pub cells: Arc<Vec<Cell>>,
    pub set: u64,
    /// The eye: the cells nearest to it are made first.
    pub eye: [f32; 3],
    /// Why the sets were made, added up since the start (for the log):
    /// sets made at once for a near face, then the faces that were gone,
    /// had moved, had another geometry, were to come in and to go out; and
    /// the gone ones by cause: their slot freed, no geometry, no longer
    /// traced, now a skeleton's.
    pub why: [u64; 10],
    /// What the main thread spent on this, added up since the start: µs in
    /// all, µs on making the sets, and the faces it looked at.
    pub spent: [u64; 3],
    /// The placed faces that moved lately, or wait for the next still set:
    /// placed anew each frame, in the top level of the posed faces.
    pub moving: Vec<Instance>,
    /// The faces of the skeletons built again this frame, by skeleton.
    pub posed: Vec<Posed>,
    pub groups: Vec<PosedGroup>,
}

/// Posed triangles built again in one frame, nearest skeleton first (the
/// nearest is always built); the others keep an earlier frame's pose.
const REBUILD_TRIANGLES: u64 = 1 << 19;
/// How far from its joint a vertex is taken to be when a joint turns (m).
const JOINT_REACH: f32 = 1.0;

/// Frames a placed face that moved keeps its place before it counts as
/// still again (two seconds: one that stops and goes more often than that
/// stays with the moving ones, and never has the scene's level built again).
const STILL_FRAMES: u16 = 120;
/// The most doublings of [`STILL_FRAMES`] a face that keeps moving out of
/// the still set earns (a door, an object a script moves now and then): it
/// then stays with the moving ones for minutes, and the scene's level is
/// not built again each time it goes.
const RELAPSES_MAX: u8 = 6;
/// Frames between two still sets at least, unless a near face of the set
/// moved: wgpu builds the level from scratch, a few milliseconds for a
/// region's faces.
const STILL_EVERY: u64 = 15;
/// Frames a near face of the still set may be gone before a set is made
/// without it: its shadow stays that long. (A face whose slot is handed
/// out again is gone too, and its shadow should stay: made at once, such
/// sets came every few frames on a busy region.)
const QUICK: u64 = 8;
/// Faces whose change waits for a still set that make one as soon as
/// [`STILL_EVERY`] allows (a region that loads, an arrival by teleport).
const MANY: usize = 256;
/// Frames after the world's origin changed in which a face's first change
/// of place is the scene writing it relative to the new origin, not a move
/// ([`StillTracker::rebase`]): the scene does so within its frames' budget,
/// the nearest first, a few seconds for a busy region.
const REBASE_FRAMES: u64 = 600;
/// Frames a few changes wait for a still set: [`SOON`] when the set shows
/// something wrong meanwhile (a far face where it no longer is, or with
/// the shape of another level of detail), [`LATER`] when it only lacks
/// faces that are traced with the moving ones meanwhile, or holds some the
/// eye left behind. Walking through a region changes a few faces all the
/// time: the level is then built a few times a second at most.
const SOON: u64 = 30;
const LATER: u64 = 120;
/// A face of the still set whose move or leaving shows at once, and makes
/// a new set on its frame: nearer than [`NEAR`] (m), or larger than
/// [`NEAR_SIZE`] (its object's radius over its distance). The others wait
/// [`SOON`].
const NEAR: f32 = 32.0;
const NEAR_SIZE: f32 = 0.05;
/// How far past the shadows' distance a face is still traced, by its
/// object's sphere: what stands a little further still casts into it.
const FAR_MARGIN: f32 = 1.25;
/// The smallest a face's object may look (its radius over its distance) and
/// be traced: the shadow of less is under a few pixels.
const SMALL: f32 = 0.003;
/// How much further, or smaller, a traced face may get before it is left
/// out: one at the limit does not come and go as the eye moves.
const KEEP: f32 = 1.25;
/// The most triangles of a still face merged with its cell's others. wgpu
/// walks every instance of a top level on each use and stages them all at
/// each build, and on a region most placed faces are a few triangles each
/// (a busy region: 50,000 of 60,000 had 64 at most, 415,000 triangles
/// together): merged, they are a few hundred instances. A larger face
/// keeps its own instance, and the bottom level its copies share.
const MERGED_TRIANGLES: u32 = 256;
/// The side of a cell (m).
const CELL: f32 = 16.0;
/// The most faces (a geometry each) and posed vertices of a cell's bottom
/// level: the cell's faces past them make another part.
const PART_FACES: usize = 4096;
const PART_VERTICES: u32 = 1 << 19;

/// Frames a look at every face takes, each in its turn, whatever changed:
/// what the changes followed by [`gather`] would miss is noticed this late
/// at most.
const SWEEP: usize = 256;
/// How far the eye moves (m) before every placed face is weighed again
/// (how far and how large it looks: [`FAR_MARGIN`], [`SMALL`], [`NEAR`]),
/// and the frames that takes.
const EYE_STEP: f32 = 0.25;
const WEIGH_FRAMES: usize = 8;

/// A face as the main thread last looked at it.
#[derive(Clone, Copy, Default)]
struct Track {
    /// The frame of the last look (0: never).
    seen: u64,
    /// The frame from which it has kept its place long enough to count as
    /// still.
    rests_at: u64,
    /// Its place, and its geometry, hashed (`place_of`, `shape_of`).
    place: u64,
    /// A placed face's cell ([`cell_of`]).
    cell: u64,
    shape: u32,
    /// A skeleton's face: its skeleton (its palette's first entry).
    skeleton: u32,
    /// What the last look found it to be.
    kind: u8,
    /// Times it moved out of a still set: each doubles the frames it must
    /// keep its place before the next one takes it back.
    relapses: u8,
    /// What it adds to the tally ([`Tally::count`]), to take it back.
    counted: u8,
    /// It has a record and a geometry.
    present: bool,
    /// Near and large enough to trace.
    kept: bool,
    /// Its move or leaving would show at once ([`NEAR`]).
    near: bool,
    /// The last still set holds it; it was near then.
    in_still: bool,
    set_near: bool,
    /// The set holds it where it no longer is: it is traced with the
    /// moving ones as well, until the next set.
    left: bool,
    /// The set holds it with a geometry it no longer has (another level of
    /// detail): it stays so until the next set.
    stale: bool,
    /// The set holds it and a look did not find it (counted once).
    missed: bool,
    /// It is in the list of the moving faces ([`StillTracker::moving`]).
    listed: bool,
    /// A face not traced yet: neither a set nor a frame's moving faces
    /// had it ([`StillTracker::emits`]).
    new: bool,
    /// It was there when the world's origin changed
    /// ([`StillTracker::rebase`]): its place may still be the one before.
    rebase: bool,
}

/// A face's kinds ([`Track::kind`]): not traced, a placed face (opaque or
/// alpha-masked), a skeleton's.
const NOT_TRACED: u8 = 0;
const PLACED_OPAQUE: u8 = 1;
const PLACED_MASKED: u8 = 2;
const POSED: u8 = 3;

impl Track {
    /// Whether it kept its place long enough to count as still.
    fn rests(&self, frame: u64) -> bool {
        frame >= self.rests_at
    }

    fn placed(&self) -> bool {
        matches!(self.kind, PLACED_OPAQUE | PLACED_MASKED)
    }

    /// Whether it is traced with the moving faces: the still set does not
    /// hold it, or not where it is.
    fn moves(&self) -> bool {
        self.present && self.placed() && self.kept && (!self.in_still || self.left)
    }
}

/// What the faces add up to, for [`StillTracker::decide`]: each placed
/// face is counted as its last look left it, and taken back before the
/// next one changes it, so that a frame only touches the faces that changed.
#[derive(Default)]
struct Tally {
    /// Faces of the still set that are there, and the near ones of them.
    found: usize,
    found_near: usize,
    /// Faces the set holds where they no longer are, and with a geometry
    /// they no longer have: what it shows wrong.
    left: usize,
    stale: usize,
    /// Faces the set should hold and does not, and holds and should not.
    arrivals: usize,
    leavers: usize,
    /// A near face of the set moved this frame.
    urgent: bool,
}

/// [`Track::counted`]: it is counted, and as what.
const COUNTED: u8 = 1;
const AS_LEFT: u8 = 2;
const AS_STALE: u8 = 4;
const AS_ARRIVAL: u8 = 8;
const AS_LEAVER: u8 = 16;

impl Tally {
    fn class(&mut self, counted: u8) -> Option<&mut usize> {
        match counted & !COUNTED {
            AS_LEFT => Some(&mut self.left),
            AS_STALE => Some(&mut self.stale),
            AS_ARRIVAL => Some(&mut self.arrivals),
            AS_LEAVER => Some(&mut self.leavers),
            _ => None,
        }
    }

    /// Counts placed face `t`, which is there: whether it is among the
    /// moving faces.
    fn count(&mut self, t: &mut Track, frame: u64) -> bool {
        self.found += t.in_still as usize;
        self.found_near += t.set_near as usize;
        let would = t.kept && t.rests(frame);
        t.counted = COUNTED
            | match (t.left, t.stale, would == t.in_still, would) {
                (true, ..) => AS_LEFT,
                (_, true, ..) => AS_STALE,
                (_, _, true, _) => 0,
                (.., true) => AS_ARRIVAL,
                _ => AS_LEAVER,
            };
        if let Some(n) = self.class(t.counted) {
            *n += 1;
        }
        t.moves()
    }

    /// Takes `t` back, before it changes.
    fn uncount(&mut self, t: &mut Track) {
        if t.counted == 0 {
            return;
        }
        self.found -= t.in_still as usize;
        self.found_near -= t.set_near as usize;
        if let Some(n) = self.class(t.counted) {
            *n -= 1;
        }
        t.counted = 0;
    }
}

/// Which placed faces stand still, so the scene's top level holds them and
/// is built again only when their set changes, while the few that move go
/// to the top level built each frame. wgpu walks every instance of a top
/// level each frame a pass uses it, and builds it from scratch: with a
/// region's tens of thousands of faces in one level built whenever one
/// moved, that was the frame's largest cost on the grid (a busy region:
/// 98,000 faces, 10 to 23 ms on the render thread).
///
/// A near face of the level that moves makes a new set at once (the level
/// would cast its shadow where it no longer is), one that goes within
/// [`QUICK`]. The other changes are gathered ([`MANY`], [`SOON`],
/// [`LATER`]): a face that
/// arrives or comes to rest is among the moving ones meanwhile, so that
/// none is ever in neither (but the small ones of a crowd of arrivals,
/// which wait a few frames for their set: [`StillTracker::emits`]); a far
/// one that moved is in both, a far one that went still in the level; one
/// whose level of detail changed keeps the level's geometry.
///
/// A face that a set had to drop because it was gone, and that comes back
/// as it was (an object a script shows and hides, a geometry that loads
/// again), must keep its place as long as one that moved before a set
/// takes it back ([`Track::relapses`]): it is traced with the moving ones
/// meanwhile, and its blinking makes no set. One that is back before any
/// set was made without it never left.
///
/// A face is looked at when it changes ([`gather`]), not each frame: what
/// its last look found stands until the next.
#[derive(Default)]
pub(crate) struct StillTracker {
    tracks: Vec<Track>,
    tally: Tally,
    /// The faces traced with the moving ones ([`Track::listed`]).
    moving: Vec<u32>,
    /// Faces of the last still set, and the near ones of them.
    held: usize,
    held_near: usize,
    /// The frame of the last still set (0: none yet), and the first frame
    /// since with a change that waits for the next (0: none).
    set_at: u64,
    changed_at: u64,
    /// The frame the world's origin last changed ([`StillTracker::rebase`]).
    rebased_at: u64,
    /// Why the sets were made, added up since the start ([`Faces::why`]).
    why: [u64; 10],
}

/// Why a look did not find a face ([`Faces::why`], after the first six).
const SLOT_FREED: usize = 6;
const NO_GEOMETRY: usize = 7;
const NOT_TRACED_NOW: usize = 8;
const POSED_NOW: usize = 9;

impl StillTracker {
    /// Forgets every face on `frame`: the world's origin changed (a border
    /// crossing, a teleport), they all stand elsewhere and none of them
    /// moved. The scene writes them at their new places over the next
    /// frames: a face's first change of place is then an arrival, which the
    /// next set takes, and not a move, which would trace it on its own for
    /// seconds (a region's faces, tens of thousands, each with a bottom
    /// level to build and a place in the level built each frame).
    fn rebase(&mut self, frame: u64) {
        *self = Self {
            why: self.why,
            rebased_at: frame,
            ..Self::default()
        };
    }

    /// A frame begins, with `faces` face slots.
    fn begin(&mut self, faces: usize) {
        if self.tracks.len() < faces {
            self.tracks.resize(faces, Track::default());
        }
        self.tally.urgent = false;
    }

    /// Whether `frame` already looked at face `id` (or has no such face).
    fn seen(&self, id: usize, frame: u64) -> bool {
        self.tracks.get(id).is_none_or(|t| t.seen == frame)
    }

    /// The skeleton the last look found face `id` in.
    fn skeleton(&self, id: usize) -> Option<u32> {
        self.tracks.get(id).filter(|t| t.present && t.kind == POSED).map(|t| t.skeleton)
    }

    /// Whether the last look found face `id` at `place` and of `shape`.
    fn is(&self, id: usize, place: u64, shape: u32) -> bool {
        self.tracks
            .get(id)
            .is_some_and(|t| t.present && t.place == place && t.shape == shape)
    }

    /// Whether face `id` is traced (for [`KEEP`]).
    fn kept(&self, id: usize) -> bool {
        self.tracks.get(id).is_some_and(|t| t.present && t.kept)
    }

    /// Placed face `id`'s cell, its triangles being weighed there.
    fn cell(&self, id: usize) -> Option<u64> {
        self.tracks.get(id).filter(|t| t.present && t.placed()).map(|t| t.cell)
    }

    /// Counts face `id`, which a look did not find for the reason `why`,
    /// if the still set holds it (for the log).
    fn missing(&mut self, id: usize, why: usize) {
        if let Some(t) = self.tracks.get_mut(id)
            && t.in_still
            && !std::mem::replace(&mut t.missed, true)
        {
            self.why[why] += 1;
        }
    }

    /// Notes that face `id` has no record or no geometry on `frame`, for
    /// the reason `why`. One the still set holds stays so until the set
    /// made for its leaving ([`StillTracker::decide`]).
    fn absent(&mut self, id: usize, frame: u64, why: usize) {
        self.missing(id, why);
        if let Some(t) = self.tracks.get_mut(id) {
            self.tally.uncount(t);
            t.present = false;
            t.seen = frame;
        }
    }

    /// Notes face `id` on `frame` as one that is not a placed face to
    /// trace (`kind`: not traced, or a face of `skeleton`).
    fn other(&mut self, id: usize, frame: u64, place: u64, shape: u32, kind: u8, skeleton: u32) {
        self.missing(id, if kind == POSED { POSED_NOW } else { NOT_TRACED_NOW });
        if let Some(t) = self.tracks.get_mut(id) {
            self.tally.uncount(t);
            // one the still set holds stays so until the set made for its
            // leaving (`decide`: it is no longer found). A placed face that
            // is no longer traced keeps the place and the geometry it had,
            // to be known if it comes back (`visit`).
            let keeps = kind == NOT_TRACED && t.seen != 0 && t.kind != POSED;
            let (place, shape, relapses) = if keeps { (t.place, t.shape, t.relapses) } else { (place, shape, 0) };
            *t = Track {
                seen: frame,
                place,
                shape,
                kind,
                skeleton,
                relapses,
                present: true,
                in_still: t.in_still,
                set_near: t.set_near,
                missed: t.missed,
                listed: t.listed,
                ..Track::default()
            };
        }
    }

    /// Notes placed face `id` on `frame`, at `place` (in `cell`) and of
    /// `shape`, opaque or not, traced or not (`kept`), near or not.
    #[allow(clippy::too_many_arguments)]
    fn visit(&mut self, id: usize, frame: u64, place: u64, shape: u32, cell: u64, kept: bool, near: bool, opaque: bool) {
        let Some(t) = self.tracks.get_mut(id) else {
            return;
        };
        self.tally.uncount(t);
        let kind = if opaque { PLACED_OPAQUE } else { PLACED_MASKED };
        let rebased = t.rebase && t.place != place && frame < self.rebased_at + REBASE_FRAMES;
        if rebased && t.present && t.placed() {
            // written relative to the new origin: it arrives there
            *t = Track {
                rests_at: frame,
                place,
                shape,
                kind,
                present: true,
                listed: t.listed,
                new: true,
                ..Track::default()
            };
        } else if !t.present || !t.placed() {
            // the face that was in this slot, back as it was?
            let back = t.seen != 0 && t.place == place && t.shape == shape;
            if back && t.in_still && !t.left && t.kind == kind {
                // before a set was made without it: it never left
                t.present = true;
            } else {
                // a new face (or a slot handed out again): still from the
                // start, unless it is one a set had to drop. What the set
                // held in its slot is gone: no longer found (`decide`).
                let relapses = if back { t.relapses } else { 0 };
                let wait = if relapses > 0 {
                    (STILL_FRAMES as u64) << relapses.min(RELAPSES_MAX)
                } else {
                    0
                };
                *t = Track {
                    rests_at: frame + wait,
                    place,
                    shape,
                    kind,
                    relapses,
                    present: true,
                    listed: t.listed,
                    new: true,
                    rebase: frame == self.rebased_at,
                    ..Track::default()
                };
            }
        } else if t.place != place {
            // it moved
            t.rebase = false;
            t.place = place;
            t.shape = shape;
            t.kind = kind;
            if t.in_still {
                if !t.left {
                    t.relapses = t.relapses.saturating_add(1);
                }
                t.left = true;
                t.stale = false;
                self.tally.urgent |= t.near || near;
            }
            t.rests_at = frame + ((STILL_FRAMES as u64) << t.relapses.min(RELAPSES_MAX));
        } else if t.shape != shape || t.kind != kind {
            // another geometry at the same place: a level of detail
            t.shape = shape;
            t.kind = kind;
            t.stale = t.in_still && !t.left;
        }
        t.seen = frame;
        t.cell = cell;
        t.kept = kept;
        t.near = near;
        if self.tally.count(t, frame) && !std::mem::replace(&mut t.listed, true) {
            self.moving.push(id as u32);
        }
    }

    /// Weighs placed face `id` again, the eye having moved: traced or not,
    /// near or not.
    fn weigh(&mut self, id: usize, frame: u64, kept: bool, near: bool) {
        let Some(t) = self.tracks.get_mut(id) else {
            return;
        };
        if !t.present || !t.placed() || (t.kept == kept && t.near == near) {
            return;
        }
        self.tally.uncount(t);
        t.kept = kept;
        t.near = near;
        if self.tally.count(t, frame) && !std::mem::replace(&mut t.listed, true) {
            self.moving.push(id as u32);
        }
    }

    /// Whether this frame makes a new still set, after the frame's looks;
    /// the tracks then say which faces it holds.
    fn decide(&mut self, frame: u64) -> bool {
        // the faces among the moving ones come to rest with no look at
        // them: counted again each frame
        let (tracks, tally) = (&mut self.tracks, &mut self.tally);
        self.moving.retain(|&id| {
            let t = &mut tracks[id as usize];
            if !t.moves() {
                t.listed = false;
                return false;
            }
            tally.uncount(t);
            tally.count(t, frame);
            true
        });
        let tally = &self.tally;
        let gone = self.held - tally.found.min(self.held);
        let urgent = tally.urgent;
        let gone_near = tally.found_near < self.held_near;
        let wrong = tally.left + tally.stale + gone;
        let changes = wrong + tally.arrivals + tally.leavers;
        if changes == 0 && !urgent {
            self.changed_at = 0;
            return false;
        }
        if self.changed_at == 0 {
            self.changed_at = frame;
        }
        let patience = match (gone_near, wrong > 0) {
            (true, _) => QUICK,
            (false, true) => SOON,
            (false, false) => LATER,
        };
        let gathered = changes >= MANY || frame - self.changed_at >= patience;
        if !(self.set_at == 0 || urgent || (gathered && frame - self.set_at >= STILL_EVERY)) {
            return false;
        }
        for (sum, n) in self
            .why
            .iter_mut()
            .zip([urgent as usize, gone, tally.left, tally.stale, tally.arrivals, tally.leavers])
        {
            *sum += n as u64;
        }
        self.held = 0;
        self.held_near = 0;
        for t in &mut self.tracks {
            let placed = t.present && t.placed();
            if t.in_still && !placed {
                // this set drops it because it is gone
                t.relapses = t.relapses.saturating_add(1);
            }
            t.in_still = placed && t.kept && t.rests(frame);
            t.new &= !t.in_still;
            t.set_near = t.in_still && t.near;
            t.left = false;
            t.stale = false;
            t.missed = false;
            // (as the set holds what it should, nothing waits for the next)
            t.counted = if placed { COUNTED } else { 0 };
            self.held += t.in_still as usize;
            self.held_near += t.set_near as usize;
        }
        self.tally = Tally {
            found: self.held,
            found_near: self.held_near,
            ..Tally::default()
        };
        let tracks = &mut self.tracks;
        self.moving.retain(|&id| {
            let t = &mut tracks[id as usize];
            t.listed = t.moves();
            t.listed
        });
        self.set_at = frame;
        self.changed_at = 0;
        true
    }

    /// The faces traced with the moving ones, after [`StillTracker::decide`].
    #[cfg(test)]
    fn moving(&self) -> &[u32] {
        &self.moving
    }

    /// Whether face `id` of the moving ones is traced this frame; `small`
    /// says a set would merge it with its cell's others. A new small face
    /// is not while [`MANY`] wait for a set, which then comes within
    /// [`STILL_EVERY`]: traced meanwhile, each would have a bottom level of
    /// its own built, for those few frames (an arrival by teleport: tens of
    /// thousands of them, and as many instances in the level built each
    /// frame).
    fn emits(&mut self, id: u32, small: bool) -> bool {
        let t = &mut self.tracks[id as usize];
        if t.new && small && t.counted & AS_ARRIVAL != 0 && self.tally.arrivals >= MANY {
            return false;
        }
        t.new = false;
        true
    }

    /// Face `id` of the still set: its cell, its place and geometry in one
    /// number (for its cell's version), and whether it is opaque.
    fn held(&self, id: usize) -> Option<(u64, u64, bool)> {
        self.tracks
            .get(id)
            .filter(|t| t.in_still)
            .map(|t| (t.cell, t.place ^ ((t.shape as u64) << 32 | t.kind as u64), t.kind == PLACED_OPAQUE))
    }

    #[cfg(test)]
    fn in_still(&self, id: usize) -> bool {
        self.tracks.get(id).is_some_and(|t| t.in_still)
    }
}

/// The faces of each object, in a chain, so that an object's change (it is
/// hidden or shown, worn or put down, its sphere moves) finds them.
#[derive(Default)]
struct ByObject {
    /// Each object's first face ([`NO_OBJECT`]: none).
    first: Vec<u32>,
    /// Each face's object, and the faces before and after it in the
    /// object's chain.
    links: Vec<[u32; 3]>,
}

impl ByObject {
    /// Face `face` is `object`'s ([`NO_OBJECT`]: no one's).
    fn set(&mut self, face: usize, object: u32) {
        if self.links.len() <= face {
            self.links.resize(face + 1, [NO_OBJECT; 3]);
        }
        let [old, before, after] = self.links[face];
        if old == object {
            return;
        }
        if old != NO_OBJECT {
            match before {
                NO_OBJECT => self.first[old as usize] = after,
                before => self.links[before as usize][2] = after,
            }
            if after != NO_OBJECT {
                self.links[after as usize][1] = before;
            }
        }
        let mut next = NO_OBJECT;
        if object != NO_OBJECT {
            if self.first.len() <= object as usize {
                self.first.resize(object as usize + 1, NO_OBJECT);
            }
            next = std::mem::replace(&mut self.first[object as usize], face as u32);
            if next != NO_OBJECT {
                self.links[next as usize][1] = face as u32;
            }
        }
        self.links[face] = [object, NO_OBJECT, next];
    }

    fn faces(&self, object: u32) -> impl Iterator<Item = usize> + '_ {
        let mut at = self.first.get(object as usize).copied().unwrap_or(NO_OBJECT);
        std::iter::from_fn(move || {
            let face = (at != NO_OBJECT).then_some(at as usize)?;
            at = self.links[face][2];
            Some(face)
        })
    }
}

/// The cell of a face placed by `model`: its origin's, in three signed 16
/// bit numbers.
fn cell_of(model: &[[f32; 4]; 4]) -> u64 {
    let at = |axis: usize| ((model[3][axis] / CELL).floor().clamp(-32768.0, 32767.0) as i16 as u16) as u64;
    at(0) << 32 | at(1) << 16 | at(2)
}

fn cell_centre(cell: u64) -> glam::Vec3 {
    let at = |shift: u32| ((cell >> shift) as u16 as i16 as f32 + 0.5) * CELL;
    glam::Vec3::new(at(32), at(16), at(0))
}

/// The distance a merged face's size is weighed at, `distance` being its
/// cell's: the power of two under it (0 within a cell's side). Weighed at
/// its own distance, the small faces of every cell around would come and
/// go as the eye moves, and each makes its cell's level again; in steps,
/// a cell changes when the eye crosses one.
fn size_step(distance: f32) -> f32 {
    if distance < CELL {
        return 0.0;
    }
    // the exponent alone
    f32::from_bits(distance.to_bits() & 0xff80_0000)
}

/// A small face of a still set, for [`cells_of`].
#[derive(Clone, Copy)]
struct Small {
    cell: u64,
    /// Its place and geometry ([`StillTracker::mark`]).
    mark: u64,
    face: Posed,
}

/// The cells of a still set's small faces, by key. A cell with the faces
/// `before` had keeps its list (and its version, so its level stands).
fn cells_of(small: &mut [Small], before: &[Cell]) -> Vec<Cell> {
    small.sort_unstable_by_key(|s| (s.cell, s.face.record));
    let mut cells = Vec::new();
    let mut start = 0;
    let mut part = 0u64;
    while start < small.len() {
        let cell = small[start].cell;
        // a part: the cell's faces until it is full
        let mut end = start;
        let mut vertices = 0u32;
        let mut version = 0x9e37_79b9_7f4a_7c15u64;
        while end < small.len()
            && small[end].cell == cell
            && end - start < PART_FACES
            && (end == start || vertices + small[end].face.index_count <= PART_VERTICES)
        {
            let f = &small[end].face;
            for word in [
                small[end].mark,
                (f.record as u64) << 32 | f.base_vertex as u64,
                (f.first_index as u64) << 32 | f.index_count as u64,
                f.opaque as u64,
            ] {
                version = (version ^ word).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(23);
            }
            vertices += f.index_count;
            end += 1;
        }
        let key = cell << 16 | part.min(0xffff);
        let faces = match before.binary_search_by_key(&key, |c| c.key) {
            Ok(n) if before[n].version == version => before[n].faces.clone(),
            _ => {
                // the opaque ones first: the level's first geometry
                // ([`cell_geometries`])
                let mut faces: Vec<Posed> = small[start..end].iter().map(|s| s.face).collect();
                faces.sort_by_key(|f| !f.opaque);
                Arc::new(faces)
            }
        };
        cells.push(Cell {
            key,
            version,
            faces,
            vertices,
        });
        part = if small.get(end).is_some_and(|s| s.cell == cell) {
            part + 1
        } else {
            0
        };
        start = end;
    }
    cells
}

/// The geometries of a merged cell's bottom level, its `faces` being the
/// opaque ones then the alpha-masked ones ([`cells_of`]): the posed
/// vertices of each kind it has, and whether opaque. Two at most, not one
/// a face: making a level costs the render thread about a microsecond a
/// geometry, and building it as much again (a region's arrival made 6,000
/// a frame). The traversal still commits the opaque ones' hits itself.
fn cell_geometries(faces: &[Posed]) -> impl Iterator<Item = (u32, bool)> {
    let split = faces.partition_point(|f| f.opaque);
    let vertices = |faces: &[Posed]| faces.iter().map(|f| f.index_count).sum::<u32>();
    [(vertices(&faces[..split]), true), (vertices(&faces[split..]), false)]
        .into_iter()
        .filter(|&(vertices, _)| vertices > 0)
}

/// What finds the face of a hit in a merged cell's level, whose geometries
/// are not its faces ([`cell_geometries`]): the number of its opaque faces
/// and of its masked ones, then for each face, in their order, its first
/// triangle in its geometry and its record (rt_common.wgsl `hit_of`).
fn cell_table(faces: &[Posed]) -> Vec<u32> {
    let split = faces.partition_point(|f| f.opaque);
    let mut table = Vec::with_capacity(2 + 2 * faces.len());
    table.extend([split as u32, (faces.len() - split) as u32]);
    for kind in [&faces[..split], &faces[split..]] {
        let mut triangle = 0;
        for f in kind {
            table.extend([triangle, f.record]);
            triangle += f.index_count / 3;
        }
    }
    table
}

/// A placed face's place in one number.
fn place_of(model: &[[f32; 4]; 4]) -> u64 {
    let pair = |c: usize, r: usize, c2: usize, r2: usize| (model[c][r].to_bits() as u64) << 32 | model[c2][r2].to_bits() as u64;
    // the matrix without its last row, which no placement changes
    let words = [
        pair(0, 0, 0, 1),
        pair(0, 2, 1, 0),
        pair(1, 1, 1, 2),
        pair(2, 0, 2, 1),
        pair(2, 2, 3, 0),
        pair(3, 1, 3, 2),
    ];
    let mut h = 0x9e37_79b9_7f4a_7c15u64;
    for word in words {
        h = (h ^ word).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(23);
    }
    h
}

/// A face's geometry (and its pass and object) in one number.
fn shape_of(face: &CullFace) -> u32 {
    let mut h = 0x811c_9dc5u32;
    for word in [face.base_vertex as u32, face.first_index, face.index_count, face.bits, face.object] {
        h = (h ^ word).wrapping_mul(0x0100_0193).rotate_left(13);
    }
    h
}

/// What the main thread remembers of the faces it traces, so that a frame
/// only looks at those that changed: the placed ones, still or moving
/// ([`StillTracker`]), and each skeleton's with its last built pose
/// ([`Skeleton`]).
#[derive(Default)]
pub(crate) struct PoseTracker {
    frame: u64,
    placed: StillTracker,
    by_object: ByObject,
    skeletons: BTreeMap<u32, Skeleton>,
    /// The last still set, with its number.
    still: Arc<Vec<Instance>>,
    cells: Arc<Vec<Cell>>,
    small: Vec<Small>,
    set: u64,
    /// The records and the objects that changed since the last frame.
    changed: Vec<u32>,
    objects: Vec<u32>,
    scratch: Vec<usize>,
    /// Where the turns of [`SWEEP`] and of the weighing are, the faces
    /// still to weigh, and the eye and the reach they are weighed for.
    sweep: usize,
    cursor: usize,
    owed: usize,
    weighed_at: glam::Vec3,
    far: f32,
    /// `Faces::spent`.
    spent: [u64; 3],
}

/// A skeleton's faces, and the pose its bottom level was last built in: a
/// skeleton whose vertices moved less than a shadow ray's start can tell
/// keeps its level (wgpu has no refit: a level is only built again from
/// scratch), the nearest skeletons first within a frame's budget.
#[derive(Default)]
struct Skeleton {
    /// Its faces' records, in order.
    ids: Vec<u32>,
    /// One of them came, went or changed since [`Skeleton::read`].
    dirty: bool,
    /// The faces as last read, with their models, the first one's record
    /// flags (how it is placed), their hash and their vertices.
    faces: Vec<Posed>,
    models: Vec<[[f32; 4]; 4]>,
    first: u32,
    hash: u64,
    vertices: u32,
    /// A level was built for these faces (one left waiting by the budget
    /// has none yet), with these joints and models.
    built: bool,
    joints: Vec<[[f32; 4]; 4]>,
    built_models: Vec<[[f32; 4]; 4]>,
    /// The most a face's own model moved it since, and the most a joint
    /// did, as each was last written (infinite: past what a shadow shows).
    moved: f32,
    drifted: f32,
}

impl Skeleton {
    /// Reads its faces again: each with its model and its record's flags.
    fn read(&mut self, faces: impl Iterator<Item = (Posed, [[f32; 4]; 4], u32)>) {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.faces.clear();
        self.models.clear();
        self.vertices = 0;
        for (face, model, rf) in faces {
            if self.faces.is_empty() {
                self.first = rf;
            }
            (face.record, face.base_vertex, face.first_index, face.index_count, face.opaque).hash(&mut hasher);
            self.vertices += face.index_count;
            self.faces.push(face);
            self.models.push(model);
        }
        let hash = hasher.finish();
        if hash != self.hash {
            // another face set: its level is made again
            self.hash = hash;
            self.built = false;
        }
        self.moved = if self.built_models.len() == self.models.len() {
            self.built_models.iter().zip(&self.models).fold(0.0, |d, (a, b)| d.max(drift(a, b)))
        } else {
            f32::INFINITY
        };
        self.dirty = false;
    }

    /// Where it stands, as rt_pose.wgsl places its first face.
    fn origin(&self, key: u32, palettes: &[[[f32; 4]; 4]]) -> glam::Vec3 {
        let Some(model) = self.models.first() else {
            return glam::Vec3::ZERO;
        };
        let palette = |i: u32| palettes.get(i as usize);
        let at = match (self.first & flags::SKINNED != 0, self.first & flags::ANCHORED != 0) {
            (true, true) => palette(key + crate::types::SKELETON_ANCHOR).map(|m| m[3]),
            (false, true) => palette(key).map(|m| mul(m, model)[3]),
            _ => None,
        }
        .unwrap_or(model[3]);
        glam::Vec3::new(at[0], at[1], at[2])
    }
}

/// How far a vertex can have moved between two placements of its joint.
fn drift(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> f32 {
    let mut d = 0.0f32;
    for c in 0..3 {
        for r in 0..3 {
            d = d.max((a[c][r] - b[c][r]).abs() * JOINT_REACH);
        }
    }
    for r in 0..3 {
        d = d.max((a[3][r] - b[3][r]).abs());
    }
    d
}

/// The most a vertex can have moved between the joints `built` and `now`:
/// infinite past `allowed` (a skeleton that moves is past it at its first
/// joints: most are not read).
fn joint_drift(built: &[[[f32; 4]; 4]], now: &[[[f32; 4]; 4]], allowed: f32) -> f32 {
    let mut most = 0.0f32;
    for (a, b) in built.iter().zip(now) {
        most = most.max(drift(a, b));
        if most > allowed {
            return f32::INFINITY;
        }
    }
    most
}

/// The drift a skeleton at `distance` may have before its shadow shows it:
/// the trace's ray start offset there (rt_shadows.wgsl).
fn drift_allowed(distance: f32) -> f32 {
    0.01 + 0.002 * distance
}

fn mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    (glam::Mat4::from_cols_array_2d(a) * glam::Mat4::from_cols_array_2d(b)).to_cols_array_2d()
}

/// A face of the records as a placed instance.
fn instance_of(id: usize, face: &CullFace, record: &DrawRecord) -> Instance {
    Instance {
        record: id as u32,
        base_vertex: face.base_vertex,
        first_index: face.first_index,
        index_count: face.index_count - face.index_count % 3,
        opaque: record.flags[0] & flags::ALPHA_MASK == 0,
        model: record.model,
    }
}

/// Whether a face is a skeleton's (an avatar's, what it wears, an animated
/// object's): its world positions exist only once posed.
fn is_posed(object: &CullObject, rf: u32) -> bool {
    object_posed(object) || rf & (flags::SKINNED | flags::ANCHORED | flags::LIP_SYNC) != 0
}

fn object_posed(object: &CullObject) -> bool {
    object.flags & (object_flags::AVATAR | object_flags::RIGGED | object_flags::POSED) != 0 || object.anchor != 0
}

/// Whether an object changed in a way the tracing of its faces follows
/// ([`PoseTracker::look`]): it is there or not, a HUD's, hidden, posed, or
/// its sphere moved while it is placed (a posed one's moves each frame, and
/// is not read).
fn object_differs(a: &CullObject, b: &CullObject) -> bool {
    let read = object_flags::ACTIVE | object_flags::HUD | object_flags::HIDDEN;
    (a.flags ^ b.flags) & read != 0 || object_posed(a) != object_posed(b) || (a.sphere != b.sphere && !object_posed(b))
}

/// Whether a placed face of `triangles` triangles in `cell` is traced, and
/// whether its move would show at once, by its object's sphere (the
/// terrain: always). `kept` says it is traced now.
fn weigh(object: Option<&CullObject>, triangles: u32, cell: u64, camera: glam::Vec3, far: f32, kept: bool) -> (bool, bool) {
    let Some(o) = object else {
        return (true, true);
    };
    let ease = if kept { KEEP } else { 1.0 };
    let distance = glam::Vec3::new(o.sphere[0], o.sphere[1], o.sphere[2]).distance(camera);
    let radius = o.sphere[3];
    let weighed = if triangles <= MERGED_TRIANGLES {
        // (from the cell's nearest corner: no further than the face)
        size_step(cell_centre(cell).distance(camera) - CELL * 0.87)
    } else {
        distance
    };
    (
        distance - radius < far * ease && radius * ease >= SMALL * weighed,
        distance - radius < NEAR || radius >= NEAR_SIZE * distance,
    )
}

/// What a look at a face reads.
struct World<'a> {
    records: &'a RecordStore,
    objects: &'a GpuTable<CullObject>,
    camera: glam::Vec3,
    far: f32,
    frame: u64,
}

impl World<'_> {
    fn object(&self, face: &CullFace) -> Option<&CullObject> {
        match face.object {
            NO_OBJECT => None,
            object => self.objects.get(object as usize),
        }
    }
}

impl PoseTracker {
    /// Looks at face `id`: whether it is one to trace (the opaque and
    /// alpha-masked faces of the terrain and of the objects that are no
    /// HUD and not hidden; blended ones are not traced yet), a skeleton's
    /// or a placed one, and where.
    fn look(&mut self, id: usize, w: &World) {
        if self.placed.seen(id, w.frame) {
            return;
        }
        self.spent[2] += 1;
        let was = self.placed.skeleton(id);
        let found = match (w.records.faces.get(id), w.records.get(id as u32)) {
            (Some(face), Some(record)) if face.index_count >= 3 && face.base_vertex >= 0 => Ok((face, record)),
            (Some(_), Some(_)) => Err(NO_GEOMETRY),
            _ => Err(SLOT_FREED),
        };
        let (face, record) = match found {
            Ok(found) => found,
            Err(why) => {
                self.leave(was, id);
                self.by_object.set(id, NO_OBJECT);
                self.placed.absent(id, w.frame, why);
                return;
            }
        };
        self.by_object.set(id, face.object);
        let rf = record.flags[0];
        let object = w.object(face);
        let traced = match object {
            None => face.object == NO_OBJECT && rf & flags::TERRAIN != 0,
            Some(object) => {
                object.flags & object_flags::ACTIVE != 0
                    && object.flags & (object_flags::HUD | object_flags::HIDDEN) == 0
                    && matches!(face.bits & pass::BITS, pass::OPAQUE | pass::OPAQUE_2S | pass::MASK | pass::MASK_2S)
                    && rf & flags::ALPHA_BLEND == 0
            }
        };
        let (place, shape) = (place_of(&record.model), shape_of(face));
        if !traced {
            self.leave(was, id);
            self.placed.other(id, w.frame, place, shape, NOT_TRACED, 0);
            return;
        }
        if object.is_some_and(|o| is_posed(o, rf)) {
            let key = record.flags[1];
            // (how it is posed and tested is part of what it is)
            let shape = shape ^ (rf & (flags::ALPHA_MASK | flags::SKINNED | flags::ANCHORED)).wrapping_mul(0x9e37_79b1);
            let same = was == Some(key) && self.placed.is(id, place, shape);
            self.placed.other(id, w.frame, place, shape, POSED, key);
            if was != Some(key) {
                self.leave(was, id);
                let ids = &mut self.skeletons.entry(key).or_default().ids;
                if let Err(at) = ids.binary_search(&(id as u32)) {
                    ids.insert(at, id as u32);
                }
            }
            if !same && let Some(s) = self.skeletons.get_mut(&key) {
                s.dirty = true;
            }
            return;
        }
        self.leave(was, id);
        let cell = cell_of(&record.model);
        let (kept, near) = weigh(object, face.index_count / 3, cell, w.camera, w.far, self.placed.kept(id));
        self.placed
            .visit(id, w.frame, place, shape, cell, kept, near, rf & flags::ALPHA_MASK == 0);
    }

    /// Face `id` is no longer `skeleton`'s.
    fn leave(&mut self, skeleton: Option<u32>, id: usize) {
        if let Some(s) = skeleton.and_then(|key| self.skeletons.get_mut(&key)) {
            if let Ok(at) = s.ids.binary_search(&(id as u32)) {
                s.ids.remove(at);
            }
            s.dirty = true;
        }
    }

    /// Weighs placed face `id` again, for where the eye is now.
    fn weigh(&mut self, id: usize, w: &World) {
        let (Some(cell), Some(face)) = (self.placed.cell(id), w.records.faces.get(id)) else {
            return;
        };
        let (kept, near) = weigh(w.object(face), face.index_count / 3, cell, w.camera, w.far, self.placed.kept(id));
        self.placed.weigh(id, w.frame, kept, near);
    }
}

/// The faces to trace this frame, from the records and the culling tables
/// the main thread keeps: the placed ones by their record, the skeletons'
/// posed. `palettes` is what the GPU's joint palettes hold and `written`
/// the entries written since the last frame (taken), `camera` the eye,
/// `far` the distance the shadows cover: a placed face further than
/// that (by [`FAR_MARGIN`]) or too small to cast a shadow that shows
/// ([`SMALL`]) is left out. `shifted` says the world's origin moved since
/// the last frame.
///
/// A region has a hundred thousand faces and this runs on the main thread:
/// a frame looks at the faces whose record, geometry or object changed
/// since the last one (the tables tell: `RecordStore::take_changed`,
/// `GpuTable::take_touched`), weighs the placed ones again when the eye
/// moved ([`EYE_STEP`]), and looks at every face in turn over [`SWEEP`]
/// frames. Looking at each face on each frame took 3 ms on a busy region.
#[allow(clippy::too_many_arguments)]
pub(crate) fn gather(
    records: &mut RecordStore,
    cull: &mut CullTables,
    palettes: &[[[f32; 4]; 4]],
    written: &mut Vec<std::ops::Range<usize>>,
    camera: glam::Vec3,
    far: f32,
    shifted: bool,
    tracker: &mut PoseTracker,
    out: &mut Faces,
) {
    let started = std::time::Instant::now();
    tracker.frame += 1;
    let frame = tracker.frame;
    let far = far.max(16.0) * FAR_MARGIN;
    // (both: each starts being followed)
    let followed = records.watch_changes() & cull.objects.watch(object_differs);
    tracker.changed.clear();
    records.take_changed(&mut tracker.changed);
    tracker.objects.clear();
    cull.objects.take_touched(&mut tracker.objects);
    merge(written);
    if shifted {
        tracker.placed.rebase(frame);
        tracker.by_object = ByObject::default();
        tracker.skeletons.clear();
        tracker.still = Arc::default();
        tracker.cells = Arc::default();
        tracker.set += 1;
    }
    // a top level's custom index holds 24 bits, one of them the merged
    // cells' (rt_common.wgsl `hit_of`)
    let faces = records.faces.len().min(1 << 23);
    tracker.placed.begin(faces);
    let world = World {
        records,
        objects: &cull.objects,
        camera,
        far,
        frame,
    };
    if !followed || shifted {
        // what changed is not known: every face
        for id in 0..faces {
            tracker.look(id, &world);
        }
        (tracker.weighed_at, tracker.far, tracker.owed) = (camera, far, 0);
    } else {
        let changed = std::mem::take(&mut tracker.changed);
        for &id in changed.iter().filter(|&&id| (id as usize) < faces) {
            tracker.look(id as usize, &world);
        }
        tracker.changed = changed;
        let mut ids = std::mem::take(&mut tracker.scratch);
        for &object in &tracker.objects {
            ids.extend(tracker.by_object.faces(object).filter(|&id| id < faces));
        }
        for id in ids.drain(..) {
            tracker.look(id, &world);
        }
        tracker.scratch = ids;
        for _ in 0..faces.div_ceil(SWEEP) {
            tracker.sweep = (tracker.sweep + 1) % faces;
            tracker.look(tracker.sweep, &world);
        }
        if camera.distance(tracker.weighed_at) > EYE_STEP || far != tracker.far {
            (tracker.weighed_at, tracker.far, tracker.owed) = (camera, far, faces);
        }
        let turn = faces.div_ceil(WEIGH_FRAMES).min(tracker.owed);
        tracker.owed -= turn;
        for _ in 0..turn {
            tracker.cursor = (tracker.cursor + 1) % faces;
            tracker.weigh(tracker.cursor, &world);
        }
    }
    let records = world.records;
    // the still set, on the frames it changes
    if tracker.placed.decide(frame) {
        let set_started = std::time::Instant::now();
        let placed = &tracker.placed;
        let mut large = Vec::new();
        tracker.small.clear();
        for id in 0..faces {
            let (Some((cell, mark, opaque)), Some(face)) = (placed.held(id), records.faces.get(id)) else {
                continue;
            };
            if face.index_count / 3 > MERGED_TRIANGLES {
                if let Some(record) = records.get(id as u32) {
                    large.push(instance_of(id, face, record));
                }
                continue;
            }
            // (its cell and whether it is opaque as its last look found
            // them: a still face's record is not read)
            tracker.small.push(Small {
                cell,
                mark,
                face: Posed {
                    record: id as u32,
                    base_vertex: face.base_vertex as u32,
                    first_index: face.first_index,
                    index_count: face.index_count - face.index_count % 3,
                    opaque,
                },
            });
        }
        tracker.still = Arc::new(large);
        tracker.cells = Arc::new(cells_of(&mut tracker.small, &tracker.cells));
        tracker.set += 1;
        tracker.spent[1] += set_started.elapsed().as_micros() as u64;
    }
    out.still = tracker.still.clone();
    out.cells = tracker.cells.clone();
    out.set = tracker.set;
    out.eye = camera.to_array();
    out.why = tracker.placed.why;
    out.moving.clear();
    let moving = std::mem::take(&mut tracker.placed.moving);
    out.moving.extend(moving.iter().filter_map(|&id| {
        let (face, record) = (records.faces.get(id as usize)?, records.get(id)?);
        tracker
            .placed
            .emits(id, face.index_count / 3 <= MERGED_TRIANGLES)
            .then(|| instance_of(id as usize, face, record))
    }));
    tracker.placed.moving = moving;
    for s in tracker.skeletons.values_mut().filter(|s| s.dirty) {
        let ids = std::mem::take(&mut s.ids);
        s.read(ids.iter().filter_map(|&id| {
            let (face, record) = (records.faces.get(id as usize)?, records.get(id)?);
            let rf = record.flags[0];
            let face = Posed {
                record: id,
                base_vertex: face.base_vertex as u32,
                first_index: face.first_index,
                index_count: face.index_count - face.index_count % 3,
                opaque: rf & flags::ALPHA_MASK == 0,
            };
            Some((face, record.model, rf))
        }));
        s.ids = ids;
    }
    out.posed.clear();
    out.groups.clear();
    choose_rebuilds(palettes, written, camera, &mut tracker.skeletons, out);
    written.clear();
    tracker.spent[0] += started.elapsed().as_micros() as u64;
    out.spent = tracker.spent;
}

/// Sorts `ranges` and joins those that touch.
fn merge(ranges: &mut Vec<std::ops::Range<usize>>) {
    ranges.sort_unstable_by_key(|r| r.start);
    let mut kept = 0;
    for i in 1..ranges.len() {
        if ranges[i].start <= ranges[kept].end {
            ranges[kept].end = ranges[kept].end.max(ranges[i].end);
        } else {
            kept += 1;
            ranges[kept] = ranges[i].clone();
        }
    }
    ranges.truncate(kept + 1);
}

/// Picks the skeletons built again this frame: new ones, those whose faces
/// changed, and those whose vertices drifted past what a shadow shows,
/// nearest first within [`REBUILD_TRIANGLES`]; and lists them all.
/// `written` is the palettes' entries written since the last frame, in
/// order and apart ([`merge`]): only those are read, the other joints are
/// as they were last measured (a skeleton's are 261 matrices, a crowd's
/// megabytes, and each skeleton's anchor is written every frame).
fn choose_rebuilds(
    palettes: &[[[f32; 4]; 4]],
    written: &[std::ops::Range<usize>],
    camera: glam::Vec3,
    skeletons: &mut BTreeMap<u32, Skeleton>,
    out: &mut Faces,
) {
    let stride = crate::types::PALETTE_STRIDE as usize;
    let joints_of = |key: u32| {
        palettes
            .get(key as usize..(key as usize + stride).min(palettes.len()))
            .unwrap_or(&[])
    };
    skeletons.retain(|_, s| !s.faces.is_empty());
    // the skeletons that must be built again, nearest first, within budget
    let mut must: Vec<(f32, u32)> = Vec::new();
    let mut next = 0;
    for (&key, s) in skeletons.iter_mut() {
        let distance = s.origin(key, palettes).distance(camera);
        let allowed = drift_allowed(distance);
        let joints = joints_of(key);
        let first = key as usize;
        while written.get(next).is_some_and(|w| w.end <= first) {
            next += 1;
        }
        if s.joints.len() != joints.len() {
            s.drifted = f32::INFINITY;
        }
        let mut at = next;
        while s.drifted <= allowed
            && let Some(w) = written.get(at).filter(|w| w.start < first + joints.len())
        {
            let (a, b) = (w.start.max(first) - first, w.end.min(first + joints.len()) - first);
            s.drifted = s.drifted.max(joint_drift(&s.joints[a..b], &joints[a..b], allowed));
            at += 1;
        }
        if !s.built || s.moved > allowed || s.drifted > allowed {
            must.push((distance, key));
        }
    }
    must.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut chosen: Vec<u32> = Vec::new();
    let mut triangles = 0u64;
    for (_, key) in must {
        let tris: u64 = skeletons
            .get(&key)
            .map_or(0, |s| s.faces.iter().map(|f| f.index_count as u64 / 3).sum());
        if triangles > 0 && triangles + tris > REBUILD_TRIANGLES {
            continue;
        }
        triangles += tris;
        chosen.push(key);
    }
    chosen.sort_unstable();
    for (&key, s) in skeletons.iter_mut() {
        let rebuild = chosen.binary_search(&key).is_ok();
        let first = out.posed.len() as u32;
        if rebuild {
            out.posed.extend_from_slice(&s.faces);
            s.built = true;
            s.joints.clear();
            s.joints.extend_from_slice(joints_of(key));
            s.built_models.clone_from(&s.models);
            (s.moved, s.drifted) = (0.0, 0.0);
        }
        out.groups.push(PosedGroup {
            key,
            first,
            count: if rebuild { s.faces.len() as u32 } else { 0 },
            vertices: s.vertices,
            faces: s.hash,
            rebuild,
        });
    }
}

/// The pass's constants (must match `Params` in rt_shadows.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct ParamsU {
    pub world_from_clip: [[f32; 4]; 4],
    pub camera: [f32; 4],
    pub light: [f32; 4],
    pub screen: [f32; 4],
    pub mode: [f32; 4],
    pub soft: [f32; 4],
}

/// The tangent of the radius of the sun's or the moon's disc as the traced
/// shadows take it: half a degree, about twice the real one, so a
/// penumbra is a centimetre wide per 60 cm from what casts it (the Legacy
/// Viewer's shadow maps are blurred wider than the real sun's too).
const SUN_DISC_TAN: f32 = 0.0087;

/// Bytes of an arena vertex (`types::Vertex`), its position first.
const VERTEX_BYTES: u64 = std::mem::size_of::<crate::types::Vertex>() as u64;
/// The most vertices a face's u16 indices reach past its first.
const FACE_VERTICES: u64 = 1 << 16;
/// Triangles of bottom levels built in one frame: the others wait (and their
/// faces are not traced) until a later frame.
const BUILD_TRIANGLES: u64 = 1 << 20;
/// Bottom levels a frame may make, whatever their triangles: the faces of
/// a region that comes or goes are tens of thousands of small ones. How
/// many of them it does make, [`BUILD_TIME`] says.
const BUILD_LEVELS: usize = 512;
/// The time a frame gives to making the bottom levels it chose: one costs
/// the render thread 2 to 11 microseconds, and a hundred now and then (350
/// of them took a frame 4 ms). Those it has no time for wait, as those
/// past the other budgets do ([`unmade`]).
const BUILD_TIME: std::time::Duration = std::time::Duration::from_micros(500);

/// Whether a frame that already builds `levels` bottom levels of
/// `triangles` in all has room for one more of `more` triangles (the first
/// always has).
fn build_room(levels: usize, triangles: u64, more: u64) -> bool {
    levels < BUILD_LEVELS && (triangles == 0 || triangles + more <= BUILD_TRIANGLES)
}

/// Takes back the bottom levels a frame chose (`new`: the key, its size,
/// whether the still set holds it) and had no time to make, those after
/// the `made` first: the still set's return to `queue` in their order (the
/// next one last), the others are chosen again by their moving faces.
/// Returns their triangles, and how many of them are moving faces' alone.
fn unmade<S>(new: &mut Vec<(Key, S, bool)>, made: usize, queue: &mut Vec<Key>, index: &mut HashMap<Key, usize>) -> (u64, usize) {
    let (mut triangles, mut moving) = (0, 0);
    for (key, _, held) in new.drain(made..).rev() {
        triangles += key.index_count as u64 / 3;
        index.remove(&key);
        if held {
            queue.push(key);
        } else {
            moving += 1;
        }
    }
    (triangles, moving)
}
/// Posed vertices of the merged cells whose levels are made in one frame,
/// the nearest first (the nearest is always made): the others keep the
/// level of an earlier set, or wait for theirs.
const MERGED_VERTICES: u64 = 3 << 17;
/// The bit of a top level's custom index that says a merged cell's level:
/// the rest is where its table starts ([`cell_table`], rt_common.wgsl `hit_of`).
const MERGED: u32 = 1 << 23;
/// Frames a bottom level no face used is kept.
const KEEP_FRAMES: u64 = 600;
/// Frames between two sweeps of the bottom levels no face uses.
const SWEEP_EVERY: u64 = 64;
/// Frames a bottom level nothing uses any more is held before its release:
/// the top levels and the frames still drawn let go of it first, so that
/// it ends when the frame has the time ([`RELEASE_TIME`]) and not all at
/// once inside a build or a submit.
const RETIRE_FRAMES: u64 = 8;
/// The time a frame gives to releasing retired bottom levels: one costs
/// the render thread 1 to 4 microseconds, and a region left behind has
/// thousands (their sweep took a frame 3 ms).
const RELEASE_TIME: std::time::Duration = std::time::Duration::from_micros(250);
/// Frames between two builds of the scene's top level while bottom levels
/// of its faces still wait for the frames' budget: it is built once more
/// when the last one is, and now and then on the way so that a region
/// that loads shows its shadows as they come.
const CATCH_UP: u64 = 8;
/// How far a ray goes (m).
const REACH: f32 = 512.0;

const RESULT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The most posed vertices (one per index) a frame traces: the faces past
/// it are left out.
const POSED_VERTICES: u64 = 1 << 22;
/// Bytes of a posed vertex (a vec4 of f32).
const POSED_BYTES: u64 = 16;

/// A posed face of the pose pass (must match `Face` in rt_pose.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PoseFace {
    record: u32,
    base_vertex: u32,
    first_index: u32,
    index_count: u32,
    out_offset: u32,
    _pad: [u32; 3],
}

/// The pose pass's constants (must match `Params` in rt_pose.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PoseParams {
    counts: [u32; 4],
}

/// The pose pass and what it writes: the posed vertices of the skeletons
/// built again this frame, their bottom levels, and a top level of them all.
struct Poser {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    faces: Option<wgpu::Buffer>,
    vertices: Option<wgpu::Buffer>,
    /// The records of the top level's skeletons' faces, in its order
    /// (rt_common.wgsl `posed_records`).
    records: wgpu::Buffer,
    record_list: Vec<u32>,
    /// Each skeleton's bottom level, by its palette's first entry.
    levels: HashMap<u32, Level>,
    /// A top level of its own: built again on the frames a skeleton was,
    /// while the scene's stands as long as nothing in it moves.
    tlas: Option<wgpu::Tlas>,
    /// The skeletons the top level holds, in its order, and the moving
    /// placed faces after them.
    placed: Vec<u32>,
    movers: usize,
    /// The slots of the top level its last build filled.
    filled: usize,
    /// Where each skeleton built this frame starts in the posed vertices,
    /// and each merged cell posed this frame after them.
    offsets: Vec<(usize, u32)>,
    baked: Vec<u32>,
    table: Vec<PoseFace>,
    frame: u64,
}

/// A skeleton's bottom level: a geometry per face, so a hit's geometry index
/// is its face (`records`).
struct Level {
    blas: wgpu::Blas,
    /// The hash of the faces it was made for.
    faces: u64,
    records: Vec<u32>,
    sizes: Vec<wgpu::BlasTriangleGeometrySizeDescriptor>,
    used: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    base_vertex: i32,
    first_index: u32,
    index_count: u32,
    opaque: bool,
}

impl Key {
    fn of(i: &Instance) -> Self {
        Self {
            base_vertex: i.base_vertex,
            first_index: i.first_index,
            index_count: i.index_count,
            opaque: i.opaque,
        }
    }
}

struct Bottom {
    blas: wgpu::Blas,
    /// The last frame a moving face used it.
    used: u64,
    /// A face of the still set uses it.
    held: bool,
}

/// The render thread's time in [`RtShadows::encode`] since the last log.
#[derive(Default)]
struct Cpu {
    frames: u32,
    us: u64,
    most: u64,
    /// The frames a still set came, and their time.
    sets: u32,
    set_us: u64,
    slowest: Slowest,
}

/// The slowest frame of [`Cpu`]: its time by part (µs: the set and the
/// cells' posing, the bottom levels, the scene's level and the builds, the
/// moving and the posed faces, the trace), and what it made.
#[derive(Default, Debug, Clone, Copy, PartialEq)]
struct Slowest {
    parts: [u64; 5],
    set: bool,
    levels: usize,
    cells: usize,
    skeletons: usize,
    moving: usize,
}

/// A merged cell's bottom level, in world space ([`cell_geometries`]).
struct Merged {
    blas: wgpu::Blas,
    /// The version of the cell it was made for (`Cell::version`).
    version: u64,
    /// Its faces by their triangles ([`cell_table`]).
    table: Vec<u32>,
    /// A cell of the still set uses it; else the frame the last one did.
    held: bool,
    used: u64,
}

/// A moving placed face in the top level built each frame.
struct Mover<'a> {
    blas: &'a wgpu::Blas,
    transform: [f32; 12],
    record: u32,
}

/// The mask the shading reads in place of the contact shadows', and the
/// traced reflections SSR's composite reads.
pub(crate) struct Targets {
    size: (u32, u32),
    /// Which targets these are: a pass's history is the one it wrote to
    /// them, and new ones of the same size hold none (a photo's tiles are
    /// made one after the other, at one size).
    serial: u64,
    pub result: wgpu::TextureView,
    pub reflect: Option<wgpu::TextureView>,
    pub gi: Option<GiTargets>,
    pub lights: Option<LightTargets>,
}

/// The traced point lights' choices and share (two of each: one read while
/// the other is written), and the sun's mask before the share joins it.
pub(crate) struct LightTargets {
    choice: [wgpu::TextureView; 2],
    share: [wgpu::TextureView; 2],
    /// The share's denoiser guide, beside it (rt_denoise.wgsl).
    guide: [wgpu::TextureView; 2],
    /// The denoiser's two images, one read while the other is written.
    ping: [wgpu::TextureView; 2],
    /// Each pixel's sample, from the trace to the reuse (rt_lights.wgsl `mid`).
    mid: wgpu::TextureView,
    sun: wgpu::TextureView,
    /// The share by colour (rt_lights.wgsl's tint), as `share`, and its two
    /// images in the denoiser: one pixel each unless `tinted`.
    tint: [wgpu::TextureView; 2],
    tint_ping: [wgpu::TextureView; 2],
    /// The point lights are traced: the tint is kept.
    tinted: bool,
}

const CHOICE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const MID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;
const SHARE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The lights pass's constants (must match `Params` in rt_lights.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightsU {
    world_from_clip: [[f32; 4]; 4],
    prev_clip_from_world: [[f32; 4]; 4],
    camera: [f32; 4],
    prev_camera: [f32; 4],
    screen: [f32; 4],
    mode: [f32; 4],
    changed: [[f32; 4]; crate::rt_light_changes::MAX_CHANGES],
    changes: [f32; 4],
    sets: [f32; 4],
    columns: [[u32; 4]; crate::light_tiles::TILES / 2],
    rows: [[u32; 4]; crate::light_tiles::TILES / 2],
    slices: [[u32; 4]; crate::light_tiles::SLICES / 2],
}

/// A light of the frame's list (must match `Light` in rt_lights.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq)]
struct TracedLightU {
    pos_radius: [f32; 4],
    color_falloff: [f32; 4],
    source: [f32; 4],
}

/// The most lights of the frame's list (the frame uniform's).
const MAX_TRACED_LIGHTS: usize = 64;
/// The frames the lamps' share averages at most (rt_lights.wgsl `HISTORY`).
const LIGHTS_HISTORY: f32 = 12.0;

/// The traced point lights' passes (shaders/rt_lights.wgsl), which average
/// and filter the sun's soft shadow too: they run with the traced shadows,
/// with no light listed unless `points`.
struct Lights {
    /// The point lights' shadows are traced (`AURORA_RT_LIGHTS`).
    points: bool,
    trace_layout: wgpu::BindGroupLayout,
    reuse_layout: wgpu::BindGroupLayout,
    filter_layout: wgpu::BindGroupLayout,
    trace: wgpu::ComputePipeline,
    reuse: wgpu::ComputePipeline,
    filter: wgpu::ComputePipeline,
    buffer: wgpu::Buffer,
    denoise: Denoise,
    /// Last frame's lights: the same list keeps last frame's choices.
    listed: Vec<TracedLightU>,
    /// Where each light was last frame, by what it shines from
    /// (`PointLight::source_id`): how far it moved shortens its shadows'
    /// history.
    places: HashMap<u32, glam::Vec3>,
    /// The lamps that came, went or changed: the history lets go there.
    changes: crate::rt_light_changes::LightChanges,
    timer: Option<Timer>,
    last: usize,
    /// The targets the history was written to (`Targets::serial`).
    history_for: Option<u64>,
    prev_clip_from_world: glam::Mat4,
    prev_camera: glam::Vec3,
    frame: u32,
}

/// The traced indirect light's history, at half resolution (two of each:
/// one read while the other is written), and its full resolution result.
pub(crate) struct GiTargets {
    half: (u32, u32),
    color: [wgpu::TextureView; 2],
    /// The history's denoiser guide, beside it (rt_denoise.wgsl).
    guide: [wgpu::TextureView; 2],
    /// The denoiser's two images at half resolution.
    ping: [wgpu::TextureView; 2],
    /// The denoiser's first pass, kept: next frame's history takes its
    /// light from it (rt_gi.wgsl `prev_color`).
    fed: wgpu::TextureView,
    /// The pixels' picks (rt_gi.wgsl `Pick`), their hits and their light:
    /// last frame's and this one's, in turn as the history.
    pick_hit: [wgpu::TextureView; 2],
    pick_light: [wgpu::TextureView; 2],
    pub result: wgpu::TextureView,
}

const GI_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// A pick's hit (rt_gi.wgsl `Pick`): a place in the world, and a whole
/// number (`pack_tail`), which 16 bits a channel would not hold.
const GI_HIT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
/// A history's denoiser guide: the normal, the distance from the eye, the
/// luminance's second moment (rt_history.wgsl `pack_normal`).
const GUIDE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The denoiser's images: rgb the value, a its variance.
const DENOISE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The denoiser's à-trous passes, their step doubling from one pixel.
const ATROUS_PASSES: usize = 4;
/// The variance under which the denoiser leaves a pixel alone. The share
/// multiplies the local light: a deviation under half a percent of it does
/// not show. The indirect light is added to dark places as they are: only
/// what is no noise at all is left.
const SHARE_NOISE_FLOOR: f32 = 2.5e-5;
const GI_NOISE_FLOOR: f32 = 1e-7;

/// A denoiser pass's constants (must match `Params` in rt_denoise.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DenoiseU {
    size: [f32; 4],
    mode: [f32; 4],
}

/// The denoiser of a traced pass's history (shaders/rt_denoise.wgsl):
/// SVGF's variance, then its à-trous wavelet.
struct Denoise {
    layout: wgpu::BindGroupLayout,
    variance: wgpu::ComputePipeline,
    atrous: wgpu::ComputePipeline,
    /// The variance pass's constants, then each à-trous pass's.
    params: Vec<wgpu::Buffer>,
    /// Which 8×8 tiles have any variance (rt_denoise.wgsl `tiles`).
    tiles: Option<wgpu::Buffer>,
    /// The size they were made for, and whether for a tint.
    params_for: Option<(u32, u32, bool)>,
    /// The variance under which a pixel is left as it is.
    floor: f32,
}
/// How far an indirect ray goes (m): the light bounced from further off
/// is faint, and shorter rays are quicker.
const GI_REACH: f32 = 64.0;
/// The frames the indirect light's history averages: about half a second
/// of them, 32 at 60 frames a second. A fixed count is a fourth of that
/// time at 240 frames a second: as much noise in each frame, crawling four
/// times as fast, which shows as grain on the still faces.
const GI_HISTORY_SECONDS: f32 = 32.0 / 60.0;
const GI_HISTORY_FRAMES: (f32, f32) = (32.0, 128.0);

fn gi_history_frames(frame_seconds: f32) -> f32 {
    (GI_HISTORY_SECONDS / frame_seconds.max(1e-4)).clamp(GI_HISTORY_FRAMES.0, GI_HISTORY_FRAMES.1)
}

/// The GI pass's constants (must match `Params` in rt_gi.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GiU {
    clip_from_world: [[f32; 4]; 4],
    world_from_clip: [[f32; 4]; 4],
    prev_clip_from_world: [[f32; 4]; 4],
    camera: [f32; 4],
    prev_camera: [f32; 4],
    lit: [[f32; 4]; 5],
    screen: [f32; 4],
    half: [f32; 4],
    mode: [f32; 4],
    changed: [[f32; 4]; crate::rt_light_changes::MAX_CHANGES],
    changes: [f32; 4],
    lamps: [f32; 4],
    scene_clip_from_world: [[f32; 4]; 4],
    scene_world_from_clip: [[f32; 4]; 4],
    scene: [f32; 4],
}

/// The traced indirect light's passes (shaders/rt_gi.wgsl).
struct Gi {
    trace_layout: wgpu::BindGroupLayout,
    reuse_layout: wgpu::BindGroupLayout,
    filter_layout: wgpu::BindGroupLayout,
    trace: wgpu::ComputePipeline,
    reuse: wgpu::ComputePipeline,
    filter: wgpu::ComputePipeline,
    buffer: wgpu::Buffer,
    denoise: Denoise,
    timer: Option<Timer>,
    /// The history written last frame (`GiTargets::color` and `guide`).
    last: usize,
    /// The targets the history was written to (`Targets::serial`), None
    /// before the first frame.
    history_for: Option<u64>,
    prev_clip_from_world: glam::Mat4,
    prev_camera: glam::Vec3,
    frame: u32,
    /// The view's last lit frame, kept through a photo and until the
    /// view's next frame (`hold_screen`).
    screen: Option<HeldScreen>,
    /// When the last frame was traced, and the frames' length smoothed
    /// (`gi_history_frames`).
    traced_at: Option<std::time::Instant>,
    frame_seconds: f32,
    /// Where the light changed, for the history to let go there.
    changes: crate::rt_light_changes::LightChanges,
}

const REFLECT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The reflection pass's constants (must match `Params` in rt_reflect.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ReflectU {
    clip_from_world: [[f32; 4]; 4],
    world_from_clip: [[f32; 4]; 4],
    camera: [f32; 4],
    lit: [[f32; 4]; 5],
    screen: [f32; 4],
    mode: [f32; 4],
}

/// The frame's view and light, as the reflections and the GI shade what
/// the view does not see.
pub(crate) struct TraceLight {
    /// The main view's lights by screen tile and depth, of the frame's
    /// list in its order (the whole list where the frame has none).
    pub light_sets: crate::light_tiles::LightSets,
    pub clip_from_world: glam::Mat4,
    pub camera: glam::Vec3,
    pub sun_dir: glam::Vec3,
    pub sun_up: bool,
    pub sun_color: [f32; 4],
    pub sky_zenith: [f32; 4],
    pub sky_horizon: [f32; 4],
    pub ground_color: [f32; 4],
}

impl TraceLight {
    /// rt_light.wgsl's `HitLight`.
    fn lit(&self) -> [[f32; 4]; 5] {
        [
            self.sun_dir
                .normalize_or(glam::Vec3::Z)
                .extend(if self.sun_up { 1.0 } else { 0.0 })
                .to_array(),
            self.sun_color,
            self.sky_zenith,
            self.sky_horizon,
            self.ground_color,
        ]
    }

    fn sky(&self) -> crate::rt_light_changes::SkyLight {
        let rgb = |c: [f32; 4]| glam::Vec3::new(c[0], c[1], c[2]);
        crate::rt_light_changes::SkyLight {
            sun: self.sun_up.then(|| self.sun_dir.normalize_or(glam::Vec3::Z)),
            sun_color: rgb(self.sun_color),
            zenith: rgb(self.sky_zenith),
            horizon: rgb(self.sky_horizon),
            ground: rgb(self.ground_color),
        }
    }
}

/// The traced reflections' pass (shaders/rt_reflect.wgsl).
struct Reflect {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    buffer: wgpu::Buffer,
    timer: Option<Timer>,
}

/// What the last frame traced (AURORA_PROFILE).
#[derive(Clone, Copy, Debug, Default)]
pub struct RtStats {
    /// Placed faces traced, and those of them in the top level built each
    /// frame.
    pub instances: usize,
    pub moving: usize,
    /// The still ones merged by cell, their cells' levels, those made last
    /// frame and those that wait.
    pub merged: usize,
    pub cells: usize,
    pub cells_built: usize,
    pub cells_waiting: usize,
    pub bottoms: usize,
    pub built: usize,
    pub built_triangles: u64,
    pub waiting: usize,
    pub posed_faces: usize,
    pub posed_vertices: u64,
    pub skeletons: usize,
    pub skeletons_built: usize,
}

pub(crate) struct RtShadows {
    layout: wgpu::BindGroupLayout,
    /// The scene's tables as the ray-traced passes read them (group 1 of
    /// rt_common.wgsl).
    scene_layout: wgpu::BindGroupLayout,
    /// A buffer past the device's binding size was logged.
    warned: bool,
    pipeline: wgpu::ComputePipeline,
    buffer: wgpu::Buffer,
    cull: Cull,
    bottoms: HashMap<Key, Bottom>,
    /// The bottom levels nothing uses any more, with the frame each was
    /// retired on, the oldest first ([`RETIRE_FRAMES`]).
    retired: VecDeque<(u64, wgpu::Blas)>,
    /// The arena buffers the bottom levels were built from: a new one
    /// (the arena grew) builds them all again.
    arena: Option<(wgpu::Buffer, wgpu::Buffer)>,
    tlas: Option<wgpu::Tlas>,
    /// The still faces the top level is made of (`Faces::still`), and
    /// their set's number: it stands, with no build, until another comes.
    still: Arc<Vec<Instance>>,
    set: Option<u64>,
    /// The merged cells of the set (`Faces::cells`), their levels by key,
    /// those of `cells` whose level is to make (the next one last), and
    /// the records of the levels the top level places, one after the
    /// other (rt_common.wgsl `merged_records`).
    cells: Arc<Vec<Cell>>,
    merged: HashMap<u64, Merged>,
    cell_queue: Vec<usize>,
    merged_records: wgpu::Buffer,
    merged_list: Vec<u32>,
    merged_faces: usize,
    /// The geometries of `still` with no bottom level yet, the next one
    /// last.
    queue: Vec<Key>,
    /// The top level lacks faces of `still` whose bottom levels waited.
    stale: bool,
    /// The frame the top level was last built, the faces it then placed,
    /// and its builds since the last log.
    built_at: u64,
    scene_instances: usize,
    scene_builds: u32,
    /// The still faces by size, for the log: how many have up to 64, 256,
    /// 1024 and 4096 triangles or more, and their triangles.
    still_sizes: [(u32, u64); 5],
    /// `Faces::why`, `Faces::spent` and the set's number at the last log,
    /// and since then: the frames encoded, the time it took on the render
    /// thread (µs) in all, at most, and on the frames a set came.
    why: [u64; 10],
    spent: [u64; 3],
    logged_set: u64,
    cpu: Cpu,
    frame: u64,
    /// The frame's point lights (rt_light.wgsl `Light`, in the frame
    /// uniform's order) and how many: written by [`Self::encode_lights`],
    /// for its pass and for the hits the reflections and the indirect light
    /// shade.
    lamps: wgpu::Buffer,
    lamps_count: u32,
    /// Whether one of them moved since the last frame.
    lamps_moved: bool,
    /// The sun's pass's constants as `write` left them, for a photo's later
    /// rounds (`retrace`).
    sun: ParamsU,
    /// A photo's lamps as its first round listed them, for the others.
    lamps_still: Vec<TracedLightU>,
    pub stats: RtStats,
    timer: Option<Timer>,
    poser: Poser,
    reflect: Option<Reflect>,
    gi: Option<Gi>,
    lights: Option<Lights>,
    /// The posed top level holds a skeleton this frame.
    posed_live: bool,
}

/// Readbacks in flight for the pass's timestamps.
const RING: usize = 4;

/// The GPU time of a pass's steps: a timestamp at each step's start and one
/// at the last one's end, resolved and read back a few frames later; their
/// averages come out every [`Timer::REPORT`] frames. The shadows have four
/// (before the pose, after it, before the trace, after it), the reflections
/// two.
struct Timer {
    queries: wgpu::QuerySet,
    /// Timestamps a frame writes.
    count: u32,
    resolve: wgpu::Buffer,
    ring: Vec<(wgpu::Buffer, std::sync::Arc<std::sync::atomic::AtomicU8>)>,
    next: usize,
    period_ns: f32,
    sums: Vec<f64>,
    samples: u32,
    frames: u32,
}

/// `Timer::ring` states.
const FREE: u8 = 0;
const WRITTEN: u8 = 1;
const MAPPING: u8 = 2;
const MAPPED: u8 = 3;

impl Timer {
    const REPORT: u32 = 240;

    fn new(device: &wgpu::Device, queue: &wgpu::Queue, count: u32) -> Option<Self> {
        // the timings are the profile's: nothing is measured, read back or
        // logged in a run without AURORA_PROFILE
        if !crate::renderer::profile() || !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("rt timer"),
            ty: wgpu::QueryType::Timestamp,
            count,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt timestamps"),
            size: count as u64 * 8,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let ring = (0..RING)
            .map(|_| {
                let b = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("rt timestamps readback"),
                    size: crate::readback::readback_size(count as u64 * 8),
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                (b, std::sync::Arc::new(std::sync::atomic::AtomicU8::new(FREE)))
            })
            .collect();
        Some(Self {
            queries,
            count,
            resolve,
            ring,
            next: 0,
            period_ns: queue.get_timestamp_period(),
            sums: vec![0.0; count as usize - 1],
            samples: 0,
            frames: 0,
        })
    }

    /// Before this frame's work: the readbacks the last frames wrote are
    /// mapped, those mapped are read. Every [`Timer::REPORT`] frames, the
    /// mean time of each step (ms) and the frames they cover.
    fn harvest(&mut self) -> Option<(Vec<f64>, u32)> {
        use std::sync::atomic::Ordering;
        let n = self.count as usize;
        for (buffer, state) in &self.ring {
            match state.load(Ordering::Acquire) {
                WRITTEN => {
                    state.store(MAPPING, Ordering::Release);
                    let state = state.clone();
                    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                        state.store(if r.is_ok() { MAPPED } else { FREE }, Ordering::Release);
                    });
                }
                MAPPED => {
                    if let Ok(data) = buffer.slice(..).get_mapped_range() {
                        let mut t = vec![0u64; n];
                        bytemuck::cast_slice_mut::<u64, u8>(&mut t).copy_from_slice(&data[..n * 8]);
                        if t.windows(2).all(|w| w[1] >= w[0]) {
                            let ms = |a: u64, b: u64| (b - a) as f64 * self.period_ns as f64 / 1e6;
                            for (k, sum) in self.sums.iter_mut().enumerate() {
                                *sum += ms(t[k], t[k + 1]);
                            }
                            self.samples += 1;
                        }
                    }
                    buffer.unmap();
                    state.store(FREE, Ordering::Release);
                }
                _ => {}
            }
        }
        self.frames += 1;
        if self.frames < Self::REPORT || self.samples == 0 {
            return None;
        }
        let k = self.samples as f64;
        let means = self.sums.iter().map(|s| s / k).collect();
        let samples = self.samples;
        self.sums.iter_mut().for_each(|s| *s = 0.0);
        self.samples = 0;
        self.frames = 0;
        Some((means, samples))
    }

    /// After this frame's work: its timestamps to a free readback.
    fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        use std::sync::atomic::Ordering;
        encoder.resolve_query_set(&self.queries, 0..self.count, &self.resolve, 0);
        let (buffer, state) = &self.ring[self.next];
        if state.load(Ordering::Acquire) == FREE {
            encoder.copy_buffer_to_buffer(&self.resolve, 0, buffer, 0, self.count as u64 * 8);
            state.store(WRITTEN, Ordering::Release);
            self.next = (self.next + 1) % RING;
        }
    }
}

impl RtShadows {
    /// The ray tracer of a device opened with ray queries, tracing
    /// `options` ([`traced`], with the sun's shadows).
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue, textures: &wgpu::BindGroupLayout, options: crate::RayTracing) -> Self {
        let cull = Override::from_env().cull;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rt shadows"),
            source: wgpu::ShaderSource::Wgsl(source(&[RT_VIEW, include_str!("shaders/rt_shadows.wgsl")]).into()),
        });
        let scene_entries: Vec<wgpu::BindGroupLayoutEntry> = (0..9).map(|b| storage_entry(b, true)).collect();
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt scene"),
            entries: &scene_entries,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt shadows"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<ParamsU>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::AccelerationStructure { vertex_return: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: RESULT_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::AccelerationStructure { vertex_return: false },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rt shadows"),
            bind_group_layouts: &[Some(&layout), Some(&scene_layout), Some(textures)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: Some("rt shadows"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_trace"),
            compilation_options: Default::default(),
            cache: None,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt shadows"),
            size: std::mem::size_of::<ParamsU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        log::info!("ray tracing on: {options:?} ({cull:?} faces culled)");
        Self {
            layout,
            pipeline,
            buffer,
            cull,
            bottoms: HashMap::new(),
            retired: VecDeque::new(),
            arena: None,
            tlas: None,
            still: Arc::default(),
            set: None,
            cells: Arc::default(),
            merged: HashMap::new(),
            cell_queue: Vec::new(),
            merged_records: records_buffer(device, "rt shadows merged records", 4096),
            merged_list: Vec::new(),
            merged_faces: 0,
            queue: Vec::new(),
            stale: true,
            built_at: 0,
            scene_instances: 0,
            scene_builds: 0,
            still_sizes: [(0, 0); 5],
            why: [0; 10],
            spent: [0; 3],
            logged_set: 0,
            cpu: Cpu::default(),
            frame: 0,
            lamps: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rt lights list"),
                size: (std::mem::size_of::<TracedLightU>() * MAX_TRACED_LIGHTS) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            lamps_count: 0,
            lamps_moved: false,
            sun: ParamsU::zeroed(),
            lamps_still: Vec::new(),
            stats: RtStats::default(),
            timer: Timer::new(device, queue, 4),
            poser: Poser::new(device, &scene_layout),
            reflect: options.reflections.then(|| Reflect::new(device, queue, &scene_layout, textures)),
            gi: options.indirect.then(|| Gi::new(device, queue, &scene_layout, textures)),
            lights: Some(Lights::new(device, queue, &scene_layout, textures, options.lights)),
            posed_live: false,
            scene_layout,
            warned: false,
        }
    }
}

impl Poser {
    fn new(device: &wgpu::Device, scene_layout: &wgpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rt shadows pose"),
            source: wgpu::ShaderSource::Wgsl(source(&[include_str!("shaders/rt_pose.wgsl")]).into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt shadows pose"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<PoseParams>() as u64),
                    },
                    count: None,
                },
                storage_entry(1, true),
                storage_entry(2, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rt shadows pose"),
            bind_group_layouts: &[Some(&layout), Some(scene_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: Some("rt shadows pose"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_pose"),
            compilation_options: Default::default(),
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt shadows pose"),
            size: std::mem::size_of::<PoseParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            pipeline,
            params,
            faces: None,
            vertices: None,
            records: posed_records_buffer(device, 4096),
            record_list: Vec::new(),
            levels: HashMap::new(),
            tlas: None,
            placed: Vec::new(),
            movers: 0,
            filled: 0,
            offsets: Vec::new(),
            baked: Vec::new(),
            table: Vec::new(),
            frame: 0,
        }
    }

    /// Poses the faces of the skeletons built again this frame into the
    /// posed buffer, then those of `cells` in their order (`baked`: the
    /// ones that fit), up to [`POSED_VERTICES`] in all: the skeletons'
    /// faces and vertices posed.
    fn pose(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene_group: &wgpu::BindGroup,
        faces: &Faces,
        cells: &[&Cell],
    ) -> (usize, u32) {
        self.frame += 1;
        self.table.clear();
        self.offsets.clear();
        self.baked.clear();
        let mut total = 0u64;
        for (g, group) in faces.groups.iter().enumerate() {
            if !group.rebuild || total + group.vertices as u64 > POSED_VERTICES {
                continue;
            }
            let run = faces
                .posed
                .get(group.first as usize..(group.first + group.count) as usize)
                .unwrap_or(&[]);
            self.offsets.push((g, total as u32));
            for f in run {
                self.table.push(PoseFace {
                    record: f.record,
                    base_vertex: f.base_vertex,
                    first_index: f.first_index,
                    index_count: f.index_count,
                    out_offset: total as u32,
                    _pad: [0; 3],
                });
                total += f.index_count as u64;
            }
        }
        let skeletons = (self.table.len(), total as u32);
        for cell in cells {
            if total + cell.vertices as u64 > POSED_VERTICES {
                break;
            }
            self.baked.push(total as u32);
            for f in cell.faces.iter() {
                self.table.push(PoseFace {
                    record: f.record,
                    base_vertex: f.base_vertex,
                    first_index: f.first_index,
                    index_count: f.index_count,
                    out_offset: total as u32,
                    _pad: [0; 3],
                });
                total += f.index_count as u64;
            }
        }
        if total == 0 {
            return (0, 0);
        }
        let table_bytes = (self.table.len() * std::mem::size_of::<PoseFace>()) as u64;
        if self.faces.as_ref().is_none_or(|b| b.size() < table_bytes) {
            self.faces = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rt shadows posed faces"),
                size: table_bytes.next_power_of_two().max(4096),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let bytes = total.next_power_of_two().max(3 << 10) * POSED_BYTES;
        if self.vertices.as_ref().is_none_or(|b| b.size() < bytes) {
            self.vertices = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rt shadows posed vertices"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::BLAS_INPUT,
                mapped_at_creation: false,
            }));
        }
        let (Some(table), Some(posed)) = (self.faces.as_ref(), self.vertices.as_ref()) else {
            self.offsets.clear();
            self.baked.clear();
            return (0, 0);
        };
        queue.write_buffer(table, 0, bytemuck::cast_slice(&self.table));
        let workgroups = (total as u32).div_ceil(64);
        let x = workgroups.min(65_535);
        let y = workgroups.div_ceil(x);
        let params = PoseParams {
            counts: [self.table.len() as u32, total as u32, 0, x * 64],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt shadows pose"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: table.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: posed.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("rt shadows pose"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.set_bind_group(1, scene_group, &[]);
        pass.dispatch_workgroups(x, y, 1);
        skeletons
    }

    /// Builds the bottom levels of the skeletons posed this frame, and the
    /// top level (with its faces' records) of the skeletons and of
    /// `movers`, the placed faces that move, when a skeleton was built,
    /// the skeletons changed or a face moves: the levels built, and the
    /// skeletons the top level holds.
    fn build(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        faces: &Faces,
        movers: &[Mover<'_>],
    ) -> (usize, usize) {
        let frame = self.frame;
        // a level made for each skeleton built with faces its level was
        // not made for
        for &(g, _) in &self.offsets {
            let group = faces.groups[g];
            if self.levels.get(&group.key).is_some_and(|l| l.faces == group.faces) {
                continue;
            }
            let run = faces
                .posed
                .get(group.first as usize..(group.first + group.count) as usize)
                .unwrap_or(&[]);
            let sizes: Vec<_> = run.iter().map(|f| posed_size(f.index_count, f.opaque)).collect();
            let blas = device.create_blas(
                &wgpu::CreateBlasDescriptor {
                    label: Some("rt shadows skeleton"),
                    flags: wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD,
                    update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                },
                wgpu::BlasGeometrySizeDescriptors::Triangles {
                    descriptors: sizes.clone(),
                },
            );
            self.levels.insert(
                group.key,
                Level {
                    blas,
                    faces: group.faces,
                    records: run.iter().map(|f| f.record).collect(),
                    sizes,
                    used: frame,
                },
            );
        }
        for group in &faces.groups {
            if let Some(level) = self.levels.get_mut(&group.key) {
                level.used = frame;
            }
        }
        self.levels.retain(|_, l| l.used == frame);
        let placed: Vec<u32> = faces.groups.iter().map(|g| g.key).filter(|k| self.levels.contains_key(k)).collect();
        // nothing posed, the same skeletons, no face that moves (nor one
        // to take out): the top level stands
        if self.offsets.is_empty() && placed == self.placed && movers.is_empty() && self.movers == 0 {
            return (0, placed.len());
        }
        let wanted = (placed.len() + movers.len()).max(1).next_power_of_two().max(64);
        if self.tlas.as_ref().is_none_or(|t| !fits(t.get().len(), wanted)) {
            self.tlas = Some(device.create_tlas(&wgpu::CreateTlasDescriptor {
                label: Some("rt shadows skeletons"),
                max_instances: wanted as u32,
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            }));
            self.filled = 0;
        }
        // the faces' records, each skeleton's where its instance says,
        // then each moving face's own
        self.record_list.clear();
        let mut starts = Vec::with_capacity(placed.len());
        for key in &placed {
            starts.push(self.record_list.len() as u32);
            if let Some(level) = self.levels.get(key) {
                self.record_list.extend_from_slice(&level.records);
            }
        }
        let first_mover = self.record_list.len() as u32;
        self.record_list.extend(movers.iter().map(|m| m.record));
        let bytes = (self.record_list.len().max(1) * 4) as u64;
        if self.records.size() < bytes {
            self.records = posed_records_buffer(device, bytes.next_power_of_two());
        }
        queue.write_buffer(&self.records, 0, bytemuck::cast_slice(&self.record_list));
        let Some(tlas) = self.tlas.as_mut() else {
            return (0, 0);
        };
        {
            // this frame's slots, and those the last build filled past
            // them, which would keep its instances
            let filled = placed.len() + movers.len();
            let slots = filled.max(std::mem::replace(&mut self.filled, filled));
            let slots = tlas.get_mut_slice(0..slots).expect("the top level's own slots");
            for (n, slot) in slots.iter_mut().enumerate() {
                *slot = match n.checked_sub(placed.len()) {
                    None => self
                        .levels
                        .get(&placed[n])
                        .map(|l| wgpu::TlasInstance::new(&l.blas, IDENTITY, starts[n] & 0xff_ffff, 0xFF)),
                    Some(m) => movers
                        .get(m)
                        .map(|mover| wgpu::TlasInstance::new(mover.blas, mover.transform, (first_mover + m as u32) & 0xff_ffff, 0xFF)),
                };
            }
        }
        let entries: Vec<wgpu::BlasBuildEntry<'_>> = match self.vertices.as_ref() {
            Some(buffer) => self
                .offsets
                .iter()
                .filter_map(|&(g, offset)| {
                    let level = self.levels.get(&faces.groups[g].key)?;
                    let mut first = offset;
                    let geometries = level
                        .sizes
                        .iter()
                        .map(|size| {
                            let geometry = wgpu::BlasTriangleGeometry {
                                size,
                                vertex_buffer: buffer,
                                first_vertex: first,
                                vertex_stride: POSED_BYTES,
                                index_buffer: None,
                                first_index: None,
                                transform_buffer: None,
                                transform_buffer_offset: None,
                            };
                            first += size.vertex_count;
                            geometry
                        })
                        .collect();
                    Some(wgpu::BlasBuildEntry {
                        blas: &level.blas,
                        geometry: wgpu::BlasGeometries::TriangleGeometries(geometries),
                    })
                })
                .collect(),
            None => Vec::new(),
        };
        encoder.build_acceleration_structures(entries.iter(), std::iter::once(&*tlas));
        let built = entries.len();
        drop(entries);
        self.placed = placed;
        self.movers = movers.len();
        (built, self.placed.len())
    }

    /// The posed top level to trace this frame, if it holds a skeleton or
    /// a moving face.
    fn traced(&self) -> Option<&wgpu::Tlas> {
        self.tlas.as_ref().filter(|_| !self.placed.is_empty() || self.movers > 0)
    }
}

fn posed_records_buffer(device: &wgpu::Device, bytes: u64) -> wgpu::Buffer {
    records_buffer(device, "rt shadows posed records", bytes)
}

fn records_buffer(device: &wgpu::Device, label: &'static str, bytes: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// A ray-traced shader's source: the enables, rt_common.wgsl, then `parts`
/// in order (the shared rt_view.wgsl and rt_light.wgsl, then its own).
fn source(parts: &[&str]) -> String {
    let mut src = format!(
        "enable wgpu_ray_query;\nenable wgpu_binding_array;\n{}\n",
        include_str!("shaders/rt_common.wgsl")
    );
    for part in parts {
        src.push_str(part);
        src.push('\n');
    }
    src
}

const RT_VIEW: &str = include_str!("shaders/rt_view.wgsl");
const RT_LIGHT: &str = include_str!("shaders/rt_light.wgsl");
const RT_HISTORY: &str = include_str!("shaders/rt_history.wgsl");

/// A posed face's geometry: `vertices` unindexed, three a triangle; an
/// alpha-masked face is tested at its hits.
fn posed_size(vertices: u32, opaque: bool) -> wgpu::BlasTriangleGeometrySizeDescriptor {
    wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count: vertices,
        index_format: None,
        index_count: None,
        flags: geometry_flags(opaque),
    }
}

fn geometry_flags(opaque: bool) -> wgpu::AccelerationStructureGeometryFlags {
    if opaque {
        wgpu::AccelerationStructureGeometryFlags::OPAQUE
    } else {
        wgpu::AccelerationStructureGeometryFlags::empty()
    }
}

impl Targets {
    /// The mask and what the other traced effects of `options` write
    /// ([`traced`]), when the sun's shadows are traced.
    pub(crate) fn new(device: &wgpu::Device, width: u32, height: u32, options: crate::RayTracing) -> Option<Self> {
        if !options.shadows {
            return None;
        }
        let size = (width.max(1), height.max(1));
        let result = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("rt shadows"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: RESULT_FORMAT,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let reflect = options.reflections.then(|| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("rt reflections"),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: REFLECT_FORMAT,
                    usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        });
        let gi = options.indirect.then(|| {
            let half = (size.0.div_ceil(2), size.1.div_ceil(2));
            let usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
            let view = |label: &'static str, (width, height): (u32, u32), format: wgpu::TextureFormat| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage,
                        view_formats: &[],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default())
            };
            GiTargets {
                half,
                color: [view("rt gi history", half, GI_FORMAT), view("rt gi history", half, GI_FORMAT)],
                guide: [view("rt gi guide", half, GUIDE_FORMAT), view("rt gi guide", half, GUIDE_FORMAT)],
                ping: [
                    view("rt gi denoise", half, DENOISE_FORMAT),
                    view("rt gi denoise", half, DENOISE_FORMAT),
                ],
                fed: view("rt gi denoise fed", half, DENOISE_FORMAT),
                pick_hit: [
                    view("rt gi pick hit", half, GI_HIT_FORMAT),
                    view("rt gi pick hit", half, GI_HIT_FORMAT),
                ],
                pick_light: [view("rt gi pick light", half, GI_FORMAT), view("rt gi pick light", half, GI_FORMAT)],
                result: view("rt gi", size, GI_FORMAT),
            }
        });
        let lights = Some({
            let usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
            let sized = |label: &'static str, format: wgpu::TextureFormat, (width, height): (u32, u32)| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage,
                        view_formats: &[],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default())
            };
            let view = |label: &'static str, format: wgpu::TextureFormat| sized(label, format, size);
            // (the passes bind a tint whether they keep one or not)
            let tint = |label: &'static str| sized(label, SHARE_FORMAT, if options.lights { size } else { (1, 1) });
            LightTargets {
                choice: [view("rt light choices", CHOICE_FORMAT), view("rt light choices", CHOICE_FORMAT)],
                share: [view("rt light share", SHARE_FORMAT), view("rt light share", SHARE_FORMAT)],
                guide: [view("rt light guide", GUIDE_FORMAT), view("rt light guide", GUIDE_FORMAT)],
                ping: [view("rt light denoise", DENOISE_FORMAT), view("rt light denoise", DENOISE_FORMAT)],
                mid: view("rt light samples", MID_FORMAT),
                sun: view("rt sun mask", RESULT_FORMAT),
                tint: [tint("rt light tint"), tint("rt light tint")],
                tint_ping: [tint("rt light tint denoise"), tint("rt light tint denoise")],
                tinted: options.lights,
            }
        });
        static MADE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        Some(Targets {
            size,
            serial: MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            result,
            reflect,
            gi,
            lights,
        })
    }
}

/// The rounds of a photo's traced passes ([`Round::Still`]). A photo is one
/// frame: it has no frame before it to average, where the view's light is
/// the mean of the lamps' 12 frames and of the indirect light's half second
/// (32 frames at 60 a second). Its passes run this many times over its
/// view instead, each on the history the round before left, so a photo
/// holds the light the view shows and not one frame's rays.
pub(crate) const STILL_ROUNDS: u32 = 32;

/// Which run of a frame's traced passes this is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Round {
    /// The view's frame: one run, on last frame's history.
    Live,
    /// A photo's (a tile's, a 360° face's): the first has no history,
    /// whatever the passes kept, and each of the others has the one before
    /// it, of the same view (0 to [`STILL_ROUNDS`] - 1).
    Still(u32),
}

impl Round {
    /// The run that does what a frame does once: the scene's levels, the
    /// frame's lights, the timers.
    pub(crate) fn first(self) -> bool {
        !matches!(self, Round::Still(n) if n > 0)
    }

    /// Whether the pass has a history to read: the view's when the pass
    /// wrote it to these targets (`kept`), a photo's from its second round.
    fn history(self, kept: bool) -> bool {
        match self {
            Round::Live => kept,
            Round::Still(n) => n > 0,
        }
    }

    /// Nothing moves between two rounds of a photo: the prepass's motion,
    /// which is since the view's last frame, is not read.
    fn still(self) -> bool {
        matches!(self, Round::Still(_))
    }
}

/// The GPU time a photo of a frame's pixels is planned by (ms): the
/// frame's (`frame_ms`) and the `more` rounds its traced passes run that
/// the frame did not, each of `traced_ms`.
pub(crate) fn still_gpu_ms(frame_ms: f32, traced_ms: f32, more: u32) -> f32 {
    frame_ms + more as f32 * traced_ms
}

/// Writes a pass's constants. The queue's writes all land before the
/// frame's commands, so a later round of a photo, which changes them
/// between two runs of one encoder, goes through a copy the encoder orders.
fn put(device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, buffer: &wgpu::Buffer, bytes: &[u8], round: Round) {
    if round.first() {
        queue.write_buffer(buffer, 0, bytes);
        return;
    }
    let staged = wgpu::util::DeviceExt::create_buffer_init(
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("rt still round"),
            contents: bytes,
            usage: wgpu::BufferUsages::COPY_SRC,
        },
    );
    encoder.copy_buffer_to_buffer(&staged, 0, buffer, 0, bytes.len() as u64);
}

/// The view's last lit frame, kept through a photo for the hits of its
/// indirect light and of the view's next frame (`RtShadows::hold_screen`).
struct HeldScreen {
    view: wgpu::TextureView,
    /// The depth it was drawn with: what it shows of a hit is told by it,
    /// the same in every tile of a photo.
    depth: wgpu::TextureView,
    clip_from_world: glam::Mat4,
    size: (u32, u32),
}

impl RtShadows {
    /// The constants of this frame's pass: the projection the depth prepass
    /// was drawn with, the eye, the light (w: whether it is up), and whether
    /// the contact mask is read.
    pub(crate) fn write(
        &mut self,
        queue: &wgpu::Queue,
        t: &Targets,
        clip_from_world: glam::Mat4,
        camera: glam::Vec3,
        light: glam::Vec3,
        light_up: bool,
        contact: bool,
    ) {
        let (w, h) = (t.size.0 as f32, t.size.1 as f32);
        let u = ParamsU {
            world_from_clip: clip_from_world.inverse().to_cols_array_2d(),
            camera: camera.extend(1.0).to_array(),
            light: light
                .normalize_or(glam::Vec3::Z)
                .extend(if light_up { 1.0 } else { 0.0 })
                .to_array(),
            screen: [w, h, 1.0 / w, 1.0 / h],
            mode: [self.cull.ray_flags() as f32, if contact { 1.0 } else { 0.0 }, REACH, 0.0],
            // soft where the lights' passes average and filter it
            soft: match (&self.lights, &t.lights) {
                (Some(l), Some(_)) => [SUN_DISC_TAN, (l.frame % (1 << 20)) as f32, 0.0, 0.0],
                _ => [0.0; 4],
            },
        };
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&u));
        self.sun = u;
    }

    /// A later round of a photo: the sun's mask traced again, its ray to
    /// another point of the disc (the frame number the lights' pass counts).
    pub(crate) fn retrace(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        depth: &wgpu::TextureView,
        contact: &wgpu::TextureView,
        scene: &crate::packet::SceneGpu,
        round: Round,
    ) {
        if let Some(l) = &self.lights {
            self.sun.soft[1] = (l.frame % (1 << 20)) as f32;
        }
        put(device, queue, encoder, &self.buffer, bytemuck::bytes_of(&self.sun), round);
        self.trace(device, encoder, t, depth, contact, scene, round);
    }

    /// Poses the posed faces, builds what is missing of the bottom levels
    /// (within a budget), the scene's top level when the still faces' set
    /// changed, and the top level of the skeletons and of the moving faces,
    /// then traces the mask from `depth` (and `contact`, the contact
    /// shadows' mask, or a black texture).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        depth: &wgpu::TextureView,
        contact: &wgpu::TextureView,
        scene: &crate::packet::SceneGpu,
        faces: &Faces,
    ) {
        let started = std::time::Instant::now();
        let (vertices, indices) = (&scene.vertices, &scene.indices);
        self.frame += 1;
        let frame = self.frame;
        if let Some(timer) = &mut self.timer {
            if let Some((ms, frames)) = timer.harvest() {
                let st = &self.stats;
                let main: [u64; 3] =
                    std::array::from_fn(|i| faces.spent[i].saturating_sub(std::mem::replace(&mut self.spent[i], faces.spent[i])));
                let sets = faces.set.saturating_sub(std::mem::replace(&mut self.logged_set, faces.set));
                let cpu = std::mem::take(&mut self.cpu);
                log::info!(
                    "rt shadows: pose {:.3} ms, build {:.3} ms, trace {:.3} ms (GPU, mean of {frames} frames); {} placed faces ({} moving, {} merged into {} cells: {} made last frame, {} waiting; the still ones' level built {} times, for [at once, gone, moved, other geometry, to come in, to go out; gone: slot freed, no geometry, no longer traced, now posed]: {:?}; still faces of up to 64 / 256 / 1024 / 4096 / more triangles: {:?}), {} bottom levels ({} built last frame, {} triangles), {} waiting; {} skeletons ({} built last frame: {} faces, {} vertices); CPU: main thread {:.2} ms a frame ({} faces looked at), {:.2} ms a set ({} sets); render thread {:.2} ms a frame, {:.2} at most, {:.2} on a set's frame; the slowest: {:?}",
                    ms[0],
                    ms[1],
                    ms[2],
                    st.instances,
                    st.moving,
                    st.merged,
                    st.cells,
                    st.cells_built,
                    st.cells_waiting,
                    std::mem::take(&mut self.scene_builds),
                    std::array::from_fn::<u64, 10, _>(|i| faces.why[i].saturating_sub(std::mem::replace(&mut self.why[i], faces.why[i]))),
                    self.still_sizes,
                    st.bottoms,
                    st.built,
                    st.built_triangles,
                    st.waiting,
                    st.skeletons,
                    st.skeletons_built,
                    st.posed_faces,
                    st.posed_vertices,
                    main[0] as f64 / 1000.0 / frames.max(1) as f64,
                    main[2] / frames.max(1) as u64,
                    main[1] as f64 / 1000.0 / sets.max(1) as f64,
                    sets,
                    cpu.us as f64 / 1000.0 / cpu.frames.max(1) as f64,
                    cpu.most as f64 / 1000.0,
                    cpu.set_us as f64 / 1000.0 / cpu.sets.max(1) as f64,
                    cpu.slowest,
                );
            }
            // the pose's start: an empty pass's timestamp, as the pose may
            // not run and wgpu times no build of its own
            drop(encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt shadows start"),
                timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: None,
                }),
            }));
        }
        let new_set = self.set != Some(faces.set);
        if new_set {
            self.still = faces.still.clone();
            self.cells = faces.cells.clone();
            self.set = Some(faces.set);
            // the merged cells: the levels the set holds, and the cells
            // that have none, or one made for other faces (which stands
            // until theirs is made)
            for m in self.merged.values_mut() {
                if std::mem::take(&mut m.held) {
                    m.used = frame;
                }
            }
            self.cell_queue.clear();
            for (n, cell) in self.cells.iter().enumerate() {
                match self.merged.get_mut(&cell.key) {
                    Some(m) => {
                        m.held = true;
                        if m.version != cell.version {
                            self.cell_queue.push(n);
                        }
                    }
                    None => self.cell_queue.push(n),
                }
            }
            let eye = glam::Vec3::from(faces.eye);
            let cells = &self.cells;
            self.cell_queue
                .sort_by_cached_key(|&n| std::cmp::Reverse(cell_centre(cells[n].key >> 16).distance_squared(eye).to_bits()));
            if self.timer.is_some() {
                self.still_sizes = [(0, 0); 5];
                let counts = self
                    .still
                    .iter()
                    .map(|i| i.index_count)
                    .chain(self.cells.iter().flat_map(|c| c.faces.iter().map(|f| f.index_count)));
                for count in counts {
                    let triangles = count as u64 / 3;
                    let size = &mut self.still_sizes[[64, 256, 1024, 4096].iter().take_while(|&&most| triangles > most).count()];
                    *size = (size.0 + 1, size.1 + triangles);
                }
            }
        }
        // the cells whose levels this frame makes, nearest first
        let cells = self.cells.clone();
        let mut bake: Vec<usize> = Vec::new();
        let mut bake_vertices = 0u64;
        while let Some(&n) = self.cell_queue.last() {
            let wanted = cells[n].vertices as u64;
            if !bake.is_empty() && bake_vertices + wanted > MERGED_VERTICES {
                break;
            }
            bake_vertices += wanted;
            bake.push(n);
            self.cell_queue.pop();
        }
        let mut parts = [0u64; 5];
        let mut lap = started;
        let mut part = |n: usize| parts[n] += std::mem::replace(&mut lap, std::time::Instant::now()).elapsed().as_micros() as u64;
        let pose_group = self.scene_group(device, scene);
        let (posed_faces, posed) = {
            let bake: Vec<&Cell> = bake.iter().map(|&n| &cells[n]).collect();
            self.poser.pose(device, queue, encoder, &pose_group, faces, &bake)
        };
        // (those the posed buffer had no room for wait)
        self.cell_queue.extend(bake.drain(self.poser.baked.len()..).rev());
        if let Some(timer) = &self.timer {
            drop(encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt shadows posed"),
                timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(1),
                    end_of_pass_write_index: None,
                }),
            }));
        }
        part(0);
        let same_arena = self.arena.as_ref().is_some_and(|(v, i)| v == vertices && i == indices);
        if !same_arena {
            self.retired.extend(self.bottoms.drain().map(|(_, b)| (frame, b.blas)));
            self.arena = Some((vertices.clone(), indices.clone()));
        }
        let arena_vertices = vertices.size() / VERTEX_BYTES;
        let arena_indices = indices.size() / 2;
        let in_arena = |key: &Key| {
            (key.base_vertex.max(0) as u64) < arena_vertices && key.first_index as u64 + key.index_count as u64 <= arena_indices
        };
        let size_of = |key: &Key| wgpu::BlasTriangleGeometrySizeDescriptor {
            vertex_format: wgpu::VertexFormat::Float32x3,
            vertex_count: (arena_vertices - key.base_vertex.max(0) as u64).min(FACE_VERTICES) as u32,
            index_format: Some(wgpu::IndexFormat::Uint16),
            index_count: Some(key.index_count),
            flags: geometry_flags(key.opaque),
        };
        // a new still set (or a new arena): the bottom levels it holds,
        // and the geometries it has none for yet
        let fresh = new_set || !same_arena || self.tlas.is_none();
        if fresh {
            for b in self.bottoms.values_mut() {
                b.held = false;
            }
            self.queue.clear();
            let mut queued = std::collections::HashSet::new();
            for i in self.still.iter() {
                let key = Key::of(i);
                match self.bottoms.get_mut(&key) {
                    Some(b) => b.held = true,
                    None if in_arena(&key) && queued.insert(key) => self.queue.push(key),
                    None => {}
                }
            }
            self.queue.reverse();
            self.stale = true;
        }
        // the bottom levels to build, until the frame's budget: the moving
        // faces' first (this frame's top level places them), then the
        // still set's in its order
        let mut new: Vec<(Key, wgpu::BlasTriangleGeometrySizeDescriptor, bool)> = Vec::new();
        let mut index: HashMap<Key, usize> = HashMap::new();
        let mut triangles = 0u64;
        let mut waiting = 0usize;
        for i in &faces.moving {
            let key = Key::of(i);
            if let Some(b) = self.bottoms.get_mut(&key) {
                b.used = frame;
                continue;
            }
            if index.contains_key(&key) || !in_arena(&key) {
                continue;
            }
            let tris = key.index_count as u64 / 3;
            if !build_room(new.len(), triangles, tris) {
                waiting += 1;
                continue;
            }
            triangles += tris;
            index.insert(key, new.len());
            new.push((key, size_of(&key), false));
        }
        while let Some(key) = self.queue.last().copied() {
            if let Some(b) = self.bottoms.get_mut(&key) {
                b.held = true;
            } else if let Some(&n) = index.get(&key) {
                new[n].2 = true;
            } else {
                let tris = key.index_count as u64 / 3;
                if !build_room(new.len(), triangles, tris) {
                    break;
                }
                triangles += tris;
                index.insert(key, new.len());
                new.push((key, size_of(&key), true));
            }
            self.queue.pop();
        }
        // made within the frame's time for it, the first always
        let making = std::time::Instant::now();
        let mut created: Vec<wgpu::Blas> = Vec::with_capacity(new.len());
        for (_, size, _) in &new {
            if !created.is_empty() && making.elapsed() >= BUILD_TIME {
                break;
            }
            created.push(device.create_blas(
                &wgpu::CreateBlasDescriptor {
                    label: Some("rt shadows face"),
                    flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                    update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                },
                wgpu::BlasGeometrySizeDescriptors::Triangles {
                    descriptors: vec![size.clone()],
                },
            ));
        }
        let (late, moving) = unmade(&mut new, created.len(), &mut self.queue, &mut index);
        triangles -= late;
        waiting += moving + self.queue.len();
        // the levels of the cells posed this frame
        let baked: Vec<(usize, u32, Vec<wgpu::BlasTriangleGeometrySizeDescriptor>, wgpu::Blas)> = bake
            .iter()
            .zip(&self.poser.baked)
            .map(|(&n, &offset)| {
                let sizes: Vec<_> = cell_geometries(&cells[n].faces)
                    .map(|(vertices, opaque)| posed_size(vertices, opaque))
                    .collect();
                let blas = device.create_blas(
                    &wgpu::CreateBlasDescriptor {
                        label: Some("rt shadows cell"),
                        flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                    },
                    wgpu::BlasGeometrySizeDescriptors::Triangles {
                        descriptors: sizes.clone(),
                    },
                );
                (n, offset, sizes, blas)
            })
            .collect();
        part(1);
        // the scene's top level: built when its set changed, when a cell's
        // level is made, when the last bottom level it waited for is, and
        // now and then on the way
        let rebuild = fresh || !baked.is_empty() || (self.stale && (self.queue.is_empty() || frame - self.built_at >= CATCH_UP));
        if rebuild {
            // grown in powers of two
            let wanted = (self.still.len() + cells.len()).max(1).next_power_of_two().max(1024);
            if self.tlas.as_ref().is_none_or(|t| !fits(t.get().len(), wanted)) {
                self.tlas = Some(device.create_tlas(&wgpu::CreateTlasDescriptor {
                    label: Some("rt shadows scene"),
                    max_instances: wanted as u32,
                    flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                    update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                }));
            }
        }
        let Some(tlas) = self.tlas.as_mut() else {
            return;
        };
        if rebuild {
            let mut placed = 0usize;
            // every slot: one past this set's would keep an earlier set's
            // face
            let slots = tlas.get().len();
            let slots = tlas.get_mut_slice(0..slots).expect("the top level's own slots");
            for i in self.still.iter() {
                let key = Key::of(i);
                let blas = match self.bottoms.get(&key) {
                    Some(b) => &b.blas,
                    None => match index.get(&key) {
                        Some(&n) => &created[n],
                        None => continue,
                    },
                };
                slots[placed] = Some(wgpu::TlasInstance::new(blas, row_major(&i.model), i.record, 0xFF));
                placed += 1;
            }
            // the cells' levels after them: this frame's, or the one an
            // earlier set left
            let placed_faces = placed;
            self.merged_list.clear();
            self.merged_faces = 0;
            let made: HashMap<usize, &wgpu::Blas> = baked.iter().map(|(n, _, _, blas)| (*n, blas)).collect();
            for (n, cell) in cells.iter().enumerate() {
                let (blas, table) = match made.get(&n) {
                    Some(&blas) => (blas, None),
                    None => match self.merged.get(&cell.key) {
                        Some(m) => (&m.blas, Some(&m.table)),
                        None => continue,
                    },
                };
                let first = self.merged_list.len() as u32;
                if first >= MERGED {
                    break;
                }
                let from = self.merged_list.len();
                match table {
                    Some(table) => self.merged_list.extend_from_slice(table),
                    None => self.merged_list.extend(cell_table(&cell.faces)),
                }
                self.merged_faces += (self.merged_list.len() - from).saturating_sub(2) / 2;
                slots[placed] = Some(wgpu::TlasInstance::new(blas, IDENTITY, MERGED | first, 0xFF));
                placed += 1;
            }
            for slot in &mut slots[placed..] {
                *slot = None;
            }
            let bytes = (self.merged_list.len().max(1) * 4) as u64;
            if self.merged_records.size() < bytes {
                self.merged_records = records_buffer(device, "rt shadows merged records", bytes.next_power_of_two());
            }
            queue.write_buffer(&self.merged_records, 0, bytemuck::cast_slice(&self.merged_list));
            self.built_at = frame;
            self.stale = !self.queue.is_empty();
            self.scene_instances = placed_faces;
            self.scene_builds += 1;
        }
        let entries: Vec<wgpu::BlasBuildEntry<'_>> = new
            .iter()
            .zip(&created)
            .map(|((key, size, _), blas)| wgpu::BlasBuildEntry {
                blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                    size,
                    vertex_buffer: vertices,
                    first_vertex: key.base_vertex.max(0) as u32,
                    vertex_stride: VERTEX_BYTES,
                    index_buffer: Some(indices),
                    first_index: Some(key.first_index),
                    transform_buffer: None,
                    transform_buffer_offset: None,
                }]),
            })
            .chain(baked.iter().filter_map(|(_, offset, sizes, blas)| {
                let buffer = self.poser.vertices.as_ref()?;
                let mut first = *offset;
                let geometries = sizes
                    .iter()
                    .map(|size| {
                        let geometry = wgpu::BlasTriangleGeometry {
                            size,
                            vertex_buffer: buffer,
                            first_vertex: first,
                            vertex_stride: POSED_BYTES,
                            index_buffer: None,
                            first_index: None,
                            transform_buffer: None,
                            transform_buffer_offset: None,
                        };
                        first += size.vertex_count;
                        geometry
                    })
                    .collect();
                Some(wgpu::BlasBuildEntry {
                    blas,
                    geometry: wgpu::BlasGeometries::TriangleGeometries(geometries),
                })
            }))
            .collect();
        if rebuild || !entries.is_empty() {
            encoder.build_acceleration_structures(entries.iter(), rebuild.then_some(&*tlas));
        }
        drop(entries);
        let cells_built = baked.len();
        for (n, _, _, blas) in baked {
            let cell = &cells[n];
            let before = self.merged.insert(
                cell.key,
                Merged {
                    blas,
                    version: cell.version,
                    table: cell_table(&cell.faces),
                    held: true,
                    used: frame,
                },
            );
            self.retired.extend(before.map(|m| (frame, m.blas)));
        }
        let built = created.len();
        for ((key, _, held), blas) in new.into_iter().zip(created) {
            self.bottoms.insert(key, Bottom { blas, used: frame, held });
        }
        // the levels no face used for a while are retired, and those
        // retired a while ago released, within the frame's time for it
        if frame.is_multiple_of(SWEEP_EVERY) {
            let old = self.bottoms.extract_if(|_, b| !b.held && frame - b.used > KEEP_FRAMES);
            self.retired.extend(old.map(|(_, b)| (frame, b.blas)));
            let old = self.merged.extract_if(|_, m| !m.held && frame - m.used > KEEP_FRAMES);
            self.retired.extend(old.map(|(_, m)| (frame, m.blas)));
        }
        let releasing = std::time::Instant::now();
        while self.retired.front().is_some_and(|&(at, _)| frame >= at + RETIRE_FRAMES) && releasing.elapsed() < RELEASE_TIME {
            self.retired.pop_front();
        }
        part(2);
        // the moving faces, placed with the skeletons: after the bottom
        // levels' build, which theirs may be part of
        let movers: Vec<Mover<'_>> = faces
            .moving
            .iter()
            .filter_map(|i| {
                self.bottoms.get(&Key::of(i)).map(|b| Mover {
                    blas: &b.blas,
                    transform: row_major(&i.model),
                    record: i.record,
                })
            })
            .collect();
        let (skeletons_built, skeletons) = self.poser.build(device, queue, encoder, faces, &movers);
        let moving = movers.len();
        drop(movers);
        part(3);
        self.posed_live = skeletons > 0 || moving > 0;
        // the trace reads the posed top level only when it holds a face
        queue.write_buffer(
            &self.buffer,
            (std::mem::offset_of!(ParamsU, mode) + 12) as u64,
            bytemuck::bytes_of(&if self.posed_live { 1.0f32 } else { 0.0 }),
        );
        self.stats = RtStats {
            instances: self.scene_instances + self.merged_faces + moving,
            moving,
            merged: self.merged_faces,
            cells: cells.len(),
            cells_built,
            cells_waiting: self.cell_queue.len(),
            bottoms: self.bottoms.len(),
            built,
            built_triangles: triangles,
            waiting,
            posed_faces,
            posed_vertices: posed as u64,
            skeletons,
            skeletons_built,
        };
        self.trace(device, encoder, t, depth, contact, scene, Round::Live);
        part(4);
        let us = started.elapsed().as_micros() as u64;
        self.cpu.frames += 1;
        self.cpu.us += us;
        if us > self.cpu.most {
            self.cpu.most = us;
            self.cpu.slowest = Slowest {
                parts,
                set: new_set,
                levels: built,
                cells: cells_built,
                skeletons: skeletons_built,
                moving,
            };
        }
        if new_set {
            self.cpu.sets += 1;
            self.cpu.set_us += us;
        }
    }

    /// The trace of the mask through the top level as it stands.
    fn trace(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        depth: &wgpu::TextureView,
        contact: &wgpu::TextureView,
        scene: &crate::packet::SceneGpu,
        round: Round,
    ) {
        // after the build: the posed records' buffer may have grown
        let scene_group = self.scene_group(device, scene);
        let Some(tlas) = self.tlas.as_ref() else {
            return;
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt shadows"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(contact),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::AccelerationStructure(tlas),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    // with the traced lights, their filter writes the mask
                    resource: wgpu::BindingResource::TextureView(t.lights.as_ref().map_or(&t.result, |l| &l.sun)),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    // without posed faces the scene's stands in, unread
                    resource: wgpu::BindingResource::AccelerationStructure(self.poser.traced().unwrap_or(tlas)),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt shadows"),
                timestamp_writes: self
                    .timer
                    .as_ref()
                    .filter(|_| round.first())
                    .map(|timer| wgpu::ComputePassTimestampWrites {
                        query_set: &timer.queries,
                        beginning_of_pass_write_index: Some(2),
                        end_of_pass_write_index: Some(3),
                    }),
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_bind_group(1, &scene_group, &[]);
            pass.set_bind_group(2, &scene.textures, &[]);
            pass.dispatch_workgroups(t.size.0.div_ceil(8), t.size.1.div_ceil(8), 1);
        }
        if let Some(timer) = self.timer.as_mut().filter(|_| round.first()) {
            timer.resolve(encoder);
        }
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn tlas_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::AccelerationStructure { vertex_return: false },
        count: None,
    }
}

impl Reflect {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, scene_layout: &wgpu::BindGroupLayout, textures: &wgpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rt reflections"),
            source: wgpu::ShaderSource::Wgsl(source(&[RT_VIEW, RT_LIGHT, include_str!("shaders/rt_reflect.wgsl")]).into()),
        });
        let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt reflections"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<ReflectU>() as u64),
                    },
                    count: None,
                },
                texture_entry(1, wgpu::TextureSampleType::Depth),
                texture_entry(2, unfiltered),
                texture_entry(3, unfiltered),
                tlas_entry(4),
                tlas_entry(5),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: REFLECT_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                storage_entry(7, true),
                texture_entry(8, unfiltered),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rt reflections"),
            bind_group_layouts: &[Some(&layout), Some(scene_layout), Some(textures)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: Some("rt reflections"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_reflect"),
            compilation_options: Default::default(),
            cache: None,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt reflections"),
            size: std::mem::size_of::<ReflectU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        log::info!("ray tracing: reflections on, in SSR's place");
        Reflect {
            layout,
            pipeline,
            buffer,
            timer: Timer::new(device, queue, 2),
        }
    }
}

fn storage_texture_entry(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn uniform_entry<T>() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<T>() as u64),
        },
        count: None,
    }
}

fn view(binding: u32, v: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(v),
    }
}

impl Denoise {
    /// `pair`: two numbers filtered at once, each by its own variance
    /// (rt_denoise.wgsl `cs_variance_pair`), in place of one colour, with a
    /// tint beside that takes the first's weights.
    fn new(device: &wgpu::Device, label: &'static str, floor: f32, pair: bool) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/rt_denoise.wgsl").into()),
        });
        let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
        let mut entries = vec![
            uniform_entry::<DenoiseU>(),
            texture_entry(1, unfiltered),
            texture_entry(2, unfiltered),
            storage_texture_entry(3, DENOISE_FORMAT),
            storage_entry(4, false),
        ];
        if pair {
            entries.extend([texture_entry(5, unfiltered), storage_texture_entry(6, SHARE_FORMAT)]);
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &'static str| {
            device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let params = (0..=ATROUS_PASSES)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: std::mem::size_of::<DenoiseU>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        Denoise {
            variance: pipeline(if pair { "cs_variance_pair" } else { "cs_variance" }),
            atrous: pipeline(if pair { "cs_atrous_pair" } else { "cs_atrous" }),
            layout,
            params,
            tiles: None,
            params_for: None,
            floor,
        }
    }

    /// The passes' constants for images of `size`, written before the
    /// denoiser's compute pass begins (`encode`). `first` is the frame
    /// number of an instance that keeps its first à-trous pass as the next
    /// frame's history: what is kept is rounded by a chance that changes
    /// with it (rt_denoise.wgsl `dithered`), at each round of a photo too.
    #[allow(clippy::too_many_arguments)]
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: (u32, u32),
        tinted: bool,
        first: Option<u32>,
        round: Round,
    ) {
        if self.params_for != Some((size.0, size.1, tinted)) {
            for (k, buffer) in self.params.iter().enumerate() {
                let step = if k == 0 { 0.0 } else { (1u32 << (k - 1)) as f32 };
                // (an instance is always given a first image, or never)
                let (held, tint_held) = calm_held(k, first.is_some());
                let flag = |on: bool| if on { 1.0 } else { 0.0 };
                let u = DenoiseU {
                    size: [size.0 as f32, size.1 as f32, step, self.floor],
                    mode: [flag(tinted), 0.0, flag(held), flag(tint_held)],
                };
                put(device, queue, encoder, buffer, bytemuck::bytes_of(&u), round);
            }
            let count = size.0.div_ceil(8) as u64 * size.1.div_ceil(8) as u64;
            self.tiles = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rt denoise tiles"),
                size: count * 4,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            }));
            self.params_for = Some((size.0, size.1, tinted));
        }
        if let (Some(frame), Some(buffer)) = (first, self.params.get(1)) {
            let u = DenoiseU {
                size: [size.0 as f32, size.1 as f32, 1.0, self.floor],
                mode: [if tinted { 1.0 } else { 0.0 }, 1.0 + (frame % (1 << 20)) as f32, 0.0, 0.0],
            };
            put(device, queue, encoder, buffer, bytemuck::bytes_of(&u), round);
        }
    }

    /// Filters `history` (rgb its mean, a its frames) of `size` beside its
    /// `guide`, through `ping`: the image of the two that holds the result.
    /// With `first`, the first à-trous pass is written there in place of
    /// `ping`, for the caller to keep as the next frame's history
    /// (`prepare` was given its frame number).
    /// A pair's `tint` is its history, its two images and whether it is
    /// kept (one pixel each otherwise, bound and left alone): the second
    /// image returned holds its result.
    fn encode<'a>(
        &mut self,
        device: &wgpu::Device,
        pass: &mut wgpu::ComputePass<'_>,
        size: (u32, u32),
        history: &'a wgpu::TextureView,
        guide: &wgpu::TextureView,
        ping: &'a [wgpu::TextureView; 2],
        first: Option<&'a wgpu::TextureView>,
        tint: Option<(&'a wgpu::TextureView, &'a [wgpu::TextureView; 2], bool)>,
    ) -> (&'a wgpu::TextureView, Option<&'a wgpu::TextureView>) {
        let mut input_tint = tint.map(|(history, _, _)| history);
        let Some(tiles) = &self.tiles else {
            return (history, input_tint);
        };
        let groups = (size.0.div_ceil(8), size.1.div_ceil(8));
        let mut input = history;
        for (k, params) in self.params.iter().enumerate() {
            let output = first.filter(|_| k == 1).unwrap_or(&ping[k % 2]);
            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                view(1, input),
                view(2, guide),
                view(3, output),
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: tiles.as_entire_binding(),
                },
            ];
            if let (Some((_, images, _)), Some(read)) = (tint, input_tint) {
                entries.extend([view(5, read), view(6, &images[k % 2])]);
                // (the variance pass leaves the tint where it is)
                if k > 0 {
                    input_tint = Some(&images[k % 2]);
                }
            }
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rt denoise"),
                layout: &self.layout,
                entries: &entries,
            });
            pass.set_pipeline(if k == 0 { &self.variance } else { &self.atrous });
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
            input = output;
        }
        (input, input_tint)
    }
}

/// Whether the image pass `k` of the denoiser writes to holds this frame's
/// tiles with no noise already, and whether its tint's does: a pass copies
/// them as they are, so the image an earlier pass of the frame wrote to has
/// them. The passes write to two images in turn (`Denoise::encode`), the
/// second to `kept` instead where there is one; the tint's start at the
/// second pass.
fn calm_held(k: usize, kept: bool) -> (bool, bool) {
    let image = |pass: usize| if kept && pass == 1 { 2 } else { pass % 2 };
    (
        (0..k).any(|before| image(before) == image(k)),
        (1..k).any(|before| before % 2 == k % 2),
    )
}

impl Gi {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, scene_layout: &wgpu::BindGroupLayout, textures: &wgpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rt gi"),
            source: wgpu::ShaderSource::Wgsl(source(&[RT_VIEW, RT_LIGHT, RT_HISTORY, include_str!("shaders/rt_gi.wgsl")]).into()),
        });
        let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
        let trace_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt gi trace"),
            entries: &[
                uniform_entry::<GiU>(),
                texture_entry(1, wgpu::TextureSampleType::Depth),
                texture_entry(3, unfiltered),
                tlas_entry(4),
                tlas_entry(5),
                texture_entry(8, unfiltered),
                texture_entry(12, unfiltered),
                texture_entry(14, unfiltered),
                texture_entry(15, unfiltered),
                storage_texture_entry(16, GI_HIT_FORMAT),
                storage_texture_entry(17, GI_FORMAT),
                storage_entry(20, true),
                texture_entry(21, wgpu::TextureSampleType::Depth),
            ],
        });
        let reuse_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt gi reuse"),
            entries: &[
                uniform_entry::<GiU>(),
                texture_entry(1, wgpu::TextureSampleType::Depth),
                texture_entry(2, unfiltered),
                texture_entry(3, unfiltered),
                tlas_entry(4),
                tlas_entry(5),
                storage_texture_entry(6, GI_FORMAT),
                storage_texture_entry(7, GUIDE_FORMAT),
                texture_entry(12, unfiltered),
                texture_entry(13, unfiltered),
                texture_entry(18, unfiltered),
                texture_entry(19, unfiltered),
                storage_entry(20, true),
            ],
        });
        let filter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt gi filter"),
            entries: &[
                uniform_entry::<GiU>(),
                texture_entry(1, wgpu::TextureSampleType::Depth),
                texture_entry(9, unfiltered),
                texture_entry(10, unfiltered),
                storage_texture_entry(11, GI_FORMAT),
            ],
        });
        let pipeline = |label: &'static str, layouts: &[Option<&wgpu::BindGroupLayout>], entry: &'static str| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: layouts,
                immediate_size: 0,
            });
            device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let trace = pipeline("rt gi trace", &[Some(&trace_layout), Some(scene_layout), Some(textures)], "cs_gi");
        // (the reuse's shadow ray reads the scene's tables and textures too)
        let reuse = pipeline(
            "rt gi reuse",
            &[Some(&reuse_layout), Some(scene_layout), Some(textures)],
            "cs_gi_reuse",
        );
        let filter = pipeline("rt gi filter", &[Some(&filter_layout)], "cs_gi_filter");
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt gi"),
            size: std::mem::size_of::<GiU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        log::info!("ray tracing: indirect light on, in the screen-space indirect light's place");
        Gi {
            trace_layout,
            reuse_layout,
            filter_layout,
            trace,
            reuse,
            filter,
            buffer,
            denoise: Denoise::new(device, "rt gi denoise", GI_NOISE_FLOOR, false),
            timer: Timer::new(device, queue, 4),
            last: 0,
            history_for: None,
            prev_clip_from_world: glam::Mat4::IDENTITY,
            prev_camera: glam::Vec3::ZERO,
            frame: 0,
            screen: None,
            traced_at: None,
            frame_seconds: 1.0 / 60.0,
            changes: crate::rt_light_changes::LightChanges::default(),
        }
    }
}

impl Lights {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene_layout: &wgpu::BindGroupLayout,
        textures: &wgpu::BindGroupLayout,
        points: bool,
    ) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rt lights"),
            source: wgpu::ShaderSource::Wgsl(source(&[RT_VIEW, RT_LIGHT, RT_HISTORY, include_str!("shaders/rt_lights.wgsl")]).into()),
        });
        let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
        let lights_entry = storage_entry(2, true);
        let trace_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt lights trace"),
            entries: &[
                uniform_entry::<LightsU>(),
                texture_entry(1, wgpu::TextureSampleType::Depth),
                lights_entry,
                texture_entry(3, unfiltered),
                tlas_entry(5),
                tlas_entry(6),
                texture_entry(12, unfiltered),
                texture_entry(13, unfiltered),
                storage_texture_entry(15, MID_FORMAT),
            ],
        });
        let reuse_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt lights reuse"),
            entries: &[
                uniform_entry::<LightsU>(),
                texture_entry(1, wgpu::TextureSampleType::Depth),
                lights_entry,
                storage_texture_entry(4, CHOICE_FORMAT),
                texture_entry(7, unfiltered),
                storage_texture_entry(8, SHARE_FORMAT),
                texture_entry(10, unfiltered),
                texture_entry(12, unfiltered),
                texture_entry(13, unfiltered),
                storage_texture_entry(14, GUIDE_FORMAT),
                texture_entry(16, wgpu::TextureSampleType::Uint),
                texture_entry(17, unfiltered),
                storage_texture_entry(18, SHARE_FORMAT),
            ],
        });
        let filter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt lights filter"),
            entries: &[
                uniform_entry::<LightsU>(),
                texture_entry(9, unfiltered),
                texture_entry(10, unfiltered),
                storage_texture_entry(11, RESULT_FORMAT),
                texture_entry(19, unfiltered),
            ],
        });
        let pipeline = |label: &'static str, layouts: &[Option<&wgpu::BindGroupLayout>], entry: &'static str| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: layouts,
                immediate_size: 0,
            });
            device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        // the trace reads the scene's tables and textures at each candidate
        // hit (rt_common.wgsl `stops`)
        let trace = pipeline(
            "rt lights trace",
            &[Some(&trace_layout), Some(scene_layout), Some(textures)],
            "cs_lights",
        );
        // (the reuse traces nothing: no scene, rt_lights.wgsl)
        let reuse = pipeline("rt lights reuse", &[Some(&reuse_layout)], "cs_lights_reuse");
        let filter = pipeline("rt lights filter", &[Some(&filter_layout)], "cs_lights_filter");
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt lights"),
            size: std::mem::size_of::<LightsU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Lights {
            points,
            trace_layout,
            reuse_layout,
            filter_layout,
            trace,
            reuse,
            filter,
            buffer,
            denoise: Denoise::new(device, "rt lights denoise", SHARE_NOISE_FLOOR, true),
            listed: Vec::new(),
            places: HashMap::new(),
            changes: crate::rt_light_changes::LightChanges::default(),
            timer: Timer::new(device, queue, 5),
            last: 0,
            history_for: None,
            prev_clip_from_world: glam::Mat4::IDENTITY,
            prev_camera: glam::Vec3::ZERO,
            frame: 0,
        }
    }
}

/// The frame's lights as rt_lights.wgsl reads them, in the frame uniform's
/// order; `places` is where each was last frame, then where it is.
fn traced_lights(lights: &[crate::types::PointLight], places: &mut HashMap<u32, glam::Vec3>) -> Vec<TracedLightU> {
    let before = std::mem::take(places);
    lights
        .iter()
        .take(MAX_TRACED_LIGHTS)
        .map(|l| {
            let mut moved = 0.0;
            if l.source_id != 0 {
                moved = before.get(&l.source_id).map_or(0.0, |was| was.distance(l.position));
                places.insert(l.source_id, l.position);
            }
            TracedLightU {
                pos_radius: l.position.extend(l.radius).to_array(),
                color_falloff: l.color.extend(l.falloff).to_array(),
                source: [
                    l.source_radius,
                    if l.projector.is_some() { 1.0 } else { 0.0 },
                    if moved.is_finite() { moved } else { 0.0 },
                    0.0,
                ],
            }
        })
        .collect()
}

impl RtShadows {
    /// Follows the options ([`traced`], still with the sun's shadows):
    /// makes what an effect turned on needs, drops what one turned off
    /// held.
    pub(crate) fn set_options(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        textures: &wgpu::BindGroupLayout,
        options: crate::RayTracing,
    ) {
        if options.reflections != self.reflect.is_some() {
            self.reflect = options
                .reflections
                .then(|| Reflect::new(device, queue, &self.scene_layout, textures));
        }
        if options.indirect != self.gi.is_some() {
            self.gi = options.indirect.then(|| Gi::new(device, queue, &self.scene_layout, textures));
        }
        if let Some(l) = &mut self.lights {
            l.points = options.lights;
        }
    }

    /// Whether the point lights' shadows are traced: the frame then binds
    /// the mask with their share (common.wgsl `local_share`).
    pub(crate) fn traces_lights(&self) -> bool {
        self.lights.as_ref().is_some_and(|l| l.points)
    }

    /// Averages and filters the sun's soft shadow and, with the traced
    /// lights, traces the point lights' share by colour into the mask's
    /// `.gba`: after
    /// the sun's pass (which wrote this frame's ray to the targets' sun
    /// mask) and before the scene is lit. Runs on every traced frame: the
    /// mask the frame binds is its. `motion` is the prepass's normals and
    /// motion, or None without them.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_lights(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        light: &TraceLight,
        point_lights: &[crate::types::PointLight],
        depth: &wgpu::TextureView,
        motion: Option<&wgpu::TextureView>,
        black: &wgpu::TextureView,
        scene: &crate::packet::SceneGpu,
        round: Round,
    ) {
        let scene_group = self.scene_group(device, scene);
        // the frame's lights, for the hits the reflections and the indirect
        // light shade too (a photo's later rounds have the first's)
        let lamps = match (round.first(), self.lights.as_mut()) {
            (false, Some(l)) if l.points => self.lamps_still.clone(),
            (false, _) => Vec::new(),
            (true, Some(l)) => traced_lights(point_lights, &mut l.places),
            (true, None) => traced_lights(point_lights, &mut HashMap::new()),
        };
        if round.first() {
            if !lamps.is_empty() {
                queue.write_buffer(&self.lamps, 0, bytemuck::cast_slice(&lamps));
            }
            self.lamps_count = lamps.len() as u32;
            self.lamps_moved = lamps.iter().any(|l| l.source[2] > 0.0);
            if round.still() {
                self.lamps_still = lamps.clone();
            }
        } else if std::mem::take(&mut self.lamps_moved) {
            // a lamp that moved since the view's last frame stands still
            // between two rounds: the history near it is no longer let go
            for lamp in &mut self.lamps_still {
                lamp.source[2] = 0.0;
            }
            put(device, queue, encoder, &self.lamps, bytemuck::cast_slice(&self.lamps_still), round);
        }
        let (Some(l), Some(tlas), Some(lt)) = (self.lights.as_mut(), self.tlas.as_ref(), t.lights.as_ref()) else {
            return;
        };
        let mesh_normals = motion.is_some();
        let motion = motion.unwrap_or(black);
        if let Some(timer) = l.timer.as_mut().filter(|_| round.first())
            && let Some((ms, frames)) = timer.harvest()
        {
            log::info!(
                "rt lights: trace {:.3} ms, reuse {:.3} ms, denoise {:.3} ms, filter {:.3} ms (GPU, mean of {frames} frames, {} lights listed)",
                ms[0],
                ms[1],
                ms[2],
                ms[3],
                l.listed.len()
            );
        }
        let (clip_from_world, camera) = (light.clip_from_world, light.camera);
        let listed = if l.points { lamps } else { Vec::new() };
        let same_lights = listed == l.listed;
        l.listed = listed;
        let (read, write) = (l.last, 1 - l.last);
        let valid = round.history(l.history_for == Some(t.serial));
        // (2: the motion beside the prepass's normals is not read, rt_history.wgsl)
        let normals = match (mesh_normals, round.still()) {
            (false, _) => 0.0,
            (true, false) => 1.0,
            (true, true) => 2.0,
        };
        let tinted = l.points && lt.tinted;
        let traced = &point_lights[..point_lights.len().min(if l.points { MAX_TRACED_LIGHTS } else { 0 })];
        let changed = l
            .changes
            .update(traced, light.sky(), clip_from_world, camera, 1.0 / LIGHTS_HISTORY, valid);
        let (w, h) = (t.size.0 as f32, t.size.1 as f32);
        let u = LightsU {
            world_from_clip: clip_from_world.inverse().to_cols_array_2d(),
            prev_clip_from_world: l.prev_clip_from_world.to_cols_array_2d(),
            camera: camera.extend(normals).to_array(),
            prev_camera: l.prev_camera.extend(if tinted { 1.0 } else { 0.0 }).to_array(),
            screen: [w, h, 1.0 / w, 1.0 / h],
            mode: [
                l.listed.len() as f32,
                if self.posed_live { 1.0 } else { 0.0 },
                (l.frame % (1 << 20)) as f32,
                match (valid, same_lights) {
                    (false, _) => 0.0,
                    (true, false) => 1.0,
                    (true, true) => 3.0,
                },
            ],
            changed: changed.places,
            changes: [changed.count as f32, 0.0, changed.flags()[2], 0.0],
            sets: [
                light.light_sets.forward[0],
                light.light_sets.forward[1],
                light.light_sets.forward[2],
                light.light_sets.mode[0] as f32,
            ],
            columns: light.light_sets.columns,
            rows: light.light_sets.rows,
            slices: light.light_sets.slices,
        };
        put(device, queue, encoder, &l.buffer, bytemuck::bytes_of(&u), round);
        let timer = l.timer.as_ref().filter(|_| round.first());
        let uniform = wgpu::BindGroupEntry {
            binding: 0,
            resource: l.buffer.as_entire_binding(),
        };
        let trace_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt lights trace"),
            layout: &l.trace_layout,
            entries: &[
                uniform.clone(),
                view(1, depth),
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.lamps.as_entire_binding(),
                },
                view(3, &lt.choice[read]),
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::AccelerationStructure(tlas),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    // without posed faces the scene's stands in, unread
                    resource: wgpu::BindingResource::AccelerationStructure(self.poser.traced().unwrap_or(tlas)),
                },
                view(12, motion),
                view(13, &lt.guide[read]),
                view(15, &lt.mid),
            ],
        });
        let reuse_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt lights reuse"),
            layout: &l.reuse_layout,
            entries: &[
                uniform.clone(),
                view(1, depth),
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.lamps.as_entire_binding(),
                },
                view(4, &lt.choice[write]),
                view(7, &lt.share[read]),
                view(8, &lt.share[write]),
                view(10, &lt.sun),
                view(12, motion),
                view(13, &lt.guide[read]),
                view(14, &lt.guide[write]),
                view(16, &lt.mid),
                view(17, &lt.tint[read]),
                view(18, &lt.tint[write]),
            ],
        });
        let groups = (t.size.0.div_ceil(8), t.size.1.div_ceil(8));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt lights trace"),
                timestamp_writes: timer.map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: None,
                }),
            });
            // with no light listed the reuse reads no sample
            if !l.listed.is_empty() {
                pass.set_pipeline(&l.trace);
                pass.set_bind_group(0, &trace_group, &[]);
                pass.set_bind_group(1, &scene_group, &[]);
                pass.set_bind_group(2, &scene.textures, &[]);
                pass.dispatch_workgroups(groups.0, groups.1, 1);
            }
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt lights reuse"),
                timestamp_writes: timer.map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(1),
                    end_of_pass_write_index: None,
                }),
            });
            pass.set_pipeline(&l.reuse);
            pass.set_bind_group(0, &reuse_group, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }
        l.denoise.prepare(device, queue, encoder, t.size, tinted, None, round);
        let (denoised, denoised_tint) = {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt lights denoise"),
                timestamp_writes: timer.map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(2),
                    end_of_pass_write_index: None,
                }),
            });
            l.denoise.encode(
                device,
                &mut pass,
                t.size,
                &lt.share[write],
                &lt.guide[write],
                &lt.ping,
                None,
                Some((&lt.tint[write], &lt.tint_ping, tinted)),
            )
        };
        let filter_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt lights filter"),
            layout: &l.filter_layout,
            entries: &[
                uniform,
                view(9, denoised),
                view(10, &lt.sun),
                view(11, &t.result),
                view(19, denoised_tint.unwrap_or(&lt.tint[write])),
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt lights filter"),
                timestamp_writes: timer.map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(3),
                    end_of_pass_write_index: Some(4),
                }),
            });
            pass.set_pipeline(&l.filter);
            pass.set_bind_group(0, &filter_group, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }
        if let Some(timer) = l.timer.as_mut().filter(|_| round.first()) {
            timer.resolve(encoder);
        }
        l.last = write;
        l.history_for = Some(t.serial);
        l.prev_clip_from_world = clip_from_world;
        l.prev_camera = camera;
        l.frame = l.frame.wrapping_add(1);
    }
}

impl RtShadows {
    /// Before a photo's targets are made: keeps `scene_copy`, the view's
    /// last lit frame and its `depth` (of `t`'s size), for the hits of the photo's indirect
    /// light, with the view it was drawn with. The first call of a photo
    /// counts (its tiles and faces follow one another with no frame of the
    /// view between them); nothing is kept unless the indirect light was
    /// traced into `t` last frame, which is when that view is known.
    pub(crate) fn hold_screen(&mut self, t: &Targets, scene_copy: &wgpu::TextureView, depth: &wgpu::TextureView) {
        if let Some(g) = self.gi.as_mut()
            && g.screen.is_none()
            && g.history_for == Some(t.serial)
        {
            g.screen = Some(HeldScreen {
                view: scene_copy.clone(),
                depth: depth.clone(),
                clip_from_world: g.prev_clip_from_world,
                size: t.size,
            });
        }
    }

    /// The rounds of a photo's traced passes into `t`: one when none of
    /// them averages over frames (the sun's hard shadow alone).
    pub(crate) fn still_rounds(&self, t: &Targets) -> u32 {
        if t.lights.is_some() || t.gi.is_some() { STILL_ROUNDS } else { 1 }
    }

    /// A frame of the view that traces no indirect light: the screen kept
    /// for a photo is let go (one that traces it reads it first, when it
    /// starts afresh, and lets it go: `encode_gi`).
    pub(crate) fn release_screen(&mut self) {
        if let Some(g) = self.gi.as_mut() {
            g.screen = None;
        }
    }

    /// Whether the traced indirect light is on: the shading reads it this
    /// frame when the frame traces.
    pub(crate) fn lights_indirectly(&self) -> bool {
        self.gi.is_some()
    }

    /// Traces this frame's indirect light into the targets' GI result,
    /// after the shadows' pass built this frame's levels and before the
    /// scene is lit; the view's hits read `scene_copy`, last frame's.
    /// `intensity` scales it as it scales GTAO's (Indirect light intensity).
    /// `point_lights` are the frame's, as the shading takes them: where one
    /// changed, the history lets go (rt_light_changes.rs).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_gi(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        light: &TraceLight,
        point_lights: &[crate::types::PointLight],
        intensity: f32,
        depth: &wgpu::TextureView,
        motion: &wgpu::TextureView,
        scene_copy: &wgpu::TextureView,
        scene: &crate::packet::SceneGpu,
        round: Round,
    ) {
        let scene_group = self.scene_group(device, scene);
        let lamps_shadowed = self.traces_lights();
        let (Some(g), Some(tlas), Some(gt)) = (self.gi.as_mut(), self.tlas.as_ref(), t.gi.as_ref()) else {
            return;
        };
        if let Some(timer) = g.timer.as_mut().filter(|_| round.first())
            && let Some((ms, frames)) = timer.harvest()
        {
            log::info!(
                "rt gi: trace and reuse {:.3} ms, denoise {:.3} ms, filter {:.3} ms (GPU, mean of {frames} frames, {}x{} rays)",
                ms[0],
                ms[1],
                ms[2],
                gt.half.0,
                gt.half.1
            );
        }
        let (read, write) = (g.last, 1 - g.last);
        let valid = round.history(g.history_for == Some(t.serial));
        let (w, h) = (t.size.0 as f32, t.size.1 as f32);
        if round.still() {
            // (a photo's rounds are no frames of the view: the next one's
            // length is counted from itself)
            g.traced_at = None;
        } else {
            let now = std::time::Instant::now();
            if let Some(before) = g.traced_at.replace(now) {
                // a pause (a window left aside) is not a frame's length
                let seconds = now.duration_since(before).as_secs_f32().min(0.1);
                g.frame_seconds += (seconds - g.frame_seconds) * 0.05;
            }
        }
        // The image a hit the view sees is read from, and the view it was
        // drawn with: `scene_copy`, last frame's. A frame with no history
        // has new targets, whose image holds nothing yet: a photo, and the
        // view's first frame after one, read the view's last frame, kept
        // for them (`hold_screen`), or shade every hit where it is.
        let (screen, screen_depth, screen_clip, screen_size) = match (&g.screen, round.still() || !valid) {
            (Some(s), true) => (&s.view, Some(&s.depth), s.clip_from_world, Some(s.size)),
            (None, true) => (scene_copy, None, g.prev_clip_from_world, None),
            (_, false) => (scene_copy, None, g.prev_clip_from_world, Some(t.size)),
        };
        let history_frames = gi_history_frames(g.frame_seconds);
        let changed = g.changes.update(
            point_lights,
            light.sky(),
            light.clip_from_world,
            light.camera,
            1.0 / history_frames,
            valid,
        );
        let u = GiU {
            clip_from_world: light.clip_from_world.to_cols_array_2d(),
            world_from_clip: light.clip_from_world.inverse().to_cols_array_2d(),
            prev_clip_from_world: g.prev_clip_from_world.to_cols_array_2d(),
            // (2: the prepass's motion is not read, rt_history.wgsl)
            camera: light.camera.extend(if round.still() { 2.0 } else { 1.0 }).to_array(),
            prev_camera: g.prev_camera.extend(1.0).to_array(),
            lit: light.lit(),
            screen: [w, h, 1.0 / w, 1.0 / h],
            half: [gt.half.0 as f32, gt.half.1 as f32, intensity, history_frames],
            mode: [
                GI_REACH,
                if self.posed_live { 1.0 } else { 0.0 },
                (g.frame % (1 << 20)) as f32,
                if valid { 1.0 } else { 0.0 },
            ],
            changed: changed.places,
            changes: changed.flags(),
            lamps: [
                self.lamps_count as f32,
                if lamps_shadowed { 1.0 } else { 0.0 },
                if self.lamps_moved { 1.0 } else { 0.0 },
                0.0,
            ],
            scene_clip_from_world: screen_clip.to_cols_array_2d(),
            scene_world_from_clip: screen_clip.inverse().to_cols_array_2d(),
            scene: match screen_size {
                Some((w, h)) => [w as f32, h as f32, 1.0, if screen_depth.is_some() { 1.0 } else { 0.0 }],
                None => [1.0, 1.0, 0.0, 0.0],
            },
        };
        put(device, queue, encoder, &g.buffer, bytemuck::bytes_of(&u), round);
        let timer = g.timer.as_ref().filter(|_| round.first());
        let levels = [
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::AccelerationStructure(tlas),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                // without posed faces the scene's stands in, unread
                resource: wgpu::BindingResource::AccelerationStructure(self.poser.traced().unwrap_or(tlas)),
            },
        ];
        let trace_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt gi trace"),
            layout: &g.trace_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: g.buffer.as_entire_binding(),
                },
                view(1, depth),
                view(3, &gt.guide[read]),
                levels[0].clone(),
                levels[1].clone(),
                view(8, screen),
                view(21, screen_depth.unwrap_or(depth)),
                view(12, motion),
                view(14, &gt.pick_hit[read]),
                view(15, &gt.pick_light[read]),
                view(16, &gt.pick_hit[write]),
                view(17, &gt.pick_light[write]),
                wgpu::BindGroupEntry {
                    binding: 20,
                    resource: self.lamps.as_entire_binding(),
                },
            ],
        });
        let reuse_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt gi reuse"),
            layout: &g.reuse_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: g.buffer.as_entire_binding(),
                },
                view(1, depth),
                view(2, &gt.color[read]),
                view(3, &gt.guide[read]),
                levels[0].clone(),
                levels[1].clone(),
                view(6, &gt.color[write]),
                view(7, &gt.guide[write]),
                view(12, motion),
                view(13, &gt.fed),
                view(18, &gt.pick_hit[write]),
                view(19, &gt.pick_light[write]),
                wgpu::BindGroupEntry {
                    binding: 20,
                    resource: self.lamps.as_entire_binding(),
                },
            ],
        });
        {
            // the trace then the reuse: the first timestamp to the
            // denoiser's is the two
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt gi trace"),
                timestamp_writes: timer.map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: None,
                }),
            });
            let groups = (gt.half.0.div_ceil(8), gt.half.1.div_ceil(8));
            pass.set_bind_group(1, &scene_group, &[]);
            pass.set_bind_group(2, &scene.textures, &[]);
            pass.set_pipeline(&g.trace);
            pass.set_bind_group(0, &trace_group, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
            pass.set_pipeline(&g.reuse);
            pass.set_bind_group(0, &reuse_group, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }
        g.denoise.prepare(device, queue, encoder, gt.half, false, Some(g.frame), round);
        let denoised = {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt gi denoise"),
                timestamp_writes: timer.map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(1),
                    end_of_pass_write_index: None,
                }),
            });
            g.denoise
                .encode(
                    device,
                    &mut pass,
                    gt.half,
                    &gt.color[write],
                    &gt.guide[write],
                    &gt.ping,
                    Some(&gt.fed),
                    None,
                )
                .0
        };
        let filter_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt gi filter"),
            layout: &g.filter_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: g.buffer.as_entire_binding(),
                },
                view(1, depth),
                view(9, denoised),
                view(10, &gt.guide[write]),
                view(11, &gt.result),
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt gi filter"),
                timestamp_writes: timer.map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(2),
                    end_of_pass_write_index: Some(3),
                }),
            });
            pass.set_pipeline(&g.filter);
            pass.set_bind_group(0, &filter_group, &[]);
            pass.dispatch_workgroups(t.size.0.div_ceil(8), t.size.1.div_ceil(8), 1);
        }
        if let Some(timer) = g.timer.as_mut().filter(|_| round.first()) {
            timer.resolve(encoder);
        }
        if round == Round::Live {
            g.screen = None;
        }
        g.last = write;
        g.history_for = Some(t.serial);
        g.prev_clip_from_world = light.clip_from_world;
        g.prev_camera = light.camera;
        g.frame = g.frame.wrapping_add(1);
    }
}

impl RtShadows {
    /// Whether the traced reflections can take SSR's place this frame:
    /// they are on and the frame traced.
    pub(crate) fn reflects(&self) -> bool {
        self.reflect.is_some() && self.tlas.is_some()
    }

    /// Traces the reflections of the glossy pixels of `gbuf` into the
    /// targets' reflection texture, after the shadows' pass built this
    /// frame's levels; the view's hits read `scene_copy`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_reflections(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        light: &TraceLight,
        depth: &wgpu::TextureView,
        gbuf: &wgpu::TextureView,
        normals: Option<&wgpu::TextureView>,
        black: &wgpu::TextureView,
        scene_copy: &wgpu::TextureView,
        scene: &crate::packet::SceneGpu,
    ) {
        let scene_group = self.scene_group(device, scene);
        let lamps_shadowed = self.traces_lights();
        let (Some(r), Some(tlas), Some(out)) = (self.reflect.as_mut(), self.tlas.as_ref(), t.reflect.as_ref()) else {
            return;
        };
        if let Some(timer) = &mut r.timer
            && let Some((ms, frames)) = timer.harvest()
        {
            log::info!("rt reflections: trace {:.3} ms (GPU, mean of {frames} frames)", ms[0]);
        }
        let (w, h) = (t.size.0 as f32, t.size.1 as f32);
        let u = ReflectU {
            clip_from_world: light.clip_from_world.to_cols_array_2d(),
            world_from_clip: light.clip_from_world.inverse().to_cols_array_2d(),
            camera: light.camera.extend(if normals.is_some() { 1.0 } else { 0.0 }).to_array(),
            lit: light.lit(),
            screen: [w, h, 1.0 / w, 1.0 / h],
            mode: [
                REACH,
                if self.posed_live { 1.0 } else { 0.0 },
                self.lamps_count as f32,
                if lamps_shadowed { 1.0 } else { 0.0 },
            ],
        };
        queue.write_buffer(&r.buffer, 0, bytemuck::bytes_of(&u));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt reflections"),
            layout: &r.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: r.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(gbuf),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(scene_copy),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::AccelerationStructure(tlas),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    // without posed faces the scene's stands in, unread
                    resource: wgpu::BindingResource::AccelerationStructure(self.poser.traced().unwrap_or(tlas)),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(out),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: self.lamps.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(normals.unwrap_or(black)),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt reflections"),
                timestamp_writes: r.timer.as_ref().map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
            });
            pass.set_pipeline(&r.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_bind_group(1, &scene_group, &[]);
            pass.set_bind_group(2, &scene.textures, &[]);
            pass.dispatch_workgroups(t.size.0.div_ceil(8), t.size.1.div_ceil(8), 1);
        }
        if let Some(timer) = &mut r.timer {
            timer.resolve(encoder);
        }
    }
}

impl RtShadows {
    /// The scene's tables for the ray-traced passes (rt_common.wgsl group
    /// 1), each bound up to the device's limit.
    fn scene_group(&mut self, device: &wgpu::Device, scene: &crate::packet::SceneGpu) -> wgpu::BindGroup {
        let max = device.limits().max_storage_buffer_binding_size;
        let buffers = [
            &scene.records,
            &scene.palettes,
            &scene.skin_binds,
            &scene.vertices,
            &scene.skin,
            &scene.indices,
            &scene.faces,
            &self.poser.records,
            &self.merged_records,
        ];
        if !self.warned && buffers.iter().any(|b| b.size() > max) {
            log::warn!("ray tracing: a scene buffer is larger than the device binds; what lies past it is not traced right");
            self.warned = true;
        }
        let entries: Vec<wgpu::BindGroupEntry<'_>> = buffers
            .iter()
            .enumerate()
            .map(|(binding, b)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: b,
                    offset: 0,
                    size: wgpu::BufferSize::new(b.size().min(max)),
                }),
            })
            .collect();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt scene"),
            layout: &self.scene_layout,
            entries: &entries,
        })
    }
}

/// Whether a top level of `slots` suits `wanted` instances (a power of
/// two): large enough, and less than four times too large. wgpu walks every
/// slot of a level at each build and sizes the build by them: the level
/// built each frame, left at the 65,536 slots of an arrival (when a
/// region's faces all count as moving for a few frames), cost over a
/// millisecond a frame on the render thread and one more on the GPU for
/// the rest of the session.
fn fits(slots: usize, wanted: usize) -> bool {
    slots >= wanted && slots < 4 * wanted
}

/// The transform of a top level's instance whose level is in world space.
const IDENTITY: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// A column-major model matrix as the top level's 3×4 row-major transform.
fn row_major(m: &[[f32; 4]; 4]) -> [f32; 12] {
    std::array::from_fn(|i| m[i % 4][i / 4])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A photo's first round reads no history, whatever the passes kept,
    /// and each of the others reads the round before; the view's frame
    /// reads what it kept. The first round alone does a frame's own work.
    #[test]
    fn a_photos_rounds_start_afresh_then_follow_one_another() {
        assert!(!Round::Still(0).history(true));
        assert!(Round::Still(1).history(false));
        assert!(Round::Still(STILL_ROUNDS - 1).history(false));
        assert!(Round::Live.history(true));
        assert!(!Round::Live.history(false));
        assert!(Round::Live.first() && Round::Still(0).first());
        assert!(!Round::Still(1).first());
        assert!(Round::Still(0).still() && Round::Still(1).still() && !Round::Live.still());
    }

    /// A photo is planned by the rounds the view's frame did not run.
    #[test]
    fn a_photo_is_planned_with_its_rounds() {
        // a frame that traces nothing over frames, or a photo's own
        assert_eq!(still_gpu_ms(4.0, 1.5, 0), 4.0);
        // the view's frame: 4 ms, 1.5 ms of them traced
        assert_eq!(still_gpu_ms(4.0, 1.5, STILL_ROUNDS - 1), 4.0 + 31.0 * 1.5);
    }

    /// The indirect light's history lasts as long at every frame rate,
    /// between its bounds.
    #[test]
    fn gi_history_lasts_half_a_second() {
        assert_eq!(gi_history_frames(1.0 / 60.0), 32.0);
        assert!((gi_history_frames(1.0 / 120.0) - 64.0).abs() < 1e-3);
        assert!((gi_history_frames(1.0 / 240.0) - 128.0).abs() < 1e-3);
        // no shorter under 60 frames a second, no longer over 240
        assert_eq!(gi_history_frames(1.0 / 20.0), 32.0);
        assert_eq!(gi_history_frames(1.0 / 500.0), 128.0);
        assert_eq!(gi_history_frames(0.0), 128.0);
    }

    #[test]
    fn rt_shadows_shaders_are_valid() {
        for (name, parts) in [
            ("rt_shadows", &[RT_VIEW, include_str!("shaders/rt_shadows.wgsl")][..]),
            ("rt_pose", &[include_str!("shaders/rt_pose.wgsl")][..]),
            ("rt_reflect", &[RT_VIEW, RT_LIGHT, include_str!("shaders/rt_reflect.wgsl")][..]),
            ("rt_gi", &[RT_VIEW, RT_LIGHT, RT_HISTORY, include_str!("shaders/rt_gi.wgsl")][..]),
            // standalone: no scene, no ray queries
            ("rt_denoise", &[include_str!("shaders/rt_denoise.wgsl")][..]),
            (
                "rt_lights",
                &[RT_VIEW, RT_LIGHT, RT_HISTORY, include_str!("shaders/rt_lights.wgsl")][..],
            ),
        ] {
            let src = if name == "rt_denoise" { parts[0].to_owned() } else { source(parts) };
            let module = match naga::front::wgsl::parse_str(&src) {
                Ok(m) => m,
                Err(e) => panic!("{name}: {}", e.emit_to_string(&src)),
            };
            let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
            if let Err(e) = v.validate(&module) {
                panic!("{name}: {}", e.emit_to_string(&src));
            }
        }
    }

    /// Reads skeleton `key`'s faces: one of `indices` indices per record,
    /// at `x`.
    fn read(t: &mut BTreeMap<u32, Skeleton>, key: u32, records: &[u32], indices: u32, x: f32) {
        let mut model = glam::Mat4::IDENTITY.to_cols_array_2d();
        model[3][0] = x;
        t.entry(key).or_default().read(records.iter().map(|&record| {
            let face = Posed {
                record,
                base_vertex: 0,
                first_index: 0,
                index_count: indices,
                opaque: true,
            };
            (face, model, 0)
        }));
    }

    fn choose(t: &mut BTreeMap<u32, Skeleton>) -> Vec<(u32, bool)> {
        let mut out = Faces::default();
        choose_rebuilds(&[], &[], glam::Vec3::ZERO, t, &mut out);
        out.groups.iter().map(|g| (g.key, g.rebuild)).collect()
    }

    #[test]
    fn rt_shadows_a_still_skeleton_keeps_its_level() {
        let mut t = BTreeMap::new();
        read(&mut t, 0, &[1], 300, 5.0);
        assert_eq!(choose(&mut t), vec![(0, true)]);
        // its faces are not read again while none changes
        assert_eq!(choose(&mut t), vec![(0, false)]);
        // less than the ray's start offset at 5 m (0.02 m): kept
        read(&mut t, 0, &[1], 300, 5.015);
        assert_eq!(choose(&mut t), vec![(0, false)]);
        // further: built again
        read(&mut t, 0, &[1], 300, 5.05);
        assert_eq!(choose(&mut t), vec![(0, true)]);
        assert_eq!(choose(&mut t), vec![(0, false)]);
        // another face set: built again even when still
        read(&mut t, 0, &[1, 2], 300, 5.05);
        assert_eq!(choose(&mut t), vec![(0, true)]);
        // its last face gone: no skeleton
        read(&mut t, 0, &[], 300, 5.05);
        assert_eq!(choose(&mut t), vec![]);
    }

    #[test]
    fn rt_shadows_rebuilds_go_nearest_first_within_the_budget() {
        let mut t = BTreeMap::new();
        let big = (REBUILD_TRIANGLES * 3 / 2) as u32;
        for (key, x) in [(1, 30.0), (2, 3.0), (3, 10.0)] {
            read(&mut t, key, &[key], big, x);
        }
        // the nearest two fit, the farthest waits for a later frame
        assert_eq!(choose(&mut t), vec![(1, false), (2, true), (3, true)]);
        assert_eq!(choose(&mut t), vec![(1, true), (2, false), (3, false)]);
    }

    #[test]
    fn rt_shadows_a_skeleton_s_joints_are_read_when_written() {
        let stride = crate::types::PALETTE_STRIDE as usize;
        let mut palettes = vec![glam::Mat4::IDENTITY.to_cols_array_2d(); 2 * stride];
        let mut t = BTreeMap::new();
        read(&mut t, 0, &[1], 300, 5.0);
        read(&mut t, stride as u32, &[2], 300, 5.0);
        let mut choose = |palettes: &[[[f32; 4]; 4]], written: &[std::ops::Range<usize>]| {
            let mut out = Faces::default();
            choose_rebuilds(palettes, written, glam::Vec3::ZERO, &mut t, &mut out);
            out.groups.iter().map(|g| g.rebuild).collect::<Vec<_>>()
        };
        assert_eq!(choose(&palettes, &[]), [true, true]);
        assert_eq!(choose(&palettes, &[]), [false, false]);
        // the second one's joint moves, and is written: built again
        palettes[stride + 7][3][0] = 1.0;
        assert_eq!(choose(&palettes, std::slice::from_ref(&(stride + 7..stride + 8))), [false, true]);
        assert_eq!(choose(&palettes, std::slice::from_ref(&(0..2 * stride))), [false, false]);
        // a little: kept, even written
        palettes[3][3][0] = 0.005;
        assert_eq!(choose(&palettes, std::slice::from_ref(&(0..stride))), [false, false]);
        // only what was written is read: the first one's anchor alone,
        // then the joint that moved, in a range that starts before it
        palettes[5][3][0] = 1.0;
        let anchor = crate::types::SKELETON_ANCHOR as usize;
        assert_eq!(
            choose(&palettes, &[anchor..anchor + 1, stride + anchor..stride + anchor + 1]),
            [false, false]
        );
        assert_eq!(choose(&palettes, &[2..6, stride - 1..stride + 1]), [true, false]);
        assert_eq!(choose(&palettes, &[]), [false, false]);
        let mut ranges = vec![40..50, 0..10, 10..12, 45..60, 70..71];
        merge(&mut ranges);
        assert_eq!(ranges, [0..12, 40..60, 70..71]);
        merge(&mut Vec::new());
    }

    #[test]
    fn rt_shadows_drift_counts_a_turn_at_the_joint_reach() {
        let a = glam::Mat4::IDENTITY.to_cols_array_2d();
        let b = glam::Mat4::from_rotation_z(0.01).to_cols_array_2d();
        let d = drift(&a, &b);
        assert!((d - 0.01 * JOINT_REACH).abs() < 1e-3, "{d}");
        assert_eq!(drift(&a, &a), 0.0);
    }

    /// A face of a test frame: its id, place, shape, traced or not, near
    /// or not.
    #[derive(Clone, Copy)]
    struct F(usize, u64, u32, bool, bool);

    /// A near, traced face at `place`.
    fn near(id: usize, place: u64) -> F {
        F(id, place, 0, true, true)
    }

    /// A far one.
    fn far(id: usize, place: u64) -> F {
        F(id, place, 0, true, false)
    }

    /// The tracker's next frame, with a look at every face: those of
    /// `faces` are there, the others are not. Whether a new still set was
    /// made, and the moving faces.
    fn frame(t: &mut StillTracker, n: &mut u64, faces: &[F]) -> (bool, Vec<usize>) {
        *n += 1;
        t.begin(t.tracks.len().max(8));
        for id in 0..t.tracks.len() {
            match faces.iter().find(|f| f.0 == id) {
                Some(&F(id, place, shape, kept, near)) => t.visit(id, *n, place, shape, 0, kept, near, true),
                None if t.tracks[id].present => t.absent(id, *n, SLOT_FREED),
                None => {}
            }
        }
        quiet_end(t, *n)
    }

    /// The tracker's next frame, with no look at any face.
    fn quiet(t: &mut StillTracker, n: &mut u64) -> (bool, Vec<usize>) {
        *n += 1;
        t.begin(t.tracks.len().max(8));
        quiet_end(t, *n)
    }

    fn quiet_end(t: &mut StillTracker, n: u64) -> (bool, Vec<usize>) {
        let set = t.decide(n);
        let mut moving: Vec<usize> = t.moving().iter().map(|&id| id as usize).collect();
        moving.sort_unstable();
        (set, moving)
    }

    /// Repeats `faces` until a still set is made: the frames it took, with
    /// the moving faces of each frame before it checked by `meanwhile`.
    fn until_set(t: &mut StillTracker, n: &mut u64, faces: &[F], meanwhile: &[usize]) -> u64 {
        for frames in 1..=40 * STILL_FRAMES as u64 {
            let (set, moving) = frame(t, n, faces);
            if set {
                return frames;
            }
            assert_eq!(moving, meanwhile, "frame {frames}");
        }
        panic!("no still set");
    }

    #[test]
    fn rt_shadows_a_new_face_is_still_at_once() {
        let (mut t, mut n) = (StillTracker::default(), 0);
        // the first frame makes the set, with every face in it
        assert_eq!(frame(&mut t, &mut n, &[near(0, 1), far(1, 2)]), (true, vec![]));
        assert!(t.in_still(0) && t.in_still(1));
        // nothing changes: the set stands, looked at or not
        for _ in 0..3 * LATER {
            assert_eq!(frame(&mut t, &mut n, &[near(0, 1), far(1, 2)]), (false, vec![]));
        }
        for _ in 0..3 * LATER {
            assert_eq!(quiet(&mut t, &mut n), (false, vec![]));
        }
        assert!(t.in_still(0) && t.in_still(1));
    }

    #[test]
    fn rt_shadows_a_face_that_moved_comes_to_rest_with_no_look() {
        let (mut t, mut n) = (StillTracker::default(), 0);
        frame(&mut t, &mut n, &[near(0, 1), near(1, 2)]);
        assert_eq!(frame(&mut t, &mut n, &[near(0, 1), near(1, 3)]), (true, vec![1]));
        // nothing looks at it again: it is traced as moving, then a set
        // takes it back
        let mut frames = 0;
        loop {
            frames += 1;
            let (set, moving) = quiet(&mut t, &mut n);
            if set {
                assert!(moving.is_empty());
                break;
            }
            assert_eq!(moving, vec![1], "frame {frames}");
            assert!(frames < 40 * STILL_FRAMES as u64);
        }
        assert!(frames >= 2 * STILL_FRAMES as u64, "back after {frames} frames");
        assert!(t.in_still(0) && t.in_still(1));
        assert_eq!(quiet(&mut t, &mut n), (false, vec![]));
    }

    #[test]
    fn rt_shadows_a_near_face_that_moves_leaves_the_still_set_at_once() {
        let (mut t, mut n) = (StillTracker::default(), 0);
        frame(&mut t, &mut n, &[near(0, 1), near(1, 2)]);
        // face 1 moves: a new set without it, and it is traced as moving
        assert_eq!(frame(&mut t, &mut n, &[near(0, 1), near(1, 3)]), (true, vec![1]));
        assert!(t.in_still(0) && !t.in_still(1));
        // it keeps moving: no set for that
        assert_eq!(frame(&mut t, &mut n, &[near(0, 1), near(1, 4)]), (false, vec![1]));
        // it rests: among the moving ones until a set takes it back, after
        // twice STILL_FRAMES at least (it left one)
        let back = until_set(&mut t, &mut n, &[near(0, 1), near(1, 4)], &[1]);
        assert!(back >= 2 * STILL_FRAMES as u64, "back after {back} frames");
        assert!(t.in_still(1));
    }

    #[test]
    fn rt_shadows_a_far_face_that_moves_is_in_both_until_the_next_set() {
        let (mut t, mut n) = (StillTracker::default(), 0);
        frame(&mut t, &mut n, &[near(0, 1), far(1, 2)]);
        // face 1 moves once: no set for it at once, the level keeps it
        // where it was and it is traced where it is
        assert_eq!(frame(&mut t, &mut n, &[near(0, 1), far(1, 3)]), (false, vec![1]));
        assert!(t.in_still(1));
        assert_eq!(until_set(&mut t, &mut n, &[near(0, 1), far(1, 3)], &[1]), SOON);
        assert!(t.in_still(0) && !t.in_still(1));
        assert_eq!(frame(&mut t, &mut n, &[near(0, 1), far(1, 3)]), (false, vec![1]));
    }

    #[test]
    fn rt_shadows_a_face_that_goes_leaves_the_still_set() {
        // a near one quickly: its shadow stays a few frames (its slot may
        // only have been handed out again)
        let (mut t, mut n) = (StillTracker::default(), 0);
        for _ in 0..2 * STILL_EVERY {
            frame(&mut t, &mut n, &[near(0, 1), near(1, 2)]);
        }
        assert_eq!(until_set(&mut t, &mut n, &[near(0, 1)], &[]), QUICK + 1);
        assert!(t.in_still(0) && !t.in_still(1));
        // and no more often than the sets may follow each other
        assert_eq!(until_set(&mut t, &mut n, &[], &[]), STILL_EVERY);
        assert!(!t.in_still(0));
        // a far one soon
        let (mut t, mut n) = (StillTracker::default(), 0);
        frame(&mut t, &mut n, &[near(0, 1), far(1, 2)]);
        assert_eq!(until_set(&mut t, &mut n, &[near(0, 1)], &[]), SOON + 1);
        assert!(t.in_still(0) && !t.in_still(1));
    }

    #[test]
    fn rt_shadows_a_gone_face_is_counted_once_by_its_cause() {
        let (mut t, mut n) = (StillTracker::default(), 0);
        frame(&mut t, &mut n, &[far(0, 1), far(1, 2)]);
        // not found by two looks: one count; a face no set holds: none
        t.absent(1, n + 1, SLOT_FREED);
        t.absent(1, n + 2, SLOT_FREED);
        t.absent(5, n + 1, NO_GEOMETRY);
        t.other(0, n + 1, 1, 0, NOT_TRACED, 0);
        t.other(0, n + 2, 1, 0, NOT_TRACED, 0);
        assert_eq!(t.why[SLOT_FREED..], [1, 0, 1, 0]);
        // face 1 is back as it was: it never left. The next set (made
        // without face 0) forgets them: gone again later counts again
        until_set(&mut t, &mut n, &[far(1, 2)], &[]);
        t.missing(1, NO_GEOMETRY);
        assert_eq!(t.why[SLOT_FREED..], [1, 1, 1, 0]);
    }

    #[test]
    fn rt_shadows_arrivals_wait_for_the_next_set_among_the_moving() {
        let (mut t, mut n) = (StillTracker::default(), 0);
        frame(&mut t, &mut n, &[near(0, 1)]);
        // face 1 arrives: traced at once, as a moving face, and in a set
        // later (nothing is wrong meanwhile)
        assert_eq!(until_set(&mut t, &mut n, &[near(0, 1), near(1, 2)], &[1]), LATER + 1);
        assert!(t.in_still(1));
        // many arrive: a set as soon as one may be made
        let many: Vec<F> = (0..MANY + 2).map(|id| near(id, 1 + id as u64)).collect();
        let (mut t, mut n) = (StillTracker::default(), 0);
        t.tracks.resize(many.len(), Track::default());
        frame(&mut t, &mut n, &many[..2]);
        let moving: Vec<usize> = (2..many.len()).collect();
        assert_eq!(until_set(&mut t, &mut n, &many, &moving), STILL_EVERY);
        assert!(t.in_still(MANY + 1));
    }

    #[test]
    fn rt_shadows_a_face_left_behind_leaves_the_set_without_being_traced() {
        let (mut t, mut n) = (StillTracker::default(), 0);
        frame(&mut t, &mut n, &[near(0, 1), far(1, 2)]);
        // face 1 is now too far: never among the moving ones, and out of
        // the set later
        assert_eq!(until_set(&mut t, &mut n, &[near(0, 1), F(1, 2, 0, false, false)], &[]), LATER + 1);
        assert!(t.in_still(0) && !t.in_still(1));
        // the same when the eye left it behind and nothing looked at it
        let (mut t, mut n) = (StillTracker::default(), 0);
        frame(&mut t, &mut n, &[near(0, 1), far(1, 2)]);
        n += 1;
        t.begin(8);
        t.weigh(1, n, false, false);
        assert!(!t.decide(n));
        for _ in 1..LATER {
            assert_eq!(quiet(&mut t, &mut n), (false, vec![]));
        }
        assert_eq!(quiet(&mut t, &mut n), (true, vec![]));
        assert!(t.in_still(0) && !t.in_still(1));
        // and it comes back as a moving face until a set takes it
        n += 1;
        t.begin(8);
        t.weigh(1, n, true, false);
        assert!(!t.decide(n));
        assert_eq!(t.moving(), [1]);
        // and after a shift of the world's origin every face is new
        t.rebase(n + 1);
        assert_eq!(frame(&mut t, &mut n, &[near(0, 9), near(1, 8)]), (true, vec![]));
    }

    #[test]
    fn rt_shadows_another_level_of_detail_keeps_the_face_in_the_still_set() {
        let (mut t, mut n) = (StillTracker::default(), 0);
        frame(&mut t, &mut n, &[near(0, 1), near(1, 2)]);
        // face 1 has another geometry at the same place: the level keeps
        // the one it has, the face is not traced twice, and a set comes soon
        assert_eq!(frame(&mut t, &mut n, &[near(0, 1), F(1, 2, 7, true, true)]), (false, vec![]));
        assert!(t.in_still(1));
        assert_eq!(until_set(&mut t, &mut n, &[near(0, 1), F(1, 2, 7, true, true)], &[]), SOON);
        // it never left: it need not rest again
        assert!(t.in_still(1));
        assert_eq!(t.tracks[1].relapses, 0);
    }

    #[test]
    fn rt_shadows_a_face_that_stops_being_traced_leaves_the_still_set() {
        let (mut t, mut n) = (StillTracker::default(), 0);
        frame(&mut t, &mut n, &[near(0, 1), near(1, 2)]);
        // face 1 turns into a face that is not traced (a blended one):
        // the next set, which comes quickly, is without it
        n += 1;
        t.begin(8);
        t.other(1, n, 2, 0, NOT_TRACED, 0);
        assert!(!t.decide(n));
        let mut frames = 1;
        while !quiet(&mut t, &mut n).0 {
            frames += 1;
            assert!(t.in_still(1) && frames < STILL_EVERY);
        }
        assert!(t.in_still(0) && !t.in_still(1));
        assert_eq!(quiet(&mut t, &mut n), (false, vec![]));
        // a skeleton's face is known by its skeleton, a placed one is not
        t.other(1, n, 2, 0, POSED, 7);
        assert_eq!((t.skeleton(1), t.skeleton(0)), (Some(7), None));
        assert!(t.is(1, 2, 0) && !t.is(1, 3, 0));
    }

    #[test]
    fn rt_shadows_a_face_that_blinks_makes_one_set() {
        let (mut t, mut n) = (StillTracker::default(), 0);
        frame(&mut t, &mut n, &[near(0, 1), near(1, 2)]);
        // face 1 is gone a few frames and back as it was, before any set
        // was made without it: it never left, and no set is made
        for _ in 0..QUICK - 1 {
            assert_eq!(frame(&mut t, &mut n, &[near(0, 1)]), (false, vec![]));
        }
        for _ in 0..2 * LATER {
            assert_eq!(frame(&mut t, &mut n, &[near(0, 1), near(1, 2)]), (false, vec![]));
        }
        assert!(t.in_still(1) && t.tracks[1].relapses == 0);
        // gone for longer: a set drops it
        until_set(&mut t, &mut n, &[near(0, 1)], &[]);
        assert!(!t.in_still(1));
        // it then blinks, a second there and a second not: traced with
        // the moving ones when there, and no set for it
        for _ in 0..8 {
            for _ in 0..60 {
                assert_eq!(frame(&mut t, &mut n, &[near(0, 1), near(1, 2)]), (false, vec![1]));
            }
            for _ in 0..60 {
                assert_eq!(frame(&mut t, &mut n, &[near(0, 1)]), (false, vec![]));
            }
        }
        // it stays: a set takes it back, after twice STILL_FRAMES at least
        let back = until_set(&mut t, &mut n, &[near(0, 1), near(1, 2)], &[1]);
        assert!(back >= 2 * STILL_FRAMES as u64, "back after {back} frames");
        assert!(t.in_still(1));
        // the same for one that is no longer traced (a blended face) and
        // then is again
        let (mut t, mut n) = (StillTracker::default(), 0);
        frame(&mut t, &mut n, &[near(0, 1), near(1, 2)]);
        n += 1;
        t.begin(8);
        t.other(1, n, 2, 9, NOT_TRACED, 0);
        assert!(!t.decide(n));
        while !quiet(&mut t, &mut n).0 {}
        assert_eq!(t.tracks[1].relapses, 1);
        assert_eq!(frame(&mut t, &mut n, &[near(0, 1), near(1, 2)]), (false, vec![1]));
        assert!(!t.tracks[1].rests(n + STILL_FRAMES as u64));
        // another face in its slot is a new one, still from the start
        let (mut t, mut n) = (StillTracker::default(), 0);
        frame(&mut t, &mut n, &[near(0, 1), near(1, 2)]);
        until_set(&mut t, &mut n, &[near(0, 1)], &[]);
        frame(&mut t, &mut n, &[near(0, 1), near(1, 5)]);
        assert!(t.tracks[1].relapses == 0 && t.tracks[1].rests(n));
    }

    #[test]
    fn rt_shadows_a_crowd_of_small_arrivals_waits_for_its_set() {
        let many: Vec<F> = (0..MANY + 2).map(|id| near(id, 1 + id as u64)).collect();
        let (mut t, mut n) = (StillTracker::default(), 0);
        t.tracks.resize(many.len(), Track::default());
        frame(&mut t, &mut n, &many[..2]);
        // one arrives: traced at once, small or not
        assert_eq!(frame(&mut t, &mut n, &many[..3]), (false, vec![2]));
        assert!(t.emits(2, true));
        // the others arrive: the small ones are not traced before their
        // set, the large ones are, and so is the one that already was
        assert!(!frame(&mut t, &mut n, &many).0);
        assert!(t.emits(2, true) && !t.emits(3, true) && t.emits(4, false));
        assert!(!t.emits(3, true) && t.emits(4, true));
        assert_eq!(
            until_set(&mut t, &mut n, &many, &(2..many.len()).collect::<Vec<_>>()),
            STILL_EVERY - 2
        );
        assert!(t.in_still(3) && !t.tracks[3].new);
        // one that moves is traced, whatever waits
        let mut moved = many.clone();
        moved[3] = far(3, 999);
        frame(&mut t, &mut n, &moved);
        assert!(t.emits(3, true));
    }

    /// rt_common.wgsl `hit_of`, for a merged cell whose table starts at
    /// `head`: the record of the face of triangle `primitive` of
    /// `geometry`, and the triangle's number in that face.
    fn table_hit(list: &[u32], head: usize, geometry: u32, primitive: u32) -> (u32, u32) {
        let opaque = list[head];
        let masked = geometry == 1 || opaque == 0;
        let base = head + 2 + if masked { 2 * opaque as usize } else { 0 };
        let (mut lo, mut hi) = (0, if masked { list[head + 1] } else { opaque } as usize);
        while hi > lo + 1 {
            let mid = (lo + hi) / 2;
            if list[base + 2 * mid] <= primitive {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (list[base + 2 * lo + 1], primitive - list[base + 2 * lo])
    }

    #[test]
    fn rt_shadows_a_cell_s_level_has_two_geometries_and_finds_its_faces() {
        let face = |record: u32, triangles: u32, opaque: bool| Posed {
            record,
            base_vertex: 0,
            first_index: 0,
            index_count: triangles * 3,
            opaque,
        };
        // the cells' faces come opaque first, in their records' order
        let mut smalls: Vec<Small> = [(7, 2, false), (3, 5, true), (9, 1, true), (4, 4, false), (8, 3, true)]
            .into_iter()
            .map(|(record, triangles, opaque)| Small {
                cell: 1,
                mark: record as u64,
                face: face(record, triangles, opaque),
            })
            .collect();
        let cells = cells_of(&mut smalls, &[]);
        let faces = &cells[0].faces;
        assert_eq!(faces.iter().map(|f| f.record).collect::<Vec<_>>(), [3, 8, 9, 4, 7]);
        // the opaque ones' geometry, then the masked ones'
        assert_eq!(cell_geometries(faces).collect::<Vec<_>>(), [(27, true), (18, false)]);
        let table = cell_table(faces);
        assert_eq!(table, [3, 2, 0, 3, 5, 8, 8, 9, 0, 4, 4, 7]);
        // every triangle finds its face, behind another cell's table
        let mut list = vec![1, 0, 0, 99];
        let head = list.len();
        list.extend(&table);
        let mut found = Vec::new();
        for (geometry, triangles) in [(0, 9), (1, 6)] {
            for primitive in 0..triangles {
                found.push(table_hit(&list, head, geometry, primitive));
            }
        }
        let expected: Vec<(u32, u32)> = faces.iter().flat_map(|f| (0..f.index_count / 3).map(|t| (f.record, t))).collect();
        assert_eq!(found, expected);
        assert_eq!(table_hit(&list, 0, 0, 0), (99, 0));
        // a cell of masked faces alone has one geometry, and of opaque
        // faces alone
        let masked = [face(5, 2, false), face(6, 1, false)];
        assert_eq!(cell_geometries(&masked).collect::<Vec<_>>(), [(9, false)]);
        let table = cell_table(&masked);
        assert_eq!((table_hit(&table, 0, 0, 1), table_hit(&table, 0, 0, 2)), ((5, 1), (6, 0)));
        let opaque = [face(5, 2, true), face(6, 1, true)];
        assert_eq!(cell_geometries(&opaque).collect::<Vec<_>>(), [(9, true)]);
        assert_eq!(table_hit(&cell_table(&opaque), 0, 0, 2), (6, 0));
    }

    #[test]
    fn rt_shadows_a_new_origin_moves_no_face() {
        let all = [near(0, 1), near(1, 2), near(2, 3)];
        let (mut t, mut n) = (StillTracker::default(), 0);
        frame(&mut t, &mut n, &all);
        // the origin changes: a set of the faces where they are
        t.rebase(n + 1);
        assert_eq!(frame(&mut t, &mut n, &all), (true, vec![]));
        // the scene writes face 0 at its new place, then face 1: arrivals,
        // still at once, that the next set takes; neither moved
        let first = [near(0, 11), near(1, 2), near(2, 3)];
        assert_eq!(frame(&mut t, &mut n, &first), (false, vec![0]));
        assert!(t.tracks[0].rests(n) && t.tracks[0].new && t.tracks[0].relapses == 0);
        let both = [near(0, 11), near(1, 12), near(2, 3)];
        assert_eq!(frame(&mut t, &mut n, &both), (false, vec![0, 1]));
        assert!(until_set(&mut t, &mut n, &both, &[0, 1]) <= STILL_EVERY);
        assert!(t.in_still(0) && t.in_still(1) && t.why[2] == 0);
        // a second change of place is a move
        let moved = [near(0, 21), near(1, 12), near(2, 3)];
        frame(&mut t, &mut n, &moved);
        assert!(!t.tracks[0].rests(n) && t.tracks[0].relapses == 1);
        // and so is the first, once the scene has had its time (face 2
        // stood where the new origin left it)
        for _ in 0..REBASE_FRAMES {
            quiet(&mut t, &mut n);
        }
        frame(&mut t, &mut n, &[near(0, 21), near(1, 12), near(2, 13)]);
        assert!(!t.tracks[2].rests(n) && t.tracks[2].relapses == 1);
        // a face that came after the origin changed moves at its first
        let (mut t, mut n) = (StillTracker::default(), 0);
        t.rebase(1);
        frame(&mut t, &mut n, &all[..2]);
        frame(&mut t, &mut n, &all);
        until_set(&mut t, &mut n, &all, &[2]);
        frame(&mut t, &mut n, &[near(0, 1), near(1, 2), near(2, 13)]);
        assert!(!t.tracks[2].rests(n));
    }

    #[test]
    fn rt_shadows_a_frame_builds_so_many_bottom_levels() {
        // the first always, however large
        assert!(build_room(0, 0, 4 * BUILD_TRIANGLES));
        // then within the triangles
        assert!(build_room(1, 100, BUILD_TRIANGLES - 100));
        assert!(!build_room(1, 100, BUILD_TRIANGLES));
        // and within the count, however small
        assert!(build_room(BUILD_LEVELS - 1, 1000, 12));
        assert!(!build_room(BUILD_LEVELS, 1000, 12));
    }

    #[test]
    fn rt_shadows_the_levels_a_frame_had_no_time_for_wait_in_their_order() {
        let key = |n: u32| Key {
            base_vertex: 0,
            first_index: n,
            index_count: 3 * n,
            opaque: true,
        };
        // two moving faces' levels, then three the still set's queue gave
        // (5 was the next, 8 is now)
        let mut queue = vec![key(9), key(8)];
        let mut new = vec![
            (key(1), (), false),
            (key(2), (), false),
            (key(5), (), true),
            (key(6), (), true),
            (key(7), (), true),
        ];
        let mut index: HashMap<Key, usize> = new.iter().enumerate().map(|(n, e)| (e.0, n)).collect();
        // the time for three: the two others are the next again, in order
        assert_eq!(unmade(&mut new, 3, &mut queue, &mut index), (13, 0));
        assert_eq!(queue, [key(9), key(8), key(7), key(6)]);
        assert_eq!(new.iter().map(|e| e.0).collect::<Vec<_>>(), [key(1), key(2), key(5)]);
        assert!(index.len() == 3 && index[&key(5)] == 2);
        // for one: a moving face's own waits for its face to ask again
        assert_eq!(unmade(&mut new, 1, &mut queue, &mut index), (7, 1));
        assert_eq!(queue, [key(9), key(8), key(7), key(6), key(5)]);
        assert!(index.len() == 1 && index[&key(1)] == 0);
        // for all: none
        assert_eq!(unmade(&mut new, 1, &mut queue, &mut index), (0, 0));
        assert_eq!((new.len(), queue.len()), (1, 5));
    }

    #[test]
    fn rt_shadows_an_object_finds_its_faces() {
        let mut by = ByObject::default();
        let of = |by: &ByObject, object: u32| by.faces(object).collect::<Vec<_>>();
        for (face, object) in [(0, 4), (1, 9), (2, 4), (5, 4)] {
            by.set(face, object);
        }
        assert_eq!(
            (of(&by, 4), of(&by, 9), of(&by, 3), of(&by, 99)),
            (vec![5, 2, 0], vec![1], vec![], vec![])
        );
        // the middle one, the first one, then the last one go
        by.set(2, 9);
        assert_eq!((of(&by, 4), of(&by, 9)), (vec![5, 0], vec![2, 1]));
        by.set(5, NO_OBJECT);
        by.set(0, NO_OBJECT);
        by.set(0, NO_OBJECT);
        assert_eq!((of(&by, 4), of(&by, 9)), (vec![], vec![2, 1]));
        by.set(0, 4);
        assert_eq!(of(&by, 4), vec![0]);
    }

    #[test]
    fn rt_shadows_an_object_s_change_is_followed_when_its_faces_read_it() {
        let placed = CullObject {
            sphere: [1.0, 2.0, 3.0, 4.0],
            flags: object_flags::ACTIVE,
            ..CullObject::NONE
        };
        let differs = |change: fn(&mut CullObject)| {
            let mut b = placed;
            change(&mut b);
            object_differs(&placed, &b)
        };
        assert!(differs(|o| o.flags |= object_flags::HIDDEN));
        assert!(differs(|o| o.flags = 0));
        assert!(differs(|o| o.sphere[0] = 9.0));
        assert!(differs(|o| o.anchor = 3));
        // what no look reads
        assert!(!differs(|o| o.flags |= object_flags::NO_PROBE | object_flags::PHYSICAL));
        assert!(!differs(|o| o.state = 5));
        // a posed object's sphere moves each frame
        let avatar = CullObject {
            flags: object_flags::ACTIVE | object_flags::AVATAR,
            ..placed
        };
        let mut moved = avatar;
        moved.sphere[0] = 9.0;
        assert!(!object_differs(&avatar, &moved));
    }

    #[test]
    fn rt_shadows_place_and_shape_tell_a_move_from_a_level_of_detail() {
        let mut m = [[0.0f32; 4]; 4];
        for (i, c) in m.iter_mut().enumerate() {
            c[i] = 1.0;
        }
        let mut face = CullFace::NONE;
        face.index_count = 36;
        let (place, shape) = (place_of(&m), shape_of(&face));
        assert_eq!((place, shape), (place_of(&m), shape_of(&face)));
        face.index_count = 72;
        assert_ne!(shape, shape_of(&face));
        m[3][2] = 0.001;
        assert_ne!(place, place_of(&m));
    }

    /// A small face of a set: its record, where its model's origin is, and
    /// its mark.
    fn small(record: u32, at: [f32; 3], mark: u64) -> Small {
        let mut model = glam::Mat4::IDENTITY.to_cols_array_2d();
        model[3] = [at[0], at[1], at[2], 1.0];
        Small {
            cell: cell_of(&model),
            mark,
            face: Posed {
                record,
                base_vertex: 0,
                first_index: record * 6,
                index_count: 6,
                opaque: true,
            },
        }
    }

    #[test]
    fn rt_shadows_small_faces_are_merged_by_cell() {
        // two cells, whatever the faces' order; one across the origin
        let mut faces = [
            small(4, [40.0, 3.0, 20.0], 1),
            small(1, [1.0, 2.0, 3.0], 1),
            small(7, [-1.0, 2.0, 3.0], 1),
            small(2, [15.0, 15.0, 15.0], 1),
        ];
        let cells = cells_of(&mut faces, &[]);
        let records = |c: &Cell| c.faces.iter().map(|f| f.record).collect::<Vec<_>>();
        assert_eq!(cells.len(), 3);
        assert!(cells.windows(2).all(|w| w[0].key < w[1].key), "by key");
        let of = |record: u32| cells.iter().find(|c| records(c).contains(&record)).expect("in a cell");
        assert_eq!(records(of(1)), [1, 2]);
        assert_eq!(of(1).vertices, 12);
        assert_eq!(records(of(4)), [4]);
        assert_eq!(records(of(7)), [7]);
        assert_eq!(cell_centre(of(1).key >> 16), glam::Vec3::splat(8.0));
        assert_eq!(cell_centre(of(7).key >> 16), glam::Vec3::new(-8.0, 8.0, 8.0));
        assert_eq!(cell_centre(of(4).key >> 16), glam::Vec3::new(40.0, 8.0, 24.0));
    }

    #[test]
    fn rt_shadows_a_cell_whose_faces_stand_keeps_its_level() {
        let mut first = [small(1, [1.0; 3], 1), small(2, [2.0; 3], 1), small(3, [40.0; 3], 1)];
        let before = cells_of(&mut first, &[]);
        // the same faces: the same lists, and versions
        let same = cells_of(&mut first, &before);
        assert_eq!(same, before);
        assert!(same.iter().zip(&before).all(|(a, b)| Arc::ptr_eq(&a.faces, &b.faces)));
        // a face of one cell moved within it, and one came into it: that
        // cell's version changes, the other's stands
        for mut changed in [
            vec![small(1, [1.0; 3], 1), small(2, [3.0; 3], 9), small(3, [40.0; 3], 1)],
            vec![
                small(1, [1.0; 3], 1),
                small(2, [2.0; 3], 1),
                small(5, [4.0; 3], 1),
                small(3, [40.0; 3], 1),
            ],
            vec![small(1, [1.0; 3], 1), small(3, [40.0; 3], 1)],
        ] {
            let after = cells_of(&mut changed, &before);
            assert_eq!(after.len(), 2);
            assert_eq!(after[0].key, before[0].key);
            assert_ne!(after[0].version, before[0].version);
            assert_eq!(after[1], before[1]);
            assert!(Arc::ptr_eq(&after[1].faces, &before[1].faces));
        }
    }

    #[test]
    fn rt_shadows_a_full_cell_is_split_into_parts() {
        let mut faces: Vec<Small> = (0..PART_FACES as u32 + 10).map(|r| small(r, [1.0; 3], 1)).collect();
        faces.push(small(1 << 20, [40.0; 3], 1));
        let cells = cells_of(&mut faces, &[]);
        assert_eq!(cells.iter().map(|c| c.faces.len()).collect::<Vec<_>>(), [PART_FACES, 10, 1]);
        assert_eq!(cells[0].key + 1, cells[1].key, "the next part of the same cell");
        assert_eq!(cells[2].key & 0xffff, 0, "another cell starts at its first part");
        // by vertices too
        let mut large: Vec<Small> = (0..3)
            .map(|r| {
                let mut s = small(r, [1.0; 3], 1);
                s.face.index_count = PART_VERTICES / 2 + 3;
                s
            })
            .collect();
        assert_eq!(cells_of(&mut large, &[]).len(), 3);
    }

    #[test]
    fn rt_shadows_a_merged_face_is_weighed_in_steps() {
        assert_eq!(size_step(CELL - 0.1), 0.0);
        assert_eq!(size_step(16.0), 16.0);
        assert_eq!(size_step(31.9), 16.0);
        assert_eq!(size_step(32.0), 32.0);
        assert_eq!(size_step(200.0), 128.0);
        assert_eq!(size_step(-5.0), 0.0);
    }

    #[test]
    fn rt_shadows_a_top_level_far_too_large_is_made_again() {
        assert!(fits(64, 64) && fits(128, 64));
        // too small, and four times too large (after an arrival's burst)
        assert!(!fits(64, 128));
        assert!(!fits(256, 64) && !fits(65536, 1024));
    }

    #[test]
    fn rt_shadows_the_variables_force_the_options_and_leave_the_others_to_the_settings() {
        use crate::RayTracing;
        let all = RayTracing {
            shadows: true,
            lights: true,
            indirect: true,
            reflections: true,
        };
        let sun = RayTracing {
            shadows: true,
            ..Default::default()
        };
        // no variable: the settings, and nothing without the sun's shadows
        let none = Override::parse(None, None, None, None);
        assert_eq!(none, Override::default());
        assert_eq!(none.applied(all), all);
        assert_eq!(none.applied(sun), sun);
        assert_eq!(none.applied(RayTracing { shadows: false, ..all }), RayTracing::default());
        // empty is unset
        assert_eq!(Override::parse(Some(""), Some(" "), None, None), none);
        // forced on, over settings that are off; the others stay theirs
        let on = Override::parse(Some("1"), Some("1"), None, None);
        assert_eq!(on.applied(RayTracing::default()), RayTracing { lights: true, ..sun });
        assert_eq!(on.applied(all), all);
        // forced off, over settings that are on
        assert_eq!(Override::parse(Some("0"), None, None, None).applied(all), RayTracing::default());
        assert_eq!(
            Override::parse(Some("off"), Some("1"), Some("1"), Some("1")).applied(all),
            RayTracing::default()
        );
        assert_eq!(Override::parse(None, Some("0"), Some("off"), Some("0")).applied(all), sun);
        // the culling comes with the sun's shadows
        assert_eq!(Override::parse(Some("1"), None, None, None).cull, Cull::None);
        let front = Override::parse(Some("front"), None, None, None);
        assert_eq!((front.cull, front.applied(RayTracing::default())), (Cull::Front, sun));
        assert_eq!(Override::parse(Some(" back "), None, None, None).cull, Cull::Back);
    }

    #[test]
    fn rt_shadows_denoiser_copies_a_calm_tile_once_to_each_image() {
        // two images in turn: the variance pass and the first filter pass
        // fill one each, the others find the tiles there
        let passes = |kept: bool| (0..=ATROUS_PASSES).map(|k| calm_held(k, kept).0).collect::<Vec<_>>();
        assert_eq!(passes(false), [false, false, true, true, true]);
        // the first filter pass's image is kept apart: the second of the
        // two is filled a pass later
        assert_eq!(passes(true), [false, false, true, false, true]);
        // the tint is not written by the variance pass
        let tint = (0..=ATROUS_PASSES).map(|k| calm_held(k, false).1).collect::<Vec<_>>();
        assert_eq!(tint, [false, false, false, true, true]);
    }

    #[test]
    fn rt_shadows_a_light_says_how_far_it_moved() {
        let lamp = |id: u32, x: f32| crate::types::PointLight {
            position: glam::Vec3::new(x, 0.0, 2.0),
            radius: 10.0,
            color: glam::Vec3::ONE,
            falloff: 0.5,
            projector: None,
            source_radius: 0.2,
            source_id: id,
        };
        let mut places = HashMap::new();
        // a light seen for the first time has not moved
        let first = traced_lights(&[lamp(7, 1.0), lamp(0, 5.0)], &mut places);
        assert_eq!([first[0].source[2], first[1].source[2]], [0.0, 0.0]);
        // the list's order changes with the camera: a light is known by
        // what it shines from, and one of no prim never moves
        let next = traced_lights(&[lamp(0, 6.0), lamp(7, 1.25)], &mut places);
        assert_eq!([next[0].source[2], next[1].source[2]], [0.0, 0.25]);
        // a light that left the list starts again when it comes back
        traced_lights(&[lamp(0, 6.0)], &mut places);
        let back = traced_lights(&[lamp(7, 4.0)], &mut places);
        assert_eq!(back[0].source[2], 0.0);
    }

    #[test]
    fn rt_shadows_row_major_keeps_the_translation_last() {
        let m = glam::Mat4::from_scale_rotation_translation(
            glam::Vec3::new(2.0, 3.0, 4.0),
            glam::Quat::from_rotation_z(0.5),
            glam::Vec3::new(10.0, 20.0, 30.0),
        );
        let r = row_major(&m.to_cols_array_2d());
        let p = glam::Vec3::new(0.3, -1.2, 2.5);
        let want = m.transform_point3(p);
        let got = glam::Vec3::new(
            r[0] * p.x + r[1] * p.y + r[2] * p.z + r[3],
            r[4] * p.x + r[5] * p.y + r[6] * p.z + r[7],
            r[8] * p.x + r[9] * p.y + r[10] * p.z + r[11],
        );
        assert!(got.distance(want) < 1e-4, "{got} != {want}");
    }
}

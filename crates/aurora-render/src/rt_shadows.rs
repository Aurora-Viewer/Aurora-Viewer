//! Ray-traced sun shadows of the main view, an experiment behind
//! `AURORA_RT_SHADOWS` (off by default; Vulkan devices with ray queries).
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

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use bytemuck::{Pod, Zeroable};

use crate::arena::RecordStore;
use crate::gpu_cull::{CullFace, CullObject, CullTables, NO_OBJECT, object_flags, pass};
use crate::pipeline_cache::CachedPipelines;
use crate::types::{DrawRecord, flags};

/// The device feature the experiment needs.
pub const FEATURES: wgpu::Features = wgpu::Features::EXPERIMENTAL_RAY_QUERY;

/// Which faces a ray passes (`AURORA_RT_SHADOWS`): `1` none, `front` those
/// it meets from their front, `back` those it meets from behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cull {
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

/// The experiment's mode, read once: None when `AURORA_RT_SHADOWS` is unset,
/// empty or `0`.
pub fn mode() -> Option<Cull> {
    static MODE: OnceLock<Option<Cull>> = OnceLock::new();
    *MODE.get_or_init(|| parse(std::env::var("AURORA_RT_SHADOWS").ok().as_deref()))
}

fn parse(value: Option<&str>) -> Option<Cull> {
    match value?.trim() {
        "" | "0" | "off" => None,
        "front" => Some(Cull::Front),
        "back" => Some(Cull::Back),
        _ => Some(Cull::None),
    }
}

/// Whether the traced reflections are on too (`AURORA_RT_REFLECTIONS`, with
/// `AURORA_RT_SHADOWS`): they take SSR's place where SSR is on.
pub fn reflections() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| mode().is_some() && std::env::var("AURORA_RT_REFLECTIONS").is_ok_and(|v| !matches!(v.trim(), "" | "0" | "off")))
}

/// Whether the traced indirect light is on too (`AURORA_RT_GI`, with
/// `AURORA_RT_SHADOWS`): it takes the screen-space indirect light's place.
pub fn gi() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| mode().is_some() && std::env::var("AURORA_RT_GI").is_ok_and(|v| !matches!(v.trim(), "" | "0" | "off")))
}

/// Whether the local point lights' shadows are traced too
/// (`AURORA_RT_LIGHTS`, with `AURORA_RT_SHADOWS`).
pub fn local_lights() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| mode().is_some() && std::env::var("AURORA_RT_LIGHTS").is_ok_and(|v| !matches!(v.trim(), "" | "0" | "off")))
}

/// Whether to ask a device for [`FEATURES`]: the experiment is on and the
/// adapter has them.
pub fn wanted(adapter: wgpu::Features) -> bool {
    mode().is_some() && adapter.contains(FEATURES)
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
    /// had moved, had another geometry, were to come in and to go out.
    pub why: [u64; 6],
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
/// moved or went: wgpu builds the level from scratch, a few milliseconds
/// for a region's faces.
const STILL_EVERY: u64 = 15;
/// Faces whose change waits for a still set that make one as soon as
/// [`STILL_EVERY`] allows (a region that loads, an arrival by teleport).
const MANY: usize = 256;
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

/// Frames between two full looks at a face that keeps its place and its
/// geometry (whether it is one to trace, how far and how large): in
/// between, only those are compared. A face that the eye left behind, or
/// that was hidden, is noticed this late at most.
const RECHECK: u64 = 8;

/// A placed face as the main thread last saw it.
#[derive(Clone, Copy, Default)]
struct Track {
    /// The frame it was last seen (0: never).
    seen: u64,
    /// Its place, and its geometry, hashed (`place_of`, `shape_of`).
    place: u64,
    shape: u32,
    /// Frames it kept its place.
    still: u16,
    /// What the last full look found it to be.
    kind: u8,
    /// Times it moved out of a still set: each doubles the frames it must
    /// keep its place before the next one takes it back.
    relapses: u8,
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
}

/// A face's kinds ([`Track::kind`]): not traced, a placed face (opaque or
/// alpha-masked), a skeleton's.
const NOT_TRACED: u8 = 0;
const PLACED_OPAQUE: u8 = 1;
const PLACED_MASKED: u8 = 2;
const POSED: u8 = 3;

impl Track {
    /// Whether it kept its place long enough to count as still.
    fn rests(&self) -> bool {
        self.still as u32 >= (STILL_FRAMES as u32) << self.relapses.min(RELAPSES_MAX)
    }

    fn placed(&self) -> bool {
        matches!(self.kind, PLACED_OPAQUE | PLACED_MASKED)
    }
}

/// What a frame's faces add up to, for [`StillTracker::decide`].
#[derive(Default)]
struct Tally {
    /// Faces of the still set this frame saw, and the near ones of them.
    found: usize,
    found_near: usize,
    /// Faces the set holds where they no longer are, and with a geometry
    /// they no longer have: what it shows wrong.
    left: usize,
    stale: usize,
    /// Faces the set should hold and does not, and holds and should not.
    arrivals: usize,
    leavers: usize,
    /// A near face of the set moved.
    urgent: bool,
}

impl Tally {
    /// Counts placed face `t`, seen this frame: whether it is among the
    /// frame's moving faces (unless the frame's set takes it).
    fn count(&mut self, t: &Track) -> bool {
        self.found += t.in_still as usize;
        self.found_near += t.set_near as usize;
        let would = t.kept && t.rests();
        if t.left {
            self.left += 1;
        } else if t.stale {
            self.stale += 1;
        } else if would != t.in_still {
            *if would { &mut self.arrivals } else { &mut self.leavers } += 1;
        }
        t.kept && (!t.in_still || t.left)
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
/// A near face of the level that moves or goes makes a new set at once
/// (the level would cast its shadow where it no longer is). The other
/// changes are gathered ([`MANY`], [`SOON`], [`LATER`]): a face that
/// arrives or comes to rest is among the moving ones meanwhile, so that
/// none is ever in neither; a far one that moved is in both, a far one
/// that went still in the level; one whose level of detail changed keeps
/// the level's geometry.
#[derive(Default)]
pub(crate) struct StillTracker {
    tracks: Vec<Track>,
    tally: Tally,
    /// Faces of the last still set, and the near ones of them.
    held: usize,
    held_near: usize,
    /// The frame of the last still set (0: none yet), and the first frame
    /// since with a change that waits for the next (0: none).
    set_at: u64,
    changed_at: u64,
    /// Why the sets were made, added up since the start ([`Faces::why`]).
    why: [u64; 6],
}

impl StillTracker {
    /// Forgets every face: after the world's origin moved, they all stand
    /// elsewhere at once and none of them moved.
    fn reset(&mut self) {
        *self = Self {
            why: self.why,
            ..Self::default()
        };
    }

    fn begin(&mut self, faces: usize) {
        if self.tracks.len() < faces {
            self.tracks.resize(faces, Track::default());
        }
        self.tally = Tally::default();
    }

    /// Whether face `id` was traced last frame (for [`KEEP`]).
    fn kept(&self, id: usize, frame: u64) -> bool {
        self.tracks.get(id).is_some_and(|t| t.kept && t.seen + 1 == frame)
    }

    /// Face `id` on `frame` when it is as the last frame left it (at
    /// `place` and of `shape` still, and no skeleton's), with no full
    /// look: whether it is among this frame's moving faces. None when it
    /// has to be looked at.
    fn same(&mut self, id: usize, frame: u64, place: u64, shape: u32) -> Option<bool> {
        let t = self.tracks.get_mut(id)?;
        if t.seen == 0 || t.seen + 1 != frame || t.place != place || t.shape != shape || t.kind == POSED {
            return None;
        }
        t.seen = frame;
        if t.kind == NOT_TRACED {
            return Some(false);
        }
        t.still = t.still.saturating_add(1);
        Some(self.tally.count(t))
    }

    /// Notes face `id` on `frame` as one that is not a placed face to
    /// trace (`kind`: not traced, or a skeleton's).
    fn other(&mut self, id: usize, frame: u64, place: u64, shape: u32, kind: u8) {
        if let Some(t) = self.tracks.get_mut(id) {
            // one the still set holds stays so until the set made for its
            // leaving (`decide`: this frame did not find it)
            *t = Track {
                seen: frame,
                place,
                shape,
                kind,
                in_still: t.in_still,
                set_near: t.set_near,
                ..Track::default()
            };
        }
    }

    /// Notes placed face `id` on `frame`, at `place` and of `shape`,
    /// opaque or not, traced or not (`kept`), near or not: whether it is
    /// among this frame's moving faces (unless the frame's set takes it).
    #[allow(clippy::too_many_arguments)]
    fn visit(&mut self, id: usize, frame: u64, place: u64, shape: u32, kept: bool, near: bool, opaque: bool) -> bool {
        let Some(t) = self.tracks.get_mut(id) else {
            return false;
        };
        let kind = if opaque { PLACED_OPAQUE } else { PLACED_MASKED };
        if t.seen == 0 || t.seen + 1 != frame || !t.placed() {
            // a new face (or a slot handed out again): still from the
            // start. What the set held in its slot is gone: this frame
            // does not find it (`decide`).
            *t = Track {
                place,
                shape,
                still: STILL_FRAMES,
                kind,
                ..Track::default()
            };
        } else if t.place != place {
            // it moved
            t.place = place;
            t.shape = shape;
            t.kind = kind;
            t.still = 0;
            if t.in_still {
                if !t.left {
                    t.relapses = t.relapses.saturating_add(1);
                }
                t.left = true;
                t.stale = false;
                self.tally.urgent |= t.near || near;
            }
        } else if t.shape != shape || t.kind != kind {
            // another geometry at the same place: a level of detail
            t.shape = shape;
            t.kind = kind;
            t.stale = t.in_still && !t.left;
            t.still = t.still.saturating_add(1);
        } else {
            t.still = t.still.saturating_add(1);
        }
        t.seen = frame;
        t.kept = kept;
        t.near = near;
        self.tally.count(t)
    }

    /// Whether this frame makes a new still set, after every face was
    /// visited; the tracks then say which faces it holds.
    fn decide(&mut self, frame: u64) -> bool {
        let tally = &self.tally;
        let gone = self.held - tally.found.min(self.held);
        let urgent = tally.urgent || tally.found_near < self.held_near;
        let wrong = tally.left + tally.stale + gone;
        let changes = wrong + tally.arrivals + tally.leavers;
        if changes == 0 && !urgent {
            self.changed_at = 0;
            return false;
        }
        if self.changed_at == 0 {
            self.changed_at = frame;
        }
        let patience = if wrong > 0 { SOON } else { LATER };
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
            t.in_still = t.seen == frame && t.kept && t.rests() && t.placed();
            t.set_near = t.in_still && t.near;
            t.left = false;
            t.stale = false;
            self.held += t.in_still as usize;
            self.held_near += t.set_near as usize;
        }
        self.set_at = frame;
        self.changed_at = 0;
        true
    }

    /// Whether the still set holds face `id`, and where it still is.
    fn in_still(&self, id: usize) -> bool {
        self.tracks.get(id).is_some_and(|t| t.in_still)
    }

    /// Whether face `id` is traced with the moving ones on a frame it was
    /// noted as one: the still set does not hold it, or not where it is.
    fn moving(&self, id: usize) -> bool {
        self.tracks.get(id).is_some_and(|t| !t.in_still || t.left)
    }

    /// Face `id`'s place and geometry in one number, for its cell's version.
    fn mark(&self, id: usize) -> u64 {
        self.tracks
            .get(id)
            .map_or(0, |t| t.place ^ ((t.shape as u64) << 32 | t.kind as u64))
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
            _ => Arc::new(small[start..end].iter().map(|s| s.face).collect()),
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

/// What the main thread remembers of each skeleton's last built pose, so a
/// skeleton whose vertices moved less than a shadow ray's start can tell
/// keeps its bottom level (wgpu has no refit: a level is only built again
/// from scratch): the nearest skeletons first, within a frame's budget.
#[derive(Default)]
pub(crate) struct PoseTracker {
    seen: HashMap<u32, Seen>,
    frame: u64,
    scratch: Vec<Candidate>,
    /// The placed faces, still or moving, and the last still set with its
    /// number.
    placed: StillTracker,
    still: Arc<Vec<Instance>>,
    cells: Arc<Vec<Cell>>,
    small: Vec<Small>,
    set: u64,
}

struct Seen {
    faces: u64,
    /// A level was built for these faces (one left waiting by the budget
    /// has none yet).
    built: bool,
    joints: Vec<[[f32; 4]; 4]>,
    models: Vec<[[f32; 4]; 4]>,
    last: u64,
}

struct Candidate {
    key: u32,
    face: Posed,
    model: [[f32; 4]; 4],
    origin: [f32; 3],
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
    let worn_or_posed = object_flags::AVATAR | object_flags::RIGGED | object_flags::POSED;
    object.flags & worn_or_posed != 0 || object.anchor != 0 || rf & (flags::SKINNED | flags::ANCHORED | flags::LIP_SYNC) != 0
}

/// The faces to trace this frame, from the records and the culling tables
/// the main thread keeps: the opaque and alpha-masked faces (blended ones
/// are not traced yet) of the terrain and of the objects that are no HUD;
/// those of a skeleton are posed, the others placed by their record.
/// `palettes` is what the GPU's joint palettes hold, `camera` the eye, `far`
/// the distance the shadows cover: a placed face further than that (by
/// [`FAR_MARGIN`]) or too small to cast a shadow that shows ([`SMALL`]) is
/// left out. `shifted` says the world's origin moved since the last frame.
///
/// Every frame compares each face's place and geometry with the last
/// frame's; the rest is looked at when they changed, and every
/// [`RECHECK`] frames (a region has a hundred thousand faces, and this runs
/// on the main thread).
#[allow(clippy::too_many_arguments)]
pub(crate) fn gather(
    records: &RecordStore,
    cull: &CullTables,
    palettes: &[[[f32; 4]; 4]],
    camera: glam::Vec3,
    far: f32,
    shifted: bool,
    tracker: &mut PoseTracker,
    out: &mut Faces,
) {
    out.moving.clear();
    out.posed.clear();
    out.groups.clear();
    tracker.frame += 1;
    tracker.scratch.clear();
    let frame = tracker.frame;
    let far = far.max(16.0) * FAR_MARGIN;
    if shifted {
        tracker.placed.reset();
        tracker.still = Arc::default();
        tracker.cells = Arc::default();
        tracker.set += 1;
    }
    // a top level's custom index holds 24 bits, one of them the merged
    // cells' (rt_common.wgsl `hit_record`)
    let faces = records.faces.len().min(1 << 23);
    tracker.placed.begin(faces);
    for id in 0..faces {
        let Some(face) = records.faces.get(id) else {
            continue;
        };
        if face.index_count < 3 || face.base_vertex < 0 {
            continue;
        }
        let Some(record) = records.get(id as u32) else {
            continue;
        };
        let (place, shape) = (place_of(&record.model), shape_of(face));
        if !(id as u64 + frame).is_multiple_of(RECHECK)
            && let Some(moving) = tracker.placed.same(id, frame, place, shape)
        {
            if moving {
                out.moving.push(instance_of(id, face, record));
            }
            continue;
        }
        let rf = record.flags[0];
        let object = match face.object {
            NO_OBJECT => None,
            object => cull.objects.get(object as usize),
        };
        let traced = match object {
            None => face.object == NO_OBJECT && rf & flags::TERRAIN != 0,
            Some(object) => {
                object.flags & object_flags::ACTIVE != 0
                    && object.flags & object_flags::HUD == 0
                    && matches!(face.bits & pass::BITS, pass::OPAQUE | pass::OPAQUE_2S | pass::MASK | pass::MASK_2S)
                    && rf & flags::ALPHA_BLEND == 0
            }
        };
        if !traced {
            tracker.placed.other(id, frame, place, shape, NOT_TRACED);
            continue;
        }
        if object.is_some_and(|o| is_posed(o, rf)) {
            tracker.placed.other(id, frame, place, shape, POSED);
            let key = record.flags[1];
            let palette = |i: u32| palettes.get(i as usize);
            // where the face stands, as rt_pose.wgsl places it
            let origin = match (rf & flags::SKINNED != 0, rf & flags::ANCHORED != 0) {
                (true, true) => palette(key + crate::types::SKELETON_ANCHOR).map(|m| m[3]),
                (false, true) => palette(key).map(|m| mul(m, &record.model)[3]),
                _ => Some(record.model[3]),
            }
            .unwrap_or(record.model[3]);
            tracker.scratch.push(Candidate {
                key,
                face: Posed {
                    record: id as u32,
                    base_vertex: face.base_vertex as u32,
                    first_index: face.first_index,
                    index_count: face.index_count - face.index_count % 3,
                    opaque: rf & flags::ALPHA_MASK == 0,
                },
                model: record.model,
                origin: [origin[0], origin[1], origin[2]],
            });
            continue;
        }
        // a placed face: the terrain always, an object's by its sphere
        let (kept, near) = object.map_or((true, true), |o| {
            let ease = if tracker.placed.kept(id, frame) { KEEP } else { 1.0 };
            let distance = glam::Vec3::new(o.sphere[0], o.sphere[1], o.sphere[2]).distance(camera);
            let radius = o.sphere[3];
            let weighed = if face.index_count / 3 <= MERGED_TRIANGLES {
                // (from the cell's nearest corner: no further than the face)
                size_step(cell_centre(cell_of(&record.model)).distance(camera) - CELL * 0.87)
            } else {
                distance
            };
            (
                distance - radius < far * ease && radius * ease >= SMALL * weighed,
                distance - radius < NEAR || radius >= NEAR_SIZE * distance,
            )
        });
        if tracker
            .placed
            .visit(id, frame, place, shape, kept, near, rf & flags::ALPHA_MASK == 0)
        {
            out.moving.push(instance_of(id, face, record));
        }
    }
    // the still set, on the frames it changes; the faces it holds are then
    // no longer among the moving ones
    if tracker.placed.decide(frame) {
        let placed = &tracker.placed;
        let mut large = Vec::new();
        tracker.small.clear();
        for id in (0..faces).filter(|&id| placed.in_still(id)) {
            let (Some(face), Some(record)) = (records.faces.get(id), records.get(id as u32)) else {
                continue;
            };
            let i = instance_of(id, face, record);
            if i.index_count / 3 > MERGED_TRIANGLES {
                large.push(i);
                continue;
            }
            tracker.small.push(Small {
                cell: cell_of(&record.model),
                mark: placed.mark(id),
                face: Posed {
                    record: i.record,
                    base_vertex: i.base_vertex as u32,
                    first_index: i.first_index,
                    index_count: i.index_count,
                    opaque: i.opaque,
                },
            });
        }
        tracker.still = Arc::new(large);
        tracker.cells = Arc::new(cells_of(&mut tracker.small, &tracker.cells));
        tracker.set += 1;
    }
    out.still = tracker.still.clone();
    out.cells = tracker.cells.clone();
    out.set = tracker.set;
    out.eye = camera.to_array();
    out.why = tracker.placed.why;
    let placed = &tracker.placed;
    out.moving.retain(|i| placed.moving(i.record as usize));
    choose_rebuilds(palettes, camera, tracker, out);
}

/// Groups the posed candidates by skeleton and picks the skeletons built
/// again this frame: new ones, those whose faces changed, and those whose
/// vertices drifted past what a shadow shows, nearest first within
/// [`REBUILD_TRIANGLES`].
fn choose_rebuilds(palettes: &[[[f32; 4]; 4]], camera: glam::Vec3, tracker: &mut PoseTracker, out: &mut Faces) {
    use std::hash::{Hash, Hasher};
    let frame = tracker.frame;
    let stride = crate::types::PALETTE_STRIDE as usize;
    tracker.scratch.sort_by_key(|c| c.key);
    // (start, end, distance, must, faces' hash) of each skeleton's run
    let mut runs: Vec<(usize, usize, f32, bool, u64)> = Vec::new();
    let mut start = 0;
    while start < tracker.scratch.len() {
        let key = tracker.scratch[start].key;
        let mut end = start;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        while end < tracker.scratch.len() && tracker.scratch[end].key == key {
            let f = tracker.scratch[end].face;
            (f.record, f.base_vertex, f.first_index, f.index_count, f.opaque).hash(&mut hasher);
            end += 1;
        }
        let faces = hasher.finish();
        let run = &tracker.scratch[start..end];
        let distance = glam::Vec3::from(run[0].origin).distance(camera);
        let joints = palettes
            .get(key as usize..(key as usize + stride).min(palettes.len()))
            .unwrap_or(&[]);
        let must = match tracker.seen.get_mut(&key) {
            Some(seen) if seen.built && seen.faces == faces && seen.joints.len() == joints.len() => {
                seen.last = frame;
                // (a skeleton that moves is past the limit at its first
                // joints: most are not read)
                let allowed = drift_allowed(distance);
                seen.joints.iter().zip(joints).any(|(a, b)| drift(a, b) > allowed)
                    || seen.models.iter().zip(run).any(|(a, c)| drift(a, &c.model) > allowed)
            }
            Some(seen) if seen.faces == faces => {
                seen.last = frame;
                true
            }
            _ => {
                tracker.seen.insert(
                    key,
                    Seen {
                        faces,
                        built: false,
                        joints: Vec::new(),
                        models: Vec::new(),
                        last: frame,
                    },
                );
                true
            }
        };
        runs.push((start, end, distance, must, faces));
        start = end;
    }
    tracker.seen.retain(|_, s| s.last == frame);
    // the skeletons that must be built again, nearest first, within budget
    let mut order: Vec<usize> = (0..runs.len()).filter(|&r| runs[r].3).collect();
    order.sort_by(|&a, &b| runs[a].2.total_cmp(&runs[b].2));
    let mut chosen = vec![false; runs.len()];
    let mut triangles = 0u64;
    for r in order {
        let (s, e, _, _, _) = runs[r];
        let tris: u64 = tracker.scratch[s..e].iter().map(|c| c.face.index_count as u64 / 3).sum();
        if triangles > 0 && triangles + tris > REBUILD_TRIANGLES {
            continue;
        }
        triangles += tris;
        chosen[r] = true;
    }
    for (r, &(s, e, _, _, faces)) in runs.iter().enumerate() {
        let run = &tracker.scratch[s..e];
        let key = run[0].key;
        let vertices: u32 = run.iter().map(|c| c.face.index_count).sum();
        let first = out.posed.len() as u32;
        if chosen[r] {
            out.posed.extend(run.iter().map(|c| c.face));
            if let Some(seen) = tracker.seen.get_mut(&key) {
                seen.built = true;
                seen.joints.clear();
                seen.joints.extend_from_slice(
                    palettes
                        .get(key as usize..(key as usize + stride).min(palettes.len()))
                        .unwrap_or(&[]),
                );
                seen.models.clear();
                seen.models.extend(run.iter().map(|c| c.model));
            }
        }
        out.groups.push(PosedGroup {
            key,
            first,
            count: if chosen[r] { run.len() as u32 } else { 0 },
            vertices,
            faces,
            rebuild: chosen[r],
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
/// Posed vertices of the merged cells whose levels are made in one frame,
/// the nearest first (the nearest is always made): the others keep the
/// level of an earlier set, or wait for theirs.
const MERGED_VERTICES: u64 = 3 << 17;
/// The bit of a top level's custom index that says a merged cell's level:
/// the rest is where its faces' records start (rt_common.wgsl `hit_record`).
const MERGED: u32 = 1 << 23;
/// Frames a bottom level no face used is kept.
const KEEP_FRAMES: u64 = 600;
/// Frames between two sweeps of the bottom levels no face uses.
const SWEEP_EVERY: u64 = 64;
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

/// A merged cell's bottom level: a geometry per face, in world space.
struct Merged {
    blas: wgpu::Blas,
    /// The version of the cell it was made for (`Cell::version`).
    version: u64,
    records: Vec<u32>,
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
    lights: wgpu::Buffer,
    denoise: Denoise,
    /// Last frame's lights: the same list keeps last frame's choices.
    listed: Vec<TracedLightU>,
    timer: Option<Timer>,
    last: usize,
    history_for: Option<(u32, u32)>,
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
    pub result: wgpu::TextureView,
}

const GI_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
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
    /// The size they were made for.
    params_for: Option<(u32, u32)>,
    /// The variance under which a pixel is left as it is.
    floor: f32,
}
/// How far an indirect ray goes (m): the light bounced from further off
/// is faint, and shorter rays are quicker.
const GI_REACH: f32 = 64.0;

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
}

/// The traced indirect light's passes (shaders/rt_gi.wgsl).
struct Gi {
    trace_layout: wgpu::BindGroupLayout,
    filter_layout: wgpu::BindGroupLayout,
    trace: wgpu::ComputePipeline,
    filter: wgpu::ComputePipeline,
    buffer: wgpu::Buffer,
    denoise: Denoise,
    timer: Option<Timer>,
    /// The history written last frame (`GiTargets::color` and `guide`).
    last: usize,
    /// The size the history was written at, None before the first frame.
    history_for: Option<(u32, u32)>,
    prev_clip_from_world: glam::Mat4,
    prev_camera: glam::Vec3,
    frame: u32,
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
    /// `Faces::why` at the last log.
    why: [u64; 6],
    frame: u64,
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
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
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
                    size: count as u64 * 8,
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
    /// None when the experiment is off or the device has no ray queries.
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue, textures: &wgpu::BindGroupLayout) -> Option<Self> {
        let cull = mode()?;
        if !device.features().contains(FEATURES) {
            log::warn!("AURORA_RT_SHADOWS: the device has no ray queries, the shadow maps stay");
            return None;
        }
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
        log::info!("AURORA_RT_SHADOWS: ray-traced sun shadows on ({cull:?} faces culled)");
        Some(Self {
            layout,
            pipeline,
            buffer,
            cull,
            bottoms: HashMap::new(),
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
            why: [0; 6],
            frame: 0,
            stats: RtStats::default(),
            timer: Timer::new(device, queue, 4),
            poser: Poser::new(device, &scene_layout),
            reflect: reflections().then(|| Reflect::new(device, queue, &scene_layout, textures)),
            gi: gi().then(|| Gi::new(device, queue, &scene_layout, textures)),
            lights: Some(Lights::new(device, queue, &scene_layout, textures)),
            posed_live: false,
            scene_layout,
            warned: false,
        })
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
        if self.tlas.as_ref().is_none_or(|t| t.get().len() < wanted) {
            self.tlas = Some(device.create_tlas(&wgpu::CreateTlasDescriptor {
                label: Some("rt shadows skeletons"),
                max_instances: wanted as u32,
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            }));
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
            // every slot: one past this frame's would keep an earlier
            // frame's instance
            let slots = tlas.get().len();
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
    /// The mask, when the device traces (AURORA_RT_SHADOWS and ray queries).
    pub(crate) fn new(device: &wgpu::Device, width: u32, height: u32) -> Option<Self> {
        if mode().is_none() || !device.features().contains(FEATURES) {
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
        let reflect = reflections().then(|| {
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
        let gi = gi().then(|| {
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
                result: view("rt gi", size, GI_FORMAT),
            }
        });
        let lights = Some({
            let usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
            let view = |label: &'static str, format: wgpu::TextureFormat| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width: size.0,
                            height: size.1,
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
            LightTargets {
                choice: [view("rt light choices", CHOICE_FORMAT), view("rt light choices", CHOICE_FORMAT)],
                share: [view("rt light share", SHARE_FORMAT), view("rt light share", SHARE_FORMAT)],
                guide: [view("rt light guide", GUIDE_FORMAT), view("rt light guide", GUIDE_FORMAT)],
                ping: [view("rt light denoise", DENOISE_FORMAT), view("rt light denoise", DENOISE_FORMAT)],
                mid: view("rt light samples", MID_FORMAT),
                sun: view("rt sun mask", RESULT_FORMAT),
            }
        });
        Some(Targets {
            size,
            result,
            reflect,
            gi,
            lights,
        })
    }
}

impl RtShadows {
    /// The constants of this frame's pass: the projection the depth prepass
    /// was drawn with, the eye, the light (w: whether it is up), and whether
    /// the contact mask is read.
    pub(crate) fn write(
        &self,
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
        let (vertices, indices) = (&scene.vertices, &scene.indices);
        self.frame += 1;
        let frame = self.frame;
        if let Some(timer) = &mut self.timer {
            if let Some((ms, frames)) = timer.harvest() {
                let st = &self.stats;
                log::info!(
                    "AURORA_RT_SHADOWS: pose {:.3} ms, build {:.3} ms, trace {:.3} ms (GPU, mean of {frames} frames); {} placed faces ({} moving, {} merged into {} cells: {} made last frame, {} waiting; the still ones' level built {} times, for [at once, gone, moved, other geometry, to come in, to go out]: {:?}; still faces of up to 64 / 256 / 1024 / 4096 / more triangles: {:?}), {} bottom levels ({} built last frame, {} triangles), {} waiting; {} skeletons ({} built last frame: {} faces, {} vertices)",
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
                    std::array::from_fn::<u64, 6, _>(|i| faces.why[i].saturating_sub(std::mem::replace(&mut self.why[i], faces.why[i]))),
                    self.still_sizes,
                    st.bottoms,
                    st.built,
                    st.built_triangles,
                    st.waiting,
                    st.skeletons,
                    st.skeletons_built,
                    st.posed_faces,
                    st.posed_vertices,
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
        let same_arena = self.arena.as_ref().is_some_and(|(v, i)| v == vertices && i == indices);
        if !same_arena {
            self.bottoms.clear();
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
            if triangles > 0 && triangles + tris > BUILD_TRIANGLES {
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
                if triangles > 0 && triangles + tris > BUILD_TRIANGLES {
                    break;
                }
                triangles += tris;
                index.insert(key, new.len());
                new.push((key, size_of(&key), true));
            }
            self.queue.pop();
        }
        waiting += self.queue.len();
        let created: Vec<wgpu::Blas> = new
            .iter()
            .map(|(_, size, _)| {
                device.create_blas(
                    &wgpu::CreateBlasDescriptor {
                        label: Some("rt shadows face"),
                        flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                    },
                    wgpu::BlasGeometrySizeDescriptors::Triangles {
                        descriptors: vec![size.clone()],
                    },
                )
            })
            .collect();
        // the levels of the cells posed this frame, a geometry per face
        let most = device.limits().max_blas_geometry_count as usize;
        let baked: Vec<(usize, u32, Vec<wgpu::BlasTriangleGeometrySizeDescriptor>, wgpu::Blas)> = bake
            .iter()
            .zip(&self.poser.baked)
            .filter(|&(&n, _)| cells[n].faces.len() <= most)
            .map(|(&n, &offset)| {
                let sizes: Vec<_> = cells[n].faces.iter().map(|f| posed_size(f.index_count, f.opaque)).collect();
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
        // the scene's top level: built when its set changed, when a cell's
        // level is made, when the last bottom level it waited for is, and
        // now and then on the way
        let rebuild = fresh || !baked.is_empty() || (self.stale && (self.queue.is_empty() || frame - self.built_at >= CATCH_UP));
        if rebuild {
            // grown in powers of two
            let wanted = (self.still.len() + cells.len()).max(1).next_power_of_two().max(1024);
            if self.tlas.as_ref().is_none_or(|t| t.get().len() < wanted) {
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
            let made: HashMap<usize, &wgpu::Blas> = baked.iter().map(|(n, _, _, blas)| (*n, blas)).collect();
            for (n, cell) in cells.iter().enumerate() {
                let (blas, records) = match made.get(&n) {
                    Some(&blas) => (blas, None),
                    None => match self.merged.get(&cell.key) {
                        Some(m) => (&m.blas, Some(&m.records)),
                        None => continue,
                    },
                };
                let first = self.merged_list.len() as u32;
                if first >= MERGED {
                    break;
                }
                match records {
                    Some(records) => self.merged_list.extend_from_slice(records),
                    None => self.merged_list.extend(cell.faces.iter().map(|f| f.record)),
                }
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
            self.merged_faces = self.merged_list.len();
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
            self.merged.insert(
                cell.key,
                Merged {
                    blas,
                    version: cell.version,
                    records: cell.faces.iter().map(|f| f.record).collect(),
                    held: true,
                    used: frame,
                },
            );
        }
        let built = created.len();
        for ((key, _, held), blas) in new.into_iter().zip(created) {
            self.bottoms.insert(key, Bottom { blas, used: frame, held });
        }
        // the levels no face used for a while go
        if frame.is_multiple_of(SWEEP_EVERY) {
            self.bottoms.retain(|_, b| b.held || frame - b.used <= KEEP_FRAMES);
            self.merged.retain(|_, m| m.held || frame - m.used <= KEEP_FRAMES);
        }
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
        self.trace(device, encoder, t, depth, contact, scene);
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
                timestamp_writes: self.timer.as_ref().map(|timer| wgpu::ComputePassTimestampWrites {
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
        if let Some(timer) = &mut self.timer {
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
        log::info!("AURORA_RT_REFLECTIONS: ray-traced reflections on, in SSR's place");
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
    /// (rt_denoise.wgsl `cs_variance_pair`), in place of one colour.
    fn new(device: &wgpu::Device, label: &'static str, floor: f32, pair: bool) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/rt_denoise.wgsl").into()),
        });
        let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &[
                uniform_entry::<DenoiseU>(),
                texture_entry(1, unfiltered),
                texture_entry(2, unfiltered),
                storage_texture_entry(3, DENOISE_FORMAT),
                storage_entry(4, false),
            ],
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

    /// Filters `history` (rgb its mean, a its frames) of `size` beside its
    /// `guide`, through `ping`: the image of the two that holds the result.
    #[allow(clippy::too_many_arguments)]
    fn encode<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &mut wgpu::ComputePass<'_>,
        size: (u32, u32),
        history: &'a wgpu::TextureView,
        guide: &wgpu::TextureView,
        ping: &'a [wgpu::TextureView; 2],
    ) -> &'a wgpu::TextureView {
        if self.params_for != Some(size) {
            for (k, buffer) in self.params.iter().enumerate() {
                let step = if k == 0 { 0.0 } else { (1u32 << (k - 1)) as f32 };
                let u = DenoiseU {
                    size: [size.0 as f32, size.1 as f32, step, self.floor],
                };
                queue.write_buffer(buffer, 0, bytemuck::bytes_of(&u));
            }
            let count = size.0.div_ceil(8) as u64 * size.1.div_ceil(8) as u64;
            self.tiles = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rt denoise tiles"),
                size: count * 4,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            }));
            self.params_for = Some(size);
        }
        let Some(tiles) = &self.tiles else {
            return history;
        };
        let groups = (size.0.div_ceil(8), size.1.div_ceil(8));
        let mut input = history;
        for (k, params) in self.params.iter().enumerate() {
            let output = &ping[k % 2];
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rt denoise"),
                layout: &self.layout,
                entries: &[
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
                ],
            });
            pass.set_pipeline(if k == 0 { &self.variance } else { &self.atrous });
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
            input = output;
        }
        input
    }
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
                texture_entry(2, unfiltered),
                texture_entry(3, unfiltered),
                tlas_entry(4),
                tlas_entry(5),
                storage_texture_entry(6, GI_FORMAT),
                storage_texture_entry(7, GUIDE_FORMAT),
                texture_entry(8, unfiltered),
                texture_entry(12, unfiltered),
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
        let filter = pipeline("rt gi filter", &[Some(&filter_layout)], "cs_gi_filter");
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt gi"),
            size: std::mem::size_of::<GiU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        log::info!("AURORA_RT_GI: ray-traced indirect light on, in the screen-space indirect light's place");
        Gi {
            trace_layout,
            filter_layout,
            trace,
            filter,
            buffer,
            denoise: Denoise::new(device, "rt gi denoise", GI_NOISE_FLOOR, false),
            timer: Timer::new(device, queue, 4),
            last: 0,
            history_for: None,
            prev_clip_from_world: glam::Mat4::IDENTITY,
            prev_camera: glam::Vec3::ZERO,
            frame: 0,
        }
    }
}

impl Lights {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, scene_layout: &wgpu::BindGroupLayout, textures: &wgpu::BindGroupLayout) -> Self {
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
            ],
        });
        let filter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt lights filter"),
            entries: &[
                uniform_entry::<LightsU>(),
                texture_entry(9, unfiltered),
                texture_entry(10, unfiltered),
                storage_texture_entry(11, RESULT_FORMAT),
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
        let lights = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt lights list"),
            size: (std::mem::size_of::<TracedLightU>() * MAX_TRACED_LIGHTS) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let points = local_lights();
        if points {
            log::info!("AURORA_RT_LIGHTS: ray-traced point light shadows on");
        }
        Lights {
            points,
            trace_layout,
            reuse_layout,
            filter_layout,
            trace,
            reuse,
            filter,
            buffer,
            lights,
            denoise: Denoise::new(device, "rt lights denoise", SHARE_NOISE_FLOOR, true),
            listed: Vec::new(),
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
/// order.
fn traced_lights(lights: &[crate::types::PointLight]) -> Vec<TracedLightU> {
    lights
        .iter()
        .take(MAX_TRACED_LIGHTS)
        .map(|l| TracedLightU {
            pos_radius: l.position.extend(l.radius).to_array(),
            color_falloff: l.color.extend(l.falloff).to_array(),
            source: [l.source_radius, if l.projector.is_some() { 1.0 } else { 0.0 }, 0.0, 0.0],
        })
        .collect()
}

impl RtShadows {
    /// Whether the point lights' shadows are traced: the frame then binds
    /// the mask with their share (common.wgsl `local_share`).
    pub(crate) fn traces_lights(&self) -> bool {
        self.lights.as_ref().is_some_and(|l| l.points)
    }

    /// Averages and filters the sun's soft shadow and, with the traced
    /// lights, traces the point lights' share into the mask's `.b`: after
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
        clip_from_world: glam::Mat4,
        camera: glam::Vec3,
        point_lights: &[crate::types::PointLight],
        depth: &wgpu::TextureView,
        motion: Option<&wgpu::TextureView>,
        black: &wgpu::TextureView,
        scene: &crate::packet::SceneGpu,
    ) {
        let scene_group = self.scene_group(device, scene);
        let (Some(l), Some(tlas), Some(lt)) = (self.lights.as_mut(), self.tlas.as_ref(), t.lights.as_ref()) else {
            return;
        };
        let mesh_normals = motion.is_some();
        let motion = motion.unwrap_or(black);
        if let Some(timer) = &mut l.timer
            && let Some((ms, frames)) = timer.harvest()
        {
            log::info!(
                "AURORA_RT_LIGHTS: trace {:.3} ms, reuse {:.3} ms, denoise {:.3} ms, filter {:.3} ms (GPU, mean of {frames} frames, {} lights listed)",
                ms[0],
                ms[1],
                ms[2],
                ms[3],
                l.listed.len()
            );
        }
        let listed = if l.points { traced_lights(point_lights) } else { Vec::new() };
        let same_lights = listed == l.listed;
        if !listed.is_empty() {
            queue.write_buffer(&l.lights, 0, bytemuck::cast_slice(&listed));
        }
        l.listed = listed;
        let (read, write) = (l.last, 1 - l.last);
        let valid = l.history_for == Some(t.size);
        let (w, h) = (t.size.0 as f32, t.size.1 as f32);
        let u = LightsU {
            world_from_clip: clip_from_world.inverse().to_cols_array_2d(),
            prev_clip_from_world: l.prev_clip_from_world.to_cols_array_2d(),
            camera: camera.extend(if mesh_normals { 1.0 } else { 0.0 }).to_array(),
            prev_camera: l.prev_camera.extend(1.0).to_array(),
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
        };
        queue.write_buffer(&l.buffer, 0, bytemuck::bytes_of(&u));
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
                    resource: l.lights.as_entire_binding(),
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
                    resource: l.lights.as_entire_binding(),
                },
                view(4, &lt.choice[write]),
                view(7, &lt.share[read]),
                view(8, &lt.share[write]),
                view(10, &lt.sun),
                view(12, motion),
                view(13, &lt.guide[read]),
                view(14, &lt.guide[write]),
                view(16, &lt.mid),
            ],
        });
        let groups = (t.size.0.div_ceil(8), t.size.1.div_ceil(8));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt lights trace"),
                timestamp_writes: l.timer.as_ref().map(|timer| wgpu::ComputePassTimestampWrites {
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
                timestamp_writes: l.timer.as_ref().map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(1),
                    end_of_pass_write_index: None,
                }),
            });
            pass.set_pipeline(&l.reuse);
            pass.set_bind_group(0, &reuse_group, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }
        let denoised = {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt lights denoise"),
                timestamp_writes: l.timer.as_ref().map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(2),
                    end_of_pass_write_index: None,
                }),
            });
            l.denoise
                .encode(device, queue, &mut pass, t.size, &lt.share[write], &lt.guide[write], &lt.ping)
        };
        let filter_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt lights filter"),
            layout: &l.filter_layout,
            entries: &[uniform, view(9, denoised), view(10, &lt.sun), view(11, &t.result)],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt lights filter"),
                timestamp_writes: l.timer.as_ref().map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(3),
                    end_of_pass_write_index: Some(4),
                }),
            });
            pass.set_pipeline(&l.filter);
            pass.set_bind_group(0, &filter_group, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }
        if let Some(timer) = &mut l.timer {
            timer.resolve(encoder);
        }
        l.last = write;
        l.history_for = Some(t.size);
        l.prev_clip_from_world = clip_from_world;
        l.prev_camera = camera;
        l.frame = l.frame.wrapping_add(1);
    }
}

impl RtShadows {
    /// Whether the traced indirect light is on: the shading reads it this
    /// frame when the frame traces.
    pub(crate) fn lights_indirectly(&self) -> bool {
        self.gi.is_some()
    }

    /// Traces this frame's indirect light into the targets' GI result,
    /// after the shadows' pass built this frame's levels and before the
    /// scene is lit; the view's hits read `scene_copy`, last frame's.
    /// `intensity` scales it as it scales GTAO's (Indirect light intensity).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_gi(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        light: &TraceLight,
        intensity: f32,
        depth: &wgpu::TextureView,
        motion: &wgpu::TextureView,
        scene_copy: &wgpu::TextureView,
        scene: &crate::packet::SceneGpu,
    ) {
        let scene_group = self.scene_group(device, scene);
        let (Some(g), Some(tlas), Some(gt)) = (self.gi.as_mut(), self.tlas.as_ref(), t.gi.as_ref()) else {
            return;
        };
        if let Some(timer) = &mut g.timer
            && let Some((ms, frames)) = timer.harvest()
        {
            log::info!(
                "AURORA_RT_GI: trace {:.3} ms, denoise {:.3} ms, filter {:.3} ms (GPU, mean of {frames} frames, {}x{} rays)",
                ms[0],
                ms[1],
                ms[2],
                gt.half.0,
                gt.half.1
            );
        }
        let (read, write) = (g.last, 1 - g.last);
        let valid = g.history_for == Some(t.size);
        let (w, h) = (t.size.0 as f32, t.size.1 as f32);
        let u = GiU {
            clip_from_world: light.clip_from_world.to_cols_array_2d(),
            world_from_clip: light.clip_from_world.inverse().to_cols_array_2d(),
            prev_clip_from_world: g.prev_clip_from_world.to_cols_array_2d(),
            camera: light.camera.extend(1.0).to_array(),
            prev_camera: g.prev_camera.extend(1.0).to_array(),
            lit: light.lit(),
            screen: [w, h, 1.0 / w, 1.0 / h],
            half: [gt.half.0 as f32, gt.half.1 as f32, intensity, 0.0],
            mode: [
                GI_REACH,
                if self.posed_live { 1.0 } else { 0.0 },
                (g.frame % (1 << 20)) as f32,
                if valid { 1.0 } else { 0.0 },
            ],
        };
        queue.write_buffer(&g.buffer, 0, bytemuck::bytes_of(&u));
        let trace_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt gi trace"),
            layout: &g.trace_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: g.buffer.as_entire_binding(),
                },
                view(1, depth),
                view(2, &gt.color[read]),
                view(3, &gt.guide[read]),
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::AccelerationStructure(tlas),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    // without posed faces the scene's stands in, unread
                    resource: wgpu::BindingResource::AccelerationStructure(self.poser.traced().unwrap_or(tlas)),
                },
                view(6, &gt.color[write]),
                view(7, &gt.guide[write]),
                view(8, scene_copy),
                view(12, motion),
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt gi trace"),
                timestamp_writes: g.timer.as_ref().map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: None,
                }),
            });
            pass.set_pipeline(&g.trace);
            pass.set_bind_group(0, &trace_group, &[]);
            pass.set_bind_group(1, &scene_group, &[]);
            pass.set_bind_group(2, &scene.textures, &[]);
            pass.dispatch_workgroups(gt.half.0.div_ceil(8), gt.half.1.div_ceil(8), 1);
        }
        let denoised = {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt gi denoise"),
                timestamp_writes: g.timer.as_ref().map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(1),
                    end_of_pass_write_index: None,
                }),
            });
            g.denoise
                .encode(device, queue, &mut pass, gt.half, &gt.color[write], &gt.guide[write], &gt.ping)
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
                timestamp_writes: g.timer.as_ref().map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(2),
                    end_of_pass_write_index: Some(3),
                }),
            });
            pass.set_pipeline(&g.filter);
            pass.set_bind_group(0, &filter_group, &[]);
            pass.dispatch_workgroups(t.size.0.div_ceil(8), t.size.1.div_ceil(8), 1);
        }
        if let Some(timer) = &mut g.timer {
            timer.resolve(encoder);
        }
        g.last = write;
        g.history_for = Some(t.size);
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
        scene_copy: &wgpu::TextureView,
        scene: &crate::packet::SceneGpu,
    ) {
        let scene_group = self.scene_group(device, scene);
        let (Some(r), Some(tlas), Some(out)) = (self.reflect.as_mut(), self.tlas.as_ref(), t.reflect.as_ref()) else {
            return;
        };
        if let Some(timer) = &mut r.timer
            && let Some((ms, frames)) = timer.harvest()
        {
            log::info!("AURORA_RT_REFLECTIONS: trace {:.3} ms (GPU, mean of {frames} frames)", ms[0]);
        }
        let (w, h) = (t.size.0 as f32, t.size.1 as f32);
        let u = ReflectU {
            clip_from_world: light.clip_from_world.to_cols_array_2d(),
            world_from_clip: light.clip_from_world.inverse().to_cols_array_2d(),
            camera: light.camera.extend(1.0).to_array(),
            lit: light.lit(),
            screen: [w, h, 1.0 / w, 1.0 / h],
            mode: [REACH, if self.posed_live { 1.0 } else { 0.0 }, 0.0, 0.0],
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
            log::warn!("AURORA_RT_SHADOWS: a scene buffer is larger than the device binds; what lies past it is not traced right");
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

/// The transform of a top level's instance whose level is in world space.
const IDENTITY: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];

/// A column-major model matrix as the top level's 3×4 row-major transform.
fn row_major(m: &[[f32; 4]; 4]) -> [f32; 12] {
    std::array::from_fn(|i| m[i % 4][i / 4])
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn candidate(key: u32, record: u32, x: f32) -> Candidate {
        let mut model = glam::Mat4::IDENTITY.to_cols_array_2d();
        model[3][0] = x;
        Candidate {
            key,
            face: Posed {
                record,
                base_vertex: 0,
                first_index: 0,
                index_count: 300,
                opaque: true,
            },
            model,
            origin: [x, 0.0, 0.0],
        }
    }

    fn choose(tracker: &mut PoseTracker, candidates: Vec<Candidate>) -> Vec<(u32, bool)> {
        tracker.frame += 1;
        tracker.scratch = candidates;
        let mut out = Faces::default();
        choose_rebuilds(&[], glam::Vec3::ZERO, tracker, &mut out);
        out.groups.iter().map(|g| (g.key, g.rebuild)).collect()
    }

    #[test]
    fn rt_shadows_a_still_skeleton_keeps_its_level() {
        let mut t = PoseTracker::default();
        assert_eq!(choose(&mut t, vec![candidate(0, 1, 5.0)]), vec![(0, true)]);
        assert_eq!(choose(&mut t, vec![candidate(0, 1, 5.0)]), vec![(0, false)]);
        // less than the ray's start offset at 5 m (0.02 m): kept
        assert_eq!(choose(&mut t, vec![candidate(0, 1, 5.015)]), vec![(0, false)]);
        // further: built again
        assert_eq!(choose(&mut t, vec![candidate(0, 1, 5.05)]), vec![(0, true)]);
        // another face set: built again even when still
        assert_eq!(choose(&mut t, vec![candidate(0, 1, 5.05), candidate(0, 2, 5.05)]), vec![(0, true)]);
    }

    #[test]
    fn rt_shadows_rebuilds_go_nearest_first_within_the_budget() {
        let mut t = PoseTracker::default();
        let big = |key: u32, x: f32| {
            let mut c = candidate(key, key, x);
            c.face.index_count = (REBUILD_TRIANGLES * 3 / 2) as u32;
            c
        };
        let groups = choose(&mut t, vec![big(1, 30.0), big(2, 3.0), big(3, 10.0)]);
        // the nearest two fit, the farthest waits for a later frame
        assert_eq!(groups, vec![(1, false), (2, true), (3, true)]);
        let groups = choose(&mut t, vec![big(1, 30.0), big(2, 3.0), big(3, 10.0)]);
        assert_eq!(groups, vec![(1, true), (2, false), (3, false)]);
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

    /// Visits `faces` on the tracker's next frame, as `gather` does (a
    /// full look when a face changed and every RECHECK frames): whether a
    /// new still set was made, and the moving faces.
    fn frame(t: &mut StillTracker, n: &mut u64, faces: &[F]) -> (bool, Vec<usize>) {
        *n += 1;
        t.begin(8);
        let maybe: Vec<usize> = faces
            .iter()
            .filter(|&&F(id, place, shape, kept, near)| {
                let look = (id as u64 + *n).is_multiple_of(RECHECK);
                (if look { None } else { t.same(id, *n, place, shape) }).unwrap_or_else(|| t.visit(id, *n, place, shape, kept, near, true))
            })
            .map(|f| f.0)
            .collect();
        let set = t.decide(*n);
        (set, maybe.into_iter().filter(|&id| t.moving(id)).collect())
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
        // nothing changes: the set stands
        for _ in 0..3 * LATER {
            assert_eq!(frame(&mut t, &mut n, &[near(0, 1), far(1, 2)]), (false, vec![]));
        }
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
        // a near one at once
        let (mut t, mut n) = (StillTracker::default(), 0);
        frame(&mut t, &mut n, &[near(0, 1), near(1, 2)]);
        assert_eq!(frame(&mut t, &mut n, &[near(0, 1)]), (true, vec![]));
        assert!(t.in_still(0) && !t.in_still(1));
        // a far one soon
        let (mut t, mut n) = (StillTracker::default(), 0);
        frame(&mut t, &mut n, &[near(0, 1), far(1, 2)]);
        assert_eq!(until_set(&mut t, &mut n, &[near(0, 1)], &[]), SOON + 1);
        assert!(t.in_still(0) && !t.in_still(1));
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
        // the set later (noticed at its next full look)
        let frames = until_set(&mut t, &mut n, &[near(0, 1), F(1, 2, 0, false, false)], &[]);
        assert!((LATER..=LATER + RECHECK).contains(&frames), "after {frames} frames");
        assert!(t.in_still(0) && !t.in_still(1));
        // and after a shift of the world's origin every face is new
        t.reset();
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
        // face 1 turns into a face that is not traced (a blended one)
        n += 1;
        t.begin(8);
        assert_eq!(t.same(0, n, 1, 0), Some(false));
        t.other(1, n, 2, 0, NOT_TRACED);
        assert!(t.decide(n));
        assert!(t.in_still(0) && !t.in_still(1));
        // and is not looked at again while it stays the same
        n += 1;
        t.begin(8);
        assert_eq!(t.same(1, n, 2, 0), Some(false));
        assert_eq!(t.same(0, n, 1, 0), Some(false));
        assert!(!t.decide(n));
        // a skeleton's face is looked at every frame
        t.other(1, n, 2, 0, POSED);
        n += 1;
        t.begin(8);
        assert_eq!(t.same(1, n, 2, 0), None);
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
    fn rt_shadows_mode_parses_the_variable() {
        assert_eq!(parse(None), None);
        assert_eq!(parse(Some("0")), None);
        assert_eq!(parse(Some("")), None);
        assert_eq!(parse(Some("1")), Some(Cull::None));
        assert_eq!(parse(Some("front")), Some(Cull::Front));
        assert_eq!(parse(Some("back")), Some(Cull::Back));
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

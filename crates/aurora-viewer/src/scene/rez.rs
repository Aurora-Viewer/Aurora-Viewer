//! The rez effect (Preferences › Aurora, "Rez effect"): nothing shows grey
//! while it loads. A face whose texture has not come (its slot still points
//! at the grey placeholder) is drawn as a glowing grid by the shader itself
//! (`rez_still_grey` in object.wgsl), near or far, from its first frame, and
//! fades into its texture the moment that texture shows its first pixels
//! (`Scene::rez_first_texels`): no face waits for another, nothing is ever
//! grey, and nothing is followed on the CPU for it. The colors run along
//! the brand gradient of the theme (`Palette::aurora_gradient`).
//!
//! On top, the events get a whole-object effect (`DrawRecord::rez`, from
//! the units below): a bright line rising from the bottom to the top
//! (`MAX_SWEEPS` at once, near and big enough on the screen), its faces in
//! the glow pass too (`FaceDraw::rez_glow`), which spreads its light.
//!
//! Avatars: the grid replaces the loading cloud (`loading.rs`) while an
//! avatar's appearance and what it wears load, then one line goes over it
//! and what it wears, from its feet. After `AVATAR_GRID_LIMIT` it shows
//! anyway, never as a cloud again. Ours gets the line when the loading or
//! teleport screen lifts and when its own appearance changes. Leaving
//! (KillObject), an avatar or a deleted object gets the reverse, a derez:
//! the line comes down and leaves the grid, then it goes; its last frame
//! stays drawn that long (`Scene::collect_garbage` waits for it), never
//! more, and a slot taken again ends it at once.
//!
//! A rez in the world gets the whole object in the grid until it has
//! loaded, then the line: ours (FLAGS_CREATE_SELECTED, set by the simulator
//! on the first update of what we create, as
//! LLViewerObject::processUpdateMessage reads it) or someone else's (a new
//! object near the camera in a region we have been watching for a while:
//! the simulator sends no "just rezzed" mark for those); a quick line when
//! it is ready at once. A linkset or an avatar with what it wears is one
//! effect, driven from its root; whatever path syncs a new object
//! (`Scene::after_full_sync`) tells it.
//!
//! Nothing for: what loaded behind the loading or teleport screen, a
//! second visit of the same object (LOD switches, texture reloads),
//! temporary objects (bullets), what leaves with a region (our teleport) or
//! in a burst.

use super::Scene;
use aurora_net::RegionHandle;
use aurora_render::{Renderer, rez_mode};
use glam::{Quat, Vec3};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Length of the line's rise: longer for a taller object, within these
/// bounds (`sweep_for`); the records keep it a little longer so its last
/// frame is drawn.
const SWEEP_MIN: Duration = Duration::from_millis(550);
const SWEEP_MAX: Duration = Duration::from_millis(800);
const SWEEP_TAIL: Duration = Duration::from_millis(60);
/// Lines running at once; beyond, an object fades in from its grid.
pub const MAX_SWEEPS: usize = 6;
/// Loaded but no line free: waits this long for one, then fades in.
const SLOT_WAIT: Duration = Duration::from_millis(400);
const AVATAR_SLOT_WAIT: Duration = Duration::from_millis(400);
/// From the grid to the textures without a line.
pub const FADE: Duration = Duration::from_millis(150);
/// Farther than this, or smaller on the screen than this many pixels: the
/// grid, then a fade, no line.
const MAX_DISTANCE: f32 = 48.0;
const MIN_PIXELS: f32 = 24.0;
/// The same for avatars (whose grid stands for the cloud farther away).
const AVATAR_DISTANCE: f32 = 96.0;
const AVATAR_PIXELS: f32 = 12.0;
/// Length of a derez, how long a killed object's last frame stays drawn.
pub const DEREZ: Duration = Duration::from_millis(700);
/// More objects than this killed at once: no derez (a burst, a cleanup).
const MAX_KILLS: usize = 24;
/// Loaded within this long of its grid (from the caches): a fade, never a
/// line, so an arrival with thousands of cached objects is no firework.
const HOLOGRAM_AFTER: Duration = Duration::from_millis(300);
/// The grid of a rezzed object stays at least this long before the line:
/// its lines trace in from the base in the first 0.2 s (`REZ_INTRO` in
/// object.wgsl).
const MIN_HOLOGRAM: Duration = Duration::from_millis(300);
/// A rezzed object ready at once (from the caches, a copy in the build
/// tools) gets a quick line instead.
const QUICK_SWEEP: Duration = Duration::from_millis(300);
/// What an avatar puts on or takes off once it has been there this long
/// rezzes or derezzes on it; this many items within `OUTFIT_WINDOW` (or
/// its bakes changing) are an outfit change: one line over the avatar.
const WORN_AFTER: Duration = Duration::from_secs(4);
const OUTFIT_ITEMS: usize = 3;
const OUTFIT_WINDOW: Duration = Duration::from_millis(1500);
/// Still loading after this long: revealed anyway. An avatar out of view
/// that long is forgotten (its cloud stays, as before).
const OBJECT_LIMIT: Duration = Duration::from_secs(8);
const AVATAR_GRID_LIMIT: Duration = Duration::from_secs(20);
const AVATAR_LIMIT: Duration = Duration::from_secs(30);
/// A new object is taken for a rez once its region and our view have been
/// there this long (what comes before is the region streaming in), and
/// within this distance of the camera.
const SETTLED: Duration = Duration::from_secs(8);
const REZ_NEAR: f32 = 40.0;
/// Prims looked at a frame in turn, for the time limits (what loads is
/// looked at when it does: `Rez::note_ready`).
const CHECKS_PER_FRAME: usize = 400;

/// Update flags of an object (object_flags.h).
const FLAGS_USE_PHYSICS: u32 = 1 << 0;
const FLAGS_TEMPORARY_ON_REZ: u32 = 1 << 29;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    /// Followed, nothing shown yet.
    Waiting,
    /// The grid, since then.
    Hologram(Instant),
    /// The line, started then.
    Sweep(Instant),
    /// From the grid to the textures without a line, started then.
    Fade(Instant),
    /// Killed: the line coming down, started then.
    Derez(Instant),
}

struct Unit {
    id: Uuid,
    avatar: bool,
    /// Just rezzed in the world (ours or someone else's).
    rezzed: bool,
    /// Arrived, or came into view (the later).
    since: Instant,
    /// Length of its line or fade, once started.
    sweep: Duration,
    phase: Phase,
    /// Its prims (the root first): a linkset, an avatar and what it wears.
    members: Vec<usize>,
    /// What their records hold, and whether their faces are in the glow
    /// pass for it (the lines, and the grids near enough for one).
    value: [u32; 4],
    glow: bool,
    waiting_line: Option<Instant>,
}

/// What the effect counted (the demo's log, the report of the
/// preferences' test).
#[derive(Debug, Default, Clone, Copy)]
pub struct RezStats {
    pub holograms: u32,
    pub sweeps: u32,
    /// From the grid to the textures without a line.
    pub fades: u32,
    /// Loaded with every line busy: a fade instead.
    pub no_line: u32,
    pub rezzed: u32,
    pub derezzed: u32,
    /// Worn items rezzed or derezzed on their avatar, outfit changes.
    pub worn: u32,
    pub outfits: u32,
    /// Faces that left the grid the shader drew for them, as their
    /// texture came.
    pub face_fades: u32,
    /// Units followed at the last update (the grids, lines and fades).
    pub units: usize,
}

#[derive(Default)]
pub struct Rez {
    /// The option (and not "reduce motion"); off, the records hold nothing.
    pub enabled: bool,
    /// Every start and end in the log (demo).
    pub log: bool,
    pub stats: RezStats,
    /// Objects synced for the first time since the last update.
    arrivals: Vec<usize>,
    /// Units to look at now: one of their textures showed its first
    /// pixels or their geometry was built.
    ready: Vec<usize>,
    watching_since: Option<Instant>,
    units: HashMap<usize, Unit>,
    member_of: HashMap<usize, usize>,
    /// Units loading, looked at in turn; units sweeping (a line or a
    /// derez); units fading.
    queue: VecDeque<usize>,
    sweeping: Vec<usize>,
    fading: Vec<usize>,
    /// When each region's first object came.
    regions: HashMap<RegionHandle, Instant>,
    /// Avatars present, since when, and their appearance (their bakes) at
    /// the last look, to see it change.
    avatars_seen: HashMap<Uuid, (Instant, u64)>,
    /// Avatars shown by the grid's time limit while still loading: never a
    /// cloud again.
    revealed: HashSet<usize>,
    /// Worn items that came lately, by avatar (an outfit change brings
    /// several at once).
    worn_lately: HashMap<usize, Vec<(Instant, usize)>>,
    /// Prims whose records must lose the effect (their unit is gone).
    to_clear: Vec<usize>,
    /// Faces fading from the grid into their texture: record, the value
    /// written, until when.
    face_fades: Vec<(u32, [u32; 4], Instant)>,
    /// When the counts last went to the log.
    logged_at: Option<Instant>,
    /// A look for some objects or avatars by id, whatever the frame's
    /// (the demo's side-by-side comparison, `AURORA_DEMO_REZ_EFFECT=styles`).
    pub styles: HashMap<Uuid, u32>,
    /// Those of them followed, by slot (a derez finds its look there).
    styled: HashMap<usize, u32>,
}

/// The records' value of a phase: x its start on the animation clock, y
/// its mode (bits 0–7), the line's length in ms (8–19) and a look of its
/// own (20–23, 0: the frame's), z and w bottom and height (f32 bits).
pub fn record_value(mode: u32, start_ms: u32, sweep: Duration, bottom: f32, height: f32) -> [u32; 4] {
    let ms = (sweep.as_millis() as u32).min(0xFFF);
    [start_ms, mode | ms << 8, bottom.to_bits(), height.to_bits()]
}

/// `value` with a look of its own (`AURORA_REZ_STYLE`'s numbers; 0 keeps
/// the frame's).
fn with_style(mut value: [u32; 4], style: u32) -> [u32; 4] {
    value[1] = value[1] & !(0xF << 20) | (style & 0xF) << 20;
    value
}

/// How long the line takes to rise over `height` metres: a little longer
/// for a building than for a cup.
pub fn sweep_for(height: f32) -> Duration {
    let ms = 520.0 + 60.0 * height.max(0.0);
    Duration::from_millis(ms as u64).clamp(SWEEP_MIN, SWEEP_MAX)
}

/// Whether a new object (a root, not worn) is a rez in the world rather
/// than the region streaming in: ours by its flag (which the simulator
/// sends to its creator only), someone else's by being new near us in a
/// region we have been watching for a while. Temporary and fast physical
/// objects (bullets, debris) are not.
pub fn is_fresh_rez(update_flags: u32, settled_for: Option<Duration>, distance: f32, speed: f32) -> bool {
    if update_flags & FLAGS_TEMPORARY_ON_REZ != 0 || update_flags & FLAGS_USE_PHYSICS != 0 && speed > 2.0 {
        return false;
    }
    if update_flags & crate::build::FLAGS_CREATE_SELECTED != 0 {
        return true;
    }
    settled_for.is_some_and(|d| d >= SETTLED) && distance <= REZ_NEAR
}

/// Vertical half extent of a box of `scale` turned by `rot`.
fn half_height(scale: Vec3, rot: Quat) -> f32 {
    let (x, y, z) = (rot * Vec3::X, rot * Vec3::Y, rot * Vec3::Z);
    0.5 * (x.z.abs() * scale.x + y.z.abs() * scale.y + z.z.abs() * scale.z)
}

/// Faces take an effect's `value` (or lose it, at zero): their records,
/// and their place in the glow pass when `glow` asks for it, which only
/// the effect gave them (`FaceDraw::rez_glow`). Whether one changed its
/// glow.
pub(super) fn rez_faces(renderer: &mut Renderer, faces: &mut [super::FaceDraw], value: [u32; 4], glow: bool) -> bool {
    let want = glow && value[1] != rez_mode::NONE;
    let mut changed = false;
    for f in faces {
        renderer.records.set_rez(f.record, value);
        if want && !f.glow {
            f.glow = true;
            f.rez_glow = true;
            changed = true;
        } else if !want && f.rez_glow {
            f.glow = false;
            f.rez_glow = false;
            changed = true;
        }
    }
    changed
}

/// What a unit does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Stay,
    Hologram,
    Sweep,
    /// The short line of what was ready at once.
    Quick,
    /// From the grid to the textures, no line.
    Fade,
    /// Shown as before (and forgotten).
    Drop,
}

/// The decision for a unit not sweeping yet. `near`: in view, near and
/// big enough for a line; `age`: since it arrived or we started watching;
/// `in_phase`: since its grid showed.
#[allow(clippy::too_many_arguments)]
fn next_step(
    phase: Phase,
    avatar: bool,
    rezzed: bool,
    loaded: bool,
    near: bool,
    watching: bool,
    age: Duration,
    in_phase: Duration,
) -> Step {
    if !watching {
        // behind a loading screen: what is ready simply shows
        return if loaded && !avatar { Step::Drop } else { Step::Stay };
    }
    match phase {
        Phase::Waiting if avatar => {
            // (out of view or far: the cloud, as before)
            if !near {
                if age > AVATAR_LIMIT { Step::Drop } else { Step::Stay }
            } else if loaded {
                Step::Sweep
            } else {
                Step::Hologram
            }
        }
        Phase::Waiting => {
            // a rezzed object ready at once (the others start in the grid)
            if !loaded {
                Step::Hologram
            } else if near {
                Step::Quick
            } else {
                Step::Drop
            }
        }
        Phase::Hologram(_) if avatar => {
            // an avatar's grid stays until it can really be drawn (the
            // signal that ends its cloud: `loading.rs`), so the line never
            // ends on a cloud or a grey body; past the limit it shows anyway
            if loaded || in_phase >= AVATAR_GRID_LIMIT {
                if near { Step::Sweep } else { Step::Fade }
            } else {
                Step::Stay
            }
        }
        Phase::Hologram(_) => {
            // (still loading, or the grid of a rezzed object shown a moment)
            if !loaded && in_phase < OBJECT_LIMIT || rezzed && in_phase < MIN_HOLOGRAM {
                Step::Stay
            } else if !rezzed && loaded && in_phase < HOLOGRAM_AFTER || !near {
                // from the caches, or no line for it
                Step::Fade
            } else {
                Step::Sweep
            }
        }
        Phase::Sweep(_) | Phase::Fade(_) | Phase::Derez(_) => Step::Stay,
    }
}

impl Rez {
    /// An object synced for the first time (`Scene::sync_arrivals`).
    pub fn arrived(&mut self, idx: usize) {
        if self.enabled {
            self.arrivals.push(idx);
        }
    }

    /// Prim `idx` may have finished loading (a texture's first pixels, its
    /// geometry built): its unit is looked at this frame.
    pub fn note_ready(&mut self, idx: usize) {
        if let Some(&root) = self.member_of.get(&idx) {
            self.ready.push(root);
        }
    }

    /// What the records of prim `idx` hold, and whether its faces glow for
    /// it, for faces built again.
    pub fn value_of(&self, idx: usize) -> Option<([u32; 4], bool)> {
        let root = self.member_of.get(&idx)?;
        self.units.get(root).map(|u| (u.value, u.glow)).filter(|(v, _)| v[1] != 0)
    }

    /// Prim `idx` leaves the scene.
    pub fn forget(&mut self, idx: usize) {
        self.styled.remove(&idx);
        self.revealed.remove(&idx);
        if let Some(root) = self.member_of.remove(&idx)
            && let Some(u) = self.units.get_mut(&root)
        {
            u.members.retain(|&m| m != idx);
        }
        if let Some(u) = self.units.remove(&idx) {
            for m in u.members {
                if m != idx {
                    self.member_of.remove(&m);
                    self.to_clear.push(m);
                }
            }
        }
    }

    /// Lines running.
    pub fn sweeping(&self) -> usize {
        self.sweeping.len()
    }

    /// Avatar `idx` is drawn by the effect (its grid, its line or its
    /// fade), or was shown by the grid's time limit: no cloud for it, and
    /// not hidden while it loads.
    pub fn shows_avatar(&self, idx: usize) -> bool {
        self.revealed.contains(&idx)
            || self
                .units
                .get(&idx)
                .is_some_and(|u| u.avatar && matches!(u.phase, Phase::Hologram(_) | Phase::Sweep(_) | Phase::Fade(_)))
    }

    /// Slot `idx` of a killed object is still being derezzed: its GPU
    /// state waits.
    pub fn holds(&self, idx: usize, now: Instant) -> bool {
        self.member_of
            .get(&idx)
            .and_then(|r| self.units.get(r))
            .is_some_and(|u| matches!(u.phase, Phase::Derez(t) if now.duration_since(t) < DEREZ + SWEEP_TAIL))
    }

    fn add_unit(&mut self, root: usize, unit: Unit) {
        if let Some(&style) = self.styles.get(&unit.id) {
            self.styled.insert(root, style);
        }
        for &m in &unit.members {
            self.member_of.insert(m, root);
        }
        self.units.insert(root, unit);
        self.queue.push_back(root);
    }

    fn remove_unit(&mut self, root: usize) -> Option<Unit> {
        let u = self.units.remove(&root)?;
        for m in &u.members {
            self.member_of.remove(m);
        }
        Some(u)
    }

    fn style_of(&self, root: usize) -> u32 {
        self.styled.get(&root).copied().unwrap_or(0)
    }
}

/// What a frame's look at the units knows.
struct Frame<'a> {
    world: &'a crate::world::World,
    view: &'a super::CullView,
    now: Instant,
    /// The animation clock (ms) now.
    ms: u32,
    watching: bool,
    watching_since: Instant,
}

impl Scene {
    /// The records of `members` hold `value`; their faces glow for it when
    /// `glow` says so.
    fn write_rez(&mut self, renderer: &mut Renderer, members: &[usize], value: [u32; 4], glow: bool) {
        for &m in members {
            let Some(g) = self.gpu.get_mut(m) else {
                continue;
            };
            if rez_faces(renderer, &mut g.faces, value, glow) {
                g.special = super::is_special(&g.faces);
                self.list_index.set(m, g);
            }
        }
    }

    /// Whether every prim of `members` shows its own textures (or never
    /// will: a texture the asset server does not have).
    fn members_loaded(&self, members: &[usize]) -> bool {
        members.iter().all(|&m| {
            let Some(g) = self.gpu.get(m) else {
                return true;
            };
            let geometry = g.wanted_geom.is_none_or(|k| g.geom == Some(k)) && g.sculpt_wait.is_none();
            geometry
                && g.faces.iter().all(|f| {
                    f.tex_id.is_nil()
                        || self.textures.is_loaded(&f.tex_id)
                        || self.textures.is_local(&f.tex_id)
                        || self.textures.is_unavailable(&f.tex_id)
                })
        })
    }

    /// Bottom and height of what `members` cover, in the world.
    fn rez_bounds(world: &crate::world::World, members: &[usize], avatar: bool, now: Instant) -> Option<(f32, f32)> {
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for (i, &m) in members.iter().enumerate() {
            let Some(o) = world.objects.get(m) else {
                continue;
            };
            let Some((pos, rot, hud)) = Self::object_transform(world, m, now, 0) else {
                continue;
            };
            if hud {
                continue;
            }
            if avatar {
                // the avatar's own size; what it wears hangs within it
                if i == 0 {
                    let h = o.scale.z.max(0.5) * 0.5;
                    lo = lo.min(pos.z - h - 0.08);
                    hi = hi.max(pos.z + h + 0.15);
                }
                continue;
            }
            let h = half_height(o.visual_scale(), rot);
            lo = lo.min(pos.z - h);
            hi = hi.max(pos.z + h);
        }
        (lo <= hi).then_some((lo, (hi - lo).max(0.05)))
    }

    /// Bottom and height of a unit: from its drawn spheres for a worn item
    /// (whose prim sizes say little of a rigged mesh), else from its prims.
    fn unit_bounds(&self, world: &crate::world::World, root: usize, members: &[usize], avatar: bool, now: Instant) -> Option<(f32, f32)> {
        let worn = !avatar && self.gpu.get(root).is_some_and(|g| g.owner_avatar.is_some());
        if worn {
            self.gpu_bounds(members)
        } else {
            Self::rez_bounds(world, members, avatar, now)
        }
    }

    /// The prims of a unit rooted at `root`: an avatar with what it wears
    /// (no HUD), or a linkset.
    fn rez_members(world: &crate::world::World, root: usize) -> Vec<usize> {
        let mut out = vec![root];
        let Some(o) = world.objects.get(root) else {
            return out;
        };
        for &c in world.objects.children_of(&o.key) {
            out.push(c);
            if o.is_avatar() {
                let Some(att) = world.objects.get(c) else {
                    continue;
                };
                if (31..=38).contains(&att.attachment_point()) {
                    out.pop();
                    continue;
                }
                out.extend_from_slice(world.objects.children_of(&att.key));
            }
        }
        out
    }

    /// Whether `root` is in view, near and big enough for a line (and its
    /// distance).
    fn rez_visible(&self, view: &super::CullView, root: usize) -> (bool, f32) {
        let Some(g) = self.gpu.get(root) else {
            return (false, f32::MAX);
        };
        let (far, px) = if g.is_avatar {
            (AVATAR_DISTANCE, AVATAR_PIXELS)
        } else {
            (MAX_DISTANCE, MIN_PIXELS)
        };
        let distance = g.center.distance(view.eye);
        let visible = !g.hud && distance <= far && view.sphere_visible(g.center, g.radius) && view.pixel_size(g.center, g.radius) >= px;
        (visible, distance)
    }

    /// The loading clouds of the avatars the effect draws go at once: a
    /// stopped cloud's particles lived on for seconds over the grid and
    /// past the line.
    pub fn drop_rez_clouds(&mut self, world: &crate::world::World) {
        let ids: Vec<Uuid> = self
            .rez
            .units
            .keys()
            .chain(self.rez.revealed.iter())
            .filter(|&&i| self.rez.shows_avatar(i))
            .filter_map(|&i| world.objects.get(i).map(|o| o.full_id))
            .collect();
        if !ids.is_empty() {
            self.particles.drop_clouds(&mut self.textures, &ids);
        }
    }

    /// Hash of an avatar's bakes, to see its appearance change.
    fn own_look(world: &crate::world::World, idx: usize) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (te, _, _) in super::avatar::BAKES {
            world.bake_texture(idx, te).hash(&mut h);
        }
        h.finish()
    }

    /// An avatar to follow from now.
    fn avatar_unit(id: Uuid, idx: usize, now: Instant) -> Unit {
        Unit {
            id,
            avatar: true,
            rezzed: false,
            since: now,
            sweep: Duration::ZERO,
            phase: Phase::Waiting,
            members: vec![idx],
            value: [0; 4],
            glow: false,
            waiting_line: None,
        }
    }

    /// Bottom and height of what `members` cover, from their drawn
    /// spheres.
    fn gpu_bounds(&self, members: &[usize]) -> Option<(f32, f32)> {
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for &m in members {
            if let Some(g) = self.gpu.get(m)
                && !g.faces.is_empty()
            {
                lo = lo.min(g.center.z - g.radius);
                hi = hi.max(g.center.z + g.radius);
            }
        }
        (lo <= hi).then_some((lo, (hi - lo).max(0.05)))
    }

    /// The faces of a derez are drawn blended while it lasts, so its grid
    /// can fade out (`rez_alpha` in object.wgsl); the object is gone after.
    fn derez_blended(&mut self, renderer: &mut Renderer, members: &[usize]) {
        for &m in members {
            let Some(g) = self.gpu.get_mut(m) else {
                continue;
            };
            let mut changed = false;
            for (fi, f) in g.faces.iter_mut().enumerate() {
                if f.pass == super::Pass::Hidden || f.pass == super::Pass::Blend {
                    continue;
                }
                f.pass = super::Pass::Blend;
                changed = true;
                if let Some(mut r) = renderer.records.get(f.record).copied() {
                    r.flags[0] |= aurora_render::flags::ALPHA_BLEND;
                    renderer.records.set(f.record, r);
                }
                renderer.records.set_face(f.record, super::cull_face(f, m, fi == 0));
            }
            if changed {
                g.special = true;
                self.list_index.set(m, g);
            }
        }
    }

    /// Objects the simulator killed (`ObjectStore::killed`): a derez for
    /// those in view, near, within the lines free, unless too many went at
    /// once. Their slots are released once it is over
    /// (`Scene::collect_garbage`). Several worn items taken off one avatar
    /// at once are an outfit change: no derez, its line comes once the new
    /// outfit is there.
    pub(super) fn note_kills(&mut self, renderer: &mut Renderer, kills: Vec<Vec<usize>>, now: Instant) {
        if !self.rez.enabled || self.rez.watching_since.is_none() || kills.len() > MAX_KILLS {
            return;
        }
        let Some(view) = self.last_cull else {
            return;
        };
        let ms = renderer.anim_clock().ms(now);
        let mut taken_off: HashMap<usize, usize> = HashMap::new();
        for members in &kills {
            if let Some(a) = members.first().and_then(|&r| self.gpu.get(r)).and_then(|g| g.owner_avatar) {
                *taken_off.entry(a).or_default() += 1;
            }
        }
        for members in kills {
            let Some(&root) = members.first() else {
                continue;
            };
            let wearer = self.gpu.get(root).and_then(|g| g.owner_avatar);
            if wearer.is_some_and(|a| taken_off.get(&a).copied().unwrap_or(0) >= OUTFIT_ITEMS) {
                continue;
            }
            if self.rez.sweeping.len() >= MAX_SWEEPS {
                self.rez.stats.no_line += 1;
                break;
            }
            let (visible, _) = self.rez_visible(&view, root);
            if !visible || self.gpu.get(root).is_none_or(|g| g.faces.is_empty() && !g.is_avatar) {
                continue;
            }
            // (its own effect, if one ran, gives way)
            if let Some(old) = self.rez.remove_unit(root) {
                self.rez.queue.retain(|&r| r != root);
                self.rez.sweeping.retain(|&r| r != root);
                self.rez.fading.retain(|&r| r != root);
                self.write_rez(renderer, &old.members, [0; 4], false);
            }
            let Some((mut lo, height)) = self.gpu_bounds(&members) else {
                continue;
            };
            let mut hi = lo + height;
            let avatar = self.gpu[root].is_avatar;
            if avatar {
                // (an avatar's sphere holds its arms: its height is smaller)
                let c = self.gpu[root].center.z;
                lo = lo.max(c - 1.1);
                hi = hi.min(c + 1.2);
            }
            let style = self.rez.style_of(root);
            let value = with_style(record_value(rez_mode::DEREZ, ms, DEREZ, lo, (hi - lo).max(0.05)), style);
            let unit = Unit {
                id: Uuid::nil(),
                avatar,
                rezzed: false,
                since: now,
                sweep: DEREZ,
                phase: Phase::Derez(now),
                members: members.clone(),
                value,
                glow: true,
                waiting_line: None,
            };
            for &m in &members {
                self.rez.member_of.insert(m, root);
            }
            self.rez.units.insert(root, unit);
            self.rez.sweeping.push(root);
            self.rez.stats.derezzed += 1;
            if wearer.is_some() {
                self.rez.stats.worn += 1;
            }
            if self.rez.log {
                log::info!(
                    "rez effect: derez of {} prims{}",
                    members.len(),
                    if avatar {
                        " (avatar)"
                    } else if wearer.is_some() {
                        " (worn)"
                    } else {
                        ""
                    }
                );
            }
            self.derez_blended(renderer, &members);
            self.write_rez(renderer, &members, value, true);
        }
    }

    /// Textures that showed their first pixels
    /// (`TextureStreamer::take_first_texels`), by slot: an object followed
    /// by a unit is looked at; any other face using one leaves the grid the
    /// shader drew for it through a short fade into its texture.
    pub(super) fn rez_first_texels(&mut self, renderer: &mut Renderer, slots: &[u32]) {
        if !self.rez.enabled || slots.is_empty() {
            return;
        }
        let now = Instant::now();
        // (no bounds: the shader takes the face's own, `rez_height`)
        let fade = record_value(rez_mode::FADE, renderer.anim_clock().ms(now), FADE, 0.0, 0.0);
        let watching = self.rez.watching_since.is_some();
        let mut users = Vec::new();
        for &slot in slots {
            users.clear();
            users.extend(self.slot_users.range((slot, 0)..=(slot, u32::MAX)).map(|&(_, i)| i as usize));
            for &idx in &users {
                if self.rez.member_of.contains_key(&idx) {
                    self.rez.note_ready(idx);
                    continue;
                }
                // (behind a loading screen it simply shows)
                let Some(g) = self.gpu.get(idx).filter(|_| watching) else {
                    continue;
                };
                for f in g.faces.iter().filter(|f| f.base_slot == slot) {
                    if renderer.records.get(f.record).is_some_and(|r| r.rez[1] == rez_mode::NONE) {
                        renderer.records.set_rez(f.record, fade);
                        self.rez.face_fades.push((f.record, fade, now + FADE + SWEEP_TAIL));
                        self.rez.stats.face_fades += 1;
                    }
                }
            }
        }
    }

    /// A unit's grid starts now: bounds for its gradient, the records, its
    /// glow when it is near enough for a line.
    fn start_hologram(&mut self, renderer: &mut Renderer, rez: &mut Rez, f: &Frame, root: usize) {
        let near = self.rez_visible(f.view, root).0;
        let Some(u) = rez.units.get_mut(&root) else {
            return;
        };
        if u.avatar {
            u.members = Self::rez_members(f.world, root);
        }
        let members = u.members.clone();
        let avatar = u.avatar;
        let (bottom, height) = self.unit_bounds(f.world, root, &members, avatar, f.now).unwrap_or((0.0, 1.0));
        let style = rez.style_of(root);
        let Some(u) = rez.units.get_mut(&root) else {
            return;
        };
        u.phase = Phase::Hologram(f.now);
        u.value = with_style(record_value(rez_mode::HOLOGRAM, f.ms, Duration::ZERO, bottom, height), style);
        u.glow = near;
        let (v, glow) = (u.value, u.glow);
        for &m in &members {
            rez.member_of.insert(m, root);
        }
        rez.stats.holograms += 1;
        self.write_rez(renderer, &members, v, glow);
    }

    /// Look at a unit not sweeping yet and move it on. Whether it stays in
    /// the queue.
    fn rez_look_at(&mut self, renderer: &mut Renderer, rez: &mut Rez, f: &Frame, root: usize) -> bool {
        let Some(u) = rez.units.get(&root) else {
            return false;
        };
        if matches!(u.phase, Phase::Sweep(_) | Phase::Fade(_) | Phase::Derez(_)) {
            return false;
        }
        if f.world.objects.get(root).is_none_or(|o| o.full_id != u.id) {
            if let Some(u) = rez.remove_unit(root) {
                self.write_rez(renderer, &u.members, [0; 4], false);
            }
            return false;
        }
        let loaded = if u.avatar {
            let own = u.id == f.world.agent_id;
            !self.loading.indices.contains(&root)
                && self.gpu.get(root).is_some_and(|g| !g.faces.is_empty())
                && (!own || self.avatar_complete(f.world, root))
                && self.members_loaded(u.members.get(1..).unwrap_or_default())
        } else {
            self.members_loaded(&u.members)
        };
        let (near, _) = self.rez_visible(f.view, root);
        let age = f.now.duration_since(u.since.max(f.watching_since));
        let in_phase = match u.phase {
            Phase::Hologram(t) => f.now.duration_since(t),
            _ => Duration::ZERO,
        };
        let mut step = next_step(u.phase, u.avatar, u.rezzed, loaded, near, f.watching, age, in_phase);
        let quick = step == Step::Quick;
        if quick {
            step = Step::Sweep;
        }
        if step == Step::Sweep && rez.sweeping.len() >= MAX_SWEEPS {
            // every line busy: the grid waits a little for one, then fades
            let Some(u) = rez.units.get_mut(&root) else {
                return false;
            };
            let waited = *u.waiting_line.get_or_insert(f.now);
            let wait = if u.avatar { AVATAR_SLOT_WAIT } else { SLOT_WAIT };
            step = if u.phase == Phase::Waiting {
                // (a quick line for what was ready at once, or nothing)
                if u.avatar { Step::Hologram } else { Step::Drop }
            } else if f.now.duration_since(waited) >= wait {
                rez.stats.no_line += 1;
                Step::Fade
            } else {
                Step::Stay
            };
        }
        let avatar = u_avatar(rez, root);
        if avatar && matches!(step, Step::Sweep | Step::Fade) && self.loading.indices.contains(&root) {
            // shown by the grid's time limit while still loading
            rez.revealed.insert(root);
        }
        match step {
            Step::Stay => {
                // an avatar in the grid: what it wears joins it as it comes
                if let Some(u) = rez.units.get_mut(&root)
                    && u.avatar
                    && matches!(u.phase, Phase::Hologram(_))
                {
                    let members = Self::rez_members(f.world, root);
                    let (v, glow) = (u.value, u.glow);
                    let new: Vec<usize> = members.iter().copied().filter(|m| !u.members.contains(m)).collect();
                    u.members = members;
                    for &m in &new {
                        rez.member_of.insert(m, root);
                    }
                    self.write_rez(renderer, &new, v, glow);
                }
                true
            }
            Step::Drop => {
                if let Some(u) = rez.remove_unit(root) {
                    self.write_rez(renderer, &u.members, [0; 4], false);
                }
                false
            }
            Step::Hologram => {
                self.start_hologram(renderer, rez, f, root);
                true
            }
            Step::Fade => {
                let style = rez.style_of(root);
                let Some(u) = rez.units.get_mut(&root) else {
                    return false;
                };
                u.phase = Phase::Fade(f.now);
                u.sweep = FADE;
                // (the grid's bounds: its gradient fades as it was)
                let (bottom, height) = (f32::from_bits(u.value[2]), f32::from_bits(u.value[3]));
                u.value = with_style(record_value(rez_mode::FADE, f.ms, FADE, bottom, height), style);
                u.glow = false;
                let (members, v) = (u.members.clone(), u.value);
                rez.stats.fades += 1;
                self.write_rez(renderer, &members, v, false);
                rez.fading.push(root);
                false
            }
            Step::Quick | Step::Sweep => {
                let Some(u) = rez.units.get_mut(&root) else {
                    return false;
                };
                if u.avatar {
                    // what it wears now
                    u.members = Self::rez_members(f.world, root);
                }
                let (members, avatar) = (u.members.clone(), u.avatar);
                let Some((bottom, height)) = self.unit_bounds(f.world, root, &members, avatar, f.now) else {
                    if let Some(u) = rez.remove_unit(root) {
                        self.write_rez(renderer, &u.members, [0; 4], false);
                    }
                    return false;
                };
                let style = rez.style_of(root);
                let Some(u) = rez.units.get_mut(&root) else {
                    return false;
                };
                u.phase = Phase::Sweep(f.now);
                u.sweep = if quick { QUICK_SWEEP } else { sweep_for(height) };
                u.value = with_style(record_value(rez_mode::SWEEP, f.ms, u.sweep, bottom, height), style);
                u.glow = true;
                rez.stats.sweeps += 1;
                if rez.log {
                    log::info!(
                        "rez effect: line over {} ({} prims, {height:.2} m high{})",
                        u.id,
                        u.members.len(),
                        if u.avatar { ", avatar" } else { "" }
                    );
                }
                let v = u.value;
                for &m in &members {
                    rez.member_of.insert(m, root);
                }
                self.write_rez(renderer, &members, v, true);
                rez.sweeping.push(root);
                false
            }
        }
    }

    /// Once a frame, after the sync and the avatars' loading: new units
    /// from what arrived, the next phase of those followed, their records.
    /// `covered`: a loading or teleport screen hides the world.
    pub fn update_rez(
        &mut self,
        renderer: &mut Renderer,
        world: &crate::world::World,
        view: &super::CullView,
        covered: bool,
        now: Instant,
    ) {
        let mut rez = std::mem::take(&mut self.rez);
        for m in rez.to_clear.drain(..) {
            self.write_rez(renderer, &[m], [0; 4], false);
        }
        // faces done fading into their texture (a record built again or
        // taken by another face since holds something else: left alone)
        rez.face_fades.retain(|&(record, value, until)| {
            if now < until && rez.enabled {
                return true;
            }
            if renderer.records.get(record).is_some_and(|r| r.rez == value) {
                renderer.records.set_rez(record, [0; 4]);
            }
            false
        });
        if !rez.enabled {
            let roots: Vec<usize> = rez.units.keys().copied().collect();
            for root in roots {
                if let Some(u) = rez.remove_unit(root) {
                    self.write_rez(renderer, &u.members, [0; 4], false);
                }
            }
            rez.queue.clear();
            rez.sweeping.clear();
            rez.fading.clear();
            rez.arrivals.clear();
            rez.ready.clear();
            rez.revealed.clear();
            rez.watching_since = None;
            rez.stats.units = 0;
            self.rez = rez;
            return;
        }
        let watching = !covered;
        let watching_since = if watching {
            *rez.watching_since.get_or_insert(now)
        } else {
            rez.watching_since = None;
            now
        };
        let watched_for = watching.then(|| now.duration_since(watching_since));
        let f = Frame {
            world,
            view,
            now,
            ms: renderer.anim_clock().ms(now),
            watching,
            watching_since,
        };

        // avatars coming into view (behind a loading screen too: they get
        // the grid or the line once it lifts)
        let mut present = HashSet::new();
        let lifted = watching && watched_for == Some(Duration::ZERO);
        for (idx, o) in world.objects.avatars() {
            present.insert(o.full_id);
            let look = Self::own_look(world, idx);
            let ours = o.full_id == world.agent_id;
            let start = match rez.avatars_seen.get_mut(&o.full_id) {
                None => {
                    rez.avatars_seen.insert(o.full_id, (now, look));
                    true
                }
                Some((since, seen)) => {
                    // an outfit change (its bakes), once it has been there
                    // a while: one line over it; ours also when the
                    // loading or teleport screen lifts
                    let changed = *seen != look && watching && now.duration_since(*since) >= WORN_AFTER;
                    *seen = look;
                    if changed {
                        rez.stats.outfits += 1;
                    }
                    changed || ours && lifted
                }
            };
            if start && !rez.units.contains_key(&idx) {
                rez.add_unit(idx, Self::avatar_unit(o.full_id, idx, now));
            }
        }
        rez.avatars_seen.retain(|id, _| present.contains(id));
        let revealed: Vec<usize> = rez.revealed.iter().copied().collect();
        for idx in revealed {
            // (back in the store's avatars, or gone: the slot is someone else's)
            if world.objects.get(idx).is_none_or(|o| !o.is_avatar()) {
                rez.revealed.remove(&idx);
            }
        }
        rez.worn_lately.retain(|_, l| {
            l.retain(|(t, _)| now.duration_since(*t) < OUTFIT_WINDOW);
            !l.is_empty()
        });

        // objects synced for the first time
        for idx in std::mem::take(&mut rez.arrivals) {
            let Some(o) = world.objects.get(idx) else {
                continue;
            };
            let region_since = *rez.regions.entry(o.key.region).or_insert(now);
            if rez.member_of.contains_key(&idx) || rez.units.contains_key(&idx) {
                continue;
            }
            if o.parent_id != 0 {
                // a prim of a linkset followed already (or worn: its
                // avatar's effect takes it while it runs)
                let parent = world.objects.parent_of(o);
                // (the parent may be a prim of the unit, as the root of
                // something worn by an avatar followed)
                let unit_root = parent.and_then(|p| {
                    if rez.units.contains_key(&p) {
                        Some(p)
                    } else {
                        rez.member_of.get(&p).copied()
                    }
                });
                if let Some(r) = unit_root
                    && let Some(u) = rez.units.get_mut(&r)
                {
                    u.members.push(idx);
                    let (v, glow) = (u.value, u.glow);
                    rez.member_of.insert(idx, r);
                    self.write_rez(renderer, &[idx], v, glow);
                    continue;
                }
                // put on by an avatar there for a while: it rezzes on it,
                // or the avatar does when it is an outfit change
                let Some(avatar) = parent.filter(|&p| world.objects.get(p).is_some_and(|a| a.is_avatar())) else {
                    continue;
                };
                let settled = world
                    .objects
                    .get(avatar)
                    .and_then(|a| rez.avatars_seen.get(&a.full_id))
                    .is_some_and(|(since, _)| now.duration_since(*since) >= WORN_AFTER);
                if !watching || !settled || (31..=38).contains(&o.attachment_point()) || !self.rez_visible(view, avatar).0 {
                    continue;
                }
                let lately = rez.worn_lately.entry(avatar).or_default();
                lately.push((now, idx));
                if lately.len() >= OUTFIT_ITEMS {
                    let items: Vec<usize> = lately.iter().map(|&(_, i)| i).collect();
                    for item in items {
                        if let Some(u) = rez.remove_unit(item) {
                            self.write_rez(renderer, &u.members, [0; 4], false);
                        }
                    }
                    let Rez {
                        queue,
                        sweeping,
                        fading,
                        units,
                        ..
                    } = &mut rez;
                    queue.retain(|r| units.contains_key(r));
                    sweeping.retain(|r| units.contains_key(r));
                    fading.retain(|r| units.contains_key(r));
                    if let Some(a) = world.objects.get(avatar) {
                        rez.stats.outfits += 1;
                        rez.add_unit(avatar, Self::avatar_unit(a.full_id, avatar, now));
                    }
                    continue;
                }
                rez.stats.worn += 1;
                rez.add_unit(idx, self.object_unit(world, idx, o.full_id, true, now));
                if !self.members_loaded(&Self::rez_members(world, idx)) {
                    self.start_hologram(renderer, &mut rez, &f, idx);
                }
                continue;
            }
            // (not placed yet, as a prim waiting for its parent: no rez near
            // us, its own flag aside)
            let distance = Self::object_transform(world, idx, now, 0).map_or(f32::MAX, |(pos, _, _)| pos.distance(view.eye));
            let settled_for = watched_for.map(|w| w.min(now.duration_since(region_since)));
            let rezzed = is_fresh_rez(o.update_flags, settled_for, distance, o.velocity.length());
            // (an object streaming in needs nothing here: its faces still
            // grey are the grid on their own)
            if !rezzed {
                continue;
            }
            let members = Self::rez_members(world, idx);
            let loaded = self.members_loaded(&members);
            if rezzed {
                rez.stats.rezzed += 1;
                if rez.log {
                    log::info!("rez effect: object {} rezzed {distance:.1} m away", o.full_id);
                }
            }
            rez.add_unit(idx, self.object_unit(world, idx, o.full_id, rezzed, now));
            if !loaded {
                self.start_hologram(renderer, &mut rez, &f, idx);
            }
        }

        // lines and fades running
        for list in [false, true] {
            let mut i = 0;
            loop {
                let roots = if list { &mut rez.fading } else { &mut rez.sweeping };
                let Some(&root) = roots.get(i) else {
                    break;
                };
                let done = rez.units.get(&root).is_none_or(|u| match u.phase {
                    Phase::Sweep(s) | Phase::Derez(s) | Phase::Fade(s) => now.duration_since(s) >= u.sweep + SWEEP_TAIL,
                    _ => true,
                });
                if done {
                    let roots = if list { &mut rez.fading } else { &mut rez.sweeping };
                    roots.swap_remove(i);
                    if let Some(u) = rez.remove_unit(root) {
                        // (a derez leaves its slots to `collect_garbage`)
                        if !matches!(u.phase, Phase::Derez(_)) {
                            self.write_rez(renderer, &u.members, [0; 4], false);
                        }
                    }
                } else {
                    i += 1;
                }
            }
        }

        // what may have finished loading, then the others in turn (for
        // their time limits)
        let mut ready = std::mem::take(&mut rez.ready);
        ready.sort_unstable();
        ready.dedup();
        for root in ready {
            self.rez_look_at(renderer, &mut rez, &f, root);
        }
        let mut checks = 0;
        let mut looked = 0;
        let queued = rez.queue.len();
        while looked < queued && checks < CHECKS_PER_FRAME {
            looked += 1;
            let Some(root) = rez.queue.pop_front() else {
                break;
            };
            checks += rez.units.get(&root).map_or(0, |u| u.members.len().max(1));
            if self.rez_look_at(renderer, &mut rez, &f, root) {
                rez.queue.push_back(root);
            }
        }
        rez.stats.units = rez.units.len();
        // (what it did, every half minute, for the grid tests)
        if rez.logged_at.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(30)) {
            rez.logged_at = Some(now);
            let s = rez.stats;
            log::info!(
                "rez effect: {} faces out of the grid, {} grids, {} lines, {} fades ({} with every line busy), {} derez, {} worn, {} outfits, {} followed now, {} avatars shown by the time limit",
                s.face_fades,
                s.holograms,
                s.sweeps,
                s.fades,
                s.no_line,
                s.derezzed,
                s.worn,
                s.outfits,
                s.units,
                rez.revealed.len()
            );
        }
        // what the effect draws is neither a cloud nor hidden
        self.loading.indices.retain(|&i| !rez.shows_avatar(i));
        self.rez = rez;
    }

    /// A unit for an object (a root) arriving or put on.
    fn object_unit(&self, world: &crate::world::World, idx: usize, id: Uuid, rezzed: bool, now: Instant) -> Unit {
        Unit {
            id,
            avatar: false,
            rezzed,
            since: now,
            sweep: Duration::ZERO,
            phase: Phase::Waiting,
            members: Self::rez_members(world, idx),
            value: [0; 4],
            glow: false,
            waiting_line: None,
        }
    }
}

/// Whether unit `root` is an avatar's.
fn u_avatar(rez: &Rez, root: usize) -> bool {
    rez.units.get(&root).is_some_and(|u| u.avatar)
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);
    const MS100: Duration = Duration::from_millis(100);

    #[test]
    fn the_record_value_packs_mode_and_bounds() {
        let v = record_value(rez_mode::SWEEP, 1234, Duration::from_millis(640), 21.5, 1.8);
        assert_eq!(v[0], 1234);
        assert_eq!(v[1] & 0xFF, rez_mode::SWEEP);
        assert_eq!(v[1] >> 8, 640);
        let styled = with_style(v, 3);
        assert_eq!(styled[1] & 0xFF, rez_mode::SWEEP);
        assert_eq!(styled[1] >> 8 & 0xFFF, 640);
        assert_eq!(styled[1] >> 20, 3);
        assert_eq!(f32::from_bits(v[2]), 21.5);
        assert_eq!(f32::from_bits(v[3]), 1.8);
    }

    #[test]
    fn a_rez_is_ours_by_its_flag_or_new_near_us_in_a_watched_region() {
        let create = crate::build::FLAGS_CREATE_SELECTED;
        // ours, flagged: even right after arriving, a little farther
        assert!(is_fresh_rez(create, None, 30.0, 0.0));
        // someone else's: near, once the region has streamed in
        assert!(is_fresh_rez(0, Some(SETTLED), 10.0, 0.0));
        assert!(!is_fresh_rez(0, Some(SETTLED - Duration::from_millis(1)), 10.0, 0.0));
        assert!(!is_fresh_rez(0, Some(SETTLED), REZ_NEAR + 1.0, 0.0));
        assert!(!is_fresh_rez(0, None, 10.0, 0.0), "behind a loading screen");
        // bullets and temporary debris never
        assert!(!is_fresh_rez(FLAGS_TEMPORARY_ON_REZ | create, Some(SETTLED), 1.0, 0.0));
        assert!(!is_fresh_rez(FLAGS_USE_PHYSICS, Some(SETTLED), 1.0, 30.0));
        assert!(is_fresh_rez(FLAGS_USE_PHYSICS, Some(SETTLED), 1.0, 0.5), "a physical box dropped");
    }

    #[test]
    fn behind_the_loading_screen_what_is_ready_just_shows() {
        let h = Phase::Hologram(Instant::now());
        assert_eq!(next_step(h, false, false, true, true, false, S, S), Step::Drop);
        assert_eq!(next_step(h, false, false, false, true, false, S, S), Step::Stay);
        // (an avatar waits for the screen to lift)
        assert_eq!(next_step(Phase::Waiting, true, false, true, true, false, S, S), Step::Stay);
    }

    #[test]
    fn the_grid_fades_or_gets_the_line_once_loaded() {
        let h = Phase::Hologram(Instant::now());
        // still loading: the grid stays, far or near
        assert_eq!(next_step(h, false, false, false, true, true, S, S), Step::Stay);
        assert_eq!(next_step(h, false, false, false, false, true, S, S), Step::Stay);
        // from the caches, within a moment: a fade, no line
        assert_eq!(next_step(h, false, false, true, true, true, MS100, MS100), Step::Fade);
        // loaded later: the line when near, else a fade
        assert_eq!(next_step(h, false, false, true, true, true, S, S), Step::Sweep);
        assert_eq!(next_step(h, false, false, true, false, true, S, S), Step::Fade);
        // still loading at the limit: shown anyway
        assert_eq!(
            next_step(h, false, false, false, true, true, OBJECT_LIMIT, OBJECT_LIMIT),
            Step::Sweep
        );
        assert_eq!(
            next_step(h, false, false, false, false, true, OBJECT_LIMIT, OBJECT_LIMIT),
            Step::Fade
        );
        // a rezzed object shows its grid a moment, then its line
        assert_eq!(next_step(h, false, true, true, true, true, S, MS100), Step::Stay);
        assert_eq!(next_step(h, false, true, true, true, true, S, MIN_HOLOGRAM), Step::Sweep);
    }

    #[test]
    fn a_rezzed_object_ready_at_once_gets_a_quick_line() {
        let w = Phase::Waiting;
        assert_eq!(
            next_step(w, false, true, true, true, true, Duration::ZERO, Duration::ZERO),
            Step::Quick
        );
        // far: nothing (it is ready: no grey to hide)
        assert_eq!(
            next_step(w, false, true, true, false, true, Duration::ZERO, Duration::ZERO),
            Step::Drop
        );
        assert_eq!(
            next_step(w, false, true, false, true, true, Duration::ZERO, Duration::ZERO),
            Step::Hologram
        );
    }

    #[test]
    fn an_avatar_loads_in_the_grid_then_gets_the_line() {
        let w = Phase::Waiting;
        // loading in view: the grid instead of the cloud
        assert_eq!(next_step(w, true, false, false, true, true, S, Duration::ZERO), Step::Hologram);
        assert_eq!(
            next_step(w, true, false, true, true, true, Duration::ZERO, Duration::ZERO),
            Step::Sweep
        );
        // out of view: the cloud as before, until it comes into view
        assert_eq!(next_step(w, true, false, true, false, true, S, Duration::ZERO), Step::Stay);
        assert_eq!(
            next_step(w, true, false, false, false, true, AVATAR_LIMIT + S, Duration::ZERO),
            Step::Drop
        );
        let h = Phase::Hologram(Instant::now());
        assert_eq!(next_step(h, true, false, false, true, true, S, S), Step::Stay);
        assert_eq!(next_step(h, true, false, true, true, true, S, S), Step::Sweep);
        assert_eq!(next_step(h, true, false, true, false, true, S, S), Step::Fade);
        // still loading at the grid's limit: shown anyway
        let long = AVATAR_GRID_LIMIT;
        assert_eq!(next_step(h, true, false, false, true, true, long, long), Step::Sweep);
    }

    #[test]
    fn a_taller_object_takes_a_little_longer() {
        assert_eq!(sweep_for(0.3), SWEEP_MIN);
        assert!(sweep_for(1.8) > SWEEP_MIN && sweep_for(1.8) < SWEEP_MAX);
        assert_eq!(sweep_for(40.0), SWEEP_MAX);
    }

    #[test]
    fn the_height_of_a_turned_box() {
        let s = Vec3::new(4.0, 2.0, 0.5);
        assert!((half_height(s, Quat::IDENTITY) - 0.25).abs() < 1e-5);
        // on its side, its 2 m stand up
        let r = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        assert!((half_height(s, r) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_unit_forgotten_clears_its_other_prims() {
        let mut rez = Rez {
            enabled: true,
            ..Default::default()
        };
        rez.add_unit(
            1,
            Unit {
                id: Uuid::from_u128(1),
                avatar: false,
                rezzed: true,
                since: Instant::now(),
                sweep: Duration::ZERO,
                phase: Phase::Waiting,
                members: vec![1, 2, 3],
                value: record_value(rez_mode::HOLOGRAM, 0, Duration::ZERO, 0.0, 1.0),
                glow: false,
                waiting_line: None,
            },
        );
        assert!(rez.value_of(3).is_some());
        // a texture of prim 2 showed: its root is looked at
        rez.note_ready(2);
        assert_eq!(rez.ready, [1]);
        rez.forget(3);
        assert!(rez.value_of(3).is_none(), "a prim gone");
        assert!(rez.value_of(2).is_some());
        rez.forget(1);
        assert!(rez.value_of(2).is_none(), "its root gone");
        assert_eq!(rez.to_clear, [2]);
    }
}

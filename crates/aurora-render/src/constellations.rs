//! Constellations over the real night sky: the engraved figures, their
//! stick lines and their names, drawn by the sky shader next to the stars
//! (`constellation_overlay` in constellations.wgsl). Local rendering only:
//! nothing comes from or goes to the grid.
//!
//! The viewer (aurora-viewer world/constellations.rs) decides what shows and
//! sends an [`Overlay`] with each frame (`FrameParams::constellations`):
//!
//! - once, the static tables ([`build_tables`]): the drawings' frames (a
//!   figure may have one from each art set), the figures' line segments and
//!   label points, sorted into the stars' cube cells (stars.rs `cell_of`),
//!   so that a sky pixel only looks at what can cover its own cell;
//! - per frame, the state of each figure ([`FrameState`]: how far its lines
//!   have come, its opacity, its name's, where its name sits in the atlas)
//!   and of each drawing (`Overlay::drawings`, written into the tables'
//!   end: how far its pen stroke has come, its opacity, where it sits in
//!   the atlas), so that one figure's outgoing drawing can fade out while
//!   the incoming one draws itself in;
//! - the pictures and names to write into the atlas ([`Upload`]): an `Rg8`
//!   texture array of `ATLAS_W` x `ATLAS_H` layers with `ATLAS_MIPS` levels,
//!   made the first time a figure shows and let go when nothing has shown
//!   for a while (`layers` 0). Each picture holds its ink coverage (red) and
//!   the moment of the pen stroke that reveals it (green); each name its
//!   text coverage (red) and a soft glow around it (green).
//!
//! The backend half ([`Gpu`]) owns the uniform, the table buffer and the
//! atlas, writes them on the render thread and tells the renderer when the
//! frame groups must be bound again. Off (no overlay, or nothing showing):
//! the uniform says so once and the shader returns at its first test; no
//! texture is sampled and nothing is uploaded.

use crate::stars::{CELL_TABLE, CELLS, cell_of, equatorial};
use glam::Vec3;
use std::sync::Arc;

/// Figures the uniform has room for, every culture of the data together
/// (a figure's number is its place in the viewer's catalogue). Must match
/// `CONSTELLATION_FIGURES` (constellations.wgsl). Two `vec4` each: 1536 make
/// the uniform 49 KiB, within the 64 KiB a uniform binding may have on every
/// GPU (`UNIFORM_LIMIT`); more figures would need their state in a storage
/// buffer. Only the figures in use are written each frame
/// (`Overlay::slots`). Drawings have no such limit: their state is in the
/// tables.
pub const MAX_FIGURES: usize = 1536;
/// The smallest `max_uniform_buffer_binding_size` of the GPUs the viewer
/// runs on (64 KiB): the uniform must fit it.
pub const UNIFORM_LIMIT: usize = 64 << 10;
const _: () = assert!(FRAME_BYTES <= UNIFORM_LIMIT, "the uniform must fit every GPU's binding");
/// Atlas layers: width, height, mip levels. Pictures of every size share
/// them, placed in blocks; one larger than a layer never shows (its figure
/// draws its lines). Must match `CONSTELLATION_ATLAS` (constellations.wgsl).
pub const ATLAS_W: u32 = 4096;
pub const ATLAS_H: u32 = 2048;
pub const ATLAS_MIPS: u32 = 3;
/// Atlas layers at most: the memory budget (16 MiB each, 21 MiB with the
/// mips). A view whose figures need more draws the farthest ones' lines
/// and names only until room frees.
// 6: hundreds of names of several cultures in a wide field; the viewer
// keeps to 3 while it lists as few figures as one culture's (its
// constellations_atlas.rs `FEW_LAYERS`)
pub const MAX_LAYERS: u32 = 6;
/// GPU memory of one atlas layer with its mips (Rg8).
pub const ATLAS_LAYER_BYTES: usize = (ATLAS_W * ATLAS_H * 2) as usize * 4 / 3;
/// Placements are made in blocks of this many texels (aligned for every
/// mip level). Must match `CONSTELLATION_BLOCK` (constellations.wgsl).
pub const BLOCK: u32 = 64;
/// How far a name may reach from its label point (radians, 10°): the cells
/// it is listed in. The viewer keeps a name within it.
pub const NAME_REACH: f32 = 0.1745;
/// How far a line's drawing reaches from its arc (radians): its widest
/// half-width and soft edge.
pub const LINE_REACH: f32 = 0.006;
/// The gap a line leaves at each of its stars (radians, 0.45°), and the
/// largest share of the line's own length it may take (constellations.wgsl
/// CONSTELLATION_GAP, CONSTELLATION_GAP_SHARE).
pub const LINE_GAP: f32 = 0.00785;
pub const LINE_GAP_SHARE: f32 = 0.15;

/// The gap a line of `arc` radians leaves at each end (constellations.wgsl
/// constellation_gap): 0.45°, or `LINE_GAP_SHARE` of a short line, so that
/// its two gaps never take more than 30 % of it and it stays a line, not a
/// dash. Lines of 3° and more keep the full gap.
pub fn line_gap(arc: f32) -> f32 {
    LINE_GAP.min(LINE_GAP_SHARE * arc.max(0.0))
}
/// How far a sparkle at a line's pen tip reaches (radians, about 1.1°):
/// the second cell lists (`Bases::wide_cells`) list each line this far
/// around its arc and past its ends; the shader reads them instead of the
/// first ones only while the sparkles are on, so that the overlay costs no
/// more with them off.
pub const SPARKLE_REACH: f32 = 0.02;
/// `FrameState::head` y: its bits. Drawings on (the shader's first test),
/// a colour per figure (`FrameState::figures` w), the sparkles.
pub const HEAD_DRAWINGS: u32 = 1;
pub const HEAD_TINTS: u32 = 2;
pub const HEAD_SPARKLES: u32 = 4;
/// `FrameState::style` z: the sparkles' shape; `SHAPE_MIX` (the demo's
/// comparison) gives each figure one of the three by its number.
pub const SHAPE_CIRCLE_CROSS: f32 = 0.0;
pub const SHAPE_CROSS: f32 = 1.0;
pub const SHAPE_STAR: f32 = 2.0;
pub const SHAPE_MIX: f32 = 3.0;
/// `FrameState::head` x: its bits. The figures show (the overlay's first
/// test), the planets' names show (`FrameState::planets`, their own test).
pub const HEAD_FIGURES: u32 = 1;
pub const HEAD_PLANET_NAMES: u32 = 2;
/// The planets that may carry a name (world/planets.rs `Body::ALL`). Must
/// match `CONSTELLATION_PLANETS` (constellations.wgsl).
pub const PLANET_NAMES: usize = 6;
/// Bytes of the uniform (`ConstellationFrame` in constellations.wgsl).
pub const FRAME_BYTES: usize = std::mem::size_of::<FrameState>();

/// Kinds of the cell lists' references (top two bits; constellations.wgsl).
const REF_PICTURE: u32 = 0;
const REF_SEGMENT: u32 = 1;
const REF_NAME: u32 = 2;
/// `vec4` per drawing record in the tables.
const DRAWING_VEC4: usize = 5;

/// One figure (a constellation) as the tables need it, angles in radians:
/// what every art set shares.
#[derive(Clone, Debug, PartialEq)]
pub struct FigureGeom {
    pub centre: [f32; 2],
    /// Its name's centre.
    pub label: [f32; 2],
    /// Stick lines: right ascension and declination of both ends, and the
    /// share of the figure's drawing time each starts and ends at.
    pub lines: Vec<Line>,
}

/// One drawing of a figure (from one art set): its picture's frame as the
/// art describes it (texel (i, j) centred at origin - (i + 0.5, j + 0.5) x
/// texel in the stereographic plane about `centre`, x east, y north), and
/// the set's own label point for the name, if it moves it.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawingGeom {
    pub figure: u32,
    pub centre: [f32; 2],
    pub picture: Picture,
    pub label: Option<[f32; 2]>,
}

/// Where each part of the tables starts (`vec4` index), for the uniform.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bases {
    pub refs: u32,
    /// The second cell table and its references: the lines listed as far
    /// as `SPARKLE_REACH` (the first table is at 0).
    pub wide_cells: u32,
    pub wide_refs: u32,
    pub drawings: u32,
    pub segments: u32,
    /// One `vec4` per figure: its shared label point.
    pub labels: u32,
    /// One `vec4` per drawing, written every frame it changes.
    pub states: u32,
    pub drawing_count: u32,
    /// The longest cell list of each table (references): the shader reads
    /// at most `MAX_REFS` of a cell.
    pub longest: u32,
    pub wide_longest: u32,
}

/// References the shader reads in one cell at most (constellations.wgsl
/// `CONSTELLATION_MAX_REFS`): a guard against a bad table, well above the
/// longest list of every culture of the data drawn together.
pub const MAX_REFS: u32 = 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Picture {
    pub texel: f32,
    pub size: [u32; 2],
    pub origin: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub t0: f32,
    pub t1: f32,
}

/// The figure's tangent frame: centre, east and north unit vectors.
pub fn tangent_frame(ra: f32, dec: f32) -> [Vec3; 3] {
    let c = equatorial(ra, dec);
    let e = Vec3::new(-ra.sin(), ra.cos(), 0.0);
    let n = c.cross(e);
    [c, e, n]
}

/// Stereographic plane point (x east, y north) about `frame` back to a
/// unit direction (preview.py's `stereo_inv`).
pub fn plane_to_dir(frame: &[Vec3; 3], x: f32, y: f32) -> Vec3 {
    let r2 = x * x + y * y;
    ((frame[1] * x + frame[2] * y) * 4.0 + frame[0] * (4.0 - r2)) / (4.0 + r2)
}

/// The tables of the shader (`constellation_data`, `vec4<u32>` elements):
/// the cell table (`CELL_TABLE` offsets into the references), then the
/// references, the drawing records, the segment records, the figures'
/// label points and room for the drawings' states (zero: nothing shows).
pub fn build_tables(figures: &[FigureGeom], drawings: &[DrawingGeom]) -> (Vec<u8>, Bases) {
    build_tables_of(figures, drawings, None)
}

/// [`build_tables`] with only the figures `listed` says (by number; `None`
/// all) in the cell lists: their lines, names and drawings. Every figure
/// keeps its number, its label point and its drawings' records and states,
/// so a figure listed or not later is the same figure; one not listed is
/// never read by the shader, and costs it nothing.
pub fn build_tables_of(figures: &[FigureGeom], drawings: &[DrawingGeom], listed: Option<&[bool]>) -> (Vec<u8>, Bases) {
    let (records, segments, labels, lists) = cell_lists(figures, drawings, LINE_REACH, 0.0, listed);
    let (_, _, _, wide) = cell_lists(figures, drawings, SPARKLE_REACH, SPARKLE_REACH, listed);
    let longest = |lists: &[Vec<u32>]| lists.iter().map(Vec::len).max().unwrap_or(0) as u32;
    let (longest, wide_longest) = (longest(&lists), longest(&wide));
    // a cell table (`CELL_TABLE` offsets, in u32, into its references) and
    // its references, padded to a vec4
    let table_of = |lists: &[Vec<u32>]| {
        let mut table: Vec<u32> = Vec::with_capacity(CELL_TABLE as usize);
        let mut refs: Vec<u32> = Vec::new();
        for list in lists {
            table.push(refs.len() as u32);
            refs.extend_from_slice(list);
        }
        table.push(refs.len() as u32);
        table.resize(CELL_TABLE as usize, refs.len() as u32);
        while !refs.len().is_multiple_of(4) {
            refs.push(0);
        }
        (table, refs)
    };
    let (table, refs) = table_of(&lists);
    let (wide_table, wide_refs) = table_of(&wide);
    let states = vec![[0.0f32; 4]; drawings.len().max(1)];
    let mut bases = Bases {
        refs: CELL_TABLE / 4,
        drawing_count: drawings.len() as u32,
        longest,
        wide_longest, // (the lists' lengths)
        ..Bases::default()
    };
    bases.wide_cells = bases.refs + refs.len() as u32 / 4;
    bases.wide_refs = bases.wide_cells + CELL_TABLE / 4;
    bases.drawings = bases.wide_refs + wide_refs.len() as u32 / 4;
    bases.segments = bases.drawings + records.len() as u32;
    bases.labels = bases.segments + segments.len() as u32;
    bases.states = bases.labels + labels.len() as u32;
    let mut bytes = Vec::with_capacity(bases.states as usize * 16 + states.len() * 16);
    bytes.extend_from_slice(bytemuck::cast_slice(&table));
    bytes.extend_from_slice(bytemuck::cast_slice(&refs));
    bytes.extend_from_slice(bytemuck::cast_slice(&wide_table));
    bytes.extend_from_slice(bytemuck::cast_slice(&wide_refs));
    bytes.extend_from_slice(bytemuck::cast_slice(&records));
    bytes.extend_from_slice(bytemuck::cast_slice(&segments));
    bytes.extend_from_slice(bytemuck::cast_slice(&labels));
    bytes.extend_from_slice(bytemuck::cast_slice(&states));
    (bytes, bases)
}

/// The drawing records, segment records and label points, and each cell's
/// list of references, the lines listed `line_reach` around their arcs and
/// `past_ends` beyond their two ends (radians); only the figures `listed`
/// says in the lists and segments.
#[allow(clippy::type_complexity)]
fn cell_lists(
    figures: &[FigureGeom],
    drawings: &[DrawingGeom],
    line_reach: f32,
    past_ends: f32,
    listed: Option<&[bool]>,
) -> (Vec<[f32; 4]>, Vec<[u32; 4]>, Vec<[f32; 4]>, Vec<Vec<u32>>) {
    let on = |fig: usize| fig < MAX_FIGURES && listed.is_none_or(|l| l.get(fig).copied().unwrap_or(false));
    let mut lists: Vec<Vec<u32>> = vec![Vec::new(); CELLS as usize];
    let mut put = |cell: u32, r: u32| {
        let list = &mut lists[cell as usize];
        if list.last() != Some(&r) && !list.contains(&r) {
            list.push(r);
        }
    };
    // a name: a disc of NAME_REACH about a label point
    let put_name = |label: Vec3, fig: u32, put: &mut dyn FnMut(u32, u32)| {
        let side = if label.z.abs() < 0.9 { Vec3::Z } else { Vec3::X };
        let e1 = label.cross(side).normalize();
        let e2 = label.cross(e1);
        put(cell_of(label), REF_NAME << 30 | fig);
        for ring in 1..=6 {
            let r = NAME_REACH * ring as f32 / 6.0 * 1.05;
            let around = 8 * ring;
            for k in 0..around {
                let a = k as f32 / around as f32 * std::f32::consts::TAU;
                put(cell_of(label + (e1 * a.cos() + e2 * a.sin()) * r), REF_NAME << 30 | fig);
            }
        }
    };
    let mut records: Vec<[f32; 4]> = Vec::with_capacity(drawings.len() * DRAWING_VEC4);
    for (d, g) in drawings.iter().enumerate() {
        let d = d as u32;
        let frame = tangent_frame(g.centre[0], g.centre[1]);
        let p = g.picture;
        let label = g.label.map(|l| equatorial(l[0], l[1]));
        records.push(frame[0].extend(p.texel).to_array());
        records.push(frame[1].extend(p.origin[0]).to_array());
        records.push(frame[2].extend(p.origin[1]).to_array());
        records.push([p.size[0] as f32, p.size[1] as f32, f32::from_bits(g.figure), 0.0]);
        records.push(label.unwrap_or(Vec3::ZERO).extend(0.0).to_array());
        // listed with its figure only (its record and state either way)
        let listed = on(g.figure as usize);
        if listed && let Some(label) = label {
            put_name(label, g.figure, &mut put);
        }
        // the picture: points over its rectangle and a margin around it,
        // closer than a cell (about 3.75°) and the stars' margin
        if listed && p.texel > 0.0 && p.size[0] > 0 && p.size[1] > 0 {
            let (w, h) = (p.size[0] as f32 * p.texel, p.size[1] as f32 * p.texel);
            let pad = 0.01;
            let step = 0.008;
            let (nx, ny) = (
                ((w + 2.0 * pad) / step).ceil() as u32 + 1,
                ((h + 2.0 * pad) / step).ceil() as u32 + 1,
            );
            for j in 0..ny {
                for i in 0..nx {
                    let x = p.origin[0] + pad - (i as f32 * (w + 2.0 * pad) / (nx - 1) as f32);
                    let y = p.origin[1] + pad - (j as f32 * (h + 2.0 * pad) / (ny - 1) as f32);
                    put(cell_of(plane_to_dir(&frame, x, y)), REF_PICTURE << 30 | d);
                }
            }
        }
    }
    let mut segments: Vec<[u32; 4]> = Vec::new();
    let mut labels: Vec<[f32; 4]> = Vec::with_capacity(figures.len());
    for (fig, f) in figures.iter().enumerate().take(MAX_FIGURES) {
        let fig = fig as u32;
        let label = equatorial(f.label[0], f.label[1]);
        labels.push(label.extend(0.0).to_array());
        if !on(fig as usize) {
            // not listed: its label point only (no segment, no name)
            continue;
        }
        // the lines: points along each arc and on either side of it
        for l in &f.lines {
            let a = equatorial(l.a[0], l.a[1]);
            let b = equatorial(l.b[0], l.b[1]);
            let n = a.cross(b);
            if n.length() < 1e-6 {
                continue;
            }
            // the segment's number: two vec4 records each (constellations.wgsl)
            let index = (segments.len() / 2) as u32;
            let t0 = (l.t0.clamp(0.0, 1.0) * 65535.0).round() as u32;
            let t1 = (l.t1.clamp(0.0, 1.0) * 65535.0).round() as u32;
            segments.push([a.x.to_bits(), a.y.to_bits(), a.z.to_bits(), fig]);
            segments.push([b.x.to_bits(), b.y.to_bits(), b.z.to_bits(), t0 | t1 << 16]);
            let n = n.normalize();
            let arc = a.dot(b).clamp(-1.0, 1.0).acos();
            // along the arc (and past its ends: a sparkle there), from
            // `-ext` to `1 + ext` of it, every 0.008 rad at most
            let ext = past_ends / arc.max(1e-6);
            let steps = (arc * (1.0 + 2.0 * ext) / 0.008).ceil().max(1.0) as u32;
            for k in 0..=steps {
                let s = -ext + (1.0 + 2.0 * ext) * k as f32 / steps as f32;
                let p = (a * (arc * (1.0 - s)).sin() + b * (arc * s).sin()) / arc.sin().max(1e-6);
                let p = p.normalize();
                for side in [-1.0, 0.0, 1.0] {
                    put(cell_of(p + n * (line_reach * side)), REF_SEGMENT << 30 | index);
                }
            }
        }
        put_name(label, fig, &mut put);
    }
    (records, segments, labels, lists)
}

/// A figure's colour for `FrameState::figures` w: RGB of 0 to 1 as 8-bit
/// unorm (`unpack4x8unorm`), and TINT_TOP in the unused alpha byte. The
/// word travels as an f32: that byte keeps it a plain float of 0.5 to 2,
/// never 0 (none), a NaN (an all-ones top byte, which a GPU compares as
/// unordered and need not keep bit for bit) nor a denormal (flushed to 0).
pub fn pack_tint(rgb: [f32; 3]) -> u32 {
    let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    c(rgb[0]) | c(rgb[1]) << 8 | c(rgb[2]) << 16 | TINT_TOP << 24
}

/// The alpha byte of `pack_tint`: sign 0, exponent 126 or 127.
const TINT_TOP: u32 = 0x3f;

/// Where a picture or a name sits in the atlas, packed for the shader:
/// layer + 1 (0: nowhere) in bits 0..3, the block column in 4..12, the
/// block row in 12..20.
pub fn pack_place(layer: u32, bx: u32, by: u32) -> u32 {
    (layer + 1) | bx << 4 | by << 12
}

/// The uniform (`ConstellationFrame` in constellations.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FrameState {
    /// x: 1 when something shows; y: `HEAD_DRAWINGS` and the bits of what
    /// is on (`HEAD_TINTS`, `HEAD_SPARKLES`); z, w: `Bases::labels`,
    /// `Bases::states`.
    pub head: [u32; 4],
    /// The references of the cell table in use (`Bases::refs`, or
    /// `wide_refs` with the sparkles), `drawings`, `segments`; w the table
    /// in use (0, or `Bases::wide_cells`).
    pub bases: [u32; 4],
    /// x: the lines' half-width (radians), y: a name texel (radians), z, w: 0.
    pub look: [f32; 4],
    /// The sparkles: x their radius (radians), y their turns while a line
    /// draws from its start to its end, z their shape (`SHAPE_*`), w their
    /// light against the lines' opacity.
    pub style: [f32; 4],
    /// Per figure: x its name's texel (radians, each name its own),
    /// y opacity, z how far its lines are drawn (their own times), w its
    /// colour (`pack_tint`, bits; 0: the overlay's own).
    pub figures: [[f32; 4]; MAX_FIGURES],
    /// Per figure's name: x its place (bits), y its width | height << 12 |
    /// lift << 22 in texels (bits; lift: the rows its picture's centre lies
    /// above its text's centre, doubled, for a reading over it), z where its
    /// text's centre is (bits: 0 the figure's label point, d + 1 drawing d's
    /// own; with the top bit, the figure's label point and the offset its
    /// name moved off its stars, in its own frame: x right in bits 16..29,
    /// y up in bits 0..15, signed quarters of its texel), w the name's
    /// opacity.
    pub names: [[f32; 4]; MAX_FIGURES],
    /// Per planet, two: [its direction (catalogue frame, unit), its name's
    /// opacity] and [its name's place (bits), width | height << 16 in
    /// texels (bits), a texel (radians), how far under the planet its centre
    /// goes (radians: its text a gap under the planet and under the names
    /// of planets in conjunction before it)].
    pub planets: [[f32; 4]; 2 * PLANET_NAMES],
}

impl Default for FrameState {
    fn default() -> Self {
        bytemuck::Zeroable::zeroed()
    }
}

/// What an upload holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Picture(u32),
    Name(u32),
    PlanetName(u32),
}

/// A picture or a name for the atlas: its block, size and every mip level
/// (`Rg8`, rows packed; level k is `ceil(w / 2^k)` x `ceil(h / 2^k)`), of
/// which this upload writes the band of level-0 rows `rows` (start a
/// multiple of `1 << (ATLAS_MIPS - 1)`) and the matching rows of the mips:
/// a large picture goes up in pieces over several frames.
#[derive(Clone, Debug)]
pub struct Upload {
    pub item: Item,
    pub layer: u32,
    pub block: [u32; 2],
    pub size: [u32; 2],
    pub rows: [u32; 2],
    pub levels: Vec<Arc<Vec<u8>>>,
}

/// The rows of mip `level` that the level-0 band `rows` of a picture `h`
/// rows tall covers.
pub fn band_rows(rows: [u32; 2], h: u32, level: u32) -> [u32; 2] {
    let len = level_len(h, level);
    let end = if rows[1] >= h { len } else { (rows[1] >> level).min(len) };
    [(rows[0] >> level).min(end), end]
}

/// One frame's constellations (`FrameParams::constellations`).
#[derive(Clone, Debug, Default)]
pub struct Overlay {
    pub state: Box<FrameState>,
    /// Per drawing: x how far its pen stroke has come (its reveal time, 0
    /// to past 1), y its opacity, z 0, w its place (`pack_place`, as f32
    /// bits). Written at `Bases::states` of the tables.
    pub drawings: Vec<[f32; 4]>,
    pub states_at: u32,
    /// The tables (`build_tables`), and what tells them apart: sent with
    /// every frame, written when the id changes.
    pub tables: Option<Arc<Vec<u8>>>,
    pub tables_id: u64,
    /// Atlas layers wanted (0: none, its memory let go).
    pub layers: u32,
    pub uploads: Vec<Upload>,
    /// The figures in use: `FrameState::figures` and `names` are read and
    /// written up to this number only (the others hold nothing the tables
    /// list), so a frame costs what its cultures hold, not `MAX_FIGURES`.
    pub slots: u32,
}

// ------------------------------------------------- the planets and the discs
//
// The planets' points and the Sun's and the Moon's discs ride the night
// sky's frame (stars.rs `StarFrame::planets`, `discs`, filled by the
// aurora-viewer world/planets.rs) into the frame uniforms (renderer.rs).

/// `StarFrame::planets`: the count, then two `vec4` per planet (at most 6).
/// Must match `sky_planets` in common.wgsl.
pub const PLANET_SLOTS: usize = 13;
/// `StarFrame::discs` w.
pub const DISC_REGION: f32 = 0.0;
pub const DISC_REAL: f32 = 1.0;
pub const DISC_HIDDEN: f32 = 2.0;

/// The Sun's and the Moon's disc words (`StarFrame::discs`) on the frame's
/// texture slots and moon direction (renderer.rs, the frame uniforms): a
/// hidden disc's slot (1 the sun, 2 the moon) left out, so that its texture
/// draws nothing (env.wgsl reads those two slots for the discs only); a real
/// moon's direction in `moon`'s xyz, its w (the scale) kept, for nothing but
/// its disc reads it. A hidden word is never moved, whatever its xyz: hide
/// wins. The sun's real direction goes to the shader in the words themselves
/// (`sky_discs`).
pub fn apply_discs(discs: [[f32; 4]; 2], tex_slots: &mut [u32; 4], moon: &mut [f32; 4]) {
    let [sun, moon_word] = discs;
    if sun[3] == DISC_HIDDEN {
        tex_slots[1] = 0;
    }
    if moon_word[3] == DISC_HIDDEN {
        tex_slots[2] = 0;
    } else if moon_word[3] == DISC_REAL {
        *moon = [moon_word[0], moon_word[1], moon_word[2], moon[3]];
    }
}

/// The overlay's GPU side, on the render thread.
pub struct Gpu {
    pub frame: wgpu::Buffer,
    pub tables: wgpu::Buffer,
    pub atlas: wgpu::Texture,
    pub atlas_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    layers: u32,
    tables_id: u64,
    written: Box<FrameState>,
    written_drawings: Vec<[f32; 4]>,
    /// The place each drawing's picture and each figure's name was last
    /// written to (0: none): a place the viewer names but this atlas never
    /// received (a new device) is drawn as nothing.
    pictures: Vec<u32>,
    names: [u32; MAX_FIGURES],
    planet_names: [u32; PLANET_NAMES],
    /// Bytes written into the atlas, for the profile.
    pub uploaded: u64,
}

fn atlas(device: &wgpu::Device, layers: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let (w, h, mips) = if layers == 0 { (1, 1, 1) } else { (ATLAS_W, ATLAS_H, ATLAS_MIPS) };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("constellation atlas"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: layers.max(1),
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    (texture, view)
}

impl Gpu {
    /// Placeholders: a zero uniform, a 16-byte table, a 1 x 1 atlas.
    pub fn new(device: &wgpu::Device) -> Self {
        let frame = crate::gpu_alloc::buffer_init(
            device,
            "constellation frame",
            bytemuck::bytes_of(&FrameState::default()),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let tables = crate::gpu_alloc::buffer_init(device, "constellation tables", &[0u8; 16], wgpu::BufferUsages::STORAGE);
        let (atlas, atlas_view) = atlas(device, 0);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("constellation atlas"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        Gpu {
            frame,
            tables,
            atlas,
            atlas_view,
            sampler,
            layers: 0,
            tables_id: 0,
            written: Box::default(),
            written_drawings: Vec::new(),
            pictures: Vec::new(),
            names: [0; MAX_FIGURES],
            planet_names: [0; PLANET_NAMES],
            uploaded: 0,
        }
    }

    /// The frame group's layout entries (bindings 33 to 36).
    pub fn layout_entries() -> [wgpu::BindGroupLayoutEntry; 4] {
        let fragment = wgpu::ShaderStages::FRAGMENT;
        [
            wgpu::BindGroupLayoutEntry {
                binding: 33,
                visibility: fragment,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 34,
                visibility: fragment,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 35,
                visibility: fragment,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 36,
                visibility: fragment,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ]
    }

    /// Takes one frame's overlay (none: off). Returns true when the frame
    /// groups must be made again (a new table buffer or atlas).
    pub fn apply(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, overlay: Option<&Overlay>) -> bool {
        let mut rebind = false;
        let Some(o) = overlay else {
            if self.written.head[0] != 0 {
                *self.written = FrameState::default();
                queue.write_buffer(&self.frame, 0, bytemuck::bytes_of(&*self.written));
            }
            // off for good: the atlas's memory goes
            if self.layers > 0 {
                (self.atlas, self.atlas_view) = atlas(device, 0);
                self.layers = 0;
                self.pictures.iter_mut().for_each(|p| *p = 0);
                self.names = [0; MAX_FIGURES];
                self.planet_names = [0; PLANET_NAMES];
                return true;
            }
            return false;
        };
        // nothing shows, nothing changes: no work
        if o.state.head[0] == 0
            && self.written.head[0] == 0
            && o.uploads.is_empty()
            && o.layers.min(MAX_LAYERS) == self.layers
            && (o.tables.is_none() || o.tables_id == self.tables_id)
        {
            return false;
        }
        if let Some(tables) = &o.tables
            && o.tables_id != self.tables_id
        {
            self.tables = crate::gpu_alloc::buffer_init(
                device,
                "constellation tables",
                tables,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            );
            self.tables_id = o.tables_id;
            // the buffer starts with every drawing's state at zero
            self.written_drawings.clear();
            rebind = true;
        }
        let layers = o.layers.min(MAX_LAYERS);
        if layers != self.layers {
            let (texture, view) = atlas(device, layers);
            if layers > 0 && self.layers > 0 {
                // the layers kept keep their pictures
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("constellation atlas growth"),
                });
                for mip in 0..ATLAS_MIPS {
                    encoder.copy_texture_to_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &self.atlas,
                            mip_level: mip,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::TexelCopyTextureInfo {
                            texture: &texture,
                            mip_level: mip,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::Extent3d {
                            width: ATLAS_W >> mip,
                            height: ATLAS_H >> mip,
                            depth_or_array_layers: layers.min(self.layers),
                        },
                    );
                }
                queue.submit([encoder.finish()]);
            }
            let kept = layers.min(self.layers);
            for place in self
                .pictures
                .iter_mut()
                .chain(self.names.iter_mut())
                .chain(self.planet_names.iter_mut())
            {
                if (*place & 15) > kept {
                    *place = 0;
                }
            }
            self.atlas = texture;
            self.atlas_view = view;
            self.layers = layers;
            rebind = true;
        }
        for u in &o.uploads {
            if u.layer >= self.layers || !upload_fits(u) {
                continue;
            }
            for (mip, data) in u.levels.iter().enumerate().take(ATLAS_MIPS as usize) {
                let (w, h) = (level_len(u.size[0], mip as u32), level_len(u.size[1], mip as u32));
                let [r0, r1] = band_rows(u.rows, u.size[1], mip as u32);
                if data.len() < (w * h * 2) as usize {
                    break;
                }
                if r1 <= r0 {
                    continue;
                }
                let bytes = &data[(r0 * w * 2) as usize..(r1 * w * 2) as usize];
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.atlas,
                        mip_level: mip as u32,
                        origin: wgpu::Origin3d {
                            x: (u.block[0] * BLOCK) >> mip,
                            y: ((u.block[1] * BLOCK) >> mip) + r0,
                            z: u.layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    bytes,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(w * 2),
                        rows_per_image: Some(r1 - r0),
                    },
                    wgpu::Extent3d {
                        width: w,
                        height: r1 - r0,
                        depth_or_array_layers: 1,
                    },
                );
                self.uploaded += bytes.len() as u64;
            }
            let place = pack_place(u.layer, u.block[0], u.block[1]);
            match u.item {
                Item::Picture(d) => {
                    let d = d as usize;
                    if self.pictures.len() <= d {
                        self.pictures.resize(d + 1, 0);
                    }
                    self.pictures[d] = place;
                }
                Item::Name(f) if (f as usize) < MAX_FIGURES => self.names[f as usize] = place,
                Item::Name(_) => {}
                Item::PlanetName(k) if (k as usize) < PLANET_NAMES => self.planet_names[k as usize] = place,
                Item::PlanetName(_) => {}
            }
        }
        // what the viewer names but this atlas does not hold is drawn as nothing
        let mut state = *o.state;
        let slots = (o.slots as usize).min(MAX_FIGURES);
        for f in 0..slots {
            if state.names[f][0].to_bits() != self.names[f] {
                state.names[f][0] = 0.0;
            }
        }
        for k in 0..PLANET_NAMES {
            if state.planets[2 * k + 1][0].to_bits() != self.planet_names[k] {
                state.planets[2 * k + 1][0] = 0.0;
            }
        }
        // the parts in use only, each when it changed: the head, the
        // figures and names up to `slots`, the planets
        let new = bytemuck::bytes_of(&state);
        let old = bytemuck::bytes_of_mut(&mut *self.written);
        for part in uniform_parts(slots) {
            if new[part.clone()] != old[part.clone()] {
                old[part.clone()].copy_from_slice(&new[part.clone()]);
                queue.write_buffer(&self.frame, part.start as u64, &new[part]);
            }
        }
        let mut drawings = o.drawings.clone();
        for (d, st) in drawings.iter_mut().enumerate() {
            if st[3].to_bits() != self.pictures.get(d).copied().unwrap_or(0) {
                st[3] = 0.0;
            }
        }
        let fits = (o.states_at as u64 + drawings.len() as u64) * 16 <= self.tables.size();
        if drawings != self.written_drawings && o.states_at > 0 && fits {
            queue.write_buffer(&self.tables, u64::from(o.states_at) * 16, bytemuck::cast_slice(&drawings));
            self.written_drawings = drawings;
        }
        rebind
    }
}

/// The byte ranges of the uniform a frame with `slots` figures in use
/// writes: the head and the first `slots` figures, their names, the
/// planets.
pub fn uniform_parts(slots: usize) -> [std::ops::Range<usize>; 3] {
    let slots = slots.min(MAX_FIGURES);
    let figures = std::mem::offset_of!(FrameState, figures);
    let names = std::mem::offset_of!(FrameState, names);
    let planets = std::mem::offset_of!(FrameState, planets);
    [0..figures + 16 * slots, names..names + 16 * slots, planets..FRAME_BYTES]
}

/// Texels of mip `level` along an edge of `len`.
pub fn level_len(len: u32, level: u32) -> u32 {
    len.div_ceil(1 << level).max(1)
}

/// An upload stays inside its layer at every level.
fn upload_fits(u: &Upload) -> bool {
    let (x, y) = (u.block[0] * BLOCK, u.block[1] * BLOCK);
    u.size[0] > 0 && u.size[1] > 0 && x + u.size[0] <= ATLAS_W && y + u.size[1] <= ATLAS_H
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_disc_words_hide_or_move_only_their_own_body() {
        let tex = [11, 12, 13, 14];
        let moon = [0.1, 0.2, 0.97, 0.9];
        let run = |discs: [[f32; 4]; 2]| {
            let (mut t, mut m) = (tex, moon);
            apply_discs(discs, &mut t, &mut m);
            (t, m)
        };
        // the region's own: nothing changes (its xyz unused)
        assert_eq!(run([[0.5, 0.5, 0.5, DISC_REGION]; 2]), (tex, moon));
        // the sun hidden: slot 1, nothing else
        let hidden = [0.0, 0.0, -1.0, DISC_HIDDEN];
        assert_eq!(run([hidden, [0.0; 4]]), ([11, 0, 13, 14], moon));
        // the moon hidden: slot 2, nothing else
        assert_eq!(run([[0.0; 4], hidden]), ([11, 12, 0, 14], moon));
        // the real moon: its direction, the scale kept; no slot touched
        let real_moon = [0.6, 0.0, 0.8, DISC_REAL];
        assert_eq!(run([[0.0; 4], real_moon]), (tex, [0.6, 0.0, 0.8, 0.9]));
        // the real sun: in the words only, nothing here changes
        assert_eq!(run([[0.0, 0.8, 0.6, DISC_REAL], [0.0; 4]]), (tex, moon));
        // hide wins for each body: a hidden word with a direction moves nothing
        let both = run([[0.0, 0.8, 0.6, DISC_HIDDEN], [0.6, 0.0, 0.8, DISC_HIDDEN]]);
        assert_eq!(both, ([11, 0, 0, 14], moon));
    }

    #[test]
    fn a_short_line_keeps_most_of_its_length() {
        // a long line keeps the full 0.45° gap, bit for bit
        for deg in [3.0f32, 5.0, 20.0, 90.0] {
            assert_eq!(line_gap(deg.to_radians()).to_bits(), LINE_GAP.to_bits(), "{deg}°");
        }
        // a short one gives each end its share, never past 30 % of it
        for deg in [0.3f32, 0.9, 1.5, 2.5] {
            let arc = deg.to_radians();
            let gap = line_gap(arc);
            assert!((gap - LINE_GAP_SHARE * arc).abs() < 1e-9, "{deg}°");
            assert!(2.0 * gap <= 0.3 * arc + 1e-9);
        }
        // nothing to divide, nothing below zero
        assert_eq!(line_gap(0.0), 0.0);
        assert_eq!(line_gap(-0.01), 0.0);
        // the shader's copy
        let wgsl = include_str!("shaders/constellations.wgsl");
        assert!(wgsl.contains("const CONSTELLATION_GAP: f32 = 0.00785;"));
        assert!(wgsl.contains("const CONSTELLATION_GAP_SHARE: f32 = 0.15;"));
        assert!(wgsl.contains("return min(CONSTELLATION_GAP, CONSTELLATION_GAP_SHARE * max(arc, 0.0));"));
        assert_eq!(wgsl.matches("constellation_gap(arc)").count(), 2, "the line and its sparkle");
    }

    fn deg(v: f32) -> f32 {
        v.to_radians()
    }

    /// Orion with one line.
    fn orion() -> FigureGeom {
        FigureGeom {
            centre: [deg(83.0), deg(-1.0)],
            label: [deg(90.0), deg(2.0)],
            lines: vec![Line {
                a: [deg(83.0017), deg(-0.2994)],
                b: [deg(85.188), deg(-1.9419)],
                t0: 0.0,
                t1: 0.5,
            }],
        }
    }

    /// A small square picture about Orion's belt.
    fn drawing(figure: u32) -> DrawingGeom {
        let texel = deg(0.04);
        DrawingGeom {
            figure,
            centre: [deg(83.0), deg(-1.0)],
            picture: Picture {
                texel,
                size: [500, 400],
                origin: [250.0 * texel, 200.0 * texel],
            },
            label: None,
        }
    }

    fn refs_of(bytes: &[u8], bases: Bases, cell: u32) -> Vec<u32> {
        let words: &[u32] = bytemuck::cast_slice(bytes);
        let (a, b) = (words[cell as usize], words[cell as usize + 1]);
        let base = bases.refs as usize * 4;
        words[base + a as usize..base + b as usize].to_vec()
    }

    fn vec_at(bytes: &[u8], at: u32) -> Vec3 {
        let words: &[u32] = bytemuck::cast_slice(bytes);
        let i = at as usize * 4;
        Vec3::new(f32::from_bits(words[i]), f32::from_bits(words[i + 1]), f32::from_bits(words[i + 2]))
    }

    #[test]
    fn every_point_of_a_picture_finds_it_in_its_cell() {
        let f = orion();
        let d = drawing(0);
        let (bytes, bases) = build_tables(std::slice::from_ref(&f), std::slice::from_ref(&d));
        let frame = tangent_frame(d.centre[0], d.centre[1]);
        let p = d.picture;
        for j in 0..40 {
            for i in 0..50 {
                let x = p.origin[0] - (i as f32 + 0.5) * 10.0 * p.texel;
                let y = p.origin[1] - (j as f32 + 0.5) * 10.0 * p.texel;
                let dir = plane_to_dir(&frame, x, y);
                assert!(refs_of(&bytes, bases, cell_of(dir)).contains(&(REF_PICTURE << 30)), "({i}, {j})");
            }
        }
        // a cell far away lists nothing
        assert!(refs_of(&bytes, bases, cell_of(equatorial(deg(260.0), deg(-60.0)))).is_empty());
        // the record: its frame's centre, its figure
        assert!(vec_at(&bytes, bases.drawings).distance(frame[0]) < 1e-6);
        let words: &[u32] = bytemuck::cast_slice(&bytes);
        assert_eq!(words[(bases.drawings as usize + 3) * 4 + 2], 0, "drawing 0 is figure 0's");
    }

    #[test]
    fn two_drawings_of_one_figure_each_have_their_record_and_state() {
        // two art sets: the same figure drawn twice, the second with its own
        // label point; each is listed under its own number
        let f = orion();
        let mut other = drawing(0);
        other.centre = [deg(84.0), deg(-2.0)];
        other.label = Some([deg(95.0), deg(4.0)]);
        let (bytes, bases) = build_tables(std::slice::from_ref(&f), &[drawing(0), other.clone()]);
        assert_eq!(bases.drawing_count, 2);
        let c = equatorial(other.centre[0], other.centre[1]);
        assert!(refs_of(&bytes, bases, cell_of(c)).contains(&(REF_PICTURE << 30 | 1)));
        assert!(vec_at(&bytes, bases.drawings + DRAWING_VEC4 as u32).distance(c) < 1e-6);
        let own = equatorial(deg(95.0), deg(4.0));
        assert!(vec_at(&bytes, bases.drawings + DRAWING_VEC4 as u32 + 4).distance(own) < 1e-6);
        assert!(
            refs_of(&bytes, bases, cell_of(own)).contains(&(REF_NAME << 30)),
            "the name is found at the set's label too"
        );
        // the states: one vec4 each, at the end, zero
        assert_eq!(bytes.len(), (bases.states as usize + 2) * 16);
        assert!(bytes[bases.states as usize * 16..].iter().all(|b| *b == 0));
    }

    #[test]
    fn a_line_is_listed_along_its_arc_and_its_label_around_its_point() {
        let f = orion();
        let (bytes, bases) = build_tables(std::slice::from_ref(&f), &[]);
        let l = f.lines[0];
        let (a, b) = (equatorial(l.a[0], l.a[1]), equatorial(l.b[0], l.b[1]));
        for k in 0..=20 {
            let p = a.lerp(b, k as f32 / 20.0).normalize();
            assert!(refs_of(&bytes, bases, cell_of(p)).contains(&(REF_SEGMENT << 30)), "{k}");
        }
        let label = equatorial(f.label[0], f.label[1]);
        let e = label.cross(Vec3::Z).normalize();
        for r in [0.0, 0.05, 0.1, NAME_REACH] {
            assert!(refs_of(&bytes, bases, cell_of((label + e * r).normalize())).contains(&(REF_NAME << 30)));
        }
        // the records: the segment's ends and times, the figure's label
        let words: &[u32] = bytemuck::cast_slice(&bytes);
        let seg = bases.segments as usize * 4;
        assert_eq!(words[seg + 3], 0, "the segment's figure");
        assert_eq!(words[seg + 7], 32768 << 16, "t0 0, t1 0.5");
        assert!(vec_at(&bytes, bases.labels).distance(label) < 1e-6);
    }

    /// The references of `cell` in the wide table.
    fn wide_refs_of(bytes: &[u8], bases: Bases, cell: u32) -> Vec<u32> {
        let words: &[u32] = bytemuck::cast_slice(bytes);
        let t = bases.wide_cells as usize * 4;
        let (a, b) = (words[t + cell as usize], words[t + cell as usize + 1]);
        let base = bases.wide_refs as usize * 4;
        words[base + a as usize..base + b as usize].to_vec()
    }

    #[test]
    fn the_wide_lists_reach_a_sparkle_around_and_past_a_line() {
        let f = orion();
        let (bytes, bases) = build_tables(std::slice::from_ref(&f), &[]);
        let l = f.lines[0];
        let (a, b) = (equatorial(l.a[0], l.a[1]), equatorial(l.b[0], l.b[1]));
        let n = a.cross(b).normalize();
        let r = REF_SEGMENT << 30;
        // across the arc and past both ends, as far as a sparkle reaches
        for k in 0..=20 {
            let p = a.lerp(b, k as f32 / 20.0).normalize();
            for side in [-0.9, -0.5, 0.5, 0.9] {
                let q = (p + n * SPARKLE_REACH * side).normalize();
                assert!(wide_refs_of(&bytes, bases, cell_of(q)).contains(&r), "{k} {side}");
            }
        }
        let along = (b - a).normalize();
        for (end, dir) in [(a, -along), (b, along)] {
            let q = (end + dir * SPARKLE_REACH * 0.9).normalize();
            assert!(wide_refs_of(&bytes, bases, cell_of(q)).contains(&r));
        }
        // the narrow lists are as before: the line's own cells only
        let far = (a + n * SPARKLE_REACH * 3.0).normalize();
        assert!(!refs_of(&bytes, bases, cell_of(far)).contains(&r) || cell_of(far) == cell_of(a));
    }

    #[test]
    fn a_segment_reference_leads_to_its_own_two_records() {
        // the shader reads segment n at bases.segments + 2n: three lines,
        // each listed under its own number along its own arc
        let mut f = orion();
        let ends = [(78.6346, -8.2015), (81.1176, -2.3978), (83.0017, -0.2994), (85.188, -1.9419)];
        f.lines = ends
            .windows(2)
            .map(|w| Line {
                a: [deg(w[0].0), deg(w[0].1)],
                b: [deg(w[1].0), deg(w[1].1)],
                t0: 0.0,
                t1: 1.0,
            })
            .collect();
        let (bytes, bases) = build_tables(std::slice::from_ref(&f), &[]);
        for (n, l) in f.lines.iter().enumerate() {
            let (a, b) = (equatorial(l.a[0], l.a[1]), equatorial(l.b[0], l.b[1]));
            let mid = (a + b).normalize();
            let r = REF_SEGMENT << 30 | n as u32;
            assert!(
                refs_of(&bytes, bases, cell_of(mid)).contains(&r),
                "segment {n} listed by its number"
            );
            let rec = bases.segments + 2 * n as u32;
            assert!(
                vec_at(&bytes, rec).distance(a) < 1e-6 && vec_at(&bytes, rec + 1).distance(b) < 1e-6,
                "segment {n}'s records"
            );
        }
    }

    #[test]
    fn a_lines_only_figure_and_bad_geometry_build_without_a_picture() {
        let mut f = orion();
        // a zero-length line is left out
        f.lines.push(Line {
            a: [0.1, 0.2],
            b: [0.1, 0.2],
            t0: 0.5,
            t1: 1.0,
        });
        let (bytes, bases) = build_tables(&[f.clone(), f], &[]);
        assert_eq!(bases.drawings, bases.segments, "no drawing records");
        assert!(bases.refs < bases.wide_cells && bases.wide_cells < bases.wide_refs && bases.wide_refs <= bases.drawings);
        assert_eq!(bases.labels - bases.segments, 2 * 2, "one segment each");
        assert_eq!(bases.states - bases.labels, 2, "a label each");
        let words: &[u32] = bytemuck::cast_slice(&bytes);
        // the cell table ends at the references' length (padded to a vec4)
        let refs = words[CELLS as usize] as usize;
        assert_eq!(((bases.wide_cells - bases.refs) * 4) as usize, refs.div_ceil(4) * 4);
        let wide = words[(bases.wide_cells * 4 + CELLS) as usize] as usize;
        assert_eq!(((bases.drawings - bases.wide_refs) * 4) as usize, wide.div_ceil(4) * 4);
        assert!(wide >= refs, "the wide lists hold at least the narrow ones");
    }

    #[test]
    fn places_and_levels_pack_as_the_shader_reads_them() {
        let p = pack_place(2, 63, 31);
        assert_eq!(p & 15, 3);
        assert_eq!(p >> 4 & 255, 63);
        assert_eq!(p >> 12 & 255, 31);
        assert_eq!(level_len(2689, 0), 2689);
        assert_eq!(level_len(2689, 1), 1345);
        assert_eq!(level_len(2689, 2), 673);
        assert_eq!(level_len(1, 2), 1);
        let up = |block: [u32; 2], size: [u32; 2]| Upload {
            item: Item::Picture(0),
            layer: 0,
            block,
            size,
            rows: [0, size[1]],
            levels: Vec::new(),
        };
        assert!(upload_fits(&up([0, 0], [2689, 1410])));
        assert!(!upload_fits(&up([30, 0], [2689, 1410])));
        assert!(!upload_fits(&up([0, 0], [0, 10])));
        assert_eq!(FRAME_BYTES, 64 + 2 * 16 * MAX_FIGURES + 2 * 16 * PLANET_NAMES);
        assert_eq!(pack_tint([1.0, 0.0, 0.5]), 255 | 128 << 16 | 0x3f << 24);
        // every colour is a plain normal float, black and white included
        for rgb in [[0.0; 3], [1.0; 3], [1.0, 0.0, 0.5], [0.2, 0.9, 0.6]] {
            let w = f32::from_bits(pack_tint(rgb));
            assert!(w.is_normal() && (0.5..2.0).contains(&w), "{rgb:?}: {w}");
        }
    }

    #[test]
    fn every_culture_together_fits_the_uniform_and_the_shader_agrees() {
        let wgsl = include_str!("shaders/constellations.wgsl");
        assert!(wgsl.contains(&format!("const CONSTELLATION_FIGURES: u32 = {MAX_FIGURES}u;")));
        assert!(wgsl.contains(&format!("figures: array<vec4<f32>, {MAX_FIGURES}>,")));
        assert!(wgsl.contains(&format!("names: array<vec4<f32>, {MAX_FIGURES}>,")));
        assert!(wgsl.contains(&format!("const CONSTELLATION_MAX_REFS: u32 = {MAX_REFS}u;")));
        // the parts written: the head and the figures in use, their names,
        // the planets; every byte of the uniform with all in use
        let [head, names, planets] = uniform_parts(3);
        assert_eq!((head.start, head.end), (0, 64 + 3 * 16));
        assert_eq!(names.end - names.start, 3 * 16);
        assert_eq!(planets.end, FRAME_BYTES);
        let [a, b, c] = uniform_parts(usize::MAX);
        assert_eq!((a.end, b.start, b.end, c.start), (b.start, b.start, c.start, c.start));
    }

    #[test]
    fn a_figure_not_listed_keeps_its_number_but_no_reference() {
        // two figures, the second drawn; the first alone listed
        let mut far = orion();
        far.centre = [deg(250.0), deg(-40.0)];
        far.label = [deg(252.0), deg(-38.0)];
        far.lines[0].a = [deg(249.0), deg(-41.0)];
        far.lines[0].b = [deg(251.0), deg(-39.0)];
        let mut d = drawing(1);
        d.centre = far.centre;
        let all = build_tables(&[orion(), far.clone()], std::slice::from_ref(&d));
        let (bytes, bases) = build_tables_of(&[orion(), far.clone()], std::slice::from_ref(&d), Some(&[true, false]));
        // the same layout of records: the label points and the drawing's
        assert_eq!(bases.drawing_count, 1);
        assert_eq!(bases.states - bases.labels, 2);
        assert!(vec_at(&bytes, bases.labels + 1).distance(equatorial(far.label[0], far.label[1])) < 1e-6);
        // nothing of the second in any cell: its segment, name, picture
        let words: &[u32] = bytemuck::cast_slice(&bytes);
        let refs = &words[bases.refs as usize * 4..bases.wide_cells as usize * 4];
        assert!(refs.iter().all(|r| *r != (REF_NAME << 30 | 1)));
        let centre = equatorial(far.centre[0], far.centre[1]);
        assert!(refs_of(&bytes, bases, cell_of(centre)).is_empty(), "no picture, no line");
        assert_eq!(bases.labels - bases.segments, 2, "the first's one segment");
        // listed, it is found where it is
        let (all_bytes, all_bases) = all;
        let at = equatorial(far.label[0], far.label[1]);
        assert!(refs_of(&all_bytes, all_bases, cell_of(at)).contains(&(REF_NAME << 30 | 1)));
        assert!(refs_of(&bytes, bases, cell_of(at)).is_empty());
        assert!(bases.longest > 0 && all_bases.longest >= bases.longest && bases.wide_longest >= bases.longest);
    }

    #[test]
    fn bands_cover_every_row_of_every_level_once() {
        for h in [1u32, 3, 4, 990, 1410, 2048] {
            for band in [4u32, 8, 168, 4096] {
                let mut next = [0u32; ATLAS_MIPS as usize];
                let mut r = 0;
                while r < h {
                    let rows = [r, (r + band).min(h)];
                    for (level, at) in next.iter_mut().enumerate() {
                        let [a, b] = band_rows(rows, h, level as u32);
                        assert_eq!(a, *at, "h {h} band {band} level {level}: a gap or an overlap");
                        *at = b;
                    }
                    r = rows[1];
                }
                for (level, at) in next.iter().enumerate() {
                    assert_eq!(*at, level_len(h, level as u32), "h {h} band {band} level {level}");
                }
            }
        }
    }
}

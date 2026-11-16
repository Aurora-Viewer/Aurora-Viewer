//! An avatar's shape on screen, from its skeleton: each bone a tapered
//! capsule (thick for the torso, thin for a finger), their union the body.
//! It tints a part of the body (`world_hints`) as one smooth see-through
//! shape (the face an oval around the head, `face`), without the darker overlaps of capsules painted one by one, and
//! tells where the pointer reveals the markers in "On hover"
//! (`overlay::markers_reveal`).
//!
//! The shape is the zero line of a distance field (inside where a capsule
//! is nearer than its radius), drawn by marching squares over a grid of a
//! few points: each cell is clipped to its inside, so every point is
//! painted once.
use super::*;

/// A bone on screen: the segment from `a` to `b`, its radius going from
/// `ra` to `rb` (a joint alone: `a == b`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Capsule {
    pub a: egui::Pos2,
    pub b: egui::Pos2,
    pub ra: f32,
    pub rb: f32,
}

impl Capsule {
    /// How far inside the capsule `p` is (negative outside), in points.
    pub(super) fn depth(&self, p: egui::Pos2) -> f32 {
        let ab = self.b - self.a;
        let len2 = ab.length_sq();
        let t = if len2 <= f32::EPSILON {
            0.0
        } else {
            ((p - self.a).dot(ab) / len2).clamp(0.0, 1.0)
        };
        let nearest = self.a + ab * t;
        self.ra + (self.rb - self.ra) * t - nearest.distance(p)
    }
}

/// A shape the outline is traced from: how far inside it a point is
/// (negative outside, zero on its edge), and the box that holds it.
pub(super) trait Field {
    fn depth(&self, p: egui::Pos2) -> f32;
    fn bounds(&self) -> egui::Rect;
}

impl Field for Capsule {
    fn depth(&self, p: egui::Pos2) -> f32 {
        Capsule::depth(self, p)
    }
    fn bounds(&self) -> egui::Rect {
        egui::Rect::from_two_pos(self.a, self.b).expand(self.ra.max(self.rb))
    }
}

/// An ellipse on screen: the outline of an ellipsoid around the head (the
/// face), as `xᵀ M⁻¹ x = 1` around its centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Ellipse {
    centre: egui::Pos2,
    /// `M`, symmetric: [xx, xy, yy].
    matrix: [f32; 3],
    /// `M⁻¹`, the same way.
    inverse: [f32; 3],
    /// `M`'s diagonal roots: the half width and half height of its box.
    half: egui::Vec2,
    /// Its smallest half axis, which scales the depth to points.
    least: f32,
}

impl Ellipse {
    /// The outline of an ellipsoid whose half axes, put on screen, are
    /// `axes` around `centre`: `M = Σ v vᵀ` (exact for a parallel
    /// projection, close enough under perspective for a head). None when
    /// it is seen edge on.
    pub(super) fn of_axes(centre: egui::Pos2, axes: [egui::Vec2; 3]) -> Option<Self> {
        let (xx, xy, yy) = axes
            .iter()
            .fold((0.0, 0.0, 0.0), |(xx, xy, yy), v| (xx + v.x * v.x, xy + v.x * v.y, yy + v.y * v.y));
        let det = xx * yy - xy * xy;
        if det <= 1e-3 {
            return None;
        }
        let trace = xx + yy;
        let least = ((trace - (trace * trace - 4.0 * det).max(0.0).sqrt()) * 0.5).max(0.0).sqrt();
        Some(Self {
            centre,
            matrix: [xx, xy, yy],
            inverse: [yy / det, -xy / det, xx / det],
            half: egui::vec2(xx.sqrt(), yy.sqrt()),
            least,
        })
    }
}

impl Ellipse {
    /// `n` points around its edge, for one smooth closed outline: `c + A
    /// (cos t, sin t)` with `A Aᵀ = M` (its Cholesky factor).
    pub(super) fn outline(&self, n: usize) -> Vec<egui::Pos2> {
        let [xx, xy, yy] = self.matrix;
        let a = xx.sqrt();
        let b = if a > 0.0 { xy / a } else { 0.0 };
        let d = (yy - b * b).max(0.0).sqrt();
        (0..n)
            .map(|k| {
                let (sin, cos) = (k as f32 / n as f32 * std::f32::consts::TAU).sin_cos();
                self.centre + egui::vec2(a * cos, b * cos + d * sin)
            })
            .collect()
    }
}

impl Field for Ellipse {
    fn depth(&self, p: egui::Pos2) -> f32 {
        let d = p - self.centre;
        let [xx, xy, yy] = self.inverse;
        let q = (d.x * d.x * xx + 2.0 * d.x * d.y * xy + d.y * d.y * yy).max(0.0).sqrt();
        (1.0 - q) * self.least
    }
    fn bounds(&self) -> egui::Rect {
        egui::Rect::from_center_size(self.centre, self.half * 2.0)
    }
}

/// How far inside the union of `shapes` `p` is (negative outside).
pub(super) fn depth<F: Field>(shapes: &[F], p: egui::Pos2) -> f32 {
    shapes.iter().map(|c| c.depth(p)).fold(f32::NEG_INFINITY, f32::max)
}

/// The face as a face: an oval around the head rather than its joints'
/// cloud, whose dots read as a mess. The ellipsoid follows the head's own
/// axes (`head`, its joint's matrix: X forward, Y left, Z up) and spans the
/// face joints' extent on each, with a little skin past them; the ears
/// stand out to the sides and the tongue and teeth lie inside, so they do
/// not size it.
pub(super) fn face(
    names: &[String],
    world: &[glam::Vec3],
    joints: &[usize],
    head: glam::Mat4,
    project: impl Fn(glam::Vec3) -> Option<egui::Pos2>,
) -> Option<Ellipse> {
    let skin = |j: &&usize| !["Ear", "Tongue", "Teeth"].iter().any(|part| names[**j].contains(part));
    let sizing: Vec<usize> = joints.iter().filter(skin).copied().collect();
    let sizing = if sizing.is_empty() { joints.to_vec() } else { sizing };
    let axes = [head.x_axis, head.y_axis, head.z_axis].map(|a| a.truncate().normalize_or_zero());
    // forward, sideways, up: the brow and the chin need more than the cheeks
    let margin = [0.012, 0.014, 0.022];
    let mut centre = glam::Vec3::ZERO;
    let mut half = [glam::Vec3::ZERO; 3];
    for (k, axis) in axes.iter().enumerate() {
        let (lo, hi) = sizing
            .iter()
            .map(|j| world[*j].dot(*axis))
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
        if !lo.is_finite() {
            return None;
        }
        centre += *axis * (lo + hi) * 0.5;
        half[k] = *axis * ((hi - lo) * 0.5 + margin[k]);
    }
    let at = project(centre)?;
    let mut on_screen = [egui::Vec2::ZERO; 3];
    for (k, h) in half.iter().enumerate() {
        on_screen[k] = project(centre + *h)? - at;
    }
    Ellipse::of_axes(at, on_screen)
}

/// A joint's flesh around the bone, in metres, by its name: the torso
/// wide, the limbs narrowing to the hands and feet, a finger thin; the face
/// joints overlap into the face. Close to the default avatar's, enough to
/// follow its shape.
pub(super) fn radius(name: &str) -> f32 {
    let has = |part: &str| name.contains(part);
    match group(name) {
        1 => 0.028,
        2 => 0.012,
        3 if has("Tail") => 0.035,
        3 => 0.03,
        _ if has("Pelvis") || has("Torso") || has("Chest") || has("Spine") => 0.13,
        _ if has("Collar") => 0.07,
        _ if has("Neck") => 0.055,
        _ if has("Head") => 0.095,
        _ if has("Shoulder") => 0.055,
        _ if has("Elbow") => 0.045,
        _ if has("Wrist") => 0.035,
        _ if has("Hip") => 0.085,
        _ if has("Knee") => 0.06,
        _ if has("Ankle") => 0.045,
        _ if has("Foot") => 0.04,
        _ if has("Toe") => 0.03,
        _ => 0.05,
    }
}

/// Which bones join a part's joints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Links {
    /// Each joint to its nearest ancestor in the set (the body, a wing,
    /// the tail).
    Inside,
    /// Or, with none, to its parent outside it, at its own size: a finger
    /// from the wrist makes the palm.
    Outside,
    /// None: the face's joints are a cloud around the head, whose links
    /// would draw a star from its centre.
    None,
}

/// The links a part draws (`part_key`'s keys).
pub(super) fn links_of(key: usize) -> Links {
    match key {
        1 => Links::None,
        2 => Links::Outside,
        _ => Links::Inside,
    }
}

/// The capsules of `joints` on screen: one per joint, and one per link
/// (`links`). `world` gives each joint's place, `project` puts a point on
/// screen (None behind the camera).
pub(super) fn capsules(
    parent: &[Option<usize>],
    names: &[String],
    world: &[glam::Vec3],
    joints: &[usize],
    links: Links,
    project: impl Fn(glam::Vec3) -> Option<egui::Pos2>,
) -> Vec<Capsule> {
    // a radius on screen: the longest of the three axes it spans there
    let on_screen = |j: usize| -> Option<(egui::Pos2, f32)> {
        let at = project(world[j])?;
        let r = radius(&names[j]);
        let span = [glam::Vec3::X, glam::Vec3::Y, glam::Vec3::Z]
            .iter()
            .filter_map(|axis| project(world[j] + *axis * r).map(|p| p.distance(at)))
            .fold(0.0, f32::max);
        Some((at, span))
    };
    let mut out = Vec::with_capacity(joints.len() * 2);
    for &j in joints {
        let Some((at, r)) = on_screen(j) else { continue };
        out.push(Capsule {
            a: at,
            b: at,
            ra: r,
            rb: r,
        });
        let Some(direct) = parent[j] else { continue };
        // the nearest ancestor in the part: the torso reaches the pelvis
        // across the spine joints, which are not the body's
        let mut ancestor = Some(direct);
        while let Some(a) = ancestor.filter(|a| !joints.contains(a)) {
            ancestor = parent[a];
        }
        let (up, inside) = match (links, ancestor) {
            (Links::None, _) => continue,
            (_, Some(a)) => (a, true),
            (Links::Outside, None) => (direct, false),
            (Links::Inside, None) => continue,
        };
        if let Some((up_at, up_r)) = on_screen(up) {
            out.push(Capsule {
                a: up_at,
                b: at,
                // a parent outside the part lends its place, not its size
                ra: if inside { up_r } else { r },
                rb: r,
            });
        }
    }
    out
}

/// The union of `capsules` ready to paint: its inside as triangles, and
/// its edge as segments.
#[derive(Default)]
pub(super) struct Shape {
    pub triangles: Vec<[egui::Pos2; 3]>,
    pub edge: Vec<[egui::Pos2; 2]>,
}

/// The cells of the grid on the longer side: enough for a smooth edge, few
/// enough for the frame.
const CELLS: f32 = 110.0;

/// Trace the union of `shapes` (marching squares).
pub(super) fn trace<F: Field>(shapes: &[F]) -> Shape {
    let mut shape = Shape::default();
    let Some(bounds) = shapes.iter().map(Field::bounds).reduce(|a, b| a.union(b)) else {
        return shape;
    };
    let bounds = bounds.expand(1.0);
    // at most CELLS a side however large the part is on screen (a body
    // close up on a 4K screen), so the work per frame stays bounded
    let cell = (bounds.width().max(bounds.height()) / CELLS).max(2.0);
    let (nx, ny) = (
        (bounds.width() / cell).ceil() as usize + 1,
        (bounds.height() / cell).ceil() as usize + 1,
    );
    let at = |x: usize, y: usize| bounds.min + egui::vec2(x as f32 * cell, y as f32 * cell);
    // each shape fills only the points near it (two cells past its edge,
    // so that a cell the edge crosses has its true values); the rest stay
    // a little outside, which is all their sign needs
    let mut values = vec![-2.0 * cell; nx * ny];
    for c in shapes {
        let near = c.bounds().expand(2.0 * cell);
        let x0 = ((near.left() - bounds.left()) / cell).floor().max(0.0) as usize;
        let y0 = ((near.top() - bounds.top()) / cell).floor().max(0.0) as usize;
        let x1 = (((near.right() - bounds.left()) / cell).ceil().max(0.0) as usize).min(nx - 1);
        let y1 = (((near.bottom() - bounds.top()) / cell).ceil().max(0.0) as usize).min(ny - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let v = &mut values[y * nx + x];
                *v = v.max(c.depth(at(x, y)));
            }
        }
    }
    let value = |x: usize, y: usize| values[y * nx + x];
    for y in 0..ny - 1 {
        for x in 0..nx - 1 {
            // the cell's corners, clockwise from the top left
            let corners = [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)].map(|(cx, cy)| (at(cx, cy), value(cx, cy)));
            if corners.iter().all(|c| c.1 <= 0.0) {
                continue;
            }
            let mut inside = Vec::with_capacity(8);
            let mut crossings = Vec::with_capacity(4);
            for k in 0..4 {
                let (pa, va) = corners[k];
                let (pb, vb) = corners[(k + 1) % 4];
                if va > 0.0 {
                    inside.push(pa);
                }
                if (va > 0.0) != (vb > 0.0) {
                    let crossing = pa + (pb - pa) * (va / (va - vb));
                    inside.push(crossing);
                    crossings.push(crossing);
                }
            }
            for k in 1..inside.len().saturating_sub(1) {
                shape.triangles.push([inside[0], inside[k], inside[k + 1]]);
            }
            for &[a, b] in crossings.as_chunks::<2>().0 {
                shape.edge.push([a, b]);
            }
        }
    }
    shape
}

/// Paint `shape`: its inside in `fill`, its edge in `stroke`.
pub(super) fn paint(painter: &egui::Painter, shape: &Shape, fill: egui::Color32, stroke: Stroke) {
    let mut mesh = egui::Mesh::default();
    for triangle in &shape.triangles {
        let first = mesh.vertices.len() as u32;
        for p in triangle {
            mesh.colored_vertex(*p, fill);
        }
        mesh.add_triangle(first, first + 1, first + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
    for segment in &shape.edge {
        painter.line_segment(*segment, stroke);
    }
}

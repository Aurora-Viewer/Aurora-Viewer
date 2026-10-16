//! Geometry of a name plate, built as plain meshes: the pill with its
//! border, its drop shadow and its voice halo, and the discs and rings on
//! it (picture, status dot, badges).
//!
//! egui's own rounded shapes are cut again into triangles every frame, with
//! four times the points a small pill needs, and its shadow is a filled
//! blurred rectangle, which would darken the glass from underneath. Here a
//! shape is a few rings of vertices around an outline, antialiased over one
//! pixel like egui's; the lights are more rings outside the plate, as a CSS
//! `box-shadow` shows only around its box.

use egui::{Color32, Mesh, Pos2, Rect, Vec2};
use std::sync::OnceLock;

/// Steps of half a turn around an outline: enough for its sides to stay
/// within a fifth of a pixel of the circle, from a typing dot to the plate.
const MIN_ARC: usize = 4;
const MAX_ARC: usize = 24;

fn arc_steps(radius_px: f32) -> usize {
    ((2.4 * radius_px.max(0.0).sqrt()).ceil() as usize).clamp(MIN_ARC, MAX_ARC)
}

/// Unit directions around a pill for `arc` steps per end, clockwise from
/// the top of its right end: the first half belongs to the right end, the
/// second to the left one.
fn directions(arc: usize) -> &'static [Vec2] {
    static TABLES: OnceLock<Vec<Vec<Vec2>>> = OnceLock::new();
    let tables = TABLES.get_or_init(|| {
        (MIN_ARC..=MAX_ARC)
            .map(|arc| {
                let end = (0..=arc).map(|i| {
                    let angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / arc as f32;
                    Vec2::new(angle.cos(), angle.sin())
                });
                let right: Vec<Vec2> = end.collect();
                right.iter().copied().chain(right.iter().map(|d| -*d)).collect()
            })
            .collect()
    });
    &tables[arc.clamp(MIN_ARC, MAX_ARC) - MIN_ARC]
}

/// Share of the light left at `x` blur widths from the edge of a blurred
/// shape (a gaussian of sigma half the blur, as CSS): one half on the edge,
/// nearly nothing one blur away.
pub fn falloff(x: f32) -> f32 {
    // complementary error function (Abramowitz and Stegun 7.1.26)
    let z = x * std::f32::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * z.abs());
    let poly = t * (0.254_829_6 + t * (-0.284_496_74 + t * (1.421_413_7 + t * (-1.453_152 + t * 1.061_405_4))));
    let v = poly * (-z * z).exp();
    0.5 * if z >= 0.0 { v } else { 2.0 - v }
}

const FALLOFF_STEPS: usize = 64;
/// The table covers this many blur widths on each side of the edge.
const FALLOFF_RANGE: f32 = 1.5;

/// `falloff` from a table: the lights are evaluated at every vertex of
/// every plate, every frame.
fn falloff_fast(x: f32) -> f32 {
    static TABLE: OnceLock<[f32; FALLOFF_STEPS + 1]> = OnceLock::new();
    let table = TABLE.get_or_init(|| std::array::from_fn(|i| falloff((i as f32 / FALLOFF_STEPS as f32 * 2.0 - 1.0) * FALLOFF_RANGE)));
    let at = ((x / FALLOFF_RANGE + 1.0) * 0.5).clamp(0.0, 1.0) * FALLOFF_STEPS as f32;
    let i = (at as usize).min(FALLOFF_STEPS - 1);
    table[i] + (table[i + 1] - table[i]) * (at - i as f32)
}

/// `top` over `under`, both premultiplied.
pub fn over(top: Color32, under: Color32) -> Color32 {
    let keep = 255 - top.a() as u32;
    let mix = |t: u8, u: u8| (t as u32 + (u as u32 * keep + 127) / 255).min(255) as u8;
    Color32::from_rgba_premultiplied(
        mix(top.r(), under.r()),
        mix(top.g(), under.g()),
        mix(top.b(), under.b()),
        mix(top.a(), under.a()),
    )
}

/// A blurred copy of the plate's shape, as a CSS `box-shadow`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glow {
    pub offset: Vec2,
    pub blur: f32,
    /// How much the shape grows before it is blurred.
    pub spread: f32,
    /// Color at full intensity.
    pub color: Color32,
}

impl Glow {
    /// How far from the plate's edge the light goes.
    fn reach(&self) -> f32 {
        self.spread + self.blur + self.offset.x.abs().max(self.offset.y.abs())
    }

    /// Color `d` outside the plate's edge, where the outline faces `dir`.
    /// The moved shape is taken as `offset · dir` nearer: exact along the
    /// straight edges, close enough around the ends of a soft light.
    fn at(&self, d: f32, dir: Vec2) -> Color32 {
        if self.blur <= 0.0 {
            return Color32::TRANSPARENT;
        }
        let x = (d - self.offset.dot(dir) - self.spread) / self.blur;
        self.color.gamma_multiply(falloff_fast(x))
    }
}

/// The outline of a pill (a circle is a pill as wide as it is high), from
/// which rings of vertices are laid at a distance.
pub struct Outline {
    radius: f32,
    right: Pos2,
    left: Pos2,
    dirs: &'static [Vec2],
    /// Width of the antialiased edge: one pixel, in points.
    pub feather: f32,
}

impl Outline {
    pub fn pill(rect: Rect, pixels_per_point: f32) -> Outline {
        let radius = rect.height() * 0.5;
        let cy = rect.center().y;
        let (right, left) = (rect.right() - radius, rect.left() + radius);
        Outline {
            radius,
            right: Pos2::new(right.max(left), cy),
            left: Pos2::new(left.min(right), cy),
            dirs: directions(arc_steps(radius * pixels_per_point)),
            feather: 1.0 / pixels_per_point,
        }
    }

    pub fn circle(center: Pos2, radius: f32, pixels_per_point: f32) -> Outline {
        Outline::pill(Rect::from_center_size(center, Vec2::splat(radius * 2.0)), pixels_per_point)
    }

    fn points(&self) -> u32 {
        self.dirs.len() as u32
    }

    fn center_of(&self, i: usize) -> Pos2 {
        if i < self.dirs.len() / 2 { self.right } else { self.left }
    }

    /// A ring of vertices `d` outside the outline (inside when negative),
    /// each in the color `color` gives for its direction. Returns the index
    /// of its first vertex.
    pub fn ring_with(&self, mesh: &mut Mesh, d: f32, mut color: impl FnMut(Vec2) -> Color32) -> u32 {
        let first = mesh.vertices.len() as u32;
        let r = (self.radius + d).max(0.0);
        for (i, dir) in self.dirs.iter().enumerate() {
            mesh.colored_vertex(self.center_of(i) + *dir * r, color(*dir));
        }
        first
    }

    pub fn ring(&self, mesh: &mut Mesh, d: f32, color: Color32) -> u32 {
        self.ring_with(mesh, d, |_| color)
    }

    /// Triangles between two rings.
    pub fn band(&self, mesh: &mut Mesh, inner: u32, outer: u32) {
        let n = self.points();
        for i in 0..n {
            let j = (i + 1) % n;
            mesh.indices
                .extend_from_slice(&[inner + i, inner + j, outer + j, inner + i, outer + j, outer + i]);
        }
    }

    /// Fill the inside of `ring` with `color`.
    pub fn fill(&self, mesh: &mut Mesh, ring: u32, color: Color32) {
        let center = mesh.vertices.len() as u32;
        mesh.colored_vertex(self.left + (self.right - self.left) * 0.5, color);
        let n = self.points();
        for i in 0..n {
            mesh.indices.extend_from_slice(&[center, ring + i, ring + (i + 1) % n]);
        }
    }

    /// A surface with a border of `width` along the inside of its edge:
    /// `fill`, then `edge` (the border's color over the fill). Returns the
    /// last opaque ring and the distance at which the surface has faded out.
    fn surface(&self, mesh: &mut Mesh, fill: Color32, edge: Color32, width: f32) -> (u32, f32) {
        let f = self.feather * 0.5;
        let inner = self.ring(mesh, -width - f, fill);
        self.fill(mesh, inner, fill);
        let mut last = self.ring(mesh, -width + f, edge);
        self.band(mesh, inner, last);
        // a one pixel border is a single line of vertices
        if -width + f < -f - 0.01 {
            let rim = self.ring(mesh, -f, edge);
            self.band(mesh, last, rim);
            last = rim;
        }
        (last, f.max(-width + f + self.feather))
    }

    /// A disc of `fill` with a border of `width` inside its edge.
    pub fn disc(&self, mesh: &mut Mesh, fill: Color32, edge: Color32, width: f32) {
        let (last, out) = self.surface(mesh, fill, edge, width);
        let end = self.ring(mesh, out, Color32::TRANSPARENT);
        self.band(mesh, last, end);
    }

    /// A plain disc.
    pub fn dot(&self, mesh: &mut Mesh, color: Color32) {
        let f = self.feather * 0.5;
        let inner = self.ring(mesh, -f, color);
        self.fill(mesh, inner, color);
        let end = self.ring(mesh, f, Color32::TRANSPARENT);
        self.band(mesh, inner, end);
    }

    /// A line of `width` along the inside of the outline.
    pub fn stroke(&self, mesh: &mut Mesh, width: f32, color: Color32) {
        self.line(mesh, -width, 0.0, color);
    }

    /// A line from `inner` to `outer` away from the outline (inside when
    /// negative), antialiased on both sides.
    pub fn line(&self, mesh: &mut Mesh, inner: f32, outer: f32, color: Color32) {
        let f = self.feather * 0.5;
        let a = self.ring(mesh, inner - f, Color32::TRANSPARENT);
        let b = self.ring(mesh, inner + f, color);
        let c = self.ring(mesh, outer - f, color);
        let d = self.ring(mesh, outer + f, Color32::TRANSPARENT);
        self.band(mesh, a, b);
        self.band(mesh, b, c);
        self.band(mesh, c, d);
    }

    /// A picture cropped to the disc: `uv` is the part of it the disc shows.
    pub fn picture(&self, mesh: &mut Mesh, uv: Rect, tint: Color32) {
        let f = self.feather * 0.5;
        let center = self.left + (self.right - self.left) * 0.5;
        let first = mesh.vertices.len() as u32;
        let vertex = |pos: Pos2, color: Color32| egui::epaint::Vertex {
            pos,
            uv: uv.lerp_inside((pos - center) / (2.0 * self.radius) + Vec2::splat(0.5)),
            color,
        };
        mesh.vertices.push(vertex(center, tint));
        for d in [-f, f] {
            let color = if d < 0.0 { tint } else { Color32::TRANSPARENT };
            for (i, dir) in self.dirs.iter().enumerate() {
                mesh.vertices.push(vertex(self.center_of(i) + *dir * (self.radius + d), color));
            }
        }
        let n = self.points();
        let (inner, outer) = (first + 1, first + 1 + n);
        for i in 0..n {
            mesh.indices.extend_from_slice(&[first, inner + i, inner + (i + 1) % n]);
        }
        self.band(mesh, inner, outer);
    }
}

/// Where the rings of the lights stand, between the plate's edge and the
/// end of the light: closer together near the edge, where a tight halo
/// falls off.
const HALO_RINGS: [f32; 6] = [0.0, 0.06, 0.15, 0.34, 0.62, 1.0];
/// The same for a soft shadow alone.
const SHADOW_RINGS: [f32; 4] = [0.0, 0.22, 0.56, 1.0];

/// The line around a plate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Border {
    /// `width` along the inside of the edge, in this color.
    Inside(Color32, f32),
    /// `width` on each side of the edge: the voice outline.
    Around(Color32, f32),
}

/// A line standing away from the plate, over its lights.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ring {
    /// From the plate's edge to the line.
    pub gap: f32,
    pub width: f32,
    pub color: Color32,
}

/// The plate itself: its surface, its border, around it the lights (drop
/// shadow, voice halo), which never reach under the surface, and over them
/// the `rings`.
pub fn plate(mesh: &mut Mesh, rect: Rect, pixels_per_point: f32, fill: Color32, border: Border, lights: &[Glow], rings: &[Ring]) {
    lit_plate(mesh, rect, pixels_per_point, fill, border, lights);
    let o = Outline::pill(rect, pixels_per_point);
    for r in rings.iter().filter(|r| r.color.a() > 0 && r.width > 0.0) {
        // thinner than a pixel, a line is a fainter pixel
        let width = r.width.max(o.feather);
        let color = r.color.gamma_multiply((r.width / o.feather).min(1.0));
        o.line(mesh, r.gap, r.gap + width, color);
    }
}

fn lit_plate(mesh: &mut Mesh, rect: Rect, pixels_per_point: f32, fill: Color32, border: Border, lights: &[Glow]) {
    let o = Outline::pill(rect, pixels_per_point);
    let n = o.points() as usize;
    // a light that has not moved is a halo: it needs the closer rings
    let rings: &[f32] = if lights.iter().any(|g| g.offset == Vec2::ZERO) {
        &HALO_RINGS
    } else {
        &SHADOW_RINGS
    };
    mesh.reserve_vertices(n * (3 + rings.len()) + 1);
    mesh.reserve_triangles(n * (2 * (2 + rings.len()) + 1));
    // a light that has not moved is the same all around a ring: only the
    // moved ones are evaluated at every vertex
    let moved = |g: &&Glow| g.offset != Vec2::ZERO;
    let around = |d: f32| {
        let still = lights.iter().filter(|g| !moved(g));
        still.fold(Color32::TRANSPARENT, |under, g| over(g.at(d, Vec2::ZERO), under))
    };
    let light_over = |still: Color32, d: f32, dir: Vec2| {
        let under = lights.iter().filter(moved);
        over(still, under.fold(Color32::TRANSPARENT, |under, g| over(g.at(d, dir), under)))
    };
    let f = o.feather * 0.5;
    let (mut last, start) = match border {
        Border::Inside(color, width) => o.surface(mesh, fill, over(color, fill), width),
        Border::Around(color, width) => {
            let inner = o.ring(mesh, -width - f, fill);
            o.fill(mesh, inner, fill);
            let a = o.ring(mesh, -width + f, over(color, fill));
            o.band(mesh, inner, a);
            // outside the surface the line lies over the lights alone
            let still = around(width - f);
            let b = o.ring_with(mesh, width - f, |dir| over(color, light_over(still, width - f, dir)));
            o.band(mesh, a, b);
            (b, width + f)
        }
    };
    let reach = lights.iter().map(Glow::reach).fold(0.0, f32::max).max(start);
    for (k, t) in rings.iter().enumerate() {
        let d = start + (reach - start) * t;
        let ring = if k + 1 == rings.len() {
            o.ring(mesh, d, Color32::TRANSPARENT)
        } else {
            let still = around(d);
            o.ring_with(mesh, d, |dir| light_over(still, d, dir))
        };
        o.band(mesh, last, ring);
        last = ring;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pill() -> Rect {
        Rect::from_min_size(Pos2::new(100.0, 50.0), Vec2::new(160.0, 50.0))
    }

    /// Signed distance from `p` to the edge of the pill `rect`.
    fn pill_distance(rect: Rect, p: Pos2) -> f32 {
        let r = rect.height() * 0.5;
        let cx = p.x.clamp(rect.left() + r, rect.right() - r);
        (p.x - cx).hypot(p.y - rect.center().y) - r
    }

    #[test]
    fn a_blur_halves_the_light_on_the_edge() {
        assert!((falloff(0.0) - 0.5).abs() < 0.001);
        assert!(falloff(-1.0) > 0.97 && falloff(1.0) < 0.03);
        assert!(falloff(0.3) > falloff(0.4));
        // the table follows the function, and ends dark and full
        for i in -40..=40 {
            let x = i as f32 * 0.05;
            assert!((falloff_fast(x) - falloff(x)).abs() < 0.002, "{x}");
        }
        assert!(falloff_fast(9.0) < 0.002 && falloff_fast(-9.0) > 0.998);
    }

    #[test]
    fn colors_are_composed_premultiplied() {
        let half_white = Color32::from_rgba_premultiplied(128, 128, 128, 128);
        assert_eq!(over(Color32::RED, Color32::BLUE), Color32::RED);
        assert_eq!(over(Color32::TRANSPARENT, Color32::BLUE), Color32::BLUE);
        let mixed = over(half_white, Color32::BLACK);
        assert_eq!((mixed.r(), mixed.a()), (128, 255));
    }

    #[test]
    fn outlines_have_more_points_as_they_grow() {
        let small = Outline::circle(Pos2::ZERO, 1.5, 1.0);
        let large = Outline::pill(pill(), 1.0);
        assert!(small.points() >= 10 && small.points() < large.points());
        // a side of the polygon stays within a fraction of a pixel of the arc
        for radius in [1.3f32, 3.4, 10.0, 25.0, 40.0] {
            let half_angle = std::f32::consts::PI / (2.0 * arc_steps(radius) as f32);
            assert!(radius * (1.0 - half_angle.cos()) < 0.25, "{radius}");
        }
        assert!(Outline::pill(pill(), 2.0).points() >= large.points());
        // every point of a ring is at its distance from the outline
        let mut mesh = Mesh::default();
        large.ring(&mut mesh, 3.0, Color32::WHITE);
        assert_eq!(mesh.vertices.len(), large.points() as usize);
        for v in &mesh.vertices {
            assert!((pill_distance(pill(), v.pos) - 3.0).abs() < 0.001);
        }
    }

    fn lights() -> [Glow; 2] {
        [
            Glow {
                offset: Vec2::new(0.0, 5.0),
                blur: 14.0,
                spread: 0.0,
                color: Color32::from_black_alpha(90),
            },
            Glow {
                offset: Vec2::ZERO,
                blur: 9.0,
                spread: 1.5,
                color: Color32::GREEN,
            },
        ]
    }

    #[test]
    fn the_lights_stay_outside_the_plate_and_fade_out() {
        let rect = pill();
        let fill = Color32::from_rgba_premultiplied(10, 12, 20, 130);
        for (border, lights) in [
            (Border::Inside(Color32::from_white_alpha(30), 1.0), &lights()[..1]),
            (Border::Around(Color32::GREEN, 0.8), &lights()[..]),
        ] {
            let mut mesh = Mesh::default();
            plate(&mut mesh, rect, 1.0, fill, border, lights, &[]);
            assert!(mesh.is_valid() && !mesh.indices.is_empty());
            let reach = lights.iter().map(Glow::reach).fold(0.0, f32::max);
            let n = Outline::pill(rect, 1.0).points() as usize;
            let last_ring = mesh.vertices.len() - n;
            let mut lit_outside = false;
            for (i, v) in mesh.vertices.iter().enumerate() {
                let d = pill_distance(rect, v.pos);
                assert!(d <= reach + 0.01, "{d}");
                if d < -1.6 {
                    assert_eq!(v.color, fill, "only the surface under the glass");
                }
                lit_outside |= d > 2.0 && v.color.a() > 0;
                if i >= last_ring {
                    assert_eq!(v.color, Color32::TRANSPARENT, "the last ring is dark");
                }
            }
            assert!(lit_outside);
            // the shadow, moved down, is stronger under the plate than over it
            let alpha_at = |y: f32| {
                let near = |v: &&egui::epaint::Vertex| (v.pos.x - 180.0).abs() < 60.0 && (v.pos.y - y).abs() < 0.3;
                mesh.vertices.iter().filter(near).map(|v| v.color.a()).max().unwrap_or(0)
            };
            let start = if matches!(border, Border::Inside(..)) { 0.5 } else { 1.3 };
            let rings: &[f32] = if lights.len() > 1 { &HALO_RINGS } else { &SHADOW_RINGS };
            let d = start + (reach - start) * rings[rings.len() - 3];
            let (above, below) = (alpha_at(rect.top() - d), alpha_at(rect.bottom() + d));
            assert!(below > above, "{below} <= {above}");
        }
    }

    #[test]
    fn a_ring_stands_away_from_the_plate_over_its_lights() {
        let rect = pill();
        let fill = Color32::from_rgba_premultiplied(10, 12, 20, 130);
        let border = Border::Inside(Color32::from_white_alpha(30), 1.0);
        let mut lit = Mesh::default();
        plate(&mut lit, rect, 1.0, fill, border, &lights(), &[]);
        let ring = Ring {
            gap: 4.0,
            width: 2.0,
            color: Color32::GREEN,
        };
        let mut mesh = Mesh::default();
        plate(&mut mesh, rect, 1.0, fill, border, &lights(), &[ring]);
        assert!(mesh.is_valid());
        // the plate and its lights first, then the ring, drawn over them
        assert_eq!(mesh.vertices[..lit.vertices.len()], lit.vertices[..]);
        let added = &mesh.vertices[lit.vertices.len()..];
        for v in added {
            let d = pill_distance(rect, v.pos);
            assert!((3.49..=6.51).contains(&d), "{d}");
            if (4.49..=5.51).contains(&d) {
                assert_eq!(v.color, Color32::GREEN);
            }
        }
        // thinner than a pixel, a ring is a fainter pixel
        let mut thin = Mesh::default();
        let hair = Ring { width: 0.5, ..ring };
        plate(&mut thin, rect, 1.0, fill, border, &lights(), &[hair]);
        let hair_alpha = thin.vertices[lit.vertices.len()..].iter().map(|v| v.color.a()).max();
        assert_eq!(hair_alpha, Some(128));
    }

    #[test]
    fn a_surface_keeps_its_border_inside_and_ends_transparent() {
        let o = Outline::circle(Pos2::new(50.0, 50.0), 10.0, 1.0);
        let (fill, edge) = (Color32::from_gray(20), Color32::from_gray(90));
        let mut mesh = Mesh::default();
        o.disc(&mut mesh, fill, edge, 1.0);
        assert!(mesh.is_valid());
        let radius = |v: &egui::epaint::Vertex| (v.pos - Pos2::new(50.0, 50.0)).length();
        assert!(mesh.vertices.iter().all(|v| radius(v) <= 10.51));
        assert!(
            mesh.vertices
                .iter()
                .filter(|v| v.color == edge)
                .all(|v| (radius(v) - 9.5).abs() < 0.01)
        );
        assert!(
            mesh.vertices
                .iter()
                .filter(|v| radius(v) > 10.4)
                .all(|v| v.color == Color32::TRANSPARENT)
        );
        // a wider border has a plateau, a line and a dot are valid too
        let mut more = Mesh::default();
        o.disc(&mut more, fill, edge, 3.0);
        o.stroke(&mut more, 2.0, edge);
        o.dot(&mut more, edge);
        assert!(more.is_valid() && more.vertices.len() > mesh.vertices.len());
    }

    #[test]
    fn a_picture_maps_its_square_onto_the_disc() {
        let o = Outline::circle(Pos2::new(50.0, 50.0), 10.0, 1.0);
        let uv = Rect::from_min_max(Pos2::new(0.25, 0.0), Pos2::new(0.75, 1.0));
        let mut mesh = Mesh::with_texture(egui::TextureId::User(1));
        o.picture(&mut mesh, uv, Color32::WHITE);
        assert!(mesh.is_valid());
        assert_eq!(mesh.vertices[0].uv, Pos2::new(0.5, 0.5));
        for v in &mesh.vertices {
            assert!(uv.expand(0.03).contains(v.uv), "{:?}", v.uv);
            let expected = uv.lerp_inside((v.pos - Pos2::new(40.0, 40.0)) / 20.0);
            assert!((v.uv - expected).length() < 0.001);
        }
    }
}

//! Snapshot fragments preserve the real portrait and text. Tessellation and
//! clipping run on a worker; each frame batches fragments by GPU texture.
use crate::theme::Palette;
use egui::epaint::{ClippedShape, Mesh, Primitive, Vertex};
use egui::{Color32, Context, Order, Pos2, Rect, Shape, TextureId, Vec2};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

#[derive(Clone)]
pub struct Snapshot {
    pub rect: Rect,
    pub action: Rect,
    pub shapes: Vec<ClippedShape>,
}

struct Fragment {
    center: Pos2,
    release: f32,
    drift: Vec2,
    spin: f32,
    /// Opacity at release: 1 for a neighbour card, `ECHO_OPACITY` for the
    /// echo of a lone card.
    opacity: f32,
    meshes: Vec<Mesh>,
}

/// A single account has no neighbour to shatter, which left its validation
/// without the burst on either side. A faint copy of the card itself breaks
/// apart instead, each half towards its own side, starting under the card.
const ECHO_OPACITY: f32 = 0.7;

/// Fragments of a lone card's echo: those left of its centre leave to the
/// left, the others to the right, each one past its own edge of the card
/// (they start hidden under it) and then as far as a neighbour's would go.
fn echo(ctx: &Context, card: &Snapshot) -> Vec<Fragment> {
    let center = card.rect.center().x;
    let half = card.rect.width() / 2.0;
    split(ctx, card, 1.0)
        .into_iter()
        .map(|mut piece| {
            let out = half - (piece.center.x - center).abs() + piece.drift.x.abs() * 1.2;
            piece.drift.x = if piece.center.x < center { -out } else { out };
            piece.opacity = ECHO_OPACITY;
            piece
        })
        .collect()
}

pub struct Transition {
    selected: Snapshot,
    others: Vec<Snapshot>,
    fragments: Arc<OnceLock<Vec<Fragment>>>,
    started: f64,
    sent: bool,
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn interpolate(a: Vertex, b: Vertex, t: f32) -> Vertex {
    let ac = a.color.to_array();
    let bc = b.color.to_array();
    let color = std::array::from_fn::<_, 4, _>(|i| (ac[i] as f32 + (bc[i] as f32 - ac[i] as f32) * t).round() as u8);
    Vertex {
        pos: a.pos + (b.pos - a.pos) * t,
        uv: a.uv + (b.uv - a.uv) * t,
        color: Color32::from_rgba_premultiplied(color[0], color[1], color[2], color[3]),
    }
}

pub(super) fn clip(vertices: Vec<Vertex>, axis: usize, edge: f32, greater: bool) -> Vec<Vertex> {
    let mut result = Vec::new();
    for i in 0..vertices.len() {
        let a = vertices[i];
        let b = vertices[(i + 1) % vertices.len()];
        let distance = |v: Vertex| {
            let d = v.pos[axis] - edge;
            if greater { d } else { -d }
        };
        let da = distance(a);
        let db = distance(b);
        if da >= 0.0 {
            result.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            result.push(interpolate(a, b, da / (da - db)));
        }
    }
    result
}

fn split(ctx: &Context, snapshot: &Snapshot, direction: f32) -> Vec<Fragment> {
    const CELL: f32 = 12.0;
    let cols = (snapshot.rect.width() / CELL).ceil() as usize;
    let rows = (snapshot.rect.height() / CELL).ceil() as usize;
    let mut pieces: HashMap<(usize, TextureId), Mesh> = HashMap::new();
    for primitive in ctx.tessellate(snapshot.shapes.clone(), ctx.pixels_per_point()) {
        let Primitive::Mesh(mesh) = primitive.primitive else {
            continue;
        };
        let bounds = primitive.clip_rect.intersect(snapshot.rect);
        for indices in mesh.indices.as_chunks::<3>().0 {
            let triangle = [
                mesh.vertices[indices[0] as usize],
                mesh.vertices[indices[1] as usize],
                mesh.vertices[indices[2] as usize],
            ];
            let triangle_rect = Rect::from_points(&triangle.map(|v| v.pos)).intersect(bounds);
            if !triangle_rect.is_positive() {
                continue;
            }
            let start_x = ((triangle_rect.left() - snapshot.rect.left()) / CELL).floor().max(0.0) as usize;
            let end_x = ((triangle_rect.right() - snapshot.rect.left()) / CELL).floor() as usize;
            let start_y = ((triangle_rect.top() - snapshot.rect.top()) / CELL).floor().max(0.0) as usize;
            let end_y = ((triangle_rect.bottom() - snapshot.rect.top()) / CELL).floor() as usize;
            for row in start_y..=end_y.min(rows.saturating_sub(1)) {
                for col in start_x..=end_x.min(cols.saturating_sub(1)) {
                    let rect = Rect::from_min_size(
                        snapshot.rect.min + Vec2::new(col as f32 * CELL, row as f32 * CELL),
                        Vec2::splat(CELL),
                    )
                    .intersect(bounds);
                    let mut polygon = triangle.to_vec();
                    for (axis, edge, greater) in [
                        (0, rect.left(), true),
                        (0, rect.right(), false),
                        (1, rect.top(), true),
                        (1, rect.bottom(), false),
                    ] {
                        polygon = clip(polygon, axis, edge, greater);
                    }
                    if polygon.len() < 3 {
                        continue;
                    }
                    let piece = pieces
                        .entry((row * cols + col, mesh.texture_id))
                        .or_insert_with(|| Mesh::with_texture(mesh.texture_id));
                    let base = piece.vertices.len() as u32;
                    for i in 1..polygon.len() - 1 {
                        piece.add_triangle(base, base + i as u32, base + i as u32 + 1);
                    }
                    piece.vertices.extend(polygon);
                }
            }
        }
    }
    let mut by_cell: HashMap<usize, Vec<Mesh>> = HashMap::new();
    for ((cell, _), mesh) in pieces {
        by_cell.entry(cell).or_default().push(mesh);
    }
    by_cell
        .into_iter()
        .map(|(cell, meshes)| {
            let col = cell % cols;
            let row = cell / cols;
            let noise = (((cell as u32).wrapping_mul(1664525).wrapping_add(1013904223)) % 1000) as f32 / 1000.0;
            Fragment {
                center: snapshot.rect.min + Vec2::new((col as f32 + 0.5) * CELL, (row as f32 + 0.5) * CELL),
                release: 0.02 + 0.26 * (0.28 * col as f32 / cols as f32 + 0.72 * row as f32 / rows as f32),
                drift: Vec2::new(direction * (30.0 + 125.0 * noise), -45.0 - 125.0 * (1.0 - noise)),
                spin: (noise - 0.5) * 1.4,
                opacity: 1.0,
                meshes,
            }
        })
        .collect()
}

impl Transition {
    pub fn new(ctx: &Context, selected: Snapshot, others: Vec<Snapshot>) -> Self {
        let fragments = Arc::new(OnceLock::new());
        let result = fragments.clone();
        let worker_ctx = ctx.clone();
        let cards = others.clone();
        let lone = selected.clone();
        let center = selected.rect.center().x;
        rayon::spawn(move || {
            let pieces = if cards.is_empty() {
                echo(&worker_ctx, &lone)
            } else {
                cards
                    .iter()
                    .flat_map(|card| split(&worker_ctx, card, if card.rect.center().x < center { -1.0 } else { 1.0 }))
                    .collect()
            };
            let _ = result.set(pieces);
            worker_ctx.request_repaint();
        });
        Self {
            selected,
            others,
            fragments,
            started: ctx.input(|i| i.time),
            sent: false,
        }
    }

    /// Emit one connection request while the 0.7-second validation animates.
    /// Keep the account visible with live progress until success or a challenge.
    pub fn draw(&mut self, ctx: &Context, p: &Palette, progress: Option<(&str, f32)>) -> bool {
        let now = ctx.input(|i| i.time);
        let elapsed = (now - self.started).max(0.0) as f32;
        // Register the animation layer before the status area, so its snapshot
        // cannot cover the live progress (and normal modals remain above both).
        let painter = egui::Area::new(egui::Id::unique("login_derez"))
            .order(Order::Foreground)
            .interactable(false)
            .fade_in(false)
            .fixed_pos(ctx.content_rect().min)
            .show(ctx, |ui| {
                ui.set_min_size(ctx.content_rect().size());
                ui.painter().clone()
            })
            .inner;
        if let Some(fragments) = self.fragments.get().filter(|_| elapsed < 0.7) {
            let mut batches: HashMap<TextureId, Mesh> = HashMap::new();
            for piece in fragments {
                let age = ((elapsed - piece.release) / 0.42).max(0.0);
                if age >= 1.0 {
                    continue;
                }
                let fly = age * age * (2.0 - age);
                let scale = 1.0 - 0.82 * smooth(age);
                let fade = (1.0 - age).powf(1.55) * piece.opacity;
                let (sin, cos) = (piece.spin * fly).sin_cos();
                for mesh in &piece.meshes {
                    let batch = batches
                        .entry(mesh.texture_id)
                        .or_insert_with(|| Mesh::with_texture(mesh.texture_id));
                    let base = batch.vertices.len() as u32;
                    batch.indices.extend(mesh.indices.iter().map(|index| index + base));
                    batch.vertices.extend(mesh.vertices.iter().map(|v| {
                        let relative = (v.pos - piece.center) * scale;
                        let pos = piece.center
                            + piece.drift * fly
                            + Vec2::new(relative.x * cos - relative.y * sin, relative.x * sin + relative.y * cos);
                        Vertex {
                            pos,
                            color: v.color.gamma_multiply(fade),
                            ..*v
                        }
                    }));
                }
            }
            if let Some(mesh) = batches.remove(&TextureId::default()) {
                painter.add(Shape::mesh(mesh));
            }
            for mesh in batches.into_values() {
                painter.add(Shape::mesh(mesh));
            }
        } else {
            for card in &self.others {
                let fade = 1.0 - smooth(elapsed / 0.55);
                if fade > 0.0 {
                    for source in &card.shapes {
                        let mut faded = painter.with_clip_rect(source.clip_rect.intersect(card.rect));
                        faded.multiply_opacity(fade);
                        faded.add(source.shape.clone());
                    }
                }
            }
        }
        let movement = smooth((elapsed - 0.08) / 0.54);
        let offset = (ctx.content_rect().center() - self.selected.rect.center()) * movement;
        let scale = 1.0 + 0.025 * movement;
        paint_snapshot(&painter, &self.selected, offset, scale);
        let transform = egui::emath::TSTransform::new(offset + self.selected.rect.center().to_vec2() * (1.0 - scale), scale);
        let card = transform.mul_rect(self.selected.rect);
        let style = ctx.global_style();
        let pulse = 0.5 + 0.2 * (now as f32 * 5.0).sin();
        painter.rect_stroke(
            card,
            style.visuals.window_corner_radius,
            egui::Stroke::new(1.5, p.teal.gamma_multiply(pulse)),
            egui::StrokeKind::Inside,
        );
        let action = transform.mul_rect(self.selected.action);
        if action.is_positive() {
            painter.rect_filled(action, style.visuals.widgets.inactive.corner_radius, p.field);
            egui::Area::new(egui::Id::unique("account_connection_progress"))
                .order(Order::Foreground)
                .fade_in(false)
                .fixed_pos(action.min + Vec2::new(6.0, 3.0))
                .show(ctx, |ui| {
                    ui.set_width(action.width() - 12.0);
                    super::credentials_form::connection_progress(ui, p, progress);
                });
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        let connect = elapsed >= 0.1 && !self.sent && !ctx.will_discard();
        self.sent |= connect;
        connect
    }
}

fn paint_snapshot(painter: &egui::Painter, snapshot: &Snapshot, offset: Vec2, scale: f32) {
    let transform = egui::emath::TSTransform::new(offset + snapshot.rect.center().to_vec2() * (1.0 - scale), scale);
    for source in &snapshot.shapes {
        let mut shape = source.shape.clone();
        shape.transform(transform);
        painter
            .with_clip_rect(transform.mul_rect(source.clip_rect.intersect(snapshot.rect)))
            .add(shape);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipped_triangles_keep_uv_and_color() {
        let vertex = |x, y| Vertex {
            pos: Pos2::new(x, y),
            uv: Pos2::new(x, y),
            color: Color32::PLACEHOLDER,
        };
        let polygon = clip(vec![vertex(0.0, 0.0), vertex(2.0, 0.0), vertex(0.0, 2.0)], 0, 1.0, true);
        assert_eq!(polygon.len(), 3);
        assert!(polygon.iter().all(|v| v.pos.x >= 1.0 && v.pos == v.uv));
    }

    #[test]
    fn a_lone_card_bursts_its_own_echo_to_both_sides() {
        let ctx = Context::default();
        let p = crate::theme::Theme::default().palette();
        let rect = Rect::from_min_size(Pos2::new(160.0, 100.0), Vec2::new(264.0, 400.0));
        let card = Snapshot {
            rect,
            action: Rect::NOTHING,
            shapes: vec![ClippedShape::new(rect, Shape::rect_filled(rect, 0.0, p.panel))],
        };
        // tessellation needs the fonts of a first frame
        let _ = super::super::test_output(ctx.run_ui(egui::RawInput::default(), |_| {}));
        let pieces = echo(&ctx, &card);
        assert!(!pieces.is_empty());
        let left = pieces.iter().filter(|piece| piece.drift.x < 0.0).count();
        let right = pieces.iter().filter(|piece| piece.drift.x > 0.0).count();
        assert!(left > 0 && right > 0, "{left} fragments left, {right} right");
        assert!(
            pieces
                .iter()
                .all(|piece| (piece.drift.x < 0.0) == (piece.center.x < rect.center().x))
        );
        assert!(pieces.iter().all(|piece| piece.opacity == ECHO_OPACITY));
        // neighbours keep their own full-opacity burst away from the chosen card
        let neighbour = split(&ctx, &card, 1.0);
        assert!(neighbour.iter().all(|piece| piece.drift.x > 0.0 && piece.opacity == 1.0));
    }

    #[test]
    fn validation_requests_once_without_waiting_for_fragments_and_keeps_progress_in_the_card() {
        let ctx = Context::default();
        let p = crate::theme::Theme::default().palette();
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0));
        let rect = Rect::from_min_size(Pos2::new(160.0, 100.0), Vec2::new(264.0, 400.0));
        let action = Rect::from_min_size(Pos2::new(176.0, 448.0), Vec2::new(232.0, 36.0));
        let mut transition = Transition {
            selected: Snapshot {
                rect,
                action,
                shapes: vec![ClippedShape::new(screen, Shape::rect_filled(rect, 2.0, p.panel))],
            },
            others: Vec::new(),
            fragments: Arc::new(OnceLock::new()),
            started: 0.0,
            sent: false,
        };
        let mut requests = 0;
        let mut output = None;
        for time in [0.05, 0.12, 0.3, 0.7, 1.2, 5.0] {
            output = Some(super::super::test_output(ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| {
                    if transition.draw(ui.ctx(), &p, Some((aurora_i18n::tr!("session-connecting-to-login-server"), 0.35))) {
                        requests += 1;
                    }
                },
            )));
            if time == 0.05 {
                assert_eq!(requests, 0);
            } else {
                assert_eq!(requests, 1);
            }
        }
        let output = output.expect("synthetic frame");
        let centered = Rect::from_center_size(screen.center(), rect.size() * 1.025);
        let progress_text = output
            .shapes
            .iter()
            .find_map(|source| match &source.shape {
                Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("session-connecting-to-login-server") => {
                    Some(Rect::from_min_size(text.pos, text.galley.size()))
                }
                _ => None,
            })
            .expect("in-card server progress");
        assert!(centered.contains_rect(progress_text));
        let text_index = output.shapes.iter().position(|source| matches!(&source.shape, Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("session-connecting-to-login-server"))).expect("status text");
        let cover_index = output
            .shapes
            .iter()
            .position(|source| matches!(&source.shape, Shape::Rect(rect) if rect.fill == p.field))
            .expect("status background");
        assert!(text_index > cover_index, "the status must paint above the account snapshot");
        assert!(
            !output
                .shapes
                .iter()
                .any(|source| matches!(&source.shape, Shape::Rect(rect) if rect.rect == screen && rect.fill.a() == 255))
        );
    }
}

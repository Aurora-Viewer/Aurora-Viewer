//! Floater edges follow LLResizeBar::handleHover (indra/llui/llresizebar.cpp):
//! clamp the size before moving the dragged edge, keeping its opposite fixed.
//! egui 0.36's window edges move the area before applying the size constraint.
use crate::theme::Palette;
use egui::epaint::tessellator::path::add_circle_quadrant;
use egui::{Context, CursorIcon, Id, Pos2, Rect, Sense, Shape, Stroke, Ui, UiBuilder, Vec2, pos2, vec2};

/// Grab areas, from the window's outline (points). Firestorm's corner
/// handle is 11 px, inside the floater (LLResizeHandle,
/// RESIZE_HANDLE_WIDTH), its side bars 3 px (LLFloater,
/// RESIZE_BAR_THICKNESS): the corners keep a 16 point grip inside the
/// window plus a little outside it, however small the mark drawn there.
const CORNER_IN: f32 = 16.0;
const CORNER_OUT: f32 = 4.0;
const SIDE_IN: f32 = 4.0;
const SIDE_OUT: f32 = 3.0;

/// The mark of the bottom-right corner: an arc of the frame, as thick as
/// two outlines, whose arms reach this far along the edges at rest and
/// when the corner is hovered or dragged.
const ARC_WIDTH: f32 = 2.0;
const ARC_REST: f32 = 11.0;
const ARC_HOT: f32 = 16.0;
/// Strength of the outline of the edges that would move, while hovered
/// (they are fully lit while dragged).
const LIT_HOVER: f32 = 0.7;

/// An edge/corner owns the current drag, even before its window is drawn.
/// Inspect all visible windows so menus close independently of their owner
/// and drawing order; hovering a grip or dragging content is not a resize.
pub(super) fn active(ctx: &Context) -> bool {
    let Some(dragged) = ctx.dragged_id() else { return false };
    ctx.memory(|memory| memory.areas().visible_layer_ids())
        .into_iter()
        .any(|layer| Edge::ALL.into_iter().any(|edge| edge.id(layer.id) == dragged))
}

/// Test the painted handles at the event's position, including their grip
/// outside the area. egui's area hit test and pointer still describe the last
/// frame when winit delivers a move followed by a press before the next frame.
pub(super) fn contains_pointer(ctx: &Context, pos: Pos2) -> bool {
    // Match egui's hit_test distance, including its rounded tolerance around
    // each grip: the raw interact_rect alone leaves that margin to the world.
    let radius = ctx.global_style().interaction.interact_radius;
    let layers = ctx.memory(|memory| memory.areas().visible_layer_ids());
    layers.into_iter().any(|layer| {
        Edge::ALL.into_iter().any(|edge| {
            ctx.read_response(edge.id(layer.id)).is_some_and(|response| {
                response.enabled()
                    && !response.interact_rect.is_negative()
                    && response.interact_rect.distance_sq_to_pos(pos) <= radius * radius
            })
        })
    })
}

#[derive(Clone, Copy)]
enum Edge {
    Left,
    Right,
    Top,
    Bottom,
    LeftTop,
    LeftBottom,
    RightTop,
    RightBottom,
}

impl Edge {
    const ALL: [Self; 8] = [
        Self::Left,
        Self::Right,
        Self::Top,
        Self::Bottom,
        Self::LeftTop,
        Self::LeftBottom,
        Self::RightTop,
        Self::RightBottom,
    ];

    fn id(self, window: Id) -> Id {
        window.with(("floater_edge", self as u8))
    }
    fn left(self) -> bool {
        matches!(self, Self::Left | Self::LeftTop | Self::LeftBottom)
    }
    fn right(self) -> bool {
        matches!(self, Self::Right | Self::RightTop | Self::RightBottom)
    }
    fn top(self) -> bool {
        matches!(self, Self::Top | Self::LeftTop | Self::RightTop)
    }
    fn bottom(self) -> bool {
        matches!(self, Self::Bottom | Self::LeftBottom | Self::RightBottom)
    }

    fn cursor(self) -> CursorIcon {
        match self {
            Self::Left | Self::Right => CursorIcon::ResizeHorizontal,
            Self::Top | Self::Bottom => CursorIcon::ResizeVertical,
            Self::LeftTop | Self::RightBottom => CursorIcon::ResizeNwSe,
            Self::LeftBottom | Self::RightTop => CursorIcon::ResizeNeSw,
        }
    }

    /// The edges a window resized in height only offers: the top and the
    /// bottom, along the whole width (no corner).
    fn moves_height_only(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }

    /// Where this edge of the window `rect` (its outline) is grabbed; along
    /// the whole side for a window resized in height only.
    fn hit_rect_of(self, rect: Rect, height_only: bool) -> Rect {
        match self {
            Self::Top if height_only => Rect::from_x_y_ranges(rect.x_range(), rect.top() - SIDE_OUT..=rect.top() + SIDE_IN),
            Self::Bottom if height_only => Rect::from_x_y_ranges(rect.x_range(), rect.bottom() - SIDE_IN..=rect.bottom() + SIDE_OUT),
            _ => self.hit_rect(rect),
        }
    }

    /// Where this edge of the window `rect` (its outline) is grabbed.
    fn hit_rect(self, rect: Rect) -> Rect {
        // `out` points away from the window
        let corner = |pos: Pos2, out: Vec2| Rect::from_two_pos(pos - out * CORNER_IN, pos + out * CORNER_OUT);
        let (x, y) = (rect.x_range().shrink(CORNER_IN), rect.y_range().shrink(CORNER_IN));
        match self {
            Self::Left => Rect::from_x_y_ranges(rect.left() - SIDE_OUT..=rect.left() + SIDE_IN, y),
            Self::Right => Rect::from_x_y_ranges(rect.right() - SIDE_IN..=rect.right() + SIDE_OUT, y),
            Self::Top => Rect::from_x_y_ranges(x, rect.top() - SIDE_OUT..=rect.top() + SIDE_IN),
            Self::Bottom => Rect::from_x_y_ranges(x, rect.bottom() - SIDE_IN..=rect.bottom() + SIDE_OUT),
            Self::LeftTop => corner(rect.left_top(), vec2(-1.0, -1.0)),
            Self::LeftBottom => corner(rect.left_bottom(), vec2(-1.0, 1.0)),
            Self::RightTop => corner(rect.right_top(), vec2(1.0, -1.0)),
            Self::RightBottom => corner(rect.right_bottom(), vec2(1.0, 1.0)),
        }
    }

    /// The sides this edge moves: left, right, top, bottom.
    fn sides(self) -> [bool; 4] {
        [self.left(), self.right(), self.top(), self.bottom()]
    }
}

#[derive(Clone, Copy)]
pub(super) struct Drag {
    start: Rect,
    edge: Edge,
    minimum: Vec2,
    delta: Vec2,
    bounds: Rect,
}

impl Drag {
    /// The edges that follow the pointer, for the snapping (`layout::resizing`).
    pub(super) fn edges(self) -> crate::ui::snap::Edges {
        crate::ui::snap::Edges {
            left: self.edge.left(),
            right: self.edge.right(),
            top: self.edge.top(),
            bottom: self.edge.bottom(),
        }
    }

    /// The least size of the window, as known so far in this gesture.
    pub(super) fn minimum(self) -> Vec2 {
        self.minimum
    }

    pub(super) fn rect(self) -> Rect {
        let mut rect = self.start;
        if self.edge.left() {
            rect.min.x = (self.start.left() + self.delta.x)
                .min(self.start.right() - self.minimum.x)
                .max(self.bounds.left());
        } else if self.edge.right() {
            rect.max.x = (self.start.right() + self.delta.x)
                .max(self.start.left() + self.minimum.x)
                .min(self.bounds.right());
        }
        if self.edge.top() {
            rect.min.y = (self.start.top() + self.delta.y)
                .min(self.start.bottom() - self.minimum.y)
                .max(self.bounds.top());
        } else if self.edge.bottom() {
            rect.max.y = (self.start.bottom() + self.delta.y)
                .max(self.start.top() + self.minimum.y)
                .min(self.bounds.bottom());
        }
        rect
    }

    /// `rect` (this frame's [`Self::rect`], snapped) for a window whose body
    /// keeps `ratio` (width over height) and which adds `chrome` around it
    /// (its title bar): see [`aspect_rect`].
    pub(super) fn keep_aspect(self, rect: Rect, ratio: f32, chrome: Vec2) -> Rect {
        aspect_rect(self.start, rect, self.edge, ratio, chrome, self.minimum, self.bounds)
    }

    pub(super) fn prepare(ctx: &Context, window: Id, minimum: Vec2) -> Option<Self> {
        let mut drag = ctx.data_mut(|data| data.get_temp::<Self>(window.with("floater_resize")));
        if drag.is_some() && ctx.input(|input| !input.pointer.primary_down() && !input.pointer.primary_released()) {
            ctx.data_mut(|data| data.remove::<Self>(window.with("floater_resize")));
            return None;
        }
        if drag.is_none() {
            let edge = Edge::ALL
                .into_iter()
                .find(|edge| ctx.read_response(edge.id(window)).is_some_and(|response| response.dragged()))?;
            let start = ctx.memory(|memory| memory.area_rect(window))?;
            drag = Some(Self {
                start,
                edge,
                minimum,
                delta: Vec2::ZERO,
                bounds: ctx.content_rect(),
            });
        }
        if let Some(drag) = &mut drag
            && let Some(delta) = ctx.input(|input| input.pointer.total_drag_delta())
        {
            drag.delta = delta;
        }
        drag
    }

    /// `requested` is the rect asked of the window this frame (`rect`, or
    /// its snapped version), `actual` the one it took.
    pub(super) fn finish(mut self, ctx: &Context, window: Id, requested: Rect, actual: Rect) {
        // Content can require more room than the configured minimum. Repeat
        // layout at that size while this gesture owns the pointer, preserving
        // the opposite edge even for a newly reflowed toolbar or dialog.
        let overflow = actual.size() - requested.size();
        let changed =
            (self.edge.left() || self.edge.right()) && overflow.x > 0.5 || (self.edge.top() || self.edge.bottom()) && overflow.y > 0.5;
        if changed {
            if overflow.x > 0.5 {
                self.minimum.x = self.minimum.x.max(actual.width());
            }
            if overflow.y > 0.5 {
                self.minimum.y = self.minimum.y.max(actual.height());
            }
            ctx.request_discard("floater content requires a larger minimum size");
        }
        let held = ctx.input(|input| input.pointer.primary_down());
        ctx.data_mut(|data| {
            if held || changed {
                data.insert_temp(window.with("floater_resize"), self);
            } else {
                data.remove::<Self>(window.with("floater_resize"));
                if (actual.size() - self.start.size()).length_sq() > 0.25 {
                    data.insert_temp(window.with("floater_resized"), actual.size());
                }
            }
        });
    }
}

/// The rect of a window that keeps its body at `ratio` (width over height),
/// `chrome` around it, while `edge` drags it from `start` to `rect`, at
/// every frame of the drag (#941): a side drives the other dimension, a
/// corner follows the axis the pointer changed the most (relative to the
/// size it started from). The edges opposite the dragged ones stay where
/// they were; a side grows the other dimension away from the window's top
/// or left. The size stays between `minimum` and the room `bounds` leaves
/// on the sides that move, both taken on the ratio.
fn aspect_rect(start: Rect, rect: Rect, edge: Edge, ratio: f32, chrome: Vec2, minimum: Vec2, bounds: Rect) -> Rect {
    let ratio = ratio.clamp(0.2, 5.0);
    let body = (rect.size() - chrome).max(Vec2::splat(1.0));
    let (from_width, from_height) = (body.x, body.y * ratio);
    let width = match (edge.left() || edge.right(), edge.top() || edge.bottom()) {
        (true, false) => from_width,
        (false, true) => from_height,
        _ => {
            let begun = (start.size() - chrome).max(Vec2::splat(1.0));
            if (body.x / begun.x - 1.0).abs() >= (body.y / begun.y - 1.0).abs() {
                from_width
            } else {
                from_height
            }
        }
    };
    let room_x = if edge.left() {
        start.right() - bounds.left()
    } else {
        bounds.right() - start.left()
    };
    let room_y = if edge.top() {
        start.bottom() - bounds.top()
    } else {
        bounds.bottom() - start.top()
    };
    let most = (room_x - chrome.x).min((room_y - chrome.y) * ratio).max(1.0);
    let least = (minimum.x - chrome.x).max((minimum.y - chrome.y) * ratio).min(most);
    let width = width.clamp(least, most);
    let size = Vec2::new(width, width / ratio) + chrome;
    let x = if edge.left() { start.right() - size.x } else { start.left() };
    let y = if edge.top() { start.bottom() - size.y } else { start.top() };
    Rect::from_min_size(pos2(x, y), size)
}

/// The edges and corners that resize the window of `response`, and their
/// marks: an arc of the frame around the bottom-right corner, in
/// `muted_dim` at rest, longer and in `violet_light` when hovered or
/// dragged; and the outline of the sides that would move, lit in
/// `violet_light` meanwhile (as egui's own windows light theirs,
/// `paint_frame_interaction`), both faded in and out.
pub(super) fn handles(ctx: &Context, p: &Palette, response: &egui::Response) {
    handles_of(ctx, p, response, false);
}

/// [`handles`]; `height_only`: the top and bottom edges alone, without the
/// corner's mark (a window whose width is set by what it lines up with).
pub(super) fn handles_of(ctx: &Context, p: &Palette, response: &egui::Response, height_only: bool) {
    let window = response.layer_id.id;
    let mut ui = Ui::new(
        ctx.clone(),
        response.id.with("floater_resize_ui"),
        UiBuilder::new().layer_id(response.layer_id).max_rect(response.rect),
    );
    ui.set_clip_rect(ctx.content_rect());
    ui.set_min_size(response.rect.size());
    let (mut hovered, mut dragged, mut corner) = ([false; 4], [false; 4], false);
    for edge in Edge::ALL.into_iter().filter(|edge| !height_only || edge.moves_height_only()) {
        let response = ui
            .interact_opt(
                edge.hit_rect_of(response.rect, height_only),
                edge.id(window),
                Sense::DRAG,
                egui::InteractOptions { move_to_top: true },
            )
            .on_hover_cursor(edge.cursor());
        response.widget_info(|| egui::WidgetInfo::new(egui::Role::Splitter));
        if response.dragged() {
            ctx.set_cursor_icon(edge.cursor());
        }
        for (side, moves) in edge.sides().into_iter().enumerate() {
            hovered[side] |= moves && response.hovered();
            dragged[side] |= moves && response.dragged();
        }
        if matches!(edge, Edge::RightBottom) {
            corner = response.hovered() || response.dragged();
        }
    }
    let (lit, strength) = if dragged.contains(&true) {
        (dragged, 1.0)
    } else {
        (hovered, LIT_HOVER)
    };
    let lit = std::array::from_fn(|side| strength * ctx.animate_bool(window.with(("floater_edge_lit", side)), lit[side]));
    let painter = ui.painter();
    outline(painter, p, response.rect, lit);
    if !height_only {
        corner_arc(
            painter,
            p,
            response.rect,
            ctx.animate_bool(window.with("floater_corner_lit"), corner),
        );
    }
}

/// The outline of the window `rect` along the sides lit (left, right, top,
/// bottom: how much), over its frame, with the window's rounded corners.
fn outline(painter: &egui::Painter, p: &Palette, rect: Rect, lit: [f32; 4]) {
    let [left, right, top, bottom] = lit;
    let radius = painter.ctx().global_style().visuals.window_corner_radius;
    // on the outline's pixels, as egui paints the frame
    let line = rect.shrink(0.5);
    let r = |corner: u8| (f32::from(corner) - 0.5).max(0.0);
    let (nw, ne, sw, se) = (r(radius.nw), r(radius.ne), r(radius.sw), r(radius.se));
    let stroke = |strength: f32| Stroke::new(1.0, p.violet_light.gamma_multiply(strength));
    let side = |strength: f32, from: Pos2, to: Pos2| {
        if strength > 0.0 {
            painter.line_segment([from, to], stroke(strength));
        }
    };
    side(left, pos2(line.left(), line.top() + nw), pos2(line.left(), line.bottom() - sw));
    side(right, pos2(line.right(), line.top() + ne), pos2(line.right(), line.bottom() - se));
    side(top, pos2(line.left() + nw, line.top()), pos2(line.right() - ne, line.top()));
    side(
        bottom,
        pos2(line.left() + sw, line.bottom()),
        pos2(line.right() - se, line.bottom()),
    );
    // a corner between two lit sides (quadrants: 0 bottom-right, 1
    // bottom-left, 2 top-left, 3 top-right)
    let corners = [
        (right.min(bottom), pos2(line.right() - se, line.bottom() - se), se, 0.0),
        (left.min(bottom), pos2(line.left() + sw, line.bottom() - sw), sw, 1.0),
        (left.min(top), pos2(line.left() + nw, line.top() + nw), nw, 2.0),
        (right.min(top), pos2(line.right() - ne, line.top() + ne), ne, 3.0),
    ];
    for (strength, center, radius, quadrant) in corners {
        if strength > 0.0 && radius > 0.0 {
            let mut points = Vec::new();
            add_circle_quadrant(&mut points, center, radius, quadrant);
            painter.add(Shape::line(points, stroke(strength)));
        }
    }
}

/// The mark of the bottom-right corner: the frame thickened around its
/// rounded corner, `hot` (0 at rest, 1 hovered or dragged) lengthening it
/// and turning it from `muted_dim` to `violet_light`.
fn corner_arc(painter: &egui::Painter, p: &Palette, rect: Rect, hot: f32) {
    let radius = f32::from(painter.ctx().global_style().visuals.window_corner_radius.se);
    // concentric with the outline, half the mark's width in: the inner
    // radius of the theme (`widgets::inner_radius`), never rounder
    let inset = ARC_WIDTH / 2.0;
    let arc = (radius - inset).max(0.0);
    let center = rect.max - Vec2::splat(inset + arc);
    let reach = egui::lerp(ARC_REST..=ARC_HOT, hot);
    let mut points = vec![pos2(rect.right() - inset, rect.bottom() - reach)];
    if arc > 0.0 {
        add_circle_quadrant(&mut points, center, arc, 0.0);
    } else {
        points.push(center);
    }
    points.push(pos2(rect.right() - reach, rect.bottom() - inset));
    painter.add(Shape::line(
        points,
        Stroke::new(ARC_WIDTH, p.muted_dim.lerp_to_gamma(p.violet_light, hot)),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widgets::Floater;

    struct Fixture {
        ctx: Context,
        frame: usize,
        content_width: f32,
    }

    impl Fixture {
        fn show(&mut self, events: Vec<egui::Event>) -> Rect {
            self.frame += 1;
            let palette = crate::theme::Theme::default().palette();
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                    time: Some(self.frame as f64 / 60.0),
                    events,
                    ..Default::default()
                },
                |ui| {
                    Floater::new(
                        "resize_fixture",
                        aurora_i18n::tr!("common-inventory"),
                        egui::pos2(300.0, 200.0),
                        egui::vec2(440.0, 300.0),
                    )
                    .silent()
                    .show(ui.ctx(), &palette, &mut true, |ui| {
                        ui.set_min_size(egui::vec2(self.content_width, 40.0));
                        ui.set_min_height(ui.available_height());
                    });
                },
            );
            output.textures_delta.clear();
            self.ctx.memory(|memory| memory.area_rect(Id::unique("resize_fixture")).unwrap())
        }

        fn press(&mut self, pos: egui::Pos2) {
            self.show(vec![egui::Event::PointerMoved(pos)]);
            self.show(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }]);
        }

        fn release(&mut self, pos: egui::Pos2) -> Rect {
            self.show(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }]);
            self.show(vec![])
        }
    }

    #[test]
    fn construction_resize_grips_block_world_events_before_the_next_ui_frame() {
        for edge in Edge::ALL {
            let mut fixture = Fixture {
                ctx: Context::default(),
                frame: 0,
                content_width: 0.0,
            };
            let mut rect = fixture.show(vec![]);
            for _ in 0..8 {
                rect = fixture.show(vec![]);
            }
            let world = egui::pos2(80.0, 80.0);
            fixture.show(vec![egui::Event::PointerMoved(world)]);
            assert!(!crate::ui::widgets::pointer_over_ui(&fixture.ctx, world));
            let mut grip = rect.center();
            if edge.left() {
                grip.x = rect.left() - 2.0;
            }
            if edge.right() {
                grip.x = rect.right() + 2.0;
            }
            if edge.top() {
                grip.y = rect.top() - 2.0;
            }
            if edge.bottom() {
                grip.y = rect.bottom() + 2.0;
            }
            assert!(!rect.contains(grip));
            assert!(!fixture.ctx.is_pointer_over_egui(), "egui still has the previous world position");
            assert!(
                crate::ui::widgets::pointer_over_ui(&fixture.ctx, grip),
                "edge {} leaks its press",
                edge as u8
            );
            fixture.press(grip);
            let target = grip + egui::vec2(60.0, 45.0);
            fixture.show(vec![egui::Event::PointerMoved(target)]);
            assert!(
                crate::ui::widgets::pointer_over_ui(&fixture.ctx, target),
                "resize must retain capture"
            );
            fixture.release(target);
            fixture.show(vec![egui::Event::PointerMoved(world)]);
            fixture.show(vec![]);
            assert!(
                !crate::ui::widgets::pointer_over_ui(&fixture.ctx, world),
                "world clicks resume after the release"
            );
        }
    }

    #[test]
    fn resize_grip_tolerance_blocks_world_events_before_the_next_ui_frame() {
        for scale in [1.0, 1.5, 2.0] {
            for radius in [5.0, 9.0] {
                for edge in Edge::ALL {
                    let mut fixture = Fixture {
                        ctx: Context::default(),
                        frame: 0,
                        content_width: 0.0,
                    };
                    fixture.ctx.set_pixels_per_point(scale);
                    fixture.ctx.global_style_mut(|style| style.interaction.interact_radius = radius);
                    let mut rect = fixture.show(vec![]);
                    for _ in 0..8 {
                        rect = fixture.show(vec![]);
                    }
                    let world = pos2(80.0, 80.0);
                    fixture.show(vec![egui::Event::PointerMoved(world)]);
                    let hit = edge.hit_rect(rect);
                    let mut border = hit.center();
                    let mut outward = Vec2::ZERO;
                    if edge.left() {
                        border.x = hit.left();
                        outward.x = -1.0;
                    } else if edge.right() {
                        border.x = hit.right();
                        outward.x = 1.0;
                    }
                    if edge.top() {
                        border.y = hit.top();
                        outward.y = -1.0;
                    } else if edge.bottom() {
                        border.y = hit.bottom();
                        outward.y = 1.0;
                    }
                    let outward = outward.normalized();
                    let grip = border + outward * radius * 0.75;
                    assert!(!hit.contains(grip));
                    assert!(!fixture.ctx.is_pointer_over_egui(), "egui still has the previous world position");
                    assert!(
                        crate::ui::widgets::pointer_over_ui(&fixture.ctx, grip),
                        "edge {} leaks its tolerance margin at scale {scale}, radius {radius}",
                        edge as u8
                    );
                    assert!(
                        !crate::ui::widgets::pointer_over_ui(&fixture.ctx, border + outward * (radius + 1.0)),
                        "outside egui's tolerance, the world must remain interactive"
                    );
                    fixture.press(grip);
                    assert_eq!(fixture.ctx.dragged_id(), Some(edge.id(Id::unique("resize_fixture"))));
                    let target = grip + outward * 60.0;
                    let resized = fixture.show(vec![egui::Event::PointerMoved(target)]);
                    assert_ne!(resized, rect, "the tolerance margin must actually resize the window");
                    assert!(crate::ui::widgets::pointer_over_ui(&fixture.ctx, world), "the drag retains capture");
                    fixture.release(target);
                    fixture.show(vec![egui::Event::PointerMoved(world)]);
                    fixture.show(vec![]);
                    assert!(!crate::ui::widgets::pointer_over_ui(&fixture.ctx, world));
                }
            }
        }
    }

    #[test]
    fn all_edges_and_corners_stop_at_the_minimum_without_moving_the_opposite_edge() {
        for content_width in [0.0, 320.0] {
            for edge in Edge::ALL {
                let mut fixture = Fixture {
                    ctx: Context::default(),
                    frame: 0,
                    content_width,
                };
                let mut original = fixture.show(vec![]);
                for _ in 0..8 {
                    original = fixture.show(vec![]);
                }
                let pos = edge.hit_rect(original).center();
                fixture.press(pos);
                let inward = egui::vec2(
                    if edge.left() {
                        original.width() + 80.0
                    } else if edge.right() {
                        -original.width() - 80.0
                    } else {
                        0.0
                    },
                    if edge.top() {
                        original.height() + 80.0
                    } else if edge.bottom() {
                        -original.height() - 80.0
                    } else {
                        0.0
                    },
                );
                let fixed = |rect: Rect| {
                    let anchor_x = if edge.left() { rect.right() } else { rect.left() };
                    let anchor_y = if edge.top() { rect.bottom() } else { rect.top() };
                    let expected_x = if edge.left() { original.right() } else { original.left() };
                    let expected_y = if edge.top() { original.bottom() } else { original.top() };
                    assert!(
                        (anchor_x - expected_x).abs() < 1.0 && (anchor_y - expected_y).abs() < 1.0,
                        "edge={}, content={content_width}: {original:?} -> {rect:?}",
                        edge as u8
                    );
                };
                let mut limit = None;
                for fraction in [1.0, 1.2, 1.4] {
                    let target = pos + inward * fraction;
                    let rect = fixture.show(vec![egui::Event::PointerMoved(target)]);
                    fixed(rect);
                    assert!(rect.width() >= 219.0 && rect.height() >= 119.0);
                    if let Some(limit) = limit {
                        assert_eq!(rect, limit, "moving further past the limit must leave the window in place");
                    }
                    limit = Some(rect);
                }
                fixed(fixture.release(pos + inward * 1.4));
                let limit = limit.unwrap();
                // Start a new gesture at the actual edge, then expand normally.
                let pos = edge.hit_rect(limit).center();
                fixture.press(pos);
                let target = pos - inward * 0.2;
                let expanded = fixture.show(vec![egui::Event::PointerMoved(target)]);
                fixed(expanded);
                assert!(expanded.width() > limit.width() || expanded.height() > limit.height());
                assert_eq!(fixture.release(target), expanded);
                if content_width == 0.0 && matches!(edge, Edge::Left) {
                    let pos = expanded.center();
                    fixture.press(pos);
                    let movement = egui::vec2(20.0, 15.0);
                    let moved = fixture.show(vec![egui::Event::PointerMoved(pos + movement)]);
                    assert_eq!(moved, expanded.translate(movement), "ordinary window dragging still works");
                    assert_eq!(fixture.release(pos + movement), moved);
                }
            }
        }
    }

    #[test]
    fn resize_handles_preserve_hover_and_clicks_inside_the_floater() {
        let ctx = Context::default();
        let mut center = egui::Pos2::ZERO;
        let mut hovered = false;
        let mut clicks = 0;
        let mut frame = 0;
        let mut show = |events| {
            frame += 1;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                    time: Some(frame as f64 / 60.0),
                    events,
                    ..Default::default()
                },
                |ui| {
                    Floater::new(
                        "button_fixture",
                        aurora_i18n::tr!("common-inventory"),
                        egui::pos2(300.0, 200.0),
                        egui::vec2(440.0, 300.0),
                    )
                    .silent()
                    .show(ui.ctx(), &crate::theme::Theme::default().palette(), &mut true, |ui| {
                        let response = ui.button("Une action");
                        center = response.rect.center();
                        hovered = response.hovered();
                        clicks += usize::from(response.clicked());
                    });
                },
            );
            output.textures_delta.clear();
            (center, hovered, clicks)
        };
        let mut pos = show(vec![]).0;
        for _ in 0..8 {
            pos = show(vec![]).0;
        }
        let result = show(vec![egui::Event::PointerMoved(pos)]);
        assert!(result.1, "the resize overlay must not hide body hover interactions");
        show(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }]);
        let result = show(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
        assert_eq!(result.2, 1, "body actions remain clickable");
    }

    #[test]
    fn grab_areas_reach_16_points_into_the_corners_and_a_few_outside() {
        let window = Rect::from_min_size(egui::pos2(300.0, 200.0), egui::vec2(440.0, 300.0));
        let corner = Edge::RightBottom.hit_rect(window);
        assert_eq!(corner.size(), Vec2::splat(CORNER_IN + CORNER_OUT));
        assert_eq!(corner.min, window.max - Vec2::splat(CORNER_IN));
        assert_eq!(Edge::LeftTop.hit_rect(window).max, window.min + Vec2::splat(CORNER_IN));
        let right = Edge::Right.hit_rect(window);
        assert_eq!(
            right.x_range(),
            egui::Rangef::new(window.right() - SIDE_IN, window.right() + SIDE_OUT)
        );
        assert_eq!(
            right.y_range(),
            egui::Rangef::new(window.top() + CORNER_IN, window.bottom() - CORNER_IN)
        );
        let top = Edge::Top.hit_rect(window);
        assert_eq!(top.y_range(), egui::Rangef::new(window.top() - SIDE_OUT, window.top() + SIDE_IN));
        const { assert!(CORNER_IN + CORNER_OUT >= 16.0 && SIDE_IN + SIDE_OUT >= 6.0) };
        // every point of the outline is grabbed by exactly one edge or corner
        for t in [0.0, 0.5, 15.0, 17.0, 150.0, 299.5] {
            let point = egui::pos2(window.right() - 0.5, window.top() + t);
            let hits = Edge::ALL.iter().filter(|edge| edge.hit_rect(window).contains(point)).count();
            assert_eq!(hits, 1, "{t}");
        }
    }

    /// The body's width over height of a window `rect` with `chrome`.
    fn body_ratio(rect: Rect, chrome: Vec2) -> f32 {
        (rect.width() - chrome.x) / (rect.height() - chrome.y)
    }

    #[test]
    fn a_window_of_fixed_shape_keeps_it_whatever_edge_drags_it() {
        let bounds = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0));
        let chrome = egui::vec2(0.0, 32.0);
        let minimum = egui::vec2(220.0, 120.0);
        for ratio in [16.0 / 9.0, 4.0 / 3.0, 9.0 / 16.0, 21.0 / 9.0] {
            let width = if ratio < 1.0 { 300.0 } else { 480.0 };
            let start = Rect::from_min_size(egui::pos2(600.0, 300.0), egui::vec2(width, width / ratio) + chrome);
            for edge in Edge::ALL {
                for delta in [
                    egui::vec2(90.0, 20.0),
                    egui::vec2(-60.0, -15.0),
                    egui::vec2(10.0, 140.0),
                    egui::vec2(-5.0, -120.0),
                ] {
                    let drag = Drag {
                        start,
                        edge,
                        minimum,
                        delta,
                        bounds,
                    };
                    let rect = drag.keep_aspect(drag.rect(), ratio, chrome);
                    assert!(
                        (body_ratio(rect, chrome) - ratio).abs() < 1e-3,
                        "edge {} by {delta:?} at {ratio}: {rect:?}",
                        edge as u8
                    );
                    // the opposite edges stay; a side grows the other way from the top or left
                    let anchor_x = if edge.left() { rect.right() } else { rect.left() };
                    let anchor_y = if edge.top() { rect.bottom() } else { rect.top() };
                    assert_eq!(anchor_x, if edge.left() { start.right() } else { start.left() });
                    assert_eq!(anchor_y, if edge.top() { start.bottom() } else { start.top() });
                    assert!(bounds.contains_rect(rect.shrink(0.01)), "{rect:?}");
                    assert!(rect.width() >= minimum.x - 0.01 && rect.height() >= minimum.y - 0.01);
                }
            }
        }
    }

    #[test]
    fn a_side_drives_the_other_dimension_and_a_corner_its_larger_change() {
        let bounds = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0));
        let chrome = egui::vec2(0.0, 30.0);
        let start = Rect::from_min_size(egui::pos2(400.0, 200.0), egui::vec2(480.0, 300.0));
        let drag = |edge, delta| {
            let drag = Drag {
                start,
                edge,
                minimum: egui::vec2(220.0, 120.0),
                delta,
                bounds,
            };
            drag.keep_aspect(drag.rect(), 16.0 / 9.0, chrome)
        };
        // the width dragged 160 wider: 90 taller, the vertical motion ignored
        assert_eq!(drag(Edge::Right, egui::vec2(160.0, 40.0)).size(), egui::vec2(640.0, 390.0));
        // the height dragged 90 taller: 160 wider
        assert_eq!(drag(Edge::Bottom, egui::vec2(-30.0, 90.0)).size(), egui::vec2(640.0, 390.0));
        // a corner: +160 in width is +33 %, +27 in height +10 %: the width leads
        assert_eq!(drag(Edge::RightBottom, egui::vec2(160.0, 27.0)).size(), egui::vec2(640.0, 390.0));
        // +16 in width is +3 %, +90 in height +33 %: the height leads
        assert_eq!(drag(Edge::LeftTop, egui::vec2(-16.0, -90.0)).size(), egui::vec2(640.0, 390.0));
    }

    #[test]
    fn a_window_of_fixed_shape_stops_at_its_minimum_and_at_the_screen_on_the_ratio() {
        let bounds = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0));
        let chrome = egui::vec2(0.0, 30.0);
        let minimum = egui::vec2(220.0, 120.0);
        // 21:9: the minimum width (220) leads, the height follows it
        let wide = Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(630.0, 300.0));
        let small = Drag {
            start: wide,
            edge: Edge::Right,
            minimum,
            delta: egui::vec2(-2000.0, 0.0),
            bounds,
        };
        let small = small.keep_aspect(small.rect(), 21.0 / 9.0, chrome);
        assert!((small.width() - 220.0).abs() < 0.01 && (body_ratio(small, chrome) - 21.0 / 9.0).abs() < 1e-3);
        // 9:16: the minimum height (120) leads, the width follows it
        let tall = Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(270.0, 510.0));
        let small = Drag {
            start: tall,
            edge: Edge::Bottom,
            minimum,
            delta: egui::vec2(0.0, -2000.0),
            bounds,
        };
        let small = small.keep_aspect(small.rect(), 9.0 / 16.0, chrome);
        assert!(small.width() >= 219.99 && (body_ratio(small, chrome) - 9.0 / 16.0).abs() < 1e-3);
        // widened past the screen's height: it stops where the bottom meets it
        let grow = Drag {
            start: Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(480.0, 300.0)),
            edge: Edge::Right,
            minimum,
            delta: egui::vec2(2000.0, 0.0),
            bounds,
        };
        let big = grow.keep_aspect(grow.rect(), 16.0 / 9.0, chrome);
        assert!((big.bottom() - 720.0).abs() < 0.01 && (body_ratio(big, chrome) - 16.0 / 9.0).abs() < 1e-3);
    }

    /// The video player window (#941): dragged by a side or a corner over
    /// several frames, its picture keeps its shape at every frame, not only
    /// once the pointer lets go.
    #[test]
    fn a_floater_of_fixed_shape_keeps_it_at_every_frame_of_a_drag() {
        let ratio = 16.0 / 9.0;
        for edge in [Edge::Right, Edge::Bottom, Edge::RightBottom, Edge::LeftTop, Edge::Left, Edge::Top] {
            let ctx = Context::default();
            let mut frame = 0;
            let mut chrome = egui::vec2(0.0, 30.0);
            let mut show = |events: Vec<egui::Event>, chrome: &mut Vec2| {
                frame += 1;
                let palette = crate::theme::Theme::default().palette();
                let mut body = None;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
                        time: Some(frame as f64 / 60.0),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        body = Floater::new(
                            "aspect_fixture",
                            aurora_i18n::tr!("ui-chat-preview-player"),
                            egui::pos2(500.0, 300.0),
                            egui::vec2(480.0, 300.0),
                        )
                        .silent()
                        .flush_body()
                        .keep_aspect(ratio, *chrome)
                        .show(ui.ctx(), &palette, &mut true, |ui| {
                            let size = ui.available_size();
                            ui.allocate_space(size);
                            size
                        });
                    },
                );
                output.textures_delta.clear();
                let outer = ctx.memory(|memory| memory.area_rect(Id::unique("aspect_fixture")).unwrap());
                if let Some(body) = body {
                    *chrome = outer.size() - body;
                }
                outer
            };
            let mut rect = show(vec![], &mut chrome);
            for _ in 0..8 {
                rect = show(vec![], &mut chrome);
            }
            let grip = edge.hit_rect(rect).center();
            show(vec![egui::Event::PointerMoved(grip)], &mut chrome);
            show(
                vec![egui::Event::PointerButton {
                    pos: grip,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }],
                &mut chrome,
            );
            let outward = egui::vec2(if edge.left() { -1.0 } else { 1.0 }, if edge.top() { -1.0 } else { 1.0 });
            let mut sizes = Vec::new();
            for step in [12.0, 37.0, 80.0, 141.0, 60.0, -40.0] {
                let pointer = grip + outward * egui::vec2(step, step * 0.4);
                let rect = show(vec![egui::Event::PointerMoved(pointer)], &mut chrome);
                assert!(
                    (body_ratio(rect, chrome) - ratio).abs() * (rect.height() - chrome.y) <= 1.0,
                    "edge {} step {step}: {rect:?} (chrome {chrome:?}) is stretched",
                    edge as u8
                );
                sizes.push(rect.size());
            }
            assert!(
                sizes.windows(2).any(|pair| pair[0] != pair[1]),
                "edge {}: the drag resized it",
                edge as u8
            );
        }
    }

    #[test]
    fn expansion_stays_within_the_viewport_and_keeps_the_opposite_edges_fixed() {
        let start = Rect::from_min_size(egui::pos2(300.0, 200.0), egui::vec2(440.0, 300.0));
        let bounds = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0));
        for edge in Edge::ALL {
            let delta = egui::vec2(
                if edge.left() { -2000.0 } else { 2000.0 },
                if edge.top() { -2000.0 } else { 2000.0 },
            );
            let rect = Drag {
                start,
                edge,
                minimum: egui::vec2(220.0, 120.0),
                delta,
                bounds,
            }
            .rect();
            assert!(bounds.contains_rect(rect));
            assert_eq!(
                if edge.left() { rect.right() } else { rect.left() },
                if edge.left() { start.right() } else { start.left() }
            );
            assert_eq!(
                if edge.top() { rect.bottom() } else { rect.top() },
                if edge.top() { start.bottom() } else { start.top() }
            );
        }
    }
}

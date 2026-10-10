//! Floater edges follow LLResizeBar::handleHover (indra/llui/llresizebar.cpp):
//! clamp the size before moving the dragged edge, keeping its opposite fixed.
//! egui 0.36's window edges move the area before applying the size constraint.
use egui::{Context, CursorIcon, Id, Rect, Sense, Ui, UiBuilder, Vec2};

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

    fn hit_rect(self, rect: Rect, side: f32, corner: f32) -> Rect {
        let corner_rect = |pos| Rect::from_center_size(pos, Vec2::splat(2.0 * corner));
        match self {
            Self::Left => Rect::from_min_max(rect.left_top(), rect.left_bottom()).expand2(egui::vec2(side, -corner)),
            Self::Right => Rect::from_min_max(rect.right_top(), rect.right_bottom()).expand2(egui::vec2(side, -corner)),
            Self::Top => Rect::from_min_max(rect.left_top(), rect.right_top()).expand2(egui::vec2(-corner, side)),
            Self::Bottom => Rect::from_min_max(rect.left_bottom(), rect.right_bottom()).expand2(egui::vec2(-corner, side)),
            Self::LeftTop => corner_rect(rect.left_top()),
            Self::LeftBottom => corner_rect(rect.left_bottom()),
            Self::RightTop => corner_rect(rect.right_top()),
            Self::RightBottom => corner_rect(rect.right_bottom()),
        }
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

    pub(super) fn finish(mut self, ctx: &Context, window: Id, actual: Rect) {
        let requested = self.rect();
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
            }
        });
    }
}

pub(super) fn handles(ctx: &Context, response: &egui::Response) {
    let style = ctx.global_style();
    let side = style.interaction.resize_grab_radius_side;
    let corner = style.interaction.resize_grab_radius_corner;
    let rect = response.rect.shrink(0.5);
    let mut ui = Ui::new(
        ctx.clone(),
        response.id.with("floater_resize_ui"),
        UiBuilder::new().layer_id(response.layer_id).max_rect(response.rect),
    );
    ui.set_clip_rect(ctx.content_rect());
    ui.set_min_size(response.rect.size());
    for edge in Edge::ALL {
        let response = ui
            .interact_opt(
                edge.hit_rect(rect, side, corner),
                edge.id(response.layer_id.id),
                Sense::DRAG,
                egui::InteractOptions { move_to_top: true },
            )
            .on_hover_cursor(edge.cursor());
        response.widget_info(|| egui::WidgetInfo::new(egui::WidgetType::ResizeHandle));
        if response.dragged() {
            ctx.set_cursor_icon(edge.cursor());
        }
    }
    // Native handles are disabled; retain the usual corner cue.
    let stroke = style.visuals.widgets.noninteractive.fg_stroke;
    for fraction in [0.25, 0.5, 0.75] {
        let offset = style.visuals.resize_corner_size * fraction;
        ui.painter()
            .line_segment([rect.max - egui::vec2(offset, 0.0), rect.max - egui::vec2(0.0, offset)], stroke);
    }
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
                    Floater::new("resize_fixture", "Inventaire", egui::pos2(300.0, 200.0), egui::vec2(440.0, 300.0))
                        .silent()
                        .show(ui.ctx(), &palette, &mut true, |ui| {
                            ui.set_min_size(egui::vec2(self.content_width, 40.0));
                            ui.set_min_height(ui.available_height());
                        });
                },
            );
            output.textures_delta.clear();
            self.ctx.memory(|memory| memory.area_rect(Id::new("resize_fixture")).unwrap())
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
                let pos = edge.hit_rect(original.shrink(0.5), 6.0, 10.0).center();
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
                let pos = edge.hit_rect(limit.shrink(0.5), 6.0, 10.0).center();
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
                    Floater::new("button_fixture", "Inventaire", egui::pos2(300.0, 200.0), egui::vec2(440.0, 300.0))
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

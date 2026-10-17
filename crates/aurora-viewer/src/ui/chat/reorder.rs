//! Open-conversation order, shared by the workspace and the older tab styles.
//! Follows LLTabContainer's locked-tab rearrangement (lltabcontainer.cpp).
//! Aurora additionally keeps each conversation in its original category.

use super::sidebar::{Kind, kind};
use crate::{theme::Palette, world::World};
use egui::{Pos2, Rect, Response};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

struct Row {
    id: Uuid,
    kind: Kind,
    rect: Rect,
}

#[derive(Default)]
pub(super) struct State {
    order: Vec<Uuid>,
    source: Option<Uuid>,
    rows: Vec<Row>,
}

impl State {
    pub fn begin(&mut self, ctx: &egui::Context, world: &World) {
        let mut remaining: HashSet<_> = world.social.ims.iter().map(|s| s.other).collect();
        self.order.retain(|id| remaining.remove(id));
        self.order
            .extend(world.social.ims.iter().map(|s| s.other).filter(|id| remaining.remove(id)));
        self.rows.clear();
        if self.source.is_some()
            && (ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                || ctx.input(|i| !i.focused || (!i.pointer.primary_down() && !i.pointer.primary_released())))
        {
            self.source = None;
        }
    }

    pub fn sessions(&self, world: &World) -> Vec<(Uuid, usize)> {
        let unread: HashMap<_, _> = world.social.ims.iter().map(|s| (s.other, s.unread)).collect();
        self.order.iter().filter_map(|id| unread.get(id).map(|n| (*id, *n))).collect()
    }

    pub fn row(&mut self, world: &World, id: Uuid, response: &Response) {
        if response.drag_started_by(egui::PointerButton::Primary) {
            self.source = Some(id);
        }
        self.rows.push(Row {
            id,
            kind: kind(world, &id),
            rect: response.rect,
        });
    }

    fn destination(&self, at: Pos2, clip: Rect) -> Option<(Uuid, bool)> {
        let source = self.rows.iter().find(|row| Some(row.id) == self.source)?;
        if !clip.contains(at) {
            return None;
        }
        // Include only the tiny gap between rows, never a section heading,
        // a pinned tab, the scrollbar or a row outside the viewport.
        let target = self
            .rows
            .iter()
            .find(|row| row.rect.intersects(clip) && row.rect.expand2(egui::vec2(0.0, 1.0)).contains(at))?;
        (target.id != source.id && target.kind == source.kind).then_some((target.id, at.y > target.rect.center().y))
    }

    pub fn finish(&mut self, ui: &mut egui::Ui, p: &Palette, demo: bool) {
        if demo
            && self.source.is_none()
            && let Some(first) = self.rows.first()
            && let Some(second) = self.rows.iter().skip(1).find(|row| row.kind == first.kind)
        {
            ui.ctx().data_mut(|d| d.insert_temp(demo_id(), [first.rect, second.rect]));
        }
        let Some(source) = self.source else { return };
        let Some(row) = self.rows.iter().find(|row| row.id == source) else {
            self.source = None;
            return;
        };
        let clip = ui.clip_rect();
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let destination = pointer.and_then(|at| self.destination(at, clip));
        ui.painter()
            .rect_filled(row.rect, ui.visuals().window_corner_radius, p.violet.gamma_multiply(0.12));
        if let Some((target, after)) = destination
            && let Some(row) = self.rows.iter().find(|row| row.id == target)
        {
            let y = if after { row.rect.bottom() } else { row.rect.top() };
            // Insertion feedback, not a separator between conversations.
            ui.painter().line_segment(
                [egui::pos2(row.rect.left() + 3.0, y), egui::pos2(row.rect.right() - 3.0, y)],
                egui::Stroke::new(2.0, p.violet_light),
            );
        }
        if ui.input(|i| i.pointer.primary_released()) {
            if let Some((target, after)) = destination {
                self.move_to(source, target, after);
            }
            self.source = None;
            return;
        }
        let valid = destination.is_some() || pointer.is_some_and(|at| row.rect.contains(at));
        ui.ctx().set_cursor_icon(if valid {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::NotAllowed
        });
        if let Some(at) = pointer.filter(|at| clip.contains(*at)) {
            let edge = 24.0;
            let speed = if at.y < clip.top() + edge {
                (clip.top() + edge - at.y) / edge
            } else if at.y > clip.bottom() - edge {
                -(at.y - clip.bottom() + edge) / edge
            } else {
                0.0
            };
            let dt = ui.input(|i| i.stable_dt.min(0.1));
            ui.scroll_with_delta(egui::vec2(0.0, speed * 420.0 * dt));
        }
        ui.ctx().request_repaint();
    }

    fn move_to(&mut self, source: Uuid, target: Uuid, after: bool) {
        let Some(from) = self.order.iter().position(|id| *id == source) else {
            return;
        };
        self.order.remove(from);
        if let Some(to) = self.order.iter().position(|id| *id == target) {
            self.order.insert(to + usize::from(after), source);
        } else {
            self.order.insert(from, source);
        }
    }
}

fn demo_id() -> egui::Id {
    egui::Id::new("conversation_reorder_demo")
}

/// Synthetic pointer events, enabled only by the explicit offline fixture.
pub(super) fn demo_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    if !(700..=760).contains(&frame) {
        return Vec::new();
    }
    let Some([first, second]) = ctx.data(|d| d.get_temp::<[Rect; 2]>(demo_id())) else {
        return Vec::new();
    };
    let start = egui::pos2(first.left() + first.width() * 0.4, first.center().y);
    let end = egui::pos2(second.left() + second.width() * 0.4, second.bottom() - 6.0);
    let pos = start.lerp(end, (frame.saturating_sub(701) as f32 / 19.0).min(1.0));
    let mut events = vec![egui::Event::PointerMoved(pos)];
    if matches!(frame, 701 | 760) {
        events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: frame == 701,
            modifiers: egui::Modifiers::NONE,
        });
    }
    events
}

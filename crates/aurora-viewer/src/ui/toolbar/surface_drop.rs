//! Stable button-face destinations, before toolbar gap/reorder destinations.
//! Human-requested extension to LLToolBar::handleDragAndDrop (indra/llui/lltoolbar.cpp).

use super::{Bar, Drag, Edit, Style, ToolbarSettings, ToolbarUi, appearance, drawers, drop_preview, structure};
use crate::{
    theme::Palette,
    ui::{
        skin::{Layout, ToolButton},
        widgets,
    },
};
use egui::{Id, LayerId, Pos2, Rect};
use std::{sync::Arc, time::Duration};

const GROUP_DELAY: f64 = 0.35;

#[derive(Clone)]
struct Hover {
    command: String,
    source: String,
    origin: Pos2,
    since: f64,
    armed: bool,
}

fn hover_id() -> Id {
    Id::unique("toolbar_surface_group_hover")
}

#[derive(Clone)]
struct Target {
    command: String,
    bar: Bar,
    mode: super::ButtonMode,
    rect: Rect,
    hit: Rect,
    layer: LayerId,
}

#[derive(Clone, Copy)]
pub(super) struct Insertion {
    pub bar: Bar,
    pub index: usize,
    pub after: bool,
}

fn insertion_id() -> Id {
    Id::unique("toolbar_surface_insertion")
}

pub(super) fn insertion(ctx: &egui::Context) -> Option<Insertion> {
    ctx.data(|data| data.get_temp::<Option<Insertion>>(insertion_id())).flatten()
}

fn targets_id() -> Id {
    Id::unique("toolbar_surface_targets")
}
fn idle_id() -> Id {
    Id::unique("toolbar_surface_idle_targets")
}
fn preferred_id() -> Id {
    Id::unique("toolbar_surface_preferred")
}
fn drag_id() -> Id {
    Id::unique("toolbar_surface_drag")
}

/// Keep broad insertion corridors on both ends, even for label-only controls.
/// Existing drawers retain their entire face as an immediate destination.
fn group_hit(rect: Rect, bar: Bar) -> Rect {
    let mut hit = rect.shrink(3.0);
    if bar.vertical() {
        hit.min.y = hit.min.y.max(rect.center().y - rect.height() * 0.2);
        hit.max.y = hit.max.y.min(rect.center().y + rect.height() * 0.2);
    } else {
        hit.min.x = hit.min.x.max(rect.center().x - rect.width() * 0.2);
        hit.max.x = hit.max.x.min(rect.center().x + rect.width() * 0.2);
    }
    hit
}

fn over(ctx: &egui::Context, target: &Target, drag: &Drag, centre: bool) -> bool {
    if structure::is_separator(&target.command) || structure::is_divider(&target.command) {
        return false;
    }
    if !drawers::is_drawer(&target.command)
        && (target.command == drag.button.command || drag.members.iter().any(|button| button.command == target.command))
    {
        return false;
    }
    let hit = if !centre {
        target.hit.expand2(if target.bar.vertical() {
            egui::vec2(0.0, 4.0)
        } else {
            egui::vec2(4.0, 0.0)
        })
    } else if drawers::is_drawer(&target.command) {
        target.hit
    } else {
        target.hit.intersect(group_hit(target.rect, target.bar))
    };
    hit.is_positive()
        && ctx.pointer_interact_pos().is_some_and(|pos| {
            ctx.content_rect().contains(pos)
                && hit.contains(pos)
                && ctx
                    .layer_id_at(pos)
                    .is_none_or(|layer| layer == target.layer || layer.order == egui::Order::Background)
        })
}

fn target_at<'a>(ctx: &egui::Context, drag: &Drag, idle: &'a [Target], current: &'a [Target], centre: bool) -> Option<&'a Target> {
    idle.iter().find(|target| over(ctx, target, drag, centre)).or_else(|| {
        // Reordering moves the rendered neighbours under the pointer. Their
        // provisional centres must not steal the original insertion corridor.
        current.iter().find(|target| {
            over(ctx, target, drag, centre)
                && (drawers::is_drawer(&target.command) || !idle.iter().any(|old| old.command == target.command))
        })
    })
}

fn ready(ctx: &egui::Context, target: Option<&Target>, drag: Option<&Drag>) -> bool {
    let (Some(target), Some(drag), Some(pos)) = (target, drag, ctx.pointer_interact_pos()) else {
        ctx.data_mut(|data| data.remove::<Option<Hover>>(hover_id()));
        return false;
    };
    if drawers::is_drawer(&target.command) {
        ctx.data_mut(|data| data.remove::<Option<Hover>>(hover_id()));
        return true;
    }
    let (now, released) = ctx.input(|input| (input.time, input.pointer.button_released(egui::PointerButton::Primary)));
    let (armed, remaining) = ctx.data_mut(|data| {
        let hover = data.get_temp_mut_or_default::<Option<Hover>>(hover_id());
        if hover
            .as_ref()
            .is_none_or(|hover| hover.command != target.command || hover.source != drag.button.command || hover.origin.distance(pos) > 6.0)
        {
            *hover = Some(Hover {
                command: target.command.clone(),
                source: drag.button.command.clone(),
                origin: pos,
                since: now,
                armed: false,
            });
        }
        let hover = hover.as_mut().expect("hover initialized above");
        // A release may only commit the grouping preview already seen, never
        // arm a new operation at the instant the pointer is let go.
        hover.armed |= !released && now - hover.since >= GROUP_DELAY;
        (hover.armed, (GROUP_DELAY - (now - hover.since)).max(0.0))
    });
    if !armed {
        ctx.request_repaint_after(Duration::from_secs_f64(remaining));
    }
    armed
}

pub(super) fn prepare(ctx: &egui::Context) {
    let drag = egui::DragAndDrop::payload::<Drag>(ctx);
    let previous = ctx.data(|data| data.get_temp::<Vec<Target>>(targets_id())).unwrap_or_default();
    let preferred = if let Some(drag) = &drag {
        let idle = ctx.data(|data| data.get_temp::<Vec<Target>>(idle_id())).unwrap_or_default();
        let target = target_at(ctx, drag, &idle, &previous, true);
        if ready(ctx, target, Some(drag)) {
            target.map(|target| target.command.clone())
        } else {
            None
        }
    } else {
        ready(ctx, None, None);
        if ToolbarUi::layout_configuration(ctx).is_none() {
            ctx.data_mut(|data| data.insert_temp(idle_id(), previous));
        }
        None
    };
    if preferred.is_some() {
        drop_preview::clear(ctx);
    }
    ctx.data_mut(|data| {
        data.insert_temp(targets_id(), Vec::<Target>::new());
        data.insert_temp(preferred_id(), preferred);
        data.insert_temp(drag_id(), drag);
        data.insert_temp(insertion_id(), None::<Insertion>);
    });
}

pub(super) fn collect(response: &egui::Response, bar: Bar, style: Style, button: &ToolButton) {
    if !response.interact_rect.is_positive()
        || structure::is_separator(&button.command)
        || structure::is_divider(&button.command)
        || appearance::settings(&response.ctx).drawer_of(&button.command).is_some()
    {
        return;
    }
    response.ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Vec<Target>>(targets_id()).push(Target {
            command: button.command.clone(),
            bar,
            mode: style.mode,
            rect: response.rect,
            hit: response.interact_rect,
            layer: response.layer_id,
        })
    });
}

pub(super) fn claimed(ctx: &egui::Context, command: &str) -> bool {
    ctx.data(|data| data.get_temp::<Option<String>>(preferred_id()))
        .flatten()
        .as_deref()
        == Some(command)
}

pub(super) fn show(ctx: &egui::Context, p: &Palette, toolbar: &mut ToolbarUi, settings: &ToolbarSettings, layout: &Layout) {
    if toolbar.pending.is_some()
        || ctx.input(|input| input.key_pressed(egui::Key::Escape))
        || ctx
            .data(|data| data.get_temp::<bool>(Id::unique("toolbar_drop_handled")))
            .unwrap_or(false)
    {
        return;
    }
    // Keep our typed cargo on the release frame even if an unrelated egui drop
    // target consumed it before checking its type (DragAndDrop::take_payload).
    let drag = egui::DragAndDrop::payload::<Drag>(ctx).or_else(|| {
        ctx.input(|input| input.pointer.button_released(egui::PointerButton::Primary))
            .then(|| ctx.data(|data| data.get_temp::<Option<Arc<Drag>>>(drag_id())).flatten())
            .flatten()
    });
    let Some(drag) = drag else { return };
    let idle = ctx.data(|data| data.get_temp::<Vec<Target>>(idle_id())).unwrap_or_default();
    let current = ctx.data(|data| data.get_temp::<Vec<Target>>(targets_id())).unwrap_or_default();
    if drag.members.is_empty()
        && let Some(target) = target_at(ctx, &drag, &idle, &current, false).filter(|target| !drawers::is_drawer(&target.command))
        && let Some(index) = settings
            .bar(target.bar, layout)
            .iter()
            .position(|button| button.command == target.command)
        && let Some(pos) = ctx.pointer_interact_pos()
    {
        let after = if target.bar.vertical() {
            pos.y >= target.rect.center().y
        } else {
            pos.x >= target.rect.center().x
        };
        ctx.data_mut(|data| {
            data.insert_temp(
                insertion_id(),
                Some(Insertion {
                    bar: target.bar,
                    index,
                    after,
                }),
            )
        });
    }
    let target = target_at(ctx, &drag, &idle, &current, true);
    if !ready(ctx, target, Some(&drag)) {
        return;
    }
    let Some(target) = target else { return };
    let Some(button) = settings
        .bar(target.bar, layout)
        .iter()
        .find(|button| button.command == target.command)
    else {
        return;
    };
    let Some(shown) = current.iter().find(|item| item.command == target.command) else {
        return;
    };
    let mode = settings
        .appearance
        .get(&target.command)
        .and_then(|value| value.mode)
        .unwrap_or(target.mode);
    let sources = if drag.members.is_empty() {
        vec![drag.button.clone()]
    } else {
        drag.members.clone()
    };
    let edit = if settings.locked || sources.iter().any(|button| drawers::is_drawer(&button.command)) {
        None
    } else if let Some(drawer) = settings.drawers.get(&target.command) {
        Some(if drag.members.is_empty() {
            Edit::DrawerInsert(target.command.clone(), drag.button.clone(), drawer.buttons.len())
        } else {
            Edit::DrawerBlock(target.command.clone(), drag.members.clone(), drawer.buttons.len())
        })
    } else {
        Some(Edit::DrawerFromDrop(target.bar, button.command.clone(), sources, mode))
    };
    ctx.data_mut(|data| {
        data.insert_temp(preferred_id(), Some(target.command.clone()));
        data.insert_temp(Id::unique("toolbar_drop_handled"), true);
    });
    drop_preview::clear(ctx);
    let mut painter = ctx.layer_painter(shown.layer);
    painter.set_clip_rect(shown.hit);
    painter.rect_stroke(
        shown.rect,
        ctx.global_style().visuals.widgets.inactive.corner_radius,
        egui::Stroke::new(2.0, if edit.is_some() { p.violet_light } else { p.muted_dim }),
        egui::StrokeKind::Inside,
    );
    if edit.is_some() && !drawers::is_drawer(&target.command) {
        let badge = Rect::from_center_size(shown.rect.right_bottom() - egui::vec2(9.0, 9.0), egui::Vec2::splat(16.0));
        painter.add(widgets::overlay_frame(ctx, p).paint(badge));
        widgets::paint_glyph(&painter, "folder-filled", badge.center(), 12.0, p.violet_light);
    }
    if edit.is_none() {
        ctx.set_cursor_icon(egui::CursorIcon::NotAllowed);
    }
    if ctx.input(|input| input.pointer.button_released(egui::PointerButton::Primary)) {
        toolbar.pending = edit;
        egui::DragAndDrop::clear_payload(ctx);
    }
}

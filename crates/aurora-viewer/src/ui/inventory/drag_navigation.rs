//! Drag navigation follows LLScrollContainer::autoScroll and LLFolderView's
//! one-second hover (Firestorm indra/llui, LGPL 2.1). The human also requested
//! closing an open folder on hover; latch each visit to prevent oscillation.
use super::*;

fn active(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.pointer.primary_down())
        && (egui::DragAndDrop::has_payload_of_type::<InvDrag>(ctx)
            || egui::DragAndDrop::has_payload_of_type::<FolderDrag>(ctx)
            || egui::DragAndDrop::has_payload_of_type::<SelectionDrag>(ctx)
            || egui::DragAndDrop::has_payload_of_type::<super::super::task_drop::TaskDrag>(ctx))
}

#[derive(Clone)]
struct Hover {
    folder: egui::Id,
    pass: u64,
    since: f64,
    toggled: bool,
}

/// Navigation does not move inventory, so it also works over the source
/// folder or a destination which refuses the actual drop.
pub(crate) fn folder(ui: &egui::Ui, r: &egui::Response, editor: Uuid, id: Uuid, open: &mut bool) {
    if !active(ui.ctx()) || !ui.rect_contains_pointer(r.rect) {
        return;
    }
    let key = egui::Id::unique(("inventory_drag_folder_visit", editor, id));
    let pass = ui.ctx().cumulative_pass_nr();
    let now = ui.input(|i| i.time);
    let toggle = ui.ctx().data_mut(|data| {
        let state = data.get_temp_mut_or_insert_with(egui::Id::unique("inventory_drag_folder_hover"), || Hover {
            folder: key,
            pass,
            since: now,
            toggled: false,
        });
        if state.folder != key || state.pass.saturating_add(1) < pass {
            *state = Hover {
                folder: key,
                pass,
                since: now,
                toggled: false,
            };
        }
        state.pass = pass;
        let toggle = !state.toggled && now - state.since >= 1.0;
        state.toggled |= toggle;
        toggle
    });
    if toggle {
        *open = !*open;
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
}

#[derive(Clone)]
struct Owner {
    scroll: egui::Id,
    frame: u64,
}
#[derive(Clone)]
struct Motion {
    direction: f32,
    since: f64,
    frame: u64,
}

/// Scroll the actual container, including virtualized search results. Retain
/// the last entered inventory when the pointer leaves vertically into the
/// header/footer/world, but never scroll a window covered by another window.
pub(crate) fn scroll<R>(ui: &egui::Ui, out: &egui::scroll_area::ScrollAreaOutput<R>, editor: Uuid) {
    let ctx = ui.ctx();
    let owner_id = egui::Id::unique("inventory_drag_scroll_owner");
    let motion_id = out.id.with("inventory_drag_scroll_motion");
    let rect = out.inner_rect.intersect(ui.clip_rect());
    if cfg!(test) || demo::enabled() {
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("inventory_drag_viewport", editor)), (rect, out.id)));
    }
    if !active(ctx) {
        ctx.data_mut(|d| {
            d.remove::<Owner>(owner_id);
            d.remove::<Motion>(motion_id);
        });
        return;
    }
    let frame = ctx.cumulative_frame_nr();
    let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) else {
        return;
    };
    let layer = ctx.layer_id_at(pos);
    let over_window = layer == Some(ui.layer_id());
    let entered = over_window && rect.contains(pos);
    let owns = ctx.data_mut(|d| {
        if entered {
            d.insert_temp(owner_id, Owner { scroll: out.id, frame });
        }
        let owner = d.get_temp::<Owner>(owner_id);
        let owns = owner.is_some_and(|o| o.scroll == out.id && o.frame.saturating_add(1) >= frame);
        if owns {
            d.insert_temp(owner_id, Owner { scroll: out.id, frame });
        }
        owns
    });
    let unobstructed = over_window || layer.is_none_or(|l| l.order == egui::Order::Background);
    let direction = if owns && unobstructed && rect.x_range().contains(pos.x) {
        edge(rect, pos.y)
    } else {
        0.0
    };
    if direction == 0.0 {
        ctx.data_mut(|d| d.remove::<Motion>(motion_id));
        return;
    }
    let (now, dt) = ctx.input(|i| (i.time, i.unstable_dt.clamp(0.0, 0.05)));
    let since = ctx.data_mut(|d| {
        let motion = d.get_temp_mut_or_insert_with(motion_id, || Motion {
            direction,
            since: now,
            frame: u64::MAX,
        });
        if motion.frame == frame {
            return None;
        }
        if motion.direction != direction || motion.frame.saturating_add(1) < frame {
            motion.since = now;
            motion.direction = direction;
        }
        motion.frame = frame;
        Some(motion.since)
    });
    let Some(since) = since else { return };
    // Firestorm's 16-point edge zone and 100..1000 points/s, with acceleration
    // based on elapsed time rather than frame count. Keep horizontal scrolling.
    let speed = (100.0 + (now - since) as f32 * 300.0).min(1000.0);
    let max = (out.content_size.y - out.inner_rect.height()).max(0.0);
    let offset = out.state.unclamped_offset();
    let next = (offset.y + direction * speed * dt).clamp(0.0, max);
    if (next - offset.y).abs() < 0.01 {
        ctx.data_mut(|d| d.remove::<Motion>(motion_id));
        return;
    }
    crate::ui::widgets::scroll_offset::set(ctx, out.id, egui::vec2(offset.x, next));
    ctx.request_repaint();
}

fn edge(rect: egui::Rect, y: f32) -> f32 {
    let zone = 16.0_f32.min(rect.height() / 3.0);
    if y < rect.top() + zone {
        -1.0
    } else if y > rect.bottom() - zone {
        1.0
    } else {
        0.0
    }
}

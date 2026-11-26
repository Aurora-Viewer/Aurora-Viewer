//! Native chrome keeps its functional context actions and shares the toolbar's
//! final customization entry. The scope follows the widget into nested menus;
//! ordinary world/window context menus remain outside it.

use super::menu;
use crate::theme::Palette;
use egui::{Context, Id, Ui};

fn scope_id() -> Id {
    Id::unique("toolbar_native_context_scope")
}
fn request_id() -> Id {
    Id::unique("toolbar_native_context_request")
}

pub(crate) fn target(ctx: &Context) -> Option<String> {
    ctx.data(|data| data.get_temp::<String>(scope_id()))
}

pub(crate) struct Scope {
    ctx: Context,
    previous: Option<String>,
}

impl Scope {
    pub fn new(ctx: &Context, command: &str) -> Self {
        let previous = ctx.data_mut(|data| {
            let previous = data.get_temp::<String>(scope_id());
            data.insert_temp(scope_id(), command.to_owned());
            previous
        });
        Self {
            ctx: ctx.clone(),
            previous,
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        self.ctx.data_mut(|data| {
            if let Some(previous) = self.previous.take() {
                data.insert_temp(scope_id(), previous);
            } else {
                data.remove_temp::<String>(scope_id());
            }
        });
    }
}

#[derive(Clone, Default)]
pub(crate) struct Request {
    pub command: Option<String>,
}

pub(crate) fn take(ctx: &Context) -> Option<Request> {
    ctx.data_mut(|data| data.remove_temp::<Request>(request_id()))
}

/// Separate button-specific actions from the shared customization group.
pub(crate) fn start_group(ui: &mut Ui, p: &Palette) {
    if ui.min_rect().height() > 0.0 {
        menu::separator(ui, p);
    }
}

/// The same wording, icon and final position, adjacent to button appearance.
pub(crate) fn item(ui: &mut Ui, p: &Palette) -> bool {
    let frame = ui.ctx().cumulative_frame_nr_for(egui::ViewportId::ROOT);
    ui.ctx()
        .data_mut(|data| data.insert_temp(ui.layer_id().id.with("toolbar_context_footer"), frame));
    menu::item(ui, p, super::drawers::CUSTOMIZE_ICON, aurora_i18n::tr!("toolbar-interface-edit"))
}

#[derive(Clone)]
struct OpenContext {
    id: Id,
    anchor: egui::Pos2,
    opened: u64,
    checked: u64,
    moved: bool,
}

fn open_context_id() -> Id {
    Id::unique("toolbar_open_context")
}

#[cfg(test)]
pub(super) fn tracked_popup(ctx: &Context) -> Option<Id> {
    ctx.data(|data| data.get_temp::<OpenContext>(open_context_id()).map(|open| open.id))
}

/// Record only context popups that actually rendered the toolbar footer.
pub(crate) fn context_shown(ctx: &Context, id: Id, was_open: bool, requested: bool) {
    let frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    let owned = ctx.data(|data| data.get_temp::<u64>(id.with("toolbar_context_footer"))) == Some(frame);
    if owned
        && (!was_open || requested)
        && egui::Popup::is_id_open(ctx, id)
        && let Some(anchor) = egui::Popup::position_of_id(ctx, id).or_else(|| ctx.pointer_interact_pos())
    {
        ctx.data_mut(|data| {
            data.insert_temp(
                open_context_id(),
                OpenContext {
                    id,
                    anchor,
                    opened: frame,
                    checked: frame,
                    moved: false,
                },
            )
        });
    }
}

/// A second stationary right click closes the owned popup even when its face
/// covers the original button. Suppress reopening for the complete input frame.
pub(crate) fn dismiss_repeated_click(ctx: &Context) -> bool {
    let frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    let dismissed = Id::unique("toolbar_context_dismissed_frame");
    if ctx.data(|data| data.get_temp::<u64>(dismissed)) == Some(frame) {
        return true;
    }
    let Some(mut open) = ctx.data(|data| data.get_temp::<OpenContext>(open_context_id())) else {
        return false;
    };
    if !egui::Popup::is_id_open(ctx, open.id) {
        ctx.data_mut(|data| data.remove::<OpenContext>(open_context_id()));
        return false;
    }
    if open.checked == frame {
        return false;
    }
    // Allow the small pointer jitter of a physical click, but remember a move
    // away and back rather than treating it as an unchanged pointer.
    let (clicked, moved) = ctx.input(|input| {
        let moved = input.pointer.interact_pos().is_none_or(|pos| pos.distance_sq(open.anchor) > 9.0)
            || input.events.iter().any(|event| match event {
                egui::Event::PointerMoved(pos) => pos.distance_sq(open.anchor) > 9.0,
                egui::Event::PointerGone => true,
                _ => false,
            });
        (input.pointer.secondary_clicked(), moved)
    });
    open.moved |= moved;
    open.checked = frame;
    if clicked && !open.moved && frame > open.opened {
        egui::Popup::close_all(ctx);
        ctx.data_mut(|data| {
            data.remove::<OpenContext>(open_context_id());
            data.insert_temp(dismissed, frame);
        });
        true
    } else {
        ctx.data_mut(|data| data.insert_temp(open_context_id(), open));
        false
    }
}

fn request(ui: &mut Ui, p: &Palette, command: Option<String>) {
    start_group(ui, p);
    if item(ui, p) {
        ui.ctx().data_mut(|data| data.insert_temp(request_id(), Request { command }));
    }
}

pub(crate) fn append(ui: &mut Ui, p: &Palette) {
    if let Some(command) = ui.ctx().data(|data| data.get_temp::<String>(scope_id())) {
        let frame = ui.ctx().cumulative_frame_nr_for(egui::ViewportId::ROOT);
        ui.ctx()
            .data_mut(|data| data.insert_temp(Id::unique(("toolbar_native_popup", &command)), frame));
        request(ui, p, Some(command));
    }
}

pub(crate) fn shown(ctx: &Context, command: &str) -> bool {
    ctx.data(|data| data.get_temp::<u64>(Id::unique(("toolbar_native_popup", command))))
        == Some(ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT))
}

/// Older uncomposed bars use the same entry without a persisted native target.
pub(crate) fn fallback(ui: &mut Ui, p: &Palette) {
    if ui.ctx().data(|data| data.get_temp::<String>(scope_id())).is_none() {
        request(ui, p, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_native_scopes_restore_their_target_and_do_not_leak_to_world_menus() {
        let ctx = Context::default();
        {
            let _outer = Scope::new(&ctx, "bar_navigation");
            {
                let _inner = Scope::new(&ctx, "bar_volume");
                assert_eq!(ctx.data(|data| data.get_temp::<String>(scope_id())).as_deref(), Some("bar_volume"));
            }
            assert_eq!(
                ctx.data(|data| data.get_temp::<String>(scope_id())).as_deref(),
                Some("bar_navigation")
            );
        }
        assert!(ctx.data(|data| data.get_temp::<String>(scope_id())).is_none());
        assert!(take(&ctx).is_none(), "scoping and hover never enqueue an edit");
    }
}

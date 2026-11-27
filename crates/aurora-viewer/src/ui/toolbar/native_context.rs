//! Native chrome keeps its functional context actions and shares the toolbar's
//! final customization entry. The scope follows the widget into nested menus;
//! ordinary world/window context menus remain outside it.

use super::menu;
use crate::theme::Palette;
use crate::ui::skin::{Layout, ToolButton};
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

/// A menu of its own drawn inside a native control (an emoji of the chat
/// block's picker or of its recent row): the control's scope is set aside
/// while this lives, so that menu does not end with the button's and the
/// block's settings (`append`).
pub(crate) struct Outside {
    ctx: Context,
    scope: Option<String>,
}

impl Outside {
    pub fn new(ctx: &Context) -> Self {
        Self {
            ctx: ctx.clone(),
            scope: ctx.data_mut(|data| data.remove_temp::<String>(scope_id())),
        }
    }
}

impl Drop for Outside {
    fn drop(&mut self) {
        if let Some(scope) = self.scope.take() {
            self.ctx.data_mut(|data| data.insert_temp(scope_id(), scope));
        }
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

/// A native control's right click ends as a button's (human request,
/// 2026-11-26): its own actions, then its button's, its block's and its
/// parent block's settings, and "Customize interface".
pub(crate) fn append(ui: &mut Ui, p: &Palette) {
    if let Some(command) = ui.ctx().data(|data| data.get_temp::<String>(scope_id())) {
        let frame = ui.ctx().cumulative_frame_nr_for(egui::ViewportId::ROOT);
        ui.ctx()
            .data_mut(|data| data.insert_temp(Id::unique(("toolbar_native_popup", &command)), frame));
        sections(ui, p, &command);
    }
}

/// What a native control's sections chose, which the bars take on their
/// next frame (`ToolbarUi::take_native`): its code draws its menu where the
/// bars' state is not at hand.
#[derive(Default)]
pub(super) struct HandOver {
    pub pending: Option<super::Edit>,
    pub block_editor: Option<(super::Bar, String, super::Style)>,
    pub drawer_editor: Option<String>,
    pub button: Option<(ToolButton, String)>,
    pub customize: Option<Option<String>>,
}

type Shared = std::sync::Arc<std::sync::Mutex<Option<HandOver>>>;

fn hand_over_id() -> Id {
    Id::unique("toolbar_native_hand_over")
}

fn sections(ui: &mut Ui, p: &Palette, command: &str) {
    let ctx = ui.ctx().clone();
    let settings = super::appearance::settings(&ctx);
    let layout = Layout::default();
    let Some(bar) = settings.location(command, &layout) else {
        // a control placed nowhere (an island of its own): "Customize
        // interface" alone
        request(ui, p, Some(command.to_owned()));
        return;
    };
    let islands = ctx.data(|data| data.get_temp::<bool>(super::islands_id())).unwrap_or(false);
    let style = super::blocks::style(&ctx, command, settings.style(bar).resolve(bar, islands));
    let button = settings
        .placed_button(command, &layout)
        .cloned()
        .unwrap_or_else(|| crate::ui::bars::top::button(command, ""));
    let rect = ctx
        .read_response(crate::ui::bars::tool_button_id(command))
        .map_or(egui::Rect::NOTHING, |response| response.rect);
    let mut stand_in = super::ToolbarUi {
        open: super::ToolbarUi::editing(&ctx),
        islands,
        ..Default::default()
    };
    let editing = stand_in.open;
    start_group(ui, p);
    stand_in.context_body(ui, p, bar, style, Some(&button), rect, settings.locked);
    let chosen = HandOver {
        pending: stand_in.pending.take(),
        block_editor: stand_in.block_editor.take(),
        drawer_editor: stand_in.drawer_editor.take(),
        button: stand_in.appearance_editor.selected(),
        customize: (stand_in.open && !editing).then(|| stand_in.selected.take()),
    };
    if chosen.pending.is_some()
        || chosen.block_editor.is_some()
        || chosen.drawer_editor.is_some()
        || chosen.button.is_some()
        || chosen.customize.is_some()
    {
        let shared: Shared = ctx.data_mut(|data| data.get_temp_mut_or_default::<Shared>(hand_over_id()).clone());
        if let Ok(mut slot) = shared.lock() {
            *slot = Some(chosen);
        }
    }
}

/// The choice of a native control's sections, once.
pub(super) fn take_hand_over(ctx: &Context) -> Option<HandOver> {
    let shared = ctx.data(|data| data.get_temp::<Shared>(hand_over_id()))?;
    shared.lock().ok()?.take()
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

    #[test]
    fn a_menu_of_its_own_stands_outside_the_native_scope_and_gives_it_back() {
        let ctx = Context::default();
        let block = Scope::new(&ctx, "bar_chat");
        {
            let _alone = Outside::new(&ctx);
            assert!(target(&ctx).is_none(), "no native target for the menu drawn here");
        }
        assert_eq!(target(&ctx).as_deref(), Some("bar_chat"));
        // outside any native control: nothing to set aside, nothing given back
        drop(block);
        drop(Outside::new(&ctx));
        assert!(target(&ctx).is_none());
    }
}

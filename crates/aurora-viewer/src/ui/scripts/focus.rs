//! Script Ctrl+F belongs to the active editor, including its controls and
//! popups. LLFloaterSearchReplace::handleKeyHere routes through its editor
//! and parent chain; the requested window-wide activation outlives field focus.

use super::{Editor, dialogs::Search};
use egui::{Id, LayerId};

#[derive(Clone)]
struct Claim {
    layer: LayerId,
    owner: Id,
    frame: u64,
}

#[derive(Clone, Default)]
struct State {
    claims: Vec<Claim>,
    active: Option<Id>,
    keyboard: Option<Id>,
    frame: u64,
}

fn state_id() -> Id {
    Id::unique("script_editor_activation")
}

/// Register the editor, its find controls, or one of its popup menus.
pub(super) fn claim(ctx: &egui::Context, layer: LayerId, owner: Id) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| {
        let state = d.get_temp_mut_or_default::<State>(state_id());
        state.claims.retain(|c| c.layer != layer && c.frame + 1 >= frame);
        state.claims.push(Claim { layer, owner, frame });
    });
}

fn resolve(ctx: &egui::Context, state: &State) -> Option<Id> {
    if ctx.memory(|m| m.top_modal_layer()).is_some() {
        return None;
    }
    let frame = ctx.cumulative_frame_nr();
    let owner = |layer| {
        state
            .claims
            .iter()
            .find(|c| c.layer == layer && c.frame + 1 >= frame)
            .map(|c| c.owner)
    };
    // Activation follows clicks, not hovering. A click outside our windows
    // releases the shortcut even if an old text field still has the keyboard.
    if let Some(at) = ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten()) {
        return ctx.layer_id_at(at).and_then(owner);
    }
    let keyboard = ctx.memory(|m| m.focused());
    if keyboard != state.keyboard
        && let Some(field) = keyboard
    {
        return ctx.read_response(field).and_then(|r| owner(r.layer_id));
    }
    state.active
}

/// Used by the app's Ctrl+F arbitration before it runs the viewer shortcut.
pub fn editor_focused(ctx: &egui::Context) -> bool {
    let state = ctx.data(|d| d.get_temp::<State>(state_id())).unwrap_or_default();
    state.frame + 1 >= ctx.cumulative_frame_nr() && resolve(ctx, &state).is_some()
}

/// Run once after all editors handled clicks, before drawing the shared find.
pub(super) fn find(ctx: &egui::Context, editors: &mut [Editor], search: &mut Search) {
    let frame = ctx.cumulative_frame_nr();
    let windows: Vec<_> = editors
        .iter()
        .map(|e| {
            let window = format!("script_editor_{}_{}", e.target.object, e.target.item);
            let owner = Id::unique((&window, "source_text"));
            (window, owner)
        })
        .collect();
    let mut state = ctx.data(|d| d.get_temp::<State>(state_id())).unwrap_or_default();
    state
        .claims
        .retain(|c| c.frame + 1 >= frame && windows.iter().any(|(_, id)| *id == c.owner));
    state.active = resolve(ctx, &state).filter(|id| windows.iter().any(|(_, live)| live == id));
    state.keyboard = ctx.memory(|m| m.focused());
    state.frame = frame;
    let active = state.active;
    ctx.data_mut(|d| d.insert_temp(state_id(), state));
    if let Some(owner) = active
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F))
        && let Some(index) = windows.iter().position(|(_, id)| *id == owner)
        && editors[index].loaded
    {
        egui::Popup::close_all(ctx);
        crate::ui::widgets::expand_floater(ctx, &windows[index].0);
        if search.owns(owner) && Search::focused(ctx) {
            search.focus = true;
        } else {
            let e = &editors[index];
            let id = if e.tools.processed { owner.with("processed") } else { owner };
            search.begin(
                ctx,
                owner,
                id,
                if e.tools.processed { &e.tools.processed_source } else { &e.source },
            );
        }
    }
}

//! LLFloaterTexturePicker::onPipetteSelect and LLToolPipette
//! (indra/newview/lltexturectrl.cpp, lltoolpipette.cpp, originally LGPL 2.1).
//! A session belongs to the picker that armed it, regardless of its caller.

use super::{BLANK_MATERIAL, TexturePicker, can_preview, inventory_asset};
use crate::world::inventory::Inventory;
use egui::{Context, Id};
use uuid::Uuid;

#[derive(Clone, Copy, Default)]
struct State {
    owner: Option<(Id, u64, bool)>,
    cursor: Option<(f32, f32)>,
    sample: Option<(Id, Uuid)>,
    highlighted: Option<(Uuid, usize)>,
    failed: bool,
}

fn key() -> Id {
    Id::unique("texture_picker_pipette")
}

fn load(ctx: &Context) -> State {
    ctx.data(|data| data.get_temp(key())).unwrap_or_default()
}

fn store(ctx: &Context, state: State) {
    ctx.data_mut(|data| data.insert_temp(key(), state));
}

pub(crate) fn owner(ctx: &Context) -> Option<Id> {
    let (id, frame, _) = load(ctx).owner?;
    (ctx.cumulative_frame_nr() <= frame + 1).then_some(id)
}

pub(crate) fn armed(ctx: &Context) -> bool {
    owner(ctx).is_some()
}

fn arm(ctx: &Context, id: Id, materials: bool) {
    let mut state = load(ctx);
    if owner(ctx) != Some(id) {
        state = State::default();
        super::super::widgets::color::pipette::release(ctx);
    }
    state.owner = Some((id, ctx.cumulative_frame_nr(), materials));
    store(ctx, state);
}

/// Mouse-up ends the tool but preserves a sample not yet consumed by the UI.
pub(crate) fn release(ctx: &Context) {
    let mut state = load(ctx);
    state.owner = None;
    state.cursor = None;
    state.highlighted = None;
    state.failed = false;
    store(ctx, state);
}

pub(crate) fn needs_sample(ctx: &Context, cursor: (f32, f32)) -> Option<bool> {
    let mut state = load(ctx);
    if !armed(ctx) || state.cursor == Some(cursor) {
        return None;
    }
    state.cursor = Some(cursor);
    store(ctx, state);
    state.owner.map(|(_, _, materials)| materials)
}

pub(crate) fn sample(ctx: &Context, hit: Option<(Uuid, usize, Uuid)>) {
    let Some(id) = owner(ctx) else { return };
    let mut state = load(ctx);
    state.highlighted = hit.map(|(object, face, _)| (object, face));
    // Empty space is a miss, not a request to clear the current texture.
    if let Some((_, _, asset)) = hit {
        state.sample = Some((id, asset));
    }
    store(ctx, state);
}

pub(crate) fn highlighted(ctx: &Context) -> Option<(Uuid, usize)> {
    armed(ctx).then(|| load(ctx).highlighted).flatten()
}

pub(crate) fn failed(ctx: &Context) -> bool {
    armed(ctx) && load(ctx).failed
}

impl TexturePicker {
    pub(super) fn pipette_input(&mut self, ctx: &Context, inv: &Inventory) {
        let Some(id) = self.pipette_id else { return };
        if !self.open || self.source != 0 {
            self.stop_pipette(ctx);
            return;
        }
        let mut state = load(ctx);
        if let Some((recipient, asset)) = state.sample
            && recipient == id
        {
            state.sample = None;
            let lookup = (inv.generation, asset);
            if self.pipette_lookup.map(|(generation, asset, _)| (generation, asset)) != Some(lookup) {
                let item = inv
                    .items
                    .keys()
                    .copied()
                    .filter(|&id| inventory_asset(inv, id, self.materials) == Some(asset) && can_preview(inv, id))
                    .min();
                self.pipette_lookup = Some((inv.generation, asset, item));
            }
            let item = self.pipette_lookup.and_then(|(_, _, item)| item);
            self.pipette_error = item.is_none() && !(self.materials && asset == BLANK_MATERIAL);
            if !self.pipette_error {
                self.select(asset, item, true);
                self.filter.clear();
                self.tree.opened(asset);
            }
            state.failed = self.pipette_error;
            store(ctx, state);
        }
        if owner(ctx) == Some(id) {
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                self.stop_pipette(ctx);
            } else {
                arm(ctx, id, self.materials);
            }
        }
    }

    pub(super) fn stop_pipette(&self, ctx: &Context) {
        if self.pipette_id.is_some_and(|id| owner(ctx) == Some(id)) {
            release(ctx);
        }
    }

    pub(super) fn pipette_button(&mut self, ui: &mut egui::Ui, p: &crate::theme::Palette) {
        if self.source != 0 {
            return;
        }
        let Some(id) = self.pipette_id else { return };
        let active = owner(ui.ctx()) == Some(id);
        use crate::ui::widgets::{ActionKind, action_button};
        if action_button(
            ui,
            p,
            "color-picker",
            aurora_i18n::tr!("color-picker-pipette"),
            if active { ActionKind::Primary } else { ActionKind::Flat },
            egui::vec2(0.0, 24.0),
        )
        .on_hover_text(aurora_i18n::tr!("picker-pipette-hint"))
        .clicked()
        {
            if active {
                self.stop_pipette(ui.ctx());
            } else {
                self.pipette_error = false;
                arm(ui.ctx(), id, self.materials);
            }
        }
    }
}

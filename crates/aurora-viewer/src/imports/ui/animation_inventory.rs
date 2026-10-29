//! Searchable object picker, with one explicit preparation action.
use super::*;
use crate::{
    imports::animation_inventory::{State, available},
    world::inventory::{Inventory, actions as rules},
};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Picker {
    generation: Option<u64>,
    rows: Vec<(Uuid, String, String, String)>,
    filtered: Vec<usize>,
    query: Option<String>,
    poll: Option<Instant>,
    loaded: bool,
    focus: bool,
}

impl Picker {
    fn prepare(&mut self, inv: &mut Inventory, query: &str) {
        if self.generation != Some(inv.generation) {
            self.loaded = false;
            self.rows = inv
                .items
                .values()
                .filter(|item| {
                    rules::original(inv, item.id)
                        .and_then(|id| inv.items.get(&id))
                        .is_some_and(|item| item.asset_type == 6)
                        && !rules::library(inv, item.id)
                })
                .map(|item| {
                    let mut path = Vec::new();
                    let mut parent = item.parent;
                    for _ in 0..32 {
                        let Some(folder) = inv.folders.get(&parent) else { break };
                        path.push(folder.info.name.as_str());
                        parent = folder.info.parent;
                    }
                    path.reverse();
                    let path = path.join(" / ");
                    let search = format!("{}\n{}", item.name, path).to_lowercase();
                    (item.id, item.name.clone(), path, search)
                })
                .collect();
            self.rows.sort_by(|a, b| a.3.cmp(&b.3).then(a.0.cmp(&b.0)));
            self.generation = Some(inv.generation);
            self.query = None;
        }
        if !self.loaded && self.poll.is_none_or(|at| at.elapsed() >= Duration::from_millis(500)) {
            self.loaded = rules::request_tree(inv, inv.root);
            self.poll = Some(Instant::now());
        }
        let query = query.trim().to_lowercase();
        if self.query.as_ref() != Some(&query) {
            self.filtered = self
                .rows
                .iter()
                .enumerate()
                .filter_map(|(index, (_, _, _, search))| search.contains(&query).then_some(index))
                .collect();
            self.query = Some(query);
        }
    }
}

/// Returns the item only after the explicit rez-and-save button. Closing
/// while preparation runs requests cleanup; it never abandons that state.
pub fn show(state: &mut State, ctx: &egui::Context, p: &Palette, inv: &mut Inventory, agent: Uuid, enabled: bool) -> Option<Uuid> {
    if let Some(pending) = &mut state.pending {
        let response = widgets::Modal::new(egui::Id::unique("animation_inventory_preparation")).show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.heading(aurora_i18n::tr!("import-animation-inventory-title"));
            ui.label(&pending.saved.name);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(if pending.cleaning.is_some() || pending.cancelled {
                    aurora_i18n::tr!("import-animation-inventory-cleaning")
                } else if pending.worker.is_some() {
                    aurora_i18n::tr!("import-animation-inventory-saving")
                } else {
                    aurora_i18n::tr!("import-animation-inventory-preparing")
                });
            });
            if !pending.cancelled && flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                pending.cancelled = true;
                state.entry = None;
            }
        });
        if response.should_close() {
            pending.cancelled = true;
            state.entry = None;
        }
        return None;
    }
    state.entry?;
    let mut chosen = None;
    let mut open = true;
    let mut cancel = false;
    Floater::new(
        "animation_inventory_model",
        aurora_i18n::tr!("import-animation-inventory-title"),
        ctx.content_rect().center() - egui::vec2(240.0, 260.0),
        egui::vec2(480.0, 520.0),
    )
    .show(ctx, p, &mut open, |ui| {
        ui.label(aurora_i18n::tr!("import-animation-inventory-help"));
        ui.add_space(8.0);
        {
            let search = widgets::search_field(
                ui,
                &mut state.query,
                aurora_i18n::tr!("import-animation-inventory-search"),
                ui.available_width(),
            );
            if !state.picker.focus {
                search.request_focus();
                state.picker.focus = true;
            }
            state.picker.prepare(inv, &state.query);
            if state
                .selected
                .is_some_and(|id| !state.picker.filtered.iter().any(|i| state.picker.rows[*i].0 == id))
            {
                state.selected = None;
            }
            egui::ScrollArea::vertical()
                .id_salt("animation_inventory_items")
                .max_height(250.0)
                .show_rows(ui, 26.0, state.picker.filtered.len(), |ui, range| {
                    for index in range {
                        let (id, name, path, _) = &state.picker.rows[state.picker.filtered[index]];
                        let allowed = available(inv, *id, agent);
                        let response = ui
                            .add_enabled_ui(allowed.is_ok(), |ui| selection(ui, &mut state.selected, Some(*id), name))
                            .inner;
                        ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("animation_inventory_item", *id)), response.rect));
                        if let Err(reason) = allowed {
                            response.on_disabled_hover_text(aurora_i18n::text(reason));
                        } else {
                            response.on_hover_text(path);
                        }
                    }
                });
            if state.picker.filtered.is_empty() {
                ui.label(RichText::new(aurora_i18n::tr!("import-animation-inventory-empty")).color(p.muted));
            }
            if !state.picker.loaded {
                ui.spinner();
            }
            ui.add_space(8.0);
            let button = ui
                .add_enabled_ui(
                    enabled && state.selected.is_some_and(|id| available(inv, id, agent).is_ok()),
                    |ui| {
                        widgets::primary_button_sized(
                            ui,
                            p,
                            aurora_i18n::tr!("import-animation-inventory-add"),
                            Vec2::new(ui.available_width(), 30.0),
                        )
                    },
                )
                .inner;
            ctx.data_mut(|data| data.insert_temp(egui::Id::unique("animation_inventory_add"), button.rect));
            if button.clicked() {
                chosen = state.selected;
                state.error = None;
            }
        }
        if let Some(error) = &state.error {
            ui.label(RichText::new(error).color(p.danger));
        }
        if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
            cancel = true;
        }
    });
    if !open || cancel {
        state.entry = None;
        if let Some(pending) = &mut state.pending {
            pending.cancelled = true;
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_model_picker_requires_an_explicit_button_after_row_selection() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let agent = crate::demo::DEMO_AGENT;
        let mut inv = Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, agent);
        let id = Uuid::from_u128(8100);
        let mut state = State {
            entry: Some(Uuid::new_v4()),
            query: inv.items[&id].name.clone(),
            ..Default::default()
        };
        let frame = |events, state: &mut State, inv: &mut Inventory| {
            let mut chosen = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 800.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    chosen = show(state, ui.ctx(), &theme.palette(), inv, agent, true);
                },
            );
            output.textures_delta.clear();
            chosen
        };
        for _ in 0..3 {
            assert!(frame(Vec::new(), &mut state, &mut inv).is_none());
        }
        let click = |pos, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        let row = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("animation_inventory_item", id))))
            .expect("object row");
        for pressed in [true, false] {
            assert!(frame(click(row.center(), pressed), &mut state, &mut inv).is_none());
        }
        assert_eq!(state.selected, Some(id));
        assert!(state.pending.is_none(), "selecting a row has no rez side effects");
        let button = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("animation_inventory_add")))
            .expect("explicit rez button");
        let mut chosen = None;
        for pressed in [true, false] {
            chosen = chosen.or(frame(click(button.center(), pressed), &mut state, &mut inv));
        }
        assert_eq!(chosen, Some(id));
    }
}

//! Native RLVa restrictions, locks, modifiers and console inspection.
//! Port of RlvFloaterBehaviours / Locks / Console (rlvfloaters.cpp, LGPL 2.1).
use super::widgets::Floater;
use crate::{
    theme::Palette,
    world::{World, rlva::diagnostics::Outcome},
};
use aurora_i18n::tr;
use egui::{RichText, Vec2};
use uuid::Uuid;

#[derive(Default)]
pub struct Tools {
    pub open: bool,
    pub tab: usize,
    input: String,
    filter: String,
    history: Vec<String>,
    browsing: Option<usize>,
}

#[derive(Default)]
pub struct Actions {
    pub input: Option<String>,
    pub closed: bool,
    pub clear_log: bool,
}

fn source_name(world: &World, source: Uuid) -> String {
    world
        .rlva_attachment_item(source)
        .and_then(|item| world.inventory.items.get(&item).map(|item| item.name.as_str()))
        .map(|name| world.rlva_filter_text(name).into_owned())
        .unwrap_or_else(|| source.to_string())
}

fn status(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Input => ">",
        Outcome::Processed => tr!("rlva-tools-processed"),
        Outcome::Queued => tr!("rlva-tools-queued"),
        Outcome::Duplicate => tr!("rlva-tools-duplicate"),
        Outcome::Unset => tr!("rlva-tools-unset"),
        Outcome::Rejected => tr!("rlva-tools-rejected"),
        Outcome::Reply => tr!("rlva-tools-reply"),
    }
}

impl Tools {
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &World) -> Actions {
        let mut actions = Actions::default();
        if !self.open {
            return actions;
        }
        Floater::new(
            "rlva_tools",
            tr!("rlva-tools-title"),
            ctx.content_rect().center() - Vec2::new(360.0, 250.0),
            Vec2::new(720.0, 500.0),
        )
        .show(ctx, p, &mut self.open, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (index, name) in [
                    tr!("rlva-active"),
                    tr!("rlva-tools-locks"),
                    tr!("rlva-tools-modifiers"),
                    tr!("rlva-tools-console"),
                ]
                .into_iter()
                .enumerate()
                {
                    ui.selectable_value(&mut self.tab, index, name);
                }
            });
            super::widgets::divider(ui, p);
            if !world.rlva.enabled() {
                ui.label(RichText::new(tr!("rlva-tools-disabled")).color(p.muted));
            }
            if self.tab == 3 {
                ui.horizontal(|ui| {
                    ui.label(tr!("rlva-tools-local-replies"));
                    actions.clear_log = ui.button(tr!("rlva-tools-clear-log")).clicked();
                });
                // the input and its button measure what the text size gives them
                let (mut list, footer) = super::widgets::above_footer(ui);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(&mut list, |ui| {
                        for entry in &world.rlva.diagnostics.entries {
                            let color = if entry.outcome == Outcome::Rejected { p.danger } else { p.ink };
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!("{} {}", status(entry.outcome), entry.text))
                                        .monospace()
                                        .color(color),
                                )
                                .selectable(true)
                                .wrap(),
                            )
                            .on_hover_text(source_name(world, entry.source));
                        }
                    });
                footer.show(ui, |ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.input)
                            .font(egui::TextStyle::Monospace)
                            .hint_text(tr!("rlva-tools-input-hint"))
                            .char_limit(4096)
                            .desired_width(f32::INFINITY),
                    );
                    if response.has_focus() {
                        let (up, down) = ui.input_mut(|input| {
                            (
                                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                            )
                        });
                        if (up || down) && !self.history.is_empty() {
                            self.browsing = match (self.browsing, up) {
                                (None, true) => Some(self.history.len() - 1),
                                (Some(index), true) => Some(index.saturating_sub(1)),
                                (Some(index), false) => (index + 1 < self.history.len()).then_some(index + 1),
                                _ => None,
                            };
                            self.input = self.browsing.map(|index| self.history[index].clone()).unwrap_or_default();
                        }
                    }
                    let enter = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    if (enter || ui.button(tr!("rlva-tools-run")).clicked()) && !self.input.trim().is_empty() {
                        let text = std::mem::take(&mut self.input);
                        if self.history.len() >= 64 {
                            self.history.remove(0);
                        }
                        self.history.push(text.clone());
                        self.browsing = None;
                        actions.input = Some(text);
                        response.request_focus();
                    }
                });
            } else {
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text(tr!("rlva-tools-filter"))
                        .desired_width(f32::INFINITY),
                );
                let filter = self.filter.to_lowercase();
                let now = std::time::Instant::now();
                let mut rows: Vec<(Uuid, String)> = if self.tab == 2 {
                    let mut values: Vec<_> = world
                        .rlva
                        .camera_values
                        .iter()
                        .map(|(name, value)| (Uuid::nil(), format!("{name} = {value}")))
                        .collect();
                    for (source, sphere) in &world.rlva.visuals.spheres {
                        let (params, distance) = sphere.sample(now);
                        values.push((
                            *source,
                            format!(
                                "setsphere: mode={}, origin={}, extend={}, params={params:?}, distance={distance:?}",
                                sphere.mode, sphere.origin, sphere.extend
                            ),
                        ));
                    }
                    for (source, overlay) in &world.rlva.visuals.overlays {
                        values.push((
                            *source,
                            format!("setoverlay: texture={}, rgba={:?}", overlay.texture, overlay.color(now)),
                        ));
                    }
                    values
                } else {
                    world.rlva.restrictions()
                };
                if self.tab == 1 {
                    rows.retain(|(_, command)| ["attach", "detach", "outfit", "wear"].iter().any(|name| command.contains(name)));
                }
                rows.retain(|(source, text)| {
                    filter.is_empty()
                        || text.to_lowercase().contains(&filter)
                        || source_name(world, *source).to_lowercase().contains(&filter)
                });
                if ui.button(tr!("rlva-tools-copy")).clicked() {
                    ctx.copy_text(
                        rows.iter()
                            .map(|(source, text)| format!("{}: {text}", source_name(world, *source)))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(
                    ui,
                    // a command over its source, at the current text size
                    ui.text_style_height(&egui::TextStyle::Monospace)
                        + ui.spacing().item_spacing.y
                        + ui.text_style_height(&egui::TextStyle::Small),
                    rows.len(),
                    |ui, range| {
                        for (source, command) in &rows[range] {
                            ui.add(egui::Label::new(RichText::new(command).monospace()).truncate())
                                .on_hover_text(command);
                            ui.add(egui::Label::new(RichText::new(source_name(world, *source)).small().color(p.muted)).truncate())
                                .on_hover_text(source.to_string());
                        }
                    },
                );
            }
        });
        actions.closed = !self.open;
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rlva_console_keeps_its_input_visible_with_long_output_at_accessible_scales() {
        crate::ui::widgets::steady_tests::assert_window_steady(
            "rlva_tools",
            &[Vec2::new(720.0, 500.0), Vec2::new(360.0, 260.0)],
            tr!("rlva-tools-input-hint"),
            || {
                let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
                world.agent_id = Uuid::from_u128(1);
                world.rlva.configure(true);
                for _ in 0..30 {
                    world.rlva_console_input("@version=0");
                }
                (
                    world,
                    Tools {
                        open: true,
                        tab: 3,
                        ..Default::default()
                    },
                )
            },
            |(world, tools), ctx, palette| {
                tools.show(ctx, palette, world);
            },
        );
    }
}

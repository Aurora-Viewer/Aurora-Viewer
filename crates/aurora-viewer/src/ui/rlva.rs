//! Native RLVa restrictions, exceptions, modifiers, locks and console inspection.
//! Port of RlvFloaterBehaviours / Locks / Console (rlvfloaters.cpp, LGPL 2.1).
use super::widgets::{self, Floater};
use crate::{
    theme::Palette,
    world::{
        World,
        inventory::asset_ids::AssetIds,
        rlva::{
            diagnostics::Outcome,
            modifiers::{ModifierValue, hide_texture_ids},
        },
    },
};
use aurora_i18n::tr;
use egui::{RichText, Vec2};
use uuid::Uuid;

/// The Legacy Viewer's behaviours floater has the first three tabs; the locks
/// and the console are floaters of their own there.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Restrictions,
    Exceptions,
    Modifiers,
    Locks,
    Console,
}

#[derive(Default)]
pub struct Tools {
    pub open: bool,
    pub tab: Tab,
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
    /// Exception ids of no known name, which may be groups.
    pub want_groups: Vec<Uuid>,
}

/// A row of the lists: its columns, then the object that issued it.
struct Row {
    cells: Vec<String>,
    source: Option<Uuid>,
    /// The Legacy Viewer greys an exception that does not apply and a modifier
    /// left at its default.
    muted: bool,
}

impl Row {
    fn new(cells: Vec<String>, source: Option<Uuid>, muted: bool) -> Self {
        Self { cells, source, muted }
    }
}

fn source_name(world: &World, source: Uuid) -> String {
    world
        .rlva_attachment_item(source)
        .and_then(|item| world.inventory.items.get(&item).map(|item| item.name.as_str()))
        .map(|name| world.rlva_filter_text(name).into_owned())
        .unwrap_or_else(|| source.to_string())
}

/// The Source column of an exception, resolved like RlvFloaterBehaviours::
/// refreshAll: an attachment, then a resident, then a group, else the option
/// as written while the name is asked. @shownames applies to the resident,
/// which is stricter than the Legacy Viewer's floater.
fn exception_source(world: &World, option: &str, want_groups: &mut Vec<Uuid>) -> String {
    let Ok(id) = Uuid::parse_str(option) else {
        return option.to_owned();
    };
    if id.is_nil() {
        return option.to_owned();
    }
    if let Some(item) = world.rlva_attachment_item(id).and_then(|item| world.inventory.items.get(&item)) {
        return world.rlva_filter_text(&item.name).into_owned();
    }
    if !world.rlva_can_name(id) {
        return crate::world::rlva::privacy::anonymous(id).display_name.clone();
    }
    if let Some(name) = world.social.avatar_names.complete(&id).or_else(|| world.avatar_name(&id)) {
        return name;
    }
    if let Some(group) = world.land.group_names.get(&id) {
        return group.clone();
    }
    if let Some(group) = world.groups.groups.iter().find(|group| group.id == id) {
        return group.name.clone();
    }
    want_groups.push(id);
    option.to_owned()
}

fn modifier_rows(world: &World, now: std::time::Instant) -> Vec<Row> {
    let ids = AssetIds::per_frame(world);
    let mut rows: Vec<Row> = world
        .rlva
        .modifiers()
        .into_iter()
        .map(|modifier| {
            let muted = modifier.value.is_none();
            let value = match modifier.value {
                Some(ModifierValue::Text(text)) => text,
                Some(ModifierValue::Asset(id)) => ids.id(id).to_string(),
                None => tr!("rlva-tools-default").to_owned(),
            };
            Row::new(vec![modifier.name.to_owned(), value], modifier.source, muted)
        })
        .collect();
    // Aurora's additions: a forced view, and the effects' own modifiers, which
    // the Legacy Viewer keeps on each object and does not list.
    if let Some(fov) = world.rlva.camera_values.get("setcam_fov") {
        rows.push(Row::new(vec!["setcam_fov".to_owned(), fov.clone()], None, false));
    }
    for (source, sphere) in &world.rlva.visuals.spheres {
        let (params, distance) = sphere.sample(now);
        rows.push(Row::new(
            vec![
                "setsphere".to_owned(),
                format!(
                    "mode={}, origin={}, extend={}, params={params:?}, distance={distance:?}",
                    sphere.mode, sphere.origin, sphere.extend
                ),
            ],
            Some(*source),
            false,
        ));
    }
    for (source, overlay) in &world.rlva.visuals.overlays {
        rows.push(Row::new(
            vec![
                "setoverlay".to_owned(),
                format!("texture={}, rgba={:?}", ids.id(overlay.texture), overlay.color(now)),
            ],
            Some(*source),
            false,
        ));
    }
    rows
}

/// The Restrictions tab, or the Locks tab with only what locks the outfit.
fn behaviour_rows(world: &World, locks_only: bool) -> Vec<Row> {
    let mut behaviours = world.rlva.behaviours();
    hide_texture_ids(&mut behaviours, || AssetIds::per_frame(world));
    behaviours
        .into_iter()
        .filter(|(_, command)| !locks_only || ["attach", "detach", "outfit", "wear"].iter().any(|name| command.contains(name)))
        .map(|(source, command)| Row::new(vec![command], Some(source), false))
        .collect()
}

fn exception_rows(world: &World, want_groups: &mut Vec<Uuid>) -> Vec<Row> {
    world
        .rlva
        .exceptions()
        .into_iter()
        .map(|exception| {
            let source = exception_source(world, &exception.option, want_groups);
            Row::new(vec![exception.behaviour, source], Some(exception.source), !exception.active)
        })
        .collect()
}

/// One line of columns, truncated to their share of the width.
fn table_row(ui: &mut egui::Ui, p: &Palette, widths: &[f32], cells: &[&str], source: Option<Uuid>, color: egui::Color32) {
    let height = ui.text_style_height(&egui::TextStyle::Monospace);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), egui::Sense::hover());
    let mut left = rect.left() + TABLE_INSET;
    for (index, (cell, width)) in cells.iter().zip(widths).enumerate() {
        let cell_rect = egui::Rect::from_min_size(egui::pos2(left, rect.top()), Vec2::new((*width - TABLE_GAP).max(1.0), height));
        left += width;
        let text = RichText::new(*cell);
        let text = if index + 1 == cells.len() {
            text.small().color(p.muted)
        } else {
            text.monospace().color(color)
        };
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(cell_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                let tip = match source {
                    Some(source) if index + 1 == cells.len() => source.to_string(),
                    _ => (*cell).to_owned(),
                };
                ui.add(egui::Label::new(text).truncate()).on_hover_text(tip);
            },
        );
    }
}

/// `widgets::header_row` starts its first label this far in.
const TABLE_INSET: f32 = 6.0;
const TABLE_GAP: f32 = 8.0;

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
                for (tab, name) in [
                    (Tab::Restrictions, tr!("rlva-tools-restrictions")),
                    (Tab::Exceptions, tr!("rlva-tools-exceptions")),
                    (Tab::Modifiers, tr!("rlva-tools-modifiers")),
                    (Tab::Locks, tr!("rlva-tools-locks")),
                    (Tab::Console, tr!("rlva-tools-console")),
                ] {
                    ui.selectable_value(&mut self.tab, tab, name);
                }
            });
            widgets::divider(ui, p);
            if !world.rlva.enabled() {
                ui.label(RichText::new(tr!("rlva-tools-disabled")).color(p.muted));
            }
            if self.tab == Tab::Console {
                ui.horizontal(|ui| {
                    ui.label(tr!("rlva-tools-local-replies"));
                    actions.clear_log = ui.button(tr!("rlva-tools-clear-log")).clicked();
                });
                // the input and its button measure what the text size gives them
                let (mut list, footer) = widgets::above_footer(ui);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(&mut list, |ui| {
                        let ids = std::cell::OnceCell::new();
                        for entry in &world.rlva.diagnostics.entries {
                            let color = if entry.outcome == Outcome::Rejected { p.danger } else { p.ink };
                            let text = entry.shown(|| ids.get_or_init(|| AssetIds::per_frame(world)));
                            ui.add(
                                egui::Label::new(RichText::new(format!("{} {text}", status(entry.outcome))).monospace().color(color))
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
                let mut rows = match self.tab {
                    Tab::Exceptions => exception_rows(world, &mut actions.want_groups),
                    Tab::Modifiers => modifier_rows(world, std::time::Instant::now()),
                    tab => behaviour_rows(world, tab == Tab::Locks),
                };
                let object_name = |row: &Row| row.source.map(|source| source_name(world, source)).unwrap_or_default();
                rows.retain(|row| {
                    filter.is_empty()
                        || row.cells.iter().any(|cell| cell.to_lowercase().contains(&filter))
                        || object_name(row).to_lowercase().contains(&filter)
                });
                if ui.button(tr!("rlva-tools-copy")).clicked() {
                    ctx.copy_text(
                        rows.iter()
                            .map(|row| {
                                let text = row.cells.join(" ");
                                match row.source {
                                    Some(source) => format!("{}: {text}", source_name(world, source)),
                                    None => text,
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                }
                let (headers, shares): (&[&str], &[f32]) = match self.tab {
                    Tab::Exceptions => (
                        &[
                            tr!("rlva-tools-col-exception"),
                            tr!("rlva-tools-col-source"),
                            tr!("rlva-tools-col-object"),
                        ],
                        &[0.3, 0.35, 0.35],
                    ),
                    Tab::Modifiers => (
                        &[
                            tr!("rlva-tools-col-modifier"),
                            tr!("rlva-tools-col-value"),
                            tr!("rlva-tools-col-object"),
                        ],
                        &[0.4, 0.3, 0.3],
                    ),
                    _ => (&[tr!("rlva-tools-col-restriction"), tr!("rlva-tools-col-object")], &[0.6, 0.4]),
                };
                let available = (ui.available_width() - TABLE_INSET).max(1.0);
                let widths: Vec<f32> = shares.iter().map(|share| share * available).collect();
                let columns: Vec<(&str, f32)> = headers.iter().copied().zip(widths.iter().copied()).collect();
                widgets::header_row(ui, p, &columns);
                if rows.is_empty() {
                    let empty = if self.tab == Tab::Exceptions {
                        tr!("rlva-tools-no-exceptions")
                    } else {
                        tr!("rlva-none")
                    };
                    ui.label(RichText::new(empty).color(p.muted));
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(
                    ui,
                    ui.text_style_height(&egui::TextStyle::Monospace),
                    rows.len(),
                    |ui, range| {
                        for row in &rows[range] {
                            let name = object_name(row);
                            let mut cells: Vec<&str> = row.cells.iter().map(String::as_str).collect();
                            cells.push(&name);
                            table_row(ui, p, &widths, &cells, row.source, if row.muted { p.muted } else { p.ink });
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

    fn world_with(commands: &str) -> World {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(1);
        world.rlva.configure(true);
        world.rlva_console_input(commands);
        world
    }

    #[test]
    fn rlva_lists_hide_a_camera_texture_id_the_rule_does_not_allow() {
        let texture = "00000000-0000-0000-0000-0000000000bb";
        let world = world_with(&format!("@setcam_textures:{texture}=n"));
        let modifiers = modifier_rows(&world, std::time::Instant::now());
        let behaviours = behaviour_rows(&world, false);
        for row in modifiers.iter().chain(&behaviours) {
            assert!(!row.cells.iter().any(|cell| cell.contains(texture)), "{:?}", row.cells);
        }
        assert!(modifiers.iter().any(|row| row.cells[1].starts_with("hidden-")));
        assert_eq!(behaviours.len(), 1);
        assert!(behaviours[0].cells[0].starts_with("setcam_textures:hidden-"));
    }

    #[test]
    fn rlva_exception_rows_name_the_id_and_keep_shownames_private() {
        let friend = Uuid::from_u128(0xa);
        let mut want_groups = Vec::new();
        let world = world_with(&format!("@recvim=n,recvim:{friend}=add,recvim:20;50=n"));
        let rows = exception_rows(&world, &mut want_groups);
        let unknown = rows
            .iter()
            .find(|row| row.cells[1] == friend.to_string())
            .expect("the id, until its name is known");
        assert!(!unknown.muted, "an exception that applies is not greyed");
        assert_eq!(want_groups, [friend], "the id may be a group: its name is asked");
        assert!(rows.iter().find(|row| row.cells[1] == "20;50").expect("a distance").muted);

        let mut want_groups = Vec::new();
        let world = world_with(&format!("@shownames=n,recvim:{friend}=add"));
        let rows = exception_rows(&world, &mut want_groups);
        assert_eq!(rows[0].cells[1], crate::world::rlva::privacy::anonymous(friend).display_name);
        assert!(want_groups.is_empty(), "nothing is asked for a name that stays hidden");
    }

    #[test]
    fn rlva_list_tabs_keep_the_window_steady_at_accessible_scales() {
        for tab in [Tab::Restrictions, Tab::Exceptions, Tab::Modifiers, Tab::Locks] {
            crate::ui::widgets::steady_tests::assert_window_steady(
                "rlva_tools",
                &[Vec2::new(720.0, 500.0), Vec2::new(360.0, 260.0)],
                tr!("rlva-tools-filter"),
                || {
                    (
                        world_with("@jump=n,detach=n,recvim=n,recvim:00000000-0000-0000-0000-00000000000a=add,fartouch:2=n"),
                        Tools {
                            open: true,
                            tab,
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
                        tab: Tab::Console,
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

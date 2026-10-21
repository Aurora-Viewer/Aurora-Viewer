//! Compact Aurora build workspace. The seven sections reorganize the
//! LLFloaterTools / LLPanelObject controls without changing their commands.

use super::{Env, common, contents, features, general, object, replace, texture, tools};
use crate::build::{BuildSettings, BuildTool, EditMode, Mods, Tool};
use crate::theme::Palette;
use crate::ui::icons;
use crate::world::World;
use egui::{RichText, Stroke, Vec2};
use glam::Vec3;

pub(super) fn sections() -> [(usize, &'static str, &'static str); 7] {
    [
        (6, "info-circle-filled", aurora_i18n::tr!("build-compact-general")),
        (1, "arrows-move", aurora_i18n::tr!("build-compact-transform")),
        (5, "box", aurora_i18n::tr!("build-compact-shape")),
        (3, "paint-filled", aurora_i18n::tr!("build-compact-materials")),
        (2, "adjustments-filled", aurora_i18n::tr!("build-compact-features")),
        (0, "lock-filled", aurora_i18n::tr!("build-compact-rights")),
        (4, "folder-filled", aurora_i18n::tr!("build-compact-contents")),
    ]
}

pub(super) fn window_heading(tool: &BuildTool) -> (&'static str, &'static str) {
    if tool.tool == Tool::Edit {
        let (_, icon, title) = sections().into_iter().find(|(tab, _, _)| *tab == tool.tab).unwrap_or(sections()[0]);
        (icon, title)
    } else {
        match tool.tool {
            Tool::Focus => ("zoom-filled", aurora_i18n::tr!("build-compact-camera")),
            Tool::Grab => ("hand-grab", aurora_i18n::tr!("build-compact-grab")),
            Tool::Create => ("wand", aurora_i18n::tr!("build-compact-create")),
            Tool::Land => ("mountain-filled", aurora_i18n::tr!("build-compact-land")),
            Tool::Edit => unreachable!(),
        }
    }
}

pub(super) fn object_status(tool: &BuildTool, world: &mut World, p: &Palette) -> Vec<(&'static str, String, egui::Color32)> {
    use aurora_net::build::perm;
    let roots = tool.roots(world);
    if roots.is_empty() {
        return Vec::new();
    }
    let locked = roots.iter().any(|&idx| {
        world
            .objects
            .get(idx)
            .and_then(|o| tool.props.get(&o.full_id))
            .is_some_and(|pr| pr.owner_mask & perm::MOVE == 0)
    });
    let no_mod = tool.sel_prims(world).iter().any(|&idx| !tool.can_modify(world, idx));
    // LLSelectMgr::selectGetOwner (indra/newview/llselectmgr.cpp): a common
    // avatar/group owner, or Multiple. Keep ownership first for every state.
    let owners: Vec<_> = roots
        .iter()
        .filter_map(|&idx| {
            world.objects.get(idx).map(|o| {
                tool.props.get(&o.full_id).map_or((o.owner_id, false), |pr| {
                    (pr.owner_id, !pr.group_id.is_nil() && pr.owner_id == pr.group_id)
                })
            })
        })
        .collect();
    let Some(&(owner, group_owned)) = owners.first() else {
        return Vec::new();
    };
    let ownership = if owners.iter().any(|&value| value != (owner, group_owned)) {
        ("users-group", aurora_i18n::tr!("status-multiple").into(), p.violet_pale)
    } else if group_owned {
        let name = world.land.group_name(&owner, &world.groups.groups).unwrap_or_else(|| "…".into());
        ("users-group", name, p.violet_pale)
    } else if owner == world.agent_id {
        ("user", aurora_i18n::tr!("build-status-self").into(), p.muted)
    } else if owner.is_nil() {
        ("user", aurora_i18n::tr!("status-public").into(), p.muted)
    } else {
        world.social.want_name(owner);
        ("user", world.social.name_of(&owner), p.violet_pale)
    };
    let mut states = vec![ownership];
    if locked {
        states.push(("lock-filled", aurora_i18n::tr!("build-ui-object-locked").into(), p.danger));
    }
    if no_mod {
        states.push(("pencil-off", aurora_i18n::tr!("build-status-no-mod").into(), p.warn));
    }
    if !locked && !no_mod {
        states.push(("check", aurora_i18n::tr!("build-status-editable").into(), p.success));
    }
    states
}

/// Actions without a second section title: the title belongs to the floater.
pub(super) fn actions(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            body(ui);
        });
    });
}

fn glyph(ui: &egui::Ui, rect: egui::Rect, name: &str, color: egui::Color32) {
    if let Some(t) = icons::global(name) {
        ui.painter().image(
            t.id(),
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            color,
        );
    }
}

fn divider(ui: &mut egui::Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, crate::ui::widgets::divider_stroke(p));
}

fn band<R>(ui: &mut egui::Ui, fill: egui::Color32, x: i8, y: i8, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(fill)
        .inner_margin(egui::Margin::symmetric(x, y))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            body(ui)
        })
        .inner
}

pub fn icon_button(ui: &mut egui::Ui, p: &Palette, name: &str, title: &str, enabled: bool) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(22.0, 24.0), egui::Sense::click());
        if response.hovered() && enabled {
            ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
        }
        glyph(
            ui,
            rect.shrink(5.0),
            name,
            if !enabled {
                p.muted_dim
            } else if response.hovered() {
                p.ink
            } else {
                p.muted
            },
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, title));
        response.on_hover_text(title)
    })
    .inner
}

pub fn heading(ui: &mut egui::Ui, p: &Palette, icon: &str, title: &str, actions: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 9.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(17.0), egui::Sense::hover());
        glyph(ui, rect, icon, p.violet_light);
        ui.label(RichText::new(title).size(14.0).strong().color(p.ink));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            actions(ui);
        });
    });
    ui.add_space(10.0);
}

fn selection(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &World, s: &BuildSettings) {
    let roots = tool.roots(world);
    let prims = tool.sel_prims(world);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(5.0, 4.0);
        ui.spacing_mut().interact_size.y = 14.0;
        if prims.is_empty() {
            common::small(ui, p, aurora_i18n::tr!("build-compact-select-hint"));
            return;
        }
        ui.horizontal(|ui| {
            let impact = tool
                .land_impact(world)
                .map(|v| aurora_i18n::integer(v.round() as i64))
                .unwrap_or_else(|| "…".into());
            if ui
                .add(
                    egui::Label::new(
                        RichText::new(aurora_i18n::tr!("build-compact-impact", impact = impact))
                            .size(10.0)
                            .color(p.violet_light),
                    )
                    .sense(egui::Sense::click()),
                )
                .clicked()
            {
                tool.ui.weights_open = !tool.ui.weights_open;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(aurora_i18n::tr!("build-compact-counts", objects = roots.len(), prims = prims.len()))
                        .size(10.0)
                        .color(p.muted),
                );
            });
        });
        ui.horizontal(|ui| {
            if let Some((_, remaining)) = tool.costs.capacity(world) {
                common::small(ui, p, &aurora_i18n::tr!("build-c3-remaining", remaining = remaining));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                replace::button(ui, p, tool, world, s);
            });
        });
    });
    ui.add_space(8.0);
}

pub(super) fn manipulation(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods) {
    ui.scope(|ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (mode, icon, title) in [
                (EditMode::Move, "arrows-move", aurora_i18n::tr!("build-compact-move")),
                (EditMode::Rotate, "rotate-clockwise", aurora_i18n::tr!("build-compact-rotate")),
                (EditMode::Stretch, "arrows-maximize", aurora_i18n::tr!("build-compact-stretch")),
                (
                    EditMode::Align,
                    "layout-align-center-filled",
                    aurora_i18n::tr!("build-compact-align"),
                ),
            ] {
                let (rect, response) = ui.allocate_exact_size(Vec2::new(28.0, 26.0), egui::Sense::click());
                let active = tool.effective_mode(mods) == mode;
                ui.painter().rect_filled(
                    rect,
                    ui.visuals().window_corner_radius,
                    if active || response.hovered() { p.raised } else { p.field },
                );
                if active {
                    ui.painter().rect_stroke(
                        rect,
                        ui.visuals().window_corner_radius,
                        Stroke::new(1.0, p.violet_light),
                        egui::StrokeKind::Inside,
                    );
                }
                glyph(
                    ui,
                    egui::Rect::from_center_size(rect.center(), Vec2::splat(17.0)),
                    icon,
                    if active { p.violet_pale } else { p.muted },
                );
                if response.on_hover_text(title).clicked() {
                    tool.set_edit_mode(world, s, mode);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, p, "grid-pattern-filled", aurora_i18n::tr!("build-compact-grid-options"), true).clicked() {
                    tool.ui.grid_options_open = !tool.ui.grid_options_open;
                }
            });
        });
    });
}

fn transform_options(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, s: &mut BuildSettings) {
    ui.add_space(4.0);
    divider(ui, p);
    ui.add_space(4.0);
    ui.spacing_mut().interact_size.y = 20.0;
    ui.horizontal(|ui| {
        if let Some(v) = common::check(ui, p, s.snap, false, aurora_i18n::tr!("build-compact-snap"), true) {
            s.snap = v;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let unit = aurora_i18n::tr!("build-compact-grid-unit", unit = aurora_i18n::number(s.grid_resolution as f64, 1));
            if ui
                .add(egui::Label::new(RichText::new(unit).size(11.0).color(p.muted)).sense(egui::Sense::click()))
                .on_hover_text(aurora_i18n::tr!("build-compact-grid-options"))
                .clicked()
            {
                tool.ui.grid_options_open = !tool.ui.grid_options_open;
            }
        });
    });
    ui.vertical(|ui| {
        for (value, title) in [
            (&mut s.scale_stretch_textures, aurora_i18n::tr!("build-compact-stretch-textures")),
            (&mut s.show_highlight, aurora_i18n::tr!("build-compact-highlight")),
            (&mut s.follow_edit_object, aurora_i18n::tr!("build-ui-tools-follow-edit-object")),
        ] {
            if let Some(v) = common::check(ui, p, *value, false, title, true) {
                *value = v;
            }
        }
    });
}

fn rail_button(ui: &mut egui::Ui, p: &Palette, icon: &str, title: &str, active: bool, height: f32) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(34.0, height), egui::Sense::click());
    if active || response.hovered() {
        ui.painter().rect_filled(
            rect,
            ui.visuals().window_corner_radius,
            if active { p.violet.gamma_multiply(0.3) } else { p.raised },
        );
    }
    if active {
        ui.painter().vline(rect.left(), rect.y_range(), Stroke::new(2.0, p.violet));
    }
    glyph(
        ui,
        egui::Rect::from_center_size(rect.center(), Vec2::splat(18.0)),
        icon,
        if active { p.violet_light } else { p.muted },
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, active, title));
    response.on_hover_text(title).clicked()
}

fn grid_mode_buttons(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &World, s: &mut BuildSettings) {
    let choices = [
        (0u8, "world-filled", aurora_i18n::tr!("build-compact-world")),
        (1, "box", aurora_i18n::tr!("build-compact-local")),
        (2, "focus-centered", aurora_i18n::tr!("build-compact-reference")),
    ];
    ui.spacing_mut().item_spacing.x = 3.0;
    let (axes, _) = ui.allocate_exact_size(Vec2::new(34.0, 26.0), egui::Sense::hover());
    #[cfg(test)]
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("build_grid_axes"), axes));
    // The XYZ content uses the same axis palette as the build manipulator.
    let origin = axes.center() + egui::vec2(-2.0, 5.0);
    for (end, color) in [
        (origin + egui::vec2(0.0, -19.0), p.indigo_light),
        (origin + egui::vec2(14.0, 4.0), p.danger),
        (origin + egui::vec2(-11.0, 7.0), p.success),
    ] {
        ui.painter().arrow(origin, end - origin, Stroke::new(1.5, color));
    }
    for (mode, icon, label) in choices {
        let active = s.grid_mode == mode;
        let kind = if active {
            crate::ui::widgets::ActionKind::Primary
        } else {
            crate::ui::widgets::ActionKind::Flat
        };
        let response = crate::ui::widgets::action_button(ui, p, "", "", kind, Vec2::new(34.0, 28.0));
        #[cfg(test)]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new(("build_grid_mode", mode)), response.rect));
        glyph(
            ui,
            egui::Rect::from_center_size(response.rect.center(), Vec2::splat(22.0)),
            icon,
            if active { p.on_accent } else { p.muted },
        );
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, active, label));
        if response
            .on_hover_text(aurora_i18n::tr!("build-compact-grid-mode", mode = label))
            .clicked()
        {
            s.grid_mode = mode;
            if mode == 2 && tool.grid_objects.is_empty() {
                tool.use_selection_for_grid(world, s);
            }
        }
    }
}

fn footer(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods) {
    use crate::ui::widgets::{self, ActionKind};
    band(ui, p.bar.gamma_multiply(0.82), 8, 6, |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(2.0, 3.0);
        ui.spacing_mut().interact_size.y = 22.0;
        let selected = !tool.selection.is_empty();
        let width = ui.available_width();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.allocate_ui_with_layout(
                Vec2::new((width - 42.0).max(180.0), 149.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_width((width - 42.0).max(180.0));
                    ui.spacing_mut().icon_width = 13.0;
                    widgets::checkbox(
                        ui,
                        p,
                        &mut s.scale_uniform,
                        RichText::new(aurora_i18n::tr!("build-compact-both-sides")).size(10.5),
                    );
                    widgets::checkbox(
                        ui,
                        p,
                        &mut s.actual_root,
                        RichText::new(aurora_i18n::tr!("build-compact-root-axis")).size(10.5),
                    );
                    ui.horizontal(|ui| {
                        let mut linked = s.edit_linked;
                        if widgets::checkbox(
                            ui,
                            p,
                            &mut linked,
                            RichText::new(aurora_i18n::tr!("build-compact-linked")).size(10.5),
                        )
                        .changed()
                        {
                            tool.set_edit_linked(world, s, linked);
                        }
                        if s.edit_linked {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if icon_button(ui, p, "caret-right-filled", aurora_i18n::tr!("build-linked-next"), selected).clicked() {
                                    tool.select_next_link(world, s, true, mods.shift);
                                }
                                let number = tool
                                    .selected_link_number(world)
                                    .map(|n| n.to_string())
                                    .unwrap_or_else(|| "—".into());
                                ui.label(RichText::new(&number).size(10.0).color(p.violet_pale))
                                    .on_hover_text(aurora_i18n::tr!("build-ui-tools-link-number", n = number));
                                if icon_button(ui, p, "caret-left-filled", aurora_i18n::tr!("build-linked-previous"), selected).clicked() {
                                    tool.select_next_link(world, s, false, mods.shift);
                                }
                            });
                        }
                    });
                    ui.horizontal(|ui| {
                        let mut face = tool.edit_mode == EditMode::Face;
                        if widgets::checkbox(
                            ui,
                            p,
                            &mut face,
                            RichText::new(aurora_i18n::tr!("build-compact-select-face")).size(10.5),
                        )
                        .changed()
                        {
                            tool.set_edit_mode(world, s, if face { EditMode::Face } else { EditMode::Move });
                        }
                        if face {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let next = icon_button(ui, p, "caret-right-filled", aurora_i18n::tr!("build-compact-next"), selected);
                                if next.clicked() {
                                    tool.select_next_part(world, s, true, mods.shift);
                                }
                                let numbers = tool.selected_face_numbers(world);
                                let number_width =
                                    (ui.available_width() - next.rect.width() - ui.spacing().item_spacing.x).clamp(12.0, 58.0);
                                ui.add_sized(
                                    Vec2::new(number_width, 20.0),
                                    egui::Label::new(RichText::new(&numbers).size(10.0).color(p.violet_pale)).truncate(),
                                )
                                .on_hover_text(aurora_i18n::tr!("build-ui-tools-faces", list = numbers));
                                if icon_button(ui, p, "caret-left-filled", aurora_i18n::tr!("build-compact-previous"), selected).clicked() {
                                    tool.select_next_part(world, s, false, mods.shift);
                                }
                            });
                        }
                    });
                    widgets::checkbox(
                        ui,
                        p,
                        &mut s.select_probes,
                        RichText::new(aurora_i18n::tr!("build-compact-probes")).size(10.5),
                    );
                    widgets::checkbox(
                        ui,
                        p,
                        &mut s.rez_under_land_group,
                        RichText::new(aurora_i18n::tr!("rez-under-land-group")).size(10.5),
                    );
                },
            );
            ui.vertical(|ui| grid_mode_buttons(ui, p, tool, world, s));
        });
        ui.horizontal(|ui| {
            if icon_button(ui, p, "arrow-back-up", aurora_i18n::tr!("build-compact-undo"), selected).clicked() {
                tool.undo(world, false);
            }
            if icon_button(ui, p, "arrow-forward-up", aurora_i18n::tr!("build-compact-redo"), selected).clicked() {
                tool.undo(world, true);
            }
            if icon_button(
                ui,
                p,
                "link",
                aurora_i18n::tr!("build-compact-link"),
                !tool.attachment_selected(world) && !s.edit_linked && tool.roots(world).len() >= 2,
            )
            .clicked()
            {
                tool.link(world);
            }
            if icon_button(
                ui,
                p,
                "unlink",
                aurora_i18n::tr!("build-compact-unlink"),
                !tool.attachment_selected(world) && selected,
            )
            .clicked()
            {
                tool.unlink(world);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::action_button_if(
                    ui,
                    p,
                    "clipboard-text-filled",
                    aurora_i18n::tr!("build-compact-uuid"),
                    ActionKind::Flat,
                    Vec2::new(68.0, 24.0),
                    selected,
                )
                .on_hover_text(aurora_i18n::tr!("build-ui-general-copy-object-keys-shift-all-parts"))
                .clicked()
                {
                    ui.ctx().copy_text(tool.copy_keys(world, s, mods.shift));
                }
                if widgets::action_button_if(
                    ui,
                    p,
                    "copy-filled",
                    aurora_i18n::tr!("build-compact-duplicate"),
                    ActionKind::Flat,
                    Vec2::new(94.0, 24.0),
                    selected && !tool.attachment_selected(world),
                )
                .on_hover_text("Ctrl+D")
                .clicked()
                {
                    tool.duplicate(world, Vec3::new(0.5, 0.5, 0.0), true);
                }
            });
        });
        if !tool.status.is_empty() {
            ui.add(egui::Label::new(RichText::new(&tool.status).size(10.0).color(p.warn)).truncate())
                .on_hover_text(&tool.status);
        }
    });
}

pub fn body(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods, env: &mut Env) {
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    if !s.expanded {
        return;
    }
    if tool.replacement.draft.is_some() || tool.replacement.job.is_some() {
        band(ui, egui::Color32::TRANSPARENT, 12, 12, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 5.0);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            replace::panel(ui, p, tool, world);
        });
        return;
    }
    // The floater owns its size; filling its body lets edge/corner drags
    // reveal more content and keeps the shared footer against the bottom.
    let height = ui.available_height().max(300.0);
    let rail_scale = (height / 494.0).min(1.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let rail = ui.allocate_ui_with_layout(Vec2::new(44.0, height), egui::Layout::top_down(egui::Align::Min), |ui| {
            band(ui, p.bar.gamma_multiply(0.82), 5, 8, |ui| {
                ui.set_min_height(height - 16.0);
                ui.spacing_mut().item_spacing.y = 4.0 * rail_scale;
                for (tab, icon, title) in sections() {
                    if rail_button(ui, p, icon, title, tool.tool == Tool::Edit && tool.tab == tab, 36.0 * rail_scale) {
                        if tool.tool != Tool::Edit {
                            tool.set_tool(Tool::Edit);
                        }
                        tool.tab = tab;
                    }
                }
                ui.add_space(5.0 * rail_scale);
                divider(ui, p);
                ui.add_space(5.0 * rail_scale);
                for (icon, title, mode) in tools::tools() {
                    if rail_button(ui, p, icon, title, tool.tool == mode, 32.0 * rail_scale) && tool.tool != mode {
                        tool.set_tool(mode);
                    }
                }
            });
        });
        let rail_edge = rail.response.rect.right();
        let width = ui.available_width();
        let editing = tool.tool == Tool::Edit;
        let shared_options = editing || tool.tool == Tool::Create;
        let hint = (!editing).then(|| {
            let text = if tool.status.is_empty() {
                tools::status_text(tool, s, mods)
            } else {
                &tool.status
            };
            egui::WidgetText::from(
                RichText::new(text)
                    .size(10.0)
                    .color(if tool.status.is_empty() { p.muted } else { p.warn }),
            )
            .into_galley(ui, Some(egui::TextWrapMode::Wrap), (width - 24.0).max(1.0), egui::TextStyle::Body)
        });
        let footer_height = if shared_options {
            194.0 + if tool.status.is_empty() { 0.0 } else { 20.0 }
        } else {
            // Measure the same galley that is displayed below. A fixed 40 px
            // estimate made Grab's wrapped hint overflow by 4 px, growing the
            // remembered window by that amount on every frame.
            hint.as_ref().map_or(16.0, |galley| galley.size().y + 16.0)
        };
        ui.allocate_ui_with_layout(Vec2::new(width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
            band(ui, egui::Color32::TRANSPARENT, 12, 10, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(5.0, 4.0);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                egui::ScrollArea::vertical()
                    .id_salt(("c3_section", tool.tool as u8, tool.tab))
                    .max_height((height - footer_height - 20.0).max(1.0))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if editing {
                            selection(ui, p, tool, world, s);
                            if tool.selection.is_empty() && tool.tab != 3 {
                                return;
                            }
                            match tool.tab {
                                0 | 6 => general::show(ui, p, tool, world, tool.tab == 0),
                                2 => {
                                    object::show_flags(ui, p, tool, world);
                                    crate::ui::widgets::divider(ui, p);
                                    features::show(ui, p, tool, world, s, env);
                                }
                                3 => texture::show(ui, p, tool, world, s, env),
                                4 => contents::show(ui, p, tool, world),
                                5 => object::show_shape(ui, p, tool, world, env),
                                _ => {
                                    object::show_transform(ui, p, tool, world, s, mods);
                                    transform_options(ui, p, tool, s);
                                }
                            }
                        } else {
                            match tool.tool {
                                Tool::Focus => tools::focus_panel(ui, p, tool, mods, env),
                                Tool::Grab => tools::grab_panel(ui, p, tool, mods),
                                Tool::Create => tools::create_panel(ui, p, s),
                                Tool::Land => {
                                    tools::land_panel(ui, p, tool, world, s);
                                    crate::ui::widgets::divider(ui, p);
                                    tools::parcel_panel(ui, p, tool, world, s);
                                }
                                Tool::Edit => {}
                            }
                        }
                    });
            });
            if shared_options {
                divider(ui, p);
                // A section's minimum content width can exceed its viewport.
                // Keep the footer in the inspector's chosen width independently.
                ui.allocate_ui_with_layout(Vec2::new(width, footer_height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    footer(ui, p, tool, world, s, mods);
                });
            } else if let Some(hint) = hint {
                band(ui, p.bar.gamma_multiply(0.82), 12, 8, |ui| {
                    ui.add(egui::Label::new(hint));
                });
            }
        });
        ui.painter().vline(
            rail_edge,
            rail.response.rect.top()..=rail.response.rect.top() + height,
            crate::ui::widgets::divider_stroke(p),
        );
    });
}

#[cfg(test)]
#[path = "compact_tests.rs"]
mod tests;

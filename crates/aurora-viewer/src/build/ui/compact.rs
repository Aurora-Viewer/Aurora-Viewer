//! Compact Aurora build workspace. The six sections reorganize the
//! LLFloaterTools / LLPanelObject controls without changing their commands.

use super::{Env, common, contents, features, general, object, replace, texture, tools};
use crate::build::{BuildSettings, BuildTool, EditMode, Mods, Tool};
use crate::theme::Palette;
use crate::ui::icons;
use crate::world::World;
use egui::{RichText, Stroke, Vec2};
use glam::Vec3;

fn sections() -> [(usize, &'static str, &'static str); 6] {
    [
        (1, "arrows-move", aurora_i18n::tr!("build-compact-transform")),
        (5, "box", aurora_i18n::tr!("build-compact-shape")),
        (3, "paint-filled", aurora_i18n::tr!("build-compact-materials")),
        (2, "adjustments-filled", aurora_i18n::tr!("build-compact-features")),
        (0, "lock-filled", aurora_i18n::tr!("build-compact-rights")),
        (4, "folder-filled", aurora_i18n::tr!("build-compact-contents")),
    ]
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
    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0, p.raised));
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
        let (rect, response) = ui.allocate_exact_size(Vec2::new(24.0, 24.0), egui::Sense::click());
        if response.hovered() && enabled {
            ui.painter().rect_filled(rect, 3.0, p.raised);
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

pub fn selection_title(tool: &BuildTool, world: &World) -> String {
    let roots = tool.roots(world);
    if roots.len() > 1 {
        return aurora_i18n::tr!("build-compact-multiple").to_owned();
    }
    tool.selection
        .first()
        .and_then(|key| world.objects.index_of(key))
        .and_then(|idx| world.objects.get(idx))
        .and_then(|o| tool.props.get(&o.full_id))
        .map(|props| props.name.as_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(aurora_i18n::tr!("build-compact-title"))
        .to_owned()
}

fn selection(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &World, s: &BuildSettings) {
    let roots = tool.roots(world);
    let prims = tool.sel_prims(world);
    let known = prims
        .first()
        .and_then(|&i| world.objects.get(i))
        .is_some_and(|o| tool.props.contains_key(&o.full_id));
    let editable = prims.iter().all(|&idx| tool.can_modify(world, idx));
    let status = if !known {
        aurora_i18n::tr!("build-compact-waiting")
    } else if editable {
        aurora_i18n::tr!("build-compact-editable")
    } else {
        aurora_i18n::tr!("build-compact-readonly")
    };
    band(ui, p.bar.gamma_multiply(0.82), 12, 5, |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(5.0, 4.0);
        ui.spacing_mut().interact_size.y = 14.0;
        if prims.is_empty() {
            common::small(ui, p, aurora_i18n::tr!("build-compact-select-hint"));
            return;
        }
        ui.horizontal(|ui| {
            let (dot, _) = ui.allocate_exact_size(Vec2::splat(5.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(dot.center(), 2.5, if editable && known { p.teal } else { p.muted });
            ui.add(egui::Label::new(RichText::new(status).size(10.0).color(p.muted)).truncate())
                .on_hover_text(status);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(aurora_i18n::tr!("build-compact-counts", objects = roots.len(), prims = prims.len()))
                        .size(10.0)
                        .color(p.muted),
                );
            });
        });
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
            if let Some((_, remaining)) = tool.costs.capacity(world) {
                ui.label(
                    RichText::new(aurora_i18n::tr!("build-c3-remaining", remaining = remaining))
                        .size(10.0)
                        .color(p.muted),
                );
            }
        });
        if let Some(info) = tool.part_info(world, s) {
            common::small(ui, p, &info);
        }
        replace::button(ui, p, tool, world, s);
    });
}

pub(super) fn manipulation(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods) {
    ui.scope(|ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (mode, icon, title) in [
                (EditMode::Move, "arrows-move", aurora_i18n::tr!("build-compact-move")),
                (EditMode::Rotate, "rotate-clockwise", aurora_i18n::tr!("build-compact-rotate")),
                (EditMode::Stretch, "arrows-maximize", aurora_i18n::tr!("build-compact-stretch")),
                (EditMode::Face, "marquee-2", aurora_i18n::tr!("build-compact-face")),
                (
                    EditMode::Align,
                    "layout-align-center-filled",
                    aurora_i18n::tr!("build-compact-align"),
                ),
            ] {
                let (rect, response) = ui.allocate_exact_size(Vec2::new(28.0, 26.0), egui::Sense::click());
                let active = tool.effective_mode(mods) == mode;
                ui.painter()
                    .rect_filled(rect, 3.0, if active || response.hovered() { p.raised } else { p.field });
                if active {
                    ui.painter()
                        .rect_stroke(rect, 3.0, Stroke::new(1.0, p.violet_light), egui::StrokeKind::Inside);
                }
                glyph(
                    ui,
                    egui::Rect::from_center_size(rect.center(), Vec2::splat(17.0)),
                    icon,
                    if active { p.violet_pale } else { p.muted },
                );
                if response.on_hover_text(title).clicked() {
                    tool.set_edit_mode(world, s, mode);
                    if mode == EditMode::Face {
                        tool.tab = 3;
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, p, "grid-pattern-filled", aurora_i18n::tr!("build-compact-grid-options"), true).clicked() {
                    tool.ui.grid_options_open = !tool.ui.grid_options_open;
                }
                let items = [
                    (aurora_i18n::tr!("build-compact-world"), 0u8),
                    (aurora_i18n::tr!("build-compact-local"), 1),
                    (aurora_i18n::tr!("build-compact-reference"), 2),
                ];
                if let Some(mode) = common::combo(ui, "compact_grid", Some(s.grid_mode), &items, 83.0, true) {
                    s.grid_mode = mode;
                    if mode == 2 && tool.grid_objects.is_empty() {
                        tool.use_selection_for_grid(world, s);
                    }
                }
            });
        });
    });
}

fn transform_options(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings) {
    ui.add_space(4.0);
    divider(ui, p);
    ui.add_space(4.0);
    ui.spacing_mut().interact_size.y = 20.0;
    if let Some(v) = common::check(ui, s.edit_linked, false, aurora_i18n::tr!("build-compact-linked"), true) {
        tool.set_edit_linked(world, s, v);
    }
    ui.horizontal(|ui| {
        if let Some(v) = common::check(ui, s.snap, false, aurora_i18n::tr!("build-compact-snap"), true) {
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
    let options = aurora_i18n::tr!("build-compact-options");
    crate::ui::widgets::collapsing(ui, p, "compact_manipulation", options, false, |ui| {
        for (value, title) in [
            (&mut s.scale_uniform, aurora_i18n::tr!("build-compact-both-sides")),
            (&mut s.scale_stretch_textures, aurora_i18n::tr!("build-compact-stretch-textures")),
            (&mut s.actual_root, aurora_i18n::tr!("build-compact-root-axis")),
            (&mut s.show_highlight, aurora_i18n::tr!("build-compact-highlight")),
            (&mut s.select_probes, aurora_i18n::tr!("build-compact-probes")),
            (&mut s.follow_edit_object, aurora_i18n::tr!("build-ui-tools-follow-edit-object")),
        ] {
            if let Some(v) = common::check(ui, *value, false, title, true) {
                *value = v;
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn sections_panel(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    env: &mut Env,
    height: f32,
) {
    let response = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let rail = ui.allocate_ui_with_layout(Vec2::new(44.0, height), egui::Layout::top_down(egui::Align::Min), |ui| {
            band(ui, p.bar.gamma_multiply(0.82), 5, 8, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                for (tab, icon, title) in sections() {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(34.0, 36.0), egui::Sense::click());
                    let active = tool.tab == tab;
                    if active || resp.hovered() {
                        ui.painter()
                            .rect_filled(rect, 3.0, if active { p.violet.gamma_multiply(0.3) } else { p.raised });
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
                    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::SelectableLabel, true, title));
                    if resp.on_hover_text(title).clicked() {
                        tool.tab = tab;
                    }
                }
            });
        });
        let rail_edge = rail.response.rect.right();
        let width = ui.available_width();
        let properties = ui.allocate_ui_with_layout(Vec2::new(width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
            band(ui, egui::Color32::TRANSPARENT, 12, 12, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(5.0, 5.0);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                egui::ScrollArea::vertical()
                    .id_salt(("c3_section", tool.tab))
                    .max_height((height - 24.0).max(1.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        if tool.selection.is_empty() {
                            common::label(ui, p, aurora_i18n::tr!("build-compact-select-hint"));
                            return;
                        }
                        if !matches!(tool.tab, 1 | 3 | 5)
                            && let Some((_, icon, title)) = sections().into_iter().find(|(tab, _, _)| *tab == tool.tab)
                        {
                            heading(ui, p, icon, title, |_| {});
                        }
                        match tool.tab {
                            0 => general::show(ui, p, tool, world, s),
                            2 => {
                                object::show_flags(ui, p, tool, world);
                                ui.separator();
                                features::show(ui, p, tool, world, s, env);
                            }
                            3 => texture::show(ui, p, tool, world, s, env),
                            4 => contents::show(ui, p, tool, world),
                            5 => object::show_shape(ui, p, tool, world, env),
                            _ => {
                                object::show_transform(ui, p, tool, world, s, mods);
                                transform_options(ui, p, tool, world, s);
                            }
                        }
                    });
            });
        });
        let bottom = rail.response.rect.bottom().max(properties.response.rect.bottom());
        ui.painter()
            .vline(rail_edge, rail.response.rect.top()..=bottom, Stroke::new(1.0, p.raised));
    });
    // The rail has a stable minimum; longer property groups scroll within the
    // available screen while short categories let the inspector shrink.
    let _ = response;
}

fn footer(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods) {
    band(ui, p.bar.gamma_multiply(0.82), 10, 6, |ui| {
        ui.spacing_mut().interact_size.y = 24.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.spacing_mut().interact_size.y = 24.0;
            ui.spacing_mut().button_padding = Vec2::new(7.0, 3.0);
            let selected = !tool.selection.is_empty();
            if tool.tool == Tool::Edit && (s.edit_linked || tool.edit_mode == EditMode::Face) {
                for (forward, icon, title) in [
                    (false, "caret-left-filled", aurora_i18n::tr!("build-compact-previous")),
                    (true, "caret-right-filled", aurora_i18n::tr!("build-compact-next")),
                ] {
                    if icon_button(ui, p, icon, title, selected).clicked() {
                        tool.select_next_part(world, s, forward, mods.shift);
                    }
                }
            }
            if icon_button(
                ui,
                p,
                "arrow-back-up",
                aurora_i18n::tr!("build-compact-undo"),
                selected || tool.tool == Tool::Land,
            )
            .clicked()
            {
                if tool.tool == Tool::Land {
                    let mut commands = Vec::new();
                    tool.land.undo(&mut commands);
                    for command in commands {
                        tool.send(command);
                    }
                } else {
                    tool.undo(world, false);
                }
            }
            if icon_button(
                ui,
                p,
                "arrow-forward-up",
                aurora_i18n::tr!("build-compact-redo"),
                selected && tool.tool != Tool::Land,
            )
            .clicked()
            {
                tool.undo(world, true);
            }
            ui.add_space(3.0);
            let (line, _) = ui.allocate_exact_size(Vec2::new(1.0, 16.0), egui::Sense::hover());
            ui.painter().vline(line.center().x, line.y_range(), Stroke::new(1.0, p.raised));
            ui.add_space(3.0);
            let label = RichText::new(aurora_i18n::tr!("build-compact-duplicate")).size(10.0);
            let button = if let Some(icon) = icons::global("copy-filled") {
                egui::Button::image_and_text(egui::Image::new((icon.id(), Vec2::splat(12.0))).tint(p.ink), label)
            } else {
                egui::Button::new(label)
            };
            if ui
                .add_enabled(
                    selected,
                    button
                        .fill(p.field)
                        .stroke(Stroke::new(1.0, p.raised))
                        .corner_radius(ui.visuals().window_corner_radius)
                        .min_size(Vec2::new(0.0, 24.0)),
                )
                .on_hover_text("Ctrl+D")
                .clicked()
            {
                tool.duplicate(world, Vec3::new(0.5, 0.5, 0.0), true);
            }
            if icon_button(
                ui,
                p,
                "link",
                aurora_i18n::tr!("build-compact-link"),
                !tool.hud_selected(world) && !s.edit_linked && tool.roots(world).len() >= 2,
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
                !tool.hud_selected(world) && selected,
            )
            .clicked()
            {
                tool.unlink(world);
            }
            let icon = if s.expanded { "caret-up-filled" } else { "caret-down-filled" };
            if icon_button(ui, p, icon, aurora_i18n::tr!("build-compact-collapse"), true).clicked() {
                s.expanded = !s.expanded;
            }
        })
    });
}

pub fn body(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods, env: &mut Env) {
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    if tool.replacement.draft.is_some() || tool.replacement.job.is_some() {
        band(ui, egui::Color32::TRANSPARENT, 12, 12, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 5.0);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            replace::panel(ui, p, tool, world);
        });
        return;
    }
    selection(ui, p, tool, world, s);
    divider(ui, p);
    band(ui, egui::Color32::TRANSPARENT, 10, 7, |ui| tools::tool_row(ui, p, tool));
    divider(ui, p);
    let height = (ui.ctx().content_rect().height() - 250.0).clamp(190.0, 354.0);
    if s.expanded || tool.tool != Tool::Edit {
        if tool.tool == Tool::Edit {
            sections_panel(ui, p, tool, world, s, mods, env, height);
        } else {
            band(ui, egui::Color32::TRANSPARENT, 12, 12, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 5.0);
                egui::ScrollArea::vertical()
                    .id_salt("c3_tool")
                    .max_height(height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| match tool.tool {
                        Tool::Focus => tools::focus_panel(ui, p, tool, mods, env),
                        Tool::Grab => tools::grab_panel(ui, p, tool, mods),
                        Tool::Create => tools::create_panel(ui, p, s),
                        Tool::Land => {
                            tools::land_panel(ui, p, tool, world, s);
                            if s.expanded {
                                ui.separator();
                                tools::parcel_panel(ui, p, tool, world, s);
                            }
                        }
                        Tool::Edit => {}
                    });
            });
        }
    }
    divider(ui, p);
    footer(ui, p, tool, world, s, mods);
    band(ui, p.bar.gamma_multiply(0.82), 10, 5, |ui| {
        let hint = if tool.status.is_empty() {
            tools::status_text(tool, s, mods)
        } else {
            &tool.status
        };
        ui.add(
            egui::Label::new(
                RichText::new(hint)
                    .size(10.0)
                    .color(if tool.status.is_empty() { p.muted } else { p.warn }),
            )
            .truncate(),
        )
        .on_hover_text(hint);
    });
}

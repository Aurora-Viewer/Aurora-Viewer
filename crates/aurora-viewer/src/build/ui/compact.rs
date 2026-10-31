//! Compact Aurora build workspace. Five sections reorganize the
//! LLFloaterTools / LLPanelObject controls without changing their commands.

use super::{Env, common, contents, features, general, layout, object, replace, texture, tools};
use crate::build::{BuildSettings, BuildTool, EditMode, Mods, Tool};
use crate::theme::Palette;
use crate::ui::icons;
use crate::world::World;
use egui::{RichText, Stroke, Vec2};
use glam::Vec3;

pub(super) fn sections() -> [(usize, &'static str, &'static str); 5] {
    [
        (6, "info-circle-filled", aurora_i18n::tr!("build-compact-general")),
        (5, "box", aurora_i18n::tr!("build-compact-shape")),
        (3, "paint-filled", aurora_i18n::tr!("build-compact-materials")),
        (2, "adjustments-filled", aurora_i18n::tr!("build-compact-features")),
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
        let glyph_rect = egui::Rect::from_center_size(rect.center(), Vec2::splat(14.0));
        #[cfg(test)]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::unique(("compact_icon_glyph", name)), glyph_rect));
        glyph(
            ui,
            glyph_rect,
            name,
            if !enabled {
                p.muted_dim
            } else if response.hovered() {
                p.ink
            } else {
                p.muted
            },
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, title));
        response.on_hover_text(title)
    })
    .inner
}

pub fn heading(ui: &mut egui::Ui, p: &Palette, icon: &str, title: &str, actions: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 9.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(17.0), egui::Sense::hover());
        glyph(ui, rect, icon, p.violet_light);
        ui.label(RichText::new(title).size(common::TITLE).strong().color(p.ink));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            actions(ui);
        });
    });
    ui.add_space(10.0);
}

pub(super) fn selection(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &World, s: &BuildSettings) {
    let roots = tool.roots(world);
    let prims = tool.sel_prims(world);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(5.0, 4.0);
        ui.spacing_mut().interact_size.y = 18.0;
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
                            .size(common::TITLE)
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
                        .size(common::TITLE)
                        .color(p.muted),
                );
            });
        });
        ui.horizontal(|ui| {
            if let Some((_, remaining)) = tool.costs.capacity(world) {
                ui.label(
                    RichText::new(aurora_i18n::tr!("build-c3-remaining", remaining = remaining))
                        .size(common::TITLE)
                        .color(p.muted),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                replace::button(ui, p, tool, world, s);
            });
        });
    });
    crate::ui::widgets::divider(ui, p);
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
                let width = ((ui.max_rect().width() - 12.0) / 4.0).max(24.0);
                let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 26.0), egui::Sense::click());
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
        });
    });
}

pub(super) fn rail_button(ui: &mut egui::Ui, p: &Palette, icon: &str, title: &str, active: bool, height: f32) -> bool {
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
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), active, title));
    ui.is_enabled() && response.on_hover_text(title).clicked()
}

fn grid_mode_button(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &World, s: &mut BuildSettings, mode: u8) {
    let (icon, label) = match mode {
        0 => ("world-filled", aurora_i18n::tr!("build-compact-world")),
        1 => ("box", aurora_i18n::tr!("build-compact-local")),
        _ => ("focus-centered", aurora_i18n::tr!("build-compact-reference")),
    };
    let active = s.grid_mode == mode;
    let kind = if active {
        crate::ui::widgets::ActionKind::Primary
    } else {
        crate::ui::widgets::ActionKind::Flat
    };
    let response = crate::ui::widgets::action_button(ui, p, "", "", kind, Vec2::new(26.0, 26.0));
    #[cfg(test)]
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::unique(("build_grid_mode", mode)), response.rect));
    glyph(
        ui,
        egui::Rect::from_center_size(response.rect.center(), Vec2::splat(18.0)),
        icon,
        if active { p.on_accent } else { p.muted },
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, active, label));
    if response.on_hover_text(label).clicked() {
        s.grid_mode = mode;
        if mode == 2 && tool.grid_objects.is_empty() {
            tool.use_selection_for_grid(world, s);
        }
    }
}

pub(super) fn action_row(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &BuildSettings) {
    let selected = !tool.selection.is_empty();
    for (redo, icon, title) in [
        (false, "arrow-back-up", aurora_i18n::tr!("build-compact-undo")),
        (true, "arrow-forward-up", aurora_i18n::tr!("build-compact-redo")),
    ] {
        if icon_button(ui, p, icon, title, selected).clicked() {
            tool.undo(world, redo);
        }
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
    if icon_button(
        ui,
        p,
        "copy-filled",
        aurora_i18n::tr!("build-compact-duplicate"),
        selected && !tool.attachment_selected(world),
    )
    .on_hover_text(crate::keybinds::chord_label("KeyD", true, false, false))
    .clicked()
    {
        tool.duplicate(world, Vec3::new(0.5, 0.5, 0.0), true);
    }
}

pub(super) fn footer(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods) {
    layout::panel(ui, p, tool, world, s, mods, crate::build::layout::Panel::Footer);
}

fn navigation_height(s: &BuildSettings) -> f32 {
    // Each navigation row is 34 points with 3 points of spacing, inside a
    // five-point band. Hidden icons release their space immediately.
    let count = s.layout.in_panel(crate::build::layout::Panel::Navigation).len();
    (count as f32 * 37.0 - 3.0).max(0.0) + 10.0
}

fn shared_heights(tool: &BuildTool, s: &BuildSettings) -> (f32, f32) {
    use crate::build::layout::{Item, Panel};
    if !matches!(tool.tool, Tool::Edit | Tool::Create) {
        return (0.0, 0.0);
    }
    let footer = s
        .layout
        .in_panel(Panel::Footer)
        .iter()
        .map(|item| {
            if matches!(item, Item::Manipulators | Item::Actions) {
                31.0
            } else {
                27.0
            }
        })
        .sum::<f32>()
        + 12.0;
    (footer.max(42.0), if s.transform_expanded { 190.0 } else { 34.0 })
}

pub(super) fn minimum_size(tool: &BuildTool, s: &BuildSettings) -> Vec2 {
    let (footer, values) = shared_heights(tool, s);
    // Reserve the framed title/status and outer margins as well as the rail.
    // Short screens may constrain the floater further; body then gives the
    // navigation priority over the independently scrollable editing panels.
    super::workspace::MIN_SIZE.max(Vec2::new(0.0, navigation_height(s) + footer + values + 80.0))
}

fn pane(ui: &mut egui::Ui, rect: egui::Rect, salt: &str) -> egui::Ui {
    let mut pane = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(salt)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    pane.set_clip_rect(ui.clip_rect().intersect(rect));
    pane.spacing_mut().item_spacing = egui::vec2(4.0, 3.0);
    pane.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
    pane
}

fn transform(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods) {
    if !s.transform_expanded {
        ui.horizontal(|ui| {
            if icon_button(ui, p, "caret-right-filled", aurora_i18n::tr!("build-layout-transform"), true).clicked() {
                s.transform_expanded = true;
            }
            let width = (ui.available_width() - 116.0).max(1.0);
            if ui
                .add_sized(
                    [width, 26.0],
                    egui::Label::new(
                        RichText::new(aurora_i18n::tr!("build-layout-transform"))
                            .size(common::BODY)
                            .color(p.muted),
                    )
                    .truncate()
                    .sense(egui::Sense::click()),
                )
                .clicked()
            {
                s.transform_expanded = true;
            }
            for mode in 0..3 {
                grid_mode_button(ui, p, tool, world, s, mode);
            }
            layout::gear(ui, p, tool);
        });
        return;
    }
    let rect = ui.available_rect_before_wrap();
    let split = rect.right() - 34.0;
    let mut values = pane(
        ui,
        egui::Rect::from_min_max(rect.min, egui::pos2(split - 5.0, rect.bottom())),
        "shared-values",
    );
    egui::ScrollArea::vertical()
        .id_salt("shared-values-scroll")
        .max_height(rect.height())
        .auto_shrink([false, false])
        .show(&mut values, |ui| {
            object::show_transform(ui, p, tool, world, s, mods);
        });
    let mut strip = pane(
        ui,
        egui::Rect::from_min_max(egui::pos2(split, rect.top()), rect.max),
        "reference-strip",
    );
    for mode in 0..3 {
        grid_mode_button(&mut strip, p, tool, world, s, mode);
    }
    strip.add_space((rect.height() - 5.0 * 29.0).max(0.0));
    layout::gear(&mut strip, p, tool);
    if icon_button(&mut strip, p, "caret-down-filled", aurora_i18n::tr!("build-layout-transform"), true).clicked() {
        s.transform_expanded = false;
    }
}

pub fn body(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods, env: &mut Env) {
    use crate::build::layout::Panel;
    if !s.expanded {
        return;
    }
    if tool.replacement.draft.is_some() || tool.replacement.job.is_some() {
        replace::panel(ui, p, tool, world);
        return;
    }
    let size = ui.available_size().max(egui::vec2(1.0, 1.0));
    let rect = ui.allocate_exact_size(size, egui::Sense::hover()).0;
    let shared = matches!(tool.tool, Tool::Edit | Tool::Create);
    let (desired_footer, desired_values) = shared_heights(tool, s);
    let remaining = (size.y - navigation_height(s)).max(0.0);
    let ratio = (remaining / (desired_footer + desired_values).max(1.0)).min(1.0);
    let footer_height = desired_footer * ratio;
    let values_height = desired_values * ratio;
    let primary_bottom = rect.bottom() - footer_height - values_height;
    let mut rail = pane(
        ui,
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + 44.0, primary_bottom)),
        "construction-rail",
    );
    #[cfg(test)]
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::unique("build_navigation_viewport"), rail.clip_rect()));
    band(&mut rail, p.bar.gamma_multiply(0.82), 5, 5, |ui| {
        layout::panel(ui, p, tool, world, s, mods, Panel::Navigation);
    });
    let mut main = pane(
        ui,
        egui::Rect::from_min_max(egui::pos2(rect.left() + 44.0, rect.top()), egui::pos2(rect.right(), primary_bottom)),
        "construction-primary",
    );
    band(&mut main, egui::Color32::TRANSPARENT, 10, 8, |ui| {
        // Lay the content out at the width it keeps once the bar shows: the
        // Create shape grid (and any wrapped text) grows taller as it widens,
        // so a width that follows the bar would show it, narrow, fit, hide it
        // and widen again on every frame (#1092).
        let width = crate::ui::widgets::scrolled_width(ui).max(1.0);
        egui::ScrollArea::vertical()
            .id_salt(("c3_section", tool.tool as u8, tool.tab))
            .max_height((primary_bottom - rect.top() - 16.0).max(1.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(width);
                if tool.tool == Tool::Edit {
                    selection(ui, p, tool, world, s);
                    if !tool.selection.is_empty() || tool.tab == 3 {
                        section(ui, p, tool, world, s, mods, env);
                    }
                } else {
                    match tool.tool {
                        Tool::Focus => tools::focus_panel(ui, p, tool, mods, env),
                        Tool::Grab => tools::grab_panel(ui, p, tool, mods),
                        Tool::Create => tools::create_panel(ui, p, s),
                        Tool::Land => {
                            tools::land_panel(ui, p, tool, world, s);
                            tools::parcel_panel(ui, p, tool, world, s, env.property_lines);
                        }
                        Tool::Edit => {}
                    }
                    common::small(ui, p, tools::status_text(tool, s, mods));
                }
            });
    });
    if shared {
        let values_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left(), primary_bottom),
            egui::pos2(rect.right(), rect.bottom() - footer_height),
        );
        let mut values = pane(ui, values_rect.shrink2(egui::vec2(8.0, 4.0)), "construction-transform");
        transform(&mut values, p, tool, world, s, mods);
        let mut footer_ui = pane(
            ui,
            egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - footer_height), rect.max).shrink2(egui::vec2(8.0, 5.0)),
            "construction-footer",
        );
        egui::ScrollArea::vertical()
            .id_salt("construction-footer-scroll")
            .max_height((footer_height - 10.0).max(1.0))
            .auto_shrink([false, false])
            .show(&mut footer_ui, |ui| {
                footer(ui, p, tool, world, s, mods);
                if !tool.status.is_empty() {
                    common::small(ui, p, &tool.status);
                }
            });
        ui.painter()
            .hline(rect.x_range(), primary_bottom, crate::ui::widgets::divider_stroke(p));
        ui.painter()
            .hline(rect.x_range(), rect.bottom() - footer_height, crate::ui::widgets::divider_stroke(p));
    }
}

/// One rendering path for all presentations; permission checks, drafts and
/// simulator commands stay with the original controls.
pub(super) fn section(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    env: &mut Env,
) {
    match tool.tab {
        0 | 1 | 6 => general::show(ui, p, tool, world, false),
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
        }
    }
}

#[cfg(test)]
#[path = "compact_tests.rs"]
mod tests;

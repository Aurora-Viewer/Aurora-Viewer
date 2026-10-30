//! Reference construction geometry from LLFloaterTools / floater_tools.xml.
//! The top block follows the 295-point reference canvas. Object keeps flags
//! and vertical XYZ vectors on the left, shape parameters on the right.
use super::{Env, common, contents, features, general, object, texture, tools};
use crate::build::{BuildSettings, BuildTool, EditMode, Mods, Tool};
use crate::theme::Palette;
use crate::ui::widgets;
use crate::world::World;
use egui::{Rect, RichText, Vec2, vec2};

pub(super) const DEFAULT_SIZE: Vec2 = vec2(360.0, 780.0);
pub(super) const MIN_SIZE: Vec2 = vec2(330.0, 620.0);
const TAG: &str = "legacy-construction-layout";

pub(super) fn active(ui: &egui::Ui) -> bool {
    ui.stack().iter().any(|node| node.tags().contains(TAG))
}

/// A bounded child prevents a translated label from changing the reference
/// column widths. Normal scrolling belongs to the complete property panel.
pub(super) fn pane(ui: &mut egui::Ui, rect: Rect, id: &str) -> egui::Ui {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(ui.clip_rect().intersect(rect));
    child.spacing_mut().item_spacing = vec2(3.0, 1.0);
    child.spacing_mut().interact_size.y = 19.0;
    child.spacing_mut().button_padding = vec2(3.0, 1.0);
    child.spacing_mut().icon_width = 12.0;
    child.spacing_mut().icon_spacing = 3.0;
    child.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
    child
}

fn slot(ui: &mut egui::Ui, canvas: Rect, xy: [f32; 4], id: &str) -> egui::Ui {
    let scale = canvas.width() / 295.0;
    pane(
        ui,
        Rect::from_min_size(canvas.min + vec2(xy[0], xy[1]) * scale, vec2(xy[2], xy[3]) * scale),
        id,
    )
}

fn tab_index(tab: usize) -> usize {
    match tab {
        1 | 5 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        _ => 0,
    }
}

fn toolbar(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, canvas: Rect) {
    for (index, (mode, icon, label)) in [
        (Tool::Focus, "zoom-filled", aurora_i18n::tr!("build-compact-camera")),
        (Tool::Grab, "hand-grab", aurora_i18n::tr!("build-compact-grab")),
        (Tool::Edit, "pointer-filled", aurora_i18n::tr!("common-edit")),
        (Tool::Create, "wand", aurora_i18n::tr!("build-compact-create")),
        (Tool::Land, "mountain-filled", aurora_i18n::tr!("build-compact-land")),
    ]
    .into_iter()
    .enumerate()
    {
        let mut cell = slot(
            ui,
            canvas,
            [10.0 + 45.0 * index as f32, 0.0, 35.0, 25.0],
            &format!("legacy-tool-{index}"),
        );
        let kind = if tool.tool == mode {
            widgets::ActionKind::Primary
        } else {
            widgets::ActionKind::Flat
        };
        let size = cell.max_rect().size();
        let response = widgets::action_button(&mut cell, p, icon, "", kind, size).on_hover_text(label);
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, tool.tool == mode, label));
        #[cfg(test)]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::unique(("legacy-build-tool", mode as u8)), response.rect));
        if response.clicked() {
            tool.set_tool(mode);
        }
    }
}

fn edit_options(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods, canvas: Rect) {
    let mut radios = slot(ui, canvas, [5.0, 51.0, 132.0, 82.0], "legacy-edit-radios");
    radios.spacing_mut().interact_size.y = 16.0;
    for (mode, label) in [
        (EditMode::Move, aurora_i18n::tr!("build-ui-tools-move")),
        (EditMode::Rotate, aurora_i18n::tr!("build-legacy-rotate")),
        (EditMode::Stretch, aurora_i18n::tr!("build-compact-stretch")),
        (EditMode::Face, aurora_i18n::tr!("build-compact-select-face")),
        (EditMode::Align, aurora_i18n::tr!("common-align")),
    ] {
        if widgets::radio(&mut radios, p, tool.effective_mode(mods) == mode, RichText::new(label).size(11.0)).clicked() {
            tool.set_edit_mode(world, s, mode);
        }
    }
    let mut options = slot(ui, canvas, [143.0, 51.0, 134.0, 96.0], "legacy-edit-checks");
    options.spacing_mut().interact_size.y = 16.0;
    for (value, label, id) in [
        (s.scale_uniform, aurora_i18n::tr!("build-compact-both-sides"), 0),
        (s.scale_stretch_textures, aurora_i18n::tr!("build-compact-stretch-textures"), 1),
        (s.snap, aurora_i18n::tr!("build-compact-snap"), 2),
        (s.actual_root, aurora_i18n::tr!("build-compact-root-axis"), 3),
        (s.show_highlight, aurora_i18n::tr!("build-compact-highlight"), 4),
        (s.select_probes, aurora_i18n::tr!("build-legacy-probes"), 5),
    ] {
        let mut value = value;
        let response = widgets::checkbox(&mut options, p, &mut value, RichText::new(label).size(11.0)).on_hover_text(label);
        if response.changed() {
            match id {
                0 => s.scale_uniform = value,
                1 => s.scale_stretch_textures = value,
                2 => s.snap = value,
                3 => s.actual_root = value,
                4 => s.show_highlight = value,
                _ => s.select_probes = value,
            }
        }
    }
    let mut linked = slot(ui, canvas, [5.0, 135.0, 132.0, 18.0], "legacy-linked");
    let mut value = s.edit_linked;
    if widgets::checkbox(
        &mut linked,
        p,
        &mut value,
        RichText::new(aurora_i18n::tr!("build-compact-linked")).size(11.0),
    )
    .changed()
    {
        tool.set_edit_linked(world, s, value);
    }
    let mut actions = slot(ui, canvas, [5.0, 156.0, 285.0, 21.0], "legacy-links-ruler");
    actions.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (next, icon, label) in [
            (false, "caret-left-filled", aurora_i18n::tr!("build-compact-previous")),
            (true, "caret-right-filled", aurora_i18n::tr!("build-compact-next")),
        ] {
            let r = widgets::action_button(ui, p, icon, "", widgets::ActionKind::Flat, vec2(15.0, 20.0)).on_hover_text(label);
            if r.clicked() {
                tool.select_next_part(world, s, next, mods.shift);
            }
        }
        if ui
            .add_enabled(
                !tool.attachment_selected(world) && !s.edit_linked && tool.roots(world).len() >= 2,
                egui::Button::new(aurora_i18n::tr!("build-legacy-link")).min_size(vec2(50.0 * canvas.width() / 295.0, 20.0)),
            )
            .on_hover_text(aurora_i18n::tr!("build-compact-link"))
            .clicked()
        {
            tool.link(world);
        }
        if ui
            .add_enabled(
                !tool.attachment_selected(world) && !tool.selection.is_empty(),
                egui::Button::new(aurora_i18n::tr!("build-legacy-unlink")).min_size(vec2(90.0 * canvas.width() / 295.0, 20.0)),
            )
            .on_hover_text(aurora_i18n::tr!("build-compact-unlink"))
            .clicked()
        {
            tool.unlink(world);
        }
        let modes = [
            (aurora_i18n::tr!("build-compact-world"), 0),
            (aurora_i18n::tr!("build-compact-local"), 1),
            (aurora_i18n::tr!("build-compact-reference"), 2),
        ];
        if let Some(mode) = common::combo(
            ui,
            "legacy-grid-mode",
            Some(s.grid_mode),
            &modes,
            ui.available_width().max(60.0),
            true,
        ) {
            s.grid_mode = mode;
            if mode == 2 && tool.grid_objects.is_empty() {
                tool.use_selection_for_grid(world, s);
            }
        }
    });
}

fn selection(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &World) {
    if tool.selection.is_empty() {
        common::small(ui, p, aurora_i18n::tr!("build-compact-select-hint"));
        return;
    }
    let impact = tool
        .land_impact(world)
        .map(|v| aurora_i18n::integer(v.round() as i64))
        .unwrap_or_else(|| "…".into());
    let counts = aurora_i18n::tr!(
        "build-compact-counts",
        objects = tool.roots(world).len(),
        prims = tool.sel_prims(world).len()
    );
    let label = format!("{} · {}", counts, aurora_i18n::tr!("build-compact-impact", impact = impact));
    if ui
        .add(
            egui::Label::new(RichText::new(label).size(11.0).color(p.ink))
                .truncate()
                .sense(egui::Sense::click()),
        )
        .clicked()
    {
        tool.ui.weights_open = !tool.ui.weights_open;
    }
    if let Some((_, remaining)) = tool.costs.capacity(world) {
        common::small(ui, p, &aurora_i18n::tr!("build-c3-remaining", remaining = remaining));
    }
}

fn object_panel(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods, env: &mut Env) {
    let width = widgets::scrolled_width(ui).max(1.0);
    let (canvas, _) = ui.allocate_exact_size(vec2(width, 430.0), egui::Sense::hover());
    let mut left = slot(ui, canvas, [10.0, 5.0, 106.0, 1000.0], "legacy-object-vectors");
    object::show_flags(&mut left, p, tool, world);
    left.add_space(8.0);
    object::show_transform(&mut left, p, tool, world, s, mods);
    let mut right = slot(ui, canvas, [125.0, 5.0, 160.0, 1000.0], "legacy-object-shape");
    object::show_shape(&mut right, p, tool, world, env);
    ui.advance_cursor_after_rect(left.min_rect().union(right.min_rect()));
}

pub(super) fn body(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    env: &mut Env,
) -> bool {
    ui.scope_builder(
        egui::UiBuilder::new().ui_stack_info(egui::UiStackInfo::default().with_tag(TAG)),
        |ui| {
            ui.spacing_mut().item_spacing = vec2(3.0, 1.0);
            ui.spacing_mut().interact_size.y = 19.0;
            ui.spacing_mut().button_padding = vec2(3.0, 1.0);
            let scale = ui.available_width() / 295.0;
            let (canvas, _) = ui.allocate_exact_size(
                vec2(ui.available_width(), (if s.expanded { 211.0 } else { 178.0 }) * scale),
                egui::Sense::hover(),
            );
            toolbar(ui, p, tool, canvas);
            let mut status = slot(ui, canvas, [8.0, 28.0, 280.0, 24.0], "legacy-tool-status");
            status.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            common::small(&mut status, p, tools::status_text(tool, s, mods));
            match tool.tool {
                Tool::Edit => edit_options(ui, p, tool, world, s, mods, canvas),
                mode => {
                    let mut options = slot(ui, canvas, [10.0, 54.0, 275.0, 122.0], "legacy-tool-options");
                    options.spacing_mut().interact_size.y = 16.0;
                    if mode == Tool::Land {
                        options.spacing_mut().slider_width = 80.0 * scale;
                    }
                    match mode {
                        Tool::Focus => tools::focus_panel(&mut options, p, tool, mods, env),
                        Tool::Grab => tools::grab_panel(&mut options, p, tool, mods),
                        Tool::Create => tools::create_legacy_panel(&mut options, p, s),
                        Tool::Land => tools::land_panel(&mut options, p, tool, world, s),
                        Tool::Edit => unreachable!(),
                    }
                }
            }
            let mut gear = slot(ui, canvas, [277.0, 77.0, 18.0, 23.0], "legacy-options-button");
            if widgets::action_button(
                &mut gear,
                p,
                "settings-filled",
                "",
                widgets::ActionKind::Flat,
                vec2(18.0 * scale, 23.0 * scale),
            )
            .on_hover_text(aurora_i18n::tr!("build-layout-settings"))
            .clicked()
            {
                tool.ui.layout.open = !tool.ui.layout.open;
            }
            if s.expanded {
                let mut summary = slot(ui, canvas, [10.0, 181.0, 275.0, 30.0], "legacy-selection");
                selection(&mut summary, p, tool, world);
                if tool.tool != Tool::Land {
                    let mut tab = tab_index(tool.tab);
                    if widgets::tabs(
                        ui,
                        p,
                        &mut tab,
                        &[
                            (aurora_i18n::tr!("build-compact-general"), true),
                            (aurora_i18n::tr!("build-legacy-object"), true),
                            (aurora_i18n::tr!("build-compact-features"), true),
                            (aurora_i18n::tr!("build-legacy-texture"), true),
                            (aurora_i18n::tr!("build-compact-contents"), true),
                        ],
                    ) {
                        tool.tab = [6, 5, 2, 3, 4][tab];
                    }
                }
                let height = (ui.available_height() - 17.0).max(30.0);
                let width = widgets::scrolled_width(ui).max(1.0);
                egui::ScrollArea::vertical()
                    .id_salt(("legacy-build-body", tool.tool as u8, tool.tab))
                    .max_height(height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(width);
                        if tool.tool == Tool::Land {
                            tools::parcel_panel(ui, p, tool, world, s, env.property_lines);
                        } else {
                            ui.push_id(("legacy-build-panel", tab_index(tool.tab)), |ui| match tab_index(tool.tab) {
                                0 => {
                                    egui::Frame::new()
                                        .inner_margin(egui::Margin::same(8))
                                        .show(ui, |ui| general::show(ui, p, tool, world, false));
                                }
                                1 => object_panel(ui, p, tool, world, s, mods, env),
                                2 => {
                                    egui::Frame::new()
                                        .inner_margin(egui::Margin::same(8))
                                        .show(ui, |ui| features::show(ui, p, tool, world, s, env));
                                }
                                3 => texture::show(ui, p, tool, world, s, env),
                                4 => {
                                    egui::Frame::new()
                                        .inner_margin(egui::Margin::same(8))
                                        .show(ui, |ui| contents::show(ui, p, tool, world));
                                }
                                _ => unreachable!(),
                            });
                        }
                    });
            }
            let icon = if s.expanded { "caret-up-filled" } else { "caret-down-filled" };
            let response = widgets::action_button(ui, p, icon, "", widgets::ActionKind::Flat, vec2(ui.available_width(), 14.0))
                .on_hover_text(aurora_i18n::tr!("build-compact-collapse"));
            #[cfg(test)]
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::unique("legacy-build-collapse"), response.rect));
            response.clicked()
        },
    )
    .inner
}

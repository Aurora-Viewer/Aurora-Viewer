//! Compact Aurora build workspace. Five sections reorganize the
//! LLFloaterTools / LLPanelObject controls without changing their commands.

use super::{Env, common, contents, detach, features, general, layout, object, replace, texture, tools};
use crate::build::{BuildSettings, BuildTool, EditMode, Mods, Tool};
use crate::theme::Palette;
use crate::ui::icons;
use crate::world::World;
use egui::{RichText, Stroke, Vec2};
use glam::Vec3;

pub(super) const ACTION_GAP: f32 = 4.0;

pub(super) fn window_heading(tool: &BuildTool) -> (&'static str, &'static str) {
    if tool.tool == Tool::Edit {
        ("box", aurora_i18n::tr!("build-workspace-title"))
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
    icon_button_sized(ui, p, name, title, enabled, Vec2::new(22.0, 24.0))
}

/// A larger icon button, for an action that has a cell of its own.
pub fn icon_button_large(ui: &mut egui::Ui, p: &Palette, name: &str, title: &str, enabled: bool) -> egui::Response {
    icon_button_glyph(ui, p, name, title, enabled, Vec2::splat(34.0), 22.0)
}

pub(super) fn icon_button_sized(ui: &mut egui::Ui, p: &Palette, name: &str, title: &str, enabled: bool, size: Vec2) -> egui::Response {
    icon_button_glyph(ui, p, name, title, enabled, size, 14.0)
}

fn icon_button_glyph(
    ui: &mut egui::Ui,
    p: &Palette,
    name: &str,
    title: &str,
    enabled: bool,
    size: Vec2,
    glyph_size: f32,
) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        if response.hovered() && enabled {
            ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
        }
        let glyph_rect = egui::Rect::from_center_size(rect.center(), Vec2::splat(glyph_size));
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

pub(super) fn selection_summary(p: &Palette, tool: &BuildTool, world: &World) -> egui::text::LayoutJob {
    let roots = tool.roots(world);
    let prims = tool.sel_prims(world);
    if prims.is_empty() {
        return egui::text::LayoutJob::simple_singleline(
            aurora_i18n::tr!("build-compact-select-hint").to_owned(),
            egui::FontId::proportional(common::BODY),
            p.muted,
        );
    }
    let impact = tool
        .land_impact(world)
        .map(|v| aurora_i18n::integer(v.round() as i64))
        .unwrap_or_else(|| "…".into());
    let impact = aurora_i18n::tr!("build-compact-impact", impact = impact);
    let capacity = if let Some((_, remaining)) = tool.costs.capacity(world) {
        aurora_i18n::tr!("build-c3-remaining", remaining = remaining)
    } else {
        aurora_i18n::tr!("build-c3-remaining", remaining = "…")
    };
    let counts = aurora_i18n::tr!("build-compact-counts", objects = roots.len(), prims = prims.len());
    let details = aurora_i18n::tr!("build-compact-summary-details", capacity = capacity, counts = counts);
    let mut summary = egui::text::LayoutJob::default();
    for (text, color) in [(&impact, p.violet_light), (&details, p.muted)] {
        summary.append(
            text,
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::proportional(common::BODY),
                color,
                ..Default::default()
            },
        );
    }
    summary
}

pub(super) fn manipulation(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for mode in [EditMode::Move, EditMode::Rotate, EditMode::Stretch, EditMode::Align] {
            let width = ((ui.max_rect().width() - 12.0) / 4.0).max(24.0);
            manipulation_choice(
                ui,
                p,
                tool,
                world,
                s,
                mods,
                mode,
                Vec2::new(width, crate::ui::widgets::TEXT_BUTTON_HEIGHT),
            );
        }
    });
}

pub(super) fn manipulation_choice(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    mode: EditMode,
    size: Vec2,
) {
    let (title, caption) = match mode {
        EditMode::Move => (aurora_i18n::tr!("build-compact-move"), aurora_i18n::tr!("build-compact-mode-move")),
        EditMode::Rotate => (
            aurora_i18n::tr!("build-compact-rotate"),
            aurora_i18n::tr!("build-compact-mode-rotate"),
        ),
        EditMode::Stretch => (
            aurora_i18n::tr!("build-compact-stretch"),
            aurora_i18n::tr!("build-compact-mode-stretch"),
        ),
        EditMode::Align => (
            aurora_i18n::tr!("build-compact-align"),
            aurora_i18n::tr!("build-compact-mode-align"),
        ),
        EditMode::Face => return,
    };
    let mut active = tool.effective_mode(mods) == mode;
    let label = RichText::new(caption).size(common::BODY);
    let response = ui
        .allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
            crate::ui::widgets::checkbox(ui, p, &mut active, label)
        })
        .inner;
    // These retain the same checkbox face as the other options while choosing
    // one manipulation mode: clicking the current mode keeps it selected.
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, ui.is_enabled(), active, caption));
    if response.on_hover_text(title).clicked() {
        tool.set_edit_mode(world, s, mode);
        ui.ctx().request_repaint();
    }
}

/// The width a bar button would like: its icon, its label or both.
pub(super) fn navigation_button_width(ui: &egui::Ui, label: &str, display: crate::build::layout::ButtonDisplay) -> f32 {
    use crate::build::layout::ButtonDisplay;
    let text = || {
        ui.painter()
            .layout_no_wrap(
                label.to_owned(),
                egui::FontId::proportional(common::BODY),
                egui::Color32::PLACEHOLDER,
            )
            .size()
            .x
    };
    match display {
        ButtonDisplay::Icon => 34.0,
        ButtonDisplay::Label => text() + 18.0,
        ButtonDisplay::Both => text() + 18.0 + NAV_GLYPH + NAV_GLYPH_GAP,
    }
}

const NAV_GLYPH: f32 = 16.0;
const NAV_GLYPH_GAP: f32 = 5.0;

/// A rail button showing its icon, its `label` or both (`display`), with
/// `tip` on hover; `underline_only` marks it lit by its underline alone (a
/// section shown in a window of its own), the active tab being also tinted;
/// `half` lights it at half (the second section, second_tab.rs).
#[allow(clippy::too_many_arguments)]
pub(super) fn navigation_button_marked(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    label: &str,
    title: &str,
    display: crate::build::layout::ButtonDisplay,
    active: bool,
    underline_only: bool,
    half: bool,
    height: f32,
) -> egui::Response {
    use crate::build::layout::ButtonDisplay;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), egui::Sense::click());
    let tinted = active && !underline_only;
    let strength = if half { 0.5 } else { 1.0 };
    if tinted && response.hovered() && half {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    }
    if tinted || response.hovered() {
        ui.painter().rect_filled(
            rect,
            ui.visuals().window_corner_radius,
            if tinted {
                p.violet.gamma_multiply(0.3 * strength)
            } else {
                p.raised
            },
        );
    }
    if active {
        ui.painter()
            .hline(rect.x_range(), rect.bottom(), Stroke::new(2.0, p.violet.gamma_multiply(strength)));
    }
    let color = if tinted && !half {
        p.violet_light
    } else if tinted {
        p.violet_light.lerp_to_gamma(p.muted, 0.5)
    } else {
        p.muted
    };
    if display == ButtonDisplay::Icon {
        glyph(ui, egui::Rect::from_center_size(rect.center(), Vec2::splat(18.0)), icon, color);
    } else {
        // the label (after the icon) centred, cut at the button's edges
        // when the bar is too narrow for every label
        let painter = ui
            .painter()
            .with_clip_rect(rect.shrink2(egui::vec2(3.0, 0.0)).intersect(ui.clip_rect()));
        let galley = painter.layout_no_wrap(label.to_owned(), egui::FontId::proportional(common::BODY), color);
        let lead = if display == ButtonDisplay::Both {
            NAV_GLYPH + NAV_GLYPH_GAP
        } else {
            0.0
        };
        let width = lead + galley.size().x;
        let left = if width + 8.0 <= rect.width() {
            rect.center().x - width / 2.0
        } else {
            rect.left() + 4.0
        };
        if display == ButtonDisplay::Both {
            glyph(
                ui,
                egui::Rect::from_center_size(egui::pos2(left + NAV_GLYPH / 2.0, rect.center().y), Vec2::splat(NAV_GLYPH)),
                icon,
                color,
            );
        }
        painter.galley(egui::pos2(left + lead, rect.center().y - galley.size().y / 2.0), galley, color);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), active, title));
    response.on_hover_text(title)
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
        if icon_button_sized(ui, p, icon, title, selected, Vec2::splat(crate::ui::widgets::TEXT_BUTTON_HEIGHT)).clicked() {
            tool.undo(world, redo);
        }
    }
    let action_width = ((ui.available_width() - ui.spacing().item_spacing.x * 2.0) / 3.0).max(1.0);
    if crate::ui::widgets::action_button_if(
        ui,
        p,
        "",
        aurora_i18n::tr!("build-compact-link-action"),
        crate::ui::widgets::ActionKind::Flat,
        Vec2::new(action_width, crate::ui::widgets::TEXT_BUTTON_HEIGHT),
        !s.edit_linked && tool.can_link(world),
    )
    .on_hover_text(aurora_i18n::tr!("build-compact-link"))
    .clicked()
    {
        tool.link(world);
    }
    if crate::ui::widgets::action_button_if(
        ui,
        p,
        "",
        aurora_i18n::tr!("build-compact-unlink-action"),
        crate::ui::widgets::ActionKind::Flat,
        Vec2::new(action_width, crate::ui::widgets::TEXT_BUTTON_HEIGHT),
        selected && tool.can_unlink(world),
    )
    .on_hover_text(aurora_i18n::tr!("build-compact-unlink"))
    .clicked()
    {
        tool.unlink(world);
    }
    if crate::ui::widgets::action_button_if(
        ui,
        p,
        "",
        aurora_i18n::tr!("build-compact-duplicate"),
        crate::ui::widgets::ActionKind::Flat,
        Vec2::new(action_width, crate::ui::widgets::TEXT_BUTTON_HEIGHT),
        selected && !tool.copyable_roots(world).is_empty(),
    )
    .on_hover_text(crate::keybinds::chord_label("KeyD", true, false, false))
    .clicked()
    {
        tool.duplicate(world, s, Vec3::new(0.5, 0.5, 0.0), true);
    }
}

pub(super) fn footer(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods) {
    let start = ui.cursor().min.y;
    layout::panel(ui, p, tool, world, s, mods, crate::build::layout::Panel::Footer);
    ui.add_enabled_ui(!tool.ui.layout.organizing, |ui| {
        // the face selector, Invert and All, then Global / Local /
        // Reference; the link number is in "Edit linked parts" (the human's
        // trial, 2026-11-22)
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let buttons = 5.0 * (layout::FOOTER_BUTTON + ui.spacing().item_spacing.x);
            let face_width = (ui.available_width() - buttons).clamp(1.0, layout::FACE_SELECTOR);
            layout::face_navigation(ui, p, tool, world, s.edit_linked, mods, face_width);
            layout::face_actions(ui, p, tool, world, Vec2::splat(layout::FOOTER_BUTTON));
            // the references at the right edge, as before
            let references = 3.0 * layout::FOOTER_BUTTON + 3.0 * ui.spacing().item_spacing.x;
            ui.add_space((ui.available_width() - references).max(0.0));
            for mode in 0..3 {
                grid_mode_button(ui, p, tool, world, s, mode);
            }
        });
    });
    tool.ui.layout.footer_height = Some(ui.cursor().min.y - start + 12.0);
}

fn navigation_height(tool: &BuildTool, s: &BuildSettings, tools: bool) -> f32 {
    // One bar above the editor, one below all shared controls; an empty one
    // opens while a button is carried, to take it.
    use crate::build::layout::Panel;
    if layout::bar_drop_open(tool, s, if tools { Panel::Tools } else { Panel::Navigation }) {
        44.0
    } else {
        0.0
    }
}

fn shared_heights(tool: &BuildTool, s: &BuildSettings) -> (f32, f32) {
    use crate::build::layout::{FooterRow, Item};
    let shared = matches!(tool.tool, Tool::Edit | Tool::Create);
    let footer = tool.ui.layout.footer_height.unwrap_or_else(|| {
        // Count packed rows until measurement, so a saved window does not
        // grow on restore just because it contains two columns of options.
        s.layout
            .footer_rows()
            .iter()
            .map(|row| match row {
                FooterRow::Full(Item::Actions) => crate::ui::widgets::TEXT_BUTTON_HEIGHT + ACTION_GAP + 1.0,
                FooterRow::Full(_) => 31.0,
                FooterRow::Cells(_) => crate::ui::widgets::TEXT_BUTTON_HEIGHT + 1.0,
            })
            .sum::<f32>()
            + 27.0
            + 12.0
    });
    (
        // the options stay in every tool, so the bars under them do not move
        footer.max(42.0),
        if !shared {
            0.0
        } else if s.transform_expanded && !detach::is_detached(s, crate::build::layout::Block::Transform) {
            tool.ui.transform_height.unwrap_or(190.0)
        } else {
            34.0
        },
    )
}

pub(super) fn minimum_size(tool: &BuildTool, s: &BuildSettings) -> Vec2 {
    let (footer, values) = shared_heights(tool, s);
    // The selection summary is a single line inside the expanded header,
    // kept in every tool.
    let selection = 18.0;
    // Reserve the title/status, both navigation rows and shared controls.
    // The editing panels scroll independently on shorter screens.
    super::workspace::MIN_SIZE.max(Vec2::new(
        0.0,
        selection + navigation_height(tool, s, false) + navigation_height(tool, s, true) + footer + values + 80.0,
    ))
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
    use crate::build::layout::Block;
    if detach::is_detached(s, Block::Transform) {
        let shown = s.detached.get(&Block::Transform).copied().unwrap_or(false);
        let hint = detach::toggle_hint(Block::Transform, shown);
        ui.horizontal(|ui| {
            let mut clicked = crate::ui::widgets::icon_toggle(ui, p, "app-window-filled", &hint, shown);
            let width = ui.available_width().max(1.0);
            clicked |= ui
                .add_sized(
                    [width, 26.0],
                    egui::Label::new(
                        RichText::new(aurora_i18n::tr!("build-layout-transform"))
                            .size(common::BODY)
                            .color(if shown { p.ink } else { p.muted }),
                    )
                    .truncate()
                    .sense(egui::Sense::click()),
                )
                .on_hover_text(&hint)
                .clicked();
            if clicked {
                detach::toggle(tool, s, Block::Transform);
            }
        });
        return;
    }
    if !s.transform_expanded {
        ui.horizontal(|ui| {
            if icon_button(ui, p, "caret-right-filled", aurora_i18n::tr!("build-layout-transform"), true).clicked() {
                s.transform_expanded = true;
            }
            let width = ui.available_width().max(1.0);
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
        });
        return;
    }
    let rect = ui.available_rect_before_wrap();
    let mut values = pane(ui, rect, "shared-values");
    let top = values.cursor().min.y;
    // arranged by its width, as in a window of its own (the human's
    // request, 2026-11-23): the build window widened for two sections
    // gives it fewer lines
    let arrange = object::TransformArrange::for_width(values.available_width());
    object::show_transform_arranged(&mut values, p, tool, world, s, mods, arrange, true);
    if !tool.selection.is_empty() {
        tool.ui.transform_height = Some(values.cursor().min.y - top + 8.0);
    }
}

pub fn body(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods, env: &mut Env) {
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
    let (_, desired_values) = shared_heights(tool, s);
    let sections_height = navigation_height(tool, s, false);
    let tools_height = navigation_height(tool, s, true);
    let controls_bottom = rect.bottom() - tools_height;
    {
        // Measure the complete controls group at its natural height: columns,
        // actions and part/reference row. It never becomes a scroll area.
        // Shown with every tool, so the bars under it stay in place.
        let mut footer_ui = pane(
            ui,
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), controls_bottom)).shrink2(egui::vec2(8.0, 5.0)),
            "construction-footer",
        );
        footer(&mut footer_ui, p, tool, world, s, mods);
    }
    let footer_height = tool.ui.layout.footer_height.unwrap_or(42.0);
    let options_bottom = rect.top() + footer_height;
    let primary_top = options_bottom + sections_height;
    let sectionless = detach::sectionless(tool, s);
    let primary_bottom = if sectionless {
        primary_top
    } else {
        (controls_bottom - desired_values).max(primary_top.min(controls_bottom))
    };
    tool.ui.body_height = Some(rect.height());
    tool.ui.sectionless_body = Some(footer_height + sections_height + desired_values + tools_height);
    #[cfg(test)]
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            egui::Id::unique("build-controls-viewport"),
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), options_bottom)),
        )
    });
    if sections_height > 0.0 {
        let mut navigation = pane(
            ui,
            egui::Rect::from_min_max(egui::pos2(rect.left(), options_bottom), egui::pos2(rect.right(), primary_top)),
            "construction-sections",
        );
        #[cfg(test)]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::unique(("build_navigation_viewport", false)), navigation.clip_rect()));
        band(&mut navigation, p.bar.gamma_multiply(0.82), 5, 5, |ui| {
            layout::navigation(ui, p, tool, world, s, mods, false);
        });
    }
    let primary = egui::Rect::from_min_max(egui::pos2(rect.left(), primary_top), egui::pos2(rect.right(), primary_bottom));
    // a second section in the right half (second_tab.rs)
    let mut columns = vec![(tool.tab, primary)];
    if let Some(tab) = super::second_tab::shown(tool, s).filter(|_| !sectionless) {
        let middle = primary.center().x.round();
        columns = vec![
            (
                tool.tab,
                egui::Rect::from_min_max(primary.min, egui::pos2(middle, primary.bottom())),
            ),
            (tab, egui::Rect::from_min_max(egui::pos2(middle, primary.top()), primary.max)),
        ];
        tool.ui.second.shown = Some(tab);
        ui.painter().vline(middle, primary.y_range(), crate::ui::widgets::divider_stroke(p));
    }
    for (column, (tab, area)) in columns.into_iter().enumerate() {
        let mut main = pane(
            ui,
            area,
            if column == 0 {
                "construction-primary"
            } else {
                "construction-second"
            },
        );
        #[cfg(test)]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::unique(("build_section_column", column)), (tab, area)));
        if !sectionless {
            section_column(&mut main, p, tool, world, s, mods, env, tab, (area.height() - 16.0).max(1.0));
        }
        if tool.tool == Tool::Edit
            && !detach::tab_detached(s, tab)
            && let Some(block) = crate::build::layout::Block::of_tab(tab)
        {
            detach::source(ui, p, tool, s, block, "content", area);
        }
    }
    if shared {
        let values_bottom = if sectionless {
            (primary_bottom + desired_values).min(controls_bottom)
        } else {
            controls_bottom
        };
        let values_rect = egui::Rect::from_min_max(egui::pos2(rect.left(), primary_bottom), egui::pos2(rect.right(), values_bottom));
        let mut values = pane(ui, values_rect.shrink2(egui::vec2(8.0, 4.0)), "construction-transform");
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique("build-transform-viewport"), values.clip_rect()));
        transform(&mut values, p, tool, world, s, mods);
        if !detach::is_detached(s, crate::build::layout::Block::Transform) {
            detach::source(ui, p, tool, s, crate::build::layout::Block::Transform, "content", values_rect);
        }
        ui.painter()
            .hline(rect.x_range(), primary_bottom, crate::ui::widgets::divider_stroke(p));
    }
    if tools_height > 0.0 {
        let mut navigation = pane(
            ui,
            egui::Rect::from_min_max(egui::pos2(rect.left(), controls_bottom), rect.max),
            "construction-tools",
        );
        #[cfg(test)]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::unique(("build_navigation_viewport", true)), navigation.clip_rect()));
        band(&mut navigation, p.bar.gamma_multiply(0.82), 5, 5, |ui| {
            layout::navigation(ui, p, tool, world, s, mods, true);
        });
    }
}

/// The section `tab` scrolled in its column of the build window, or the
/// panel of the tool in use, at most `height` tall.
#[allow(clippy::too_many_arguments)]
fn section_column(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    env: &mut Env,
    tab: usize,
    height: f32,
) {
    band(ui, egui::Color32::TRANSPARENT, 10, 8, |ui| {
        // Lay the content out at the width it keeps once the bar shows: the
        // Create shape grid (and any wrapped text) grows taller as it widens,
        // so a width that follows the bar would show it, narrow, fit, hide it
        // and widen again on every frame (#1092).
        let width = crate::ui::widgets::scrolled_width(ui).max(1.0);
        egui::ScrollArea::vertical()
            .id_salt(("c3_section", tool.tool as u8, tab))
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(width);
                if tool.tool == Tool::Edit {
                    if detach::tab_detached(s, tab) {
                        // Every section is in a window of its own.
                    } else if !tool.selection.is_empty() || matches!(tab, 3 | 5) {
                        section_tab(ui, p, tool, world, s, mods, env, tab);
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
}

/// The section of `tab`, in the build window or in a window of its own:
/// one rendering path for all presentations; permission checks, drafts and
/// simulator commands stay with the original controls.
#[allow(clippy::too_many_arguments)]
pub(super) fn section_tab(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    env: &mut Env,
    tab: usize,
) {
    match tab {
        0 | 1 | 6 => general::show(ui, p, tool, world, false),
        2 => features::show(ui, p, tool, world, s, env),
        3 => texture::show(ui, p, tool, world, s, env),
        4 => contents::show(ui, p, tool, world),
        5 => {
            ui.scope(|ui| object::show_flags(ui, p, tool, world));
            replace::entry(ui, p, tool, world, s);
            crate::ui::widgets::divider(ui, p);
            object::show_shape(ui, p, tool, world, env);
        }
        _ => {
            object::show_transform(ui, p, tool, world, s, mods);
        }
    }
}

#[cfg(test)]
#[path = "compact_tests.rs"]
mod tests;

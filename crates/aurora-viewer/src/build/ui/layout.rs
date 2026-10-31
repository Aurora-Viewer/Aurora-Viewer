//! The side settings share their exact controls with the footer. Organization
//! changes only destinations/order; underlying option values are untouched.

use super::{common, compact, floaters, notecards};
use crate::build::{
    BuildSettings, BuildTool, EditMode, Mods, Tool,
    layout::{Item, Panel},
};
use crate::theme::Palette;
use crate::ui::widgets;
use crate::world::World;
use egui::{Rect, RichText, Vec2};

#[derive(Default)]
pub struct State {
    pub open: bool,
    pub organizing: bool,
    pub drag: Option<Item>,
    pub targets: Vec<(Item, Panel, Rect)>,
    pub zones: Vec<(Panel, Rect)>,
}

pub(super) fn title(item: Item) -> &'static str {
    match item {
        Item::Manipulators => aurora_i18n::tr!("build-layout-manipulators"),
        Item::Actions => aurora_i18n::tr!("build-layout-actions"),
        Item::Face => aurora_i18n::tr!("build-compact-select-face"),
        Item::StretchTextures => aurora_i18n::tr!("build-compact-stretch-textures"),
        Item::Linked => aurora_i18n::tr!("build-compact-linked"),
        Item::ScaleBoth => aurora_i18n::tr!("build-compact-both-sides"),
        Item::RootAxis => aurora_i18n::tr!("build-compact-root-axis"),
        Item::Snap => aurora_i18n::tr!("build-compact-snap"),
        Item::Highlight => aurora_i18n::tr!("build-compact-highlight"),
        Item::Probes => aurora_i18n::tr!("build-compact-probes"),
        Item::OwnedOnly => aurora_i18n::tr!("build-compact-owned-only"),
        Item::MovableOnly => aurora_i18n::tr!("build-compact-movable-only"),
        Item::Surrounding => aurora_i18n::tr!("build-compact-surrounding"),
        Item::LandGroup => aurora_i18n::tr!("rez-under-land-group"),
        Item::FollowEdit => aurora_i18n::tr!("build-ui-tools-follow-edit-object"),
        Item::General => aurora_i18n::tr!("build-compact-general"),
        Item::Shape => aurora_i18n::tr!("build-compact-shape"),
        Item::Materials => aurora_i18n::tr!("build-compact-materials"),
        Item::Features => aurora_i18n::tr!("build-compact-features"),
        Item::Contents => aurora_i18n::tr!("build-compact-contents"),
        Item::Camera => aurora_i18n::tr!("build-compact-camera"),
        Item::Grab => aurora_i18n::tr!("build-compact-grab"),
        Item::Edit => aurora_i18n::tr!("common-edit"),
        Item::Create => aurora_i18n::tr!("build-compact-create"),
        Item::Land => aurora_i18n::tr!("build-compact-land"),
    }
}

pub(super) fn section(item: Item) -> Option<usize> {
    match item {
        Item::General => Some(6),
        Item::Shape => Some(5),
        Item::Materials => Some(3),
        Item::Features => Some(2),
        Item::Contents => Some(4),
        _ => None,
    }
}
pub(super) fn mode(item: Item) -> Option<Tool> {
    match item {
        Item::Camera => Some(Tool::Focus),
        Item::Grab => Some(Tool::Grab),
        Item::Edit => Some(Tool::Edit),
        Item::Create => Some(Tool::Create),
        Item::Land => Some(Tool::Land),
        _ => None,
    }
}
pub(super) fn icon(item: Item) -> &'static str {
    match item {
        Item::General => "info-circle-filled",
        Item::Shape => "box",
        Item::Materials => "paint-filled",
        Item::Features => "adjustments-filled",
        Item::Contents => "folder-filled",
        Item::Camera => "zoom-filled",
        Item::Grab => "hand-grab",
        Item::Edit => "pointer-filled",
        Item::Create => "wand",
        Item::Land => "mountain-filled",
        _ => "arrows-move",
    }
}

pub(super) fn gear(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool) {
    if compact::icon_button(ui, p, "settings-filled", aurora_i18n::tr!("build-layout-settings"), true).clicked() {
        tool.ui.layout.open = !tool.ui.layout.open;
        if !tool.ui.layout.open {
            tool.ui.layout.organizing = false;
            tool.ui.layout.drag = None;
        }
    }
}

/// Disabled descendants retain their normal appearance while a single row
/// interaction owns pointer and keyboard input. Clicking cannot change options.
pub(super) fn row(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    item: Item,
    panel: Panel,
) {
    let organizing = tool.ui.layout.organizing;
    let height = if item.navigation() {
        34.0
    } else if matches!(item, Item::Manipulators | Item::Actions) {
        28.0
    } else {
        24.0
    };
    let rect = ui
        .allocate_exact_size(Vec2::new(ui.available_width(), height), egui::Sense::hover())
        .0;
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("layout-row", item))
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.set_clip_rect(ui.clip_rect().intersect(rect));
    child.spacing_mut().item_spacing.x = 3.0;
    child.spacing_mut().interact_size.y = 22.0;
    if organizing {
        child.visuals_mut().disabled_alpha = 1.0;
        child.disable();
    }
    if item.navigation() {
        if panel == Panel::Navigation {
            let active = section(item).is_some_and(|tab| tool.tool == Tool::Edit && tool.tab == tab)
                || mode(item).is_some_and(|mode| tool.tool == mode);
            if compact::rail_button(&mut child, p, icon(item), title(item), active, height) {
                if let Some(tab) = section(item) {
                    if tool.tool != Tool::Edit {
                        tool.set_tool(Tool::Edit);
                    }
                    tool.tab = tab;
                } else if let Some(mode) = mode(item)
                    && tool.tool != mode
                {
                    tool.set_tool(mode);
                }
            }
        } else {
            widgets::glyph(&mut child, icon(item), p.muted, 18.0);
            common::label(&mut child, p, title(item));
        }
    } else {
        control(&mut child, p, tool, world, s, mods, item);
    }
    let visible = rect.intersect(ui.clip_rect());
    if visible.is_positive() {
        tool.ui.layout.targets.push((item, panel, visible));
    }
    if organizing {
        let response = ui
            .interact(rect, ui.scope_id().with(("organize", item)), egui::Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::Grab);
        if response.drag_started() {
            tool.ui.layout.drag = Some(item);
        }
        if response.clicked() {
            response.request_focus();
        }
        if response.has_focus() {
            let direction = ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    -1
                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    1
                } else {
                    0
                }
            });
            let items = s.layout.in_panel(panel);
            if direction != 0
                && let Some(index) = items.iter().position(|key| *key == item)
                && let Some(target) = items.get(index.wrapping_add_signed(direction))
            {
                s.layout.place(item, panel, Some(*target), direction > 0);
            }
            if ui.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) || i.consume_key(egui::Modifiers::NONE, egui::Key::Space)
            }) {
                let next = if item.navigation() {
                    if panel == Panel::Hidden { Panel::Navigation } else { Panel::Hidden }
                } else {
                    match panel {
                        Panel::Footer => Panel::Settings,
                        Panel::Settings => Panel::Hidden,
                        _ => Panel::Footer,
                    }
                };
                s.layout.place(item, next, None, false);
            }
        }
        if let Some(drag) = tool.ui.layout.drag.filter(|drag| *drag != item && drag.accepts(panel))
            && let Some(pos) = ui.ctx().pointer_interact_pos().filter(|pos| rect.contains(*pos))
        {
            let _ = drag;
            let y = if pos.y < rect.center().y { rect.top() } else { rect.bottom() };
            ui.painter().hline(rect.x_range(), y, egui::Stroke::new(2.0, p.violet_light));
        }
    }
}

pub(super) fn control(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    item: Item,
) {
    if item == Item::Manipulators {
        compact::manipulation(ui, p, tool, world, s, mods);
        return;
    }
    if item == Item::Actions {
        compact::action_row(ui, p, tool, world, s);
        return;
    }
    let mut value = match item {
        Item::Face => tool.edit_mode == EditMode::Face,
        Item::Linked => s.edit_linked,
        Item::StretchTextures => s.scale_stretch_textures,
        Item::ScaleBoth => s.scale_uniform,
        Item::RootAxis => s.actual_root,
        Item::Snap => s.snap,
        Item::Highlight => s.show_highlight,
        Item::Probes => s.select_probes,
        Item::OwnedOnly => s.select_owned_only,
        Item::MovableOnly => s.select_movable_only,
        Item::Surrounding => s.select_surrounding,
        Item::LandGroup => s.rez_under_land_group,
        Item::FollowEdit => s.follow_edit_object,
        _ => return,
    };
    let navigation = matches!(item, Item::Face | Item::Linked);
    let width = if navigation {
        (ui.available_width() - 79.0).max(1.0)
    } else {
        ui.available_width()
    };
    ui.allocate_ui_with_layout(Vec2::new(width, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        if widgets::checkbox(ui, p, &mut value, RichText::new(title(item)).size(common::BODY)).changed() {
            match item {
                Item::Face => tool.set_edit_mode(world, s, if value { EditMode::Face } else { EditMode::Move }),
                Item::Linked => tool.set_edit_linked(world, s, value),
                Item::StretchTextures => s.scale_stretch_textures = value,
                Item::ScaleBoth => s.scale_uniform = value,
                Item::RootAxis => s.actual_root = value,
                Item::Snap => s.snap = value,
                Item::Highlight => s.show_highlight = value,
                Item::Probes => s.select_probes = value,
                Item::OwnedOnly => s.select_owned_only = value,
                Item::MovableOnly => s.select_movable_only = value,
                Item::Surrounding => s.select_surrounding = value,
                Item::LandGroup => s.rez_under_land_group = value,
                Item::FollowEdit => s.follow_edit_object = value,
                _ => {}
            }
        }
    });
    if navigation {
        // Reserve the same 76 pixels when off; its row never changes height.
        let rect = ui.allocate_exact_size(Vec2::new(76.0, 22.0), egui::Sense::hover()).0;
        if value {
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let enabled = !tool.selection.is_empty();
                    if compact::icon_button(ui, p, "caret-left-filled", aurora_i18n::tr!("build-compact-previous"), enabled).clicked() {
                        if item == Item::Linked {
                            tool.select_next_link(world, s, false, mods.shift);
                        } else {
                            tool.select_next_part(world, s, false, mods.shift);
                        }
                    }
                    let number = if item == Item::Face {
                        tool.selected_face_numbers(world)
                    } else {
                        tool.selected_link_number(world).map_or_else(|| "—".into(), |n| n.to_string())
                    };
                    ui.add_sized(
                        [30.0, 22.0],
                        egui::Label::new(RichText::new(&number).size(common::SMALL).color(p.violet_pale)).truncate(),
                    )
                    .on_hover_text(&number);
                    if compact::icon_button(ui, p, "caret-right-filled", aurora_i18n::tr!("build-compact-next"), enabled).clicked() {
                        if item == Item::Linked {
                            tool.select_next_link(world, s, true, mods.shift);
                        } else {
                            tool.select_next_part(world, s, true, mods.shift);
                        }
                    }
                });
            });
        }
    }
}

pub(super) fn panel(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    panel: Panel,
) {
    let start = ui.cursor().min;
    if panel == Panel::Hidden && s.layout.in_panel(panel).is_empty() {
        ui.add_sized(
            [ui.available_width(), 30.0],
            egui::Label::new(
                egui::RichText::new(aurora_i18n::tr!("build-layout-drop-hidden"))
                    .size(common::SMALL)
                    .color(p.muted),
            ),
        );
    }
    for item in s.layout.in_panel(panel) {
        row(ui, p, tool, world, s, mods, item, panel);
    }
    let end = ui.cursor().min.y.max(start.y + 30.0);
    let rect = Rect::from_min_max(start, egui::pos2(ui.max_rect().right(), end));
    if tool.ui.layout.organizing && panel != Panel::Hidden && s.layout.in_panel(panel).is_empty() {
        ui.allocate_space(Vec2::new(rect.width(), 30.0));
    }
    tool.ui.layout.zones.push((panel, rect));
}

pub(super) fn settings(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods) {
    if !tool.ui.layout.open || !tool.open {
        return;
    }
    let Some(anchor) = ctx.memory(|m| m.area_rect(egui::Id::unique(super::INSPECTOR_ID))) else {
        return;
    };
    // Follow the editor in the same movement room: floating bars leave space
    // outside the smaller first-launch work area too.
    let work = crate::ui::layout::move_area(ctx);
    let right = anchor.center().x <= work.center().x + 1.0;
    let available = if right {
        work.right() - anchor.right()
    } else {
        anchor.left() - work.left()
    } - 8.0;
    let width = available.clamp(160.0, 340.0).min(work.width());
    let x =
        if right { anchor.right() + 8.0 } else { anchor.left() - width - 8.0 }.clamp(work.left(), (work.right() - width).max(work.left()));
    let bottom = anchor.bottom().min(work.bottom());
    let area = egui::Area::new(egui::Id::unique("construction-settings"))
        .order(egui::Order::Middle)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(egui::pos2(x, bottom))
        .movable(false);
    area.show(ctx, |ui| {
        ui.set_width(width);
        // Area remembers its measured height. Let the scroll area grow when
        // organizing adds controls instead of keeping its previous short size.
        ui.set_max_height((bottom - work.top()).max(104.0));
        widgets::overlay(ui, p, egui::Margin::same(12), |ui| {
            common::typography(ui);
            ui.set_width((width - 24.0).max(1.0));
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            egui::ScrollArea::vertical()
                .id_salt("construction-settings-scroll")
                .max_height((bottom - work.top() - 24.0).max(80.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    let heading = if tool.ui.layout.organizing {
                        "build-layout-hidden"
                    } else {
                        "build-layout-settings"
                    };
                    if widgets::organization_header(ui, p, aurora_i18n::text(heading)).clicked() {
                        tool.ui.layout.open = false;
                        tool.ui.layout.organizing = false;
                        tool.ui.layout.drag = None;
                    }
                    if s.legacy_window {
                        for item in [
                            Item::OwnedOnly,
                            Item::MovableOnly,
                            Item::Surrounding,
                            Item::LandGroup,
                            Item::FollowEdit,
                        ] {
                            control(ui, p, tool, world, s, mods, item);
                        }
                        widgets::divider(ui, p);
                        common::section(ui, p, aurora_i18n::tr!("build-layout-ruler"));
                        floaters::ruler_controls(ui, p, s);
                        widgets::divider(ui, p);
                        common::section(ui, p, aurora_i18n::tr!("build-layout-distances"));
                        floaters::distance_controls(ui, p, s);
                        widgets::divider(ui, p);
                        common::section(ui, p, aurora_i18n::tr!("build-layout-handles"));
                        floaters::handle_controls(ui, p, s);
                        return;
                    }
                    if tool.ui.layout.organizing {
                        widgets::surface(ui, p, |ui| {
                            panel(ui, p, tool, world, s, mods, Panel::Hidden);
                        });
                        for (mode, icon, text) in [
                            (
                                notecards::Mode::Export,
                                "clipboard-text-filled",
                                aurora_i18n::tr!("build-layout-export-notecard"),
                            ),
                            (
                                notecards::Mode::Load,
                                "folder-filled",
                                aurora_i18n::tr!("build-layout-load-notecard"),
                            ),
                        ] {
                            if widgets::action_button(ui, p, icon, text, widgets::ActionKind::Flat, Vec2::new(ui.available_width(), 28.0))
                                .clicked()
                            {
                                if mode == notecards::Mode::Load {
                                    let folders: Vec<_> = world
                                        .inventory
                                        .folders
                                        .values()
                                        .filter(|folder| !folder.library && folder.info.type_default == 7)
                                        .map(|folder| folder.info.id)
                                        .collect();
                                    for folder in folders {
                                        world.inventory.request(folder);
                                    }
                                }
                                tool.ui.notecards.open = Some(mode);
                                tool.ui.notecards.name = aurora_i18n::tr!("build-layout-notecard-name").into();
                                tool.ui.notecards.message.clear();
                            }
                        }
                        widgets::divider(ui, p);
                        common::section(ui, p, aurora_i18n::tr!("build-layout-settings"));
                    }
                    panel(ui, p, tool, world, s, mods, Panel::Settings);
                    let title = if tool.ui.layout.organizing {
                        "build-layout-finish"
                    } else {
                        "build-layout-organize"
                    };
                    if widgets::action_button(
                        ui,
                        p,
                        "adjustments-filled",
                        aurora_i18n::text(title),
                        widgets::ActionKind::Flat,
                        Vec2::new(ui.available_width(), 28.0),
                    )
                    .clicked()
                    {
                        tool.ui.layout.organizing = !tool.ui.layout.organizing;
                    }
                    widgets::divider(ui, p);
                    common::section(ui, p, aurora_i18n::tr!("build-layout-ruler"));
                    ui.scope(|ui| {
                        if tool.ui.layout.organizing {
                            ui.visuals_mut().disabled_alpha = 1.0;
                            ui.disable();
                        }
                        floaters::ruler_controls(ui, p, s);
                    });
                    widgets::divider(ui, p);
                    common::section(ui, p, aurora_i18n::tr!("build-layout-distances"));
                    ui.scope(|ui| {
                        if tool.ui.layout.organizing {
                            ui.visuals_mut().disabled_alpha = 1.0;
                            ui.disable();
                        }
                        floaters::distance_controls(ui, p, s);
                    });
                    widgets::divider(ui, p);
                    common::section(ui, p, aurora_i18n::tr!("build-layout-handles"));
                    ui.scope(|ui| {
                        if tool.ui.layout.organizing {
                            ui.visuals_mut().disabled_alpha = 1.0;
                            ui.disable();
                        }
                        floaters::handle_controls(ui, p, s);
                    });
                    if !tool.ui.notecards.message.is_empty() {
                        common::small(ui, p, &tool.ui.notecards.message);
                    }
                });
        });
    });
}

pub(super) fn finish(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, s: &mut BuildSettings) {
    if let Some(item) = tool.ui.layout.drag {
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some(pos) = ctx.pointer_interact_pos() {
            widgets::organization_drag_preview(ctx, p, egui::Id::unique("build-layout-drag"), title(item));
            if ctx.input(|i| i.pointer.any_released()) {
                if let Some((target, panel, rect)) = tool
                    .ui
                    .layout
                    .targets
                    .iter()
                    .rev()
                    .find(|(target, panel, rect)| *target != item && item.accepts(*panel) && rect.contains(pos))
                    .copied()
                {
                    s.layout.place(item, panel, Some(target), pos.y >= rect.center().y);
                } else if let Some((panel, _)) = tool
                    .ui
                    .layout
                    .zones
                    .iter()
                    .rev()
                    .find(|(panel, rect)| item.accepts(*panel) && rect.contains(pos))
                {
                    s.layout.place(item, *panel, None, false);
                }
                tool.ui.layout.drag = None;
            }
        }
        if !ctx.input(|i| i.pointer.primary_down()) {
            tool.ui.layout.drag = None;
        }
    }
}

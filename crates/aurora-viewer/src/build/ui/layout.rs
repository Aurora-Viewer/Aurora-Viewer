//! The side settings share their exact controls with the footer. Organization
//! changes only destinations/order; underlying option values are untouched.

use super::{common, compact, floaters, notecards};
use crate::build::{
    BuildSettings, BuildTool, EditMode, Mods, Tool,
    layout::{FooterColumn, FooterRow, Item, Panel},
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
    pub footer_cells: Vec<FooterDropCell>,
    /// Measured at the current width, including wrapped two-column options.
    pub footer_height: Option<f32>,
}

#[derive(Clone, Copy)]
pub struct FooterDropCell {
    pub rect: Rect,
    pub column: FooterColumn,
    pub row: FooterRow,
    append: Option<(Option<Item>, bool)>,
    actions_only: bool,
}

impl FooterDropCell {
    fn accepts(self, item: Item) -> bool {
        item.accepts(Panel::Footer) && (item == Item::Actions) == self.actions_only
    }

    fn destination(self, item: Item, pos: egui::Pos2) -> (Option<Item>, bool) {
        if self.actions_only {
            return self.append.unwrap_or((None, false));
        }
        if item.footer_column_item()
            && let Some(destination) = self.append
        {
            return destination;
        }
        let after = pos.y >= self.rect.center().y;
        let column = self.column;
        match self.row {
            FooterRow::Full(target) => (Some(target), after),
            FooterRow::Cells(cells) if item.footer_column_item() => {
                if let Some(target) = cells[column.index()] {
                    (Some(target), after)
                } else {
                    (cells[1 - column.index()], true)
                }
            }
            FooterRow::Cells(cells) => {
                let mut items = cells.into_iter().flatten().filter(|target| *target != item);
                (if after { items.next_back() } else { items.next() }, after)
            }
        }
    }
}

pub(super) fn title(item: Item) -> &'static str {
    match item {
        Item::Manipulators => aurora_i18n::tr!("build-layout-manipulators"),
        Item::MoveMode => aurora_i18n::tr!("build-compact-mode-move"),
        Item::RotateMode => aurora_i18n::tr!("build-compact-mode-rotate"),
        Item::StretchMode => aurora_i18n::tr!("build-compact-mode-stretch"),
        Item::AlignMode => aurora_i18n::tr!("build-compact-mode-align"),
        Item::Actions => aurora_i18n::tr!("build-layout-actions"),
        Item::Face => aurora_i18n::tr!("build-compact-select-face"),
        Item::StretchTextures => aurora_i18n::tr!("build-compact-stretch-textures"),
        Item::Linked => aurora_i18n::tr!("build-compact-linked"),
        Item::ScaleBoth => aurora_i18n::tr!("build-compact-both-sides"),
        Item::RootAxis => aurora_i18n::tr!("build-compact-root-axis"),
        Item::Snap => aurora_i18n::tr!("build-snap"),
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

fn manipulation_mode(item: Item) -> Option<EditMode> {
    match item {
        Item::MoveMode => Some(EditMode::Move),
        Item::RotateMode => Some(EditMode::Rotate),
        Item::StretchMode => Some(EditMode::Stretch),
        Item::AlignMode => Some(EditMode::Align),
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
    let height = if item == Item::Snap && !s.legacy_window {
        widgets::TEXT_BUTTON_HEIGHT
    } else if panel == Panel::Footer && !matches!(item, Item::Manipulators | Item::Actions | Item::Snap) {
        let width = ui.available_width();
        let compact = !s.legacy_window;
        let padding = ui.spacing().icon_width + ui.spacing().icon_spacing;
        let width = (width - padding).max(1.0);
        let text = RichText::new(title(item)).size(common::BODY);
        let height = egui::WidgetText::from(text)
            .into_galley(ui, Some(egui::TextWrapMode::Wrap), width, egui::TextStyle::Button)
            .size()
            .y;
        if compact {
            height.max(widgets::TEXT_BUTTON_HEIGHT)
        } else {
            height.max(22.0) + 2.0
        }
    } else if item.navigation() {
        34.0
    } else if manipulation_mode(item).is_some() || (item == Item::Actions && !s.legacy_window) {
        widgets::TEXT_BUTTON_HEIGHT
    } else if matches!(item, Item::Manipulators | Item::Actions) {
        28.0
    } else if item == Item::Snap && !organizing && s.snap_mode() == crate::build::SnapMode::Objects {
        SNAP_ROWS_HEIGHT
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
    if panel == Panel::Footer {
        child.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
    }
    child.spacing_mut().item_spacing.x = 3.0;
    child.spacing_mut().interact_size.y =
        if !s.legacy_window && (panel == Panel::Footer || manipulation_mode(item).is_some() || item == Item::Snap) {
            widgets::TEXT_BUTTON_HEIGHT
        } else {
            22.0
        };
    if organizing {
        child.visuals_mut().disabled_alpha = 1.0;
        child.disable();
    }
    if item.navigation() {
        if panel == Panel::Navigation {
            let active = section(item).is_some_and(|tab| tool.tool == Tool::Edit && tool.tab == tab)
                || mode(item).is_some_and(|mode| tool.tool == mode);
            if compact::navigation_button(&mut child, p, icon(item), title(item), active, height) {
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
        control(&mut child, p, tool, world, s, mods, item, panel);
    }
    let visible = rect.intersect(ui.clip_rect());
    if visible.is_positive() {
        tool.ui.layout.targets.push((item, panel, visible));
    }
    if organizing {
        let response = ui
            .interact(
                rect,
                egui::Id::unique(("build-layout-organize", item)),
                egui::Sense::click_and_drag(),
            )
            .on_hover_cursor(egui::CursorIcon::Grab);
        let response = if tool.ui.layout.drag.is_none() {
            response.on_hover_text(aurora_i18n::tr!("build-layout-drag-hint"))
        } else {
            response
        };
        widgets::organization_source(
            ui,
            p,
            rect,
            item.footer_column_item() && (item != Item::Snap || (panel == Panel::Footer && !s.legacy_window)),
            tool.ui.layout.drag == Some(item) || (tool.ui.layout.drag.is_none() && (response.hovered() || response.has_focus())),
        );
        if response.drag_started() {
            tool.ui.layout.drag = Some(item);
        }
        if response.clicked() {
            response.request_focus();
        }
        if response.clicked() || response.has_focus() {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        vertical_arrows: true,
                        horizontal_arrows: panel == Panel::Navigation || (panel == Panel::Footer && item.footer_column_item()),
                        ..Default::default()
                    },
                );
            });
        }
        if response.has_focus() {
            if panel == Panel::Footer && item.footer_column_item() {
                let column = ui.input_mut(|i| {
                    if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft) {
                        Some(FooterColumn::Left)
                    } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight) {
                        Some(FooterColumn::Right)
                    } else {
                        None
                    }
                });
                if let Some(column) = column {
                    s.layout.place_in_footer(item, column, Some(item), false);
                    // egui queues directional focus before the row consumes
                    // its first arrow event, before its focus filter is active.
                    ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
                }
            }
            let direction = ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                    || (panel == Panel::Navigation && i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft))
                {
                    -1
                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                    || (panel == Panel::Navigation && i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight))
                {
                    1
                } else {
                    0
                }
            });
            let items: Vec<_> = s
                .layout
                .in_panel(panel)
                .into_iter()
                .filter(|other| panel != Panel::Navigation || mode(*other).is_some() == mode(item).is_some())
                .filter(|other| {
                    panel != Panel::Footer
                        || !item.footer_column_item()
                        || !other.footer_column_item()
                        || s.layout.footer_columns.get(other) == s.layout.footer_columns.get(&item)
                })
                .collect();
            if direction != 0 {
                ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            }
            if direction != 0 && panel == Panel::Footer && item == Item::Actions {
                let columns: Vec<_> = items.iter().copied().filter(|item| item.footer_column_item()).collect();
                let target = if direction < 0 { columns.first() } else { columns.last() };
                if let Some(target) = target {
                    s.layout.place(item, panel, Some(*target), direction > 0);
                }
            } else if direction != 0
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
        if panel != Panel::Footer
            && let Some(drag) = tool.ui.layout.drag.filter(|drag| *drag != item && drag.accepts(panel))
        {
            let _ = drag;
            let pos = ui.ctx().pointer_interact_pos().filter(|pos| rect.contains(*pos));
            widgets::organization_drop_target(
                ui,
                p,
                rect.shrink(0.5),
                None,
                if panel == Panel::Navigation {
                    None
                } else {
                    pos.map(|pos| pos.y >= rect.center().y)
                },
            );
            if panel == Panel::Navigation
                && let Some(pos) = pos
            {
                let x = if pos.x < rect.center().x { rect.left() } else { rect.right() };
                ui.painter().vline(x, rect.y_range(), egui::Stroke::new(2.0, p.violet_light));
            }
        }
    }
}

/// Full-width controls and an entirely empty footer expose both column targets.
fn footer_drop_row(ui: &egui::Ui, p: &Palette, tool: &mut BuildTool, start: egui::Pos2, row: FooterRow, actions_top: bool) {
    let rect = Rect::from_min_max(start, egui::pos2(ui.max_rect().right(), ui.cursor().min.y));
    for column in [FooterColumn::Left, FooterColumn::Right] {
        let mut half = rect;
        if column == FooterColumn::Left {
            half.max.x = rect.center().x;
        } else {
            half.min.x = rect.center().x;
        }
        let cell = FooterDropCell {
            rect: half.intersect(ui.clip_rect()),
            column,
            row,
            append: (row == FooterRow::Full(Item::Actions)).then_some((Some(Item::Actions), actions_top)),
            actions_only: false,
        };
        footer_drop_cell(ui, p, tool, cell);
    }
}

fn footer_drop_cell(ui: &egui::Ui, p: &Palette, tool: &mut BuildTool, mut cell: FooterDropCell) {
    cell.rect = cell.rect.intersect(ui.clip_rect());
    if !cell.rect.is_positive() {
        return;
    }
    if tool.ui.layout.drag.is_some_and(|item| !cell.accepts(item)) {
        return;
    }
    tool.ui.layout.footer_cells.push(cell);
    if !tool.ui.layout.organizing {
        return;
    }
    let Some(drag) = tool.ui.layout.drag.filter(|item| item.accepts(Panel::Footer)) else {
        return;
    };
    let empty = matches!(cell.row, FooterRow::Cells(cells) if cells[cell.column.index()].is_none());
    let hovered = ui.ctx().pointer_interact_pos().filter(|pos| cell.rect.contains(*pos));
    let insertion = hovered.map(|pos| cell.destination(drag, pos).1);
    let preview = if hovered.is_some() && !drag.footer_column_item() {
        Rect::from_min_max(
            egui::pos2(ui.max_rect().left(), cell.rect.top()),
            egui::pos2(ui.max_rect().right(), cell.rect.bottom()),
        )
    } else {
        cell.rect
    };
    widgets::organization_drop_target(
        ui,
        p,
        preview.shrink2(egui::vec2(ui.spacing().item_spacing.x * 0.25, 1.0)),
        empty.then(|| aurora_i18n::tr!("build-layout-drop-here")),
        insertion,
    );
}

/// The action bar can only bracket the complete options block. These strips
/// reuse its existing top/bottom edges and never allocate another row.
fn footer_action_targets(ui: &egui::Ui, p: &Palette, tool: &mut BuildTool, s: &BuildSettings, empty: Rect) {
    tool.ui.layout.footer_cells.clear();
    let bounds = tool
        .ui
        .layout
        .targets
        .iter()
        .filter(|(item, panel, _)| *panel == Panel::Footer && item.footer_column_item())
        .map(|(_, _, rect)| *rect)
        .reduce(|a, b| a.union(b));
    let columns: Vec<_> = s
        .layout
        .in_panel(Panel::Footer)
        .into_iter()
        .filter(|item| item.footer_column_item())
        .collect();
    if let (Some(bounds), Some(&first), Some(&last)) = (bounds, columns.first(), columns.last()) {
        for (target, after, top, bottom) in [
            (first, false, bounds.top() - 2.0, bounds.top() + 8.0),
            (last, true, bounds.bottom() - 8.0, bounds.bottom() + 2.0),
        ] {
            footer_drop_cell(
                ui,
                p,
                tool,
                FooterDropCell {
                    rect: Rect::from_min_max(egui::pos2(ui.max_rect().left(), top), egui::pos2(ui.max_rect().right(), bottom)),
                    column: FooterColumn::Left,
                    row: FooterRow::Full(target),
                    append: Some((Some(target), after)),
                    actions_only: true,
                },
            );
        }
    } else if s.layout.in_panel(Panel::Footer).is_empty() {
        footer_drop_cell(
            ui,
            p,
            tool,
            FooterDropCell {
                rect: empty,
                column: FooterColumn::Left,
                row: FooterRow::Cells([None; 2]),
                append: Some((None, false)),
                actions_only: true,
            },
        );
    }
}

/// Each column owns its vertical stack. A wrapped label or insertion in one
/// column must not add a spacer or change the positions in its neighbour.
fn footer_columns(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    rows: &[[Option<Item>; 2]],
    before: Option<Item>,
) {
    let bounds = ui.max_rect();
    let mut ends = [ui.cursor().min.y; 2];
    let mut last = [None; 2];
    let mut targets = Vec::new();
    ui.columns(2, |columns| {
        for (index, column) in columns.iter_mut().enumerate() {
            let side = if index == 0 { FooterColumn::Left } else { FooterColumn::Right };
            let x = if index == 0 {
                bounds.left()..=bounds.center().x
            } else {
                bounds.center().x..=bounds.right()
            };
            for &cells in rows {
                if let Some(item) = cells[index] {
                    let top = column.cursor().min.y;
                    row(column, p, tool, world, s, mods, item, Panel::Footer);
                    targets.push(FooterDropCell {
                        rect: Rect::from_min_max(egui::pos2(*x.start(), top), egui::pos2(*x.end(), column.cursor().min.y)),
                        column: side,
                        row: FooterRow::Cells(cells),
                        append: None,
                        actions_only: false,
                    });
                    last[index] = Some(item);
                }
            }
            ends[index] = column.cursor().min.y;
        }
    });
    let bottom = ui.cursor().min.y;
    for (index, column) in [FooterColumn::Left, FooterColumn::Right].into_iter().enumerate() {
        if bottom - ends[index] > 1.0 {
            let mut cells = [None; 2];
            cells[1 - index] = last[1 - index];
            let (left, right) = if index == 0 {
                (bounds.left(), bounds.center().x)
            } else {
                (bounds.center().x, bounds.right())
            };
            targets.push(FooterDropCell {
                rect: Rect::from_min_max(egui::pos2(left, ends[index]), egui::pos2(right, bottom)),
                column,
                row: FooterRow::Cells(cells),
                // An empty tail appends to this column's own last item, or
                // before its following full-width row when the column is empty.
                append: Some(last[index].map_or((before, false), |item| (Some(item), true))),
                actions_only: false,
            });
        }
    }
    for cell in targets {
        footer_drop_cell(ui, p, tool, cell);
    }
}

/// The Snap item's three lines in Objects mode, Place on surface and its
/// rotation under the choice in Legacy settings (`snap_mode`).
const SNAP_ROWS_HEIGHT: f32 = 84.0;

fn compact_snap(ui: &mut egui::Ui, p: &Palette, s: &mut BuildSettings) {
    use crate::build::SnapMode;
    ui.spacing_mut().item_spacing.x = 2.0;
    let mut enabled = s.snap_mode() != SnapMode::Off;
    if widgets::checkbox(ui, p, &mut enabled, RichText::new(title(Item::Snap)).size(common::BODY)).changed() {
        s.set_snap_enabled(enabled);
    }
    let preferred = s.snap_choice();
    for (index, (mode, label)) in [
        (SnapMode::Grid, aurora_i18n::tr!("build-snap-grid")),
        (SnapMode::Objects, aurora_i18n::tr!("build-snap-objects")),
    ]
    .into_iter()
    .enumerate()
    {
        ui.label(RichText::new(if index == 0 { "-" } else { "/" }).size(common::BODY).color(p.muted));
        let selected = enabled && preferred == mode;
        let text = RichText::new(label).size(common::BODY);
        let text = if selected {
            text.strong().underline().color(p.violet_light)
        } else {
            text.color(p.muted)
        };
        let response = ui
            .add(egui::Label::new(text).truncate().sense(egui::Sense::click()))
            .on_hover_text(label)
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, ui.is_enabled(), selected, label));
        #[cfg(test)]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::unique(("build_snap_choice", index)), response.rect));
        #[cfg(not(test))]
        let _ = index;
        if response.clicked() {
            s.set_snap_mode(mode);
            ui.ctx().request_repaint();
        }
    }
}

/// "Snap": off, to the grid or to objects, one at a time
/// (`BuildSettings::snap_mode`); under it in Objects mode, "Place on
/// surface", then Keep rotation or Auto rotate with the side put against
/// the surface (`build/place.rs`).
fn snap_mode(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &World, s: &mut BuildSettings) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        snap_rows(ui, p, tool, world, s);
    });
}

fn snap_rows(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &World, s: &mut BuildSettings) {
    use crate::build::SnapMode;
    const MODES: [SnapMode; 3] = [SnapMode::Off, SnapMode::Grid, SnapMode::Objects];
    let options = [
        // (the accent for Aurora's own choice, as everywhere)
        (aurora_i18n::tr!("build-snap-off"), false),
        (aurora_i18n::tr!("build-snap-grid"), false),
        (aurora_i18n::tr!("build-snap-objects"), true),
    ];
    let mut selected = MODES.iter().position(|m| *m == s.snap_mode()).unwrap_or(0);
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), 22.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(RichText::new(title(Item::Snap)).size(common::BODY));
            ui.add_space((ui.available_width() - widgets::segmented_size(ui, &options).x).max(4.0));
            if widgets::segmented(ui, p, &mut selected, &options) {
                s.set_snap_mode(MODES[selected]);
            }
        },
    );
    if s.snap_mode() == crate::build::SnapMode::Objects {
        snap_placement(ui, p, tool, world, s);
    }
}

fn snap_placement(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &World, s: &mut BuildSettings) {
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), 24.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let kind = if tool.place.active {
                widgets::ActionKind::Primary
            } else {
                widgets::ActionKind::Flat
            };
            let label = aurora_i18n::tr!("build-place");
            if widgets::action_button_if(
                ui,
                p,
                "box-align-bottom-filled",
                label,
                kind,
                Vec2::new(0.0, 22.0),
                tool.can_place(world),
            )
            .on_hover_text(aurora_i18n::tr!("build-place-tip"))
            .clicked()
            {
                tool.toggle_place(world);
            }
            // its distance off the surface
            ui.add_space((ui.available_width() - 136.0).max(4.0));
            ui.label(RichText::new(aurora_i18n::tr!("build-place-offset")).size(common::BODY))
                .on_hover_text(aurora_i18n::tr!("build-place-offset-tip"));
            ui.add_space((ui.available_width() - 70.0).max(4.0));
            common::spin(ui, &mut s.place_offset, 0.01, -10.0..=10.0, 3, 64.0, false);
        },
    );
    // Keep rotation, or Auto rotate onto a chosen side
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), 24.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let options = [
                (aurora_i18n::tr!("build-place-keep"), true),
                (aurora_i18n::tr!("build-place-rotate"), true),
            ];
            let mut selected = usize::from(s.place_rotate);
            ui.scope(|ui| {
                if widgets::segmented(ui, p, &mut selected, &options) {
                    s.place_rotate = selected == 1;
                }
            })
            .response
            .on_hover_text(aurora_i18n::tr!("build-place-rotate-tip"));
            let sides: Vec<(&str, crate::build::place::Side)> =
                crate::build::place::Side::ALL.iter().map(|side| (side.label(), *side)).collect();
            ui.add_space((ui.available_width() - 96.0).max(4.0));
            ui.scope(|ui| {
                if let Some(side) = common::combo(ui, "place-side", Some(s.place_side), &sides, 90.0, s.place_rotate) {
                    s.place_side = side;
                }
            })
            .response
            .on_hover_text(aurora_i18n::tr!("build-place-side-tip"));
        },
    );
}

pub(super) fn control(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    item: Item,
    panel: Panel,
) {
    if let Some(mode) = manipulation_mode(item) {
        let size = Vec2::new(ui.available_width(), ui.available_height().max(widgets::TEXT_BUTTON_HEIGHT));
        compact::manipulation_choice(ui, p, tool, world, s, mods, mode, size);
        return;
    }
    if item == Item::Manipulators {
        compact::manipulation(ui, p, tool, world, s, mods);
        return;
    }
    if item == Item::Actions {
        compact::action_row(ui, p, tool, world, s);
        return;
    }
    if item == Item::Snap {
        if s.legacy_window {
            snap_mode(ui, p, tool, world, s);
        } else {
            compact_snap(ui, p, s);
        }
        return;
    }
    let mut value = match item {
        Item::Face => tool.edit_mode == EditMode::Face,
        Item::Linked => s.edit_linked,
        Item::StretchTextures => s.scale_stretch_textures,
        Item::ScaleBoth => s.scale_uniform,
        Item::RootAxis => s.actual_root,
        Item::Highlight => s.show_highlight,
        Item::Probes => s.select_probes,
        Item::OwnedOnly => s.select_owned_only,
        Item::MovableOnly => s.select_movable_only,
        Item::Surrounding => s.select_surrounding,
        Item::LandGroup => s.rez_under_land_group,
        Item::FollowEdit => s.follow_edit_object,
        _ => return,
    };
    let compact_footer = panel == Panel::Footer && !s.legacy_window;
    let width = ui.available_width();
    let height = if compact_footer {
        ui.available_height().max(widgets::TEXT_BUTTON_HEIGHT)
    } else {
        22.0
    };
    ui.allocate_ui_with_layout(Vec2::new(width, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(width);
        let text = RichText::new(title(item)).size(common::BODY);
        let response = widgets::checkbox(ui, p, &mut value, text);
        if response.changed() {
            match item {
                Item::Face => tool.set_edit_mode(world, s, if value { EditMode::Face } else { EditMode::Move }),
                Item::Linked => tool.set_edit_linked(world, s, value),
                Item::StretchTextures => s.scale_stretch_textures = value,
                Item::ScaleBoth => s.scale_uniform = value,
                Item::RootAxis => s.actual_root = value,
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
}

pub(super) fn selection_navigation(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    item: Item,
    width: f32,
) {
    let number = if item == Item::Face && tool.edit_mode == EditMode::Face {
        tool.selection
            .first()
            .and_then(|key| world.objects.index_of(key))
            .map(|_| tool.selected_face_numbers(world))
    } else if item == Item::Linked && s.edit_linked {
        tool.selected_link_number(world).map(|n| n.to_string())
    } else {
        None
    };
    let response = ui.allocate_ui_with_layout(Vec2::new(width, 26.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.spacing_mut().interact_size.y = widgets::TEXT_BUTTON_HEIGHT;
        let enabled = number.is_some() && !tool.selection.is_empty();
        let label = if item == Item::Linked {
            aurora_i18n::tr!("build-compact-link-value", number = number.as_deref().unwrap_or("-"))
        } else {
            aurora_i18n::tr!("build-compact-face-value", number = number.as_deref().unwrap_or("-"))
        };
        for next in [false, true] {
            if next {
                widgets::readout_sized(
                    ui,
                    RichText::new(&label)
                        .size(common::SMALL)
                        .color(if number.is_some() { p.violet_pale } else { p.muted }),
                    Vec2::new((width - 40.0).max(1.0), widgets::TEXT_BUTTON_HEIGHT),
                )
                .on_hover_text(&label);
            }
            if widgets::action_button_if(
                ui,
                p,
                if next { "caret-right-filled" } else { "caret-left-filled" },
                "",
                widgets::ActionKind::Flat,
                Vec2::splat(widgets::TEXT_BUTTON_HEIGHT),
                enabled,
            )
            .on_hover_text(if next {
                aurora_i18n::tr!("build-compact-next")
            } else {
                aurora_i18n::tr!("build-compact-previous")
            })
            .clicked()
            {
                if item == Item::Linked {
                    tool.select_next_link(world, s, next, mods.shift);
                } else {
                    tool.select_next_part(world, s, next, mods.shift);
                }
            }
        }
    });
    #[cfg(test)]
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::unique(("build_part_navigation", item)), response.response.rect));
    #[cfg(not(test))]
    let _ = response;
}

pub(super) fn navigation(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    mods: Mods,
    tools: bool,
) {
    let items: Vec<_> = s
        .layout
        .in_panel(Panel::Navigation)
        .into_iter()
        .filter(|item| mode(*item).is_some() == tools)
        .collect();
    if items.is_empty() {
        return;
    }
    let start = ui.cursor().min;
    ui.columns(items.len(), |columns| {
        for (column, item) in columns.iter_mut().zip(items) {
            row(column, p, tool, world, s, mods, item, Panel::Navigation);
        }
    });
    let end = ui.cursor().min.y.max(start.y + 30.0);
    tool.ui
        .layout
        .zones
        .push((Panel::Navigation, Rect::from_min_max(start, egui::pos2(ui.max_rect().right(), end))));
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
    if panel == Panel::Navigation {
        navigation(ui, p, tool, world, s, mods, false);
        navigation(ui, p, tool, world, s, mods, true);
        return;
    }
    let start = ui.cursor().min;
    if panel == Panel::Hidden && s.layout.in_panel(panel).is_empty() {
        let hint = if tool.ui.layout.drag.is_some_and(|item| item.accepts(panel)) {
            aurora_i18n::tr!("build-layout-drop-hidden")
        } else {
            ""
        };
        ui.add_sized(
            [ui.available_width(), 30.0],
            egui::Label::new(egui::RichText::new(hint).size(common::SMALL).color(p.muted)),
        );
    }
    let items = s.layout.in_panel(panel);
    if panel == Panel::Footer {
        ui.spacing_mut().item_spacing.y = 1.0;
        tool.ui.layout.footer_cells.clear();
        let rows = s.layout.footer_rows();
        // An empty footer still needs a destination, but a populated footer
        // uses its existing cells and row edges without a trailing blank row.
        if tool.ui.layout.organizing && rows.is_empty() {
            let top = ui.cursor().min;
            ui.allocate_space(Vec2::new(ui.available_width(), widgets::TEXT_BUTTON_HEIGHT));
            footer_drop_row(ui, p, tool, top, FooterRow::Cells([None; 2]), s.layout.footer_actions_top());
        }
        let mut columns = Vec::new();
        for footer_row in rows {
            match footer_row {
                FooterRow::Full(item) => {
                    if !columns.is_empty() {
                        footer_columns(ui, p, tool, world, s, mods, &columns, Some(item));
                        columns.clear();
                    }
                    if !s.legacy_window && item == Item::Actions {
                        ui.add_space(compact::ACTION_GAP);
                    }
                    let top = ui.cursor().min;
                    row(ui, p, tool, world, s, mods, item, panel);
                    footer_drop_row(ui, p, tool, top, footer_row, s.layout.footer_actions_top());
                }
                FooterRow::Cells(cells) => columns.push(cells),
            }
        }
        if !columns.is_empty() {
            footer_columns(ui, p, tool, world, s, mods, &columns, None);
        }
        tool.ui.layout.footer_height = Some(ui.cursor().min.y - start.y + 12.0);
    } else {
        for &item in &items {
            row(ui, p, tool, world, s, mods, item, panel);
        }
    }
    let end = ui.cursor().min.y.max(start.y + 30.0);
    let rect = Rect::from_min_max(start, egui::pos2(ui.max_rect().right(), end));
    if tool.ui.layout.organizing && tool.ui.layout.drag == Some(Item::Actions) && panel == Panel::Footer {
        footer_action_targets(ui, p, tool, s, rect);
    }
    if tool.ui.layout.organizing
        && panel != Panel::Footer
        && items.is_empty()
        && tool.ui.layout.drag.is_some_and(|item| item.accepts(panel))
    {
        widgets::organization_drop_target(
            ui,
            p,
            rect.shrink(1.0),
            (panel != Panel::Hidden).then(|| aurora_i18n::tr!("build-layout-drop-here")),
            None,
        );
    }
    if tool.ui.layout.organizing && panel == Panel::Settings && s.layout.in_panel(panel).is_empty() {
        ui.allocate_space(Vec2::new(rect.width(), 30.0));
    }
    tool.ui.layout.zones.push((panel, rect));
}

pub(super) fn settings(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, mods: Mods) {
    if !tool.ui.layout.open || !tool.open || widgets::floater_minimized(ctx, super::INSPECTOR_ID) {
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
                            Item::Snap,
                            Item::OwnedOnly,
                            Item::MovableOnly,
                            Item::Surrounding,
                            Item::LandGroup,
                            Item::FollowEdit,
                        ] {
                            control(ui, p, tool, world, s, mods, item, Panel::Settings);
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
                        if widgets::action_button(
                            ui,
                            p,
                            "restore",
                            aurora_i18n::tr!("ui-options-reset-all"),
                            widgets::ActionKind::Flat,
                            Vec2::new(ui.available_width(), 28.0),
                        )
                        .clicked()
                        {
                            s.layout = Default::default();
                            tool.ui.layout.drag = None;
                            tool.ui.layout.footer_height = None;
                            tool.ui.notecards.pending_load = None;
                            tool.ui.notecards.message.clear();
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
                    // Surface placement stays available in settings without
                    // changing the compact Snap row in the editable columns.
                    if s.snap_mode() == crate::build::SnapMode::Objects {
                        widgets::divider(ui, p);
                        common::section(ui, p, aurora_i18n::tr!("build-snap-placement-settings"));
                        ui.scope(|ui| {
                            if tool.ui.layout.organizing {
                                ui.disable();
                            }
                            snap_placement(ui, p, tool, world, s);
                        });
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
        let demo_hold = ctx.cumulative_frame_nr() >= 700
            && ctx.data(|data| data.get_temp::<Item>(egui::Id::unique("build-demo-layout-drag"))) == Some(item);
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some(pos) = ctx.pointer_interact_pos() {
            widgets::organization_drag_preview(ctx, p, egui::Id::unique("build-layout-drag"), title(item));
            if item == Item::Actions
                && tool
                    .ui
                    .layout
                    .zones
                    .iter()
                    .any(|(panel, rect)| *panel == Panel::Footer && rect.contains(pos))
                && !tool
                    .ui
                    .layout
                    .footer_cells
                    .iter()
                    .any(|cell| cell.accepts(item) && cell.rect.contains(pos))
            {
                ctx.set_cursor_icon(egui::CursorIcon::NotAllowed);
            }
            if !demo_hold && ctx.input(|i| i.pointer.any_released()) {
                if let Some(cell) = tool
                    .ui
                    .layout
                    .footer_cells
                    .iter()
                    .find(|cell| cell.accepts(item) && cell.rect.contains(pos))
                    .copied()
                {
                    let (target, after) = cell.destination(item, pos);
                    s.layout.place_in_footer(item, cell.column, target, after);
                } else if let Some((target, panel, rect)) = tool
                    .ui
                    .layout
                    .targets
                    .iter()
                    .rev()
                    .find(|(target, panel, rect)| *panel != Panel::Footer && *target != item && item.accepts(*panel) && rect.contains(pos))
                    .copied()
                {
                    let after = if panel == Panel::Navigation {
                        pos.x >= rect.center().x
                    } else {
                        pos.y >= rect.center().y
                    };
                    s.layout.place(item, panel, Some(target), after);
                } else if let Some((panel, _)) = tool
                    .ui
                    .layout
                    .zones
                    .iter()
                    .rev()
                    .find(|(panel, rect)| *panel != Panel::Footer && item.accepts(*panel) && rect.contains(pos))
                {
                    s.layout.place(item, *panel, None, false);
                }
                tool.ui.layout.drag = None;
            }
        }
        if !demo_hold && !ctx.input(|i| i.pointer.primary_down()) {
            tool.ui.layout.drag = None;
        }
    }
}

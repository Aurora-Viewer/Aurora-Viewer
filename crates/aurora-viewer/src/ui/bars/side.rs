//! The left and right toolbars (LLToolBarView's TOOLBAR_LEFT and
//! TOOLBAR_RIGHT, indra/newview/lltoolbarview.cpp, originally LGPL 2.1).
//! With the islands, a column of the dock's buttons in a glass island 8 px
//! from the screen's edge, placed in the windows' room by its alignment;
//! with Firestorm's bars, a panel the screen's height between the top and
//! bottom bars, like Firestorm's. A toolbar without buttons is not drawn,
//! except while a button is dragged, when it shows where it can go
//! (LLToolBarView::draw), without taking room from the windows.

use super::dock::{BUTTON_ICON, MIN_BUTTON, narrow};
use super::islands::{GAP, MARGIN, PAD};
use super::{BarAction, ButtonLook, DOCK_MIC_W, MicButton, Row, bar_frame, natural_width, sizing, toolbar_row};
use crate::theme::Palette;
use crate::ui::Panels;
use crate::ui::icons::Icons;
use crate::ui::skin::{Layout, ToolButton};
use crate::ui::toolbar::{Bar, ButtonMode, Style, ToolbarSettings, ToolbarUi};
use egui::{Id, Pos2, Rect, Sense, Vec2};

/// Room between the island's edge and its buttons, above and below.
const V_PAD: f32 = 4.0;
/// Firestorm's side toolbar: its buttons' height and icon width.
const CLASSIC_H: f32 = 24.0;
const CLASSIC_SQUARE: f32 = 28.0;
const CLASSIC_PAD: f32 = 3.0;

/// The island of a side toolbar with the islands: `widths` are its
/// buttons' own widths for a button height (`natural`); they all take the
/// widest. Its column stands in `room` (the windows' room left by the top
/// and bottom bars) by `style.align`; 8 px from the screen's edge. When it
/// is too tall, its buttons take the reduced size. Returns the island, the
/// button height and the buttons' width.
#[cfg(test)]
pub fn geometry(screen: Rect, room: Rect, bar: Bar, style: Style, natural: impl Fn(f32) -> Vec<f32>) -> (Rect, f32, f32) {
    let (rect, button, width, _) = geometry_fixed(screen, room, bar, style, natural, &[], &[], &[]);
    (rect, button, width)
}

#[allow(clippy::too_many_arguments)]
fn geometry_fixed(
    screen: Rect,
    room: Rect,
    bar: Bar,
    style: Style,
    natural: impl Fn(f32) -> Vec<f32>,
    blocks: &[Option<f32>],
    stretch: &[bool],
    scopes: &[sizing::BlockSizing],
) -> (Rect, f32, f32, Vec<f32>) {
    let column = |button: f32| {
        let widths = natural(button);
        let width = widths.iter().copied().fold(button, f32::max);
        let mut heights = if widths.is_empty() {
            vec![button]
        } else {
            (0..widths.len())
                .map(|i| blocks.get(i).copied().flatten().unwrap_or(button))
                .collect()
        };
        if !scopes.is_empty() {
            let fixed: Vec<_> = (0..heights.len())
                .map(|index| blocks.get(index).is_some_and(Option::is_some))
                .collect();
            heights = sizing::fit_blocks(&heights, &fixed, scopes, stretch, room.height() - V_PAD * 2.0, GAP, MIN_BUTTON);
        }
        (width, heights)
    };
    let extent = |heights: &[f32]| heights.iter().sum::<f32>() + GAP * heights.len().saturating_sub(1) as f32 + V_PAD * 2.0;
    let mut button = if narrow(screen) { 32.0 } else { 34.0 };
    let (mut width, mut heights) = column(button);
    if extent(&heights) > room.height() {
        button = MIN_BUTTON;
        (width, heights) = column(button);
    }
    crate::ui::toolbar::structure::distribute(&mut heights, stretch, room.height() - V_PAD * 2.0, GAP);
    let aligned = sizing::align_blocks(&mut heights, scopes, room.height() - V_PAD * 2.0, GAP, style.align);
    let height = extent(&heights);
    let outer = width + PAD * 2.0;
    let x = match bar {
        Bar::Right => screen.right() - MARGIN - outer,
        _ => screen.left() + MARGIN,
    };
    let y = aligned.map_or_else(
        || sizing::place(style.align, room.top(), room.bottom(), height),
        |lead| room.top() + lead,
    );
    (
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(outer, height)),
        button,
        width,
        heights,
    )
}

/// The room the windows keep beside a side island: up to 8 px from it.
pub fn room_beside(room: Rect, bar: Bar, island: Rect) -> Rect {
    let mut room = room;
    match bar {
        Bar::Left => room.min.x = room.min.x.max(island.right() + MARGIN),
        Bar::Right => room.max.x = room.max.x.min(island.left() - MARGIN),
        Bar::Bottom | Bar::Top => {}
    }
    room
}

/// The side toolbars, shown when `shown` (the interface's left and right
/// edges). Returns the windows' room left beside them and the action of
/// their buttons.
#[allow(clippy::too_many_arguments)]
pub(crate) fn bars(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    layout: &Layout,
    toolbar: &mut ToolbarUi,
    settings: &ToolbarSettings,
    panels: &mut Panels,
    mic: &mut MicButton,
    base: &Row,
    room: Rect,
    islands: bool,
    shown: [bool; 2],
) -> (Rect, BarAction) {
    let mut room = room;
    let mut action = BarAction::None;
    let screen = ui.ctx().content_rect();
    let mut edges = [screen.left(), screen.right()];
    let dragging = ToolbarUi::dragging(ui.ctx()) && !settings.locked;
    for (bar, shown) in [(Bar::Left, shown[0]), (Bar::Right, shown[1])] {
        let display = crate::ui::toolbar::structure::Display::for_bar(ui.ctx(), bar, settings.bar(bar, layout), base.editing);
        let buttons = display.buttons();
        if (!shown && !ToolbarUi::editing(ui.ctx())) || (buttons.is_empty() && !dragging && !ToolbarUi::editing(ui.ctx())) {
            continue;
        }
        let style = settings.style(bar).resolve(bar, islands);
        let row = Row { bar, style, ..*base };
        // Keep a destination's layout backend stable while its provisional
        // contents change. An empty Legacy bar must already use its panel;
        // switching from an Area during hover invalidates the snapped target.
        if islands {
            // the chat island raised over the dock's row takes its room
            // from the windows, not from a bar that does not reach it
            let (own, raised) = super::chrome::side_room(ui.ctx(), room);
            let raised = raised.map(|raised| (raised, room));
            let island = column(
                ui,
                p,
                icons,
                buttons,
                toolbar,
                panels,
                mic,
                &row,
                own,
                raised,
                &mut action,
                &display,
            );
            if !buttons.is_empty() {
                room = room_beside(room, bar, island);
                edges[usize::from(bar == Bar::Right)] = if bar == Bar::Left { island.right() } else { island.left() };
            }
        } else {
            let width = classic(ui, p, icons, buttons, toolbar, panels, mic, &row, &mut action, &display);
            match bar {
                Bar::Left => room.min.x += width,
                _ => room.max.x -= width,
            }
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(Id::unique("toolbar_side_edges"), edges));
    (room, action)
}

/// The inner sides of the left and right toolbars last frame (the screen's
/// edges without them), for the hidden interface's edges.
pub fn edges(ctx: &egui::Context) -> [f32; 2] {
    let screen = ctx.content_rect();
    ctx.data(|d| d.get_temp::<[f32; 2]>(Id::unique("toolbar_side_edges")))
        .unwrap_or([screen.left(), screen.right()])
}

/// Where the pop-ups of a side toolbar's buttons open: towards the screen.
fn opens(bar: Bar) -> egui::RectAlign {
    if bar == Bar::Right {
        egui::RectAlign::LEFT
    } else {
        egui::RectAlign::RIGHT
    }
}

/// A side toolbar in a glass island (the islands' look, and the drop zone
/// of an empty one in both looks). Returns the island. `raised` is the
/// place the bottom dock's raised chat island takes in `room`, and the room
/// that keeps clear of it: the bar takes that one only when it would stand
/// over the island, so that a bar that does not reach it stays where it is
/// when the island rises and comes down.
#[allow(clippy::too_many_arguments)]
fn column(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    buttons: &[ToolButton],
    toolbar: &mut ToolbarUi,
    panels: &mut Panels,
    mic: &mut MicButton,
    row: &Row,
    room: Rect,
    raised: Option<(Rect, Rect)>,
    action: &mut BarAction,
    display: &crate::ui::toolbar::structure::Display,
) -> Rect {
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let style = row.style;
    let natural = |square: f32| -> Vec<f32> {
        buttons
            .iter()
            .map(|b| {
                if crate::ui::toolbar::structure::is_separator(&b.command) || crate::ui::toolbar::structure::is_divider(&b.command) {
                    0.0
                } else {
                    natural_width(&ctx, b, style.mode, square, BUTTON_ICON.min(square - 6.0), DOCK_MIC_W)
                }
            })
            .collect()
    };
    let blocks: Vec<_> = buttons
        .iter()
        .map(|button| {
            if crate::ui::toolbar::structure::is_divider(&button.command) {
                Some(crate::ui::toolbar::structure::DIVIDER_SIZE)
            } else if crate::ui::toolbar::structure::is_separator(&button.command) {
                Some(crate::ui::toolbar::structure::style(&ctx, &button.command).size)
            } else {
                crate::ui::toolbar::structure::is_block(&button.command).then(|| super::chrome::height(&ctx, &button.command))
            }
        })
        .collect();
    let scopes = sizing::block_styles(&ctx, buttons, style.sizing, style.mode != ButtonMode::IconsOnly);
    let stretch = crate::ui::toolbar::structure::stretch(&ctx, buttons);
    let place = |room: Rect| geometry_fixed(screen, room, row.bar, style, natural, &blocks, &stretch, &scopes);
    let editing = ToolbarUi::editing(&ctx);
    let mut room = room;
    let mut placed = place(room);
    // (being edited, the bar takes its whole room)
    if let Some((raised, clear)) = raised
        && raised.left() < placed.0.right()
        && placed.0.left() < raised.right()
        && (editing || placed.0.bottom() > raised.top())
    {
        room = clear;
        placed = place(room);
    }
    let (mut island, button, width, heights) = placed;
    let contents = island;
    let widths = sizing::block_widths(buttons, &natural(button), button, &scopes);
    if editing {
        island.min.y = room.top();
        island.max.y = room.bottom();
    }
    let name = if row.bar == Bar::Left { "toolbar_left" } else { "toolbar_right" };
    let split = buttons
        .iter()
        .any(|button| crate::ui::toolbar::structure::is_separator(&button.command))
        || sizing::has_alignment(&ctx, buttons);
    super::islands::area_surface(&ctx, p, name, island, !split && !row.editing, |ui| {
        crate::ui::toolbar::edit_mode::region(ui, p, row.bar, island, buttons.is_empty());
        let target = ui.interact(island, Id::unique(("toolbar_background", name)), Sense::click());
        toolbar.context(&target, p, row.bar, style, None, row.locked);
        let inner = contents.shrink2(Vec2::new(PAD, V_PAD));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::top_down(if row.bar == Bar::Right {
                    egui::Align::Max
                } else {
                    egui::Align::Min
                })),
        );
        child.spacing_mut().item_spacing.y = GAP;
        let look = |b: &ToolButton| ButtonLook {
            size: Vec2::new(
                buttons
                    .iter()
                    .position(|candidate| candidate.command == b.command)
                    .and_then(|index| widths.get(index))
                    .copied()
                    .unwrap_or(width),
                buttons
                    .iter()
                    .position(|candidate| candidate.command == b.command)
                    .and_then(|index| heights.get(index))
                    .copied()
                    .unwrap_or(button),
            ),
            icon: BUTTON_ICON.min(button - 6.0),
            dock: true,
            mode: style.mode,
            left: true,
            opens: opens(row.bar),
            editing: row.editing,
        };
        let rects = toolbar_row(&mut child, p, icons, buttons, look, toolbar, panels, mic, row, action);
        // a button dropped a little past either end still lands in the bar
        let drop = ui.interact(
            if row.editing {
                island
            } else {
                inner.expand2(Vec2::new(PAD, button))
            },
            Id::unique(("toolbar_drop", name)),
            Sense::hover(),
        );
        toolbar.drop_bar(&drop, row.bar, display.original(), &display.full_rects(&rects), row.locked);
    });
    island
}

/// A side toolbar in the Legacy Viewer's look: a panel the height of the
/// screen between the top and bottom bars, its background behind its
/// buttons only, as the Legacy Viewer's `button_panel` (its
/// `bg_opaque_image`, sized to the buttons and placed by the alignment in
/// LLToolBar::updateLayoutAsNeeded, indra/llui/lltoolbar.cpp): the world
/// shows above and below them, all along when they fill the bar (human
/// request, 2026-11-26). Returns its width.
#[allow(clippy::too_many_arguments)]
fn classic(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    buttons: &[ToolButton],
    toolbar: &mut ToolbarUi,
    panels: &mut Panels,
    mic: &mut MicButton,
    row: &Row,
    action: &mut BarAction,
    display: &crate::ui::toolbar::structure::Display,
) -> f32 {
    let style = row.style;
    let labelled = style.mode != ButtonMode::IconsOnly;
    let width = buttons
        .iter()
        .map(|b| {
            if crate::ui::toolbar::structure::is_separator(&b.command) || crate::ui::toolbar::structure::is_divider(&b.command) {
                0.0
            } else {
                natural_width(ui.ctx(), b, style.mode, CLASSIC_SQUARE, 16.0, CLASSIC_SQUARE)
            }
        })
        .fold(CLASSIC_SQUARE, f32::max);
    let panel_w = width + CLASSIC_PAD * 2.0;
    let scopes = sizing::block_styles(ui.ctx(), buttons, style.sizing, labelled);
    let widths = sizing::block_widths(
        buttons,
        &buttons
            .iter()
            .map(|button| {
                if crate::ui::toolbar::structure::is_separator(&button.command)
                    || crate::ui::toolbar::structure::is_divider(&button.command)
                {
                    0.0
                } else {
                    natural_width(ui.ctx(), button, style.mode, CLASSIC_SQUARE, 16.0, CLASSIC_SQUARE)
                }
            })
            .collect::<Vec<_>>(),
        CLASSIC_SQUARE,
        &scopes,
    );
    let id = if row.bar == Bar::Left { "toolbar_left" } else { "toolbar_right" };
    let panel = if row.bar == Bar::Left {
        egui::Panel::left(id)
    } else {
        egui::Panel::right(id)
    };
    let mut glass = None;
    // the background is painted behind the buttons only, below
    let fill = bar_frame(p).fill;
    let mut backing = None;
    let resp = panel
        .frame(
            bar_frame(p)
                .fill(p.bar.gamma_multiply(0.0))
                .inner_margin(egui::Margin::same(CLASSIC_PAD as i8)),
        )
        .exact_size(panel_w)
        .resizable(false)
        // no line along the world: the background stops at the buttons
        .show_separator_line(false)
        .show(ui, |ui| {
            glass = Some((ui.painter().clone(), ui.painter().add(egui::Shape::Noop)));
            let zone = ui.available_rect_before_wrap();
            crate::ui::toolbar::edit_mode::region(ui, p, row.bar, zone, buttons.is_empty());
            let target = ui.interact(zone, Id::unique(("toolbar_background", id)), Sense::click());
            toolbar.context(&target, p, row.bar, style, None, row.locked);
            let n = buttons.len().max(1) as f32;
            let mut heights: Vec<_> = buttons
                .iter()
                .map(|button| {
                    if crate::ui::toolbar::structure::is_divider(&button.command) {
                        crate::ui::toolbar::structure::DIVIDER_SIZE
                    } else if crate::ui::toolbar::structure::is_separator(&button.command) {
                        crate::ui::toolbar::structure::style(ui.ctx(), &button.command).size
                    } else if crate::ui::toolbar::structure::is_block(&button.command) {
                        super::chrome::height(ui.ctx(), &button.command)
                    } else {
                        CLASSIC_H
                    }
                })
                .collect();
            heights = sizing::fit_blocks(
                &heights,
                &buttons
                    .iter()
                    .map(|button| crate::ui::toolbar::structure::fixed_width(&button.command))
                    .collect::<Vec<_>>(),
                &scopes,
                &crate::ui::toolbar::structure::stretch(ui.ctx(), buttons),
                zone.height(),
                GAP,
                CLASSIC_H,
            );
            crate::ui::toolbar::structure::distribute(
                &mut heights,
                &crate::ui::toolbar::structure::stretch(ui.ctx(), buttons),
                zone.height(),
                GAP,
            );
            let aligned = sizing::align_blocks(&mut heights, &scopes, zone.height(), GAP, style.align);
            let height = heights.iter().sum::<f32>() + (n - 1.0) * GAP;
            let top = aligned.map_or_else(
                || sizing::place(style.align, zone.top(), zone.bottom(), height),
                |lead| zone.top() + lead,
            );
            if !buttons.is_empty() {
                backing = Some(Rect::from_min_max(
                    Pos2::new(zone.left() - CLASSIC_PAD, top - CLASSIC_PAD),
                    Pos2::new(zone.right() + CLASSIC_PAD, top + height + CLASSIC_PAD),
                ));
            }
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_max(Pos2::new(zone.left(), top), zone.max))
                    .layout(egui::Layout::top_down(if row.bar == Bar::Right {
                        egui::Align::Max
                    } else {
                        egui::Align::Min
                    })),
            );
            child.spacing_mut().item_spacing.y = GAP;
            let look = |b: &ToolButton| ButtonLook {
                size: Vec2::new(
                    buttons
                        .iter()
                        .position(|candidate| candidate.command == b.command)
                        .and_then(|index| widths.get(index))
                        .copied()
                        .unwrap_or(width),
                    buttons
                        .iter()
                        .position(|candidate| candidate.command == b.command)
                        .and_then(|index| heights.get(index))
                        .copied()
                        .unwrap_or(CLASSIC_H),
                ),
                icon: 16.0,
                dock: false,
                mode: style.mode,
                left: labelled,
                opens: opens(row.bar),
                editing: row.editing,
            };
            let rects = toolbar_row(&mut child, p, icons, buttons, look, toolbar, panels, mic, row, action);
            toolbar.drop_bar(&target, row.bar, display.original(), &display.full_rects(&rects), row.locked);
        });
    if let Some(((painter, slot), backing)) = glass.zip(backing).filter(|_| !row.editing) {
        let backing = backing.intersect(resp.response.rect);
        painter.set(slot, egui::Shape::rect_filled(backing, 0.0, fill));
        crate::ui::widgets::finish_bar_glass(&painter, slot, backing, p);
    }
    resp.response.rect.width()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::toolbar::Align;

    #[test]
    fn separator_lengths_change_height_without_widening_a_side_bar() {
        let screen = screen(1600.0);
        let room = room(screen);
        let fixed = [None, Some(64.0), None, Some(12.0)];
        let (compact, _, width, heights) = geometry_fixed(
            screen,
            room,
            Bar::Left,
            Style::default(),
            |button| vec![button, 0.0, button, 0.0],
            &fixed,
            &[],
            &[],
        );
        assert_eq!(width, 34.0);
        assert_eq!(heights, [34.0, 64.0, 34.0, 12.0]);
        let (expanded, _, wide, heights) = geometry_fixed(
            screen,
            room,
            Bar::Left,
            Style::default(),
            |button| vec![button, 0.0, button, 0.0],
            &fixed,
            &[false, false, false, true],
            &[],
        );
        assert_eq!(wide, width);
        assert_eq!(expanded.height(), room.height());
        assert!(expanded.height() > compact.height());
        assert_eq!(heights[1], 64.0);
    }

    fn screen(w: f32) -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(w, w * 9.0 / 16.0))
    }

    /// The windows' room of the islands at 1536 × 864: under the top
    /// islands, above the dock.
    fn room(s: Rect) -> Rect {
        Rect::from_min_max(Pos2::new(8.0, 52.0), Pos2::new(s.right() - 8.0, s.bottom() - 58.0))
    }

    /// The mockup's left bar: three of the dock's buttons in an island 42
    /// px wide, 8 px from the left edge, centred in the windows' room; the
    /// windows stop 8 px beside it.
    #[test]
    fn a_side_island_stands_8_px_from_its_edge_and_the_windows_beside_it() {
        let s = screen(1536.0);
        let style = Style::default();
        let (island, button, width) = geometry(s, room(s), Bar::Left, style, |b| vec![b; 3]);
        assert_eq!((button, width), (34.0, 34.0));
        assert_eq!(island.left(), MARGIN);
        assert_eq!(island.width(), 42.0);
        assert_eq!(island.height(), 3.0 * 34.0 + 2.0 * GAP + 8.0);
        assert!((island.center().y - room(s).center().y).abs() <= 1.0, "{island:?}");
        assert_eq!(room_beside(room(s), Bar::Left, island).left(), island.right() + MARGIN);
        let (island, ..) = geometry(s, room(s), Bar::Right, style, |b| vec![b; 3]);
        assert_eq!(island.right(), s.right() - MARGIN);
        assert_eq!(room_beside(room(s), Bar::Right, island).right(), island.left() - MARGIN);
    }

    /// Top and bottom alignments hold the island to the room's ends; a
    /// column too tall takes the reduced buttons; labels widen the column
    /// to the widest button.
    #[test]
    fn a_side_island_follows_its_alignment_and_its_widest_label() {
        let s = screen(1280.0);
        let r = room(s);
        let top = Style {
            align: Align::Start,
            ..Default::default()
        };
        let (island, button, _) = geometry(s, r, Bar::Left, top, |b| vec![b; 2]);
        assert_eq!(button, 32.0, "the narrow screen's buttons");
        assert_eq!(island.top(), r.top());
        let bottom = Style {
            align: Align::End,
            ..Default::default()
        };
        let (island, ..) = geometry(s, r, Bar::Left, bottom, |b| vec![b; 2]);
        assert_eq!(island.bottom(), r.bottom());
        let (island, button, _) = geometry(s, r, Bar::Left, top, |b| vec![b; 30]);
        assert_eq!(button, MIN_BUTTON);
        assert!(island.height() <= 30.0 * MIN_BUTTON + 29.0 * GAP + 8.0);
        let (island, _, width) = geometry(s, r, Bar::Left, top, |b| vec![b, 90.0, 120.0]);
        assert_eq!(width, 120.0);
        assert_eq!(island.width(), 128.0);
    }
}

//! The one look shared by every right-click menu of the interface, after
//! Firestorm's LLContextMenu / LLMenuItemGL (indra/llui/llmenugl.cpp,
//! originally LGPL 2.1): flat rows, label on the left, a ▸ for sub-menus,
//! thin separators, unavailable entries greyed out rather than hidden.
//! Aurora adds a Phosphor icon at the start of every row, and draws check
//! marks at the right end (the left column holds the icon).
//!
//! Menus are egui popups (`Popup` / `SubMenu`): a menu closes when an
//! enabled entry is clicked, on Escape, or on a click outside; clicking a
//! greyed entry or a separator keeps it open, as in Firestorm.

use crate::theme::Palette;
use egui::{Color32, CornerRadius, FontId, Id, LayerId, Popup, PopupAnchor, PopupCloseBehavior, PopupKind, Sense, Stroke, Ui, Vec2};

/// Row height and paddings (LLMenuItemGL: 20 px rows with a check column;
/// a bit taller here for the icons).
const ROW_H: f32 = 22.0;
const PAD: f32 = 8.0;
const ICON: f32 = 15.0;
const RIGHT: f32 = 22.0;
const MIN_W: f32 = 180.0;

/// Tooltip of the entries Firestorm has but Aurora does not do yet.
pub const NOT_YET: &str = "Pas encore disponible dans Aurora";

/// What the right end of a row shows.
#[derive(Clone, Copy, PartialEq)]
enum End {
    None,
    Caret,
    Check(bool),
}

/// Rows of the menu opened with [`popup_at`], for the hover sounds of the
/// world menu: (menu layer, rows drawn, hovered row).
#[derive(Clone, Copy)]
struct RowTrack {
    layer: LayerId,
    rows: usize,
    hovered: Option<usize>,
}

fn rows_id() -> Id {
    Id::new("aurora_menu_rows")
}

/// Menu style: the popup and sub-menu frames (`Frame::menu`) are built
/// from it, so every level gets the panel color and border.
fn style(p: &Palette) -> egui::style::StyleModifier {
    let (fill, border) = (p.panel, p.raised);
    egui::style::StyleModifier::new(move |s| {
        egui::containers::menu::menu_style(s);
        s.spacing.item_spacing = Vec2::ZERO;
        s.spacing.menu_margin = egui::Margin::same(4);
        s.visuals.window_fill = fill;
        s.visuals.window_stroke = Stroke::new(1.0, border);
        s.visuals.menu_corner_radius = CornerRadius::same(3);
    })
}

/// Right-click menu of a widget (replaces `Response::context_menu`).
/// Returns whether the menu is open.
pub fn context_menu(resp: &egui::Response, p: &Palette, body: impl FnOnce(&mut Ui)) -> bool {
    // below-right of the pointer like the world menu, pushed back into the
    // window by the area's constraint instead of flipping above it
    Popup::context_menu(resp)
        .align(egui::RectAlign::BOTTOM_START)
        .align_alternatives(&[])
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .style(style(p))
        .show(body)
        .is_some()
}

/// Menu opened at a screen position, not attached to a widget (the world
/// right click). `open` is cleared when it closes. Returns the hovered row
/// (top level only) for the slice sounds.
pub fn popup_at(ctx: &egui::Context, id: Id, pos: egui::Pos2, open: &mut bool, p: &Palette, body: impl FnOnce(&mut Ui)) -> Option<usize> {
    let layer = LayerId::new(egui::Order::Foreground, id);
    ctx.data_mut(|d| {
        d.insert_temp(
            rows_id(),
            RowTrack {
                layer,
                rows: 0,
                hovered: None,
            },
        )
    });
    // top-left corner at the pointer, moved back into the window when it
    // does not fit (LLMenuGL::showPopup), with last frame's size
    let mut pos = pos;
    if let Some(r) = ctx.read_response(id) {
        let screen = ctx.content_rect();
        pos.x = pos.x.min(screen.right() - r.rect.width()).max(screen.left());
        pos.y = pos.y.min(screen.bottom() - r.rect.height()).max(screen.top());
    }
    Popup::new(id, ctx.clone(), PopupAnchor::Position(pos), LayerId::background())
        .kind(PopupKind::Menu)
        .align(egui::RectAlign::BOTTOM_START)
        .align_alternatives(&[])
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .style(style(p))
        .open_bool(open)
        .show(body);
    let hovered = ctx.data(|d| d.get_temp::<RowTrack>(rows_id())).and_then(|t| t.hovered);
    ctx.data_mut(|d| d.remove::<RowTrack>(rows_id()));
    hovered
}

/// One row: icon, label, right end. Hover highlight like a menu item.
fn row(ui: &mut Ui, p: &Palette, icon: &str, label: &str, enabled: bool, end: End, open: bool) -> egui::Response {
    let font = FontId::proportional(12.5);
    let ink = if enabled { p.ink } else { p.muted_dim };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    let natural = (PAD + ICON + PAD + galley.size().x + RIGHT + PAD).max(MIN_W);
    // natural width while egui measures the menu, then the menu's width
    let w = if ui.is_sizing_pass() {
        natural
    } else {
        ui.available_width().max(natural)
    };
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, ROW_H), sense);
    let hot = enabled && (resp.hovered() || open);
    track(ui, hot && resp.hovered());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let painter = ui.painter();
    if hot {
        painter.rect_filled(rect, CornerRadius::same(2), p.violet.gamma_multiply(0.35));
    }
    let tint = if !enabled {
        p.muted_dim
    } else if hot {
        p.ink
    } else {
        p.violet_light
    };
    let icon_rect = egui::Rect::from_center_size(rect.left_center() + Vec2::new(PAD + ICON * 0.5, 0.0), Vec2::splat(ICON));
    paint_icon(painter, icon, icon_rect, tint);
    painter.galley(
        egui::pos2(rect.left() + PAD + ICON + PAD, rect.center().y - galley.size().y * 0.5),
        galley,
        ink,
    );
    let end_rect = egui::Rect::from_center_size(rect.right_center() - Vec2::new(PAD + 6.0, 0.0), Vec2::splat(12.0));
    match end {
        End::None | End::Check(false) => {}
        End::Caret => paint_icon(painter, "caret-right", end_rect, if enabled { p.muted } else { p.muted_dim }),
        End::Check(true) => paint_icon(painter, "check", end_rect, if enabled { p.violet_light } else { p.muted_dim }),
    }
    resp
}

fn paint_icon(painter: &egui::Painter, icon: &str, rect: egui::Rect, tint: Color32) {
    match super::icons::global(icon) {
        Some(t) => {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(t.id(), rect, uv, tint);
        }
        // no icon theme: a small dot keeps the column aligned
        None => {
            painter.circle_filled(rect.center(), 2.0, tint);
        }
    }
}

/// Count the top-level rows of the [`popup_at`] menu (hover sounds).
fn track(ui: &Ui, hovered: bool) {
    let layer = ui.layer_id();
    ui.ctx().data_mut(|d| {
        if let Some(mut t) = d.get_temp::<RowTrack>(rows_id())
            && t.layer == layer
        {
            if hovered {
                t.hovered = Some(t.rows);
            }
            t.rows += 1;
            d.insert_temp(rows_id(), t);
        }
    });
}

/// Enabled entry; true when clicked (the menu closes).
pub fn item(ui: &mut Ui, p: &Palette, icon: &str, label: &str) -> bool {
    item_if(ui, p, icon, label, true)
}

/// Entry greyed out when `enabled` is false (Firestorm's on_enable).
pub fn item_if(ui: &mut Ui, p: &Palette, icon: &str, label: &str, enabled: bool) -> bool {
    let clicked = row(ui, p, icon, label, enabled, End::None, false).clicked();
    if clicked {
        ui.close();
    }
    clicked && enabled
}

/// Firestorm entry Aurora does not do yet: greyed, with a tooltip.
pub fn todo(ui: &mut Ui, p: &Palette, icon: &str, label: &str) {
    row(ui, p, icon, label, false, End::None, false).on_hover_text(NOT_YET);
}

/// Check entry (menu_item_check); true when clicked (the menu closes).
pub fn check(ui: &mut Ui, p: &Palette, icon: &str, label: &str, checked: bool, enabled: bool) -> bool {
    let clicked = row(ui, p, icon, label, enabled, End::Check(checked), false).clicked();
    if clicked {
        ui.close();
    }
    clicked && enabled
}

/// Check entry that leaves the menu open (several options in a row, like
/// the blocked properties of a block list entry).
pub fn toggle(ui: &mut Ui, p: &Palette, icon: &str, label: &str, on: &mut bool) -> bool {
    let clicked = row(ui, p, icon, label, true, End::Check(*on), false).clicked();
    if clicked {
        *on = !*on;
    }
    clicked
}

/// Entry opening a sub-menu on hover (▸). A disabled sub-menu does not open.
pub fn submenu(ui: &mut Ui, p: &Palette, icon: &str, label: &str, enabled: bool, body: impl FnOnce(&mut Ui)) {
    let id = ui.next_auto_id();
    let open = enabled && Popup::is_id_open(ui.ctx(), egui::containers::menu::SubMenu::id_from_widget_id(id));
    let resp = row(ui, p, icon, label, enabled, End::Caret, open);
    if enabled {
        egui::containers::menu::SubMenu::new().show(ui, &resp, body);
    }
}

/// Entry whose icon is a color dot (the mini-map marks); true when clicked.
pub fn swatch(ui: &mut Ui, p: &Palette, color: Color32, label: &str) -> bool {
    let resp = row(ui, p, "", label, true, End::None, false);
    let c = resp.rect.left_center() + Vec2::new(PAD + ICON * 0.5, 0.0);
    ui.painter().circle_filled(c, ICON * 0.38, color);
    ui.painter().circle_stroke(c, ICON * 0.38, Stroke::new(1.0, p.raised));
    if resp.clicked() {
        ui.close();
    }
    resp.clicked()
}

/// Thin line between groups of entries.
pub fn separator(ui: &mut Ui, p: &Palette) {
    let w = if ui.is_sizing_pass() { MIN_W } else { ui.available_width() };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 7.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range().shrink(4.0), rect.center().y, Stroke::new(1.0, p.raised));
}

/// Muted line that is not an action (an avatar's complexity…).
pub fn info(ui: &mut Ui, p: &Palette, icon: &str, text: &str) {
    let font = FontId::proportional(11.5);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, p.muted);
    let natural = (PAD + ICON + PAD + galley.size().x + PAD).max(MIN_W);
    let w = if ui.is_sizing_pass() {
        natural
    } else {
        ui.available_width().max(natural)
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, ROW_H - 2.0), Sense::hover());
    let icon_rect = egui::Rect::from_center_size(rect.left_center() + Vec2::new(PAD + ICON * 0.5, 0.0), Vec2::splat(ICON - 2.0));
    paint_icon(ui.painter(), icon, icon_rect, p.muted);
    ui.painter().galley(
        egui::pos2(rect.left() + PAD + ICON + PAD, rect.center().y - galley.size().y * 0.5),
        galley,
        p.muted,
    );
}

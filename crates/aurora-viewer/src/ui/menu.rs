//! The one look shared by every right-click menu of the interface, after
//! Firestorm's LLContextMenu / LLMenuItemGL (indra/llui/llmenugl.cpp,
//! originally LGPL 2.1): flat rows, label on the left, a ▸ for sub-menus,
//! thin separators, unavailable entries greyed out rather than hidden.
//! Aurora adds a Tabler icon at the start of every row, and draws check
//! marks at the right end (the left column holds the icon).
//!
//! Menus are egui popups (`Popup` / `SubMenu`): a menu closes when an
//! enabled entry is clicked, on Escape, or on a click outside; clicking a
//! greyed entry or a separator keeps it open, as in Firestorm.

use crate::theme::Palette;
use egui::{Color32, FontId, Id, LayerId, Popup, PopupAnchor, PopupCloseBehavior, PopupKind, Sense, Stroke, Ui, Vec2};

/// Row height and paddings (LLMenuItemGL: 20 px rows with a check column;
/// a bit taller here for the icons).
const ROW_H: f32 = 22.0;
const PAD: f32 = 8.0;
const ICON: f32 = 15.0;
const RIGHT: f32 = 22.0;
const MIN_W: f32 = 180.0;

/// Tooltip of the entries Firestorm has but Aurora does not do yet.
pub fn not_yet() -> &'static str {
    aurora_i18n::tr!("common-not-available-in-aurora-yet")
}

/// What the right end of a row shows.
#[derive(Clone, Copy, PartialEq)]
enum End<'a> {
    None,
    Caret,
    Check(bool),
    /// The keys of the entry's shortcut.
    Keys(&'a str),
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
        s.visuals.menu_corner_radius = s.visuals.window_corner_radius;
    })
}

/// The rectangle and corner radius of a stock pop-up frame in `shape`: the
/// theme's window face inside its `border`. The panels drawn with
/// `widgets::overlay` have another border and bring their own glass.
fn stock_frame(shape: &egui::Shape, faces: [Color32; 2], border: Color32) -> Option<(egui::Rect, f32)> {
    match shape {
        egui::Shape::Rect(r) if faces.contains(&r.fill) && r.stroke.color == border => Some((r.rect, f32::from(r.corner_radius.nw))),
        egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| stock_frame(s, faces, border)),
        _ => None,
    }
}

/// The windows' glass under every pop-up of the frame: menus and their
/// sub-menus, right-click menus, the lists of the combo boxes. Each one is
/// alone in its layer and egui paints its frame first there; the blur of the
/// world and the windows' tint go over that face, inside its border. Called
/// in world, once the interface of the frame is built: one place for all the
/// menus instead of one call per menu. An opaque skin (panel_alpha = 255)
/// keeps its plain menus.
pub fn glaze(ctx: &egui::Context, p: &Palette) {
    if p.panel_alpha == 255 {
        return;
    }
    let tint = Color32::from_rgba_unmultiplied(p.panel.r(), p.panel.g(), p.panel.b(), p.panel_alpha);
    let faces = [tint, p.panel];
    let layers: Vec<LayerId> = ctx
        .memory(|m| m.areas().visible_layer_ids())
        .into_iter()
        .filter(|layer| layer.order == egui::Order::Foreground)
        .collect();
    ctx.graphics_mut(|graphics| {
        for layer in layers {
            let Some(list) = graphics.get_mut(layer) else {
                continue;
            };
            if list.is_empty() {
                continue;
            }
            list.mutate_shape(egui::layers::ShapeIdx(0), |first| {
                if let Some((rect, radius)) = stock_frame(&first.shape, faces, p.raised) {
                    let frame = std::mem::replace(&mut first.shape, egui::Shape::Noop);
                    first.shape = egui::Shape::Vec(vec![frame, aurora_render::glass_panel(rect.shrink(1.0), radius, tint)]);
                }
            });
        }
    });
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

/// Menu opened by a click on the widget `resp` (a "more actions" button, a
/// status light), with the rows and the frame of the right-click menus at
/// every level. `close` is egui's rule for the clicks inside it: a stock
/// `menu_button` closes on any of them, `CloseOnClickOutside` leaves the menu
/// open until an entry closes it. Returns whether the menu is open.
pub fn dropdown(resp: &egui::Response, p: &Palette, close: PopupCloseBehavior, body: impl FnOnce(&mut Ui)) -> bool {
    use egui::containers::menu::MenuConfig;
    // the tag hands the same style and rule to the sub-menus
    let config = MenuConfig::new().close_behavior(close).style(style(p));
    Popup::menu(resp)
        .close_behavior(close)
        .style(style(p))
        .info(egui::UiStackInfo::new(egui::UiKind::Menu).with_tag_value(MenuConfig::MENU_CONFIG_TAG, config))
        .show(body)
        .is_some()
}

/// `button` opening a [`dropdown`] that closes on any click, like a stock
/// `menu_button`.
pub fn button(ui: &mut Ui, p: &Palette, button: egui::Button<'_>, body: impl FnOnce(&mut Ui)) -> egui::Response {
    let resp = ui.add(button);
    dropdown(&resp, p, PopupCloseBehavior::CloseOnClick, body);
    resp
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
    let keys = match end {
        End::Keys(keys) => {
            let color = if enabled { p.muted } else { p.muted_dim };
            Some(ui.painter().layout_no_wrap(keys.to_owned(), FontId::proportional(11.5), color))
        }
        _ => None,
    };
    let right = keys.as_ref().map_or(RIGHT, |k| k.size().x + PAD * 2.0);
    let natural = (PAD + ICON + PAD + galley.size().x + right + PAD).max(MIN_W);
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
        painter.rect_filled(rect, ui.visuals().window_corner_radius, p.violet.gamma_multiply(0.35));
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
        End::Caret => paint_icon(painter, "caret-right-filled", end_rect, if enabled { p.muted } else { p.muted_dim }),
        End::Check(true) => paint_icon(painter, "check", end_rect, if enabled { p.violet_light } else { p.muted_dim }),
        End::Keys(_) => {
            if let Some(keys) = keys {
                let pos = egui::pos2(rect.right() - PAD - keys.size().x, rect.center().y - keys.size().y * 0.5);
                painter.galley(pos, keys, p.muted);
            }
        }
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

/// Entry with the keys of its shortcut at the right end; true when clicked
/// (the menu closes).
pub fn item_keys(ui: &mut Ui, p: &Palette, icon: &str, label: &str, keys: &str) -> bool {
    let clicked = row(ui, p, icon, label, true, End::Keys(keys), false).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// Entry with a tooltip (an empty `tip` shows none), greyed out when
/// `enabled` is false; true when clicked (the menu closes).
pub fn item_tip(ui: &mut Ui, p: &Palette, icon: &str, label: &str, enabled: bool, tip: &str) -> bool {
    let resp = row(ui, p, icon, label, enabled, End::None, false);
    let clicked = resp.clicked();
    if !tip.is_empty() {
        resp.on_hover_text(tip);
    }
    if clicked {
        ui.close();
    }
    clicked && enabled
}

/// Firestorm entry Aurora does not do yet: greyed, with a tooltip.
pub fn todo(ui: &mut Ui, p: &Palette, icon: &str, label: &str) {
    row(ui, p, icon, label, false, End::None, false).on_hover_text(not_yet());
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
    submenu_heading(ui, p, icon, label, enabled, body);
}

/// [`submenu`] that returns its own row, for a heading that is also an
/// action when clicked (the import menu).
pub fn submenu_heading(ui: &mut Ui, p: &Palette, icon: &str, label: &str, enabled: bool, body: impl FnOnce(&mut Ui)) -> egui::Response {
    let id = ui.next_auto_id();
    let open = enabled && Popup::is_id_open(ui.ctx(), egui::containers::menu::SubMenu::id_from_widget_id(id));
    let resp = row(ui, p, icon, label, enabled, End::Caret, open);
    if enabled {
        egui::containers::menu::SubMenu::new().show(ui, &resp, body);
    }
    resp
}

/// Entry whose icon is a color dot (the mini-map marks); true when clicked.
pub fn swatch(ui: &mut Ui, p: &Palette, color: Color32, label: &str) -> bool {
    let resp = row(ui, p, "", label, true, End::None, false);
    dot(ui, p, &resp, color);
    if resp.clicked() {
        ui.close();
    }
    resp.clicked()
}

/// Color dot entry with a check mark, which leaves the menu open (one choice
/// among several, like the availability of the status light); true when
/// clicked.
pub fn swatch_toggle(ui: &mut Ui, p: &Palette, color: Color32, label: &str, on: bool) -> bool {
    let resp = row(ui, p, "", label, true, End::Check(on), false);
    dot(ui, p, &resp, color);
    resp.clicked()
}

/// The color dot of a swatch row, in the icon column.
fn dot(ui: &Ui, p: &Palette, row: &egui::Response, color: Color32) {
    let c = row.rect.left_center() + Vec2::new(PAD + ICON * 0.5, 0.0);
    ui.painter().circle_filled(c, ICON * 0.38, color);
    ui.painter().circle_stroke(c, ICON * 0.38, Stroke::new(1.0, p.raised));
}

/// Thin line between groups of entries.
pub fn separator(ui: &mut Ui, p: &Palette) {
    let w = if ui.is_sizing_pass() { MIN_W } else { ui.available_width() };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 7.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range().shrink(4.0), rect.center().y, super::widgets::divider_stroke(p));
}

/// Muted line that is not an action (an avatar's complexity…).
pub fn info(ui: &mut Ui, p: &Palette, icon: &str, text: &str) {
    info_line(
        ui,
        icon,
        text,
        FontId::proportional(11.5),
        ICON - 2.0,
        ROW_H - 2.0,
        p.muted,
        p.muted,
    );
}

/// A line that is not an action, at a row's size, its icon and its text in
/// their own colours (a parcel's right, red when forbidden). Returns its
/// response, for a tooltip.
pub fn info_colored(ui: &mut Ui, icon: &str, text: &str, icon_color: Color32, color: Color32) -> egui::Response {
    info_line(ui, icon, text, FontId::proportional(12.5), ICON, ROW_H, icon_color, color)
}

#[allow(clippy::too_many_arguments)]
fn info_line(
    ui: &mut Ui,
    icon: &str,
    text: &str,
    font: FontId,
    icon_size: f32,
    h: f32,
    icon_color: Color32,
    color: Color32,
) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let natural = (PAD + ICON + PAD + galley.size().x + PAD).max(MIN_W);
    let w = if ui.is_sizing_pass() {
        natural
    } else {
        ui.available_width().max(natural)
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    let icon_rect = egui::Rect::from_center_size(rect.left_center() + Vec2::new(PAD + ICON * 0.5, 0.0), Vec2::splat(icon_size));
    paint_icon(ui.painter(), icon, icon_rect, icon_color);
    ui.painter().galley(
        egui::pos2(rect.left() + PAD + ICON + PAD, rect.center().y - galley.size().y * 0.5),
        galley,
        color,
    );
    resp
}

/// The title of a group of rows, in small capitals (the parcel's rights,
/// the recent places of the place menu).
pub fn heading(ui: &mut Ui, p: &Palette, text: &str) {
    let font = FontId::new(10.5, super::fonts::semibold(ui.ctx()));
    let galley = ui.painter().layout_no_wrap(text.to_uppercase(), font, p.muted_dim);
    let natural = (PAD + galley.size().x + PAD).max(MIN_W);
    let w = if ui.is_sizing_pass() {
        natural
    } else {
        ui.available_width().max(natural)
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 21.0), Sense::hover());
    ui.painter().galley(
        egui::pos2(rect.left() + PAD, rect.bottom() - 3.0 - galley.size().y),
        galley,
        p.muted_dim,
    );
}

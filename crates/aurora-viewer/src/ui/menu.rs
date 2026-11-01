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
//!
//! A menu wrapped in [`searchable`] (the wolf menu) starts with a search
//! field: while it holds a query, every row checks itself against it, and
//! the sub-menus list their matching entries in place, under their path;
//! the arrow keys select among them and Enter clicks the selected one.

use crate::theme::Palette;
use egui::{Color32, FontId, Id, LayerId, Popup, PopupAnchor, PopupCloseBehavior, PopupKind, Sense, Stroke, Ui, Vec2};

/// Row height and paddings (LLMenuItemGL: 20 px rows with a check column;
/// a bit taller here for the icons).
const ROW_H: f32 = 22.0;
const PAD: f32 = 8.0;
const ICON: f32 = 15.0;
const RIGHT: f32 = 22.0;
const MIN_W: f32 = 180.0;
/// Room of the check mark after the keys of a [`check_keys`] entry.
const CHECK_ROOM: f32 = RIGHT + 4.0;
/// Where the text of a row starts: a line under an entry (its value) lines
/// up with it.
pub const TEXT_INDENT: f32 = PAD + ICON + PAD;

/// Tooltip of the entries Firestorm has but Aurora does not do yet.
pub fn not_yet() -> &'static str {
    aurora_i18n::tr!("common-not-available-in-aurora-yet")
}

/// What the right end of a row shows.
#[derive(Clone, Copy, PartialEq)]
enum End<'a> {
    None,
    Caret,
    Down,
    Check(bool),
    /// The keys of the entry's shortcut, or a detail of the entry (where a
    /// landmark leads), muted at the right end.
    Keys(&'a str),
    /// A check entry with the keys of its shortcut before the check mark.
    CheckKeys(bool, &'a str),
    /// A small label in the brand teal (a suggestion).
    Tag(&'a str),
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
    Id::unique("aurora_menu_rows")
}

/// Menu style: the popup and sub-menu frames (`Frame::menu`) are built
/// from it, so every level gets the panel color and border.
fn style(p: &Palette) -> egui::style::StyleModifier {
    let (fill, border) = (p.panel, frame_border(p));
    egui::style::StyleModifier::new(move |s| {
        egui::containers::menu::menu_style(s);
        s.spacing.item_spacing = Vec2::ZERO;
        s.spacing.menu_margin = egui::Margin::same(4);
        s.visuals.window_fill = fill;
        s.visuals.window_stroke = Stroke::new(1.0, border);
        s.visuals.menu_corner_radius = s.visuals.window_corner_radius;
    })
}

/// The border of the menus' frames: the raised neutral, or the light one of
/// high contrast (`widgets::border_stroke`).
fn frame_border(p: &Palette) -> Color32 {
    crate::ui::widgets::border_stroke(p, Stroke::new(1.0, p.raised)).color
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
    let distinct = ctx
        .data(|d| d.get_temp::<(u64, Vec<LayerId>)>(Id::unique("distinct_menu_surfaces")))
        .filter(|(frame, _)| *frame == ctx.cumulative_frame_nr())
        .map(|(_, layers)| layers)
        .unwrap_or_default();
    if p.panel_alpha == 255 && distinct.is_empty() {
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
                if let Some((rect, radius)) = stock_frame(&first.shape, faces, frame_border(p)) {
                    let frame = std::mem::replace(&mut first.shape, egui::Shape::Noop);
                    let mut shapes = vec![frame];
                    if p.panel_alpha < 255 {
                        let tint = if distinct.contains(&layer) {
                            Color32::from_rgba_unmultiplied(p.bar.r(), p.bar.g(), p.bar.b(), p.panel_alpha.max(245))
                        } else {
                            tint
                        };
                        shapes.push(aurora_render::glass_panel(rect.shrink(1.0), radius, tint));
                    }
                    if distinct.contains(&layer) {
                        shapes.push(egui::Shape::rect_stroke(
                            rect,
                            radius,
                            Stroke::new(1.0, p.muted_dim),
                            egui::StrokeKind::Inside,
                        ));
                    }
                    first.shape = egui::Shape::Vec(shapes);
                }
            });
        }
    });
}

/// A denser menu face and clearer edge when its parent uses the same panel face.
/// Registration keeps all surface painting in the shared menu glass pass.
pub fn distinct_surface(ui: &Ui) {
    let frame = ui.ctx().cumulative_frame_nr();
    ui.ctx().data_mut(|d| {
        let surfaces = d.get_temp_mut_or_default::<(u64, Vec<LayerId>)>(Id::unique("distinct_menu_surfaces"));
        if surfaces.0 != frame {
            *surfaces = (frame, Vec::new());
        }
        if !surfaces.1.contains(&ui.layer_id()) {
            surfaces.1.push(ui.layer_id());
        }
    });
}

/// Right-click menu of a widget (replaces `Response::context_menu`).
/// Returns whether the menu is open.
pub fn context_menu(resp: &egui::Response, p: &Palette, body: impl FnOnce(&mut Ui)) -> bool {
    context_menu_popup(Popup::context_menu(resp), p, body)
}

/// [`context_menu`] of a widget that also opens a [`dropdown`] on a left
/// click: the right-click menu needs an id of its own (egui keys both on
/// the widget's).
pub fn context_menu_of_button(resp: &egui::Response, p: &Palette, body: impl FnOnce(&mut Ui)) -> bool {
    context_menu_popup(Popup::context_menu(resp).id(resp.id.with("context_menu")), p, body)
}

/// [`context_menu_of_button`] that `also` opens at the pointer: a right
/// click on the buttons laid over `resp`, which take it (the system island's
/// audio controls).
pub fn context_menu_also(resp: &egui::Response, p: &Palette, also: bool, body: impl FnOnce(&mut Ui)) -> bool {
    let mut popup = Popup::context_menu(resp).id(resp.id.with("context_menu"));
    if also {
        popup = popup.open_memory(egui::SetOpenCommand::Bool(true));
    }
    context_menu_popup(popup, p, body)
}

/// A context menu owned by a containing view instead of a transient row.
/// Its caller opens `id` in popup memory and retains the original position.
pub fn context_menu_at(ui: &Ui, id: Id, pos: egui::Pos2, p: &Palette, body: impl FnOnce(&mut Ui)) -> bool {
    context_menu_popup(
        Popup::new(id, ui.ctx().clone(), pos, ui.layer_id())
            .kind(PopupKind::Menu)
            .layout(egui::Layout::top_down_justified(egui::Align::Min))
            .open_memory(None),
        p,
        body,
    )
}

/// Moving or resizing any window dismisses menus at once, as the Legacy Viewer's
/// LLMenuHolderGL::handleMouseDown does before LLDragHandle/LLResizeBar take
/// capture (indra/llui/llmenugl.cpp, lldraghandle.cpp, llresizebar.cpp). Check
/// before painting, including menus drawn before the window being moved or
/// resized and menus with external state.
pub(crate) fn dismiss_on_window_change(ctx: &egui::Context) -> bool {
    let changed = ctx
        .data(|d| d.get_temp::<u64>(Id::unique("menus_native_window_change")))
        .is_some_and(|frame| ctx.cumulative_frame_nr() <= frame);
    if !changed && !super::widgets::window_dragging(ctx) {
        return false;
    }
    Popup::close_all(ctx);
    true
}

/// Native move/resize events arrive between egui frames. Keep dismissal through
/// the next frame and all its layout passes, including externally owned menus.
pub(crate) fn window_geometry_changed(ctx: &egui::Context) {
    let through = ctx.cumulative_frame_nr() + 1;
    ctx.data_mut(|d| d.insert_temp(Id::unique("menus_native_window_change"), through));
    Popup::close_all(ctx);
}

fn context_menu_popup(popup: Popup<'_>, p: &Palette, body: impl FnOnce(&mut Ui)) -> bool {
    if dismiss_on_window_change(popup.ctx()) {
        return false;
    }
    // below-right of the pointer like the world menu, pushed back into the
    // window by the area's constraint instead of flipping above it
    popup
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
    dropdown_placed(resp, p, close, None, body)
}

/// [`dropdown`] at a given side of the widget and distance from it: the
/// menus of the bars' islands open 10 points under their island, those at
/// its right end aligned on its right (`RectAlign::BOTTOM_END`).
pub fn dropdown_placed(
    resp: &egui::Response,
    p: &Palette,
    close: PopupCloseBehavior,
    place: Option<(egui::RectAlign, f32)>,
    body: impl FnOnce(&mut Ui),
) -> bool {
    if dismiss_on_window_change(&resp.ctx) {
        return false;
    }
    // a click on a voice row's slider or mute button leaves the menu open
    let close = if close == PopupCloseBehavior::CloseOnClick && clicks_kept_row(&resp.ctx, Popup::default_response_id(resp)) {
        PopupCloseBehavior::CloseOnClickOutside
    } else {
        close
    };
    placed(Popup::menu(resp), p, close, place, body)
}

/// [`dropdown_placed`] opened by its caller instead of by a click on
/// `resp` (a button held down, a right click): `open` opens it this frame.
/// It stays open until a click outside or an entry closes it. Returns
/// whether the menu is open.
pub fn dropdown_held(
    resp: &egui::Response,
    p: &Palette,
    open: bool,
    place: Option<(egui::RectAlign, f32)>,
    body: impl FnOnce(&mut Ui),
) -> bool {
    if dismiss_on_window_change(&resp.ctx) {
        return false;
    }
    let popup = Popup::menu(resp).open_memory(open.then_some(egui::SetOpenCommand::Bool(true)));
    placed(popup, p, PopupCloseBehavior::CloseOnClickOutside, place, body)
}

fn placed(
    popup: Popup<'_>,
    p: &Palette,
    close: PopupCloseBehavior,
    place: Option<(egui::RectAlign, f32)>,
    body: impl FnOnce(&mut Ui),
) -> bool {
    use egui::containers::menu::MenuConfig;
    // the tag hands the same style and rule to the sub-menus
    let config = MenuConfig::new().close_behavior(close).style(style(p));
    let mut popup = popup;
    if let Some((align, gap)) = place {
        popup = popup.align(align).gap(gap);
    }
    popup
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
    if dismiss_on_window_change(ctx) {
        *open = false;
        return None;
    }
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

/// What the left of a row shows.
#[derive(Clone, Copy)]
enum Lead<'a> {
    /// A Tabler icon, by name.
    Icon(&'a str),
    /// A presence mark, whose shape remains readable without its colour.
    Status(super::presence::Shape, Color32),
    /// A picture in its own colours (an emoji), or a character when it has
    /// none.
    Picture(Option<&'a egui::TextureHandle>, &'a str),
}

/// A row of a list of suggestions shown under the keyboard (the emoji of a
/// `:code` being typed): its `picture`, the `label`, a muted `detail` at the
/// right end, lit like a hovered entry while `selected` (the arrows' choice).
/// It does not close anything: the list belongs to its field.
pub fn picture_row(
    ui: &mut Ui,
    p: &Palette,
    picture: Option<&egui::TextureHandle>,
    fallback: &str,
    label: &str,
    detail: &str,
    selected: bool,
) -> egui::Response {
    row_lead(ui, p, Lead::Picture(picture, fallback), label, true, End::Keys(detail), selected)
}

/// A row of a list holding the place we are in (the arrows' teleport
/// list): its `detail` muted at the right end; the `current` row is lit
/// like a hovered entry and takes no click.
pub fn current_row(ui: &mut Ui, p: &Palette, icon: &str, label: &str, detail: &str, current: bool) -> egui::Response {
    let sense = if current { Sense::hover() } else { Sense::click() };
    row_lead_sensed(ui, p, Lead::Icon(icon), label, true, End::Keys(detail), current, sense)
}

/// [`picture_row`] with a Tabler icon: a resident to mention, a gesture.
pub fn suggestion_row(ui: &mut Ui, p: &Palette, icon: &str, label: &str, detail: &str, selected: bool) -> egui::Response {
    row_lead(ui, p, Lead::Icon(icon), label, true, End::Keys(detail), selected)
}

/// One row: icon, label, right end. Hover highlight like a menu item.
fn row(ui: &mut Ui, p: &Palette, icon: &str, label: &str, enabled: bool, end: End, open: bool) -> egui::Response {
    row_lead(ui, p, Lead::Icon(icon), label, enabled, end, open)
}

fn row_lead(ui: &mut Ui, p: &Palette, lead: Lead, label: &str, enabled: bool, end: End, open: bool) -> egui::Response {
    row_lead_sensed(
        ui,
        p,
        lead,
        label,
        enabled,
        end,
        open,
        if enabled { Sense::click() } else { Sense::hover() },
    )
}

#[allow(clippy::too_many_arguments)]
fn row_lead_sensed(ui: &mut Ui, p: &Palette, lead: Lead, label: &str, enabled: bool, end: End, open: bool, sense: Sense) -> egui::Response {
    if !filter_row(ui, p, label) {
        return hidden(ui);
    }
    let keyed = if enabled && sense.senses_click() { keyboard_row() } else { None };
    let font = FontId::proportional(12.5);
    let ink = if enabled { p.ink } else { p.muted_dim };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    let keys = match end {
        End::Keys(keys) | End::CheckKeys(_, keys) => {
            let color = if enabled { p.muted } else { p.muted_dim };
            Some(ui.painter().layout_no_wrap(keys.to_owned(), FontId::proportional(11.5), color))
        }
        End::Tag(tag) => Some(ui.painter().layout_no_wrap(tag.to_owned(), FontId::proportional(10.5), p.teal)),
        _ => None,
    };
    let tag_room = match end {
        End::Tag(_) => TAG_PAD * 2.0,
        End::CheckKeys(..) => CHECK_ROOM,
        _ => 0.0,
    };
    let right = keys.as_ref().map_or(RIGHT, |k| k.size().x + PAD * 2.0 + tag_room);
    let natural = (PAD + ICON + PAD + galley.size().x + right + PAD).max(MIN_W);
    // natural width while egui measures the menu, then the menu's width
    let w = if ui.is_sizing_pass() {
        natural
    } else {
        ui.available_width().max(natural)
    };
    let text_height = galley.size().y.max(keys.as_ref().map_or(0.0, |keys| keys.size().y));
    // a whole number of points: a font with a tall line (OpenDyslexic)
    // makes the row taller than ROW_H, and a fractional height left its
    // letters 0.6 px off its middle at 1.25
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(w, ROW_H.max((text_height + 6.0).ceil())), sense);
    // a filtered row lights up when the arrow keys or the pointer select it,
    // so that only one row is lit; Enter clicks the row it was lit on, never
    // also the one the pointer reaches in the same frame
    let hot = match keyed {
        Some(k) => {
            let selected = k.selected || (resp.hovered() && ui.input(|i| i.pointer.delta() != Vec2::ZERO) && point_row(k.index));
            if selected && k.scroll {
                resp.scroll_to_me(None);
            }
            if k.selected && k.activate {
                resp.flags.insert(egui::response::Flags::FAKE_PRIMARY_CLICKED);
            }
            selected || open
        }
        None => enabled && (resp.hovered() || open),
    };
    track(ui, hot && resp.hovered());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let painter = ui.painter();
    let mut face = p.panel;
    if hot {
        let wash = p.violet.gamma_multiply(0.35);
        painter.rect_filled(rect, ui.visuals().window_corner_radius, wash);
        face = face.blend(wash);
    }
    let tint = if !enabled {
        p.muted_dim
    } else if hot {
        p.ink
    } else {
        p.violet_light
    };
    let icon_rect = egui::Rect::from_center_size(rect.left_center() + Vec2::new(PAD + ICON * 0.5, 0.0), Vec2::splat(ICON));
    match lead {
        Lead::Icon(icon) => paint_icon(painter, icon, icon_rect, tint),
        Lead::Status(shape, color) => {
            // Cuts must be opaque, including over the hover wash (#744).
            super::presence::paint_mark(painter, icon_rect.center(), ICON * 0.38, shape, color, face);
        }
        Lead::Picture(Some(t), _) => {
            let size = t.size_vec2();
            let fit = egui::Rect::from_center_size(icon_rect.center(), size * (ICON / size.max_elem()));
            painter.image(
                t.id(),
                fit,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                super::widgets::PICTURE_TINT,
            );
        }
        Lead::Picture(None, text) => {
            painter.text(
                icon_rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                FontId::proportional(13.0),
                ink,
            );
        }
    }
    super::widgets::paint_row_text(painter, galley, rect.left() + PAD + ICON + PAD, rect.center().y, ink);
    let end_rect = egui::Rect::from_center_size(rect.right_center() - Vec2::new(PAD + 6.0, 0.0), Vec2::splat(12.0));
    match end {
        End::None | End::Check(false) => {}
        End::Caret => paint_icon(painter, "caret-right-filled", end_rect, if enabled { p.muted } else { p.muted_dim }),
        End::Down => paint_icon(painter, "caret-down-filled", end_rect, if enabled { p.muted } else { p.muted_dim }),
        End::Check(true) => paint_icon(painter, "check", end_rect, if enabled { p.violet_light } else { p.muted_dim }),
        End::Keys(_) => {
            if let Some(keys) = keys {
                let pos = super::widgets::row_text_origin(painter, &keys, rect.right() - PAD - keys.size().x, rect.center().y);
                painter.galley(pos, keys, p.muted);
            }
        }
        End::CheckKeys(checked, _) => {
            if checked {
                paint_icon(painter, "check", end_rect, if enabled { p.violet_light } else { p.muted_dim });
            }
            if let Some(keys) = keys {
                let right = rect.right() - CHECK_ROOM - keys.size().x;
                let pos = super::widgets::row_text_origin(painter, &keys, right, rect.center().y);
                painter.galley(pos, keys, if enabled { p.muted } else { p.muted_dim });
            }
        }
        End::Tag(_) => {
            if let Some(tag) = keys {
                let height = 16.0_f32.max(tag.size().y + 2.0);
                let pill = egui::Rect::from_min_size(
                    egui::pos2(rect.right() - PAD - tag.size().x - TAG_PAD * 2.0, rect.center().y - height / 2.0),
                    Vec2::new(tag.size().x + TAG_PAD * 2.0, height),
                );
                painter.rect_filled(pill, ui.visuals().window_corner_radius, p.teal.gamma_multiply(0.12));
                super::widgets::paint_ink_centered(painter, tag, pill.center(), p.teal);
            }
        }
    }
    resp
}

/// A menu row whose response belongs to a layout editor. Inactive commands
/// stay grey but can still be rearranged; the caller guards command execution.
#[allow(clippy::too_many_arguments)]
pub(crate) fn editable_row(
    ui: &mut Ui,
    p: &Palette,
    icon: &str,
    label: &str,
    enabled: bool,
    checked: Option<bool>,
    down: Option<bool>,
    editable: bool,
) -> egui::Response {
    let end = match down {
        Some(true) => End::Down,
        Some(false) => End::Caret,
        None => checked.map_or(End::None, End::Check),
    };
    let sense = if editable {
        Sense::click_and_drag()
    } else if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    row_lead_sensed(ui, p, Lead::Icon(icon), label, enabled, end, false, sense)
}

/// One absolute cell of an editable menu toolbar, including its empty cells.
pub(crate) fn icon_cell(ui: &mut Ui, p: &Palette, icon: &str, label: &str, enabled: bool, editable: bool, size: Vec2) -> egui::Response {
    let sense = if editable {
        Sense::click_and_drag()
    } else if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    if editable {
        ui.painter().rect_stroke(
            rect.shrink(1.0),
            ui.visuals().widgets.inactive.corner_radius,
            Stroke::new(1.0, p.raised),
            egui::StrokeKind::Inside,
        );
    }
    if resp.hovered() && (editable || enabled) {
        ui.painter()
            .rect_filled(rect, ui.visuals().window_corner_radius, p.violet.gamma_multiply(0.35));
    }
    if !icon.is_empty() {
        paint_icon(
            ui.painter(),
            icon,
            egui::Rect::from_center_size(rect.center(), Vec2::splat(ICON)),
            if enabled { p.violet_light } else { p.muted_dim },
        );
    }
    if label.is_empty() { resp } else { resp.on_hover_text(label) }
}

/// Preserve the parent when a child editor handles its own dismissal.
pub(crate) fn keep_parent_open_this_pass(ui: &Ui) {
    let id = ui.layer_id().id.with("menu_child_close_guard");
    egui::containers::menu::MenuState::from_ui(ui, |state, _| state.open_item = Some(id));
    egui::containers::menu::MenuState::mark_shown(ui.ctx(), id);
}

/// Child choice panel using the same surface and popup hierarchy as menus.
pub(crate) fn choice_panel(ui: &Ui, p: &Palette, id: Id, pos: egui::Pos2, open: &mut bool, body: impl FnOnce(&mut Ui)) {
    // Register the child in egui's menu hierarchy so a click in its name
    // field does not dismiss the menu being edited underneath it.
    egui::containers::menu::MenuState::from_ui(ui, |state, _| state.open_item = Some(id));
    Popup::new(id, ui.ctx().clone(), PopupAnchor::Position(pos), ui.layer_id())
        .kind(PopupKind::Menu)
        .align(egui::RectAlign::BOTTOM_START)
        .align_alternatives(&[])
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .style(style(p))
        .open_bool(open)
        .show(body);
}

/// A menu's editing companion, kept beside its owner until it is locked again.
pub(crate) fn settings_panel(ui: &Ui, p: &Palette, id: Id, body: impl FnOnce(&mut Ui)) {
    ui.ctx().data_mut(|d| d.insert_temp(ui.layer_id().id.with("editing_companion"), id));
    let deepest = egui::containers::menu::MenuState::is_deepest_open_sub_menu(ui.ctx(), ui.layer_id().id);
    egui::containers::menu::MenuState::from_ui(ui, |state, _| {
        if deepest {
            state.open_item = Some(id);
        }
    });
    let anchor = ui.min_rect().expand(4.0);
    let work = ui.ctx().content_rect();
    let left = anchor.left() - work.left() - 8.0;
    let right = work.right() - anchor.right() - 8.0;
    let width = left.max(right).clamp(160.0, 340.0).min(work.width());
    let (align, other) = if right >= left {
        (egui::RectAlign::RIGHT_END, egui::RectAlign::LEFT_END)
    } else {
        (egui::RectAlign::LEFT_END, egui::RectAlign::RIGHT_END)
    };
    Popup::new(id, ui.ctx().clone(), PopupAnchor::ParentRect(anchor), ui.layer_id())
        .kind(PopupKind::Menu)
        .align(align)
        .align_alternatives(&[other])
        .gap(8.0)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .close_behavior(PopupCloseBehavior::IgnoreClicks)
        .style(style(p))
        .frame(egui::Frame::NONE)
        .open(true)
        .show(|ui| {
            let spacing = ui.ctx().global_style().spacing.item_spacing;
            ui.spacing_mut().item_spacing = spacing;
            super::widgets::overlay(ui, p, egui::Margin::same(12), |ui| {
                ui.set_width((width - 24.0).max(1.0));
                super::widgets::organization_typography(ui);
                body(ui);
            });
        });
}

/// Editing can cross between the menu, its submenus and the settings shelf.
/// The owner handles clicks outside that complete region in editing mode.
pub(crate) fn editable_submenu(
    ui: &mut Ui,
    p: &Palette,
    response: &egui::Response,
    editing: bool,
    click_only: bool,
    persistent: bool,
    body: impl FnOnce(&mut Ui),
) {
    if click_only {
        if response.clicked() {
            response.request_focus();
        }
        let id = egui::containers::menu::SubMenu::id_from_widget_id(response.id);
        let open = egui::containers::menu::MenuState::from_ui(ui, |state, _| state.open_item == Some(id));
        if open && response.clicked_by(egui::PointerButton::Primary) {
            egui::containers::menu::MenuState::from_ui(ui, |state, _| state.open_item = None);
            return;
        }
    }
    let mut submenu = egui::containers::menu::SubMenu::new();
    if editing {
        // The companion only guards the parent's outside clicks. It must not
        // count as an already open submenu, which would suppress hover opening.
        let companion = ui.ctx().data(|d| d.get_temp::<Id>(ui.layer_id().id.with("editing_companion")));
        egui::containers::menu::MenuState::from_ui(ui, |state, _| {
            if state.open_item == companion {
                state.open_item = None;
            }
        });
        submenu = submenu.config(
            egui::containers::menu::MenuConfig::new()
                .close_behavior(PopupCloseBehavior::IgnoreClicks)
                .style(style(p)),
        );
    } else if persistent {
        // Scene checkable choices stay open until an action explicitly closes
        // them. Preserve the hierarchy's normal outside-click dismissal.
        submenu = submenu.config(
            egui::containers::menu::MenuConfig::new()
                .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
                .style(style(p)),
        );
    }
    let hover_guard = response.id.with("click_only_hover_guard");
    if click_only {
        // A temporary sibling suppresses native hover opening, while the
        // submenu still handles pointer activation, focus and arrow keys.
        egui::containers::menu::MenuState::mark_shown(ui.ctx(), hover_guard);
        egui::containers::menu::MenuState::from_ui(ui, |state, _| {
            if state.open_item.is_none() {
                state.open_item = Some(hover_guard);
            }
        });
    }
    submenu.show(ui, response, body);
    if click_only {
        egui::containers::menu::MenuState::from_ui(ui, |state, _| {
            if state.open_item == Some(hover_guard) {
                state.open_item = None;
            }
        });
    }
}

/// Horizontal room inside a tag.
const TAG_PAD: f32 = 6.0;

/// Entry with a detail at its right end (where a landmark leads); true when
/// clicked (the menu closes).
pub fn item_detail(ui: &mut Ui, p: &Palette, icon: &str, label: &str, detail: &str) -> bool {
    let clicked = row(ui, p, icon, label, true, End::Keys(detail), false).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// Entry with a small teal tag at its right end (a suggestion); true when
/// clicked (the menu closes).
pub fn item_tag(ui: &mut Ui, p: &Palette, icon: &str, label: &str, tag: &str) -> bool {
    let clicked = row(ui, p, icon, label, true, End::Tag(tag), false).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// The title of a group of entries: small capitals, muted.
pub fn section(ui: &mut Ui, p: &Palette, title: &str) {
    if filtering() {
        return;
    }
    let galley = ui.painter().layout_no_wrap(
        title.to_uppercase(),
        FontId::new(10.5, super::fonts::semibold(ui.ctx())),
        p.muted_dim,
    );
    let w = if ui.is_sizing_pass() {
        (galley.size().x + PAD * 2.0).max(MIN_W)
    } else {
        ui.available_width()
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 18.0_f32.max(galley.size().y + 2.0)), Sense::hover());
    ui.painter().galley(
        egui::pos2(rect.left() + PAD - 1.0, rect.bottom() - galley.size().y - 1.0),
        galley,
        p.muted_dim,
    );
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

/// Check entry with the keys of its shortcut (an empty `keys` shows none);
/// true when clicked (the menu closes).
pub fn check_keys(ui: &mut Ui, p: &Palette, icon: &str, label: &str, checked: bool, enabled: bool, keys: &str) -> bool {
    let clicked = row(ui, p, icon, label, enabled, End::CheckKeys(checked, keys), false).clicked();
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
    // a search lists the entries of an available sub-menu in place
    if enabled && filtering() {
        FILTER.with_borrow_mut(|f| f.as_mut().map(|f| f.path.push(label.to_owned())));
        body(ui);
        FILTER.with_borrow_mut(|f| f.as_mut().map(|f| f.path.pop()));
        return hidden(ui);
    }
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

/// Presence entry with a check mark, leaving the menu open; true when clicked.
pub fn status_toggle(ui: &mut Ui, p: &Palette, shape: super::presence::Shape, color: Color32, label: &str, on: bool) -> bool {
    row_lead(ui, p, Lead::Status(shape, color), label, true, End::Check(on), false).clicked()
}

/// The color dot of a swatch row, in the icon column.
fn dot(ui: &Ui, p: &Palette, row: &egui::Response, color: Color32) {
    let c = row.rect.left_center() + Vec2::new(PAD + ICON * 0.5, 0.0);
    ui.painter().circle_filled(c, ICON * 0.38, color);
    ui.painter().circle_stroke(c, ICON * 0.38, Stroke::new(1.0, p.raised));
}

/// What a resident's voice row shows: the avatar inspector's volume_slider
/// and mute_btn (LLInspectAvatar::updateVolumeSlider, inspect_avatar.xml).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceVolume {
    /// 0..1, 0.5 heard as they send it (LLSpeakerVolumeStorage).
    pub volume: f32,
    /// Their voice is blocked (LLMute::flagVoiceChat): the slider shows 0,
    /// greyed, as the inspector does.
    pub muted: bool,
    /// The mute button works: not a Linden (the inspector greys it).
    pub can_mute: bool,
    /// Whether a moderator muted them in nearby voice, when we moderate it
    /// (LLNearbyVoiceModeration::isNearbyChatModerator): the "for
    /// everyone" entries under the row.
    pub moderated: Option<bool>,
}

/// A change made on a [`voice_volume`] row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VoiceEdit {
    Volume(f32),
    ToggleMute,
}

/// The percentage next to the slider: 0.5 (as sent) is 100 %, its top end
/// 200 %, like the volume menu of a call's participants.
pub fn volume_percent(volume: f32) -> i32 {
    (volume.clamp(0.0, 1.0) * 200.0).round() as i32
}

const VOICE_ROW_H: f32 = 28.0;
const VOICE_PERCENT_W: f32 = 38.0;
const VOICE_BUTTON: f32 = 20.0;

/// A resident's voice as the first row of their menu: the speaker in the
/// icon column, the volume slider, its percentage and the voice mute button
/// at the right end. Dragging or clicking it leaves the menu open; the
/// change is returned at once (applied live by the app).
pub fn voice_volume(ui: &mut Ui, p: &Palette, v: VoiceVolume) -> Option<VoiceEdit> {
    let natural = MIN_W + 40.0;
    let w = if ui.is_sizing_pass() {
        natural
    } else {
        ui.available_width().max(natural)
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, VOICE_ROW_H), Sense::hover());
    let (key, frame) = (keep_open_id(ui.layer_id()), ui.ctx().cumulative_frame_nr());
    ui.ctx().data_mut(|d| d.insert_temp(key, (frame, rect)));
    if !ui.is_rect_visible(rect) {
        return None;
    }
    let shown = if v.muted { 0.0 } else { v.volume };
    let icon_rect = egui::Rect::from_center_size(rect.left_center() + Vec2::new(PAD + ICON * 0.5, 0.0), Vec2::splat(ICON));
    let tint = if v.muted { p.muted_dim } else { p.violet_light };
    paint_icon(ui.painter(), super::audio::speaker_icon(shown, v.muted), icon_rect, tint);
    // the mute button where the other rows draw their check mark
    let button = egui::Rect::from_center_size(rect.right_center() - Vec2::new(PAD + 6.0, 0.0), Vec2::splat(VOICE_BUTTON));
    let percent_right = button.left() - 6.0;
    let slider = egui::Rect::from_min_max(
        egui::pos2(rect.left() + PAD + ICON + PAD, rect.top()),
        egui::pos2(percent_right - VOICE_PERCENT_W, rect.bottom()),
    );
    let mut edit = None;
    let mut value = shown;
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("voice_volume_slider")
            .max_rect(slider)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.spacing_mut().slider_width = slider.width();
    let r = super::widgets::volume_slider(&mut child, &mut value, super::widgets::VolumeScale::Voice, !v.muted, |s| {
        s.show_value(false).trailing_fill(true)
    })
    .on_hover_text(aurora_i18n::tr!("ui-call-voice-volume"));
    if r.changed() && !v.muted {
        edit = Some(VoiceEdit::Volume(value));
    }
    ui.painter().text(
        egui::pos2(percent_right, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        format!("{} %", volume_percent(value)),
        FontId::proportional(11.5),
        if v.muted { p.muted_dim } else { p.muted },
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("voice_volume_mute")
            .max_rect(button)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let clicked = if v.can_mute {
        let tip = if v.muted {
            aurora_i18n::tr!("ui-call-unmute-voice")
        } else {
            aurora_i18n::tr!("ui-call-mute-voice")
        };
        super::widgets::icon_toggle(&mut child, p, "microphone-off", tip, v.muted)
    } else {
        super::widgets::icon_button(
            &mut child,
            p,
            "microphone-off",
            aurora_i18n::tr!("ui-call-mute-voice-linden"),
            false,
        )
    };
    if clicked {
        edit = Some(VoiceEdit::ToggleMute);
    }
    edit
}

/// egui temp data: where the voice row of the menu in `layer` was drawn,
/// and in which frame.
fn keep_open_id(layer: LayerId) -> Id {
    Id::unique("aurora_menu_keep_open").with(layer.id)
}

/// A click this frame lands on the voice row the menu `popup` drew last
/// frame: a menu that closes on any click stays open for it.
pub(crate) fn clicks_kept_row(ctx: &egui::Context, popup: Id) -> bool {
    let layer = LayerId::new(egui::Order::Foreground, popup);
    let Some((frame, rect)) = ctx.data(|d| d.get_temp::<(u64, egui::Rect)>(keep_open_id(layer))) else {
        return false;
    };
    frame + 1 >= ctx.cumulative_frame_nr()
        && ctx.input(|i| i.pointer.any_click() && i.pointer.interact_pos().is_some_and(|pos| rect.contains(pos)))
}

/// Thin line between groups of entries.
pub fn separator(ui: &mut Ui, p: &Palette) {
    if filtering() {
        return;
    }
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
    if filtering() {
        return hidden(ui);
    }
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
    let natural = (PAD + ICON + PAD + galley.size().x + PAD).max(MIN_W);
    let w = if ui.is_sizing_pass() {
        natural
    } else {
        ui.available_width().max(natural)
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h.max(galley.size().y + 6.0)), Sense::hover());
    let icon_rect = egui::Rect::from_center_size(rect.left_center() + Vec2::new(PAD + ICON * 0.5, 0.0), Vec2::splat(icon_size));
    paint_icon(ui.painter(), icon, icon_rect, icon_color);
    super::widgets::paint_row_text(ui.painter(), galley, rect.left() + PAD + ICON + PAD, rect.center().y, color);
    resp
}

/// The title of a group of rows, in small capitals (the parcel's rights,
/// the recent places of the place menu).
pub fn heading(ui: &mut Ui, p: &Palette, text: &str) {
    if !filtering() {
        heading_line(ui, p, text);
    }
}

fn heading_line(ui: &mut Ui, p: &Palette, text: &str) {
    let font = FontId::new(10.5, super::fonts::semibold(ui.ctx()));
    let galley = ui.painter().layout_no_wrap(text.to_uppercase(), font, p.muted_dim);
    let natural = (PAD + galley.size().x + PAD).max(MIN_W);
    let w = if ui.is_sizing_pass() {
        natural
    } else {
        ui.available_width().max(natural)
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 21.0_f32.max(galley.size().y + 4.0)), Sense::hover());
    ui.painter().galley(
        egui::pos2(rect.left() + PAD, rect.bottom() - 3.0 - galley.size().y),
        galley,
        p.muted_dim,
    );
}

/// The query of the [`searchable`] menu being drawn, and where its rows
/// stand: the sub-menus they are in, the path whose entries were drawn
/// last, the paths whose heading was drawn, the entries shown; then the
/// keyboard selection: the rows that can be selected so far, the selected
/// one, the one the pointer moved onto, and whether Enter clicks it or the
/// arrows moved it this frame.
struct Filter {
    words: Vec<String>,
    path: Vec<String>,
    shown: Option<Vec<String>>,
    headed: Vec<Vec<String>>,
    matches: usize,
    selectable: usize,
    selected: usize,
    pointed: Option<usize>,
    activate: bool,
    scroll: bool,
}

/// A filtered row that the arrow keys can select.
#[derive(Clone, Copy)]
struct KeyRow {
    index: usize,
    selected: bool,
    activate: bool,
    scroll: bool,
}

/// The next selectable row of the filtered menu, `None` when not filtering.
fn keyboard_row() -> Option<KeyRow> {
    FILTER.with_borrow_mut(|f| {
        let f = f.as_mut()?;
        let index = f.selectable;
        f.selectable += 1;
        Some(KeyRow {
            index,
            selected: index == f.selected,
            activate: f.activate,
            scroll: f.scroll,
        })
    })
}

/// The pointer moved onto the row `index`: it becomes the selected one.
fn point_row(index: usize) -> bool {
    FILTER.with_borrow_mut(|f| f.as_mut().map(|f| f.pointed = Some(index)));
    true
}

/// The selection after the arrow keys, over `count` rows, wrapping around
/// at both ends.
fn step_selection(selected: usize, count: usize, down: bool, up: bool) -> usize {
    if count == 0 {
        return 0;
    }
    let selected = selected.min(count - 1);
    match (down, up) {
        (true, false) => (selected + 1) % count,
        (false, true) => (selected + count - 1) % count,
        _ => selected,
    }
}

/// What a [`searchable`] menu keeps between frames.
#[derive(Clone, Default)]
struct SearchState {
    query: String,
    frame: u64,
    selected: usize,
    /// Selectable rows drawn on the last frame.
    count: usize,
    /// The search field, whose focus the keys need.
    field: Option<Id>,
}

thread_local! {
    static FILTER: std::cell::RefCell<Option<Filter>> = const { std::cell::RefCell::new(None) };
}

/// A [`searchable`] menu is being filtered: its rows not matching, its
/// separators and its titles are not drawn.
fn filtering() -> bool {
    FILTER.with_borrow(Option::is_some)
}

/// The words of a query, lower case.
fn query_words(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

/// Every word of the query is in the entry's label or in the name of one of
/// the sub-menus it is in ("home" finds Go Home and Set Home to Here, "world
/// map" the Map of World).
fn entry_matches(words: &[String], path: &[String], label: &str) -> bool {
    let label = label.to_lowercase();
    let path: Vec<String> = path.iter().map(|s| s.to_lowercase()).collect();
    words
        .iter()
        .all(|w| label.contains(w.as_str()) || path.iter().any(|s| s.contains(w.as_str())))
}

/// What goes above an entry found by a search.
#[derive(Debug, PartialEq)]
enum Above {
    /// Under the entries drawn just before it.
    Nothing,
    /// The first entry of its path: the path as a heading.
    Heading(String),
    /// Back in a path already headed (Me after Me › Avatar health), or at
    /// the top level after a sub-menu: a divider, the heading is not
    /// repeated.
    Divider,
}

/// The lead of an entry of `path`, the previous one having been drawn
/// under `shown` (`None`: the first entry), and the paths already headed.
fn above(path: &[String], shown: &mut Option<Vec<String>>, headed: &mut Vec<Vec<String>>) -> Above {
    if shown.as_deref() == Some(path) {
        return Above::Nothing;
    }
    let first = shown.replace(path.to_vec()).is_none();
    if path.is_empty() {
        return if first { Above::Nothing } else { Above::Divider };
    }
    if headed.iter().any(|h| h == path) {
        return Above::Divider;
    }
    headed.push(path.to_vec());
    Above::Heading(path.join(" › "))
}

/// Whether the row `label` is drawn; while filtering, the heading of its
/// path is drawn before the first entry found under it.
fn filter_row(ui: &mut Ui, p: &Palette, label: &str) -> bool {
    let lead = FILTER.with_borrow_mut(|f| {
        let Some(f) = f.as_mut() else {
            return Some(Above::Nothing);
        };
        if !entry_matches(&f.words, &f.path, label) {
            return None;
        }
        f.matches += 1;
        Some(above(&f.path, &mut f.shown, &mut f.headed))
    });
    match lead {
        None => false,
        Some(Above::Nothing) => true,
        Some(Above::Heading(path)) => {
            heading_line(ui, p, &path);
            true
        }
        Some(Above::Divider) => {
            let w = if ui.is_sizing_pass() { MIN_W } else { ui.available_width() };
            let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 7.0), Sense::hover());
            ui.painter()
                .hline(rect.x_range().shrink(4.0), rect.center().y, super::widgets::divider_stroke(p));
            true
        }
    }
}

/// The response of a row a search hides: it takes no room and is never
/// clicked.
fn hidden(ui: &mut Ui) -> egui::Response {
    ui.allocate_exact_size(Vec2::ZERO, Sense::hover()).1
}

/// A menu with a search field at its top (the wolf menu): typing filters its
/// entries, those of the sub-menus listed in place under their path ("Me ›
/// Avatar health"); a long list scrolls under the field. The field takes the keyboard when the menu opens and starts
/// empty each time; a click in it leaves a menu that closes on any click
/// open. While it filters, the first entry found is selected, the up and
/// down arrows move the selection (wrapping around) and Enter clicks it.
/// Aurora's own: the Legacy Viewer's menus have no search.
pub fn searchable(ui: &mut Ui, p: &Palette, body: impl FnOnce(&mut Ui)) {
    let id = ui.layer_id().id.with("aurora_menu_search");
    let frame = ui.ctx().cumulative_frame_nr();
    let mut state = ui.data(|d| d.get_temp::<SearchState>(id)).unwrap_or_default();
    let opened = state.frame + 1 < frame;
    if opened {
        state = SearchState::default();
    }
    // taken before the field sees them, and only while it has the keyboard
    // (Tab onto its clear button leaves Enter to that button): a one-line
    // field has no use for the arrows, and Enter, even with nothing found,
    // would only drop its focus
    let focused = state.field.is_some_and(|f| ui.memory(|m| m.has_focus(f)));
    let (down, up, enter) = if !focused || query_words(&state.query).is_empty() {
        (false, false, false)
    } else {
        ui.input_mut(|i| {
            let down = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
            let up = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
            let enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) && state.count > 0;
            (down, up, enter)
        })
    };
    let before = state.query.clone();
    let w = if ui.is_sizing_pass() { MIN_W } else { ui.available_width() };
    let field = ui.horizontal(|ui| super::widgets::clearable_search_box(ui, p, &mut state.query, aurora_i18n::tr!("common-search"), w));
    if opened {
        field.inner.request_focus();
    }
    state.field = Some(field.inner.id);
    state.selected = if state.query == before {
        step_selection(state.selected, state.count, down, up)
    } else {
        0
    };
    state.frame = frame;
    let (key, rect) = (keep_open_id(ui.layer_id()), field.response.rect);
    ui.ctx().data_mut(|d| {
        d.insert_temp(key, (frame, rect));
        d.insert_temp(id, state.clone());
    });
    ui.add_space(4.0);
    let words = query_words(&state.query);
    if words.is_empty() {
        body(ui);
        return;
    }
    FILTER.set(Some(Filter {
        words,
        path: Vec::new(),
        shown: None,
        headed: Vec::new(),
        matches: 0,
        selectable: 0,
        selected: state.selected,
        pointed: None,
        activate: enter,
        scroll: down || up,
    }));
    // a broad query lists more entries than the window holds: they scroll
    // and the field stays in place (the popup, sized on the last frame,
    // would otherwise hold them to the height of the unfiltered menu)
    let room = (ui.ctx().content_rect().bottom() - ui.cursor().top() - 12.0).max(120.0);
    egui::ScrollArea::vertical()
        .max_height(room)
        .min_scrolled_height(room)
        .show(ui, body);
    let filter = FILTER.take();
    let found = filter.as_ref().is_some_and(|f| f.matches > 0);
    if let Some(f) = filter {
        state.count = f.selectable;
        state.selected = f.pointed.unwrap_or(state.selected);
        ui.ctx().data_mut(|d| d.insert_temp(id, state.clone()));
    }
    if !found {
        let text = aurora_i18n::tr!("ui-bars-menu-no-match", query = state.query.trim().to_string());
        info(ui, p, "search-filled", &text);
    }
}

/// `AURORA_DEMO_MENU_SEARCH=<query>`: the wolf menu of the islands (the
/// default bars) clicked at frames 540–541 and `query` typed in its search
/// field at 560; capture frame 620.
pub fn search_demo_events(frame: u64) -> Vec<egui::Event> {
    static QUERY: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let Some(query) = QUERY.get_or_init(|| std::env::var("AURORA_DEMO_MENU_SEARCH").ok()) else {
        return Vec::new();
    };
    let wolf = egui::pos2(30.0, 30.0);
    let click = |pressed| egui::Event::PointerButton {
        pos: wolf,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    match frame {
        540 => vec![egui::Event::PointerMoved(wolf), click(true)],
        541 => vec![click(false)],
        // off the menu, so that no row is hovered
        550 => vec![egui::Event::PointerMoved(egui::pos2(600.0, 400.0))],
        560 => vec![egui::Event::Text(query.clone())],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widgets::{Floater, elide_tests::Frames};

    fn words(q: &str) -> Vec<String> {
        query_words(q)
    }

    #[test]
    fn search_finds_every_entry_with_the_word() {
        let me = ["Me".to_owned()];
        assert!(entry_matches(&words("home"), &me, "Go Home"));
        assert!(entry_matches(&words("HOME"), &[], "Set Home to Here"));
        assert!(!entry_matches(&words("home"), &me, "Sit on ground"));
    }

    #[test]
    fn search_needs_every_word_in_the_label_or_the_path() {
        let world = ["World".to_owned()];
        assert!(entry_matches(&words("world map"), &world, "Map"));
        assert!(!entry_matches(&words("world map"), &world, "Search"));
        assert!(entry_matches(&words("  mini   map "), &world, "Mini-map"));
        assert!(!entry_matches(&words("teleport home"), &[], "Go Home"));
    }

    #[test]
    fn search_arrows_wrap_around_the_entries_found() {
        assert_eq!(step_selection(0, 3, true, false), 1);
        assert_eq!(step_selection(2, 3, true, false), 0);
        assert_eq!(step_selection(0, 3, false, true), 2);
        assert_eq!(step_selection(1, 3, false, false), 1);
        // fewer entries since the last frame: back onto the last one
        assert_eq!(step_selection(5, 3, false, false), 2);
        assert_eq!(step_selection(4, 0, true, false), 0);
    }

    #[test]
    fn search_query_splits_on_whitespace() {
        assert!(words("   ").is_empty());
        assert_eq!(words("Go  HOME"), ["go", "home"]);
    }

    #[test]
    fn search_heads_each_path_once() {
        let path = |s: &str| -> Vec<String> { s.split('/').filter(|w| !w.is_empty()).map(String::from).collect() };
        let (mut shown, mut headed) = (None, Vec::new());
        let mut next = |s: &str| above(&path(s), &mut shown, &mut headed);
        // Me, Me › Health, back in Me, then the top level
        assert_eq!(next("Me"), Above::Heading("Me".into()));
        assert_eq!(next("Me"), Above::Nothing);
        assert_eq!(next("Me/Health"), Above::Heading("Me › Health".into()));
        assert_eq!(next("Me"), Above::Divider);
        assert_eq!(next("Me"), Above::Nothing);
        assert_eq!(next(""), Above::Divider);
        assert_eq!(next("World"), Above::Heading("World".into()));
        // a top-level entry found first has nothing above it
        let (mut shown, mut headed) = (None, Vec::new());
        assert_eq!(above(&[], &mut shown, &mut headed), Above::Nothing);
    }

    struct Menus {
        frames: Frames,
        kind: u8,
        resize_first: bool,
        bare: bool,
        target: egui::Rect,
        open: bool,
        shown: bool,
        world: Option<crate::ui::context::ContextMenu>,
    }

    impl Menus {
        fn frame(&mut self, events: Vec<egui::Event>) -> egui::Rect {
            let p = crate::theme::Theme::default().palette();
            let Self {
                frames,
                kind,
                resize_first,
                bare,
                target,
                open,
                shown,
                world,
            } = self;
            frames.run(events, |ui| {
                let resize = |ctx: &egui::Context| {
                    if *bare {
                        crate::ui::widgets::bare_window(
                            ctx,
                            &p,
                            "resize_other",
                            "Other window",
                            egui::Frame::window(&ctx.global_style()),
                            egui::Rect::from_min_size(egui::pos2(600.0, 200.0), egui::vec2(400.0, 300.0)),
                            egui::vec2(220.0, 120.0),
                            false,
                            |_| {},
                        );
                    } else {
                        Floater::new("resize_other", "Other window", egui::pos2(600.0, 200.0), egui::vec2(400.0, 300.0))
                            .silent()
                            .show(ctx, &p, &mut true, |_| {});
                    }
                };
                if *resize_first {
                    resize(ui.ctx());
                }
                *shown = false;
                Floater::new("menu_owner", "Menu owner", egui::pos2(30.0, 100.0), egui::vec2(300.0, 260.0))
                    .silent()
                    .show(ui.ctx(), &p, &mut true, |ui| {
                        let response = ui.button("Right click here");
                        *target = response.rect;
                        if *kind == 0 {
                            context_menu(&response, &p, |ui| {
                                *shown = true;
                                item(ui, &p, "copy", "Menu action");
                            });
                            return;
                        }
                        if response.secondary_clicked() {
                            *open = true;
                            if *kind == 1 {
                                Popup::open_id(ui.ctx(), Id::unique("anchored_menu"));
                            }
                            *world = Some(crate::ui::context::ContextMenu {
                                pos: target.center(),
                                point: glam::Vec3::ZERO,
                                target: crate::ui::context::Target::Ground,
                                pie_path: Vec::new(),
                                pie_input_frame: None,
                            });
                        }
                        match *kind {
                            1 => {
                                context_menu_at(ui, Id::unique("anchored_menu"), target.center(), &p, |ui| {
                                    *shown = true;
                                    item(ui, &p, "copy", "Menu action");
                                });
                            }
                            2 => {
                                popup_at(ui.ctx(), Id::unique("bool_menu"), target.center(), open, &p, |ui| {
                                    *shown = true;
                                    item(ui, &p, "copy", "Menu action");
                                });
                            }
                            _ => {
                                crate::ui::context::show(
                                    ui.ctx(),
                                    &p,
                                    world,
                                    &Default::default(),
                                    *kind == 4,
                                    &mut Default::default(),
                                    &mut Default::default(),
                                );
                                *shown = world.is_some();
                            }
                        }
                    });
                if !*resize_first {
                    resize(ui.ctx());
                }
            });
            self.frames
                .ctx
                .memory(|m| m.area_rect(Id::unique("resize_other")).expect("other window"))
        }
    }

    /// Menus close while a different window is resized, regardless of drawing
    /// order (#589). The pie's modal surface prevents grabbing other windows;
    /// native resizing below exercises its dismissal without bypassing capture.
    #[test]
    fn resizing_any_window_dismisses_context_menus() {
        dragging_any_window_dismisses_context_menus(false);
    }

    #[test]
    fn moving_any_window_dismisses_context_menus() {
        dragging_any_window_dismisses_context_menus(true);
    }

    fn dragging_any_window_dismisses_context_menus(moving: bool) {
        for kind in 0..4 {
            for resize_first in [false, true] {
                for bare in [false, true] {
                    let mut h = Menus {
                        frames: Frames::new(),
                        kind,
                        resize_first,
                        bare,
                        target: egui::Rect::NOTHING,
                        open: false,
                        shown: false,
                        world: None,
                    };
                    for _ in 0..8 {
                        h.frame(vec![]);
                    }
                    let at = h.target.center();
                    let button = |pos, button, pressed| egui::Event::PointerButton {
                        pos,
                        button,
                        pressed,
                        modifiers: Default::default(),
                    };
                    h.frame(vec![egui::Event::PointerMoved(at)]);
                    h.frame(vec![button(at, egui::PointerButton::Secondary, true)]);
                    let before = h.frame(vec![button(at, egui::PointerButton::Secondary, false)]);
                    h.frame(vec![]);
                    assert!(h.shown, "kind={kind}, first={resize_first}, bare={bare}: menu opened");
                    let grab = if !moving {
                        before.right_bottom() - egui::vec2(6.0, 6.0)
                    } else if bare {
                        before.center()
                    } else {
                        h.frames
                            .ctx
                            .read_response(Id::unique("resize_other").with("title_double_click"))
                            .expect("other window title")
                            .rect
                            .center()
                    };
                    h.frame(vec![egui::Event::PointerMoved(grab)]);
                    assert!(h.shown, "hovering a window grip keeps the menu open");
                    h.frame(vec![button(grab, egui::PointerButton::Primary, true)]);
                    if moving {
                        let jittered = h.frame(vec![egui::Event::PointerMoved(grab + egui::vec2(2.0, 1.0))]);
                        assert_eq!(jittered, before, "click jitter does not move the window");
                        assert!(h.shown, "click jitter keeps the menu open");
                    }
                    let resized = h.frame(vec![egui::Event::PointerMoved(grab + egui::vec2(40.0, 30.0))]);
                    assert!(
                        if moving {
                            resized.min.distance(before.min) > 10.0
                        } else {
                            (resized.size() - before.size()).length() > 10.0
                        },
                        "kind={kind}, first={resize_first}, bare={bare}, moving={moving}: the real window changed: {before:?} -> {resized:?}"
                    );
                    if moving {
                        assert_eq!(resized.size(), before.size(), "moving preserves the window's size");
                    }
                    assert!(h.frames.ctx.input(|i| i.pointer.primary_down()));
                    assert!(
                        !h.shown,
                        "kind={kind}, first={resize_first}, bare={bare}, moving={moving}: menu dismissed before release"
                    );
                    assert!(!Popup::is_any_open(&h.frames.ctx), "popup memory is cleared too");
                }
            }
        }
    }

    #[test]
    fn native_window_changes_dismiss_menus_without_blocking_reopening() {
        for kind in 0..5 {
            let mut h = Menus {
                frames: Frames::new(),
                kind,
                resize_first: false,
                bare: false,
                target: egui::Rect::NOTHING,
                open: false,
                shown: false,
                world: None,
            };
            for _ in 0..8 {
                h.frame(vec![]);
            }
            for resized in [false, true] {
                let at = h.target.center();
                h.frame(vec![egui::Event::PointerMoved(at)]);
                for pressed in [true, false] {
                    h.frame(vec![egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: Default::default(),
                    }]);
                }
                h.frame(vec![]);
                assert!(h.shown, "kind={kind}, resized={resized}: menu opens");
                if !resized {
                    // The winit adapter forwards this event between egui frames.
                    window_geometry_changed(&h.frames.ctx);
                    for _ in 0..4 {
                        h.frame(vec![]);
                        assert!(!h.shown, "kind={kind}: native resize dismisses the menu");
                        assert!(!Popup::is_any_open(&h.frames.ctx));
                    }
                }
            }
        }
    }
}

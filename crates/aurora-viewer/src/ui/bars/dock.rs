//! Aurora's bottom bar: the local chat in a glass island on the left and the
//! toolbar's buttons in a centred glass dock (variant A of the bars mockup
//! the human validated), instead of Firestorm's full-width bar. The chat
//! island keeps the chat bar's behaviour (`chat::bar_input`, after
//! LLFloaterIMNearbyChat) with its width handle on its right edge; the dock
//! keeps the toolbar's commands, drags, menus and Speak panel
//! (`bars::toolbar_row`, after LLToolBar), in fixed buttons. Both sample the
//! windows' blur like the top islands (`islands::area`): no extra GPU pass.
//! Preferences › Aurora › Appearance › "Top and bottom bars" brings
//! Firestorm's bar back (`bars::bottom_bar`, unchanged).

use super::islands::{GAP, MARGIN, PAD, SLOT, area, slot_face, slot_ink};
use super::{BarAction, ButtonLook, DOCK_MIC_W, MicButton, Row, paint_icon, toolbar_row};
use crate::theme::Palette;
use crate::ui::Panels;
use crate::ui::icons::Icons;
use egui::{Id, Pos2, Rect, RichText, Sense, Vec2};

/// Height of the dock.
pub const DOCK_H: f32 = 40.0;
/// Height of the chat island.
pub const CHAT_H: f32 = 38.0;
/// Height of a dock button.
const BUTTON_H: f32 = 32.0;
/// Icons of the dock's buttons.
const BUTTON_ICON: f32 = 18.0;
/// Narrowest dock button, when the buttons do not fit in their width.
const MIN_BUTTON: f32 = 24.0;
/// Room between the chat island and the dock, at the least.
const AFTER_CHAT: f32 = 12.0;
/// Narrowest chat island.
const MIN_CHAT: f32 = 200.0;
/// Room in the chat field before the text and after the emoji.
const FIELD_PAD: f32 = 9.0;
const EMOJI_ICON: f32 = 15.0;

/// What the windows keep free above the bottom islands: the dock and the
/// usual 8 px gap (`islands::work_area`).
pub const RESERVED: f32 = MARGIN + DOCK_H + MARGIN;

/// A screen under 1408 px wide (the mockup's 1280) takes the narrower
/// chat island and buttons.
fn narrow(screen: Rect) -> bool {
    screen.width() < 1408.0
}

/// The chat island's width when the user has not dragged it (0 saved).
pub fn default_chat_width(screen: Rect) -> f32 {
    if narrow(screen) { 300.0 } else { 380.0 }
}

/// Where the bottom islands stand on `screen`: the chat island, the dock,
/// and the width of its buttons (the Speak button is `DOCK_MIC_W` wide).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub chat: Rect,
    pub dock: Rect,
    pub button: f32,
}

fn dock_width(buttons: usize, mic: bool, button: f32) -> f32 {
    if buttons == 0 {
        // an empty dock keeps one slot, where a button can be dropped
        return PAD * 2.0 + BUTTON_H;
    }
    let plain = buttons - usize::from(mic);
    PAD * 2.0 + plain as f32 * button + if mic { DOCK_MIC_W } else { 0.0 } + GAP * (buttons - 1) as f32
}

/// The bottom islands' layout: the chat island on the left, `chat_width`
/// wide (its default when 0), the dock centred on the screen but at least
/// `AFTER_CHAT` after the chat island; buttons that do not fit narrow down
/// to `MIN_BUTTON`, and a chat island that would push the dock out shrinks.
pub fn geometry(screen: Rect, chat_width: f32, buttons: usize, mic: bool) -> Geometry {
    let bottom = screen.bottom() - MARGIN;
    let left = screen.left() + MARGIN;
    let right = screen.right() - MARGIN;
    let wanted = if chat_width > 0.0 { chat_width } else { default_chat_width(screen) };
    let max_chat = (right - left - AFTER_CHAT - dock_width(buttons, mic, MIN_BUTTON)).max(MIN_CHAT);
    let chat_w = wanted.clamp(MIN_CHAT, max_chat);
    let chat = Rect::from_min_max(Pos2::new(left, bottom - CHAT_H), Pos2::new(left + chat_w, bottom));
    let mut button = if narrow(screen) { 32.0 } else { 34.0 };
    let mut width = dock_width(buttons, mic, button);
    let mut x = (left + ((right - left - width) / 2.0).round()).max(chat.right() + AFTER_CHAT);
    if x + width > right && buttons > usize::from(mic) {
        x = chat.right() + AFTER_CHAT;
        let plain = (buttons - usize::from(mic)) as f32;
        let fixed = PAD * 2.0 + if mic { DOCK_MIC_W } else { 0.0 } + GAP * (buttons - 1) as f32;
        button = ((right - x - fixed) / plain).floor().max(MIN_BUTTON);
        width = dock_width(buttons, mic, button);
    }
    let dock = Rect::from_min_max(Pos2::new(x, bottom - DOCK_H), Pos2::new(x + width, bottom));
    Geometry { chat, dock, button }
}

/// The bottom islands. Returns what the chat field sent, the dock's
/// action, the top of what the islands keep (`RESERVED` above the screen's
/// bottom: the windows' work area and the chat lines stop there) and
/// whether the chat island's width was set (to save).
#[allow(clippy::too_many_arguments)]
pub fn bottom(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    layout: &crate::ui::skin::Layout,
    toolbar: &mut crate::ui::toolbar::ToolbarUi,
    toolbar_settings: &crate::ui::toolbar::ToolbarSettings,
    panels: &mut Panels,
    chat: &mut crate::ui::chat::ChatUi,
    gestures: &crate::gestures::Manager,
    emoji: Option<&mut crate::ui::emoji::Emoji>,
    row: &Row,
    chat_width: &mut f32,
    mic: &mut MicButton,
) -> (Option<crate::ui::chat::OutgoingChat>, BarAction, f32, bool) {
    let ctx = ui.ctx().clone();
    let screen = ui.max_rect();
    let buttons = toolbar_settings.buttons(layout);
    let has_mic = buttons.iter().any(|b| b.command == "speak");
    let geo = geometry(screen, *chat_width, buttons.len(), has_mic);
    let mut action = BarAction::None;
    let mut committed = false;

    let out = area(&ctx, p, "chat", geo.chat, |ui| {
        chat_island(ui, p, icons, panels, chat, gestures, emoji, geo, chat_width, &mut committed)
    });

    area(&ctx, p, "dock", geo.dock, |ui| {
        // the dock's background: its right-click menu and the drop target
        let target = ui.interact(geo.dock, Id::new("toolbar_background"), Sense::click());
        toolbar.context(&target, p, None, row.locked);
        let inner = geo.dock.shrink2(Vec2::new(PAD, (DOCK_H - BUTTON_H) / 2.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
        child.spacing_mut().item_spacing.x = GAP;
        let look = |b: &crate::ui::skin::ToolButton| ButtonLook {
            size: Vec2::new(if b.command == "speak" { DOCK_MIC_W } else { geo.button }, BUTTON_H),
            icon: BUTTON_ICON,
            dock: true,
        };
        let rects = toolbar_row(&mut child, p, icons, buttons, look, toolbar, panels, mic, row, &mut action);
        toolbar.drop_bar(&target, &rects, p, row.locked);
    });
    (out, action, screen.bottom() - RESERVED, committed)
}

/// The chat island: the field with "Nearby chat" as its placeholder and the
/// emoji picker at its end, the button that opens the local chat, and the
/// width handle on the island's right edge (double click: the default).
#[allow(clippy::too_many_arguments)]
fn chat_island(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    panels: &mut Panels,
    chat: &mut crate::ui::chat::ChatUi,
    gestures: &crate::gestures::Manager,
    emoji: Option<&mut crate::ui::emoji::Emoji>,
    geo: Geometry,
    chat_width: &mut f32,
    committed: &mut bool,
) -> Option<crate::ui::chat::OutgoingChat> {
    let island = geo.chat;
    let top = island.center().y - SLOT / 2.0;
    let button = Rect::from_min_size(Pos2::new(island.right() - PAD - SLOT, top), Vec2::splat(SLOT));
    let field = Rect::from_min_max(Pos2::new(island.left() + PAD, top), Pos2::new(button.left() - GAP, top + SLOT));
    let radius = ui.visuals().window_corner_radius;
    ui.painter().rect_filled(field, radius, p.field);

    // the emoji, at the field's end
    let smile = Rect::from_center_size(
        Pos2::new(field.right() - FIELD_PAD - EMOJI_ICON / 2.0, field.center().y),
        Vec2::splat(EMOJI_ICON),
    );
    let smile_resp = ui.interact(smile.expand(4.0), Id::new("chat_island_emoji"), Sense::click());
    let hot = smile_resp.hovered() || chat.bar_emoji_open;
    paint_icon(ui, icons, "mood-smile-filled", smile, EMOJI_ICON, slot_ink(p, hot, true));
    if smile_resp.on_hover_text(aurora_i18n::tr!("ui-chat-emoji")).clicked() {
        chat.bar_emoji_open = !chat.bar_emoji_open;
    }
    if super::demo::get().open == Some(super::demo::Open::Emoji) && ui.ctx().cumulative_frame_nr() == 300 {
        chat.bar_emoji_open = true;
    }

    let text = Rect::from_min_max(
        Pos2::new(field.left() + FIELD_PAD, field.top()),
        Pos2::new(smile.left() - 6.0, field.bottom()),
    );
    let hint = RichText::new(aurora_i18n::tr!("common-nearby-chat")).size(12.5).color(p.muted_dim);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text).layout(egui::Layout::left_to_right(egui::Align::Center)));
    // egui paints a placeholder in the weak text colour, whatever its own
    child.visuals_mut().weak_text_color = Some(p.muted_dim);
    let mut emoji = emoji;
    let out = crate::ui::chat::bar_input(&mut child, chat, gestures, emoji.as_deref_mut(), |edit| {
        edit.frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(egui::FontId::proportional(12.5))
            .text_color(p.ink)
            .hint_text(hint)
            .desired_width(text.width())
            .vertical_align(egui::Align::Center)
    });
    if chat.bar_emoji_open {
        emoji_picker(ui, p, icons, chat, emoji, smile);
    }

    // the local chat in the Conversations window, as Firestorm's nearby
    // chat button (`nearbychat_18`)
    let open = panels.chat && chat.selected.is_none() && !chat.contacts;
    let resp = ui.interact(button, Id::new("chat_island_open"), Sense::click());
    slot_face(ui, p, button, resp.hovered(), open);
    paint_icon(ui, icons, "message-circle-filled", button, super::islands::ICON, slot_ink(p, resp.hovered() || open, true));
    if resp.on_hover_text(aurora_i18n::tr!("common-nearby-chat")).clicked() {
        if open {
            crate::ui::widgets::toggle_floater(ui.ctx(), "conversations", &mut panels.chat);
        } else {
            chat.selected = None;
            chat.contacts = false;
            crate::ui::widgets::open_floater(ui.ctx(), "conversations", &mut panels.chat);
        }
    }

    // the width handle, along the island's right edge
    let handle = Rect::from_min_max(
        Pos2::new(island.right() - 4.0, island.top() + 6.0),
        Pos2::new(island.right() + 2.0, island.bottom() - 6.0),
    );
    let h = ui.interact(handle, Id::new("chat_island_width"), Sense::click_and_drag());
    if h.hovered() || h.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if h.dragged() {
        // the geometry bounds it on the next frame, against the dock
        *chat_width = (island.width() + h.drag_delta().x).max(MIN_CHAT);
    }
    if h.drag_stopped() {
        *committed = true;
    }
    if h.double_clicked() {
        *chat_width = 0.0;
        *committed = true;
    }
    out
}

/// The emoji picker of the chat field, above its button (the one of the
/// Conversations window); a pick goes at the end of the field, Shift keeps
/// the picker open.
fn emoji_picker(
    ui: &egui::Ui,
    p: &Palette,
    icons: &Icons,
    chat: &mut crate::ui::chat::ChatUi,
    emoji: Option<&mut crate::ui::emoji::Emoji>,
    anchor: Rect,
) {
    let Some(emoji) = emoji else {
        chat.bar_emoji_open = false;
        return;
    };
    let mut close = false;
    let shown = egui::Area::new(Id::new("chat_island_emoji_picker"))
        .fade_in(false)
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .fixed_pos(Pos2::new(anchor.right() + FIELD_PAD, anchor.top() - (CHAT_H - EMOJI_ICON) / 2.0 - GAP * 3.0))
        .show(ui.ctx(), |ui| {
            crate::ui::widgets::overlay(ui, p, egui::Margin::same(6), |ui| {
                ui.set_width(300.0);
                if let Some(c) = emoji.picker(ui, p, icons) {
                    chat.input.push(c);
                    close = !ui.input(|i| i.modifiers.shift);
                }
            });
        });
    let away = shown.response.clicked_elsewhere() && !ui.rect_contains_pointer(anchor.expand(4.0));
    if close || away || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        chat.bar_emoji_open = false;
        chat.focus_request = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(w: f32) -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(w, w * 9.0 / 16.0))
    }

    /// The mockup's dock: 17 buttons (Speak among them) centred on the
    /// screen, 34 px buttons (32 at 1280), the chat island 380 px (300).
    #[test]
    fn the_dock_is_centred_after_the_chat_island() {
        for (w, chat_w, button) in [(1280.0, 300.0, 32.0), (1536.0, 380.0, 34.0), (1920.0, 380.0, 34.0)] {
            let s = screen(w);
            let g = geometry(s, 0.0, 17, true);
            assert_eq!(g.chat.width(), chat_w);
            assert_eq!(g.button, button);
            assert_eq!(g.chat.left(), MARGIN);
            assert_eq!(g.chat.bottom(), s.bottom() - MARGIN);
            assert_eq!(g.dock.bottom(), s.bottom() - MARGIN);
            assert_eq!(g.dock.height(), DOCK_H);
            assert_eq!(g.chat.height(), CHAT_H);
            assert!((g.dock.center().x - s.center().x).abs() <= 1.0, "{w}: dock {:?}", g.dock);
            assert!(g.dock.left() >= g.chat.right() + AFTER_CHAT);
            assert_eq!(g.dock.width(), PAD * 2.0 + 16.0 * button + DOCK_MIC_W + GAP * 16.0);
        }
    }

    /// A wide chat island pushes the dock right; the dock never leaves the
    /// screen: its buttons narrow, then the chat island gives way.
    #[test]
    fn the_dock_stays_on_screen() {
        let s = screen(1280.0);
        let g = geometry(s, 500.0, 17, true);
        assert_eq!(g.chat.width(), 500.0);
        assert_eq!(g.dock.left(), g.chat.right() + AFTER_CHAT);
        assert!(g.dock.right() <= s.right() - MARGIN);
        assert!(g.button < 32.0 && g.button >= MIN_BUTTON);
        let g = geometry(s, 2000.0, 30, true);
        assert!(g.dock.right() <= s.right() - MARGIN + 0.5, "{:?}", g);
        assert!(g.chat.width() >= MIN_CHAT);
        // an empty dock keeps a slot to drop a button on
        let g = geometry(s, 0.0, 0, false);
        assert_eq!(g.dock.width(), PAD * 2.0 + BUTTON_H);
    }
}

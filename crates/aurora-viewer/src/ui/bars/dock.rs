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

use super::islands::{GAP, MARGIN, PAD, area, slot_face, slot_ink};
use super::{BarAction, ButtonLook, DOCK_MIC_W, MicButton, Row, natural_width, paint_icon, sizing, toolbar_row};
use crate::theme::Palette;
use crate::ui::Panels;
use crate::ui::icons::Icons;
use crate::ui::toolbar::{Align, ButtonMode, Style};
use egui::{Id, Pos2, Rect, RichText, Sense, Vec2};

/// Room between an island's edge and its buttons, above and below.
const V_PAD: f32 = 4.0;
/// Icons of the dock's buttons (smaller in the reduced buttons).
pub(super) const BUTTON_ICON: f32 = 18.0;
/// The reduced buttons, when the others do not fit in the screen's width.
pub(super) const MIN_BUTTON: f32 = 24.0;
/// Room between the chat island and the dock, at the least.
const AFTER_CHAT: f32 = 12.0;
/// Narrowest chat island.
const MIN_CHAT: f32 = 200.0;
/// Room in the chat field before the text and after the emoji.
const FIELD_PAD: f32 = 9.0;
const EMOJI_ICON: f32 = 15.0;

/// A screen under 1408 px wide (the mockup's 1280) takes the narrower
/// chat island and buttons.
pub(super) fn narrow(screen: Rect) -> bool {
    screen.width() < 1408.0
}

/// The chat island's width when the user has not dragged it (0 saved).
pub fn default_chat_width(screen: Rect) -> f32 {
    if narrow(screen) { 300.0 } else { 380.0 }
}

/// Where the bottom islands stand on `screen`: the chat island, the dock,
/// the size of the dock's buttons (their height; square with their icon
/// alone, the Speak button `DOCK_MIC_W` wide), each button's width, and the
/// one height of both islands, the buttons' and their margins (the human's
/// rule: the two islands share their top and their bottom).
#[derive(Debug, Clone, PartialEq)]
pub struct Geometry {
    pub chat: Rect,
    pub dock: Rect,
    pub button: f32,
    pub height: f32,
    pub widths: Vec<f32>,
}

impl Geometry {
    /// What the windows keep free above the bottom islands: the islands
    /// and the usual 8 px gap (`islands::work_area`).
    pub fn reserved(&self) -> f32 {
        MARGIN + self.height + MARGIN
    }
}

/// The width of a row of buttons `widths` wide; an empty dock keeps one
/// slot, where a button can be dropped.
fn row_width(widths: &[f32], button: f32) -> f32 {
    if widths.is_empty() {
        return PAD * 2.0 + button;
    }
    PAD * 2.0 + widths.iter().sum::<f32>() + GAP * (widths.len() - 1) as f32
}

/// The dock of `buttons` icons (Speak among them when `mic`), centred with
/// fixed buttons: the layout before PR 6, for the tests.
#[cfg(test)]
pub fn geometry(screen: Rect, chat_width: f32, buttons: usize, mic: bool) -> Geometry {
    geometry_styled(screen, chat_width, Style::default(), |square| {
        (0..buttons).map(|i| if mic && i == 0 { DOCK_MIC_W } else { square }).collect()
    })
}

/// The bottom islands' layout: the chat island on the left, `chat_width`
/// wide (its default when 0), the dock centred on the screen (or at the
/// start or end of the room after the chat island, by `style.align`) but at
/// least `AFTER_CHAT` after the chat island, its buttons as wide as
/// `natural` gives them for a button height and `style.sizing` lays them
/// out (`Fill`: the whole room). When they do not fit, they all take the
/// reduced size (`MIN_BUTTON`) and their labels are cut, and a chat island
/// that would still push the dock out shrinks. Both islands are as high as
/// the buttons and their margins.
pub fn geometry_styled(screen: Rect, chat_width: f32, style: Style, natural: impl Fn(f32) -> Vec<f32>) -> Geometry {
    let bottom = screen.bottom() - MARGIN;
    let left = screen.left() + MARGIN;
    let right = screen.right() - MARGIN;
    let labelled = style.mode != ButtonMode::IconsOnly;
    let least = natural(MIN_BUTTON);
    let least = if labelled { vec![MIN_BUTTON; least.len()] } else { least };
    let wanted = if chat_width > 0.0 { chat_width } else { default_chat_width(screen) };
    let max_chat = (right - left - AFTER_CHAT - row_width(&least, MIN_BUTTON)).max(MIN_CHAT);
    let chat_w = wanted.clamp(MIN_CHAT, max_chat);
    let chat_right = left + chat_w;
    let start = chat_right + AFTER_CHAT;
    let room = right - start;
    let row = |button: f32| sizing::fit(&natural(button), style.sizing, labelled, room - PAD * 2.0, GAP, MIN_BUTTON);
    let mut button = if narrow(screen) { 32.0 } else { 34.0 };
    let mut widths = row(button);
    if row_width(&widths, button) > room {
        button = MIN_BUTTON;
        widths = row(button);
    }
    let width = row_width(&widths, button);
    let x = match style.align {
        // centred on the screen, as the mockup's dock
        Align::Center => (left + ((right - left - width) / 2.0).round()).max(start),
        align => sizing::place(align, start, right, width),
    };
    let height = button + V_PAD * 2.0;
    let chat = Rect::from_min_max(Pos2::new(left, bottom - height), Pos2::new(chat_right, bottom));
    let dock = Rect::from_min_max(Pos2::new(x, bottom - height), Pos2::new(x + width, bottom));
    Geometry {
        chat,
        dock,
        button,
        height,
        widths,
    }
}

/// The bottom islands. Returns what the chat field sent, the dock's
/// action, the top of what the islands keep (`Geometry::reserved` above the screen's
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
    let style = row.style;
    let geo = geometry_styled(screen, *chat_width, style, |square| {
        buttons
            .iter()
            .map(|b| natural_width(&ctx, b, style.mode, square, BUTTON_ICON.min(square - 6.0), DOCK_MIC_W))
            .collect()
    });
    let mut action = BarAction::None;
    let mut committed = false;

    let out = area(&ctx, p, "chat", geo.chat, |ui| {
        chat_island(
            ui,
            p,
            icons,
            panels,
            chat,
            gestures,
            emoji,
            row,
            geo.clone(),
            chat_width,
            &mut committed,
        )
    });

    area(&ctx, p, "dock", geo.dock, |ui| {
        // the dock's background: its right-click menu and the drop target
        let target = ui.interact(geo.dock, Id::new("toolbar_background"), Sense::click());
        toolbar.context(&target, p, row.bar, style, None, row.locked);
        let inner = geo.dock.shrink2(Vec2::new(PAD, V_PAD));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.x = GAP;
        // Speak keeps its width for its arrow, within the common height
        let look = |b: &crate::ui::skin::ToolButton| {
            let at = buttons.iter().position(|x| x.command == b.command).unwrap_or(0);
            ButtonLook {
                size: Vec2::new(geo.widths.get(at).copied().unwrap_or(geo.button), geo.button),
                icon: BUTTON_ICON.min(geo.button - 6.0),
                dock: true,
                mode: style.mode,
                left: false,
                opens: egui::RectAlign::TOP,
                editing: row.editing,
            }
        };
        let rects = toolbar_row(&mut child, p, icons, buttons, look, toolbar, panels, mic, row, &mut action);
        if cfg!(test)
            && let Some(speak) = buttons.iter().position(|b| b.command == "speak").and_then(|i| rects.get(i))
        {
            ctx.data_mut(|d| d.insert_temp(Id::new("dock_speak_rect"), *speak));
        }
        // a button dropped a little past either end still lands in the dock;
        // the marker spans the buttons' row less 2 px at each end, the
        // mockup's 28 px `.dropguide`
        let mut drop_rect = inner.expand2(Vec2::new(geo.button + PAD, 0.0));
        // A wider toolbar must not cover the chat's resize handle with a
        // hover-only drop target on the dock's higher layer.
        drop_rect.min.x = drop_rect.min.x.max(geo.chat.right() + AFTER_CHAT);
        let drop = ui.interact(drop_rect, Id::new("dock_drop"), Sense::hover());
        toolbar.drop_bar(&drop, row.bar, &rects, p, row.locked);
    });
    (out, action, screen.bottom() - geo.reserved(), committed)
}

/// The chat island: the field with "Nearby chat" as its placeholder and the
/// emoji picker at its end, the button of the local chat and the
/// conversations, and the width handle on the island's right edge (double
/// click: the default).
#[allow(clippy::too_many_arguments)]
fn chat_island(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    panels: &mut Panels,
    chat: &mut crate::ui::chat::ChatUi,
    gestures: &crate::gestures::Manager,
    mut emoji: Option<&mut crate::ui::emoji::Emoji>,
    row: &Row,
    geo: Geometry,
    chat_width: &mut f32,
    committed: &mut bool,
) -> Option<crate::ui::chat::OutgoingChat> {
    let island = geo.chat;
    // the field and the button, centred in the islands' common height
    let slot = geo.button - 4.0;
    let top = island.center().y - slot / 2.0;
    let button = Rect::from_min_size(Pos2::new(island.right() - PAD - slot, top), Vec2::splat(slot));
    let field = Rect::from_min_max(Pos2::new(island.left() + PAD, top), Pos2::new(button.left() - GAP, top + slot));
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
    let hint = RichText::new(aurora_i18n::tr!("common-nearby-chat")).size(12.5);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(text)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    // egui paints a placeholder in the weak text colour, whatever its own
    child.visuals_mut().weak_text_color = Some(p.muted_dim);
    let out = crate::ui::chat::bar_input(&mut child, p, chat, gestures, emoji.as_deref_mut(), |edit| {
        edit.frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(egui::FontId::proportional(12.5))
            .text_color(p.ink)
            .hint_text(hint)
            .desired_width(text.width())
            .vertical_align(egui::Align::Center)
    });
    if chat.bar_emoji_open {
        emoji_picker(ui, p, icons, chat, emoji, smile, Pos2::new(field.right(), island.top() - 6.0));
    }

    conversations_button(ui, p, icons, panels, chat, row, button);

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

/// The emoji picker of the chat field (the one of the Conversations
/// window), its bottom right corner at `corner`, above the island; a pick
/// goes at the end of the field, Shift keeps the picker open, a click away
/// from it and from its `button` closes it.
fn emoji_picker(
    ui: &egui::Ui,
    p: &Palette,
    icons: &Icons,
    chat: &mut crate::ui::chat::ChatUi,
    emoji: Option<&mut crate::ui::emoji::Emoji>,
    button: Rect,
    corner: Pos2,
) {
    let Some(emoji) = emoji else {
        chat.bar_emoji_open = false;
        return;
    };
    let mut close = false;
    let mut escape = false;
    let shown = egui::Area::new(Id::new("chat_island_emoji_picker"))
        .fade_in(false)
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .fixed_pos(corner)
        .show(ui.ctx(), |ui| {
            crate::ui::widgets::overlay(ui, p, egui::Margin::same(6), |ui| {
                ui.set_width(300.0);
                let picked = emoji.picker(ui, p, icons);
                if let Some(c) = picked.chosen {
                    chat.input.push(c);
                    close = !ui.input(|i| i.modifiers.shift);
                }
                escape = picked.escape;
            });
        });
    let away = shown.response.clicked_elsewhere() && !ui.rect_contains_pointer(button.expand(4.0));
    if escape {
        // the field gets the keyboard back, its caret where it was
        chat.bar_emoji_open = false;
        ui.ctx().memory_mut(|m| m.request_focus(crate::ui::chat::bar_input_id()));
    } else if close || away {
        chat.bar_emoji_open = false;
        chat.focus_request = true;
    }
}

/// The button of the chat island: the local chat and the conversations
/// (the human's choice: the Conversations button leaves the default dock).
/// It carries the one violet unread counter while the conversations are
/// closed or minimized, as Firestorm flashes its Conversations button
/// (LLIMMgr, flashCommand). A click shows the newest unread conversation,
/// else the local chat; on that conversation already, it closes the window.
fn conversations_button(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    panels: &mut Panels,
    chat: &mut crate::ui::chat::ChatUi,
    row: &Row,
    button: Rect,
) {
    let minimized = crate::ui::widgets::floater_minimized(ui.ctx(), "conversations");
    let shown = panels.chat && !minimized;
    let target = row.latest_unread.filter(|_| row.unread > 0);
    let on_target = shown
        && match target {
            Some(id) => chat.selected == Some(id),
            None => chat.selected.is_none() && !chat.contacts,
        };
    let resp = ui.interact(button, Id::new("chat_island_open"), Sense::click());
    slot_face(ui, p, button, resp.hovered(), shown);
    // its flash while the window is closed (`ButtonFlash`)
    if row.flash > 0.0 && !panels.chat {
        ui.painter()
            .rect_filled(button, ui.visuals().window_corner_radius, p.violet.gamma_multiply(row.flash));
    }
    paint_icon(
        ui,
        icons,
        "message-filled",
        button,
        super::islands::ICON,
        slot_ink(p, resp.hovered() || shown, true),
    );
    let badge = row.unread > 0 && !shown;
    if badge {
        // as on the dock's buttons: 3 px in from the right edge, 1 px above
        let slot = Rect::from_min_max(
            Pos2::new(button.left(), button.top() - 1.0),
            Pos2::new(button.right() - 5.0, button.bottom()),
        );
        super::islands::count(ui, p, slot, row.unread, 1.0, false);
    }
    let name = aurora_i18n::tr!("ui-bars-nearby-chat-and-conversations");
    let tip = if row.unread > 0 {
        aurora_i18n::tr!("ui-bars-unread", value0 = name.to_string(), unread = row.unread.to_string()).to_string()
    } else {
        name.to_string()
    };
    if resp.on_hover_text(tip).clicked() {
        if on_target {
            crate::ui::widgets::toggle_floater(ui.ctx(), "conversations", &mut panels.chat);
        } else {
            match target {
                Some(id) => chat.open(id),
                None => {
                    chat.selected = None;
                    chat.contacts = false;
                }
            }
            crate::ui::widgets::open_floater(ui.ctx(), "conversations", &mut panels.chat);
        }
    }
}

/// The skin's toolbar without the Conversations button, for the dock: the
/// chat island's button opens the conversations and counts their unread
/// lines. Only the default dock leaves it out: a saved dock that holds it
/// keeps it, and the toolbar's catalogue still offers it. Kept from one
/// frame to the next, built again when the skin's toolbar changes.
#[derive(Default)]
pub struct DockLayout {
    source: Vec<crate::ui::skin::ToolButton>,
    layout: Option<crate::ui::skin::Layout>,
}

impl DockLayout {
    pub fn of(&mut self, skin: &crate::ui::skin::Layout) -> &crate::ui::skin::Layout {
        if self.layout.is_none() || self.source != skin.toolbar {
            self.source.clone_from(&skin.toolbar);
            let mut layout = skin.clone();
            layout.toolbar.retain(|b| b.command != "conversations");
            self.layout = Some(layout);
        }
        self.layout.get_or_insert_with(|| skin.clone())
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
            assert_eq!(g.dock.height(), button + V_PAD * 2.0);
            assert_eq!(g.chat.height(), g.dock.height());
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
        let g = geometry(s, 700.0, 17, true);
        assert_eq!(g.chat.width(), 700.0);
        assert_eq!(g.dock.left(), g.chat.right() + AFTER_CHAT);
        assert!(g.dock.right() <= s.right() - MARGIN);
        assert_eq!(g.button, MIN_BUTTON);
        let g = geometry(s, 2000.0, 30, true);
        assert!(g.dock.right() <= s.right() - MARGIN + 0.5, "{:?}", g);
        assert!(g.chat.width() >= MIN_CHAT);
        // an empty dock keeps a slot to drop a button on
        let g = geometry(s, 0.0, 0, false);
        assert_eq!(g.dock.width(), PAD * 2.0 + g.button);
    }

    /// The chat island and the dock share their top and their bottom (the
    /// human's rule), at the three widths, with the full or the reduced
    /// buttons and a widened chat island; Speak stays within them.
    #[test]
    fn the_chat_island_and_the_dock_share_their_top_and_bottom() {
        for w in [1280.0, 1536.0, 1920.0] {
            for (chat, buttons) in [(0.0, 18), (700.0, 18), (0.0, 30), (900.0, 24)] {
                let g = geometry(screen(w), chat, buttons, true);
                let case = format!("{w} chat {chat} buttons {buttons}: {g:?}");
                assert!((g.chat.top() - g.dock.top()).abs() <= 0.5, "{case}");
                assert!((g.chat.bottom() - g.dock.bottom()).abs() <= 0.5, "{case}");
                assert!([32.0, 34.0, MIN_BUTTON].contains(&g.button), "{case}");
                assert_eq!(g.height, g.button + V_PAD * 2.0, "{case}");
            }
        }
        // the frame's Speak button, within the dock's buttons' row
        let mut bench = Bench::shown();
        bench.frame(vec![]);
        let speak = bench
            .ctx
            .data(|d| d.get_temp::<Rect>(Id::new("dock_speak_rect")))
            .expect("the dock's Speak button");
        let layout = crate::ui::skin::Layout::default();
        let docked = super::DockLayout::default().of(&layout).toolbar.len();
        let g = geometry(screen(1536.0), 0.0, docked, true);
        assert!(
            speak.top() >= g.dock.top() + V_PAD - 0.5 && speak.bottom() <= g.dock.bottom() - V_PAD + 0.5,
            "{speak:?} {g:?}"
        );
        assert_eq!(speak.height(), g.button);
    }

    /// One viewer frame of the bottom islands on a 1536 × 864 screen.
    struct Bench {
        ctx: egui::Context,
        panels: Panels,
        chat: crate::ui::chat::ChatUi,
        width: f32,
        toolbar: crate::ui::toolbar::ToolbarUi,
        unread: usize,
        latest: Option<uuid::Uuid>,
    }

    impl Bench {
        fn new() -> Self {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            Bench {
                ctx,
                panels: Panels::from_settings(&crate::settings::Settings::default()),
                chat: Default::default(),
                width: 0.0,
                toolbar: Default::default(),
                unread: 0,
                latest: None,
            }
        }

        /// The islands are egui areas: shown from their second frame on.
        fn shown() -> Self {
            let mut bench = Self::new();
            bench.frame(vec![]);
            bench.frame(vec![]);
            bench
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> (Option<crate::ui::chat::OutgoingChat>, f32, bool) {
            let p = crate::theme::Theme::default().palette();
            let layout = crate::ui::skin::Layout::default();
            let settings = crate::ui::toolbar::ToolbarSettings::default();
            let view = super::super::favorites::View::default();
            let row = Row {
                unread: self.unread,
                latest_unread: self.latest,
                flash: 0.0,
                flying: false,
                seated: false,
                mouselook: false,
                ao_active: false,
                locked: false,
                favorites: &view,
                bar: crate::ui::toolbar::Bar::Bottom,
                style: Style::default(),
                editing: false,
            };
            let mut result = (None, 0.0, false);
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen(1536.0)),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let (out, _, top, set) = bottom(
                        ui,
                        &p,
                        &Icons::default(),
                        &layout,
                        &mut self.toolbar,
                        &settings,
                        &mut self.panels,
                        &mut self.chat,
                        &crate::gestures::Manager::default(),
                        None,
                        &row,
                        &mut self.width,
                        &mut MicButton::default(),
                    );
                    result = (out, top, set);
                },
            );
            output.textures_delta.clear();
            result
        }
    }

    fn key(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        }
    }

    /// The island's field is the chat bar's: Enter says, Ctrl+Enter shouts,
    /// `/5` picks the channel, and the islands keep the 56 px of the dock.
    #[test]
    fn the_chat_island_says_and_shouts_like_the_chat_bar() {
        let mut bench = Bench::shown();
        bench.chat.focus_request = true;
        let (_, top, _) = bench.frame(vec![]);
        assert_eq!(top, 864.0 - geometry(screen(1536.0), 0.0, 17, true).reserved());
        bench.frame(vec![egui::Event::Text("hello".into())]);
        let (out, _, _) = bench.frame(vec![key(egui::Key::Enter, true, egui::Modifiers::NONE)]);
        let out = out.expect("said");
        assert_eq!((out.message.as_str(), out.channel), ("hello", 0));
        assert_eq!(out.chat_type, aurora_net::ChatType::Normal);
        assert!(bench.chat.input.is_empty());
        bench.chat.focus_request = true;
        bench.frame(vec![]);
        bench.frame(vec![egui::Event::Text("/5 loud".into())]);
        let (out, _, _) = bench.frame(vec![
            egui::Event::ModifiersChanged(egui::Modifiers::CTRL),
            key(egui::Key::Enter, true, egui::Modifiers::CTRL),
        ]);
        let out = out.expect("shouted");
        assert_eq!((out.message.as_str(), out.channel), ("loud", 5));
        assert_eq!(out.chat_type, aurora_net::ChatType::Shout);
    }

    /// The Enter that starts the chat (the viewer's shortcut, in the same
    /// frame as its request) gives the field the keyboard: it is not taken
    /// as Enter on an empty bar, which hands the keyboard back to the world.
    #[test]
    fn the_enter_that_starts_the_chat_keeps_the_keyboard() {
        let mut bench = Bench::shown();
        bench.frame(vec![]);
        bench.chat.focus_request = true;
        bench.frame(vec![key(egui::Key::Enter, true, egui::Modifiers::NONE)]);
        bench.frame(vec![]);
        assert!(bench.chat.has_focus);
        bench.frame(vec![egui::Event::Text("hi".into())]);
        assert_eq!(bench.chat.input, "hi");
        // Enter on the empty bar still returns to the world
        bench.chat.input.clear();
        bench.frame(vec![key(egui::Key::Enter, true, egui::Modifiers::NONE)]);
        bench.frame(vec![]);
        assert!(!bench.chat.has_focus);
    }

    /// With "keep the chat bar active" (`keep_focus`): a message sent leaves
    /// the field with the keyboard, Enter on the empty field gives it back to
    /// the world, and the next Enter (the viewer's chat shortcut) takes it
    /// again. Off, a message sent lets the keyboard go, as before.
    #[test]
    fn the_chat_bar_can_keep_the_keyboard_after_sending() {
        let mut bench = Bench::shown();
        bench.chat.keep_focus = true;
        bench.chat.focus_request = true;
        bench.frame(vec![]);
        bench.frame(vec![egui::Event::Text("one".into())]);
        let (out, _, _) = bench.frame(vec![key(egui::Key::Enter, true, egui::Modifiers::NONE)]);
        assert_eq!(out.expect("sent").message, "one");
        bench.frame(vec![]);
        bench.frame(vec![]);
        assert!(bench.chat.has_focus, "the bar kept the keyboard after sending");
        // the next message goes straight in
        bench.frame(vec![egui::Event::Text("two".into())]);
        assert_eq!(bench.chat.input, "two");
        let (out, _, _) = bench.frame(vec![key(egui::Key::Enter, true, egui::Modifiers::NONE)]);
        assert_eq!(out.expect("sent").message, "two");
        bench.frame(vec![]);
        bench.frame(vec![]);
        assert!(bench.chat.has_focus);
        // Enter on the empty bar: back to the world
        let (out, _, _) = bench.frame(vec![key(egui::Key::Enter, true, egui::Modifiers::NONE)]);
        assert!(out.is_none());
        bench.frame(vec![]);
        assert!(!bench.chat.has_focus, "Enter on the empty bar let the keyboard go");
        // the viewer's Enter shortcut asks for the bar again
        bench.chat.focus_request = true;
        bench.frame(vec![key(egui::Key::Enter, true, egui::Modifiers::NONE)]);
        bench.frame(vec![]);
        assert!(bench.chat.has_focus, "the next Enter took the bar again");

        // off: a message sent lets the keyboard go
        let mut bench = Bench::shown();
        bench.chat.focus_request = true;
        bench.frame(vec![]);
        bench.frame(vec![egui::Event::Text("three".into())]);
        let (out, _, _) = bench.frame(vec![key(egui::Key::Enter, true, egui::Modifiers::NONE)]);
        assert!(out.is_some());
        bench.frame(vec![]);
        bench.frame(vec![]);
        assert!(!bench.chat.has_focus, "off: the bar let the keyboard go after sending");
    }

    /// The handle on the island's right edge sets its width, saved on
    /// release; a double click goes back to the default.
    #[test]
    fn the_chat_island_width_follows_its_handle() {
        let mut bench = Bench::shown();
        let buttons = crate::ui::skin::Layout::default().toolbar.len();
        let g = geometry(screen(1536.0), 0.0, buttons, true);
        let from = Pos2::new(g.chat.right() - 1.0, g.chat.center().y);
        let target = bench.ctx.read_response(Id::new("dock_drop")).expect("the toolbar drop target");
        assert!(
            !target.rect.contains(from),
            "the toolbar target must leave the chat handle interactive"
        );
        let to = from + Vec2::new(60.0, 0.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        bench.frame(vec![egui::Event::PointerMoved(from), button(from, true)]);
        bench.frame(vec![egui::Event::PointerMoved(from + Vec2::new(30.0, 0.0))]);
        bench.frame(vec![egui::Event::PointerMoved(to)]);
        let (_, _, set) = bench.frame(vec![button(to, false)]);
        assert!(set, "saved on release");
        // egui counts the drag from its threshold on: 30 to 60 px wider
        assert!((410.0..=441.0).contains(&bench.width), "{}", bench.width);
        let g = geometry(screen(1536.0), bench.width, buttons, true);
        assert!(g.dock.left() >= g.chat.right() + AFTER_CHAT);
    }
    /// The default dock leaves the Conversations button to the chat
    /// island; a saved dock that holds it keeps it.
    #[test]
    fn the_default_dock_leaves_conversations_to_the_chat_island() {
        let skin = crate::ui::skin::Layout::default();
        assert!(skin.toolbar.iter().any(|b| b.command == "conversations"));
        let mut dock = DockLayout::default();
        let layout = dock.of(&skin);
        assert!(layout.toolbar.iter().all(|b| b.command != "conversations"));
        assert_eq!(layout.toolbar.len(), skin.toolbar.len() - 1);
        let saved = crate::ui::toolbar::ToolbarSettings {
            buttons: Some(skin.toolbar.clone()),
            ..Default::default()
        };
        assert!(saved.buttons(dock.of(&skin)).iter().any(|b| b.command == "conversations"));
    }

    /// The chat island's button: the newest unread conversation while there
    /// are unread lines, else the local chat; a second click on the same
    /// tab closes the window.
    #[test]
    fn the_chat_button_opens_the_newest_unread_conversation_or_the_local_chat() {
        let mut bench = Bench::shown();
        let g = geometry(screen(1536.0), 0.0, 17, true);
        let at = Pos2::new(g.chat.right() - PAD - (g.button - 4.0) / 2.0, g.chat.center().y);
        let click = |bench: &mut Bench| {
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            bench.frame(vec![egui::Event::PointerMoved(at), button(true)]);
            bench.frame(vec![button(false)]);
        };
        let im = uuid::Uuid::from_u128(42);
        bench.panels.chat = false;
        bench.unread = 3;
        bench.latest = Some(im);
        click(&mut bench);
        assert!(bench.panels.chat);
        assert_eq!(bench.chat.selected, Some(im));
        click(&mut bench);
        assert!(!bench.panels.chat, "the same conversation again closes the window");
        bench.unread = 0;
        bench.latest = None;
        bench.chat.selected = Some(im);
        click(&mut bench);
        assert!(bench.panels.chat);
        assert_eq!(bench.chat.selected, None, "the local chat");
    }
}

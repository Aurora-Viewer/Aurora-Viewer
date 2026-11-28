//! Live chrome blocks are measured before bars and painted into their reserved
//! slots afterwards. This keeps a single toolbar layout on every edge while the
//! original navigation, audio and notification widgets keep their live state.

use super::{BarAction, BellState, StatusInfo, islands};
use crate::theme::Palette;
use crate::ui::{
    Panels,
    icons::Icons,
    skin::ToolButton,
    toolbar::{Bar, Style, ToolbarSettings, ToolbarUi, structure},
};
use egui::{Id, Rect, Vec2};

#[derive(Clone, Default)]
struct Metrics {
    widths: [f32; structure::BLOCKS.len()],
    active: bool,
    editing: bool,
    conversations_vertical: bool,
    conversations_height: f32,
    /// What the chat block takes beside its field: the recent emojis' row.
    chat_recent: f32,
    /// Whether the chat block draws its field (else its button alone, or
    /// nothing while the interface is edited).
    chat_field: bool,
    /// What the chat block's island is raised by over its row of the bar
    /// (`lift_chat`), instead of the row being as high as it; None in a
    /// bar that does not raise it.
    chat_lift: Option<f32>,
    /// The chat island of the bottom dock standing over the dock's row this
    /// frame (the recent emojis' row, a paragraph): its sides, and what it
    /// is raised by. The dock says so (`raise_chat`), for the side bars
    /// (`side_room`).
    chat_raised: Option<(egui::Rangef, f32)>,
}

#[derive(Clone)]
struct Slot {
    button: ToolButton,
    rect: Rect,
    clip: Rect,
    layer: egui::LayerId,
    bar: Bar,
    style: Style,
    locked: bool,
    placeholder: bool,
    floating: bool,
}

/// Where the chat block was drawn in this frame, for what stands by it
/// (the nearby chat lines over the world, `ui::chat::LinesPlace`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChatPlace {
    /// The block, its raised part included (`lift_chat`).
    pub rect: Rect,
    pub bar: Bar,
    /// In a floating block, not on a bar.
    pub floating: bool,
    frame: u64,
}

impl ChatPlace {
    #[cfg(test)]
    pub fn at(rect: Rect, bar: Bar, floating: bool) -> Self {
        Self {
            rect,
            bar,
            floating,
            frame: 0,
        }
    }
}

fn chat_place_id() -> Id {
    Id::unique("toolbar_chat_place")
}

/// The chat block of this frame, when a bar drew it (`show_chat`, earlier
/// in the frame): none while the interface is hidden or edited.
pub fn chat_place(ctx: &egui::Context) -> Option<ChatPlace> {
    let frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    ctx.data(|data| data.get_temp::<ChatPlace>(chat_place_id()))
        .filter(|place| place.frame == frame)
}

fn metrics_id() -> Id {
    Id::unique("toolbar_chrome_metrics")
}
fn slots_id() -> Id {
    Id::unique("toolbar_chrome_slots")
}

fn index(command: &str) -> Option<usize> {
    structure::BLOCKS.iter().position(|id| *id == command)
}

fn system_item(command: &str) -> Option<islands::Item> {
    Some(match command {
        "bar_clock" => islands::Item::Clock,
        "bar_ping" => islands::Item::Ping,
        "bar_fps" => islands::Item::Fps,
        "bar_network" => islands::Item::Net,
        "bar_media" => islands::Item::MediaButton,
        "bar_music" => islands::Item::Music,
        "bar_volume" => islands::Item::Volume,
        _ => return None,
    })
}

pub fn prepare(ui: &egui::Ui, p: &Palette, st: &StatusInfo, settings: &ToolbarSettings, editing: bool) {
    let shown = ToolbarUi::layout_configuration(ui.ctx());
    let settings = shown.as_deref().unwrap_or(settings);
    // Side toolbars follow LLToolBarEnums::getOrientation (indra/llui/lltoolbar.cpp).
    // Extending that orientation to conversation icons is a human-requested improvement.
    let mut widths = [
        if settings.address_visible.unwrap_or(true) {
            super::place::minimum_width(ui, st)
        } else {
            0.0
        },
        islands::account_width(ui, st),
        islands::system_width(ui, st),
        islands::SLOT,
        0.0,
        islands::account_part_width(ui, p, st, islands::AccountPart::Status),
        islands::account_part_width(ui, p, st, islands::AccountPart::Balance),
        islands::system_part_width(ui, st, islands::Item::Clock),
        islands::system_part_width(ui, st, islands::Item::Ping),
        islands::system_part_width(ui, st, islands::Item::Fps),
        islands::system_part_width(ui, st, islands::Item::Net),
        islands::system_part_width(ui, st, islands::Item::MediaButton),
        islands::system_part_width(ui, st, islands::Item::Music),
        islands::system_part_width(ui, st, islands::Item::Volume),
        super::dock::default_chat_width(ui.ctx().content_rect()),
    ];
    let conversations_vertical = settings
        .left
        .iter()
        .chain(&settings.right)
        .any(|button| button.command == "bar_conversations")
        || settings.drawer_of("bar_conversations").is_some_and(|id| {
            let drawer = &settings.drawers[id];
            let bar = settings.location(id, &crate::ui::skin::Layout::default()).unwrap_or(Bar::Left);
            crate::ui::toolbar::drawers::vertical(
                bar,
                drawer
                    .buttons
                    .iter()
                    .filter(|button| editing || index(&button.command).is_none_or(|i| i == 4 || widths[i] > 0.0)),
                drawer.mode,
                settings,
            )
        });
    let screen = ui.ctx().content_rect();
    let conversation_extent = crate::ui::chat::thumbnails::top_width(
        st.conversation_thumbnails,
        islands::SLOT,
        islands::PAD,
        if conversations_vertical {
            screen.height() / 2.0
        } else {
            screen.width() / 3.0
        },
    );
    widths[4] = if conversations_vertical && conversation_extent > 0.0 {
        islands::HEIGHT
    } else {
        conversation_extent
    };
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            metrics_id(),
            Metrics {
                widths,
                active: settings.composed,
                editing,
                conversations_vertical,
                conversations_height: conversation_extent,
                chat_recent: 0.0,
                chat_field: false,
                chat_lift: None,
                chat_raised: None,
            },
        );
        data.insert_temp(slots_id(), Vec::<Slot>::new());
        data.remove_temp::<f32>(Id::unique("bars_favorites_row_bottom"));
    });
}

pub fn width(ctx: &egui::Context, command: &str) -> Option<f32> {
    let i = index(command)?;
    Some(ctx.data(|data| data.get_temp::<Metrics>(metrics_id())).map_or(
        [
            220.0, 112.0, 360.0, 28.0, 0.0, 42.0, 80.0, 110.0, 70.0, 50.0, 80.0, 36.0, 36.0, 36.0, 380.0,
        ][i],
        |metrics| {
            if metrics.editing && metrics.widths[i] == 0.0 {
                if i == 4 && metrics.conversations_vertical {
                    islands::HEIGHT
                } else {
                    100.0
                }
            } else {
                metrics.widths[i]
            }
        },
    ))
}

pub fn height(ctx: &egui::Context, command: &str) -> f32 {
    if command == "bar_chat" {
        // the field, and the room of the recent emojis' row the island draws
        // over it: without it the row took the field's, and what was typed
        // went under the bar. A bar
        // that raises the block's island instead keeps its height
        // (`lift_chat`)
        let (recent, lift) = ctx
            .data(|data| data.get_temp::<Metrics>(metrics_id()))
            .map_or((0.0, 0.0), |metrics| (metrics.chat_recent, metrics.chat_lift.unwrap_or(0.0)));
        return chat_editor(ctx) + 8.0 + recent - lift;
    }
    if command == "bar_conversations"
        && let Some(metrics) = ctx.data(|data| data.get_temp::<Metrics>(metrics_id()))
        && metrics.conversations_vertical
    {
        return metrics.conversations_height.max(islands::HEIGHT);
    }
    islands::HEIGHT
}

/// The height of the chat block's field: a line, or the paragraph typed.
fn chat_editor(ctx: &egui::Context) -> f32 {
    crate::ui::chat::input::height(ctx, crate::ui::chat::bar_input_id(), islands::SLOT).max(islands::SLOT)
}

pub fn register(ui: &mut egui::Ui, button: &ToolButton, row: &super::Row, size: Vec2) -> Option<Rect> {
    let active = ui
        .ctx()
        .data(|data| data.get_temp::<Metrics>(metrics_id()))
        .is_some_and(|metrics| metrics.active);
    if !structure::is_block(&button.command) && !(active && matches!(button.command.as_str(), "aurora_menu" | "report_bug")) {
        return None;
    }
    if width(ui.ctx(), &button.command) == Some(0.0) {
        return Some(Rect::NOTHING);
    }
    let size = Vec2::new(
        size.x,
        if structure::is_block(&button.command) {
            size.y.max(height(ui.ctx(), &button.command))
        } else {
            size.y
        },
    );
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    register_at(ui, button, row, rect);
    Some(rect)
}

pub(super) fn register_at(ui: &egui::Ui, button: &ToolButton, row: &super::Row, rect: Rect) {
    let placeholder = index(&button.command).is_some_and(|i| {
        ui.ctx()
            .data(|data| data.get_temp::<Metrics>(metrics_id()))
            .is_some_and(|metrics| metrics.widths[i] == 0.0)
    });
    let slot = Slot {
        button: button.clone(),
        rect,
        clip: ui.clip_rect(),
        layer: ui.layer_id(),
        bar: row.bar,
        style: crate::ui::toolbar::blocks::style(ui.ctx(), &button.command, row.style),
        locked: row.locked,
        placeholder,
        floating: row.floating,
    };
    ui.ctx()
        .data_mut(|data| data.get_temp_mut_or_default::<Vec<Slot>>(slots_id()).push(slot));
}

/// Chat shares the same slots and edit interception as all native controls,
/// while its outgoing message still follows the existing local-chat handler.
pub fn prepare_chat(ctx: &egui::Context, chat: &crate::ui::chat::ChatUi, width: f32) {
    let width = super::chat_bar::width(ctx.content_rect(), width);
    // the row comes and goes with the field (`dock::chat_island`), which
    // the interface's edit mode does not draw
    let field = chat.bar_shown() && !ToolbarUi::intercepting(ctx);
    let recent = if field { chat.recent_emojis.height() } else { 0.0 };
    ctx.data_mut(|data| {
        let metrics = data.get_temp_mut_or_default::<Metrics>(metrics_id());
        metrics.widths[14] = if chat.bar_shown() || metrics.editing {
            width
        } else {
            islands::SLOT + islands::PAD * 2.0
        };
        metrics.chat_recent = recent;
        metrics.chat_field = field;
        metrics.chat_lift = None;
        metrics.chat_raised = None;
    });
}

/// The bottom dock's chat island stands `by` over the dock's row this
/// frame, between the sides `columns`. The island of its own and the dock's
/// chat block can both be raised: the room lost is the higher one's, and
/// the columns hold both.
pub(super) fn raise_chat(ctx: &egui::Context, columns: egui::Rangef, by: f32) {
    if by > 0.0 {
        ctx.data_mut(|data| {
            let raised = &mut data.get_temp_mut_or_default::<Metrics>(metrics_id()).chat_raised;
            *raised = Some(raised.map_or((columns, by), |(was, lift)| {
                (egui::Rangef::new(was.min.min(columns.min), was.max.max(columns.max)), lift.max(by))
            }));
        });
    }
}

/// The room of a side bar, from `room`, what the bars leave to the windows,
/// and the place the raised chat island takes in it, its margin included.
/// The windows keep clear of the chat island raised over the dock's row,
/// all along the screen; a side bar goes down to the dock's row. Its
/// buttons then keep their place when the recent emojis' row comes and
/// goes or a paragraph is typed, where a centred bar moved by half of what
/// the island rose (the human's report, 2026-11-27). Only a bar that would
/// stand over the raised island gives it room (`side::column`).
pub(super) fn side_room(ctx: &egui::Context, room: Rect) -> (Rect, Option<Rect>) {
    let Some((columns, by)) = ctx
        .data(|data| data.get_temp::<Metrics>(metrics_id()))
        .and_then(|metrics| metrics.chat_raised)
    else {
        return (room, None);
    };
    let own = Rect::from_min_max(room.min, egui::pos2(room.right(), room.bottom() + by));
    let raised = Rect::from_x_y_ranges(columns.expand(islands::MARGIN), room.bottom()..=own.bottom());
    (own, Some(raised))
}

/// The bar that holds the chat block raises the block's island over its
/// row, by what the field takes beyond a line (the recent emojis' row, the
/// paragraph typed), as the chat island did before the field became a
/// block: the block keeps a line's height, and the bar's other blocks
/// theirs. The top bar hangs the island under its row the same way
/// (`lifted`). What the island is raised by: nothing for a field of one
/// line, or for the block's button alone.
pub(super) fn lift_chat(ctx: &egui::Context) -> f32 {
    let editor = chat_editor(ctx);
    ctx.data_mut(|data| {
        let metrics = data.get_temp_mut_or_default::<Metrics>(metrics_id());
        let lift = if metrics.chat_field {
            editor - islands::SLOT + metrics.chat_recent
        } else {
            0.0
        };
        metrics.chat_lift = Some(lift);
        lift
    })
}

/// What the chat block's island is raised by over its row, in a bar that
/// raises it (`lift_chat`).
pub(super) fn chat_lift(ctx: &egui::Context) -> Option<f32> {
    ctx.data(|data| data.get_temp::<Metrics>(metrics_id()))
        .and_then(|metrics| metrics.chat_lift)
}

/// The chat block's rectangle with what its island stands out of its bar's
/// row by: over the dock's, under the top bar's.
fn lifted(rect: Rect, bar: Bar, lift: f32) -> Rect {
    let mut rect = rect;
    if bar == Bar::Top {
        rect.max.y += lift;
    } else {
        rect.min.y -= lift;
    }
    rect
}

#[allow(clippy::too_many_arguments)]
pub fn show_chat(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    panels: &mut Panels,
    chat: &mut crate::ui::chat::ChatUi,
    gestures: &crate::gestures::Manager,
    mut emoji: Option<&mut crate::ui::emoji::Emoji>,
    row: &super::Row,
    width: &mut f32,
) -> (Option<crate::ui::chat::OutgoingChat>, bool) {
    if ToolbarUi::intercepting(ui.ctx()) {
        return (None, false);
    }
    let slots = ui.ctx().data(|data| data.get_temp::<Vec<Slot>>(slots_id())).unwrap_or_default();
    let mut out = None;
    let mut committed = false;
    for slot in slots
        .into_iter()
        .filter(|slot| slot.button.command == "bar_chat" && !slot.placeholder)
    {
        let _context = crate::ui::toolbar::native_context::Scope::new(ui.ctx(), "bar_chat");
        let lift = if matches!(slot.bar, Bar::Bottom | Bar::Top) {
            chat_lift(ui.ctx())
        } else {
            None
        };
        let drawn = lifted(slot.rect, slot.bar, lift.unwrap_or(0.0));
        let place = ChatPlace {
            rect: drawn,
            bar: slot.bar,
            floating: slot.floating,
            frame: ui.ctx().cumulative_frame_nr_for(egui::ViewportId::ROOT),
        };
        ui.ctx().data_mut(|data| data.insert_temp(chat_place_id(), place));
        if let Some(lift) = lift {
            out = show_lifted_chat(
                ui.ctx(),
                p,
                icons,
                panels,
                chat,
                gestures,
                emoji.as_deref_mut(),
                row,
                &slot,
                lift,
                width,
                &mut committed,
            );
            continue;
        }
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("toolbar_chat_input")
                .layer_id(slot.layer)
                .max_rect(slot.rect),
        );
        child.set_clip_rect(slot.clip);
        let row = super::Row {
            bar: slot.bar,
            style: slot.style,
            ..*row
        };
        out = super::chat_bar::show(
            &mut child,
            p,
            icons,
            panels,
            chat,
            gestures,
            emoji.as_deref_mut(),
            &row,
            slot.rect,
            width,
            &mut committed,
        );
    }
    (out, committed)
}

/// The chat block of the dock, its island raised by `lift` over the dock's
/// row (`lift_chat`; the dock paints the raised surface), or of the top
/// bar, its island hung by `lift` under the bar's row.
///
/// What stands out of the dock is an area of its own, kept right over the
/// dock's: the pointer there is over the interface, not the world. The
/// whole block is drawn on that area's layer, so that its controls take the
/// clicks before the dock under them, in the dock as above it. The area is
/// there for a field of one line too, with no height: an area draws nothing
/// on its first frame, and the field would blink when its second line or
/// the first recent emoji raises the island.
#[allow(clippy::too_many_arguments)]
fn show_lifted_chat(
    ctx: &egui::Context,
    p: &Palette,
    icons: &Icons,
    panels: &mut Panels,
    chat: &mut crate::ui::chat::ChatUi,
    gestures: &crate::gestures::Manager,
    emoji: Option<&mut crate::ui::emoji::Emoji>,
    row: &super::Row,
    slot: &Slot,
    lift: f32,
    width: &mut f32,
    committed: &mut bool,
) -> Option<crate::ui::chat::OutgoingChat> {
    let rect = lifted(slot.rect, slot.bar, lift);
    let over = if slot.bar == Bar::Top {
        Rect::from_min_max(
            egui::pos2(rect.left() - islands::PAD, slot.rect.bottom() + islands::PAD),
            egui::pos2(rect.right() + islands::PAD, rect.bottom() + islands::PAD),
        )
    } else {
        Rect::from_min_max(
            egui::pos2(rect.left() - islands::PAD, rect.top() - islands::PAD),
            egui::pos2(rect.right() + islands::PAD, slot.rect.top() - islands::PAD),
        )
    };
    let row = super::Row {
        bar: slot.bar,
        style: slot.style,
        ..*row
    };
    if lift > 0.0 {
        // (the windows keep their title clear of it, as of the islands)
        crate::ui::layout::island_drawn(ctx, over);
    }
    let (out, layer) = egui::Area::new(Id::unique("bars_chat_lift"))
        .order(egui::Order::Middle)
        .fixed_pos(over.min)
        .constrain(false)
        .fade_in(false)
        .show(ctx, |ui| {
            ui.ctx().set_sublayer(slot.layer, ui.layer_id());
            let first = ui.painter().add(egui::Shape::Noop);
            ui.set_min_size(over.size());
            let mut child = ui.new_child(egui::UiBuilder::new().id_salt("toolbar_chat_input").max_rect(rect));
            let out = super::chat_bar::show(&mut child, p, icons, panels, chat, gestures, emoji, &row, rect, width, committed);
            // the bars' opacity, as on the dock's layer (`islands::area_surface`)
            crate::ui::surface_opacity::fade_after(ui.ctx(), ui.layer_id(), first, p, crate::ui::surface_opacity::bars());
            (out, ui.layer_id())
        })
        .inner;
    // (the block's own reading looks at one layer: it stands on two)
    chat.bar_hovered = ctx
        .pointer_interact_pos()
        .is_some_and(|pos| rect.contains(pos) && ctx.layer_id_at(pos).is_some_and(|at| at == layer || at == slot.layer));
    out
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    panels: &mut Panels,
    st: &StatusInfo,
    audio: &mut crate::settings::AudioSettings,
    audio_ui: &mut crate::ui::audio::AudioUi,
    media: crate::ui::audio::MediaState,
    notif: &mut crate::ui::notifications::NotifUi,
    bell: BellState,
    toolbar: &mut ToolbarUi,
    settings: &ToolbarSettings,
) -> (BarAction, crate::ui::audio::AudioAction) {
    let slots = ui.ctx().data(|data| data.get_temp::<Vec<Slot>>(slots_id())).unwrap_or_default();
    let mut action = BarAction::None;
    let mut audio_action = crate::ui::audio::AudioAction::None;
    let mut volume_shown = false;
    let tip_id = Id::unique("toolbar_system_tip_control");
    let previous_tip = ui.ctx().data(|data| data.get_temp::<String>(tip_id)).unwrap_or_default();
    let mut tip = None;
    for slot in slots {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("toolbar_chrome", &slot.button.command))
                .layer_id(slot.layer)
                .max_rect(slot.rect),
        );
        child.set_clip_rect(slot.clip);
        let intercepting = ToolbarUi::intercepting(child.ctx());
        if intercepting && (slot.placeholder || slot.button.command == "bar_chat") {
            let response = crate::ui::toolbar::edit_mode::cell(
                &mut child,
                p,
                icons,
                toolbar,
                &slot.button,
                slot.bar,
                slot.style,
                slot.locked,
                super::ButtonLook {
                    size: slot.rect.size(),
                    icon: islands::ICON,
                    dock: true,
                    mode: slot.style.mode,
                    left: slot.bar.vertical(),
                    opens: egui::RectAlign::BOTTOM_START,
                    editing: true,
                },
            );
            crate::ui::toolbar::edit_mode::source_menu(&response, p, toolbar, settings, &slot.button.command);
            continue;
        }
        if intercepting {
            crate::ui::toolbar::edit_mode::preview(&mut child);
        }
        let response = child.interact(
            slot.rect,
            if intercepting {
                Id::unique(("toolbar_chrome_preview", &slot.button.command))
            } else {
                super::tool_button_id(&slot.button.command)
            },
            egui::Sense::click_and_drag(),
        );
        if !intercepting && (slot.placeholder || matches!(slot.button.command.as_str(), "bar_chat" | "bar_conversations")) {
            toolbar.button(&response, slot.bar, slot.style, &slot.button, p, slot.locked);
        } else if !intercepting {
            toolbar.button_source(&response, slot.bar, slot.style, &slot.button, slot.locked);
        }
        if slot.placeholder {
            if slot.bar.vertical() && slot.button.command == "bar_conversations" {
                crate::ui::widgets::paint_glyph(child.painter(), "messages-filled", slot.rect.center(), islands::ICON, p.muted);
                response.on_hover_text(crate::ui::toolbar::label(&slot.button));
                continue;
            }
            crate::ui::widgets::paint_elided(
                &child,
                crate::ui::toolbar::label(&slot.button),
                slot.rect.left_center() + egui::vec2(4.0, 0.0),
                slot.rect.width() - 8.0,
                egui::FontId::proportional(12.0),
                p.muted,
            );
            continue;
        }
        let _context = (!intercepting && structure::is_block(&slot.button.command))
            .then(|| crate::ui::toolbar::native_context::Scope::new(child.ctx(), &slot.button.command));
        match slot.button.command.as_str() {
            "bar_navigation" => {
                let (rect, detail) = super::place::layout(&child, st, slot.rect);
                if let Some(next) = super::place::island(&mut child, p, icons, panels, st, rect, detail) {
                    action = next;
                }
                if st.favorites.access == crate::settings::FavoritesAccess::Row {
                    let (favorites, next) = super::favorites::row(child.ctx(), p, icons, st.favorites, rect);
                    if slot.bar == Bar::Top {
                        child
                            .ctx()
                            .data_mut(|data| data.insert_temp(Id::unique("bars_favorites_row_bottom"), favorites.bottom()));
                    }
                    if let Some(next) = next {
                        action = next;
                    }
                }
            }
            "bar_account" => islands::account(&mut child, p, st, slot.rect, &mut action),
            "bar_status" | "bar_balance" => islands::account_part(
                &mut child,
                p,
                st,
                slot.rect,
                &mut action,
                if slot.button.command == "bar_status" {
                    islands::AccountPart::Status
                } else {
                    islands::AccountPart::Balance
                },
            ),
            "bar_system" | "bar_clock" | "bar_ping" | "bar_fps" | "bar_network" | "bar_media" | "bar_music" | "bar_volume" => {
                let item = system_item(&slot.button.command);
                volume_shown |= item == Some(islands::Item::Volume) || (item.is_none() && st.system_island.media);
                let (next, hovered) = islands::system_parts(
                    &mut child,
                    p,
                    icons,
                    panels,
                    st,
                    audio,
                    audio_ui,
                    media,
                    slot.rect,
                    &mut action,
                    item,
                );
                if !matches!(next, crate::ui::audio::AudioAction::None) {
                    audio_action = next;
                }
                if item.is_none_or(|item| matches!(item, islands::Item::Clock | islands::Item::Ping | islands::Item::Net))
                    && (hovered
                        || (!tip.as_ref().is_some_and(|(_, _, hovered)| *hovered)
                            && (tip.is_none() || slot.button.command == previous_tip)))
                {
                    tip = Some((slot.button.command.clone(), slot.rect, hovered));
                }
            }
            "bar_notifications" => {
                let rect = Rect::from_center_size(slot.rect.center(), Vec2::splat(islands::SLOT));
                let mut bell_ui = child.new_child(egui::UiBuilder::new().max_rect(rect));
                let response = if bell.aurora {
                    crate::ui::notification_center::bell(&mut bell_ui, p, icons, &notif.center, bell.unread, notif.open, bell.dnd, true)
                } else {
                    crate::ui::notifications::bell(&mut bell_ui, p, icons, bell.unread, notif.open, bell.dnd, true)
                };
                notif.anchor = Some(egui::pos2(response.rect.left(), slot.rect.bottom()));
                notif.bell = Some(response.rect);
                let screen = child.ctx().content_rect();
                let inset = islands::MARGIN.min(screen.width().max(0.0) * 0.5);
                let width = crate::ui::notification_center::WIDTH.min((screen.width() - inset * 2.0).max(0.0));
                let right = slot.rect.right().clamp(screen.left() + inset + width, screen.right() - inset);
                notif.column = Some(Rect::from_min_max(
                    egui::pos2(right - width, screen.top() + islands::MARGIN),
                    egui::pos2(right, screen.top() + islands::MARGIN + islands::HEIGHT),
                ));
                if response.clicked() {
                    notif.open = !notif.open;
                }
                if super::bell_menu(&response, p, bell.dnd) {
                    action = BarAction::SetStatus(crate::ui::notifications::bell_dnd_toggled(st.status));
                }
            }
            "bar_conversations" => crate::ui::chat::thumbnails::reserve_toolbar(
                child.ctx(),
                slot.rect,
                islands::SLOT,
                slot.layer,
                slot.clip,
                slot.bar.vertical(),
            ),
            "aurora_menu" => {
                let response = super::top::native_styled(
                    &mut child,
                    p,
                    icons,
                    &slot.button,
                    slot.rect,
                    Id::unique(if intercepting { "bars_preview_menu" } else { "bars_island_menu" }),
                    slot.style.mode,
                );
                islands::menu_options(&response, p, st, &mut action, toolbar, settings.locked);
                toolbar.bar_source(&response, &slot.button, slot.bar, settings.locked);
                crate::ui::menu::dropdown_placed(
                    &response,
                    p,
                    egui::PopupCloseBehavior::CloseOnClickOutside,
                    Some((egui::RectAlign::BOTTOM_START, 10.0)),
                    |ui| {
                        crate::ui::menu::searchable(ui, p, |ui| crate::ui::toolbar::commands::show(ui, p, toolbar, settings, ""));
                    },
                );
            }
            "report_bug" => {
                let response = super::top::native_styled(
                    &mut child,
                    p,
                    icons,
                    &slot.button,
                    slot.rect,
                    Id::unique(if intercepting { "bars_preview_bug" } else { "bars_island_bug" }),
                    slot.style.mode,
                );
                toolbar.bar_source(&response, &slot.button, slot.bar, settings.locked);
                super::bug_menu(&response, p, toolbar, settings.locked, &mut action, false);
                if response.clicked() {
                    action = BarAction::ReportBug;
                }
            }
            _ => {}
        }
        // A native field may consume the pointer without owning right-click
        // actions. Give its remaining face the same final customization entry.
        if !intercepting
            && structure::is_block(&slot.button.command)
            && !matches!(slot.button.command.as_str(), "bar_chat" | "bar_conversations")
            && !crate::ui::toolbar::native_context::shown(child.ctx(), &slot.button.command)
        {
            let also = child
                .ctx()
                .input(|input| input.pointer.secondary_clicked().then_some(input.pointer.interact_pos()).flatten())
                .is_some_and(|pos| slot.rect.contains(pos) && child.ctx().layer_id_at(pos) == Some(slot.layer));
            crate::ui::menu::context_menu_also(&response, p, also, |_| {});
        }
        if intercepting {
            let mut target = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("toolbar_chrome_edit", &slot.button.command))
                    .layer_id(slot.layer)
                    .max_rect(slot.rect),
            );
            target.set_clip_rect(slot.clip);
            crate::ui::toolbar::edit_mode::native_target(
                &mut target,
                p,
                toolbar,
                &slot.button,
                slot.bar,
                slot.style,
                slot.locked,
                slot.rect,
            );
        }
    }
    if !volume_shown {
        audio_ui.panel_open = false;
    }
    if let Some((command, rect, hovered)) = tip {
        if hovered {
            ui.ctx().data_mut(|data| data.insert_temp(tip_id, command));
        }
        islands::system_tip(ui.ctx(), p, icons, st, media, rect, hovered && !audio_ui.panel_open, &mut action);
    }
    if let Some(next) = toolbar.drawer_action.take()
        && !matches!(next, BarAction::None)
    {
        action = next;
    }
    (action, audio_action)
}

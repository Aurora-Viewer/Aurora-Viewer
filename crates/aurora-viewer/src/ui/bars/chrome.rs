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
    widths: [f32; 5],
    active: bool,
    editing: bool,
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
}

fn metrics_id() -> Id {
    Id::unique("toolbar_chrome_metrics")
}
fn slots_id() -> Id {
    Id::unique("toolbar_chrome_slots")
}

fn index(command: &str) -> Option<usize> {
    Some(match command {
        "bar_navigation" => 0,
        "bar_account" => 1,
        "bar_system" => 2,
        "bar_notifications" => 3,
        "bar_conversations" => 4,
        _ => return None,
    })
}

pub fn prepare(ui: &egui::Ui, p: &Palette, st: &StatusInfo, settings: &ToolbarSettings, editing: bool) {
    let widths = [
        if settings.address_visible.unwrap_or(true) {
            super::place::minimum_width(ui, st)
        } else {
            0.0
        },
        islands::account_width(ui, p, st),
        islands::system_width(ui, st),
        islands::SLOT,
        crate::ui::chat::thumbnails::top_width(
            st.conversation_thumbnails,
            islands::SLOT,
            islands::PAD,
            ui.ctx().content_rect().width() / 3.0,
        ),
    ];
    ui.ctx().data_mut(|data| {
        data.insert_temp(
            metrics_id(),
            Metrics {
                widths,
                active: settings.composed,
                editing,
            },
        );
        data.insert_temp(slots_id(), Vec::<Slot>::new());
        data.remove_temp::<f32>(Id::unique("bars_favorites_row_bottom"));
    });
}

pub fn width(ctx: &egui::Context, command: &str) -> Option<f32> {
    let i = index(command)?;
    Some(
        ctx.data(|data| data.get_temp::<Metrics>(metrics_id()))
            .map_or([220.0, 112.0, 360.0, 28.0, 0.0][i], |metrics| {
                if metrics.editing && metrics.widths[i] == 0.0 {
                    100.0
                } else {
                    metrics.widths[i]
                }
            }),
    )
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
            size.y.max(islands::HEIGHT)
        } else {
            size.y
        },
    );
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
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
        style: row.style,
        locked: row.locked,
        placeholder,
    };
    ui.ctx()
        .data_mut(|data| data.get_temp_mut_or_default::<Vec<Slot>>(slots_id()).push(slot));
    Some(rect)
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
    for slot in slots {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("toolbar_chrome", &slot.button.command))
                .layer_id(slot.layer)
                .max_rect(slot.rect),
        );
        child.set_clip_rect(slot.clip);
        let response = child.interact(
            slot.rect,
            super::tool_button_id(&slot.button.command),
            egui::Sense::click_and_drag(),
        );
        toolbar.button(&response, slot.bar, slot.style, &slot.button, p, slot.locked);
        if slot.placeholder {
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
            "bar_system" => {
                let (next, hovered) = islands::system(&mut child, p, icons, panels, st, audio, audio_ui, media, slot.rect, &mut action);
                if !matches!(next, crate::ui::audio::AudioAction::None) {
                    audio_action = next;
                }
                islands::system_tip(
                    child.ctx(),
                    p,
                    icons,
                    st,
                    media,
                    slot.rect,
                    hovered && !audio_ui.panel_open,
                    &mut action,
                );
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
                if response.secondary_clicked() {
                    action = BarAction::SetStatus(crate::ui::notifications::bell_dnd_toggled(st.status));
                }
            }
            "bar_conversations" => {
                crate::ui::chat::thumbnails::reserve_toolbar(child.ctx(), slot.rect, islands::SLOT, slot.layer, slot.clip)
            }
            "aurora_menu" => {
                let response = super::top::native_styled(
                    &mut child,
                    p,
                    icons,
                    &slot.button,
                    slot.rect,
                    Id::unique("bars_island_menu"),
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
                    Id::unique("bars_island_bug"),
                    slot.style.mode,
                );
                toolbar.bar_source(&response, &slot.button, slot.bar, settings.locked);
                if response.secondary_clicked() {
                    action = BarAction::CopyVersion;
                }
                if response.clicked() {
                    action = BarAction::ReportBug;
                }
            }
            _ => {}
        }
    }
    (action, audio_action)
}

//! The nearby media panel, under the top bar's media button: every media
//! around with its tick, Start all / Stop all, a Show choice, and the
//! controls of the selected one (play, pause, stop, volume, mute, zoom).
//! It opens while the pointer is on the button, like the volume panel, and
//! a right click on a row copies its address.
//!
//! Port of LLPanelNearByMedia (indra/newview/llpanelnearbymedia.cpp,
//! panel_nearby_media.xml, menu_nearby_media.xml) and its opening from
//! LLStatusBar::onMouseEnterNearbyMedia (llstatusbar.cpp), originally
//! LGPL 2.1. The rows come from `media::nearby`.

use super::audio::{AudioUi, hover_step};
use super::{menu, widgets};
use crate::media::MediaKey;
use crate::media::nearby::{Row, Show, copy_data};
use crate::theme::Palette;
use egui::{RichText, Vec2};

/// Width of the panel (panel_nearby_media.xml: 328 px).
const WIDTH: f32 = 330.0;
/// Rows shown before the list scrolls.
const LIST_HEIGHT: f32 = 170.0;
const ROW_H: f32 = 22.0;

#[derive(Default)]
pub struct NearbyMediaUi {
    pub open: bool,
    left_at: Option<f64>,
    pub show: Show,
    pub selected: Option<MediaKey>,
}

/// What the panel asks for; the app applies it.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Start all (true) / Stop all (false).
    All(bool),
    /// A row's tick, or the Stop button (false).
    On(MediaKey, bool),
    Play(MediaKey),
    Pause(MediaKey),
    Volume(MediaKey, f32),
    Mute(MediaKey, bool),
    Zoom(MediaKey),
    Unzoom,
    Copy(String),
}

fn show_label(show: Show) -> &'static str {
    match show {
        Show::All => aurora_i18n::tr!("ui-nearby-media-show-all"),
        Show::InParcel => aurora_i18n::tr!("ui-nearby-media-show-in-parcel"),
        Show::OutsideParcel => aurora_i18n::tr!("ui-nearby-media-show-outside-parcel"),
        Show::OnOthers => aurora_i18n::tr!("ui-nearby-media-show-on-others"),
    }
}

fn row_name(row: &Row) -> String {
    let name = if row.key == MediaKey::Parcel {
        aurora_i18n::tr!("ui-nearby-media-parcel-media").to_owned()
    } else if row.name.is_empty() {
        aurora_i18n::tr!("ui-nearby-media-empty").to_owned()
    } else {
        row.name.clone()
    };
    if row.showing {
        aurora_i18n::tr!("ui-nearby-media-playing", name = name.as_str())
    } else {
        name
    }
}

/// The panel for one frame. `button`: the media button's rect and whether
/// the pointer is on it (set by `audio::top_controls`); `zoomed`: the media
/// the camera zoomed on. The volume panel closes when this one opens.
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    st: &mut NearbyMediaUi,
    audio: &mut AudioUi,
    rows: &[Row],
    zoomed: Option<MediaKey>,
    demo_open: bool,
) -> Vec<Action> {
    let mut out = Vec::new();
    let Some(button) = audio.media_button else {
        st.open = false;
        return out;
    };
    let on_button = audio.media_button_hovered || demo_open;
    if on_button {
        hover_step(&mut st.open, &mut st.left_at, true, false, false, 0.0);
        audio.panel_open = false;
    }
    if !st.open {
        return out;
    }
    // the selection follows the rows: a media gone leaves nothing selected
    if st.selected.is_some_and(|k| !rows.iter().any(|r| r.key == k)) {
        st.selected = None;
    }
    let anchor = egui::pos2(button.right() + 4.0, button.bottom() + 6.0);
    let area = egui::Area::new(egui::Id::unique("nearby_media_panel"))
        .fade_in(false)
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_TOP)
        .fixed_pos(anchor)
        .show(ctx, |ui| {
            widgets::overlay(ui, p, egui::Margin::symmetric(10, 8), |ui| {
                ui.set_width(WIDTH);
                body(ui, p, st, rows, zoomed, &mut out);
            });
        });
    let on_panel = area.response.contains_pointer();
    let (held, pressed, now) = ctx.input(|i| (i.pointer.any_down(), i.pointer.any_pressed(), i.time));
    // the Show choice and a row's menu are popups outside the panel
    let popup = egui::Popup::is_any_open(ctx);
    if !demo_open {
        if pressed && !on_panel && !on_button && !popup {
            st.open = false;
            st.left_at = None;
        } else if hover_step(&mut st.open, &mut st.left_at, on_button, on_panel || popup, held && !pressed, now) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(super::audio::CLOSE_DELAY));
        }
    }
    out
}

fn body(ui: &mut egui::Ui, p: &Palette, st: &mut NearbyMediaUi, rows: &[Row], zoomed: Option<MediaKey>, out: &mut Vec<Action>) {
    // Stop all, Start all, and the Show choice
    ui.horizontal(|ui| {
        let any_on = rows.iter().any(|r| r.on && !r.paused);
        let any_off = rows.iter().any(|r| !r.on || r.paused);
        if widgets::action_button_if(
            ui,
            p,
            "player-stop-filled",
            aurora_i18n::tr!("ui-nearby-media-stop-all"),
            widgets::ActionKind::Flat,
            Vec2::new(0.0, ROW_H),
            any_on,
        )
        .on_hover_text(aurora_i18n::tr!("ui-nearby-media-stop-all-tip"))
        .clicked()
        {
            out.push(Action::All(false));
        }
        if widgets::action_button_if(
            ui,
            p,
            "player-play-filled",
            aurora_i18n::tr!("ui-nearby-media-start-all"),
            widgets::ActionKind::Flat,
            Vec2::new(0.0, ROW_H),
            any_off,
        )
        .on_hover_text(aurora_i18n::tr!("ui-nearby-media-start-all-tip"))
        .clicked()
        {
            out.push(Action::All(true));
        }
    });
    // Show (show_combo), on its own line: its longest choices would run
    // into the buttons
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-nearby-media-show-label"))
                .size(12.0)
                .color(p.muted),
        );
        let resp = widgets::action_button(
            ui,
            p,
            "chevron-down",
            show_label(st.show),
            widgets::ActionKind::Flat,
            Vec2::new(0.0, ROW_H),
        )
        .on_hover_text(aurora_i18n::tr!("ui-nearby-media-show"));
        menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            for show in Show::ALL {
                if menu::check(ui, p, "filter-filled", show_label(show), st.show == show, true) {
                    st.show = show;
                }
            }
        });
    });
    ui.add_space(4.0);
    let shown: Vec<&Row> = rows.iter().filter(|r| st.show.shows(r.class)).collect();
    if shown.is_empty() {
        ui.label(RichText::new(aurora_i18n::tr!("ui-nearby-media-none")).size(12.0).color(p.muted));
    } else {
        egui::ScrollArea::vertical()
            .max_height(LIST_HEIGHT)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for row in shown {
                    list_row(ui, p, st, row, out);
                }
            });
    }
    ui.add_space(4.0);
    widgets::divider(ui, p);
    ui.add_space(2.0);
    controls(ui, p, st.selected.and_then(|k| rows.iter().find(|r| r.key == k)), zoomed, out);
}

fn list_row(ui: &mut egui::Ui, p: &Palette, st: &mut NearbyMediaUi, row: &Row, out: &mut Vec<Action>) {
    let selected = st.selected == Some(row.key);
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H), egui::Sense::click());
    if selected || resp.hovered() {
        let fill = if selected { p.raised } else { p.raised.gamma_multiply(0.5) };
        ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    }
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(4.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let mut on = row.on;
    if widgets::checkbox(&mut child, p, &mut on, "")
        .on_hover_text(if row.on {
            aurora_i18n::tr!("ui-nearby-media-turn-off")
        } else {
            aurora_i18n::tr!("ui-nearby-media-turn-on")
        })
        .changed()
    {
        out.push(Action::On(row.key, on));
    }
    // dimmed while it does not show (updateListItem: alpha 0.25)
    let color = if row.showing { p.ink } else { p.muted };
    child.add(
        egui::Label::new(RichText::new(row_name(row)).size(12.0).color(color))
            .truncate()
            .selectable(false),
    );
    let resp = if row.url.is_empty() { resp } else { resp.on_hover_text(&row.url) };
    if resp.clicked() || resp.secondary_clicked() {
        st.selected = Some(row.key);
    }
    menu::context_menu(&resp, p, |ui| {
        if menu::item_if(
            ui,
            p,
            "copy-filled",
            aurora_i18n::tr!("ui-nearby-media-copy-url"),
            !row.url.is_empty(),
        ) {
            out.push(Action::Copy(row.url.clone()));
        }
        if let Some(data) = copy_data(&row.url)
            && menu::item(ui, p, "copy-filled", aurora_i18n::tr!("ui-nearby-media-copy-data"))
        {
            out.push(Action::Copy(data));
        }
    });
}

/// The selected media's controls (updateControls: showBasicControls,
/// showTimeBasedControls, showDisabledControls).
fn controls(ui: &mut egui::Ui, p: &Palette, row: Option<&Row>, zoomed: Option<MediaKey>, out: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let Some(row) = row else {
            widgets::icon_button(ui, p, "player-stop-filled", aurora_i18n::tr!("ui-nearby-media-stop"), false);
            widgets::icon_button(ui, p, "zoom-in-filled", aurora_i18n::tr!("ui-nearby-media-zoom"), false);
            ui.label(RichText::new(aurora_i18n::tr!("ui-nearby-media-select")).size(11.5).color(p.muted));
            return;
        };
        let playing = row.on && !row.paused;
        if row.time_based && playing {
            if widgets::icon_button(ui, p, "player-pause-filled", aurora_i18n::tr!("ui-media-pause"), true) {
                out.push(Action::Pause(row.key));
            }
        } else if !playing && widgets::icon_button(ui, p, "player-play-filled", aurora_i18n::tr!("ui-media-playback"), true) {
            out.push(Action::Play(row.key));
        }
        if (row.on || row.time_based) && widgets::icon_button(ui, p, "player-stop-filled", aurora_i18n::tr!("ui-nearby-media-stop"), row.on)
        {
            out.push(Action::On(row.key, false));
        }
        let icon = super::audio::speaker_icon(row.volume, row.muted);
        let tip = if row.muted {
            aurora_i18n::tr!("common-unmute")
        } else {
            aurora_i18n::tr!("common-mute")
        };
        if widgets::icon_toggle(ui, p, icon, tip, row.muted) {
            out.push(Action::Mute(row.key, !row.muted));
        }
        ui.spacing_mut().slider_width = 110.0;
        let mut volume = row.volume;
        if widgets::volume_slider(ui, &mut volume, widgets::VolumeScale::Gain, !row.muted, |s| {
            s.custom_parser(aurora_i18n::parse_number).show_value(false).trailing_fill(true)
        })
        .on_hover_text(aurora_i18n::tr!("ui-media-volume-for-this-media"))
        .changed()
        {
            out.push(Action::Volume(row.key, volume));
        }
        if row.zoomable() {
            if zoomed == Some(row.key) {
                if widgets::icon_button(ui, p, "zoom-out-filled", aurora_i18n::tr!("ui-nearby-media-unzoom"), true) {
                    out.push(Action::Unzoom);
                }
            } else if widgets::icon_button(ui, p, "zoom-in-filled", aurora_i18n::tr!("ui-nearby-media-zoom"), true) {
                out.push(Action::Zoom(row.key));
            }
        }
    });
}

//! Media controls of the focused media on a prim (LLPanelPrimMediaControls):
//! back, forward, home, reload / stop, address bar, play / pause for
//! videos, volume, mute and close. Shown under the top bar while a media
//! face has the focus; navigation needs the face's control permission.

use super::icons::Icons;
use crate::media::{MediaKey, MediaManager};
use crate::theme::Palette;
use aurora_media::MediaStatus;
use egui::{RichText, Vec2};

#[derive(Default)]
pub struct MediaUi {
    /// Address being typed (None: shows the current location).
    url_edit: Option<String>,
    shown_for: Option<MediaKey>,
}

fn button(ui: &mut egui::Ui, p: &Palette, icons: &Icons, icon: &str, tip: &str, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(24.0, 22.0),
        if enabled { egui::Sense::click() } else { egui::Sense::hover() },
    );
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
    }
    if let Some(t) = icons.get(icon) {
        let tint = if !enabled {
            p.muted_dim.gamma_multiply(0.6)
        } else if resp.hovered() {
            p.ink
        } else {
            p.muted
        };
        let r = egui::Rect::from_center_size(rect.center(), Vec2::splat(15.0));
        ui.painter().image(
            t.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    enabled && resp.on_hover_text(tip).clicked()
}

fn fmt_time(t: f64) -> String {
    let s = t.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Draw the controls of the focused media (if any) at `top` (logical px).
pub fn show(ctx: &egui::Context, p: &Palette, icons: &Icons, media: &mut MediaManager, st: &mut MediaUi, top: f32) {
    let Some(key) = media.focus else {
        st.url_edit = None;
        st.shown_for = None;
        return;
    };
    if st.shown_for != Some(key) {
        st.url_edit = None;
        st.shown_for = Some(key);
    }
    let Some(m) = media.find_mut(&key) else {
        return;
    };
    // LLPanelPrimMediaControls: navigation needs perms_control
    let can_control = m.entry.as_ref().is_none_or(|e| e.perms_control != 0);
    let mut close = false;
    egui::Area::new(egui::Id::unique("media_controls"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, top + 6.0))
        .show(ctx, |ui| {
            super::widgets::overlay(ui, p, egui::Margin::symmetric(8, 5), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    let time_media = m.is_time_media();
                    if !time_media {
                        if button(
                            ui,
                            p,
                            icons,
                            "arrow-left",
                            aurora_i18n::tr!("common-previous"),
                            m.can_back && can_control,
                        ) {
                            m.back();
                        }
                        if button(
                            ui,
                            p,
                            icons,
                            "arrow-right",
                            aurora_i18n::tr!("common-next"),
                            m.can_forward && can_control,
                        ) {
                            m.forward();
                        }
                        let has_home = m.entry.as_ref().is_some_and(|e| !e.home_url.trim().is_empty());
                        if button(
                            ui,
                            p,
                            icons,
                            "home-filled",
                            aurora_i18n::tr!("ui-media-home-page"),
                            has_home && can_control,
                        ) {
                            m.home();
                        }
                        if m.status == MediaStatus::Loading {
                            if button(ui, p, icons, "x", aurora_i18n::tr!("ui-media-stop-loading"), true) {
                                m.stop_loading();
                            }
                        } else if button(ui, p, icons, "refresh", aurora_i18n::tr!("ui-media-reload"), m.is_loaded()) {
                            m.reload();
                        }
                    } else if m.paused {
                        if button(ui, p, icons, "player-play-filled", aurora_i18n::tr!("ui-media-playback"), true) {
                            m.play();
                        }
                    } else if button(ui, p, icons, "player-pause-filled", aurora_i18n::tr!("ui-media-pause"), true) {
                        m.pause();
                    }
                    // address bar
                    let shown = if m.location.is_empty() { m.url.clone() } else { m.location.clone() };
                    let mut text = st.url_edit.clone().unwrap_or(shown);
                    let edit = egui::TextEdit::singleline(&mut text)
                        .desired_width(360.0)
                        .font(egui::FontId::proportional(12.5))
                        .interactive(can_control);
                    let resp = ui.add(edit);
                    if resp.changed() {
                        st.url_edit = Some(text.clone());
                    }
                    if resp.lost_focus() {
                        if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !text.trim().is_empty() {
                            m.navigate_user(&text);
                        }
                        st.url_edit = None;
                    }
                    if time_media && m.duration > 0.0 {
                        ui.label(
                            RichText::new(format!("{} / {}", fmt_time(m.current_time), fmt_time(m.duration)))
                                .size(11.5)
                                .color(p.muted),
                        );
                    }
                    let status = match m.status {
                        MediaStatus::Loading => aurora_i18n::tr!("common-loading-2"),
                        MediaStatus::Error => aurora_i18n::tr!("ui-media-error"),
                        _ if !m.is_loaded() && m.failed.is_some() => aurora_i18n::tr!("ui-media-unavailable"),
                        _ if !m.is_loaded() => aurora_i18n::tr!("ui-media-pending"),
                        _ => "",
                    };
                    if !status.is_empty() {
                        ui.label(RichText::new(status).size(11.5).color(p.muted));
                    }
                    // volume and mute
                    let icon = if m.muted || m.volume <= 0.001 { "volume-off" } else { "volume" };
                    if button(
                        ui,
                        p,
                        icons,
                        icon,
                        if m.muted {
                            aurora_i18n::tr!("common-unmute")
                        } else {
                            aurora_i18n::tr!("common-mute")
                        },
                        true,
                    ) {
                        m.muted = !m.muted;
                    }
                    ui.spacing_mut().slider_width = 70.0;
                    super::widgets::volume_slider(ui, &mut m.volume, super::widgets::VolumeScale::Gain, true, |s| {
                        s.custom_parser(aurora_i18n::parse_number).trailing_fill(true).show_value(false)
                    })
                    .on_hover_text(aurora_i18n::tr!("ui-media-volume-for-this-media"));
                    if button(ui, p, icons, "x", aurora_i18n::tr!("ui-media-close-esc"), true) {
                        close = true;
                    }
                });
                if let Some(err) = m.failed.as_ref() {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("ui-media-media-player-unavailable", err = err.to_string()))
                            .size(11.0)
                            .color(p.muted),
                    );
                }
            });
        });
    if close {
        media.unfocus();
    }
}

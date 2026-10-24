//! Preferences › Media: web pages and videos on the parcels, the objects,
//! the HUDs and the other avatars (the Legacy Viewer's Sound & Media), the
//! filter of their sites (MediaEnableFilter, FloaterMediaLists) and the
//! media plugins.

use super::super::cx::{Cx, Nothing, field};
use super::super::{SLIDER_W, media_pack_error};
use crate::ui::widgets;
use egui::RichText;

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("ui-options-playback"), |cx| {
        cx.toggle(
            tr!("ui-options-enable-media"),
            tr!("ui-options-parcel-and-prim-web-pages-and-videos-audiostreamingmedia"),
            &field!(media.enabled),
        );
        cx.toggle(
            tr!("ui-options-auto-media"),
            tr!("ui-options-auto-play-parcel-media-after-standing-still-for-5"),
            &field!(audio.media_autoplay),
        );
        cx.toggle(
            tr!("ui-options-prim-media"),
            tr!("ui-options-shared-media-displayed-on-object-faces-primmediamasterenabled"),
            &field!(media.prim_media),
        );
        cx.toggle(
            tr!("ui-options-on-other-avatars"),
            tr!("ui-options-media-on-other-people-s-attachments-mediashowonothers-disabled-by"),
            &field!(media.show_on_others),
        );
        cx.toggle(
            tr!("ui-options-first-click-interaction"),
            tr!("ui-options-the-first-click-goes-directly-to-the-page-when"),
            &field!(media.first_click_interact),
        );
        cx.toggle(
            tr!("ui-options-hud-media"),
            tr!("ui-options-auto-play-hud-media-mediaautoplayhuds"),
            &field!(media.autoplay_huds),
        );
        cx.toggle(
            tr!("ui-options-scripted-media"),
            tr!("ui-options-allow-scripts-to-start-parcel-media-permallowscriptedmedia-disabled-by"),
            &field!(media.allow_scripted),
        );
        cx.row(
            tr!("ui-options-simultaneous-players"),
            tr!("ui-options-maximum-pages-videos-loaded-at-once-plugininstancestotal"),
            &field!(media.max_instances),
            |ui, s| {
                let m = &mut s.media;
                let mut n = m.max_instances as u32;
                let ch = ui
                    .add(
                        egui::Slider::new(&mut n, 1..=16)
                            .custom_parser(aurora_i18n::parse_number)
                            .trailing_fill(true),
                    )
                    .changed();
                m.max_instances = n as usize;
                ch
            },
        );
    });
    cx.group(tr!("ui-options-filter"), |cx| {
        cx.toggle(
            tr!("ui-options-media-filter"),
            tr!("ui-options-media-filter-help"),
            &field!(media.filter),
        );
        let off = !cx.s.media.filter;
        cx.subs(off, |cx| {
            cx.toggle(
                tr!("ui-options-media-filter-own-worn"),
                tr!("ui-options-media-filter-own-worn-help"),
                &field!(media.filter_trust_own_worn),
            );
            cx.below().row_with(
                tr!("ui-options-media-filter-list"),
                tr!("ui-options-media-filter-list-help"),
                &Nothing,
                |ui, _, st, _| {
                    let width = ui.available_width().max(SLIDER_W);
                    crate::ui::media_filter::list(
                        ui,
                        &p,
                        st.media_filter.as_deref(),
                        &mut st.media_filter_typed,
                        &mut st.media_filter_edits,
                        width,
                    );
                    false
                },
            );
        });
        // the note about the filter shown once
        cx.track(&field!(media.filter_explained));
    });
    cx.group(tr!("ui-options-plugins"), |cx| {
        cx.row(
            tr!("ui-options-plugins-folder"),
            tr!("ui-options-folder-containing-llplugin-blank-aurora-s-own-plugins"),
            &field!(media.plugin_dir),
            |ui, s| {
                ui.add(
                    egui::TextEdit::singleline(&mut s.media.plugin_dir)
                        .desired_width(SLIDER_W + 60.0)
                        .hint_text(tr!("ui-options-plugins-folder-automatic")),
                )
                .changed()
            },
        );
        cx.wide().row_with(tr!("ui-options-plugins"), "", &Nothing, |ui, _, st, _| {
            use crate::media::pack::PackState;
            let megabytes = |bytes: u64| aurora_i18n::integer((bytes / (1024 * 1024)) as i64);
            // plugins found: their folder; otherwise where Aurora's pack stands
            let (status, download, retry) = if !st.media_plugins.is_empty() {
                let text = tr!(
                    "status-active-player-s",
                    value0 = st.media_plugins.to_string(),
                    value1 = format!("{}", st.media_running)
                );
                (text, None, false)
            } else {
                match st.media_pack {
                    PackState::Downloading(d) if !d.preparing => {
                        let text = tr!(
                            "media-plugins-downloading",
                            received = megabytes(d.received),
                            total = megabytes(d.total)
                        );
                        (text, Some(d.received as f32 / d.total.max(1) as f32), false)
                    }
                    PackState::Downloading(_) => (tr!("media-plugins-installing").to_string(), Some(1.0), false),
                    PackState::Failed(failure) => (media_pack_error(failure).to_string(), None, true),
                    PackState::Idle | PackState::Ready => (tr!("media-plugins-missing").to_string(), None, false),
                }
            };
            if let Some(fraction) = download {
                widgets::progress(ui, &p, "media_pack", fraction, SLIDER_W + 60.0, &status, None);
                return false;
            }
            if retry
                && widgets::action_button(
                    ui,
                    &p,
                    "refresh",
                    tr!("media-plugins-retry"),
                    widgets::ActionKind::Flat,
                    egui::vec2(0.0, 24.0),
                )
                .clicked()
            {
                st.media_pack_retry = true;
            }
            // the path of a folder, of any length: cut to the row (whole on
            // hover)
            ui.add(widgets::elided_label(ui, RichText::new(status).size(12.0).color(p.muted), 380.0));
            false
        });
    });
}

//! "Media" (LLPanelLandMedia: refresh, populateMIMECombo, onCommitType,
//! onCommitAny; LLFloaterURLEntry for the home page) and the MIME widget
//! sets of mime_types.xml (LLMIMETypes).

use super::{Dialog, LandUi, View, check, dim, text};
use crate::theme::Palette;
use crate::world::World;
use crate::world::land::powers;
use egui::{RichText, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

/// Widget sets in the combo's order (sorted by name, "none" last), with
/// their label, default type, allow_resize and allow_looping.
pub(super) fn widgets() -> [(&'static str, &'static str, &'static str, bool, bool); 5] {
    [
        ("audio", aurora_i18n::tr!("common-sound"), "audio/*", false, true),
        ("image", aurora_i18n::tr!("common-image"), "image/*", false, false),
        ("movie", aurora_i18n::tr!("ui-land-media-movie"), "video/*", false, true),
        ("web", aurora_i18n::tr!("ui-land-media-web-browser"), "text/html", true, false),
        ("none", aurora_i18n::tr!("ui-land-media-no-contents"), "none/none", false, false),
    ]
}

/// LLMIMETypes::widgetType: exact match in mime_types.xml, else "none".
pub(super) fn widget_type(mime: &str) -> &'static str {
    match mime {
        "audio/*" | "application/ogg" | "audio/mid" | "audio/mpeg" | "audio/x-aiff" | "audio/x-wav" => "audio",
        "video/*"
        | "video/vnd.secondlife.qt.legacy"
        | "application/smil"
        | "application/octet-stream"
        | "video/mpeg"
        | "video/mp4"
        | "video/x-flv"
        | "video/quicktime"
        | "video/x-ms-asf"
        | "video/x-ms-wmv"
        | "video/x-msvideo" => "movie",
        "image/*"
        | "application/pdf"
        | "application/postscript"
        | "application/rtf"
        | "application/x-director"
        | "image/bmp"
        | "image/gif"
        | "image/jpeg"
        | "image/png"
        | "image/svg+xml"
        | "image/tiff" => "image",
        "text/html" | "application/javascript" | "application/xhtml+xml" => "web",
        "text/plain" | "text/xml" => "text",
        _ => "none",
    }
}

/// The widget set of a MIME type: (key, label, default type, allow_resize,
/// allow_looping).
pub(super) fn widget(mime: &str) -> Option<(&'static str, &'static str, &'static str, bool, bool)> {
    let w = widget_type(mime);
    widgets().into_iter().find(|x| x.0 == w)
}

pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut LandUi,
    v: &View,
    world: &mut World,
    images: &HashMap<Uuid, egui::TextureHandle>,
) {
    let Some(parcel) = v.parcel.clone() else {
        super::no_selection(ui, p);
        return;
    };
    let can = v.can(powers::LAND_CHANGE_MEDIA);
    let mime = if parcel.media.mime.is_empty() || parcel.media.mime == "none/none" {
        "aucun/aucun".to_owned()
    } else {
        parcel.media.mime.clone()
    };
    let w = widget(&parcel.media.mime);
    let allow_resize = w.is_some_and(|w| w.3);
    let allow_loop = w.is_some_and(|w| w.4);
    super::row(ui, p, aurora_i18n::tr!("common-type-2"), |ui| {
        let current = w.map_or("", |w| w.1);
        ui.add_enabled_ui(can, |ui| {
            egui::ComboBox::from_id_salt("land_media_type")
                .selected_text(current)
                .width(150.0)
                .show_ui(ui, |ui| {
                    for (key, label, default_type, _, _) in widgets() {
                        if ui.selectable_label(w.is_some_and(|w| w.0 == key), label).clicked() && w.is_none_or(|w| w.0 != key) {
                            // onCommitType: the widget's default type
                            world.land.update(|u| u.media_type = default_type.to_owned());
                        }
                    }
                })
                .response
                .on_hover_text(aurora_i18n::tr!("ui-land-media-specify-whether-the-url-points-to-a-movie-web"));
        });
        text(ui, p, mime);
    });
    super::row(ui, p, aurora_i18n::tr!("common-home-page"), |ui| {
        let mut url = parcel.media_url.clone();
        crate::ui::widgets::text_field(ui, false, &mut url, |b, ui| {
            egui::TextEdit::singleline(b).desired_width(ui.available_width() - 80.0)
        });
        if super::button(ui, p, aurora_i18n::tr!("common-choose"), can).clicked() {
            world.land.media_type = None;
            s.dialog = Some(Dialog::MediaUrl(parcel.media_url.clone()));
        }
    });
    super::row(ui, p, aurora_i18n::tr!("common-description"), |ui| {
        s.media_desc.sync(&parcel.media.desc);
        let r = crate::ui::widgets::text_field(ui, can, &mut s.media_desc.text, |b, _| {
            egui::TextEdit::singleline(b).desired_width(f32::INFINITY)
        });
        let r = r.on_hover_text(aurora_i18n::tr!("ui-land-media-text-displayed-beside-the-play-load-button"));
        if let Some(d) = s.media_desc.after(&r) {
            world.land.update(|u| u.media_desc = d);
        }
    });
    ui.horizontal_top(|ui| {
        super::key(ui, p, aurora_i18n::tr!("ui-land-media-replace-texture"));
        let size = Vec2::new(64.0, 80.0);
        let (rect, r) = ui.allocate_exact_size(size, if can { egui::Sense::click() } else { egui::Sense::hover() });
        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
        let id = parcel.media.media_id;
        if !id.is_nil() {
            s.wanted_images.insert(id);
            if let Some(t) = images.get(&id) {
                ui.painter().image(
                    t.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    super::super::widgets::PICTURE_TINT,
                );
            }
        }
        if r.on_hover_text(aurora_i18n::tr!("common-click-to-select-an-image")).clicked() {
            s.texture_snapshot = false;
            s.texture.open(aurora_i18n::tr!("ui-land-media-media-texture"), id);
        }
        ui.add_space(8.0);
        ui.add(
            egui::Label::new(
                RichText::new(aurora_i18n::tr!("ui-land-media-objects-using-this-texture-show-the-movie-or-web"))
                    .size(12.0)
                    .color(p.muted),
            )
            .wrap(),
        );
    });
    ui.horizontal(|ui| {
        ui.add_space(super::KEY_W);
        let (r, on) = check(
            ui,
            p,
            can,
            parcel.media.auto_scale,
            aurora_i18n::tr!("ui-land-media-automatic-scaling"),
        );
        r.clone().on_hover_text(aurora_i18n::tr!(
            "ui-land-media-selecting-this-option-automatically-scales-the-content-of-this"
        ));
        if r.changed() {
            world.land.update(|u| u.media_auto_scale = on);
        }
    });
    super::row(ui, p, aurora_i18n::tr!("common-size"), |ui| {
        // disallowed sizes show 0 (findAllowResize)
        let (mut width, mut height) = if allow_resize {
            (parcel.media.width, parcel.media.height)
        } else {
            (0, 0)
        };
        let tip = aurora_i18n::tr!("ui-land-media-web-media-size-leave-0-for-the-default");
        let rw = ui
            .add_enabled(
                can && allow_resize,
                egui::DragValue::new(&mut width)
                    .custom_parser(aurora_i18n::parse_number)
                    .range(0..=1024),
            )
            .on_hover_text(tip);
        let rh = ui
            .add_enabled(
                can && allow_resize,
                egui::DragValue::new(&mut height)
                    .custom_parser(aurora_i18n::parse_number)
                    .range(0..=1024),
            )
            .on_hover_text(tip);
        dim(ui, p, "pixels");
        let commit = |r: &egui::Response| (r.changed() && !r.dragged()) || r.drag_stopped();
        if commit(&rw) || commit(&rh) {
            world.land.update(|u| {
                u.media_width = width;
                u.media_height = height;
            });
        }
    });
    super::row(ui, p, aurora_i18n::tr!("ui-land-media-options"), |ui| {
        // DEV-10042: no looping for static types
        let (r, on) = check(
            ui,
            p,
            can && allow_loop,
            allow_loop && parcel.media.looping,
            aurora_i18n::tr!("ui-land-media-looping"),
        );
        r.clone()
            .on_hover_text(aurora_i18n::tr!("ui-land-media-loop-media-when-playback-ends-it-starts-again"));
        if r.changed() {
            world.land.update(|u| u.media_loop = on);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_types_like_mime_types_xml() {
        assert_eq!(widget_type("text/html"), "web");
        assert_eq!(widget_type("video/mp4"), "movie");
        assert_eq!(widget_type("video/vnd.secondlife.qt.legacy"), "movie");
        assert_eq!(widget_type("image/png"), "image");
        assert_eq!(widget_type("text/plain"), "text");
        // no wildcard matching: unknown types are "none"
        assert_eq!(widget_type("video/webm"), "none");
        assert!(widget("text/html").is_some_and(|w| w.3 && !w.4));
    }
}

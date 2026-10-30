//! Preferences › Accessibility (Aurora's own page; the Legacy Viewer has
//! no colour blindness option): the high contrast palette
//! (`crate::high_contrast`, #906), the colour vision the interface adapts
//! to (`crate::colorblind`, #418) with a preview of the status dots, the
//! reduced motion, larger controls, always-underlined links, quicker
//! tooltips and a visible keyboard focus ring, plus the settings of other
//! pages that help to see, read and hear, gathered here (`Cx::borrow`:
//! their own rows, so the same settings).

use super::super::cx::{field, Cx, Nothing};
use super::super::Page;
use crate::colorblind::Deficiency;
use crate::ui::presence::{self, Status};
use egui::{RichText, Sense, Vec2};

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("ui-options-vision"), |cx| {
        cx.borrow(Page::Layout, 0, &["ui_scale"]);
        cx.borrow(Page::Appearance, 0, &["font_scale", "font"]);
        cx.toggle(
            tr!("ui-options-high-contrast"),
            tr!("ui-options-high-contrast-help"),
            &field!(high_contrast),
        );
        cx.borrow(Page::Avatars, 0, &["name_tag_distance"]);
        cx.borrow(
            Page::Avatars,
            0,
            &["show_own_name_tag", "show_friend_name_tags", "show_other_name_tags"],
        );
    });
    cx.group(tr!("ui-options-interface-colors"), |cx| {
        if crate::high_contrast::current() {
            cx.note(tr!("ui-options-interface-colors-high-contrast"));
        }
        cx.borrow(
            Page::Appearance,
            2,
            &["interface_colors.accent", "interface_colors.surface", "interface_colors.text"],
        );
    });
    cx.group(tr!("ui-options-color-blindness"), |cx| {
        cx.note(tr!("ui-options-color-blindness-help"));
        let choices: Vec<(Deficiency, &str)> = Deficiency::ALL.iter().map(|d| (*d, d.label())).collect();
        let help = Deficiency::ALL
            .iter()
            .map(|d| format!("{} — {}", d.label(), d.help()))
            .collect::<Vec<_>>()
            .join("\n");
        cx.combo(tr!("ui-options-color-vision"), &help, &field!(color_blindness), &choices);
        let off = cx.s.color_blindness == Deficiency::Off;
        cx.subs(off, |cx| {
            cx.row(
                tr!("ui-options-color-blindness-strength"),
                tr!("ui-options-color-blindness-strength-help"),
                &field!(color_blindness_strength),
                |ui, s| {
                    let mut percent = (s.color_blindness_strength * 100.0).round();
                    let changed = super::super::slider_whole(ui, &mut percent, 0.0..=100.0, "%");
                    if changed {
                        s.color_blindness_strength = (percent / 100.0).clamp(0.0, 1.0);
                    }
                    changed
                },
            );
        });
        cx.wide().row(
            tr!("ui-options-color-blindness-preview"),
            tr!("ui-options-color-blindness-preview-help"),
            &Nothing,
            |ui, _| {
                preview(ui, &p);
                false
            },
        );
    });
    cx.group(tr!("ui-options-controls-a11y"), |cx| {
        cx.toggle(
            tr!("ui-options-larger-controls"),
            tr!("ui-options-larger-controls-help"),
            &field!(larger_controls),
        );
        cx.toggle(
            tr!("ui-options-quick-tooltips"),
            tr!("ui-options-quick-tooltips-help"),
            &field!(quick_tooltips),
        );
        cx.toggle(
            tr!("ui-options-show-keyboard-focus"),
            tr!("ui-options-show-keyboard-focus-help"),
            &field!(show_keyboard_focus),
        );
    });
    cx.group(tr!("ui-options-reading"), |cx| {
        cx.toggle(
            tr!("ui-options-underline-links"),
            tr!("ui-options-underline-links-help"),
            &field!(underline_links),
        );
        cx.borrow(Page::Chat, 0, &["chat_toast_seconds"]);
        cx.borrow(Page::Chat, 1, &["im_toast_seconds"]);
        // world tooltips name what is under the pointer — useful when focus
        // is hard to track; same rows as World › World tooltips
        cx.borrow(Page::Layout, 1, &["hover_tips.enabled", "hover_tips.delay"]);
        cx.borrow(Page::Inventory, 0, &["inventory.thumbnail_tooltips"]);
    });
    cx.group(tr!("ui-options-motion"), |cx| {
        cx.toggle(
            tr!("ui-options-reduce-motion"),
            tr!("ui-options-reduce-motion-help"),
            &field!(reduce_motion),
        );
        cx.borrow(Page::Chat, 0, &["flash_unread_conversations"]);
        cx.borrow(Page::Chat, 0, &["flash_conversations_button", "flash_conversations_nearby"]);
        cx.borrow(Page::Chat, 1, &["flash_on_message", "flash_on_object_im", "flash_on_script_dialog"]);
        cx.borrow(Page::Graphics, 4, &["dof.enabled"]);
    });
    cx.group(tr!("ui-options-hearing"), |cx| {
        cx.borrow(Page::Sound, 0, &["audio.perceptual_volume"]);
        cx.borrow(Page::Sound, 1, &["audio.ui_sounds"]);
        cx.borrow(Page::Sound, 2, &["audio.voice_visualizer"]);
        cx.borrow(Page::Chat, 0, &["link_previews.subtitles"]);
    });
}

/// The four status dots as the lists draw them, each with its name.
fn preview(ui: &mut egui::Ui, p: &crate::theme::Palette) {
    ui.spacing_mut().item_spacing.x = 4.0;
    for status in [Status::Online, Status::Away, Status::Busy, Status::Offline] {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(14.0, 16.0), Sense::hover());
        presence::paint_mark(ui.painter(), rect.center(), 5.0, status.shape(), status.color(p), p.panel);
        ui.label(RichText::new(status.label()).size(12.0).color(p.ink));
        ui.add_space(8.0);
    }
}

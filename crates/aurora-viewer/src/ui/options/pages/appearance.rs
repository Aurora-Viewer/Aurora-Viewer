//! Preferences › Appearance: the interface's font and the size of its text
//! (bundled fonts or a font of the system), the colours of the chat and
//! of the mini-map (the Legacy Viewer's Colors panel), the interface's own
//! colours (`crate::interface_colors`) and where the skin's files are.

use super::super::cx::{Cx, F, Nothing, field, fields};
use super::super::{COMBO_W, swatch, toggle};
use crate::ui::fonts;
use egui::RichText;

pub fn page(cx: &mut Cx, tab: usize) {
    match tab {
        0 => font(cx),
        1 => colors(cx),
        _ => {
            use aurora_i18n::tr;
            let p = cx.p;
            interface_colors(cx);
            cx.group(tr!("extra-skin"), |cx| {
                cx.link(
                    tr!("ui-options-high-contrast"),
                    tr!("ui-options-high-contrast-skin"),
                    super::super::Page::Accessibility.name(),
                    super::super::Page::Accessibility,
                    0,
                );
                cx.note(tr!("ui-options-customize-colors-icons-and-the-toolbar-in"));
                cx.below().row(tr!("ui-options-skin-folder"), "", &Nothing, |ui, _| {
                    let dir = crate::settings::config_dir().join("skins").join("aurora");
                    ui.vertical(|ui| {
                        ui.label(RichText::new(dir.display().to_string()).size(12.0).monospace().color(p.teal));
                        ui.label(
                            RichText::new(tr!("ui-options-theme-json-layout-json-icons-reloaded-automatically"))
                                .size(11.0)
                                .color(p.muted),
                        );
                    });
                    false
                });
            });
        }
    }
}

/// The accent, background and text of the interface, the rest following
/// from them; greyed while high contrast, which has its own colours.
fn interface_colors(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    let skin = crate::interface_colors::skin().unwrap_or(p);
    let shown = crate::interface_colors::current();
    cx.group(tr!("ui-options-interface-colors"), |cx| {
        let high_contrast = crate::high_contrast::current();
        cx.note(if high_contrast {
            tr!("ui-options-interface-colors-high-contrast")
        } else {
            tr!("ui-options-interface-colors-help")
        });
        cx.greyed(high_contrast);
        base_color(
            cx,
            tr!("ui-options-accent-color"),
            tr!("ui-options-accent-color-help"),
            &field!(interface_colors.accent),
            shown.accent,
            skin.violet,
        );
        base_color(
            cx,
            tr!("ui-options-surface-color"),
            tr!("ui-options-surface-color-help"),
            &field!(interface_colors.surface),
            shown.surface,
            skin.panel,
        );
        base_color(
            cx,
            tr!("ui-options-text-color"),
            tr!("ui-options-text-color-help"),
            &field!(interface_colors.text),
            shown.text,
            skin.ink,
        );
        cx.row(tr!("ui-options-all-interface-colors"), "", &Nothing, |ui, s| {
            let reset = crate::ui::widgets::action_button_if(
                ui,
                &p,
                "restore",
                tr!("ui-options-reset-all"),
                crate::ui::widgets::ActionKind::Flat,
                egui::vec2(0.0, 26.0),
                !s.interface_colors.is_default(),
            )
            .clicked();
            if reset {
                s.interface_colors = Default::default();
            }
            reset
        });
        if !high_contrast {
            let adjusted = crate::interface_colors::adjusted(&cx.s.interface_colors);
            if adjusted.text {
                cx.note(tr!("ui-options-interface-colors-text-adjusted"));
            }
            if adjusted.accent {
                cx.note(tr!("ui-options-interface-colors-accent-adjusted"));
            }
        }
    });
}

/// One base colour: its swatch shows the colour in use (a demo run's
/// included), the skin's until one is chosen (the row's modified dot puts
/// the skin's back).
fn base_color(
    cx: &mut Cx,
    label: &str,
    hint: &str,
    f: &F<Option<crate::settings::Rgba>>,
    shown: Option<crate::settings::Rgba>,
    skin: egui::Color32,
) {
    let (p, get) = (cx.p, f.get_mut);
    cx.row(label, hint, f, |ui, s| {
        let slot = get(s);
        let mut color = slot.or(shown).unwrap_or([skin.r(), skin.g(), skin.b(), 255]);
        ui.spacing_mut().interact_size = egui::Vec2::new(38.0, 18.0);
        let changed = crate::ui::widgets::color_button(ui, &p, ui.scope_id().with("swatch"), &mut color, false).changed();
        if changed {
            *slot = Some(color);
        }
        changed
    });
}

fn font(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("ui-options-font"), |cx| {
        // the bundled fonts side by side, across both columns (four names,
        // and OpenDyslexic draws wide); a font of the system below
        let builtins: Vec<&'static str> = fonts::BUILTINS.iter().map(|b| b.name).collect();
        cx.wide()
            .keywords(&builtins.join(" "))
            .row_with(tr!("ui-options-bundled-fonts"), "", &field!(font), |ui, s, _, r| {
                let options: Vec<(String, bool)> = builtins
                    .iter()
                    .map(|b| {
                        let label = if *b == fonts::DEFAULT {
                            tr!("ui-options-default", builtin = b.to_string())
                        } else {
                            (*b).to_owned()
                        };
                        (label, true)
                    })
                    .collect();
                let refs: Vec<(&str, bool)> = options.iter().map(|(l, a)| (l.as_str(), *a)).collect();
                let current = builtins
                    .iter()
                    .position(|b| s.font == *b || (s.font.is_empty() && *b == fonts::DEFAULT))
                    .unwrap_or(usize::MAX);
                let mut i = current;
                if crate::ui::widgets::segmented(ui, &p, &mut i, &refs)
                    && let Some(b) = builtins.get(i)
                {
                    s.font = (*b).to_owned();
                    r.font_changed = true;
                    return true;
                }
                false
            });
        cx.row_with(
            tr!("ui-options-system-fonts"),
            tr!("ui-options-system-font-help"),
            &Nothing,
            |ui, s, st, r| {
                let list = st.system_fonts.get_or_insert_with(fonts::system_fonts);
                let shown = list
                    .iter()
                    .find(|f| s.font == format!("system:{}", f.path.display()))
                    .map_or_else(|| tr!("ui-options-no-system-font").to_owned(), |f| f.name.clone());
                let mut changed = false;
                egui::ComboBox::from_id_salt(ui.scope_id().with("system-font"))
                    .width(COMBO_W)
                    .selected_text(shown)
                    .truncate()
                    .height(320.0)
                    .show_ui(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut st.font_filter)
                                .hint_text(tr!("ui-options-search-fonts"))
                                .desired_width(ui.available_width()),
                        );
                        let filter = st.font_filter.to_lowercase();
                        for f in list.iter().filter(|f| filter.is_empty() || f.name.to_lowercase().contains(&filter)) {
                            let key = format!("system:{}", f.path.display());
                            let sel = s.font == key;
                            if ui.selectable_label(sel, &f.name).clicked() && !sel {
                                s.font = key;
                                r.font_changed = true;
                                changed = true;
                            }
                        }
                    });
                changed
            },
        );
        cx.row_with(tr!("ui-options-text-size"), "", &field!(font_scale), |ui, s, st, _| {
            st.text_scale.show_text(ui, &mut s.font_scale)
        });
        cx.below().row(tr!("ui-options-preview"), "", &Nothing, |ui, _| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(tr!("ui-options-the-violet-wolf-howls-under-the-northern-lights"))
                        .size(15.0)
                        .color(p.ink),
                );
                ui.label(RichText::new(tr!("font-sample")).size(12.0).color(p.muted));
            });
            false
        });
    });
}

fn colors(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("ui-options-messages"), |cx| {
        cx.color(tr!("ui-options-my-messages"), "", &field!(colors.chat_mine));
        cx.color(tr!("common-others"), "", &field!(colors.chat_others));
        cx.color(tr!("common-objects"), "", &field!(colors.chat_objects));
        cx.color(tr!("common-friends"), "", &field!(colors.chat_friends));
        cx.color("Linden Lab", "", &field!(colors.chat_linden));
        cx.color(tr!("ui-options-ignored"), "", &field!(colors.chat_muted));
        cx.color(tr!("common-system"), "", &field!(colors.chat_system));
        cx.color(tr!("ui-options-errors"), "", &field!(colors.chat_errors));
        cx.color(tr!("ui-options-object-ims"), "", &field!(colors.chat_object_im));
        cx.color(tr!("common-owner"), "", &field!(colors.chat_owner));
        cx.color(tr!("status-urls"), "", &field!(colors.chat_urls));
        cx.color(tr!("ui-options-url-path"), "", &field!(colors.chat_slurl));
        cx.color(tr!("ui-options-direct"), "", &field!(colors.chat_direct));
    });
    cx.group(tr!("ui-options-mini-map-markers"), |cx| {
        cx.color(tr!("common-me"), "", &field!(colors.map_me));
        cx.color(tr!("common-others"), "", &field!(colors.map_other));
        cx.color(tr!("common-friends"), "", &field!(colors.map_friend));
        cx.color("Linden Lab", "", &field!(colors.map_linden));
        cx.color(tr!("ui-options-ignored"), "", &field!(colors.map_muted));
        cx.toggle(
            tr!("ui-options-show-whisper-normal-chat-and-shout-ranges-on-the"),
            "",
            &field!(colors.map_ranges),
        );
        let off = !cx.s.colors.map_ranges;
        cx.subs(off, |cx| {
            cx.row(
                tr!("ui-options-whisper"),
                "",
                &fields!(colors.map_whisper_on, colors.map_whisper),
                |ui, s| toggle(ui, &p, &mut s.colors.map_whisper_on) | swatch(ui, &p, "map_range_whisper", &mut s.colors.map_whisper),
            );
            cx.row(
                tr!("ui-options-normal-voice"),
                "",
                &fields!(colors.map_say_on, colors.map_say),
                |ui, s| toggle(ui, &p, &mut s.colors.map_say_on) | swatch(ui, &p, "map_range_say", &mut s.colors.map_say),
            );
            cx.row(
                tr!("ui-options-shout"),
                "",
                &fields!(colors.map_shout_on, colors.map_shout),
                |ui, s| toggle(ui, &p, &mut s.colors.map_shout_on) | swatch(ui, &p, "map_range_shout", &mut s.colors.map_shout),
            );
        });
    });
    cx.group(tr!("ui-options-emitter"), |cx| {
        cx.color(tr!("ui-options-avatars"), "", &field!(colors.sender_avatar));
        cx.color(tr!("common-objects"), "", &field!(colors.sender_object));
    });
    cx.group(tr!("ui-options-chat-mentions"), |cx| {
        cx.color(tr!("ui-options-font"), "", &field!(colors.mention_text));
        cx.color(tr!("ui-options-residents"), "", &field!(colors.mention_residents));
        cx.color(tr!("common-me"), "", &field!(colors.mention_me));
    });
    // the colours of the names are on Avatars › Name tags; every colour
    // back to its default, as the button of the former Colors page
    cx.group(tr!("ui-options-all-colors"), |cx| {
        cx.row(tr!("ui-options-all-colors"), "", &Nothing, |ui, s| {
            let reset = crate::ui::widgets::action_button(
                ui,
                &p,
                "restore",
                tr!("ui-options-default-colors"),
                crate::ui::widgets::ActionKind::Flat,
                egui::vec2(0.0, 26.0),
            )
            .clicked();
            if reset {
                s.colors = crate::settings::ColorSettings::default();
            }
            reset
        });
    });
}

//! Preferences › Appearance: the interface's font and the size of its text
//! (bundled fonts or a font of the system), the colours of the chat and
//! of the mini-map (the Legacy Viewer's Colors panel), the interface's own
//! colours (`crate::interface_colors`) and where the skin's files are.

use super::super::cx::{Cx, F, Nothing, field, fields};
use super::super::{COMBO_W, swatch, toggle};

use crate::ui::{fonts, widgets};
use egui::RichText;

/// Size of the system font list's rows, under its search field (about
/// the width the list took when it sized itself to the longest name).
const FONT_LIST_W: f32 = 360.0;
const FONT_LIST_H: f32 = 280.0;

pub fn page(cx: &mut Cx, tab: usize) {
    match tab {
        0 => font(cx),
        1 => colors(cx),
        _ => {
            use aurora_i18n::tr;
            let p = cx.p;
            interface_colors(cx);
            opacity(cx);
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

/// How much of the world shows through (`ui::surface_opacity`); the Legacy
/// Viewer's Colors › Floater Opacity, and the bars. Greyed in high contrast,
/// which keeps every surface opaque.
fn opacity(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-opacity"), |cx| {
        cx.note(tr!("ui-options-opacity-help"));
        cx.greyed(crate::high_contrast::current());
        cx.slider(
            tr!("ui-options-opacity-active"),
            tr!("ui-options-opacity-active-hint"),
            &field!(window_opacity_active),
            0.0..=1.0,
            "",
        );
        cx.slider(
            tr!("ui-options-opacity-inactive"),
            tr!("ui-options-opacity-inactive-hint"),
            &field!(window_opacity_inactive),
            0.0..=1.0,
            "",
        );
        cx.slider(
            tr!("ui-options-opacity-conversations"),
            tr!("ui-options-opacity-conversations-hint"),
            &field!(conversations_opacity),
            0.0..=1.0,
            "",
        );
        cx.toggle(
            tr!("ui-options-opacity-conversations-override"),
            "",
            &field!(conversations_active_opacity_override),
        );
        cx.slider(
            tr!("ui-options-opacity-bars"),
            tr!("ui-options-opacity-bars-hint"),
            &field!(bar_opacity),
            0.0..=1.0,
            "",
        );
        cx.slider(
            tr!("ui-options-opacity-console"),
            tr!("ui-options-opacity-console-hint"),
            &field!(console_opacity),
            0.0..=1.0,
            "",
        );
        cx.slider(
            tr!("ui-options-opacity-minimap"),
            tr!("ui-options-opacity-minimap-hint"),
            &field!(minimap_opacity),
            0.0..=1.0,
            "",
        );
        // the menus and the pie menu side by side: both are right-click menus
        cx.slider(
            tr!("ui-options-opacity-menus"),
            tr!("ui-options-opacity-menus-hint"),
            &field!(menu_opacity),
            0.0..=1.0,
            "",
        );
        cx.toggle(tr!("ui-options-opacity-pie-override"), "", &field!(pie_opacity_override));
        let off = !cx.s.pie_opacity_override;
        cx.subs(off, |cx| {
            cx.slider(tr!("ui-options-opacity-pie"), "", &field!(pie_opacity), 0.0..=1.0, "");
        });
        // ChatBubbleOpacity: one setting for the bubbles and the tags
        cx.slider(
            tr!("ui-options-chat-bubble-opacity"),
            "",
            &field!(chat_bubble_opacity),
            0.0..=1.0,
            "",
        );
        cx.greyed(false);
    });
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
                let salt = ui.scope_id().with("system-font");
                let popup = system_font_popup(ui, salt);
                let opening = !egui::Popup::is_id_open(ui.ctx(), popup);
                if st.demo_font_list.is_some() {
                    #[cfg(test)]
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(egui::Id::unique("font-list-popup-test-id"), popup));
                    egui::Popup::open_id(ui.ctx(), popup);
                }
                let_font_list_grow(ui.ctx(), popup, opening);
                system_font_combo(salt, shown).show_ui(ui, |ui| {
                    // one width for the whole popup: egui lays it out in its
                    // last frame's size, so content a little wider than that
                    // made it grow every frame (#2510, as #718)
                    ui.set_width(FONT_LIST_W + crate::ui::widgets::scroll_strip(ui));
                    if opening {
                        // each opening starts on the whole list
                        st.font_filter.clear();
                    }
                    system_font_search(ui, &p, &mut st.font_filter, opening);
                    let filter = st.font_filter.to_lowercase();
                    let shown: Vec<&fonts::SystemFont> = list
                        .iter()
                        .filter(|f| filter.is_empty() || f.name.to_lowercase().contains(&filter))
                        .collect();
                    // only the rows on screen are laid out, at one height,
                    // each name drawn in its own face by a thread as it
                    // scrolls into view (`fonts::preview`)
                    // no shorter than a button, or rows would drift from
                    // where `show_rows` puts them
                    let row_h = (ui.text_style_height(&egui::TextStyle::Body) * 1.5).max(ui.spacing().interact_size.y);
                    // one width: a wide face truncates, never widens the list
                    let width = FONT_LIST_W;
                    let pad = ui.spacing().button_padding.x;
                    // the size a row's button gives its text
                    let size = egui::TextStyle::Button.resolve(ui.style()).size * fonts::scale(ui.ctx());
                    let mut previews = fonts::preview::Previews::begin(ui.ctx(), size, width - 2.0 * pad);
                    let selected_row = shown.iter().position(|f| s.font == format!("system:{}", f.path.display()));
                    let list_area = widgets::search_scroll_to_row(ui, egui::ScrollArea::vertical(), selected_row, row_h)
                        .auto_shrink([false, true])
                        .max_height(FONT_LIST_H)
                        .show_rows(ui, row_h, shown.len(), |ui, rows| {
                            for i in rows {
                                let f = shown[i];
                                let key = format!("system:{}", f.path.display());
                                let sel = s.font == key;
                                let drawn = previews.drawn(f);
                                let mut name = RichText::new(&f.name);
                                if drawn.is_some() {
                                    // laid out for the row's size and its
                                    // screen reader label, its picture
                                    // painted in its place
                                    name = name.color(egui::Color32::TRANSPARENT);
                                }
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(width, row_h), egui::Sense::hover());
                                let layout = egui::Layout::top_down_justified(egui::Align::Min);
                                // a child leaves the cursor where the row's own
                                // allocation put it, clipped to the row whatever
                                // the face's height
                                let mut row_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(layout));
                                row_ui.set_clip_rect(rect.intersect(ui.clip_rect()));
                                let row = row_ui.add(egui::Button::selectable(sel, name).truncate().min_size(rect.size()));
                                if let Some(drawn) = drawn {
                                    // the colour the row's text would take
                                    // (hovered, selected), on whole pixels
                                    let color = row_ui.style().interact_selectable(&row, sel).text_color();
                                    let ppp = ui.ctx().pixels_per_point();
                                    let top = rect.center().y - drawn.size.y / 2.0 + drawn.y_offset;
                                    let min = egui::pos2(((rect.left() + pad) * ppp).round(), (top * ppp).round()) / ppp;
                                    row_ui.painter().image(
                                        drawn.texture(),
                                        egui::Rect::from_min_size(min, drawn.size),
                                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                        color,
                                    );
                                }
                                previews.visible(i);
                                if row.clicked() {
                                    if !sel {
                                        s.font = key;
                                        r.font_changed = true;
                                        changed = true;
                                    }
                                    ui.close();
                                }
                            }
                            if st.demo_font_list == Some(true) {
                                ui.scroll_with_delta(egui::vec2(0.0, -6.0));
                            }
                        });
                    // rows cut off by the popup's last size: room next frame
                    if list_area.inner_rect.height() + 0.5 < list_area.content_size.y.min(FONT_LIST_H) {
                        ui.ctx().data_mut(|d| d.insert_temp(popup.with("grow"), true));
                        ui.ctx().request_repaint();
                    }
                    previews.finish(ui.ctx(), &shown);
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

/// The System fonts list's popup, which the demo keeps open.
fn system_font_popup(ui: &egui::Ui, salt: egui::Id) -> egui::Id {
    ui.make_persistent_id(egui::IdSalt::new(salt)).with("popup")
}

/// Room for the System fonts list to grow back as its search is erased.
/// egui lays a popup out in its last frame's size, so a list the filter
/// had shrunk kept that size (#2136). On a frame after rows were cut off,
/// the popup gets back the height of the whole list, measured as it opens,
/// and fits its rows again once drawn. Only then: the room counts as the
/// popup's for clicks, and above its button it would set the rows apart.
fn let_font_list_grow(ctx: &egui::Context, popup: egui::Id, opening: bool) {
    let whole = popup.with("whole-height");
    let grow = popup.with("grow");
    if opening {
        ctx.data_mut(|d| {
            d.remove::<f32>(whole);
            d.remove::<bool>(grow);
        });
        return;
    }
    let Some(mut area) = egui::AreaState::load(ctx, popup) else {
        return;
    };
    let Some(size) = area.size.as_mut() else {
        return;
    };
    let (height, cut) = ctx.data_mut(|d| {
        let height = d.get_temp_mut_or_default::<f32>(whole);
        *height = height.max(size.y);
        (*height, d.remove_temp::<bool>(grow).unwrap_or(false))
    });
    if cut && size.y < height {
        size.y = height;
        ctx.memory_mut(|m| m.areas_mut().set_state(egui::LayerId::new(egui::Order::Foreground, popup), area));
    }
}

/// The System fonts list. A click in its search field must not close it
/// (#2136): only a click outside does, or a picked font (`ui.close()`).
fn system_font_combo(salt: egui::Id, shown: String) -> egui::ComboBox<'static> {
    egui::ComboBox::from_id_salt(salt)
        .width(COMBO_W)
        .selected_text(shown)
        .truncate()
        .height(FONT_LIST_H + 40.0)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
}

/// The list's search field, focused as the list opens so typing filters
/// at once.
fn system_font_search(ui: &mut egui::Ui, p: &crate::theme::Palette, filter: &mut String, opening: bool) -> egui::Response {
    // the list's own width, never what the popup offers (#2510)
    let search = crate::ui::widgets::search_field(ui, p, filter, aurora_i18n::tr!("ui-options-search-fonts"), FONT_LIST_W);
    if opening {
        search.request_focus();
    }
    search
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One frame of the System fonts list, open, with a search field and a
    /// font row; returns their rectangles and whether the list is open.
    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, filter: &mut String) -> (egui::Rect, egui::Rect, bool) {
        let palette = crate::theme::Theme::default().palette();
        let mut rects = (egui::Rect::NOTHING, egui::Rect::NOTHING);
        let mut popup = egui::Id::NULL;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let salt = ui.scope_id().with("system-font");
            popup = system_font_popup(ui, salt);
            system_font_combo(salt, "Font".into()).show_ui(ui, |ui| {
                rects.0 = system_font_search(ui, &palette, filter, false).rect;
                let row = ui.button("Some font");
                rects.1 = row.rect;
                if row.clicked() {
                    ui.close();
                }
            });
        });
        output.textures_delta.clear();
        (rects.0, rects.1, egui::Popup::is_id_open(ctx, popup))
    }

    fn click(at: egui::Pos2) -> Vec<Vec<egui::Event>> {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        vec![vec![egui::Event::PointerMoved(at)], vec![button(true)], vec![button(false)], vec![]]
    }

    /// #2136: a click in the search field keeps the list open; a pick closes it.
    #[test]
    fn search_click_keeps_the_font_list_open() {
        let ctx = egui::Context::default();
        let mut filter = String::new();
        let (_, _, open) = frame(&ctx, vec![], &mut filter);
        assert!(!open);
        // open it as the combo's own button would
        let mut popup = egui::Id::NULL;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            popup = system_font_popup(ui, ui.scope_id().with("system-font"));
        });
        output.textures_delta.clear();
        egui::Popup::open_id(&ctx, popup);
        let (search, row, open) = frame(&ctx, vec![], &mut filter);
        assert!(open, "the list opens");
        for events in click(search.center()) {
            let (_, _, open) = frame(&ctx, events, &mut filter);
            assert!(open, "a click in the search field keeps the list open");
        }
        for events in click(row.center()) {
            frame(&ctx, events, &mut filter);
        }
        assert!(!frame(&ctx, vec![], &mut filter).2, "picking a font closes the list");
    }
}

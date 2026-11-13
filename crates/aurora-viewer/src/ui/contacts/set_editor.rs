//! The editor of a contact set (FSFloaterContactSetConfiguration,
//! fsfloatercontactsetconfiguration.cpp, floater_fs_contact_set_
//! configuration.xml, originally LGPL 2.1), in Aurora's look: a live
//! preview of the set's look on the left (its plate, a chat line, a list
//! row, and its marker from afar while it has one); on the right
//! the set's tile, its name edited in place and its counts, then four tabs:
//! Look (colour, where it shows, badge, marker, plate size), Behaviour
//! (sorting, notices, a chat section of its own, priority), Auto-replies
//! (set_autoresponse_*) and Members. Every change applies at once, as in
//! the Legacy Viewer's floater.
//!
//! Differences from the Legacy Viewer: the options shared by every set
//! (the default colour, the online notices as toasts or in chat, the
//! badges stacked) moved to their own window, opened from People's gear or
//! from the Behaviour tab; the Legacy Viewer shows the default colour's
//! swatch in each set's floater. The name is renamed in place (Enter or
//! leaving the field) rather than with a Rename button.

use super::*;
use crate::world::contact_sets::appearance::{Appearance, ICON_LABELS, ICONS, MARKER_FADE_DEFAULT, MARKER_FADE_RANGE};
use crate::world::contact_sets::{ContactSet, MarkerScope, MarkerShape, PlateSize, ResponseMode, SetResponse, Visuals};
use egui::{Pos2, Rect};

/// The window's size when it first opens, and the preview's column.
const SIZE: Vec2 = Vec2::new(980.0, 720.0);
const PREVIEW_MIN: f32 = 240.0;
const PREVIEW_MAX: f32 = 300.0;
/// The set's tile beside its name, and the tab bar.
const TILE: f32 = 52.0;
const TABS_H: f32 = 40.0;
const FOOTER_H: f32 = 52.0;
/// The round shoulders of the preview's figures, far and near (a drawing,
/// not chrome).
const FAR_SHOULDERS: f32 = 8.0;
const NEAR_SHOULDERS: f32 = 22.0;

/// The editor's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum EditorTab {
    #[default]
    Look,
    Behaviour,
    Replies,
    Members,
}

/// What the editor keeps between frames.
#[derive(Default)]
pub(crate) struct EditorUi {
    pub tab: EditorTab,
    /// The badges' grid unfolded.
    pub badges_open: bool,
    /// The badge's or the marker's part scrolled into view once (the
    /// demo's captures).
    pub reveal_badges: bool,
    pub reveal_marker: bool,
}

/// Where AURORA_DEMO_SET_NOTICES takes the editor: its auto-replies, or
/// the options shared by every set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DemoScroll {
    Responses,
    Bottom,
}

/// The presets of a set's colour, in sRGB (the theme's accents and states
/// a set is likely to take).
const PRESETS: [(&str, [u8; 3]); 9] = [
    ("set-editor-colour-pink", [0xF4, 0x72, 0xB6]),
    ("set-editor-colour-teal", [0x5E, 0xEA, 0xD4]),
    ("set-editor-colour-amber", [0xFC, 0xD3, 0x4D]),
    ("set-editor-colour-indigo", [0x81, 0x8C, 0xF8]),
    ("set-editor-colour-green", [0x4A, 0xDE, 0x80]),
    ("set-editor-colour-orange", [0xFB, 0x92, 0x3C]),
    ("set-editor-colour-red", [0xF8, 0x71, 0x71]),
    ("set-editor-colour-violet", [0xA7, 0x8B, 0xFA]),
    ("set-editor-colour-sky", [0x7D, 0xD3, 0xFC]),
];

/// A preset as a set stores its colour (linear, straight alpha).
fn preset_color(rgb: [u8; 3]) -> [f32; 4] {
    egui::Rgba::from(Color32::from_rgb(rgb[0], rgb[1], rgb[2])).to_rgba_unmultiplied()
}

/// Whether a set's colour is this preset (as the screen shows it).
fn is_preset(color: [f32; 4], rgb: [u8; 3]) -> bool {
    let [r, g, b, a] = color;
    let shown = Color32::from(egui::Rgba::from_rgba_unmultiplied(r, g, b, a));
    shown.r() == rgb[0] && shown.g() == rgb[1] && shown.b() == rgb[2]
}

/// The window, when a set is being edited.
pub(super) fn window(ctx: &egui::Context, p: &Palette, world: &mut World, st: &mut ContactsUi, pics: &HashMap<Uuid, egui::TextureHandle>) {
    let Some(set) = st.config.clone() else {
        return;
    };
    let Some(cfg) = world.contact_sets.set(&set).cloned() else {
        st.config = None;
        return;
    };
    match st.demo_scroll.take() {
        Some(DemoScroll::Responses) => st.editor.tab = EditorTab::Replies,
        Some(DemoScroll::Bottom) => st.options_open = true,
        None => {}
    }
    let mut open = true;
    widgets::Floater::new(
        "contact_set_config",
        aurora_i18n::tr!("ui-contacts-configure", set = set.clone()),
        ctx.content_rect().center() - SIZE / 2.0,
        SIZE,
    )
    .flush_body()
    .show(ctx, p, &mut open, |ui| {
        let full = ui.available_rect_before_wrap();
        let side_w = (full.width() * 0.36).clamp(PREVIEW_MIN, PREVIEW_MAX).round();
        let side = Rect::from_min_max(full.min, Pos2::new(full.left() + side_w, full.bottom()));
        let main = Rect::from_min_max(Pos2::new(side.right(), full.top()), full.max);
        ui.painter().rect_filled(side, 0.0, p.bar);
        let mut side_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("set-editor-preview")
                .max_rect(side.shrink(18.0))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        preview(&mut side_ui, p, &cfg);
        let mut main_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("set-editor-main")
                .max_rect(main.shrink2(Vec2::new(24.0, 20.0)))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        body(&mut main_ui, p, world, st, &set, &cfg, pics);
        ui.allocate_rect(full, Sense::hover());
    });
    if !open {
        close(world, st, &set);
    }
}

fn close(world: &mut World, st: &mut ContactsUi, set: &str) {
    save_responses(world, st, set);
    st.response_edit = None;
    st.config = None;
}

/// The right side: the set's tile, name and counts, the tabs, the tab
/// shown, and the footer.
fn body(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &mut World,
    st: &mut ContactsUi,
    set: &str,
    cfg: &ContactSet,
    pics: &HashMap<Uuid, egui::TextureHandle>,
) {
    ui.spacing_mut().item_spacing.y = 0.0;
    identity(ui, p, world, st, set, cfg);
    ui.add_space(16.0);
    tab_bar(ui, p, &mut st.editor.tab, cfg.friends.len());
    ui.add_space(18.0);
    let rows_h = (ui.available_height() - FOOTER_H).max(0.0);
    egui::ScrollArea::vertical()
        .id_salt(("set-editor-tab", st.editor.tab as u8))
        .auto_shrink([false, false])
        .max_height(rows_h)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            match st.editor.tab {
                EditorTab::Look => look(ui, p, world, st, set, cfg),
                EditorTab::Behaviour => behaviour(ui, p, world, st, set, cfg),
                EditorTab::Replies => replies(ui, p, world, st, set, cfg),
                EditorTab::Members => members(ui, p, world, st, set, cfg, pics),
            }
            ui.add_space(8.0);
        });
    footer(ui, p, world, st, set);
}

/// The set's tile (a click goes to its badges), its name edited in place
/// and its counts with its place in the priority order.
fn identity(ui: &mut egui::Ui, p: &Palette, world: &mut World, st: &mut ContactsUi, set: &str, cfg: &ContactSet) {
    let line = Vec2::new(ui.available_width(), TILE);
    ui.allocate_ui_with_layout(line, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let (tile, resp) = ui.allocate_exact_size(Vec2::splat(TILE), Sense::click());
        set_tile(ui, p, tile, cfg);
        if resp.hovered() {
            ui.painter()
                .rect_filled(tile, ui.visuals().widgets.inactive.corner_radius, p.ink.gamma_multiply(0.06));
        }
        if resp.on_hover_text(aurora_i18n::tr!("set-editor-change-badge")).clicked() {
            st.editor.tab = EditorTab::Look;
            st.editor.badges_open = true;
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let field = egui::TextEdit::singleline(&mut st.rename)
                .id(egui::Id::unique("set-editor-name"))
                .frame(egui::Frame::NONE)
                .font(egui::FontId::new(19.0, crate::ui::fonts::semibold(ui.ctx())))
                .text_color(p.ink)
                .desired_width(ui.available_width())
                .margin(egui::Margin::symmetric(8, 4));
            let rect = Rect::from_min_size(ui.cursor().min - Vec2::new(8.0, 0.0), Vec2::new(ui.available_width() + 8.0, 32.0));
            let focused = ui.ctx().memory(|m| m.has_focus(egui::Id::unique("set-editor-name")));
            let hovered = ui.rect_contains_pointer(rect);
            let radius = ui.visuals().widgets.inactive.corner_radius;
            if focused {
                ui.painter().rect_filled(rect, radius, p.field);
            } else if hovered {
                ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.5));
            }
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            let resp = child.add(field).on_hover_text(aurora_i18n::tr!("set-editor-rename-tip"));
            if resp.has_focus() {
                ui.painter()
                    .rect_stroke(rect, radius, egui::Stroke::new(1.5, p.violet), egui::StrokeKind::Inside);
            } else if hovered {
                let pen = Rect::from_center_size(Pos2::new(rect.right() - 14.0, rect.center().y), Vec2::splat(14.0));
                if let Some(t) = crate::ui::icons::global("edit-filled") {
                    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                    ui.painter().image(t.id(), pen, uv, p.muted);
                }
            }
            ui.allocate_rect(rect, Sense::hover());
            if resp.lost_focus() {
                // Escape gives the name back, anything else applies it
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    st.rename = set.to_owned();
                } else {
                    rename(world, st, set);
                }
            }
            let online = cfg
                .friends
                .iter()
                .filter(|id| world.social.friends.iter().any(|f| f.id == **id && f.online))
                .count();
            let mut parts = vec![aurora_i18n::tr!("people-sets-people", count = cfg.friends.len())];
            if online > 0 {
                parts.push(aurora_i18n::tr!("people-sets-online", count = online));
            }
            if let Some((rank, total)) = world.contact_sets.priority_rank(set) {
                parts.push(aurora_i18n::tr!("set-editor-priority-short", rank = rank + 1, total = total));
            }
            ui.add(egui::Label::new(RichText::new(parts.join(" · ")).size(12.0).color(p.muted)).truncate());
        });
    });
}

/// The name typed, applied: renamed when it is new, put back when it
/// cannot be (empty, internal, or another set's).
fn rename(world: &mut World, st: &mut ContactsUi, set: &str) {
    let new = st.rename.trim().to_owned();
    if new == set {
        return;
    }
    if !new.is_empty() && world.contact_sets.rename_set(set, &new) {
        st.config = Some(new.clone());
        if st.set_choice.as_deref() == Some(set) {
            st.set_choice = Some(new.clone());
        }
        if let Some((name, _)) = &mut st.response_edit
            && name == set
        {
            *name = new;
        }
    } else {
        world.system_message(aurora_i18n::tr!(
            "ui-contacts-cannot-rename-contact-set-to-the-new-name-is",
            set = set.to_owned(),
            new = new
        ));
        st.rename = set.to_owned();
    }
}

/// The tabs, side by side in one track: the chosen one raised, with its
/// icon in the accent; Members with its count.
fn tab_bar(ui: &mut egui::Ui, p: &Palette, tab: &mut EditorTab, members: usize) {
    let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), TABS_H), Sense::hover());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    ui.painter().rect_filled(bar, radius, p.field);
    let tabs = [
        (EditorTab::Look, "palette-filled", aurora_i18n::tr!("set-editor-tab-look"), None),
        (
            EditorTab::Behaviour,
            "adjustments-filled",
            aurora_i18n::tr!("set-editor-tab-behaviour"),
            None,
        ),
        (
            EditorTab::Replies,
            "message-circle-filled",
            aurora_i18n::tr!("set-editor-tab-replies"),
            None,
        ),
        (
            EditorTab::Members,
            "users",
            aurora_i18n::tr!("set-editor-tab-members"),
            Some(members),
        ),
    ];
    let w = (bar.width() - 6.0 - 3.0 * 3.0) / 4.0;
    // the raised face slides to the tab chosen, as the theme's segments do
    let chosen = tabs.iter().position(|t| t.0 == *tab).unwrap_or(0) as f32;
    let slide = ui
        .ctx()
        .animate_value_with_time(ui.scope_id().with("set-editor-tab-slide"), chosen, ui.style().animation_time);
    ui.painter().rect_filled(
        Rect::from_min_size(
            Pos2::new(bar.left() + 3.0 + slide * (w + 3.0), bar.top() + 3.0),
            Vec2::new(w, TABS_H - 6.0),
        ),
        radius,
        p.raised,
    );
    for (i, (id, icon, label, count)) in tabs.into_iter().enumerate() {
        let rect = Rect::from_min_size(
            Pos2::new(bar.left() + 3.0 + i as f32 * (w + 3.0), bar.top() + 3.0),
            Vec2::new(w, TABS_H - 6.0),
        );
        let resp = ui.interact(rect, ui.scope_id().with(("set-editor-tab", i)), Sense::click());
        let on = *tab == id;
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, on, label));
        if !on && resp.hovered() {
            ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.45));
        }
        let ink = if on || resp.hovered() { p.ink } else { p.muted };
        let font = if on {
            crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), 12.5, ink).font_id
        } else {
            egui::FontId::proportional(12.5)
        };
        let g = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
        let badge = count.map(|n| {
            let g = ui
                .painter()
                .layout_no_wrap(aurora_i18n::integer(n as i64), egui::FontId::proportional(10.5), ink);
            let w = (g.size().x + 10.0).max(18.0);
            (g, w)
        });
        let content = 16.0 + 7.0 + g.size().x + badge.as_ref().map_or(0.0, |(_, w)| w + 7.0);
        let mut x = rect.center().x - content / 2.0;
        if let Some(t) = crate::ui::icons::global(icon) {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            let tint = if on { p.violet_light } else { ink };
            ui.painter().image(
                t.id(),
                Rect::from_center_size(Pos2::new(x + 8.0, rect.center().y), Vec2::splat(16.0)),
                uv,
                tint,
            );
        }
        x += 16.0 + 7.0;
        let gw = g.size().x;
        widgets::paint_row_text(ui.painter(), g, x, rect.center().y, ink);
        x += gw + 7.0;
        if let Some((g, w)) = badge {
            let pill = Rect::from_min_size(Pos2::new(x, rect.center().y - 8.0), Vec2::new(w, 16.0));
            let fill = if on { p.violet.gamma_multiply(0.35) } else { p.raised };
            ui.painter().rect_filled(pill, radius, fill);
            let gw = g.size().x;
            widgets::paint_row_text(ui.painter(), g, pill.center().x - gw / 2.0, pill.center().y, ink);
        }
        if resp.clicked() {
            *tab = id;
        }
    }
}

/// A small capitals heading over a part of a tab.
fn section(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add(egui::Label::new(
        RichText::new(text.to_uppercase())
            .size(10.5)
            .color(p.muted)
            .family(crate::ui::fonts::semibold(ui.ctx())),
    ));
    ui.add_space(12.0);
}

/// The flags of "Show the set's colour on", by place: (label, hint).
const SHOWN_ON: [(&str, &str); 7] = [
    ("set-editor-on-plate-name", "set-editor-on-plate-name-hint"),
    ("set-editor-on-border", "set-editor-on-border-hint"),
    ("set-editor-on-background", "set-editor-on-background-hint"),
    ("set-editor-on-ui-name", "set-editor-on-ui-name-hint"),
    ("set-editor-on-chat-name", "set-editor-on-chat-name-hint"),
    ("set-editor-on-chat-text", "set-editor-on-chat-text-hint"),
    ("set-editor-on-minimap", "set-editor-on-minimap-hint"),
];

/// The flag of `SHOWN_ON[i]`.
fn shown_on(v: &mut Visuals, i: usize) -> &mut bool {
    match i {
        0 => &mut v.nameplate_name,
        1 => &mut v.border,
        2 => &mut v.background,
        3 => &mut v.ui_name,
        4 => &mut v.chat_name,
        5 => &mut v.chat_text,
        _ => &mut v.minimap,
    }
}

/// Look: colour, where it shows, badge, marker, plate size.
fn look(ui: &mut egui::Ui, p: &Palette, world: &mut World, st: &mut ContactsUi, set: &str, cfg: &ContactSet) {
    let mut v = cfg.visuals.clone();
    // colour
    section(ui, p, aurora_i18n::tr!("set-editor-colour"));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(12.0, 10.0);
        ui.add_space(4.0);
        for (key, rgb) in PRESETS {
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
            let on = is_preset(cfg.color, rgb);
            let fill = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            ui.painter().circle_filled(rect.center(), 12.0, fill);
            if on {
                ui.painter().circle_stroke(rect.center(), 15.0, egui::Stroke::new(2.0, p.ink));
            } else if resp.hovered() {
                ui.painter().circle_stroke(rect.center(), 14.0, egui::Stroke::new(1.0, p.muted));
            }
            if resp.on_hover_text(aurora_i18n::text(key)).clicked() {
                world.contact_sets.set_color(set, preset_color(rgb));
            }
        }
        custom_colour(ui, p, world, set, cfg);
    });
    ui.add_space(28.0);
    // where it shows
    section(ui, p, aurora_i18n::tr!("set-editor-shown-on"));
    let gap = 8.0;
    let w = ((ui.available_width() - gap) / 2.0).floor();
    for pair in (0..SHOWN_ON.len()).collect::<Vec<_>>().chunks(2) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for &i in pair {
                let (label, hint) = SHOWN_ON[i];
                let flag = shown_on(&mut v, i);
                flag_tile(ui, p, w, aurora_i18n::text(label), aurora_i18n::text(hint), flag);
            }
        });
        ui.add_space(gap);
    }
    ui.add_space(20.0);
    // badge
    section(ui, p, aurora_i18n::tr!("set-editor-badge"));
    badge(ui, p, st, &mut v, cfg);
    ui.add_space(28.0);
    // marker
    section(ui, p, aurora_i18n::tr!("contact-visual-marker"));
    if std::mem::take(&mut st.editor.reveal_marker) {
        ui.scroll_to_cursor(Some(egui::Align::TOP));
    }
    labelled(ui, p, aurora_i18n::tr!("set-editor-marker-show"), |ui| marker_scope(ui, p, &mut v));
    if v.marker != MarkerScope::Off {
        ui.add_space(12.0);
        let color = crate::interface_colors::on_panel(crate::ui::colors::set_color(cfg.color));
        labelled(ui, p, aurora_i18n::tr!("set-editor-marker-shape"), |ui| {
            marker_shapes(ui, p, &mut v, color)
        });
        ui.add_space(12.0);
        labelled(ui, p, aurora_i18n::tr!("set-editor-marker-fade"), |ui| {
            let mut fades = v.marker_fade.is_some();
            if widgets::switch(ui, p, &mut fades)
                .on_hover_text(aurora_i18n::tr!("contact-visual-marker-fade"))
                .changed()
            {
                v.marker_fade = fades.then_some(MARKER_FADE_DEFAULT);
            }
            if let Some(fade) = &mut v.marker_fade {
                ui.add_space(8.0);
                ui.add(egui::Slider::new(fade, MARKER_FADE_RANGE).suffix(" m"));
            }
        });
        ui.add_space(6.0);
        ui.add(
            egui::Label::new(
                RichText::new(aurora_i18n::tr!("contact-visual-marker-hint"))
                    .size(11.0)
                    .color(p.muted),
            )
            .wrap(),
        );
    }
    ui.add_space(28.0);
    // plate size
    section(ui, p, aurora_i18n::tr!("contact-visual-size"));
    let options = [
        (aurora_i18n::tr!("set-editor-size-inherit"), false),
        (aurora_i18n::tr!("set-editor-size-expanded"), false),
        (aurora_i18n::tr!("set-editor-size-reduced"), false),
    ];
    let mut i = match v.size {
        PlateSize::Inherit => 0,
        PlateSize::Expanded => 1,
        PlateSize::Reduced => 2,
    };
    if widgets::segmented(ui, p, &mut i, &options) {
        v.size = [PlateSize::Inherit, PlateSize::Expanded, PlateSize::Reduced][i];
    }
    ui.add_space(6.0);
    ui.add(
        egui::Label::new(
            RichText::new(aurora_i18n::tr!("contact-visual-size-hint"))
                .size(11.0)
                .color(p.muted),
        )
        .wrap(),
    );
    world.contact_sets.set_visuals(set, v);
}

/// A label before its control, the labels of a part lined up.
fn labelled(ui: &mut egui::Ui, p: &Palette, label: &str, control: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(130.0, 26.0), Sense::hover());
        let g = ui
            .painter()
            .layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.5), p.ink);
        widgets::paint_row_text(ui.painter(), g, rect.left(), rect.center().y, p.ink);
        control(ui);
    });
}

/// A tile of "Show the set's colour on": its place, its hint, its switch;
/// a click anywhere on it switches it.
fn flag_tile(ui: &mut egui::Ui, p: &Palette, w: f32, label: &str, hint: &str, on: &mut bool) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 50.0), Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let fill = if *on {
        p.field.lerp_to_gamma(p.raised, 0.6)
    } else if resp.hovered() {
        p.field.lerp_to_gamma(p.raised, 0.35)
    } else {
        p.field
    };
    ui.painter().rect_filled(rect, radius, fill);
    let switch = Rect::from_center_size(Pos2::new(rect.right() - 12.0 - 15.0, rect.center().y), Vec2::new(30.0, 16.0));
    let text_w = (switch.left() - 10.0 - rect.left() - 12.0).max(0.0);
    let line = |text: &str, size: f32, color: Color32| {
        let mut job =
            egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(egui::FontId::proportional(size), color));
        job.wrap = egui::text::TextWrapping::truncate_at_width(text_w);
        ui.painter().layout_job(job)
    };
    widgets::paint_row_text(
        ui.painter(),
        line(label, 12.5, p.ink),
        rect.left() + 12.0,
        rect.center().y - 8.0,
        p.ink,
    );
    widgets::paint_row_text(
        ui.painter(),
        line(hint, 11.0, p.muted),
        rect.left() + 12.0,
        rect.center().y + 9.0,
        p.muted,
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(switch.expand(4.0))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let toggled = widgets::switch(&mut child, p, on).changed();
    if resp.clicked() && !toggled {
        *on = !*on;
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, *on, label));
}

/// The badge: one button showing it, which unfolds the grid of badges
/// under it; then whether it takes the friend symbol's place.
fn badge(ui: &mut egui::Ui, p: &Palette, st: &mut ContactsUi, v: &mut Visuals, cfg: &ContactSet) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let face = if resp.hovered() {
        p.raised.lerp_to_gamma(p.ink, 0.05)
    } else {
        p.raised
    };
    ui.painter().rect_filled(rect, radius, face);
    if st.editor.badges_open {
        ui.painter()
            .rect_stroke(rect, radius, egui::Stroke::new(1.5, p.violet), egui::StrokeKind::Inside);
    }
    let color = crate::interface_colors::on_panel(crate::ui::colors::set_color(cfg.color));
    let disc = Rect::from_center_size(Pos2::new(rect.left() + 10.0 + 15.0, rect.center().y), Vec2::splat(30.0));
    ui.painter().circle_filled(disc.center(), 15.0, p.bar);
    ui.painter()
        .circle_stroke(disc.center(), 15.0, egui::Stroke::new(1.0, color.gamma_multiply(0.6)));
    let name = match v.icon.and_then(|i| ICONS.get(i).map(|icon| (i, icon))) {
        Some((i, icon)) => {
            if let Some(t) = crate::ui::icons::global(icon) {
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                ui.painter()
                    .image(t.id(), Rect::from_center_size(disc.center(), Vec2::splat(15.0)), uv, color);
            }
            aurora_i18n::text(ICON_LABELS[i])
        }
        None => aurora_i18n::text("set-editor-no-badge"),
    };
    let left = disc.right() + 10.0;
    let g = ui.painter().layout_no_wrap(
        name.to_owned(),
        crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), 12.5, p.ink).font_id,
        p.ink,
    );
    widgets::paint_row_text(ui.painter(), g, left, rect.center().y - 7.0, p.ink);
    let g = ui.painter().layout_no_wrap(
        aurora_i18n::tr!("set-editor-badge-hint").to_owned(),
        egui::FontId::proportional(11.0),
        p.muted,
    );
    widgets::paint_row_text(ui.painter(), g, left, rect.center().y + 8.0, p.muted);
    let chevron = if st.editor.badges_open { "chevron-up" } else { "chevron-down" };
    if let Some(t) = crate::ui::icons::global(chevron) {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        ui.painter().image(
            t.id(),
            Rect::from_center_size(Pos2::new(rect.right() - 18.0, rect.center().y), Vec2::splat(14.0)),
            uv,
            p.muted,
        );
    }
    if std::mem::take(&mut st.editor.reveal_badges) {
        resp.scroll_to_me(Some(egui::Align::TOP));
    }
    if resp.clicked() {
        st.editor.badges_open = !st.editor.badges_open;
    }
    if st.editor.badges_open {
        ui.add_space(8.0);
        if let Some(pick) = badge_grid(ui, p, v.icon, cfg) {
            v.icon = pick;
            st.editor.badges_open = false;
        }
    }
    if v.icon.is_some() {
        ui.add_space(10.0);
        widgets::switch_labeled(ui, p, &mut v.replace_friend, aurora_i18n::tr!("set-editor-replace-friend"));
    }
}

/// The custom colour: a dashed circle with a plus, ringed while the set's
/// colour is none of the presets, that opens the colour picker under it.
fn custom_colour(ui: &mut egui::Ui, p: &Palette, world: &mut World, set: &str, cfg: &ContactSet) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
    let custom = !PRESETS.iter().any(|(_, rgb)| is_preset(cfg.color, *rgb));
    let ink = if resp.hovered() { p.ink } else { p.muted };
    let points: Vec<Pos2> = (0..=48)
        .map(|i| {
            let a = i as f32 / 48.0 * std::f32::consts::TAU;
            rect.center() + Vec2::new(a.cos(), a.sin()) * 11.0
        })
        .collect();
    ui.painter()
        .extend(egui::Shape::dashed_line(&points, egui::Stroke::new(1.5, ink), 3.0, 2.6));
    if let Some(t) = crate::ui::icons::global("plus") {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        ui.painter()
            .image(t.id(), Rect::from_center_size(rect.center(), Vec2::splat(12.0)), uv, ink);
    }
    if custom {
        ui.painter().circle_stroke(rect.center(), 15.0, egui::Stroke::new(2.0, p.ink));
    }
    let resp = resp.on_hover_text(aurora_i18n::tr!("set-editor-colour-custom"));
    let id = ui.make_persistent_id("set-editor-custom-colour");
    let [r, g, b, a] = cfg.color;
    let mut srgba = Color32::from(egui::Rgba::from_rgba_unmultiplied(r, g, b, a)).to_srgba_unmultiplied();
    menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        let mut state = ui
            .ctx()
            .data(|d| d.get_temp::<widgets::color::ColorPickState>(id))
            .unwrap_or_else(|| widgets::color::ColorPickState::new(id, srgba));
        if widgets::color::color_panel(ui, p, &mut state, &mut srgba, false) {
            let [r, g, b, a] = srgba;
            let c = egui::Rgba::from(Color32::from_rgba_unmultiplied(r, g, b, a)).to_rgba_unmultiplied();
            world.contact_sets.set_color(set, c);
        }
        ui.ctx().data_mut(|d| d.insert_temp(id, state));
    });
}

/// What a segment of `glyph_choice` shows before its word.
enum Glyph<'a> {
    Icon(&'a str),
    /// The marker's own shape, in the set's colour.
    Shape(MarkerShape, Color32),
}

/// A small choice in one track, each segment a glyph and its word, the lit
/// face sliding to the one chosen (the theme's `segmented`, with glyphs).
/// Returns the segment clicked, if it was not the chosen one.
fn glyph_choice(ui: &mut egui::Ui, p: &Palette, id: &str, chosen: usize, cells: &[(Glyph, &str, &str)]) -> Option<usize> {
    const H: f32 = 28.0;
    const PAD: f32 = 2.0;
    let font = egui::FontId::proportional(12.0);
    let galleys: Vec<_> = cells
        .iter()
        .map(|(_, word, _)| ui.painter().layout_no_wrap((*word).to_owned(), font.clone(), p.ink))
        .collect();
    let widths: Vec<f32> = galleys.iter().map(|g| 10.0 + 14.0 + 6.0 + g.size().x + 12.0).collect();
    let (track, _) = ui.allocate_exact_size(Vec2::new(widths.iter().sum::<f32>() + PAD * 2.0, H), Sense::hover());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    ui.painter().rect_filled(track, radius, p.bar);
    let mut rects = Vec::new();
    let mut x = track.left() + PAD;
    for w in &widths {
        rects.push(Rect::from_min_size(Pos2::new(x, track.top() + PAD), Vec2::new(*w, H - PAD * 2.0)));
        x += w;
    }
    let inner = widgets::inner_radius(ui.visuals());
    if let Some(on) = rects.get(chosen) {
        let key = ui.scope_id().with(("glyph-choice", id));
        let time = ui.style().animation_time;
        let left = ui.ctx().animate_value_with_time(key.with("left"), on.left() - track.left(), time);
        let right = ui.ctx().animate_value_with_time(key.with("right"), on.right() - track.left(), time);
        let lit = Rect::from_x_y_ranges(track.left() + left..=track.left() + right, on.y_range());
        ui.painter().rect_filled(lit, inner, p.raised);
    }
    let mut picked = None;
    for (i, ((glyph, word, tip), galley)) in cells.iter().zip(galleys).enumerate() {
        let rect = rects[i];
        let resp = ui.interact(rect, ui.scope_id().with(("glyph-choice", id, i)), Sense::click());
        let on = i == chosen;
        if !on && resp.hovered() {
            ui.painter().rect_filled(rect, inner, p.raised.gamma_multiply(0.5));
        }
        let ink = if on || resp.hovered() { p.ink } else { p.muted };
        let cell = Rect::from_center_size(Pos2::new(rect.left() + 10.0 + 7.0, rect.center().y), Vec2::splat(14.0));
        match glyph {
            Glyph::Icon(icon) => {
                if let Some(t) = crate::ui::icons::global(icon) {
                    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                    let tint = if on { p.violet_light } else { ink };
                    ui.painter().image(t.id(), cell.shrink(1.0), uv, tint);
                }
            }
            Glyph::Shape(shape, color) => {
                let c = cell.center();
                let fill = if on { *color } else { color.gamma_multiply(0.6) };
                match shape {
                    MarkerShape::Circle => ui.painter().circle_filled(c, 5.5, fill),
                    MarkerShape::Diamond => ui.painter().add(egui::Shape::convex_polygon(
                        vec![
                            c + Vec2::new(0.0, -6.5),
                            c + Vec2::new(6.5, 0.0),
                            c + Vec2::new(0.0, 6.5),
                            c + Vec2::new(-6.5, 0.0),
                        ],
                        fill,
                        egui::Stroke::NONE,
                    )),
                };
            }
        }
        widgets::paint_row_text(ui.painter(), galley, cell.right() + 6.0, rect.center().y, ink);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, *word));
        if resp.on_hover_text(*tip).clicked() && !on {
            picked = Some(i);
        }
    }
    picked
}

/// Where the marker shows: off, on my parcel, in my region.
fn marker_scope(ui: &mut egui::Ui, p: &Palette, v: &mut Visuals) {
    let scopes = [MarkerScope::Off, MarkerScope::Parcel, MarkerScope::Region];
    let cells = [
        (
            Glyph::Icon("eye-off"),
            aurora_i18n::tr!("contact-visual-marker-off"),
            aurora_i18n::tr!("set-editor-marker-off-hint"),
        ),
        (
            Glyph::Icon("home-filled"),
            aurora_i18n::tr!("set-editor-marker-parcel"),
            aurora_i18n::tr!("set-editor-marker-parcel-hint"),
        ),
        (
            Glyph::Icon("world-filled"),
            aurora_i18n::tr!("set-editor-marker-region"),
            aurora_i18n::tr!("set-editor-marker-region-hint"),
        ),
    ];
    let chosen = scopes.iter().position(|s| *s == v.marker).unwrap_or(0);
    if let Some(i) = glyph_choice(ui, p, "marker-scope", chosen, &cells) {
        v.marker = scopes[i];
    }
}

/// The marker's shape, each choice showing the shape in the set's colour.
fn marker_shapes(ui: &mut egui::Ui, p: &Palette, v: &mut Visuals, color: Color32) {
    let shapes = [MarkerShape::Circle, MarkerShape::Diamond];
    let circle = aurora_i18n::tr!("contact-visual-marker-shape-circle");
    let diamond = aurora_i18n::tr!("contact-visual-marker-shape-diamond");
    let cells = [
        (Glyph::Shape(MarkerShape::Circle, color), circle, circle),
        (Glyph::Shape(MarkerShape::Diamond, color), diamond, diamond),
    ];
    let chosen = usize::from(v.marker_shape == MarkerShape::Diamond);
    if let Some(i) = glyph_choice(ui, p, "marker-shape", chosen, &cells) {
        v.marker_shape = shapes[i];
    }
}

/// The badges, as they show on a plate: a disc each, in the set's colour,
/// the one chosen ringed in it; "None" first. Returns the badge clicked.
fn badge_grid(ui: &mut egui::Ui, p: &Palette, current: Option<usize>, cfg: &ContactSet) -> Option<Option<usize>> {
    let color = crate::interface_colors::on_panel(crate::ui::colors::set_color(cfg.color));
    let mut picked = None;
    egui::Frame::new()
        .fill(p.bar)
        .stroke(egui::Stroke::new(1.0, p.raised))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(aurora_i18n::tr!("set-editor-badge-choose")).size(12.0).color(p.muted));
            ui.add_space(10.0);
            const CELL: f32 = 36.0;
            const GAP: f32 = 8.0;
            let per_row = (((ui.available_width() + GAP) / (CELL + GAP)).floor() as usize).max(1);
            let picks: Vec<Option<usize>> = std::iter::once(None).chain((0..ICONS.len()).map(Some)).collect();
            for row in picks.chunks(per_row) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = GAP;
                    for pick in row {
                        let on = *pick == current;
                        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(CELL), Sense::click());
                        let hot = resp.hovered();
                        let fill = if on {
                            color.gamma_multiply(0.18)
                        } else if hot {
                            p.raised
                        } else {
                            p.field
                        };
                        let r = if hot && !on { CELL / 2.0 } else { CELL / 2.0 - 1.0 };
                        ui.painter().circle_filled(rect.center(), r, fill);
                        if on {
                            ui.painter().circle_stroke(rect.center(), r, egui::Stroke::new(1.5, color));
                        }
                        let (icon, tint, tip) = match pick {
                            None => ("ban", p.muted, aurora_i18n::tr!("set-editor-no-badge")),
                            Some(i) => (
                                ICONS[*i],
                                if on || hot { color } else { color.gamma_multiply(0.7) },
                                aurora_i18n::text(ICON_LABELS[*i]),
                            ),
                        };
                        if let Some(t) = crate::ui::icons::global(icon) {
                            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                            ui.painter()
                                .image(t.id(), Rect::from_center_size(rect.center(), Vec2::splat(17.0)), uv, tint);
                        }
                        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, tip));
                        if resp.on_hover_text(tip).clicked() {
                            picked = Some(*pick);
                        }
                    }
                });
                ui.add_space(GAP);
            }
        });
    picked
}

/// A card of a tab: a title, its hint, and a control on the right.
fn card(ui: &mut egui::Ui, p: &Palette, title: &str, hint: &str, control: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().widgets.inactive.corner_radius)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    control(ui);
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add(egui::Label::new(RichText::new(title).size(13.0).color(p.ink)).wrap());
                        if !hint.is_empty() {
                            ui.add(egui::Label::new(RichText::new(hint).size(11.5).color(p.muted)).wrap());
                        }
                    });
                });
            });
        });
    ui.add_space(10.0);
}

/// Behaviour: sorting, notices, a chat section of its own, priority, and
/// the way to the options shared by every set.
fn behaviour(ui: &mut egui::Ui, p: &Palette, world: &mut World, st: &mut ContactsUi, set: &str, cfg: &ContactSet) {
    card(
        ui,
        p,
        aurora_i18n::tr!("ui-contacts-sort-by-online-status"),
        aurora_i18n::tr!("set-editor-sort-hint"),
        |ui| {
            let mut on = cfg.sort_by_online;
            if widgets::switch(ui, p, &mut on).changed() {
                world.contact_sets.set_sort_by_online(set, on);
            }
        },
    );
    card(
        ui,
        p,
        aurora_i18n::tr!("ui-contacts-show-notifications"),
        aurora_i18n::tr!("ui-contacts-show-notifications-tip"),
        |ui| {
            let mut on = cfg.notify;
            if widgets::switch(ui, p, &mut on).changed() {
                world.contact_sets.set_notify(set, on);
            }
        },
    );
    card(
        ui,
        p,
        aurora_i18n::tr!("ui-contacts-separate-chats"),
        aurora_i18n::tr!("ui-contacts-separate-chats-tip"),
        |ui| {
            let mut on = cfg.separate_chats;
            if widgets::switch(ui, p, &mut on).changed() {
                world.contact_sets.set_separate_chats(set, on);
            }
        },
    );
    let (rank, total) = world.contact_sets.priority_rank(set).unwrap_or((0, 1));
    card(
        ui,
        p,
        &aurora_i18n::tr!("contact-visual-priority", rank = rank + 1, total = total),
        aurora_i18n::tr!("set-editor-priority-hint"),
        |ui| {
            if widgets::icon_button(ui, p, "arrow-down", aurora_i18n::tr!("contact-visual-lower"), rank + 1 < total) {
                world.contact_sets.move_priority(set, false);
            }
            if widgets::icon_button(ui, p, "arrow-up", aurora_i18n::tr!("contact-visual-higher"), rank > 0) {
                world.contact_sets.move_priority(set, true);
            }
        },
    );
    ui.add_space(6.0);
    if widgets::action_button(
        ui,
        p,
        "settings-filled",
        aurora_i18n::tr!("set-editor-all-sets-options"),
        widgets::ActionKind::Flat,
        Vec2::new(0.0, 30.0),
    )
    .clicked()
    {
        st.options_open = true;
    }
}

/// Auto-replies: the set's own automatic responses (set_autoresponse_* of
/// floater_fs_contact_set_configuration.xml), a card per status mode, its
/// text greyed while it is off.
fn replies(ui: &mut egui::Ui, p: &Palette, world: &mut World, st: &mut ContactsUi, set: &str, cfg: &ContactSet) {
    if st.response_edit.as_ref().is_none_or(|(name, _)| name != set) {
        st.response_edit = Some((set.to_owned(), ResponseMode::ALL.map(|m| cfg.response(m).text.clone())));
    }
    ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("set-editor-replies-hint")).size(12.0).color(p.muted)).wrap());
    ui.add_space(12.0);
    for (i, mode) in ResponseMode::ALL.into_iter().enumerate() {
        let label = match mode {
            ResponseMode::Busy => aurora_i18n::tr!("ui-contacts-set-response-busy"),
            ResponseMode::Autorespond => aurora_i18n::tr!("ui-contacts-set-response-autorespond"),
            ResponseMode::NonFriends => aurora_i18n::tr!("ui-contacts-set-response-nonfriends"),
        };
        // greyed under RLVa @sendim, like Preferences' do not disturb text
        // (the world does not send it then)
        let locked = mode == ResponseMode::Busy && world.rlva.has("sendim");
        let mut enabled = cfg.response(mode).enabled;
        let mut toggled = false;
        egui::Frame::new()
            .fill(p.field)
            .corner_radius(ui.visuals().widgets.inactive.corner_radius)
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(label).size(13.0).color(p.ink));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        toggled = ui
                            .add_enabled_ui(!locked, |ui| widgets::switch(ui, p, &mut enabled).changed())
                            .inner;
                    });
                });
                let Some((_, texts)) = st.response_edit.as_mut() else {
                    return;
                };
                if enabled {
                    ui.add_space(8.0);
                    let field = ui.add_enabled(
                        !locked,
                        egui::TextEdit::multiline(&mut texts[i])
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    );
                    if field.lost_focus() {
                        let text = texts[i].clone();
                        world.contact_sets.set_response(set, mode, SetResponse { enabled, text });
                    }
                }
            });
        if toggled && let Some((_, texts)) = st.response_edit.as_ref() {
            let text = texts[i].clone();
            world.contact_sets.set_response(set, mode, SetResponse { enabled, text });
        }
        ui.add_space(10.0);
    }
}

/// What is still typed in the response fields, when the window closes.
fn save_responses(world: &mut World, st: &ContactsUi, set: &str) {
    let Some((name, texts)) = &st.response_edit else {
        return;
    };
    if name != set {
        return;
    }
    for (i, mode) in ResponseMode::ALL.into_iter().enumerate() {
        let Some(enabled) = world.contact_sets.set(set).map(|c| c.response(mode).enabled) else {
            return;
        };
        let text = texts[i].clone();
        world.contact_sets.set_response(set, mode, SetResponse { enabled, text });
    }
}

/// Members: the set's residents, each taken out with a confirmation, and
/// "Add people" (the resident picker).
fn members(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &mut World,
    st: &mut ContactsUi,
    set: &str,
    cfg: &ContactSet,
    pics: &HashMap<Uuid, egui::TextureHandle>,
) {
    let names = &world.social.avatar_names;
    let name_of = |id: &Uuid| {
        names
            .get(id)
            .map(|n| n.display(&names.options))
            .unwrap_or_else(|| world.social.name_of(id))
    };
    let mut ids: Vec<Uuid> = cfg.friends.iter().copied().collect();
    ids.sort_by_cached_key(|id| crate::natural_order::key(&name_of(id)));
    if ids.is_empty() {
        ui.label(RichText::new(aurora_i18n::tr!("ui-contacts-this-contact-set-is-empty")).color(p.muted));
        ui.add_space(10.0);
    }
    for id in &ids {
        let name = name_of(id);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::hover());
        ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.field);
        let disc = Rect::from_center_size(Pos2::new(rect.left() + 8.0 + 14.0, rect.center().y), Vec2::splat(28.0));
        if !pics.contains_key(id) {
            st.wanted_pics.insert(*id);
        }
        widgets::avatar_disc(ui, p, disc, pics.get(id), &name, false);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(Vec2::new(8.0, 6.0)))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        if widgets::action_button(
            &mut child,
            p,
            "circle-minus",
            aurora_i18n::tr!("people-sets-remove"),
            widgets::ActionKind::FlatDanger,
            Vec2::new(0.0, 26.0),
        )
        .clicked()
        {
            st.ask_remove_from_set(set, vec![*id]);
        }
        let g = ui.painter().layout_no_wrap(name, egui::FontId::proportional(13.0), p.ink);
        widgets::paint_row_text(ui.painter(), g, disc.right() + 10.0, rect.center().y, p.ink);
        ui.add_space(3.0);
    }
    ui.add_space(8.0);
    if widgets::action_button(
        ui,
        p,
        "user-plus",
        aurora_i18n::tr!("people-sets-add-people"),
        widgets::ActionKind::Flat,
        Vec2::new(0.0, 30.0),
    )
    .clicked()
    {
        st.pick_for_set(set);
    }
}

/// Delete the set, the reminder that changes apply at once, and Done.
fn footer(ui: &mut egui::Ui, p: &Palette, world: &mut World, st: &mut ContactsUi, set: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), FOOTER_H), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.top(), widgets::divider_stroke(p));
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(Pos2::new(rect.left(), rect.top() + 12.0), rect.max))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 10.0;
    if widgets::action_button(
        &mut row,
        p,
        "trash-filled",
        aurora_i18n::tr!("set-editor-delete"),
        widgets::ActionKind::FlatDanger,
        Vec2::new(0.0, 30.0),
    )
    .clicked()
    {
        st.ask_remove_set(set);
    }
    row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::primary_button_sized(ui, p, aurora_i18n::tr!("set-editor-done"), Vec2::new(90.0, 30.0)).clicked() {
            close(world, st, set);
        }
        ui.label(
            RichText::new(aurora_i18n::tr!("set-editor-applies-at-once"))
                .size(11.5)
                .color(p.muted),
        );
    });
}

/// The set alone, to show its look without the others' (in memory only,
/// with the same cascade and painters as the live plates and markers).
fn sample(cfg: &ContactSet) -> std::sync::Arc<Appearance> {
    let mut sample = ContactSets::default();
    sample.add_set(&cfg.name);
    sample.set_color(&cfg.name, cfg.color);
    sample.set_visuals(&cfg.name, cfg.visuals.clone());
    let id = Uuid::from_u128(1);
    sample.add_to_set(&[id], &cfg.name, |_| true);
    sample.appearance(&id).clone()
}

/// What the marker's preview says when the set has one: where it shows,
/// and what it does up close.
fn marker_caption(v: &Visuals) -> Option<[&'static str; 2]> {
    let scope = match v.marker {
        MarkerScope::Off => return None,
        MarkerScope::Parcel => aurora_i18n::text("set-editor-preview-far-parcel"),
        MarkerScope::Region => aurora_i18n::text("set-editor-preview-far-region"),
    };
    let near = match (v.icon.is_some(), v.marker_fade.is_some()) {
        (true, _) => aurora_i18n::text("set-editor-preview-near-lands"),
        (false, true) => aurora_i18n::text("set-editor-preview-near-fades"),
        (false, false) => aurora_i18n::text("set-editor-preview-near-stays"),
    };
    Some([scope, near])
}

/// The live preview: the plate over a resident, a chat line, a list row,
/// and the marker over a resident far away while the set has one.
fn preview(ui: &mut egui::Ui, p: &Palette, cfg: &ContactSet) {
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.label(
        RichText::new(aurora_i18n::tr!("set-editor-preview").to_uppercase())
            .size(10.5)
            .color(p.muted)
            .family(crate::ui::fonts::semibold(ui.ctx())),
    );
    ui.add_space(12.0);
    let appearance = sample(cfg);
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let body = p.raised.lerp_to_gamma(p.field, 0.2);
    // a resident near: shoulders and head, their plate over them
    let (scene, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 180.0), Sense::hover());
    ui.painter().rect_filled(scene, radius, p.field);
    let base = Pos2::new(scene.center().x, scene.bottom());
    ui.painter().rect_filled(
        Rect::from_min_max(base - Vec2::new(40.0, 44.0), base + Vec2::new(40.0, 0.0)),
        NEAR_SHOULDERS,
        body,
    );
    ui.painter().circle_filled(base - Vec2::new(0.0, 64.0), 16.0, body);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                Pos2::new(scene.left(), scene.top() + 10.0),
                Pos2::new(scene.right(), scene.bottom() - 82.0),
            ))
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    crate::ui::nameplate::preview(&mut child, p, appearance.clone());
    ui.add_space(10.0);
    let shown =
        |c: Option<[f32; 4]>, fallback: Color32| c.map_or(fallback, |c| crate::interface_colors::on_panel(crate::ui::colors::set_color(c)));
    let sample_name = aurora_i18n::tr!("contact-visual-sample-name");
    // a chat line
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(radius)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let mut job = egui::text::LayoutJob::default();
            job.append(
                sample_name,
                0.0,
                crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), 12.5, shown(appearance.chat_name, p.ink)),
            );
            job.append(
                aurora_i18n::tr!("set-editor-sample-chat"),
                0.0,
                egui::TextFormat::simple(
                    egui::FontId::proportional(12.5),
                    shown(appearance.chat_text, p.ink.gamma_multiply(0.85)),
                ),
            );
            ui.add(egui::Label::new(job).wrap());
        });
    ui.add_space(10.0);
    // a list row
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 44.0), Sense::hover());
    ui.painter().rect_filled(row, radius, p.field);
    let disc = Rect::from_center_size(Pos2::new(row.left() + 8.0 + 13.0, row.center().y), Vec2::splat(26.0));
    widgets::avatar_disc(ui, p, disc, None, sample_name, false);
    let mut job = egui::text::LayoutJob::single_section(
        sample_name.to_owned(),
        crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), 12.5, shown(appearance.ui_name, p.ink)),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((row.right() - disc.right() - 18.0).max(0.0));
    widgets::paint_row_text(
        ui.painter(),
        ui.painter().layout_job(job),
        disc.right() + 8.0,
        row.center().y,
        p.ink,
    );
    // the marker, over a resident far away, while the set has one
    if let (Some(marker), Some([scope, near])) = (&appearance.marker, marker_caption(&cfg.visuals)) {
        ui.add_space(10.0);
        egui::Frame::new()
            .fill(p.field)
            .corner_radius(radius)
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let (cell, _) = ui.allocate_exact_size(Vec2::new(54.0, 84.0), Sense::hover());
                    let base = Pos2::new(cell.center().x, cell.bottom() - 2.0);
                    ui.painter().circle_filled(base - Vec2::new(0.0, 26.0), 6.0, body);
                    ui.painter().rect_filled(
                        Rect::from_min_max(base - Vec2::new(15.0, 18.0), base + Vec2::new(15.0, 0.0)),
                        FAR_SHOULDERS,
                        body,
                    );
                    crate::ui::contact_markers::paint_preview_at(ui.painter(), p, base - Vec2::new(0.0, 60.0), marker);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        ui.add(egui::Label::new(RichText::new(scope).size(11.5).color(p.ink)).wrap());
                        ui.add(egui::Label::new(RichText::new(near).size(11.0).color(p.muted)).wrap());
                    });
                });
            });
    }
    ui.add_space(14.0);
    ui.label(
        RichText::new(aurora_i18n::tr!("set-editor-applies-to", count = cfg.friends.len()))
            .size(12.0)
            .color(p.muted),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_place_of_the_colour_is_a_flag_of_its_own() {
        let mut v = Visuals::default();
        for i in 0..SHOWN_ON.len() {
            *shown_on(&mut v, i) = true;
        }
        assert!(v.nameplate_name && v.border && v.background && v.ui_name && v.chat_name && v.chat_text && v.minimap);
        // each index sets a different flag
        for i in 0..SHOWN_ON.len() {
            let mut v = Visuals::default();
            *shown_on(&mut v, i) = true;
            let set = [
                v.nameplate_name,
                v.border,
                v.background,
                v.ui_name,
                v.chat_name,
                v.chat_text,
                v.minimap,
            ];
            assert_eq!(set.iter().filter(|b| **b).count(), 1, "{i}");
            assert!(set[i], "{i}");
        }
    }

    #[test]
    fn a_preset_is_recognised_once_stored() {
        for (_, rgb) in PRESETS {
            assert!(is_preset(preset_color(rgb), rgb), "{rgb:?}");
        }
        assert!(!is_preset(preset_color(PRESETS[0].1), PRESETS[1].1));
    }

    #[test]
    fn the_marker_preview_shows_only_with_a_marker_and_says_what_it_does() {
        let mut v = Visuals::default();
        assert_eq!(marker_caption(&v), None, "no marker: no preview");
        v.marker = MarkerScope::Region;
        let region = aurora_i18n::text("set-editor-preview-far-region");
        assert_eq!(
            marker_caption(&v),
            Some([region, aurora_i18n::text("set-editor-preview-near-stays")])
        );
        v.marker_fade = Some(10);
        assert_eq!(
            marker_caption(&v),
            Some([region, aurora_i18n::text("set-editor-preview-near-fades")])
        );
        // with a badge it lands on it, fading or not
        v.icon = Some(0);
        assert_eq!(
            marker_caption(&v),
            Some([region, aurora_i18n::text("set-editor-preview-near-lands")])
        );
        v.marker = MarkerScope::Parcel;
        assert_eq!(
            marker_caption(&v).map(|c| c[0]),
            Some(aurora_i18n::text("set-editor-preview-far-parcel"))
        );
    }

    #[test]
    fn a_rename_that_cannot_be_done_puts_the_name_back() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.contact_sets.add_set("A");
        world.contact_sets.add_set("B");
        let mut st = ContactsUi::default();
        st.configure_set("A");
        st.set_choice = Some("A".into());
        // another set's name
        st.rename = "B".into();
        rename(&mut world, &mut st, "A");
        assert_eq!(st.rename, "A");
        assert!(world.contact_sets.is_valid("A"));
        // empty
        st.rename = "  ".into();
        rename(&mut world, &mut st, "A");
        assert_eq!(st.rename, "A");
        // a new name follows in the editor and the choice
        st.rename = "C".into();
        rename(&mut world, &mut st, "A");
        assert!(world.contact_sets.is_valid("C") && !world.contact_sets.is_valid("A"));
        assert_eq!(st.config.as_deref(), Some("C"));
        assert_eq!(st.set_choice.as_deref(), Some("C"));
    }
}

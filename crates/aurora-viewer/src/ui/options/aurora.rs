//! Preferences › Aurora: what Aurora does differently from Firestorm, and
//! for each redrawn element the way back to Firestorm's look. The entry of
//! the category list and the head of the page carry the aurora gradient,
//! the one exception to the theme here (docs/BRANDING.md); the settings
//! below keep the cards and controls of the other pages.

use super::{SLIDER_W, group, slider, toggle};
use crate::settings::{ChatStyle, NotificationsStyle, PhotoWindowStyle, PlateClick, Settings, WorldMapStyle};
use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind};
use crate::world::lookat::{Aim, Style};
use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, FontId, Pos2, Rect, Sense, Shape, Vec2};

/// The two looks a redrawn element can take.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Look {
    Aurora,
    Firestorm,
}

/// Every choice of the Appearance, Gaze, Movement and Animations groups set to one
/// look; the avatar plates' own settings are left as they are.
pub fn set_all(s: &mut Settings, look: Look) {
    let aurora = look == Look::Aurora;
    s.chat_style = if aurora { ChatStyle::Aurora } else { ChatStyle::Legacy };
    s.classic_name_tags = !aurora;
    s.lookat.style = if aurora { Style::Aurora } else { Style::Legacy };
    s.lookat.aim = if aurora { Aim::Point } else { Aim::Direction };
    s.notifications_style = if aurora {
        NotificationsStyle::Aurora
    } else {
        NotificationsStyle::Legacy
    };
    s.world_map_style = if aurora { WorldMapStyle::Aurora } else { WorldMapStyle::Legacy };
    s.classic_bars = !aurora;
    s.classic_people = !aurora;
    s.classic_profiles = !aurora;
    s.photo_window = if aurora {
        PhotoWindowStyle::Aurora
    } else {
        PhotoWindowStyle::Legacy
    };
    s.classic_about_land = !aurora;
    s.classic_search = !aurora;
    s.classic_region_estate = !aurora;
    s.classic_inventory = !aurora;
    s.classic_places = !aurora;
    s.classic_pay_window = !aurora;
    s.classic_controls = !aurora;
    s.classic_restart_alert = !aurora;
    s.classic_chat_lines = !aurora;
    s.classic_rez_effect = !aurora;
    s.predict_own_motion = aurora;
    s.water_effects.enabled = aurora;
    s.synchronize_animations = aurora;
}

/// The aurora gradient at `t` (0 at its start, 1 at its end): the brand's
/// green, turquoise, indigo, violet and rose, taken from the palette.
fn aurora_color(p: &Palette, t: f32) -> Color32 {
    let stops = [p.success, p.teal, p.indigo_light, p.violet_light, p.rose];
    let x = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let i = (x.floor() as usize).min(stops.len() - 2);
    stops[i].lerp_to_gamma(stops[i + 1], x - i as f32)
}

fn vertex(pos: Pos2, color: Color32) -> Vertex {
    Vertex { pos, uv: WHITE_UV, color }
}

/// A strip of the aurora gradient, along its width when `horizontal`, else
/// from its top to its bottom.
fn aurora_strip(p: &Palette, r: Rect, horizontal: bool) -> Shape {
    const STEPS: usize = 16;
    let mut m = Mesh::default();
    for i in 0..=STEPS {
        let t = i as f32 / STEPS as f32;
        let c = aurora_color(p, t);
        if horizontal {
            let x = r.left() + r.width() * t;
            m.vertices.push(vertex(Pos2::new(x, r.top()), c));
            m.vertices.push(vertex(Pos2::new(x, r.bottom()), c));
        } else {
            let y = r.top() + r.height() * t;
            m.vertices.push(vertex(Pos2::new(r.left(), y), c));
            m.vertices.push(vertex(Pos2::new(r.right(), y), c));
        }
        if i > 0 {
            let k = (i as u32) * 2;
            m.add_triangle(k - 2, k - 1, k);
            m.add_triangle(k - 1, k + 1, k);
        }
    }
    Shape::mesh(m)
}

/// A soft glow: `color` at the centre of an ellipse, transparent at its edge.
fn glow(center: Pos2, radius: Vec2, color: Color32) -> Shape {
    const SEGMENTS: u32 = 48;
    let mut m = Mesh::default();
    m.vertices.push(vertex(center, color));
    for i in 0..=SEGMENTS {
        let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let pos = center + Vec2::new(a.cos() * radius.x, a.sin() * radius.y);
        m.vertices.push(vertex(pos, Color32::TRANSPARENT));
        if i > 0 {
            m.add_triangle(0, i, i + 1);
        }
    }
    Shape::mesh(m)
}

/// The entry of the page in the category list: the neighbours' entry, with
/// the aurora gradient in its edge mark and its name.
pub fn nav_entry(ui: &mut egui::Ui, p: &Palette, label: &str, icon: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let face = if selected {
        p.violet.gamma_multiply(0.24)
    } else if response.hovered() {
        p.raised
    } else {
        p.raised.gamma_multiply(0.3)
    };
    ui.painter().rect_filled(rect, radius, face);
    ui.painter().add(aurora_strip(
        p,
        Rect::from_min_max(rect.left_top(), Pos2::new(rect.left() + 3.0, rect.bottom())),
        false,
    ));
    if let Some(texture) = crate::ui::icons::global(icon) {
        ui.painter().image(
            texture.id(),
            Rect::from_center_size(rect.left_center() + Vec2::new(17.0, 0.0), Vec2::splat(17.0)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            p.teal,
        );
    }
    // the name in the gradient, one letter after the other
    let font = FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx()));
    let count = label.chars().count().max(2) as f32 - 1.0;
    let mut job = egui::text::LayoutJob::default();
    for (i, ch) in label.chars().enumerate() {
        let color = aurora_color(p, i as f32 / count);
        job.append(&ch.to_string(), 0.0, egui::TextFormat::simple(font.clone(), color));
    }
    let galley = ui.painter().layout_job(job);
    ui.painter()
        .galley(rect.left_center() + Vec2::new(34.0, -galley.size().y / 2.0), galley, p.ink);
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label));
    response.on_hover_text(label)
}

/// Head of the page: the aurora over the night sky, the title, what the
/// page is for, and the two buttons that set every look at once.
fn hero(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, reset: &mut bool) -> bool {
    const PAD: f32 = 16.0;
    const BUTTON_H: f32 = 26.0;
    let width = ui.available_width();
    let title = ui.painter().layout_no_wrap(
        aurora_i18n::tr!("ui-options-aurora").to_owned(),
        FontId::new(18.0, crate::ui::fonts::semibold(ui.ctx())),
        p.ink,
    );
    // the text runs under the title and the buttons, as wide as a short line
    let intro = ui.painter().layout(
        aurora_i18n::tr!("ui-options-aurora-intro").to_owned(),
        FontId::proportional(12.5),
        p.muted,
        (width - PAD * 2.0).min(560.0),
    );
    let intro_top = PAD + title.size().y.max(BUTTON_H) + 4.0;
    let h = intro_top + intro.size().y + PAD + 2.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, h), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, ui.visuals().window_corner_radius, p.navy);
    // the aurora rising from the bottom edge: green, indigo, rose
    let below = rect.bottom() + h * 0.2;
    for (x, color, alpha) in [(0.2, p.success, 0.8), (0.55, p.indigo_light, 0.8), (0.9, p.rose, 0.65)] {
        painter.add(glow(
            Pos2::new(rect.left() + rect.width() * x, below),
            Vec2::new(rect.width() * 0.3, h * 0.9),
            color.gamma_multiply(alpha),
        ));
    }
    // the night darkening toward the bottom, then the hairline
    let mut veil = Mesh::default();
    let (top, bottom) = (p.navy.gamma_multiply(0.2), p.navy.gamma_multiply(0.5));
    veil.vertices.extend([
        vertex(rect.left_top(), top),
        vertex(rect.right_top(), top),
        vertex(rect.right_bottom(), bottom),
        vertex(rect.left_bottom(), bottom),
    ]);
    veil.add_triangle(0, 1, 2);
    veil.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(veil));
    painter.add(aurora_strip(
        p,
        Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 2.0), rect.right_bottom()),
        true,
    ));

    let title_pos = Pos2::new(rect.left() + PAD, rect.top() + PAD + (BUTTON_H - title.size().y).max(0.0) / 2.0);
    painter.galley(title_pos, title, p.ink);
    painter.galley(Pos2::new(rect.left() + PAD, rect.top() + intro_top), intro, p.muted);

    // the two buttons at the top right, on the title's line (a child that
    // leaves the page's cursor where the head ends)
    let size = Vec2::new(0.0, BUTTON_H);
    let mut changed = false;
    let slot = Rect::from_min_max(
        Pos2::new(rect.left() + PAD, rect.top() + PAD),
        Pos2::new(rect.right() - PAD, rect.top() + PAD + BUTTON_H),
    );
    let mut buttons = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(slot)
            .layout(egui::Layout::right_to_left(egui::Align::Min)),
    );
    buttons.spacing_mut().item_spacing.x = 6.0;
    let aurora = widgets::action_button(
        &mut buttons,
        p,
        "sparkles-filled",
        aurora_i18n::tr!("ui-options-aurora-all-aurora"),
        ActionKind::Primary,
        size,
    );
    let legacy = widgets::action_button(
        &mut buttons,
        p,
        "history",
        aurora_i18n::tr!("ui-options-aurora-all-firestorm"),
        ActionKind::Flat,
        size,
    );
    // "Reset this section…" of the other pages, its icon alone here
    *reset |= super::reset_section_button(&mut buttons, p, super::TAB_AURORA, true);
    if aurora.clicked() {
        set_all(s, Look::Aurora);
        changed = true;
    }
    if legacy.clicked() {
        set_all(s, Look::Firestorm);
        changed = true;
    }
    changed
}

/// One setting of the page: its name over its description on the left,
/// its control at the right edge, centred on the text.
fn item(ui: &mut egui::Ui, p: &Palette, name: &str, description: &str, control: Vec2, draw: impl FnOnce(&mut egui::Ui) -> bool) -> bool {
    item_with(ui, p, name, None, description, control, draw)
}

/// `item` with a warning triangle after its name, `warning` its tooltip (an
/// experimental setting).
fn item_with(
    ui: &mut egui::Ui,
    p: &Palette,
    name: &str,
    warning: Option<&str>,
    description: &str,
    control: Vec2,
    draw: impl FnOnce(&mut egui::Ui) -> bool,
) -> bool {
    const PAD: f32 = 9.0;
    const MARK: f32 = 14.0;
    const GAP: f32 = 16.0;
    let width = ui.available_width();
    let text_w = (width - control.x - GAP).max(120.0);
    let name_g = ui.painter().layout(
        name.to_owned(),
        FontId::new(13.0, crate::ui::fonts::semibold(ui.ctx())),
        p.ink,
        text_w,
    );
    let desc_g = ui
        .painter()
        .layout(description.to_owned(), FontId::proportional(12.0), p.muted, text_w);
    let text_h = name_g.size().y + 2.0 + desc_g.size().y;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, text_h.max(control.y) + PAD * 2.0), Sense::hover());
    let top = rect.center().y - text_h / 2.0;
    let desc_top = top + name_g.size().y + 2.0;
    let name_rect = Rect::from_min_size(Pos2::new(rect.left(), top), name_g.size());
    ui.painter().galley(name_rect.min, name_g, p.ink);
    ui.painter().galley(Pos2::new(rect.left(), desc_top), desc_g, p.muted);
    if let Some(tip) = warning {
        let mark = Rect::from_center_size(
            Pos2::new(name_rect.right() + 6.0 + MARK / 2.0, name_rect.center().y),
            Vec2::splat(MARK),
        );
        crate::ui::sheet::icon_at(ui, "alert-triangle-filled", mark.center(), MARK, p.warn);
        ui.interact(mark.expand(3.0), ui.id().with(("aurora_item_warning", name)), Sense::hover())
            .on_hover_text(tip);
    }
    let slot = Rect::from_center_size(Pos2::new(rect.right() - control.x / 2.0, rect.center().y), control);
    // a child that does not move the page's cursor: the row is already laid
    // out. Its own id: without it every row's child had the same, and the
    // segments of the same place and label ("Aurora", first of several
    // rows) shared an id, the last row's taking the clicks of the others.
    draw(&mut ui.new_child(egui::UiBuilder::new().id_salt(("aurora_item", name)).max_rect(slot)))
}

/// The faint line between two settings of a card.
fn between(ui: &mut egui::Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        egui::Stroke::new(1.0, p.raised.gamma_multiply(0.5)),
    );
}

/// Width of the drop-down list of the plate click.
const PLATE_CLICK_W: f32 = 200.0;

/// The name of a plate click action in its drop-down list: the names of
/// the avatar menu's entries they run.
fn plate_click_label(action: PlateClick) -> &'static str {
    match action {
        PlateClick::Nothing => aurora_i18n::tr!("ui-options-plate-click-nothing"),
        PlateClick::Profile => aurora_i18n::tr!("common-view-profile"),
        PlateClick::InstantMessage => aurora_i18n::tr!("ui-context-send-im"),
        PlateClick::OfferTeleport => aurora_i18n::tr!("common-offer-teleport"),
        PlateClick::Menu => aurora_i18n::tr!("ui-options-plate-click-menu"),
    }
}

/// A choice between Aurora's look (first, in the accent) and Firestorm's.
fn look_choice(ui: &mut egui::Ui, p: &Palette, name: &str, description: &str, aurora: &str, firestorm: &str, legacy: &mut bool) -> bool {
    look_choice_with(ui, p, name, None, description, aurora, firestorm, legacy)
}

/// `look_choice` with a warning triangle after its name (`item_with`).
#[allow(clippy::too_many_arguments)]
fn look_choice_with(
    ui: &mut egui::Ui,
    p: &Palette,
    name: &str,
    warning: Option<&str>,
    description: &str,
    aurora: &str,
    firestorm: &str,
    legacy: &mut bool,
) -> bool {
    let options = [(aurora, true), (firestorm, false)];
    let size = widgets::segmented_size(ui, &options);
    item_with(ui, p, name, warning, description, size, |ui| {
        let mut selected = usize::from(*legacy);
        let changed = widgets::segmented(ui, p, &mut selected, &options);
        *legacy = selected == 1;
        changed
    })
}

/// The Conversations window's four looks: Aurora's workspace, its compact
/// rows, the bubbles, Firestorm's lines (the two Aurora ones in the accent).
fn chat_style_choice(ui: &mut egui::Ui, p: &Palette, style: &mut ChatStyle) -> bool {
    const STYLES: [ChatStyle; 4] = [ChatStyle::Aurora, ChatStyle::Compact, ChatStyle::Bubbles, ChatStyle::Legacy];
    let options = [
        (aurora_i18n::tr!("ui-options-aurora-workspace"), true),
        (aurora_i18n::tr!("ui-options-aurora-compact"), true),
        (aurora_i18n::tr!("ui-options-aurora-bubbles"), true),
        (aurora_i18n::tr!("ui-options-aurora-lines"), false),
    ];
    let size = widgets::segmented_size(ui, &options);
    item(
        ui,
        p,
        aurora_i18n::tr!("ui-options-aurora-conversations"),
        aurora_i18n::tr!("ui-options-aurora-conversations-help"),
        size,
        |ui| {
            let mut selected = STYLES.iter().position(|s| s == style).unwrap_or(0);
            let changed = widgets::segmented(ui, p, &mut selected, &options);
            *style = STYLES[selected];
            changed
        },
    )
}

fn animation_clock_choice(ui: &mut egui::Ui, p: &Palette, synchronized: &mut bool) -> bool {
    let mut legacy = !*synchronized;
    let changed = look_choice_with(
        ui,
        p,
        aurora_i18n::tr!("ui-options-aurora-animation-clock"),
        Some(aurora_i18n::tr!("ui-options-aurora-animation-clock-experimental")),
        aurora_i18n::tr!("ui-options-aurora-animation-clock-help"),
        aurora_i18n::tr!("ui-options-aurora-animation-clock-synchronized"),
        aurora_i18n::tr!("ui-options-aurora-animation-clock-legacy"),
        &mut legacy,
    );
    *synchronized = !legacy;
    changed
}

/// `reset`: "Reset this section…" was clicked in the page's head.
pub fn page(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, reset: &mut bool) -> bool {
    let mut c = hero(ui, p, s, reset);
    c |= water_page(ui, p, s);
    ui.add_space(14.0);
    group(ui, p, aurora_i18n::tr!("ui-options-aurora-appearance"), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        c |= chat_style_choice(ui, p, &mut s.chat_style);
        between(ui, p);
        c |= look_choice(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-chat-lines"),
            aurora_i18n::tr!("ui-options-aurora-chat-lines-help"),
            aurora_i18n::tr!("ui-options-aurora-chat-lines-cards"),
            aurora_i18n::tr!("ui-options-aurora-chat-lines-classic"),
            &mut s.classic_chat_lines,
        );
        between(ui, p);
        c |= look_choice(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-name-tags"),
            aurora_i18n::tr!("ui-options-aurora-name-tags-help"),
            aurora_i18n::tr!("ui-options-aurora-plates"),
            aurora_i18n::tr!("ui-options-aurora-classic"),
            &mut s.classic_name_tags,
        );
        between(ui, p);
        let mut legacy = s.lookat.style == Style::Legacy;
        if look_choice(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-look-at"),
            aurora_i18n::tr!("ui-options-aurora-look-at-help"),
            aurora_i18n::tr!("ui-options-aurora-halo"),
            aurora_i18n::tr!("ui-options-aurora-crosshair"),
            &mut legacy,
        ) {
            s.lookat.style = if legacy { Style::Legacy } else { Style::Aurora };
            c = true;
        }
        between(ui, p);
        let mut legacy = s.notifications_style == NotificationsStyle::Legacy;
        if look_choice(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-notifications"),
            aurora_i18n::tr!("ui-options-aurora-notifications-help"),
            aurora_i18n::tr!("ui-options-aurora-animated-bell"),
            aurora_i18n::tr!("ui-options-aurora-toasts"),
            &mut legacy,
        ) {
            s.notifications_style = if legacy {
                NotificationsStyle::Legacy
            } else {
                NotificationsStyle::Aurora
            };
            c = true;
        }
        between(ui, p);
        c |= look_choice(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-bars"),
            aurora_i18n::tr!("ui-options-aurora-bars-help"),
            aurora_i18n::tr!("ui-options-aurora-bars-islands"),
            aurora_i18n::tr!("ui-options-aurora-bars-classic"),
            &mut s.classic_bars,
        );
        between(ui, p);
        let mut legacy = s.world_map_style == WorldMapStyle::Legacy;
        if look_choice(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-world-map"),
            aurora_i18n::tr!("ui-options-aurora-world-map-help"),
            aurora_i18n::tr!("ui-options-aurora-world-map-full"),
            aurora_i18n::tr!("ui-options-aurora-world-map-side"),
            &mut legacy,
        ) {
            s.world_map_style = if legacy { WorldMapStyle::Legacy } else { WorldMapStyle::Aurora };
            c = true;
        }
        between(ui, p);
        c |= look_choice(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-places"),
            aurora_i18n::tr!("ui-options-aurora-places-help"),
            aurora_i18n::tr!("ui-options-aurora-places-aurora"),
            aurora_i18n::tr!("ui-options-aurora-places-classic"),
            &mut s.classic_places,
        );
        between(ui, p);
        c |= look_choice(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-people"),
            aurora_i18n::tr!("ui-options-aurora-people-help"),
            aurora_i18n::tr!("ui-options-aurora-people-rows"),
            aurora_i18n::tr!("ui-options-aurora-people-table"),
            &mut s.classic_people,
        );
        between(ui, p);
        c |= look_choice(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-profiles"),
            aurora_i18n::tr!("ui-options-aurora-profiles-help"),
            aurora_i18n::tr!("ui-options-aurora-profiles-aurora"),
            aurora_i18n::tr!("ui-options-aurora-profiles-classic"),
            &mut s.classic_profiles,
        );
        between(ui, p);
        let mut legacy = s.photo_window == PhotoWindowStyle::Legacy;
        if look_choice(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-photo-window"),
            aurora_i18n::tr!("ui-options-aurora-photo-window-help"),
            aurora_i18n::tr!("ui-options-aurora-photo-panel"),
            aurora_i18n::tr!("ui-options-aurora-photo-classic"),
            &mut legacy,
        ) {
            s.photo_window = if legacy {
                PhotoWindowStyle::Legacy
            } else {
                PhotoWindowStyle::Aurora
            };
            c = true;
        }
        between(ui, p);
        c |= look_choice(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-about-land"),
            aurora_i18n::tr!("ui-options-aurora-about-land-help"),
            aurora_i18n::tr!("ui-options-aurora-about-land-aurora"),
            aurora_i18n::tr!("ui-options-aurora-about-land-classic"),
            &mut s.classic_about_land,
        );
        between(ui, p);
        c |= look_choice(
            ui,
            p,
            aurora_i18n::tr!("search-title"),
            aurora_i18n::tr!("ui-options-aurora-search-help"),
            aurora_i18n::tr!("ui-options-aurora-about-land-aurora"),
            aurora_i18n::tr!("ui-options-aurora-search-classic"),
            &mut s.classic_search,
        );
        between(ui, p);
        c |= look_choice(
            ui,
            p,
            aurora_i18n::tr!("region-title"),
            aurora_i18n::tr!("ui-options-aurora-region-estate-help"),
            aurora_i18n::tr!("ui-options-aurora-about-land-aurora"),
            aurora_i18n::tr!("ui-options-aurora-about-land-classic"),
            &mut s.classic_region_estate,
        );
        between(ui, p);
        c |= look_choice(
            ui,
            p,
            aurora_i18n::tr!("common-inventory"),
            aurora_i18n::tr!("ui-options-aurora-inventory-help"),
            aurora_i18n::tr!("ui-options-aurora-about-land-aurora"),
            aurora_i18n::tr!("ui-options-aurora-about-land-classic"),
            &mut s.classic_inventory,
        );
        between(ui, p);
        c |= look_choice(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-pay-window"),
            aurora_i18n::tr!("ui-options-aurora-pay-window-help"),
            aurora_i18n::tr!("ui-options-aurora-pay-aurora"),
            aurora_i18n::tr!("ui-options-aurora-pay-classic"),
            &mut s.classic_pay_window,
        );
        between(ui, p);
        c |= look_choice(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-controls"),
            aurora_i18n::tr!("ui-options-aurora-controls-help"),
            aurora_i18n::tr!("ui-options-aurora-controls-aurora"),
            aurora_i18n::tr!("ui-options-aurora-controls-classic"),
            &mut s.classic_controls,
        );
        between(ui, p);
        c |= look_choice(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-restart-alert"),
            aurora_i18n::tr!("ui-options-aurora-restart-alert-help"),
            aurora_i18n::tr!("ui-options-aurora-restart-card"),
            aurora_i18n::tr!("ui-options-aurora-restart-classic"),
            &mut s.classic_restart_alert,
        );
        between(ui, p);
        c |= look_choice(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-rez-effect"),
            aurora_i18n::tr!("ui-options-aurora-rez-effect-help"),
            aurora_i18n::tr!("ui-options-aurora-rez-effect-aurora"),
            aurora_i18n::tr!("ui-options-aurora-rez-effect-classic"),
            &mut s.classic_rez_effect,
        );
    });
    group(ui, p, aurora_i18n::tr!("ui-options-aurora-gaze"), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut legacy = s.lookat.aim == Aim::Direction;
        if look_choice(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-aim"),
            aurora_i18n::tr!("ui-options-aurora-aim-help"),
            aurora_i18n::tr!("ui-options-aurora-aim-point"),
            aurora_i18n::tr!("ui-options-aurora-aim-direction"),
            &mut legacy,
        ) {
            s.lookat.aim = if legacy { Aim::Direction } else { Aim::Point };
            c = true;
        }
    });
    group(ui, p, aurora_i18n::tr!("ui-options-aurora-movement"), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        // Firestorm's until the human has tried it on the grid, marked
        // experimental meanwhile
        let mut legacy = !s.predict_own_motion;
        if look_choice_with(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-own-motion"),
            Some(aurora_i18n::tr!("ui-options-aurora-own-motion-experimental")),
            aurora_i18n::tr!("ui-options-aurora-own-motion-help"),
            aurora_i18n::tr!("ui-options-aurora-own-motion-instant"),
            aurora_i18n::tr!("ui-options-aurora-own-motion-server"),
            &mut legacy,
        ) {
            s.predict_own_motion = !legacy;
            c = true;
        }
    });
    group(ui, p, aurora_i18n::tr!("ui-options-aurora-animations"), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        c |= animation_clock_choice(ui, p, &mut s.synchronize_animations);
        between(ui, p);
        c |= item(
            ui,
            p,
            aurora_i18n::tr!("ui-options-aurora-ground-feet"),
            aurora_i18n::tr!("ui-options-aurora-ground-feet-help"),
            widgets::SWITCH_SIZE,
            |ui| toggle(ui, p, &mut s.ground_avatar_feet),
        );
        between(ui, p);
        ui.add_enabled_ui(s.ground_avatar_feet, |ui| {
            c |= item(
                ui,
                p,
                aurora_i18n::tr!("ui-options-aurora-ground-feet-height"),
                aurora_i18n::tr!("ui-options-aurora-ground-feet-height-help"),
                Vec2::new(SLIDER_W + 70.0, 26.0),
                |ui| {
                    slider(
                        ui,
                        &mut s.ground_feet_max_height,
                        0.0..=crate::settings::MAX_GROUND_FEET_HEIGHT,
                        " m",
                    )
                },
            );
        });
    });
    group(ui, p, aurora_i18n::tr!("ui-options-aurora-avatar-plates"), |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        // the plates' settings mean nothing to the classic name tags
        ui.add_enabled_ui(!s.classic_name_tags, |ui| {
            c |= item(
                ui,
                p,
                aurora_i18n::tr!("ui-options-plates-hover-expand"),
                aurora_i18n::tr!("ui-options-plates-hover-expand-help"),
                widgets::SWITCH_SIZE,
                |ui| toggle(ui, p, &mut s.plates_hover_expand),
            );
            between(ui, p);
            c |= item(
                ui,
                p,
                aurora_i18n::tr!("ui-options-full-plates-friends-only"),
                aurora_i18n::tr!("ui-options-full-plates-friends-only-help"),
                widgets::SWITCH_SIZE,
                |ui| toggle(ui, p, &mut s.plates_friends_only_full),
            );
            between(ui, p);
            c |= item(
                ui,
                p,
                aurora_i18n::tr!("ui-options-plate-click"),
                aurora_i18n::tr!("ui-options-plate-click-help"),
                Vec2::new(PLATE_CLICK_W, 26.0),
                |ui| {
                    let mut changed = false;
                    egui::ComboBox::from_id_salt("plate-click")
                        .width(PLATE_CLICK_W)
                        .selected_text(plate_click_label(s.plate_click))
                        .show_ui(ui, |ui| {
                            for action in PlateClick::ALL {
                                changed |= ui.selectable_value(&mut s.plate_click, action, plate_click_label(action)).changed();
                            }
                        });
                    changed
                },
            );
        });
    });
    c
}

fn water_page(ui: &mut egui::Ui, p: &Palette, s: &mut Settings) -> bool {
    let mut changed = false;
    group(ui, p, aurora_i18n::tr!("water-effects-title"), |ui| {
        changed |= super::row_experimental(
            ui,
            p,
            aurora_i18n::tr!("water-effects-enabled"),
            aurora_i18n::tr!("water-effects-help"),
            |ui| toggle(ui, p, &mut s.water_effects.enabled),
        );
        ui.add_enabled_ui(s.water_effects.enabled, |ui| {
            changed |= super::row(
                ui,
                p,
                aurora_i18n::tr!("water-effects-quality"),
                aurora_i18n::tr!("water-effects-quality-help"),
                |ui| {
                    super::stepped(
                        ui,
                        p,
                        &mut s.water_effects.quality,
                        &[
                            aurora_i18n::tr!("ui-options-low"),
                            aurora_i18n::tr!("water-effects-medium"),
                            aurora_i18n::tr!("ui-options-high"),
                            aurora_i18n::tr!("ui-options-ultra"),
                        ],
                    )
                },
            );
            for (key, value) in [
                ("water-effects-waves", &mut s.water_effects.waves),
                ("water-effects-absorption", &mut s.water_effects.absorption),
                ("water-effects-fresnel", &mut s.water_effects.fresnel),
                ("water-effects-shore", &mut s.water_effects.shore_foam),
                ("water-effects-wet-ground", &mut s.water_effects.wet_ground),
                ("water-effects-contact", &mut s.water_effects.contact_foam),
                ("water-effects-crests", &mut s.water_effects.crest_foam),
                ("water-effects-interactions", &mut s.water_effects.interactions),
                ("water-effects-caustics", &mut s.water_effects.caustics),
                ("water-effects-splashes", &mut s.water_effects.splashes),
                ("water-effects-bubbles", &mut s.water_effects.bubbles),
            ] {
                changed |= super::row(ui, p, aurora_i18n::text(key), "", |ui| toggle(ui, p, value));
            }
        });
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experimental_water_is_opt_in_saved_and_reset_with_aurora() {
        for settings in [
            Settings::default(),
            serde_json::from_str("{}").expect("missing water preferences"),
            serde_json::from_str(r#"{"water_effects":{}}"#).expect("partial water preferences"),
        ] {
            assert!(!settings.water_effects.enabled);
            assert_eq!(settings.water_effects.render().flags(), 0);
        }
        for enabled in [false, true] {
            let mut settings = Settings::default();
            settings.water_effects.enabled = enabled;
            settings.water_effects.bubbles = false;
            let json = serde_json::to_string(&settings).expect("saved water preferences");
            let mut back: Settings = serde_json::from_str(&json).expect("read water preferences");
            assert_eq!(back.water_effects, settings.water_effects);
            for preset in crate::graphics_presets::Preset::ALL {
                preset.apply(&mut back);
                assert_eq!(
                    back.water_effects, settings.water_effects,
                    "graphics presets keep the explicit water choice"
                );
            }
            super::super::sections::reset(super::super::TAB_AURORA, &mut back);
            assert!(!back.water_effects.enabled, "Aurora reset restores the disabled master");
            assert_eq!(back.water_effects.render().flags(), 0);
        }
    }

    #[test]
    fn animation_timing_defaults_to_legacy_is_saved_and_resets_with_aurora() {
        let old: Settings = serde_json::from_str("{}").expect("old settings without the preference");
        assert!(!old.synchronize_animations);
        for synchronized in [true, false] {
            let s = Settings {
                synchronize_animations: synchronized,
                ..Settings::default()
            };
            let json = serde_json::to_string(&s).expect("settings serialize");
            let mut back: Settings = serde_json::from_str(&json).expect("settings read back");
            assert_eq!(back.synchronize_animations, synchronized);
            super::super::sections::reset(super::super::TAB_AURORA, &mut back);
            assert!(!back.synchronize_animations, "section reset restores Legacy");
        }
    }

    #[test]
    fn grounding_defaults_off_is_saved_and_resets_with_its_section() {
        let old: Settings = serde_json::from_str("{}").expect("older settings");
        assert!(!Settings::default().ground_avatar_feet);
        assert!(!old.ground_avatar_feet);
        assert_eq!(old.ground_feet_max_height, 0.4);
        for enabled in [false, true] {
            let settings = Settings {
                ground_avatar_feet: enabled,
                ground_feet_max_height: 1.0,
                ..Settings::default()
            };
            let mut back: Settings = serde_json::from_str(&serde_json::to_string(&settings).expect("save")).expect("load");
            assert_eq!(back.ground_avatar_feet, enabled);
            assert_eq!(back.ground_feet_max_height, 1.0);
            super::super::sections::reset(super::super::TAB_AURORA, &mut back);
            assert!(!back.ground_avatar_feet);
            assert_eq!(back.ground_feet_max_height, 0.4);
        }
    }

    #[test]
    fn animation_timing_buttons_change_the_saved_preference() {
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        let p = crate::theme::Theme::default().palette();
        let mut s = Settings::default();
        let mut texts = Vec::new();
        let draw = |ui: &mut egui::Ui, s: &mut Settings| {
            ui.set_width(620.0);
            animation_clock_choice(ui, &p, &mut s.synchronize_animations)
        };
        for _ in 0..2 {
            texts = frames.run(Vec::new(), |ui| {
                draw(ui, &mut s);
            });
        }
        for (key, synchronized) in [
            ("ui-options-aurora-animation-clock-synchronized", true),
            ("ui-options-aurora-animation-clock-legacy", false),
        ] {
            let at = texts
                .iter()
                .find(|t| t.source == aurora_i18n::text(key))
                .expect("the segment is visible")
                .rect
                .center();
            let mut changed = false;
            for event in [
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ] {
                texts = frames.run(vec![event], |ui| {
                    changed |= draw(ui, &mut s);
                });
            }
            assert!(changed, "{key} reports a change for saving");
            assert_eq!(s.synchronize_animations, synchronized);
        }
    }

    #[test]
    fn the_two_buttons_set_every_look_and_leave_the_plates_alone() {
        let mut s = Settings {
            plates_friends_only_full: true,
            ..Settings::default()
        };
        set_all(&mut s, Look::Firestorm);
        assert_eq!(s.chat_style, ChatStyle::Legacy);
        assert!(s.classic_name_tags);
        assert_eq!(s.lookat.style, Style::Legacy);
        assert_eq!(s.lookat.aim, Aim::Direction);
        assert_eq!(s.notifications_style, NotificationsStyle::Legacy);
        assert_eq!(s.world_map_style, WorldMapStyle::Legacy);
        assert!(s.classic_bars, "the full-width bars come back");
        assert!(s.classic_people, "People › Near you shows the radar table");
        assert!(s.classic_profiles);
        assert_eq!(s.photo_window, PhotoWindowStyle::Legacy);
        assert!(s.classic_about_land, "About Land opens Firestorm's nine tabs");
        assert!(s.classic_search, "Search opens the Legacy Viewer's tabs");
        assert!(s.classic_region_estate);
        assert!(s.classic_inventory, "the Legacy Viewer's single inventory tree");
        assert!(s.classic_places, "Places opens the Legacy Viewer's three tabs");
        assert!(s.classic_pay_window);
        assert!(s.classic_controls, "Firestorm's two control windows");
        assert!(s.classic_restart_alert, "Firestorm's yellow restart box");
        assert!(s.classic_chat_lines, "the Legacy Viewer's one-line chat lines");
        assert!(s.classic_rez_effect, "grey objects while they load");
        assert!(!s.predict_own_motion, "the avatar waits for the simulator");
        assert!(!s.synchronize_animations);
        assert!(!s.water_effects.enabled);
        assert!(s.plates_friends_only_full, "a plate setting is not a look");
        set_all(&mut s, Look::Aurora);
        let fresh = Settings::default();
        assert_eq!(s.chat_style, fresh.chat_style);
        assert_eq!(s.classic_name_tags, fresh.classic_name_tags);
        assert_eq!(s.lookat.style, fresh.lookat.style);
        assert_eq!(s.lookat.aim, fresh.lookat.aim);
        assert_eq!(s.notifications_style, NotificationsStyle::Aurora);
        assert_eq!(fresh.notifications_style, NotificationsStyle::Aurora);
        assert_eq!(s.world_map_style, fresh.world_map_style);
        assert!(!s.classic_bars && !fresh.classic_bars, "the islands are the default");
        assert!(
            !s.classic_people && !fresh.classic_people,
            "the rows grouped by range are the default"
        );
        assert_eq!(s.classic_profiles, fresh.classic_profiles);
        assert_eq!(s.photo_window, PhotoWindowStyle::Aurora);
        assert_eq!(fresh.photo_window, PhotoWindowStyle::Aurora);
        assert_eq!(s.classic_about_land, fresh.classic_about_land);
        assert!(!fresh.classic_about_land, "the Aurora About Land is the default");
        assert!(!s.classic_search && !fresh.classic_search, "the Aurora Search is the default");
        assert!(!s.classic_region_estate && !fresh.classic_region_estate);
        assert!(
            !s.classic_inventory && !fresh.classic_inventory,
            "the Aurora inventory is the default"
        );
        assert!(!s.classic_places && !fresh.classic_places, "Aurora's Places is the default");
        assert!(!s.classic_pay_window && !fresh.classic_pay_window);
        assert!(!s.classic_controls && !fresh.classic_controls);
        assert!(!s.classic_restart_alert && !fresh.classic_restart_alert);
        assert!(!s.classic_chat_lines && !fresh.classic_chat_lines, "the cards are the default");
        assert!(!s.classic_rez_effect && !fresh.classic_rez_effect, "the rez effect is the default");
        assert!(s.predict_own_motion, "Aurora's movement");
        assert!(!fresh.predict_own_motion, "Firestorm's movement until tried on the grid");
        assert!(s.synchronize_animations, "the Aurora bulk choice enables receipt clocks");
        assert!(s.water_effects.enabled, "the explicit Aurora bulk choice enables water effects");
        assert!(!fresh.water_effects.enabled, "enhanced water is disabled by default");
        assert!(!fresh.synchronize_animations, "Legacy animation timing is the default");
        assert!(s.plates_friends_only_full);
    }

    /// The notifications' style is saved; a file from before it was added
    /// gets the animated bell.
    #[test]
    fn the_notifications_style_is_saved() {
        let s = Settings {
            notifications_style: NotificationsStyle::Legacy,
            ..Settings::default()
        };
        let json = serde_json::to_string(&s).expect("settings serialize");
        let back: Settings = serde_json::from_str(&json).expect("settings read back");
        assert_eq!(back.notifications_style, NotificationsStyle::Legacy);
        let old: Settings = serde_json::from_str("{}").expect("empty settings");
        assert_eq!(old.notifications_style, NotificationsStyle::Aurora);
    }

    #[test]
    fn the_aurora_gradient_runs_from_green_to_rose() {
        let p = crate::theme::Theme::default().palette();
        assert_eq!(aurora_color(&p, 0.0), p.success);
        assert_eq!(aurora_color(&p, 1.0), p.rose);
        assert_eq!(aurora_color(&p, 0.5), p.indigo_light);
    }

    /// The texts of one frame of the page, top to bottom.
    fn page_texts(settings: &mut Settings) -> Vec<String> {
        let p = crate::theme::Theme::default().palette();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(620.0, 2400.0))),
                ..Default::default()
            },
            |ui| {
                page(ui, &p, settings, &mut false);
            },
        );
        output.textures_delta.clear();
        fn collect(shape: &Shape, out: &mut Vec<(f32, String)>) {
            match shape {
                Shape::Text(t) => out.push((t.pos.y, t.galley.text().to_owned())),
                Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        let mut texts = Vec::new();
        output.shapes.iter().for_each(|s| collect(&s.shape, &mut texts));
        texts.sort_by(|a, b| a.0.total_cmp(&b.0));
        texts.into_iter().map(|(_, t)| t).collect()
    }

    /// The groups and settings of the mockup, in its order.
    #[test]
    fn the_page_shows_the_mockup_in_its_order() {
        let mut settings = Settings::default();
        let texts = page_texts(&mut settings);
        let order = [
            "ui-options-aurora",
            "ui-options-aurora-intro",
            "water-effects-title",
            "water-effects-enabled",
            "water-effects-quality",
            "water-effects-waves",
            "water-effects-absorption",
            "water-effects-fresnel",
            "water-effects-shore",
            "water-effects-wet-ground",
            "water-effects-contact",
            "water-effects-crests",
            "water-effects-interactions",
            "water-effects-caustics",
            "water-effects-splashes",
            "water-effects-bubbles",
            "ui-options-aurora-appearance",
            "ui-options-aurora-conversations",
            "ui-options-aurora-name-tags",
            "ui-options-aurora-look-at",
            "ui-options-aurora-notifications",
            "ui-options-aurora-bars",
            "ui-options-aurora-world-map",
            "ui-options-aurora-people",
            "ui-options-aurora-profiles",
            "ui-options-aurora-about-land",
            "ui-options-aurora-pay-window",
            "ui-options-aurora-controls",
            "ui-options-aurora-restart-alert",
            "ui-options-aurora-gaze",
            "ui-options-aurora-aim",
            "ui-options-aurora-movement",
            "ui-options-aurora-own-motion",
            "ui-options-aurora-animations",
            "ui-options-aurora-animation-clock",
            "ui-options-aurora-avatar-plates",
            "ui-options-plates-hover-expand",
            "ui-options-full-plates-friends-only",
            "ui-options-plate-click",
        ]
        .map(aurora_i18n::text);
        let mut at = 0;
        for want in order {
            let found = texts[at..].iter().position(|t| *t == want);
            assert!(found.is_some(), "{want:?} comes after the previous one in {texts:?}");
            at += found.unwrap_or(0) + 1;
        }
        // the choices start on Aurora's look
        for choice in [
            "ui-options-aurora-bubbles",
            "ui-options-aurora-plates",
            "ui-options-aurora-halo",
            "ui-options-aurora-world-map-full",
            "ui-options-aurora-people-rows",
            "ui-options-aurora-profiles-aurora",
            "ui-options-aurora-about-land-aurora",
            "ui-options-aurora-aim-point",
            "ui-options-aurora-animated-bell",
            "ui-options-aurora-bars-islands",
        ] {
            assert!(texts.iter().any(|t| t == aurora_i18n::text(choice)), "{choice}");
        }
    }

    /// Each moved setting has one place: the Aurora page, not Interface.
    #[test]
    fn the_moved_settings_are_only_on_the_aurora_page() {
        let options = include_str!("../options.rs");
        let aurora = include_str!("aurora.rs");
        for (key, field) in [
            ("ui-options-full-plates-friends-only\"", "&mut s.plates_friends_only_full"),
            ("ui-options-plates-hover-expand\"", "&mut s.plates_hover_expand"),
            ("ui-options-plate-click\"", "&mut s.plate_click"),
            ("ui-options-aurora-name-tags\"", "&mut s.classic_name_tags"),
        ] {
            assert!(aurora.contains(key) && aurora.contains(field), "{key} is on the Aurora page");
            assert!(!options.contains(key), "{key} is no longer on another page");
            assert!(!options.contains(field), "{field} is set only on the Aurora page");
        }
        assert!(!options.contains("name-tag-style"), "the former name tag style row is gone");
    }

    /// The Conversations choice of the page switches the setting, through
    /// real egui frames: each of its four segments (the window follows the
    /// setting every frame, `ChatUi::set_style`).
    #[test]
    fn the_conversations_choice_switches_the_style() {
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        let p = crate::theme::Theme::default().palette();
        let mut s = Settings::default();
        let mut reset = false;
        let mut texts = Vec::new();
        for _ in 0..2 {
            texts = frames.run(Vec::new(), |ui| {
                page(ui, &p, &mut s, &mut reset);
            });
        }
        // the row of the segments ("Aurora" is also the page's title)
        let row = texts
            .iter()
            .find(|t| t.source == aurora_i18n::tr!("ui-options-aurora-compact"))
            .expect("the Compact segment")
            .rect
            .center()
            .y;
        for (label, style) in [
            (aurora_i18n::tr!("ui-options-aurora-compact"), ChatStyle::Compact),
            (aurora_i18n::tr!("ui-options-aurora-bubbles"), ChatStyle::Bubbles),
            (aurora_i18n::tr!("ui-options-aurora-lines"), ChatStyle::Legacy),
            (aurora_i18n::tr!("ui-options-aurora-workspace"), ChatStyle::Aurora),
        ] {
            let at = texts
                .iter()
                .find(|t| t.source == label && (t.rect.center().y - row).abs() < 4.0)
                .unwrap_or_else(|| panic!("{label} is painted"))
                .rect
                .center();
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            for events in [
                vec![egui::Event::PointerMoved(at)],
                vec![button(true)],
                vec![button(false)],
                Vec::new(),
            ] {
                texts = frames.run(events, |ui| {
                    page(ui, &p, &mut s, &mut reset);
                });
            }
            assert_eq!(s.chat_style, style, "{label}");
        }
    }
}

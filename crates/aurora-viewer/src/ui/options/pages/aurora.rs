//! Preferences › Aurora: what Aurora does differently from the Legacy
//! Viewer, and for each redrawn element the way back to the Legacy look.
//! The entry of the sidebar carries the aurora gradient, the one exception
//! to the theme here (docs/BRANDING.md); the settings keep the rows of the
//! other pages.

use super::super::cx::{Cx, F, field};
use crate::settings::{ChatStyle, NotificationsStyle, PhotoWindowStyle, Settings, WorldMapStyle};
use crate::theme::Palette;
use crate::world::lookat::{Aim, Style};
use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, FontId, Pos2, Rect, Sense, Shape, Vec2};

/// The two looks a redrawn element can take.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Look {
    Aurora,
    Legacy,
}

/// Every choice of the page set to one look (the colored sky ambient
/// included), and Aurora's water with it;
/// the avatar plates' own settings and the filing of received items are
/// left as they are.
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
    s.build.legacy_window = !aurora;
    s.classic_places = !aurora;
    s.classic_pay_window = !aurora;
    s.legacy_money_tracker = !aurora;
    s.classic_controls = !aurora;
    s.classic_restart_alert = !aurora;
    s.classic_chat_lines = !aurora;
    s.classic_rez_effect = !aurora;
    s.classic_hover_tips = !aurora;
    s.predict_own_motion = aurora;
    s.water_effects.enabled = aurora;
    s.synchronize_animations = aurora;
    s.colored_sky_ambient = aurora;
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

/// A strip of the aurora gradient, from its top to its bottom.
fn aurora_strip(p: &Palette, r: Rect) -> Shape {
    const STEPS: usize = 16;
    let mut m = Mesh::default();
    for i in 0..=STEPS {
        let t = i as f32 / STEPS as f32;
        let c = aurora_color(p, t);
        let y = r.top() + r.height() * t;
        m.vertices.push(vertex(Pos2::new(r.left(), y), c));
        m.vertices.push(vertex(Pos2::new(r.right(), y), c));
        if i > 0 {
            let k = (i as u32) * 2;
            m.add_triangle(k - 2, k - 1, k);
            m.add_triangle(k - 1, k + 1, k);
        }
    }
    Shape::mesh(m)
}

/// The entry of the page in the sidebar: the neighbours' entry, with the
/// aurora gradient in its edge mark and its name.
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
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    response.on_hover_text(label)
}

/// A choice between Aurora's look (first, in the accent) and the Legacy
/// one, over a "classic" switch.
fn look(cx: &mut Cx, name: &str, help: &str, aurora: &str, legacy: &str, f: &F<bool>) {
    cx.wide().choice(name, help, f, &[(false, aurora, true), (true, legacy, false)]);
}

/// An experimental choice: its warning joins its help.
fn warned(help: &str, warning: &str) -> String {
    format!("{help}\n{warning}")
}

pub fn page(cx: &mut Cx, tab: usize) {
    use aurora_i18n::tr;
    let aurora = tr!("ui-options-aurora-about-land-aurora");
    let classic = tr!("ui-options-aurora-about-land-classic");
    match tab {
        0 => cx.group(tr!("ui-options-aurora-redrawn-windows"), |cx| {
            cx.wide().choice(
                tr!("ui-options-aurora-conversations"),
                tr!("ui-options-aurora-conversations-help"),
                &field!(chat_style),
                &[
                    (ChatStyle::Aurora, tr!("ui-options-aurora-workspace"), true),
                    (ChatStyle::Compact, tr!("ui-options-aurora-compact"), true),
                    (ChatStyle::Bubbles, tr!("ui-options-aurora-bubbles"), true),
                    (ChatStyle::Legacy, tr!("ui-options-aurora-lines"), false),
                ],
            );
            look(
                cx,
                tr!("ui-options-aurora-chat-lines"),
                tr!("ui-options-aurora-chat-lines-help"),
                tr!("ui-options-aurora-chat-lines-cards"),
                tr!("ui-options-aurora-chat-lines-classic"),
                &field!(classic_chat_lines),
            );
            cx.wide().choice(
                tr!("ui-options-aurora-world-map"),
                tr!("ui-options-aurora-world-map-help"),
                &field!(world_map_style),
                &[
                    (WorldMapStyle::Aurora, tr!("ui-options-aurora-world-map-full"), true),
                    (WorldMapStyle::Legacy, tr!("ui-options-aurora-world-map-side"), false),
                ],
            );
            look(
                cx,
                tr!("ui-options-aurora-places"),
                tr!("ui-options-aurora-places-help"),
                tr!("ui-options-aurora-places-aurora"),
                tr!("ui-options-aurora-places-classic"),
                &field!(classic_places),
            );
            look(
                cx,
                tr!("ui-options-aurora-people"),
                tr!("ui-options-aurora-people-help"),
                tr!("ui-options-aurora-people-rows"),
                tr!("ui-options-aurora-people-table"),
                &field!(classic_people),
            );
            look(
                cx,
                tr!("ui-options-aurora-profiles"),
                tr!("ui-options-aurora-profiles-help"),
                tr!("ui-options-aurora-profiles-aurora"),
                tr!("ui-options-aurora-profiles-classic"),
                &field!(classic_profiles),
            );
            let classic_profiles = cx.s.classic_profiles;
            cx.subs(classic_profiles, |cx| {
                cx.toggle(
                    tr!("ui-options-aurora-profiles-rights"),
                    tr!("ui-options-aurora-profiles-rights-help"),
                    &field!(profile_rights),
                );
            });
            cx.wide().choice(
                tr!("ui-options-aurora-photo-window"),
                tr!("ui-options-aurora-photo-window-help"),
                &field!(photo_window),
                &[
                    (PhotoWindowStyle::Aurora, tr!("ui-options-aurora-photo-viewfinder"), true),
                    (PhotoWindowStyle::Panel, tr!("ui-options-aurora-photo-panel"), true),
                    (PhotoWindowStyle::Legacy, tr!("ui-options-aurora-photo-classic"), false),
                ],
            );
            let viewfinder = cx.s.photo_window == PhotoWindowStyle::Aurora;
            cx.subs(!viewfinder, |cx| {
                use crate::snapshot::viewfinder::OnScreen;
                let labels = [
                    (tr!("ui-options-aurora-vf-controls-always"), true),
                    (tr!("ui-options-aurora-vf-controls-hover"), true),
                    (tr!("ui-options-aurora-vf-controls-hidden"), false),
                ];
                let choices: Vec<(OnScreen, &str, bool)> = OnScreen::ALL.iter().zip(labels).map(|(o, (l, a))| (*o, l, a)).collect();
                cx.wide().choice(
                    tr!("ui-options-aurora-vf-controls"),
                    tr!("ui-options-aurora-vf-controls-help"),
                    &field!(snapshot.viewfinder.controls),
                    &choices,
                );
            });
            look(
                cx,
                tr!("ui-options-aurora-about-land"),
                tr!("ui-options-aurora-about-land-help"),
                aurora,
                classic,
                &field!(classic_about_land),
            );
            look(
                cx,
                tr!("search-title"),
                tr!("ui-options-aurora-search-help"),
                aurora,
                tr!("ui-options-aurora-search-classic"),
                &field!(classic_search),
            );
            look(
                cx,
                tr!("region-title"),
                tr!("ui-options-aurora-region-estate-help"),
                aurora,
                classic,
                &field!(classic_region_estate),
            );
            look(
                cx,
                tr!("common-inventory"),
                tr!("ui-options-aurora-inventory-help"),
                aurora,
                classic,
                &field!(classic_inventory),
            );
            look(
                cx,
                tr!("ui-options-aurora-build"),
                tr!("ui-options-aurora-build-help"),
                aurora,
                classic,
                &field!(build.legacy_window),
            );
            look(
                cx,
                tr!("ui-options-aurora-pay-window"),
                tr!("ui-options-aurora-pay-window-help"),
                tr!("ui-options-aurora-pay-aurora"),
                tr!("ui-options-aurora-pay-classic"),
                &field!(classic_pay_window),
            );
            look(
                cx,
                tr!("money-tracker-title"),
                tr!("ui-options-aurora-money-tracker-help"),
                aurora,
                classic,
                &field!(legacy_money_tracker),
            );
            look(
                cx,
                tr!("ui-options-aurora-controls"),
                tr!("ui-options-aurora-controls-help"),
                tr!("ui-options-aurora-controls-aurora"),
                tr!("ui-options-aurora-controls-classic"),
                &field!(classic_controls),
            );
        }),
        1 => {
            cx.group(tr!("ui-options-aurora-in-the-world"), |cx| {
                look(
                    cx,
                    tr!("ui-options-aurora-name-tags"),
                    tr!("ui-options-aurora-name-tags-help"),
                    tr!("ui-options-aurora-plates"),
                    tr!("ui-options-aurora-classic"),
                    &field!(classic_name_tags),
                );
                cx.wide().choice(
                    tr!("ui-options-aurora-look-at"),
                    tr!("ui-options-aurora-look-at-help"),
                    &field!(lookat.style),
                    &[
                        (Style::Aurora, tr!("ui-options-aurora-halo"), true),
                        (Style::Legacy, tr!("ui-options-aurora-crosshair"), false),
                    ],
                );
                look(
                    cx,
                    tr!("ui-options-aurora-rez-effect"),
                    tr!("ui-options-aurora-rez-effect-help"),
                    tr!("ui-options-aurora-rez-effect-aurora"),
                    tr!("ui-options-aurora-rez-effect-classic"),
                    &field!(classic_rez_effect),
                );
                look(
                    cx,
                    tr!("ui-options-aurora-hover-tips"),
                    tr!("ui-options-aurora-hover-tips-help"),
                    aurora,
                    tr!("ui-options-aurora-hover-tips-legacy"),
                    &field!(classic_hover_tips),
                );
                // also in Graphics › Sky and environment, the same setting
                cx.wide().experimental().choice(
                    tr!("ui-options-colored-sky-ambient"),
                    tr!("ui-options-colored-sky-ambient-help"),
                    &field!(colored_sky_ambient),
                    &[(true, aurora, true), (false, tr!("ui-options-aurora-classic"), false)],
                );
            });
            cx.group(tr!("common-interface"), |cx| {
                cx.wide().choice(
                    tr!("ui-options-aurora-notifications"),
                    tr!("ui-options-aurora-notifications-help"),
                    &field!(notifications_style),
                    &[
                        (NotificationsStyle::Aurora, tr!("ui-options-aurora-animated-bell"), true),
                        (NotificationsStyle::Legacy, tr!("ui-options-aurora-toasts"), false),
                    ],
                );
                look(
                    cx,
                    tr!("ui-options-aurora-bars"),
                    tr!("ui-options-aurora-bars-help"),
                    tr!("ui-options-aurora-bars-islands"),
                    tr!("ui-options-aurora-bars-classic"),
                    &field!(classic_bars),
                );
                look(
                    cx,
                    tr!("ui-options-aurora-restart-alert"),
                    tr!("ui-options-aurora-restart-alert-help"),
                    tr!("ui-options-aurora-restart-card"),
                    tr!("ui-options-aurora-restart-classic"),
                    &field!(classic_restart_alert),
                );
            });
        }
        _ => {
            cx.group(tr!("ui-options-aurora-gaze"), |cx| {
                cx.wide().choice(
                    tr!("ui-options-aurora-aim"),
                    tr!("ui-options-aurora-aim-help"),
                    &field!(lookat.aim),
                    &[
                        (Aim::Point, tr!("ui-options-aurora-aim-point"), true),
                        (Aim::Direction, tr!("ui-options-aurora-aim-direction"), false),
                    ],
                );
            });
            // the Legacy Viewer's until the human has tried it on the grid,
            // marked experimental meanwhile
            cx.group(tr!("ui-options-aurora-movement"), |cx| {
                let help = warned(
                    tr!("ui-options-aurora-own-motion-help"),
                    tr!("ui-options-aurora-own-motion-experimental"),
                );
                cx.wide().experimental().choice(
                    tr!("ui-options-aurora-own-motion"),
                    &help,
                    &field!(predict_own_motion),
                    &[
                        (true, tr!("ui-options-aurora-own-motion-instant"), true),
                        (false, tr!("ui-options-aurora-own-motion-server"), false),
                    ],
                );
            });
            cx.group(tr!("ui-options-aurora-animations"), |cx| {
                let help = warned(
                    tr!("ui-options-aurora-animation-clock-help"),
                    tr!("ui-options-aurora-animation-clock-experimental"),
                );
                cx.wide().experimental().choice(
                    tr!("ui-options-aurora-animation-clock"),
                    &help,
                    &field!(synchronize_animations),
                    &[
                        (true, tr!("ui-options-aurora-animation-clock-synchronized"), true),
                        (false, tr!("ui-options-aurora-animation-clock-legacy"), false),
                    ],
                );
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn received_filing_stays_opt_in_when_the_looks_change() {
        let mut settings = Settings::default();
        set_all(&mut settings, Look::Aurora);
        assert!(!settings.sort_received_items);
        settings.sort_received_items = true;
        set_all(&mut settings, Look::Legacy);
        assert!(settings.sort_received_items);
    }

    #[test]
    fn the_two_buttons_set_the_colored_sky_ambient_too() {
        let mut s = Settings::default();
        assert!(!s.colored_sky_ambient, "off by default");
        set_all(&mut s, Look::Aurora);
        assert!(s.colored_sky_ambient);
        set_all(&mut s, Look::Legacy);
        assert!(!s.colored_sky_ambient);
    }

    #[test]
    fn the_two_buttons_set_every_look_and_leave_the_plates_alone() {
        let mut s = Settings {
            plates_friends_only_full: true,
            ..Settings::default()
        };
        set_all(&mut s, Look::Legacy);
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
        assert!(s.classic_about_land, "About Land opens the Legacy Viewer's nine tabs");
        assert!(s.classic_search, "Search opens the Legacy Viewer's tabs");
        assert!(s.classic_region_estate);
        assert!(s.classic_inventory, "the Legacy Viewer's single inventory tree");
        assert!(s.build.legacy_window, "the Legacy Viewer's build tools and tabs");
        assert!(s.classic_places, "Places opens the Legacy Viewer's three tabs");
        assert!(s.classic_pay_window);
        assert!(s.legacy_money_tracker, "the Legacy Viewer's Time / Name / Amount list");
        assert!(s.classic_controls, "the Legacy Viewer's two control windows");
        assert!(s.classic_restart_alert, "the Legacy Viewer's yellow restart box");
        assert!(s.classic_chat_lines, "the Legacy Viewer's one-line chat lines");
        assert!(s.classic_rez_effect, "grey objects while they load");
        assert!(s.classic_hover_tips, "the Legacy Viewer's text tooltips");
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
        assert!(!s.build.legacy_window && !fresh.build.legacy_window);
        assert!(!s.classic_pay_window && !fresh.classic_pay_window);
        assert!(!s.legacy_money_tracker && !fresh.legacy_money_tracker);
        assert!(!s.classic_controls && !fresh.classic_controls);
        assert!(!s.classic_restart_alert && !fresh.classic_restart_alert);
        assert!(!s.classic_chat_lines && !fresh.classic_chat_lines, "the cards are the default");
        assert!(!s.classic_rez_effect && !fresh.classic_rez_effect, "the rez effect is the default");
        assert!(!s.classic_hover_tips && !fresh.classic_hover_tips, "Aurora's tips are the default");
        assert!(s.predict_own_motion, "Aurora's movement");
        assert!(!fresh.predict_own_motion, "the Legacy movement until tried on the grid");
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
}

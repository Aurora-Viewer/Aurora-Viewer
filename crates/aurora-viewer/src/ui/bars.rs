//! Firestorm-like chrome: menu/status bar and navigation bar at the top,
//! chat input + feature buttons at the bottom. Flat, skinnable via
//! `skin::Layout` and the theme palette.

use super::Panels;
use super::icons::Icons;
use super::skin::Layout;
use crate::theme::Palette;
use egui::{Color32, RichText, Vec2};

#[derive(Default)]
pub enum BarAction {
    #[default]
    None,
    /// ShowBanLines: 0 hidden, 1 on collision, 2 on proximity.
    BanLines(u8),
    /// Build floater: 0 toggle, 1 edit, 2 create, 3 land.
    Build(u8),
    Logout,
    Quit,
    TeleportHome,
    ToggleFly,
    ToggleMouselook,
    SitGround,
    StandUp,
    ResetCamera,
    /// Teleport history (navigation bar arrows).
    TeleportBack,
    TeleportForward,
    /// Voice light clicked: open "Son et voix".
    VoicePrefs,
    /// Location typed or pasted in the navigation bar (SLURL, Region/x/y/z...).
    TeleportTo(String),
    /// Communiquer > Statut de connexion changed.
    SetStatus(crate::world::status::StatusModes),
    /// Open the chat preferences (automatic responses).
    ChatPrefs,
    /// Moi > Profil.
    MyProfile,
    /// Monde › Environnement › Utiliser les environnements partagés
    /// (setSharedEnvironment: drops every local choice).
    SharedEnvironment,
    /// Monde › Historique de téléportation (ToggleTeleportHistory, Alt+H).
    TeleportHistory,
}

pub struct StatusInfo<'a> {
    pub region: &'a str,
    /// Parcel the agent stands on (may be empty).
    pub parcel: &'a str,
    /// Destinations of the back / forward arrows (None = disabled).
    pub tp_back: Option<String>,
    pub tp_forward: Option<String>,
    pub maturity: &'a str,
    pub position: glam::Vec3,
    pub fps: f32,
    pub flying: bool,
    pub seated: bool,
    pub ping_ms: u32,
    pub balance: Option<i32>,
    /// Voice connection light and its explanation.
    pub voice: (crate::voice::VoiceLight, String),
    /// SLURL of the current position (shown when the location is edited).
    pub slurl: String,
    /// Online status modes (away, do not disturb, autoresponse...).
    pub status: crate::world::status::StatusModes,
    /// ShowBanLines mode (Monde › Lignes d'interdiction).
    pub ban_lines: u8,
    /// Abilities of the current parcel (right end of the location field).
    pub parcel_icons: Vec<super::parcel_icons::ParcelIcon>,
    /// A local environment is shown (selector, preset, personal lighting).
    pub local_env: bool,
}

pub fn maturity_name(sim_access: u8) -> &'static str {
    match sim_access {
        13 => "Général",
        21 => "Modéré",
        42 => "Adulte",
        _ => "",
    }
}

fn slt_time() -> String {
    // SLT is US Pacific time; DST approximated by day of year (mid-March .. early Nov).
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let year_day = (secs / 86400).rem_euclid(365);
    let off = if (68..=307).contains(&year_day) { -7 } else { -8 };
    let t = (secs + off * 3600).rem_euclid(86400);
    format!("{:02}:{:02}:{:02} SLT", t / 3600, (t / 60) % 60, t % 60)
}

fn bar_frame(p: &Palette, alpha: u8) -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(p.bar.r(), p.bar.g(), p.bar.b(), alpha))
        .inner_margin(egui::Margin::symmetric(6, 0))
}

fn icon_button(ui: &mut egui::Ui, p: &Palette, icons: &Icons, icon: &str, tip: &str, size: f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(size + 6.0, size + 4.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 2.0, p.raised);
    }
    if let Some(t) = icons.get(icon) {
        let tint = if resp.hovered() { p.ink } else { p.muted };
        let r = egui::Rect::from_center_size(rect.center(), Vec2::splat(size));
        ui.painter().image(
            t.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    resp.on_hover_text(tip).clicked()
}

/// Local time of day choices (index = `Panels::time_of_day`).
pub const TIME_OF_DAY: [&str; 5] = [
    "Environnements partagés (région)",
    "Lever du soleil",
    "Midi",
    "Coucher du soleil",
    "Minuit",
];

/// Color of the online status: violet away, red do not disturb, orange
/// automatic response, green available.
fn status_color(p: &Palette, m: &crate::world::status::StatusModes) -> Color32 {
    if m.dnd {
        p.danger
    } else if m.away {
        p.violet
    } else if m.autorespond || m.autorespond_nonfriends {
        p.amber
    } else {
        p.success
    }
}

fn status_name(m: &crate::world::status::StatusModes) -> &'static str {
    if m.dnd {
        "Ne pas déranger"
    } else if m.away {
        "Absent"
    } else if m.autorespond {
        "Réponse automatique"
    } else if m.autorespond_nonfriends {
        "Réponse automatique aux non-amis"
    } else {
        "Disponible"
    }
}

/// Entries of Communiquer > Statut de connexion (Firestorm's menu).
fn status_menu(ui: &mut egui::Ui, status: crate::world::status::StatusModes, action: &mut BarAction) {
    let mut m = status;
    let mut c = false;
    c |= ui.checkbox(&mut m.away, "Absent").changed();
    c |= ui.checkbox(&mut m.dnd, "Ne pas déranger").changed();
    c |= ui.checkbox(&mut m.autorespond, "Réponse automatique").changed();
    c |= ui
        .checkbox(&mut m.autorespond_nonfriends, "Réponse automatique aux non-amis")
        .changed();
    ui.separator();
    c |= ui
        .checkbox(&mut m.reject_teleports, "Rejeter les offres et demandes de téléportation")
        .changed();
    c |= ui
        .checkbox(&mut m.reject_group_invites, "Rejeter toutes les invitations de groupe")
        .changed();
    c |= ui
        .checkbox(&mut m.reject_friendship, "Rejeter toutes les demandes d'amitié")
        .changed();
    ui.menu_button("Conférences ad hoc", |ui| {
        c |= ui
            .checkbox(&mut m.ignore_adhoc, "Tout ignorer et quitter automatiquement")
            .changed();
        ui.add_enabled_ui(m.ignore_adhoc, |ui| {
            c |= ui
                .checkbox(&mut m.report_ignored_adhoc, "Signaler les ignorés dans le chat local")
                .changed();
            c |= ui.checkbox(&mut m.adhoc_from_friends, "Ne pas ignorer mes amis").changed();
        });
    });
    ui.separator();
    if ui.button("Textes des réponses automatiques…").clicked() {
        *action = BarAction::ChatPrefs;
        ui.close();
        return;
    }
    if c {
        *action = BarAction::SetStatus(m);
    }
}

/// Status dot next to the bell: its color tells the status, a click opens
/// the status choices.
fn status_dot(ui: &mut egui::Ui, p: &Palette, status: crate::world::status::StatusModes, action: &mut BarAction) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(20.0, 20.0), egui::Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    if resp.hovered() || open {
        ui.painter().rect_filled(rect, 2.0, p.raised);
    }
    let col = status_color(p, &status);
    ui.painter().circle_filled(rect.center(), 5.0, col);
    ui.painter()
        .circle_stroke(rect.center(), 5.0, egui::Stroke::new(1.0, col.gamma_multiply(0.55)));
    // refusals on: a thin ring around the dot
    if status.reject_teleports || status.reject_group_invites || status.reject_friendship || status.ignore_adhoc {
        ui.painter().circle_stroke(rect.center(), 7.5, egui::Stroke::new(1.0, p.muted));
    }
    let mut tip = format!("Statut : {}", status_name(&status));
    for (on, what) in [
        (status.reject_teleports, "téléportations rejetées"),
        (status.reject_group_invites, "invitations de groupe rejetées"),
        (status.reject_friendship, "demandes d'amitié rejetées"),
        (status.ignore_adhoc, "conférences ad hoc ignorées"),
    ] {
        if on {
            tip.push_str(&format!("\n• {what}"));
        }
    }
    let resp = resp.on_hover_text(tip);
    // demo captures: AURORA_DEMO_STATUSMENU=1 opens it
    static DEMO_OPEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *DEMO_OPEN.get_or_init(|| std::env::var_os("AURORA_DEMO_STATUSMENU").is_some()) && ui.ctx().cumulative_frame_nr() == 300 {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&resp));
    }
    egui::Popup::menu(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(250.0);
            // availability: one of three, like a presence menu
            let mut m = status;
            let current = if m.dnd {
                2
            } else if m.away {
                1
            } else {
                0
            };
            for (i, label, col) in [
                (0, "Disponible", p.success),
                (1, "Absent", p.violet),
                (2, "Ne pas déranger", p.danger),
            ] {
                let r = ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(Vec2::new(14.0, 16.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.5, col);
                    ui.add(egui::Button::selectable(current == i, label))
                });
                if r.inner.clicked() && current != i {
                    m.away = i == 1;
                    m.dnd = i == 2;
                    *action = BarAction::SetStatus(m);
                }
            }
            ui.separator();
            let mut sub = BarAction::None;
            // automatic responses and refusals: the same entries as the menu
            let mut rest = status;
            rest.away = m.away;
            rest.dnd = m.dnd;
            let mut c = false;
            c |= ui.checkbox(&mut rest.autorespond, "Réponse automatique").changed();
            c |= ui
                .checkbox(&mut rest.autorespond_nonfriends, "Réponse automatique aux non-amis")
                .changed();
            ui.separator();
            c |= ui.checkbox(&mut rest.reject_teleports, "Rejeter les téléportations").changed();
            c |= ui
                .checkbox(&mut rest.reject_group_invites, "Rejeter les invitations de groupe")
                .changed();
            c |= ui.checkbox(&mut rest.reject_friendship, "Rejeter les demandes d'amitié").changed();
            c |= ui.checkbox(&mut rest.ignore_adhoc, "Ignorer les conférences ad hoc").changed();
            if c {
                sub = BarAction::SetStatus(rest);
            }
            ui.separator();
            if ui.button("Textes des réponses automatiques…").clicked() {
                sub = BarAction::ChatPrefs;
                ui.close();
            }
            if !matches!(sub, BarAction::None) {
                *action = sub;
            }
        });
}

/// Viewer menus: in the top bar, and as submenus of the wolf menu button
/// of the navigation bar (without "Avancé").
fn main_menus(ui: &mut egui::Ui, p: &Palette, panels: &mut Panels, st: &StatusInfo, action: &mut BarAction, advanced: bool) {
    ui.menu_button(small("Moi", p.ink), |ui| {
        ui.checkbox(&mut panels.appearance, "Apparence…");
        if ui.button("Profil…").clicked() {
            *action = BarAction::MyProfile;
            ui.close();
        }
        ui.separator();
        if ui.button("Rentrer chez moi").clicked() {
            *action = BarAction::TeleportHome;
            ui.close();
        }
        ui.separator();
        if ui.button("S'asseoir par terre").clicked() {
            *action = BarAction::SitGround;
            ui.close();
        }
        if ui.button("Se lever").clicked() {
            *action = BarAction::StandUp;
            ui.close();
        }
        if ui.button(if st.flying { "Arrêter de voler" } else { "Voler" }).clicked() {
            *action = BarAction::ToggleFly;
            ui.close();
        }
        ui.separator();
        if ui.button("Préférences…").clicked() {
            panels.settings = true;
            ui.close();
        }
        ui.separator();
        if ui.button("Se déconnecter").clicked() {
            *action = BarAction::Logout;
            ui.close();
        }
        if ui.button("Quitter Aurora Viewer").clicked() {
            *action = BarAction::Quit;
            ui.close();
        }
    });
    ui.menu_button(small("Communiquer", p.ink), |ui| {
        ui.checkbox(&mut panels.chat, "Conversations");
        if ui.button("Amis").clicked() {
            panels.people = true;
            panels.people_tab = 1;
            ui.close();
        }
        if ui.button("Personnes à proximité").clicked() {
            panels.people = true;
            panels.people_tab = 0;
            ui.close();
        }
        if ui.button("Groupes").clicked() {
            panels.people = true;
            panels.people_tab = 2;
            ui.close();
        }
        if ui.button("Liste de blocage").clicked() {
            panels.people = true;
            panels.people_tab = 4;
            ui.close();
        }
        ui.separator();
        // Firestorm "Online Status" menu
        ui.menu_button("Statut de connexion", |ui| status_menu(ui, st.status, action));
    });
    ui.menu_button(small("Monde", p.ink), |ui| {
        // menu_viewer.xml: Historique de téléportation (Alt+H), Lieux
        if ui
            .add(egui::Button::new("Historique de téléportation").shortcut_text("Alt+H"))
            .clicked()
        {
            *action = BarAction::TeleportHistory;
            ui.close();
        }
        ui.checkbox(&mut panels.places, "Lieux");
        ui.checkbox(&mut panels.minimap, "Mini-carte");
        ui.checkbox(&mut panels.world_map, "Carte du monde");
        ui.menu_button("Lignes d'interdiction", |ui| {
            for (v, label) in [(0u8, "Masquées"), (1, "À la collision"), (2, "À proximité")] {
                if ui.radio(st.ban_lines == v, label).clicked() {
                    *action = BarAction::BanLines(v);
                    ui.close();
                }
            }
        });
        ui.menu_button("Environnement", |ui| {
            // back to the region / parcel EEP cycle, in real time
            let shared = !st.local_env;
            if ui
                .add(egui::Button::new("Utiliser les environnements partagés").selected(shared))
                .clicked()
            {
                panels.time_of_day = 0;
                *action = BarAction::SharedEnvironment;
                ui.close();
            }
            ui.separator();
            for (i, label) in TIME_OF_DAY.iter().enumerate().skip(1) {
                if ui.radio(panels.time_of_day == i as u8, *label).clicked() {
                    panels.time_of_day = i as u8;
                    ui.close();
                }
            }
            ui.separator();
            // Firestorm's quick preferences lists, and World > Environment >
            // Personal Lighting...
            if ui.button("Sélecteur d'environnement…").clicked() {
                panels.environment = true;
                ui.close();
            }
            if ui.button("Éclairage personnel…").clicked() {
                panels.personal_lighting = true;
                ui.close();
            }
        });
        ui.separator();
        if ui.button("Vue souris (M)").clicked() {
            *action = BarAction::ToggleMouselook;
            ui.close();
        }
        if ui.button("Réinitialiser la caméra (Échap)").clicked() {
            *action = BarAction::ResetCamera;
            ui.close();
        }
    });
    ui.menu_button(small("Construire", p.ink), |ui| {
        if ui.button("Construire (Ctrl+B)").clicked() {
            *action = BarAction::Build(0);
            ui.close();
        }
        if ui.button("Modifier des objets (Ctrl+3)").clicked() {
            *action = BarAction::Build(1);
            ui.close();
        }
        if ui.button("Créer des prims (Ctrl+4)").clicked() {
            *action = BarAction::Build(2);
            ui.close();
        }
        if ui.button("Modifier le terrain (Ctrl+5)").clicked() {
            *action = BarAction::Build(3);
            ui.close();
        }
    });
    ui.menu_button(small("Contenu", p.ink), |ui| {
        ui.checkbox(&mut panels.inventory, "Inventaire");
    });
    if advanced {
        ui.menu_button(small("Avancé", p.ink), |ui| {
            ui.checkbox(&mut panels.perf, "Performances (Ctrl+Maj+1)");
            ui.checkbox(&mut panels.settings, "Préférences graphiques");
        });
    }
    ui.menu_button(small("Aide", p.ink), |ui| {
        ui.label(small(format!("Aurora Viewer {}", env!("CARGO_PKG_VERSION")), p.muted));
        ui.separator();
        ui.label(small("Marcher : flèches (double appui = courir)", p.ink));
        ui.label(small(
            "Sauter / monter : Page↑   ·   S'accroupir : Page↓   ·   Voler : Origine",
            p.ink,
        ));
        ui.label(small("Vue souris : M   ·   Chat : Entrée", p.ink));
        ui.label(small("Clic droit + glisser : orbiter   ·   Molette : zoom", p.ink));
        ui.label(small("Raccourcis modifiables : Préférences › Raccourcis", p.muted));
    });
}

/// Draw a (white mask) icon tinted, centered in `rect`.
fn paint_icon(ui: &egui::Ui, icons: &Icons, name: &str, rect: egui::Rect, size: f32, tint: Color32) {
    if let Some(t) = icons.get(name) {
        let r = egui::Rect::from_center_size(rect.center(), Vec2::splat(size));
        ui.painter().image(
            t.id(),
            r,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
}

/// Back / forward arrow of the navigation bar; `dest` is the history entry
/// it leads to (None = disabled).
fn nav_arrow(ui: &mut egui::Ui, p: &Palette, icons: &Icons, left: bool, dest: Option<&str>) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(20.0, 20.0), egui::Sense::click());
    let enabled = dest.is_some();
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, 2.0, p.raised);
    }
    let col = if !enabled {
        p.muted_dim.gamma_multiply(0.6)
    } else if resp.hovered() {
        p.ink
    } else {
        p.muted
    };
    paint_icon(ui, icons, if left { "arrow-left" } else { "arrow-right" }, rect, 15.0, col);
    let tip = match dest {
        Some(d) if left => format!("Retour : {d}"),
        Some(d) => format!("Suivant : {d}"),
        None if left => "Aucune destination précédente".to_owned(),
        None => "Aucune destination suivante".to_owned(),
    };
    resp.on_hover_text(tip).clicked() && enabled
}

/// Circled "i" (About Land) at the start of the location field.
fn info_button(ui: &mut egui::Ui, p: &Palette, icons: &Icons) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(16.0, 16.0), egui::Sense::click());
    let col = if resp.hovered() { p.ink } else { p.muted };
    paint_icon(ui, icons, "info", rect, 16.0, col);
    resp.on_hover_text("À propos du terrain").clicked()
}

/// Voice connection light: green connected, orange connecting, red not
/// connected, grey turned off. Returns true when clicked (voice settings).
fn voice_light(ui: &mut egui::Ui, p: &Palette, icons: &Icons, light: crate::voice::VoiceLight, why: &str) -> bool {
    use crate::voice::VoiceLight as L;
    let (col, state) = match light {
        L::Connected => (p.success, "Voix connectée"),
        L::Connecting => (p.amber, "Connexion à la voix…"),
        L::Down => (p.danger, "Voix non connectée"),
        L::Off => (p.muted_dim, "Voix désactivée"),
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(48.0, 18.0), egui::Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect, 9.0, if resp.hovered() { p.raised } else { p.field });
    let dot = egui::pos2(rect.left() + 9.0, rect.center().y);
    if light == L::Connecting {
        // pulse while connecting
        let t = ui.input(|i| i.time) as f32;
        let a = 0.55 + 0.45 * (t * 5.0).sin().abs();
        painter.circle_filled(dot, 4.0, col.gamma_multiply(a));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    } else {
        painter.circle_filled(dot, 4.0, col);
    }
    if light == L::Connected {
        painter.circle_stroke(dot, 6.0, egui::Stroke::new(1.0, col.gamma_multiply(0.35)));
    }
    let icon_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 23.0, rect.center().y), Vec2::splat(12.0));
    paint_icon(
        ui,
        icons,
        "microphone",
        icon_rect,
        12.0,
        if resp.hovered() { p.ink } else { p.muted },
    );
    ui.painter().text(
        egui::pos2(rect.left() + 31.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "Voix",
        egui::FontId::proportional(10.5),
        if resp.hovered() { p.ink } else { p.muted },
    );
    let tip = if why.is_empty() || why == state {
        state.to_owned()
    } else {
        format!("{state}\n{why}")
    };
    resp.on_hover_text(format!("{tip}\nClic : paramètres son et voix")).clicked()
}

/// Location of the navigation bar (LLLocationInputCtrl): shows "Parcel,
/// Region (x, y, z)"; a click turns it into the SLURL of the position,
/// selected and ready to copy; typing or pasting a place then Enter
/// teleports there; Escape or a click elsewhere restores the location.
fn location_field(ui: &mut egui::Ui, p: &Palette, panels: &mut Panels, loc: &str, slurl: &str, w: f32) -> Option<BarAction> {
    let id = egui::Id::new("nav_location_edit");
    let Some(text) = panels.nav_edit.as_mut() else {
        // exactly the text width (truncated when too long): the maturity badge
        // stays right after the location
        let text_w = ui
            .painter()
            .layout_no_wrap(loc.to_owned(), egui::FontId::proportional(12.0), p.ink)
            .size()
            .x;
        let lw = text_w.min(w).ceil();
        let resp = ui
            .allocate_ui_with_layout(Vec2::new(lw, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_max_width(lw);
                ui.add(egui::Label::new(small(loc, p.ink)).truncate().sense(egui::Sense::click()))
            })
            .inner
            .on_hover_text(format!("{loc}\nCliquez pour copier la SLURL ou saisir une destination"));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        if resp.clicked() {
            panels.nav_edit = Some(slurl.to_owned());
            panels.nav_edit_new = true;
        }
        return None;
    };
    let resp = ui.add(
        egui::TextEdit::singleline(text)
            .id(id)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .font(egui::FontId::proportional(12.0))
            .text_color(p.ink)
            .hint_text("SLURL, Région/x/y/z ou nom de région")
            .desired_width(w),
    );
    if panels.nav_edit_new {
        // focus first; once focused, select everything (ready to copy, or to
        // be replaced by typing / pasting)
        resp.request_focus();
        if resp.has_focus() {
            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
            let n = text.chars().count();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(n),
            )));
            state.store(ui.ctx(), id);
            panels.nav_edit_new = false;
        }
        ui.ctx().request_repaint();
        return None;
    }
    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    if enter {
        let typed = std::mem::take(text);
        panels.nav_edit = None;
        if !typed.trim().is_empty() {
            return Some(BarAction::TeleportTo(typed));
        }
    } else if escape || !resp.has_focus() {
        // Escape or a click elsewhere: back to the location as it was
        panels.nav_edit = None;
        ui.memory_mut(|m| m.surrender_focus(id));
    }
    None
}

fn small(text: impl Into<String>, color: Color32) -> RichText {
    RichText::new(text).size(12.0).color(color)
}

/// Top chrome: a single navigation bar (Aurora menu, notifications,
/// teleport history, location) with the status on its right (balance,
/// time, ping, media and volumes, frame rate).
#[allow(clippy::too_many_arguments)]
pub fn top_bars(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    layout: &Layout,
    panels: &mut Panels,
    st: &StatusInfo,
    audio: &mut crate::settings::AudioSettings,
    audio_ui: &mut super::audio::AudioUi,
    media: super::audio::MediaState,
    notif: &mut super::notifications::NotifUi,
    unread: usize,
) -> (BarAction, super::audio::AudioAction) {
    let mut action = BarAction::None;
    let mut audio_action = super::audio::AudioAction::None;
    egui::Panel::top("nav_bar")
        .frame(bar_frame(p, 245))
        .exact_size(layout.nav_bar_height)
        .show(ui, |ui| {
            // the bar's background spans the whole window width
            ui.set_min_width(ui.available_width());
            ui.horizontal_centered(|ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.style_mut().visuals.button_frame = false;
                // Aurora menu (white wolf): Moi, Communiquer, Monde, Construire, Contenu, Avancé, Aide
                if let Some(t) = icons.get("wolf") {
                    let img = egui::Image::new(t).fit_to_exact_size(Vec2::new(18.0, 18.0)).tint(p.ink);
                    ui.menu_image_button(img, |ui| {
                        main_menus(ui, p, panels, st, &mut action, true);
                    })
                    .response
                    .on_hover_text("Menu Aurora");
                }
                // notification center (bell) next to the wolf
                let (clicked, bell) = super::notifications::bell(ui, p, icons, unread, notif.open);
                notif.anchor = Some(bell.left_bottom());
                if clicked {
                    notif.open = !notif.open;
                }
                // online status (away, do not disturb, automatic responses)
                status_dot(ui, p, st.status, &mut action);
                ui.add_space(2.0);
                // back / forward through the teleport history, home (Firestorm navigation bar)
                if nav_arrow(ui, p, icons, true, st.tp_back.as_deref()) {
                    action = BarAction::TeleportBack;
                }
                if nav_arrow(ui, p, icons, false, st.tp_forward.as_deref()) {
                    action = BarAction::TeleportForward;
                }
                if icon_button(ui, p, icons, "Home_Off", "Rentrer chez moi", 15.0) {
                    action = BarAction::TeleportHome;
                }
                ui.add_space(6.0);

                // status on the right (laid out from the right edge), the location
                // field takes what is left
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.add_space(2.0);
                    let fps_col = if st.fps >= 50.0 {
                        p.teal
                    } else if st.fps >= 25.0 {
                        p.amber
                    } else {
                        p.danger
                    };
                    // a click opens (or closes) the performance window
                    let fps = ui
                        .add(egui::Label::new(small(format!("{:.1}", st.fps), fps_col)).sense(egui::Sense::click()))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Images par seconde : cliquez pour les performances");
                    if fps.clicked() {
                        panels.perf = !panels.perf;
                    }
                    audio_action = super::audio::top_controls(ui, p, icons, audio, audio_ui, media);
                    if voice_light(ui, p, icons, st.voice.0, &st.voice.1) {
                        action = BarAction::VoicePrefs;
                    }
                    ui.label(small(format!("{} ms", st.ping_ms), p.muted))
                        .on_hover_text("Latence avec la région");
                    ui.label(small(slt_time(), p.muted))
                        .on_hover_text("Heure de Second Life (Pacifique)");
                    if let Some(b) = st.balance {
                        ui.label(small(format!("{b} L$"), p.ink)).on_hover_text("Solde");
                    }
                    for (on, text) in [(st.flying, "En vol"), (st.seated, "Assis")] {
                        if on {
                            egui::Frame::new()
                                .fill(p.indigo)
                                .corner_radius(egui::CornerRadius::same(2))
                                .inner_margin(egui::Margin::symmetric(5, 0))
                                .show(ui, |ui| ui.label(small(text, Color32::WHITE)));
                        }
                    }
                    ui.add_space(4.0);
                    egui::Frame::new()
                        .fill(p.field)
                        .corner_radius(egui::CornerRadius::same(2))
                        .inner_margin(egui::Margin::symmetric(8, 1))
                        .show(ui, |ui| {
                            ui.set_width((ui.available_width() - 4.0).max(160.0));
                            // the whole field is clickable (widgets added later, like the
                            // ⓘ button, stay on top of this area)
                            let field = ui.max_rect().expand2(Vec2::new(8.0, 1.0));
                            let field = egui::Rect::from_x_y_ranges(
                                field.x_range(),
                                ui.max_rect().center().y - 10.0..=ui.max_rect().center().y + 10.0,
                            );
                            let bg = ui.interact(field, ui.id().with("nav_location_area"), egui::Sense::click());
                            if panels.nav_edit.is_none() {
                                if bg.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
                                }
                                if bg.clicked() {
                                    panels.nav_edit = Some(st.slurl.clone());
                                    panels.nav_edit_new = true;
                                }
                            }
                            let mut loc = String::new();
                            if !st.parcel.is_empty() {
                                loc.push_str(st.parcel);
                                loc.push_str(", ");
                            }
                            loc.push_str(&format!(
                                "{} ({:.0}, {:.0}, {:.0})",
                                st.region, st.position.x, st.position.y, st.position.z
                            ));
                            if !st.maturity.is_empty() {
                                loc.push_str(&format!(" - {}", st.maturity));
                            }
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                if info_button(ui, p, icons) {
                                    panels.about_land = !panels.about_land;
                                }
                                // keep the maturity badge and the parcel icons visible when
                                // the field is narrow; the icons go first in a very narrow
                                // window (they would spill over the clock)
                                let icons_w = if st.parcel_icons.is_empty() {
                                    0.0
                                } else {
                                    super::parcel_icons::width(ui, &st.parcel_icons) + 8.0
                                };
                                let icons_w = if ui.available_width() - 24.0 - icons_w >= 80.0 {
                                    icons_w
                                } else {
                                    0.0
                                };
                                let w = (ui.available_width() - 24.0 - icons_w).max(40.0);
                                ui.allocate_ui(Vec2::new(w, 18.0), |ui| {
                                    if let Some(a) = location_field(ui, p, panels, &loc, &st.slurl, w) {
                                        action = a;
                                    }
                                });
                                super::widgets::maturity_badge(ui, p, st.maturity, 14.0);
                                // parcel abilities, anchored to the right edge of the field
                                if icons_w > 0.0 {
                                    ui.add_space((ui.available_width() - icons_w + 8.0).max(0.0));
                                    ui.scope(|ui| {
                                        if super::parcel_icons::show(ui, p, icons, &st.parcel_icons, p.field) {
                                            panels.about_land = !panels.about_land;
                                        }
                                    });
                                }
                            });
                        });
                });
            });
        });
    (action, audio_action)
}

/// Execute a toolbar command; returns an action for the app.
pub fn run_command(cmd: &str, panels: &mut Panels) -> BarAction {
    match cmd {
        "conversations" => panels.chat = !panels.chat,
        "people" => {
            panels.people = !panels.people;
            panels.people_tab = 0;
        }
        "friends" => {
            panels.people = !panels.people;
            panels.people_tab = 1;
        }
        "inventory" => panels.inventory = !panels.inventory,
        "appearance" => panels.appearance = !panels.appearance,
        "minimap" => panels.minimap = !panels.minimap,
        "worldmap" => panels.world_map = !panels.world_map,
        "places" => panels.places = !panels.places,
        "performance" => panels.perf = !panels.perf,
        "preferences" => panels.settings = !panels.settings,
        "fly" => return BarAction::ToggleFly,
        "mouselook" => return BarAction::ToggleMouselook,
        "sit" => return BarAction::SitGround,
        "home" => return BarAction::TeleportHome,
        _ => {}
    }
    BarAction::None
}

fn command_active(cmd: &str, panels: &Panels, flying: bool, seated: bool, mouselook: bool) -> bool {
    match cmd {
        "conversations" => panels.chat,
        "people" | "friends" => panels.people,
        "inventory" => panels.inventory,
        "appearance" => panels.appearance,
        "minimap" => panels.minimap,
        "worldmap" => panels.world_map,
        "places" => panels.places,
        "performance" => panels.perf,
        "preferences" => panels.settings,
        "fly" => flying,
        "sit" => seated,
        "mouselook" => mouselook,
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn tool_button(ui: &mut egui::Ui, p: &Palette, icons: &Icons, icon: &str, tip: &str, active: bool, width: f32, badge: bool) -> bool {
    let size = Vec2::new(width, 22.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let icon_size = (width - 6.0).clamp(10.0, 16.0);
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect, 1.0, p.violet.gamma_multiply(0.30));
        painter.rect_stroke(
            rect,
            1.0,
            egui::Stroke::new(1.0, p.violet.gamma_multiply(0.45)),
            egui::StrokeKind::Inside,
        );
    } else if resp.hovered() {
        painter.rect_filled(rect, 2.0, p.field);
    }
    if let Some(t) = icons.get(icon) {
        let tint = if active || resp.hovered() { p.ink } else { p.muted };
        let ir = egui::Rect::from_center_size(rect.center(), Vec2::splat(icon_size));
        painter.image(
            t.id(),
            ir,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    if badge {
        painter.circle_filled(
            egui::pos2(rect.right() - (width * 0.18).clamp(4.0, 8.0), rect.center().y),
            (width * 0.07).clamp(2.0, 3.0),
            p.violet,
        );
    }
    resp.on_hover_text(tip).clicked()
}

/// Microphone button state (Firestorm "Parler" button with its arrow).
#[derive(Default)]
pub struct MicButton {
    /// Toggle mode: microphone switched on.
    pub on: bool,
    /// Hold-to-talk mode (else click toggles).
    pub hold: bool,
    /// Hold mode: button pressed this frame (output).
    pub held: bool,
    /// Transmitting (button / push-to-talk key).
    pub talking: bool,
    /// Own voice level 0..1.
    pub level: f32,
    /// Voice connected in this region.
    pub connected: bool,
    /// Output: open the "Son et voix" preferences.
    pub open_prefs: bool,
    /// Output: the mode switch changed.
    pub mode_changed: bool,
}

fn mic_button(ui: &mut egui::Ui, p: &Palette, icons: &Icons, width: f32, mic: &mut MicButton) {
    let caret_w = if width >= 30.0 { 9.0 } else { 0.0 };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 22.0), egui::Sense::hover());
    let main = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right() - caret_w, rect.bottom()));
    let resp = ui.interact(main, ui.id().with("mic_main"), egui::Sense::click_and_drag());
    if mic.hold {
        mic.held = resp.is_pointer_button_down_on();
    } else if resp.clicked() {
        mic.on = !mic.on;
    }
    let painter = ui.painter();
    let live = mic.talking;
    if live {
        painter.rect_filled(rect, 1.0, p.violet.gamma_multiply(0.30));
        painter.rect_stroke(
            rect,
            1.0,
            egui::Stroke::new(1.0, p.violet.gamma_multiply(0.45)),
            egui::StrokeKind::Inside,
        );
        // own level, bottom of the button
        let w = (main.width() - 6.0) * mic.level.clamp(0.0, 1.0);
        if w > 0.5 {
            painter.rect_filled(
                egui::Rect::from_min_size(egui::pos2(main.left() + 3.0, main.bottom() - 3.0), Vec2::new(w, 2.0)),
                1.0,
                p.teal,
            );
        }
    } else if resp.hovered() {
        painter.rect_filled(rect, 2.0, p.field);
    }
    let icon = if live || (!mic.hold && mic.on) {
        "microphone"
    } else {
        "microphone-slash"
    };
    if let Some(t) = icons.get(icon) {
        let size = (main.width() - 6.0).clamp(10.0, 16.0);
        let tint = if live || resp.hovered() { p.ink } else { p.muted };
        let ir = egui::Rect::from_center_size(main.center(), Vec2::splat(size));
        painter.image(
            t.id(),
            ir,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    let state = if !mic.connected {
        "voix non connectée"
    } else if live {
        "vous parlez"
    } else {
        "micro coupé"
    };
    let tip = if mic.hold {
        format!("Maintenir pour parler ({state})")
    } else {
        format!("Cliquer pour activer / couper le micro ({state})")
    };
    resp.on_hover_text(tip);

    // arrow: mode switch and voice preferences
    if caret_w > 0.0 {
        let caret = egui::Rect::from_min_max(egui::pos2(main.right(), rect.top()), rect.max);
        let cresp = ui.interact(caret, ui.id().with("mic_caret"), egui::Sense::click());
        let col = if cresp.hovered() { p.ink } else { p.muted_dim };
        paint_icon(ui, icons, "caret-down", caret, 8.0, col);
        egui::Popup::menu(&cresp).align(egui::RectAlign::TOP_END).show(|ui| {
            ui.set_min_width(210.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Maintenir pour parler").size(12.5).color(p.ink));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if super::widgets::switch(ui, p, &mut mic.hold).changed() {
                        mic.mode_changed = true;
                        mic.on = false;
                    }
                });
            });
            ui.label(
                RichText::new(if mic.hold {
                    "Parler tant que le bouton ou la touche est enfoncé"
                } else {
                    "Un clic active le micro, un autre le coupe"
                })
                .size(11.0)
                .color(p.muted),
            );
            ui.separator();
            if ui.button(RichText::new("Paramètres son et voix…").size(12.5)).clicked() {
                mic.open_prefs = true;
                ui.close();
            }
        });
    }
}

/// Bottom bar: "Chat local" input on the left, feature buttons spread over
/// the rest of the width. Returns (chat, action, top y of the bar).
#[allow(clippy::too_many_arguments)]
pub fn bottom_bar(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    layout: &Layout,
    panels: &mut Panels,
    chat: &mut super::chat::ChatUi,
    unread: usize,
    flying: bool,
    seated: bool,
    mouselook: bool,
    chat_width: &mut f32,
    mic: &mut MicButton,
) -> (Option<super::chat::OutgoingChat>, BarAction, f32, bool) {
    let mut out = None;
    let mut action = BarAction::None;
    let mut width_committed = false;
    let resp = egui::Panel::bottom("bottom_bar")
        .frame(bar_frame(p, 245).inner_margin(egui::Margin::symmetric(8, 3)))
        .exact_size(layout.bottom_bar_height)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_centered(|ui| {
                let total = ui.available_width();
                let n = layout.toolbar.len().max(1) as f32;
                // the toolbar keeps at least a narrow button per command
                const MIN_BTN: f32 = 22.0;
                let label_w = 70.0;
                let max_chat = (total - label_w - 16.0 - n * (MIN_BTN + 2.0)).max(120.0);
                let wanted = if *chat_width > 0.0 {
                    *chat_width
                } else {
                    total * layout.chat_bar_fraction
                };
                let chat_w = wanted.clamp(120.0, max_chat);
                out = super::chat::bar(ui, p, chat, chat_w);

                // draggable separator (dotted, like Firestorm's) between chat and toolbar
                let (handle, hresp) = ui.allocate_exact_size(Vec2::new(10.0, 22.0), egui::Sense::drag());
                let hot = hresp.hovered() || hresp.dragged();
                if hot {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                let col = if hot { p.violet_light } else { p.muted_dim };
                let x = handle.center().x;
                let mut y = handle.top() + 3.0;
                while y < handle.bottom() - 2.0 {
                    ui.painter().circle_filled(egui::pos2(x, y), 0.9, col);
                    y += 3.0;
                }
                if hresp.dragged() {
                    *chat_width = (chat_w + hresp.drag_delta().x).clamp(120.0, max_chat);
                }
                if hresp.drag_stopped() {
                    width_committed = true;
                }
                if hresp.double_clicked() {
                    // back to the skin's default proportion
                    *chat_width = 0.0;
                    width_committed = true;
                }

                // buttons stretch over the whole remaining width (hover and
                // click area included), a thin gap between them
                let remaining = ui.available_width();
                let gap = 2.0;
                let btn_w = ((remaining - gap * (n - 1.0) - 1.0) / n).floor().max(MIN_BTN);
                ui.spacing_mut().item_spacing.x = gap;
                for (i, b) in layout.toolbar.iter().enumerate() {
                    // subtle divider in the gap: where each button starts and ends
                    if i > 0 {
                        let r = ui.max_rect();
                        let x = ui.cursor().left() - gap * 0.5;
                        let h = r.height() * 0.3;
                        ui.painter().line_segment(
                            [egui::pos2(x, r.center().y - h), egui::pos2(x, r.center().y + h)],
                            egui::Stroke::new(1.0, p.muted_dim.gamma_multiply(0.45)),
                        );
                    }
                    if b.command == "speak" {
                        mic_button(ui, p, icons, btn_w, mic);
                        continue;
                    }
                    let active = command_active(&b.command, panels, flying, seated, mouselook);
                    let badge = b.command == "conversations" && unread > 0 && !panels.chat;
                    let tip = if badge {
                        format!("{} ({unread} non lus)", b.tooltip)
                    } else {
                        b.tooltip.clone()
                    };
                    if tool_button(ui, p, icons, &b.icon, &tip, active, btn_w, badge) {
                        let a = run_command(&b.command, panels);
                        if !matches!(a, BarAction::None) {
                            action = a;
                        }
                    }
                }
            });
        });
    (out, action, resp.response.rect.top(), width_committed)
}

//! "Options" (LLPanelLandOptions::refresh / refreshSearch / onCommitAny /
//! onClickSet / onClickClear, FIRE-10043 teleport to the landing point,
//! FIRE-6604 terraform checkbox).

use super::general::set;
use super::{Dialog, LandUi, View, check, dim, text};
use crate::theme::Palette;
use crate::world::World;
use crate::world::land::{MIN_PARCEL_AREA, powers};
use aurora_net::{ParcelInfo, parcel_flags as pf, region_flags as rf};
use egui::{RichText, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

/// PARCEL_DIRECTORY_FEE (L$ per week).
const DIRECTORY_FEE: i32 = 30;

/// Categories of the combo, in its order (LLParcel::ECategory, French skin).
const CATEGORIES: [(i32, &str); 13] = [
    (0, "Toutes catégories"),
    (1, "Appartenant aux Lindens"),
    (3, "Art et Culture"),
    (4, "Affaires"),
    (5, "Éducation"),
    (6, "Jeux"),
    (7, "Favoris"),
    (8, "Accueil pour les nouveaux"),
    (9, "Parcs et Nature"),
    (10, "Résidentiel"),
    (11, "Shopping"),
    (14, "Location"),
    (13, "Autre"),
];

/// Teleport routing (LLParcel::ELandingType).
const LANDING_TYPES: [&str; 3] = ["Bloqué", "Lieu d'arrivée fixe", "Lieu d'arrivée libre"];

/// The options as the panel shows them: group boxes are checked when
/// "everyone" is (refresh), and onCommitAny derives the flags back.
#[derive(Clone, Copy)]
struct Shown {
    edit_objects: bool,
    edit_group: bool,
    all_entry: bool,
    group_entry: bool,
    terraform: bool,
    safe: bool,
    fly: bool,
    group_scripts: bool,
    other_scripts: bool,
    push: bool,
    see_avs: bool,
    directory: bool,
    mature: bool,
}

impl Shown {
    fn of(p: &ParcelInfo, region_push: bool, mature: bool) -> Shown {
        let f = |b| p.flags & b != 0;
        Shown {
            edit_objects: f(pf::CREATE_OBJECTS),
            edit_group: f(pf::CREATE_GROUP_OBJECTS) || f(pf::CREATE_OBJECTS),
            all_entry: f(pf::ALLOW_ALL_OBJECT_ENTRY),
            group_entry: f(pf::ALLOW_GROUP_OBJECT_ENTRY) || f(pf::ALLOW_ALL_OBJECT_ENTRY),
            terraform: f(pf::ALLOW_TERRAFORM),
            safe: !f(pf::ALLOW_DAMAGE),
            fly: f(pf::ALLOW_FLY),
            group_scripts: f(pf::ALLOW_GROUP_SCRIPTS) || f(pf::ALLOW_OTHER_SCRIPTS),
            other_scripts: f(pf::ALLOW_OTHER_SCRIPTS),
            // a region override shows (and sends) the restriction
            push: f(pf::RESTRICT_PUSHOBJECT) || region_push,
            see_avs: p.see_avatars,
            directory: f(pf::SHOW_DIRECTORY),
            mature,
        }
    }

    /// onCommitAny: the parcel flags of what is shown.
    fn flags(&self, old: u32) -> u32 {
        let mut f = old;
        f = set(f, pf::CREATE_OBJECTS, self.edit_objects);
        f = set(f, pf::CREATE_GROUP_OBJECTS, self.edit_group || self.edit_objects);
        f = set(f, pf::ALLOW_ALL_OBJECT_ENTRY, self.all_entry);
        f = set(f, pf::ALLOW_GROUP_OBJECT_ENTRY, self.group_entry || self.all_entry);
        f = set(f, pf::ALLOW_TERRAFORM, self.terraform);
        f = set(f, pf::ALLOW_DAMAGE, !self.safe);
        f = set(f, pf::ALLOW_FLY, self.fly);
        // landmarks cannot be restricted
        f = set(f, pf::ALLOW_LANDMARK, true);
        f = set(f, pf::ALLOW_GROUP_SCRIPTS, self.group_scripts || self.other_scripts);
        f = set(f, pf::ALLOW_OTHER_SCRIPTS, self.other_scripts);
        f = set(f, pf::SHOW_DIRECTORY, self.directory);
        f = set(f, pf::ALLOW_PUBLISH, false);
        f = set(f, pf::MATURE_PUBLISH, self.mature);
        set(f, pf::RESTRICT_PUSHOBJECT, self.push)
    }
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
    let can_options = v.can(powers::LAND_OPTIONS);
    let can_terraform = v.can(powers::LAND_EDIT);
    let can_identity = v.can(powers::LAND_CHANGE_IDENTITY);
    let can_landing = v.can(powers::LAND_SET_LANDING_POINT);
    let sim_access = v.region.as_ref().map_or(0, |r| r.sim_access);
    let mature_value = match sim_access {
        21 => parcel.flags & pf::MATURE_PUBLISH != 0,
        42 => true,
        _ => false,
    };
    let before = Shown::of(&parcel, parcel.region_push_override, mature_value);
    let mut now = before;
    let mut extra: Option<Box<dyn FnOnce(&mut aurora_net::land::ParcelUpdate)>> = None;

    ui.label(RichText::new("Autoriser les autres résidents à :").size(12.0).color(p.ink));
    let line = |ui: &mut egui::Ui, label: &str, body: &mut dyn FnMut(&mut egui::Ui)| {
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(Vec2::new(150.0, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.label(RichText::new(label).size(12.0).color(p.muted));
            });
            body(ui);
        });
    };
    let col = |ui: &mut egui::Ui, w: f32, body: &mut dyn FnMut(&mut egui::Ui)| {
        ui.allocate_ui_with_layout(Vec2::new(w, 18.0), egui::Layout::left_to_right(egui::Align::Center), body);
    };
    line(ui, "Modifier le terrain :", &mut |ui| {
        col(ui, 130.0, &mut |ui| {
            let (r, on) = check(ui, can_terraform, now.terraform, "Tous");
            r.on_hover_text(
                "Si cette option est cochée, tout le monde pourra terraformer votre terrain. Mieux vaut la laisser décochée : vous pouvez toujours modifier votre propre terrain.",
            );
            now.terraform = on;
        });
        dim(ui, p, "(À utiliser prudemment !)");
    });
    line(ui, "Voler :", &mut |ui| {
        let (r, on) = check(ui, can_options, now.fly, "Tous");
        r.on_hover_text("Si cette option est cochée, les résidents peuvent voler sur votre terrain. Sinon, ils ne peuvent que le survoler ou y arriver en volant.");
        now.fly = on;
    });
    line(ui, "Construire :", &mut |ui| {
        col(ui, 130.0, &mut |ui| {
            let (r, on) = check(ui, can_options, now.edit_objects, "Tous");
            r.on_hover_text("Si la case est cochée, les résidents peuvent créer et rezzer des objets sur votre terrain.");
            now.edit_objects = on;
        });
        // explicitly on (and greyed) while everyone may build
        let (r, on) = check(ui, can_options && !before.edit_objects, now.edit_group, "Groupe");
        r.on_hover_text("Si la case est cochée, les membres du groupe de la parcelle peuvent créer et rezzer des objets sur votre terrain.");
        now.edit_group = on;
    });
    line(ui, "Laisser entrer des objets :", &mut |ui| {
        col(ui, 130.0, &mut |ui| {
            let (r, on) = check(ui, can_options, now.all_entry, "Tous");
            r.on_hover_text("Si la case est cochée, les résidents peuvent déplacer leurs objets depuis d'autres parcelles jusqu'à celle-ci.");
            now.all_entry = on;
        });
        let (r, on) = check(ui, can_options && !before.all_entry, now.group_entry, "Groupe");
        r.on_hover_text("Si la case est cochée, les membres du groupe de la parcelle peuvent déplacer leurs objets depuis d'autres parcelles jusqu'à celle-ci.");
        now.group_entry = on;
    });
    line(ui, "Exécuter des scripts :", &mut |ui| {
        col(ui, 130.0, &mut |ui| {
            let (r, on) = check(ui, can_options, now.other_scripts, "Tous");
            r.on_hover_text("Si la case est cochée, les résidents peuvent exécuter des scripts sur votre parcelle, y compris ceux de leurs attachements.");
            now.other_scripts = on;
        });
        let (r, on) = check(ui, can_options && !before.other_scripts, now.group_scripts, "Groupe");
        r.on_hover_text("Si la case est cochée, les membres du groupe de la parcelle peuvent exécuter des scripts sur votre parcelle, y compris ceux de leurs attachements.");
        now.group_scripts = on;
    });
    ui.separator();
    // refreshSearch: small parcels stay out of search unless already listed
    let can_search = v.can(powers::LAND_FIND_PLACES) && !v.region_flag(rf::BLOCK_PARCEL_SEARCH);
    let large = parcel.area >= MIN_PARCEL_AREA;
    let (search_on, search_tip) = match (large, can_search) {
        (true, true) => (true, "Permettre aux autres résidents de voir cette parcelle dans les résultats de recherche"),
        (false, true) if before.directory => (true, "Permettre aux autres résidents de voir cette parcelle dans les résultats de recherche"),
        (false, true) => (
            false,
            "Cette option est désactivée car la superficie de cette parcelle est inférieure ou égale à 128 m². Seules les parcelles de grande taille peuvent apparaître dans la recherche.",
        ),
        _ => (false, "Cette option est désactivée car vous ne pouvez pas modifier les options de cette parcelle."),
    };
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        let (r, on) = check(ui, can_options, now.safe, "Sécurisé (pas de dégâts)");
        r.on_hover_text("Si cette option est cochée, le terrain est sécurisé : pas de dégâts lors des combats. Décochée, les dégâts sont possibles.");
        now.safe = on;
        let label = if DIRECTORY_FEE == 0 {
            "Lieu dans la recherche".to_owned()
        } else {
            format!("Lieu dans la recherche ({DIRECTORY_FEE}L$/sem.)")
        };
        let (r, on) = check(ui, search_on, now.directory, &label);
        r.on_hover_text(search_tip).on_disabled_hover_text(search_tip);
        now.directory = on;
        // maturity: PG never, mature by choice, adult always
        if sim_access == 21 || sim_access == 42 || sim_access == 13 {
            ui.horizontal(|ui| {
                let adult = sim_access == 42;
                let (r, on) = check(ui, sim_access == 21 && can_identity, now.mature, "");
                now.mature = on;
                super::widgets::maturity_badge(ui, p, if adult { "Adulte" } else { "Modéré" }, 14.0);
                text(ui, p, if adult { "Contenu Adult" } else { "Contenu Modéré" });
                r.on_hover_text(if adult {
                    "Les informations ou contenu de votre parcelle sont classés Adult."
                } else {
                    "Les informations ou contenu de votre parcelle sont classés Modéré."
                });
            });
        }
        let current = CATEGORIES.iter().find(|c| c.0 == parcel.category).map_or("(autre)", |c| c.1);
        ui.add_enabled_ui(search_on, |ui| {
            egui::ComboBox::from_id_salt("land_category")
                .selected_text(current)
                .width(190.0)
                .show_ui(ui, |ui| {
                    for (id, label) in CATEGORIES {
                        if ui.selectable_label(id == parcel.category, label).clicked() && id != parcel.category {
                            extra = Some(Box::new(move |u| u.category = id as u8));
                        }
                    }
                })
                .response
                .on_hover_text(search_tip)
                .on_disabled_hover_text(search_tip);
        });

        let ui = &mut cols[1];
        let push_label = if parcel.region_push_override {
            "Pas de bousculades (les règles de la région priment)"
        } else {
            "Pas de bousculades"
        };
        let (r, on) = check(ui, can_options && !parcel.region_push_override, now.push, push_label);
        r.on_hover_text("Empêche l'utilisation de scripts causant des bousculades. Utile pour éviter les comportements abusifs sur votre terrain.");
        now.push = on;
        ui.horizontal_top(|ui| {
            // the server must know SeeAVs (getHaveNewParcelLimitData)
            let enabled = can_options && parcel.have_new_parcel_limit_data;
            let (r, on) = check(ui, enabled, now.see_avs, "");
            r.on_hover_text(
                "Permettre aux avatars présents sur d'autres parcelles de voir et chatter avec les avatars présents sur cette parcelle, et à vous de les voir et de chatter avec eux.",
            );
            now.see_avs = on;
            // toggleSeeAvatars: the label toggles the box too
            let l = ui.add(
                egui::Label::new(
                    RichText::new("Les avatars sur d'autres parcelles peuvent voir et chatter avec les avatars sur cette parcelle")
                        .size(12.0)
                        .color(if enabled { p.ink } else { p.muted }),
                )
                .wrap()
                .sense(egui::Sense::click()),
            );
            if enabled && l.clicked() {
                now.see_avs = !now.see_avs;
            }
        });
    });
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new("Photo :").size(12.0).color(p.muted));
            let size = Vec2::new(195.0, 150.0);
            let (rect, r) = ui.allocate_exact_size(size, if can_identity { egui::Sense::click() } else { egui::Sense::hover() });
            ui.painter().rect_filled(rect, 2.0, p.field);
            if parcel.snapshot_id.is_nil() {
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "(aucune)", egui::FontId::proportional(12.0), p.muted);
            } else {
                s.wanted_images.insert(parcel.snapshot_id);
                match images.get(&parcel.snapshot_id) {
                    Some(t) => {
                        ui.painter()
                            .image(t.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                    }
                    None => {
                        ui.painter()
                            .text(rect.center(), egui::Align2::CENTER_CENTER, "Chargement…", egui::FontId::proportional(12.0), p.muted);
                    }
                }
            }
            if r.on_hover_text("Cliquez pour sélectionner une image").clicked() {
                s.texture_snapshot = true;
                s.texture.open("Photo du terrain", parcel.snapshot_id);
            }
        });
        ui.add_space(14.0);
        ui.vertical(|ui| {
            ui.add_space(60.0);
            let lp = parcel.user_location;
            let has_lp = lp != glam::Vec3::ZERO;
            let landing = if has_lp {
                let look = parcel.user_look_at;
                let deg = ((look.y.atan2(-look.x) + std::f32::consts::TAU).to_degrees() + 0.5) as u32;
                format!("{}, {}, {} ({}°)", lp.x.round(), lp.y.round(), lp.z.round(), (deg - 90) % 360)
            } else {
                "(aucun)".to_owned()
            };
            text(ui, p, format!("Lieu d'arrivée : {landing}"));
            ui.horizontal(|ui| {
                if super::button(ui, p, "Définir", can_landing)
                    .on_hover_text("Définit le point d'arrivée des visiteurs à l'emplacement de votre avatar sur ce terrain.")
                    .clicked()
                {
                    // onClickSet: only from inside the parcel
                    let inside = v.same_region && world.parcel.as_ref().is_some_and(|a| a.local_id == parcel.local_id);
                    if inside {
                        let (pos, at) = (world.agent.position, world.agent.forward());
                        world.land.update(|u| {
                            u.user_location = pos;
                            u.user_look_at = at;
                        });
                    } else {
                        s.dialog = Some(Dialog::Message(
                            "Pour définir le point d'atterrissage, vous devez vous trouver à l'intérieur de la parcelle.".into(),
                        ));
                    }
                }
                if super::button(ui, p, "Annuler", can_landing).on_hover_text("Effacer le lieu d'arrivée").clicked() {
                    world.land.update(|u| {
                        u.user_location = glam::Vec3::ZERO;
                        u.user_look_at = glam::Vec3::ZERO;
                    });
                }
                if super::button(ui, p, "Teleport", has_lp).on_hover_text("Se téléporter au lieu d'arrivée").clicked() {
                    let (ox, oy) = aurora_net::handle_to_origin(v.handle);
                    world.map.track_location(ox as f64 + lp.x as f64, oy as f64 + lp.y as f64, lp.z, true);
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new("Règles de téléportation :").size(12.0).color(p.muted));
            let current = LANDING_TYPES.get(parcel.landing_type as usize).copied().unwrap_or("");
            ui.add_enabled_ui(can_landing, |ui| {
                egui::ComboBox::from_id_salt("landing_type")
                    .selected_text(current)
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        for (i, label) in LANDING_TYPES.iter().enumerate() {
                            if ui.selectable_label(i as i32 == parcel.landing_type, *label).clicked() && i as i32 != parcel.landing_type {
                                extra = Some(Box::new(move |u| u.landing_type = i as u8));
                            }
                        }
                    })
                    .response
                    .on_hover_text("Règles de téléportation : choisissez comment les téléportations sur votre terrain sont gérées");
            });
        });
    });

    let changed = before.flags(parcel.flags) != now.flags(parcel.flags) || before.see_avs != now.see_avs;
    if changed || extra.is_some() {
        // the region allows damage: scripts cannot be turned off
        let region_damage = v.region_flag(rf::ALLOW_DAMAGE);
        let scripts_off = (!now.other_scripts && parcel.flags & pf::ALLOW_OTHER_SCRIPTS != 0)
            || (!(now.group_scripts || now.other_scripts) && parcel.flags & pf::ALLOW_GROUP_SCRIPTS != 0);
        if region_damage && scripts_off {
            s.dialog = Some(Dialog::Message(
                "Impossible de désactiver les scripts. Les dégâts sont autorisés dans toute la région. Pour que les armes fonctionnent, les scripts doivent être autorisés."
                    .into(),
            ));
            return;
        }
        let flags = now.flags(parcel.flags);
        let see = now.see_avs;
        world.land.update(|u| {
            u.flags = flags;
            u.see_avs = see;
            if let Some(f) = extra {
                f(u);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everyone_implies_group_like_on_commit_any() {
        let p = ParcelInfo {
            flags: pf::CREATE_OBJECTS | pf::ALLOW_OTHER_SCRIPTS,
            ..Default::default()
        };
        let shown = Shown::of(&p, false, false);
        assert!(shown.edit_group && shown.group_scripts);
        let f = shown.flags(p.flags);
        assert!(f & pf::CREATE_GROUP_OBJECTS != 0);
        assert!(f & pf::ALLOW_GROUP_SCRIPTS != 0);
        assert!(f & pf::ALLOW_LANDMARK != 0);
        // turning "everyone" off keeps group (it was shown checked)
        let mut off = shown;
        off.edit_objects = false;
        let f = off.flags(p.flags);
        assert!(f & pf::CREATE_OBJECTS == 0 && f & pf::CREATE_GROUP_OBJECTS != 0);
    }

    #[test]
    fn region_push_override_is_sent() {
        let p = ParcelInfo::default();
        let shown = Shown::of(&p, true, false);
        assert!(shown.flags(0) & pf::RESTRICT_PUSHOBJECT != 0);
        // damage: "safe" is the opposite flag
        assert!(shown.flags(0) & pf::ALLOW_DAMAGE == 0);
    }
}

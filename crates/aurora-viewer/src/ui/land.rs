//! "À propos du terrain" (About Land): the parcel the agent stands on, from
//! the simulator's ParcelProperties, plus its region.

use super::widgets::Floater;
use crate::theme::Palette;
use crate::world::World;
use aurora_net::parcel_flags as pf;
use egui::{RichText, Vec2};

const KEY_W: f32 = 150.0;

fn row(ui: &mut egui::Ui, p: &Palette, k: &str, v: impl Into<String>) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(KEY_W, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.label(RichText::new(k).size(12.0).color(p.muted));
        });
        ui.add(egui::Label::new(RichText::new(v.into()).size(12.0).color(p.ink)).wrap());
    });
}

fn section(ui: &mut egui::Ui, p: &Palette, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title.to_uppercase()).size(10.5).color(p.violet_light));
    ui.add_space(2.0);
}

fn yes_no(v: bool) -> &'static str {
    if v { "Oui" } else { "Non" }
}

pub fn show(ctx: &egui::Context, p: &Palette, world: &World, open: &mut bool) {
    let screen = ctx.content_rect();
    Floater::new(
        "about_land",
        "À propos du terrain",
        egui::pos2(screen.center().x - 220.0, 90.0),
        Vec2::new(440.0, 460.0),
    )
    .help("Informations de la parcelle où vous vous trouvez")
    .show(ctx, p, open, |ui| {
        let Some(parcel) = world.parcel.as_ref() else {
            ui.add_space(8.0);
            ui.label(
                RichText::new("Informations de la parcelle en attente du simulateur…")
                    .size(12.0)
                    .color(p.muted),
            );
            return;
        };
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            section(ui, p, "Général");
            row(ui, p, "Nom", parcel.name.clone());
            if !parcel.desc.is_empty() {
                row(ui, p, "Description", parcel.desc.clone());
            }
            let owner = if parcel.is_group_owned {
                "Groupe".to_owned()
            } else {
                world.social.name_of(&parcel.owner_id)
            };
            row(ui, p, "Propriétaire", owner);
            row(ui, p, "Surface", format!("{} m²", parcel.area));
            if parcel.flags & pf::FOR_SALE != 0 {
                row(ui, p, "À vendre", format!("L$ {}", parcel.sale_price));
            }
            let region = world.main().map(|r| r.name.clone()).unwrap_or_default();
            let maturity = world
                .main()
                .and_then(|r| r.info.as_ref())
                .map(|i| super::bars::maturity_name(i.sim_access))
                .unwrap_or("");
            row(ui, p, "Région", format!("{region} - {maturity}"));

            section(ui, p, "Objets");
            let used = parcel.owner_prims + parcel.group_prims + parcel.other_prims + parcel.selected_prims;
            row(ui, p, "Prims", format!("{used} / {}", parcel.max_prims));
            row(
                ui,
                p,
                "Détail",
                format!(
                    "propriétaire {} · groupe {} · autres {}",
                    parcel.owner_prims, parcel.group_prims, parcel.other_prims
                ),
            );
            row(
                ui,
                p,
                "Région entière",
                format!("{} / {}", parcel.sim_total_prims, parcel.sim_max_prims),
            );

            section(ui, p, "Options");
            let f = parcel.flags;
            row(ui, p, "Voler", yes_no(f & pf::ALLOW_FLY != 0));
            row(
                ui,
                p,
                "Construire",
                if f & pf::CREATE_OBJECTS != 0 {
                    "Tout le monde"
                } else if f & pf::CREATE_GROUP_OBJECTS != 0 {
                    "Groupe"
                } else {
                    "Propriétaire"
                },
            );
            row(
                ui,
                p,
                "Scripts",
                if f & pf::ALLOW_OTHER_SCRIPTS != 0 {
                    "Tout le monde"
                } else if f & pf::ALLOW_GROUP_SCRIPTS != 0 {
                    "Groupe"
                } else {
                    "Propriétaire"
                },
            );
            row(ui, p, "Dégâts (non sécurisé)", yes_no(f & pf::ALLOW_DAMAGE != 0));
            row(
                ui,
                p,
                "Poussée",
                if f & pf::RESTRICT_PUSHOBJECT != 0 {
                    "Restreinte"
                } else {
                    "Autorisée"
                },
            );
            row(ui, p, "Terraformer", yes_no(f & pf::ALLOW_TERRAFORM != 0));
            row(ui, p, "Avatars visibles de l'extérieur", yes_no(parcel.see_avatars));
            row(ui, p, "Sons limités à la parcelle", yes_no(f & pf::SOUND_LOCAL != 0));
            row(ui, p, "Voix", yes_no(f & pf::ALLOW_VOICE_CHAT != 0));
            row(
                ui,
                p,
                "Accès",
                if f & (pf::USE_ACCESS_GROUP | pf::USE_ACCESS_LIST) != 0 {
                    "Restreint"
                } else {
                    "Public"
                },
            );
            row(ui, p, "Environnement de parcelle", yes_no(parcel.region_allow_env_override));

            if !parcel.music_url.is_empty() || !parcel.media_url.is_empty() {
                section(ui, p, "Média");
                if !parcel.music_url.is_empty() {
                    row(ui, p, "Musique", parcel.music_url.clone());
                }
                if !parcel.media_url.is_empty() {
                    row(ui, p, "Média", parcel.media_url.clone());
                }
            }
        });
    });
}

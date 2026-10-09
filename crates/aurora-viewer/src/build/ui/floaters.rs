//! Secondary floaters of the build floater: grid options
//! (floater_build_options.xml), « Avancé » weights (LLFloaterObjectWeights),
//! media settings (LLFloaterMediaSettings and its three panels, the
//! whitelist entry dialog), the group picker (LLFloaterGroupPicker), the
//! contents permissions (LLFloaterBulkPermission), an item's properties,
//! the texture picker and the confirmations (notifications.xml).

use super::common::{check, label, row, section, small, spin, spin_i, text};
use super::{Confirm, Env};
use crate::build::{BuildSettings, BuildTool};
use crate::media::entry::MediaEntry;
use crate::theme::Palette;
use crate::ui::widgets::{Floater, flat_button, tabs};
use crate::world::World;
use aurora_net::build::{BuildCmd, perm};
use egui::{RichText, Vec2};

/// The media settings being edited (LLFloaterMediaSettings).
#[derive(Debug, Clone)]
pub struct MediaSettingsState {
    pub entry: MediaEntry,
    pub tab: usize,
    whitelist_sel: Option<usize>,
    /// The « Entrée de la liste blanche » dialog, open with its text.
    whitelist_entry: Option<String>,
}

impl MediaSettingsState {
    /// From the first selected face's entry; a new media autoplays and
    /// scales (EXT-5172 defaults of the panel).
    pub fn new(entry: Option<MediaEntry>) -> MediaSettingsState {
        MediaSettingsState {
            entry: entry.unwrap_or(MediaEntry {
                auto_play: true,
                auto_scale: true,
                width_pixels: 256,
                height_pixels: 256,
                ..Default::default()
            }),
            tab: 0,
            whitelist_sel: None,
            whitelist_entry: None,
        }
    }
}

pub fn secondary(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, env: &mut Env) {
    grid_options(ctx, p, tool, s);
    weights(ctx, p, tool, world);
    media_settings(ctx, p, tool, world, env);
    group_picker(ctx, p, tool, world);
    bulk_perms(ctx, p, tool, world);
    item_properties(ctx, p, tool, world);
    confirm(ctx, p, tool, world, env);
    if let Some(id) = tool.ui.picker.show(ctx, p, world, env.images)
        && let Some(target) = tool.ui.pick_target.take()
    {
        tool.on_texture_picked(world, env, target, id);
    }
    if !tool.ui.picker.open {
        tool.ui.pick_target = None;
    }
    // the picker's previews are wanted images too
    let w: Vec<uuid::Uuid> = tool.ui.picker.wanted_images.drain().collect();
    tool.ui.wanted_images.extend(w);
}

/// floater_build_options.xml (« Options de la règle »).
fn grid_options(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, s: &mut BuildSettings) {
    if !tool.ui.grid_options_open {
        return;
    }
    let mut open = true;
    let pos = ctx.content_rect().left_top() + Vec2::new(352.0, 60.0);
    Floater::new("build_options", "Options de la règle", pos, Vec2::new(260.0, 200.0))
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            row(ui, p, "Unité (mètres)", 150.0, |ui| {
                spin(ui, &mut s.grid_resolution, 0.01, 0.01..=5.0, 3, 70.0, false);
            });
            row(ui, p, "Intersections jusqu'à (mètres)", 150.0, |ui| {
                spin(ui, &mut s.grid_draw_size, 0.5, 1.0..=50.0, 1, 70.0, false);
            });
            row(ui, p, "Précision décimale", 150.0, |ui| {
                let mut d = s.decimal_precision as i32;
                if spin_i(ui, &mut d, 0..=7, 70.0).changed {
                    s.decimal_precision = d as u8;
                }
            });
            row(ui, p, "Angle (degrés)", 150.0, |ui| {
                spin(ui, &mut s.rotation_step, 0.1, 0.01..=359.99, 2, 70.0, false);
            });
            if let Some(v) = check(ui, s.grid_sub_unit, false, "Afficher les graduations intermédiaires", true) {
                s.grid_sub_unit = v;
            }
            ui.horizontal(|ui| {
                label(ui, p, "Opacité :");
                ui.add(egui::Slider::new(&mut s.grid_opacity, 0.0..=1.0).step_by(0.05));
            });
        });
    tool.ui.grid_options_open = open;
}

/// LLFloaterObjectWeights (« Avancé »).
fn weights(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &World) {
    if !tool.ui.weights_open {
        return;
    }
    let mut open = true;
    let pos = ctx.content_rect().left_top() + Vec2::new(352.0, 290.0);
    Floater::new("object_weights", "Avancé", pos, Vec2::new(210.0, 360.0))
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            let none = tool.selection.is_empty();
            let v = |x: Option<String>| if none { "--".to_owned() } else { x.unwrap_or_else(|| "…".into()) };
            let line = |ui: &mut egui::Ui, value: String, name: &str| {
                ui.horizontal(|ui| {
                    ui.add_sized([60.0, 16.0], egui::Label::new(RichText::new(value).size(12.0).color(p.ink)));
                    label(ui, p, name);
                });
            };
            section(ui, p, "SÉLECTIONNÉ");
            line(ui, v(Some(tool.roots(world).len().to_string())), "Objets");
            line(ui, v(Some(tool.sel_prims(world).len().to_string())), "Prims");
            section(ui, p, "POIDS DE LA SÉLECTION");
            let w = match &tool.costs.weights {
                Some(Ok(w)) => Some(*w),
                Some(Err(_)) => None,
                None => None,
            };
            let failed = matches!(tool.costs.weights, Some(Err(_)));
            let f = |x: Option<f32>| if failed { Some("--".into()) } else { x.map(|x| format!("{x:.1}")) };
            line(ui, v(f(w.map(|w| w.download))), "Téléchargement");
            line(ui, v(f(w.map(|w| w.physics))), "Physiques");
            line(ui, v(f(w.map(|w| w.server))), "Serveur");
            line(ui, v(Some(tool.render_cost(world).to_string())), "Affichage");
            section(ui, p, "IMPACTS SUR LE TERRAIN");
            let cap = tool.costs.capacity(world);
            let rezzed = tool.costs.parcel.as_ref().map(|(_, pi)| pi.sim_total_prims);
            line(ui, v(tool.land_impact(world).map(|x| (x as i32).to_string())), "Sélectionné");
            line(ui, v(rezzed.map(|x| x.to_string())), "Rezzé sur le terrain");
            line(ui, v(cap.map(|c| c.1.to_string())), "Capacité restante");
            line(ui, v(cap.map(|c| c.0.to_string())), "Capacité totale");
            section(ui, p, "Rendering Info");
            let (tris, area) = tool.render_info(world);
            line(ui, v(Some(tris.to_string())), "Triangles Shown");
            line(ui, v(Some(format!("{area:.0}"))), "Pixel Area");
        });
    tool.ui.weights_open = open;
}

/// LLFloaterMediaSettings: Général, Personnaliser, Sécurité; OK / Annuler / Appliquer.
fn media_settings(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World, env: &mut Env) {
    let Some(mut st) = tool.ui.media_settings.take() else { return };
    let mut open = true;
    let mut apply = false;
    let mut close = false;
    let pos = ctx.content_rect().center() - Vec2::new(200.0, 220.0);
    Floater::new("media_settings", "Paramètres Multimédia", pos, Vec2::new(400.0, 440.0))
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            tabs(
                ui,
                p,
                &mut st.tab,
                &[("Général", true), ("Personnaliser", true), ("Sécurité", true)],
            );
            ui.add_space(6.0);
            let e = &mut st.entry;
            match st.tab {
                0 => {
                    label(ui, p, "Page d'accueil :");
                    ui.add(
                        egui::TextEdit::singleline(&mut e.home_url)
                            .char_limit(1024)
                            .desired_width(f32::INFINITY),
                    );
                    if !e.home_url.is_empty() && !e.check_url(&e.home_url) {
                        ui.label(
                            RichText::new("(Cette page ne satisfait pas les critères de la liste blanche)")
                                .size(11.0)
                                .color(p.warn),
                        );
                    }
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(200.0, 110.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, p.field);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Prévisualiser",
                        egui::FontId::proportional(12.0),
                        p.muted,
                    );
                    ui.horizontal(|ui| {
                        label(ui, p, "Page actuelle :");
                        text(ui, p, &e.current_url);
                    });
                    if ui
                        .button(RichText::new("Réinitialiser").size(12.0))
                        .on_hover_text("Revenir à la page d'accueil")
                        .clicked()
                    {
                        tool.media_home(world, env, &e.home_url);
                    }
                    if let Some(v) = check(ui, e.auto_loop, false, "Lecture en boucle", true) {
                        e.auto_loop = v;
                    }
                    if let Some(v) = check(ui, e.first_click_interact, false, "Interaction lors du premier clic", true) {
                        e.first_click_interact = v;
                    }
                    if let Some(v) = check(ui, e.auto_zoom, false, "Zoom automatique", true) {
                        e.auto_zoom = v;
                    }
                    if let Some(v) = check(ui, e.auto_play, false, "Jouer automatiquement le média", true) {
                        e.auto_play = v;
                    }
                    small(ui, p, "Note : Les résidents peuvent ignorer ce paramètre");
                    if let Some(v) = check(
                        ui,
                        e.auto_scale,
                        false,
                        "Redimensionnement automatique du média à la taille de la face de l'objet",
                        true,
                    ) {
                        e.auto_scale = v;
                    }
                    ui.horizontal(|ui| {
                        label(ui, p, "Taille :");
                        ui.add_enabled_ui(!e.auto_scale, |ui| {
                            let mut w = e.width_pixels as i32;
                            let mut h = e.height_pixels as i32;
                            spin_i(ui, &mut w, 0..=2048, 60.0);
                            ui.label("X");
                            spin_i(ui, &mut h, 0..=2048, 60.0);
                            e.width_pixels = w as u16;
                            e.height_pixels = h as u16;
                        });
                    });
                }
                1 => {
                    let controls = [("Standard", 0u8), ("Mini", 1)];
                    ui.horizontal(|ui| {
                        label(ui, p, "Contrôles :");
                        if let Some(c) = super::common::combo(ui, "media_controls", Some(e.controls), &controls, 100.0, true) {
                            e.controls = c;
                        }
                    });
                    let group_name = tool
                        .props
                        .values()
                        .next()
                        .and_then(|pr| world.groups.group(&pr.group_id))
                        .map(|g| g.name.clone())
                        .unwrap_or_default();
                    for (title, bit) in [
                        ("Propriétaire".to_owned(), crate::media::entry::PERM_OWNER),
                        (format!("Groupe : {group_name}"), crate::media::entry::PERM_GROUP),
                        ("N'importe qui".to_owned(), crate::media::entry::PERM_ANYONE),
                    ] {
                        section(ui, p, &title);
                        if let Some(v) = check(
                            ui,
                            e.perms_interact & bit != 0,
                            false,
                            "Autoriser la navigation & l'interactivité",
                            true,
                        ) {
                            e.perms_interact = if v { e.perms_interact | bit } else { e.perms_interact & !bit };
                        }
                        if let Some(v) = check(ui, e.perms_control & bit != 0, false, "Afficher la barre de contrôle", true) {
                            e.perms_control = if v { e.perms_control | bit } else { e.perms_control & !bit };
                        }
                    }
                }
                _ => {
                    if let Some(v) = check(
                        ui,
                        e.whitelist_enable,
                        false,
                        "Autoriser l'accès uniquement aux URL spécifiés",
                        true,
                    ) {
                        e.whitelist_enable = v;
                    }
                    egui::Frame::new().fill(p.field).inner_margin(4).show(ui, |ui| {
                        ui.set_min_size(Vec2::new(370.0, 200.0));
                        for (i, w) in e.whitelist.iter().enumerate() {
                            let rejected = !e.home_url.is_empty() && {
                                let probe = MediaEntry {
                                    whitelist_enable: true,
                                    whitelist: vec![w.clone()],
                                    ..Default::default()
                                };
                                !probe.check_url(&e.home_url)
                            };
                            let mut t = RichText::new(w).size(12.0);
                            if rejected {
                                t = t.color(p.danger);
                            }
                            if ui.selectable_label(st.whitelist_sel == Some(i), t).clicked() {
                                st.whitelist_sel = Some(i);
                            }
                        }
                    });
                    small(ui, p, "Les entrées dont la page d'accueil est rejetée sont marquées en rouge.");
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(e.whitelist.len() < 64, egui::Button::new(RichText::new("Ajouter").size(12.0)))
                            .clicked()
                        {
                            st.whitelist_entry = Some(String::new());
                        }
                        if ui
                            .add_enabled(st.whitelist_sel.is_some(), egui::Button::new(RichText::new("Supprimer").size(12.0)))
                            .clicked()
                            && let Some(i) = st.whitelist_sel.take()
                            && i < e.whitelist.len()
                        {
                            e.whitelist.remove(i);
                        }
                    });
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                if flat_button(ui, p, "OK").clicked() {
                    apply = true;
                    close = true;
                }
                if flat_button(ui, p, "Annuler").clicked() {
                    close = true;
                }
                if flat_button(ui, p, "Appliquer").clicked() {
                    apply = true;
                }
            });
        });
    // whitelist entry dialog (floater_whitelist_entry.xml)
    if let Some(mut entry) = st.whitelist_entry.take() {
        let mut keep = true;
        egui::Modal::new(egui::Id::new("whitelist_entry")).show(ctx, |ui| {
            ui.label(RichText::new("Entrée de la liste blanche").size(13.0).color(p.ink));
            small(ui, p, "Saisissez une URL ou un masque d'URL à ajouter à la liste");
            ui.add(egui::TextEdit::singleline(&mut entry).desired_width(300.0));
            ui.horizontal(|ui| {
                if flat_button(ui, p, "OK").clicked() {
                    let t = entry.trim();
                    if !t.is_empty() && st.entry.whitelist.len() < 64 {
                        st.entry.whitelist.push(t.to_owned());
                    }
                    keep = false;
                }
                if flat_button(ui, p, "Annuler").clicked() {
                    keep = false;
                }
            });
        });
        if keep {
            st.whitelist_entry = Some(entry);
        }
    }
    if apply {
        // home_url is required for a new media (selectionSetMedia)
        let entry = (!st.entry.home_url.trim().is_empty()).then(|| st.entry.clone());
        if entry.is_some() {
            tool.apply_media(world, env, entry);
        }
    }
    if open && !close {
        tool.ui.media_settings = Some(st);
    }
}

/// LLFloaterGroupPicker: the agent's groups and « aucun ».
fn group_picker(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World) {
    if !tool.ui.group_picker_open {
        return;
    }
    let mut open = true;
    let mut chosen = None;
    let pos = ctx.content_rect().center() - Vec2::new(130.0, 150.0);
    Floater::new("group_picker", "Choisir un groupe", pos, Vec2::new(260.0, 300.0)).show(ctx, p, &mut open, |ui| {
        egui::ScrollArea::vertical().max_height(230.0).show(ui, |ui| {
            if ui.selectable_label(false, RichText::new("aucun").size(12.0)).double_clicked() {
                chosen = Some(uuid::Uuid::nil());
            }
            let mut groups: Vec<(uuid::Uuid, String)> = world.groups.groups.iter().map(|g| (g.id, g.name.clone())).collect();
            groups.sort_by_key(|g| g.1.to_lowercase());
            for (id, name) in groups {
                if ui.selectable_label(false, RichText::new(name).size(12.0)).double_clicked() {
                    chosen = Some(id);
                }
            }
        });
        small(ui, p, "Double-cliquez sur un groupe.");
    });
    if let Some(g) = chosen {
        tool.set_group(world, g);
        open = false;
    }
    tool.ui.group_picker_open = open;
}

/// LLFloaterBulkPermission for the object's contents.
fn bulk_perms(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World) {
    if !tool.ui.bulk_perms_open {
        return;
    }
    let mut open = true;
    let mut apply = false;
    let pos = ctx.content_rect().center() - Vec2::new(140.0, 140.0);
    let b = &mut tool.ui.bulk;
    Floater::new("bulk_perms", "Droits du contenu", pos, Vec2::new(280.0, 260.0))
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            section(ui, p, "Appliquer à");
            ui.horizontal_wrapped(|ui| {
                for (i, n) in ["Scripts", "Notes", "Textures", "Objets", "Sons", "Animations", "Autres"]
                    .iter()
                    .enumerate()
                {
                    if let Some(v) = check(ui, b.types[i], false, n, true) {
                        b.types[i] = v;
                    }
                }
            });
            section(ui, p, "Droits");
            if let Some(v) = check(ui, b.share, false, "Partager avec le groupe", true) {
                b.share = v;
            }
            if let Some(v) = check(ui, b.everyone_copy, false, "N'importe qui peut copier", true) {
                b.everyone_copy = v;
            }
            label(ui, p, "Le prochain propriétaire peut :");
            ui.horizontal(|ui| {
                if let Some(v) = check(ui, b.next_modify, false, "Modifier", true) {
                    b.next_modify = v;
                }
                if let Some(v) = check(ui, b.next_copy, false, "Copier", true) {
                    b.next_copy = v;
                }
                if let Some(v) = check(ui, b.next_transfer, false, "Transférer", true) {
                    b.next_transfer = v;
                }
            });
            if flat_button(ui, p, "Appliquer").clicked() {
                apply = true;
            }
        });
    tool.ui.bulk_perms_open = open;
    if apply {
        tool.apply_bulk_perms(world);
    }
}

/// Choices of the contents permissions floater (BulkChange* settings).
#[derive(Debug, Clone)]
pub struct BulkPerms {
    /// Scripts, notecards, textures, objects, sounds, animations, others.
    pub types: [bool; 7],
    pub share: bool,
    pub everyone_copy: bool,
    pub next_modify: bool,
    pub next_copy: bool,
    pub next_transfer: bool,
}

impl Default for BulkPerms {
    fn default() -> Self {
        BulkPerms {
            types: [true; 7],
            share: false,
            everyone_copy: false,
            next_modify: false,
            next_copy: true,
            next_transfer: false,
        }
    }
}

impl BuildTool {
    /// LLFloaterBulkPermission::handleInventory: new masks on every
    /// matching item of the selected objects (UpdateTaskInventory).
    fn apply_bulk_perms(&mut self, world: &World) {
        let b = self.ui.bulk.clone();
        let kind = |t: i8| match t {
            10 => 0,
            7 => 1,
            0 => 2,
            6 => 3,
            1 => 4,
            20 => 5,
            _ => 6,
        };
        for idx in self.sel_prims(world) {
            let Some(o) = world.objects.get(idx) else { continue };
            let (handle, local_id, id) = (o.key.region, o.key.local_id, o.full_id);
            if !self.contents_editable(world, idx) {
                continue;
            }
            let items: Vec<_> = self
                .contents
                .objects
                .get(&id)
                .map(|inv| {
                    inv.items
                        .iter()
                        .filter(|i| !i.is_folder && b.types[kind(i.asset_type)])
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            for mut it in items {
                let mut next = perm::MOVE;
                if b.next_modify {
                    next |= perm::MODIFY;
                }
                if b.next_copy {
                    next |= perm::COPY;
                }
                if b.next_transfer {
                    next |= perm::TRANSFER;
                }
                it.next_owner_mask = next;
                it.everyone_mask = if b.everyone_copy { perm::COPY } else { 0 };
                it.group_mask = if b.share { perm::COPY | perm::MODIFY | perm::MOVE } else { 0 };
                self.send(BuildCmd::UpdateTaskInventory {
                    handle,
                    local_id,
                    item: Box::new(it),
                });
            }
        }
        self.status = "Droits du contenu envoyés.".into();
    }
}

/// An item's properties (show_task_item_profile, read only).
fn item_properties(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World) {
    let Some(item) = tool.ui.item_properties.clone() else { return };
    let mut open = true;
    let pos = ctx.content_rect().center() - Vec2::new(150.0, 120.0);
    world.social.want_name(item.creator_id);
    world.social.want_name(item.owner_id);
    Floater::new("task_item_props", "Propriétés de l'élément", pos, Vec2::new(300.0, 240.0))
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            row(ui, p, "Nom :", 90.0, |ui| text(ui, p, &item.name));
            row(ui, p, "Description :", 90.0, |ui| text(ui, p, &item.description));
            row(ui, p, "Créateur :", 90.0, |ui| {
                text(ui, p, &world.social.name_of(&item.creator_id))
            });
            row(ui, p, "Propriétaire :", 90.0, |ui| {
                text(ui, p, &world.social.name_of(&item.owner_id))
            });
            let perms = |m: u32| {
                let mut v = Vec::new();
                for (b, n) in [(perm::MODIFY, "modifier"), (perm::COPY, "copier"), (perm::TRANSFER, "transférer")] {
                    if m & b != 0 {
                        v.push(n);
                    }
                }
                if v.is_empty() { "aucun".into() } else { v.join(", ") }
            };
            row(ui, p, "Vous :", 90.0, |ui| text(ui, p, &perms(item.owner_mask)));
            row(ui, p, "Prochain :", 90.0, |ui| text(ui, p, &perms(item.next_owner_mask)));
        });
    if !open {
        tool.ui.item_properties = None;
    }
}

/// The confirmations, as modal dialogs.
fn confirm(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &mut World, env: &mut Env) {
    let Some(c) = tool.ui.confirm.clone() else { return };
    let (title, body, yes) = match &c {
        Confirm::Deed(_) => (
            "Céder l'objet",
            "Si vous cédez cet objet, le groupe :\n* recevra les L$ versés pour l'objet ;\n* pourra le modifier et le vendre.\nCéder l'objet ?",
            "Céder",
        ),
        Confirm::Divide => (
            "Sous-diviser",
            "Diviser le terrain ? La sélection deviendra une nouvelle parcelle.",
            "Diviser",
        ),
        Confirm::Join => ("Fusionner", "Fusionner les parcelles sélectionnées en une seule ?", "Fusionner"),
        Confirm::Release => (
            "Abandonner le terrain",
            "Abandonner cette parcelle ? Elle reviendra au propriétaire de la région.",
            "Abandonner",
        ),
        Confirm::DeleteMedia => ("Supprimer le média", "Supprimer le média des faces sélectionnées ?", "Supprimer"),
        Confirm::HideWater => (
            "Occulter l'eau",
            "Si vous cochez la case « cacher l'eau », les choix de texture, d'aspérités et de brillance seront écrasés.",
            "Continuer",
        ),
        Confirm::Probe => (
            "Sonde de réflexion",
            "Cette prim devient une sonde de réflexion : elle sera fantôme et transparente.",
            "Continuer",
        ),
    };
    let mut answer = None;
    egui::Modal::new(egui::Id::new("build_confirm")).show(ctx, |ui| {
        ui.set_max_width(320.0);
        ui.label(RichText::new(title).size(14.0).strong().color(p.ink));
        ui.add_space(4.0);
        ui.label(RichText::new(body).size(12.0).color(p.ink));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(RichText::new(yes).size(12.0).color(egui::Color32::WHITE)).fill(p.violet))
                .clicked()
            {
                answer = Some(true);
            }
            if flat_button(ui, p, "Annuler").clicked() {
                answer = Some(false);
            }
        });
    });
    let Some(ok) = answer else { return };
    tool.ui.confirm = None;
    if !ok {
        return;
    }
    match c {
        Confirm::Deed(group) => tool.deed(world, group),
        Confirm::Divide | Confirm::Join => {
            if let Some((handle, west, south, east, north)) = tool.land.selection_local(world) {
                tool.send(if c == Confirm::Divide {
                    BuildCmd::ParcelDivide {
                        handle,
                        west,
                        south,
                        east,
                        north,
                    }
                } else {
                    BuildCmd::ParcelJoin {
                        handle,
                        west,
                        south,
                        east,
                        north,
                    }
                });
            }
        }
        Confirm::Release => {
            let target = tool.land.parcel.clone().or_else(|| {
                let h = world.regions.values().find(|r| r.info.as_ref().is_some_and(|i| i.is_main))?.handle;
                world.parcel.clone().map(|pi| (h, pi))
            });
            if let Some((handle, pi)) = target {
                tool.send(BuildCmd::ParcelRelease {
                    handle,
                    local_id: pi.local_id,
                });
                tool.land.selection = None;
            }
        }
        Confirm::DeleteMedia => tool.apply_media(world, env, None),
        Confirm::HideWater => {
            // FSPanelFace::onCommitHideWater
            let sel: Vec<(usize, u8)> = tool.sel_te(world);
            tool.edit_faces(world, |f, _, _| {
                f.texture = uuid::Uuid::from_u128(0xe97cf410_8e61_7005_ec06_629eba4cd1fb);
                f.bump_shiny_fullbright &= 0x20;
                f.color[3] = 1.0;
            });
            let puts = sel.into_iter().map(|(i, f)| (i, f, None)).collect();
            tool.legacy_put(world, puts);
        }
        Confirm::Probe => {
            if let Some(idx) = tool.selection.first().and_then(|k| world.objects.index_of(k)) {
                tool.make_probe(world, idx);
                if !tool.ui.probe_warned {
                    tool.status = "Les sondes de réflexion ne se sélectionnent que si « Sél. les sondes de réfl. » est cochée.".into();
                    tool.ui.probe_warned = true;
                }
            }
        }
    }
}

impl BuildTool {
    /// « Réinitialiser » of the media settings: the selected faces' media
    /// go back to the home page (LLPanelMediaSettingsGeneral::onBtnResetCurrentUrl).
    fn media_home(&mut self, world: &World, env: &mut Env, home: &str) {
        for (idx, face) in self.sel_te(world) {
            let Some(id) = world.objects.get(idx).map(|o| o.full_id) else {
                continue;
            };
            if let Some(m) = env.media.find_mut(&crate::media::MediaKey::Prim { object: id, face }) {
                m.navigate(home);
            }
        }
    }

    /// « Affichage » of the weights (LLSelectMgr::getSelectedObjectRenderCost,
    /// coarse: triangles of the selection's volumes / 10, textures 1 each).
    pub fn render_cost(&self, world: &World) -> u32 {
        let (tris, _) = self.render_info(world);
        let textures: std::collections::HashSet<uuid::Uuid> = self
            .sel_prims(world)
            .into_iter()
            .filter_map(|i| world.objects.get(i)?.te.clone())
            .flat_map(|te| te.faces.iter().map(|f| f.texture).collect::<Vec<_>>())
            .collect();
        (tris / 10) as u32 + textures.len() as u32
    }

    /// Rendering info of the weights floater: triangles and screen area.
    pub fn render_info(&self, world: &World) -> (usize, f32) {
        let mut tris = 0;
        let mut area = 0.0;
        for i in self.sel_prims(world) {
            let Some(o) = world.objects.get(i) else { continue };
            if !o.volume.is_mesh() {
                tris += aurora_prim::volume::num_faces(&o.volume) * 2 * 16;
            }
            if let Some((pos, _, _)) = crate::scene::Scene::object_transform(world, i, std::time::Instant::now(), 0) {
                let d = pos.distance(self.cam.eye).max(0.1);
                let r = o.scale.length() * 0.5 * self.cam.pixel_meter_ratio() / d;
                area += std::f32::consts::PI * r * r;
            }
        }
        (tris, area)
    }
}

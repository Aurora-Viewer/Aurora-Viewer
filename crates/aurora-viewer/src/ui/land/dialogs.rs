//! The questions and small floaters of About Land: confirmations
//! (ReleaseLandWarning, DeedLandToGroup, LandBuyPass, ReturnObjects*,
//! SettingsConfirmReset), LLFloaterGroupPicker, LLFloaterSellLand,
//! LLFloaterBanDuration and LLFloaterURLEntry (French texts of
//! notifications.xml and the floaters' XML).

use super::{Dialog, LandUi, PickFor, View, dim};
use crate::theme::Palette;
use crate::world::World;
use crate::world::notifications::{Data, Kind};
use aurora_net::land::{LandCommand, ObjectOwner, RT_GROUP, RT_LIST, RT_OTHER, RT_OWNER};
use aurora_net::{ParcelInfo, parcel_flags as pf};
use egui::{RichText, Vec2};
use uuid::Uuid;

pub(super) enum Confirm {
    Deed,
    Release,
    BuyPass,
    ReturnOwner,
    ReturnGroup,
    ReturnOther,
    ReturnList(ObjectOwner),
    ResetEnvironment,
}

/// LLFloaterSellLand being filled.
pub(super) struct SellForm {
    price: String,
    /// sell_to: 0 "- Sélectionnez -", 1 anyone, 2 a specific resident.
    to: u8,
    pub(super) buyer: Option<Uuid>,
    with_objects: bool,
    /// The confirmation (ConfirmLandSale[ToAnyone]Change) is shown.
    confirming: bool,
}

impl SellForm {
    pub(super) fn new(p: &ParcelInfo) -> SellForm {
        SellForm {
            price: if p.sale_price > 0 { p.sale_price.to_string() } else { "0".into() },
            to: if p.auth_buyer.is_nil() { 0 } else { 2 },
            buyer: (!p.auth_buyer.is_nil()).then_some(p.auth_buyer),
            with_objects: p.flags & pf::SELL_PARCEL_OBJECTS != 0,
            confirming: false,
        }
    }
}

fn group_name(world: &mut World, id: &Uuid) -> String {
    let groups = world.groups.groups.clone();
    world.land.group_name(id, &groups).unwrap_or_else(|| "…".into())
}

fn info(world: &mut World, body: String) {
    world.notifications.push(Kind::Info, "À propos du terrain", body, Data::None);
}

/// The text of a confirmation, and what OK does.
fn question(c: &Confirm, parcel: &ParcelInfo, world: &mut World) -> String {
    let me = world.agent_id;
    match c {
        Confirm::Release => format!(
            "Vous vous apprêtez à libérer {} m² de terrain. Si vous libérez cette parcelle, elle sera supprimée de votre patrimoine, mais vous ne recevrez pas de L$.\n\nLibérer ce terrain ?",
            parcel.area
        ),
        Confirm::Deed => {
            let group = group_name(world, &parcel.group_id);
            let contribution = if parcel.flags & pf::CONTRIBUTE_WITH_DEED != 0 {
                format!(
                    " Elle inclura une contribution simultanée au groupe de la part de '{}'.",
                    world.social.name_of(&parcel.owner_id)
                )
            } else {
                String::new()
            };
            format!(
                "La cession de cette parcelle requiert que le groupe dispose en permanence d'un crédit suffisant pour payer les frais d'occupation de terrain.{contribution} Le prix d'achat du terrain n'est pas remboursé au propriétaire. Si une parcelle cédée est vendue, son prix de vente est redistribué à part égale entre les membres du groupe.\n\nCéder ces {} m² de terrain au groupe '{group}' ?",
                parcel.area
            )
        }
        Confirm::BuyPass => format!(
            "Pour {} L$ vous pouvez pénétrer sur ce terrain ({}) et y rester {:.2} heures.\n\nAcheter un pass ?",
            parcel.pass_price, parcel.name, parcel.pass_hours
        ),
        Confirm::ReturnOwner if parcel.owner_id == me => format!(
            "Êtes-vous certain de vouloir renvoyer tous les objets que vous possédez sur cette parcelle dans votre inventaire ?\n\nObjets : {}",
            parcel.owner_prims
        ),
        Confirm::ReturnOwner => format!(
            "Êtes-vous certain de vouloir renvoyer tous les objets que {} possède sur cette parcelle dans son inventaire ?\n\nObjets : {}",
            world.social.name_of(&parcel.owner_id),
            parcel.owner_prims
        ),
        Confirm::ReturnGroup => format!(
            "Êtes-vous certain de vouloir renvoyer tous les objets partagés par le groupe '{}' sur cette parcelle de terrain dans l'inventaire du propriétaire précédent ?\n\n*AVERTISSEMENT* Cela supprimera les objets non transférables cédés au groupe !\n\nObjets : {}",
            group_name(world, &parcel.group_id),
            parcel.group_prims
        ),
        Confirm::ReturnOther if parcel.is_group_owned => format!(
            "Renvoyer les objets de cette parcelle qui ne sont PAS partagés avec le groupe {} à leur propriétaire ?\n\nObjets : {}",
            group_name(world, &parcel.group_id),
            parcel.other_prims
        ),
        Confirm::ReturnOther if parcel.owner_id == me => format!(
            "Êtes-vous certain de vouloir renvoyer tous les objets que vous ne possédez pas sur cette parcelle dans l'inventaire de leur propriétaire ? Les objets transférables cédés à un groupe seront renvoyés aux propriétaires précédents.\n\n*Avertissement* Tous les objets non transférables cédés au groupe seront supprimés !\n\nObjets : {}",
            parcel.other_prims
        ),
        Confirm::ReturnOther => format!(
            "Êtes-vous certain de vouloir renvoyer tous les objets que {} ne possède pas sur cette parcelle dans l'inventaire de leur propriétaire ? Les objets transférables cédés à un groupe seront renvoyés aux propriétaires précédents.\n\n*Avertissement* Tous les objets non transférables cédés au groupe seront supprimés !\n\nObjets : {}",
            world.social.name_of(&parcel.owner_id),
            parcel.other_prims
        ),
        Confirm::ReturnList(o) if o.is_group => format!(
            "Êtes-vous certain de vouloir renvoyer tous les objets partagés par le groupe '{}' sur cette parcelle de terrain dans l'inventaire du propriétaire précédent ?\n\n*AVERTISSEMENT* Cela supprimera les objets non transférables cédés au groupe !\n\nObjets : {}",
            group_name(world, &o.id),
            o.count
        ),
        Confirm::ReturnList(o) => format!(
            "Êtes-vous certain de vouloir renvoyer tous les objets que {} possède sur cette parcelle dans son inventaire ?\n\nObjets : {}",
            world.social.name_of(&o.id),
            o.count
        ),
        Confirm::ResetEnvironment => {
            "Vous êtes sur le point de supprimer tous les paramètres appliqués. Voulez-vous vraiment continuer ?".into()
        }
    }
}

/// What OK does (the panels' callbacks).
fn accept(c: Confirm, parcel: &ParcelInfo, world: &mut World) {
    let me = world.agent_id;
    let returned = |world: &mut World, return_type: u32, owners: Vec<Uuid>| {
        world.land.parcel_command(|handle, local_id| LandCommand::ReturnObjects {
            handle,
            local_id,
            return_type,
            owners,
        });
        // the callbacks send the parcel again so its counts refresh
        world.land.update(|_| {});
    };
    match c {
        Confirm::Release => world
            .land
            .parcel_command(|handle, local_id| LandCommand::Release { handle, local_id }),
        Confirm::Deed => {
            let group = parcel.group_id;
            world
                .land
                .parcel_command(|handle, local_id| LandCommand::DeedToGroup { handle, local_id, group });
        }
        Confirm::BuyPass => world
            .land
            .parcel_command(|handle, local_id| LandCommand::BuyPass { handle, local_id }),
        Confirm::ReturnOwner => {
            let body = if parcel.owner_id == me {
                "Les objets que vous possédez sur la parcelle de terrain sélectionnée ont été renvoyés dans votre inventaire.".to_owned()
            } else {
                format!(
                    "Les objets de la parcelle de terrain sélectionnée appartenant à {} ont été renvoyés vers son inventaire.",
                    world.social.name_of(&parcel.owner_id)
                )
            };
            info(world, body);
            returned(world, RT_OWNER, Vec::new());
        }
        Confirm::ReturnGroup => {
            let body = format!(
                "Les objets sélectionnés sur la parcelle de terrain partagée avec le groupe {} ont été renvoyés dans l'inventaire de leur propriétaire.",
                group_name(world, &parcel.group_id)
            );
            info(world, body);
            returned(world, RT_GROUP, Vec::new());
        }
        Confirm::ReturnOther => {
            info(
                world,
                "Les objets sélectionnés sur la parcelle et qui ne sont pas à vous ont été rendus à leurs propriétaires.".into(),
            );
            returned(world, RT_OTHER, Vec::new());
        }
        Confirm::ReturnList(o) => {
            let body = if o.is_group {
                format!(
                    "Les objets sélectionnés sur la parcelle de terrain partagée avec le groupe {} ont été renvoyés dans l'inventaire de leur propriétaire.",
                    group_name(world, &o.id)
                )
            } else {
                format!(
                    "Les objets sur la parcelle de terrain sélectionnée appartenant au résident {} ont été rendus à leur propriétaire.",
                    world.social.name_of(&o.id)
                )
            };
            info(world, body);
            returned(world, RT_LIST, vec![o.id]);
        }
        Confirm::ResetEnvironment => world.land.environment(|local_id| LandCommand::EnvironmentReset { local_id }),
    }
}

/// A fixed, centered dialog window.
fn window(ctx: &egui::Context, p: &Palette, title: &str, width: f32, body: impl FnOnce(&mut egui::Ui)) -> bool {
    let mut open = true;
    let screen = ctx.content_rect();
    super::super::widgets::Floater::new(
        "land_dialog",
        title,
        screen.center() - Vec2::new(width / 2.0, 110.0),
        Vec2::new(width, 160.0),
    )
    .fixed()
    .show(ctx, p, &mut open, body);
    open
}

/// OK / Annuler (or Oui / Non) at the bottom right; Some(true) for OK.
fn buttons(ui: &mut egui::Ui, p: &Palette, ok: &str, cancel: &str, ok_enabled: bool) -> Option<bool> {
    let mut out = None;
    ui.add_space(6.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if super::super::widgets::flat_button(ui, p, cancel).clicked() {
            out = Some(false);
        }
        if super::button(ui, p, ok, ok_enabled).clicked() {
            out = Some(true);
        }
    });
    out
}

pub(super) fn show(ctx: &egui::Context, p: &Palette, s: &mut LandUi, v: &View, world: &mut World) {
    let Some(dialog) = s.dialog.take() else {
        return;
    };
    let parcel = v.parcel.clone();
    let next = match dialog {
        Dialog::Message(msg) => {
            let mut done = false;
            let open = window(ctx, p, "À propos du terrain", 380.0, |ui| {
                ui.add(egui::Label::new(RichText::new(&msg).size(12.5).color(p.ink)).wrap());
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    done = super::super::widgets::flat_button(ui, p, "OK").clicked();
                });
            });
            (open && !done).then_some(Dialog::Message(msg))
        }
        Dialog::Confirm(c) => {
            let Some(parcel) = parcel else {
                return;
            };
            let text = question(&c, &parcel, world);
            let (ok, cancel) = if matches!(c, Confirm::ResetEnvironment) {
                ("Oui", "Non")
            } else {
                ("OK", "Annuler")
            };
            let mut answer = None;
            let open = window(ctx, p, "À propos du terrain", 420.0, |ui| {
                ui.add(egui::Label::new(RichText::new(&text).size(12.5).color(p.ink)).wrap());
                answer = buttons(ui, p, ok, cancel, true);
            });
            match answer {
                Some(true) => {
                    accept(c, &parcel, world);
                    None
                }
                Some(false) => None,
                None => open.then_some(Dialog::Confirm(c)),
            }
        }
        Dialog::GroupPicker => {
            let mut keep = true;
            let open = window(ctx, p, "Groupes", 300.0, |ui| {
                ui.label(RichText::new("Choisissez un groupe :").size(12.0).color(p.muted));
                let groups = world.groups.sorted();
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    if groups.is_empty() {
                        ui.label(RichText::new("Vous n'êtes membre d'aucun groupe.").size(12.0).color(p.muted));
                    }
                    for g in groups {
                        if ui.selectable_label(false, RichText::new(&g.name).size(12.0)).clicked() {
                            // setGroup
                            let id = g.id;
                            world.land.update(|u| u.group_id = id);
                            keep = false;
                        }
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if super::super::widgets::flat_button(ui, p, "Annuler").clicked() {
                        keep = false;
                    }
                });
            });
            (open && keep).then_some(Dialog::GroupPicker)
        }
        Dialog::BanDuration {
            ids,
            mut temporary,
            mut hours,
        } => {
            let mut answer = None;
            let open = window(ctx, p, "Durée de l'interdiction", 300.0, |ui| {
                ui.label(RichText::new("Durée de l'interdiction :").size(12.0).color(p.muted));
                ui.radio_value(&mut temporary, false, "Toujours");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut temporary, true, "Temporaire");
                    ui.add_enabled(temporary, egui::DragValue::new(&mut hours).range(1..=8766));
                    dim(ui, p, "heures");
                });
                answer = buttons(ui, p, "OK", "Annuler", true);
            });
            match answer {
                Some(true) => {
                    // onClickBan: expiry time, 0 = always
                    let time = if temporary { (v.now + hours as i64 * 3600) as i32 } else { 0 };
                    super::access::add_banned(world, &ids, time);
                    None
                }
                Some(false) => None,
                None => open.then_some(Dialog::BanDuration { ids, temporary, hours }),
            }
        }
        Dialog::Sell(mut form) => {
            let Some(parcel) = parcel else {
                return;
            };
            let mut close = false;
            let open = window(ctx, p, "Vendre le terrain", 420.0, |ui| {
                sell_form(ui, p, s, world, &parcel, &mut form, &mut close)
            });
            (open && !close).then_some(Dialog::Sell(form))
        }
        Dialog::MediaUrl(mut url) => {
            let mut close = false;
            let waiting = world.land.media_type.as_ref().is_some_and(|m| m.1.is_none());
            // headerFetchComplete: the type found (or "none/none"), then the URL
            if let Some((u, Some(mime))) = world.land.media_type.take() {
                world.land.update(|up| {
                    up.media_type = mime;
                    up.media_url = u.clone();
                    up.media_current_url = u;
                });
                return;
            }
            let open = window(ctx, p, "URL du média", 420.0, |ui| {
                ui.horizontal(|ui| {
                    dim(ui, p, "URL :");
                    ui.add_enabled(!waiting, egui::TextEdit::singleline(&mut url).desired_width(f32::INFINITY));
                });
                if waiting {
                    dim(ui, p, "Chargement...");
                }
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if super::button(ui, p, "Annuler", !waiting).clicked() {
                        close = true;
                    }
                    if super::button(ui, p, "Effacer", !waiting).clicked() {
                        url.clear();
                    }
                    if super::button(ui, p, "OK", !waiting).clicked() {
                        world.land.find_media_type(&url);
                    }
                });
            });
            (open && !close).then_some(Dialog::MediaUrl(url))
        }
    };
    // a dialog may have opened another one (the sale's price restriction)
    if s.dialog.is_none() {
        s.dialog = next;
    }
}

/// LLFloaterSellLandUI: price, buyer, objects, then the confirmation.
fn sell_form(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, world: &mut World, parcel: &ParcelInfo, f: &mut SellForm, close: &mut bool) {
    let price: i32 = f.price.trim().parse().unwrap_or(-1);
    let buyer_name = f.buyer.map(|id| world.social.name_of(&id));
    if f.confirming {
        let name = if f.to == 1 {
            "Tout le monde".to_owned()
        } else {
            buyer_name.clone().unwrap_or_default()
        };
        let text = if f.to == 1 {
            format!(
                "ATTENTION : en cliquant sur Vendre à n'importe qui, vous rendez votre terrain disponible à toute la communauté de Second Life, même aux personnes qui ne sont pas dans cette région.\n\nLe terrain sélectionné, de {} m², est mis en vente. Votre prix de vente sera de {price} L$ et la vente sera disponible à {name}.",
                parcel.area
            )
        } else {
            format!(
                "Le terrain sélectionné, de {} m², est mis en vente. Votre prix de vente sera de {price} L$ et la vente sera disponible à {name}.",
                parcel.area
            )
        };
        ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.ink)).wrap());
        match buttons(ui, p, "OK", "Annuler", true) {
            Some(true) => {
                // onConfirmSale
                let (to_user, buyer, with_objects) = (f.to == 2, f.buyer.unwrap_or_default(), f.with_objects);
                world.land.update(|u| {
                    u.flags |= pf::FOR_SALE;
                    u.flags = super::general::set(u.flags, pf::SELL_PARCEL_OBJECTS, with_objects);
                    u.sale_price = price;
                    u.auth_buyer = if to_user { buyer } else { Uuid::nil() };
                });
                *close = true;
            }
            Some(false) => f.confirming = false,
            None => {}
        }
        return;
    }
    super::row(ui, p, "Terrain :", |ui| super::text(ui, p, parcel.name.clone()));
    super::row(ui, p, "Surface :", |ui| super::text(ui, p, format!("{} m²", parcel.area)));
    ui.add_space(4.0);
    ui.label(RichText::new("1. Définissez un prix :").size(12.5).color(p.ink));
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut f.price).desired_width(90.0));
        f.price.retain(|c| c.is_ascii_digit());
        dim(ui, p, "L$");
        if price >= 0 && parcel.area > 0 {
            dim(ui, p, format!("({:.2} L$ par m²)", price as f32 / parcel.area as f32));
        }
    });
    ui.label(RichText::new("2. Vendez le terrain à :").size(12.5).color(p.ink));
    ui.horizontal(|ui| {
        let label = match f.to {
            1 => "Tout le monde",
            2 => "Personne spécifique :",
            _ => "- Sélectionnez -",
        };
        egui::ComboBox::from_id_salt("land_sell_to")
            .selected_text(label)
            .width(160.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut f.to, 1, "Tout le monde");
                ui.selectable_value(&mut f.to, 2, "Personne spécifique :");
            });
        if f.to == 2 {
            if let Some(n) = &buyer_name {
                super::text(ui, p, n.clone());
            }
            if super::super::widgets::flat_button(ui, p, "Sélectionnez").clicked() {
                s.pick_for = Some(PickFor::SellTo);
                s.picker.open("land_avatar_picker", false);
            }
        }
    });
    ui.label(RichText::new("3. Vendre les objets avec le terrain ?").size(12.5).color(p.ink));
    ui.radio_value(&mut f.with_objects, false, "Non, garder la propriété des objets");
    ui.radio_value(&mut f.with_objects, true, "Oui, vendre les objets avec le terrain");
    ui.label(RichText::new("ATTENTION : Toute vente est définitive.").size(12.0).color(p.warn));
    let ready = price >= 0 && (f.to == 1 || (f.to == 2 && f.buyer.is_some()));
    match buttons(ui, p, "Mettre ce terrain en vente", "Annuler", ready) {
        Some(true) => {
            // a sale to anyone needs a price (SalePriceRestriction)
            if parcel.flags & pf::FOR_SALE == 0 && price == 0 && f.to == 1 {
                s.dialog = Some(Dialog::Message(
                    "Pour rendre l'annonce disponible à tous, le prix de vente doit être supérieur à 0 L$. Si le prix de vente est de 0 L$, vous devez choisir un acheteur spécifique."
                        .into(),
                ));
                *close = true;
            } else {
                f.confirming = true;
            }
        }
        Some(false) => *close = true,
        None => {}
    }
}

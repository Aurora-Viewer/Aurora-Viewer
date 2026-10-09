//! "Général" (LLPanelLandGeneral::refresh / onCommitAny).

use super::dialogs::{Confirm, SellForm};
use super::{Dialog, LandUi, NOT_YET, View, button, check, dim, row, text};
use crate::theme::Palette;
use crate::world::World;
use crate::world::land::{can_agent_buy, powers};
use aurora_net::land::{LandCommand, OS_LEASE_PENDING, OS_LEASED};
use aurora_net::{parcel_flags as pf, region_flags as rf};
use egui::RichText;

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World) {
    let Some(parcel) = v.parcel.clone() else {
        super::no_selection(ui, p);
        return;
    };
    let agent = v.agent;
    let is_public = parcel.owner_id.is_nil();
    let is_leased = parcel.status == OS_LEASED;
    let region_owner = v.region.as_ref().is_some_and(|r| r.owner == agent);
    let estate_manager = v.region.as_ref().is_some_and(|r| r.estate_manager);
    let can_identity = v.can(powers::LAND_CHANGE_IDENTITY);

    row(ui, p, "Nom :", |ui| {
        s.name.sync(&parcel.name);
        let r = ui.add_enabled(
            can_identity,
            egui::TextEdit::singleline(&mut s.name.text).desired_width(f32::INFINITY),
        );
        // validateASCIIPrintableNoPipe, on what is typed (a name set
        // elsewhere keeps its other characters)
        if r.changed() {
            let base = s.name.base.clone();
            s.name.text.retain(|c| ((' '..='~').contains(&c) && c != '|') || base.contains(c));
        }
        if let Some(name) = s.name.after(&r) {
            world.land.update(|u| u.name = name);
        }
    });
    row(ui, p, "ID terrain :", |ui| {
        let id = match world.land.sel.as_ref().and_then(|s| s.parcel_uuid) {
            Some(Some(id)) => id.to_string(),
            Some(None) => "Impossible de résoudre l'ID de la parcelle.".into(),
            None => String::new(),
        };
        let mut id = id;
        ui.add_enabled(false, egui::TextEdit::singleline(&mut id).desired_width(f32::INFINITY));
    });
    ui.horizontal_top(|ui| {
        super::key(ui, p, "Description :");
        s.desc.sync(&parcel.desc);
        let r = ui.add_enabled(
            can_identity,
            egui::TextEdit::multiline(&mut s.desc.text)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
        if let Some(desc) = s.desc.after(&r) {
            world.land.update(|u| u.desc = desc);
        }
    });
    row(ui, p, "Type :", |ui| {
        text(ui, p, super::product_name(v.region.as_ref().map_or("", |r| r.product.as_str())));
    });
    row(ui, p, "Catégorie :", |ui| super::maturity(ui, p, v));
    row(ui, p, "Propriétaire :", |ui| {
        if is_public {
            dim(ui, p, "(public)");
        } else if parcel.is_group_owned {
            text(ui, p, "(propriété du groupe)");
        } else {
            super::agent_link(ui, p, world, parcel.owner_id);
        }
        if parcel.status == OS_LEASE_PENDING {
            text(ui, p, "(vente en cours)");
        }
    });
    row(ui, p, "Groupe :", |ui| {
        let name = if is_public || parcel.group_id.is_nil() {
            "(aucun)".to_owned()
        } else {
            let groups = world.groups.groups.clone();
            world
                .land
                .group_name(&parcel.group_id, &groups)
                .unwrap_or_else(|| "Chargement...".into())
        };
        text(ui, p, name);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Set...: our own land only, not group owned (GP_NO_POWERS)
            if button(ui, p, "Choisir", v.can(powers::NONE) && !parcel.is_group_owned).clicked() {
                s.dialog = Some(Dialog::GroupPicker);
            }
        });
    });
    // deeding: our land with a group we are in; doing it needs GP_LAND_DEED
    let enable_deed = parcel.owner_id == agent && !parcel.group_id.is_nil() && v.rights().in_group(&parcel.group_id);
    let allow_deed = parcel.flags & pf::ALLOW_DEED_TO_GROUP != 0;
    ui.horizontal(|ui| {
        ui.add_space(super::KEY_W);
        let (r, on) = check(ui, enable_deed, allow_deed, "Autoriser la cession au groupe");
        r.clone().on_hover_text(
            "Un officier du groupe peut céder ce terrain à ce groupe, afin qu'il soit pris en charge par l'allocation de terrains du groupe.",
        );
        if r.changed() {
            world.land.update(|u| u.flags = set(u.flags, pf::ALLOW_DEED_TO_GROUP, on));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let can_deed = v.rights().has_power(&parcel.group_id, powers::LAND_DEED);
            if button(ui, p, "Céder", allow_deed && !parcel.group_id.is_nil() && can_deed && !parcel.is_group_owned)
                .on_hover_text("Vous ne pouvez céder le terrain que si vous avez un rôle d'officier dans le groupe sélectionné.")
                .clicked()
            {
                s.dialog = Some(Dialog::Confirm(Confirm::Deed));
            }
        });
    });
    ui.horizontal(|ui| {
        ui.add_space(super::KEY_W);
        let (r, on) = check(
            ui,
            enable_deed && allow_deed,
            parcel.flags & pf::CONTRIBUTE_WITH_DEED != 0,
            "Le propriétaire contribue en cédant du terrain",
        );
        r.clone()
            .on_hover_text("Lorsqu'un terrain est cédé au groupe, l'ancien propriétaire fait également un don de terrain suffisant.");
        if r.changed() {
            world.land.update(|u| u.flags = set(u.flags, pf::CONTRIBUTE_WITH_DEED, on));
        }
    });

    // sale
    let region_xfer = !v.region_flag(rf::BLOCK_LAND_RESELL);
    let estate_sellable = parcel.auction_id == 0 && estate_manager && v.region.as_ref().is_some_and(|r| parcel.owner_id == r.owner);
    let owner_sellable = region_xfer && parcel.auction_id == 0 && v.can(powers::LAND_SET_SALE_INFO);
    let can_be_sold = owner_sellable || estate_sellable;
    let for_sale = parcel.flags & pf::FOR_SALE != 0;
    row(ui, p, "À vendre :", |ui| {
        if for_sale {
            let per_m = if parcel.area > 0 {
                parcel.sale_price as f32 / parcel.area as f32
            } else {
                0.0
            };
            text(ui, p, format!("Prix : {} L$ ({per_m:.1} L$/m²)", super::money(parcel.sale_price)));
        } else {
            text(ui, p, "Pas à vendre");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if can_be_sold && for_sale && button(ui, p, "Annuler la vente du terrain", true).clicked() {
                // onClickStopSellLand
                world.land.update(|u| {
                    u.flags &= !pf::FOR_SALE;
                    u.sale_price = 0;
                    u.auth_buyer = uuid::Uuid::nil();
                });
            }
            if can_be_sold && !for_sale && button(ui, p, "Vendre le terrain", true).clicked() {
                s.dialog = Some(Dialog::Sell(SellForm::new(&parcel)));
            }
        });
    });
    if for_sale {
        ui.horizontal(|ui| {
            ui.add_space(super::KEY_W);
            ui.label(RichText::new("À vendre à :").size(12.0).color(p.ink));
            if parcel.auth_buyer.is_nil() {
                text(ui, p, "Tout le monde");
            } else {
                super::agent_link(ui, p, world, parcel.auth_buyer);
            }
        });
        ui.horizontal(|ui| {
            ui.add_space(super::KEY_W);
            text(
                ui,
                p,
                if parcel.flags & pf::SELL_PARCEL_OBJECTS != 0 {
                    "Objets inclus dans la vente"
                } else {
                    "Objets non inclus dans la vente"
                },
            );
        });
    }
    // SalePending: our purchase to approve, or the auction
    let pending = if is_public {
        String::new()
    } else if !is_leased && parcel.owner_id == agent {
        "Pour modifier ce terrain, vous devez approuver votre achat.".into()
    } else if parcel.auction_id != 0 {
        format!("Code de l'enchère : {}", parcel.auction_id)
    } else {
        String::new()
    };
    if !pending.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(super::KEY_W);
            ui.label(RichText::new(pending).size(12.0).color(p.warn));
        });
    }
    row(ui, p, "Acquis :", |ui| {
        if !is_public {
            let r = text(ui, p, super::claim_date(parcel.claim_date as i64));
            if !is_leased {
                r.on_hover_text("Achat en attente d'approbation");
            }
        }
    });
    row(ui, p, "Surface :", |ui| text(ui, p, format!("{} m²", parcel.area)));
    row(ui, p, "Trafic :", |ui| {
        let dwell = world.land.sel.as_ref().and_then(|s| s.dwell);
        text(ui, p, dwell.map_or("Chargement...".into(), |d| format!("{d:.0}")));
    });

    // buttons (bottom of floater_about_land.xml)
    ui.add_space(6.0);
    let can_access = true;
    let buy = can_agent_buy(&parcel, &v.rights(), None, can_access);
    let buy_group = false;
    let release = v.owns(powers::LAND_RELEASE) || (estate_manager && v.region.as_ref().is_some_and(|r| parcel.owner_id != r.owner));
    // a pass: someone else's land selling passes (not while banned)
    let use_pass = parcel.owner_id != agent && parcel.flags & pf::USE_PASS_LIST != 0;
    ui.horizontal(|ui| {
        button(ui, p, "Acheter le terrain", false).on_disabled_hover_text(if buy {
            NOT_YET
        } else {
            "Ce terrain n'est pas à vendre pour vous."
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if region_owner {
                // reclaimParcel: the region owner takes back someone's parcel
                if button(ui, p, "Récupérer le terrain", !is_public && parcel.owner_id != agent).clicked() {
                    world
                        .land
                        .parcel_command(|handle, local_id| LandCommand::Reclaim { handle, local_id });
                }
            } else if button(ui, p, "Abandonner le terrain", release).clicked() {
                s.dialog = Some(Dialog::Confirm(Confirm::Release));
            }
        });
    });
    ui.horizontal(|ui| {
        if v.region.as_ref().is_some_and(|r| r.has_land_resources) {
            button(ui, p, "Infos sur les scripts", false).on_disabled_hover_text(NOT_YET);
        }
        if button(ui, p, "Acheter un pass", use_pass)
            .on_hover_text("Un pass vous donne un accès temporaire à ce terrain.")
            .clicked()
        {
            s.dialog = Some(Dialog::Confirm(Confirm::BuyPass));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            button(ui, p, "Acheter pour le groupe", buy_group).on_disabled_hover_text(NOT_YET);
        });
    });
}

/// LLParcel::setParcelFlag.
pub(super) fn set(flags: u32, flag: u32, on: bool) -> u32 {
    if on { flags | flag } else { flags & !flag }
}

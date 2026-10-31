//! "General" (LLPanelLandGeneral::refresh / onCommitAny).

use super::dialogs::{Confirm, SellForm};
use super::{Dialog, LandUi, View, button, check, dim, row, text};
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

    crate::ui::widgets::section_header(ui, p, "file-info-filled", aurora_i18n::tr!("workspace-land-identity"));
    row(ui, p, aurora_i18n::tr!("common-name-2"), |ui| {
        s.name.sync(&parcel.name);
        let r = crate::ui::widgets::text_field(ui, can_identity, &mut s.name.text, |b, _| {
            egui::TextEdit::singleline(b).desired_width(f32::INFINITY)
        });
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
    row(ui, p, aurora_i18n::tr!("ui-land-general-land-id"), |ui| {
        let id = match world.land.sel.as_ref().and_then(|s| s.parcel_uuid) {
            Some(Some(id)) => id.to_string(),
            Some(None) => aurora_i18n::tr!("ui-land-general-could-not-resolve-parcel-id").into(),
            None => String::new(),
        };
        let mut id = id;
        crate::ui::widgets::text_field(ui, false, &mut id, |b, _| {
            egui::TextEdit::singleline(b).desired_width(f32::INFINITY)
        });
    });
    ui.horizontal_top(|ui| {
        super::key(ui, p, aurora_i18n::tr!("common-description"));
        s.desc.sync(&parcel.desc);
        let mut out = ui
            .add_enabled_ui(can_identity, |ui| {
                egui::TextEdit::multiline(&mut s.desc.text)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .show(ui)
            })
            .inner;
        // spell checked, as floater_about_land.xml
        if can_identity {
            crate::ui::spelling::field(ui, p, &mut out, &mut s.desc.text, false);
        }
        if let Some(desc) = s.desc.after(&out.response) {
            world.land.update(|u| u.desc = desc);
        }
    });
    row(ui, p, aurora_i18n::tr!("common-type-2"), |ui| {
        text(ui, p, super::product_name(v.region.as_ref().map_or("", |r| r.product.as_str())));
    });
    row(ui, p, aurora_i18n::tr!("common-category"), |ui| super::maturity(ui, p, v));
    row(ui, p, aurora_i18n::tr!("common-owner-2"), |ui| {
        if is_public {
            dim(ui, p, aurora_i18n::tr!("status-public"));
        } else if parcel.is_group_owned {
            // the owner is the group of the next row: Firestorm only links
            // that one, here the owner opens its profile whatever it is
            let group = if parcel.group_id.is_nil() {
                parcel.owner_id
            } else {
                parcel.group_id
            };
            super::group_link(ui, p, group, aurora_i18n::tr!("ui-land-general-group-owned"));
        } else {
            super::agent_link(ui, p, world, parcel.owner_id);
        }
        if parcel.status == OS_LEASE_PENDING {
            text(ui, p, aurora_i18n::tr!("ui-land-general-sale-pending"));
        }
    });
    row(ui, p, aurora_i18n::tr!("common-group-2"), |ui| {
        if is_public || parcel.group_id.is_nil() {
            text(ui, p, aurora_i18n::tr!("status-none"));
        } else {
            let groups = world.groups.groups.clone();
            let name = world
                .land
                .group_name(&parcel.group_id, &groups)
                .unwrap_or_else(|| aurora_i18n::tr!("common-loading").into());
            // LLSLURL("group", id, "inspect")
            super::group_link(ui, p, parcel.group_id, &name);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Set...: our own land only, not group owned (GP_NO_POWERS)
            if button(
                ui,
                p,
                aurora_i18n::tr!("common-choose"),
                v.can(powers::NONE) && !parcel.is_group_owned,
            )
            .clicked()
            {
                s.dialog = Some(Dialog::GroupPicker);
            }
        });
    });
    // deeding: our land with a group we are in; doing it needs GP_LAND_DEED
    let enable_deed = parcel.owner_id == agent && !parcel.group_id.is_nil() && v.rights().in_group(&parcel.group_id);
    let allow_deed = parcel.flags & pf::ALLOW_DEED_TO_GROUP != 0;
    ui.horizontal(|ui| {
        ui.add_space(super::KEY_W);
        let (r, on) = check(
            ui,
            p,
            enable_deed,
            allow_deed,
            aurora_i18n::tr!("ui-land-general-allow-deeding-to-group"),
        );
        r.clone()
            .on_hover_text(aurora_i18n::tr!("ui-land-general-a-group-officer-can-deed-this-land-to-the"));
        if r.changed() {
            world.land.update(|u| u.flags = set(u.flags, pf::ALLOW_DEED_TO_GROUP, on));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let can_deed = v.rights().has_power(&parcel.group_id, powers::LAND_DEED);
            if button(
                ui,
                p,
                aurora_i18n::tr!("common-deed"),
                allow_deed && !parcel.group_id.is_nil() && can_deed && !parcel.is_group_owned,
            )
            .on_hover_text(aurora_i18n::tr!("ui-land-general-you-can-only-deed-land-if-you-have-an"))
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
            p,
            enable_deed && allow_deed,
            parcel.flags & pf::CONTRIBUTE_WITH_DEED != 0,
            aurora_i18n::tr!("ui-land-general-owner-contributes-when-deeding-land"),
        );
        r.clone()
            .on_hover_text(aurora_i18n::tr!("ui-land-general-when-land-is-deeded-to-a-group-the-previous"));
        if r.changed() {
            world.land.update(|u| u.flags = set(u.flags, pf::CONTRIBUTE_WITH_DEED, on));
        }
    });

    ui.add_space(10.0);
    crate::ui::widgets::section_header(ui, p, "tag-filled", aurora_i18n::tr!("workspace-land-sale"));
    // sale
    let region_xfer = !v.region_flag(rf::BLOCK_LAND_RESELL);
    let estate_sellable = parcel.auction_id == 0 && estate_manager && v.region.as_ref().is_some_and(|r| parcel.owner_id == r.owner);
    let owner_sellable = region_xfer && parcel.auction_id == 0 && v.can(powers::LAND_SET_SALE_INFO);
    let can_be_sold = owner_sellable || estate_sellable;
    let for_sale = parcel.flags & pf::FOR_SALE != 0;
    row(ui, p, aurora_i18n::tr!("common-for-sale"), |ui| {
        if for_sale {
            let per_m = if parcel.area > 0 {
                parcel.sale_price as f32 / parcel.area as f32
            } else {
                0.0
            };
            text(
                ui,
                p,
                aurora_i18n::tr!(
                    "extra-price-l-l-m",
                    value0 = super::money(parcel.sale_price).to_string(),
                    per_m = aurora_i18n::number_text(&format!("{:.1}", per_m))
                ),
            );
        } else {
            text(ui, p, aurora_i18n::tr!("ui-land-general-not-for-sale"));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if can_be_sold && for_sale && button(ui, p, aurora_i18n::tr!("ui-land-general-cancel-land-sale"), true).clicked() {
                // onClickStopSellLand
                world.land.update(|u| {
                    u.flags &= !pf::FOR_SALE;
                    u.sale_price = 0;
                    u.auth_buyer = uuid::Uuid::nil();
                });
            }
            if can_be_sold && !for_sale && button(ui, p, aurora_i18n::tr!("common-sell-land"), true).clicked() {
                s.dialog = Some(Dialog::Sell(SellForm::new(&parcel)));
            }
        });
    });
    if for_sale {
        ui.horizontal(|ui| {
            ui.add_space(super::KEY_W);
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-land-general-for-sale-to"))
                    .size(12.0)
                    .color(p.ink),
            );
            if parcel.auth_buyer.is_nil() {
                text(ui, p, aurora_i18n::tr!("common-everyone"));
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
                    aurora_i18n::tr!("ui-land-general-objects-included-in-sale")
                } else {
                    aurora_i18n::tr!("ui-land-general-objects-not-included-in-sale")
                },
            );
        });
    }
    // SalePending: our purchase to approve, or the auction
    let pending = if is_public {
        String::new()
    } else if !is_leased && parcel.owner_id == agent {
        aurora_i18n::tr!("ui-land-general-you-must-approve-your-purchase-before-editing-this-land").into()
    } else if parcel.auction_id != 0 {
        aurora_i18n::tr!("ui-land-general-auction-id", value0 = format!("{}", parcel.auction_id))
    } else {
        String::new()
    };
    if !pending.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(super::KEY_W);
            ui.label(RichText::new(pending).size(12.0).color(p.warn));
        });
    }
    row(ui, p, aurora_i18n::tr!("ui-land-general-acquired"), |ui| {
        if !is_public {
            let r = text(ui, p, super::claim_date(parcel.claim_date as i64));
            if !is_leased {
                r.on_hover_text(aurora_i18n::tr!("ui-land-general-purchase-pending-approval"));
            }
        }
    });
    row(ui, p, aurora_i18n::tr!("common-area"), |ui| {
        text(ui, p, format!("{} m²", parcel.area))
    });
    row(ui, p, aurora_i18n::tr!("ui-land-general-traffic"), |ui| {
        let dwell = world.land.sel.as_ref().and_then(|s| s.dwell);
        text(
            ui,
            p,
            dwell.map_or(aurora_i18n::tr!("common-loading").into(), |d| aurora_i18n::number((d) as f64, 0)),
        );
    });

    ui.add_space(10.0);
    crate::ui::widgets::section_header(ui, p, "alert-triangle-filled", aurora_i18n::tr!("workspace-land-actions"));
    // buttons (bottom of floater_about_land.xml)
    ui.add_space(6.0);
    let can_access = true;
    let buy = can_agent_buy(&parcel, &v.rights(), None, can_access);
    // LLPanelLandGeneral::refresh: for the active group, with the power
    let active_group = world.groups.active;
    let buy_group = !active_group.is_nil() && can_agent_buy(&parcel, &v.rights(), Some(active_group), can_access);
    let release = v.owns(powers::LAND_RELEASE) || (estate_manager && v.region.as_ref().is_some_and(|r| parcel.owner_id != r.owner));
    // a pass: someone else's land selling passes (not while banned)
    let use_pass = super::overview::can_buy_pass(&parcel, agent, super::overview::collision_banned(world));
    ui.horizontal(|ui| {
        // onClickBuyLand: startBuyLand on the selection
        if button(ui, p, aurora_i18n::tr!("ui-land-general-buy-land"), buy)
            .on_disabled_hover_text(aurora_i18n::tr!("ui-land-general-this-land-is-not-for-sale-to-you"))
            .clicked()
        {
            world.land.request_buy(false);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if region_owner {
                // reclaimParcel: the region owner takes back someone's parcel
                if button(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-land-general-reclaim-land"),
                    !is_public && parcel.owner_id != agent,
                )
                .clicked()
                {
                    world
                        .land
                        .parcel_command(|handle, local_id| LandCommand::Reclaim { handle, local_id });
                }
            } else if button(ui, p, aurora_i18n::tr!("common-abandon-land"), release).clicked() {
                s.dialog = Some(Dialog::Confirm(Confirm::Release));
            }
        });
    });
    ui.horizontal(|ui| {
        if v.region.as_ref().is_some_and(|r| r.has_land_resources)
            && button(ui, p, aurora_i18n::tr!("ui-land-general-script-info"), true).clicked()
        {
            s.scripts.open();
        }
        if button(ui, p, aurora_i18n::tr!("ui-land-general-buy-a-pass"), use_pass)
            .on_hover_text(aurora_i18n::tr!("ui-land-general-a-pass-grants-temporary-access-to-this-land"))
            .clicked()
        {
            s.dialog = Some(Dialog::Confirm(Confirm::BuyPass));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if button(ui, p, aurora_i18n::tr!("ui-land-general-buy-for-group"), buy_group)
                .on_disabled_hover_text(aurora_i18n::tr!("ui-land-general-this-land-is-not-for-sale-to-you"))
                .clicked()
            {
                world.land.request_buy(true);
            }
        });
    });
}

/// LLParcel::setParcelFlag.
pub(super) fn set(flags: u32, flag: u32, on: bool) -> u32 {
    if on { flags | flag } else { flags & !flag }
}

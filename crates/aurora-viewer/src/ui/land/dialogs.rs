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
    world
        .notifications
        .push(Kind::Info, aurora_i18n::tr!("common-about-land"), body, Data::None);
}

/// The text of a confirmation, and what OK does.
fn question(c: &Confirm, parcel: &ParcelInfo, world: &mut World) -> String {
    let me = world.agent_id;
    match c {
        Confirm::Release => aurora_i18n::tr!(
            "ui-land-dialogs-you-are-about-to-abandon-m2-of-land-if",
            value0 = format!("{}", parcel.area)
        ),
        Confirm::Deed => {
            let group = group_name(world, &parcel.group_id);
            let contribution = if parcel.flags & pf::CONTRIBUTE_WITH_DEED != 0 {
                aurora_i18n::tr!(
                    "ui-land-dialogs-it-will-include-a-simultaneous-group-contribution-from",
                    value0 = world.social.name_of(&parcel.owner_id).to_string()
                )
            } else {
                String::new()
            };
            aurora_i18n::tr!(
                "ui-land-dialogs-deeding-this-parcel-requires-the-group-to-permanently-have",
                contribution = contribution.to_string(),
                value0 = format!("{}", parcel.area),
                group = group.to_string()
            )
        }
        Confirm::BuyPass => aurora_i18n::tr!(
            "ui-land-dialogs-for-l-you-can-enter-this-parcel-and-stay",
            value0 = format!("{}", parcel.pass_price),
            value1 = parcel.name.to_string(),
            value2 = aurora_i18n::number_text(&format!("{:.2}", parcel.pass_hours))
        ),
        Confirm::ReturnOwner if parcel.owner_id == me => aurora_i18n::tr!(
            "ui-land-dialogs-are-you-sure-you-want-to-return-all-objects-3",
            value0 = format!("{}", parcel.owner_prims)
        ),
        Confirm::ReturnOwner => aurora_i18n::tr!(
            "ui-land-dialogs-are-you-sure-you-want-to-return-all-objects-5",
            value0 = world.social.name_of(&parcel.owner_id).to_string(),
            value1 = format!("{}", parcel.owner_prims)
        ),
        Confirm::ReturnGroup => aurora_i18n::tr!(
            "ui-land-dialogs-are-you-sure-you-want-to-return-all-objects",
            value0 = group_name(world, &parcel.group_id).to_string(),
            value1 = format!("{}", parcel.group_prims)
        ),
        Confirm::ReturnOther if parcel.is_group_owned => aurora_i18n::tr!(
            "ui-land-dialogs-return-objects-on-this-parcel-that-are-not-shared",
            value0 = group_name(world, &parcel.group_id).to_string(),
            value1 = format!("{}", parcel.other_prims)
        ),
        Confirm::ReturnOther if parcel.owner_id == me => aurora_i18n::tr!(
            "ui-land-dialogs-are-you-sure-you-want-to-return-all-objects-2",
            value0 = format!("{}", parcel.other_prims)
        ),
        Confirm::ReturnOther => aurora_i18n::tr!(
            "ui-land-dialogs-are-you-sure-you-want-to-return-all-objects-4",
            value0 = world.social.name_of(&parcel.owner_id).to_string(),
            value1 = format!("{}", parcel.other_prims)
        ),
        Confirm::ReturnList(o) if o.is_group => aurora_i18n::tr!(
            "ui-land-dialogs-are-you-sure-you-want-to-return-all-objects",
            value0 = group_name(world, &o.id).to_string(),
            value1 = format!("{}", o.count)
        ),
        Confirm::ReturnList(o) => aurora_i18n::tr!(
            "ui-land-dialogs-are-you-sure-you-want-to-return-all-objects-5",
            value0 = world.social.name_of(&o.id).to_string(),
            value1 = format!("{}", o.count)
        ),
        Confirm::ResetEnvironment => aurora_i18n::tr!("ui-land-dialogs-you-are-about-to-remove-all-applied-settings-do").into(),
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
                aurora_i18n::tr!("ui-land-dialogs-your-objects-on-the-selected-parcel-have-been-returned").to_owned()
            } else {
                aurora_i18n::tr!(
                    "ui-land-dialogs-objects-on-the-selected-parcel-owned-by-have-been",
                    value0 = world.social.name_of(&parcel.owner_id).to_string()
                )
            };
            info(world, body);
            returned(world, RT_OWNER, Vec::new());
        }
        Confirm::ReturnGroup => {
            let body = aurora_i18n::tr!(
                "ui-land-dialogs-selected-objects-on-land-shared-with-group-have-been",
                value0 = group_name(world, &parcel.group_id).to_string()
            );
            info(world, body);
            returned(world, RT_GROUP, Vec::new());
        }
        Confirm::ReturnOther => {
            info(
                world,
                aurora_i18n::tr!("ui-land-dialogs-selected-objects-on-the-parcel-that-are-not-yours").into(),
            );
            returned(world, RT_OTHER, Vec::new());
        }
        Confirm::ReturnList(o) => {
            let body = if o.is_group {
                aurora_i18n::tr!(
                    "ui-land-dialogs-selected-objects-on-land-shared-with-group-have-been",
                    value0 = group_name(world, &o.id).to_string()
                )
            } else {
                aurora_i18n::tr!(
                    "ui-land-dialogs-objects-on-the-selected-parcel-owned-by-resident-have",
                    value0 = world.social.name_of(&o.id).to_string()
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
            let open = window(ctx, p, aurora_i18n::tr!("common-about-land"), 380.0, |ui| {
                ui.add(egui::Label::new(RichText::new(&msg).size(12.5).color(p.ink)).wrap());
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    done = super::super::widgets::flat_button(ui, p, aurora_i18n::tr!("common-ok")).clicked();
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
                (aurora_i18n::tr!("common-yes"), aurora_i18n::tr!("common-no"))
            } else {
                (aurora_i18n::tr!("common-ok"), aurora_i18n::tr!("common-cancel"))
            };
            let mut answer = None;
            let open = window(ctx, p, aurora_i18n::tr!("common-about-land"), 420.0, |ui| {
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
            let open = window(ctx, p, aurora_i18n::tr!("common-groups"), 300.0, |ui| {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-land-dialogs-choose-a-group"))
                        .size(12.0)
                        .color(p.muted),
                );
                let groups = world.groups.sorted();
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    if groups.is_empty() {
                        ui.label(
                            RichText::new(aurora_i18n::tr!("ui-land-dialogs-you-do-not-belong-to-any-groups"))
                                .size(12.0)
                                .color(p.muted),
                        );
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
                    if super::super::widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
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
            let open = window(ctx, p, aurora_i18n::tr!("ui-land-dialogs-ban-duration"), 300.0, |ui| {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-land-dialogs-ban-duration-2"))
                        .size(12.0)
                        .color(p.muted),
                );
                super::super::widgets::radio_value(ui, p, &mut temporary, false, aurora_i18n::tr!("common-always"));
                ui.horizontal(|ui| {
                    super::super::widgets::radio_value(ui, p, &mut temporary, true, aurora_i18n::tr!("common-temporary"));
                    ui.add_enabled(
                        temporary,
                        egui::DragValue::new(&mut hours)
                            .custom_parser(aurora_i18n::parse_number)
                            .range(1..=8766),
                    );
                    dim(ui, p, "heures");
                });
                answer = buttons(ui, p, aurora_i18n::tr!("common-ok"), aurora_i18n::tr!("common-cancel"), true);
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
            let open = window(ctx, p, aurora_i18n::tr!("common-sell-land"), 420.0, |ui| {
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
            let open = window(ctx, p, aurora_i18n::tr!("ui-land-dialogs-media-url"), 420.0, |ui| {
                ui.horizontal(|ui| {
                    dim(ui, p, aurora_i18n::tr!("status-url"));
                    ui.add_enabled(!waiting, egui::TextEdit::singleline(&mut url).desired_width(f32::INFINITY));
                });
                if waiting {
                    dim(ui, p, aurora_i18n::tr!("common-loading"));
                }
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if super::button(ui, p, aurora_i18n::tr!("common-cancel"), !waiting).clicked() {
                        close = true;
                    }
                    if super::button(ui, p, aurora_i18n::tr!("common-clear"), !waiting).clicked() {
                        url.clear();
                    }
                    if super::button(ui, p, aurora_i18n::tr!("common-ok"), !waiting).clicked() {
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
            aurora_i18n::tr!("common-everyone").to_owned()
        } else {
            buyer_name.clone().unwrap_or_default()
        };
        let text = if f.to == 1 {
            aurora_i18n::tr!(
                "ui-land-dialogs-warning-clicking-sell-to-anyone-makes-your-land-available",
                value0 = format!("{}", parcel.area),
                price = format!("{}", price),
                name = name.to_string()
            )
        } else {
            aurora_i18n::tr!(
                "ui-land-dialogs-the-selected-m2-of-land-is-for-sale-your",
                value0 = format!("{}", parcel.area),
                price = format!("{}", price),
                name = name.to_string()
            )
        };
        ui.add(egui::Label::new(RichText::new(text).size(12.5).color(p.ink)).wrap());
        match buttons(ui, p, aurora_i18n::tr!("common-ok"), aurora_i18n::tr!("common-cancel"), true) {
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
    super::row(ui, p, aurora_i18n::tr!("ui-land-dialogs-land"), |ui| {
        super::text(ui, p, parcel.name.clone())
    });
    super::row(ui, p, aurora_i18n::tr!("common-area"), |ui| {
        super::text(ui, p, format!("{} m²", parcel.area))
    });
    ui.add_space(4.0);
    ui.label(
        RichText::new(aurora_i18n::tr!("ui-land-dialogs-1-set-a-price"))
            .size(12.5)
            .color(p.ink),
    );
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut f.price).desired_width(90.0));
        f.price.retain(|c| c.is_ascii_digit());
        dim(ui, p, "L$");
        if price >= 0 && parcel.area > 0 {
            dim(
                ui,
                p,
                aurora_i18n::tr!(
                    "status-l-per-m",
                    value0 = aurora_i18n::number_text(&format!("{:.2}", price as f32 / parcel.area as f32))
                ),
            );
        }
    });
    ui.label(
        RichText::new(aurora_i18n::tr!("ui-land-dialogs-2-sell-the-land-to"))
            .size(12.5)
            .color(p.ink),
    );
    ui.horizontal(|ui| {
        let label = match f.to {
            1 => aurora_i18n::tr!("common-everyone"),
            2 => aurora_i18n::tr!("ui-land-dialogs-specific-person"),
            _ => aurora_i18n::tr!("ui-land-dialogs-select"),
        };
        egui::ComboBox::from_id_salt("land_sell_to")
            .selected_text(label)
            .width(160.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut f.to, 1, aurora_i18n::tr!("common-everyone"));
                ui.selectable_value(&mut f.to, 2, aurora_i18n::tr!("ui-land-dialogs-specific-person"));
            });
        if f.to == 2 {
            if let Some(n) = &buyer_name {
                super::text(ui, p, n.clone());
            }
            if super::super::widgets::flat_button(ui, p, aurora_i18n::tr!("ui-land-dialogs-select-2")).clicked() {
                s.pick_for = Some(PickFor::SellTo);
                s.picker.open("land_avatar_picker", false);
            }
        }
    });
    ui.label(
        RichText::new(aurora_i18n::tr!("ui-land-dialogs-3-sell-the-objects-with-the-land"))
            .size(12.5)
            .color(p.ink),
    );
    super::super::widgets::radio_value(
        ui,
        p,
        &mut f.with_objects,
        false,
        aurora_i18n::tr!("ui-land-dialogs-no-keep-object-ownership"),
    );
    super::super::widgets::radio_value(
        ui,
        p,
        &mut f.with_objects,
        true,
        aurora_i18n::tr!("ui-land-dialogs-yes-sell-objects-with-land"),
    );
    ui.label(
        RichText::new(aurora_i18n::tr!("ui-land-dialogs-warning-all-sales-are-final"))
            .size(12.0)
            .color(p.warn),
    );
    let ready = price >= 0 && (f.to == 1 || (f.to == 2 && f.buyer.is_some()));
    match buttons(
        ui,
        p,
        aurora_i18n::tr!("ui-land-dialogs-offer-this-land-for-sale"),
        aurora_i18n::tr!("common-cancel"),
        ready,
    ) {
        Some(true) => {
            // a sale to anyone needs a price (SalePriceRestriction)
            if parcel.flags & pf::FOR_SALE == 0 && price == 0 && f.to == 1 {
                s.dialog = Some(Dialog::Message(
                    aurora_i18n::tr!("ui-land-dialogs-the-asking-price-must-exceed-l-0-to-sell").into(),
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

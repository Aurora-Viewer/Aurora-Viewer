//! The questions and small floaters of About Land: confirmations
//! (ReleaseLandWarning, DeedLandToGroup, LandBuyPass, ReturnObjects*,
//! SettingsConfirmReset), LLFloaterGroupPicker, LLFloaterSellLand,
//! LLFloaterBanDuration and LLFloaterURLEntry (French texts of
//! notifications.xml and the floaters' XML).

use super::{Dialog, LandUi, PickFor, View, dim};
use crate::theme::Palette;
use crate::world::World;
use crate::world::env_select::{self, SettingsCatalog, SettingsKind};
use crate::world::notifications::{Data, Kind};
use aurora_net::land::{
    AL_BLOCK_EXPERIENCE, FoundExperience, LandCommand, ObjectOwner, RT_GROUP, RT_LIST, RT_OTHER, RT_OWNER, XP_PROPERTY_GRID,
    XP_PROPERTY_PRIVILEGED,
};
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
    /// Several owners of the list at once: one ParcelReturnObjects with all
    /// their IDs (RT_LIST takes a list).
    ReturnOwners(Vec<ObjectOwner>),
    ResetEnvironment,
}

/// LLFloaterExperiencePicker being filled: the list it adds to, the search
/// and the rating filter (FilterOverRating).
pub(super) struct ExperiencePicker {
    pub(super) kind: u32,
    pub(super) query: String,
    pub(super) maturity: u8,
    pub(super) selected: Option<Uuid>,
    /// Search at once (demo captures).
    pub(super) search_now: bool,
}

impl ExperiencePicker {
    pub(super) fn new(kind: u32, maturity: u8) -> ExperiencePicker {
        ExperiencePicker {
            kind,
            query: String::new(),
            maturity,
            selected: None,
            search_now: false,
        }
    }
}

/// The filters of the parcel's experience lists (LLPanelLandExperiences::
/// postBuild): no grid-wide experience in the allowed list, none both
/// privileged and grid-wide in the blocked one; then the rating chosen
/// (FilterOverRating) and those already in the list (FilterMatching).
pub(super) fn experience_hidden(e: &FoundExperience, kind: u32, maturity: u8, list: &[Uuid]) -> bool {
    let by_kind = if kind == AL_BLOCK_EXPERIENCE {
        let both = XP_PROPERTY_PRIVILEGED | XP_PROPERTY_GRID;
        e.properties & both == both
    } else {
        e.properties & XP_PROPERTY_GRID != 0
    };
    by_kind || e.maturity > maturity || list.contains(&e.id)
}

/// LLFloaterSettingsPicker for the parcel environment: the whole day
/// (`track` None) or one track (0 water, 1 ground sky, 2–4 skies).
pub(super) struct EnvironmentPicker {
    pub(super) track: Option<i32>,
    pub(super) filter: String,
    pub(super) selected: Option<Uuid>,
}

/// The settings kinds a target takes: any for the whole day, water for
/// track 0, a sky for the others (the drop targets of the altitudes).
pub(super) fn settings_kinds(track: Option<i32>) -> &'static [SettingsKind] {
    match track {
        None => &[SettingsKind::Day, SettingsKind::Sky, SettingsKind::Water],
        Some(0) => &[SettingsKind::Water],
        Some(_) => &[SettingsKind::Sky],
    }
}

/// onPickerCommitted: FLAG_NOMOD / FLAG_NOTRANS from the item's permissions.
pub(crate) fn settings_flags(owner_mask: u32) -> u32 {
    const PERM_TRANSFER: u32 = 1 << 13;
    const PERM_MODIFY: u32 = 1 << 14;
    const FLAG_NOMOD: u32 = 1 << 1;
    const FLAG_NOTRANS: u32 = 1 << 2;
    let mut flags = 0;
    if owner_mask & PERM_MODIFY == 0 {
        flags |= FLAG_NOMOD;
    }
    if owner_mask & PERM_TRANSFER == 0 {
        flags |= FLAG_NOTRANS;
    }
    flags
}

/// LLFloaterSellLand being filled.
pub(super) struct SellForm {
    price: String,
    /// sell_to: 0 "- Select -", 1 anyone, 2 a specific resident.
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
        Confirm::ReturnOwners(owners) => aurora_i18n::tr!(
            "ui-land-dialogs-return-objects-of-owners",
            count = owners.iter().map(|o| i64::from(o.count)).sum::<i64>(),
            owners = owners.len() as i64
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
        // the callbacks drop the highlight and send the parcel again so
        // its counts refresh
        world.land.unhighlight();
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
        Confirm::ReturnOwners(owners) => {
            info(world, aurora_i18n::tr!("ui-land-dialogs-objects-of-owners-returned").into());
            returned(world, RT_LIST, owners.iter().map(|o| o.id).collect());
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

/// A row of buttons at the right, one button high: a right-to-left layout
/// alone takes all the height the dialog may grow to.
fn right_row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let row = Vec2::new(ui.available_width(), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), add)
        .inner
}

/// OK / Cancel (or Yes / No) at the bottom right; Some(true) for OK.
fn buttons(ui: &mut egui::Ui, p: &Palette, ok: &str, cancel: &str, ok_enabled: bool) -> Option<bool> {
    let mut out = None;
    ui.add_space(6.0);
    right_row(ui, |ui| {
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
                right_row(ui, |ui| {
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
                right_row(ui, |ui| {
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
            replace,
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
                    if replace {
                        // a new duration for residents already banned:
                        // addToBanList keeps the later expiry, so they go first
                        if let Some(sel) = world.land.sel.as_mut() {
                            sel.bans.retain(|e| !ids.contains(&e.id));
                        }
                    }
                    super::access::add_banned(world, &ids, time);
                    None
                }
                Some(false) => None,
                None => open.then_some(Dialog::BanDuration {
                    ids,
                    temporary,
                    hours,
                    replace,
                }),
            }
        }
        Dialog::ExperiencePicker(mut picker) => {
            let mut close = false;
            let open = window(ctx, p, aurora_i18n::tr!("ui-land-picker-experience-title"), 420.0, |ui| {
                experience_picker(ui, p, world, &mut picker, &mut close)
            });
            (open && !close).then_some(Dialog::ExperiencePicker(picker))
        }
        Dialog::EnvironmentPicker(mut picker) => {
            let mut close = false;
            let open = window(ctx, p, aurora_i18n::tr!("ui-land-picker-environment-title"), 420.0, |ui| {
                environment_picker(ui, p, world, &mut picker, &mut close)
            });
            (open && !close).then_some(Dialog::EnvironmentPicker(picker))
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
                right_row(ui, |ui| {
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

/// A selectable row of a picker's list.
pub(crate) fn pick_row(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, selected: bool) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), egui::Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if selected {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.28));
    } else if r.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.5));
    }
    if let Some(t) = super::super::icons::global(icon) {
        ui.painter().image(
            t.id(),
            egui::Rect::from_center_size(rect.left_center() + Vec2::new(14.0, 0.0), Vec2::splat(14.0)),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            p.muted,
        );
    }
    ui.painter().text(
        rect.left_center() + Vec2::new(28.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(12.5),
        p.ink,
    );
    r
}

/// LLPanelExperiencePicker: search by name, filter, choose.
fn experience_picker(ui: &mut egui::Ui, p: &Palette, world: &mut World, f: &mut ExperiencePicker, close: &mut bool) {
    let list: Vec<Uuid> = world.land.sel.as_ref().map_or(Vec::new(), |sel| {
        if f.kind == AL_BLOCK_EXPERIENCE {
            sel.blocked_experiences.clone()
        } else {
            sel.allowed_experiences.clone()
        }
    });
    if std::mem::take(&mut f.search_now) {
        world.land.search_experiences(&f.query);
    }
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(&mut f.query)
                .hint_text(aurora_i18n::tr!("ui-land-picker-experience-hint"))
                .desired_width(ui.available_width() - 100.0),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if super::super::widgets::flat_button(ui, p, aurora_i18n::tr!("common-search")).clicked() || enter {
            world.land.search_experiences(&f.query);
            f.selected = None;
        }
    });
    ui.horizontal(|ui| {
        dim(ui, p, aurora_i18n::tr!("ui-land-picker-experience-rating"));
        // SIM_ACCESS_PG / MATURE / ADULT
        let ratings = [13u8, 21, 42];
        let mut choice = ratings.iter().position(|r| *r >= f.maturity).unwrap_or(2);
        let labels = [
            (aurora_i18n::tr!("common-general"), true),
            (aurora_i18n::tr!("common-moderate"), true),
            (aurora_i18n::tr!("common-adult"), true),
        ];
        if super::super::widgets::segmented(ui, p, &mut choice, &labels) {
            f.maturity = ratings[choice];
        }
    });
    ui.add_space(4.0);
    let results = world.land.experience_search.clone();
    egui::ScrollArea::vertical()
        .id_salt("land_xp_picker")
        .max_height(220.0)
        .min_scrolled_height(220.0)
        .auto_shrink([false, false])
        .show(ui, |ui| match results {
            None => {
                dim(ui, p, aurora_i18n::tr!("ui-land-picker-experience-start"));
            }
            Some((_, None)) => {
                dim(ui, p, aurora_i18n::tr!("common-search"));
            }
            Some((_, Some(None))) => {
                dim(ui, p, aurora_i18n::tr!("ui-land-picker-experience-failed"));
            }
            Some((_, Some(Some(found)))) => {
                let shown: Vec<&FoundExperience> = found.iter().filter(|e| !experience_hidden(e, f.kind, f.maturity, &list)).collect();
                if shown.is_empty() {
                    dim(ui, p, aurora_i18n::tr!("common-no-results"));
                }
                for e in shown {
                    let label = if e.name.is_empty() {
                        aurora_i18n::tr!("ui-land-picker-experience-untitled").to_owned()
                    } else {
                        e.name.clone()
                    };
                    let r = pick_row(ui, p, "flask-filled", &label, f.selected == Some(e.id));
                    if r.clicked() {
                        f.selected = Some(e.id);
                    }
                    if r.double_clicked() {
                        world.land.add_experience(f.kind, e.id);
                        *close = true;
                    }
                }
            }
        });
    if let Some(answer) = buttons(
        ui,
        p,
        aurora_i18n::tr!("common-choose"),
        aurora_i18n::tr!("common-cancel"),
        f.selected.is_some(),
    ) {
        if answer && let Some(id) = f.selected {
            world.land.add_experience(f.kind, id);
        }
        *close = true;
    }
}

/// LLFloaterSettingsPicker: the settings of the inventory and the library
/// the target takes; OK applies it (onPickerCommitted).
fn environment_picker(ui: &mut egui::Ui, p: &Palette, world: &mut World, f: &mut EnvironmentPicker, close: &mut bool) {
    // the settings folders load as the environment selector's do
    let loading = env_select::scan_step(&mut world.inventory) > 0;
    let order = crate::ui::inventory::view::picker_order(ui.ctx(), &world.inventory);
    let catalog = SettingsCatalog::build(&world.inventory, |id| order.rank(id));
    ui.add(
        egui::TextEdit::singleline(&mut f.filter)
            .hint_text(aurora_i18n::tr!("common-search"))
            .desired_width(f32::INFINITY),
    );
    ui.add_space(4.0);
    let needle = f.filter.trim().to_lowercase();
    let mut chosen = None;
    egui::ScrollArea::vertical()
        .id_salt("land_env_picker")
        .max_height(240.0)
        .min_scrolled_height(240.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut any = false;
            for (kind, c) in catalog.choices(settings_kinds(f.track), |id| order.rank(id)) {
                let icon = match kind {
                    SettingsKind::Day => "sunrise-filled",
                    SettingsKind::Sky => "sun-filled",
                    SettingsKind::Water => "droplet-filled",
                };
                if !needle.is_empty() && !c.name.to_lowercase().contains(&needle) {
                    continue;
                }
                any = true;
                let r = pick_row(ui, p, icon, &c.name, f.selected == Some(c.asset));
                if r.clicked() {
                    f.selected = Some(c.asset);
                }
                if r.double_clicked() {
                    chosen = Some((c.asset, c.name.clone()));
                }
            }
            if !any {
                dim(
                    ui,
                    p,
                    if loading {
                        aurora_i18n::tr!("common-loading")
                    } else {
                        aurora_i18n::tr!("ui-land-picker-environment-none")
                    },
                );
            }
        });
    if let Some(answer) = buttons(
        ui,
        p,
        aurora_i18n::tr!("ui-land-picker-environment-apply"),
        aurora_i18n::tr!("common-cancel"),
        f.selected.is_some(),
    ) {
        if answer && let Some(asset) = f.selected {
            let name = settings_kinds(f.track)
                .iter()
                .find_map(|k| catalog.name_of(*k, asset))
                .unwrap_or_default()
                .to_owned();
            chosen = Some((asset, name));
        }
        if !answer {
            *close = true;
        }
    }
    if let Some((asset, name)) = chosen {
        let owner_mask = world
            .inventory
            .items
            .values()
            .find(|i| i.asset_id == asset)
            .map_or(u32::MAX, |i| i.owner_mask);
        let (day_length, day_offset) = match world.land.sel.as_ref().map(|s| &s.env) {
            Some(crate::world::land::EnvState::Ready(e)) => (e.day_length as i32, e.day_offset as i32),
            _ => (-1, -1),
        };
        let (track, flags) = (f.track, settings_flags(owner_mask));
        world.land.environment(|local_id| LandCommand::EnvironmentApply {
            local_id,
            track,
            asset,
            name,
            flags,
            day_length,
            day_offset,
        });
        *close = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::land::AL_ALLOW_EXPERIENCE;

    fn xp(n: u128, maturity: u8, properties: i32) -> FoundExperience {
        FoundExperience {
            id: Uuid::from_u128(n),
            name: String::new(),
            owner: Uuid::nil(),
            maturity,
            properties,
        }
    }

    /// LLPanelLandExperiences' filters, the rating and the list's own.
    #[test]
    fn the_experience_picker_hides_what_a_list_cannot_take() {
        let grid = xp(1, 13, XP_PROPERTY_GRID);
        let privileged_grid = xp(2, 13, XP_PROPERTY_GRID | XP_PROPERTY_PRIVILEGED);
        let plain = xp(3, 13, 0);
        let adult = xp(4, 42, 0);
        assert!(experience_hidden(&grid, AL_ALLOW_EXPERIENCE, 42, &[]));
        assert!(!experience_hidden(&grid, AL_BLOCK_EXPERIENCE, 42, &[]), "a grid one may be blocked");
        assert!(experience_hidden(&privileged_grid, AL_BLOCK_EXPERIENCE, 42, &[]));
        assert!(!experience_hidden(&plain, AL_ALLOW_EXPERIENCE, 42, &[]));
        assert!(experience_hidden(&plain, AL_ALLOW_EXPERIENCE, 42, &[plain.id]), "already listed");
        assert!(experience_hidden(&adult, AL_ALLOW_EXPERIENCE, 21, &[]), "over the rating");
    }

    #[test]
    fn settings_for_each_target_and_their_flags() {
        assert_eq!(settings_kinds(None).len(), 3);
        assert_eq!(settings_kinds(Some(0)), &[SettingsKind::Water]);
        assert_eq!(settings_kinds(Some(3)), &[SettingsKind::Sky]);
        assert_eq!(settings_flags(u32::MAX), 0);
        assert_eq!(settings_flags(0), (1 << 1) | (1 << 2));
        assert_eq!(settings_flags(1 << 14), 1 << 2, "modifiable, not transferable");
    }
}

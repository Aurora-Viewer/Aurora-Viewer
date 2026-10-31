//! "Buy Land", for the selected land: what it is, what buying it will do
//! (account plan, land use fee, L$ to buy and to pay), the estate's rules
//! and covenant to agree to, then the purchase. Port of LLFloaterBuyLandUI
//! (Firestorm indra/newview/llfloaterbuyland.cpp, floater_buy_land.xml, and
//! the amount field of LLCurrencyUIManager, llcurrencyuimanager.cpp;
//! originally LGPL 2.1). The rules live in world/land_buy.rs.
//!
//! Differences from Firestorm, on purpose:
//! - the window starts over only when what is for sale changes (owner,
//!   price, area…): the regular refresh of the selection keeps the covenant
//!   ticked and the amount typed (Firestorm only refreshes on a new
//!   selection, so it never meets the case);
//! - the balance shown follows the account while the window is open
//!   (Firestorm reads it once, when the parcel is set);
//! - "Buy" also waits for the estimate of an amount being typed, so the
//!   website is never sent the cost of another amount;
//! - while the purchase is processed the window says so with a spinner,
//!   under "Purchase in progress" (Firestorm puts the same text under "Not
//!   for sale:").

use super::widgets::{self, Floater, Modal, flat_button_sized};
use crate::theme::Palette;
use crate::world::World;
use crate::world::land_buy::{self, AreaNote, Buyer, Funds, Offer, Region};
use crate::world::notifications::{Data, Kind};
use aurora_net::currency::{CurrencyError, CurrencyQuote, Password};
use aurora_net::land::LandCommand;
use aurora_net::land_tool::{LandToolBuy, LandToolInfo};
use aurora_net::{NetCommand, ParcelInfo, RegionHandle, parcel_flags as pf, region_flags as rf};
use egui::{RichText, Vec2};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Pause in typing before an estimate is asked (CURRENCY_ESTIMATE_FREQUENCY).
const ESTIMATE_PAUSE: Duration = Duration::from_secs(2);
const SIZE: Vec2 = Vec2::new(800.0, 640.0);
const LEFT_W: f32 = 430.0;
const BUTTON: Vec2 = Vec2::new(120.0, 28.0);
const CONFIRM_WIDTH: f32 = 420.0;
const PICTURE: Vec2 = Vec2::new(150.0, 112.0);

/// The land the window is about, from the selection that opened it.
#[derive(Debug, Clone)]
pub struct Target {
    pub handle: RegionHandle,
    pub parcel: Arc<ParcelInfo>,
    /// The selection covers several parcels (PARCEL_RESULT_MULTIPLE).
    pub multiple: bool,
    /// The selected rectangle (west, south, east, north), region
    /// coordinates: what a claim of public land takes.
    pub rect: (f32, f32, f32, f32),
    /// The rectangle runs into another region (a claim refuses it).
    pub spans_regions: bool,
}

/// Where the selection stands (LLFloaterBuyLandUI::SelectionObserver).
pub enum Selection {
    /// Nothing selected any more: the window closes.
    Gone,
    /// Selected, the parcel not received yet.
    Pending,
    Ready(Target),
}

/// What of the target makes a new offer: a refresh with the same values
/// keeps the window as it is.
#[derive(Debug, Clone, PartialEq)]
struct OfferKey {
    handle: RegionHandle,
    local_id: i32,
    owner: Uuid,
    group: Uuid,
    group_owned: bool,
    sale_flags: u32,
    price: i32,
    auth_buyer: Uuid,
    area: i32,
    multiple: bool,
    public_count: i32,
    other_count: i32,
}

impl OfferKey {
    fn of(t: &Target) -> OfferKey {
        let p = &t.parcel;
        OfferKey {
            handle: t.handle,
            local_id: p.local_id,
            owner: p.owner_id,
            group: p.group_id,
            group_owned: p.is_group_owned,
            sale_flags: p.flags & (pf::FOR_SALE | pf::SELL_PARCEL_OBJECTS),
            price: p.sale_price,
            auth_buyer: p.auth_buyer,
            area: p.area,
            multiple: t.multiple,
            public_count: p.public_count,
            other_count: p.other_count,
        }
    }
}

/// The exchange with the website in flight (mTransaction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SiteTxn {
    Preflight { billable_area: i32, currency_buy: i32 },
    Buy,
}

/// The badge of a step (LLViewChildren::Badge).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Badge {
    Ok,
    Note,
    Warn,
    Error,
}

#[derive(Default)]
pub struct BuyLandUi {
    pub open: bool,
    for_group: bool,
    helper: Option<&'static str>,
    target: Option<Target>,
    key: Option<OfferKey>,
    offer: Offer,
    /// mCannotBuyIsError: what the website or the L$ estimate refused.
    error: Option<CurrencyError>,
    /// The purchase is processed (startBuyPostConfirm).
    processing: bool,
    /// What the website said (mSiteValid when Some).
    site: Option<LandToolInfo>,
    site_txn: Option<SiteTxn>,
    /// Index of the plan chosen when the account must change plan.
    plan: usize,
    /// L$ to buy with the land, as typed (LLCurrencyUIManager).
    amount: i32,
    text: String,
    typed_at: Option<Instant>,
    changed: bool,
    quote_asked: Option<i32>,
    usd_cents: Option<i32>,
    local_cost: Option<String>,
    agreed: bool,
    remove_contribution: bool,
    /// The website's confirmation being answered: the password typed.
    confirm: Option<String>,
    /// The ParcelBuy / ParcelClaim sent once the website side is done.
    pending: Option<LandCommand>,
    /// The purchase is sent: the window closes (mBought).
    bought: bool,
    /// AURORA_DEMO_BUY_LAND=…:confirm: agree and open the confirmation once
    /// the website has answered.
    demo_confirm: bool,
    pub wanted_images: HashSet<Uuid>,
}

fn wrapped(ui: &mut egui::Ui, text: RichText) {
    ui.add(egui::Label::new(text).wrap());
}

impl BuyLandUi {
    /// LLFloaterBuyLand::buyLand, once the app checked the group power.
    /// `helper` is the grid's helper page (`aurora_net::currency::helper_uri`).
    pub fn open(&mut self, for_group: bool, helper: Option<&'static str>) {
        if self.site_txn == Some(SiteTxn::Buy) {
            // the purchase in flight keeps its window
            self.open = true;
            return;
        }
        *self = BuyLandUi {
            open: true,
            for_group,
            helper,
            wanted_images: std::mem::take(&mut self.wanted_images),
            ..Default::default()
        };
    }

    /// AURORA_DEMO_BUY_LAND=…:confirm (see docs/TESTING.md).
    pub fn demo_confirm(&mut self) {
        self.demo_confirm = true;
    }

    fn close(&mut self) {
        let images = std::mem::take(&mut self.wanted_images);
        *self = BuyLandUi {
            wanted_images: images,
            ..Default::default()
        };
    }

    /// mCanBuy.
    fn can_buy(&self) -> bool {
        self.offer.can_buy() && self.error.is_none() && !self.processing
    }

    fn funds(&self, world: &World) -> Funds {
        Funds {
            balance: world.balance.unwrap_or(0),
            currency_buy: self.amount,
            price: self.offer.price,
        }
    }

    /// The website must not be sent anything while it estimates, buys, or
    /// while an amount typed waits for its estimate.
    fn idle(&self) -> bool {
        self.site_txn.is_none() && self.quote_asked.is_none() && !self.changed
    }

    /// The Purchase button (refreshUI).
    fn buy_enabled(&self, world: &World) -> bool {
        self.can_buy() && self.site.is_some() && self.funds(world).will_have_enough() && self.idle() && self.agrees(world)
    }

    /// The covenant checkbox; an estate without a covenant agrees for us
    /// (onCovenantTextUpdated with a null asset).
    fn agrees(&self, world: &World) -> bool {
        self.agreed || self.no_covenant(world)
    }

    fn no_covenant(&self, world: &World) -> bool {
        self.target
            .as_ref()
            .is_some_and(|t| world.land.covenant.as_ref().is_some_and(|c| c.handle == t.handle && c.id.is_nil()))
    }

    /// Follow the selection (SelectionObserver::changed), and run what is
    /// due this frame.
    fn sync(&mut self, world: &mut World, selection: Selection, now: Instant) -> Vec<NetCommand> {
        let mut out = Vec::new();
        if self.site_txn == Some(SiteTxn::Buy) {
            // the user is buying: the selection does not change
            return out;
        }
        match selection {
            Selection::Gone => {
                if self.site_txn.is_none() {
                    self.close();
                }
                return out;
            }
            Selection::Pending => {
                if self.target.is_none() {
                    self.offer = Offer::default();
                    self.offer.refusal = Some(land_buy::Refusal::NoLandSelected);
                }
            }
            Selection::Ready(t) => {
                let key = OfferKey::of(&t);
                if self.key.as_ref() != Some(&key) {
                    self.key = Some(key);
                    self.target = Some(t);
                    out.extend(self.set_parcel(world));
                } else {
                    // same offer: newer figures (objects, snapshot) only
                    self.target = Some(t);
                }
            }
        }
        out.extend(self.poll(now));
        out
    }

    fn assess(&self, world: &World) -> Offer {
        let Some(t) = &self.target else {
            return Offer {
                refusal: Some(land_buy::Refusal::NoLandSelected),
                ..Default::default()
            };
        };
        let region = world.regions.get(&t.handle).and_then(|r| {
            r.info.as_ref().map(|i| Region {
                name: &i.name,
                billable_factor: i.billable_factor,
                max_tasks: r.max_tasks,
            })
        });
        let buyer = Buyer {
            id: world.agent_id,
            groups: &world.groups.groups,
            active_group: world.groups.active,
            for_group: self.for_group,
        };
        land_buy::assess(Some(&t.parcel), t.multiple, region.as_ref(), &buyer)
    }

    /// setParcel: updateAgentInfo, updateParcelInfo, updateCovenantInfo,
    /// then updateWebSiteInfo when the land can be bought.
    fn set_parcel(&mut self, world: &mut World) -> Vec<NetCommand> {
        self.offer = self.assess(world);
        self.error = None;
        self.processing = false;
        self.agreed = false;
        self.remove_contribution = false;
        // setAmount(cashBuy, true): the shortfall, estimated by the preflight
        let shortfall = self.funds(world).shortfall();
        self.amount = shortfall;
        self.text = if shortfall == 0 { String::new() } else { shortfall.to_string() };
        self.changed = false;
        self.typed_at = None;
        self.usd_cents = None;
        self.local_cost = None;
        if let Some(t) = &self.target {
            world.land.request_covenant(t.handle);
        }
        let mut out = Vec::new();
        if self.can_buy() {
            out.extend(self.update_web_site_info());
        }
        out
    }

    /// updateWebSiteInfo: one preflight per billable area and amount.
    fn update_web_site_info(&mut self) -> Option<NetCommand> {
        let billable_area = if self.for_group { 0 } else { self.offer.billable_area };
        let ask = SiteTxn::Preflight {
            billable_area,
            currency_buy: self.amount,
        };
        if self.site_txn == Some(ask) {
            return None;
        }
        let Some(helper) = self.helper else {
            self.tell_user_error(CurrencyError {
                message: aurora_i18n::tr!("buy-land-unavailable-on-this-grid").into(),
                uri: String::new(),
            });
            return None;
        };
        self.site_txn = Some(ask);
        Some(NetCommand::LandToolPreflight {
            helper: helper.to_owned(),
            billable_area,
            currency_buy: self.amount,
        })
    }

    /// tellUserError.
    fn tell_user_error(&mut self, e: CurrencyError) {
        self.processing = false;
        self.error = Some(e);
    }

    /// finishWebSiteInfo.
    pub fn preflight_reply(&mut self, billable_area: i32, currency_buy: i32, result: Result<LandToolInfo, CurrencyError>) {
        if self.site_txn
            != Some(SiteTxn::Preflight {
                billable_area,
                currency_buy,
            })
        {
            return;
        }
        self.site_txn = None;
        match result {
            Ok(info) => {
                // setUSDEstimate / setLocalEstimate
                if info.usd_cents.is_some() {
                    self.usd_cents = info.usd_cents;
                }
                if info.local_cost.is_some() {
                    self.local_cost = info.local_cost.clone();
                }
                self.plan = 0;
                self.site = Some(info);
            }
            Err(e) => {
                self.site = None;
                self.tell_user_error(e);
            }
        }
    }

    /// currencyKey: digits that fit an S32.
    fn typed(&mut self, now: Instant) {
        self.text.retain(|c| c.is_ascii_digit());
        let value = if self.text.is_empty() {
            Some(0)
        } else {
            self.text.parse::<i32>().ok()
        };
        let Some(value) = value else {
            self.text = if self.amount == 0 { String::new() } else { self.amount.to_string() };
            return;
        };
        if value == self.amount {
            return;
        }
        self.amount = value;
        self.changed = true;
        self.typed_at = Some(now);
        self.usd_cents = None;
        self.local_cost = None;
    }

    /// considerUpdateCurrency: the estimate of an amount typed, once the
    /// typing has paused.
    fn poll(&mut self, now: Instant) -> Option<NetCommand> {
        if !self.changed || self.quote_asked.is_some() {
            return None;
        }
        if self.typed_at.is_some_and(|t| now.duration_since(t) < ESTIMATE_PAUSE) {
            return None;
        }
        self.changed = false;
        if self.amount == 0 {
            return None;
        }
        let helper = self.helper?;
        self.quote_asked = Some(self.amount);
        Some(NetCommand::CurrencyQuote {
            helper: helper.to_owned(),
            amount: self.amount,
        })
    }

    /// finishCurrencyInfo; false when the answer is not this window's.
    pub fn quote_reply(&mut self, amount: i32, result: &Result<CurrencyQuote, CurrencyError>) -> bool {
        if self.quote_asked != Some(amount) {
            return false;
        }
        self.quote_asked = None;
        if self.changed || amount != self.amount {
            return true;
        }
        match result {
            Ok(q) => {
                // the helper may round the amount: the field follows
                if q.amount != self.amount {
                    self.amount = q.amount;
                    self.text = q.amount.to_string();
                }
                self.usd_cents = q.usd_cents;
                self.local_cost = q.local_cost.clone();
            }
            // draw: tellUserError(mCurrency.errorMessage())
            Err(e) if self.can_buy() => self.tell_user_error(e.clone()),
            Err(_) => {}
        }
        true
    }

    /// LLCurrencyUIManager::getLocalEstimate.
    fn local_estimate(&self) -> Option<String> {
        self.local_cost.clone().or_else(|| {
            self.usd_cents
                .map(|c| aurora_i18n::tr!("currency-usd", amount = aurora_i18n::number(f64::from(c) / 100.0, 2)).to_owned())
        })
    }

    /// mParcelSellerName (updateNames).
    fn seller(&self, world: &mut World) -> String {
        let Some(p) = self.target.as_ref().map(|t| t.parcel.clone()) else {
            return String::new();
        };
        if self.offer.is_claim {
            "Linden Lab".into()
        } else if p.is_group_owned {
            let groups = world.groups.groups.clone();
            world.land.group_name(&p.group_id, &groups).unwrap_or_else(|| "…".into())
        } else {
            world.social.name_of(&p.owner_id).to_string()
        }
    }

    /// startBuyPreConfirm: the summary the confirmation shows.
    fn action_summary(&self, seller: &str) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(site) = &self.site {
            if site.membership_upgrade {
                lines.push(site.membership_action.clone());
                if let Some((_, name)) = site.levels.get(self.plan) {
                    lines.push(format!(" • {name}"));
                }
            }
            if site.land_use_upgrade {
                lines.push(site.land_use_action.clone());
            }
        }
        if self.amount > 0 {
            lines.push(aurora_i18n::tr!(
                "buy-land-buy-for-local",
                amount = aurora_i18n::integer(i64::from(self.amount)),
                cost = self.local_estimate().unwrap_or_default()
            ));
        }
        lines.push(aurora_i18n::tr!(
            "buy-land-pay-to-for-land",
            amount = aurora_i18n::integer(i64::from(self.offer.price)),
            seller = seller.to_owned()
        ));
        lines
    }

    /// onClickBuy → startBuyPreConfirm: the website says which confirmation
    /// it wants (LLConfirmationManager::confirm).
    fn buy(&mut self, world: &mut World) -> Vec<NetCommand> {
        if !self.buy_enabled(world) {
            return Vec::new();
        }
        match self.site.as_ref().map(|s| s.confirm.as_str()) {
            Some("click" | "password") => {
                self.confirm = Some(String::new());
                Vec::new()
            }
            _ => self.start_buy(world, Password::default()),
        }
    }

    /// startBuyPostConfirm: runWebSitePrep, then "processing".
    fn start_buy(&mut self, world: &mut World, password: Password) -> Vec<NetCommand> {
        self.confirm = None;
        let out = self.run_web_site_prep(world, password);
        if !self.bought && self.error.is_none() {
            self.processing = true;
        }
        out
    }

    /// LLViewerParcelMgr::setupParcelBuy: the message the purchase ends
    /// with; None (with Firestorm's alert) when it cannot be built.
    fn parcel_buy(&self, world: &mut World) -> Option<LandCommand> {
        let Some(t) = &self.target else {
            alert(world, aurora_i18n::tr!("buy-land-nothing-selected"));
            return None;
        };
        let group = world.groups.active;
        if self.offer.is_claim {
            if t.spans_regions {
                alert(world, aurora_i18n::tr!("buy-land-across-regions"));
                return None;
            }
            let (west, south, east, north) = t.rect;
            Some(LandCommand::Claim {
                handle: t.handle,
                group,
                for_group: self.for_group,
                west,
                south,
                east,
                north,
            })
        } else {
            Some(LandCommand::Buy {
                handle: t.handle,
                local_id: t.parcel.local_id,
                group,
                for_group: self.for_group,
                remove_contribution: self.remove_contribution,
                price: t.parcel.sale_price,
                area: t.parcel.area,
            })
        }
    }

    /// runWebSitePrep: straight to the ParcelBuy when the website has
    /// nothing to do, else buyLandPrep first.
    fn run_web_site_prep(&mut self, world: &mut World, password: Password) -> Vec<NetCommand> {
        if !self.can_buy() {
            return Vec::new();
        }
        let buy = self.parcel_buy(world);
        let Some(site) = self.site.clone() else {
            return Vec::new();
        };
        if buy.is_some() && !site.membership_upgrade && !site.land_use_upgrade && self.amount == 0 && site.confirm != "password" {
            self.bought = true;
            return buy.into_iter().map(NetCommand::Land).collect();
        }
        let Some(helper) = self.helper else {
            return Vec::new();
        };
        let level_id = if site.membership_upgrade {
            site.levels
                .get(self.plan)
                .map_or_else(|| "noChange".to_owned(), |(id, _)| id.clone())
        } else {
            "noChange".to_owned()
        };
        self.pending = buy;
        self.site_txn = Some(SiteTxn::Buy);
        vec![NetCommand::LandToolBuy {
            helper: helper.to_owned(),
            request: LandToolBuy {
                level_id,
                billable_area: if self.for_group { 0 } else { self.offer.billable_area },
                currency_buy: self.amount,
                usd_cents: self.usd_cents.unwrap_or(0),
                local_cost: self.local_estimate().unwrap_or_default(),
                confirm: site.confirm.clone(),
                password,
            },
        }]
    }

    /// finishWebSitePrep → sendBuyLand.
    pub fn prepared(&mut self, result: Result<(), CurrencyError>) -> Vec<NetCommand> {
        if self.site_txn != Some(SiteTxn::Buy) {
            return Vec::new();
        }
        self.site_txn = None;
        match result {
            Ok(()) => match self.pending.take() {
                Some(buy) => {
                    self.bought = true;
                    vec![NetCommand::Land(buy)]
                }
                None => Vec::new(),
            },
            Err(e) => {
                self.pending = None;
                self.tell_user_error(e);
                Vec::new()
            }
        }
    }

    /// The window. `selection` is the land of the selection that opened it.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        images: &HashMap<Uuid, egui::TextureHandle>,
        selection: Selection,
    ) -> Vec<NetCommand> {
        if !self.open {
            return Vec::new();
        }
        let mut out = self.sync(world, selection, Instant::now());
        if !self.open {
            return out;
        }
        if self.demo_confirm && self.site.is_some() && self.idle() {
            self.demo_confirm = false;
            self.agreed = true;
            out.extend(self.buy(world));
        }
        let seller = self.seller(world);
        let center = ctx.content_rect().center();
        let mut open = true;
        let mut close = false;
        let mut floater = Floater::new(
            "buy_land",
            aurora_i18n::tr!("buy-land-title"),
            egui::pos2(center.x - SIZE.x / 2.0, (center.y - SIZE.y / 2.0).max(40.0)),
            SIZE,
        )
        .help(aurora_i18n::tr!("buy-land-help"));
        floater.min_size = Vec2::new(740.0, 560.0);
        // held at its size: with a larger text its body measured a little
        // more than the window, which grew at every frame
        floater.steady_size().show(ctx, p, &mut open, |ui| {
            // the buttons first, along the bottom; the body takes the rest
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), BUTTON.y),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        let cancel = ui
                            .add_enabled_ui(self.site_txn.is_none(), |ui| {
                                flat_button_sized(ui, p, aurora_i18n::tr!("common-cancel"), BUTTON)
                            })
                            .inner;
                        if cancel.clicked() {
                            close = true;
                        }
                        // greyed out until everything is in place (refreshUI)
                        let buy = widgets::action_button_if(
                            ui,
                            p,
                            "shopping-cart-filled",
                            aurora_i18n::tr!("buy-land-purchase"),
                            widgets::ActionKind::Primary,
                            BUTTON,
                            self.buy_enabled(world),
                        );
                        if buy.clicked() {
                            out.extend(self.buy(world));
                        }
                        if self.processing || self.site_txn.is_some() {
                            ui.add(egui::Spinner::new().size(14.0).color(p.violet));
                        }
                    },
                );
                ui.add_space(8.0);
                let body_h = ui.available_height();
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), body_h),
                    egui::Layout::left_to_right(egui::Align::Min),
                    |ui| {
                        ui.allocate_ui_with_layout(Vec2::new(LEFT_W, body_h), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_width(LEFT_W);
                            egui::ScrollArea::vertical()
                                .id_salt("buy_land_left")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    self.parcel_card(ui, p, images);
                                    ui.add_space(12.0);
                                    self.steps(ui, p, world, &seller, &mut close);
                                });
                        });
                        widgets::divider(ui, p);
                        ui.allocate_ui_with_layout(
                            Vec2::new(ui.available_width(), body_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_width(ui.available_width());
                                self.estate(ui, p, world);
                            },
                        );
                    },
                );
            });
        });
        out.extend(self.confirm_dialog(ctx, p, world, &seller));
        if self.bought {
            self.close();
        } else if !open || close {
            // canClose: not while the website is asked
            if self.site_txn.is_some() {
                alert(world, aurora_i18n::tr!("buy-land-cannot-close"));
            } else {
                self.close();
            }
        }
        out
    }

    /// Section zero: the parcel (info_image, info_parcel, info_size,
    /// info_price).
    fn parcel_card(&mut self, ui: &mut egui::Ui, p: &Palette, images: &HashMap<Uuid, egui::TextureHandle>) {
        let parcel = self.target.as_ref().map(|t| t.parcel.clone()).filter(|_| self.offer.valid);
        widgets::surface(ui, p, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                let snapshot = parcel.as_ref().map_or(Uuid::nil(), |p| p.snapshot_id);
                if !snapshot.is_nil() {
                    self.wanted_images.insert(snapshot);
                }
                widgets::picture(
                    ui,
                    p,
                    images.get(&snapshot),
                    PICTURE,
                    "photo-filled",
                    aurora_i18n::tr!("buy-land-no-picture"),
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    let Some(parcel) = &parcel else {
                        ui.label(RichText::new(aurora_i18n::tr!("buy-land-no-parcel-selected")).color(p.muted));
                        return;
                    };
                    if !parcel.name.is_empty() {
                        wrapped(ui, RichText::new(&parcel.name).size(15.0).strong().color(p.ink));
                    }
                    ui.horizontal(|ui| {
                        widgets::glyph(ui, "map-pin-filled", p.violet_light, 14.0);
                        ui.label(RichText::new(&self.offer.location).size(12.0).color(p.muted));
                    });
                    ui.horizontal(|ui| {
                        widgets::glyph(ui, "dimensions", p.violet_light, 14.0);
                        ui.label(
                            RichText::new(aurora_i18n::tr!(
                                "buy-land-size",
                                area = aurora_i18n::integer(i64::from(self.offer.actual_area)),
                                objects = aurora_i18n::integer(i64::from(self.offer.supported_objects))
                            ))
                            .size(12.0)
                            .color(p.ink),
                        );
                    });
                    if self.offer.for_sale {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(aurora_i18n::tr!(
                                    "buy-land-price",
                                    amount = aurora_i18n::integer(i64::from(self.offer.price))
                                ))
                                .size(18.0)
                                .strong()
                                .color(p.ink),
                            );
                            ui.label(
                                RichText::new(aurora_i18n::tr!(
                                    "buy-land-price-per-m2",
                                    price = aurora_i18n::number(f64::from(self.offer.price_per_m2()), 1)
                                ))
                                .size(12.0)
                                .color(p.muted),
                            );
                        });
                        let (icon, text, color) = if self.offer.sold_with_objects {
                            ("box", aurora_i18n::tr!("buy-land-sold-with-objects"), p.success)
                        } else {
                            ("box", aurora_i18n::tr!("buy-land-sold-without-objects"), p.muted)
                        };
                        ui.horizontal(|ui| {
                            widgets::glyph(ui, icon, color, 14.0);
                            ui.label(RichText::new(text).size(12.0).color(color));
                        });
                    }
                });
            });
        });
    }

    /// The error section, or the three steps (refreshUI).
    fn steps(&mut self, ui: &mut egui::Ui, p: &Palette, world: &mut World, seller: &str, close: &mut bool) {
        let can_buy = self.can_buy();
        let heading = if self.processing {
            aurora_i18n::tr!("buy-land-processing-title")
        } else if can_buy {
            if self.for_group {
                aurora_i18n::tr!("buy-land-buying-for-group")
            } else {
                aurora_i18n::tr!("buy-land-buying-will")
            }
        } else if self.error.is_some() {
            aurora_i18n::tr!("buy-land-cannot-buy-now")
        } else {
            aurora_i18n::tr!("buy-land-not-for-sale")
        };
        ui.label(RichText::new(heading).size(17.0).strong().color(p.ink));
        ui.add_space(6.0);
        if self.processing {
            step(ui, p, Badge::Note, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0).color(p.violet));
                    wrapped(ui, RichText::new(aurora_i18n::tr!("buy-land-processing")).color(p.ink));
                });
            });
            return;
        }
        if !can_buy || self.site.is_none() {
            let badge = if self.error.is_some() { Badge::Error } else { Badge::Warn };
            step(ui, p, badge, |ui| {
                if let Some(e) = &self.error {
                    wrapped(ui, RichText::new(aurora_i18n::tr!("buy-land-fetching-error")).strong().color(p.ink));
                    if !e.message.is_empty() {
                        wrapped(ui, RichText::new(&e.message).size(12.0).color(p.ink));
                    }
                    // the website's page, never another scheme
                    if (e.uri.starts_with("https://") || e.uri.starts_with("http://"))
                        && widgets::action_button(
                            ui,
                            p,
                            "external-link-filled",
                            aurora_i18n::tr!("buy-land-go-to-website"),
                            widgets::ActionKind::Flat,
                            Vec2::new(170.0, 26.0),
                        )
                        .clicked()
                    {
                        super::chat::open_web_link(ui.ctx(), &e.uri);
                        *close = true;
                    }
                } else if let Some(refusal) = self.offer.refusal {
                    wrapped(ui, RichText::new(refusal.text()).color(p.ink));
                } else {
                    // can buy, the website has not answered yet
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0).color(p.violet));
                        ui.label(RichText::new(aurora_i18n::tr!("buy-land-waiting-for-data")).color(p.muted));
                    });
                }
            });
            return;
        }
        let Some(site) = self.site.clone() else {
            return;
        };
        // section one: account
        step(ui, p, if site.membership_upgrade { Badge::Note } else { Badge::Ok }, |ui| {
            if !site.membership_action.is_empty() {
                wrapped(ui, RichText::new(&site.membership_action).strong().color(p.ink));
            }
            wrapped(
                ui,
                RichText::new(if site.membership_upgrade {
                    aurora_i18n::tr!("buy-land-must-upgrade")
                } else {
                    aurora_i18n::tr!("buy-land-cant-own-land")
                })
                .size(12.0)
                .color(p.muted),
            );
            if site.membership_upgrade && !site.levels.is_empty() {
                let current = site.levels.get(self.plan).map_or("", |(_, n)| n.as_str());
                egui::ComboBox::from_id_salt("buy_land_plan")
                    .selected_text(current)
                    .width(ui.available_width().min(320.0))
                    .show_ui(ui, |ui| {
                        for (i, (_, name)) in site.levels.iter().enumerate() {
                            if ui.selectable_label(self.plan == i, name).clicked() {
                                self.plan = i;
                            }
                        }
                    });
            }
        });
        // section two: land use fees
        step(ui, p, if site.land_use_upgrade { Badge::Note } else { Badge::Ok }, |ui| {
            if !site.land_use_action.is_empty() {
                wrapped(ui, RichText::new(&site.land_use_action).strong().color(p.ink));
            }
            let mut lines = Vec::new();
            if self.for_group {
                lines.push(aurora_i18n::tr!(
                    "buy-land-insufficient-land-credits",
                    group = world.groups.group(&world.groups.active).map(|g| g.name.clone()).unwrap_or_default()
                ));
            } else {
                lines.push(aurora_i18n::tr!(
                    "buy-land-land-holdings",
                    area = aurora_i18n::integer(i64::from(world.land_committed))
                ));
            }
            lines.push(match land_buy::area_note(&self.offer) {
                AreaNote::NoParcel => aurora_i18n::tr!("buy-land-no-parcel-selected").to_owned(),
                AreaNote::Plain(a) => aurora_i18n::tr!("buy-land-parcel-meters", area = aurora_i18n::integer(i64::from(a))),
                AreaNote::Premium(a) => aurora_i18n::tr!("buy-land-premium-land", area = aurora_i18n::integer(i64::from(a))),
                AreaNote::Discounted(a) => {
                    aurora_i18n::tr!("buy-land-discounted-land", area = aurora_i18n::integer(i64::from(a)))
                }
            });
            wrapped(ui, RichText::new(lines.join(" ")).size(12.0).color(p.muted));
        });
        // section three: purchase & currency
        let funds = self.funds(world);
        let badge = if !funds.will_have_enough() {
            Badge::Warn
        } else if self.amount > 0 {
            Badge::Note
        } else {
            Badge::Ok
        };
        step(ui, p, badge, |ui| {
            wrapped(
                ui,
                RichText::new(aurora_i18n::tr!(
                    "buy-land-pay-to-for-land",
                    amount = aurora_i18n::integer(i64::from(self.offer.price)),
                    seller = seller.to_owned()
                ))
                .strong()
                .color(p.ink),
            );
            let balance = i64::from(funds.balance);
            wrapped(
                ui,
                RichText::new(if funds.have_enough() {
                    aurora_i18n::tr!("buy-land-have-enough", amount = aurora_i18n::integer(balance))
                } else {
                    aurora_i18n::tr!(
                        "buy-land-not-enough",
                        amount = aurora_i18n::integer(balance),
                        missing = aurora_i18n::integer(i64::from(funds.shortfall()))
                    )
                })
                .size(12.0)
                .color(p.muted),
            );
            // mCurrency.updateUI(!haveEnough): the L$ to buy with the land
            if !funds.have_enough() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(aurora_i18n::tr!("buy-land-buy-additional")).size(12.0).color(p.ink));
                    let field = crate::ui::widgets::text_field(ui, self.site_txn.is_none(), &mut self.text, |b, _| {
                        egui::TextEdit::singleline(b)
                            .desired_width(80.0)
                            .char_limit(10)
                            .align(egui::Align2::RIGHT_TOP)
                    });
                    ui.label(RichText::new("L$").size(12.0).color(p.muted));
                    if field.changed() {
                        self.typed(Instant::now());
                    }
                    if self.amount > 0 {
                        match self.local_estimate().filter(|_| !self.changed && self.quote_asked.is_none()) {
                            Some(cost) => {
                                ui.label(
                                    RichText::new(aurora_i18n::tr!("buy-land-for-approx", cost = cost))
                                        .size(12.0)
                                        .color(p.violet_light),
                                );
                            }
                            None => {
                                ui.add(egui::Spinner::new().size(12.0).color(p.violet));
                            }
                        }
                    }
                });
            }
            let final_balance = funds.final_balance();
            let (text, color) = if funds.will_have_enough() {
                (
                    aurora_i18n::tr!("buy-land-balance-left", amount = aurora_i18n::integer(final_balance)),
                    p.ink,
                )
            } else {
                (
                    aurora_i18n::tr!("buy-land-balance-needed", amount = aurora_i18n::integer(-final_balance)),
                    p.warn,
                )
            };
            wrapped(ui, RichText::new(text).size(12.0).color(color));
            // remove_contribution: a group seller, and a contribution of ours
            let min = land_buy::min_contribution(self.offer.billable_area);
            if self.offer.is_group_land && self.offer.group_contribution > 0 {
                let enough = self.offer.group_contribution >= min;
                if !enough {
                    self.remove_contribution = false;
                }
                ui.add_enabled_ui(enough, |ui| {
                    widgets::checkbox(
                        ui,
                        p,
                        &mut self.remove_contribution,
                        aurora_i18n::tr!("buy-land-remove-contribution", area = aurora_i18n::integer(i64::from(min))),
                    );
                });
            }
        });
    }

    /// The right column: the region and its covenant.
    fn estate(&mut self, ui: &mut egui::Ui, p: &Palette, world: &mut World) {
        let handle = self.target.as_ref().map(|t| t.handle);
        let info = handle.and_then(|h| world.regions.get(&h)).and_then(|r| r.info.clone());
        let covenant = world.land.covenant.clone().filter(|c| Some(c.handle) == handle);
        let unknown = aurora_i18n::tr!("buy-land-unknown");
        widgets::section_header(ui, p, "world-filled", aurora_i18n::tr!("buy-land-region-section"));
        ui.add_space(4.0);
        let row = |ui: &mut egui::Ui, label: &str, value: &mut dyn FnMut(&mut egui::Ui)| {
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(Vec2::new(150.0, 16.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_width(150.0);
                    ui.add(egui::Label::new(RichText::new(label).size(12.0).color(p.muted)).truncate());
                });
                value(ui);
            });
        };
        row(ui, aurora_i18n::tr!("buy-land-region"), &mut |ui| {
            match &info {
                Some(i) => {
                    ui.label(RichText::new(&i.name).size(12.0).strong().color(p.ink));
                    widgets::maturity_badge_of(ui, p, i.sim_access, 14.0);
                }
                None => {
                    ui.label(RichText::new(unknown).size(12.0).color(p.muted));
                }
            };
        });
        row(ui, aurora_i18n::tr!("buy-land-type"), &mut |ui| {
            let t = info
                .as_ref()
                .map_or_else(|| unknown.to_owned(), |i| super::land::product_name(&i.product_name));
            ui.add(egui::Label::new(RichText::new(t).size(12.0).color(p.ink)).truncate());
        });
        row(ui, aurora_i18n::tr!("buy-land-estate"), &mut |ui| {
            let t = covenant.as_ref().map_or(unknown, |c| c.estate_name.as_str());
            ui.add(egui::Label::new(RichText::new(t).size(12.0).color(p.ink)).truncate());
        });
        let owner = covenant
            .as_ref()
            .filter(|c| !c.estate_owner.is_nil())
            .map(|c| world.social.name_of(&c.estate_owner).to_string());
        row(ui, aurora_i18n::tr!("buy-land-estate-owner"), &mut |ui| {
            ui.add(egui::Label::new(RichText::new(owner.as_deref().unwrap_or(unknown)).size(12.0).color(p.ink)).truncate());
        });
        ui.add_space(6.0);
        ui.label(RichText::new(aurora_i18n::tr!("buy-land-purchased-land")).size(12.0).color(p.ink));
        if let Some(i) = &info {
            let clause = |ui: &mut egui::Ui, yes: bool, on: &str, off: &str| {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    if yes {
                        widgets::glyph(ui, "circle-check-filled", p.success, 14.0);
                    } else {
                        widgets::glyph(ui, "circle-x-filled", p.warn, 14.0);
                    }
                    ui.label(RichText::new(if yes { on } else { off }).size(12.0).color(p.ink));
                });
            };
            clause(
                ui,
                i.region_flags & rf::BLOCK_LAND_RESELL == 0,
                aurora_i18n::tr!("buy-land-can-resell"),
                aurora_i18n::tr!("buy-land-can-not-resell"),
            );
            clause(
                ui,
                i.region_flags & rf::ALLOW_PARCEL_CHANGES != 0,
                aurora_i18n::tr!("buy-land-can-change"),
                aurora_i18n::tr!("buy-land-can-not-change"),
            );
        }
        ui.add_space(10.0);
        widgets::section_header(ui, p, "file-description-filled", aurora_i18n::tr!("buy-land-covenant-section"));
        let no_covenant = self.no_covenant(world);
        if !no_covenant {
            ui.label(RichText::new(aurora_i18n::tr!("buy-land-must-agree")).size(12.0).color(p.muted));
        }
        if let Some(c) = covenant.as_ref().filter(|c| c.timestamp != 0) {
            ui.label(
                RichText::new(aurora_i18n::tr!(
                    "ui-land-covenant-last-modified",
                    when = super::land::covenant_date(i64::from(c.timestamp))
                ))
                .size(11.0)
                .color(p.muted_dim),
            );
        }
        let body = covenant
            .as_ref()
            .and_then(|c| c.text.clone())
            .unwrap_or_else(|| aurora_i18n::tr!("common-loading").into());
        // the checkbox along the bottom, the covenant text above it
        let mut agreed = self.agrees(world);
        let lock = no_covenant || self.site_txn == Some(SiteTxn::Buy);
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            // agree_covenant: ticked and locked without a covenant
            ui.add_enabled_ui(!lock, |ui| {
                if widgets::checkbox(ui, p, &mut agreed, aurora_i18n::tr!("buy-land-agree-covenant")).changed() {
                    self.agreed = agreed;
                }
            });
            ui.add_space(4.0);
            let box_h = (ui.available_height() - 16.0).max(60.0);
            egui::Frame::new()
                .fill(p.field)
                .stroke(egui::Stroke::new(1.0, p.raised))
                .corner_radius(ui.visuals().window_corner_radius)
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    // the text reads from its top, whatever the column's layout
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("buy_land_covenant")
                            .max_height(box_h)
                            .min_scrolled_height(box_h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(RichText::new(body).size(12.0).color(p.ink))
                                        .wrap()
                                        .selectable(true),
                                );
                            });
                    });
                });
        });
    }

    /// ConfirmPurchase / ConfirmPurchasePassword.
    fn confirm_dialog(&mut self, ctx: &egui::Context, p: &Palette, world: &mut World, seller: &str) -> Vec<NetCommand> {
        let Some(mut password) = self.confirm.take() else {
            return Vec::new();
        };
        let needs_password = self.site.as_ref().is_some_and(|s| s.confirm == "password");
        let lines = self.action_summary(seller);
        let mut answer = None;
        let modal = Modal::new(egui::Id::unique("buy_land_confirm")).show(ctx, |ui| {
            ui.set_width(CONFIRM_WIDTH);
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
            ui.horizontal(|ui| {
                widgets::glyph(ui, "shopping-cart-filled", p.violet_light, 20.0);
                ui.label(RichText::new(aurora_i18n::tr!("buy-land-title")).size(17.0).strong().color(p.ink));
            });
            ui.label(RichText::new(aurora_i18n::tr!("currency-confirm-intro")).color(p.ink));
            for line in &lines {
                wrapped(ui, RichText::new(line).strong().color(p.ink));
            }
            wrapped(ui, RichText::new(aurora_i18n::tr!("currency-confirm-question")).color(p.ink));
            if needs_password {
                wrapped(ui, RichText::new(aurora_i18n::tr!("currency-confirm-password")).color(p.ink));
                let field = ui.add(
                    egui::TextEdit::singleline(&mut password)
                        .password(true)
                        .desired_width(f32::INFINITY),
                );
                if !field.has_focus() && password.is_empty() {
                    field.request_focus();
                }
            }
            let size = Vec2::new((ui.available_width() - ui.spacing().item_spacing.x) / 2.0, BUTTON.y);
            ui.horizontal(|ui| {
                let ok = widgets::action_button_if(
                    ui,
                    p,
                    "",
                    aurora_i18n::tr!("common-ok"),
                    widgets::ActionKind::Primary,
                    size,
                    !needs_password || !password.is_empty(),
                );
                if ok.clicked() {
                    answer = Some(true);
                }
                if flat_button_sized(ui, p, aurora_i18n::tr!("common-cancel"), size).clicked() {
                    answer = Some(false);
                }
            });
        });
        if modal.should_close() {
            answer = answer.or(Some(false));
        }
        match answer {
            Some(true) => self.start_buy(world, Password(password)),
            Some(false) => Vec::new(),
            None => {
                self.confirm = Some(password);
                Vec::new()
            }
        }
    }
}

/// A step of the purchase: its badge, then what it says.
fn step(ui: &mut egui::Ui, p: &Palette, badge: Badge, body: impl FnOnce(&mut egui::Ui)) {
    let (icon, color) = match badge {
        Badge::Ok => ("circle-check-filled", p.success),
        Badge::Note => ("info-circle-filled", p.violet_light),
        Badge::Warn => ("alert-triangle-filled", p.warn),
        Badge::Error => ("circle-x-filled", p.danger),
    };
    widgets::surface(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            widgets::glyph(ui, icon, color, 22.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                body(ui);
            });
        });
    });
    ui.add_space(6.0);
}

/// One of Firestorm's alertmodal notifications of a purchase.
fn alert(world: &mut World, text: &str) {
    world
        .notifications
        .push(Kind::Alert, aurora_i18n::tr!("buy-land-title"), text.to_owned(), Data::None);
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELPER: &str = "https://helper.invalid/";

    fn test_world() -> World {
        World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()))
    }

    fn site(confirm: &str) -> LandToolInfo {
        LandToolInfo {
            confirm: confirm.into(),
            ..Default::default()
        }
    }

    /// A window that can buy, its offer set by hand (the rules have their
    /// own tests in world/land_buy.rs).
    fn ready(price: i32, amount: i32) -> BuyLandUi {
        BuyLandUi {
            open: true,
            helper: Some(HELPER),
            offer: Offer {
                valid: true,
                for_sale: true,
                price,
                actual_area: 512,
                billable_area: 512,
                ..Default::default()
            },
            amount,
            ..Default::default()
        }
    }

    /// The window keeps its size, the covenant's box over its checkbox.
    #[test]
    fn the_window_keeps_its_size() {
        crate::ui::widgets::steady_tests::assert_window_steady(
            "buy_land",
            &[SIZE, Vec2::new(740.0, 560.0)],
            aurora_i18n::tr!("common-cancel"),
            || (ready(4500, 300), test_world()),
            |(ui, world), ctx, p| {
                ui.show(ctx, p, world, &HashMap::new(), Selection::Pending);
            },
        );
    }

    #[test]
    fn one_preflight_per_area_and_amount() {
        let mut ui = ready(4500, 300);
        let first = ui.update_web_site_info();
        assert!(matches!(
            first,
            Some(NetCommand::LandToolPreflight {
                billable_area: 512,
                currency_buy: 300,
                ..
            })
        ));
        assert!(ui.update_web_site_info().is_none());
        // a stale answer is dropped, the right one fills the site side
        ui.preflight_reply(512, 100, Ok(site("click")));
        assert!(ui.site.is_none() && ui.site_txn.is_some());
        let info = LandToolInfo {
            usd_cents: Some(129),
            ..site("click")
        };
        ui.preflight_reply(512, 300, Ok(info));
        assert!(ui.site.is_some() && ui.site_txn.is_none());
        assert_eq!(
            ui.local_estimate().as_deref(),
            Some(&*aurora_i18n::tr!("currency-usd", amount = aurora_i18n::number(1.29, 2)))
        );
    }

    #[test]
    fn group_purchase_bills_no_area() {
        let mut ui = ready(10, 0);
        ui.for_group = true;
        assert!(matches!(
            ui.update_web_site_info(),
            Some(NetCommand::LandToolPreflight { billable_area: 0, .. })
        ));
    }

    #[test]
    fn no_helper_is_an_error() {
        let mut ui = ready(10, 0);
        ui.helper = None;
        assert!(ui.update_web_site_info().is_none());
        assert!(ui.error.is_some() && !ui.can_buy());
    }

    #[test]
    fn refusal_of_the_website_stops_the_purchase() {
        let mut ui = ready(10, 0);
        ui.update_web_site_info();
        ui.preflight_reply(
            512,
            0,
            Err(CurrencyError {
                message: "Account on hold".into(),
                uri: "https://example.invalid/".into(),
            }),
        );
        assert!(!ui.can_buy() && ui.site.is_none());
    }

    #[test]
    fn typed_amount_is_estimated_after_a_pause() {
        let mut ui = ready(4500, 3300);
        ui.text = "4000".into();
        let t0 = Instant::now();
        ui.typed(t0);
        assert!(!ui.idle());
        assert!(ui.poll(t0 + Duration::from_millis(500)).is_none());
        assert!(matches!(
            ui.poll(t0 + ESTIMATE_PAUSE),
            Some(NetCommand::CurrencyQuote { amount: 4000, .. })
        ));
        let quote = CurrencyQuote {
            amount: 4000,
            usd_cents: Some(1600),
            local_cost: Some("US$ 16.00".into()),
            confirm: "click".into(),
        };
        // the other window's estimate is not ours
        assert!(!ui.quote_reply(1000, &Ok(quote.clone())));
        assert!(ui.quote_reply(4000, &Ok(quote)));
        assert!(ui.idle());
        assert_eq!(ui.local_estimate().as_deref(), Some("US$ 16.00"));
    }

    #[test]
    fn nothing_to_prepare_buys_at_once() {
        let mut ui = ready(10, 0);
        ui.site = Some(site(""));
        ui.target = Some(Target {
            handle: 7,
            parcel: Arc::new(ParcelInfo {
                local_id: 3,
                sale_price: 10,
                area: 512,
                ..Default::default()
            }),
            multiple: false,
            rect: (0.0, 0.0, 16.0, 16.0),
            spans_regions: false,
        });
        let mut world = test_world();
        let out = ui.run_web_site_prep(&mut world, Password::default());
        assert!(matches!(
            out.as_slice(),
            [NetCommand::Land(LandCommand::Buy {
                handle: 7,
                local_id: 3,
                price: 10,
                area: 512,
                for_group: false,
                ..
            })]
        ));
        assert!(ui.bought);
    }

    #[test]
    fn website_first_when_it_has_work() {
        let mut ui = ready(4500, 3300);
        ui.usd_cents = Some(1300);
        ui.site = Some(LandToolInfo {
            membership_upgrade: true,
            levels: vec![("basic".into(), "Basic".into()), ("premium".into(), "Premium".into())],
            ..site("password")
        });
        ui.plan = 1;
        ui.target = Some(Target {
            handle: 7,
            parcel: Arc::new(ParcelInfo {
                local_id: 3,
                sale_price: 4500,
                area: 512,
                ..Default::default()
            }),
            multiple: false,
            rect: (0.0, 0.0, 16.0, 16.0),
            spans_regions: false,
        });
        let mut world = test_world();
        let out = ui.start_buy(&mut world, Password("typed".into()));
        let [NetCommand::LandToolBuy { request, .. }] = out.as_slice() else {
            panic!("buyLandPrep expected, got {out:?}");
        };
        assert_eq!(request.level_id, "premium");
        assert_eq!((request.billable_area, request.currency_buy, request.usd_cents), (512, 3300, 1300));
        assert!(ui.processing && !ui.bought);
        // the website done, the ParcelBuy follows and the window closes
        let after = ui.prepared(Ok(()));
        assert!(matches!(after.as_slice(), [NetCommand::Land(LandCommand::Buy { .. })]));
        assert!(ui.bought);
        // a late answer is not a second purchase
        assert!(ui.prepared(Ok(())).is_empty());
    }

    #[test]
    fn website_refusal_after_confirmation() {
        let mut ui = ready(4500, 3300);
        ui.site = Some(site("click"));
        ui.target = Some(Target {
            handle: 7,
            parcel: Arc::new(ParcelInfo::default()),
            multiple: false,
            rect: (0.0, 0.0, 16.0, 16.0),
            spans_regions: false,
        });
        let mut world = test_world();
        ui.start_buy(&mut world, Password::default());
        assert!(ui.prepared(Err(CurrencyError::default())).is_empty());
        assert!(!ui.processing && ui.error.is_some() && !ui.bought);
    }

    #[test]
    fn claim_across_regions_is_refused() {
        let mut ui = ready(64, 0);
        ui.offer.is_claim = true;
        ui.site = Some(site(""));
        ui.target = Some(Target {
            handle: 7,
            parcel: Arc::new(ParcelInfo::default()),
            multiple: false,
            rect: (240.0, 0.0, 264.0, 16.0),
            spans_regions: true,
        });
        let mut world = test_world();
        assert!(
            ui.run_web_site_prep(&mut world, Password::default())
                .iter()
                .all(|c| !matches!(c, NetCommand::Land(_)))
        );
        if let Some(t) = ui.target.as_mut() {
            t.spans_regions = false;
        }
        assert!(matches!(
            ui.run_web_site_prep(&mut world, Password::default()).as_slice(),
            [NetCommand::Land(LandCommand::Claim {
                west: 240.0,
                east: 264.0,
                ..
            })]
        ));
    }
}

//! "Buy L$", opened by the balance of the top bar. Port of
//! LLFloaterBuyCurrencyUI, of the estimate / purchase cycle of
//! LLCurrencyUIManager and of LLFloaterAddPaymentMethod (Firestorm
//! indra/newview/llfloaterbuycurrency.cpp, llcurrencyuimanager.cpp,
//! llfloateraddpaymentmethod.cpp, originally LGPL 2.1).
//!
//! Differences from Firestorm, on purpose:
//! - a refusal of the currency helper is shown in the window, which keeps
//!   its amount (Firestorm closes it and raises a notification);
//! - an estimate that comes back for an amount typed over since is dropped
//!   (Firestorm puts the former amount back in the field);
//! - the account without a payment method gets one view with the
//!   explanation and the link (Firestorm: a window, then a notification);
//! - a profile that does not answer no longer keeps the window from opening:
//!   the helper is the one that accepts or refuses the purchase.

use super::widgets::{self, Floater, Modal, flat_button_sized, primary_button_sized};
use crate::theme::Palette;
use crate::world::World;
use aurora_net::NetCommand;
use aurora_net::currency::{CurrencyError, CurrencyQuote, Password};
use egui::{RichText, Vec2};
use std::time::{Duration, Instant};

/// Pause in typing before an estimate is asked (CURRENCY_ESTIMATE_FREQUENCY).
const ESTIMATE_PAUSE: Duration = Duration::from_secs(2);
/// LLFloaterAddPaymentMethod's `continue_url`.
const ADD_PAYMENT_URL: &str = "https://secondlife.com/my/lindex/buy.php?associate_for_viewer=1";
/// Account page named by BuyLindenDollarSuccess.
pub const ACCOUNT_URL: &str = "http://secondlife.com/account/";
/// How long our own profile may take to tell whether the account can pay.
const PROFILE_WAIT: Duration = Duration::from_secs(8);
const WIDTH: f32 = 320.0;
const CONFIRM_WIDTH: f32 = 400.0;
const BUTTON_HEIGHT: f32 = 26.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    /// The grid has no known currency helper.
    Unavailable,
    /// Waiting for our own profile (LLFetchAvatarPaymentInfo).
    Checking,
    /// No payment method on file (LLFloaterAddPaymentMethod).
    NoPayment,
    Form,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Transaction {
    /// Estimate asked for this amount.
    Quote(i32),
    Buy,
}

pub struct BuyCurrencyUi {
    pub open: bool,
    helper: Option<&'static str>,
    stage: Stage,
    /// When the profile was asked for (`Stage::Checking`).
    opened: Option<Instant>,
    /// Digits of the amount field.
    text: String,
    /// mUserCurrencyBuy.
    amount: i32,
    /// mCurrencyChanged: the amount has no estimate asked yet.
    changed: bool,
    /// mCurrencyKeyTimer; None when the amount was not typed.
    last_key: Option<Instant>,
    quote: Option<CurrencyQuote>,
    transaction: Option<Transaction>,
    /// Confirmation wanted by the helper, being answered: the password typed.
    confirm: Option<String>,
    error: Option<CurrencyError>,
    focus: bool,
    /// AURORA_DEMO_BUY_CURRENCY: open the confirmation once the estimate is in.
    demo_confirm: bool,
}

impl Default for BuyCurrencyUi {
    fn default() -> Self {
        BuyCurrencyUi {
            open: false,
            helper: None,
            stage: Stage::Unavailable,
            opened: None,
            text: String::new(),
            amount: 0,
            changed: false,
            last_key: None,
            quote: None,
            transaction: None,
            confirm: None,
            error: None,
            focus: false,
            demo_confirm: false,
        }
    }
}

fn money(amount: i64) -> String {
    format!("{} L$", aurora_i18n::integer(amount))
}

/// The amount as the field shows it: nothing for zero (updateUI).
fn amount_text(amount: i32) -> String {
    if amount == 0 { String::new() } else { amount.to_string() }
}

/// LocalEstimateUSD, from the US cents of an old helper.
fn usd(cents: i32) -> String {
    aurora_i18n::tr!("currency-usd", amount = aurora_i18n::number(f64::from(cents) / 100.0, 2))
}

/// A row of the summary: its label, and its value against the right edge.
fn row(ui: &mut egui::Ui, p: &Palette, label: &str, value: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(p.ink));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), value);
    });
}

fn wrapped(ui: &mut egui::Ui, text: RichText) {
    ui.add(egui::Label::new(text).wrap());
}

impl BuyCurrencyUi {
    /// LLFloaterBuyCurrency::buyCurrency: an empty amount, once the profile
    /// has told whether the account can pay. `helper` is the grid's helper
    /// page (`aurora_net::currency::helper_uri`).
    pub fn open(&mut self, world: &mut World, helper: Option<&'static str>) {
        if self.buying() {
            // the purchase in flight keeps its window
            self.open = true;
            return;
        }
        *self = BuyCurrencyUi {
            open: true,
            helper,
            stage: if helper.is_some() { Stage::Checking } else { Stage::Unavailable },
            opened: Some(Instant::now()),
            focus: true,
            ..Default::default()
        };
        if helper.is_some() {
            world.profiles.refresh(world.agent_id);
        }
    }

    /// LLCurrencyUIManager::setAmount: an amount chosen by the viewer,
    /// estimated without the typing pause.
    pub fn set_amount(&mut self, amount: i32) {
        self.amount = amount.max(0);
        self.text = amount_text(self.amount);
        self.quote = None;
        self.changed = true;
        self.last_key = None;
    }

    fn buying(&self) -> bool {
        self.transaction == Some(Transaction::Buy)
    }

    /// canBuy.
    fn can_buy(&self) -> bool {
        self.transaction.is_none() && self.quote.is_some() && self.amount > 0
    }

    /// currencyKey, after the field changed (validateNonNegativeS32: digits
    /// that fit an S32).
    fn typed(&mut self, now: Instant) {
        self.text.retain(|c| c.is_ascii_digit());
        let value = if self.text.is_empty() {
            Some(0)
        } else {
            self.text.parse::<i32>().ok()
        };
        let Some(value) = value else {
            self.text = amount_text(self.amount);
            return;
        };
        self.last_key = Some(now);
        // typing again dismisses a refusal, and asks again for the same amount
        let retry = self.error.take().is_some();
        if value == self.amount && !retry {
            return;
        }
        self.amount = value;
        self.quote = None;
        self.changed = true;
    }

    /// considerUpdateCurrency + updateCurrencyInfo: one estimate at a time,
    /// once the typing has paused.
    fn poll(&mut self, now: Instant) -> Option<NetCommand> {
        if self.stage != Stage::Form || !self.changed || self.transaction.is_some() {
            return None;
        }
        if self.last_key.is_some_and(|t| now.duration_since(t) < ESTIMATE_PAUSE) {
            return None;
        }
        self.changed = false;
        self.quote = None;
        if self.amount == 0 {
            return None;
        }
        self.transaction = Some(Transaction::Quote(self.amount));
        Some(NetCommand::CurrencyQuote {
            helper: self.helper?.to_owned(),
            amount: self.amount,
        })
    }

    /// finishCurrencyInfo.
    pub fn quote_reply(&mut self, amount: i32, result: Result<CurrencyQuote, CurrencyError>) {
        if self.transaction != Some(Transaction::Quote(amount)) {
            return;
        }
        self.transaction = None;
        if self.changed || amount != self.amount {
            return;
        }
        match result {
            Ok(quote) => {
                // the helper may round the amount: the field follows
                if quote.amount != self.amount {
                    self.amount = quote.amount;
                    self.text = quote.amount.to_string();
                }
                self.quote = Some(quote);
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// LLCurrencyUIManager::buy: the helper says which confirmation it wants
    /// (LLConfirmationManager::confirm).
    fn buy(&mut self) -> Vec<NetCommand> {
        let mut out = Vec::new();
        if !self.can_buy() {
            return out;
        }
        match self.quote.as_ref().map(|q| q.confirm.as_str()) {
            Some("click" | "password") => self.confirm = Some(String::new()),
            _ => out.extend(self.start_buy(Password::default())),
        }
        // onClickBuy: Update L$ balance
        out.push(NetCommand::RequestBalance);
        out
    }

    /// startCurrencyBuy.
    fn start_buy(&mut self, password: Password) -> Option<NetCommand> {
        self.confirm = None;
        let helper = self.helper?;
        let quote = self.quote.take()?;
        self.transaction = Some(Transaction::Buy);
        self.changed = false;
        Some(NetCommand::BuyCurrency {
            helper: helper.to_owned(),
            quote,
            password,
        })
    }

    /// finishCurrencyBuy; true when the L$ were bought (the window closes,
    /// BuyLindenDollarSuccess follows).
    pub fn bought(&mut self, result: Result<(), CurrencyError>) -> bool {
        if !self.buying() {
            return false;
        }
        self.transaction = None;
        match result {
            Ok(()) => {
                *self = BuyCurrencyUi::default();
                true
            }
            Err(e) => {
                self.error = Some(e);
                false
            }
        }
    }

    /// Cost shown under the amount; None while it is being estimated.
    fn estimate_text(&self) -> Option<String> {
        if self.amount == 0 {
            return Some(aurora_i18n::tr!("currency-estimate-zero").into());
        }
        let quote = self.quote.as_ref()?;
        quote.local_cost.clone().or_else(|| quote.usd_cents.map(usd))
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &World) -> Vec<NetCommand> {
        let mut out = Vec::new();
        if !self.open {
            return out;
        }
        if self.stage == Stage::Checking {
            if !world.profiles.awaiting(&world.agent_id) {
                // LLFloaterBuyCurrency::handleBuyCurrency
                let on_file = world
                    .profiles
                    .get(&world.agent_id)
                    .is_some_and(crate::world::profiles::payment_info_on_file);
                self.stage = if on_file { Stage::Form } else { Stage::NoPayment };
            } else if self.opened.is_some_and(|t| t.elapsed() >= PROFILE_WAIT) {
                self.stage = Stage::Form;
            }
        }
        out.extend(self.poll(Instant::now()));
        if self.demo_confirm && self.can_buy() {
            self.demo_confirm = false;
            self.confirm = Some(String::new());
        }
        let center = ctx.content_rect().center();
        let mut open = true;
        let mut close = false;
        Floater::new(
            "buy_currency",
            aurora_i18n::tr!("currency-title"),
            egui::pos2(center.x - WIDTH / 2.0 - 10.0, center.y - 130.0),
            Vec2::new(WIDTH, 220.0),
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            ui.set_width(WIDTH);
            ui.add_space(4.0);
            match self.stage {
                Stage::Form => self.form(ui, p, world, &mut out, &mut close),
                Stage::NoPayment => self.no_payment(ui, p, &mut close),
                Stage::Checking => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0).color(p.violet));
                        ui.label(RichText::new(aurora_i18n::tr!("currency-checking-account")).color(p.muted));
                    });
                    ui.add_space(4.0);
                }
                Stage::Unavailable => {
                    wrapped(
                        ui,
                        RichText::new(aurora_i18n::tr!("currency-unavailable-on-this-grid")).color(p.ink),
                    );
                    ui.add_space(10.0);
                    let size = Vec2::new((WIDTH - ui.spacing().item_spacing.x) / 2.0, BUTTON_HEIGHT);
                    ui.horizontal(|ui| {
                        ui.add_space(size.x + ui.spacing().item_spacing.x);
                        if flat_button_sized(ui, p, aurora_i18n::tr!("common-close"), size).clicked() {
                            close = true;
                        }
                    });
                }
            }
        });
        self.confirm_dialog(ctx, p, &mut out);
        // canClose: not while the purchase is with the helper
        if (!open || close) && !self.buying() {
            *self = BuyCurrencyUi::default();
        }
        out
    }

    fn form(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, out: &mut Vec<NetCommand>, close: &mut bool) {
        let buying = self.buying();
        let balance = world.balance.map(i64::from);
        widgets::surface(ui, p, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            row(ui, p, aurora_i18n::tr!("currency-you-have"), |ui| {
                ui.label(RichText::new(balance.map_or_else(|| "—".to_owned(), money)).strong().color(p.ink));
            });
            // the amount leaves while the helper holds the purchase (updateUI)
            ui.add_enabled_ui(!buying, |ui| {
                row(ui, p, aurora_i18n::tr!("currency-you-want-to-buy"), |ui| {
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.text)
                            .desired_width(96.0)
                            .char_limit(10)
                            .align(egui::Align2::RIGHT_TOP),
                    );
                    ui.label(RichText::new("L$").color(p.muted));
                    if field.changed() {
                        self.typed(Instant::now());
                    }
                    if std::mem::take(&mut self.focus) {
                        field.request_focus();
                    }
                });
            });
            // one line, whatever it says: the window keeps its height
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 16.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if buying {
                        ui.label(
                            RichText::new(aurora_i18n::tr!("currency-contacting-lindex"))
                                .size(12.0)
                                .color(p.muted),
                        );
                        ui.add(egui::Spinner::new().size(12.0).color(p.violet));
                    } else if let Some(cost) = self.estimate_text() {
                        ui.label(
                            RichText::new(aurora_i18n::tr!("currency-about", amount = cost))
                                .size(12.0)
                                .color(p.violet_light),
                        );
                    } else if self.error.is_none() {
                        ui.label(RichText::new(aurora_i18n::tr!("currency-estimating")).size(12.0).color(p.muted));
                        ui.add(egui::Spinner::new().size(12.0).color(p.violet));
                    }
                },
            );
            widgets::divider(ui, p);
            row(ui, p, aurora_i18n::tr!("currency-new-balance"), |ui| {
                let total = balance.map(|b| b + i64::from(self.amount));
                ui.label(RichText::new(total.map_or_else(|| "—".to_owned(), money)).strong().color(p.ink));
            });
        });
        if let Some(error) = &self.error {
            ui.add_space(8.0);
            ui.horizontal_top(|ui| {
                widgets::glyph(ui, "alert-triangle-filled", p.danger, 16.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(aurora_i18n::tr!("currency-cannot-buy")).strong().color(p.danger));
                    if !error.message.is_empty() {
                        wrapped(ui, RichText::new(&error.message).size(12.0).color(p.ink));
                    }
                    // the helper's page, never another scheme
                    if (error.uri.starts_with("https://") || error.uri.starts_with("http://"))
                        && ui.link(aurora_i18n::tr!("currency-more-information")).clicked()
                    {
                        super::chat::open_web_link(ui.ctx(), &error.uri);
                    }
                });
            });
        }
        ui.add_space(10.0);
        let size = Vec2::new((ui.available_width() - ui.spacing().item_spacing.x) / 2.0, BUTTON_HEIGHT);
        ui.horizontal(|ui| {
            let buy = ui
                .add_enabled_ui(self.can_buy(), |ui| {
                    primary_button_sized(ui, p, aurora_i18n::tr!("common-buy"), size)
                })
                .inner;
            if buy.clicked() {
                out.extend(self.buy());
            }
            let cancel = ui
                .add_enabled_ui(!buying, |ui| flat_button_sized(ui, p, aurora_i18n::tr!("common-cancel"), size))
                .inner;
            if cancel.clicked() {
                *close = true;
                // onClickCancel: Update L$ balance
                out.push(NetCommand::RequestBalance);
            }
        });
    }

    fn no_payment(&mut self, ui: &mut egui::Ui, p: &Palette, close: &mut bool) {
        ui.horizontal_top(|ui| {
            widgets::glyph(ui, "credit-card-filled", p.violet_light, 24.0);
            ui.vertical(|ui| {
                wrapped(
                    ui,
                    RichText::new(aurora_i18n::tr!("currency-add-payment-method")).strong().color(p.ink),
                );
                ui.add_space(4.0);
                wrapped(
                    ui,
                    RichText::new(aurora_i18n::tr!("currency-add-payment-steps"))
                        .size(12.0)
                        .color(p.muted),
                );
            });
        });
        ui.add_space(10.0);
        let size = Vec2::new((ui.available_width() - ui.spacing().item_spacing.x) / 2.0, BUTTON_HEIGHT);
        ui.horizontal(|ui| {
            if primary_button_sized(ui, p, aurora_i18n::tr!("currency-continue"), size).clicked() {
                super::chat::open_web_link(ui.ctx(), ADD_PAYMENT_URL);
                *close = true;
            }
            if flat_button_sized(ui, p, aurora_i18n::tr!("currency-later"), size).clicked() {
                *close = true;
            }
        });
    }

    /// ConfirmPurchase / ConfirmPurchasePassword.
    fn confirm_dialog(&mut self, ctx: &egui::Context, p: &Palette, out: &mut Vec<NetCommand>) {
        let (Some(quote), Some(password)) = (self.quote.as_ref(), self.confirm.as_mut()) else {
            self.confirm = None;
            return;
        };
        let needs_password = quote.confirm == "password";
        let cost = quote.local_cost.clone().or_else(|| quote.usd_cents.map(usd)).unwrap_or_default();
        let mut answer = None;
        let modal = Modal::new(egui::Id::unique("buy_currency_confirm")).show(ctx, |ui| {
            ui.set_width(CONFIRM_WIDTH);
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
            ui.horizontal(|ui| {
                widgets::glyph(ui, "coin-filled", p.violet_light, 20.0);
                ui.label(RichText::new(aurora_i18n::tr!("currency-title")).size(17.0).strong().color(p.ink));
            });
            ui.label(RichText::new(aurora_i18n::tr!("currency-confirm-intro")).color(p.ink));
            wrapped(
                ui,
                RichText::new(aurora_i18n::tr!(
                    "currency-confirm-action",
                    lindens = aurora_i18n::integer(i64::from(quote.amount)),
                    cost = cost
                ))
                .strong()
                .color(p.ink),
            );
            wrapped(ui, RichText::new(aurora_i18n::tr!("currency-confirm-question")).color(p.ink));
            if needs_password {
                wrapped(ui, RichText::new(aurora_i18n::tr!("currency-confirm-password")).color(p.ink));
                let field = ui.add(egui::TextEdit::singleline(password).password(true).desired_width(f32::INFINITY));
                if !field.has_focus() && password.is_empty() {
                    field.request_focus();
                }
            }
            let size = Vec2::new((ui.available_width() - ui.spacing().item_spacing.x) / 2.0, BUTTON_HEIGHT);
            ui.horizontal(|ui| {
                let ok = ui
                    .add_enabled_ui(!needs_password || !password.is_empty(), |ui| {
                        primary_button_sized(ui, p, aurora_i18n::tr!("common-ok"), size)
                    })
                    .inner;
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
            Some(true) => {
                let password = Password(self.confirm.take().unwrap_or_default());
                out.extend(self.start_buy(password));
            }
            Some(false) => self.confirm = None,
            None => {}
        }
    }

    /// AURORA_DEMO_BUY_CURRENCY: the window as its mode shows it, without
    /// waiting for a click (see docs/TESTING.md).
    pub fn demo(&mut self, world: &mut World, helper: Option<&'static str>, mode: &str) {
        self.open(world, helper.filter(|_| mode != "grid"));
        match mode {
            "grid" => {}
            "nopayment" => self.stage = Stage::NoPayment,
            "empty" => {}
            _ => {
                self.set_amount(mode.parse().unwrap_or(1000));
                self.demo_confirm = matches!(mode, "confirm" | "password");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELPER: &str = "https://helper.invalid/";

    fn form() -> BuyCurrencyUi {
        BuyCurrencyUi {
            open: true,
            helper: Some(HELPER),
            stage: Stage::Form,
            ..Default::default()
        }
    }

    fn quote(amount: i32, confirm: &str) -> CurrencyQuote {
        CurrencyQuote {
            amount,
            usd_cents: Some(425),
            local_cost: Some("US$ 4.25".into()),
            confirm: confirm.into(),
        }
    }

    fn type_amount(ui: &mut BuyCurrencyUi, text: &str, now: Instant) {
        ui.text = text.to_owned();
        ui.typed(now);
    }

    /// Type, wait for the pause, receive the estimate.
    fn estimated(amount: i32, confirm: &str) -> BuyCurrencyUi {
        let mut ui = form();
        let t0 = Instant::now();
        type_amount(&mut ui, &amount.to_string(), t0);
        assert!(ui.poll(t0 + ESTIMATE_PAUSE).is_some());
        ui.quote_reply(amount, Ok(quote(amount, confirm)));
        ui
    }

    #[test]
    fn estimate_waits_for_the_typing_pause() {
        let mut ui = form();
        let t0 = Instant::now();
        type_amount(&mut ui, "1000", t0);
        assert_eq!(ui.estimate_text(), None);
        assert!(ui.poll(t0 + Duration::from_millis(1900)).is_none());
        let cmd = ui.poll(t0 + ESTIMATE_PAUSE);
        assert!(matches!(cmd, Some(NetCommand::CurrencyQuote { ref helper, amount: 1000 }) if helper == HELPER));
        // one transaction at a time
        assert!(ui.poll(t0 + ESTIMATE_PAUSE * 2).is_none());
        assert!(!ui.can_buy());
        ui.quote_reply(1000, Ok(quote(1000, "click")));
        assert_eq!(ui.estimate_text().as_deref(), Some("US$ 4.25"));
        assert!(ui.can_buy());
    }

    #[test]
    fn estimate_for_an_amount_typed_over_is_dropped() {
        let mut ui = form();
        let t0 = Instant::now();
        type_amount(&mut ui, "1000", t0);
        assert!(ui.poll(t0 + ESTIMATE_PAUSE).is_some());
        let t1 = t0 + ESTIMATE_PAUSE + Duration::from_millis(100);
        type_amount(&mut ui, "2500", t1);
        ui.quote_reply(1000, Ok(quote(1000, "click")));
        assert_eq!((ui.amount, ui.text.as_str()), (2500, "2500"));
        assert!(!ui.can_buy());
        assert!(matches!(
            ui.poll(t1 + ESTIMATE_PAUSE),
            Some(NetCommand::CurrencyQuote { amount: 2500, .. })
        ));
    }

    #[test]
    fn zero_needs_no_estimate_and_cannot_be_bought() {
        let mut ui = estimated(1000, "click");
        let t = Instant::now();
        type_amount(&mut ui, "", t);
        assert!(ui.poll(t + ESTIMATE_PAUSE).is_none());
        assert!(ui.estimate_text().is_some());
        assert!(!ui.can_buy());
        assert!(ui.buy().is_empty());
    }

    #[test]
    fn field_keeps_digits_that_fit() {
        let mut ui = form();
        let t = Instant::now();
        type_amount(&mut ui, "12a3", t);
        assert_eq!((ui.amount, ui.text.as_str()), (123, "123"));
        type_amount(&mut ui, "99999999999", t);
        assert_eq!((ui.amount, ui.text.as_str()), (123, "123"));
    }

    #[test]
    fn helper_may_adjust_the_amount() {
        let mut ui = form();
        let t0 = Instant::now();
        type_amount(&mut ui, "10", t0);
        assert!(ui.poll(t0 + ESTIMATE_PAUSE).is_some());
        ui.quote_reply(10, Ok(quote(250, "")));
        assert_eq!((ui.amount, ui.text.as_str()), (250, "250"));
        assert!(ui.can_buy());
    }

    #[test]
    fn click_confirmation_precedes_the_purchase() {
        let mut ui = estimated(1000, "click");
        let cmds = ui.buy();
        assert!(matches!(cmds.as_slice(), [NetCommand::RequestBalance]));
        assert!(ui.confirm.is_some() && !ui.buying());
        let cmd = ui.start_buy(Password::default());
        assert!(matches!(cmd, Some(NetCommand::BuyCurrency { ref quote, .. }) if quote.amount == 1000));
        assert!(ui.buying() && ui.confirm.is_none() && !ui.can_buy());
    }

    #[test]
    fn no_confirmation_buys_at_once() {
        let mut ui = estimated(1000, "");
        let cmds = ui.buy();
        assert!(matches!(
            cmds.as_slice(),
            [NetCommand::BuyCurrency { .. }, NetCommand::RequestBalance]
        ));
        assert!(ui.buying());
    }

    #[test]
    fn purchase_outcome() {
        let mut ui = estimated(1000, "");
        ui.buy();
        assert!(!ui.bought(Err(CurrencyError {
            message: "refused".into(),
            uri: String::new(),
        })));
        assert!(ui.open && ui.error.is_some() && !ui.can_buy());
        // typing the same amount again asks for a new estimate
        let t = Instant::now();
        type_amount(&mut ui, "1000", t);
        assert!(ui.error.is_none());
        assert!(ui.poll(t + ESTIMATE_PAUSE).is_some());
        ui.quote_reply(1000, Ok(quote(1000, "")));
        ui.buy();
        assert!(ui.bought(Ok(())));
        assert!(!ui.open);
        // a late answer is not a second purchase
        assert!(!ui.bought(Ok(())));
    }

    #[test]
    fn refused_estimate_is_shown() {
        let mut ui = form();
        let t0 = Instant::now();
        type_amount(&mut ui, "5", t0);
        assert!(ui.poll(t0 + ESTIMATE_PAUSE).is_some());
        ui.quote_reply(
            5,
            Err(CurrencyError {
                message: "too small".into(),
                uri: String::new(),
            }),
        );
        assert_eq!(ui.error.as_ref().map(|e| e.message.as_str()), Some("too small"));
        assert!(!ui.can_buy());
        assert!(ui.poll(t0 + ESTIMATE_PAUSE * 3).is_none());
    }
}

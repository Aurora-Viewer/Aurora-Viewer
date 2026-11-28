//! The Pay window: L$ to a resident or a group, and the Aurora look of the
//! payment to an object. Port of LLFloaterPay (indra/newview/llfloaterpay.cpp,
//! floater_pay.xml, floater_pay_object.xml, originally LGPL 2.1):
//! - the quick amounts L$ 1, 5, 10, 20 (those of llSetPayPrice for an
//!   object), the free amount of nine digits and the payment message of a
//!   resident (127 bytes);
//! - the last amount paid filled in again (FIRE-16812);
//! - FSConfirmPayments: a confirmation above FSPaymentConfirmationThreshold,
//!   when the balance covers it and the payee is not ourselves (FIRE-24208);
//! - give_money: the Buy L$ window when the balance falls short.
//!
//! The Aurora look (the human's choice of the mockup, variant A) keeps the
//! window open until the server answers and says what was sent; Firestorm's
//! (`Settings::classic_pay_window`) closes at once, as floater_pay.xml does.
//!
//! With `Settings::pay_contactless` (on by default) the Aurora look pays with
//! its contactless terminal instead of its Pay button: `terminal.rs`.

use super::widgets::{self, ActionKind, DialogButton, Floater, Modal};
use crate::interaction::{Action, Interactions};
use crate::theme::Palette;
use crate::world::World;
use aurora_net::NetCommand;
use egui::{Align, Layout, RichText, Sense, Vec2};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use uuid::Uuid;

mod terminal;

/// PAY_BUTTON_DEFAULT_0..3 (lllslconstants.h).
pub const QUICK_AMOUNTS: [i32; 4] = [1, 5, 10, 20];
/// max_length_bytes of floater_pay.xml's payment_message.
const MESSAGE_BYTES: usize = 127;
/// max_length_bytes of its amount field.
const AMOUNT_DIGITS: usize = 9;
/// TRANS_GIFT (lltransactiontypes.h).
const TRANS_GIFT: i32 = 5001;
/// How long the window waits for the server before it says the payment left.
const ANSWER_WAIT: Duration = Duration::from_secs(15);

const WIDTH: f32 = 380.0;
const BUTTON_H: f32 = 30.0;

fn money(amount: i32) -> String {
    format!("{} {}", aurora_i18n::currency(), aurora_i18n::integer(i64::from(amount.max(0))))
}

/// The width of the window whose body is `body` wide: the floater's margins
/// and outline around it. Given the body's width alone, the window grew to
/// hold its body while its title bar kept the width asked for, its buttons
/// 22 px short of the right edge.
fn window_width(ctx: &egui::Context, body: f32) -> f32 {
    body + widgets::floater_chrome(ctx).x
}

/// How the windows pay, from the Preferences.
#[derive(Clone, Copy, Debug)]
pub struct Rules {
    /// The Legacy look (`Settings::classic_pay_window`).
    pub classic: bool,
    /// The Aurora look pays with its terminal (`Settings::pay_contactless`):
    /// the click held on the card, instead of the Pay button.
    pub contactless: bool,
    /// FSConfirmPayments and FSPaymentConfirmationThreshold.
    pub confirm: bool,
    pub threshold: i32,
    /// Preferences › Accessibility › Reduce motion.
    pub reduce_motion: bool,
}

/// Where the window is in a payment.
#[derive(Clone, Debug, PartialEq)]
enum Stage {
    Edit,
    /// PayConfirmation is asked for this amount.
    Confirm(i32),
    /// MoneyTransferRequest sent; the MoneyBalanceReply is awaited.
    Sending {
        amount: i32,
        since: Instant,
    },
    /// The server took it; `balance` is the one it reported.
    Sent {
        amount: i32,
        balance: Option<i32>,
    },
    /// The server refused it, with its reason.
    Failed(String),
}

/// How long an object's window waits for the new balance after its payment
/// left, its terminal saying "Sending", before it closes without more; and
/// how long it stays on "Payment accepted" before closing by itself. In
/// seconds of egui's time.
const OBJECT_ANSWER_WAIT: f64 = 5.0;
const OBJECT_ACCEPTED: f64 = 1.5;

/// The payment an object's window just made with its terminal.
#[derive(Default)]
struct Receipt {
    /// When the window began to show it.
    since: Option<f64>,
    /// The balance changed since: the server took the payment.
    answered: bool,
    /// When the terminal began to say that it was accepted.
    accepted: Option<f64>,
}

/// A resident or a group being paid.
pub struct Payee {
    pub id: Uuid,
    pub group: bool,
    amount: String,
    message: String,
    stage: Stage,
    focus: bool,
}

/// What the window asks of the application.
#[derive(Debug)]
pub enum PayAction {
    Send(Box<NetCommand>),
    /// LLBuyCurrencyHTML::openCurrencyFloater for the L$ missing.
    BuyCurrency(i32),
    Profile(Uuid),
}

#[derive(Default)]
pub struct PayUi {
    pub payee: Option<Payee>,
    raise: bool,
    /// sLastAmount: the last amount paid, shared by every Pay window.
    last_amount: i32,
    /// A payment to an object waiting for its confirmation in Firestorm's
    /// look (PayConfirmation): the amount and the object's name.
    pub object_confirm: Option<(i32, String)>,
    /// The payee's profile picture (a resident) or insignia (a group), asked
    /// of the app while the Aurora look draws it.
    pub wanted_pics: HashSet<Uuid>,
    pub wanted_images: HashSet<Uuid>,
    /// The contactless terminals of the Aurora look, one per window: the
    /// resident's or the group's, and the object's.
    terminal: terminal::Terminal,
    object_terminal: terminal::Terminal,
    /// The object's window `object_terminal` belongs to: its object and
    /// when it opened.
    object_window: Option<(Uuid, Instant)>,
    /// The payment the object's window just made, while it stays to say
    /// what became of it.
    object_receipt: Option<Receipt>,
    /// AURORA_DEMO_PAY=hold: a hold drawn at this fraction, for captures.
    demo_hold: Option<f32>,
    /// AURORA_DEMO_PAY=button: the Pay button for this run, whatever
    /// `Settings::pay_contactless` says (a test never changes the settings
    /// every viewer shares).
    pub demo_button: bool,
    /// AURORA_DEMO_PAY=classic: the Legacy look for this run, whatever
    /// `Settings::classic_pay_window` says.
    pub demo_classic: bool,
}

/// Textures by id: the profile pictures by avatar, the UI images by asset
/// (a group's insignia).
type Pictures<'a> = (&'a HashMap<Uuid, egui::TextureHandle>, &'a HashMap<Uuid, egui::TextureHandle>);

/// The amount typed, if it is a positive number (LLFloaterPay::onKeystroke).
fn parse_amount(text: &str) -> Option<i32> {
    text.trim().parse::<i32>().ok().filter(|v| *v > 0)
}

/// RLVa `@sendim` (LLFloaterPay::payDirectly): the payment message reaches
/// the payee like an IM, so it is greyed, and emptied, while we may not send
/// them one. The payment itself stays possible.
fn message_allowed(world: &World, payee: &mut Payee) -> bool {
    let allowed = world.rlva_allows_im("sendim", payee.id);
    if !allowed {
        payee.message.clear();
    }
    allowed
}

/// FSConfirmPayments: above the threshold, with the balance to cover it and
/// someone else to pay (LLFloaterPay::onGive).
pub fn needs_confirmation(confirm: bool, threshold: i32, amount: i32, balance: Option<i32>, to_self: bool) -> bool {
    confirm && amount > threshold && balance.is_some_and(|b| b >= amount) && !to_self
}

impl PayUi {
    /// Open a resident or group payment only when the current RLVa rules allow it.
    pub fn open_if_allowed(&mut self, id: Uuid, group: bool, world: &World) -> bool {
        if !world.rlva_can_pay() {
            return false;
        }
        self.open(id, group);
        true
    }

    /// LLFloaterPayUtil::payDirectly: a resident, or a group with `group`.
    pub fn open(&mut self, id: Uuid, group: bool) {
        self.raise = true;
        self.terminal = terminal::Terminal::default();
        self.payee = Some(Payee {
            id,
            group,
            amount: if self.last_amount > 0 {
                self.last_amount.to_string()
            } else {
                String::new()
            },
            message: String::new(),
            stage: Stage::Edit,
            focus: true,
        });
    }

    /// Close a resident/group payment not sent yet when RLVa's live
    /// restrictions forbid paying. A payment already sent (Sending, Sent,
    /// Failed) stays: its L$ went or the server answered, and closing it
    /// with the "restricted" notice would say the opposite. The Legacy
    /// Viewer only refuses to open LLFloaterPay and leaves an open one
    /// alone; closing an unsent one is stricter, never looser.
    pub fn close_resident(&mut self) -> bool {
        if !self
            .payee
            .as_ref()
            .is_some_and(|p| matches!(p.stage, Stage::Edit | Stage::Confirm(_)))
        {
            return false;
        }
        self.payee = None;
        self.raise = false;
        true
    }

    /// LLFloaterPay::give, or the Buy L$ window when the balance is short
    /// (give_money). The Aurora look waits for the server's answer.
    fn give(&mut self, amount: i32, world: &World, actions: &mut Vec<PayAction>) {
        let Some(payee) = self.payee.as_mut() else { return };
        if !world.rlva_can_pay() {
            payee.stage = Stage::Failed(aurora_i18n::tr!("rlva-blocked").into());
            return;
        }
        let balance = world.balance;
        match balance {
            Some(b) if b >= amount => {
                self.last_amount = amount;
                actions.push(PayAction::Send(Box::new(NetCommand::PayResident {
                    to: payee.id,
                    group: payee.group,
                    amount,
                    message: payee.message.trim().to_owned(),
                })));
                payee.stage = Stage::Sending {
                    amount,
                    since: Instant::now(),
                };
            }
            _ => {
                actions.push(PayAction::BuyCurrency(amount - balance.unwrap_or(0).max(0)));
                payee.stage = Stage::Edit;
            }
        }
    }

    /// LLFloaterPay::onGive with the amount chosen.
    fn pay(&mut self, amount: i32, world: &World, confirm: bool, threshold: i32, actions: &mut Vec<PayAction>) {
        let Some(payee) = self.payee.as_mut() else { return };
        if !world.rlva_can_pay() {
            payee.stage = Stage::Failed(aurora_i18n::tr!("rlva-blocked").into());
            return;
        }
        let to_self = !payee.group && payee.id == world.agent_id;
        if needs_confirmation(confirm, threshold, amount, world.balance, to_self) {
            payee.stage = Stage::Confirm(amount);
        } else {
            self.give(amount, world, actions);
        }
    }

    /// The outcome of a transaction (MoneyBalanceReply's TransactionInfo).
    pub fn on_transaction(&mut self, success: bool, kind: i32, dest: Uuid, description: &str) {
        if let Some(payee) = self.payee.as_mut()
            && kind == TRANS_GIFT
            && dest == payee.id
            && let Stage::Sending { amount, .. } = payee.stage
        {
            payee.stage = if success {
                Stage::Sent { amount, balance: None }
            } else {
                Stage::Failed(description.to_owned())
            };
        }
    }

    /// The new balance: a payment still waiting took it (grids that send no
    /// TransactionInfo), and a finished one shows it.
    pub fn on_balance(&mut self, balance: i32) {
        if let Some(shown) = self.object_receipt.as_mut() {
            shown.answered = true;
        }
        if let Some(payee) = self.payee.as_mut() {
            match payee.stage {
                Stage::Sending { amount, .. } => {
                    payee.stage = Stage::Sent {
                        amount,
                        balance: Some(balance),
                    }
                }
                Stage::Sent { amount, balance: None } => {
                    payee.stage = Stage::Sent {
                        amount,
                        balance: Some(balance),
                    }
                }
                _ => {}
            }
        }
    }

    /// The window of the look chosen; `rules.classic`: floater_pay.xml.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &World, pics: Pictures, rules: Rules) -> Vec<PayAction> {
        let mut actions = Vec::new();
        if self.payee.is_some() {
            if std::mem::take(&mut self.raise) {
                widgets::expand_floater(ctx, if rules.classic { "pay_classic" } else { "pay" });
            }
            if rules.classic {
                self.show_classic(ctx, p, world, rules.confirm, rules.threshold, &mut actions);
            } else {
                self.show_aurora(ctx, p, world, pics, rules, &mut actions);
            }
        }
        actions
    }

    fn payee_name(world: &World, payee: &Payee) -> (String, String) {
        if payee.group {
            let name = world.groups.group(&payee.id).map(|g| g.name.clone()).unwrap_or_else(|| "…".into());
            (name, aurora_i18n::tr!("ui-pay-group-receives").into())
        } else {
            (world.social.display_name_of(&payee.id), world.social.name_of(&payee.id))
        }
    }

    /// The payee's picture: a resident's profile picture, a group's
    /// insignia; asked of the app until it is there (initials meanwhile).
    fn payee_picture<'a>(&mut self, world: &World, (avatars, images): Pictures<'a>) -> Option<&'a egui::TextureHandle> {
        let payee = self.payee.as_ref()?;
        if payee.group {
            let insignia = crate::ui::chat::group_insignia(world, &payee.id)?;
            self.wanted_images.insert(insignia);
            images.get(&insignia)
        } else {
            self.wanted_pics.insert(payee.id);
            avatars.get(&payee.id)
        }
    }

    fn show_aurora(&mut self, ctx: &egui::Context, p: &Palette, world: &World, pics: Pictures, rules: Rules, actions: &mut Vec<PayAction>) {
        let Rules {
            contactless,
            confirm,
            threshold,
            ..
        } = rules;
        let frozen = self.demo_hold;
        let picture = self.payee_picture(world, pics);
        let Some(payee) = self.payee.as_mut() else { return };
        if let Stage::Sending { amount, since } = payee.stage
            && since.elapsed() > ANSWER_WAIT
        {
            payee.stage = Stage::Sent { amount, balance: None };
        }
        let center = ctx.content_rect().center();
        let title = if payee.group {
            aurora_i18n::tr!("ui-pay-group")
        } else {
            aurora_i18n::tr!("ui-pay-resident")
        };
        let mut open = true;
        let mut pay = None;
        let mut give = None;
        let mut close = false;
        let mut again = false;
        let (name, sub) = Self::payee_name(world, payee);
        let outer = window_width(ctx, WIDTH);
        Floater::new(
            "pay",
            title,
            egui::pos2(center.x - outer / 2.0, center.y - 200.0),
            Vec2::new(outer, 360.0),
        )
        .auto_height()
        .show(ctx, p, &mut open, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 10.0;
            // the payee's card
            card(ui, p, |ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                if payee.group {
                    widgets::avatar_picture(ui, p, rect, picture, &name, false);
                } else {
                    widgets::avatar_disc(ui, p, rect, picture, &name, false);
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(RichText::new(&name).size(14.0).strong().color(p.ink));
                    ui.add(egui::Label::new(RichText::new(&sub).size(11.5).color(p.muted)).truncate());
                });
                if !payee.group {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::icon_button(ui, p, "user-filled", aurora_i18n::tr!("common-view-profile"), true) {
                            actions.push(PayAction::Profile(payee.id));
                        }
                    });
                }
            });
            // (with the terminal the window keeps its fields, and the
            // terminal says where the payment is)
            match payee.stage.clone() {
                Stage::Sent { amount, balance } if !contactless => {
                    sent(ui, p, &name, amount, balance, &payee.message);
                    match widgets::dialog_buttons(
                        ui,
                        p,
                        &[
                            button("", aurora_i18n::tr!("ui-pay-again"), ActionKind::Flat, true),
                            button("", aurora_i18n::tr!("common-close"), ActionKind::Primary, true),
                        ],
                        BUTTON_H,
                    ) {
                        Some(0) => again = true,
                        Some(_) => close = true,
                        None => {}
                    }
                    return;
                }
                Stage::Sending { .. } if !contactless => {
                    ui.vertical_centered(|ui| {
                        ui.add_space(14.0);
                        ui.add(egui::Spinner::new().color(p.violet_light));
                        ui.label(RichText::new(aurora_i18n::tr!("ui-pay-sending")).color(p.muted));
                        ui.add_space(8.0);
                    });
                    return;
                }
                _ => {}
            }
            // greyed while the terminal's payment is on its way, and once made
            let locked = contactless && matches!(payee.stage, Stage::Sending { .. } | Stage::Sent { .. });
            let mut amount = None;
            ui.add_enabled_ui(!locked, |ui| {
                amount_field(ui, p, &mut payee.amount, &mut payee.focus);
                amount = parse_amount(&payee.amount);
                // the quick amounts fill the field
                ui.columns(4, |cols| {
                    for (col, value) in cols.iter_mut().zip(QUICK_AMOUNTS) {
                        let on = amount == Some(value);
                        let kind = if on { ActionKind::Primary } else { ActionKind::Flat };
                        if widgets::action_button(col, p, "", &money(value), kind, Vec2::new(col.available_width(), 28.0)).clicked() {
                            payee.amount = value.to_string();
                        }
                    }
                });
                if !payee.group {
                    let enabled = message_allowed(world, payee);
                    ui.add_enabled(
                        enabled,
                        egui::TextEdit::singleline(&mut payee.message)
                            .hint_text(aurora_i18n::tr!("ui-pay-message-hint"))
                            .char_limit(MESSAGE_BYTES)
                            .desired_width(f32::INFINITY),
                    );
                    truncate_bytes(&mut payee.message, MESSAGE_BYTES);
                }
            });
            let after = amount.zip(world.balance).map(|(a, b)| b - a).filter(|_| !locked);
            ui.horizontal(|ui| {
                ui.label(RichText::new(aurora_i18n::tr!("ui-pay-balance")).size(12.0).color(p.muted));
                ui.label(
                    RichText::new(world.balance.map_or_else(|| "…".to_owned(), money))
                        .size(12.0)
                        .strong()
                        .color(p.ink),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(after) = after {
                        let text = if after < 0 { format!("−{}", money(-after)) } else { money(after) };
                        let color = if after < 0 { p.danger } else { p.ink };
                        ui.label(RichText::new(text).size(12.0).strong().color(color));
                        ui.label(RichText::new(aurora_i18n::tr!("ui-pay-after")).size(12.0).color(p.muted));
                    }
                });
            });
            let short = after.is_some_and(|a| a < 0);
            if short && let Some(missing) = after.map(|a| -a) {
                band(ui, p.danger, |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(
                            RichText::new(aurora_i18n::tr!("ui-pay-missing", amount = money(missing)))
                                .strong()
                                .color(p.danger),
                        );
                        ui.label(RichText::new(aurora_i18n::tr!("ui-pay-missing-help")).size(11.5).color(p.muted));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if widgets::action_button(
                            ui,
                            p,
                            "shopping-cart-filled",
                            aurora_i18n::tr!("ui-pay-buy-currency"),
                            ActionKind::Primary,
                            Vec2::new(0.0, 26.0),
                        )
                        .clicked()
                        {
                            actions.push(PayAction::BuyCurrency(missing));
                        }
                    });
                });
            }
            if let Stage::Failed(reason) = &payee.stage {
                band(ui, p.danger, |ui| {
                    let text = if reason.is_empty() {
                        aurora_i18n::tr!("ui-pay-failed").to_owned()
                    } else {
                        reason.clone()
                    };
                    ui.add(egui::Label::new(RichText::new(text).color(p.danger)).wrap());
                });
            }
            if let Stage::Confirm(value) = payee.stage {
                band(ui, p.amber, |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(
                            RichText::new(aurora_i18n::tr!("ui-pay-confirm", amount = money(value), name = name.clone()))
                                .strong()
                                .color(p.amber),
                        );
                        ui.label(
                            RichText::new(aurora_i18n::tr!("ui-pay-confirm-help", amount = money(threshold)))
                                .size(11.5)
                                .color(p.muted),
                        );
                    });
                });
                match widgets::dialog_buttons(
                    ui,
                    p,
                    &[
                        button("", aurora_i18n::tr!("common-no"), ActionKind::Flat, true),
                        button("coin-filled", aurora_i18n::tr!("common-pay"), ActionKind::Primary, true),
                    ],
                    BUTTON_H,
                ) {
                    Some(0) => payee.stage = Stage::Edit,
                    Some(_) => give = Some(value),
                    None => {}
                }
                return;
            }
            if contactless {
                // the terminal in the Pay button's place: the same payment,
                // under the same conditions, once the click was held
                let holder = world.social.display_name_of(&world.agent_id);
                let missing = after
                    .filter(|a| *a < 0)
                    .map(|a| aurora_i18n::tr!("ui-pay-missing", amount = money(-a)).to_owned());
                let received = match payee.stage {
                    Stage::Sent { amount, .. } => {
                        aurora_i18n::tr!("ui-pay-terminal-accepted-help", name = name.clone(), amount = money(amount)).to_owned()
                    }
                    _ => String::new(),
                };
                let status = match (&payee.stage, &missing) {
                    (Stage::Sending { .. }, _) => terminal::Status::Sending,
                    (Stage::Sent { .. }, _) => terminal::Status::Accepted(&received),
                    _ if amount.is_none() => terminal::Status::Blocked {
                        label: aurora_i18n::tr!("ui-pay-terminal-amount"),
                        text: aurora_i18n::tr!("ui-pay-terminal-amount-help"),
                        alert: false,
                    },
                    (_, Some(missing)) => terminal::Status::Blocked {
                        label: aurora_i18n::tr!("ui-pay-terminal-short"),
                        text: missing,
                        alert: true,
                    },
                    _ => terminal::Status::Ready,
                };
                let view = terminal::View {
                    amount: match payee.stage {
                        Stage::Sending { amount, .. } | Stage::Sent { amount, .. } => Some(amount),
                        _ => amount,
                    },
                    holder: &holder,
                    status,
                    reduce_motion: rules.reduce_motion,
                    frozen,
                };
                if self.terminal.show(ui, p, &view) {
                    pay = amount;
                }
                if matches!(payee.stage, Stage::Sent { .. }) {
                    match widgets::dialog_buttons(
                        ui,
                        p,
                        &[
                            button("", aurora_i18n::tr!("ui-pay-again"), ActionKind::Flat, true),
                            button("", aurora_i18n::tr!("common-close"), ActionKind::Primary, true),
                        ],
                        BUTTON_H,
                    ) {
                        Some(0) => again = true,
                        Some(_) => close = true,
                        None => {}
                    }
                } else {
                    // the Pay button holds the card in our place: the same
                    // hold, with nothing to keep pressed; Cancel takes the
                    // card back while it is on its way
                    let sending = matches!(payee.stage, Stage::Sending { .. });
                    let playing = self.terminal.playing();
                    let label = amount.map_or_else(
                        || aurora_i18n::tr!("common-pay").to_owned(),
                        |a| aurora_i18n::tr!("ui-pay-amount", amount = money(a)).to_owned(),
                    );
                    // a payment on its way is no longer cancelled by closing
                    let leave = if sending {
                        aurora_i18n::tr!("common-close")
                    } else {
                        aurora_i18n::tr!("common-cancel")
                    };
                    let can_pay = amount.is_some() && !short && !sending && !playing;
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    match widgets::dialog_buttons(
                        ui,
                        p,
                        &[
                            button("", leave, ActionKind::Flat, true),
                            button("coin-filled", &label, ActionKind::Primary, can_pay),
                        ],
                        BUTTON_H,
                    ) {
                        Some(0) if playing => self.terminal.stop(),
                        Some(0) => close = true,
                        Some(_) => self.terminal.play(),
                        None if enter && can_pay => self.terminal.play(),
                        None => {}
                    }
                }
                return;
            }
            let label = amount.map_or_else(
                || aurora_i18n::tr!("common-pay").to_owned(),
                |a| aurora_i18n::tr!("ui-pay-amount", amount = money(a)).to_owned(),
            );
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            match widgets::dialog_buttons(
                ui,
                p,
                &[
                    button("", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, true),
                    button("coin-filled", &label, ActionKind::Primary, amount.is_some() && !short),
                ],
                BUTTON_H,
            ) {
                Some(0) => close = true,
                Some(_) => pay = amount,
                None if enter && !short => pay = amount,
                None => {}
            }
        });
        if again && let Some(payee) = self.payee.as_mut() {
            payee.stage = Stage::Edit;
            payee.focus = true;
        }
        if let Some(amount) = pay {
            self.pay(amount, world, confirm, threshold, actions);
        }
        if let Some(amount) = give {
            self.give(amount, world, actions);
        }
        if !open || close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) && ctx.memory(|m| m.focused().is_none()) {
            self.payee = None;
        }
    }

    /// floater_pay.xml: the payee, the 2 × 2 quick amounts that pay at once,
    /// the free amount, the message, Pay and Cancel; closed on giving.
    fn show_classic(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &World,
        confirm: bool,
        threshold: i32,
        actions: &mut Vec<PayAction>,
    ) {
        let Some(payee) = self.payee.as_mut() else { return };
        let center = ctx.content_rect().center();
        let title = if payee.group {
            aurora_i18n::tr!("ui-pay-classic-group")
        } else {
            aurora_i18n::tr!("ui-pay-classic-resident")
        };
        let (name, _) = Self::payee_name(world, payee);
        let mut open = true;
        let mut pay = None;
        let mut cancel = false;
        let mut give = None;
        let outer = window_width(ctx, 300.0);
        Floater::new(
            "pay_classic",
            title,
            egui::pos2(center.x - outer / 2.0, center.y - 130.0),
            Vec2::new(outer, 260.0),
        )
        .auto_height()
        .show(ctx, p, &mut open, |ui| {
            ui.set_width(300.0);
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.label(RichText::new(aurora_i18n::tr!("ui-pay-classic-pay")).color(p.muted));
                if ui.link(RichText::new(&name).color(p.violet_light)).clicked() && !payee.group {
                    actions.push(PayAction::Profile(payee.id));
                }
            });
            let half = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
            egui::Grid::new("pay_classic_quick")
                .num_columns(2)
                .spacing(Vec2::new(8.0, 6.0))
                .show(ui, |ui| {
                    for (i, value) in QUICK_AMOUNTS.into_iter().enumerate() {
                        if widgets::flat_button_sized(ui, p, &money(value), Vec2::new(half, 24.0)).clicked() {
                            pay = Some(value);
                        }
                        if i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
            ui.horizontal(|ui| {
                ui.label(aurora_i18n::tr!("ui-object-actions-or-choose-an-amount"));
                widgets::restricted_text_field(ui, true, &mut payee.amount, widgets::TextPolicy::Digits, |b, _| {
                    egui::TextEdit::singleline(b).char_limit(AMOUNT_DIGITS).desired_width(90.0)
                });
            });
            if !payee.group {
                let enabled = message_allowed(world, payee);
                ui.horizontal(|ui| {
                    ui.label(aurora_i18n::tr!("ui-pay-classic-message"));
                    ui.add_enabled(
                        enabled,
                        egui::TextEdit::singleline(&mut payee.message)
                            .char_limit(MESSAGE_BYTES)
                            .desired_width(f32::INFINITY),
                    );
                    truncate_bytes(&mut payee.message, MESSAGE_BYTES);
                });
            }
            let amount = parse_amount(&payee.amount);
            match widgets::dialog_buttons(
                ui,
                p,
                &[
                    button("", aurora_i18n::tr!("common-pay"), ActionKind::Flat, amount.is_some()),
                    button("", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, true),
                ],
                24.0,
            ) {
                Some(0) => pay = amount,
                Some(_) => cancel = true,
                None => {}
            }
        });
        // PayConfirmation, a notification in Firestorm
        if let Some(Payee {
            stage: Stage::Confirm(value),
            ..
        }) = self.payee
        {
            let answer = Modal::new(egui::Id::unique("pay_classic_confirm"))
                .show(ctx, |ui| {
                    ui.label(RichText::new(aurora_i18n::tr!("ui-pay-confirm", amount = money(value), name = name.clone())).color(p.ink));
                    widgets::dialog_buttons(
                        ui,
                        p,
                        &[
                            button("", aurora_i18n::tr!("common-pay"), ActionKind::Primary, true),
                            button("", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, true),
                        ],
                        24.0,
                    )
                })
                .inner;
            match answer {
                Some(0) => give = Some(value),
                Some(_) => {
                    if let Some(payee) = self.payee.as_mut() {
                        payee.stage = Stage::Edit;
                    }
                }
                None => {}
            }
        }
        if let Some(amount) = pay {
            self.pay(amount, world, confirm, threshold, actions);
        }
        if let Some(amount) = give {
            self.give(amount, world, actions);
        }
        // Firestorm closes the window as it gives
        if !open
            || cancel
            || self
                .payee
                .as_ref()
                .is_some_and(|payee| matches!(payee.stage, Stage::Sending { .. }) || pay.is_some_and(|_| payee.stage == Stage::Edit))
        {
            self.payee = None;
        }
    }

    /// The Aurora look of a payment to an object (floater_pay_object.xml):
    /// its buttons of llSetPayPrice, the free amount unless the script hides
    /// it, the confirmation above the threshold. Returns the amount to pay,
    /// which `Interactions::confirm` checks again.
    pub fn show_object(&mut self, ctx: &egui::Context, p: &Palette, state: &mut Interactions, world: &World, rules: Rules) -> Option<i32> {
        let Rules {
            contactless,
            confirm,
            threshold,
            ..
        } = rules;
        // the window of a payment just made stays a moment: its terminal says
        // the payment is on its way, then accepted once the balance changed,
        // and the window closes by itself (`Interactions::paid`). Without
        // the server's answer it closes without saying so.
        let receipt = !state.dialog.as_ref().is_some_and(|d| d.target.action == Action::Pay);
        if receipt {
            // (a payment the terminal did not make itself, like the demo's
            // scripted one, leaves nothing to show: the window is closed)
            let Some(shown) = self.object_receipt.as_mut().filter(|_| contactless && state.paid.is_some()) else {
                state.paid = None;
                self.object_receipt = None;
                return None;
            };
            let now = ctx.input(|i| i.time);
            let since = *shown.since.get_or_insert(now);
            if shown.answered && shown.accepted.is_none() {
                shown.accepted = Some(now);
            }
            let over = match shown.accepted {
                Some(at) => now - at >= OBJECT_ACCEPTED,
                None => now - since >= OBJECT_ANSWER_WAIT,
            };
            if over {
                state.paid = None;
                self.object_receipt = None;
                self.object_window = None;
                return None;
            }
            ctx.request_repaint_after_secs(0.1);
        } else {
            self.object_receipt = None;
        }
        let accepted = self.object_receipt.as_ref().is_some_and(|shown| shown.accepted.is_some());
        let d = if receipt { state.paid.as_mut() } else { state.dialog.as_mut() }?;
        // the terminal of this window only: a hold never passes from the
        // window of an object to the next one
        let window = (d.target.object, d.opened);
        if self.object_window != Some(window) {
            self.object_window = Some(window);
            self.object_terminal = terminal::Terminal::default();
        }
        let center = ctx.content_rect().center();
        let mut open = true;
        let mut close = false;
        let mut result = None;
        let confirm_id = egui::Id::unique("pay_object_confirm");
        let mut asked: Option<i32> = ctx.data(|data| data.get_temp(confirm_id));
        let available = world.objects.index_of_uuid(&d.target.object).is_some();
        let owner = d.props.as_ref().map(|props| {
            world
                .social
                .avatar_names
                .complete(&props.owner_id)
                .or_else(|| world.avatar_name(&props.owner_id))
                .unwrap_or_else(|| aurora_i18n::tr!("ui-object-actions-loading-name").into())
        });
        let outer = window_width(ctx, WIDTH);
        Floater::new(
            "object_payment",
            aurora_i18n::tr!("common-pay-object"),
            egui::pos2(center.x - outer / 2.0, center.y - 180.0),
            Vec2::new(outer, 300.0),
        )
        .auto_height()
        .show(ctx, p, &mut open, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 10.0;
            card(ui, p, |ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
                if let Some(t) = super::icons::global("box") {
                    ui.painter().image(
                        t.id(),
                        egui::Rect::from_center_size(rect.center(), Vec2::splat(20.0)),
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        p.violet_pale,
                    );
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let name = d.props.as_ref().map_or_else(
                        || aurora_i18n::tr!("ui-object-actions-loading-object-information").to_owned(),
                        |props| props.name.clone(),
                    );
                    ui.add(egui::Label::new(RichText::new(name).size(14.0).strong().color(p.ink)).truncate());
                    if let Some(owner) = &owner {
                        ui.label(
                            RichText::new(aurora_i18n::tr!("ui-pay-owned-by", name = owner.clone()))
                                .size(11.5)
                                .color(p.muted),
                        );
                    }
                });
            });
            if !available {
                ui.label(RichText::new(aurora_i18n::tr!("ui-object-actions-this-object-is-no-longer-available")).color(p.danger));
            }
            let Some((default, buttons)) = d.prices.clone() else {
                let text = if d.opened.elapsed().as_secs() > 15 {
                    aurora_i18n::tr!("ui-object-actions-the-simulator-did-not-respond-close-and-try-again")
                } else {
                    aurora_i18n::tr!("ui-object-actions-loading-suggested-amounts")
                };
                ui.label(RichText::new(text).color(p.muted));
                if widgets::dialog_buttons(
                    ui,
                    p,
                    &[button("", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, true)],
                    BUTTON_H,
                )
                .is_some()
                {
                    close = true;
                }
                return;
            };
            // PAY_PRICE_HIDE (-1) hides the free amount
            let free = default != -1;
            let quick: Vec<i32> = buttons.iter().copied().filter(|v| *v > 0).collect();
            // with the free amount, a button fills it in; without it (a vendor
            // selling at its prices), the button pays at once, like the
            // Legacy Viewer's quick-pay buttons (LLFloaterPay::onGive). With
            // the terminal a vendor's button chooses its price, and the card
            // or the Pay button pays it: the only price is chosen already,
            // and an amount that is none of them (the last one paid
            // elsewhere) is not.
            let chooses = free || contactless;
            let mut amount = None;
            let mut pay = None;
            // (greyed once the payment is made)
            ui.add_enabled_ui(!receipt, |ui| {
                if free {
                    amount_field(ui, p, &mut d.amount, &mut false);
                } else {
                    ui.label(RichText::new(aurora_i18n::tr!("ui-pay-object-amounts")).size(12.0).color(p.muted));
                }
                if !receipt && contactless && !free && parse_amount(&d.amount).is_none_or(|a| !quick.contains(&a)) {
                    d.amount = match quick.as_slice() {
                        [only] => only.to_string(),
                        _ => String::new(),
                    };
                }
                amount = if chooses { parse_amount(&d.amount) } else { None };
                if !quick.is_empty() {
                    ui.columns(quick.len(), |cols| {
                        for (col, value) in cols.iter_mut().zip(quick.iter().copied()) {
                            let (icon, kind) = if !chooses {
                                ("coin-filled", ActionKind::Primary)
                            } else if amount == Some(value) {
                                ("", ActionKind::Primary)
                            } else {
                                ("", ActionKind::Flat)
                            };
                            let h = if chooses { 28.0 } else { 40.0 };
                            if widgets::action_button(col, p, icon, &money(value), kind, Vec2::new(col.available_width(), h)).clicked()
                                && asked.is_none()
                            {
                                if chooses {
                                    d.amount = value.to_string();
                                } else if available {
                                    pay = Some(value);
                                }
                            }
                        }
                    });
                }
            });
            let after = amount.zip(world.balance).map(|(a, b)| b - a).filter(|_| !receipt);
            ui.horizontal(|ui| {
                ui.label(RichText::new(aurora_i18n::tr!("ui-pay-balance")).size(12.0).color(p.muted));
                ui.label(
                    RichText::new(world.balance.map_or_else(|| "…".to_owned(), money))
                        .size(12.0)
                        .strong()
                        .color(p.ink),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(after) = after.filter(|a| *a >= 0) {
                        ui.label(RichText::new(money(after)).size(12.0).strong().color(p.ink));
                        ui.label(RichText::new(aurora_i18n::tr!("ui-pay-after")).size(12.0).color(p.muted));
                    }
                });
            });
            // a notice only: the payment stays possible and opens Buy L$ for
            // what is missing (give_money, can_afford_transaction)
            let short = if free || amount.is_some() {
                after.is_some_and(|a| a < 0)
            } else {
                world.balance.zip(quick.iter().min()).is_some_and(|(b, low)| *low > b)
            };
            if short {
                band(ui, p.danger, |ui| {
                    ui.label(RichText::new(aurora_i18n::tr!("common-insufficient-or-unknown-balance")).color(p.danger));
                });
            }
            if let Some(error) = d.error.as_ref().filter(|_| !receipt) {
                ui.label(RichText::new(error).color(p.danger));
            }
            if let Some(value) = asked {
                band(ui, p.amber, |ui| {
                    let name = d.props.as_ref().map(|props| props.name.clone()).unwrap_or_default();
                    ui.label(
                        RichText::new(aurora_i18n::tr!("ui-pay-confirm", amount = money(value), name = name))
                            .strong()
                            .color(p.amber),
                    );
                });
                match widgets::dialog_buttons(
                    ui,
                    p,
                    &[
                        button("", aurora_i18n::tr!("common-no"), ActionKind::Flat, true),
                        button("coin-filled", aurora_i18n::tr!("common-pay"), ActionKind::Primary, true),
                    ],
                    BUTTON_H,
                ) {
                    Some(0) => asked = None,
                    Some(_) => {
                        result = Some(value);
                        asked = None;
                    }
                    None => {}
                }
                return;
            }
            let label = amount.map_or_else(
                || aurora_i18n::tr!("common-pay").to_owned(),
                |a| aurora_i18n::tr!("ui-pay-amount", amount = money(a)).to_owned(),
            );
            if contactless {
                // the terminal in the Pay button's place, and a Pay button
                // that holds the card in our place; short of L$ the payment
                // stays possible and opens Buy L$, as above
                let holder = world.social.display_name_of(&world.agent_id);
                let received = if accepted {
                    let name = d.props.as_ref().map(|props| props.name.clone()).unwrap_or_default();
                    aurora_i18n::tr!("ui-pay-terminal-accepted-help", name = name, amount = money(amount.unwrap_or(0)))
                } else {
                    String::new()
                };
                let status = if accepted {
                    terminal::Status::Accepted(&received)
                } else if receipt {
                    terminal::Status::Sending
                } else if !available {
                    terminal::Status::Blocked {
                        label: aurora_i18n::tr!("ui-pay-terminal-ready"),
                        text: aurora_i18n::tr!("ui-object-actions-this-object-is-no-longer-available"),
                        alert: true,
                    }
                } else if amount.is_none() {
                    terminal::Status::Blocked {
                        label: aurora_i18n::tr!("ui-pay-terminal-amount"),
                        text: if free {
                            aurora_i18n::tr!("ui-pay-terminal-amount-help")
                        } else {
                            aurora_i18n::tr!("ui-pay-terminal-price-help")
                        },
                        alert: false,
                    }
                } else {
                    terminal::Status::Ready
                };
                let view = terminal::View {
                    amount,
                    holder: &holder,
                    status,
                    reduce_motion: rules.reduce_motion,
                    frozen: None,
                };
                if self.object_terminal.show(ui, p, &view) {
                    pay = amount;
                }
                let playing = self.object_terminal.playing();
                let can_pay = available && amount.is_some() && !playing;
                if receipt {
                    // nothing left to cancel: the window closes by itself
                    let leave = [button("", aurora_i18n::tr!("common-close"), ActionKind::Flat, true)];
                    if widgets::dialog_buttons(ui, p, &leave, BUTTON_H).is_some() {
                        close = true;
                    }
                } else {
                    match widgets::dialog_buttons(
                        ui,
                        p,
                        &[
                            button("", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, true),
                            button("coin-filled", &label, ActionKind::Primary, can_pay),
                        ],
                        BUTTON_H,
                    ) {
                        Some(0) if playing => self.object_terminal.stop(),
                        Some(0) => close = true,
                        Some(_) => self.object_terminal.play(),
                        None => {}
                    }
                }
            } else {
                // without the free amount the Legacy Viewer hides its Pay
                // button: the prices above pay
                let mut buttons = vec![button("", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, true)];
                if free {
                    buttons.push(button("coin-filled", &label, ActionKind::Primary, available && amount.is_some()));
                }
                match widgets::dialog_buttons(ui, p, &buttons, BUTTON_H) {
                    Some(0) => close = true,
                    Some(_) => pay = amount,
                    None => {}
                }
            }
            if let Some(a) = pay {
                if needs_confirmation(confirm, threshold, a, world.balance, false) {
                    asked = Some(a);
                } else {
                    result = Some(a);
                }
            }
        });
        ctx.data_mut(|data| match asked {
            Some(a) => {
                data.insert_temp(confirm_id, a);
            }
            None => {
                data.remove::<i32>(confirm_id);
            }
        });
        if !open || close {
            state.dialog = None;
            state.paid = None;
            self.object_window = None;
            self.object_receipt = None;
            ctx.data_mut(|data| {
                data.remove::<i32>(confirm_id);
            });
            return None;
        }
        // the payment leaves: if `Interactions::confirm` sends it, the window
        // stays to say what became of it
        if result.is_some() && contactless {
            self.object_receipt = Some(Receipt::default());
        }
        result
    }

    /// PayConfirmation of a payment to an object in Firestorm's look: the
    /// answer once given.
    pub fn show_object_confirm(&mut self, ctx: &egui::Context, p: &Palette) -> Option<i32> {
        let (amount, name) = self.object_confirm.clone()?;
        let answer = Modal::new(egui::Id::unique("pay_object_confirm_classic"))
            .show(ctx, |ui| {
                ui.label(RichText::new(aurora_i18n::tr!("ui-pay-confirm", amount = money(amount), name = name)).color(p.ink));
                widgets::dialog_buttons(
                    ui,
                    p,
                    &[
                        button("", aurora_i18n::tr!("common-pay"), ActionKind::Primary, true),
                        button("", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, true),
                    ],
                    24.0,
                )
            })
            .inner;
        match answer {
            Some(0) => {
                self.object_confirm = None;
                Some(amount)
            }
            Some(_) => {
                self.object_confirm = None;
                None
            }
            None => None,
        }
    }

    /// AURORA_DEMO_PAY: the window in the state its mode names
    /// (docs/TESTING.md).
    pub fn demo(&mut self, id: Uuid, mode: &str) {
        let group = mode == "group";
        self.open(id, group);
        // the terminal most of the way through a hold, the Pay button alone,
        // or the Legacy look
        self.demo_hold = (mode == "hold").then_some(0.9);
        self.demo_button = mode == "button";
        self.demo_classic = mode == "classic";
        if let Some(payee) = self.payee.as_mut() {
            match mode {
                "short" => payee.amount = "1500".into(),
                "empty" => payee.amount.clear(),
                "confirm" => {
                    payee.amount = "500".into();
                    payee.stage = Stage::Confirm(500);
                }
                "sent" => {
                    payee.amount = "25".into();
                    payee.message = aurora_i18n::tr!("demo-pay-message").into();
                    payee.stage = Stage::Sent {
                        amount: 25,
                        balance: Some(975),
                    };
                }
                _ => payee.amount = "25".into(),
            }
        }
    }
}

fn button<'a>(icon: &'a str, label: &'a str, kind: ActionKind, enabled: bool) -> DialogButton<'a> {
    DialogButton {
        icon,
        label,
        kind,
        enabled,
    }
}

/// Cut `text` to `max` bytes on a character boundary (max_length_bytes).
fn truncate_bytes(text: &mut String, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

/// A raised card of the window, its content in a row.
fn card(ui: &mut egui::Ui, p: &Palette, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(10, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                body(ui);
            });
        });
}

/// A band of `color` with its edge mark: a confirmation or a refusal.
fn band(ui: &mut egui::Ui, color: egui::Color32, body: impl FnOnce(&mut egui::Ui)) {
    let response = egui::Frame::new()
        .fill(color.gamma_multiply(0.10))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin {
            left: 12,
            right: 10,
            top: 8,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| body(ui));
        })
        .response;
    let r = response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_max(r.left_top(), egui::pos2(r.left() + 3.0, r.bottom())),
        egui::CornerRadius::ZERO,
        color,
    );
}

/// The amount in large figures, ready to type into (digits only).
fn amount_field(ui: &mut egui::Ui, p: &Palette, text: &mut String, focus: &mut bool) {
    egui::Frame::new()
        .fill(p.bar)
        .corner_radius(ui.visuals().window_corner_radius)
        .stroke(egui::Stroke::new(1.0, p.violet_light.gamma_multiply(0.45)))
        .inner_margin(egui::Margin::symmetric(10, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // "L$" on the figures' baseline
            let row = Vec2::new(ui.available_width(), 40.0);
            ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Max), |ui| {
                let font = egui::FontId::proportional(32.0);
                let digits_w = ui
                    .painter()
                    .layout_no_wrap(if text.is_empty() { "0".into() } else { text.clone() }, font.clone(), p.ink)
                    .size()
                    .x
                    + 8.0;
                let cur_w = 26.0;
                ui.add_space(((ui.available_width() - digits_w - cur_w) / 2.0).max(0.0));
                ui.label(RichText::new(aurora_i18n::currency()).size(18.0).strong().color(p.muted));
                // the hint in the figures' font: egui lays a hint out in the
                // body text style, not in the field's `font` (a small "0"
                // above the baseline of "L$")
                let response = widgets::restricted_text_field(ui, true, text, widgets::TextPolicy::Digits, |b, _| {
                    egui::TextEdit::singleline(b)
                        .font(font.clone())
                        .text_color(p.ink)
                        .hint_text(RichText::new("0").font(font).color(p.muted_dim))
                        .frame(egui::Frame::NONE)
                        .char_limit(AMOUNT_DIGITS)
                        .desired_width(digits_w)
                });
                if std::mem::take(focus) {
                    response.request_focus();
                }
            });
        });
}

/// "L$ 25 sent to Nova", the new balance and the message sent.
fn sent(ui: &mut egui::Ui, p: &Palette, name: &str, amount: i32, balance: Option<i32>, message: &str) {
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.add_space(10.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 23.0, p.success.gamma_multiply(0.15));
        if let Some(t) = super::icons::global("check") {
            ui.painter().image(
                t.id(),
                egui::Rect::from_center_size(rect.center(), Vec2::splat(26.0)),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                p.success,
            );
        }
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-pay-sent", amount = money(amount), name = name.to_owned()))
                .size(15.0)
                .strong()
                .color(p.ink),
        );
        let mut detail = balance.map_or_else(String::new, |b| {
            aurora_i18n::tr!("ui-pay-new-balance", amount = money(b)).to_owned()
        });
        if !message.trim().is_empty() {
            if !detail.is_empty() {
                detail.push_str(" · ");
            }
            detail.push_str(&format!("« {} »", message.trim()));
        }
        if !detail.is_empty() {
            ui.label(RichText::new(detail).size(12.0).color(p.muted));
        }
        ui.add_space(4.0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::NetEvent;
    use widgets::elide_tests::Frames;

    /// The Aurora look with its terminal or its Pay button alone, and no
    /// confirmation.
    fn rules(contactless: bool) -> Rules {
        Rules {
            classic: false,
            contactless,
            confirm: false,
            threshold: 200,
            reduce_motion: false,
        }
    }

    /// A click on the text `label` of the window `draw` shows.
    fn click(frames: &mut Frames, mut draw: impl FnMut(&mut egui::Ui), label: &str) {
        let texts = frames.run(Vec::new(), &mut draw);
        let pos = texts
            .iter()
            .find(|t| t.source == label)
            .unwrap_or_else(|| panic!("no {label} in the window"))
            .rect
            .center();
        for pressed in [true, false] {
            frames.run(
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                &mut draw,
            );
        }
    }

    /// One frame of the window `draw` shows; true when it says `text`.
    fn shows(frames: &mut Frames, draw: impl FnMut(&mut egui::Ui), text: &str) -> bool {
        frames.run(Vec::new(), draw).iter().any(|t| t.source == text)
    }

    /// With the terminal a vendor's only price is chosen already and a click
    /// on it pays nothing; the Pay button holds the card to the end of the
    /// hold, then the payment leaves, the window says it was accepted once
    /// the balance changed and closes by itself; Cancel on the way takes the
    /// card back: nothing leaves and the window stays.
    #[test]
    fn the_terminal_pays_a_vendor_after_its_hold_and_cancel_takes_the_card_back() {
        let library = std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load());
        for stop in [false, true] {
            let mut world = World::new(library.clone());
            for event in crate::demo::events().into_iter().chain(crate::demo::action_events()) {
                world.apply(event);
            }
            world.balance = Some(1000);
            let idx = world.objects.index_of_uuid(&crate::demo::action_id(972)).unwrap();
            let target = crate::interaction::menu_target(&world, idx, Action::Pay).unwrap();
            let mut state = Interactions::default();
            state.open(target);
            state.on_properties(&[aurora_net::build::ObjectProps {
                object_id: target.root,
                name: "Vendor".into(),
                ..Default::default()
            }]);
            state.on_prices(target.object, -1, vec![180, -1, -1, -1]);
            let mut frames = Frames::new();
            let theme = crate::theme::Theme::default();
            theme.apply(&frames.ctx, 1.0);
            let p = theme.palette();
            let mut pay = PayUi::default();
            let commands = std::cell::RefCell::new(Vec::new());
            let open = std::cell::Cell::new(true);
            let receipt = std::cell::Cell::new(false);
            let accepted = aurora_i18n::tr!("ui-pay-terminal-accepted");
            let mut said = false;
            let mut draw = |ui: &mut egui::Ui| {
                if let Some(amount) = pay.show_object(ui.ctx(), &p, &mut state, &world, rules(true))
                    && let Some(command) = state.confirm(&world, Some(amount))
                {
                    commands.borrow_mut().push(command);
                    // the server's answer, as the demo gives it
                    pay.on_balance(820);
                }
                open.set(state.dialog.is_some());
                receipt.set(state.paid.is_some());
            };
            for _ in 0..3 {
                frames.run(Vec::new(), &mut draw);
            }
            click(&mut frames, &mut draw, &money(180));
            for _ in 0..90 {
                frames.run(Vec::new(), &mut draw);
            }
            assert!(commands.borrow().is_empty(), "a click on the price pays nothing");
            click(&mut frames, &mut draw, &aurora_i18n::tr!("ui-pay-amount", amount = money(180)));
            for _ in 0..30 {
                frames.run(Vec::new(), &mut draw);
            }
            assert!(commands.borrow().is_empty(), "nothing before the end of the hold");
            if stop {
                click(&mut frames, &mut draw, aurora_i18n::tr!("common-cancel"));
            }
            for _ in 0..90 {
                said |= shows(&mut frames, &mut draw, accepted);
            }
            if stop {
                assert!(commands.borrow().is_empty(), "the card taken back");
                assert!(open.get(), "Cancel on the way keeps the window");
                assert!(!receipt.get() && !said);
            } else {
                assert!(
                    matches!(commands.borrow().as_slice(), [NetCommand::PayObject { amount: 180, .. }]),
                    "{:?}",
                    commands.borrow()
                );
                assert!(!open.get(), "nothing left to pay in the window");
                assert!(receipt.get() && said, "the window stays to say the payment was accepted");
                // then closes by itself, with no second payment
                for _ in 0..120 {
                    frames.run(Vec::new(), &mut draw);
                }
                assert!(!receipt.get(), "the window closed by itself");
                assert_eq!(commands.borrow().len(), 1);
                // a payment the terminal did not make (the demo's scripted
                // one) shows nothing: its window is closed at once
                state.open(target);
                state.on_properties(&[aurora_net::build::ObjectProps {
                    object_id: target.root,
                    name: "Vendor".into(),
                    ..Default::default()
                }]);
                state.on_prices(target.object, -1, vec![180, -1, -1, -1]);
                assert!(state.confirm(&world, Some(180)).is_some());
                assert!(state.paid.is_some());
                frames.run(Vec::new(), |ui| {
                    assert!(pay.show_object(ui.ctx(), &p, &mut state, &world, rules(true)).is_none());
                });
                assert!(state.paid.is_none());
            }
        }
    }

    /// The resident's window with the terminal: its Pay button sends the
    /// payment at the end of the hold, not at the click.
    #[test]
    fn the_terminal_pays_a_resident_at_the_end_of_the_hold() {
        let to = Uuid::from_u128(1);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.apply(NetEvent::Balance(100));
        let mut frames = Frames::new();
        let theme = crate::theme::Theme::default();
        theme.apply(&frames.ctx, 1.0);
        let p = theme.palette();
        let (avatars, images) = (HashMap::new(), HashMap::new());
        let mut pay = PayUi::default();
        pay.open(to, false);
        pay.payee.as_mut().unwrap().amount = "25".into();
        let sent = std::cell::RefCell::new(Vec::new());
        let sending = std::cell::Cell::new(false);
        let mut draw = |ui: &mut egui::Ui| {
            for action in pay.show(ui.ctx(), &p, &world, (&avatars, &images), rules(true)) {
                sent.borrow_mut().push(action);
            }
            sending.set(pay.payee.as_ref().is_some_and(|payee| matches!(payee.stage, Stage::Sending { .. })));
        };
        for _ in 0..3 {
            frames.run(Vec::new(), &mut draw);
        }
        click(&mut frames, &mut draw, &aurora_i18n::tr!("ui-pay-amount", amount = money(25)));
        for _ in 0..30 {
            frames.run(Vec::new(), &mut draw);
        }
        assert!(sent.borrow().is_empty() && !sending.get(), "nothing before the end of the hold");
        for _ in 0..60 {
            frames.run(Vec::new(), &mut draw);
        }
        assert!(
            matches!(
                sent.borrow().as_slice(),
                [PayAction::Send(cmd)] if matches!(**cmd, NetCommand::PayResident { amount: 25, group: false, .. })
            ),
            "{:?}",
            sent.borrow()
        );
        assert!(sending.get(), "the window waits for the server's answer");
    }

    #[test]
    fn both_object_pay_windows_send_menu_payments_and_close() {
        let library = std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load());
        for classic in [false, true] {
            for linked in [false, true] {
                let mut world = World::new(library.clone());
                for event in crate::demo::events().into_iter().chain(crate::demo::action_events()) {
                    world.apply(event);
                }
                let parent = world.objects.index_of_uuid(&crate::demo::action_id(972)).unwrap();
                world.objects.get_mut(parent).unwrap().click_action = crate::interaction::code::TOUCH;
                let idx = if linked {
                    let child = world.objects.index_of_uuid(&crate::demo::action_id(974)).unwrap();
                    world.objects.get_mut(child).unwrap().parent_id = 972;
                    child
                } else {
                    parent
                };
                let target = crate::interaction::menu_target(&world, idx, Action::Pay).unwrap();
                let mut state = Interactions::default();
                state.open(target);
                state.on_properties(&[aurora_net::build::ObjectProps {
                    object_id: target.root,
                    name: "Tip jar".into(),
                    ..Default::default()
                }]);
                state.on_prices(target.object, 10, vec![1, 5, 10, 20]);
                let mut frames = widgets::elide_tests::Frames::new();
                let theme = crate::theme::Theme::default();
                theme.apply(&frames.ctx, 1.0);
                let p = theme.palette();
                let mut pay = PayUi::default();
                let mut commands = Vec::new();
                {
                    let mut draw = |ui: &mut egui::Ui| {
                        let answer = if classic {
                            super::super::object_actions::show(ui.ctx(), &p, &mut state, &world)
                        } else {
                            pay.show_object(ui.ctx(), &p, &mut state, &world, rules(false)).map(Some)
                        };
                        if let Some(amount) = answer
                            && let Some(command) = state.confirm(&world, amount)
                        {
                            commands.push(command);
                        }
                    };
                    let label = if classic {
                        aurora_i18n::tr!("common-pay").to_owned()
                    } else {
                        aurora_i18n::tr!("ui-pay-amount", amount = money(10)).to_owned()
                    };
                    for _ in 0..3 {
                        frames.run(Vec::new(), &mut draw);
                    }
                    let texts = frames.run(Vec::new(), &mut draw);
                    let pos = texts.iter().find(|t| t.source == label).expect("Pay button").rect.center();
                    for pressed in [true, false] {
                        frames.run(
                            vec![
                                egui::Event::PointerMoved(pos),
                                egui::Event::PointerButton {
                                    pos,
                                    button: egui::PointerButton::Primary,
                                    pressed,
                                    modifiers: Default::default(),
                                },
                            ],
                            &mut draw,
                        );
                    }
                    frames.run(Vec::new(), &mut draw);
                }
                assert!(matches!(commands.as_slice(), [NetCommand::PayObject { object, amount: 10, .. }] if *object == target.clicked_id));
                assert!(state.dialog.is_none(), "classic={classic}, linked={linked}");
            }
        }
    }

    /// Issue #1869: a vendor hiding the free amount (llSetPayPrice(PAY_HIDE,
    /// [180, …])) sells at one click on its price, in both windows, like
    /// the Legacy Viewer's quick-pay buttons; short of L$, the same click
    /// asks for Buy L$ instead of a greyed button.
    #[test]
    fn a_vendor_price_pays_at_one_click_or_asks_for_the_missing_l() {
        let library = std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load());
        for classic in [false, true] {
            for (balance, last) in [(Some(1000), 0), (Some(1000), 25), (Some(100), 0), (None, 0)] {
                let mut world = World::new(library.clone());
                for event in crate::demo::events().into_iter().chain(crate::demo::action_events()) {
                    world.apply(event);
                }
                world.balance = balance;
                let idx = world.objects.index_of_uuid(&crate::demo::action_id(972)).unwrap();
                let target = crate::interaction::menu_target(&world, idx, Action::Pay).unwrap();
                let mut state = Interactions::default();
                if last > 0 {
                    // an earlier payment at another price (`last_amount`)
                    state.open(target);
                    state.on_properties(&[aurora_net::build::ObjectProps {
                        object_id: target.root,
                        ..Default::default()
                    }]);
                    state.on_prices(target.object, 10, vec![last]);
                    let mut rich = World::new(library.clone());
                    for event in crate::demo::events().into_iter().chain(crate::demo::action_events()) {
                        rich.apply(event);
                    }
                    rich.balance = Some(1000);
                    assert!(state.confirm(&rich, Some(last)).is_some());
                }
                state.open(target);
                state.on_properties(&[aurora_net::build::ObjectProps {
                    object_id: target.root,
                    name: "Vendor".into(),
                    ..Default::default()
                }]);
                state.on_prices(target.object, -1, vec![180, -1, -1, -1]);
                let mut frames = widgets::elide_tests::Frames::new();
                let theme = crate::theme::Theme::default();
                theme.apply(&frames.ctx, 1.0);
                let p = theme.palette();
                let mut pay = PayUi::default();
                let mut commands = Vec::new();
                let mut shortfall = None;
                {
                    let mut draw = |ui: &mut egui::Ui| {
                        let answer = if classic {
                            super::super::object_actions::show(ui.ctx(), &p, &mut state, &world)
                        } else {
                            pay.show_object(ui.ctx(), &p, &mut state, &world, rules(false)).map(Some)
                        };
                        if let Some(amount) = answer {
                            match state.confirm(&world, amount) {
                                Some(command) => commands.push(command),
                                None => shortfall = state.take_shortfall(),
                            }
                        }
                    };
                    for _ in 0..3 {
                        frames.run(Vec::new(), &mut draw);
                    }
                    let texts = frames.run(Vec::new(), &mut draw);
                    let pos = texts.iter().find(|t| t.source == money(180)).expect("price button").rect.center();
                    for pressed in [true, false] {
                        frames.run(
                            vec![
                                egui::Event::PointerMoved(pos),
                                egui::Event::PointerButton {
                                    pos,
                                    button: egui::PointerButton::Primary,
                                    pressed,
                                    modifiers: Default::default(),
                                },
                            ],
                            &mut draw,
                        );
                    }
                    frames.run(Vec::new(), &mut draw);
                }
                let case = format!("classic={classic}, balance={balance:?}, last={last}");
                match balance {
                    Some(1000) => {
                        assert!(
                            matches!(commands.as_slice(), [NetCommand::PayObject { object, amount: 180, .. }] if *object == target.clicked_id),
                            "{case}: {commands:?}"
                        );
                        assert!(state.dialog.is_none(), "{case}");
                    }
                    _ => {
                        assert!(commands.is_empty(), "{case}");
                        assert_eq!(shortfall, Some(180 - balance.unwrap_or(0)), "{case}");
                        assert!(state.dialog.is_some(), "{case}: the window stays to try again");
                    }
                }
            }
        }
    }

    #[test]
    fn a_payment_is_confirmed_above_the_threshold_only() {
        // LLFloaterPay::onGive with FSConfirmPayments and its threshold
        assert!(!needs_confirmation(true, 200, 200, Some(1000), false));
        assert!(needs_confirmation(true, 200, 201, Some(1000), false));
        assert!(!needs_confirmation(false, 200, 500, Some(1000), false));
        // not when the balance falls short: give_money opens Buy L$ instead
        assert!(!needs_confirmation(true, 200, 500, Some(100), false));
        // nor when paying oneself (FIRE-24208)
        assert!(!needs_confirmation(true, 200, 500, Some(1000), true));
    }

    #[test]
    fn rlva_pay_closes_an_unsent_payment_and_leaves_a_sent_one() {
        let recipient = Uuid::from_u128(3);
        for (stage, closes) in [
            (Stage::Edit, true),
            (Stage::Confirm(25), true),
            (
                Stage::Sending {
                    amount: 25,
                    since: Instant::now(),
                },
                false,
            ),
            (
                Stage::Sent {
                    amount: 25,
                    balance: Some(75),
                },
                false,
            ),
            (Stage::Failed("refused".into()), false),
        ] {
            let mut pay = PayUi::default();
            pay.open(recipient, false);
            pay.payee.as_mut().unwrap().stage = stage.clone();
            assert_eq!(pay.close_resident(), closes, "{stage:?}");
            assert_eq!(pay.payee.is_none(), closes, "{stage:?}");
        }
        assert!(!PayUi::default().close_resident(), "nothing open, no notice");
    }

    #[test]
    fn rlva_pay_restriction_refuses_residents_and_groups_without_a_sent_state() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.rlva_configure(true);
        let restriction = aurora_net::ChatMessage {
            from_name: "RLVa test object".into(),
            source_id: Uuid::from_u128(1),
            owner_id: Uuid::from_u128(2),
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: "@pay=n".into(),
            region: None,
        };
        assert!(world.rlva.receive(&restriction));
        assert!(!world.rlva_can_pay());
        // enough L$: without the check the payment would go out (and not
        // open Buy L$ instead)
        world.balance = Some(100);
        for group in [false, true] {
            let mut pay = PayUi::default();
            let recipient = Uuid::from_u128(if group { 4 } else { 3 });
            pay.open(recipient, group);
            let mut actions = Vec::new();
            pay.give(25, &world, &mut actions);
            assert!(actions.is_empty());
            pay.on_transaction(true, TRANS_GIFT, recipient, "");
            pay.on_balance(75);
            assert_eq!(
                pay.payee.as_ref().unwrap().stage,
                Stage::Failed(aurora_i18n::tr!("rlva-blocked").into())
            );
        }
    }

    #[test]
    fn rlva_pay_restriction_does_not_open_a_resident_or_group_payment() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.rlva_configure(true);
        let restriction = aurora_net::ChatMessage {
            from_name: "RLVa test object".into(),
            source_id: Uuid::from_u128(1),
            owner_id: Uuid::from_u128(2),
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: "@pay=n".into(),
            region: None,
        };
        assert!(world.rlva.receive(&restriction));
        for group in [false, true] {
            let mut pay = PayUi::default();
            assert!(!pay.open_if_allowed(Uuid::from_u128(3), group, &world));
            assert!(pay.payee.is_none());
        }
    }

    #[test]
    fn the_last_amount_is_filled_in_again() {
        let mut pay = PayUi::default();
        pay.open(Uuid::from_u128(1), false);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.apply(NetEvent::Balance(100));
        assert_eq!(pay.payee.as_ref().unwrap().amount, "");
        let mut actions = Vec::new();
        pay.give(25, &world, &mut actions);
        assert!(matches!(
            actions.as_slice(),
            [PayAction::Send(cmd)] if matches!(**cmd, NetCommand::PayResident { amount: 25, group: false, .. })
        ));
        pay.open(Uuid::from_u128(2), false);
        assert_eq!(pay.payee.as_ref().unwrap().amount, "25");
    }

    #[test]
    fn a_short_balance_opens_buy_currency_for_what_is_missing() {
        let mut pay = PayUi::default();
        pay.open(Uuid::from_u128(1), false);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.apply(NetEvent::Balance(1000));
        let mut actions = Vec::new();
        pay.give(1500, &world, &mut actions);
        assert!(matches!(actions.as_slice(), [PayAction::BuyCurrency(500)]));
        assert_eq!(pay.payee.as_ref().unwrap().stage, Stage::Edit);
    }

    #[test]
    fn the_server_answer_ends_the_payment() {
        let to = Uuid::from_u128(1);
        let mut pay = PayUi::default();
        pay.open(to, false);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.apply(NetEvent::Balance(100));
        pay.give(25, &world, &mut Vec::new());
        // another transaction does not count
        pay.on_transaction(true, TRANS_GIFT, Uuid::from_u128(9), "");
        assert!(matches!(pay.payee.as_ref().unwrap().stage, Stage::Sending { .. }));
        pay.on_transaction(true, TRANS_GIFT, to, "");
        pay.on_balance(75);
        assert_eq!(
            pay.payee.as_ref().unwrap().stage,
            Stage::Sent {
                amount: 25,
                balance: Some(75)
            }
        );
        // a refusal keeps the window open with its reason
        pay.open(to, false);
        pay.give(25, &world, &mut Vec::new());
        pay.on_transaction(false, TRANS_GIFT, to, "Refused");
        pay.on_balance(100);
        assert_eq!(pay.payee.as_ref().unwrap().stage, Stage::Failed("Refused".into()));
    }

    #[test]
    fn the_payee_picture_is_asked_of_the_app() {
        // issue #1195: the card showed the initials, its picture never asked for
        let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let (avatars, images) = (HashMap::new(), HashMap::new());
        let mut pay = PayUi::default();
        let id = Uuid::from_u128(1);
        pay.open(id, false);
        assert!(pay.payee_picture(&world, (&avatars, &images)).is_none());
        assert!(pay.wanted_pics.contains(&id));
        // a group with no insignia known asks for nothing
        pay.open(Uuid::from_u128(2), true);
        assert!(pay.payee_picture(&world, (&avatars, &images)).is_none());
        assert!(pay.wanted_images.is_empty());
    }

    #[test]
    fn the_message_is_cut_on_a_character_boundary() {
        let mut text = "é".repeat(100);
        truncate_bytes(&mut text, MESSAGE_BYTES);
        assert_eq!(text.len(), 126);
    }
}

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

use super::widgets::{self, ActionKind, DialogButton, Floater, Modal};
use crate::interaction::{Action, Interactions};
use crate::theme::Palette;
use crate::world::World;
use aurora_net::NetCommand;
use egui::{Align, Layout, RichText, Sense, Vec2};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use uuid::Uuid;

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
    format!("L$ {}", aurora_i18n::integer(i64::from(amount.max(0))))
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
}

/// Textures by id: the profile pictures by avatar, the UI images by asset
/// (a group's insignia).
type Pictures<'a> = (&'a HashMap<Uuid, egui::TextureHandle>, &'a HashMap<Uuid, egui::TextureHandle>);

/// The amount typed, if it is a positive number (LLFloaterPay::onKeystroke).
fn parse_amount(text: &str) -> Option<i32> {
    text.trim().parse::<i32>().ok().filter(|v| *v > 0)
}

/// FSConfirmPayments: above the threshold, with the balance to cover it and
/// someone else to pay (LLFloaterPay::onGive).
pub fn needs_confirmation(confirm: bool, threshold: i32, amount: i32, balance: Option<i32>, to_self: bool) -> bool {
    confirm && amount > threshold && balance.is_some_and(|b| b >= amount) && !to_self
}

impl PayUi {
    /// LLFloaterPayUtil::payDirectly: a resident, or a group with `group`.
    pub fn open(&mut self, id: Uuid, group: bool) {
        self.raise = true;
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

    /// LLFloaterPay::give, or the Buy L$ window when the balance is short
    /// (give_money). The Aurora look waits for the server's answer.
    fn give(&mut self, amount: i32, balance: Option<i32>, actions: &mut Vec<PayAction>) {
        let Some(payee) = self.payee.as_mut() else { return };
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
        let to_self = !payee.group && payee.id == world.agent_id;
        if needs_confirmation(confirm, threshold, amount, world.balance, to_self) {
            payee.stage = Stage::Confirm(amount);
        } else {
            self.give(amount, world.balance, actions);
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

    /// The window of the look chosen; `classic`: floater_pay.xml.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &World,
        pics: Pictures,
        classic: bool,
        confirm: bool,
        threshold: i32,
    ) -> Vec<PayAction> {
        let mut actions = Vec::new();
        if self.payee.is_some() {
            if std::mem::take(&mut self.raise) {
                widgets::expand_floater(ctx, if classic { "pay_classic" } else { "pay" });
            }
            if classic {
                self.show_classic(ctx, p, world, confirm, threshold, &mut actions);
            } else {
                self.show_aurora(ctx, p, world, pics, confirm, threshold, &mut actions);
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

    fn show_aurora(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &World,
        pics: Pictures,
        confirm: bool,
        threshold: i32,
        actions: &mut Vec<PayAction>,
    ) {
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
        Floater::new(
            "pay",
            title,
            egui::pos2(center.x - WIDTH / 2.0, center.y - 200.0),
            Vec2::new(WIDTH, 360.0),
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
            match payee.stage.clone() {
                Stage::Sent { amount, balance } => {
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
                Stage::Sending { .. } => {
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
            amount_field(ui, p, &mut payee.amount, &mut payee.focus);
            let amount = parse_amount(&payee.amount);
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
                ui.add(
                    egui::TextEdit::singleline(&mut payee.message)
                        .hint_text(aurora_i18n::tr!("ui-pay-message-hint"))
                        .char_limit(MESSAGE_BYTES)
                        .desired_width(f32::INFINITY),
                );
                truncate_bytes(&mut payee.message, MESSAGE_BYTES);
            }
            let after = amount.zip(world.balance).map(|(a, b)| b - a);
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
            self.give(amount, world.balance, actions);
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
        Floater::new(
            "pay_classic",
            title,
            egui::pos2(center.x - 150.0, center.y - 130.0),
            Vec2::new(300.0, 260.0),
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
                ui.add(
                    egui::TextEdit::singleline(&mut payee.amount)
                        .char_limit(AMOUNT_DIGITS)
                        .desired_width(90.0),
                );
                payee.amount.retain(|c| c.is_ascii_digit());
            });
            if !payee.group {
                ui.horizontal(|ui| {
                    ui.label(aurora_i18n::tr!("ui-pay-classic-message"));
                    ui.add(
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
            self.give(amount, world.balance, actions);
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
    pub fn show_object(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        state: &mut Interactions,
        world: &World,
        confirm: bool,
        threshold: i32,
    ) -> Option<i32> {
        let d = state.dialog.as_mut().filter(|d| d.target.action == Action::Pay)?;
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
        Floater::new(
            "object_payment",
            aurora_i18n::tr!("common-pay-object"),
            egui::pos2(center.x - WIDTH / 2.0, center.y - 180.0),
            Vec2::new(WIDTH, 300.0),
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
            if free {
                amount_field(ui, p, &mut d.amount, &mut false);
            } else {
                ui.label(RichText::new(aurora_i18n::tr!("ui-pay-object-amounts")).size(12.0).color(p.muted));
            }
            let amount = if free {
                parse_amount(&d.amount)
            } else {
                parse_amount(&d.amount).filter(|a| quick.contains(a))
            };
            if !quick.is_empty() {
                ui.columns(quick.len(), |cols| {
                    for (col, value) in cols.iter_mut().zip(quick.iter().copied()) {
                        let kind = if amount == Some(value) {
                            ActionKind::Primary
                        } else {
                            ActionKind::Flat
                        };
                        let h = if free { 28.0 } else { 40.0 };
                        if widgets::action_button(col, p, "", &money(value), kind, Vec2::new(col.available_width(), h)).clicked() {
                            d.amount = value.to_string();
                        }
                    }
                });
            }
            let after = amount.zip(world.balance).map(|(a, b)| b - a);
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
            let short = after.is_some_and(|a| a < 0);
            if short {
                band(ui, p.danger, |ui| {
                    ui.label(RichText::new(aurora_i18n::tr!("common-insufficient-or-unknown-balance")).color(p.danger));
                });
            }
            if let Some(error) = &d.error {
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
            match widgets::dialog_buttons(
                ui,
                p,
                &[
                    button("", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, true),
                    button("coin-filled", &label, ActionKind::Primary, available && amount.is_some() && !short),
                ],
                BUTTON_H,
            ) {
                Some(0) => close = true,
                Some(_) => {
                    if let Some(a) = amount {
                        if needs_confirmation(confirm, threshold, a, world.balance, false) {
                            asked = Some(a);
                        } else {
                            result = Some(a);
                        }
                    }
                }
                None => {}
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
            ctx.data_mut(|data| {
                data.remove::<i32>(confirm_id);
            });
            return None;
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
                ui.label(RichText::new("L$").size(18.0).strong().color(p.muted));
                // the hint in the figures' font: egui lays a hint out in the
                // body text style, not in the field's `font` (a small "0"
                // above the baseline of "L$")
                let response = ui.add(
                    egui::TextEdit::singleline(text)
                        .font(font.clone())
                        .text_color(p.ink)
                        .hint_text(RichText::new("0").font(font).color(p.muted_dim))
                        .frame(egui::Frame::NONE)
                        .char_limit(AMOUNT_DIGITS)
                        .desired_width(digits_w),
                );
                if std::mem::take(focus) {
                    response.request_focus();
                }
                text.retain(|c| c.is_ascii_digit());
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
                            pay.show_object(ui.ctx(), &p, &mut state, &world, false, 200).map(Some)
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
    fn the_last_amount_is_filled_in_again() {
        let mut pay = PayUi::default();
        pay.open(Uuid::from_u128(1), false);
        assert_eq!(pay.payee.as_ref().unwrap().amount, "");
        let mut actions = Vec::new();
        pay.give(25, Some(100), &mut actions);
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
        let mut actions = Vec::new();
        pay.give(1500, Some(1000), &mut actions);
        assert!(matches!(actions.as_slice(), [PayAction::BuyCurrency(500)]));
        assert_eq!(pay.payee.as_ref().unwrap().stage, Stage::Edit);
    }

    #[test]
    fn the_server_answer_ends_the_payment() {
        let to = Uuid::from_u128(1);
        let mut pay = PayUi::default();
        pay.open(to, false);
        pay.give(25, Some(100), &mut Vec::new());
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
        pay.give(25, Some(100), &mut Vec::new());
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

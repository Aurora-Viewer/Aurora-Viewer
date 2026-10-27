//! Application bridge for the Pay windows (`ui/pay.rs`): the look chosen in
//! Preferences › Aurora, what they ask for, and the demo's scenario.

use super::*;
use crate::interaction::Action;
use ui::pay::PayAction;

impl App {
    /// The Pay window of a resident or a group, and the payment to an object
    /// in the look chosen (`Settings::classic_pay_window`).
    pub(super) fn show_pay(&mut self, ctx: &egui::Context, p: &Palette, a: &mut UiActions) {
        let classic = self.settings.classic_pay_window;
        let (confirm, threshold) = (self.settings.confirm_payments, self.settings.payment_confirm_threshold);
        let paying_object = self.interactions.dialog.as_ref().is_some_and(|d| d.target.action == Action::Pay);
        if paying_object && !classic {
            a.object_confirm = self
                .pay_ui
                .show_object(ctx, p, &mut self.interactions, &self.world, confirm, threshold)
                .map(Some);
        } else {
            let answer = ui::object_actions::show(ctx, p, &mut self.interactions, &self.world);
            // PayConfirmation before a payment to an object (LLFloaterPay::onGive)
            match answer {
                Some(Some(amount)) if ui::pay::needs_confirmation(confirm, threshold, amount, self.world.balance, false) => {
                    let name = self
                        .interactions
                        .dialog
                        .as_ref()
                        .and_then(|d| d.props.as_ref())
                        .map(|props| props.name.clone())
                        .unwrap_or_default();
                    self.pay_ui.object_confirm = Some((amount, name));
                }
                other => a.object_confirm = other,
            }
            if let Some(amount) = self.pay_ui.show_object_confirm(ctx, p) {
                a.object_confirm = Some(Some(amount));
            }
        }
        let pics = (&self.avatar_pics, &self.ui_images);
        for action in self.pay_ui.show(ctx, p, &self.world, pics, classic, confirm, threshold) {
            match action {
                PayAction::Send(cmd) => self.send(*cmd),
                PayAction::BuyCurrency(missing) => {
                    self.open_buy_currency();
                    self.buy_currency_ui.set_amount(missing);
                }
                PayAction::Profile(id) => a.ctx_action = Some(ui::context::CtxAction::Profile(id)),
            }
        }
    }

    /// AURORA_DEMO_PAY=resident|short|confirm|sent|group (docs/TESTING.md):
    /// the Pay window in that state, Nova as the payee. An object's payment
    /// is AURORA_DEMO_ACTIONS=pay.
    pub(super) fn demo_pay(&mut self) {
        let Ok(mode) = std::env::var("AURORA_DEMO_PAY") else {
            return;
        };
        self.world.apply(NetEvent::Balance(1000));
        match mode.as_str() {
            // the group with an insignia
            "group" => self.pay_ui.demo(crate::demo::DEMO_GROUP2, "group"),
            mode => self.pay_ui.demo(crate::demo::DEMO_NOVA, mode),
        }
        self.panels.perf = false;
    }
}

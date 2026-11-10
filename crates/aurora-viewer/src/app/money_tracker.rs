//! Application bridge for the money tracker (`ui/money_tracker.rs`): what it
//! records and when, the look chosen in Preferences › Aurora, what it asks
//! for, and the demo's scenario; with the demo of a direct payment's card in
//! the conversation with the other resident (`world/payments.rs`).

use super::*;
use aurora_net::trans::{GIFT, GROUP_JOIN, OBJECT_SALE, PAY_OBJECT, UPLOAD_CHARGE};
use ui::money_tracker::{Party, TrackerAction};

/// `AURORA_DEMO_PAYMENT=earning`: a payment this often.
const EARNING_EVERY: std::time::Duration = std::time::Duration::from_secs(10);

/// L$ of an `earning` payment, 20 to 300, from a random number.
fn earning_amount(random: u128) -> i32 {
    20 + (random % 281) as i32
}

impl App {
    /// Whether transactions are recorded: the window is open (minimized
    /// included) or tracks when closed.
    fn money_tracking(&self) -> bool {
        self.panels.money_tracker || self.settings.always_track_payments
    }

    /// A MoneyBalanceReply naming a transaction.
    pub(super) fn record_money(&mut self, ev: &NetEvent) {
        if !self.money_tracking() {
            return;
        }
        let agent = self.world.agent_id;
        if let Some(Party::Resident(id)) = self.money_tracker_ui.tracker.record(ev, agent, std::time::SystemTime::now()) {
            self.world.social.want_name(id);
        }
    }

    /// The money tracker in the look chosen (`Settings::legacy_money_tracker`).
    pub(super) fn show_money_tracker(&mut self, ctx: &egui::Context, p: &Palette, a: &mut UiActions) {
        if self.money_tracking() {
            self.money_tracker_ui.tracker.start(std::time::SystemTime::now());
        }
        let (always, legacy) = (self.settings.always_track_payments, self.settings.legacy_money_tracker);
        let mut open = self.panels.money_tracker;
        let actions = self
            .money_tracker_ui
            .show(ctx, p, &self.world, &self.avatar_pics, &mut open, always, legacy);
        self.panels.money_tracker = open;
        for action in actions {
            match action {
                TrackerAction::Profile(id) => a.ctx_action = Some(ui::context::CtxAction::Profile(id)),
                TrackerAction::GroupProfile(id) => self.group_profile_ui.open(&mut self.world, id),
                TrackerAction::Pay { id, group } => self.pay_ui.open(id, group),
                TrackerAction::AlwaysTrack(on) => {
                    self.settings.always_track_payments = on;
                    self.settings.save();
                }
            }
        }
    }

    /// AURORA_DEMO_MONEY_TRACKER=aurora|legacy|select|undo|empty
    /// (docs/TESTING.md): the window opened on a session of payments, in
    /// the look named (Aurora by default), two rows selected or just
    /// removed, or nothing recorded yet.
    pub(super) fn demo_money_tracker(&mut self) {
        let Ok(mode) = std::env::var("AURORA_DEMO_MONEY_TRACKER") else {
            return;
        };
        self.world.apply(NetEvent::Balance(12480));
        self.settings.legacy_money_tracker = mode == "legacy";
        self.panels.money_tracker = true;
        self.panels.perf = false;
        if mode == "empty" {
            return;
        }
        use crate::demo::{DEMO_FRIEND, DEMO_LOUP, DEMO_NOVA, DEMO_TESS};
        let group = crate::demo::DEMO_GROUP1;
        let r = Party::Resident;
        // (seconds ago, who, received, L$, type, item): newest last
        let rows = [
            (4380, r(DEMO_NOVA), true, 50, PAY_OBJECT, "Tip jar"),
            (4200, r(DEMO_LOUP), false, 450, OBJECT_SALE, "Steel helmet"),
            (3840, r(DEMO_TESS), true, 100, PAY_OBJECT, "Tip jar"),
            (3420, r(DEMO_FRIEND), true, 25, PAY_OBJECT, "Tip jar"),
            (2940, r(DEMO_NOVA), true, 75, GIFT, "Thanks for the set!"),
            (2640, Party::System, false, 10, UPLOAD_CHARGE, ""),
            (2100, Party::Group(group), false, 150, GROUP_JOIN, ""),
            (1860, r(DEMO_TESS), false, 20, GIFT, "Payment"),
            (1380, r(DEMO_LOUP), true, 500, OBJECT_SALE, "Floating lantern"),
            (720, r(DEMO_FRIEND), true, 25, PAY_OBJECT, "Tip jar"),
            (300, r(DEMO_TESS), true, 1000, PAY_OBJECT, "Tip jar"),
            (5, r(DEMO_NOVA), true, 50, PAY_OBJECT, "Tip jar"),
        ];
        self.money_tracker_ui.demo(&rows);
        for (_, party, ..) in rows {
            if let Party::Resident(id) = party {
                self.world.social.want_name(id);
            }
        }
        match mode.as_str() {
            "select" => self.money_tracker_ui.demo_select(&[1, 2]),
            "undo" => {
                self.money_tracker_ui.demo_select(&[0, 2]);
                self.money_tracker_ui.demo_delete();
            }
            _ => {}
        }
    }

    /// The payments of `AURORA_DEMO_PAYMENT=balance` as their times come,
    /// and those of `earning`, for as long as the viewer runs.
    pub(super) fn demo_gains_tick(&mut self, now: Instant) {
        if self.demo_earning.is_some_and(|next| now >= next) {
            self.demo_earning = Some(now + EARNING_EVERY);
            // the source of the demo's chance: no crate only for this
            let random = uuid::Uuid::new_v4();
            let number = random.as_u128();
            let amount = earning_amount(number >> 8);
            // the grid could send any balance: none of this overflows
            let balance = self.world.balance.unwrap_or(0);
            let (kind, item) = match number % 5 {
                0 => (OBJECT_SALE, "Steel helmet"),
                1 => (PAY_OBJECT, "Tip jar"),
                2 => (GIFT, "Payment"),
                // a purchase, and L$ bought: only the balance moves
                3 => {
                    self.world.apply(NetEvent::Balance(balance.saturating_sub(amount)));
                    return;
                }
                _ => {
                    self.world.apply(NetEvent::Balance(balance.saturating_add(amount * 10)));
                    return;
                }
            };
            let payers = [crate::demo::DEMO_NOVA, crate::demo::DEMO_TESS, crate::demo::DEMO_FRIEND];
            let ev = NetEvent::MoneyTransaction {
                id: random,
                success: true,
                kind,
                source: payers[(number >> 4) as usize % payers.len()],
                source_group: false,
                dest: self.world.agent_id,
                dest_group: false,
                amount,
                item: item.into(),
                description: "Transaction".into(),
            };
            self.demo_gains.push_back((now, ev));
        }
        while self.demo_gains.front().is_some_and(|(at, _)| *at <= now) {
            let Some((_, ev)) = self.demo_gains.pop_front() else {
                return;
            };
            if let NetEvent::MoneyTransaction { amount, .. } = ev {
                self.world.payment(&ev);
                let balance = self.world.balance.unwrap_or(0).saturating_add(amount);
                self.world.apply(NetEvent::Balance(balance));
            } else {
                self.world.apply(ev);
            }
        }
    }

    /// AURORA_DEMO_PAYMENT=card|legacy|sent|sent-open|sent-friend|friend|anyone|off|sale|balance|earning
    /// (docs/TESTING.md): a resident's conversation open on their message,
    /// then their two payments as cards, in the bubbles or the legacy lines;
    /// `sent`: then ours to them (the demo keeps no transcript: what comes
    /// back when the conversation is opened again is a unit test's);
    /// `sent-open`: no conversation open, ours opens it with "Open the
    /// conversation when you pay a resident" on Anyone; `sent-friend`: we pay
    /// a stranger and a friend with it on Friends only, the friend's opens;
    /// `friend`: a friend's payment to us opens their conversation, a
    /// stranger's (no conversation) shows nowhere; `anyone`: both open;
    /// `off`: the same on Never, no conversation opens; `sale`: the toasts of
    /// L$ received without a card; `balance`: payments and a purchase at the
    /// balance, no toast; `earning`: 20 to 300 L$ every 10 seconds, a sale,
    /// an object paid, a direct payment, a purchase or L$ bought, with the
    /// saved switches.
    pub(super) fn demo_payment(&mut self) {
        let Ok(mode) = std::env::var("AURORA_DEMO_PAYMENT") else {
            return;
        };
        use crate::demo::{DEMO_FRIEND, DEMO_NOVA, DEMO_TESS};
        let agent = self.world.agent_id;
        let event = |n: u128, kind, source, dest, amount, item: &str| NetEvent::MoneyTransaction {
            id: uuid::Uuid::from_u128(0x9A1D_0000 + n),
            success: true,
            kind,
            source,
            source_group: false,
            dest,
            dest_group: false,
            amount,
            item: item.into(),
            description: "Transaction".into(),
        };
        let gift = |n, source, dest, amount, item: &str| event(n, GIFT, source, dest, amount, item);
        let paid = |n, source, amount, item: &str| gift(n, source, agent, amount, item);
        if mode == "sale" {
            self.demo_money_notices = Some((true, true));
            // this frame's payments, before the next frame reads it
            self.world.payments.notify_direct = true;
            self.world.payments.notify_received = true;
            let earned = |n, kind, amount, item: &str| event(n, kind, DEMO_NOVA, agent, amount, item);
            self.world.payment(&earned(7, OBJECT_SALE, 100, "Steel helmet"));
            self.world.payment(&earned(8, PAY_OBJECT, 25, "Tip jar"));
            self.world.payment(&earned(9, PAY_OBJECT, 5, ""));
            self.world.payment(&paid(10, DEMO_TESS, 20, "Payment"));
            return;
        }
        if mode == "balance" {
            self.demo_money_notices = Some((false, false));
            self.world.apply(NetEvent::Balance(12480));
            self.balance_gains.resync();
            let start = Instant::now() + std::time::Duration::from_secs(6);
            for (n, (after, source, amount)) in [(0, DEMO_NOVA, 100), (1, DEMO_TESS, 2500), (2, DEMO_FRIEND, 25)]
                .into_iter()
                .enumerate()
            {
                let at = start + std::time::Duration::from_millis(after * 900);
                self.demo_gains.push_back((at, paid(11 + n as u128, source, amount, "Payment")));
            }
            // and a purchase: only the balance moves
            self.demo_gains
                .push_back((start + std::time::Duration::from_millis(2700), NetEvent::Balance(14805)));
            return;
        }
        if mode == "earning" {
            // the saved switches stay: they can be turned while it runs
            self.world.apply(NetEvent::Balance(12480));
            self.balance_gains.resync();
            self.demo_earning = Some(Instant::now() + EARNING_EVERY);
            return;
        }
        self.panels.perf = false;
        self.panels.chat = true;
        ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(480.0, 300.0), egui::vec2(600.0, 440.0));
        use crate::world::payments::Opens;
        if mode == "sent-open" {
            self.settings.payment_sent_opens = Opens::Anyone;
            self.world.payments.open_sent = Opens::Anyone;
            self.world
                .payment(&gift(6, agent, DEMO_NOVA, 300, aurora_i18n::tr!("demo-payment-rent")));
            self.chat_ui.open(DEMO_NOVA);
            return;
        }
        if mode == "sent-friend" {
            self.settings.payment_sent_opens = Opens::Friends;
            self.world.payments.open_sent = Opens::Friends;
            self.world.payment(&gift(7, agent, DEMO_TESS, 20, "Payment"));
            self.world
                .payment(&gift(8, agent, DEMO_FRIEND, 250, aurora_i18n::tr!("demo-payment-boat")));
            if self.world.social.has_session(&DEMO_FRIEND) {
                self.chat_ui.open(DEMO_FRIEND);
            }
            return;
        }
        let received = match mode.as_str() {
            "friend" => Some(Opens::Friends),
            "anyone" => Some(Opens::Anyone),
            "off" => Some(Opens::Never),
            _ => None,
        };
        if let Some(rule) = received {
            self.settings.payment_received_opens = rule;
            self.world.payments.open_received = rule;
            self.world.payment(&paid(1, DEMO_TESS, 20, "Payment"));
            self.world
                .payment(&paid(2, DEMO_FRIEND, 250, aurora_i18n::tr!("demo-payment-boat")));
            if self.world.social.has_session(&DEMO_FRIEND) {
                self.chat_ui.open(DEMO_FRIEND);
            }
            return;
        }
        self.world.apply(NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: DEMO_NOVA,
            from_name: String::new(),
            to_agent_id: agent,
            dialog: 0,
            session_id: uuid::Uuid::nil(),
            message: aurora_i18n::tr!("demo-payment-sending").into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        }));
        self.world
            .payment(&paid(3, DEMO_NOVA, 1500, aurora_i18n::tr!("demo-payment-thanks")));
        self.world.payment(&paid(4, DEMO_NOVA, 75, "Payment"));
        if mode == "sent" {
            self.world
                .payment(&gift(5, agent, DEMO_NOVA, 300, aurora_i18n::tr!("demo-payment-rent")));
        }
        self.chat_ui.open(DEMO_NOVA);
        if mode == "legacy" {
            self.chat_ui.demo_style = Some(crate::settings::ChatStyle::Legacy);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::earning_amount;

    #[test]
    fn an_earning_stays_between_20_and_300() {
        assert_eq!(earning_amount(0), 20);
        assert_eq!(earning_amount(280), 300);
        assert_eq!(earning_amount(281), 20);
        assert!((0..2000).all(|n| (20..=300).contains(&earning_amount(n * u128::from(u64::MAX)))));
    }
}

//! Application bridge for the money tracker (`ui/money_tracker.rs`): what it
//! records and when, the look chosen in Preferences › Aurora, what it asks
//! for, and the demo's scenario; with the demo of a direct payment's card in
//! the conversation with the other resident (`world/payments.rs`).

use super::*;
use ui::money_tracker::{Party, TrackerAction};

/// TRANS_PAY_OBJECT, TRANS_GIFT, TRANS_OBJECT_SALE, TRANS_GROUP_JOIN and
/// TRANS_UPLOAD_CHARGE (lltransactiontypes.h), for the demo's session.
const PAY_OBJECT: i32 = 5008;
const GIFT: i32 = 5001;
const OBJECT_SALE: i32 = 5000;
const GROUP_JOIN: i32 = 1004;
const UPLOAD_CHARGE: i32 = 1101;

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

    /// AURORA_DEMO_PAYMENT=card|legacy|sent|sent-open|friend|off
    /// (docs/TESTING.md): a resident's conversation open on their message,
    /// then their two payments as cards, in the bubbles or the legacy lines;
    /// `sent`: then ours to them (the demo keeps no transcript: what comes
    /// back when the conversation is opened again is a unit test's);
    /// `sent-open`: no conversation open, ours opens it with "Open the
    /// conversation when you pay a resident" on; `friend`: a friend's payment opens their conversation, a stranger's
    /// (no conversation) shows nowhere; `off`: the same with the option off,
    /// no conversation opens.
    pub(super) fn demo_payment(&mut self) {
        let Ok(mode) = std::env::var("AURORA_DEMO_PAYMENT") else {
            return;
        };
        use crate::demo::{DEMO_FRIEND, DEMO_NOVA, DEMO_TESS};
        let agent = self.world.agent_id;
        let gift = |n: u128, source, dest, amount, item: &str| NetEvent::MoneyTransaction {
            id: uuid::Uuid::from_u128(0x9A1D_0000 + n),
            success: true,
            kind: GIFT,
            source,
            source_group: false,
            dest,
            dest_group: false,
            amount,
            item: item.into(),
            description: String::new(),
        };
        let paid = |n, source, amount, item: &str| gift(n, source, agent, amount, item);
        self.panels.perf = false;
        self.panels.chat = true;
        ui::widgets::place_floater(&self.egui_ctx, "conversations", egui::pos2(480.0, 300.0), egui::vec2(600.0, 440.0));
        if mode == "sent-open" {
            self.settings.payment_sent_opens_conversation = true;
            self.world.payments.open_when_paying = true;
            self.world
                .payment(&gift(6, agent, DEMO_NOVA, 300, aurora_i18n::tr!("demo-payment-rent")));
            self.chat_ui.open(DEMO_NOVA);
            return;
        }
        if mode == "friend" || mode == "off" {
            self.settings.payment_opens_conversation = mode == "friend";
            self.world.payments.open_for_friends = self.settings.payment_opens_conversation;
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

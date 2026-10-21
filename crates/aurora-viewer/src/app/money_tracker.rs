//! Application bridge for the money tracker (`ui/money_tracker.rs`): what it
//! records and when, the look chosen in Preferences › Aurora, what it asks
//! for, and the demo's scenario.

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
}

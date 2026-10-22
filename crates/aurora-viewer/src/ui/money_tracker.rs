//! The money tracker: the L$ received and paid during the session, with who
//! and when. Port of FSMoneyTracker (indra/newview/fsmoneytracker.cpp,
//! floater_fs_money_tracker.xml, menu_fs_moneytracker_list.xml, originally
//! LGPL 2.1) and of its feed in process_money_balance_reply
//! (indra/newview/llviewermessage.cpp):
//! - a successful transaction that names someone is recorded while the
//!   window is open or minimized, or always with "Track when closed"
//!   (FSAlwaysTrackPayments, off by default);
//! - a reply whose TransactionID came within the last ones is dropped (the
//!   simulator may send it twice);
//! - received when the source is someone else, paid otherwise: the other
//!   party is the source of what we receive, the destination of what we pay;
//! - closing the window clears the list, unless it tracks when closed;
//! - "Copy as CSV" (time;name;amount, negative when paid) and "Delete" act
//!   on the selected rows, "Clear" on all of them.
//!
//! The Aurora look (`aurora.rs`, the human's choice of the mockup, variant
//! A) adds the totals, a received / paid filter, what each transaction was
//! (TransactionInfo's type and item), the newest first and an undo after a
//! deletion; `legacy.rs` is the Legacy Viewer's window
//! (`Settings::legacy_money_tracker`).

mod aurora;
mod legacy;

use crate::theme::Palette;
use crate::world::World;
use aurora_net::NetEvent;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Window ids: the two looks keep their own place and size.
pub const WINDOW: &str = "money_tracker";
pub const LEGACY_WINDOW: &str = "money_tracker_legacy";

/// The menu's entry (Floater.Toggle): the window of either look comes back
/// when minimized, else it opens or closes.
pub fn toggle(ctx: &egui::Context, open: &mut bool) {
    let minimized = [WINDOW, LEGACY_WINDOW]
        .into_iter()
        .find(|id| *open && crate::ui::widgets::floater_minimized(ctx, id));
    crate::ui::widgets::toggle_floater(ctx, minimized.unwrap_or(WINDOW), open);
}

/// MAX_LOOKBACK and POP_FRONT_SIZE of process_money_balance_reply's
/// recent transactions.
const MAX_LOOKBACK: usize = 30;
const POP_FRONT: usize = 12;

/// The transaction types named in the list (lltransactiontypes.h).
mod trans {
    pub const GROUP_CREATE: i32 = 1002;
    pub const GROUP_JOIN: i32 = 1004;
    pub const UPLOAD_CHARGE: i32 = 1101;
    pub const CLASSIFIED_CHARGE: i32 = 1103;
    pub const OBJECT_SALE: i32 = 5000;
    pub const GIFT: i32 = 5001;
    pub const LAND_SALE: i32 = 5002;
    pub const LAND_PASS_SALE: i32 = 5006;
    pub const PAY_OBJECT: i32 = 5008;
    pub const OBJECT_PAYS: i32 = 5009;
    pub const GROUP_LAND_DEED: i32 = 6001;
}

/// Who the L$ came from or went to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Party {
    Resident(Uuid),
    Group(Uuid),
    /// No one: a fee paid to the system (an upload, a group's creation).
    System,
}

/// One transaction of the list.
#[derive(Clone, Debug, PartialEq)]
pub struct Payment {
    /// Order of arrival, which also names the row.
    pub serial: u64,
    pub time: SystemTime,
    pub party: Party,
    pub received: bool,
    pub amount: i32,
    /// TransactionInfo's TransactionType.
    pub kind: i32,
    /// TransactionInfo's ItemDescription.
    pub item: String,
}

impl Payment {
    /// The amount of the Legacy Viewer's column: negative when paid.
    pub fn signed(&self) -> i64 {
        if self.received {
            i64::from(self.amount)
        } else {
            -i64::from(self.amount)
        }
    }

    /// What the transaction was, from its type (reason_from_transaction_type)
    /// and its item: "Purchase · Steel helmet", "Gift · “Thanks!”".
    pub fn detail(&self) -> String {
        let kind = kind_label(self.kind, self.received);
        let item = self.item.trim();
        // the simulator says "Payment" for a gift without a message
        let item = if self.kind == trans::GIFT && item == "Payment" { "" } else { item };
        match (self.kind, item.is_empty()) {
            (_, true) => kind.to_owned(),
            (trans::GIFT, false) => format!("{kind} · {}", aurora_i18n::tr!("money-tracker-quoted", text = item)),
            (_, false) => format!("{kind} · {item}"),
        }
    }
}

/// The name of a type of transaction, as the list shows it.
fn kind_label(kind: i32, received: bool) -> &'static str {
    match kind {
        trans::OBJECT_SALE if received => aurora_i18n::tr!("money-tracker-kind-sale"),
        trans::OBJECT_SALE => aurora_i18n::tr!("money-tracker-kind-purchase"),
        trans::GIFT => aurora_i18n::tr!("money-tracker-kind-gift"),
        trans::PAY_OBJECT => aurora_i18n::tr!("money-tracker-kind-pay-object"),
        trans::OBJECT_PAYS => aurora_i18n::tr!("money-tracker-kind-object-pays"),
        trans::LAND_SALE => aurora_i18n::tr!("money-tracker-kind-land"),
        trans::LAND_PASS_SALE => aurora_i18n::tr!("money-tracker-kind-land-pass"),
        trans::GROUP_LAND_DEED => aurora_i18n::tr!("money-tracker-kind-land-deed"),
        trans::GROUP_CREATE => aurora_i18n::tr!("money-tracker-kind-group-create"),
        trans::GROUP_JOIN => aurora_i18n::tr!("money-tracker-kind-group-join"),
        trans::UPLOAD_CHARGE => aurora_i18n::tr!("money-tracker-kind-upload"),
        trans::CLASSIFIED_CHARGE => aurora_i18n::tr!("money-tracker-kind-classified"),
        _ => aurora_i18n::tr!("money-tracker-kind-other"),
    }
}

/// The recorded transactions, oldest first.
#[derive(Default)]
pub struct Tracker {
    payments: Vec<Payment>,
    /// When the list started (the window opened, the session began while it
    /// tracks when closed, or the list was cleared).
    since: Option<SystemTime>,
    recent: VecDeque<Uuid>,
    next: u64,
}

impl Tracker {
    /// A MoneyTransaction: recorded when it succeeded and names someone
    /// (FSMoneyTracker::addPayment from process_money_balance_reply).
    /// Returns who it names, for the name lookup.
    pub fn record(&mut self, ev: &NetEvent, agent: Uuid, time: SystemTime) -> Option<Party> {
        let NetEvent::MoneyTransaction {
            id,
            success,
            kind,
            source,
            source_group,
            dest,
            dest_group,
            amount,
            item,
            ..
        } = ev
        else {
            return None;
        };
        if !id.is_nil() {
            if self.recent.contains(id) {
                return None;
            }
            if self.recent.len() > MAX_LOOKBACK {
                self.recent.drain(..POP_FRONT);
            }
            self.recent.push_back(*id);
        }
        // a pure balance update, or no L$ moved (giving an object to someone
        // sends a 0 L$ transaction, issue #672)
        if !success || (source.is_nil() && dest.is_nil()) || *amount <= 0 {
            return None;
        }
        let received = *source != agent;
        let (who, group) = if received { (*source, *source_group) } else { (*dest, *dest_group) };
        let party = match (who.is_nil(), group) {
            (true, _) => Party::System,
            (false, true) => Party::Group(who),
            (false, false) => Party::Resident(who),
        };
        self.push(time, party, received, *amount, *kind, item);
        Some(party)
    }

    /// Add a transaction at the end of the list.
    pub fn push(&mut self, time: SystemTime, party: Party, received: bool, amount: i32, kind: i32, item: &str) {
        self.since.get_or_insert(time);
        self.payments.push(Payment {
            serial: self.next,
            time,
            party,
            received,
            amount: amount.max(0),
            kind,
            item: item.to_owned(),
        });
        self.next += 1;
    }

    pub fn payments(&self) -> &[Payment] {
        &self.payments
    }

    /// When the list started, once it did.
    pub fn since(&self) -> Option<SystemTime> {
        self.since
    }

    /// The list starts now, unless it already has.
    pub fn start(&mut self, now: SystemTime) {
        self.since.get_or_insert(now);
    }

    /// L$ received and paid.
    pub fn totals(&self) -> (i64, i64) {
        self.payments.iter().fold((0, 0), |(received, paid), x| {
            if x.received {
                (received + i64::from(x.amount), paid)
            } else {
                (received, paid + i64::from(x.amount))
            }
        })
    }

    /// Take these rows out of the list; returns them (for an undo).
    pub fn remove(&mut self, serials: &BTreeSet<u64>) -> Vec<Payment> {
        let (out, keep) = std::mem::take(&mut self.payments)
            .into_iter()
            .partition(|x| serials.contains(&x.serial));
        self.payments = keep;
        out
    }

    /// Empty the list (FSMoneyTracker::clear); it starts again with the
    /// next transaction or when tracking resumes. Returns what it held.
    pub fn clear(&mut self) -> (Vec<Payment>, Option<SystemTime>) {
        (std::mem::take(&mut self.payments), self.since.take())
    }

    /// Put rows taken out back in their place.
    pub fn restore(&mut self, payments: Vec<Payment>, since: Option<SystemTime>) {
        self.payments.extend(payments);
        self.payments.sort_by_key(|x| x.serial);
        if since.is_some() {
            self.since = since;
        }
    }
}

/// The name of a party in the list: the resident's display name, the
/// group's name, or what was paid for when it went to the system.
pub fn party_name(world: &World, payment: &Payment) -> String {
    match payment.party {
        Party::Resident(id) => world.social.display_name_of(&id),
        Party::Group(id) => crate::ui::chat::app_link_group_name(world, id),
        Party::System => kind_label(payment.kind, payment.received).to_owned(),
    }
}

/// Time of a row in Second Life time with its seconds, as the Legacy
/// Viewer's column ([hour]:[min]:[sec]).
pub fn hhmmss(time: SystemTime) -> String {
    let seconds = time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() % 60);
    format!("{}:{seconds:02}", crate::ui::chat::hhmm(time))
}

/// The selected rows as the Legacy Viewer copies them: one
/// "time;name;amount" line each, in the list's order.
pub fn csv(payments: &[&Payment], name: impl Fn(&Payment) -> String) -> String {
    payments
        .iter()
        .map(|x| format!("{};{};{}", hhmmss(x.time), name(x), x.signed()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Which rows the Aurora look shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Received,
    Paid,
}

impl Filter {
    fn keeps(self, x: &Payment) -> bool {
        match self {
            Filter::All => true,
            Filter::Received => x.received,
            Filter::Paid => !x.received,
        }
    }
}

/// What the window asks of the application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackerAction {
    Profile(Uuid),
    GroupProfile(Uuid),
    Pay {
        id: Uuid,
        group: bool,
    },
    /// "Track when closed" changed.
    AlwaysTrack(bool),
}

/// Rows taken out, which "Undo" puts back while it shows.
struct Undo {
    payments: Vec<Payment>,
    since: Option<SystemTime>,
    /// When the bar appeared (egui time).
    shown: f64,
}

/// The window, in either look, and its list.
#[derive(Default)]
pub struct MoneyTrackerUi {
    pub tracker: Tracker,
    filter: Filter,
    selected: BTreeSet<u64>,
    /// The row a Shift+click selects from.
    anchor: Option<u64>,
    undo: Option<Undo>,
    /// The Legacy look's sort: its column, and whether it goes down.
    legacy_sort: (legacy::Column, bool),
    /// Shown at the last frame (to notice a close from elsewhere).
    was_open: bool,
    /// Profile pictures of the residents shown.
    pub wanted_pics: HashSet<Uuid>,
}

impl MoneyTrackerUi {
    /// Show the window in the look chosen while `open`; closing it clears
    /// the list unless it tracks when closed (FSMoneyTracker::onClose).
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &World,
        pics: &HashMap<Uuid, egui::TextureHandle>,
        open: &mut bool,
        always: bool,
        legacy: bool,
    ) -> Vec<TrackerAction> {
        let mut actions = Vec::new();
        // closed by its button, the menu or the toolbar
        if !*open {
            if std::mem::take(&mut self.was_open) {
                self.closed(always);
            }
            return actions;
        }
        self.was_open = true;
        if legacy {
            self.show_legacy(ctx, p, world, open, always, &mut actions);
        } else {
            self.show_aurora(ctx, p, world, pics, open, always, &mut actions);
        }
        if !*open {
            self.was_open = false;
            self.closed(always);
        }
        actions
    }

    /// The window was closed.
    pub fn closed(&mut self, always: bool) {
        if !always {
            self.tracker.clear();
        }
        self.selected.clear();
        self.anchor = None;
        self.undo = None;
    }

    /// A click on a row: alone, with Ctrl added or taken out, with Shift the
    /// rows from the last one clicked (`order`: the rows as shown).
    fn click(&mut self, serial: u64, order: &[u64], ctrl: bool, shift: bool) {
        match (shift, self.anchor) {
            (true, Some(anchor)) => {
                let at = |s: u64| order.iter().position(|&x| x == s);
                if let (Some(a), Some(b)) = (at(anchor), at(serial)) {
                    if !ctrl {
                        self.selected.clear();
                    }
                    self.selected.extend(&order[a.min(b)..=a.max(b)]);
                    return;
                }
                self.selected = BTreeSet::from([serial]);
            }
            _ if ctrl => {
                if !self.selected.remove(&serial) {
                    self.selected.insert(serial);
                }
            }
            _ => self.selected = BTreeSet::from([serial]),
        }
        self.anchor = Some(serial);
    }

    /// The selected rows that are still listed, in the list's order.
    fn selection(&self) -> Vec<&Payment> {
        self.tracker
            .payments()
            .iter()
            .filter(|x| self.selected.contains(&x.serial))
            .collect()
    }

    /// Take the selected rows out of the list ("Delete").
    fn delete_selected(&mut self, now: f64) {
        let gone = self.tracker.remove(&self.selected);
        self.selected.clear();
        self.anchor = None;
        if !gone.is_empty() {
            self.undo = Some(Undo {
                payments: gone,
                since: None,
                shown: now,
            });
        }
    }

    /// Empty the list ("Clear").
    fn clear_all(&mut self, now: f64) {
        let (gone, since) = self.tracker.clear();
        self.selected.clear();
        self.anchor = None;
        self.undo = (!gone.is_empty()).then_some(Undo {
            payments: gone,
            since,
            shown: now,
        });
    }

    fn undo(&mut self) {
        if let Some(undo) = self.undo.take() {
            self.tracker.restore(undo.payments, undo.since);
        }
    }

    /// AURORA_DEMO_MONEY_TRACKER: a session of payments, the most recent
    /// one a few seconds old.
    pub fn demo(&mut self, rows: &[(u64, Party, bool, i32, i32, &str)]) {
        let now = SystemTime::now();
        let first = rows.first().map_or(0, |r| r.0);
        self.tracker.start(now - std::time::Duration::from_secs(first + 120));
        for &(ago, party, received, amount, kind, item) in rows {
            self.tracker
                .push(now - std::time::Duration::from_secs(ago), party, received, amount, kind, item);
        }
    }

    /// Select rows (the demo's menu and delete scenarios).
    pub fn demo_select(&mut self, from_newest: &[usize]) {
        let all = self.tracker.payments();
        self.selected = from_newest
            .iter()
            .filter_map(|&i| all.len().checked_sub(i + 1).map(|at| all[at].serial))
            .collect();
    }

    /// The demo's undo bar: the selected rows taken out, the bar kept until
    /// used (a capture comes long after).
    pub fn demo_delete(&mut self) {
        self.delete_selected(f64::INFINITY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const ME: Uuid = Uuid::from_u128(1);
    const NOVA: Uuid = Uuid::from_u128(2);
    const GROUP: Uuid = Uuid::from_u128(3);

    fn tx(id: u128, success: bool, kind: i32, source: Uuid, dest: Uuid, amount: i32, item: &str) -> NetEvent {
        NetEvent::MoneyTransaction {
            id: Uuid::from_u128(id),
            success,
            kind,
            source,
            source_group: source == GROUP,
            dest,
            dest_group: dest == GROUP,
            amount,
            item: item.into(),
            description: String::new(),
        }
    }

    fn at(s: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_790_000_000 + s)
    }

    #[test]
    fn received_and_paid_name_the_other_party() {
        let mut t = Tracker::default();
        assert_eq!(
            t.record(&tx(1, true, trans::GIFT, NOVA, ME, 50, "Payment"), ME, at(0)),
            Some(Party::Resident(NOVA))
        );
        assert_eq!(
            t.record(&tx(2, true, trans::GROUP_JOIN, ME, GROUP, 150, ""), ME, at(1)),
            Some(Party::Group(GROUP))
        );
        assert_eq!(
            t.record(&tx(3, true, trans::UPLOAD_CHARGE, ME, Uuid::nil(), 10, ""), ME, at(2)),
            Some(Party::System)
        );
        let rows = t.payments();
        assert!(rows[0].received && !rows[1].received && !rows[2].received);
        assert_eq!(t.totals(), (50, 160));
        assert_eq!(rows.iter().map(Payment::signed).collect::<Vec<_>>(), [50, -150, -10]);
        assert_eq!(t.since(), Some(at(0)));
    }

    #[test]
    fn failures_balance_updates_and_repeats_are_not_listed() {
        let mut t = Tracker::default();
        assert_eq!(t.record(&tx(1, false, trans::GIFT, ME, NOVA, 50, ""), ME, at(0)), None);
        assert_eq!(
            t.record(&tx(2, true, trans::GIFT, Uuid::nil(), Uuid::nil(), 0, ""), ME, at(0)),
            None
        );
        // an object given to someone: no L$ moved
        assert_eq!(t.record(&tx(4, true, trans::GIFT, ME, NOVA, 0, ""), ME, at(0)), None);
        assert!(t.record(&tx(3, true, trans::GIFT, NOVA, ME, 20, ""), ME, at(0)).is_some());
        assert_eq!(t.record(&tx(3, true, trans::GIFT, NOVA, ME, 20, ""), ME, at(1)), None);
        assert_eq!(t.payments().len(), 1);
        // the window of recent ids is cut, an old one counts again
        for id in 10..60 {
            t.record(&tx(id, true, trans::GIFT, NOVA, ME, 1, ""), ME, at(2));
        }
        assert!(t.record(&tx(3, true, trans::GIFT, NOVA, ME, 20, ""), ME, at(3)).is_some());
    }

    #[test]
    fn the_detail_names_the_type_and_the_item() {
        let mut t = Tracker::default();
        t.push(at(0), Party::Resident(NOVA), true, 75, trans::GIFT, "Payment");
        t.push(at(0), Party::Resident(NOVA), false, 450, trans::OBJECT_SALE, "Steel helmet");
        t.push(at(0), Party::Resident(NOVA), true, 500, trans::OBJECT_SALE, "Lantern");
        let d: Vec<String> = t.payments().iter().map(Payment::detail).collect();
        assert_eq!(d[0], aurora_i18n::tr!("money-tracker-kind-gift"));
        assert!(d[1].starts_with(aurora_i18n::tr!("money-tracker-kind-purchase")) && d[1].ends_with("Steel helmet"));
        assert!(d[2].starts_with(aurora_i18n::tr!("money-tracker-kind-sale")));
    }

    #[test]
    fn delete_clear_and_undo_keep_the_order() {
        let mut ui = MoneyTrackerUi::default();
        for (i, amount) in [10, 20, 30, 40].into_iter().enumerate() {
            ui.tracker.push(at(i as u64), Party::Resident(NOVA), true, amount, trans::GIFT, "");
        }
        let order: Vec<u64> = ui.tracker.payments().iter().rev().map(|x| x.serial).collect();
        ui.click(order[0], &order, false, false);
        ui.click(order[2], &order, false, true);
        assert_eq!(ui.selection().len(), 3);
        ui.click(order[1], &order, true, false);
        assert_eq!(ui.selection().iter().map(|x| x.amount).collect::<Vec<_>>(), [20, 40]);
        ui.delete_selected(0.0);
        assert_eq!(ui.tracker.totals(), (40, 0));
        ui.undo();
        assert_eq!(ui.tracker.payments().iter().map(|x| x.amount).collect::<Vec<_>>(), [10, 20, 30, 40]);
        ui.clear_all(0.0);
        assert!(ui.tracker.payments().is_empty() && ui.tracker.since().is_none());
        ui.undo();
        assert_eq!((ui.tracker.payments().len(), ui.tracker.since()), (4, Some(at(0))));
    }

    #[test]
    fn closing_clears_unless_it_tracks_when_closed() {
        let mut ui = MoneyTrackerUi::default();
        ui.tracker.push(at(0), Party::System, false, 10, trans::UPLOAD_CHARGE, "");
        ui.closed(true);
        assert_eq!(ui.tracker.payments().len(), 1);
        ui.closed(false);
        assert!(ui.tracker.payments().is_empty());
    }

    /// The window keeps its size frame after frame, in both looks, at the
    /// interface scales, with its buttons inside it (a footer taller than
    /// the room kept for it made egui grow the window every frame).
    #[test]
    fn the_window_keeps_its_size_and_its_footer() {
        use crate::ui::widgets::elide_tests::Frames;
        let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let p = crate::theme::Theme::default().palette();
        let pics = HashMap::new();
        for legacy in [false, true] {
            for zoom in [1.0f32, 1.25, 1.5] {
                for size in [egui::vec2(380.0, 600.0), egui::vec2(300.0, 260.0)] {
                    let mut frames = Frames::new();
                    crate::theme::Theme::default().apply(&frames.ctx, 1.0);
                    crate::ui::fonts::apply(&frames.ctx, crate::ui::fonts::DEFAULT);
                    frames.ctx.set_zoom_factor(zoom);
                    let id = if legacy { LEGACY_WINDOW } else { WINDOW };
                    crate::ui::widgets::place_floater(&frames.ctx, id, egui::pos2(40.0, 40.0), size);
                    let mut ui = MoneyTrackerUi::default();
                    for i in 0..40 {
                        ui.tracker.push(at(i), Party::System, i % 2 == 0, 10, trans::GIFT, "");
                    }
                    let case = format!("legacy {legacy}, zoom {zoom}, {size:?}");
                    let mut rects = Vec::new();
                    let mut texts = Vec::new();
                    for _ in 0..30 {
                        texts = frames.run(Vec::new(), |egui_ui| {
                            let mut open = true;
                            ui.show(egui_ui.ctx(), &p, &world, &pics, &mut open, false, legacy);
                        });
                        rects.push(frames.ctx.memory(|m| m.area_rect(egui::Id::unique(id))).expect("window"));
                    }
                    let first = rects[2];
                    assert!(
                        rects[2..].iter().all(|r| *r == first),
                        "{case}: the window moved or grew: {:?} … {:?}",
                        first,
                        rects.last()
                    );
                    let clear = texts
                        .iter()
                        .find(|t| t.source == aurora_i18n::tr!("money-tracker-clear"))
                        .unwrap_or_else(|| panic!("{case}: Clear is drawn"));
                    assert!(
                        clear.rect.bottom() <= first.bottom(),
                        "{case}: Clear at {:?} in {first:?}",
                        clear.rect
                    );
                }
            }
        }
    }

    #[test]
    fn the_copy_is_the_legacy_viewer_csv() {
        let mut t = Tracker::default();
        t.push(at(5), Party::System, false, 10, trans::UPLOAD_CHARGE, "");
        let rows: Vec<&Payment> = t.payments().iter().collect();
        let line = csv(&rows, |x| kind_label(x.kind, x.received).to_owned());
        let fields: Vec<&str> = line.split(';').collect();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].len(), 8);
        assert_eq!(fields[1], aurora_i18n::tr!("money-tracker-kind-upload"));
        assert_eq!(fields[2], "-10");
    }
}

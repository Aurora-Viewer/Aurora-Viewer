//! L$ a resident paid us directly (TRANS_GIFT, the Pay window's payment) as
//! a card in the conversation with the payer, like their inventory offers'
//! (#1198). The Legacy Viewer says it in a PaymentReceived toast, or in
//! nearby chat with FSPaymentInfoInChat (process_money_balance_reply_extended
//! and reason_from_transaction_type, indra/newview/llviewermessage.cpp,
//! originally LGPL 2.1): same rule for what counts and for the payer's
//! message, which the simulator gives as "Payment" when there is none.
//!
//! The card is for direct payments. Every other L$ received (a sale, an
//! object paid, a parcel, a group's money) and a direct payment that shows
//! no card get a toast and a line in the notification list, with its
//! "Notify me when I receive L$" switch: the Legacy Viewer's PaymentReceived,
//! persistent and logged to the payer's transcript (log_to_im, #2675). The
//! switch is off by default (`Settings::notify_money_received`) and the
//! rules differ in three places, on purpose. A toast names the object paid
//! when the simulator gives its name, where reason_from_transaction_type
//! leaves TRANS_PAY_OBJECT without one: a seller wants to know what sold.
//! It is stricter too: only for L$ to us that succeeded and that come from
//! someone, so it never says "paid you" of a group's money or of nobody.
//!
//! The card goes into the payer's conversation when one
//! is open; a friend's payment opens it with "Open a conversation for a
//! friend's payment" (Preferences › Chat, on by default). The payment is
//! always written to the payer's transcript, with or without a card, as the
//! offers' are (LLHandlerUtil::logToIMP2P).
//!
//! Our own direct payment to a resident (the Legacy Viewer's PaymentSent,
//! you_paid_ldollars_gift) is the same the other way: its card in the
//! payee's conversation when it is open, or always with "Open the
//! conversation when you pay a resident" (off by default), and its line in
//! their transcript, so that it is still there when the conversation is
//! opened again, as a received payment is (#1198).

use super::notifications::{Data, Kind};
use super::{ChatKind, ChatLine, Origin, World};
use crate::ui::money_tracker::trans::{self, GIFT};
use std::collections::VecDeque;
use std::time::SystemTime;
use uuid::Uuid;

/// Payments kept for the conversation cards; an older card shows its text.
const MAX_PAYMENTS: usize = 100;

/// Transaction ids remembered: the simulator may send a reply twice
/// (process_money_balance_reply's MAX_LOOKBACK).
const MAX_RECENT: usize = 30;

#[derive(Debug, Clone)]
pub struct Payment {
    pub id: u64,
    /// The resident of the conversation: the payer, or the payee of ours.
    pub other: Uuid,
    /// We paid them.
    pub sent: bool,
    pub amount: i32,
    /// The payer's message, empty when they wrote none.
    pub message: String,
}

#[derive(Default)]
pub struct Payments {
    list: VecDeque<Payment>,
    next: u64,
    recent: VecDeque<Uuid>,
    /// A friend's payment opens their conversation, from the preferences.
    pub open_for_friends: bool,
    /// Our own payment opens the payee's conversation, from the preferences.
    pub open_when_paying: bool,
    /// L$ received show a toast (NotifyMoneyReceived), from the preferences.
    pub notify_received: bool,
    /// L$ received rise into the balance, from the preferences.
    pub rise_into_balance: bool,
    /// The amounts waiting for `ui/balance_gains.rs`, taken every frame.
    pub gains: Vec<i32>,
}

impl Payments {
    pub fn get(&self, id: u64) -> Option<&Payment> {
        self.list.iter().find(|v| v.id == id)
    }

    fn push(&mut self, other: Uuid, sent: bool, amount: i32, message: String) -> u64 {
        self.next += 1;
        self.list.push_back(Payment {
            id: self.next,
            other,
            sent,
            amount,
            message,
        });
        if self.list.len() > MAX_PAYMENTS {
            self.list.pop_front();
        }
        self.next
    }

    /// Whether this transaction was seen already, remembering it.
    fn seen(&mut self, transaction: Uuid) -> bool {
        if transaction.is_nil() {
            return false;
        }
        if self.recent.contains(&transaction) {
            return true;
        }
        if self.recent.len() >= MAX_RECENT {
            self.recent.pop_front();
        }
        self.recent.push_back(transaction);
        false
    }
}

/// reason_from_transaction_type for TRANS_GIFT: "Payment" is the simulator's
/// word for no message.
fn message_of(item: &str) -> &str {
    if item == "Payment" { "" } else { item.trim() }
}

/// A text the grid brings into a toast (an object's name, a stranger's
/// message), `dialog_names::shown`'s rule, and never a link: a labelled
/// `secondlife://` link could read as another sentence of the toast.
fn plain(text: &str) -> String {
    super::dialog_names::shown(text.trim()).replace("://", ": //")
}

/// reason_from_transaction_type for L$ received: what they were for, empty
/// without a reason to give. Unlike there, an object paid names the object.
fn reason_of(kind: i32, item: &str) -> String {
    let named = |item: &str| match plain(item) {
        item if item.is_empty() => String::new(),
        item => aurora_i18n::tr!("world-payments-for-item", item = item),
    };
    match kind {
        trans::OBJECT_SALE | trans::PAY_OBJECT | trans::OBJECT_PAYS => named(item),
        trans::LAND_SALE => aurora_i18n::tr!("world-payments-for-land").into(),
        trans::LAND_PASS_SALE => aurora_i18n::tr!("world-payments-for-land-pass").into(),
        trans::GROUP_LAND_DEED => aurora_i18n::tr!("world-payments-for-land-deed").into(),
        _ => String::new(),
    }
}

/// "L$1,000" as the amount reads in the card and the transcript.
pub fn amount_label(amount: i32) -> String {
    aurora_i18n::integer(i64::from(amount.max(0)))
}

/// The card's title: "Jane paid you L$75", or "You paid Jane L$75" for ours.
pub fn title(name: String, sent: bool, amount: i32) -> String {
    if sent {
        aurora_i18n::tr!("world-payments-you-paid", name = name, amount = amount_label(amount))
    } else {
        aurora_i18n::tr!("world-payments-paid-you", name = name, amount = amount_label(amount))
    }
}

/// The sentence of a payment, with the other resident as `name`.
pub fn sentence(name: String, sent: bool, amount: i32, message: &str) -> String {
    match (sent, message.is_empty()) {
        (_, true) => title(name, sent, amount),
        (false, false) => aurora_i18n::tr!(
            "world-payments-paid-you-message",
            name = name,
            amount = amount_label(amount),
            message = message.to_owned()
        ),
        (true, false) => aurora_i18n::tr!(
            "world-payments-you-paid-message",
            name = name,
            amount = amount_label(amount),
            message = message.to_owned()
        ),
    }
}

impl World {
    /// A MoneyBalanceReply naming a transaction: a direct payment between a
    /// resident and us gets its card and its line in their transcript, and
    /// L$ received their notice (process_money_balance_reply_extended).
    pub fn payment(&mut self, ev: &aurora_net::NetEvent) {
        let aurora_net::NetEvent::MoneyTransaction {
            id,
            success,
            kind,
            source,
            source_group,
            dest,
            dest_group,
            amount,
            item,
            description,
        } = ev
        else {
            return;
        };
        let sent = *source == self.agent_id;
        let other = if sent { *dest } else { *source };
        if !success
            || *amount <= 0
            || (!sent && (*dest != self.agent_id || *dest_group))
            || other.is_nil()
            || other == self.agent_id
            || self.payments.seen(*id)
        {
            return;
        }
        let card = *kind == GIFT && !*source_group && !*dest_group && self.direct_payment(other, sent, *amount, item);
        if sent {
            return;
        }
        if self.payments.rise_into_balance {
            self.payments.gains.push(*amount);
        }
        // process_money_balance_reply: nothing to display without a description
        if self.payments.notify_received && !card && !description.is_empty() {
            self.received_notice(other, *source_group, *kind, *amount, item);
        }
    }

    /// A direct payment's card and transcript line. Whether a card shows.
    fn direct_payment(&mut self, other: Uuid, sent: bool, amount: i32, item: &str) -> bool {
        if !sent && self.mutes.text_muted(&other, &self.social.name_of(&other)) {
            return false;
        }
        let message = message_of(item).to_owned();
        let text = sentence(super::teleport_invites::agent_link(other), sent, amount, &message);
        let note = self.notice_line(text.clone());
        self.transcript_only(other, &note);
        let open = self.social.has_session(&other);
        let opens = if sent {
            self.payments.open_when_paying
        } else {
            self.payments.open_for_friends && self.social.is_friend(&other)
        };
        if !open && !(opens && self.rlva_can_start_im(other)) {
            return false;
        }
        self.social.session_mut(other);
        let payment = self.payments.push(other, sent, amount, message);
        let line = ChatLine {
            time: SystemTime::now(),
            from: self.social.name_of(&other),
            text,
            kind: ChatKind::Payment(payment),
            source: other,
            origin: Origin::Live,
        };
        self.conversation_card(other, line);
        if sent {
            // our own doing: nothing new to read
            let s = self.session_lines(other);
            s.unread = s.unread.saturating_sub(1);
        }
        true
    }

    /// PaymentReceived: the toast, its entry in the notification list and
    /// its line in the payer's transcript. A muted payer, or one whose IMs
    /// RLVa holds back, gives no reason (FIRE-21803: a payment's message
    /// is not a way around them), and the transcript stays without them.
    fn received_notice(&mut self, from: Uuid, group: bool, kind: i32, amount: i32, item: &str) {
        let name = if group {
            format!("secondlife:///app/group/{from}/about")
        } else {
            self.social.want_name(from);
            super::teleport_invites::agent_link(from)
        };
        let muted = self.mutes.text_muted(&from, &self.social.name_of(&from));
        let silent = muted || !self.rlva_allows_im("recvim", from);
        let text = if silent {
            title(name, false, amount)
        } else if kind == GIFT {
            sentence(name, false, amount, &plain(message_of(item)))
        } else {
            match reason_of(kind, item) {
                reason if reason.is_empty() => title(name, false, amount),
                reason => aurora_i18n::tr!(
                    "world-payments-paid-you-reason",
                    name = name,
                    amount = amount_label(amount),
                    reason = reason
                ),
            }
        };
        // a direct payment's line went to the transcript with its card's
        if kind != GIFT && !group && !muted {
            let note = self.notice_line(text.clone());
            self.transcript_only(from, &note);
        }
        let id = self
            .notifications
            .push(Kind::Info, aurora_i18n::tr!("world-payments-received-title"), text, Data::None);
        let notice = self.notice_of(id, super::attention::Open::Notifications);
        self.ask_attention(super::attention::Cause::Notice, notice);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::NetEvent;

    const AGENT: Uuid = Uuid::from_u128(1);
    const JANE: Uuid = Uuid::from_u128(2);

    fn world() -> World {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.agent_id = AGENT;
        w.payments.open_for_friends = true;
        w
    }

    fn paid(id: u128, kind: i32, source: Uuid, dest: Uuid, amount: i32, item: &str) -> NetEvent {
        NetEvent::MoneyTransaction {
            id: Uuid::from_u128(id),
            success: true,
            kind,
            source,
            source_group: false,
            dest,
            dest_group: false,
            amount,
            item: item.into(),
            description: String::new(),
        }
    }

    fn cards(w: &World, with: Uuid) -> Vec<Payment> {
        w.social
            .ims
            .iter()
            .filter(|s| s.other == with)
            .flat_map(|s| s.lines.iter())
            .filter_map(|l| match l.kind {
                ChatKind::Payment(id) => w.payments.get(id).cloned(),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_payment_shows_in_the_open_conversation_only() {
        let mut w = world();
        w.payment(&paid(10, GIFT, JANE, AGENT, 75, "Thanks for the set!"));
        assert!(w.social.ims.is_empty(), "no conversation opened for a stranger");
        w.social.session_mut(JANE);
        w.payment(&paid(11, GIFT, JANE, AGENT, 1200, "Payment"));
        let got = cards(&w, JANE);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].amount, got[0].message.as_str()), (1200, ""));
        assert_eq!(w.social.ims[0].unread, 1);
    }

    #[test]
    fn a_friend_s_payment_opens_the_conversation_when_asked() {
        for open in [true, false] {
            let mut w = world();
            w.payments.open_for_friends = open;
            w.social.form_friendship(JANE);
            w.payment(&paid(10, GIFT, JANE, AGENT, 50, "Rent"));
            assert_eq!(cards(&w, JANE).len(), usize::from(open), "{open}");
        }
    }

    #[test]
    fn only_direct_payments_to_us_once() {
        let mut w = world();
        w.social.session_mut(JANE);
        // an object paid, a sale, a failure: none
        w.payment(&paid(1, 5008, JANE, AGENT, 10, "Tip jar"));
        w.payment(&paid(2, 5000, JANE, AGENT, 10, "Lamp"));
        let mut failed = paid(4, GIFT, JANE, AGENT, 10, "Payment");
        if let NetEvent::MoneyTransaction { success, .. } = &mut failed {
            *success = false;
        }
        w.payment(&failed);
        assert!(cards(&w, JANE).is_empty());
        // the simulator's reply twice: one card
        w.payment(&paid(5, GIFT, JANE, AGENT, 10, "Payment"));
        w.payment(&paid(5, GIFT, JANE, AGENT, 10, "Payment"));
        assert_eq!(cards(&w, JANE).len(), 1);
    }

    #[test]
    fn the_sentence_names_the_message_when_there_is_one() {
        assert_eq!(message_of("Payment"), "");
        let plain = sentence("Jane".into(), false, 1000, "");
        let with = sentence("Jane".into(), false, 75, "Thanks");
        assert!(plain.contains("Jane") && plain.contains("1"), "{plain}");
        assert!(with.contains("75") && with.ends_with("Thanks"), "{with}");
        let ours = sentence("Jane".into(), true, 75, "Thanks");
        assert!(ours != with && ours.contains("Jane") && ours.ends_with("Thanks"), "{ours}");
    }

    #[test]
    fn our_payment_opens_the_conversation_when_asked() {
        for open in [true, false] {
            let mut w = world();
            w.payments.open_when_paying = open;
            w.payment(&paid(30, GIFT, AGENT, JANE, 120, "Payment"));
            assert_eq!(cards(&w, JANE).len(), usize::from(open), "{open}");
            if open {
                assert_eq!(w.social.ims[0].unread, 0, "our own doing is not unread");
            }
        }
        // a received payment from a stranger still opens nothing
        let mut w = world();
        w.payments.open_when_paying = true;
        w.payment(&paid(31, GIFT, JANE, AGENT, 120, "Payment"));
        assert!(w.social.ims.is_empty());
    }

    #[test]
    fn our_payment_shows_in_the_open_conversation_without_opening_one() {
        let mut w = world();
        w.social.form_friendship(JANE);
        // no conversation: not opened, even for a friend (we just paid them)
        w.payment(&paid(20, GIFT, AGENT, JANE, 300, "Rent"));
        assert!(w.social.ims.is_empty());
        w.social.session_mut(JANE);
        w.payment(&paid(21, GIFT, AGENT, JANE, 40, "Payment"));
        let got = cards(&w, JANE);
        assert_eq!(got.len(), 1);
        assert!(got[0].sent && got[0].other == JANE && got[0].amount == 40);
        assert_eq!(w.social.ims[0].unread, 0, "our own doing is not unread");
        // to a group, or failed: none
        let mut group = paid(22, GIFT, AGENT, JANE, 10, "Payment");
        if let NetEvent::MoneyTransaction { dest_group, .. } = &mut group {
            *dest_group = true;
        }
        w.payment(&group);
        assert_eq!(cards(&w, JANE).len(), 1);
    }

    /// A transaction as the simulator gives it, with its description.
    fn received(id: u128, kind: i32, source: Uuid, amount: i32, item: &str) -> NetEvent {
        match paid(id, kind, source, AGENT, amount, item) {
            NetEvent::MoneyTransaction {
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
            } => NetEvent::MoneyTransaction {
                id,
                success,
                kind,
                source,
                source_group,
                dest,
                dest_group,
                amount,
                item,
                description: "Transaction".into(),
            },
            other => other,
        }
    }

    fn noticing() -> World {
        let mut w = world();
        w.payments.notify_received = true;
        w.social.names.insert(JANE, "Jane Doe".into());
        w
    }

    /// The notices of the list, their links not resolved.
    fn notices(w: &World) -> Vec<String> {
        w.notifications.listed().map(|n| n.body.clone()).collect()
    }

    fn from_jane(text: &str) -> String {
        format!("secondlife:///app/agent/{JANE}/completename{text}")
    }

    #[test]
    fn a_sale_and_an_object_paid_notify_with_what_was_bought() {
        let mut w = noticing();
        w.payment(&received(40, trans::OBJECT_SALE, JANE, 100, "Steel helmet"));
        w.payment(&received(41, trans::PAY_OBJECT, JANE, 25, "Tip jar"));
        w.payment(&received(42, trans::OBJECT_PAYS, JANE, 5, ""));
        assert_eq!(
            notices(&w),
            [
                from_jane(" paid you L$100 for Steel helmet"),
                from_jane(" paid you L$25 for Tip jar"),
                from_jane(" paid you L$5"),
            ]
        );
    }

    #[test]
    fn land_gives_the_legacy_reasons() {
        let mut w = noticing();
        w.payment(&received(50, trans::LAND_SALE, JANE, 10, "ignored"));
        w.payment(&received(51, trans::LAND_PASS_SALE, JANE, 11, "ignored"));
        w.payment(&received(52, trans::GROUP_LAND_DEED, JANE, 12, "ignored"));
        assert_eq!(
            notices(&w),
            [
                from_jane(" paid you L$10 for a parcel of land"),
                from_jane(" paid you L$11 for a land access pass"),
                from_jane(" paid you L$12 for deeding land"),
            ]
        );
    }

    #[test]
    fn a_gift_notifies_only_without_a_card() {
        let mut w = noticing();
        w.payment(&received(60, GIFT, JANE, 75, "Thanks!"));
        w.payment(&received(61, GIFT, JANE, 20, "Payment"));
        assert_eq!(notices(&w), [from_jane(" paid you L$75: Thanks!"), from_jane(" paid you L$20")]);
        assert!(w.social.ims.is_empty());
        // the conversation is open: its card says it
        w.social.session_mut(JANE);
        w.payment(&received(62, GIFT, JANE, 30, "Payment"));
        assert_eq!(cards(&w, JANE).len(), 1);
        assert_eq!(notices(&w).len(), 2);
    }

    #[test]
    fn a_group_can_pay_and_gets_a_group_link() {
        const GROUP: Uuid = Uuid::from_u128(3);
        let mut w = noticing();
        let mut ev = received(70, GIFT, GROUP, 40, "Dividend");
        if let NetEvent::MoneyTransaction { source_group, .. } = &mut ev {
            *source_group = true;
        }
        w.payment(&ev);
        assert_eq!(
            notices(&w),
            [format!("secondlife:///app/group/{GROUP}/about paid you L$40: Dividend")]
        );
    }

    #[test]
    fn nothing_for_ours_a_failure_a_duplicate_or_an_empty_description() {
        let mut w = noticing();
        w.payments.rise_into_balance = true;
        // ours
        w.payment(&received(80, trans::OBJECT_SALE, AGENT, 10, "Hat"));
        let mut ours = paid(81, trans::OBJECT_SALE, AGENT, JANE, 10, "Hat");
        if let NetEvent::MoneyTransaction { description, .. } = &mut ours {
            *description = "Transaction".into();
        }
        w.payment(&ours);
        // failed
        let mut failed = received(82, trans::OBJECT_SALE, JANE, 10, "Hat");
        if let NetEvent::MoneyTransaction { success, .. } = &mut failed {
            *success = false;
        }
        w.payment(&failed);
        // to someone else, to a group, from no one
        let mut other = received(83, trans::OBJECT_SALE, JANE, 10, "Hat");
        if let NetEvent::MoneyTransaction { dest, .. } = &mut other {
            *dest = Uuid::from_u128(9);
        }
        w.payment(&other);
        let mut group = received(84, trans::OBJECT_SALE, JANE, 10, "Hat");
        if let NetEvent::MoneyTransaction { dest_group, .. } = &mut group {
            *dest_group = true;
        }
        w.payment(&group);
        w.payment(&received(85, trans::OBJECT_SALE, Uuid::nil(), 10, "Hat"));
        assert!(notices(&w).is_empty() && w.payments.gains.is_empty());
        // the simulator sends a reply twice
        w.payment(&received(86, trans::OBJECT_SALE, JANE, 10, "Hat"));
        w.payment(&received(86, trans::OBJECT_SALE, JANE, 10, "Hat"));
        assert_eq!(notices(&w).len(), 1);
        assert_eq!(w.payments.gains, [10]);
        // process_money_balance_reply: no description, nothing to display
        w.payment(&paid(87, trans::OBJECT_SALE, JANE, AGENT, 10, "Hat"));
        assert_eq!(notices(&w).len(), 1);
    }

    #[test]
    fn a_muted_payer_gives_no_reason() {
        let mut w = noticing();
        w.mutes.reset(AGENT);
        w.block(JANE, "Jane Doe", crate::world::mutes::MuteType::Agent).expect("blocked");
        w.payment(&received(90, trans::OBJECT_SALE, JANE, 100, "Spam for you"));
        w.payment(&received(91, GIFT, JANE, 20, "Hello again"));
        assert_eq!(notices(&w), [from_jane(" paid you L$100"), from_jane(" paid you L$20")]);
    }

    #[test]
    fn rlva_held_ims_give_no_reason_but_keep_the_notice() {
        let mut w = noticing();
        w.rlva.configure(true);
        w.rlva_console_input("@recvim=n");
        w.payment(&received(95, GIFT, JANE, 20, "A way around it"));
        w.payment(&received(96, trans::OBJECT_SALE, JANE, 100, "Hat"));
        assert_eq!(notices(&w), [from_jane(" paid you L$20"), from_jane(" paid you L$100")]);
    }

    #[test]
    fn a_name_from_the_grid_cannot_bring_a_link_or_break_the_line() {
        let mut w = noticing();
        let link = format!("[secondlife:///app/agent/{AGENT}/about Bob paid you L$9999]");
        w.payment(&received(100, GIFT, JANE, 1, &link));
        w.payment(&received(101, trans::PAY_OBJECT, JANE, 2, "Box\nsecond line"));
        let got = notices(&w);
        assert!(
            !got[0].contains("secondlife:///app/agent/00000000-0000-0000-0000-000000000001"),
            "{}",
            got[0]
        );
        assert!(got[1].ends_with("for Box second line"), "{}", got[1]);
    }

    #[test]
    fn by_default_a_payment_is_highlighted_without_a_toast() {
        let settings = crate::settings::Settings::default();
        assert!(!settings.notify_money_received);
        assert!(settings.balance_gain_animation);
    }

    #[test]
    fn each_switch_works_alone() {
        for (toast, rise) in [(true, true), (true, false), (false, true), (false, false)] {
            let mut w = world();
            w.payments.notify_received = toast;
            w.payments.rise_into_balance = rise;
            w.payment(&received(110, trans::OBJECT_SALE, JANE, 100, "Hat"));
            assert_eq!(notices(&w).len(), usize::from(toast), "{toast} {rise}");
            assert_eq!(w.payments.gains.len(), usize::from(rise), "{toast} {rise}");
        }
    }

    #[test]
    fn a_gift_shown_as_a_card_still_rises_into_the_balance() {
        let mut w = noticing();
        w.payments.rise_into_balance = true;
        w.social.session_mut(JANE);
        w.payment(&received(120, GIFT, JANE, 30, "Payment"));
        assert_eq!(w.payments.gains, [30]);
        assert!(notices(&w).is_empty());
    }
}

//! Port of LLAttachmentsMgr::requestAttachments
//! (indra/newview/llattachmentsmgr.cpp, originally LGPL 2.1): attachment
//! requests leave one object per RezMultipleAttachmentsFromInv, never as a
//! burst. "Requesting many attachments at once causes frequent server-side
//! failures" (one per idle pass since FIRE-6070).
//!
//! One per pass is not enough on a busy region: on 2026-10-27, 18 requests
//! 20 ms apart, after the region had put 14 objects back by itself, got the
//! first 14 worn and the last 4 refused ("Attempt to rez an object failed");
//! the refused items then stayed off for the session, whatever asked for
//! them again. So at most `MAX_IN_FLIGHT` requests wait for their object at
//! a time (until an object worn from the item arrives, or
//! `IN_FLIGHT_TIMEOUT`), and a refusal pauses the queue for `PAUSE`. The
//! Legacy Viewer has neither and loses objects the same way now and then.

use super::*;
use std::collections::VecDeque;

/// Shortest gap between two requests.
const SPACING: Duration = Duration::from_millis(100);
/// Requests waiting for their object at once.
const MAX_IN_FLIGHT: usize = 4;
/// A request whose object has not arrived by then frees its place.
const IN_FLIGHT_TIMEOUT: Duration = Duration::from_secs(10);
/// No request leaves for this long after the simulator refused one.
const PAUSE: Duration = Duration::from_secs(5);
/// The simulator's refusal (AlertMessage).
const REZ_FAILED: &str = "Attempt to rez an object failed";

#[derive(Default)]
pub(super) struct AttachQueue {
    pending: VecDeque<AttachRequest>,
    /// Items requested whose object has not arrived yet, and when.
    in_flight: HashMap<Uuid, Instant>,
    last_sent: Option<Instant>,
    paused_until: Option<Instant>,
}

impl AttachQueue {
    /// Queues `list`; a request for an item still waiting takes its place
    /// (the latest point and add mode win).
    pub(super) fn push(&mut self, list: Vec<AttachRequest>) {
        for request in list {
            match self.pending.iter_mut().find(|p| p.item_id == request.item_id) {
                Some(slot) => *slot = request,
                None => self.pending.push_back(request),
            }
        }
    }

    /// The next request to send now, if its turn has come.
    pub(super) fn next(&mut self, now: Instant) -> Option<AttachRequest> {
        self.in_flight
            .retain(|_, at| now.saturating_duration_since(*at) < IN_FLIGHT_TIMEOUT);
        if self.pending.is_empty()
            || self.in_flight.len() >= MAX_IN_FLIGHT
            || self.paused_until.is_some_and(|until| now < until)
            || self.last_sent.is_some_and(|at| now.saturating_duration_since(at) < SPACING)
        {
            return None;
        }
        let request = self.pending.pop_front()?;
        self.last_sent = Some(now);
        self.in_flight.insert(request.item_id, now);
        Some(request)
    }

    /// Objects arrived: one worn from a requested item frees its place.
    pub(super) fn observe<'a>(&mut self, name_values: impl Iterator<Item = &'a str>) {
        if self.in_flight.is_empty() {
            return;
        }
        for nv in name_values {
            if let Some(item) = attach_item_id(nv) {
                self.in_flight.remove(&item);
            }
        }
    }

    /// An alert from the simulator: a refused request pauses the queue.
    pub(super) fn on_alert(&mut self, message: &str, now: Instant) {
        if message.contains(REZ_FAILED) && !(self.pending.is_empty() && self.in_flight.is_empty()) {
            log::info!(
                "attachments: the simulator refused one, {} waiting resume in {} s",
                self.pending.len(),
                PAUSE.as_secs()
            );
            self.paused_until = Some(now + PAUSE);
        }
    }

    pub(super) fn len(&self) -> usize {
        self.pending.len()
    }
}

/// The item an object is worn from (its `AttachItemID` name-value).
fn attach_item_id(name_values: &str) -> Option<Uuid> {
    name_values
        .lines()
        .find(|l| l.starts_with("AttachItemID"))
        .and_then(|l| l.split_whitespace().last())
        .and_then(|id| id.parse().ok())
}

impl Session<'_> {
    /// Sends the next queued attachment request when its turn has come.
    pub(super) fn send_queued_attachment(&mut self, now: Instant) {
        let Some(a) = self.attach_queue.next(now) else {
            return;
        };
        let mut m = RezMultipleAttachmentsFromInv::default();
        m.agent_data.agent_id = self.agent_id();
        m.agent_data.session_id = self.session_id();
        m.header_data.compound_msg_id = uuid::Uuid::new_v4();
        m.header_data.total_objects = 1;
        m.header_data.first_detach_all = false;
        m.object_data = vec![rez_multiple_attachments_from_inv::ObjectData {
            item_id: a.item_id,
            owner_id: a.owner_id,
            attachment_pt: a.point | if a.add { 0x80 } else { 0 },
            item_flags: a.flags,
            group_mask: a.group_mask,
            everyone_mask: a.everyone_mask,
            next_owner_mask: a.next_owner_mask,
            name: str_field(&a.name),
            description: str_field(&a.desc),
        }];
        self.send_main(&m, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: u128, point: u8) -> AttachRequest {
        AttachRequest {
            item_id: Uuid::from_u128(id),
            owner_id: Uuid::nil(),
            point,
            add: true,
            flags: 0,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            name: String::new(),
            desc: String::new(),
        }
    }

    fn worn(id: u128) -> String {
        format!("AttachItemID STRING RW SV {}", Uuid::from_u128(id))
    }

    #[test]
    fn requests_leave_spaced_and_a_few_at_a_time() {
        let mut q = AttachQueue::default();
        let t0 = Instant::now();
        q.push((1..=6).map(|id| request(id, 0)).collect());
        assert_eq!(q.next(t0).map(|a| a.item_id.as_u128()), Some(1));
        assert!(q.next(t0 + SPACING / 2).is_none(), "not before the gap");
        let mut at = t0;
        for _ in 0..10 {
            at += SPACING;
            let _ = q.next(at);
        }
        assert_eq!(q.len(), 2, "four wait for their object");
        // an object worn from item 2 arrives: its place is free
        q.observe([worn(2)].iter().map(String::as_str));
        assert_eq!(q.next(at + SPACING).map(|a| a.item_id.as_u128()), Some(5));
        // the others free theirs after the timeout
        assert_eq!(q.next(t0 + IN_FLIGHT_TIMEOUT + SPACING).map(|a| a.item_id.as_u128()), Some(6));
    }

    #[test]
    fn a_refusal_pauses_the_queue() {
        let mut q = AttachQueue::default();
        let t0 = Instant::now();
        q.push(vec![request(1, 0), request(2, 0)]);
        assert!(q.next(t0).is_some());
        q.on_alert("Attempt to rez an object failed.", t0);
        assert!(q.next(t0 + PAUSE - Duration::from_millis(1)).is_none());
        assert_eq!(q.next(t0 + PAUSE).map(|a| a.item_id.as_u128()), Some(2));
        // other alerts change nothing
        q.push(vec![request(3, 0)]);
        q.on_alert("You cannot do that here.", t0 + PAUSE);
        assert!(q.next(t0 + PAUSE + SPACING).is_some());
    }

    #[test]
    fn a_waiting_item_asked_again_keeps_its_place_with_the_new_point() {
        let mut q = AttachQueue::default();
        let t0 = Instant::now();
        q.push(vec![request(1, 0), request(2, 0)]);
        q.push(vec![request(1, 35), request(3, 0)]);
        assert_eq!(q.len(), 3);
        let first = q.next(t0).expect("first");
        assert_eq!((first.item_id, first.point), (Uuid::from_u128(1), 35));
    }

    #[test]
    fn the_item_comes_from_the_attach_item_id_name_value() {
        let id = Uuid::from_u128(7);
        assert_eq!(
            attach_item_id(&format!("Title STRING RW SV hi\nAttachItemID STRING RW SV {id}")),
            Some(id)
        );
        assert_eq!(attach_item_id("Title STRING RW SV hi"), None);
    }
}

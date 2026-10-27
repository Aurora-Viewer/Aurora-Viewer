//! Port of LLAttachmentsMgr::requestAttachments
//! (indra/newview/llattachmentsmgr.cpp, originally LGPL 2.1): attachment
//! requests leave one object per RezMultipleAttachmentsFromInv, one message
//! per pass of the loop, never as a burst. "Requesting many attachments at
//! once causes frequent server-side failures" (one per idle pass since
//! FIRE-6070); a burst at login loses the objects the simulator is busy
//! restoring by itself.

use super::*;
use std::collections::VecDeque;

/// Shortest gap between two requests: about one network tick, the Legacy
/// Viewer's one per frame during a login.
const SPACING: Duration = Duration::from_millis(20);

#[derive(Default)]
pub(super) struct AttachQueue {
    pending: VecDeque<AttachRequest>,
    last_sent: Option<Instant>,
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

    /// The next request to send now, if the last one left long enough ago.
    pub(super) fn next(&mut self, now: Instant) -> Option<AttachRequest> {
        if self.last_sent.is_some_and(|at| now.saturating_duration_since(at) < SPACING) {
            return None;
        }
        let request = self.pending.pop_front()?;
        self.last_sent = Some(now);
        Some(request)
    }

    pub(super) fn len(&self) -> usize {
        self.pending.len()
    }
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

    #[test]
    fn requests_leave_one_at_a_time() {
        let mut q = AttachQueue::default();
        let t0 = Instant::now();
        q.push(vec![request(1, 0), request(2, 0), request(3, 0)]);
        assert_eq!(q.next(t0).map(|a| a.item_id), Some(Uuid::from_u128(1)));
        // not before the gap
        assert!(q.next(t0 + Duration::from_millis(5)).is_none());
        assert_eq!(q.next(t0 + SPACING).map(|a| a.item_id), Some(Uuid::from_u128(2)));
        assert_eq!(q.next(t0 + SPACING * 2).map(|a| a.item_id), Some(Uuid::from_u128(3)));
        assert!(q.next(t0 + SPACING * 3).is_none());
        assert_eq!(q.len(), 0);
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
}

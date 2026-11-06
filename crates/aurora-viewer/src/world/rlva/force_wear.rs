//! RlvForceWear (rlvhelper.cpp, originally LGPL 2.1): the force commands one
//! object sends in a row (`@detachall:a=force,@attachallover:b=force`) gather
//! into a single `Action::Force`, one COF update, as RlvForceWear::done does
//! at the end of a command batch. One update per item made outfit HUDs
//! (JasX, Void Form) detach and rebake once per step, and a step planned
//! while the previous detach was in flight put the item back (#1265).
use super::super::{World, appearance};
use appearance::{Action, WearRequest};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long the queue waits for a requested detach to land before going on.
const DETACH_WAIT: Duration = Duration::from_secs(10);
/// How long a forced wear stands for its repeats (MIN_RETRY_REQUEST_TIME,
/// llattachmentsmgr.cpp): scripts send these many times a second.
const WEAR_RETRY: Duration = Duration::from_secs(5);

/// The `Force` batch of `source` at the end of the queue, opened if needed.
fn batch(queue: &mut Vec<(Uuid, Action)>, source: Uuid) -> (&mut Vec<Uuid>, &mut Vec<WearRequest>) {
    if !matches!(queue.last(), Some((s, Action::Force { .. })) if *s == source) {
        queue.push((
            source,
            Action::Force {
                remove: Vec::new(),
                wear: Vec::new(),
            },
        ));
    }
    match queue.last_mut() {
        Some((_, Action::Force { remove, wear })) => (remove, wear),
        _ => unreachable!("a Force batch was just pushed"),
    }
}

/// The slot a wear replaces: an attachment point (none for the default
/// point, which RlvForceWear::addAttachment never clears), a clothing layer
/// or a body part type.
fn slot(world: &World, request: &WearRequest) -> Option<(i32, u32)> {
    let it = world.inventory.items.get(&request.item)?;
    match it.asset_type {
        6 => {
            let point = if request.point == 0 {
                it.flags & 0xff
            } else {
                u32::from(request.point)
            };
            (point != 0).then_some((6, point))
        }
        5 | 13 => Some((it.asset_type, it.flags & 0xff)),
        _ => None,
    }
}

impl World {
    /// RlvForceWear::remAttachment / remWearable: a removal cancels a
    /// pending wear of the same item.
    pub(super) fn rlva_force_remove(&mut self, source: Uuid, items: Vec<Uuid>) {
        if items.is_empty() {
            return;
        }
        let (remove, wear) = batch(&mut self.rlva.appearance, source);
        for item in items {
            wear.retain(|w| w.item != item);
            if !remove.contains(&item) {
                remove.push(item);
            }
        }
    }

    /// RlvForceWear::addAttachment / addWearable: a wear cancels a pending
    /// removal of the same item ("@remattach=force,attach:worn=force" keeps
    /// it on), and a replacing wear drops the pending wears of its slot. A
    /// body part always replaces its type.
    pub(super) fn rlva_force_wear(&mut self, source: Uuid, request: WearRequest) {
        let slot_of = |r: &WearRequest| slot(self, r);
        let new_slot = slot_of(&request);
        let clears = new_slot.filter(|(kind, _)| request.replace || *kind == 13);
        let pending: Vec<_> = match self.rlva.appearance.last() {
            Some((s, Action::Force { wear, .. })) if *s == source => wear.iter().map(|w| (w.item, slot_of(w))).collect(),
            _ => Vec::new(),
        };
        let dropped: Vec<Uuid> = pending
            .into_iter()
            .filter(|(item, s)| *item == request.item || (clears.is_some() && *s == clears))
            .map(|(item, _)| item)
            .collect();
        let (remove, wear) = batch(&mut self.rlva.appearance, source);
        remove.retain(|r| *r != request.item);
        wear.retain(|w| !dropped.contains(&w.item));
        wear.push(request);
    }

    /// Whether a forced batch is to be skipped when its turn comes
    /// (`appearance::force_changes_nothing`). If not, its attachments count as
    /// requested from now until they arrive, like
    /// LLAttachmentsMgr::onAttachmentRequested and onAttachmentArrived.
    pub fn rlva_force_settled(&mut self, action: &Action, now: Instant) -> bool {
        let points = self.outfit_attachment_points();
        self.rlva
            .requested_wears
            .retain(|item, at| !points.contains_key(item) && now.saturating_duration_since(*at) < WEAR_RETRY);
        let requested = self.rlva.requested_wears.keys().copied().collect();
        if appearance::force_changes_nothing(&self.inventory, action, &points, &requested) {
            return true;
        }
        if let Action::Force { wear, .. } = action {
            for request in wear {
                let item = super::super::inventory::actions::original(&self.inventory, request.item).unwrap_or(request.item);
                if self.inventory.items.get(&item).is_some_and(|it| it.asset_type == 6) && !points.contains_key(&item) {
                    self.rlva.requested_wears.insert(item, now);
                }
            }
        }
        false
    }

    /// A detach this viewer asked for is still on the avatar: the next RLVa
    /// outfit step waits for it, or the planner would fold the object back
    /// into the COF and the sync would attach it again.
    pub fn rlva_detach_in_flight(&self, now: Instant) -> bool {
        let expected = &self.rlva.watchdog.expected_detach;
        if expected.is_empty() {
            return false;
        }
        let worn = self.outfit_attachment_items();
        expected
            .iter()
            .any(|(item, at)| worn.contains(item) && now.saturating_duration_since(*at) < DETACH_WAIT)
    }
}

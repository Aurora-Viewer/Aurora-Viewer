//! Attachment lock recovery, following RlvAttachmentLockWatchdog (rlvlocks.cpp,
//! originally LGPL 2.1). Restore requests use the exact lost item and point;
//! they never grant a general exception to scripted or user wear operations.
use super::inventory::LockInputs;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const ACTOR: Uuid = Uuid::from_u128(0x726c7661_7265_636f_7665_727900000001);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attachment {
    pub root: Uuid,
    pub point: u8,
}

pub struct Reattach {
    pub attachment: Attachment,
    pub due: Instant,
}

pub struct Wear {
    /// The order of the request: arrivals seen in one pass are handled in
    /// it, as the simulator attaches a batch one object after the other.
    pub seq: u64,
    pub replace: bool,
    pub preserve: BTreeSet<Uuid>,
    pub allowed: BTreeSet<u8>,
    pub previous: BTreeMap<Uuid, Attachment>,
    pub at: Instant,
}

#[derive(Default)]
pub struct Watchdog {
    pub deny_locked_replacement: bool,
    pub known: BTreeMap<Uuid, Attachment>,
    pub sources: BTreeMap<Uuid, Uuid>,
    pub pending: BTreeMap<Uuid, Reattach>,
    pub rejected: BTreeSet<Uuid>,
    pub wears: BTreeMap<Uuid, Wear>,
    pub expected_detach: BTreeMap<Uuid, Instant>,
    pub next_seq: u64,
    saved: BTreeMap<Uuid, Instant>,
    initialized: bool,
    pub(super) checked: Option<Checked>,
}

/// The lock checks of a pass and what they read: they run again only when
/// the rules, the inventory or what is worn changed, as the reference
/// watchdog acts on attach and detach events instead of polling each frame.
pub(super) struct Checked {
    inputs: LockInputs,
    known: Vec<(Uuid, u8)>,
    pending: Vec<(Uuid, u8)>,
    pub(super) locked: BTreeSet<Uuid>,
    pub(super) allowed: BTreeMap<Uuid, bool>,
}

pub struct Notice {
    pub verb: &'static str,
    pub point: u8,
    pub allowed: bool,
}

impl Watchdog {
    pub fn saved(&mut self, item: Uuid, now: Instant) {
        // Most inventory saves are unrelated to attachments. Remember only
        // an observed attachment, including a save delivered before its kill.
        if (!self.known.contains_key(&item) && !self.pending.contains_key(&item))
            || (self.saved.len() >= 256 && !self.saved.contains_key(&item))
        {
            return;
        }
        self.saved.insert(item, now);
        if let Some(pending) = self.pending.get_mut(&item) {
            pending.due = now;
        }
    }

    pub fn restore_allowed(&self, item: Uuid, point: u8) -> bool {
        self.pending.get(&item).is_some_and(|p| point == 0 || point == p.attachment.point)
    }

    pub fn point(&self, item: Uuid) -> Option<u8> {
        self.known
            .get(&item)
            .map(|a| a.point)
            .or_else(|| self.pending.get(&item).map(|p| p.attachment.point))
    }

    pub fn requested_detach(&mut self, items: &[Uuid], now: Instant) {
        for item in items.iter().take(256) {
            self.expected_detach.insert(*item, now);
        }
    }

    pub fn attempted(&mut self, item: Uuid, now: Instant) {
        if let Some(pending) = self.pending.get_mut(&item) {
            pending.due = now + Duration::from_secs(30);
        }
    }

    pub fn observe(
        &mut self,
        current: BTreeMap<Uuid, Attachment>,
        locked_removed: &BTreeSet<Uuid>,
        add_allowed: &BTreeMap<Uuid, bool>,
        item_exists: impl Fn(&Uuid) -> bool,
        now: Instant,
    ) -> Vec<Notice> {
        self.wears.retain(|_, wear| now.saturating_duration_since(wear.at).as_secs() <= 60);
        self.expected_detach
            .retain(|_, at| now.saturating_duration_since(*at).as_secs() <= 60);
        self.pending.retain(|item, _| item_exists(item));
        self.saved.retain(|_, at| now.saturating_duration_since(*at).as_secs() <= 60);
        if !self.initialized {
            self.known = current;
            self.initialized = true;
            return Vec::new();
        }
        let mut notices = Vec::new();
        for (&item, &attachment) in &self.known {
            if current.get(&item).is_some_and(|a| a.point == attachment.point) {
                continue;
            }
            let expected = self.expected_detach.remove(&item).is_some();
            let locked = !expected && locked_removed.contains(&item);
            if locked && item_exists(&item) {
                self.pending.entry(item).or_insert(Reattach {
                    attachment,
                    due: if self.saved.remove(&item).is_some() {
                        now
                    } else {
                        now + Duration::from_secs(15)
                    },
                });
            }
            notices.push(Notice {
                verb: "detached",
                point: attachment.point,
                allowed: !locked,
            });
        }
        // RlvAttachmentLockWatchdog::onAttach runs once per object, in the
        // order the simulator attaches them: the arrivals of this pass are
        // taken in the order they were asked for
        let mut arrivals: Vec<(Uuid, Attachment)> = current
            .iter()
            .filter(|(item, attachment)| !self.known.get(item).is_some_and(|a| a.point == attachment.point))
            .map(|(item, attachment)| (*item, *attachment))
            .collect();
        arrivals.sort_by_key(|(item, _)| self.wears.get(item).map_or(0, |w| w.seq));
        let mut later: BTreeSet<Uuid> = arrivals.iter().map(|(item, _)| *item).collect();
        for (item, attachment) in arrivals {
            later.remove(&item);
            // what the point holds when this object arrives
            let on_point: Vec<Uuid> = current
                .iter()
                .filter(|(other, a)| **other != item && a.point == attachment.point && !later.contains(*other))
                .map(|(other, _)| *other)
                .collect();
            if self.pending.get(&item).is_some_and(|p| p.attachment.point == attachment.point) {
                self.pending.remove(&item);
                self.wears.remove(&item);
                notices.push(Notice {
                    verb: "reattached",
                    point: attachment.point,
                    allowed: true,
                });
                continue;
            }
            let point_pending = self.pending.values().any(|p| p.attachment.point == attachment.point);
            let wear = self.wears.remove(&item);
            let allowed = !point_pending
                && wear.as_ref().map_or_else(
                    || add_allowed.get(&item).copied().unwrap_or(true),
                    |w| {
                        w.allowed.contains(&attachment.point)
                            && !(self.deny_locked_replacement
                                && w.replace
                                && on_point.iter().any(|old| locked_removed.contains(old) || w.preserve.contains(old)))
                    },
                );
            // a replace detaches everything else on the point, an object of
            // the same request that arrived first included, but what is
            // locked (detach(idxAttachPt, idsAttachObjExcept))
            if allowed
                && let Some(wear) = &wear
                && wear.replace
            {
                for old_item in &on_point {
                    if !locked_removed.contains(old_item) && !wear.preserve.contains(old_item) {
                        self.rejected.insert(*old_item);
                    }
                }
            }
            if !allowed {
                self.rejected.insert(item);
                if let Some(wear) = wear {
                    for (&old_item, &old) in &wear.previous {
                        if old.point == attachment.point && !current.contains_key(&old_item) && item_exists(&old_item) {
                            self.pending.entry(old_item).or_insert(Reattach {
                                attachment: old,
                                due: now + Duration::from_secs(15),
                            });
                        }
                    }
                }
            }
            notices.push(Notice {
                verb: "attached",
                point: attachment.point,
                allowed,
            });
        }
        self.known = current;
        notices
    }
}

impl super::super::World {
    fn rlva_attachment_snapshot(&self) -> BTreeMap<Uuid, Attachment> {
        let Some(agent) = self.objects.index_of_uuid(&self.agent_id).and_then(|i| self.objects.get(i)) else {
            return BTreeMap::new();
        };
        self.objects
            .children_of(&agent.key)
            .iter()
            .filter_map(|i| {
                // (not the copies drawn from the cache, world/avatar_cache)
                let object = self.objects.get(*i).filter(|o| !o.is_client_preview())?;
                let item = object.attachment_item_id().filter(|item| !item.is_nil()).unwrap_or(object.full_id);
                let point = object.attachment_point();
                (point != 0).then_some((
                    item,
                    Attachment {
                        root: object.full_id,
                        point,
                    },
                ))
            })
            .collect()
    }

    /// Called only after the common policy has authorized the native command.
    pub fn rlva_note_attachment_command(&mut self, command: &aurora_net::NetCommand, actor: Uuid, now: Instant) {
        if !self.rlva.enabled() {
            return;
        }
        match command {
            aurora_net::NetCommand::DetachObject { handle, local_id } => {
                if let Some(object) = self
                    .objects
                    .index_of(&super::super::objects::ObjKey {
                        region: *handle,
                        local_id: *local_id,
                    })
                    .and_then(|i| self.objects.get(i))
                {
                    let item = object.attachment_item_id().filter(|id| !id.is_nil()).unwrap_or(object.full_id);
                    self.rlva.watchdog.requested_detach(&[item], now);
                }
            }
            aurora_net::NetCommand::DetachAttachments(items) => self.rlva.watchdog.requested_detach(items, now),
            aurora_net::NetCommand::RezAttachments(items) if actor != ACTOR => {
                let previous = self.rlva_attachment_snapshot();
                for item in items.iter().take(256) {
                    // (the viewer's bridge is no wear to watch: fRlvForce
                    // keeps it out of onWearAttachment)
                    if (self.rlva.watchdog.wears.len() >= 256 && !self.rlva.watchdog.wears.contains_key(&item.item_id))
                        || super::super::bridge::viewer_bridge(self, item.item_id, item.point)
                    {
                        continue;
                    }
                    let allowed = self
                        .avatar_lib
                        .attach_points
                        .keys()
                        .copied()
                        .filter(|point| self.rlva_can_add(item.item_id, *point, actor))
                        .collect();
                    self.rlva.watchdog.next_seq += 1;
                    let seq = self.rlva.watchdog.next_seq;
                    self.rlva.watchdog.wears.insert(
                        item.item_id,
                        Wear {
                            seq,
                            replace: !item.add,
                            preserve: previous.keys().filter(|id| !self.rlva_can_remove(**id, actor)).copied().collect(),
                            allowed,
                            previous: previous.clone(),
                            at: now,
                        },
                    );
                }
            }
            _ => {}
        }
    }

    pub fn rlva_watchdog_tick(&mut self, now: Instant) {
        if !self.rlva.enabled() || !self.movement_complete {
            return;
        }
        let current = self.rlva_attachment_snapshot();
        let bindings: Vec<_> = self
            .rlva
            .rules
            .keys()
            .filter_map(|source| self.rlva_attachment_item(*source).map(|item| (*source, item)))
            .collect();
        self.rlva.watchdog.sources.extend(bindings);
        self.rlva.watchdog.sources.retain(|source, _| self.rlva.rules.contains_key(source));
        let inputs = self.rlva_lock_inputs(current.iter().map(|(item, attachment)| (*item, attachment.point)));
        let known: Vec<_> = self.rlva.watchdog.known.iter().map(|(item, a)| (*item, a.point)).collect();
        let pending: Vec<_> = self
            .rlva
            .watchdog
            .pending
            .iter()
            .map(|(item, p)| (*item, p.attachment.point))
            .collect();
        let checked = match self.rlva.watchdog.checked.take() {
            Some(checked) if checked.inputs == inputs && checked.known == known && checked.pending == pending => checked,
            _ => {
                // the folder locks are resolved once for every attachment
                let locks = super::inventory::FolderLocks::default();
                // The viewer's own bridge is never put back nor taken off
                // (RlvAttachmentLockWatchdog::onDetach asks
                // FSLSLBridge::canDetach first, indra/newview/rlvlocks.cpp;
                // its wear is not one onAttach knows).
                let bridge = |item: &Uuid, point: u8| super::super::bridge::viewer_bridge(self, *item, point);
                let locked = self
                    .rlva
                    .watchdog
                    .known
                    .iter()
                    .chain(current.iter())
                    .filter_map(|(item, attachment)| {
                        // The snapshot already resolves each point. Do not
                        // rebuild the world's worn-item map per attachment.
                        let allowed = bridge(item, attachment.point)
                            || (self.rlva_attachment_remove_at(*item, attachment.point, Uuid::nil())
                                && self.rlva_item_folders_removable_in(&locks, *item, Uuid::nil()));
                        (!allowed).then_some(*item)
                    })
                    .collect();
                let allowed = current
                    .iter()
                    .map(|(item, attachment)| {
                        let allowed = bridge(item, attachment.point) || self.rlva_can_add_in(&locks, *item, attachment.point, Uuid::nil());
                        (*item, allowed)
                    })
                    .collect();
                Checked {
                    inputs,
                    known,
                    pending,
                    locked,
                    allowed,
                }
            }
        };
        let inventory = &self.inventory.items;
        self.rlva.watchdog.deny_locked_replacement = !self.rlva.preferences.replace_unlocked;
        let notices = self.rlva.watchdog.observe(
            current.clone(),
            &checked.locked,
            &checked.allowed,
            |item| inventory.contains_key(item),
            now,
        );
        self.rlva.watchdog.checked = Some(checked);
        for notice in notices {
            let point = self
                .avatar_lib
                .attach_names
                .get(&notice.point)
                .map(String::as_str)
                .unwrap_or_default();
            self.rlva.notify(&format!(
                "{} {} {}",
                notice.verb,
                if notice.allowed { "legally" } else { "illegally" },
                point
            ));
        }
        // The links only matter for rejected items: most passes have none.
        let cof = super::super::appearance::cof(&self.inventory)
            .filter(|id| !self.rlva.watchdog.rejected.is_empty() && super::super::appearance::loaded(&self.inventory, *id));
        let links = cof.map(|id| super::super::appearance::folder_links(&self.inventory, id));
        if let Some(links) = &links {
            self.rlva
                .watchdog
                .rejected
                .retain(|item| current.contains_key(item) || links.iter().any(|link| link.target == *item));
        }
        // Detach rejected objects promptly. The COF cleanup is serialized with
        // other outfit transactions, so a late reply cannot reintroduce them.
        let rejected: Vec<_> = self
            .rlva
            .watchdog
            .rejected
            .iter()
            .filter(|item| current.contains_key(item) && !self.rlva.watchdog.expected_detach.contains_key(item))
            .copied()
            .collect();
        if !rejected.is_empty() && self.rlva.recovery.len() < 256 {
            self.rlva.watchdog.requested_detach(&rejected, now);
            let mut regular = Vec::new();
            for item in rejected {
                if let Some(key) = self.rlva_temporary_attachment(item) {
                    self.rlva.recovery.push(aurora_net::NetCommand::DetachObject {
                        handle: key.region,
                        local_id: key.local_id,
                    });
                } else {
                    regular.push(item);
                }
            }
            if !regular.is_empty() {
                self.rlva.recovery.push(aurora_net::NetCommand::DetachAttachments(regular));
            }
        }
        if self.rlva.appearance_busy || !self.rlva.appearance.is_empty() {
            return;
        }
        if let Some(links) = links {
            let rejected: Vec<_> = links
                .iter()
                .filter(|link| self.rlva.watchdog.rejected.contains(&link.target))
                .map(|link| link.target)
                .collect();
            if !rejected.is_empty() {
                self.rlva
                    .appearance
                    .push((ACTOR, super::super::appearance::Action::RemoveItems(rejected)));
                return;
            }
        }
        let ready = self
            .rlva
            .watchdog
            .pending
            .iter()
            .find(|(_, pending)| pending.due <= now)
            .map(|(item, pending)| (*item, pending.attachment.point));
        if let Some((item, point)) = ready {
            self.rlva.appearance.push((
                ACTOR,
                super::super::appearance::Action::WearItem {
                    item,
                    point,
                    replace: false,
                },
            ));
            self.rlva.watchdog.attempted(item, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RlvAttachmentLockWatchdog::onAttach (rlvlocks.cpp): a replacing
    /// object detaches what its point holds when it arrives, an object of
    /// the same request that arrived first included; the last one asked
    /// stays, whether both arrive in one pass or in two.
    #[test]
    fn rlva_watchdog_replacing_objects_of_one_request_keep_the_last_asked() {
        let now = Instant::now();
        let (first, second) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let on_hand = |root| Attachment {
            root: Uuid::from_u128(root),
            point: 6,
        };
        let wear = |seq| Wear {
            seq,
            replace: true,
            preserve: BTreeSet::new(),
            allowed: BTreeSet::from([6]),
            previous: BTreeMap::new(),
            at: now,
        };
        // (the second asked has the smaller id: the order is the request's)
        for (a, b) in [(second, first), (first, second)] {
            let mut one_pass = Watchdog::default();
            one_pass.observe(BTreeMap::new(), &BTreeSet::new(), &BTreeMap::new(), |_| true, now);
            one_pass.wears.insert(a, wear(1));
            one_pass.wears.insert(b, wear(2));
            let both = BTreeMap::from([(a, on_hand(11)), (b, on_hand(12))]);
            one_pass.observe(both.clone(), &BTreeSet::new(), &BTreeMap::new(), |_| true, now);
            assert_eq!(one_pass.rejected, BTreeSet::from([a]));

            let mut two_passes = Watchdog::default();
            two_passes.observe(BTreeMap::new(), &BTreeSet::new(), &BTreeMap::new(), |_| true, now);
            two_passes.wears.insert(a, wear(1));
            two_passes.wears.insert(b, wear(2));
            two_passes.observe(
                BTreeMap::from([(a, on_hand(11))]),
                &BTreeSet::new(),
                &BTreeMap::new(),
                |_| true,
                now,
            );
            assert!(two_passes.rejected.is_empty());
            two_passes.observe(both, &BTreeSet::new(), &BTreeMap::new(), |_| true, now);
            assert_eq!(two_passes.rejected, BTreeSet::from([a]));
        }
    }

    #[test]
    fn rlva_watchdog_replacement_keeps_locked_predecessors_and_removes_only_the_wear_snapshot() {
        let now = Instant::now();
        let item = Uuid::from_u128(1);
        let locked = Uuid::from_u128(2);
        let unlocked = Uuid::from_u128(3);
        let concurrent = Uuid::from_u128(4);
        let nostrip = Uuid::from_u128(5);
        let attachment = |root| Attachment {
            root: Uuid::from_u128(root),
            point: 5,
        };
        let previous = BTreeMap::from([(locked, attachment(12)), (unlocked, attachment(13)), (nostrip, attachment(15))]);
        let mut watchdog = Watchdog::default();
        watchdog.observe(previous.clone(), &BTreeSet::new(), &BTreeMap::new(), |_| true, now);
        watchdog.wears.insert(
            item,
            Wear {
                seq: 0,
                replace: true,
                preserve: BTreeSet::from([nostrip]),
                allowed: BTreeSet::from([5]),
                previous: previous.clone(),
                at: now,
            },
        );
        let mut current = previous;
        current.insert(item, attachment(11));
        current.insert(concurrent, attachment(14));
        let mut strict = Watchdog {
            deny_locked_replacement: true,
            ..Default::default()
        };
        strict.observe(watchdog.known.clone(), &BTreeSet::from([locked]), &BTreeMap::new(), |_| true, now);
        strict.wears.insert(
            item,
            Wear {
                seq: 0,
                replace: true,
                preserve: BTreeSet::from([nostrip]),
                allowed: BTreeSet::from([5]),
                previous: watchdog.known.clone(),
                at: now,
            },
        );
        strict.observe(current.clone(), &BTreeSet::from([locked]), &BTreeMap::new(), |_| true, now);
        assert_eq!(
            strict.rejected,
            BTreeSet::from([item]),
            "strict replacement rejects only the arriving item after its real point is known"
        );
        watchdog.observe(current, &BTreeSet::from([locked]), &BTreeMap::new(), |_| true, now);
        assert_eq!(watchdog.rejected, BTreeSet::from([unlocked]));
        assert!(watchdog.pending.is_empty());
    }

    #[test]
    fn rlva_watchdog_handles_save_before_detach_and_never_restores_temporary_inventory() {
        let now = Instant::now();
        let real = Uuid::from_u128(1);
        let temporary = Uuid::from_u128(2);
        let mut watchdog = Watchdog::default();
        watchdog.observe(
            BTreeMap::from([
                (
                    real,
                    Attachment {
                        root: Uuid::from_u128(11),
                        point: 3,
                    },
                ),
                (temporary, Attachment { root: temporary, point: 4 }),
            ]),
            &BTreeSet::new(),
            &BTreeMap::new(),
            |id| *id == real,
            now,
        );
        watchdog.saved(real, now);
        let notices = watchdog.observe(
            BTreeMap::new(),
            &BTreeSet::from([real, temporary]),
            &BTreeMap::new(),
            |id| *id == real,
            now,
        );
        assert!(notices.iter().all(|notice| !notice.allowed));
        assert_eq!(watchdog.pending.len(), 1);
        assert_eq!(watchdog.pending[&real].due, now);
    }

    #[test]
    fn rlva_watchdog_restores_the_exact_item_after_asset_save_and_retries_at_reference_intervals() {
        let item = Uuid::from_u128(1);
        let attachment = Attachment {
            root: Uuid::from_u128(2),
            point: 5,
        };
        let now = Instant::now();
        let inventory = BTreeSet::from([item]);
        let mut watchdog = Watchdog::default();
        watchdog.observe(
            BTreeMap::from([(item, attachment)]),
            &BTreeSet::new(),
            &BTreeMap::new(),
            |id| inventory.contains(id),
            now,
        );
        let notices = watchdog.observe(
            BTreeMap::new(),
            &BTreeSet::from([item]),
            &BTreeMap::new(),
            |id| inventory.contains(id),
            now,
        );
        assert!(!notices[0].allowed);
        assert_eq!(watchdog.pending[&item].due, now + Duration::from_secs(15));
        assert!(watchdog.restore_allowed(item, 5) && watchdog.restore_allowed(item, 0));
        assert!(!watchdog.restore_allowed(item, 6) && !watchdog.restore_allowed(Uuid::from_u128(99), 5));
        watchdog.saved(item, now + Duration::from_secs(2));
        assert_eq!(watchdog.pending[&item].due, now + Duration::from_secs(2));
        watchdog.attempted(item, now + Duration::from_secs(2));
        assert_eq!(watchdog.pending[&item].due, now + Duration::from_secs(32));
        let notices = watchdog.observe(
            BTreeMap::from([(
                item,
                Attachment {
                    root: Uuid::from_u128(3),
                    ..attachment
                },
            )]),
            &BTreeSet::new(),
            &BTreeMap::new(),
            |id| inventory.contains(id),
            now + Duration::from_secs(3),
        );
        assert_eq!(notices[0].verb, "reattached");
        assert!(watchdog.pending.is_empty());
    }

    #[test]
    fn rlva_watchdog_respects_authorized_detaches_and_rolls_back_a_wear_on_a_locked_point() {
        let old = Uuid::from_u128(1);
        let new = Uuid::from_u128(2);
        let now = Instant::now();
        let attachment = Attachment {
            root: Uuid::from_u128(11),
            point: 3,
        };
        let mut watchdog = Watchdog::default();
        let before = BTreeMap::from([(old, attachment)]);
        let inventory = BTreeSet::from([old, new]);
        watchdog.observe(before.clone(), &BTreeSet::new(), &BTreeMap::new(), |id| inventory.contains(id), now);
        watchdog.requested_detach(&[old], now);
        let notices = watchdog.observe(
            BTreeMap::new(),
            &BTreeSet::from([old]),
            &BTreeMap::new(),
            |id| inventory.contains(id),
            now,
        );
        assert!(notices[0].allowed && watchdog.pending.is_empty());
        watchdog.wears.insert(
            new,
            Wear {
                seq: 0,
                replace: true,
                preserve: BTreeSet::new(),
                allowed: BTreeSet::from([4]),
                previous: before,
                at: now,
            },
        );
        let current = BTreeMap::from([(
            new,
            Attachment {
                root: Uuid::from_u128(12),
                point: 3,
            },
        )]);
        let notices = watchdog.observe(current, &BTreeSet::new(), &BTreeMap::new(), |id| inventory.contains(id), now);
        assert!(!notices[0].allowed);
        assert!(watchdog.rejected.contains(&new) && watchdog.pending.contains_key(&old));
    }
}

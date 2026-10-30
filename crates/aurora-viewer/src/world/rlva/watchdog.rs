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
        for (&item, &attachment) in &current {
            if self.known.get(&item).is_some_and(|a| a.point == attachment.point) {
                continue;
            }
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
                                && w.previous.iter().any(|(old_item, old)| {
                                    *old_item != item
                                        && old.point == attachment.point
                                        && (locked_removed.contains(old_item) || w.preserve.contains(old_item))
                                }))
                    },
                );
            if allowed
                && let Some(wear) = &wear
                && wear.replace
            {
                for (&old_item, old) in &wear.previous {
                    if old_item != item
                        && old.point == attachment.point
                        && current.contains_key(&old_item)
                        && !locked_removed.contains(&old_item)
                        && !wear.preserve.contains(&old_item)
                    {
                        self.rejected.insert(old_item);
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
                let object = self.objects.get(*i)?;
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
                    self.rlva_note_wear(item.item_id, !item.add, actor, &previous, now);
                }
            }
            // Force wear goes through UpdateOutfit first; register the same
            // RlvWearInfo transaction Legacy records in onWearAttachment so
            // folder locks do not fight the scripted strip mid-COF.
            aurora_net::NetCommand::UpdateOutfit { change, .. } if actor != ACTOR => {
                let Some(cof) = super::super::appearance::cof(&self.inventory) else {
                    return;
                };
                let before = super::super::appearance::folder_links(&self.inventory, cof);
                let links = if change.folder == cof {
                    &change.links
                } else if let Some((id, links)) = &change.cof {
                    if *id != cof {
                        return;
                    }
                    links
                } else {
                    return;
                };
                let previous = self.rlva_attachment_snapshot();
                let mut removed = Vec::new();
                for old in &before {
                    if old.folder || links.iter().any(|link| link.target == old.target) {
                        continue;
                    }
                    if self.inventory.items.get(&old.target).is_some_and(|item| item.asset_type == 6) {
                        removed.push(old.target);
                    }
                }
                if !removed.is_empty() {
                    self.rlva.watchdog.requested_detach(&removed, now);
                }
                for link in links.iter().take(256) {
                    if link.folder || before.iter().any(|old| old.target == link.target) {
                        continue;
                    }
                    if !self.inventory.items.get(&link.target).is_some_and(|item| item.asset_type == 6) {
                        continue;
                    }
                    let point = previous
                        .get(&link.target)
                        .map(|attachment| attachment.point)
                        .or_else(|| self.inventory.items.get(&link.target).map(|item| (item.flags & 0xff) as u8))
                        .unwrap_or(0);
                    let replace = removed.iter().any(|old| {
                        previous.get(old).is_some_and(|attachment| attachment.point == point)
                            || (point != 0 && self.inventory.items.get(old).is_some_and(|item| (item.flags & 0xff) as u8 == point))
                    });
                    self.rlva_note_wear(link.target, replace, actor, &previous, now);
                }
            }
            _ => {}
        }
    }

    fn rlva_note_wear(&mut self, item: Uuid, replace: bool, actor: Uuid, previous: &BTreeMap<Uuid, Attachment>, now: Instant) {
        if self.rlva.watchdog.wears.len() >= 256 && !self.rlva.watchdog.wears.contains_key(&item) {
            return;
        }
        let allowed = self
            .avatar_lib
            .attach_points
            .keys()
            .copied()
            .filter(|point| self.rlva_can_add(item, *point, actor))
            .collect();
        self.rlva.watchdog.wears.insert(
            item,
            Wear {
                replace,
                preserve: previous.keys().filter(|id| !self.rlva_can_remove(**id, actor)).copied().collect(),
                allowed,
                previous: previous.clone(),
                at: now,
            },
        );
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
                let locked = self
                    .rlva
                    .watchdog
                    .known
                    .iter()
                    .chain(current.iter())
                    .filter_map(|(item, attachment)| {
                        // The snapshot already resolves each point. Do not
                        // rebuild the world's worn-item map per attachment.
                        let allowed = self.rlva_attachment_remove_at(*item, attachment.point, Uuid::nil())
                            && self
                                .inventory
                                .items
                                .get(item)
                                .is_none_or(|it| self.rlva_folder_allowed_in(&locks, it.parent, false, Uuid::nil()));
                        (!allowed).then_some(*item)
                    })
                    .collect();
                let allowed = current
                    .iter()
                    .map(|(item, attachment)| (*item, self.rlva_can_add_in(&locks, *item, attachment.point, Uuid::nil())))
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
        let defer_notices = self.rlva.appearance_busy || !self.rlva.appearance.is_empty();
        for notice in notices {
            if defer_notices {
                continue;
            }
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

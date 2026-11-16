//! Outgoing inventory offers use a reliable IM. Its transport receipt confirms
//! the simulator saw the message; only a separate resident reply accepts it.
use super::{Inventory, actions};
use aurora_net::{
    InventoryOfferStatus, NetCommand,
    inventory::{InvItem, ais::ItemReads},
};
use crossbeam_channel::{Receiver, TryRecvError};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct Outgoing {
    pending: Vec<Pending>,
    // A delayed inventory snapshot may contain an original already given. If
    // it reappears in this session, check it by UUID before another manual give.
    removed: HashMap<Uuid, Uuid>,
    // Other folders observed during a move concurrent with an unresolved give.
    moved_parents: HashSet<Uuid>,
}

struct Pending {
    session: Uuid,
    recipient: Uuid,
    name: String,
    folder: bool,
    originals: HashMap<Uuid, Uuid>,
    state: State,
}

enum State {
    Sending {
        receipt: Receiver<InventoryOfferStatus>,
        transmitted: bool,
    },
    Checking {
        reply: Receiver<ItemReads>,
        baseline: HashMap<Uuid, Option<InvItem>>,
        retry_on_change: bool,
    },
    Retry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Feedback {
    Unsent,
    Pending,
    Uncertain,
    Checking,
    CheckFailed,
    Checked,
}

impl Feedback {
    pub fn text(self) -> String {
        match self {
            Self::Unsent => aurora_i18n::tr!("world-inventory-offers-unsent"),
            Self::Pending => aurora_i18n::tr!("world-inventory-offers-pending"),
            Self::Uncertain => aurora_i18n::tr!("world-inventory-offers-uncertain"),
            Self::Checking => aurora_i18n::tr!("world-inventory-offers-checking"),
            Self::CheckFailed => aurora_i18n::tr!("world-inventory-offers-check-failed"),
            Self::Checked => aurora_i18n::tr!("world-inventory-offers-checked"),
        }
        .into()
    }
}

pub(crate) enum Effect {
    Sent { recipient: Uuid, name: String, folder: bool },
    Feedback(Feedback),
    Check(Box<NetCommand>),
}

impl Outgoing {
    pub fn has_originals(&self) -> bool {
        !self.moved_parents.is_empty() || !self.removed.is_empty() || self.pending.iter().any(|p| !p.originals.is_empty())
    }

    pub fn uncached_folders(&self, items: &super::items::Items) -> HashSet<Uuid> {
        let mut parents = self.moved_parents.clone();
        for p in &self.pending {
            if let State::Checking { baseline, .. } = &p.state {
                parents.extend(baseline.values().flatten().map(|item| item.parent));
            }
        }
        for (id, parent) in self.pending.iter().flat_map(|p| &p.originals).chain(&self.removed) {
            parents.insert(*parent);
            if let Some(item) = items.get(id) {
                parents.insert(item.parent);
            }
        }
        parents
    }
}

impl Pending {
    fn check(&mut self, items: &super::items::Items, retry_on_change: bool) -> NetCommand {
        let (reply, reads) = crossbeam_channel::unbounded();
        let baseline = self.originals.keys().map(|id| (*id, items.get(id).cloned())).collect();
        self.state = State::Checking {
            reply: reads,
            baseline,
            retry_on_change,
        };
        NetCommand::CheckInventoryOffer {
            session: self.session,
            items: self.originals.keys().copied().collect(),
            reply,
        }
    }
}

impl Inventory {
    /// The caller has just rebuilt the bucket through the normal share rules.
    pub(crate) fn begin_offer(
        &mut self,
        session: Option<Uuid>,
        recipient: Uuid,
        root: Uuid,
        bucket: Vec<u8>,
    ) -> Result<NetCommand, Feedback> {
        let session = session.ok_or(Feedback::Unsent)?;
        let ids: Vec<_> = bucket
            .as_chunks::<17>()
            .0
            .iter()
            .filter_map(|tuple| Uuid::from_slice(&tuple[1..]).ok())
            .collect();
        // Reserve every protected original even for a folder. Copyable items
        // remain independently offerable to several residents.
        if let Some(pending) = self
            .outgoing
            .pending
            .iter_mut()
            .find(|p| ids.iter().any(|id| p.originals.contains_key(id)))
        {
            return match pending.state {
                State::Sending { .. } => Err(Feedback::Pending),
                State::Checking { .. } => Err(Feedback::Checking),
                State::Retry => Ok(pending.check(&self.items, true)),
            };
        }
        let reappeared: HashMap<_, _> = ids
            .iter()
            .filter_map(|id| self.outgoing.removed.get(id).map(|parent| (*id, *parent)))
            .collect();
        let name = actions::name(self, root);
        let folder = self.folders.contains_key(&root);
        if !reappeared.is_empty() {
            let mut pending = Pending {
                session,
                recipient,
                name,
                folder,
                originals: reappeared,
                state: State::Retry,
            };
            let check = pending.check(&self.items, true);
            self.outgoing.pending.push(pending);
            return Ok(check);
        }
        let originals = ids
            .iter()
            .filter_map(|id| {
                self.items
                    .get(id)
                    .filter(|it| actions::held(self, it) & actions::COPY == 0)
                    .map(|it| (it.id, it.parent))
            })
            .collect();
        let (receipt, rx) = crossbeam_channel::unbounded();
        self.outgoing.pending.push(Pending {
            session,
            recipient,
            name: name.clone(),
            folder,
            originals,
            state: State::Sending {
                receipt: rx,
                transmitted: false,
            },
        });
        Ok(NetCommand::OfferInventory {
            session,
            to: recipient,
            id: Uuid::new_v4(),
            message: name,
            bucket,
            receipt,
        })
    }

    fn remove_offer_originals(&mut self, originals: &mut HashMap<Uuid, Uuid>, ids: Vec<Uuid>) {
        let mut remove = Vec::new();
        for id in ids {
            let Some(source) = originals.remove(&id) else { continue };
            if let Some(item) = self.items.get(&id)
                && item.parent != source
            {
                self.outgoing.moved_parents.insert(item.parent);
            }
            self.outgoing.removed.insert(id, source);
            remove.push(id);
        }
        if !remove.is_empty() {
            self.remove(&remove);
        }
    }

    fn reconcile_offer(
        &mut self,
        originals: &mut HashMap<Uuid, Uuid>,
        mut reads: ItemReads,
        baseline: &HashMap<Uuid, Option<InvItem>>,
    ) -> bool {
        // Ordinary network events may have removed or changed the item since this
        // private read started. An older result cannot restore it or its old rights.
        let mut changed = false;
        let mut fresh = |id: &Uuid| {
            let Some(before) = baseline.get(id) else { return false };
            let current = self.items.get(id);
            if current == before.as_ref() {
                return true;
            }
            for item in before.iter().chain(current) {
                if originals.get(id) != Some(&item.parent) {
                    self.outgoing.moved_parents.insert(item.parent);
                }
            }
            changed = true;
            false
        };
        reads.items.retain(|it| fresh(&it.id));
        reads.missing.retain(&mut fresh);
        for item in &reads.items {
            originals.remove(&item.id);
            self.outgoing.removed.remove(&item.id);
        }
        if !reads.items.is_empty() {
            self.add_items(reads.items);
        }
        self.remove_offer_originals(originals, reads.missing);
        changed
    }

    pub(crate) fn poll_offers(&mut self, session: Option<Uuid>) -> Vec<Effect> {
        let mut effects = Vec::new();
        if self.outgoing.pending.is_empty() || session.is_none() {
            return effects;
        }
        // Move the work out to let an authoritative reply update the ordinary
        // inventory indices (including a moved item), using the usual helpers.
        let mut pending = std::mem::take(&mut self.outgoing.pending);
        pending.retain_mut(|p| {
            if Some(p.session) != session {
                return false;
            }
            match &mut p.state {
                State::Sending { receipt, transmitted } => loop {
                    match receipt.try_recv() {
                        Ok(InventoryOfferStatus::Transmitted) => *transmitted = true,
                        Ok(InventoryOfferStatus::Acknowledged) => {
                            // LLGiveInventory::giveInventoryItemToAgent removes no-copy
                            // originals after send. Wait for the actual simulator ACK.
                            effects.push(Effect::Sent {
                                recipient: p.recipient,
                                name: std::mem::take(&mut p.name),
                                folder: p.folder,
                            });
                            let ids = p
                                .originals
                                .iter()
                                .filter_map(|(id, source)| self.items.get(id).is_none_or(|item| item.parent == *source).then_some(*id))
                                .collect();
                            self.remove_offer_originals(&mut p.originals, ids);
                            if p.originals.is_empty() {
                                return false;
                            }
                            effects.push(Effect::Check(Box::new(p.check(&self.items, true))));
                            return true;
                        }
                        Err(TryRecvError::Empty) => return true,
                        Err(TryRecvError::Disconnected) if !*transmitted => {
                            effects.push(Effect::Feedback(Feedback::Unsent));
                            return false;
                        }
                        Err(TryRecvError::Disconnected) => {
                            effects.push(Effect::Feedback(Feedback::Uncertain));
                            if p.originals.is_empty() {
                                return false;
                            }
                            effects.push(Effect::Check(Box::new(p.check(&self.items, true))));
                            return true;
                        }
                    }
                },
                State::Checking {
                    reply,
                    baseline,
                    retry_on_change,
                } => match reply.try_recv() {
                    Err(TryRecvError::Empty) => true,
                    result => {
                        if let Ok(reads) = result
                            && self.reconcile_offer(&mut p.originals, reads, baseline)
                            && *retry_on_change
                        {
                            // An inventory update raced this check. Retry the read once;
                            // the gift itself is never repeated.
                            effects.push(Effect::Check(Box::new(p.check(&self.items, false))));
                            return true;
                        }
                        if p.originals.is_empty() {
                            effects.push(Effect::Feedback(Feedback::Checked));
                            return false;
                        }
                        effects.push(Effect::Feedback(Feedback::CheckFailed));
                        p.state = State::Retry;
                        true
                    }
                },
                State::Retry => true,
            }
        });
        self.outgoing.pending = pending;
        effects
    }
}

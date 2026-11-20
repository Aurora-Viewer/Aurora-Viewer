//! Photo-posing consent, asked by our own bridge the way any posing HUD asks
//! it: a standard script dialog (llDialog) shows the resident a Yes / No
//! question, then once they accept how to stop, with a Stop button and
//! `stop` on [`POSE_STOP_CHANNEL`] in nearby chat (`world/bridge/query.lsl`).
//! It works whatever viewer the resident uses, and nothing goes from our
//! viewer to theirs (Second Life's shared experience policy; AGENTS.md, "No
//! data of our own in shared places"). The pose itself is drawn on our
//! screen only (`ik_edit::poser`). A consent holds while both stay in the
//! region: it ends when either leaves, when they stop it, or when we cancel.
use super::World;
use aurora_net::{
    NetCommand,
    bridge::{
        Endpoint,
        query::{POSE_STOP_CHANNEL, PoseDialog, PoseKind, Query},
    },
};
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};
use uuid::Uuid;

const TIMEOUT: Duration = Duration::from_secs(60);
const COOLDOWN: Duration = Duration::from_secs(600);
const LIMIT: usize = 64;
#[derive(Clone, Copy)]
pub struct Request {
    pub id: Uuid,
    pub peer: Uuid,
    pub at: Instant,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ask,
    Pending,
    Accepted,
    Refused,
    NoAnswer,
}
/// What our bridge is told to do: show a resident the dialog, or forget
/// what it asked them (cancelled, timed out, ended).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Out {
    Ask { request: Uuid, peer: Uuid },
    Forget { request: Uuid, peer: Uuid },
}
#[derive(Default)]
pub struct State {
    agent: Uuid,
    region: Option<u64>,
    outgoing: HashMap<Uuid, Request>,
    accepted: HashMap<Uuid, Uuid>,
    refused: HashMap<Uuid, (Instant, bool)>,
    out: VecDeque<Out>,
    pub notice: Option<&'static str>,
}
#[cfg(test)]
mod tests;
pub fn present(world: &World, peer: Uuid) -> bool {
    let Some(own_index) = world.objects.index_of_uuid(&world.agent_id) else {
        return false;
    };
    let Some(other_index) = world.objects.index_of_uuid(&peer) else {
        return false;
    };
    let Some(own) = world.objects.get(own_index) else {
        return false;
    };
    let Some(other) = world.objects.get(other_index) else {
        return false;
    };
    peer != world.agent_id
        && other.is_avatar()
        && other.key.region == own.key.region
        && Some(other.key.region) == world.main_region
        && !world.mutes.is_muted_id(&peer)
}
pub fn distance(world: &World, peer: Uuid) -> Option<f32> {
    let own = world.objects.index_of_uuid(&world.agent_id)?;
    let other = world.objects.index_of_uuid(&peer)?;
    let now = Instant::now();
    let a = crate::scene::Scene::object_transform(world, own, now, 0)?;
    let b = crate::scene::Scene::object_transform(world, other, now, 0)?;
    Some(a.0.distance(b.0))
}
pub fn nearby(world: &World, peer: Uuid) -> bool {
    present(world, peer) && distance(world, peer).is_some_and(|d| d <= 50.0)
}
pub fn name(world: &World, peer: Uuid) -> String {
    world
        .social
        .avatar_names
        .complete(&peer)
        .or_else(|| world.person_name(&peer))
        .unwrap_or_else(|| aurora_i18n::tr!("poser-resident").to_string())
}
/// The dialog's texts, in our language: the resident reads what we wrote.
pub fn dialog() -> PoseDialog {
    use aurora_i18n::tr;
    let channel = POSE_STOP_CHANNEL.to_string();
    PoseDialog::new(
        &tr!("poser-consent-dialog-question", channel = channel.clone()),
        tr!("poser-consent-dialog-yes"),
        tr!("poser-consent-dialog-no"),
        &tr!("poser-consent-dialog-granted", channel = channel),
        tr!("poser-consent-dialog-stop"),
        tr!("poser-consent-dialog-stopped"),
    )
}
impl State {
    pub fn sync(&mut self, world: &World, now: Instant) {
        // a region change ends every consent; the bridge forgets them too
        if self.agent != world.agent_id || self.region != world.main_region || world.teleporting {
            *self = Self {
                agent: world.agent_id,
                region: world.main_region,
                ..Default::default()
            };
        }
        let departed: Vec<_> = self.accepted.keys().copied().filter(|p| !present(world, *p)).collect();
        for p in departed {
            self.cancel(p);
            self.notice = Some("poser-consent-ended");
        }
        let expired: Vec<_> = self
            .outgoing
            .values()
            .filter(|r| now.duration_since(r.at) >= TIMEOUT || !nearby(world, r.peer))
            .copied()
            .collect();
        for r in expired {
            self.cancel(r.peer);
            self.refused.insert(r.peer, (now, true));
        }
        self.refused.retain(|_, (at, _)| now.duration_since(*at) < COOLDOWN);
    }
    fn send(&mut self, out: Out) {
        if self.out.len() < LIMIT * 2 {
            self.out.push_back(out);
        }
    }
    pub fn request(&mut self, world: &World, peer: Uuid, now: Instant) -> bool {
        if !nearby(world, peer)
            || self.outgoing.len() >= LIMIT
            || self.status(peer) != Status::Ask && self.status(peer) != Status::NoAnswer
            || self.refused.contains_key(&peer)
        {
            return false;
        }
        let request = Request {
            id: Uuid::new_v4(),
            peer,
            at: now,
        };
        self.outgoing.insert(peer, request);
        self.send(Out::Ask { request: request.id, peer });
        true
    }
    pub fn status(&self, peer: Uuid) -> Status {
        if self.accepted.contains_key(&peer) {
            Status::Accepted
        } else if self.outgoing.contains_key(&peer) {
            Status::Pending
        } else if let Some((_, timeout)) = self.refused.get(&peer) {
            if *timeout { Status::NoAnswer } else { Status::Refused }
        } else {
            Status::Ask
        }
    }
    /// The avatars whose owners accepted our request, nearby or not
    /// (`can_pose` says whether one may be posed now).
    pub fn accepted_peers(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.accepted.keys().copied()
    }
    pub fn can_pose(&self, world: &World, peer: Uuid) -> bool {
        self.accepted.contains_key(&peer) && nearby(world, peer)
    }
    pub fn cancel(&mut self, peer: Uuid) {
        if let Some(r) = self.outgoing.remove(&peer) {
            // an ask not sent yet is simply dropped
            let queued = self.out.len();
            self.out.retain(|o| *o != Out::Ask { request: r.id, peer });
            if self.out.len() == queued {
                self.send(Out::Forget { request: r.id, peer });
            }
        }
        if let Some(id) = self.accepted.remove(&peer) {
            self.send(Out::Forget { request: id, peer });
        }
    }
    /// Called only after the worn bridge root and its owner/region were
    /// checked: `peer` is the avatar who answered our bridge's dialog.
    pub fn receive(&mut self, world: &World, text: &str, now: Instant) {
        if text.len() > 160 {
            return;
        }
        let Some(body) = text.strip_prefix("<auroraPose>").and_then(|s| s.strip_suffix("</auroraPose>")) else {
            return;
        };
        let fields: Vec<_> = body.split('|').collect();
        if fields.len() != 3 {
            return;
        }
        let (Ok(id), Some(kind), Ok(peer)) = (Uuid::parse_str(fields[0]), PoseKind::parse(fields[1]), Uuid::parse_str(fields[2])) else {
            return;
        };
        if id.is_nil() || !present(world, peer) {
            return;
        }
        match kind {
            PoseKind::Accept | PoseKind::Refuse => {
                let Some(r) = self
                    .outgoing
                    .get(&peer)
                    .filter(|r| r.id == id && now.duration_since(r.at) < TIMEOUT)
                    .copied()
                else {
                    return;
                };
                self.outgoing.remove(&peer);
                if kind == PoseKind::Accept {
                    self.accepted.insert(peer, r.id);
                } else {
                    self.refused.insert(peer, (now, false));
                }
            }
            PoseKind::Withdraw => {
                if self.accepted.get(&peer) == Some(&id) {
                    self.accepted.remove(&peer);
                    self.notice = Some("poser-consent-ended");
                }
            }
        }
    }
    pub fn commands(&mut self, endpoint: Option<Endpoint>) -> Vec<NetCommand> {
        let Some(endpoint) = endpoint else { return Vec::new() };
        self.out
            .drain(..)
            .map(|out| {
                let (request, target, ask) = match out {
                    Out::Ask { request, peer } => (request, peer, Some(dialog())),
                    Out::Forget { request, peer } => (request, peer, None),
                };
                NetCommand::BridgeQuery {
                    request: Uuid::new_v4(),
                    endpoint: endpoint.clone(),
                    query: Query::Pose { request, target, ask },
                }
            })
            .collect()
    }
    /// Offline fixture only; callers gate this on the application's demo mode.
    /// The compact demo's stand-in for the peer: a request to `peer` older
    /// than `after` is accepted, as her answer to the dialog would be.
    pub fn demo_accept(&mut self, world: &World, peer: Uuid, after: Duration) {
        let now = Instant::now();
        if let Some(r) = self.outgoing.get(&peer).filter(|r| now.duration_since(r.at) >= after) {
            let id = r.id;
            self.receive(world, &format!("<auroraPose>{id}|accept|{peer}</auroraPose>"), now);
        }
    }
    pub fn demo(&mut self, world: &World, peer: Uuid, mode: &str, frame: u64) {
        let now = Instant::now();
        // consent-reply: nothing asked by itself; whoever the user asks
        // accepts two seconds later, as a resident would
        if mode == "consent-reply" {
            let due: Vec<Request> = self
                .outgoing
                .values()
                .filter(|r| now.duration_since(r.at) >= Duration::from_secs(2))
                .copied()
                .collect();
            for r in due {
                self.receive(world, &format!("<auroraPose>{}|accept|{}</auroraPose>", r.id, r.peer), now);
            }
            return;
        }
        if frame == 450 {
            self.request(world, peer, now);
        }
        if frame == 480 {
            if mode == "consent-timeout" {
                if let Some(r) = self.outgoing.get_mut(&peer) {
                    r.at = now - TIMEOUT - Duration::from_secs(1);
                }
                self.sync(world, now);
            } else if let Some(r) = self.outgoing.get(&peer) {
                let id = r.id;
                let kind = if mode == "consent-refuse" { "refuse" } else { "accept" };
                self.receive(world, &format!("<auroraPose>{id}|{kind}|{peer}</auroraPose>"), now);
            }
        }
        // consent-withdraw: she presses Stop, or types stop in nearby chat
        if frame == 560
            && mode == "consent-withdraw"
            && let Some(id) = self.accepted.get(&peer).copied()
        {
            self.receive(world, &format!("<auroraPose>{id}|withdraw|{peer}</auroraPose>"), now);
        }
    }
}

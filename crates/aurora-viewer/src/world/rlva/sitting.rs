//! RlvHandler::onSitOrStand and forced sit/ground-sit transitions
//! (rlvhandler.cpp and rlvactions.cpp, originally LGPL 2.1).
use super::{super::World, Rlva};
use aurora_net::{NetCommand, control};
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Seat {
    Standing,
    Ground,
    Object(Uuid),
}

#[derive(Default)]
pub struct Sitting {
    previous: Option<Seat>,
    sit_actor: Uuid,
    stand_actor: Uuid,
    pending_ground: Option<Uuid>,
}

impl Rlva {
    /// Called only after the common action guard accepts the command. An
    /// unsuccessful request must not immediately report a sit or a stand.
    pub fn note_sit_command(&mut self, command: &NetCommand, actor: Uuid) {
        if !self.enabled() {
            return;
        }
        let (sit, stand) = match command {
            NetCommand::RequestSit { .. } => (true, false),
            NetCommand::OneShotControl(flags) => (flags & control::SIT_ON_GROUND != 0, flags & control::STAND_UP != 0),
            _ => return,
        };
        if stand {
            self.sitting.stand_actor = actor;
            self.sitting.sit_actor = Uuid::nil();
        } else if sit {
            self.sitting.sit_actor = actor;
            self.sitting.stand_actor = Uuid::nil();
        }
    }
}

impl World {
    fn rlva_seat(&self) -> Option<Seat> {
        if self.agent.ground_sit {
            return Some(Seat::Ground);
        }
        if !self.agent.seated {
            return Some(Seat::Standing);
        }
        self.objects
            .index_of_uuid(&self.agent_id)
            .and_then(|i| self.objects.get(i))
            .and_then(|o| self.objects.parent_of(o))
            .and_then(|i| self.objects.get(i))
            .map(|o| Seat::Object(o.full_id))
    }

    /// getsitid reports the seat's root linkset, whereas notifications name
    /// the actual parent prim. Both remain independent of location privacy.
    pub(super) fn rlva_sit_id(&self) -> Uuid {
        if !self.agent.seated {
            return Uuid::nil();
        }
        let Some(mut index) = self
            .objects
            .index_of_uuid(&self.agent_id)
            .and_then(|i| self.objects.get(i))
            .and_then(|o| self.objects.parent_of(o))
        else {
            return Uuid::nil();
        };
        for _ in 0..64 {
            let Some(o) = self.objects.get(index) else { return Uuid::nil() };
            if o.parent_id == 0 {
                return o.full_id;
            }
            let Some(parent) = self.objects.parent_of(o) else {
                return Uuid::nil();
            };
            index = parent;
        }
        Uuid::nil()
    }

    pub(super) fn rlva_force_ground_sit(&mut self, actor: Uuid) -> Option<NetCommand> {
        if self.rlva.has_except("sit", actor) || (self.agent.is_sitting() && self.rlva.has_except("unsit", actor)) || self.agent.ground_sit
        {
            return None;
        }
        if self.agent.seated {
            self.rlva.sitting.pending_ground = Some(actor);
            Some(NetCommand::OneShotControl(control::STAND_UP))
        } else {
            self.rlva.sitting.pending_ground = None;
            Some(NetCommand::OneShotControl(control::SIT_ON_GROUND))
        }
    }

    /// Observe confirmed avatar state, not intent. Keep the previous seat
    /// through missing parent updates, and emit removals before additions.
    /// A stand return takes precedence over a delayed ground sit.
    pub fn rlva_sitting_tick(&mut self) -> Vec<(Uuid, NetCommand)> {
        if !self.rlva.enabled() {
            return Vec::new();
        }
        let Some(current) = self.rlva_seat() else { return Vec::new() };
        if let Some(previous) = self.rlva.sitting.previous
            && previous != current
        {
            if previous != Seat::Standing {
                let actor = std::mem::take(&mut self.rlva.sitting.stand_actor);
                let legal = !self.rlva.has_except("unsit", actor);
                self.rlva_sit_notice("unsat", previous, legal);
            }
            if current != Seat::Standing {
                let actor = std::mem::take(&mut self.rlva.sitting.sit_actor);
                let legal = !self.rlva.has_except("sit", actor);
                self.rlva_sit_notice("sat", current, legal);
            }
        }
        self.rlva.sitting.previous = Some(current);
        if self.rlva.stand_return.is_some() {
            self.rlva.sitting.pending_ground = None;
        } else if current == Seat::Standing
            && let Some(actor) = self.rlva.sitting.pending_ground.take()
        {
            let command = NetCommand::OneShotControl(control::SIT_ON_GROUND);
            if self.rlva_allows_command(&command, actor) {
                return vec![(actor, command)];
            }
        }
        Vec::new()
    }

    fn rlva_sit_notice(&mut self, verb: &str, seat: Seat, legal: bool) {
        let legal = if legal { "legally" } else { "illegally" };
        let text = match seat {
            Seat::Standing => return,
            Seat::Ground => format!("{verb} ground {legal}"),
            Seat::Object(id) => format!("{verb} object {legal} {id}"),
        };
        self.rlva.notify(&text);
    }
}

#[cfg(test)]
mod tests {
    use super::super::Command;
    use super::*;
    const A: Uuid = Uuid::from_u128(1);
    const B: Uuid = Uuid::from_u128(2);
    const ROOT: Uuid = Uuid::from_u128(101);
    const CHILD: Uuid = Uuid::from_u128(102);

    fn world() -> World {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            w.apply(event);
        }
        for (id, local, parent) in [(ROOT, 31001, 0), (CHILD, 31002, 31001)] {
            let mut o = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
                item_id: id,
                owner_id: w.agent_id,
                point: 1,
                add: true,
                flags: 0,
                group_mask: 0,
                everyone_mask: 0,
                next_owner_mask: 0,
                name: String::new(),
                desc: String::new(),
            });
            o.local_id = local;
            o.parent_id = parent;
            o.state = 0;
            o.name_values.clear();
            w.objects.upsert(w.main_region.unwrap(), o);
        }
        w.rlva.configure(true);
        restrict(&mut w, A, "notify:123=n");
        notices(&mut w);
        w.rlva_sitting_tick();
        w
    }

    fn restrict(w: &mut World, actor: Uuid, text: &str) {
        w.rlva.process(actor, Command::parse(text).unwrap());
    }
    fn seated(w: &mut World, on: bool) {
        w.agent.seated = on;
        w.agent.ground_sit = false;
        let own = w.objects.index_of_uuid(&w.agent_id).unwrap();
        w.objects.get_mut(own).unwrap().parent_id = if on { 31002 } else { 0 };
    }
    fn notices(w: &mut World) -> Vec<String> {
        std::mem::take(&mut w.rlva.out)
            .into_iter()
            .filter_map(|c| match c {
                NetCommand::ChatReply { message, .. } => Some(message),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn rlva_ground_sit_waits_for_the_stand_and_preserves_actors_in_both_notifications() {
        let mut w = world();
        seated(&mut w, true);
        w.rlva_sitting_tick();
        notices(&mut w);
        restrict(&mut w, A, "sit=n");
        restrict(&mut w, A, "unsit=n");
        notices(&mut w);
        let stand = w.rlva_force_ground_sit(A).unwrap();
        assert!(matches!(stand, NetCommand::OneShotControl(control::STAND_UP)));
        assert!(w.rlva_allows_command(&stand, A));
        w.rlva.note_sit_command(&stand, A);
        assert!(w.rlva_sitting_tick().is_empty());
        assert!(notices(&mut w).is_empty(), "the request is not a confirmed stand");
        seated(&mut w, false);
        let ground = w.rlva_sitting_tick();
        assert!(matches!(&ground[..], [(actor, NetCommand::OneShotControl(control::SIT_ON_GROUND))] if *actor == A));
        assert_eq!(notices(&mut w), [format!("/unsat object legally {CHILD}")]);
        w.rlva.note_sit_command(&ground[0].1, A);
        w.agent.ground_sit = true;
        assert!(w.rlva_sitting_tick().is_empty());
        assert_eq!(notices(&mut w), ["/sat ground legally"]);
        w.agent.ground_sit = false;
        w.rlva_sitting_tick();
        assert_eq!(notices(&mut w), ["/unsat ground illegally"]);
    }

    #[test]
    fn rlva_delayed_ground_sit_rechecks_new_locks_and_yields_to_stand_returns_and_disable() {
        let mut w = world();
        seated(&mut w, true);
        w.rlva_sitting_tick();
        let stand = w.rlva_force_ground_sit(A).unwrap();
        w.rlva.note_sit_command(&stand, A);
        restrict(&mut w, B, "sit=n");
        seated(&mut w, false);
        assert!(w.rlva_sitting_tick().is_empty(), "a later sit lock cancels the delayed action");
        restrict(&mut w, B, "clear");
        assert!(w.rlva_sitting_tick().is_empty());
        seated(&mut w, true);
        w.rlva_sitting_tick();
        w.rlva_force_ground_sit(A);
        seated(&mut w, false);
        w.rlva.stand_return = Some((42, glam::Vec3::ZERO));
        assert!(w.rlva_sitting_tick().is_empty());
        w.rlva.stand_return = None;
        assert!(w.rlva_sitting_tick().is_empty(), "standtp does not leave a latent ground sit");
        seated(&mut w, true);
        w.rlva_force_ground_sit(A);
        w.rlva.configure(false);
        seated(&mut w, false);
        assert!(w.rlva_sitting_tick().is_empty());
        assert!(w.rlva.sitting.pending_ground.is_none());
    }

    #[test]
    fn rlva_sit_queries_use_root_ids_and_forced_object_sits_obey_their_own_locks() {
        let mut w = world();
        let request = NetCommand::RequestSit {
            handle: w.main_region.unwrap(),
            target: CHILD,
            offset: glam::Vec3::ZERO,
        };
        assert!(w.rlva_allows_command(&request, A), "force bypasses distance only");
        restrict(&mut w, A, "sit=n");
        assert!(!w.rlva_allows_command(&request, A));
        assert!(!w.rlva_allows_command(&NetCommand::OneShotControl(control::SIT_ON_GROUND), Uuid::nil()));
        assert!(
            w.rlva_allows_command(&NetCommand::OneShotControl(control::SIT_ON_GROUND), A),
            "ground-sit force has the reference's actor exception"
        );
        notices(&mut w);
        seated(&mut w, true);
        w.rlva_sitting_tick();
        assert_eq!(w.rlva_sit_id(), ROOT);
        assert_eq!(notices(&mut w), [format!("/sat object illegally {CHILD}")]);
        restrict(&mut w, A, "clear");
        restrict(&mut w, A, "unsit=n");
        assert!(
            !w.rlva_allows_command(&request, A),
            "forced object sit cannot switch seats under its own unsit lock"
        );
        notices(&mut w);
        restrict(&mut w, A, &format!("sit:{}=force", Uuid::nil()));
        assert!(matches!(&w.rlva_pending()[..], [(actor, NetCommand::OneShotControl(control::STAND_UP))] if *actor == A));
        seated(&mut w, false);
        assert!(w.rlva_sit_id().is_nil());
        restrict(&mut w, A, "unsit=force");
        assert!(w.rlva_pending().is_empty(), "standing already is a no-op");
        restrict(&mut w, A, "sitground=n");
        assert!(!w.rlva.has("sitground"), "sitground is force-only in the reference dictionary");
    }
}

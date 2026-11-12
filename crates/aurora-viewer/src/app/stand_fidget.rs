//! A standing avatar shifts its weight now and then: every 8 to 20 s the
//! viewer asks the simulator for one of the four stock stand variants, or
//! none, so everyone sees it. Port of LLAgent::fidget and
//! LLAgent::stopFidget (indra/newview/llagent.cpp), called from
//! LLVOAvatar::idleUpdate while a stand is signalled
//! (indra/newview/llvoavatar.cpp), and of LLAgent::onAnimStop, which stops
//! the variant when the stand ends.
//!
//! Aurora leaves it out while its own AO is on, whose stands replace the
//! stock one (a scripted AO that overrides Standing already removes the
//! stock stand, so the rule below covers it too), and while sitting.

use super::*;
use uuid::Uuid;

/// MIN_FIDGET_TIME and MAX_FIDGET_TIME (seconds).
const MIN_FIDGET: f32 = 8.0;
const MAX_FIDGET: f32 = 20.0;

/// ANIM_AGENT_STAND_1 to ANIM_AGENT_STAND_4 (llanimationstates.cpp).
pub(super) const VARIANTS: [Uuid; 4] = [
    uuid::uuid!("15468e00-3400-bb66-cecc-646d7c14458e"),
    uuid::uuid!("370f3a20-6ca6-9971-848c-9a01bc42ae3c"),
    uuid::uuid!("42b46214-4b44-79ae-deb8-0df61424ff4b"),
    uuid::uuid!("f22fed8b-a5ed-2c93-64d5-bdd8b93c889f"),
];

/// mCurrentFidget and mNextFidgetTime.
#[derive(Default)]
pub(super) struct Fidget {
    /// 0 the plain stand, 1 to 4 a variant.
    current: usize,
    /// None: due at once, as mNextFidgetTime starts at 0.
    next: Option<Instant>,
}

/// What the simulator is asked for: the four variants stopped, then this
/// one started, if any (stopFidget then sendAnimationRequest).
#[derive(Debug, PartialEq)]
pub(super) struct Change {
    pub start: Option<Uuid>,
}

impl Fidget {
    /// One frame. `standing`: the stock stand is signalled for us;
    /// `blocked`: our AO is on or we sit, so no variant may play; `away`:
    /// no new variant while away (getAFK), the one playing goes on.
    /// `pick` gives the variant (0 to 4) and a fraction of the 8–20 s span.
    pub fn step(&mut self, standing: bool, blocked: bool, away: bool, now: Instant, pick: impl FnOnce() -> (usize, f32)) -> Option<Change> {
        if !standing || blocked {
            // onAnimStop(ANIM_AGENT_STAND) -> stopFidget
            let playing = std::mem::take(&mut self.current) != 0;
            return playing.then_some(Change { start: None });
        }
        if away || self.next.is_some_and(|next| now < next) {
            return None;
        }
        let (variant, fraction) = pick();
        let variant = variant.min(VARIANTS.len());
        self.next = Some(now + Duration::from_secs_f32(MIN_FIDGET + fraction.clamp(0.0, 1.0) * (MAX_FIDGET - MIN_FIDGET)));
        if variant == self.current {
            return None;
        }
        self.current = variant;
        Some(Change {
            start: variant.checked_sub(1).map(|i| VARIANTS[i]),
        })
    }
}

/// ll_rand(NUM_AGENT_STAND_ANIMS) and ll_frand.
fn random_pick() -> (usize, f32) {
    let r = Uuid::new_v4().as_u128();
    (
        ((r >> 64) as u64 % (VARIANTS.len() as u64 + 1)) as usize,
        (r as u32) as f32 / u32::MAX as f32,
    )
}

impl App {
    /// A frame of the stand variants, once the animations of the frame are in.
    pub(super) fn update_stand_fidget(&mut self, now: Instant) {
        let me = self.world.agent_id;
        // AURORA_DEMO_FIDGET=1: the demo's simulator plays its own idle
        // animation; from frame 60 it signals the stock stand instead, so
        // the variants start (the log gives each request)
        if self.demo && self.frame_count >= 60 && std::env::var_os("AURORA_DEMO_FIDGET").is_some() {
            let anims = self.world.animations.entry(me).or_default();
            if !anims.iter().any(|a| a.id == crate::own_motion::ANIM_STAND) {
                anims.push(crate::world::PlayingAnimation::from_signal(
                    crate::own_motion::ANIM_STAND,
                    1,
                    now,
                    None,
                ));
            }
        }
        let standing = self.in_world()
            && self
                .world
                .animations
                .get(&me)
                .is_some_and(|anims| anims.iter().any(|a| a.id == crate::own_motion::ANIM_STAND));
        let ao = self.settings.ao.accounts.get(&me).is_some_and(|a| a.enabled);
        let blocked = ao || self.world.agent.is_sitting();
        let Some(change) = self.stand_fidget.step(standing, blocked, self.world.status.away, now, random_pick) else {
            return;
        };
        log::info!(
            "stand fidget: {}",
            change.start.map_or("plain stand".to_owned(), |id| format!("variant {id}"))
        );
        self.send(NetCommand::AgentAnimations {
            anims: VARIANTS.to_vec(),
            start: false,
        });
        if let Some(anim) = change.start {
            self.send(NetCommand::AgentAnimation { anim, start: true });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_standing_avatar_takes_a_variant_then_waits_8_to_20_seconds() {
        let t0 = Instant::now();
        let mut f = Fidget::default();
        // due at once, as mNextFidgetTime starts at 0
        assert_eq!(
            f.step(true, false, false, t0, || (2, 0.0)),
            Some(Change { start: Some(VARIANTS[1]) })
        );
        assert_eq!(
            f.step(true, false, false, t0 + Duration::from_secs(7), || (3, 0.0)),
            None,
            "8 s at least"
        );
        assert_eq!(
            f.step(true, false, false, t0 + Duration::from_secs(8), || (0, 1.0)),
            Some(Change { start: None })
        );
        // the fraction 1 gives the full 20 s
        assert_eq!(f.step(true, false, false, t0 + Duration::from_secs(27), || (4, 0.0)), None);
        assert_eq!(
            f.step(true, false, false, t0 + Duration::from_secs(28), || (4, 0.0)),
            Some(Change { start: Some(VARIANTS[3]) })
        );
    }

    #[test]
    fn the_same_pick_sends_nothing_and_away_keeps_the_variant() {
        let t0 = Instant::now();
        let mut f = Fidget::default();
        assert_eq!(
            f.step(true, false, false, t0, || (0, 0.0)),
            None,
            "the plain stand again: nothing to send"
        );
        let t1 = t0 + Duration::from_secs(9);
        assert!(f.step(true, false, false, t1, || (1, 0.0)).is_some());
        assert_eq!(f.step(true, false, true, t1 + Duration::from_secs(30), || (2, 0.0)), None, "away");
    }

    #[test]
    fn walking_the_ao_or_sitting_stop_the_variant_once() {
        let t0 = Instant::now();
        for (standing, blocked) in [(false, false), (true, true)] {
            let mut f = Fidget::default();
            f.step(true, false, false, t0, || (3, 0.0));
            assert_eq!(f.step(standing, blocked, false, t0, || (1, 0.0)), Some(Change { start: None }));
            assert_eq!(f.step(standing, blocked, false, t0, || (1, 0.0)), None, "once");
        }
        // nothing to stop when none played
        let mut f = Fidget::default();
        assert_eq!(f.step(false, false, false, t0, || (1, 0.0)), None);
    }
}

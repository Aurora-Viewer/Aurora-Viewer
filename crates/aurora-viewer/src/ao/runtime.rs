//! AOEngine::override / cycle / checkBelowWater / foreignAnimations.
//! Raw simulator signals remain authoritative for movement and transitions;
//! only our rendered pose is remapped, and AgentAnimation broadcasts changes.

use super::*;
use crate::world::PlayingAnimation;
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub struct Frame<'a> {
    pub raw: &'a [PlayingAnimation],
    /// Movement enhancer's bounded ground-contact grace; never changes raw signals.
    pub run_hold: Option<PlayingAnimation>,
    pub swim_hold: Option<(State, PlayingAnimation)>,
    /// Hands a stock run back to ordinary running without a stop/start flicker.
    pub native_running: bool,
    pub enabled: bool,
    pub mouselook: bool,
    pub on_object: bool,
    pub seat_animations: bool,
    pub underwater: bool,
    pub region: Option<u64>,
}

#[derive(Default)]
struct Cursor {
    index: usize,
    signal: Option<Instant>,
    next: Option<Instant>,
    manual: bool,
    cycle: Option<Cycle>,
}

pub struct Runtime {
    pub current: Option<State>,
    pub paused: bool,
    pub stands: bool,
    set: Uuid,
    region: Option<u64>,
    cursors: HashMap<(State, Uuid), Cursor>,
    active: Vec<PlayingAnimation>,
    owned: HashMap<Uuid, Instant>,
    completed: HashSet<Uuid>,
    run_transition: Option<Instant>,
    run_asserted: Option<Instant>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            current: None,
            paused: false,
            stands: true,
            set: Uuid::nil(),
            region: None,
            cursors: HashMap::new(),
            active: Vec::new(),
            owned: HashMap::new(),
            completed: HashSet::new(),
            run_transition: None,
            run_asserted: None,
        }
    }
}

impl Runtime {
    pub fn index(&self, state: State, slot: Uuid) -> usize {
        self.cursors.get(&(state, slot)).map_or(0, |c| c.index)
    }
    pub fn choose(&mut self, state: State, slot: Uuid, index: usize) {
        let c = self.cursors.entry((state, slot)).or_default();
        c.index = index;
        c.next = None;
        c.manual = true;
    }
    pub fn finished(&mut self, id: Uuid) {
        if self.owned.contains_key(&id) {
            self.completed.insert(id);
            self.active.retain(|a| a.id != id);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn pick(
        &mut self,
        state: State,
        slot: Uuid,
        len: usize,
        cycle: &Cycle,
        signal: Instant,
        now: Instant,
        roll: &mut impl FnMut(usize) -> usize,
    ) -> Option<(usize, bool)> {
        if len == 0 {
            return None;
        }
        let c = self.cursors.entry((state, slot)).or_default();
        c.index = c.index.min(len - 1);
        let entered = c.signal != Some(signal);
        let timeout = c.next.is_some_and(|t| now >= t);
        if (entered || timeout) && cycle.cycle && !c.manual {
            c.index = if cycle.random { roll(len) % len } else { (c.index + 1) % len };
        }
        let changed = entered || timeout || c.manual;
        let config_changed = c.cycle.as_ref() != Some(cycle);
        if changed || config_changed {
            c.signal = Some(signal);
            c.next = if cycle.cycle && cycle.seconds.is_finite() && cycle.seconds > 0.0 {
                now.checked_add(Duration::from_secs_f32(cycle.seconds.clamp(0.01, 86400.0)))
            } else {
                None
            };
            c.manual = false;
            c.cycle = Some(cycle.clone());
        }
        Some((c.index, changed))
    }

    fn state_assets(
        &mut self,
        set: &Set,
        state: State,
        signal: Instant,
        now: Instant,
        roll: &mut impl FnMut(usize) -> usize,
        desired: &mut Vec<Uuid>,
    ) {
        let Some(config) = set.states.get(&state) else {
            return;
        };
        if let Some((index, changed)) = self.pick(state, config.folder, config.steps.len(), &config.cycle, signal, now, roll) {
            if changed {
                for a in &config.steps[index].members {
                    self.completed.remove(&a.asset);
                }
            }
            desired.extend(config.steps[index].members.iter().map(|a| a.asset).filter(|id| !id.is_nil()));
        }
        for track in &config.tracks {
            if let Some((index, changed)) = self.pick(state, track.id, track.members.len(), &track.cycle, signal, now, roll) {
                let asset = track.members[index].asset;
                if changed {
                    self.completed.remove(&asset);
                }
                if !asset.is_nil() {
                    desired.push(asset);
                }
            }
        }
    }

    pub fn update(
        &mut self,
        set: Option<&Set>,
        frame: Frame<'_>,
        now: Instant,
        mut ready: impl FnMut(&Uuid) -> bool,
        mut roll: impl FnMut(usize) -> usize,
    ) -> (Vec<PlayingAnimation>, Vec<(Uuid, bool)>) {
        let selected = set.map_or(Uuid::nil(), |s| s.id);
        if self.set != selected {
            self.cursors.clear();
            self.completed.clear();
            self.set = selected;
        }
        let reassert = frame.region != self.region;
        self.region = frame.region;
        let mut primary = frame
            .raw
            .iter()
            .filter_map(|a| {
                let state = State::from_motion(a.id)?;
                (state != State::Typing && !STAND_VARIANTS.contains(&a.id)).then_some((state, *a))
            })
            .max_by_key(|(state, a)| (a.continuous_start, *state as usize));
        // Standing variants accompany stand; never steal the locomotion state.
        if primary.is_none() {
            primary = frame
                .raw
                .iter()
                .find(|a| STAND_VARIANTS.contains(&a.id))
                .map(|a| (State::Standing, *a));
        }
        if let Some(signal) = frame.run_hold {
            primary = Some((State::Running, signal));
        }
        if let Some((state, signal)) = frame.swim_hold {
            // Swim states share stock UUIDs with flight; keep the explicit state.
            primary = Some((state, signal));
        }
        self.current = primary.map(|(state, _)| state);
        let mut desired = Vec::new();
        let mut remap = Vec::new();
        let mut states = HashSet::new();
        let running = frame.enabled && !self.paused;
        if let Some(set) = set.filter(|_| running) {
            if let Some((base, a)) = primary {
                let state = if frame.underwater {
                    let swim = match base {
                        State::Hovering => Some(State::UnderwaterFloating),
                        State::Flying | State::FlyingSlow => Some(State::SwimmingForward),
                        State::FlyingUp => Some(State::SwimmingUp),
                        State::FlyingDown => Some(State::SwimmingDown),
                        _ => None,
                    };
                    swim.filter(|s| set.states.get(s).is_some_and(|c| !c.steps.is_empty() || !c.tracks.is_empty()))
                        .unwrap_or(base)
                } else {
                    base
                };
                self.current = Some(state);
                let allowed = match base {
                    State::Standing => self.stands && !(set.mouselook && frame.mouselook),
                    State::TurningRight | State::TurningLeft => self.stands,
                    State::Sitting => set.sits && !(set.smart && frame.seat_animations),
                    State::GroundSit => !frame.on_object,
                    _ => true,
                };
                if allowed {
                    let start = desired.len();
                    self.state_assets(set, state, a.continuous_start, now, &mut roll, &mut desired);
                    if desired.len() > start {
                        states.insert(state);
                        remap.push((base, desired[start..].to_vec()));
                        if base == State::Sitting {
                            desired.push(SIT_GENERIC);
                        }
                    }
                }
            }
            if let Some(a) = frame.raw.iter().find(|a| a.id == State::Typing.motion()) {
                let start = desired.len();
                self.state_assets(set, State::Typing, a.continuous_start, now, &mut roll, &mut desired);
                if desired.len() > start {
                    states.insert(State::Typing);
                    remap.push((State::Typing, desired[start..].to_vec()));
                }
            }
            // Always is independent of the locomotion state and its timers.
            let signal = self
                .cursors
                .get(&(State::Always, Uuid::nil()))
                .and_then(|c| c.signal)
                .unwrap_or(now);
            self.cursors.entry((State::Always, Uuid::nil())).or_default().signal = Some(signal);
            self.state_assets(set, State::Always, signal, now, &mut roll, &mut desired);
            states.insert(State::Always);
        }
        if !running
            && let Some(set) = set.filter(|set| set.swimming_slots() == State::SWIMMING.len())
            && let Some((state, signal)) = frame.swim_hold.filter(|(state, _)| state.swimming())
        {
            // Active swimming owns its slots even when the regular AO
            // is off/paused. Broadcast through the same AgentAnimation owner.
            self.state_assets(set, state, signal.continuous_start, now, &mut roll, &mut desired);
            states.insert(state);
            remap.push((state, desired.clone()));
        }
        let run_assets = remap
            .iter()
            .find(|(state, _)| *state == State::Running)
            .map(|(_, assets)| assets.as_slice())
            .unwrap_or_default();
        let handoff = frame.native_running && self.active.iter().any(|a| a.id == State::Running.motion());
        let fallback_run = (frame.run_hold.is_some() || handoff) && !run_assets.iter().any(&mut ready);
        if fallback_run {
            // This belongs to the enhancer even when Aurora AO is disabled.
            // AgentAnimation makes the standard run visible to other viewers.
            desired.push(State::Running.motion());
        }
        let fall = frame
            .raw
            .iter()
            .filter(|a| a.id == State::Falling.motion())
            .map(|a| a.continuous_start)
            .max();
        let run_reassert = frame.run_hold.is_some()
            && ((fall.is_some() && fall != self.run_transition)
                || self
                    .run_asserted
                    .is_some_and(|last| now.saturating_duration_since(last) >= Duration::from_millis(500)));
        self.run_transition = frame.run_hold.and(fall);
        if frame.run_hold.is_none() {
            self.run_asserted = None;
        }
        let mut seen = HashSet::new();
        desired.retain(|id| seen.insert(*id) && !self.completed.contains(id));
        let mut commands = Vec::new();
        for a in &self.active {
            if !desired.contains(&a.id) {
                commands.push((a.id, false));
            }
        }
        let previous = std::mem::take(&mut self.active);
        for id in desired {
            let old = previous.iter().find(|a| a.id == id);
            let is_run = frame.run_hold.is_some() && (run_assets.contains(&id) || (fallback_run && id == State::Running.motion()));
            let missing_run = is_run && !frame.raw.iter().any(|a| a.id == id);
            if old.is_none() || reassert || (is_run && run_reassert && (fall.is_some() || missing_run)) {
                commands.push((id, true));
                if is_run {
                    self.run_asserted = Some(now);
                }
            }
            self.active.push(PlayingAnimation::from_signal(id, 0, now, old));
            self.owned.insert(id, now);
        }
        self.owned.retain(|id, seen| {
            if self.active.iter().any(|a| a.id == *id) || frame.raw.iter().any(|a| a.id == *id) {
                *seen = now;
                true
            } else {
                now.saturating_duration_since(*seen) < Duration::from_secs(2)
            }
        });
        // Unlike a private AO asset, the stock run is also simulator-owned.
        // A late stop echo must not hide a later ordinary run indefinitely.
        if !self.active.iter().any(|a| a.id == State::Running.motion()) {
            self.owned.remove(&State::Running.motion());
        }
        let loaded: Vec<_> = self.active.iter().copied().filter(|a| ready(&a.id)).collect();
        let held_pose = frame.run_hold.is_some() && loaded.iter().any(|a| run_assets.contains(&a.id) || a.id == State::Running.motion());
        let swimming_pose = frame.swim_hold.is_some()
            && remap
                .iter()
                .any(|(state, assets)| state.swimming() && loaded.iter().any(|a| assets.contains(&a.id)));
        let mut pose: Vec<_> = frame
            .raw
            .iter()
            .copied()
            .filter(|a| {
                if self.owned.contains_key(&a.id) {
                    return false;
                }
                let state = State::from_motion(a.id);
                if held_pose
                    && matches!(
                        state,
                        Some(State::Standing | State::Walking | State::Running | State::Falling | State::TurningLeft | State::TurningRight)
                    )
                {
                    return false;
                }
                if swimming_pose
                    && state.is_some_and(|state| {
                        !matches!(state, State::Typing | State::Always | State::Sitting | State::GroundSit) && !state.transient()
                    })
                {
                    return false;
                }
                // Preserve LL transition motions and their finish control flags.
                // Retain the stock pose while the replacement is downloading.
                !state.is_some_and(|s| {
                    !s.transient()
                        && remap
                            .iter()
                            .any(|(base, assets)| *base == s && loaded.iter().any(|a| assets.contains(&a.id)))
                }) && !(a.id == SIT_FEMALE
                    && remap
                        .iter()
                        .any(|(base, assets)| *base == State::Sitting && loaded.iter().any(|a| assets.contains(&a.id))))
            })
            .collect();
        pose.extend(loaded);
        // Re-entering after an observed stop restarts its cycle. An active
        // swim context must keep its cursor with the ordinary AO off too, so
        // a finished non-looping entry/exit clip cannot restart every frame.
        for ((state, _), cursor) in &mut self.cursors {
            if !states.contains(state) {
                cursor.signal = None;
                cursor.next = None;
            }
        }
        (pose, commands)
    }
}

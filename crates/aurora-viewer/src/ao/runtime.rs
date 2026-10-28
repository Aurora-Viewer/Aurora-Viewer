//! AOEngine::override / cycle / checkBelowWater / foreignAnimations.
//! Raw simulator signals remain authoritative for movement and transitions;
//! only our rendered pose is remapped, and AgentAnimation broadcasts changes.

use super::*;
use crate::world::PlayingAnimation;
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

pub struct Frame<'a> {
    pub raw: &'a [PlayingAnimation],
    /// Movement enhancer's bounded ground-contact grace; never changes raw signals.
    pub run_hold: Option<PlayingAnimation>,
    pub swim_hold: Option<(State, PlayingAnimation)>,
    pub water_walk_hold: Option<(State, PlayingAnimation)>,
    /// Our native Movelock owns a real ground sit, with a verified standing AO.
    pub anchor_hold: Option<PlayingAnimation>,
    /// An authored post-loop water recovery owns the pose until its tail ends.
    pub swim_outro: bool,
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
    assertions: HashMap<Uuid, Instant>,
    preload_key: Option<(Uuid, u64)>,
    preload: VecDeque<Uuid>,
    raw_hold: Option<(State, PlayingAnimation)>,
    suppressed: HashMap<Uuid, PlayingAnimation>,
    restoring: Option<(State, PlayingAnimation, Instant)>,
    anchor_blend: super::anchor_blend::Blend,
    anchor_signal: Option<PlayingAnimation>,
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
            assertions: HashMap::new(),
            preload_key: None,
            preload: VecDeque::new(),
            raw_hold: None,
            suppressed: HashMap::new(),
            restoring: None,
            anchor_blend: Default::default(),
            anchor_signal: None,
        }
    }
}

impl Runtime {
    pub fn anchor_prepared(&self, region: Option<u64>) -> bool {
        self.region == region && self.anchor_blend.prepared()
    }
    pub fn suppressed_locomotion(&self) -> Option<PlayingAnimation> {
        self.raw_hold
            .map(|(_, signal)| signal)
            .filter(|a| self.suppressed.contains_key(&a.id))
    }
    /// Bound speculative asset requests per frame. The catalogue's inventory
    /// revision rebuilds the queue only when assignments may have changed.
    pub fn prefetch(&mut self, set: Option<&Set>, revision: u64, mut load: impl FnMut(&Uuid)) {
        let key = set.map(|s| (s.id, revision));
        if self.preload_key != key {
            self.preload_key = key;
            self.preload.clear();
            let mut seen = HashSet::new();
            if let Some(set) = set {
                for state in State::ALL {
                    if let Some(config) = set.animation_config(state) {
                        self.preload.extend(
                            config
                                .steps
                                .iter()
                                .flat_map(|s| &s.members)
                                .chain(config.tracks.iter().flat_map(|t| &t.members))
                                .map(|a| a.asset)
                                .filter(|id| !id.is_nil() && seen.insert(*id)),
                        );
                    }
                }
            }
        }
        for _ in 0..4 {
            let Some(id) = self.preload.pop_front() else { break };
            load(&id);
        }
    }

    pub fn selected_animations<'a>(&'a self, set: &'a Set, state: State) -> impl Iterator<Item = &'a Animation> {
        let config = set.animation_config(state);
        let main = config
            .and_then(|c| c.steps.get(self.index(state, c.folder)))
            .into_iter()
            .flat_map(|s| &s.members);
        let tracks = config
            .into_iter()
            .flat_map(|c| &c.tracks)
            .filter_map(move |t| t.members.get(self.index(state, t.id)));
        main.chain(tracks).filter(|a| !a.asset.is_nil())
    }

    /// Repair a missing simulator echo only for an active looping clip. A
    /// finished one-shot must never be replayed merely because it disappeared.
    pub fn reconcile_loops(&mut self, raw: &[PlayingAnimation], now: Instant, mut looping: impl FnMut(&Uuid) -> bool) -> Vec<(Uuid, bool)> {
        let mut changes = Vec::new();
        for active in &self.active {
            let id = active.id;
            if !raw.iter().any(|a| a.id == id)
                && self
                    .assertions
                    .get(&id)
                    .is_some_and(|last| now.saturating_duration_since(*last) >= Duration::from_secs(2))
                && looping(&id)
            {
                self.assertions.insert(id, now);
                changes.push((id, true));
            }
        }
        changes
    }

    pub fn index(&self, state: State, slot: Uuid) -> usize {
        self.cursors.get(&(state, slot)).map_or(0, |c| c.index)
    }
    pub fn choose(&mut self, state: State, slot: Uuid, index: usize) {
        let c = self.cursors.entry((state, slot)).or_default();
        c.index = index;
        c.next = None;
        c.manual = true;
    }
    /// AOEngine::cycle(CyclePrevious / CycleNext): step the main list of
    /// `state` by hand, wrapping at both ends. Like its resetTimer, the
    /// state's automatic cycle restarts from the chosen animation.
    pub fn step(&mut self, set: &Set, state: State, forward: bool) -> bool {
        let Some(config) = set.animation_config(state) else { return false };
        let len = config.steps.len();
        if len < 2 {
            return false;
        }
        let index = self.index(state, config.folder).min(len - 1);
        let index = if forward { (index + 1) % len } else { (index + len - 1) % len };
        self.choose(state, config.folder, index);
        true
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
        let Some(config) = set.animation_config(state) else {
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

    #[cfg(test)]
    pub fn update(
        &mut self,
        set: Option<&Set>,
        frame: Frame<'_>,
        now: Instant,
        ready: impl FnMut(&Uuid) -> bool,
        roll: impl FnMut(usize) -> usize,
    ) -> (Vec<PlayingAnimation>, Vec<(Uuid, bool)>) {
        self.update_with_easing(set, frame, now, ready, |_| Some(Duration::ZERO), roll)
    }

    pub fn update_with_easing(
        &mut self,
        set: Option<&Set>,
        frame: Frame<'_>,
        now: Instant,
        mut ready: impl FnMut(&Uuid) -> bool,
        mut ease_in: impl FnMut(&Uuid) -> Option<Duration>,
        mut roll: impl FnMut(usize) -> usize,
    ) -> (Vec<PlayingAnimation>, Vec<(Uuid, bool)>) {
        let previous_state = self.current;
        let selected = set.map_or(Uuid::nil(), |s| s.id);
        let same_set = self.set == selected;
        if self.set != selected {
            self.cursors.clear();
            self.completed.clear();
            self.set = selected;
        }
        let reassert = frame.region != self.region;
        self.region = frame.region;
        if reassert {
            self.anchor_blend = Default::default();
            self.anchor_signal = None;
            self.raw_hold = None;
            self.suppressed.clear();
            self.restoring = None;
        }
        self.restoring = self
            .restoring
            .filter(|(state, _, _)| !frame.on_object || (*state == State::Sitting && !frame.seat_animations));
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
        if let Some((state, mut signal)) = primary {
            if self.suppressed.contains_key(&signal.id)
                && let Some((_, old)) = self.raw_hold.filter(|(old_state, old)| *old_state == state && old.id == signal.id)
            {
                signal.continuous_start = old.continuous_start;
            }
            self.restoring = None;
            primary = Some((state, signal));
        } else {
            // Our own default-animation stop echo must not erase the state
            // that selected the AO. A new stock signal still takes precedence.
            primary = self
                .raw_hold
                .filter(|(state, signal)| self.suppressed.contains_key(&signal.id) && (!frame.on_object || *state == State::Sitting))
                .or_else(|| {
                    self.restoring
                        .filter(|(_, _, sent)| now.saturating_duration_since(*sent) < Duration::from_secs(2))
                        .map(|(state, signal, _)| (state, signal))
                });
        }
        self.raw_hold = primary;
        if let Some(signal) = frame.run_hold {
            primary = Some((State::Running, signal));
        }
        if let Some((state, signal)) = frame.swim_hold {
            // Swim states share stock UUIDs with flight; keep the explicit state.
            primary = Some((state, signal));
        }
        if let Some(hold) = frame.water_walk_hold {
            primary = Some(hold);
        }
        if let Some(signal) = frame.anchor_hold {
            let signal = *self.anchor_signal.get_or_insert_with(|| {
                // Keep an existing stand's cycle/phase when the anchor takes
                // over. A new clock here used to cycle it at each sit echo.
                let start = set
                    .and_then(|s| s.animation_config(State::Standing))
                    .and_then(|c| self.cursors.get(&(State::Standing, c.folder)))
                    .and_then(|c| c.signal)
                    .filter(|_| previous_state == Some(State::Standing))
                    .unwrap_or(signal.continuous_start);
                PlayingAnimation::from_signal(State::Standing.motion(), 0, start, None)
            });
            primary = Some((State::Standing, signal));
        } else {
            self.anchor_signal = None;
        }
        self.current = primary.map(|(state, _)| state);
        let mut desired = Vec::new();
        let mut remap = Vec::new();
        let mut states = HashSet::new();
        let running = (frame.anchor_hold.is_some() || (frame.enabled && !self.paused)) && !frame.swim_outro;
        if let Some(set) = set.filter(|_| running) {
            if let Some((base, a)) = primary {
                let state = if frame.underwater {
                    let swim = match base {
                        State::Hovering => Some(State::UnderwaterFloating),
                        State::Flying | State::FlyingSlow => Some(State::UnderwaterSwimming),
                        State::FlyingUp => Some(State::SwimmingUp),
                        State::FlyingDown => Some(State::SwimmingDown),
                        _ => None,
                    };
                    swim.filter(|s| {
                        set.animation_config(*s)
                            .is_some_and(|c| !c.steps.is_empty() || !c.tracks.is_empty())
                    })
                    .unwrap_or(base)
                } else {
                    base
                };
                self.current = Some(state);
                let allowed = match base {
                    State::Standing => frame.anchor_hold.is_some() || (self.stands && !(set.mouselook && frame.mouselook)),
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
            && !frame.swim_outro
            && let Some(set) = set
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
        if let Some((state, _)) = frame.water_walk_hold {
            let replacement = remap.iter().find(|(base, _)| *base == state);
            if !replacement.is_some_and(|(_, assets)| assets.iter().any(&mut ready)) {
                desired.push(state.motion());
            }
        }
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
        let water_loop = |state| {
            matches!(
                state,
                Some(
                    State::Floating
                        | State::SwimmingForward
                        | State::UnderwaterSwimming
                        | State::UnderwaterFloating
                        | State::SwimmingUp
                        | State::SwimmingDown
                )
            )
        };
        let holding = same_set
            && !self.active.is_empty()
            && !desired.is_empty()
            && (previous_state == self.current || (water_loop(previous_state) && water_loop(self.current)))
            && desired.iter().any(|id| !ready(id));
        if holding {
            // Keep the last complete selection while a cycle/continuous swim
            // replacement downloads. Network and local selection change together.
            desired = self.active.iter().map(|a| a.id).collect();
        }
        let manual_seat = frame.on_object || (frame.anchor_hold.is_none() && frame.raw.iter().any(|a| a.id == State::GroundSit.motion()));
        let protect = frame.anchor_hold.is_some() || (running && !manual_seat && frame.swim_hold.is_none());
        if protect {
            let candidate = remap.first().map(|(_, ids)| ids.as_slice()).unwrap_or_default();
            let phase = if frame.anchor_hold.is_some() {
                super::anchor_blend::Pose::Anchored
            } else if self.current == Some(State::Standing) {
                super::anchor_blend::Pose::Standing
            } else {
                super::anchor_blend::Pose::Moving
            };
            desired.extend(
                self.anchor_blend
                    .update(phase, candidate, &self.active, frame.raw, now, &mut ease_in),
            );
        } else {
            self.anchor_blend = Default::default();
        }
        seen.clear();
        desired.retain(|id| seen.insert(*id));
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
                self.assertions.insert(id, now);
                if is_run {
                    self.run_asserted = Some(now);
                }
            }
            self.active.push(PlayingAnimation::from_signal(id, 0, now, old));
            self.owned.insert(id, now);
        }
        self.assertions.retain(|id, _| self.active.iter().any(|a| a.id == *id));
        self.owned.retain(|id, seen| {
            if self.active.iter().any(|a| a.id == *id) || frame.raw.iter().any(|a| a.id == *id) {
                *seen = now;
                true
            } else {
                now.saturating_duration_since(*seen) < Duration::from_secs(2)
            }
        });
        // Stock fallbacks also belong to the simulator. Releasing our hold
        // must not hide a later ordinary stand, walk, run or jump as an echo.
        for state in [
            State::Standing,
            State::Walking,
            State::Running,
            State::Crouching,
            State::CrouchWalk,
            State::Jumping,
            State::Falling,
        ] {
            if !self.active.iter().any(|a| a.id == state.motion()) {
                self.owned.remove(&state.motion());
            }
        }
        let loaded: Vec<_> = self.active.iter().copied().filter(|a| ready(&a.id)).collect();
        let held_pose = frame.run_hold.is_some() && loaded.iter().any(|a| run_assets.contains(&a.id) || a.id == State::Running.motion());
        let walking_pose = frame.water_walk_hold.is_some_and(|(state, _)| {
            loaded.iter().any(|a| a.id == state.motion())
                || remap
                    .iter()
                    .any(|(base, ids)| *base == state && loaded.iter().any(|a| ids.contains(&a.id)))
        });
        let swimming_pose = (frame.swim_hold.is_some()
            && ((holding && water_loop(self.current))
                || remap
                    .iter()
                    .any(|(state, assets)| state.swimming() && loaded.iter().any(|a| assets.contains(&a.id)))))
            || walking_pose;
        let mut pose: Vec<_> = frame
            .raw
            .iter()
            .copied()
            .filter(|a| {
                if self.owned.contains_key(&a.id) {
                    return false;
                }
                if frame.anchor_hold.is_some()
                    && a.id == State::GroundSit.motion()
                    && remap
                        .iter()
                        .any(|(state, assets)| *state == State::Standing && loaded.iter().any(|a| assets.contains(&a.id)))
                {
                    // Mask only the local pose. Keep the original system signal
                    // on the simulator and other viewers for actual sitting.
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
                        !matches!(
                            state,
                            State::Typing | State::Always | State::Sitting | State::GroundSit | State::Jumping
                        ) && !state.transient()
                    })
                {
                    return false;
                }
                // Preserve LL transition motions and their finish control flags.
                // Retain the stock pose while the replacement is downloading.
                // A replaced jump hides too: LLVOAvatar::startMotion never
                // starts an overridden motion locally, so a low priority AO
                // jump is not masked by the stock one (#1284).
                !state.is_some_and(|s| {
                    !s.transient()
                        && ((holding && previous_state == Some(s))
                            || remap
                                .iter()
                                .any(|(base, assets)| *base == s && loaded.iter().any(|a| assets.contains(&a.id))))
                }) && !(a.id == SIT_FEMALE
                    && remap
                        .iter()
                        .any(|(base, assets)| *base == State::Sitting && loaded.iter().any(|a| assets.contains(&a.id))))
            })
            .collect();
        pose.extend(loaded);
        // Suppress replaced standard clips for observers too, rather than
        // depending on upload priorities. Keep jump/landing control motions
        // intact (requestStopMotion finish flags), as in the Legacy AO.
        let steady = |state| {
            !matches!(
                state,
                State::Jumping
                    | State::PreJumping
                    | State::Typing
                    | State::GroundSit
                    | State::Landing
                    | State::SoftLanding
                    | State::StandingUp
            )
        };
        let overrides = |state| {
            steady(state)
                && (swimming_pose
                    || (held_pose
                        && matches!(
                            state,
                            State::Standing | State::Walking | State::Running | State::Falling | State::TurningLeft | State::TurningRight
                        ))
                    || (holding && previous_state == Some(state))
                    || remap
                        .iter()
                        .any(|(base, assets)| *base == state && pose.iter().any(|a| assets.contains(&a.id))))
        };
        let mut suppressed = HashMap::new();
        if let Some((state, signal)) = self.raw_hold {
            if overrides(state) && !self.active.iter().any(|a| a.id == signal.id) {
                let old = self.suppressed.get(&signal.id).copied().unwrap_or_else(|| {
                    commands.push((signal.id, false));
                    signal
                });
                suppressed.insert(signal.id, old);
                self.restoring = None;
            } else if self.suppressed.contains_key(&signal.id) && !(state == State::Sitting && frame.on_object && frame.seat_animations) {
                commands.push((signal.id, true));
                self.restoring = Some((state, signal, now));
            }
        }
        for signal in frame.raw {
            if State::from_motion(signal.id).is_some_and(overrides) && !self.active.iter().any(|a| a.id == signal.id) {
                if self
                    .suppressed
                    .get(&signal.id)
                    .is_none_or(|old| old.sequence_start != signal.sequence_start)
                {
                    commands.push((signal.id, false));
                }
                suppressed.insert(signal.id, *signal);
            }
        }
        self.suppressed = suppressed;
        if let Some((_, signal, _)) = self.restoring
            && !pose.iter().any(|a| a.id == signal.id)
        {
            pose.push(signal);
        }
        commands.sort_by_key(|(_, start)| *start);
        let mut unique_changes = HashSet::new();
        commands.retain(|change| unique_changes.insert(*change));
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

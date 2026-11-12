//! When the viewer says "Your avatar complexity is N", and how many people
//! around may not see us fully: the rules of LLAvatarRenderNotifier
//! (indra/newview/llavatarrendernotifier.cpp, originally LGPL 2.1) for the
//! notices AgentComplexity and AgentComplexityWithVisibility. The counts
//! come from the regions' `AvatarRenderInfo` answers (aurora_net's
//! render_info.rs), our complexity from the scene's refresh.
//!
//! Pure: the caller gives the time, the setting and the camera, and shows
//! what [`Notifier`] returns.

use aurora_net::render_info::Visibility;
use std::time::{Duration, Instant};

/// RENDER_ALLOWED_CHANGE_PCT: the share of viewers not showing us must move
/// by more than a tenth of itself for a new notice.
const RENDER_ALLOWED_CHANGE: f32 = 0.1;
/// OVER_LIMIT_UPDATE_DELAY: after a change of our complexity (and at
/// start), the regions' counts wait this long, for them to catch up.
pub const OVER_LIMIT_UPDATE_DELAY: Duration = Duration::from_secs(70);
/// ComplexityChangesPopUpDelay (settings.xml, a debug setting): the time
/// between two "may not see you" notices.
pub const POP_UP_DELAY: Duration = Duration::from_secs(300);
/// The notices of our own complexity come in bursts (Aurora): the values of
/// an outfit forming replace the notice still out for this long, then none
/// shows before [`POP_UP_DELAY`] is over. The Legacy Viewer tells each
/// change at once; a complexity moving on its own showed a notice every
/// few seconds (#3381).
pub const OWN_BURST: Duration = Duration::from_secs(30);

/// The second line of AgentComplexityWithVisibility
/// (LLAvatarRenderNotifier::overLimitMessage).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverLimit {
    /// av_render_everyone_now (shown only after the count was above 0).
    EveryoneNow,
    NotEveryone,
    OverHalf,
    Most,
    Anyone,
}

impl OverLimit {
    fn of(percent: f32) -> OverLimit {
        if percent >= 99.0 {
            OverLimit::Anyone
        } else if percent >= 75.0 {
            OverLimit::Most
        } else if percent >= 50.0 {
            OverLimit::OverHalf
        } else if percent > 10.0 {
            OverLimit::NotEveryone
        } else {
            OverLimit::EveryoneNow
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            OverLimit::EveryoneNow => aurora_i18n::tr!("avatar-render-everyone-now"),
            OverLimit::NotEveryone => aurora_i18n::tr!("avatar-render-not-everyone"),
            OverLimit::OverHalf => aurora_i18n::tr!("avatar-render-over-half"),
            OverLimit::Most => aurora_i18n::tr!("avatar-render-most-of"),
            OverLimit::Anyone => aurora_i18n::tr!("avatar-render-anyone"),
        }
    }
}

/// A notice to show, replacing the one still out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Notice {
    pub complexity: u32,
    /// AgentComplexityWithVisibility's line; None for AgentComplexity.
    pub over_limit: Option<OverLimit>,
    /// How long it stays (ShowMyComplexityChanges).
    pub seconds: u32,
}

/// What the viewer does with the notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Show(Notice),
    /// The other notice still out goes, with nothing in its place (a
    /// change of kind while the new one may not show).
    Cancel,
}

/// What the notices depend on, given at each call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context {
    /// ShowMyComplexityChanges, seconds (0: no notices).
    pub seconds: u32,
    /// No notice in mouselook.
    pub mouselook: bool,
}

/// Our avatar as the scene last measured it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnAvatar {
    /// LLVOAvatar::mVisualComplexity.
    pub complexity: u32,
    /// The outfit's wearables known (areWearablesLoaded): our appearance
    /// came.
    pub wearables_loaded: bool,
    /// What we wear all there (isAttachmentStateComplete): the outfit has
    /// formed.
    pub outfit_complete: bool,
    /// The outfit's version (LLAppearanceMgr::getCOFVersion): it changes
    /// with every outfit change.
    pub cof_version: i32,
    /// Our rezzed status (`Scene::avatar_rez_status`): it drops when an
    /// outfit change starts.
    pub rez_status: u8,
}

#[derive(Debug, Clone, Copy)]
struct Shown {
    with_visibility: bool,
    until: Instant,
}

/// LLAvatarRenderNotifier.
#[derive(Debug)]
pub struct Notifier {
    agents_count: u32,
    over_limit_agents: u32,
    agent_complexity: u32,
    over_limit_pct: f32,
    latest_agents_count: u32,
    latest_over_limit_agents: u32,
    latest_agent_complexity: u32,
    latest_over_limit_pct: f32,
    /// The last notice shown was the "may not see you" one.
    show_over_limit_agents: bool,
    /// The outfit changed since it first formed: complexity changes show.
    notify_outfit_loading: bool,
    last_cof_version: i32,
    last_outfit_rez_status: i32,
    pop_up_delay_until: Instant,
    /// [`OWN_BURST`]: the end of the current burst of our complexity's
    /// notices, and the time the next burst may start.
    own_burst_until: Instant,
    own_quiet_until: Instant,
    /// The notice still out (mNotificationPtr while active).
    shown: Option<Shown>,
}

impl Notifier {
    pub fn new(now: Instant) -> Self {
        Notifier {
            agents_count: 0,
            over_limit_agents: 0,
            agent_complexity: 0,
            over_limit_pct: 0.0,
            latest_agents_count: 0,
            latest_over_limit_agents: 0,
            latest_agent_complexity: 0,
            latest_over_limit_pct: 0.0,
            show_over_limit_agents: false,
            notify_outfit_loading: false,
            last_cof_version: -1,
            last_outfit_rez_status: -1,
            pop_up_delay_until: now + OVER_LIMIT_UPDATE_DELAY,
            own_burst_until: now,
            own_quiet_until: now,
            shown: None,
        }
    }

    fn visible(&self, now: Instant) -> bool {
        self.shown.is_some_and(|s| now < s.until)
    }

    /// displayNotification.
    fn display(&mut self, show_over_limit: bool, now: Instant, cx: Context) -> Option<Action> {
        self.agent_complexity = self.latest_agent_complexity;
        self.show_over_limit_agents = show_over_limit;
        let over_limit = show_over_limit.then(|| {
            // remember what the situation was, to notify only on a change
            self.agents_count = self.latest_agents_count;
            self.over_limit_agents = self.latest_over_limit_agents;
            self.over_limit_pct = self.latest_over_limit_pct;
            OverLimit::of(self.latest_over_limit_pct)
        });
        // the other kind of notice still out is cancelled by hand (the
        // unique tag only replaces the same notification)
        let cancel = self.shown.is_some_and(|s| s.with_visibility != show_over_limit && now < s.until);
        if cancel {
            self.shown = None;
        }
        if cx.seconds == 0 || cx.mouselook {
            return cancel.then_some(Action::Cancel);
        }
        self.shown = Some(Shown {
            with_visibility: show_over_limit,
            until: now + Duration::from_secs(u64::from(cx.seconds)),
        });
        Some(Action::Show(Notice {
            complexity: self.latest_agent_complexity,
            over_limit,
            seconds: cx.seconds,
        }))
    }

    /// updateNotificationRegion: a region's counts. Shown when the share of
    /// viewers not showing us went to or from zero, or moved by more than a
    /// tenth, once the pop-up delay is over (or while the last such notice
    /// is still out).
    pub fn region_answer(&mut self, v: Visibility, now: Instant, cx: Context) -> Option<Action> {
        if v.reporting == 0 {
            // data not ready
            return None;
        }
        // (ourselves are counted, unless everyone is over the limit)
        self.latest_agents_count = if v.reporting > v.over_limit { v.reporting - 1 } else { v.reporting };
        self.latest_over_limit_agents = v.over_limit;
        self.latest_over_limit_pct = if self.latest_agents_count != 0 {
            v.over_limit as f32 / self.latest_agents_count as f32 * 100.0
        } else {
            0.0
        };
        if self.agents_count == self.latest_agents_count && self.over_limit_agents == self.latest_over_limit_agents {
            return None;
        }
        let ready = now >= self.pop_up_delay_until || (self.visible(now) && self.show_over_limit_agents);
        let changed = (self.over_limit_pct > 0.0 || self.latest_over_limit_pct > 0.0)
            && (self.over_limit_pct - self.latest_over_limit_pct).abs() > self.latest_over_limit_pct * RENDER_ALLOWED_CHANGE;
        if !(ready && changed) {
            return None;
        }
        let action = self.display(true, now, cx);
        self.pop_up_delay_until = now + POP_UP_DELAY;
        action
    }

    /// updateNotificationState: whether an outfit change started since the
    /// outfit first formed (a new outfit version, or our rezzed status
    /// dropping). The notices say nothing of the outfit we logged in with.
    fn track_outfit(&mut self, own: &OwnAvatar) {
        if self.last_cof_version < 0 && own.wearables_loaded && own.outfit_complete {
            self.last_cof_version = own.cof_version;
        } else if self.last_cof_version >= 0 && self.last_cof_version != own.cof_version {
            self.notify_outfit_loading = true;
            self.last_cof_version = own.cof_version;
        }
        let rez = i32::from(own.rez_status);
        if rez < self.last_outfit_rez_status {
            self.notify_outfit_loading = true;
        }
        self.last_outfit_rez_status = rez;
    }

    /// processComplexityCostChange → updateNotificationAgent: our
    /// complexity, after each refresh. A change shows AgentComplexity once
    /// the outfit changed since login, or while people already do not see
    /// us; the counts then wait [`OVER_LIMIT_UPDATE_DELAY`]. The changes
    /// come in bursts ([`OWN_BURST`]); one held back by the quiet time
    /// shows once it is over, with the complexity of then.
    pub fn own_avatar(&mut self, own: OwnAvatar, now: Instant, cx: Context) -> Option<Action> {
        // (LLVOAvatar::updateFullyLoaded runs updateNotificationState on its own)
        self.track_outfit(&own);
        if cx.seconds == 0 {
            return None;
        }
        self.latest_agent_complexity = own.complexity;
        if !own.wearables_loaded {
            return None;
        }
        if !self.notify_outfit_loading {
            if self.latest_over_limit_agents > 0 {
                // some already cannot see us: tell about the growth
                self.notify_outfit_loading = true;
            } else {
                // the outfit we arrived with, still loading: no notice
                self.agent_complexity = self.latest_agent_complexity;
                return None;
            }
        }
        if self.agent_complexity == self.latest_agent_complexity {
            return None;
        }
        let in_burst = now < self.own_burst_until && self.visible(now) && !self.show_over_limit_agents;
        if !in_burst {
            if now < self.own_quiet_until {
                return None;
            }
            self.own_burst_until = now + OWN_BURST;
            self.own_quiet_until = now + POP_UP_DELAY;
        }
        let action = self.display(false, now, cx);
        self.pop_up_delay_until = now + OVER_LIMIT_UPDATE_DELAY;
        action
    }
}

#[cfg(test)]
mod tests;

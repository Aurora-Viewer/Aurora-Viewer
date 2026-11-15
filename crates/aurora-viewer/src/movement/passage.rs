//! Match physical outcomes to the requested action; receipt alone is not
//! success. Who may pass through an object at all.
use crate::world::land::BuildRight;
use aurora_net::bridge::PassageReport;
use std::time::{Duration, Instant};

/// Why a passage is refused before anything is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Neither the object nor the land lets the agent through.
    NotAllowed,
    /// The object or a parcel is not known yet; it is asked for, so a
    /// later press can pass.
    Unknown,
}

impl Refusal {
    pub fn message(self) -> String {
        let reason = match self {
            Self::NotAllowed => aurora_i18n::tr!("movement-pass-no-rights"),
            Self::Unknown => aurora_i18n::tr!("movement-pass-checking"),
        };
        aurora_i18n::tr!("movement-pass-not-sent", reason = reason.to_owned())
    }
}

/// An object may be passed through when it is ours or we may edit it
/// (LLViewerObject::permYouOwner / permModify, read on the linkset root's
/// update flags, `None` while that root is not known), or when we may
/// build both where we stand and where we would come out
/// (LLViewerParcelMgr::allowAgentBuild, [`crate::world::land::agent_can_build`]).
/// The Legacy Viewer has no such action; this keeps it to places the agent
/// could change anyway. Missing data never allows.
pub fn rights(wall_root_flags: Option<u32>, here: BuildRight, there: BuildRight) -> Result<(), Refusal> {
    use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_YOU_OWNER};
    if wall_root_flags.is_some_and(|f| f & (OBJECT_YOU_OWNER | OBJECT_MODIFY) != 0) {
        return Ok(());
    }
    match (here, there) {
        (BuildRight::Allowed, BuildRight::Allowed) => Ok(()),
        // the object could still turn out to be ours
        (BuildRight::Denied, _) | (_, BuildRight::Denied) if wall_root_flags.is_some() => Err(Refusal::NotAllowed),
        _ => Err(Refusal::Unknown),
    }
}

#[derive(Default)]
pub struct Feedback {
    pending: Option<(u32, Instant)>,
    previous: Option<PassageReport>,
}
impl Feedback {
    pub fn start(&mut self, action: u32, now: Instant) {
        self.pending = Some((action, now + Duration::from_secs(5)));
        self.previous = None;
    }
    pub fn update(&mut self, report: Option<PassageReport>, now: Instant) -> Option<String> {
        let (action, deadline) = self.pending?;
        if let Some(report) = report.filter(|report| report.action == action && (now < deadline || report.finished())) {
            if report.finished() {
                self.pending = None;
            }
            if self.previous != Some(report) {
                self.previous = Some(report);
                let state = aurora_i18n::text(match report.state {
                    1 => "movement-pass-accepted",
                    2 => "movement-pass-driving",
                    3 => "movement-pass-reached",
                    4 => "movement-pass-not-reached",
                    5 => "movement-pass-cancelled",
                    6 => "movement-pass-locked",
                    _ => "movement-run-invalid-mass",
                });
                return Some(aurora_i18n::tr!(
                    "movement-pass-feedback",
                    state = state,
                    progress = format!("{:.2}", report.progress_cm as f32 / 100.0),
                    remaining = format!("{:.2}", report.remaining_cm as f32 / 100.0),
                    speed = format!("{:.1}", report.speed_cm as f32 / 100.0)
                ));
            }
        }
        if now >= deadline {
            self.pending = None;
            return Some(aurora_i18n::tr!("movement-pass-no-result").into());
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::context::flags::{OBJECT_ANY_OWNER, OBJECT_COPY, OBJECT_MODIFY, OBJECT_YOU_OWNER};
    use BuildRight::{Allowed, Denied, Unknown};

    #[test]
    fn an_object_we_own_or_may_edit_lets_us_through_on_any_land() {
        for flags in [OBJECT_YOU_OWNER, OBJECT_MODIFY, OBJECT_YOU_OWNER | OBJECT_MODIFY] {
            for land in [Allowed, Denied, Unknown] {
                assert_eq!(rights(Some(flags), land, land), Ok(()), "{flags:#x} {land:?}");
            }
        }
    }

    #[test]
    fn someone_elses_object_needs_build_rights_on_both_sides() {
        let theirs = Some(OBJECT_ANY_OWNER | OBJECT_COPY);
        assert_eq!(rights(theirs, Allowed, Allowed), Ok(()));
        assert_eq!(rights(theirs, Denied, Allowed), Err(Refusal::NotAllowed), "where we stand");
        assert_eq!(rights(theirs, Allowed, Denied), Err(Refusal::NotAllowed), "where we come out");
        assert_eq!(rights(theirs, Denied, Denied), Err(Refusal::NotAllowed));
    }

    #[test]
    fn unknown_land_or_object_refuses_until_it_is_known() {
        let theirs = Some(0);
        assert_eq!(rights(theirs, Allowed, Unknown), Err(Refusal::Unknown));
        assert_eq!(rights(theirs, Unknown, Allowed), Err(Refusal::Unknown));
        assert_eq!(rights(theirs, Unknown, Denied), Err(Refusal::NotAllowed), "a known refusal wins");
        // the root is not known: only land open to us on both sides passes
        assert_eq!(rights(None, Allowed, Allowed), Ok(()));
        assert_eq!(rights(None, Denied, Allowed), Err(Refusal::Unknown));
        assert_eq!(rights(None, Unknown, Unknown), Err(Refusal::Unknown));
    }

    #[test]
    fn refusals_say_why() {
        assert_ne!(Refusal::NotAllowed.message(), Refusal::Unknown.message());
        assert!(
            Refusal::NotAllowed
                .message()
                .contains(&*aurora_i18n::tr!("movement-pass-no-rights"))
        );
    }
}

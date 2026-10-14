//! Experiences seen by the agent: their details (the part of
//! LLExperienceCache the viewer needs), the permission questions waiting
//! for them, and the agent's permission for each one (allow, block, forget),
//! shown in the experience profile. Firestorm (indra/newview, originally LGPL
//! 2.1): process_script_question asks LLExperienceCache::get before adding
//! ScriptQuestionExperience (process_script_experience_details), and
//! LLFloaterExperienceProfile reads and changes the permission.

use aurora_net::NetCommand;
use aurora_net::experience::{ExperienceDetails, ExperiencePermission, ExperiencePermissionChange};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// A permission question from an experience, waiting for its details.
#[derive(Debug, Clone, PartialEq)]
pub struct ExperienceQuestion {
    pub task: Uuid,
    pub item: Uuid,
    pub object_name: String,
    pub owner_name: String,
    pub questions: i32,
    pub experience: Uuid,
    pub region: aurora_net::RegionHandle,
}

#[derive(Debug, Default)]
pub struct Experiences {
    pub details: HashMap<Uuid, ExperienceDetails>,
    requested: HashSet<Uuid>,
    waiting: Vec<ExperienceQuestion>,
    /// The agent's permission, once read or changed.
    pub permissions: HashMap<Uuid, ExperiencePermission>,
    /// A permission request in flight (its buttons wait).
    pub busy: HashSet<Uuid>,
    /// The last permission request that failed.
    pub errors: HashMap<Uuid, String>,
    /// The experience shown in the profile window.
    pub profile: Option<Uuid>,
    out: Vec<NetCommand>,
}

impl Experiences {
    /// The details, asked for once when unknown.
    pub fn want(&mut self, id: Uuid) -> Option<&ExperienceDetails> {
        if !self.details.contains_key(&id) && self.requested.insert(id) {
            self.out.push(NetCommand::ExperienceInfo(vec![id]));
        }
        self.details.get(&id)
    }

    /// A question from an experience: ready at once when its details are
    /// known, otherwise kept until they arrive.
    pub fn question(&mut self, question: ExperienceQuestion) -> Option<ExperienceQuestion> {
        if self.want(question.experience).is_some() {
            Some(question)
        } else {
            self.waiting.push(question);
            None
        }
    }

    /// Details arrived (unknown ones included): the questions they release.
    pub fn arrived(&mut self, list: Vec<ExperienceDetails>) -> Vec<ExperienceQuestion> {
        for d in list {
            self.requested.remove(&d.id);
            self.details.insert(d.id, d);
        }
        let (ready, waiting) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition(|q| self.details.contains_key(&q.experience));
        self.waiting = waiting;
        ready
    }

    /// Read or change the permission (the profile's Allow / Block / Forget,
    /// the question's "Block experience").
    pub fn change(&mut self, experience: Uuid, change: ExperiencePermissionChange) {
        self.busy.insert(experience);
        self.errors.remove(&experience);
        self.out.push(NetCommand::ExperiencePermission { experience, change });
    }

    /// The server's answer to `change`.
    pub fn permission(&mut self, experience: Uuid, result: Result<ExperiencePermission, String>) {
        self.busy.remove(&experience);
        match result {
            Ok(p) => {
                self.permissions.insert(experience, p);
            }
            Err(e) => {
                self.errors.insert(experience, e);
            }
        }
    }

    /// Open the profile: its details and the current permission.
    pub fn open_profile(&mut self, experience: Uuid) {
        self.profile = Some(experience);
        self.want(experience);
        self.change(experience, ExperiencePermissionChange::Query);
    }

    pub fn take_commands(&mut self) -> Vec<NetCommand> {
        std::mem::take(&mut self.out)
    }
}

/// process_script_question: a question that includes a caution permission
/// (take L$, privileged land access) gets the caution dialog even from an
/// experience (PermissionsCautionEnabled, on by default).
pub fn caution(questions: i32) -> bool {
    const DEBIT: i32 = 1 << 1;
    const PRIVILEGED_LAND_ACCESS: i32 = 1 << 19;
    questions & (DEBIT | PRIVILEGED_LAND_ACCESS) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(experience: Uuid) -> ExperienceQuestion {
        ExperienceQuestion {
            task: Uuid::from_u128(1),
            item: Uuid::from_u128(2),
            object_name: "Door".into(),
            owner_name: "Owner Resident".into(),
            questions: 1 << 12,
            experience,
            region: 7,
        }
    }

    #[test]
    fn a_question_waits_once_for_its_experience_then_is_released() {
        let mut xp = Experiences::default();
        let id = Uuid::from_u128(0xE);
        assert!(xp.question(question(id)).is_none());
        // a second question for the same experience asks nothing more
        assert!(xp.question(question(id)).is_none());
        assert!(matches!(&xp.take_commands()[..], [NetCommand::ExperienceInfo(ids)] if ids == &vec![id]));
        let released = xp.arrived(vec![ExperienceDetails::unknown(id)]);
        assert_eq!(released.len(), 2);
        assert!(released.iter().all(|q| q.region == 7 && q.experience == id));
        // known now: the next question is ready at once, without a request
        assert!(xp.question(question(id)).is_some());
        assert!(xp.take_commands().is_empty());
    }

    #[test]
    fn details_of_another_experience_release_nothing() {
        let mut xp = Experiences::default();
        let (a, b) = (Uuid::from_u128(0xA), Uuid::from_u128(0xB));
        assert!(xp.question(question(a)).is_none());
        assert!(xp.arrived(vec![ExperienceDetails::unknown(b)]).is_empty());
        assert_eq!(xp.arrived(vec![ExperienceDetails::unknown(a)]).len(), 1);
    }

    #[test]
    fn permission_changes_and_their_answers() {
        let mut xp = Experiences::default();
        let id = Uuid::from_u128(0xC);
        xp.open_profile(id);
        let cmds = xp.take_commands();
        assert!(cmds.iter().any(|c| matches!(c, NetCommand::ExperienceInfo(_))));
        assert!(cmds.iter().any(|c| matches!(
            c,
            NetCommand::ExperiencePermission { experience, change: ExperiencePermissionChange::Query } if *experience == id
        )));
        assert!(xp.busy.contains(&id));
        xp.permission(id, Ok(ExperiencePermission::Blocked));
        assert_eq!(xp.permissions.get(&id), Some(&ExperiencePermission::Blocked));
        assert!(!xp.busy.contains(&id));
        xp.change(id, ExperiencePermissionChange::Forget);
        xp.permission(id, Err("status 500".into()));
        // a failure keeps the last known permission and says why
        assert_eq!(xp.permissions.get(&id), Some(&ExperiencePermission::Blocked));
        assert!(xp.errors.contains_key(&id));
    }

    #[test]
    fn caution_permissions_skip_the_experience_dialog() {
        assert!(!caution(1 << 12 | 1 << 17 | 1 << 18));
        assert!(caution(1 << 1));
        assert!(caution(1 << 19 | 1 << 12));
    }
}

//! AURORA_DEMO_EXPERIENCE=1|auto|profile|blocked: an experience's teleport
//! door without a grid. A violet door stands right of the avatar; a click on
//! it makes its script call llRequestExperiencePermissions, like the doors
//! of a club: the simulator asks once (ScriptQuestion with the experience),
//! then, once the experience is allowed, teleports at every click without
//! asking. `auto` asks once the loading screen is gone, without a click, and
//! `profile` also opens the experience's profile; `blocked` starts with the experience blocked: the door only says so in
//! the chat, and its profile ("Forget") lets it ask again. The demo plays
//! the simulator and the capabilities: GetExperienceInfo,
//! ExperiencePreferences, ScriptAnswerYes.

use super::{HANDLE, height, prim, shape, te};
use aurora_net::NetCommand;
use aurora_net::NetEvent;
use aurora_net::experience::{ExperienceDetails, ExperiencePermission, ExperiencePermissionChange};
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec3};
use uuid::Uuid;

/// The demo experience (apart from the demo's other fixed ids).
pub const EXPERIENCE: Uuid = Uuid::from_u128(0xde7e_e8e0_0000_4000_8000_0000_0000_0001);
const LOCAL_ID: u32 = 82_000;
/// The door's script.
const ITEM: Uuid = Uuid::from_u128(0xde7e_e8e0_0000_4000_8000_0000_0000_0002);
const ARRIVE_FRAME: u64 = 300;
const AUTO_FRAME: u64 = 420;
/// PERMISSION_TELEPORT.
const TELEPORT: i32 = 1 << 12;
/// Where the door leads: up on the plaza's edge.
const DESTINATION: Vec3 = Vec3::new(150.0, 140.0, 60.0);

pub struct ExperienceDemo {
    auto: bool,
    profile: bool,
    /// The simulator's record of the agent's choice.
    permission: ExperiencePermission,
    placed: bool,
    asked: bool,
}

/// What a command leads to.
pub enum Reply {
    Events(Vec<NetEvent>),
    /// A message of the door's script in the local chat.
    Say(String),
    /// Nothing visible (logged).
    Nothing,
}

impl ExperienceDemo {
    pub fn from_env() -> Option<Self> {
        let value = std::env::var("AURORA_DEMO_EXPERIENCE").ok()?;
        let value = value.trim();
        Some(ExperienceDemo {
            auto: value == "auto" || value == "profile",
            profile: value == "profile",
            permission: if value == "blocked" {
                ExperiencePermission::Blocked
            } else {
                ExperiencePermission::Unset
            },
            placed: false,
            asked: false,
        })
    }

    /// Once a frame: the door, then in `auto` the question.
    pub fn tick(&mut self, frame: u64, world: &mut crate::world::World) {
        if !self.placed && frame >= ARRIVE_FRAME {
            self.placed = true;
            world.apply(door(world.agent.position));
            log::info!("demo experience: the door stands, experience {:?}", self.permission);
        }
        if self.auto && self.placed && !self.asked && frame >= AUTO_FRAME {
            self.asked = true;
            world.apply(question());
            if self.profile {
                world.experiences.open_profile(EXPERIENCE);
            }
        }
    }

    /// The simulator's and the capabilities' answers to `cmd`, None when
    /// the command is not the demo's.
    pub fn reply(&mut self, cmd: &NetCommand) -> Option<Reply> {
        match cmd {
            NetCommand::ObjectGrab { local_id, .. } if *local_id == LOCAL_ID => Some(self.touched()),
            NetCommand::ObjectRelease { local_id, .. } if *local_id == LOCAL_ID => Some(Reply::Nothing),
            NetCommand::ScriptAnswer { item_id, questions, .. } if *item_id == ITEM => {
                log::info!("demo experience: ScriptAnswerYes {questions:#x}");
                if questions & TELEPORT != 0 {
                    // the simulator records the grant, the script teleports
                    self.permission = ExperiencePermission::Allowed;
                    Some(Reply::Events(teleport()))
                } else {
                    Some(Reply::Say(aurora_i18n::tr!("demo-experience-door-declined").into()))
                }
            }
            NetCommand::ExperienceInfo(ids) if ids.contains(&EXPERIENCE) => Some(Reply::Events(vec![NetEvent::ExperienceInfo(
                ids.iter().map(|id| details(*id)).collect(),
            )])),
            NetCommand::ExperiencePermission { experience, change } if *experience == EXPERIENCE => {
                self.permission = match change {
                    ExperiencePermissionChange::Query => self.permission,
                    ExperiencePermissionChange::Allow => ExperiencePermission::Allowed,
                    ExperiencePermissionChange::Block => ExperiencePermission::Blocked,
                    ExperiencePermissionChange::Forget => ExperiencePermission::Unset,
                };
                log::info!("demo experience: ExperiencePreferences {change:?} -> {:?}", self.permission);
                Some(Reply::Events(vec![NetEvent::ExperiencePermission {
                    experience: EXPERIENCE,
                    change: *change,
                    result: Ok(self.permission),
                }]))
            }
            _ => None,
        }
    }

    /// touch_start: llRequestExperiencePermissions. The simulator asks only
    /// when the agent has not chosen yet.
    fn touched(&mut self) -> Reply {
        match self.permission {
            ExperiencePermission::Unset => Reply::Events(vec![question()]),
            ExperiencePermission::Allowed => Reply::Events(teleport()),
            ExperiencePermission::Blocked => Reply::Say(aurora_i18n::tr!("demo-experience-door-blocked").into()),
        }
    }
}

/// The door's object id (`prim` derives it from the local id).
pub fn object_id() -> Uuid {
    Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | LOCAL_ID as u128)
}

fn question() -> NetEvent {
    NetEvent::ScriptQuestion {
        task_id: object_id(),
        item_id: ITEM,
        object_name: aurora_i18n::tr!("demo-experience-door").into(),
        owner_name: "Pixel Boutique Resident".into(),
        questions: TELEPORT,
        experience: EXPERIENCE,
        region: HANDLE,
    }
}

fn details(id: Uuid) -> ExperienceDetails {
    if id != EXPERIENCE {
        return ExperienceDetails::unknown(id);
    }
    ExperienceDetails {
        id,
        name: aurora_i18n::tr!("demo-experience-name").into(),
        description: aurora_i18n::tr!("demo-experience-description").into(),
        properties: 0,
        maturity: 13,
        owner: Uuid::from_u128(0x2222),
        group: Uuid::nil(),
        slurl: "secondlife:///app/region/Aurora%20Demo/150/140/60".into(),
    }
}

/// llTeleportAgent inside the region: TeleportLocal, then the arrival.
fn teleport() -> Vec<NetEvent> {
    vec![
        NetEvent::TeleportLocal { flying: false },
        NetEvent::AgentMovementComplete {
            handle: HANDLE,
            position: DESTINATION,
            look_at: Vec3::X,
        },
    ]
}

/// Where the door stands: in front of the camera, left of the avatar.
pub const DOOR_XY: (f32, f32) = (136.0, 128.5);

/// A tall violet panel right of the avatar, with its hover text. Its script
/// handles touch (FLAGS_HANDLE_TOUCH): without the flag a left click sends
/// nothing, as for any object without a touch event.
fn door(avatar: Vec3) -> NetEvent {
    let (x, y) = DOOR_XY;
    let mut object = prim(
        LOCAL_ID,
        Vec3::new(x, y, avatar.z.max(height(x, y) + 1.1)),
        Quat::IDENTITY,
        Vec3::new(0.1, 1.2, 2.2),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te([0.55, 0.36, 0.96, 1.0], 0, false, 0.05),
        ExtraParams::default(),
        aurora_i18n::tr!("demo-experience-door-hover"),
    );
    object.update_flags = crate::ui::context::flags::HANDLE_TOUCH;
    object.owner_id = Uuid::from_u128(0x2222);
    NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![object],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grab() -> NetCommand {
        NetCommand::ObjectGrab {
            handle: HANDLE,
            local_id: LOCAL_ID,
            offset: Vec3::ZERO,
            surface: Default::default(),
        }
    }

    #[test]
    fn the_door_handles_touch_so_a_left_click_reaches_it() {
        let NetEvent::ObjectUpdates { objects, .. } = door(Vec3::new(132.0, 126.0, 33.0)) else {
            panic!("an object update");
        };
        assert_eq!(objects[0].full_id, object_id());
        assert_ne!(objects[0].update_flags & crate::ui::context::flags::HANDLE_TOUCH, 0);
    }

    #[test]
    fn the_door_asks_once_then_teleports_and_a_blocked_one_only_talks() {
        let mut demo = ExperienceDemo {
            auto: false,
            profile: false,
            permission: ExperiencePermission::Unset,
            placed: true,
            asked: false,
        };
        assert!(matches!(demo.reply(&grab()), Some(Reply::Events(e)) if matches!(e[..], [NetEvent::ScriptQuestion { .. }])));
        let answer = NetCommand::ScriptAnswer {
            task_id: object_id(),
            item_id: ITEM,
            questions: TELEPORT,
            region: HANDLE,
        };
        assert!(matches!(demo.reply(&answer), Some(Reply::Events(e)) if e.len() == 2));
        // allowed now: the next click teleports at once
        assert!(matches!(demo.reply(&grab()), Some(Reply::Events(e)) if matches!(e[..], [NetEvent::TeleportLocal { .. }, _])));
        let block = NetCommand::ExperiencePermission {
            experience: EXPERIENCE,
            change: ExperiencePermissionChange::Block,
        };
        assert!(demo.reply(&block).is_some());
        assert!(matches!(demo.reply(&grab()), Some(Reply::Say(_))));
        let forget = NetCommand::ExperiencePermission {
            experience: EXPERIENCE,
            change: ExperiencePermissionChange::Forget,
        };
        demo.reply(&forget);
        assert!(matches!(demo.reply(&grab()), Some(Reply::Events(e)) if matches!(e[..], [NetEvent::ScriptQuestion { .. }])));
        // other objects and experiences are not the demo's
        assert!(demo.reply(&NetCommand::ExperienceInfo(vec![Uuid::from_u128(5)])).is_none());
    }
}

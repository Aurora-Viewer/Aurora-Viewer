//! AURORA_DEMO_EXPERIENCE=1|auto|profile|blocked|walk|arrive: experiences
//! asking for permissions without a grid. In SL, llRequestExperiencePermissions
//! can follow any event of the script: a click (touch_start), walking into
//! an object or a phantom volume (collision_start, volume_detect), arriving
//! on the parcel, a timer… The viewer only ever sees the simulator's
//! ScriptQuestion, which must show whatever the cause, even for an object out
//! of view. The demo plays the simulator, GetExperienceInfo and
//! ExperiencePreferences:
//! - `1`: a violet club door in front of the camera; a click asks once, then,
//!   once the experience is allowed, every click teleports without asking;
//!   `auto` asks without a click, `profile` also opens the experience's
//!   profile, `blocked` starts with the experience blocked (the door only
//!   says so in the chat; the profile's "Forget" lets it ask again);
//! - `walk`: a translucent teal pad in front of the avatar (a phantom
//!   volume): walking into it asks, then teleports;
//! - `arrive`: a land-scope experience of the parcel asks on arrival, from a
//!   greeter object the viewer has never seen.

use super::{HANDLE, floor_at, height, prim, shape, te};
use aurora_llsd::{Llsd, llsd_map};
use aurora_net::NetCommand;
use aurora_net::NetEvent;
use aurora_net::experience::{ExperienceDetails, ExperiencePermission, ExperiencePermissionChange};
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec3};
use std::collections::HashMap;
use uuid::Uuid;

/// The club's experience (door and pad), and the parcel's (greeter).
pub const EXPERIENCE: Uuid = Uuid::from_u128(0xde7e_e8e0_0000_4000_8000_0000_0000_0001);
pub const GREETER_EXPERIENCE: Uuid = Uuid::from_u128(0xde7e_e8e0_0000_4000_8000_0000_0000_0006);
const ARRIVE_FRAME: u64 = 300;
const AUTO_FRAME: u64 = 420;
/// PERMISSION_TELEPORT, PERMISSION_TRIGGER_ANIMATION, PERMISSION_CHANGE_ENVIRONMENT.
const TELEPORT: i32 = 1 << 12;
const ANIMATE: i32 = 1 << 4;
const ENVIRONMENT: i32 = 1 << 18;
/// Where the door and the pad lead: up on the plaza's edge.
const DESTINATION: Vec3 = Vec3::new(150.0, 140.0, 60.0);
/// Where the door stands: in front of the camera, left of the avatar.
pub const DOOR_XY: (f32, f32) = (136.0, 128.5);
/// The pad: in front of the avatar, on its way when it walks forward.
pub const PAD_XY: (f32, f32) = (135.5, 126.0);
const PAD_SIZE: (f32, f32) = (1.8, 2.2);

/// An object whose script asks for an experience's permissions.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Source {
    /// 0: an object the viewer does not know (out of view).
    local_id: u32,
    task: Uuid,
    item: Uuid,
    name: &'static str,
    experience: Uuid,
    questions: i32,
}

fn door_source() -> Source {
    Source {
        local_id: 82_000,
        task: prim_id(82_000),
        item: Uuid::from_u128(0xde7e_e8e0_0000_4000_8000_0000_0000_0002),
        name: aurora_i18n::tr!("demo-experience-door"),
        experience: EXPERIENCE,
        questions: TELEPORT,
    }
}

fn pad_source() -> Source {
    Source {
        local_id: 82_001,
        task: prim_id(82_001),
        item: Uuid::from_u128(0xde7e_e8e0_0000_4000_8000_0000_0000_0003),
        name: aurora_i18n::tr!("demo-experience-pad"),
        experience: EXPERIENCE,
        questions: TELEPORT,
    }
}

fn greeter_source() -> Source {
    Source {
        local_id: 0,
        task: Uuid::from_u128(0xde7e_e8e0_0000_4000_8000_0000_0000_0004),
        item: Uuid::from_u128(0xde7e_e8e0_0000_4000_8000_0000_0000_0005),
        name: aurora_i18n::tr!("demo-experience-greeter"),
        experience: GREETER_EXPERIENCE,
        questions: ANIMATE | ENVIRONMENT,
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Click,
    Auto,
    Profile,
    Walk,
    Arrive,
}

pub struct ExperienceDemo {
    mode: Mode,
    /// The simulator's record of the agent's choice, per experience.
    permissions: HashMap<Uuid, ExperiencePermission>,
    placed: bool,
    asked: bool,
    /// The avatar stands in the pad (collision_start on entering).
    inside: bool,
}

/// What a command or a frame leads to.
pub enum Reply {
    Events(Vec<NetEvent>),
    /// A message of the object's script in the local chat.
    Say(String),
    /// Nothing visible (logged).
    Nothing,
}

impl ExperienceDemo {
    pub fn from_env() -> Option<Self> {
        let value = std::env::var("AURORA_DEMO_EXPERIENCE").ok()?;
        let value = value.trim();
        let mode = match value {
            "auto" => Mode::Auto,
            "profile" => Mode::Profile,
            "walk" => Mode::Walk,
            "arrive" => Mode::Arrive,
            _ => Mode::Click,
        };
        let mut permissions = HashMap::new();
        if value == "blocked" {
            permissions.insert(EXPERIENCE, ExperiencePermission::Blocked);
        }
        Some(Self::new(mode, permissions))
    }

    fn new(mode: Mode, permissions: HashMap<Uuid, ExperiencePermission>) -> Self {
        ExperienceDemo {
            mode,
            permissions,
            placed: false,
            asked: false,
            inside: false,
        }
    }

    fn permission(&self, experience: Uuid) -> ExperiencePermission {
        self.permissions.get(&experience).copied().unwrap_or(ExperiencePermission::Unset)
    }

    /// Once a frame: the door or the pad, then what asks on its own (`auto`,
    /// `profile`, `arrive`) or because the avatar walked into the pad.
    pub fn tick(&mut self, frame: u64, world: &mut crate::world::World) -> Option<Reply> {
        if !self.placed && frame >= ARRIVE_FRAME {
            self.placed = true;
            match self.mode {
                Mode::Walk => {
                    world.apply(pad());
                }
                Mode::Arrive => {}
                _ => {
                    world.apply(door(world.agent.position));
                }
            }
            log::info!(
                "demo experience: {:?}, club experience {:?}",
                self.mode,
                self.permission(EXPERIENCE)
            );
        }
        if !self.placed {
            return None;
        }
        match self.mode {
            Mode::Auto | Mode::Profile | Mode::Arrive if !self.asked && frame >= AUTO_FRAME => {
                self.asked = true;
                let source = if self.mode == Mode::Arrive {
                    greeter_source()
                } else {
                    door_source()
                };
                if self.mode == Mode::Profile {
                    world.experiences.open_profile(EXPERIENCE);
                }
                log::info!("demo experience: {} asks without a click", source.name);
                Some(Reply::Events(vec![question(source)]))
            }
            Mode::Walk => {
                let inside = in_pad(world.agent.position);
                let entered = inside && !self.inside;
                self.inside = inside;
                // collision_start of the phantom volume
                entered.then(|| {
                    log::info!("demo experience: collision_start of the pad");
                    self.triggered(pad_source())
                })
            }
            _ => None,
        }
    }

    /// The simulator's and the capabilities' answers to `cmd`, None when
    /// the command is not the demo's.
    pub fn reply(&mut self, cmd: &NetCommand) -> Option<Reply> {
        let door = door_source();
        match cmd {
            NetCommand::ObjectGrab { local_id, .. } if *local_id == door.local_id => Some(self.triggered(door)),
            NetCommand::ObjectRelease { local_id, .. } if *local_id == door.local_id => Some(Reply::Nothing),
            NetCommand::ScriptAnswer { item_id, questions, .. } => {
                let source = [door_source(), pad_source(), greeter_source()]
                    .into_iter()
                    .find(|s| s.item == *item_id)?;
                log::info!("demo experience: ScriptAnswerYes {questions:#x} for {}", source.name);
                if *questions == 0 {
                    return Some(Reply::Say(aurora_i18n::tr!("demo-experience-declined", object = source.name)));
                }
                // the simulator records the grant, then the script acts
                self.permissions.insert(source.experience, ExperiencePermission::Allowed);
                Some(self.act(source))
            }
            NetCommand::ExperienceInfo(ids) if ids.iter().any(|id| *id == EXPERIENCE || *id == GREETER_EXPERIENCE) => Some(Reply::Events(
                vec![NetEvent::ExperienceInfo(ids.iter().map(|id| details(*id)).collect())],
            )),
            NetCommand::ExperiencePermission { experience, change } if *experience == EXPERIENCE || *experience == GREETER_EXPERIENCE => {
                let permission = match change {
                    ExperiencePermissionChange::Query => self.permission(*experience),
                    ExperiencePermissionChange::Allow => ExperiencePermission::Allowed,
                    ExperiencePermissionChange::Block => ExperiencePermission::Blocked,
                    ExperiencePermissionChange::Forget => ExperiencePermission::Unset,
                };
                self.permissions.insert(*experience, permission);
                log::info!("demo experience: ExperiencePreferences {change:?} -> {permission:?}");
                Some(Reply::Events(vec![NetEvent::ExperiencePermission {
                    experience: *experience,
                    change: *change,
                    result: Ok(permission),
                }]))
            }
            _ => None,
        }
    }

    /// llRequestExperiencePermissions from any event: the simulator asks
    /// only when the agent has not chosen yet.
    fn triggered(&mut self, source: Source) -> Reply {
        match self.permission(source.experience) {
            ExperiencePermission::Unset => Reply::Events(vec![question(source)]),
            ExperiencePermission::Allowed => self.act(source),
            ExperiencePermission::Blocked => Reply::Say(aurora_i18n::tr!("demo-experience-blocked", object = source.name)),
        }
    }

    /// What the script does once allowed.
    fn act(&self, source: Source) -> Reply {
        if source.questions & TELEPORT != 0 {
            Reply::Events(teleport())
        } else if source.questions & ENVIRONMENT != 0 {
            Reply::Events(vec![environment(source.experience)])
        } else {
            Reply::Say(aurora_i18n::tr!("demo-experience-greeter-done").into())
        }
    }
}

/// The id `prim` gives an object from its local id.
fn prim_id(local_id: u32) -> Uuid {
    Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | local_id as u128)
}

/// The door's object id.
#[cfg(test)]
fn object_id() -> Uuid {
    door_source().task
}

fn question(source: Source) -> NetEvent {
    NetEvent::ScriptQuestion {
        task_id: source.task,
        item_id: source.item,
        object_name: source.name.into(),
        owner_name: "Pixel Boutique Resident".into(),
        questions: source.questions,
        experience: source.experience,
        region: HANDLE,
    }
}

fn details(id: Uuid) -> ExperienceDetails {
    let (name, description, slurl) = if id == EXPERIENCE {
        (
            aurora_i18n::tr!("demo-experience-name"),
            aurora_i18n::tr!("demo-experience-description"),
            "secondlife:///app/region/Aurora%20Demo/150/140/60",
        )
    } else if id == GREETER_EXPERIENCE {
        (
            aurora_i18n::tr!("demo-experience-greeter-name"),
            aurora_i18n::tr!("demo-experience-greeter-description"),
            "secondlife:///app/region/Aurora%20Demo/128/128/30",
        )
    } else {
        return ExperienceDetails::unknown(id);
    };
    ExperienceDetails {
        id,
        name: name.into(),
        description: description.into(),
        properties: 0,
        maturity: 13,
        owner: Uuid::from_u128(0x2222),
        group: Uuid::nil(),
        slurl: slurl.into(),
    }
}

/// llSetEnvironment of a land experience: a violet sunset over the sky and a
/// red water fog, over 3 s (PushPartialEnvironment).
fn environment(experience: Uuid) -> NetEvent {
    let rgb = |r: f64, g: f64, b: f64| Llsd::Array(vec![r.into(), g.into(), b.into()]);
    NetEvent::ExperienceEnvironment {
        experience,
        message: llsd_map! {
            "action" => "PushPartialEnvironment",
            "action_data" => llsd_map! {
                "transition_time" => 3.0,
                "settings" => llsd_map! {
                    "sky" => llsd_map! {
                        "sunlight_color" => rgb(2.0, 0.6, 1.4),
                        "blue_horizon" => rgb(0.9, 0.2, 0.6),
                        "blue_density" => rgb(0.5, 0.1, 0.5),
                    },
                    "water" => llsd_map! { "water_fog_color" => rgb(0.6, 0.05, 0.05) },
                },
            },
        },
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

fn in_pad(avatar: Vec3) -> bool {
    let (x, y) = PAD_XY;
    let floor = floor_at(x, y);
    (avatar.x - x).abs() <= PAD_SIZE.0 / 2.0 && (avatar.y - y).abs() <= PAD_SIZE.1 / 2.0 && avatar.z - floor < 3.0
}

/// A tall violet panel, with its hover text. Its script handles touch
/// (FLAGS_HANDLE_TOUCH): without the flag a left click sends nothing, as
/// for any object without a touch event.
fn door(avatar: Vec3) -> NetEvent {
    let (x, y) = DOOR_XY;
    let source = door_source();
    let mut object = prim(
        source.local_id,
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

/// A flat translucent teal pad on the floor: a phantom volume
/// (llVolumeDetect) whose script asks when the avatar walks into it.
fn pad() -> NetEvent {
    let (x, y) = PAD_XY;
    let source = pad_source();
    let mut object = prim(
        source.local_id,
        Vec3::new(x, y, floor_at(x, y) + 0.04),
        Quat::IDENTITY,
        Vec3::new(PAD_SIZE.0, PAD_SIZE.1, 0.06),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te([0.37, 0.92, 0.83, 0.55], 0, true, 0.2),
        ExtraParams::default(),
        aurora_i18n::tr!("demo-experience-pad-hover"),
    );
    // FLAGS_PHANTOM: the avatar walks through it
    object.update_flags = 1 << 10;
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
            local_id: door_source().local_id,
            offset: Vec3::ZERO,
            surface: Default::default(),
        }
    }

    fn asks(reply: Option<Reply>) -> bool {
        matches!(reply, Some(Reply::Events(e)) if matches!(e[..], [NetEvent::ScriptQuestion { .. }]))
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
        let mut demo = ExperienceDemo::new(Mode::Click, HashMap::new());
        assert!(asks(demo.reply(&grab())));
        let answer = NetCommand::ScriptAnswer {
            task_id: object_id(),
            item_id: door_source().item,
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
        assert!(asks(demo.reply(&grab())));
        // other experiences are not the demo's
        assert!(demo.reply(&NetCommand::ExperienceInfo(vec![Uuid::from_u128(5)])).is_none());
    }

    #[test]
    fn walking_into_the_pad_asks_once_per_entry() {
        let mut demo = ExperienceDemo::new(Mode::Walk, HashMap::new());
        demo.placed = true;
        let (x, y) = PAD_XY;
        let step = |demo: &mut ExperienceDemo, pos: Vec3| {
            let entered = in_pad(pos) && !demo.inside;
            demo.inside = in_pad(pos);
            entered.then(|| demo.triggered(pad_source()))
        };
        let floor = floor_at(x, y);
        assert!(step(&mut demo, Vec3::new(x - 3.0, y, floor + 0.9)).is_none());
        assert!(asks(step(&mut demo, Vec3::new(x, y, floor + 0.9))));
        // standing in it asks nothing more
        assert!(step(&mut demo, Vec3::new(x + 0.2, y, floor + 0.9)).is_none());
        assert!(step(&mut demo, Vec3::new(x + 3.0, y, floor + 0.9)).is_none());
        assert!(asks(step(&mut demo, Vec3::new(x, y, floor + 0.9))));
    }

    #[test]
    fn the_greeter_asks_for_an_object_out_of_view_and_answers_in_the_chat() {
        let greeter = greeter_source();
        assert_eq!(greeter.local_id, 0);
        let mut demo = ExperienceDemo::new(Mode::Arrive, HashMap::new());
        let NetEvent::ScriptQuestion { experience, task_id, .. } = question(greeter) else {
            panic!("a question");
        };
        assert_eq!((experience, task_id), (GREETER_EXPERIENCE, greeter.task));
        let answer = NetCommand::ScriptAnswer {
            task_id,
            item_id: greeter.item,
            questions: greeter.questions,
            region: HANDLE,
        };
        assert!(matches!(
            demo.reply(&answer),
            Some(Reply::Events(events)) if matches!(events[..], [NetEvent::ExperienceEnvironment { experience: GREETER_EXPERIENCE, .. }])
        ));
        assert_eq!(demo.permission(GREETER_EXPERIENCE), ExperiencePermission::Allowed);
        assert!(!details(GREETER_EXPERIENCE).invalid());
    }
}

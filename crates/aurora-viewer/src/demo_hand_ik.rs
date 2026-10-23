//! Offline motorcycle contacts, driven through the real handshake/chat path.
use super::*;
use crate::scene::{Scene, anim::BoundAnim};
use crate::world::World;
use crate::world::hand_ik::{CHANNEL, PREFIX};
use aurora_assets::anim::{JointMotion, RotKey};
use aurora_net::{ChatMessage, ChatSourceType, ChatType, NetCommand};

pub const ROOT: u32 = 9840;
pub const LEFT: u32 = 9841;
pub const RIGHT: u32 = 9842;
const OTHER_ROOT: u32 = 9850;
const ANIM: Uuid = Uuid::from_u128(0x9840_aaaa);
const DRIVE: Uuid = Uuid::from_u128(0x9840_aaab);
const ACCEL: Uuid = Uuid::from_u128(0x9840_aaac);
pub const RIDER: Uuid = Uuid::from_u128(0x9840_bbbb);

pub fn properties() -> std::collections::HashMap<Uuid, aurora_net::build::ObjectProps> {
    [
        (ROOT, "Contact rig"),
        (LEFT, "Left"),
        (RIGHT, "Right"),
        (OTHER_ROOT, "Independent grip"),
    ]
    .into_iter()
    .map(|(id, name)| {
        let id = action_id(id);
        (
            id,
            aurora_net::build::ObjectProps {
                object_id: id,
                name: name.into(),
                ..Default::default()
            },
        )
    })
    .collect()
}

pub fn objects(angle: f32, seated: bool) -> Vec<ObjectUpdate> {
    let mut root = prim(
        ROOT,
        Vec3::new(132.0, 126.0, floor_at(132.0, 126.0) + 0.6),
        Quat::from_euler(glam::EulerRot::ZYX, angle * 0.3, angle * 0.2, angle),
        Vec3::new(1.5, 0.4, 0.3),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te([0.15, 0.12, 0.25, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    );
    root.owner_id = DEMO_AGENT;
    let mut left = root.clone();
    left.local_id = LEFT;
    left.full_id = action_id(LEFT);
    left.parent_id = ROOT;
    left.position = Vec3::new(0.05, 0.28, 1.0);
    left.rotation = Quat::from_rotation_z(-0.5);
    left.scale = Vec3::new(0.08, 0.18, 0.08);
    left.texture_entry = Some(te([0.15, 0.8, 0.5, 1.0], 0, false, 0.0));
    let mut right = left.clone();
    right.local_id = RIGHT;
    right.full_id = action_id(RIGHT);
    right.position.y = -left.position.y;
    right.rotation = Quat::from_rotation_z(0.5);
    right.texture_entry = Some(te([0.95, 0.4, 0.15, 1.0], 0, false, 0.0));
    let mut avatar = action_avatar(seated);
    if seated {
        avatar.parent_id = ROOT;
        avatar.position = Vec3::new(-0.18, 0.0, 0.55);
        avatar.rotation = Quat::IDENTITY;
    }
    vec![root, left, right, avatar]
}

pub fn reply(command: &NetCommand) -> Vec<NetEvent> {
    if let NetCommand::Build(aurora_net::build::BuildCmd::Select { local_ids, .. }) = command {
        let mut properties = properties();
        let props: Vec<_> = local_ids.iter().filter_map(|id| properties.remove(&action_id(*id))).collect();
        return if props.is_empty() {
            Vec::new()
        } else {
            vec![NetEvent::ObjectProperties(props)]
        };
    }
    if let NetCommand::RequestObjectProperties { object, .. } = command {
        let root = if [action_id(LEFT), action_id(RIGHT)].contains(object) {
            action_id(ROOT)
        } else {
            *object
        };
        if let Some(props) = properties().remove(&root) {
            return vec![NetEvent::ObjectProperties(vec![props])];
        }
    }
    let NetCommand::ScriptDialogReply {
        channel: CHANNEL, label, ..
    } = command
    else {
        return Vec::new();
    };
    let fields: Vec<_> = label.split('|').collect();
    if fields.len() == 5
        && matches!(fields[0], "auroraIKObserve" | "auroraIKEdit")
        && fields[1] == action_id(ROOT).to_string()
        && (fields[2] == RIDER.to_string() || fields[2] == DEMO_AGENT.to_string())
    {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let separate = matches!(std::env::var("AURORA_DEMO_HAND_IK").as_deref(), Ok("objects" | "observer-objects"));
        let bindings = [
            ("mWristLeft", "mShoulderLeft", LEFT, 0.055),
            ("mWristRight", "mShoulderRight", RIGHT, -0.055),
        ]
        .map(|(bone, root, link, y)| {
            let target = if separate && link == RIGHT {
                serde_json::json!({"kind":"link","object":action_id(OTHER_ROOT),"link":"_ROOT_"})
            } else {
                serde_json::json!({"kind":"link","object":"source","link":if link == LEFT { "Left" } else { "Right" }})
            };
            serde_json::json!({"bone":bone,"target":target,
                "root":root,"contact":[0.0,y,-0.015]})
        });
        let mut rig = serde_json::json!({"nodes":[],"bindings":bindings});
        let profiles = matches!(
            std::env::var("AURORA_DEMO_HAND_IK").as_deref(),
            Ok("profiles" | "observer-profiles")
        );
        if profiles {
            rig["profiles"] = serde_json::json!([
                {"name":"drive","bones":["mWristLeft","mWristRight"]},
                {"name":"gas_only","bones":["mWristRight"]}]);
        }
        let subject = if fields[2] == RIDER.to_string() { RIDER } else { DEMO_AGENT };
        let bytes = rig.to_string();
        let parts: Vec<_> = bytes.as_bytes().chunks(if profiles { 320 } else { 8192 }).collect();
        return parts
            .iter()
            .enumerate()
            .map(|(part, bytes)| {
                NetEvent::Chat(ChatMessage {
                    from_name: "Observed motorcycle contacts".into(),
                    source_id: action_id(ROOT),
                    owner_id: subject,
                    source_type: ChatSourceType::Object,
                    chat_type: ChatType::Other(9),
                    position: Vec3::ZERO,
                    message: format!(
                        "{PREFIX}{}",
                        serde_json::json!({
                            "avatar":subject,"nonce":fields[3],"revision":1,
                            "bundle":Uuid::from_u128(0x9840_eeee),"epoch":Uuid::from_u128(0x9840_ffff),
                            "enabled":true,"time":0.0,"profile":if profiles { "drive" } else { "" },
                            "cycles":if profiles { vec![1,1] } else { Vec::new() },
                            "part":part,"count":parts.len(),"data":STANDARD.encode(bytes)
                        })
                    ),
                })
            })
            .collect();
    }
    Vec::new()
}

pub fn tick(world: &mut World, scene: &mut Scene, frame: u64, mode: &str) {
    if frame < 300 {
        return;
    }
    let observer = mode.starts_with("observer");
    let profiles = matches!(mode, "profiles" | "observer-profiles");
    let separate = matches!(mode, "objects" | "observer-objects");
    if matches!(mode, "moving" | "observer") && matches!(frame, 600 | 3600 | 6500) {
        let subject = if observer { RIDER } else { world.agent_id };
        if let Some(contacts) = world.hand_contacts.for_avatar(world.agent_id, subject) {
            for graph in contacts.graphs() {
                log::info!(
                    "demo IK continuity: frame={frame}, activation={:?}, bones={}",
                    graph.activation,
                    graph.bones.len()
                );
            }
        }
    }
    let release = matches!(mode, "release" | "observer-release");
    if frame == 300 || profiles || separate || mode == "moving" || (observer && !release) || (release && frame == 720) {
        let angle = if mode == "moving" || separate || (observer && !release) {
            ((frame - 300) as f32 * 0.012).sin() * 0.25
        } else {
            0.0
        };
        let mut objects = objects(angle, !release || frame < 720);
        if profiles {
            let phase = ((frame - 300) % 90) as f32 / 55.0;
            let angle = if phase < 1.0 {
                (phase * std::f32::consts::PI).sin() * -0.5
            } else {
                0.0
            };
            objects[2].rotation *= Quat::from_rotation_y(angle);
        }
        if observer {
            for o in objects.iter_mut().take(3) {
                o.owner_id = RIDER;
            }
            objects[0].position.y += 1.8;
            objects[3].full_id = RIDER;
            objects[3].local_id = 9988;
            if objects[3].parent_id == 0 {
                objects[3].position.y += 1.8;
            }
            objects.push(action_avatar(false));
        }
        if matches!(mode, "ownerless" | "observer-ownerless") {
            for object in objects.iter_mut().take(3) {
                object.owner_id = Uuid::nil();
            }
        }
        if separate {
            let mut other = objects.remove(2);
            other.local_id = OTHER_ROOT;
            other.full_id = action_id(OTHER_ROOT);
            other.parent_id = 0;
            other.position = objects[0].position + objects[0].rotation * other.position;
            other.rotation = objects[0].rotation * other.rotation;
            objects.push(other);
        }
        world.apply(NetEvent::ObjectUpdates { handle: HANDLE, objects });
    }
    if frame == 300 {
        let mut animation = idle_animation();
        animation.joints = [
            ("mHipLeft", Quat::from_rotation_y(-1.1)),
            ("mHipRight", Quat::from_rotation_y(-1.1)),
            ("mKneeLeft", Quat::from_rotation_y(1.5)),
            ("mKneeRight", Quat::from_rotation_y(1.5)),
            ("mShoulderLeft", Quat::from_rotation_z(-1.2)),
            ("mShoulderRight", Quat::from_rotation_z(1.2)),
        ]
        .into_iter()
        .map(|(name, rotation)| JointMotion {
            joint_name: name.into(),
            priority: 4,
            rot_keys: vec![RotKey { time: 0.0, rotation }],
            pos_keys: Vec::new(),
        })
        .collect();
        scene
            .anims
            .insert(ANIM, Arc::new(BoundAnim::bind(animation, &world.avatar_lib.rig)));
        if profiles {
            if let Some(animation) = scene.anims.get(&ANIM) {
                scene.anims.insert(DRIVE, animation);
            }
            let mut acceleration = idle_animation();
            acceleration.joints = vec![JointMotion {
                joint_name: "mWristRight".into(),
                priority: 5,
                rot_keys: vec![RotKey {
                    time: 0.0,
                    rotation: Quat::from_rotation_x(0.4),
                }],
                pos_keys: Vec::new(),
            }];
            scene
                .anims
                .insert(ACCEL, Arc::new(BoundAnim::bind(acceleration, &world.avatar_lib.rig)));
        }
        world.apply(NetEvent::AvatarAnimations {
            avatar: if observer { RIDER } else { world.agent_id },
            sources: vec![action_id(ROOT)],
            anims: vec![(ANIM, 1)],
        });
    }
    if profiles && matches!(frame, 480 | 600 | 720 | 840 | 960) {
        PROFILE_REPLY.with(|cached| {
            if let Some(chat) = cached.borrow().clone() {
                world.apply(NetEvent::Chat(profile_packet(chat, frame, true)));
            }
        });
    }
    if profiles && matches!(frame, 480 | 530 | 720 | 840 | 960) {
        let avatar = if observer { RIDER } else { world.agent_id };
        if frame == 960 {
            world.apply(NetEvent::AvatarAnimations {
                avatar,
                sources: Vec::new(),
                anims: Vec::new(),
            });
        }
        let anims = match frame {
            480 => vec![(DRIVE, 2), (ACCEL, 1)],
            530 => vec![(DRIVE, 2)],
            720 => Vec::new(),
            840 => vec![(ANIM, 3)],
            _ => vec![(ANIM, 4)],
        };
        world.apply(NetEvent::AvatarAnimations {
            avatar,
            sources: vec![action_id(ROOT)],
            anims,
        });
    }
    if observer && matches!(frame, 560 | 620 | 800 | 920 | 980) {
        let count = world
            .hand_contacts
            .for_avatar(world.agent_id, RIDER)
            .map_or(0, |c| c.graphs().len());
        log::info!(
            "demo observer IK: frame={frame}, active_graphs={count}, local_graphs={}, active_bones={}",
            world.hand_contacts.graphs().len(),
            world
                .hand_contacts
                .for_avatar(world.agent_id, RIDER)
                .map_or(0, |c| c.graphs().iter().map(|g| g.bones.len()).sum::<usize>())
        );
    }
}

std::thread_local! {
    // Only the local demo's synthetic publisher remembers its last subscription.
    static PROFILE_REPLY: std::cell::RefCell<Option<ChatMessage>> = const { std::cell::RefCell::new(None) };
}

fn profile_packet(mut chat: ChatMessage, frame: u64, state_only: bool) -> ChatMessage {
    let mut packet: serde_json::Value = serde_json::from_str(chat.message.strip_prefix(PREFIX).expect("demo prefix")).expect("demo packet");
    let (profile, cycles, enabled, mask) = match frame {
        480..600 => ("gas_only", vec![1, 1], true, 1),
        600..720 => ("drive", vec![2, 1], true, 0),
        720..840 => ("drive", vec![2, 1], false, 0),
        840..960 => ("drive", vec![3, 2], true, 0),
        960.. => ("drive", vec![4, 3], true, 0),
        _ => ("drive", vec![1, 1], true, 0),
    };
    packet["profile"] = profile.into();
    packet["cycles"] = serde_json::json!(cycles);
    packet["enabled"] = enabled.into();
    packet["mask"] = mask.into();
    packet["revision"] = (frame + 1).into();
    if state_only {
        for key in ["part", "count", "data"] {
            packet.as_object_mut().expect("demo object").remove(key);
        }
    }
    chat.message = format!("{PREFIX}{packet}");
    chat
}

/// Exercise the editor's asynchronous reload through the normal app dispatch.
pub fn editor_reply(command: &NetCommand, frame: u64, mode: &str) -> Vec<NetEvent> {
    if frame >= 460 && matches!(mode, "seat-reload-waiting" | "library-object-waiting") {
        return Vec::new();
    }
    let mut events = reply(command);
    if matches!(
        std::env::var("AURORA_DEMO_HAND_IK").as_deref(),
        Ok("profiles" | "observer-profiles")
    ) {
        for event in &mut events {
            if let NetEvent::Chat(chat) = event {
                *chat = profile_packet(chat.clone(), frame, false);
                PROFILE_REPLY.with(|cached| *cached.borrow_mut() = Some(chat.clone()));
            }
        }
    }

    if frame >= 460 && mode == "seat-reload-off" {
        for event in &mut events {
            if let NetEvent::Chat(chat) = event {
                chat.message = chat.message.replace("\"enabled\":true", "\"enabled\":false");
            }
        }
    }
    events
}

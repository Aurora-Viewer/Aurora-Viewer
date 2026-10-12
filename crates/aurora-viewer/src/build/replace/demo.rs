//! Synthetic 15-part board and an independent replacement, with stable IDs.

use aurora_net::{NetEvent, objects::ObjectUpdate};
use aurora_prim::{ExtraParams, TextureEntry, VolumeParams, te::TextureFace};
use glam::{Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

pub const ROOT: u32 = 99680;
pub const TARGET: u32 = ROOT + 8;
pub const SOURCE: u32 = 99700;
pub fn id(local: u32) -> Uuid {
    Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | local as u128)
}

pub fn events(handle: u64, ground: f32) -> Vec<NetEvent> {
    let base = Vec3::new(135.0, 125.0, ground + 1.3);
    let mut objects = Vec::new();
    for n in 0..16 {
        let local = if n == 15 { SOURCE } else { ROOT + n };
        let target = local == TARGET;
        let position = if n == 0 {
            base
        } else if n == 15 {
            base + Vec3::new(2.0, 0.0, 0.1)
        } else {
            Vec3::new(((n - 1) % 5) as f32 * 0.46 - 0.9, ((n - 1) / 5) as f32 * 0.46 - 0.46, 0.15)
        };
        let color = if n == 0 {
            [0.07, 0.1, 0.19, 1.0]
        } else if n == 15 {
            [0.3, 0.08, 0.72, 1.0]
        } else if target {
            [0.65, 0.52, 0.23, 1.0]
        } else {
            [0.12, 0.42, 0.38, 1.0]
        };
        objects.push(ObjectUpdate {
            local_id: local,
            full_id: id(local),
            parent_id: if n > 0 && n < 15 { ROOT } else { 0 },
            pcode: aurora_prim::params::LL_PCODE_VOLUME,
            state: 0,
            crc: 0,
            material: 3,
            click_action: 0,
            scale: if n == 0 {
                Vec3::new(2.8, 1.8, 0.1)
            } else if target {
                Vec3::new(0.4, 0.28, 0.13)
            } else if n == 15 {
                Vec3::new(0.55, 0.38, 0.23)
            } else {
                Vec3::new(0.15, 0.15, 0.22)
            },
            position,
            rotation: if n == 15 { Quat::from_rotation_z(0.5) } else { Quat::IDENTITY },
            velocity: Vec3::ZERO,
            acceleration: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            update_flags: crate::ui::context::flags::OBJECT_MODIFY
                | crate::ui::context::flags::OBJECT_MOVE
                | crate::ui::context::flags::OBJECT_YOU_OWNER,
            // A full ObjectUpdate may omit OwnerID for a silent prim. Its
            // ownership still comes from flags and ObjectProperties replies.
            owner_id: Uuid::nil(),
            volume: VolumeParams::default(),
            texture_entry: Some(Arc::new(TextureEntry {
                faces: vec![
                    TextureFace {
                        texture: aurora_prim::te::BLANK_TEXTURE,
                        color,
                        ..Default::default()
                    };
                    6
                ],
            })),
            extra: ExtraParams::default(),
            name_values: String::new(),
            text: if n == 0 {
                aurora_i18n::tr!("replace-demo-board").into()
            } else if target {
                aurora_i18n::tr!("replace-demo-old").into()
            } else if n == 15 {
                aurora_i18n::tr!("replace-demo-new").into()
            } else {
                format!("#{}", n + 1)
            },
            text_color: [232, 237, 251, 255],
            media_url: String::new(),
            tree_species: None,
            texture_anim: None,
            foot_plane: None,
            particles: Default::default(),
            sound: Default::default(),
        });
    }
    vec![NetEvent::ObjectUpdates { handle, objects }]
}

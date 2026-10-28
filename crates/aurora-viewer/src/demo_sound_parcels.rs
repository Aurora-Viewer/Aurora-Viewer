//! Offline parcel sound boundaries: AURORA_DEMO_SOUND=parcel-open|parcel-local|parcel-source.

use super::{DEMO_AGENT, HANDLE, prim};
use aurora_net::{NetEvent, objects::AttachedSound, parcel_flags};
use glam::{Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

pub fn events(sound: Uuid, mode: &str) -> Vec<NetEvent> {
    let mut parcel = super::parcel_visibility::parcel_at(132.0, 132.0);
    if mode == "parcel-local" {
        Arc::make_mut(&mut parcel).flags |= parcel_flags::SOUND_LOCAL;
    }
    let z = super::floor_at(124.0, 132.0) + 1.0;
    let mut emitter = prim(
        12220,
        Vec3::new(124.0, 132.0, z),
        Quat::IDENTITY,
        Vec3::ONE,
        aurora_prim::VolumeParams::default(),
        super::te([0.6, 0.3, 0.8, 1.0], 0, false, 0.0),
        Default::default(),
        "",
    );
    emitter.owner_id = DEMO_AGENT;
    emitter.sound = AttachedSound {
        id: sound,
        gain: 1.0,
        flags: 1,
        radius: 0.0,
    };
    let mut events = vec![
        NetEvent::AgentParcel(parcel),
        NetEvent::ObjectUpdates {
            handle: HANDLE,
            objects: vec![emitter],
        },
    ];
    for sequence in 0..4 {
        let data = (0..1024)
            .map(|i| if mode == "parcel-source" && i % 64 < 32 { 0x20 } else { 0 })
            .collect();
        events.push(NetEvent::ParcelOverlay {
            handle: HANDLE,
            sequence,
            data,
        });
    }
    events
}

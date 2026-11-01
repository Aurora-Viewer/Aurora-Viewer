use super::*;
use aurora_net::{ParcelInfo, parcel_flags};

fn fixture() -> (World, SoundManager, Uuid, Uuid, Listener) {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events().into_iter().chain(crate::demo::hud::events()) {
        world.apply(event);
    }
    let object = world.objects.iter().find(|(_, o)| o.key.local_id == 101).unwrap().1.full_id;
    let idx = world.objects.index_of_uuid(&object).unwrap();
    world.objects.get_mut(idx).unwrap().position = Vec3::new(124.0, 132.0, 30.0);
    world.parcel = Some(crate::demo::parcel_visibility::parcel_at(132.0, 132.0));
    let asset = Uuid::from_u128(1222);
    let mut sounds = SoundManager::new(std::env::temp_dir().join("aurora-parcel-sound-tests"));
    sounds.insert(asset, SoundClip::from_pcm(&[0.0; 48_000], 1, 48_000));
    let listener = Listener {
        pos: Vec3::new(124.0, 131.0, 30.0),
        right: Vec3::X,
        forward: Vec3::Y,
        up: Vec3::Z,
        underwater: false,
        binaural: false,
    };
    (world, sounds, object, asset, listener)
}

#[test]
fn parcel_sound_loops_stop_and_resume_with_live_parcel_and_overlay_changes() {
    let (mut world, mut sounds, object, asset, listener) = fixture();
    sounds.set_attached(None, object, asset, 1.0, FLAG_LOOP);
    let now = Instant::now();
    sounds.update(None, &world, &listener, now);
    let first = sounds.sources[&object].playing.unwrap().voice;
    Arc::make_mut(world.parcel.as_mut().unwrap()).flags |= parcel_flags::SOUND_LOCAL;
    sounds.update(None, &world, &listener, now + Duration::from_millis(10));
    assert_eq!(
        sounds.voices(),
        0,
        "the camera is next to the source, but the avatar parcel is isolated"
    );
    assert!(sounds.sources[&object].pending.is_some());
    assert!(sounds.history.entries().all(|e| e.stopped.is_some()));
    world.parcel = Some(crate::demo::parcel_visibility::parcel_at(124.0, 132.0));
    sounds.update(None, &world, &listener, now + Duration::from_millis(20));
    assert_ne!(
        sounds.sources[&object].playing.unwrap().voice,
        first,
        "crossing into the parcel resumes the loop"
    );
    world.parcel = Some(crate::demo::parcel_visibility::parcel_at(132.0, 132.0));
    let handle = world.main_region.unwrap();
    world.map.apply_overlay(handle, (256, 256), 2, &[0x20; 1024]);
    sounds.update(None, &world, &listener, now + Duration::from_millis(30));
    assert_eq!(sounds.voices(), 0, "an isolated source parcel is silent outside it too");
    world.map.apply_overlay(handle, (256, 256), 2, &[0; 1024]);
    sounds.update(None, &world, &listener, now + Duration::from_millis(40));
    assert_eq!(sounds.voices(), 1, "removing the restriction resumes an existing source");
}

#[test]
fn parcel_sound_filter_discards_triggered_and_attached_one_shots_without_delaying_them() {
    let (mut world, mut sounds, object, asset, listener) = fixture();
    sounds.set_attached(None, object, asset, 1.0, 0);
    sounds.trigger(asset, listener.pos, None, 1.0);
    let now = Instant::now();
    sounds.update(None, &world, &listener, now);
    assert_eq!(sounds.voices(), 2);
    sounds.set_attached(None, object, asset, 1.0, FLAG_QUEUE);
    Arc::make_mut(world.parcel.as_mut().unwrap()).flags |= parcel_flags::SOUND_LOCAL;
    sounds.preview(std::iter::once(asset));
    sounds.update(None, &world, &listener, now + Duration::from_millis(10));
    assert_eq!(sounds.voices(), 0);
    assert!(sounds.triggered.is_empty());
    assert!(sounds.sources[&object].queued.is_none());
    assert!(sounds.previewing(), "local previews are independent of parcel restrictions");
    sounds.set_attached(None, object, asset, 1.0, 0);
    sounds.trigger(asset, listener.pos, None, 1.0);
    sounds.update(None, &world, &listener, now + Duration::from_millis(20));
    assert_eq!(sounds.voices(), 0, "pending one shots are filtered too");
    world.parcel = Some(Arc::new(ParcelInfo::default()));
    sounds.update(None, &world, &listener, now + Duration::from_millis(30));
    assert_eq!(sounds.voices(), 0, "blocked sounds never play late after a crossing");
}

#[test]
fn parcel_sound_demo_object_updates_follow_the_same_playback_path() {
    for mode in ["parcel-open", "parcel-local", "parcel-source"] {
        let (mut world, mut sounds, _, asset, listener) = fixture();
        for event in crate::demo::sound_parcels::events(asset, mode) {
            world.apply(event);
        }
        let object = world.objects.iter().find(|(_, o)| o.key.local_id == 12220).unwrap().1;
        sounds.apply_object_sound(None, object.full_id, object.sound);
        sounds.update(None, &world, &listener, Instant::now());
        assert_eq!(sounds.voices(), usize::from(mode == "parcel-open"), "{mode}");
    }
}

#[test]
fn parcel_sound_hud_uses_its_wearer_and_vehicle_exception_still_honors_radius() {
    let (mut world, mut sounds, object, asset, listener) = fixture();
    let hud = world
        .objects
        .iter()
        .find(|(_, o)| o.key.local_id == crate::demo::hud::CHILD)
        .unwrap()
        .1
        .full_id;
    let avatar = world.objects.index_of_uuid(&world.agent_id).unwrap();
    world.agent.get_off(Vec3::new(132.0, 132.0, 30.0));
    Arc::make_mut(world.parcel.as_mut().unwrap()).flags |= parcel_flags::SOUND_LOCAL;
    sounds.set_attached(None, hud, asset, 0.6, FLAG_LOOP);
    let now = Instant::now();
    sounds.update(None, &world, &listener, now);
    let gain = sounds.sources[&hud].playing.unwrap().gain;
    assert!((gain.0 - 0.6).abs() < 1e-5 && (gain.1 - 0.6).abs() < 1e-5);
    let seat = world.objects.index_of_uuid(&object).unwrap();
    let seat_local_id = world.objects.get(seat).unwrap().key.local_id;
    world.objects.get_mut(avatar).unwrap().parent_id = seat_local_id;
    world.objects.get_mut(avatar).unwrap().position = Vec3::ZERO;
    world.agent.seated = true;
    sounds.set_attached(None, object, asset, 1.0, FLAG_LOOP);
    sounds.update(None, &world, &listener, now + Duration::from_millis(10));
    assert!(
        sounds.sources[&object].playing.is_some(),
        "a vehicle crossing ahead of its rider stays audible"
    );
    sounds.set_radius(object, 0.5);
    sounds.update(None, &world, &listener, now + Duration::from_millis(20));
    assert!(
        sounds.sources[&object].playing.is_none(),
        "the seat exception applies only to parcel isolation"
    );
    world.objects.get_mut(avatar).unwrap().parent_id = 0;
    world.agent.seated = false;
    sounds.set_radius(object, 0.0);
    sounds.update(None, &world, &listener, now + Duration::from_millis(30));
    assert!(sounds.sources[&object].playing.is_none(), "standing up removes the exception");
}

//! Offline selection and give beams (`AURORA_DEMO_BEAMS`): another avatar
//! edits a cube, its beam sent again every 0.25 s like the Legacy Viewer's
//! idleUpdateTractorBeam, and gives to a second one now and then; `self`:
//! we edit a cube of our own, `legacy`: the first with the Legacy look.

use super::*;
use crate::build::{BuildSettings, BuildTool, Tool};
use crate::world::World;
use crate::world::beams::{GIVE_DURATION, SELECTION_DURATION, Style};

/// The demo's beam colour: a light blue chosen by the "sender".
const COLOR: [u8; 4] = [127, 209, 255, 255];
const EDITED: u32 = 9410;
const GIVEN: u32 = 9411;
/// Frames between two sends of the edit beam (0.25 s at 60 frames a second)
/// and between two gives.
const RESEND_FRAMES: u64 = 15;
const GIVE_FRAMES: u64 = 150;
/// Where each edits the cube, from its centre: the side seen from the
/// scenario's camera that faces the avatar.
const OTHER_CLICK: Vec3 = Vec3::new(0.25, 0.05, 0.08);
const SELF_CLICK: glam::DVec3 = glam::DVec3::new(-0.25, 0.0, 0.08);

fn cube(local_id: u32, at: Vec3, color: [f32; 4]) -> ObjectUpdate {
    prim(
        local_id,
        at,
        Quat::IDENTITY,
        Vec3::splat(0.5),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        te(color, 0, false, 0.0),
        ExtraParams::default(),
        "",
    )
}

/// Where the edited cube stands, in front of the other avatar's left hand,
/// on the open side of the plaza (where it is drawn: it may sit).
pub fn edited_at(world: &World) -> Option<Vec3> {
    let idx = world.objects.index_of_uuid(&DEMO_LOUP)?;
    let (at, _, _) = crate::scene::Scene::object_transform(world, idx, std::time::Instant::now(), 0)?;
    Some(at + Vec3::new(-1.3, -1.4, 0.3))
}

/// Where the avatar with the beam and the cube it edits are drawn, for the
/// camera: the other avatar, or ours in `self`.
pub fn view(world: &World, mode: &str) -> Option<(Vec3, Vec3)> {
    let at = |id: &Uuid| {
        let idx = world.objects.index_of_uuid(id)?;
        Some(crate::scene::Scene::object_transform(world, idx, std::time::Instant::now(), 0)?.0)
    };
    let avatar = if mode == "self" { world.agent_id } else { DEMO_LOUP };
    at(&avatar).zip(at(&edited_id()))
}

/// The edited cube's id (`prim`'s ids).
fn edited_id() -> Uuid {
    Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | u128::from(EDITED))
}

/// The style the scenario shows, if it asks for one.
pub fn style(mode: &str) -> Option<Style> {
    (mode == "legacy").then_some(Style::Legacy)
}

pub fn tick(world: &mut World, build: &mut BuildTool, settings: &mut BuildSettings, frame: u64, mode: &str) {
    if frame < 600 {
        return;
    }
    if frame == 600 {
        let (edited, given) = if mode == "self" {
            let p = world.agent.position;
            (p + Vec3::new(3.0, 1.0, 0.2), p + Vec3::new(3.0, -1.5, 0.2))
        } else {
            let Some(at) = edited_at(world) else {
                log::warn!("demo beams: the other avatar is not in the scene");
                return;
            };
            (at, at + Vec3::new(-2.6, 0.6, -0.3))
        };
        let mut a = cube(EDITED, edited, [0.7, 0.4, 0.9, 1.0]);
        a.owner_id = world.agent_id;
        let b = cube(GIVEN, given, [0.35, 0.75, 0.6, 1.0]);
        let id = a.full_id;
        world.apply(NetEvent::ObjectUpdates {
            handle: HANDLE,
            objects: vec![a, b],
        });
        if mode == "self" {
            build.open_build(Tool::Edit);
            let idx = world.objects.index_of_uuid(&id);
            build.click_select(world, settings, idx, false);
            // clicked on the side that faces us: the beam ends there
            build.point_pick = idx.and_then(|i| world.objects.get(i)).map(|o| (o.key, SELF_CLICK));
        }
    }
    if mode == "self" {
        return;
    }
    // like idleUpdateTractorBeam: no object, the global point clicked
    let clicked = view(world, mode).zip(world.main_origin()).map(|((_, cube), (mx, my))| {
        let p = cube + OTHER_CLICK;
        [f64::from(p.x) + f64::from(mx), f64::from(p.y) + f64::from(my), f64::from(p.z)]
    });
    if (frame - 600).is_multiple_of(RESEND_FRAMES)
        && let Some(position) = clicked
    {
        world.apply(NetEvent::Beam {
            effect: Uuid::from_u128(9410),
            source: DEMO_LOUP,
            target: Uuid::nil(),
            position,
            color: COLOR,
            duration: SELECTION_DURATION,
        });
    }
    if (frame - 600) % GIVE_FRAMES == GIVE_FRAMES / 2 {
        world.apply(NetEvent::Beam {
            effect: Uuid::new_v4(),
            source: DEMO_LOUP,
            target: Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | u128::from(GIVEN)),
            position: [0.0; 3],
            color: COLOR,
            duration: GIVE_DURATION,
        });
    }
    if frame == 760 {
        log::info!("demo beams: mode {mode}, {} beams shown", world.beams.shown.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_other_avatar_edits_a_cube_and_its_beam_shows() {
        let library = Arc::new(crate::scene::avatar::AvatarLibrary::load());
        let mut world = World::new(library);
        for event in events() {
            world.apply(event);
        }
        let mut build = BuildTool::default();
        let mut settings = BuildSettings::default();
        for frame in 600..=700 {
            tick(&mut world, &mut build, &mut settings, frame, "1");
        }
        let edit = &world.beams.shown[&Uuid::from_u128(9410)];
        assert_eq!(edit.source, DEMO_LOUP);
        // a point, like the Legacy Viewer's selection beam
        assert!(edit.target.is_none());
        assert_eq!(edit.color, COLOR);
        // the give fired once by frame 675
        assert_eq!(world.beams.shown.len(), 2);
    }
}

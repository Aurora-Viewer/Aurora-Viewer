//! AURORA_DEMO_SNOW_SPIN=1|settles-off: flat boxes turning in front of the
//! avatar in lying snow (#3642), with `AURORA_SNOW` and `AURORA_SNOW_LYING`:
//! one turning at 0.005 rad/s (0.3°/s, under the old threshold that left it
//! in the rain's shelter map), one at 0.3 rad/s, and one a "script" steps by
//! 15° every second with no motion reported (llSetRot on a timer). None
//! takes the ground's snow: what moves is out of the shelter map, so the
//! snow memory's surface stays the ground under them. `settles-off`: at frame 700
//! the weather turns to rain with "Snow settles" off, for this run only
//! (`weather`): the snow lying must go at once, not melt away in view.

use super::*;

const SLOW: u32 = 3_642_001;
const FAST: u32 = 3_642_002;
const STEPPED: u32 = 3_642_003;
/// The frame `settles-off` turns the weather.
const TURN_FRAME: u64 = 700;

pub fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_SNOW_SPIN").ok()
}

/// Where the boxes stand: a row 6 m north of the demo's arrival, clear of
/// the avatar in the camera's view.
fn place(i: u32) -> Vec3 {
    let x = [127.0, 129.8, 135.2][i as usize % 3];
    let y = 132.0;
    Vec3::new(x, y, floor_at(x, y) + 0.45)
}

fn plank(id: u32, at: Vec3, rot: Quat) -> ObjectUpdate {
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    prim(
        id,
        at,
        rot,
        Vec3::new(2.4, 0.7, 0.15),
        boxp,
        te([0.85, 0.35, 0.25, 1.0], 0, false, 0.0),
        ExtraParams::default(),
        "",
    )
}

/// The camera on the row at frame 61, from above and behind it (the avatar
/// out of the way): all three seen at once.
pub fn view(frame: u64) -> Option<(Vec3, Vec3)> {
    if mode().is_none() || frame != 61 {
        return None;
    }
    let mid = (place(0) + place(2)) * 0.5;
    Some((mid + Vec3::new(0.0, -3.0, 6.5), mid))
}

/// Once a frame: the boxes at frame 60, then the stepped one turned once a
/// second (60 frames), as a script's llSetRot: a new rotation, no spin.
pub fn tick(world: &mut crate::world::World, frame: u64) {
    if mode().is_none() {
        return;
    }
    let objects = if frame == 60 {
        let mut slow = plank(SLOW, place(0), Quat::IDENTITY);
        slow.angular_velocity = Vec3::Z * 0.005;
        let mut fast = plank(FAST, place(1), Quat::IDENTITY);
        fast.angular_velocity = Vec3::Z * 0.3;
        log::info!("demo snow spin: three planks (0.005 rad/s, 0.3 rad/s, stepped 15°/s)");
        vec![slow, fast, plank(STEPPED, place(2), Quat::IDENTITY)]
    } else if frame > 60 && frame.is_multiple_of(60) {
        let steps = (frame - 60) / 60;
        let turn = Quat::from_rotation_z((15.0 * steps as f32).to_radians());
        vec![plank(STEPPED, place(2), turn)]
    } else {
        return;
    };
    world.apply(NetEvent::ObjectUpdates { handle: HANDLE, objects });
}

/// `settles-off`: from frame 700, rain with "Snow settles" off, laid over
/// the weather drawn (never the settings, which the demo shares with the
/// viewer's own file).
pub fn weather(frame: u64, now: &mut crate::weather::WeatherSettings) {
    if mode().as_deref() != Some("settles-off") || frame < TURN_FRAME {
        return;
    }
    now.touched = true;
    now.precipitation = crate::weather::Precipitation::Rain;
    now.amount = 0.3;
    now.snow_settles = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stepped_plank_reports_no_motion() {
        let p = plank(STEPPED, place(2), Quat::from_rotation_z(0.3));
        assert_eq!(p.angular_velocity, Vec3::ZERO);
        assert_eq!(p.velocity, Vec3::ZERO);
    }
}

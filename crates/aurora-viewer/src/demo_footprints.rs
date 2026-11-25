//! AURORA_DEMO_FOOTPRINTS_OFF=<frame>: a trail of footprints in the lying
//! snow north of the arrival (with `AURORA_SNOW` and `AURORA_SNOW_LYING`),
//! seen from above, then "Footprints" turned off at this frame, for this
//! run only (#4019). The trail must go at once: no frozen footprint map
//! left on the ground.

use super::*;

/// The frame the trail is pressed (the region's first prints are made: a
/// region change before them would clear it), and the camera set on it.
const PRESS_FRAME: u64 = 300;

/// The frame footprints turn off, if the scenario is on.
fn off_frame() -> Option<u64> {
    static F: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("AURORA_DEMO_FOOTPRINTS_OFF").ok().and_then(|v| v.trim().parse().ok()))
}

/// Where the trail runs: north, 2 to 10 m from the demo's arrival.
fn step(i: u32) -> (f32, f32) {
    let side = if i.is_multiple_of(2) { -0.14 } else { 0.14 };
    (132.0 + side, 128.0 + i as f32 * 0.5)
}

/// At `PRESS_FRAME`, sixteen steps added to the region's footprints, as
/// `SnowTrails::press` keeps them (pressed now, a foot's sole heading
/// north on the ground).
pub fn tick(scene: &mut crate::scene::Scene, frame: u64) {
    if off_frame().is_none() || frame != PRESS_FRAME {
        return;
    }
    let trails = &mut scene.snow_trails;
    let fallen = trails.fallen;
    let prints = std::sync::Arc::make_mut(&mut trails.prints);
    for i in 0..16 {
        let (x, y) = step(i);
        prints.push([x, y, std::f32::consts::FRAC_PI_2, 0.13, floor_at(x, y), fallen, 0.0, 0.0]);
    }
    trails.generation += 1;
    log::info!("demo footprints: a trail of 16 steps pressed at frame {frame}");
}

/// The camera on the trail at `PRESS_FRAME`, from above and south of it.
pub fn view(frame: u64) -> Option<(Vec3, Vec3)> {
    if off_frame().is_none() || frame != PRESS_FRAME {
        return None;
    }
    let (x, y) = step(8);
    let at = Vec3::new(x, y, floor_at(x, y));
    Some((at + Vec3::new(0.0, -6.0, 5.0), at))
}

/// From the scenario's frame, footprints off in the weather drawn (never
/// the settings, which the demo shares with the viewer's own file).
pub fn weather(frame: u64, now: &mut crate::weather::WeatherSettings) {
    if off_frame().is_some_and(|f| frame >= f) {
        if off_frame() == Some(frame) {
            log::info!("demo footprints: Footprints off at frame {frame}");
        }
        now.snow_footprints = false;
    }
}

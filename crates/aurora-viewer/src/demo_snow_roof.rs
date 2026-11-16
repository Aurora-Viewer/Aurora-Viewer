//! AURORA_DEMO_SNOW_ROOF=1: four canopies 3 m above the plaza in the snow
//! (with `AURORA_SNOW` and `AURORA_SNOW_LYING`), for the rain's shelter map,
//! which holds what is seen from above: a plain one (snow on it, none under
//! it), one whose top face is fully transparent with its underside still
//! seen (a skybox's painted ceiling: the snow falls through it onto the
//! floor), a glass one at 30 % (snow on it, none under it), and leaves: a
//! blended underside under a fully transparent top (a tree's cards that
//! face down: no snow under it either).

use super::*;

const PLAIN: u32 = 3_700_001;
const OPEN_TOP: u32 = 3_700_002;
const GLASS: u32 = 3_700_003;
const LEAVES: u32 = 3_700_004;

pub fn enabled() -> bool {
    std::env::var_os("AURORA_DEMO_SNOW_ROOF").is_some()
}

/// Where the canopies stand: a row north of the demo's arrival.
fn place(i: u32) -> Vec3 {
    let x = [126.0, 131.0, 136.0, 121.0][i as usize % 4];
    let y = 134.0;
    Vec3::new(x, y, floor_at(x, y) + 3.0)
}

fn canopy(id: u32, at: Vec3, color: [f32; 4], top_alpha: f32) -> ObjectUpdate {
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let mut entry = (*te(color, 0, false, 0.0)).clone();
    // a box's face 0 is its top
    if let Some(top) = entry.faces.first_mut() {
        top.color[3] = top_alpha;
    }
    prim(
        id,
        at,
        Quat::IDENTITY,
        Vec3::new(3.6, 3.6, 0.12),
        boxp,
        Arc::new(entry),
        ExtraParams::default(),
        "",
    )
}

/// The camera on the row at frame 61, from the side and above: the tops and
/// the floor under each seen at once, the row left of the screen's middle
/// (the demo's Inventory window covers its right quarter).
pub fn view(frame: u64) -> Option<(Vec3, Vec3)> {
    if !enabled() || frame != 61 {
        return None;
    }
    let mid = place(1) + Vec3::new(-1.0, 0.0, -3.0);
    Some((mid + Vec3::new(0.0, -14.0, 9.0), mid))
}

pub fn tick(world: &mut crate::world::World, frame: u64) {
    if !enabled() || frame != 60 {
        return;
    }
    let plain = [0.55, 0.45, 0.4, 1.0];
    let glass = [0.6, 0.8, 1.0, 0.3];
    let objects = vec![
        canopy(PLAIN, place(0), plain, 1.0),
        canopy(OPEN_TOP, place(1), [0.35, 0.55, 0.85, 1.0], 0.0),
        canopy(GLASS, place(2), glass, 0.3),
        canopy(LEAVES, place(3), [0.3, 0.55, 0.25, 0.7], 0.0),
    ];
    log::info!("demo snow roof: a plain canopy, one with a fully transparent top, a glass one, leaves");
    world.apply(NetEvent::ObjectUpdates { handle: HANDLE, objects });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_open_canopy_has_only_its_top_transparent() {
        let c = canopy(OPEN_TOP, place(1), [0.35, 0.55, 0.85, 1.0], 0.0);
        let entry = c.texture_entry.as_ref().expect("a texture entry");
        assert_eq!(entry.face(0).color[3], 0.0);
        assert_eq!(entry.face(5).color[3], 1.0);
    }
}

//! Parcel sound isolation, after LLViewerParcelMgr::canHearSound and
//! LLViewerParcelOverlay::isSoundLocal (indra/newview, originally LGPL 2.1).

use super::World;
use aurora_net::parcel_flags;
use glam::Vec3;

/// PARCEL_SOUND_LOCAL in the simulator's byte-per-cell ParcelOverlay.
const SOUND_LOCAL: u8 = 0x20;

fn cell(pos: glam::DVec3, size: (u32, u32)) -> Option<usize> {
    if !pos.is_finite() || !(0.0..f64::from(size.0)).contains(&pos.x) || !(0.0..f64::from(size.1)).contains(&pos.y) {
        return None;
    }
    Some(pos.y as usize / 4 * (size.0 as usize / 4) + pos.x as usize / 4)
}

impl World {
    /// Whether a world sound at a render-space position can cross parcel
    /// borders. Membership is the avatar's parcel, independent of the camera
    /// and About Land's selection; height does not change parcel membership.
    pub fn can_hear_sound(&self, pos: Vec3) -> bool {
        if !pos.is_finite() {
            return false;
        }
        if let Some(parcel) = &self.parcel {
            if let Some(region) = self.main()
                && let Some(i) = cell(pos.as_dvec3(), (region.heightmap.size_x, region.heightmap.size_y))
                && parcel.bitmap.get(i / 8).is_some_and(|b| b & (1 << (i % 8)) != 0)
            {
                return true;
            }
            if parcel.flags & parcel_flags::SOUND_LOCAL != 0 {
                return false;
            }
        }
        let Some((x, y)) = self.main_origin() else {
            return true;
        };
        let global = pos.as_dvec3() + glam::DVec3::new(f64::from(x), f64::from(y), 0.0);
        for region in self.regions.values() {
            let (x, y) = aurora_net::handle_to_origin(region.handle);
            let local = global - glam::DVec3::new(f64::from(x), f64::from(y), 0.0);
            if let Some(i) = cell(local, (region.heightmap.size_x, region.heightmap.size_y)) {
                return self.map.overlays.get(&region.handle).is_none_or(|(cx, cy, cells)| {
                    (*cx, *cy) != (region.heightmap.size_x / 4, region.heightmap.size_y / 4)
                        || cells.get(i).is_none_or(|flags| flags & SOUND_LOCAL == 0)
                });
            }
        }
        // Like the Legacy Viewer, missing region/overlay data does not mute
        // an otherwise unrestricted sound.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn world() -> World {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        world.parcel = Some(crate::demo::parcel_visibility::parcel_at(132.0, 132.0));
        world
    }

    #[test]
    fn parcel_sound_isolation_is_bidirectional_and_same_parcel_takes_precedence() {
        let mut world = world();
        let handle = world.main_region.unwrap();
        let same = Vec3::new(132.0, 132.0, 4000.0);
        let other = Vec3::new(124.0, 132.0, 30.0);
        for current_local in [false, true] {
            for source_local in [false, true] {
                Arc::make_mut(world.parcel.as_mut().unwrap()).flags = if current_local { parcel_flags::SOUND_LOCAL } else { 0 };
                world
                    .map
                    .apply_overlay(handle, (256, 256), 2, &[if source_local { SOUND_LOCAL } else { 0 }; 1024]);
                assert!(
                    world.can_hear_sound(same),
                    "same parcel, regardless of height and either local flag"
                );
                assert_eq!(world.can_hear_sound(other), !current_local && !source_local);
            }
        }
        // The exact bitmap has a hole; its bounding box alone is not membership.
        let parcel = Arc::make_mut(world.parcel.as_mut().unwrap());
        let i = 33 * 64 + 33;
        parcel.bitmap[i / 8] &= !(1 << (i % 8));
        assert!(!world.can_hear_sound(same));
    }

    #[test]
    fn parcel_sound_checks_neighbour_regions_and_handles_missing_overlay_data() {
        let mut world = world();
        let (x, y) = world.main_origin().unwrap();
        let neighbour = aurora_net::origin_to_handle(x + 256, y);
        world.ensure_region(neighbour, (256, 256));
        let sound = Vec3::new(388.0, 132.0, 30.0);
        assert!(world.can_hear_sound(sound));
        world.map.apply_overlay(neighbour, (256, 256), 2, &[SOUND_LOCAL; 1024]);
        assert!(!world.can_hear_sound(sound));
        assert!(
            world.can_hear_sound(Vec3::new(132.0, 132.0, 30.0)),
            "same coordinates in another region are distinct"
        );
        Arc::make_mut(world.parcel.as_mut().unwrap()).flags |= parcel_flags::SOUND_LOCAL;
        world.map.overlays.clear();
        assert!(
            !world.can_hear_sound(sound),
            "the current parcel's restriction does not need the source overlay"
        );
        assert!(!world.can_hear_sound(Vec3::new(-1.0, 132.0, 30.0)));
        assert!(!world.can_hear_sound(Vec3::splat(f32::NAN)));
        world.parcel = None;
        assert!(world.can_hear_sound(sound), "unknown data alone must not mute unrestricted sounds");
        world.map.apply_overlay(neighbour, (256, 256), 2, &[SOUND_LOCAL; 1024]);
        assert!(
            !world.can_hear_sound(sound),
            "a source restriction still applies before our parcel is known"
        );
    }

    #[test]
    fn parcel_sound_uses_rectangular_region_stride_and_exact_cell_edges() {
        let mut world = world();
        let handle = world.main_region.unwrap();
        world.regions.get_mut(&handle).unwrap().heightmap = super::super::terrain::Heightmap::new(512, 256);
        let mut parcel = aurora_net::ParcelInfo {
            flags: parcel_flags::SOUND_LOCAL,
            bitmap: vec![0; 1024],
            ..Default::default()
        };
        let i = 33 * 128 + 100;
        parcel.bitmap[i / 8] |= 1 << (i % 8);
        world.parcel = Some(Arc::new(parcel));
        assert!(world.can_hear_sound(Vec3::new(400.0, 132.0, 30.0)));
        assert!(world.can_hear_sound(Vec3::new(403.9, 135.9, 30.0)));
        for pos in [
            Vec3::new(404.0, 132.0, 30.0),
            Vec3::new(400.0, 136.0, 30.0),
            Vec3::new(512.0, 132.0, 30.0),
        ] {
            assert!(!world.can_hear_sound(pos));
        }
        Arc::make_mut(world.parcel.as_mut().unwrap()).flags = 0;
        let mut overlay = vec![0; 2048];
        overlay[i + 1 - 4096] = SOUND_LOCAL;
        world.map.apply_overlay(handle, (512, 256), 2, &overlay);
        assert!(!world.can_hear_sound(Vec3::new(404.0, 132.0, 30.0)));
    }
}

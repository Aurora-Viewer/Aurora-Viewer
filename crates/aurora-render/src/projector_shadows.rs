//! Projector shadow budget and atlas packing. Sun cascades keep their three
//! original tiles; projector maps keep half a cascade's resolution as the
//! atlas grows, so increasing the budget never reduces per-map quality.

pub const MAX_PROJECTOR_SHADOWS: usize = 16;

pub fn atlas_size(cascade: u32, count: u8) -> (u32, u32) {
    let extra = u32::from(count.min(MAX_PROJECTOR_SHADOWS as u8)).saturating_sub(4).div_ceil(4);
    (cascade * 2, cascade * 2 + extra * (cascade / 2))
}

pub fn map_origin(cascade: u32, slot: usize) -> (u32, u32) {
    let half = cascade / 2;
    if slot < 4 {
        (cascade + slot as u32 % 2 * half, cascade + slot as u32 / 2 * half)
    } else {
        let k = slot as u32 - 4;
        (k % 4 * half, cascade * 2 + k / 4 * half)
    }
}

/// Effective budget at the requested sun resolution on this device. Never
/// silently lower sun or projector resolution to accommodate more maps.
pub fn effective_limit(resolution: u32, requested: u8, max_dimension: u32) -> u8 {
    let size = resolution.clamp(512, 8192).min(max_dimension / 2);
    let count = requested.min(MAX_PROJECTOR_SHADOWS as u8);
    (0..=count).rev().find(|&n| atlas_size(size, n).1 <= max_dimension).unwrap_or(0)
}

/// The map the rain's shelter takes (Aurora Weather): the first one past
/// the `count` projectors that cast shadows, so that none of them loses its
/// map while the ground is wet; the last projector's only when the atlas
/// has no room for one more (the full budget). The atlas is made with
/// [`atlas_maps`] of them.
pub fn shelter_slot(resolution: u32, count: u8, max_dimension: u32) -> usize {
    let size = resolution.clamp(512, 8192).min(max_dimension / 2);
    let room = usize::from(count) < MAX_PROJECTOR_SHADOWS && atlas_size(size, count + 1).1 <= max_dimension;
    if room {
        usize::from(count)
    } else {
        usize::from(count).saturating_sub(1)
    }
}

/// How many maps the atlas holds: the projectors' and the shelter's.
pub fn atlas_maps(count: u8, shelter: usize) -> u8 {
    count.max(shelter as u8 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projector_atlas_preserves_resolution_and_does_not_overlap_the_sun() {
        for count in 0..=MAX_PROJECTOR_SHADOWS as u8 {
            let size = 4096;
            let half = size / 2;
            let (width, height) = atlas_size(size, count);
            let mut rectangles = vec![(0, 0, size), (size, 0, size), (0, size, size)];
            for slot in 0..usize::from(count) {
                let (x, y) = map_origin(size, slot);
                assert!(x + half <= width && y + half <= height);
                assert!(
                    rectangles
                        .iter()
                        .all(|&(a, b, extent)| x + half <= a || a + extent <= x || y + half <= b || b + extent <= y)
                );
                rectangles.push((x, y, half));
            }
        }
        assert_eq!(atlas_size(4096, 2), (8192, 8192));
        assert_eq!(atlas_size(4096, 10), (8192, 12288));
    }

    #[test]
    fn the_shelter_takes_the_map_past_the_projectors_when_the_atlas_has_room() {
        // the default budget: map 2, in the fourth tile already there
        assert_eq!(shelter_slot(4096, 2, 16384), 2);
        assert_eq!(atlas_size(4096, atlas_maps(2, 2)), atlas_size(4096, 2));
        assert_eq!(shelter_slot(4096, 0, 16384), 0);
        // four projectors: a fifth map, one more row
        assert_eq!(shelter_slot(4096, 4, 16384), 4);
        assert_eq!(atlas_maps(4, 4), 5);
        // no room (the full budget, or the texture limit): the last
        // projector's map, the atlas unchanged
        assert_eq!(shelter_slot(2048, 16, 16384), 15);
        assert_eq!(atlas_maps(16, 15), 16);
        assert_eq!(shelter_slot(4096, 4, 8192), 3);
        assert_eq!(atlas_maps(4, 3), 4);
        for count in 0..=MAX_PROJECTOR_SHADOWS as u8 {
            let slot = shelter_slot(4096, count, 16384);
            assert!(slot < MAX_PROJECTOR_SHADOWS);
            assert!(atlas_size(4096, atlas_maps(count, slot)).1 <= 16384);
        }
    }

    #[test]
    fn projector_budget_respects_texture_limits_without_changing_resolution() {
        assert_eq!(effective_limit(4096, 10, 16384), 10);
        assert_eq!(effective_limit(4096, 16, 8192), 4);
        assert_eq!(effective_limit(2048, 16, 8192), 16);
    }
}

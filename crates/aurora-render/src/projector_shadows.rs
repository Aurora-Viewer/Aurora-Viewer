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
    fn projector_budget_respects_texture_limits_without_changing_resolution() {
        assert_eq!(effective_limit(4096, 10, 16384), 10);
        assert_eq!(effective_limit(4096, 16, 8192), 4);
        assert_eq!(effective_limit(2048, 16, 8192), 16);
    }
}

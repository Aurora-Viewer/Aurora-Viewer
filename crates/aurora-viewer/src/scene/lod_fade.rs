//! LOD crossfade: an object that changes detail level fades from one level
//! to the other instead of popping.
//!
//! An Aurora addition: the Legacy Viewer swaps the geometry at once
//! (LLVOVolume::calcLOD then LLVolumeGeometryManager::rebuildGeom). Which
//! level is chosen, and when, does not change (`lod_for`); only the swap is
//! drawn over a short time.
//!
//! When the sync binds the new level, the faces of the old one keep their
//! draw records (a "ghost") and the old geometry stays referenced. Both are
//! drawn for `length` ms: the GPU lists draw every face that has a record
//! and an entry in the face table, so the ghost needs nothing more than its
//! records. The records carry the fade's start and length
//! (`aurora_render::lod_fade`, `DrawRecord::identity`): the shader shows the
//! new level over a growing share of a dither pattern and the old one over
//! the rest (object.wgsl `lod_fade_clip`), in the depth prepass, the colour
//! and the shadows. While a fade runs, the frame takes the prepass and
//! shadow pipelines with a fragment shader (`DrawLists::portal_clipping`,
//! as for a portal cut); the rest of the time nothing changes.
//!
//! Only opaque and alpha-masked faces fade: the blended and glowing ones
//! are listed by the CPU from the object's faces (`ObjGpu::special`), so
//! the old level's would not be drawn. Rigged, anchored and HUD objects,
//! and the swaps that are not between two levels of the same geometry (a
//! first load, a placeholder, a shape edit), swap at once as before. So do
//! the CPU lists (AURORA_CPU_CULL=1), which only draw the object's faces.
//!
//! Preferences › Graphics "Fade between levels of detail"
//! (`Settings::lod_crossfade`, on by default) turns it on and off;
//! `AURORA_LOD_FADE=<ms>` sets the length instead, 0 turning it off.

use super::GeomKey;

/// Length of a fade (ms): short enough not to show both levels at once for
/// a visible while, long enough to hide the pop.
pub const DEFAULT_MS: u32 = 300;

/// Longest fade (ms) `AURORA_LOD_FADE` may ask for.
const MAX_MS: u32 = 2000;

/// `AURORA_LOD_FADE`: the length of a fade in ms (0 for none) over the
/// setting's; None when unset.
pub fn length_from_env() -> Option<u32> {
    static LENGTH: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *LENGTH.get_or_init(|| parse_length(std::env::var("AURORA_LOD_FADE").ok().as_deref()))
}

fn parse_length(value: Option<&str>) -> Option<u32> {
    match value.map(str::trim) {
        None | Some("") => None,
        Some(v) => Some(v.parse::<u32>().map(|ms| ms.min(MAX_MS)).unwrap_or(DEFAULT_MS)),
    }
}

/// The length of a fade (ms, 0 for none) with the setting `on`.
pub fn length_for(on: bool) -> u32 {
    length_from_env().unwrap_or(if on { DEFAULT_MS } else { 0 })
}

/// `old` and `new` are two detail levels of the same geometry: the swap
/// fades. A placeholder, a first load, a flexible prim (rebuilt as it
/// moves), a shape edit (another hash) or an avatar part does not.
pub fn same_geometry(old: GeomKey, new: GeomKey) -> bool {
    match (old, new) {
        (GeomKey::Prim { hash: a, lod: la }, GeomKey::Prim { hash: b, lod: lb })
        | (GeomKey::Sculpt { hash: a, lod: la }, GeomKey::Sculpt { hash: b, lod: lb }) => a == b && la != lb,
        (GeomKey::Mesh { id: a, lod: la }, GeomKey::Mesh { id: b, lod: lb }) => a == b && la != lb,
        (GeomKey::Tree { species: a, lod: la }, GeomKey::Tree { species: b, lod: lb }) => a == b && la != lb,
        _ => false,
    }
}

/// The old level of an object still drawn while it fades out.
pub struct Ghost {
    pub idx: usize,
    /// Its faces' records, each still in the face table.
    pub records: Vec<u32>,
    /// Its glowing faces (draw, in LL's alpha pool): the glow pass is a CPU
    /// list, and the new level's glow only lands where the new level is
    /// drawn (its depth test), so the old level's glow fills the rest; else
    /// the glow dims and comes back at every change of level.
    pub glow: Vec<(aurora_render::DrawCmd, bool)>,
    /// The new level's records that fade in, cleared at the end.
    pub fresh: Vec<u32>,
    /// The old geometry, referenced until the end.
    pub geom: GeomKey,
    /// Effects-clock ms of its end (`AnimClock`, wrapping).
    pub until: u32,
}

/// The fades under way.
#[derive(Default)]
pub struct LodFade {
    /// Length of a fade (ms), 0 for none.
    pub length: u32,
    ghosts: Vec<Ghost>,
}

impl LodFade {
    pub fn new(length: u32) -> LodFade {
        LodFade {
            length,
            ghosts: Vec::new(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.length > 0
    }

    /// A fade runs: the frame needs the clipping pipelines.
    pub fn active(&self) -> bool {
        !self.ghosts.is_empty()
    }

    pub fn start(&mut self, ghost: Ghost) {
        self.ghosts.push(ghost);
    }

    /// The ghost of object `idx`, taken out (its fade ends now).
    pub fn take(&mut self, idx: usize) -> Option<Ghost> {
        let i = self.ghosts.iter().position(|g| g.idx == idx)?;
        Some(self.ghosts.swap_remove(i))
    }

    /// The ghost records of object `idx` (placed again with it).
    pub fn records_of(&self, idx: usize) -> impl Iterator<Item = u32> + '_ {
        self.ghosts
            .iter()
            .filter(move |g| g.idx == idx)
            .flat_map(|g| g.records.iter().copied())
    }

    /// The glowing faces of the level object `idx` fades out of.
    pub fn glow_of(&self, idx: usize) -> impl Iterator<Item = (aurora_render::DrawCmd, bool)> + '_ {
        self.ghosts
            .iter()
            .filter(move |g| g.idx == idx)
            .flat_map(|g| g.glow.iter().copied())
    }

    /// The fades over at `ms`, taken out.
    pub fn take_ended(&mut self, ms: u32) -> Vec<Ghost> {
        let mut ended = Vec::new();
        let mut i = 0;
        while i < self.ghosts.len() {
            if ended_at(self.ghosts[i].until, ms) {
                ended.push(self.ghosts.swap_remove(i));
            } else {
                i += 1;
            }
        }
        ended
    }
}

/// `until` is reached at `ms`, on the wrapping clock.
fn ended_at(until: u32, ms: u32) -> bool {
    (ms.wrapping_sub(until) as i32) >= 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn ghost(idx: usize, until: u32) -> Ghost {
        Ghost {
            idx,
            records: vec![idx as u32 * 10, idx as u32 * 10 + 1],
            glow: vec![],
            fresh: vec![],
            geom: GeomKey::Prim { hash: 1, lod: 3 },
            until,
        }
    }

    #[test]
    fn the_variable_overrides_the_length() {
        assert_eq!(parse_length(None), None);
        assert_eq!(parse_length(Some("")), None);
        assert_eq!(parse_length(Some("0")), Some(0));
        assert_eq!(parse_length(Some("1")), Some(1));
        assert_eq!(parse_length(Some(" 250 ")), Some(250));
        assert_eq!(parse_length(Some("on")), Some(DEFAULT_MS));
        assert_eq!(parse_length(Some("60000")), Some(MAX_MS));
    }

    #[test]
    fn only_two_levels_of_the_same_geometry_fade() {
        let id = Uuid::from_u128(7);
        let mesh = |lod| GeomKey::Mesh { id, lod };
        assert!(same_geometry(mesh(3), mesh(2)));
        assert!(!same_geometry(mesh(2), mesh(2)));
        assert!(!same_geometry(
            mesh(3),
            GeomKey::Mesh {
                id: Uuid::from_u128(8),
                lod: 2
            }
        ));
        assert!(same_geometry(GeomKey::Prim { hash: 5, lod: 0 }, GeomKey::Prim { hash: 5, lod: 1 }));
        // a shape edit
        assert!(!same_geometry(GeomKey::Prim { hash: 5, lod: 0 }, GeomKey::Prim { hash: 6, lod: 1 }));
        assert!(same_geometry(
            GeomKey::Tree { species: 2, lod: 0 },
            GeomKey::Tree { species: 2, lod: 3 }
        ));
        // the sphere of a missing sculpt map, then the sculpt
        assert!(!same_geometry(
            GeomKey::SculptPlaceholder { hash: 9, lod: 2 },
            GeomKey::Sculpt { hash: 9, lod: 2 }
        ));
        assert!(!same_geometry(
            GeomKey::Flexible { id, hash: 1, lod: 1 },
            GeomKey::Flexible { id, hash: 1, lod: 2 }
        ));
    }

    #[test]
    fn a_fade_ends_on_time_even_across_the_clock_wrap() {
        // (the fades under way all end within MAX_MS of the clock)
        let mut fades = LodFade::new(300);
        fades.start(ghost(1, 1000));
        fades.start(ghost(2, 1300));
        assert!(fades.take_ended(999).is_empty());
        let ended: Vec<usize> = fades.take_ended(1000).iter().map(|g| g.idx).collect();
        assert_eq!(ended, vec![1]);
        assert!(fades.take_ended(1299).is_empty());
        assert_eq!(fades.take_ended(1450).len(), 1, "a late frame ends it too");
        assert!(!fades.active());
        // one ending just before the clock wraps, one started before it and
        // ending after it
        fades.start(ghost(3, u32::MAX - 10));
        fades.start(ghost(4, 20));
        assert!(fades.take_ended(u32::MAX - 11).is_empty());
        let ended: Vec<usize> = fades.take_ended(u32::MAX - 10).iter().map(|g| g.idx).collect();
        assert_eq!(ended, vec![3]);
        assert!(fades.take_ended(19).is_empty());
        assert_eq!(fades.take_ended(20).len(), 1);
        assert!(!fades.active());
    }

    #[test]
    fn a_ghost_is_found_and_taken_by_its_object() {
        let mut fades = LodFade::new(300);
        fades.start(ghost(4, 500));
        fades.start(ghost(5, 500));
        assert_eq!(fades.records_of(5).collect::<Vec<_>>(), vec![50, 51]);
        assert_eq!(fades.records_of(6).count(), 0);
        assert_eq!(fades.take(4).map(|g| g.idx), Some(4));
        assert!(fades.take(4).is_none());
        assert!(fades.active());
    }
}

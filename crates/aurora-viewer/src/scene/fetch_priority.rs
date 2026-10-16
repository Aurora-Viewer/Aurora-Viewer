//! Rank of every asset request in the fetcher's queue (`aurora_net::Lane`).
//!
//! Two lanes. The urgent one holds what is small and that something waits
//! on, ranked by kind; it goes first and has a few slots of its own, as
//! Firestorm gives each kind of asset its own connections
//! (`LLAppCoreHttp::init_data`). Meshes and textures share the bulk lane,
//! ranked together by the size of what they show on screen, in pixels
//! (Firestorm scores a pending mesh by radius over distance,
//! `calculate_score` in llmeshrepository.cpp, and a texture by its pixel
//! area): the near and large first, whatever their kind. Ranks follow the
//! view while a request waits (`changed`).

/// Ranks within the urgent lane.
pub mod urgent {
    /// The sky and water settings, and a landmark a window waits on.
    pub const SKY: f32 = 5.0;
    pub const WINDOW: f32 = 5.0;
    /// Our own avatar's bakes and the images of the interface.
    pub const OWN: f32 = 4.0;
    /// An avatar stands still until its animation arrives.
    pub const ANIMATION: f32 = 3.0;
    pub const SOUND: f32 = 2.0;
    /// PBR materials: a few hundred bytes each, the faces wait on them.
    pub const MATERIAL: f32 = 1.0;
}

/// Added to what shows nothing yet: grey or missing comes before sharper.
pub const NOTHING_SHOWN: f32 = 1e5;

/// Sizes are held below `NOTHING_SHOWN` (a face repeating its texture many
/// times can reach past it).
const MOST_PX: f32 = NOTHING_SHOWN * 0.5;

/// A texture covering `px` pixels on screen (0 out of view).
pub fn texture(px: f32, shown: bool) -> f32 {
    px.clamp(1.0, MOST_PX) + if shown { 0.0 } else { NOTHING_SHOWN }
}

/// A mesh whose largest user covers `px` pixels on screen. Before a
/// texture of the same size: without its mesh an object is not there at
/// all, without its texture it is a grey shape (and its textures are only
/// asked for once its faces are built).
pub fn mesh(px: f32) -> f32 {
    (2.0 * px).clamp(1.0, MOST_PX) + NOTHING_SHOWN
}

/// The rank of a waiting request moved enough to tell the fetcher: its
/// size on screen by half at least, either way, or across `NOTHING_SHOWN`.
/// Small moves of the camera send nothing.
pub fn changed(sent: f32, now: f32) -> bool {
    let size = |p: f32| if p >= NOTHING_SHOWN { p - NOTHING_SHOWN } else { p };
    let (a, b) = (size(sent), size(now));
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    hi > lo * 1.5 || (sent >= NOTHING_SHOWN) != (now >= NOTHING_SHOWN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_shows_nothing_comes_first_then_the_largest() {
        // a small grey texture before a large one being sharpened
        assert!(texture(10.0, false) > texture(5000.0, true));
        assert!(texture(800.0, false) > texture(100.0, false));
        // a mesh before a texture of the same size, a near texture before a far mesh
        assert!(mesh(300.0) > texture(300.0, false));
        assert!(texture(900.0, false) > mesh(50.0));
        // out of view: still ranked, last
        assert!(texture(0.0, false) < texture(2.0, false));
    }

    #[test]
    fn only_real_moves_are_sent() {
        assert!(!changed(texture(400.0, false), texture(430.0, false)));
        assert!(!changed(1000.0, 1400.0));
        assert!(changed(1000.0, 1600.0));
        assert!(changed(1600.0, 1000.0));
        assert!(changed(texture(400.0, true), texture(400.0, false)));
        // what shows nothing yet follows its size too
        assert!(changed(texture(400.0, false), texture(2000.0, false)));
        assert!(changed(mesh(400.0), mesh(100.0)));
    }
}

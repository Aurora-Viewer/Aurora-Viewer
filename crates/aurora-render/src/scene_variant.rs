//! The variant of the scene pass's opaque and masked pipelines a frame draws
//! with (`SCENE_VARIANT`, common.wgsl). The scene shader carries every path
//! any face may take; their branches are uniform, yet compiled in they set
//! the registers every pixel pays for. On Apple GPUs the scene pass ran at
//! 13 % fragment occupancy, latency-bound, and a variant without the paths
//! a frame provably does not take was 5 ms faster at 2400 x 1300 on an
//! M2 Pro (issue #3218). The image is the same: a lean variant is chosen
//! only when none of the paths it leaves out could change a pixel.
//!
//! The rez effect is common (objects rezzing in, the grid over faces whose
//! texture still loads) and costs the lean variant about 1 ms when compiled
//! in: it has lean variants of its own (3, 4), and the lean variants
//! without it (1, 2) draw once nothing rezzes and every texture arrived.
//!
//! Within a lean variant, each face's pipeline is also specialised to its
//! material class (`material_class`: a legacy material, a PBR one, neither),
//! in the blended pass and in the scene pass's CPU lists. The draws keep
//! their order, in runs of one class: a face never changes place, so what
//! covers what (and the order of the blended faces) is unchanged.
//!
//! `AURORA_SCENE_VARIANT=full` keeps the full variant and the generic
//! pipelines, `lean` takes the lean ones whenever allowed (the default), for a
//! comparison.

/// Debug views (`debug_view`, common.wgsl): the low byte of
/// `frame.light_mode.z`.
const DEBUG_VIEW: u32 = 0xff;
/// `LEGACY_NAN_CHECK` of object.wgsl.
const LEGACY_NAN_CHECK: u32 = 0x40_0000;

/// What decides the variant, as the frame's uniforms hold it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct FrameState {
    /// `frame.light_mode.z`.
    pub light_mode: u32,
    /// `is_classic_sky()`: `frame.sky_ll.x > 0.5`.
    pub classic: bool,
    /// The water effects' bits the shaders read (`water_fx.counts.z`).
    pub water_effects: u32,
    /// The rez effect may draw: a record's rez in progress
    /// (`RecordStore::scene_needs`), or the rez grid over faces still
    /// loading (`frame.rez_gradient[1].w`) while some texture still loads
    /// (`TextureTable::loading`; `rez_still_grey`, object.wgsl).
    pub rez: bool,
    /// Some live record has a portal clip (`RecordStore::scene_needs`).
    pub portal_records: bool,
}

/// The value of `SCENE_VARIANT` for a frame: 0 full, 1 lean with the classic
/// sky, 2 lean with a sky with a probe ambiance, 3 and 4 the same with the
/// rez grid.
pub(crate) fn choose(s: FrameState) -> u32 {
    let v = if forced_full() { 0 } else { variant(s) };
    // the log says when the variant changes, and why
    static LAST: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(u32::MAX);
    if LAST.swap(v, std::sync::atomic::Ordering::Relaxed) != v {
        log::info!("scene variant {v} ({s:?})");
    }
    v
}

fn variant(s: FrameState) -> u32 {
    if s.light_mode & (DEBUG_VIEW | LEGACY_NAN_CHECK) != 0 || s.water_effects != 0 || s.portal_records {
        return 0;
    }
    // odd: classic sky (`scene_classic`, common.wgsl)
    let sky = if s.classic { 1 } else { 2 };
    if s.rez { sky + 2 } else { sky }
}

/// The material class of a blended face (`MATERIAL_CLASS`, object.wgsl): 0
/// any (the generic pipeline), 1 a legacy material (LLMaterial), 2 a PBR
/// material, 3 neither. A fullbright face or a portal surface takes the
/// generic pipeline. Each
/// class's pipeline leaves out the paths its faces never take, so that the
/// blended pass holds fewer registers; the light is the same.
pub(crate) fn material_class(flags: u32) -> u8 {
    use crate::types::flags;
    if flags & (flags::FULLBRIGHT | flags::PORTAL_EFFECT) != 0 || forced_full() {
        0
    } else if flags & flags::PBR != 0 {
        2
    } else if flags & flags::LEGACY_MAT != 0 {
        1
    } else {
        3
    }
}

/// `material_class` of each blended face, from its draw record (0 for a
/// record not in the store).
pub(crate) fn material_classes(blend: &[crate::types::DrawCmd], records: &crate::arena::RecordStore, out: &mut Vec<u8>) {
    out.clear();
    out.extend(
        blend
            .iter()
            .map(|c| records.get(c.record).map_or(0, |r| material_class(r.flags[0]))),
    );
}

fn forced_full() -> bool {
    static FULL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FULL.get_or_init(|| std::env::var("AURORA_SCENE_VARIANT").is_ok_and(|v| v.trim() == "full"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blended_faces_take_the_pipeline_of_their_material() {
        use crate::types::flags;
        assert_eq!(material_class(flags::ALPHA_BLEND), 3);
        assert_eq!(material_class(flags::LEGACY_MAT | flags::ALPHA_BLEND), 1);
        assert_eq!(material_class(flags::PBR | flags::ALPHA_BLEND), 2);
        // the generic pipeline for the paths no class keeps
        assert_eq!(material_class(flags::PBR | flags::FULLBRIGHT), 0);
        assert_eq!(material_class(flags::FULLBRIGHT), 0);
        assert_eq!(material_class(flags::PORTAL_EFFECT), 0);
    }

    #[test]
    fn a_plain_frame_draws_lean_with_its_sky() {
        let classic = FrameState {
            classic: true,
            ..Default::default()
        };
        assert_eq!(variant(classic), 1);
        assert_eq!(variant(FrameState::default()), 2);
        // other bits of light_mode.z (shadow filter, specular AA...) do not
        // touch the paths a lean variant leaves out
        let other_bits = FrameState {
            light_mode: 0x8000 | 0x400 | 0x2000 | 0x10_0000,
            ..classic
        };
        assert_eq!(variant(other_bits), 1);
    }

    #[test]
    fn the_rez_effect_keeps_a_lean_variant_of_its_own() {
        let rez = FrameState {
            rez: true,
            ..Default::default()
        };
        assert_eq!(variant(rez), 4);
        assert_eq!(variant(FrameState { classic: true, ..rez }), 3);
        // a portal clip still takes the full variant
        assert_eq!(
            variant(FrameState {
                portal_records: true,
                ..rez
            }),
            0
        );
    }

    #[test]
    fn each_path_the_lean_variant_leaves_out_takes_the_full_one() {
        for s in [
            FrameState {
                light_mode: 2,
                ..Default::default()
            },
            FrameState {
                light_mode: LEGACY_NAN_CHECK,
                ..Default::default()
            },
            FrameState {
                water_effects: 32,
                ..Default::default()
            },
            FrameState {
                portal_records: true,
                classic: true,
                ..Default::default()
            },
        ] {
            assert_eq!(variant(s), 0, "{s:?}");
        }
    }

    #[test]
    fn the_shader_knows_the_bits_and_the_variants() {
        let object = include_str!("shaders/object.wgsl");
        assert!(object.contains(&format!("const LEGACY_NAN_CHECK: u32 = 0x{LEGACY_NAN_CHECK:x}u;")));
        assert!(object.contains("} else if (scene_classic(classic)) {"));
        assert!(object.contains("    if (scene_rez()) {\n        col = rez_apply("));
        let common = include_str!("shaders/common.wgsl");
        assert!(common.contains("override SCENE_VARIANT: u32 = 0u;"));
        assert!(common.contains("return SCENE_VARIANT == 0u || SCENE_VARIANT >= 3u;"));
        assert!(common.contains("return (SCENE_VARIANT & 1u) == 1u;"));
        assert!(common.contains("let view = frame.light_mode.z & 0xffu;"));
    }
}

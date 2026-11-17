//! The rows of the Phototools window (`ui/phototools.rs`), taken from the
//! Preferences pages that own them (`Cx::borrow`: their own declarations,
//! so the same setting, label, help, range, clamp, "modified" dot and RLVa
//! greying, and no second copy of any of them). Each Legacy group box of
//! floater_phototools.xml (EnvironmentTab, P_General) is one [`Section`],
//! drawn inside a collapsing section of the window, its rows in the Legacy
//! order.
//!
//! Left out, with the reason (TASKS.md lists them too). A Legacy row Aurora
//! has no setting for is not invented here:
//!
//! - Obsolete in Aurora's renderer:
//!   - RenderTransparentWater: the water always refracts a copy of the
//!     scene (aurora-render/src/renderer.rs), there is no opaque path;
//!   - RenderReflectionProbeDetail (static / dynamic / realtime): one probe
//!     scheme, its level is `reflection_probes`; the hero probes' resolution
//!     and update rate fold into the `mirrors` level;
//!   - RenderUseFarClip: the projection is infinite and the culling is
//!     always at the draw distance;
//!   - RenderDynamicLOD: the level of detail is always by distance, scaled
//!     by `lod_factor`;
//!   - RenderTerrainLODFactor: the terrain is meshed in fixed chunks at the
//!     height map's resolution, there is no patch LOD to scale;
//!   - RenderFlexTimeFactor: every flexible object within the draw distance
//!     simulates every frame, no update is spaced out.
//! - Out of scope: SelectReflectionProbes (a build-selection aid of the
//!   Legacy renderer).
//! - Missing, for later (a renderer or settings change of their own):
//!   - RenderReflectionProbeResolution: fixed at 128 px (`PROBE_RES`);
//!   - RenderAvatarLODFactor ("Avi Detail"): no avatar mesh LOD factor;
//!   - RenderTerrainScale: fixed at 12 m per texture repeat;
//!   - RenderNormalMapScale ("Texture Bumpiness"): the normals built from
//!     bump heights use a fixed scale of 64, the Legacy default.

use super::cx::Cx;
use super::{OptionsUi, Page};

/// One Legacy group box.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    /// Environment tab, "Reflection Settings".
    Reflections,
    /// General tab, "General Render Settings".
    GeneralRender,
    /// General tab, the check boxes under it.
    RenderOptions,
    /// General tab, the anti-aliasing, filtering and texture group.
    ImageQuality,
}

/// A Preferences row, by the page and tab that declare it and the setting
/// it edits.
type Row = (Page, usize, &'static str);

/// Legacy order (floater_phototools.xml): Screen Space Reflections,
/// Mirrors, the probe level, the probe draw distance.
const REFLECTIONS: &[Row] = &[
    (Page::Graphics, 3, "ssr"),
    (Page::Graphics, 3, "mirrors"),
    (Page::Graphics, 3, "reflection_probes"),
    (Page::Graphics, 3, "probe_range"),
];

/// The Legacy sliders: Draw Dist., Avi Count, Avi Physics, Particle Cnt.,
/// Object Detail, Tree Detail, Sharpening, Exposure.
const GENERAL_RENDER: &[Row] = &[
    (Page::Graphics, 0, "draw_distance"),
    (Page::Graphics, 0, "max_avatars"),
    (Page::Graphics, 0, "avatar_physics_detail"),
    (Page::Graphics, 0, "max_particles"),
    (Page::Graphics, 0, "lod_factor"),
    (Page::Graphics, 0, "tree_lod_factor"),
    (Page::Graphics, 4, "sharpen"),
    (Page::Graphics, 1, "exposure"),
];

/// "Render only Objects visible to the Camera" and "Render Particles
/// Attached to other Avatars".
const RENDER_OPTIONS: &[Row] = &[(Page::Graphics, 0, "occlusion_culling"), (Page::Graphics, 0, "attached_particles")];

/// Antialiasing, Anisotropic Filtering, Max Texture Resolution, Avatar
/// Display.
const IMAGE_QUALITY: &[Row] = &[
    (Page::Graphics, 4, "antialiasing"),
    (Page::Graphics, 0, "anisotropy"),
    (Page::Graphics, 0, "max_texture_resolution"),
    (Page::Graphics, 0, "complexity_mode"),
];

impl Section {
    fn rows(self) -> &'static [Row] {
        match self {
            Section::Reflections => REFLECTIONS,
            Section::GeneralRender => GENERAL_RENDER,
            Section::RenderOptions => RENDER_OPTIONS,
            Section::ImageQuality => IMAGE_QUALITY,
        }
    }
}

/// The rows of `section`, one `borrow` each so that the Legacy order wins
/// over the page's.
pub(super) fn draw(cx: &mut Cx, section: Section) {
    // the collapsing section names the group
    cx.group("", |cx| {
        for &(page, tab, path) in section.rows() {
            cx.borrow(page, tab, &[path]);
        }
    });
}

impl OptionsUi {
    /// What the app tells Preferences about the world, for a second window
    /// of borrowed rows (Phototools has its own `OptionsUi`, so that the
    /// two windows never share the sizes the rows measured): the fields the
    /// rows of the pages it borrows from read.
    pub fn share_from(&mut self, from: &OptionsUi) {
        self.rlva_camera_preset_locked = from.rlva_camera_preset_locked;
        self.rlva_camera_fov_locked = from.rlva_camera_fov_locked;
        self.sky_tone_mapped = from.sky_tone_mapped;
        self.vram = from.vram;
        if self.gpu_in_use != from.gpu_in_use || self.gpus != from.gpus {
            self.gpu_in_use.clone_from(&from.gpu_in_use);
            self.gpus.clone_from(&from.gpus);
        }
        self.gpu_notice = from.gpu_notice;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    const ALL: [Section; 4] = [
        Section::Reflections,
        Section::GeneralRender,
        Section::RenderOptions,
        Section::ImageQuality,
    ];

    /// Every setting named above has a row in Preferences, on the page and
    /// tab named: a renamed or moved setting fails here, not on screen.
    #[test]
    fn every_row_is_one_of_preferences() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        for section in ALL {
            for &(page, tab, path) in section.rows() {
                let mut s = Settings::default();
                let d = Settings::default();
                let mut st = OptionsUi::default();
                let mut r = super::super::OptionsResult::default();
                let mut found = false;
                super::super::run_ui(&ctx, egui::RawInput::default(), |ui| {
                    let mut cx = Cx::new(ui, p, &mut s, &d, &mut st, &mut r, super::super::cx::Mode::Collect, "");
                    cx.enter(page, tab);
                    super::super::pages::draw(&mut cx, page, tab);
                    found = cx.collected.iter().any(|(pg, name)| *pg == page && *name == path);
                });
                assert!(found, "{path} is not a row of {page:?} tab {tab}");
            }
        }
    }

    /// Each section draws its rows: the borrowed declarations reach the
    /// screen, in a window as wide as Phototools'.
    #[test]
    fn each_section_draws_rows() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        for section in ALL {
            let mut s = Settings::default();
            let mut st = OptionsUi::default();
            let mut height = 0.0;
            super::super::run_ui(&ctx, egui::RawInput::default(), |ui| {
                ui.set_width(460.0);
                let before = ui.min_rect().height();
                super::super::phototools_section(ui, &p, &mut s, &mut st, section);
                height = ui.min_rect().height() - before;
            });
            // a row is 34 points at least
            assert!(
                height >= 34.0 * section.rows().len() as f32 * 0.9,
                "{section:?} drew {height} points for {} rows",
                section.rows().len()
            );
        }
    }
}

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

use super::cx::{Cx, Mode};
use super::{OptionsResult, OptionsUi, Page, pages, run_ui};
use std::collections::HashMap;
use std::sync::OnceLock;

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
    const ALL: [Section; 4] = [
        Section::Reflections,
        Section::GeneralRender,
        Section::RenderOptions,
        Section::ImageQuality,
    ];

    fn rows(self) -> &'static [Row] {
        match self {
            Section::Reflections => REFLECTIONS,
            Section::GeneralRender => GENERAL_RENDER,
            Section::RenderOptions => RENDER_OPTIONS,
            Section::ImageQuality => IMAGE_QUALITY,
        }
    }
}

/// Rows of one page tab that the page already declares in the order of the
/// section: one `borrow` (one pass over the page's declarations) draws them.
struct Run {
    page: Page,
    tab: usize,
    paths: Vec<&'static str>,
}

/// The settings of a tab in the order its page declares them.
fn page_order(page: Page, tab: usize) -> Vec<&'static str> {
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let (mut s, d) = (crate::settings::Settings::default(), crate::settings::Settings::default());
    let (mut st, mut r) = (OptionsUi::default(), OptionsResult::default());
    let mut order = Vec::new();
    run_ui(&ctx, egui::RawInput::default(), |ui| {
        let mut cx = Cx::new(ui, p, &mut s, &d, &mut st, &mut r, Mode::Collect, "");
        cx.enter(page, tab);
        pages::draw(&mut cx, page, tab);
        order = cx.collected.iter().filter(|(pg, _)| *pg == page).map(|(_, name)| *name).collect();
    });
    order
}

/// The section's rows cut into the fewest runs that keep the Legacy order:
/// a `borrow` draws a page's rows in the page's order, so a row the page
/// declares before the one drawn just above starts a new run.
fn build_runs(rows: &[Row]) -> Vec<Run> {
    let mut orders: HashMap<(Page, usize), Vec<&'static str>> = HashMap::new();
    let mut runs: Vec<Run> = Vec::new();
    let mut last = 0;
    for &(page, tab, path) in rows {
        let order = orders.entry((page, tab)).or_insert_with(|| page_order(page, tab));
        // a name no page declares gets a run of its own (the test says so)
        let at = order.iter().position(|name| *name == path).unwrap_or(usize::MAX);
        match runs.last_mut() {
            Some(run) if run.page == page && run.tab == tab && at > last && at != usize::MAX => run.paths.push(path),
            _ => runs.push(Run {
                page,
                tab,
                paths: vec![path],
            }),
        }
        last = at;
    }
    runs
}

/// The runs of every section, worked out once.
fn runs(section: Section) -> &'static [Run] {
    static RUNS: OnceLock<Vec<Vec<Run>>> = OnceLock::new();
    let all = RUNS.get_or_init(|| Section::ALL.iter().map(|s| build_runs(s.rows())).collect());
    &all[section as usize]
}

/// The rows of `section`. The caller's `Cx` is never `enter`ed on a page:
/// the rows would then take the ids of the same rows in Preferences, and two
/// windows drawing one id at once is a warning in a development build.
pub(super) fn draw(cx: &mut Cx, section: Section) {
    // the collapsing section names the group
    cx.group("", |cx| {
        for run in runs(section) {
            cx.borrow(run.page, run.tab, &run.paths);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    /// Every setting named above has a row in Preferences, on the page and
    /// tab named: a renamed or moved setting fails here, not on screen.
    #[test]
    fn every_row_is_one_of_preferences() {
        for section in Section::ALL {
            for &(page, tab, path) in section.rows() {
                assert!(page_order(page, tab).contains(&path), "{path} is not a row of {page:?} tab {tab}");
            }
        }
    }

    /// The runs keep the Legacy order and are fewer than the rows.
    #[test]
    fn the_runs_keep_the_legacy_order() {
        for section in Section::ALL {
            let flat: Vec<&str> = runs(section).iter().flat_map(|r| r.paths.iter().copied()).collect();
            let legacy: Vec<&str> = section.rows().iter().map(|r| r.2).collect();
            assert_eq!(flat, legacy, "{section:?}");
        }
        let passes: usize = Section::ALL.iter().map(|s| runs(*s).len()).sum();
        let rows: usize = Section::ALL.iter().map(|s| s.rows().len()).sum();
        assert!(passes < rows, "{passes} passes for {rows} rows");
    }

    /// Each section draws exactly its rows, in a window as wide as
    /// Phototools': every borrowed declaration reaches the screen.
    #[test]
    fn each_section_draws_its_rows() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        for section in Section::ALL {
            let (mut s, d) = (Settings::default(), Settings::default());
            let (mut st, mut r) = (OptionsUi::default(), OptionsResult::default());
            let mut drawn = 0;
            run_ui(&ctx, egui::RawInput::default(), |ui| {
                ui.set_width(460.0);
                let mut cx = Cx::new(ui, p, &mut s, &d, &mut st, &mut r, Mode::Show, "");
                draw(&mut cx, section);
                drawn = cx.drawn;
            });
            assert_eq!(drawn, section.rows().len(), "{section:?}");
        }
    }

    /// A change of a borrowed row sets `changed` and writes the setting
    /// Preferences' row edits.
    #[test]
    fn a_borrowed_rows_change_sets_changed() {
        use crate::ui::widgets::elide_tests::Frames;
        let label = super::super::describe_settings()
            .into_iter()
            .find(|(path, ..)| path == "occlusion_culling")
            .expect("the row")
            .1;
        let p = crate::theme::Theme::default().palette();
        let mut flipped = false;
        // the switch sits at the right end of the row: try the last points
        for back in (6..60).step_by(6) {
            let mut frames = Frames::new();
            let (mut s, mut st) = (Settings::default(), OptionsUi::default());
            let before = s.occlusion_culling;
            let mut changed = false;
            let mut frame = |events, s: &mut Settings, st: &mut OptionsUi| {
                frames.run(events, |ui| {
                    ui.set_width(460.0);
                    changed |= super::super::phototools_section(ui, &p, s, st, Section::RenderOptions).changed;
                })
            };
            frame(Vec::new(), &mut s, &mut st);
            let texts = frame(Vec::new(), &mut s, &mut st);
            let row = texts.iter().find(|t| t.source == label).expect("its label").rect;
            let at = egui::pos2(460.0 - back as f32, row.center().y);
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            for events in [
                vec![egui::Event::PointerMoved(at)],
                vec![button(true)],
                vec![button(false)],
                Vec::new(),
            ] {
                frame(events, &mut s, &mut st);
            }
            if s.occlusion_culling != before {
                assert!(changed, "the row changed the setting without saying so");
                flipped = true;
                break;
            }
        }
        assert!(flipped, "no click on the row's right end flipped the setting");
    }
}

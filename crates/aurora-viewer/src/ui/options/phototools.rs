//! The rows of the Phototools window (`ui/phototools.rs`), taken from the
//! Preferences pages that own them (`Cx::borrow`: their own declarations,
//! so the same setting, label, help, range, clamp, "modified" dot and RLVa
//! greying, and no second copy of any of them). Each Legacy group box of
//! floater_phototools.xml (EnvironmentTab, P_Lighting, P_DoF_Glow,
//! P_General) is one [`Section`], drawn inside a collapsing section of the
//! window, its rows in the Legacy order.
//!
//! While the photo window is open (a [`PhotoLayer`]) the rows it overrides
//! are the photo's own controls instead (`photo_row`): the depth of field's
//! switch, aperture and focus, the shadow quality, the ambient occlusion,
//! the worn lights and the exposure. Every other row stays Preferences'.
//! Closed, all of them are Preferences' rows.
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
//! - Obsolete, Light and Depth of field & glow tabs:
//!   - RenderShaderLightingMaxLevel ("Local lights detail"): a shader-class
//!     workaround of the Legacy renderer, "Sun/Moon only" is a number of
//!     nearby lights of 0;
//!   - the shadow resolution scale folds into the shadow quality levels;
//!     the blur distance factor, the FOV cutoff, the spot offset and the
//!     split exponent are internals of the Legacy shadow map (Aurora's
//!     cascades are set by the level);
//!   - the SSAO scale, max scale, factor, irradiance and effect: parameters
//!     of the Legacy SSAO (Aurora draws GTAO or its own port, by `ao_method`
//!     and `gtao_quality`);
//!   - the glow's HDR, resolution and iterations: pass internals (Aurora has
//!     `glow`, `bloom` and `bloom_strength`);
//!   - "Show the Current FOV of Viewer Screen": a camera debug text, with
//!     the camera position data (a later change of the Aids tab).
//! - Missing, for later (a settings change of their own): the shadow blur
//!   size and softening, the shadow bias, the AO softening and the glow's
//!   strength and width.
//! - Missing, for later (a renderer or settings change of their own):
//!   - RenderReflectionProbeResolution: fixed at 128 px (`PROBE_RES`);
//!   - RenderAvatarLODFactor ("Avi Detail"): no avatar mesh LOD factor;
//!   - RenderTerrainScale: fixed at 12 m per texture repeat;
//!   - RenderNormalMapScale ("Texture Bumpiness"): the normals built from
//!     bump heights use a fixed scale of 64, the Legacy default.

use super::cx::{Cx, Mode, Nothing, field};
use super::{OptionsResult, OptionsUi, Page, pages, run_ui};
use crate::snapshot::light::{EXPOSURE, PhotoLight, photo_level};
use crate::ui::widgets;
use egui::{Sense, Vec2};
use std::collections::HashMap;
use std::sync::OnceLock;

/// What the app tells the rows about the photo window, which is open.
#[derive(Clone, Copy, Debug, Default)]
pub struct PhotoState {
    /// The sky's own sun (azimuth, elevation), what the sun sliders restore.
    pub sun: (f32, f32),
    /// An item the photo picked is still loading.
    pub loading: bool,
    /// The viewfinder is on: the view angle is the photo's.
    pub viewfinder: bool,
    /// The photo quality's boost is on: the rows show the raised values.
    pub boost: bool,
}

/// The photo window's values while it is open: the rows it overrides edit
/// these instead of the saved settings.
pub struct PhotoLayer<'a> {
    pub light: &'a mut PhotoLight,
    pub state: PhotoState,
    /// A photo's value changed (not a saved setting): its preview follows.
    pub touched: bool,
}

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
    /// Light tab, "Lights and Shadows Settings".
    Shadows,
    /// Light tab, "Ambient Occlusion Settings".
    AmbientOcclusion,
    /// DoF & glow tab, "Depth of Field Settings": the switch and the focus.
    DepthOfField,
    /// DoF & glow tab, the lens: view angle, aperture, focal length,
    /// transition and blur.
    Lens,
    /// DoF & glow tab, "HDR Tone Mapping Settings".
    ToneMapping,
    /// DoF & glow tab, "Glow Settings".
    Glow,
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

/// Shadow Detail, "Enable Attached Lights (Face Lights)", then the nearby
/// lights (RenderLocalLightCount).
const SHADOWS: &[Row] = &[
    (Page::Graphics, 1, "shadow_quality"),
    (Page::Graphics, 1, "projector_shadows"),
    (Page::Graphics, 1, "others_attached_lights"),
    (Page::Graphics, 1, "local_light_count"),
];

/// "Ambient Occlusion (Depth Perception)" and its method.
const AMBIENT_OCCLUSION: &[Row] = &[
    (Page::Graphics, 1, "ssao"),
    (Page::Graphics, 1, "ao_method"),
    (Page::Graphics, 1, "gtao_quality"),
];

/// "Enable Depth of Field (DoF)", the focus (FSFocusPointFollowsPointer
/// and its kin) and "Show the focus point" (FSFocusPointRender).
const DEPTH_OF_FIELD: &[Row] = &[
    (Page::Graphics, 4, "dof.enabled"),
    (Page::Graphics, 4, "dof.focus"),
    (Page::Graphics, 4, "dof.show_focus_point"),
];

/// The Legacy sliders: View Angle (CameraAngle), Field of View
/// (CameraFieldOfView), F-number, Focal Length, Focus Time, Max Blur
/// (CameraMaxCoF), Resolution (CameraDoFResScale).
const LENS: &[Row] = &[
    (Page::Camera, 0, "camera.fov"),
    (Page::Graphics, 4, "dof.reference_fov"),
    (Page::Graphics, 4, "dof.f_number"),
    (Page::Graphics, 4, "dof.focal_length"),
    (Page::Graphics, 4, "dof.transition"),
    (Page::Graphics, 4, "dof.max_coc"),
    (Page::Graphics, 4, "dof.quality"),
];

/// "Enable HDR and Emissive", the tone mapping type and its mix.
const TONE_MAPPING: &[Row] = &[
    (Page::Graphics, 4, "hdr_emissive"),
    (Page::Graphics, 4, "tonemapper"),
    (Page::Graphics, 4, "tonemap_mix"),
];

/// "Enable Glow Effect" (and Aurora's bloom).
const GLOW: &[Row] = &[
    (Page::Graphics, 1, "glow"),
    (Page::Graphics, 1, "bloom"),
    (Page::Graphics, 1, "bloom_strength"),
];

/// A row the photo window overrides: while it is open, the row is the
/// photo's own control (`PhotoLight`, `settings.snapshot.dof`), not the
/// saved setting's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PhotoRow {
    DofEnabled,
    DofFNumber,
    DofFocus,
    ShadowQuality,
    AmbientOcclusion,
    AttachedLights,
    Exposure,
}

/// The photo's control that stands for the Preferences row of `path`, while
/// the photo window is open. The view angle is no row of this list: the
/// viewfinder writes its focal length into `camera.fov` itself, so the
/// Preferences row already is the photo's while it is on. Everything else
/// (the DoF's focal length, blur, quality; the projector shadows; the
/// tone mapping...) keeps its saved setting, which the photo uses too.
fn photo_row(path: &str) -> Option<PhotoRow> {
    Some(match path {
        "dof.enabled" => PhotoRow::DofEnabled,
        "dof.f_number" => PhotoRow::DofFNumber,
        "dof.focus" => PhotoRow::DofFocus,
        "shadow_quality" => PhotoRow::ShadowQuality,
        "ssao" => PhotoRow::AmbientOcclusion,
        "others_attached_lights" => PhotoRow::AttachedLights,
        "exposure" => PhotoRow::Exposure,
        _ => return None,
    })
}

impl Section {
    const ALL: [Section; 10] = [
        Section::Reflections,
        Section::GeneralRender,
        Section::RenderOptions,
        Section::ImageQuality,
        Section::Shadows,
        Section::AmbientOcclusion,
        Section::DepthOfField,
        Section::Lens,
        Section::ToneMapping,
        Section::Glow,
    ];

    fn rows(self) -> &'static [Row] {
        match self {
            Section::Reflections => REFLECTIONS,
            Section::GeneralRender => GENERAL_RENDER,
            Section::RenderOptions => RENDER_OPTIONS,
            Section::ImageQuality => IMAGE_QUALITY,
            Section::Shadows => SHADOWS,
            Section::AmbientOcclusion => AMBIENT_OCCLUSION,
            Section::DepthOfField => DEPTH_OF_FIELD,
            Section::Lens => LENS,
            Section::ToneMapping => TONE_MAPPING,
            Section::Glow => GLOW,
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

/// One drawing step of a section: a pass over a Preferences tab, or the
/// photo's own control of a row.
enum Step {
    Borrow(Run),
    Photo(PhotoRow),
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

/// The section's rows cut into the fewest steps that keep the Legacy order:
/// a `borrow` draws a page's rows in the page's order, so a row the page
/// declares before the one drawn just above starts a new run, and a row the
/// photo window overrides (`open`) is a step of its own.
fn build_steps(rows: &[Row], open: bool) -> Vec<Step> {
    let mut orders: HashMap<(Page, usize), Vec<&'static str>> = HashMap::new();
    let mut steps: Vec<Step> = Vec::new();
    let mut last = 0;
    for &(page, tab, path) in rows {
        if open && let Some(kind) = photo_row(path) {
            steps.push(Step::Photo(kind));
            continue;
        }
        let order = orders.entry((page, tab)).or_insert_with(|| page_order(page, tab));
        // a name no page declares gets a run of its own (the test says so)
        let at = order.iter().position(|name| *name == path).unwrap_or(usize::MAX);
        match steps.last_mut() {
            Some(Step::Borrow(run)) if run.page == page && run.tab == tab && at > last && at != usize::MAX => run.paths.push(path),
            _ => steps.push(Step::Borrow(Run {
                page,
                tab,
                paths: vec![path],
            })),
        }
        last = at;
    }
    steps
}

/// The steps of every section, closed and open, worked out once.
fn steps(section: Section, open: bool) -> &'static [Step] {
    static STEPS: OnceLock<Vec<[Vec<Step>; 2]>> = OnceLock::new();
    let all = STEPS.get_or_init(|| {
        Section::ALL
            .iter()
            .map(|s| [build_steps(s.rows(), false), build_steps(s.rows(), true)])
            .collect()
    });
    &all[section as usize][usize::from(open)]
}

/// The rows of `section`; `photo` is the photo window's layer while it is
/// open. The caller's `Cx` is never `enter`ed on a page: the rows would then
/// take the ids of the same rows in Preferences, and two windows drawing
/// one id at once is a warning in a development build.
pub(super) fn draw(cx: &mut Cx, section: Section, mut photo: Option<&mut PhotoLayer>) {
    // the collapsing section names the group
    cx.group("", |cx| {
        for step in steps(section, photo.is_some()) {
            match step {
                Step::Borrow(run) => {
                    // The rows that sit under a switch grey with it: under
                    // the photo's layer that is the photo's switch and
                    // level, which the rows' own declarations read from the
                    // saved settings. None of these settings is edited by a
                    // borrowed row of this pass.
                    let saved = photo.as_deref().map(|photo| {
                        let saved = (cx.s.dof.enabled, cx.s.shadow_quality, cx.s.ssao);
                        let boost = photo.state.boost;
                        cx.s.dof.enabled = cx.s.snapshot.dof.enabled;
                        cx.s.shadow_quality = photo_level(true, photo.light.shadow_quality, saved.1, boost);
                        cx.s.ssao = photo_level(true, photo.light.ambient_occlusion, saved.2, boost);
                        saved
                    });
                    cx.borrow(run.page, run.tab, &run.paths);
                    if let Some((enabled, shadow_quality, ssao)) = saved {
                        cx.s.dof.enabled = enabled;
                        cx.s.shadow_quality = shadow_quality;
                        cx.s.ssao = ssao;
                    }
                }
                Step::Photo(kind) => {
                    if let Some(photo) = photo.as_deref_mut() {
                        photo_control(cx, *kind, photo);
                    }
                }
            }
        }
    });
}

/// The mark of a row that is the photo's: the camera, which says on hover
/// that the change holds until the photo window closes.
fn photo_mark(ui: &mut egui::Ui, p: &crate::theme::Palette) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
    widgets::paint_glyph(ui.painter(), "camera-filled", rect.center(), 14.0, p.violet_light);
    resp.on_hover_text(aurora_i18n::tr!("phototools-photo-mark"));
}

/// A slider on one of the photo's values, on Preferences' look.
fn photo_slider(value: &mut f32, range: std::ops::RangeInclusive<f32>) -> egui::Slider<'_> {
    egui::Slider::new(value, range)
        .custom_parser(aurora_i18n::parse_number)
        .trailing_fill(true)
}

/// The photo's own control of a row it overrides (`photo_row`), drawn as
/// the Preferences row it replaces: its label and help, its place, the
/// photo's mark before the control.
fn photo_control(cx: &mut Cx, kind: PhotoRow, photo: &mut PhotoLayer) {
    use crate::dof::PhotoFocus;
    use aurora_i18n::tr;
    let p = cx.p;
    let PhotoLayer { light, state, touched } = photo;
    let boost = state.boost;
    match kind {
        PhotoRow::DofEnabled => {
            cx.row(
                tr!("ui-options-dof-enable"),
                tr!("photo-dof-enable-hint"),
                &field!(snapshot.dof.enabled),
                |ui, s| {
                    photo_mark(ui, &p);
                    super::toggle(ui, &p, &mut s.snapshot.dof.enabled)
                },
            );
        }
        PhotoRow::DofFNumber => {
            cx.row(
                tr!("ui-options-dof-aperture"),
                tr!("ui-options-dof-aperture-help"),
                &field!(snapshot.dof.f_number),
                |ui, s| {
                    photo_mark(ui, &p);
                    let on = s.snapshot.dof.enabled;
                    // (the photo's range, PhotoDof::sanitize)
                    ui.add_enabled(
                        on,
                        photo_slider(&mut s.snapshot.dof.f_number, 1.0..=22.0)
                            .logarithmic(true)
                            .prefix("f/")
                            .fixed_decimals(1),
                    )
                    .changed()
                },
            );
        }
        PhotoRow::DofFocus => {
            cx.row(
                tr!("ui-options-dof-focus"),
                tr!("phototools-photo-focus-help"),
                &field!(snapshot.dof.focus),
                |ui, s| {
                    photo_mark(ui, &p);
                    let on = s.snapshot.dof.enabled;
                    ui.add_enabled_ui(on, |ui| {
                        let modes = [PhotoFocus::Click, PhotoFocus::Pointer, PhotoFocus::Manual];
                        let mut i = modes.iter().position(|m| *m == s.snapshot.dof.focus).unwrap_or(0);
                        let options = [
                            (tr!("photo-dof-focus-click"), true),
                            (tr!("photo-dof-focus-pointer"), true),
                            (tr!("photo-dof-focus-manual"), true),
                        ];
                        let changed = widgets::segmented(ui, &p, &mut i, &options);
                        if changed {
                            s.snapshot.dof.focus = modes[i];
                        }
                        changed
                    })
                    .inner
                },
            );
            if cx.s.snapshot.dof.focus == PhotoFocus::Manual {
                cx.row(
                    tr!("photo-dof-distance"),
                    tr!("phototools-photo-distance-help"),
                    &field!(snapshot.dof.distance),
                    |ui, s| {
                        photo_mark(ui, &p);
                        let on = s.snapshot.dof.enabled;
                        ui.add_enabled(
                            on,
                            photo_slider(&mut s.snapshot.dof.distance, 0.3..=200.0)
                                .logarithmic(true)
                                .suffix(" m")
                                .fixed_decimals(1),
                        )
                        .changed()
                    },
                );
            }
        }
        PhotoRow::ShadowQuality => {
            let levels = [
                tr!("ui-options-disabled-2"),
                tr!("ui-options-low"),
                tr!("common-medium"),
                tr!("ui-options-high"),
                tr!("ui-options-ultra"),
            ];
            cx.row(
                tr!("ui-options-shadow-quality"),
                tr!("ui-options-cascaded-shadow-resolution-and-distance"),
                &Nothing,
                |ui, s| {
                    photo_mark(ui, &p);
                    photo_level_control(ui, &p, &mut light.shadow_quality, s.shadow_quality, boost, &levels, touched);
                    false
                },
            );
        }
        PhotoRow::AmbientOcclusion => {
            let levels = [tr!("ui-options-disabled"), tr!("ui-options-low"), tr!("ui-options-high")];
            cx.row(
                tr!("ui-options-ambient-occlusion"),
                tr!("ui-options-darkens-corners-and-contact-areas-ssao"),
                &Nothing,
                |ui, s| {
                    photo_mark(ui, &p);
                    photo_level_control(ui, &p, &mut light.ambient_occlusion, s.ssao, boost, &levels, touched);
                    false
                },
            );
        }
        PhotoRow::AttachedLights => {
            cx.row(tr!("photo-light-attached"), tr!("photo-light-attached-hint"), &Nothing, |ui, _| {
                photo_mark(ui, &p);
                if super::toggle(ui, &p, &mut light.attached_lights) {
                    *touched = true;
                }
                false
            });
        }
        PhotoRow::Exposure => {
            cx.row(tr!("ui-options-exposure"), "", &Nothing, |ui, s| {
                photo_mark(ui, &p);
                // (greyed without HDR, as Preferences' own row)
                let r = ui.add_enabled_ui(s.hdr_emissive, |ui| {
                    // (shown, the sky's exposure must not turn into a fixed one)
                    let mut value = light.exposure.unwrap_or(s.exposure);
                    if ui.add(photo_slider(&mut value, EXPOSURE).logarithmic(true)).changed() {
                        light.exposure = Some(value);
                        *touched = true;
                    }
                    if widgets::icon_button(ui, &p, "restore", tr!("photo-light-exposure-reset"), light.exposure.is_some()) {
                        light.exposure = None;
                        *touched = true;
                    }
                });
                r.response.on_disabled_hover_text(tr!("ui-options-needs-hdr-emissive"));
                false
            });
        }
    }
}

/// A level of the photo's (shadows, occlusion) on Preferences' stepped
/// slider: Preferences' `current` until one is chosen; under the photo
/// quality's boost never past it (`photo_level`); its restore button gives
/// Preferences' back.
fn photo_level_control(
    ui: &mut egui::Ui,
    p: &crate::theme::Palette,
    value: &mut Option<u8>,
    current: u8,
    boost: bool,
    labels: &[&str],
    touched: &mut bool,
) {
    let mut level = photo_level(true, *value, current, boost);
    if super::stepped(ui, p, &mut level, labels) {
        *value = Some(photo_level(true, Some(level), current, boost));
        *touched = true;
    }
    if widgets::icon_button(ui, p, "restore", aurora_i18n::tr!("photo-light-own"), value.is_some()) {
        *value = None;
        *touched = true;
    }
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

    /// The steps keep the Legacy order, with the photo window closed and
    /// open, and are fewer than the rows.
    #[test]
    fn the_steps_keep_the_legacy_order() {
        for open in [false, true] {
            for section in Section::ALL {
                let flat: Vec<&str> = steps(section, open)
                    .iter()
                    .flat_map(|s| match s {
                        Step::Borrow(run) => run.paths.clone(),
                        Step::Photo(_) => vec!["<photo>"],
                    })
                    .collect();
                let legacy: Vec<&str> = section
                    .rows()
                    .iter()
                    .map(|r| if open && photo_row(r.2).is_some() { "<photo>" } else { r.2 })
                    .collect();
                assert_eq!(flat, legacy, "{section:?}, photo window open: {open}");
            }
        }
        let passes: usize = Section::ALL.iter().map(|s| steps(*s, false).len()).sum();
        let rows: usize = Section::ALL.iter().map(|s| s.rows().len()).sum();
        assert!(passes < rows, "{passes} passes for {rows} rows");
    }

    /// Each section draws exactly its rows, in a window as wide as
    /// Phototools': every declaration reaches the screen, with the photo
    /// window closed and open.
    #[test]
    fn each_section_draws_its_rows() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        for open in [false, true] {
            for section in Section::ALL {
                let (mut s, d) = (Settings::default(), Settings::default());
                let (mut st, mut r) = (OptionsUi::default(), OptionsResult::default());
                let mut light = PhotoLight::default();
                let mut drawn = 0;
                run_ui(&ctx, egui::RawInput::default(), |ui| {
                    ui.set_width(460.0);
                    let mut layer = PhotoLayer {
                        light: &mut light,
                        state: PhotoState::default(),
                        touched: false,
                    };
                    let mut cx = Cx::new(ui, p, &mut s, &d, &mut st, &mut r, Mode::Show, "");
                    draw(&mut cx, section, open.then_some(&mut layer));
                    drawn = cx.drawn;
                });
                assert_eq!(drawn, section.rows().len(), "{section:?}, photo window open: {open}");
            }
        }
    }

    /// Clicks the switch at the right end of the row labelled `label`.
    /// Returns whether `flipped` saw a change.
    fn click_switch(
        label: &str,
        section: Section,
        s: &mut Settings,
        light: &mut PhotoLight,
        open: bool,
        flipped: impl Fn(&Settings, &PhotoLight) -> bool,
    ) -> bool {
        use crate::ui::widgets::elide_tests::Frames;
        let p = crate::theme::Theme::default().palette();
        let mut st = OptionsUi::default();
        // the switch sits at the right end of the row: try the last points
        for back in (6..60).step_by(6) {
            let mut frames = Frames::new();
            let (mut s2, mut light2) = (s.clone(), *light);
            let mut frame = |events, s: &mut Settings, light: &mut PhotoLight, st: &mut OptionsUi| {
                frames.run(events, |ui| {
                    ui.set_width(460.0);
                    let mut layer = PhotoLayer {
                        light,
                        state: PhotoState::default(),
                        touched: false,
                    };
                    super::super::phototools_section(ui, &p, s, st, section, open.then_some(&mut layer));
                })
            };
            frame(Vec::new(), &mut s2, &mut light2, &mut st);
            let texts = frame(Vec::new(), &mut s2, &mut light2, &mut st);
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
                frame(events, &mut s2, &mut light2, &mut st);
            }
            if flipped(&s2, &light2) {
                *s = s2;
                *light = light2;
                return true;
            }
        }
        false
    }

    /// A change of a borrowed row sets `changed` and writes the setting
    /// Preferences' row edits.
    #[test]
    fn a_borrowed_rows_change_sets_changed() {
        let label = super::super::describe_settings()
            .into_iter()
            .find(|(path, ..)| path == "occlusion_culling")
            .expect("the row")
            .1;
        let (mut s, mut light) = (Settings::default(), PhotoLight::default());
        let before = s.occlusion_culling;
        assert!(click_switch(&label, Section::RenderOptions, &mut s, &mut light, false, |s, _| s
            .occlusion_culling
            != before));
    }

    /// The worn lights row is the photo's while its window is open, and
    /// Preferences' while it is closed: an edit never reaches the other.
    #[test]
    fn an_edit_goes_to_the_layer_that_is_shown() {
        let photo_label = aurora_i18n::tr!("photo-light-attached");
        let own = Settings::default().others_attached_lights;
        let (mut s, mut light) = (Settings::default(), PhotoLight::default());
        assert!(click_switch(photo_label, Section::Shadows, &mut s, &mut light, true, |_, l| !l.attached_lights));
        assert_eq!(
            s.others_attached_lights, own,
            "the saved setting is untouched while the photo window is open"
        );

        let saved_label = super::super::describe_settings()
            .into_iter()
            .find(|(path, ..)| path == "others_attached_lights")
            .expect("the row")
            .1;
        let (mut s, mut light) = (Settings::default(), PhotoLight::default());
        assert!(click_switch(&saved_label, Section::Shadows, &mut s, &mut light, false, |s, _| s
            .others_attached_lights
            != own));
        assert!(light.attached_lights, "the photo's value is untouched while it is closed");
    }
}

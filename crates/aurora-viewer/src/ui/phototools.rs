//! "Phototools" (the toolbar's `phototools` command): the settings that
//! matter to a photographer, in one window.
//!
//! Port of FloaterPhototools (newview/quickprefs.cpp,
//! floater_phototools.xml, originally LGPL 2.1; getIsPhototools). The tabs
//! come in the Legacy Viewer's order (Environment, Light, Depth of field &
//! glow, General, Aids, Camera) and each of its group boxes is a collapsing
//! section, in its order. Only the tabs built so far show: Environment,
//! Light, Depth of field & glow and General. The others come with their own
//! changes and appear then.
//!
//! The window follows the view. With the photo window closed, a row changes
//! the saved setting. While the photo window is open, the rows it
//! overrides (the depth of field's switch, aperture and focus, the
//! exposure, the sky, water, day and time of day, the paused clouds, the
//! worn lights, the shadow quality and the ambient occlusion) change the
//! photo's values instead, under a banner that says so, until the window
//! closes (`options/phototools.rs`, `PhotoLayer`). "Keep this light" of the
//! banner is the photo window's own: the environment only.
//!
//! The rows are Preferences' own (`options/phototools.rs` borrows them from
//! the pages: same setting, range, clamp, help, "modified" dot and RLVa
//! greying), so a setting is changed the same way here and there and
//! applied and saved like a change of Preferences. The environment block
//! is the Environment window's and Quick preferences' own
//! (`EnvironmentUi::selector_body`).

use super::environment::{EnvAction, EnvironmentUi};
use super::options::{self, OptionsUi, PhotoLayer, PhotoSection, PhotoState};
use super::snapshot::{self, EnvView};
use super::widgets::{self, ActionKind, Floater};
use crate::settings::Settings;
use crate::snapshot::light::PhotoLight;
use crate::theme::Palette;
use crate::world::eep_env::LocalView;
use crate::world::eep_photo::PhotoChoice;
use crate::world::env_select::SettingsKind;
use egui::{RichText, Vec2};

/// The window's id (Floater.Toggle "phototools").
pub const WINDOW: &str = "phototools";

const WIDTH: f32 = 480.0;
/// The labels' column of the Sky, Water and Day lists: "Water environment"
/// in French is the longest.
const LABEL_W: f32 = 130.0;

/// The tabs built so far, in the Legacy Viewer's order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    /// EnvironmentTab.
    Environment,
    /// P_Lighting.
    Light,
    /// P_DoF_Glow.
    DofGlow,
    /// P_General.
    General,
}

const TABS: [Tab; 4] = [Tab::Environment, Tab::Light, Tab::DofGlow, Tab::General];

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Environment => aurora_i18n::tr!("common-environment"),
            Tab::Light => aurora_i18n::tr!("common-light"),
            Tab::DofGlow => aurora_i18n::tr!("phototools-tab-dof-glow"),
            Tab::General => aurora_i18n::tr!("common-general"),
        }
    }

    /// The tab has rows the photo window overrides, which the banner tells
    /// of.
    fn has_photo_rows(self) -> bool {
        matches!(self, Tab::Environment | Tab::Light | Tab::DofGlow | Tab::General)
    }

    /// `AURORA_DEMO_PHOTOTOOLS=<tab>[:photo]`: the tab, and whether the
    /// photo window is opened too.
    pub fn parse(text: &str) -> Option<(usize, bool)> {
        let lower = text.trim().to_ascii_lowercase();
        let (name, photo) = match lower.split_once(':') {
            Some((name, "photo")) => (name, true),
            Some(_) => return None,
            None => (lower.as_str(), false),
        };
        let tab = match name {
            "env" | "environment" => Tab::Environment,
            "light" | "lighting" => Tab::Light,
            "dof" | "glow" | "dof-glow" => Tab::DofGlow,
            "general" | "gen" => Tab::General,
            _ => return None,
        };
        TABS.iter().position(|t| *t == tab).map(|i| (i, photo))
    }
}

/// The environment's state, from the app (as for Quick preferences).
pub struct Env<'a> {
    pub ui: &'a mut EnvironmentUi,
    pub view: LocalView,
    pub loading: bool,
    /// The shown time of day preset (0: none).
    pub time_of_day: u8,
    /// "Personal lighting" is asked.
    pub lighting: &'a mut bool,
    /// The photo window's light: "Pause clouds" is a state of the session
    /// shared with it (the Legacy Viewer's pauseCloudScroll lasts the
    /// session), and the photo's own values while it is open.
    pub light: &'a mut PhotoLight,
    /// The photo window is open: what the rows need to know of it.
    pub photo: Option<PhotoState>,
    /// The Director's lens is shown, over both layers.
    pub director_dof: bool,
}

/// What the window asks of the app.
#[derive(Default)]
pub struct Out {
    /// The environment block's choices.
    pub env: Vec<EnvAction>,
    /// "New Sky / Water / Day Preset…": the editor to open.
    pub new_preset: Option<SettingsKind>,
    /// A setting changed: apply them like a change of Preferences.
    pub changed: bool,
    /// What the borrowed rows ask of the app besides (Preferences' result).
    pub toggle_flycam: bool,
    /// "Keep this light" of the banner.
    pub keep_light: bool,
    /// A value of the photo changed: its preview follows.
    pub photo_changed: bool,
}

#[derive(Default)]
pub struct PhototoolsUi {
    /// The rows' engine, its own so that the sizes the rows measured are
    /// not Preferences'.
    rows: OptionsUi,
    /// The tab shown (index in `TABS`).
    pub tab: usize,
    /// A setting changed and waits for the button to be released to be saved.
    unsaved: bool,
}

impl PhototoolsUi {
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        open: &mut bool,
        settings: &mut Settings,
        preferences: &OptionsUi,
        mut env: Env<'_>,
        keep_fov: Option<f32>,
    ) -> Out {
        let mut out = Out::default();
        // the pending save first: a drag that ended with the window closed
        // is still saved
        self.save_when_released(ctx, settings, keep_fov);
        if !*open {
            return out;
        }
        self.rows.share_from(preferences);
        let screen = ctx.content_rect();
        let pos = egui::pos2(screen.right() - WIDTH - 300.0, screen.top() + 90.0);
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("toolbar-command-phototools"),
            pos,
            Vec2::new(WIDTH, (screen.height() - 120.0).clamp(260.0, 680.0)),
        );
        floater.min_size = Vec2::new(400.0, 260.0);
        floater.show(ctx, p, open, |ui| {
            self.body(ui, p, settings, &mut env, &mut out);
        });
        self.unsaved |= out.changed;
        self.save_when_released(ctx, settings, keep_fov);
        out
    }

    /// Saved once the button is released, not at every frame of a drag.
    /// `keep_fov`: the user's view angle while the viewfinder is on (it
    /// writes its own focal length into the setting): what is written.
    fn save_when_released(&mut self, ctx: &egui::Context, settings: &Settings, keep_fov: Option<f32>) {
        if self.unsaved && !ctx.input(|i| i.pointer.any_down()) {
            self.unsaved = false;
            settings.save_with_fov(keep_fov);
        }
    }

    fn body(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &mut Settings, env: &mut Env<'_>, out: &mut Out) {
        self.tab = self.tab.min(TABS.len() - 1);
        let labels = TABS.map(|t| (t.label(), true));
        widgets::tabs(ui, p, &mut self.tab, &labels);
        ui.add_space(6.0);
        // (every tab built so far has a row the photo window overrides:
        // the General tab's exposure)
        if let Some(state) = env.photo.filter(|_| TABS[self.tab].has_photo_rows()) {
            banner(ui, p, env, state, out);
        }
        let tab = TABS[self.tab];
        let rows_width = (widgets::scrolled_width(ui) - 6.0).max(0.0);
        egui::ScrollArea::vertical()
            .id_salt(("phototools-tab", self.tab))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(rows_width);
                match tab {
                    Tab::Environment => self.environment(ui, p, settings, env, out),
                    Tab::Light => self.light(ui, p, settings, env, out),
                    Tab::DofGlow => self.dof_glow(ui, p, settings, env, out),
                    Tab::General => self.general(ui, p, settings, env, out),
                }
                ui.add_space(12.0);
            });
    }

    /// One Legacy group box.
    fn rows(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        settings: &mut Settings,
        env: &mut Env<'_>,
        out: &mut Out,
        title: &str,
        section: PhotoSection,
    ) {
        // the id is the section's, not its translated title
        widgets::collapsing(ui, p, ("phototools", section as u8), title, true, |ui| {
            self.rows_of(ui, p, settings, env, out, section);
        });
    }

    /// The rows of one Legacy group box, the photo window's own where it
    /// is open and overrides them.
    fn rows_of(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        settings: &mut Settings,
        env: &mut Env<'_>,
        out: &mut Out,
        section: PhotoSection,
    ) {
        let mut layer = env.photo.map(|state| PhotoLayer {
            light: &mut *env.light,
            state,
            touched: false,
        });
        let r = options::phototools_section(ui, p, settings, &mut self.rows, section, layer.as_mut());
        // a value of the photo changed: the preview follows, and the photo's
        // depth of field (a saved setting) is written once the button is
        // released, without the app's pass for a changed setting
        if layer.is_some_and(|l| l.touched) {
            out.photo_changed = true;
            self.unsaved = true;
        }
        out.changed |= r.changed;
        out.toggle_flycam |= r.toggle_flycam;
    }

    /// EnvironmentTab: Environment Settings, Quick Environments,
    /// Environment Presets, Reflection Settings.
    fn environment(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &mut Settings, env: &mut Env<'_>, out: &mut Out) {
        use aurora_i18n::tr;
        widgets::collapsing(
            ui,
            p,
            ("phototools", "settings"),
            tr!("phototools-environment-settings"),
            true,
            |ui| {
                match env.photo {
                    // the photo's own lists and time of day, the photo
                    // window's controls
                    Some(state) => {
                        let view = EnvView {
                            catalog: env.ui.catalog(),
                            own: env.view,
                            sun: state.sun,
                            locked: env.ui.rlva_locked,
                            loading: state.loading,
                        };
                        let before = env.light.env;
                        ui.add_enabled_ui(!view.locked, |ui| snapshot::env_rows(ui, p, env.light, &view))
                            .response
                            .on_disabled_hover_text(tr!("rlva-blocked"));
                        out.photo_changed |= env.light.env != before;
                        ui.add_space(4.0);
                        if widgets::flat_button(ui, p, tr!("common-personal-lighting")).clicked() {
                            *env.lighting = true;
                        }
                    }
                    None => env
                        .ui
                        .selector_body(ui, p, env.view, env.loading, None, LABEL_W, env.lighting, None, &mut out.env),
                }
                ui.add_space(4.0);
                pause_clouds(ui, p, env, out);
            },
        );
        let free = !env.ui.rlva_locked;
        widgets::collapsing(ui, p, ("phototools", "quick"), tr!("phototools-quick-environments"), true, |ui| {
            ui.add_enabled_ui(free, |ui| {
                // the photo window's own time row (above) stands for the times of
                // day while it is open
                ui.horizontal_wrapped(|ui| {
                    let times = [
                        tr!("ui-environment-sunrise"),
                        tr!("common-midday"),
                        tr!("ui-environment-lie-down"),
                        tr!("common-midnight"),
                    ];
                    for (i, name) in times.iter().enumerate().filter(|_| env.photo.is_none()) {
                        let preset = i as u8 + 1;
                        if widgets::underlined_button(ui, p, name, env.time_of_day == preset)
                            .on_hover_text(super::bars::time_of_day()[usize::from(preset)])
                            .clicked()
                        {
                            // FSRepeatedEnvTogglesShared is off by default:
                            // the same preset again stays
                            out.env.push(EnvAction::Preset(preset));
                        }
                    }
                });
                if widgets::flat_button(ui, p, tr!("ui-environment-shared-environment"))
                    .on_hover_text(tr!("ui-environment-return-to-shared-environment"))
                    .clicked()
                {
                    if env.photo.is_some() {
                        // the photo's own environment is the user's again
                        env.light.env = PhotoChoice::default();
                        out.photo_changed = true;
                    } else {
                        out.env.push(EnvAction::Shared);
                    }
                }
            });
        });
        widgets::collapsing(
            ui,
            p,
            ("phototools", "presets"),
            tr!("phototools-environment-presets"),
            true,
            |ui| {
                ui.add_enabled_ui(free, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (kind, icon, label) in [
                            (SettingsKind::Sky, "sun-filled", tr!("ui-bars-new-sky")),
                            (SettingsKind::Water, "droplet-filled", tr!("ui-bars-new-water")),
                            (SettingsKind::Day, "sunset-2-filled", tr!("ui-bars-new-day-cycle")),
                        ] {
                            if widgets::action_button(ui, p, icon, label, ActionKind::Flat, Vec2::new(0.0, 26.0)).clicked() {
                                out.new_preset = Some(kind);
                            }
                        }
                    });
                });
            },
        );
        self.rows(ui, p, settings, env, out, tr!("ui-options-reflections"), PhotoSection::Reflections);
    }

    /// P_Lighting: Lights and Shadows Settings, Ambient Occlusion Settings.
    fn light(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &mut Settings, env: &mut Env<'_>, out: &mut Out) {
        use aurora_i18n::tr;
        self.rows(ui, p, settings, env, out, tr!("phototools-lights-shadows"), PhotoSection::Shadows);
        self.rows(
            ui,
            p,
            settings,
            env,
            out,
            tr!("phototools-ambient-occlusion"),
            PhotoSection::AmbientOcclusion,
        );
    }

    /// P_DoF_Glow: Depth of Field Settings (the focus, then the lens),
    /// HDR Tone Mapping Settings, Glow Settings.
    fn dof_glow(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &mut Settings, env: &mut Env<'_>, out: &mut Out) {
        use aurora_i18n::tr;
        widgets::collapsing(
            ui,
            p,
            ("phototools", PhotoSection::DepthOfField as u8),
            tr!("phototools-dof"),
            true,
            |ui| {
                if env.director_dof {
                    note(ui, p, tr!("phototools-director-lens"));
                }
                self.rows_of(ui, p, settings, env, out, PhotoSection::DepthOfField);
                if env.photo.is_some_and(|state| state.viewfinder) {
                    note(ui, p, tr!("phototools-view-angle-viewfinder"));
                }
                self.rows_of(ui, p, settings, env, out, PhotoSection::Lens);
            },
        );
        self.rows(ui, p, settings, env, out, tr!("phototools-tone-mapping"), PhotoSection::ToneMapping);
        self.rows(ui, p, settings, env, out, tr!("phototools-glow"), PhotoSection::Glow);
    }

    /// P_General: General Render Settings, then the two boxes under it.
    fn general(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &mut Settings, env: &mut Env<'_>, out: &mut Out) {
        use aurora_i18n::tr;
        self.rows(
            ui,
            p,
            settings,
            env,
            out,
            tr!("phototools-general-render"),
            PhotoSection::GeneralRender,
        );
        self.rows(
            ui,
            p,
            settings,
            env,
            out,
            tr!("phototools-render-options"),
            PhotoSection::RenderOptions,
        );
        self.rows(ui, p, settings, env, out, tr!("ui-options-quality"), PhotoSection::ImageQuality);
    }
}

/// "Pause Clouds": a switch of the session, never saved, shared with the
/// photo window's (`snapshots.ui.light.clouds_paused`).
fn pause_clouds(ui: &mut egui::Ui, p: &Palette, env: &mut Env<'_>, out: &mut Out) {
    use aurora_i18n::tr;
    // greyed like the presets under @setenv / @setsphere
    // (FloaterQuickPrefs::onClickPauseClouds, quickprefs.cpp)
    ui.add_enabled_ui(!env.ui.rlva_locked, |ui| {
        let before = env.light.clouds_paused;
        widgets::switch_labeled(ui, p, &mut env.light.clouds_paused, tr!("photo-light-pause-clouds"))
            .on_hover_text(tr!("photo-light-pause-clouds-hint"));
        // the photo's preview shows the clouds held still
        out.photo_changed |= env.light.clouds_paused != before;
    });
}

/// A line of help in a section, muted.
fn note(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(11.5).color(p.muted)).wrap());
    ui.add_space(4.0);
}

/// The banner of the photo window being open: what the rows below change,
/// and its "Keep this light" (`EnvSelector::keep_photo`: the environment
/// only, as the photo window's button).
fn banner(ui: &mut egui::Ui, p: &Palette, env: &mut Env<'_>, state: PhotoState, out: &mut Out) {
    use aurora_i18n::tr;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
        widgets::paint_glyph(ui.painter(), "camera-filled", rect.center(), 14.0, p.violet_light);
        ui.label(RichText::new(tr!("phototools-photo-banner")).size(12.0).color(p.ink));
    });
    // on a line of its own: the text takes the width
    ui.add_space(2.0);
    let keep = !env.light.env.is_empty() && !state.loading && !env.ui.rlva_locked;
    let button = ui
        .add_enabled_ui(keep, |ui| widgets::flat_button(ui, p, tr!("photo-light-keep")))
        .inner
        .on_hover_text(tr!("photo-light-keep-tip"))
        .on_disabled_hover_text(tr!("photo-light-keep-tip"));
    out.keep_light |= button.clicked();
    if state.boost {
        note(ui, p, tr!("phototools-photo-boost"));
    }
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_names_its_tabs() {
        assert_eq!(Tab::parse("env"), Some((0, false)));
        assert_eq!(Tab::parse("light"), Some((1, false)));
        assert_eq!(Tab::parse("dof"), Some((2, false)));
        assert_eq!(Tab::parse("General"), Some((3, false)));
        // the photo window opened too
        assert_eq!(Tab::parse("light:photo"), Some((1, true)));
        assert_eq!(Tab::parse("dof:photo"), Some((2, true)));
        // tabs not built yet
        assert_eq!(Tab::parse("camera"), None);
        assert_eq!(Tab::parse("light:other"), None);
    }
}

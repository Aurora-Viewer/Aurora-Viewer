//! "Phototools" (the toolbar's `phototools` command): the settings that
//! matter to a photographer, in one window.
//!
//! Port of FloaterPhototools (newview/quickprefs.cpp,
//! floater_phototools.xml, originally LGPL 2.1; getIsPhototools). The tabs
//! come in the Legacy Viewer's order (Environment, Light, Depth of field &
//! glow, General, Aids, Camera) and each of its group boxes is a collapsing
//! section, in its order. Only the tabs built so far show: Environment and
//! General. The others come with their own changes and appear then.
//!
//! The rows are Preferences' own (`options/phototools.rs` borrows them from
//! the pages: same setting, range, clamp, help, "modified" dot and RLVa
//! greying), so a setting is changed the same way here and there and
//! applied and saved like a change of Preferences. The environment block
//! is the Environment window's and Quick preferences' own
//! (`EnvironmentUi::selector_body`).

use super::environment::{EnvAction, EnvironmentUi};
use super::options::{self, OptionsUi, PhotoSection};
use super::widgets::{self, ActionKind, Floater};
use crate::settings::Settings;
use crate::theme::Palette;
use crate::world::eep_env::LocalView;
use crate::world::env_select::SettingsKind;
use egui::Vec2;

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
    /// P_General.
    General,
}

const TABS: [Tab; 2] = [Tab::Environment, Tab::General];

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Environment => aurora_i18n::tr!("common-environment"),
            Tab::General => aurora_i18n::tr!("common-general"),
        }
    }

    /// `AURORA_DEMO_PHOTOTOOLS=<tab>`.
    pub fn parse(text: &str) -> Option<usize> {
        let tab = match text.trim().to_ascii_lowercase().as_str() {
            "env" | "environment" => Tab::Environment,
            "general" | "gen" => Tab::General,
            _ => return None,
        };
        TABS.iter().position(|t| *t == tab)
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
    /// "Pause clouds": a state of the session, shared with the photo
    /// window (the Legacy Viewer's pauseCloudScroll lasts the session).
    pub clouds: &'a mut bool,
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
    ) -> Out {
        let mut out = Out::default();
        // the pending save first: a drag that ended with the window closed
        // is still saved
        self.save_when_released(ctx, settings);
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
        self.save_when_released(ctx, settings);
        out
    }

    /// Saved once the button is released, not at every frame of a drag.
    fn save_when_released(&mut self, ctx: &egui::Context, settings: &Settings) {
        if self.unsaved && !ctx.input(|i| i.pointer.any_down()) {
            self.unsaved = false;
            settings.save();
        }
    }

    fn body(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &mut Settings, env: &mut Env<'_>, out: &mut Out) {
        self.tab = self.tab.min(TABS.len() - 1);
        let labels = TABS.map(|t| (t.label(), true));
        widgets::tabs(ui, p, &mut self.tab, &labels);
        ui.add_space(6.0);
        let tab = TABS[self.tab];
        let rows_width = (widgets::scrolled_width(ui) - 6.0).max(0.0);
        egui::ScrollArea::vertical()
            .id_salt(("phototools-tab", self.tab))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(rows_width);
                match tab {
                    Tab::Environment => self.environment(ui, p, settings, env, out),
                    Tab::General => self.general(ui, p, settings, out),
                }
                ui.add_space(12.0);
            });
    }

    /// One Legacy group box.
    fn rows(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &mut Settings, out: &mut Out, title: &str, section: PhotoSection) {
        // the id is the section's, not its translated title
        widgets::collapsing(ui, p, ("phototools", section as u8), title, true, |ui| {
            let r = options::phototools_section(ui, p, settings, &mut self.rows, section);
            out.changed |= r.changed;
            out.toggle_flycam |= r.toggle_flycam;
        });
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
                env.ui
                    .selector_body(ui, p, env.view, env.loading, None, LABEL_W, env.lighting, None, &mut out.env);
                ui.add_space(4.0);
                pause_clouds(ui, p, env);
            },
        );
        let free = !env.ui.rlva_locked;
        widgets::collapsing(ui, p, ("phototools", "quick"), tr!("phototools-quick-environments"), true, |ui| {
            ui.add_enabled_ui(free, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let times = [
                        tr!("ui-environment-sunrise"),
                        tr!("common-midday"),
                        tr!("ui-environment-lie-down"),
                        tr!("common-midnight"),
                    ];
                    for (i, name) in times.iter().enumerate() {
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
                    out.env.push(EnvAction::Shared);
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
        self.rows(ui, p, settings, out, tr!("ui-options-reflections"), PhotoSection::Reflections);
    }

    /// P_General: General Render Settings, then the two boxes under it.
    fn general(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &mut Settings, out: &mut Out) {
        use aurora_i18n::tr;
        self.rows(ui, p, settings, out, tr!("phototools-general-render"), PhotoSection::GeneralRender);
        self.rows(ui, p, settings, out, tr!("phototools-render-options"), PhotoSection::RenderOptions);
        self.rows(ui, p, settings, out, tr!("ui-options-quality"), PhotoSection::ImageQuality);
    }
}

/// "Pause Clouds": a switch of the session, never saved, shared with the
/// photo window's (`snapshots.ui.light.clouds_paused`).
fn pause_clouds(ui: &mut egui::Ui, p: &Palette, env: &mut Env<'_>) {
    use aurora_i18n::tr;
    // greyed like the presets under @setenv / @setsphere
    // (FloaterQuickPrefs::onClickPauseClouds, quickprefs.cpp)
    ui.add_enabled_ui(!env.ui.rlva_locked, |ui| {
        widgets::switch_labeled(ui, p, env.clouds, tr!("photo-light-pause-clouds")).on_hover_text(tr!("photo-light-pause-clouds-hint"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demo_names_its_tabs() {
        assert_eq!(Tab::parse("env"), Some(0));
        assert_eq!(Tab::parse("General"), Some(1));
        // tabs not built yet
        assert_eq!(Tab::parse("light"), None);
        assert_eq!(Tab::parse("camera"), None);
    }
}

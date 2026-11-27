//! The constellations and the planets over the real night sky
//! (world/constellations.rs, world/planets.rs, ui/constellations_window.rs),
//! on the app's side: their part of the frame, their windows, the menu and
//! toolbar entries and the demo's set-up. Aurora's own: the Legacy Viewer
//! draws no constellations and no planets. Local rendering only.
use super::*;
use crate::world::planets::Wanted;

/// Their state on the main thread (`App::constellations`).
#[derive(Default)]
pub(super) struct NightSkyExtras {
    /// The figures, their lines and names (world/constellations.rs).
    pub(super) overlay: crate::world::constellations::Constellations,
    /// The planets and the real Sun and Moon (world/planets.rs).
    planets: crate::world::planets::Cache,
    /// The overlay's settings as last saved (saved again when changed).
    saved: crate::world::constellations::ConstellationSettings,
    /// The demo's AURORA_DEMO_CONSTELLATIONS_WINDOW and
    /// AURORA_DEMO_CONSTELLATION_CREDITS were looked at.
    window_demo: bool,
}

impl App {
    /// This frame's planets and real Sun and Moon, placed into `stars` (just
    /// made by night_sky.rs `star_frame` for `moment`, by whose rotation they
    /// stand), then the constellations; display only. `sun`: the region's;
    /// `dt`: the frame's time (0 while a photo holds its instant).
    pub(super) fn constellations_frame(
        &mut self,
        stars: &mut aurora_render::stars::StarFrame,
        sun: Vec3,
        moment: f64,
        dt: f32,
        window_height: u32,
        device: &aurora_render::wgpu::Device,
    ) {
        let c = &mut self.constellations;
        c.planets.fill(stars, Wanted::of(&self.settings), moment, self.demo);
        c.overlay.update(
            &crate::world::constellations::Input {
                settings: &self.settings.constellations,
                real_sky: stars.catalogue == Some(aurora_render::stars::REAL),
                stars,
                sun,
                look: self.camera.forward(),
                fov_y: self.camera.fov_y,
                window_height,
                dt,
                planets: c.planets.labels(),
            },
            self.demo,
            &self.egui_ctx,
            device,
        );
    }

    /// World › Environment › Constellations, Preferences › Just for fun
    /// (`toggle` false: open, a minimized window expands) and the toolbar's
    /// button (`toggle`: open or close); under @setenv nothing opens, as
    /// the Aurora Weather window.
    pub(super) fn open_constellations(&mut self, toggle: bool) {
        if self.rlva_environment_windows_locked() {
            self.panels.constellations = false;
        } else if toggle {
            ui::widgets::toggle_floater(&self.egui_ctx, ui::constellations_window::WINDOW, &mut self.panels.constellations);
        } else {
            ui::widgets::open_floater(&self.egui_ctx, ui::constellations_window::WINDOW, &mut self.panels.constellations);
        }
    }

    /// The Constellations window and its credits window, beside the
    /// weather's: their settings act at once, and are saved once a slider
    /// is let go (the planets' and the Sun and Moon's switches at once).
    pub(super) fn constellations_windows(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if self.demo && !self.constellations.window_demo {
            if ui::constellations_window::demo_credits_scroll().is_some() {
                self.constellations.window_demo = true;
                self.panels.constellation_credits = true;
            } else if ui::constellations_window::demo_window("1") {
                self.constellations.window_demo = true;
                self.panels.constellations = true;
                // the window shows the scenario's settings (in memory,
                // never saved: the copy taken as saved is the same)
                self.settings.constellations = crate::world::constellations::demo_settings(&self.settings.constellations);
                self.constellations.saved = self.settings.constellations.clone();
                // and the scenario's planets and Sun and Moon switches
                crate::world::planets::demo_bodies(Wanted::of(&self.settings)).store(&mut self.settings);
            }
        }
        let status = self.constellations.overlay.status().clone();
        let mut bodies = Wanted::of(&self.settings);
        let bodies_before = bodies;
        ui::constellations_window::show(
            ctx,
            p,
            &mut self.panels.constellations,
            &mut self.panels.constellation_credits,
            &mut self.settings.constellations,
            &mut bodies,
            &status,
        );
        if bodies != bodies_before {
            bodies.store(&mut self.settings);
            self.settings.save();
        }
        ui::constellations_window::show_credits(ctx, p, &mut self.panels.constellation_credits);
        if self.settings.constellations != self.constellations.saved && !ctx.input(|i| i.pointer.any_down()) {
            self.constellations.saved = self.settings.constellations.clone();
            self.settings.save();
        }
    }

    /// AURORA_DEMO_CONSTELLATIONS=zenith=<figure>: the real sky's place and
    /// real time that put that figure overhead.
    pub(super) fn constellations_demo(&mut self) {
        let moment = crate::demo::night_sky::instant().unwrap_or_else(crate::world::night_sky::unix_days_now);
        if let Some((latitude, longitude)) = crate::world::constellations::demo_zenith(moment) {
            self.settings.night_sky_place = crate::world::night_sky::SkyPlace::Custom;
            self.settings.night_sky_latitude = latitude;
            self.settings.night_sky_longitude = longitude;
            self.settings.night_sky_hour = crate::world::night_sky::StarHour::RealTime;
            self.settings.sync_night_sky();
        }
    }
}

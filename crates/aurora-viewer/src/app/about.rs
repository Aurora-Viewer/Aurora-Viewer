//! The app's side of Help › About Aurora Viewer (`ui/about.rs`): the
//! information LLFloaterAbout::getInfo gathers (indra/newview/
//! llfloaterabout.cpp, originally LGPL 2.1) — the viewer, the computer,
//! the graphics, where we are and the link — read from the app while the
//! window is open. The computer is read in the background
//! (`system_info.rs`) when the window opens, then again every few seconds
//! for the viewer's memory. The region and the position follow RLVa's
//! @showloc, as the Legacy Viewer's AboutPositionRLVShowLoc.

use super::App;
use crate::system_info::{Machine, Reading};
use crate::ui::about::{AboutUi, AboutView, Row, Section};
use aurora_render::Renderer;
use std::time::{Duration, Instant};

/// The viewer's memory is read again this often while the window is open.
const MACHINE_EVERY: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct About {
    pub ui: AboutUi,
    machine: Option<Machine>,
    reading: Option<(Reading, Instant)>,
}

impl App {
    /// The window, drawn with the interface.
    pub(super) fn about_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette, renderer: &Renderer) {
        if !self.panels.about {
            // read afresh at the next opening
            self.about.machine = None;
            self.about.reading = None;
            return;
        }
        if let Some((reading, _)) = &self.about.reading
            && let Some(machine) = reading.get()
        {
            self.about.machine = Some(machine.clone());
        }
        let due = self
            .about
            .reading
            .as_ref()
            .is_none_or(|(reading, at)| reading.get().is_some() && at.elapsed() >= MACHINE_EVERY);
        if due {
            self.about.reading = Some((Reading::start(), Instant::now()));
        }
        if self.about.machine.is_none() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let view = AboutView {
            sections: self.about_sections(renderer),
            logo: self.logo.as_ref(),
        };
        self.about.ui.show(ctx, p, &view, &mut self.panels.about);
    }

    fn about_sections(&self, renderer: &Renderer) -> Vec<Section> {
        let s = &self.settings;
        let gpu = &renderer.info;
        let mut sections = Vec::new();
        let (w, h) = renderer.size();
        sections.push(Section {
            icon: "app-window-filled",
            title: "about-section-viewer",
            rows: vec![
                Row::text("about-commit", aurora_version::commit()),
                Row::text(
                    "about-build",
                    format!("{} ({})", crate::ui::about::build_label(), env!("AURORA_BUILD_PROFILE")),
                ),
                Row::text("about-language", aurora_i18n::language()),
                Row::text("about-window-size", format!("{w} × {h} px")),
                Row::text("about-ui-scale", format!("{:.0} %", s.ui_scale * 100.0)),
                if s.rlva_enabled {
                    Row::text("about-rlva", crate::world::rlva::VERSION)
                } else {
                    Row::key("about-rlva", "about-off")
                },
            ],
        });
        sections.push(Section {
            icon: "device-desktop-filled",
            title: "about-section-computer",
            rows: match &self.about.machine {
                Some(m) => vec![
                    Row::text("about-os", m.os.clone()),
                    Row::text("about-cpu", m.cpu.clone()),
                    Row::text("about-threads", m.threads.to_string()),
                    Row::text("about-memory", m.memory_text()),
                ],
                None => vec![Row::key("about-os", "about-reading")],
            },
        });
        let aa = ["none", "FXAA", "MSAA 2x", "MSAA 4x", "MSAA 8x", "TAA", "SMAA"]
            .get(s.antialiasing as usize)
            .copied()
            .unwrap_or("?");
        sections.push(Section {
            icon: "photo-filled",
            title: "about-section-graphics",
            rows: vec![
                Row::text(
                    "about-gpu",
                    format!("{} ({})", gpu.name, if gpu.integrated { "integrated" } else { "discrete" }),
                ),
                Row::text("about-graphics-api", gpu.backend.clone()),
                Row::text("about-driver", gpu.driver.clone()),
                match gpu.vram_mb {
                    Some(mb) => Row::text("about-vram", format!("{mb} MB")),
                    None => Row::key("about-vram", "about-unknown"),
                },
                Row::text(
                    "about-texture-budget",
                    format!("{} MB", s.texture_budget(gpu.vram_mb, gpu.integrated)),
                ),
                Row::text(
                    "about-display",
                    format!(
                        "{:.0} Hz, {}",
                        self.screen_rate.hz(),
                        super::present_mode_name(renderer.present_mode())
                    ),
                ),
                Row::text(
                    "about-render-settings",
                    format!(
                        "draw distance {:.0} m, LOD {}, shadows {}, {aa}",
                        s.draw_distance,
                        s.lod_factor,
                        if s.shadows { "on" } else { "off" }
                    ),
                ),
            ],
        });
        if let Some(region) = self.world.main() {
            let hidden = self.world.rlva.has("showloc");
            let place = |text: String| {
                if hidden {
                    crate::ui::about::Value::Key("about-hidden-rlva")
                } else {
                    crate::ui::about::Value::Text(text)
                }
            };
            let at = self.world.agent.position;
            let grid = crate::world::chat_log::grid_name(&s.grid, &s.custom_login_uri, self.demo);
            sections.push(Section {
                icon: "map-pin-filled",
                title: "about-section-location",
                rows: vec![
                    Row::text("about-grid", grid),
                    Row {
                        label: "about-region",
                        value: place(region.name.clone()),
                    },
                    Row {
                        label: "about-position",
                        value: place(format!("{:.1}, {:.1}, {:.1}", at.x, at.y, at.z)),
                    },
                    match self.world.estate.versions.get(&region.handle) {
                        Some(version) => Row::text("about-simulator", version.clone()),
                        None => Row::key("about-simulator", "about-unknown"),
                    },
                ],
            });
            let ping = self.net.stats.ping_ms.load(std::sync::atomic::Ordering::Relaxed);
            let loss = crate::net_profile::loss_percent(&self.net_profile.last);
            sections.push(Section {
                icon: "antenna-bars-5",
                title: "about-section-network",
                rows: vec![
                    Row::text("about-ping", format!("{ping} ms")),
                    Row::text("about-packet-loss", format!("{loss:.1} %")),
                    Row::text("about-bandwidth", format!("{} kbit/s", s.bandwidth_kbps)),
                ],
            });
        }
        sections
    }
}

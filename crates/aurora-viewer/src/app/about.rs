//! The app's side of Help › About Aurora Viewer (`ui/about.rs`): the
//! information LLFloaterAbout::getInfo gathers (indra/newview/
//! llfloaterabout.cpp, originally LGPL 2.1) — the viewer, the computer,
//! the graphics, where we are and the link — read from the app while the
//! window is open. The computer is read in the background
//! (`system_info.rs`) when the window opens, then again every few seconds
//! for the viewer's memory. The region and the position follow RLVa's
//! @showloc, as the Legacy Viewer's AboutPositionRLVShowLoc
//! (`place_rows`).
//!
//! The same text, in English, goes to a resident in a conversation: the
//! conversation menu's "Send system information", asked first
//! (FSFloaterIM::onSysinfoButtonClicked, SendSysinfoToIM), and a line that
//! starts with "/sysinfo", sent at once (FSFloaterIM::sendMsg,
//! indra/newview/fsfloaterim.cpp; FSData::getSystemInfo, fsdata.cpp).

use super::App;
use crate::system_info::{Machine, Reading};
use crate::ui::about::{AboutUi, AboutView, Row, Section};
use aurora_net::NetCommand;
use aurora_render::Renderer;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The viewer's memory is read again this often while the window is open.
const MACHINE_EVERY: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct About {
    pub ui: AboutUi,
    machine: Option<Machine>,
    reading: Option<(Reading, Instant)>,
    sysinfo: Sysinfo,
}

/// The system information on its way to a conversation.
#[derive(Default)]
struct Sysinfo {
    /// The computer read for this resident; true: asked from the menu,
    /// confirmed before it is sent.
    pending: Option<(Uuid, bool, Reading)>,
    /// The text shown, waiting for Send or Cancel.
    confirm: Option<(Uuid, String)>,
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
            sections: self.about_sections(renderer, self.about.machine.as_ref()),
            logo: self.logo.as_ref(),
        };
        self.about.ui.show(ctx, p, &view, &mut self.panels.about);
    }

    /// Send system information to `to`: the computer read in the
    /// background, then the text shown for confirmation (`confirm`, the
    /// menu's) or sent at once ("/sysinfo").
    pub(super) fn ask_sysinfo(&mut self, to: Uuid, confirm: bool) {
        self.about.sysinfo = Sysinfo {
            pending: Some((to, confirm, Reading::start())),
            confirm: None,
        };
    }

    /// The text once the computer is read, and its confirmation
    /// (SendSysinfoToIM: the text, Send / Cancel).
    pub(super) fn sysinfo_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette, renderer: &Renderer) {
        use crate::ui::widgets::{self, ActionKind};
        if let Some((to, confirm, reading)) = &self.about.sysinfo.pending {
            let Some(machine) = reading.get().cloned() else {
                ctx.request_repaint_after(Duration::from_millis(100));
                return;
            };
            let (to, confirm) = (*to, *confirm);
            self.about.sysinfo.pending = None;
            let mut sections = self.about_sections(renderer, Some(&machine));
            if self.demo {
                demo_hardware(&mut sections);
            }
            let text = crate::ui::about::copy_text(&sections);
            if confirm {
                self.about.sysinfo.confirm = Some((to, text));
            } else {
                self.send_sysinfo(to, text);
            }
        }
        let Some((_, text)) = &self.about.sysinfo.confirm else {
            return;
        };
        let mut answer = None;
        let modal = widgets::Modal::new(egui::Id::unique("send_sysinfo")).show(ctx, |ui| {
            ui.set_width(440.0);
            let title = aurora_i18n::tr!("ui-chat-send-system-info");
            ui.label(egui::RichText::new(title).size(15.0).strong().color(p.ink));
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("ui-chat-send-system-info-ask"))
                    .size(12.5)
                    .color(p.muted),
            );
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(text).size(12.0).color(p.ink)).wrap());
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let size = egui::Vec2::new(0.0, 26.0);
                if widgets::action_button(ui, p, "send-filled", aurora_i18n::tr!("common-send"), ActionKind::Primary, size).clicked() {
                    answer = Some(true);
                }
                if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    answer = Some(false);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        if answer.is_some()
            && let Some((to, text)) = self.about.sysinfo.confirm.take()
            && answer == Some(true)
        {
            self.send_sysinfo(to, text);
        }
    }

    /// AURORA_DEMO_SYSINFO=1: "Send system information" chosen in the
    /// friend's conversation at frame 600; its confirmation shows once the
    /// computer is read.
    pub(super) fn demo_sysinfo(&mut self) {
        if self.demo && self.frame_count == 600 && std::env::var_os("AURORA_DEMO_SYSINFO").is_some() {
            self.ask_sysinfo(crate::demo::DEMO_FRIEND, true);
        }
    }

    /// One IM, the network splitting it where it is too long, as the Legacy
    /// Viewer sends its two parts.
    fn send_sysinfo(&mut self, to: Uuid, text: String) {
        self.world.own_im(to, &text);
        self.send(NetCommand::SendIm { to, message: text });
    }

    fn about_sections(&self, renderer: &Renderer, machine: Option<&Machine>) -> Vec<Section> {
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
                // (RlvHandler::isEnabled: what runs, not the preference)
                if self.rlva_switch.running() {
                    Row::text("about-rlva", crate::world::rlva::VERSION)
                } else {
                    Row::key("about-rlva", "about-off")
                },
            ],
        });
        sections.push(Section {
            icon: "device-desktop-filled",
            title: "about-section-computer",
            rows: match machine {
                Some(m) => vec![
                    Row::text("about-os", m.os.clone()),
                    Row::text("about-cpu", m.cpu.clone()),
                    Row::text("about-threads", m.threads.to_string()),
                    Row::text("about-memory", m.memory_text()),
                ],
                None => vec![Row::key("about-os", "about-reading")],
            },
        });
        let aa = aa_name(s.antialiasing, renderer.dlss_drawing());
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
                // a discrete card as before ("N MB"); an integrated GPU's two
                // heaps in the interface's language
                match (gpu.dedicated_mb, gpu.shared_mb) {
                    (Some(mb), None) => Row::text("about-vram", format!("{mb} MB")),
                    (dedicated, Some(shared)) => Row::text(
                        "about-vram",
                        aurora_i18n::tr!(
                            "about-vram-shared",
                            dedicated = aurora_i18n::number_text(&dedicated.unwrap_or(0).to_string()),
                            shared = aurora_i18n::number_text(&shared.to_string())
                        )
                        .to_string(),
                    ),
                    (None, None) => Row::key("about-vram", "about-unknown"),
                },
                Row::text(
                    "about-texture-budget",
                    format!("{} MB", s.texture_budget(gpu.dedicated_mb, gpu.shared_mb, gpu.integrated)),
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
            let at = self.world.agent.position;
            // On OpenSim, the grid we are on now: after a Hypergrid teleport
            // the visited one, by its saved name or its address.
            let grid = crate::grids::current_label(s, self.world.opensim_extras())
                .filter(|_| !self.demo)
                .unwrap_or_else(|| crate::world::chat_log::grid_name(&s.grid, &s.custom_login_uri, self.demo));
            sections.push(Section {
                icon: "map-pin-filled",
                title: "about-section-location",
                rows: std::iter::once(Row::text("about-grid", grid))
                    .chain(place_rows(hidden, &region.name, at))
                    .chain(std::iter::once(match self.world.estate.versions.get(&region.handle) {
                        Some(version) => Row::text("about-simulator", version.clone()),
                        None => Row::key("about-simulator", "about-unknown"),
                    }))
                    .collect(),
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

/// The demo sends a made-up computer and graphics card: a demo capture of
/// the text never shows the PC it runs on.
/// The region and the position of the Location card. Under RLVa
/// `@showloc` the region is "(Hidden region)" and the position is left
/// out, the server version kept (LLAppViewer::getViewerInfo with
/// AboutPositionRLVShowLoc, indra/newview/llappviewer.cpp: "You are in
/// [REGION]").
/// The anti-aliasing a frame is drawn with, for Help › About and the
/// Dev Toolbox: the setting's, except FSR 3.1's place taken by DLSS, which
/// the setting cannot tell from the FSR that stands in for a DLSS that
/// failed or is still being made (`Renderer::dlss_drawing`).
pub(super) fn aa_name(antialiasing: u8, dlss: Option<aurora_render::DlssDrawing>) -> &'static str {
    match (antialiasing, dlss) {
        (7, Some(aurora_render::DlssDrawing::Dlss)) => "NVIDIA DLSS",
        (7, Some(aurora_render::DlssDrawing::Dlaa)) => "NVIDIA DLAA",
        (7, Some(aurora_render::DlssDrawing::Rr)) => "NVIDIA DLSS Ray Reconstruction",
        _ => ["none", "FXAA", "MSAA 2x", "MSAA 4x", "MSAA 8x", "TAA", "SMAA", "FSR 3.1"]
            .get(antialiasing as usize)
            .copied()
            .unwrap_or("?"),
    }
}

pub(super) fn place_rows(hidden: bool, region: &str, at: glam::Vec3) -> Vec<Row> {
    if hidden {
        return vec![Row::key("about-region", "rlva-hidden-region")];
    }
    vec![
        Row::text("about-region", region),
        Row::text("about-position", format!("{:.1}, {:.1}, {:.1}", at.x, at.y, at.z)),
    ]
}

fn demo_hardware(sections: &mut [Section]) {
    for row in sections.iter_mut().flat_map(|section| section.rows.iter_mut()) {
        let text = match row.label {
            "about-os" => "Windows 11 Pro 26100 (x86_64)",
            "about-cpu" => "Demo CPU, 8 cores",
            "about-threads" => "16",
            "about-memory" => "32768 MB (viewer: 2048 MB)",
            "about-gpu" => "Demo GPU (discrete)",
            "about-driver" => "1.0.0",
            "about-vram" => "8192 MB",
            "about-texture-budget" => "4096 MB",
            "about-display" => "60 Hz, vsync",
            _ => continue,
        };
        row.value = crate::ui::about::Value::Text(text.into());
    }
}

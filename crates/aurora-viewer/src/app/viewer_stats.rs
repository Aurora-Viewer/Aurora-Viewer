//! The viewer's part of Linden Lab's viewer statistics (aurora_net's
//! viewer_stats.rs): the frame times, the foreground time, the distance
//! travelled and the machine, handed to the network side about once a
//! second. The session posts the report itself, at arrival, every five
//! minutes and at logout.
use super::App;
use aurora_net::viewer_stats::{FrameJitter, GpuInfo, SystemInfo, ViewerSide};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// How often the network side gets fresh values.
const REFRESH: Duration = Duration::from_secs(1);

pub(super) struct Tracker {
    started: Instant,
    /// The first frame drawn (gRenderStartTime).
    render_start: Option<Instant>,
    frames: u64,
    foreground_time: f64,
    foreground_frames: u64,
    jitter: FrameJitter,
    meters_traveled: f64,
    last_position: Option<glam::DVec3>,
    pushed_at: Option<Instant>,
    /// The machine, read once off the frame (sysinfo takes a moment).
    machine: Arc<OnceLock<Machine>>,
    machine_asked: bool,
    /// `AURORA_DEMO_VIEWER_STATS`: the report was written to the log.
    demo_logged: bool,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            render_start: None,
            frames: 0,
            foreground_time: 0.0,
            foreground_frames: 0,
            jitter: FrameJitter::default(),
            meters_traveled: 0.0,
            last_position: None,
            pushed_at: None,
            machine: Arc::new(OnceLock::new()),
            machine_asked: false,
            demo_logged: false,
        }
    }
}

/// What sysinfo and the CPU report.
#[derive(Debug, Clone)]
struct Machine {
    ram_kb: i32,
    os: String,
    cpu: String,
    cpu_sse: Vec<String>,
    hardware_concurrency: i32,
}

fn read_machine() -> Machine {
    use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
    let system = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::nothing().with_frequency())
            .with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    // LLCPUInfo: the brand name, then its speed when it looks sane
    let cpu = system.cpus().first().map_or_else(
        || "Unknown".to_string(),
        |c| {
            let mhz = c.frequency();
            if (201..10_000).contains(&mhz) {
                format!("{} ({mhz} MHz)", c.brand().trim())
            } else {
                c.brand().trim().to_string()
            }
        },
    );
    Machine {
        ram_kb: i32::try_from(system.total_memory() / 1024).unwrap_or(i32::MAX),
        os: System::long_os_version().unwrap_or_else(|| std::env::consts::OS.into()),
        cpu,
        cpu_sse: sse_versions(),
        hardware_concurrency: std::thread::available_parallelism().map_or(0, |n| n.get() as i32),
    }
}

/// LLCPUInfo's mSSEVersions.
fn sse_versions() -> Vec<String> {
    #[cfg(target_arch = "x86_64")]
    {
        let found = [
            (std::arch::is_x86_feature_detected!("sse"), "1"),
            (std::arch::is_x86_feature_detected!("sse2"), "2"),
            (std::arch::is_x86_feature_detected!("sse3"), "3"),
            (std::arch::is_x86_feature_detected!("ssse3"), "3S"),
            (std::arch::is_x86_feature_detected!("sse4.1"), "4.1"),
            (std::arch::is_x86_feature_detected!("sse4.2"), "4.2"),
            (std::arch::is_x86_feature_detected!("sse4a"), "4a"),
        ];
        found.iter().filter(|(on, _)| *on).map(|(_, v)| (*v).to_string()).collect()
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        Vec::new()
    }
}

/// The Legacy Viewer's shader level, from Aurora's lighting: it always
/// renders deferred (3), with ambient occlusion (4) or shadows (5).
fn shader_level(shadows: bool, ssao: u8) -> i32 {
    if shadows {
        5
    } else if ssao > 0 {
        4
    } else {
        3
    }
}

impl App {
    /// Once per frame, after the limiter: `paced` is the frame's time, `gpu`
    /// the adapter drawing (`App::gfx` is out of `self` during the frame).
    pub(super) fn note_viewer_stats(&mut self, paced: Duration, gpu: &aurora_render::GpuInfo) {
        let now = Instant::now();
        let position = self.in_world().then(|| self.world.agent_global()).flatten();
        let focused = self.focused;
        let t = &mut self.viewer_stats;
        t.render_start.get_or_insert(now);
        t.frames += 1;
        let dt = paced.as_secs_f64();
        t.jitter.frame(dt);
        if focused {
            t.foreground_time += dt;
            t.foreground_frames += 1;
        }
        // LLAgentCamera::updateCamera: every move of the agent, teleports
        // included
        if let Some(position) = position {
            if let Some(last) = t.last_position {
                t.meters_traveled += (position - last).length();
            }
            t.last_position = Some(position);
        }
        if !t.machine_asked {
            t.machine_asked = true;
            let cell = t.machine.clone();
            std::thread::spawn(move || {
                let _ = cell.set(read_machine());
            });
        }
        if t.pushed_at.is_some_and(|at| now.duration_since(at) < REFRESH) {
            return;
        }
        t.pushed_at = Some(now);
        let side = self.viewer_side(now, gpu);
        if self.demo {
            self.demo_log_viewer_stats(&side);
        }
        self.net.set_viewer_stats(side);
    }

    fn viewer_side(&self, now: Instant, g: &aurora_render::GpuInfo) -> ViewerSide {
        let t = &self.viewer_stats;
        let machine = t.machine.get();
        // the login's own `id0` (the same cached value, not waited for)
        let id0 = crate::machine_id::ready_login_ids().map(|ids| ids.id0).unwrap_or_default();
        let avatars = &self.scene.stats.avatars;
        let inventory = &self.world.inventory;
        ViewerSide {
            version: format!("{} {}", aurora_net::login::CHANNEL, aurora_version::login_version()),
            language: aurora_i18n::language().to_owned(),
            run_time: now.duration_since(t.started).as_secs_f64(),
            foreground_time: t.foreground_time,
            foreground_frames: t.foreground_frames,
            frames: t.frames,
            render_time: t.render_start.map_or(0.0, |s| now.duration_since(s).as_secs_f64()),
            jitter: t.jitter.stats,
            agents_in_view: (avatars.full_in_view + avatars.impostors_in_view) as u32,
            meters_traveled: t.meters_traveled,
            mem_use_kb: crate::sys_probe::process_memory().map_or(0.0, |m| m.working_set as f64 / 1024.0),
            system: SystemInfo {
                ram_kb: machine.map_or(0, |m| m.ram_kb),
                os: machine.map(|m| m.os.clone()).unwrap_or_default(),
                cpu: machine.map(|m| m.cpu.clone()).unwrap_or_default(),
                cpu_sse: machine.map(|m| m.cpu_sse.clone()).unwrap_or_default(),
                hardware_concurrency: machine.map_or(0, |m| m.hardware_concurrency),
                mac_address: aurora_net::viewer_stats::mac_address_string(crate::machine_id::adapter_mac()),
                serial_number: id0,
                gpu: GpuInfo {
                    name: g.name.clone(),
                    vendor_id: g.vendor_id,
                    driver: g.driver.clone(),
                    api: g.backend.clone(),
                    vram_mb: g.vram_mb.map_or(0, |mb| i32::try_from(mb).unwrap_or(i32::MAX)),
                    max_texture_size: i32::try_from(g.max_texture_size).unwrap_or(i32::MAX),
                },
            },
            shader_level: shader_level(self.settings.shadows, self.settings.ssao),
            display_names: self.settings.use_display_names,
            show_usernames: self.settings.show_usernames,
            inventory_usable: !inventory.root.is_nil() && inventory.folders.contains_key(&inventory.root),
        }
    }

    /// `AURORA_DEMO_VIEWER_STATS=1`: the report the session would post,
    /// written once to the log (with made-up network counters, and without
    /// the session id), once the machine has been read and the world shown.
    fn demo_log_viewer_stats(&mut self, side: &ViewerSide) {
        if self.viewer_stats.demo_logged
            || !self.in_world()
            || self.viewer_stats.machine.get().is_none()
            || side.render_time < 5.0
            || std::env::var("AURORA_DEMO_VIEWER_STATS").is_err()
        {
            return;
        }
        self.viewer_stats.demo_logged = true;
        let counters = aurora_net::viewer_stats::NetCounters::read(&self.net.stats);
        let unix_now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let fps = side.frames as f64 / side.render_time;
        let body = aurora_net::viewer_stats::body(side, &counters, fps, unix_now, 1);
        log::info!(
            "demo viewer statistics (not sent): {}",
            aurora_llsd::to_xml_string(&aurora_net::viewer_stats::loggable(&body))
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_level_follows_the_lighting() {
        assert_eq!(shader_level(true, 0), 5);
        assert_eq!(shader_level(true, 2), 5);
        assert_eq!(shader_level(false, 1), 4);
        assert_eq!(shader_level(false, 0), 3);
    }

    #[test]
    fn the_cpu_flags_use_the_legacy_viewer_names() {
        let flags = sse_versions();
        for flag in &flags {
            assert!(["1", "2", "3", "3S", "4.1", "4.2", "4a"].contains(&flag.as_str()), "{flag}");
        }
        // every x86-64 processor has SSE and SSE2
        if cfg!(target_arch = "x86_64") {
            assert!(flags.starts_with(&["1".to_string(), "2".to_string()]), "{flags:?}");
        }
    }
}

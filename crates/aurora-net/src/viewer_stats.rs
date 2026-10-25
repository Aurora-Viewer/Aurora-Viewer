//! Linden Lab's viewer statistics: the LLSD posted to the agent region's
//! `ViewerStats` capability once the agent has arrived, every five minutes
//! after that and at logout. The Third Party Viewer Policy asks viewers not
//! to leave this report out; it goes to the simulator and nowhere else.
//!
//! Port of send_viewer_stats, LLViewerStats::addToMessage and
//! LLViewerStats::updateFrameStats (indra/newview/llviewerstats.cpp), with
//! the timer of LLAppViewer::idle and the last report of
//! LLAppViewer::requestQuit (indra/newview/llappviewer.cpp), originally
//! LGPL 2.1. The Legacy Viewer's own changes are kept: no preferences in the
//! last report and no `ui` block.
//!
//! The fields the Legacy Viewer reads from OpenGL carry what Aurora really
//! has: the graphics API it draws with (Vulkan), the adapter's name, vendor
//! and driver, and 0 where it has no equivalent.

use crate::stats::NetStats;
use aurora_llsd::{Llsd, Map, llsd_map};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// SEND_STATS_PERIOD (llviewerstats.h).
pub const SEND_PERIOD: Duration = Duration::from_secs(300);

/// A report from before half the period is marked with a run time of 0,
/// so that Linden Lab drops its frame rates ("not useful yet").
const VALID_AFTER_SECS: f64 = SEND_PERIOD.as_secs_f64() / 2.0;

/// StatsFrametimeSampleSeconds: the window of the period jitter and of the
/// frame time variance.
const JITTER_WINDOW_SECS: f64 = 5.0;

/// What only the viewer knows, refreshed by it about once a second
/// (`NetClient::set_viewer_stats`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ViewerSide {
    /// LLVersionInfo::getChannelAndVersion: "Aurora Viewer 0.7.5.0".
    pub version: String,
    /// The interface language (LLUI::getLanguage).
    pub language: String,
    /// Seconds since the viewer started (LLFrameTimer::getElapsedSeconds).
    pub run_time: f64,
    /// Seconds and frames the window spent in the foreground.
    pub foreground_time: f64,
    pub foreground_frames: u64,
    /// Frames drawn since the start (gFrameCount), and seconds since the
    /// first one (gRenderStartTime): the report's `sim_fps`.
    pub frames: u64,
    pub render_time: f64,
    pub jitter: Jitter,
    /// Avatars in the view, drawn in full or as impostors
    /// (LLVOAvatar::sNumVisibleAvatars).
    pub agents_in_view: u32,
    /// Distance our avatar moved since the start, teleports included
    /// (LLAgent::getDistanceTraveled).
    pub meters_traveled: f64,
    /// Resident memory of the process, KB (LLMemory::getCurrentRSS / 1024).
    pub mem_use_kb: f64,
    pub system: SystemInfo,
    /// 3 deferred, 4 with ambient occlusion, 5 with shadows (the Legacy
    /// Viewer's own scale; Aurora always renders deferred).
    pub shader_level: i32,
    /// UseDisplayNames and NameTagShowUsernames.
    pub display_names: bool,
    pub show_usernames: bool,
    /// LLInventoryModel::isInventoryUsable: the root folder is known.
    pub inventory_usable: bool,
}

/// The machine, read once by the viewer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SystemInfo {
    /// Physical memory, KB.
    pub ram_kb: i32,
    pub os: String,
    /// LLCPUInfo::getCPUString: the brand name, then " (<MHz> MHz)".
    pub cpu: String,
    /// LLCPUInfo::getSSEVersions: "1", "2", "3", "3S", "4.1", "4.2", "4a".
    pub cpu_sse: Vec<String>,
    pub hardware_concurrency: i32,
    /// The network adapter's address, `xx-xx-xx-xx-xx-xx`
    /// ([`mac_address_string`]), and the hashed serial the login sends as
    /// `id0` (LLAppViewer::getSerialNumber). Neither is ever logged
    /// ([`loggable`]).
    pub mac_address: String,
    pub serial_number: String,
    pub gpu: GpuInfo,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GpuInfo {
    /// The adapter's name ("NVIDIA GeForce RTX 4090").
    pub name: String,
    /// The PCI vendor id.
    pub vendor_id: u32,
    /// The driver and its version, as the adapter gives them.
    pub driver: String,
    /// The graphics API the viewer draws with ("Vulkan").
    pub api: String,
    /// Dedicated video memory, MB (0 when unknown).
    pub vram_mb: i32,
    pub max_texture_size: i32,
}

impl GpuInfo {
    /// LLGLManager::mGLVendorShort, from the vendor id instead of the
    /// OpenGL vendor string.
    pub fn vendor_short(&self) -> &'static str {
        match self.vendor_id {
            0x1002 | 0x1022 => "AMD",
            0x10de => "NVIDIA",
            0x8086 => "INTEL",
            0x106b => "APPLE",
            _ => "MISC",
        }
    }
}

/// send_viewer_stats' `mac_address`: the six bytes LLUUID::getNodeID gives,
/// as "%02x-%02x-%02x-%02x-%02x-%02x" (indra/newview/llviewerstats.cpp). With
/// no adapter the Legacy Viewer formats a buffer it never filled; the report
/// says all zeros instead.
pub fn mac_address_string(node: Option<[u8; 6]>) -> String {
    let b = node.unwrap_or([0; 6]);
    format!("{:02x}-{:02x}-{:02x}-{:02x}-{:02x}-{:02x}", b[0], b[1], b[2], b[3], b[4], b[5])
}

/// The report as it may be written to a log: the adapter address and the
/// serial replaced ("The session ID token must never appear in logs", and
/// neither do the machine's identifiers).
pub fn loggable(body: &Llsd) -> Llsd {
    let mut copy = body.clone();
    if let Llsd::Map(top) = &mut copy
        && let Some(Llsd::Map(system)) = top.get_mut("system")
    {
        for key in ["mac_address", "serial_number"] {
            if system.contains_key(key) {
                system.insert(key.into(), Llsd::from("(not logged)"));
            }
        }
    }
    copy
}

/// The frame time statistics of LLViewerStats::updateFrameStats.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Jitter {
    /// Sum of |frame time − previous frame time| over the time, for the
    /// whole session.
    pub session: f64,
    /// Over the last 5 s window: the standard deviation of that jitter over
    /// the mean frame time (the Legacy Viewer divides the jitter's deviation,
    /// not the frame times', and the report keeps that meaning), and the
    /// jitter summed over the window's time.
    pub frametime_variance: f64,
    pub period: f64,
}

/// Running state of [`Jitter`], fed one frame time at a time.
#[derive(Debug, Clone, Default)]
pub struct FrameJitter {
    last: f64,
    frames: u64,
    total_time: f64,
    total_jitter: f64,
    window: Vec<f64>,
    window_jitter: Vec<f64>,
    window_time: f64,
    pub stats: Jitter,
}

impl FrameJitter {
    pub fn frame(&mut self, dt: f64) {
        if self.frames > 0 && self.last > 0.0 {
            self.total_time += dt;
            let jitter = (self.last - dt).abs();
            self.total_jitter += jitter;
            self.stats.session = self.total_jitter / self.total_time;
            self.window.push(dt);
            self.window_jitter.push(jitter);
            self.window_time += dt;
            if self.window_time >= JITTER_WINDOW_SECS {
                let mean = self.window.iter().sum::<f64>() / self.window.len() as f64;
                self.stats.frametime_variance = stddev(&self.window_jitter) / mean;
                self.stats.period = self.window_jitter.iter().sum::<f64>() / self.window_time;
                self.window.clear();
                self.window_jitter.clear();
                self.window_time = 0.0;
            }
        }
        self.last = dt;
        self.frames += 1;
    }
}

/// calcStddev: the population deviation, 0 below two values.
fn stddev(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let square = values.iter().map(|v| v * v).sum::<f64>() / n;
    (square - mean * mean).max(0.0).sqrt()
}

/// The message system's counters of LLMessageSystem the report carries.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NetCounters {
    pub bytes_in: u64,
    pub packets_in: u64,
    pub zero_coded_in: u64,
    pub zero_coded_bytes_in: u64,
    pub zero_decoded_bytes_in: u64,
    pub bytes_out: u64,
    pub packets_out: u64,
    pub send_failures: u64,
    /// Packets that never came (mDroppedPackets): sequence gaps not filled
    /// by a late packet.
    pub dropped: u64,
    pub resent: u64,
    /// Reliable packets given up after their resends (mFailedResendPackets).
    pub failed_resends: u64,
    pub off_circuit: u64,
    /// Packets that did not decode (mInvalidOnCircuitPackets).
    pub invalid: u64,
    pub texture_bytes: u64,
    pub mesh_bytes: u64,
    /// gAvgSimPing, ms.
    pub ping_ms: f32,
}

impl NetCounters {
    pub fn read(stats: &NetStats) -> Self {
        let l = |a: &std::sync::atomic::AtomicU64| a.load(Ordering::Relaxed);
        NetCounters {
            bytes_in: l(&stats.bytes_in),
            packets_in: l(&stats.packets_in),
            zero_coded_in: l(&stats.zero_coded_in),
            zero_coded_bytes_in: l(&stats.zero_coded_bytes_in),
            zero_decoded_bytes_in: l(&stats.zero_decoded_bytes_in),
            bytes_out: l(&stats.bytes_out),
            packets_out: l(&stats.packets_out),
            send_failures: l(&stats.send_errors),
            dropped: l(&stats.seq_gaps).saturating_sub(l(&stats.seq_late)),
            resent: l(&stats.resent),
            failed_resends: l(&stats.dropped_reliable),
            off_circuit: l(&stats.off_circuit),
            invalid: l(&stats.decode_errors),
            texture_bytes: l(&stats.texture_bytes),
            mesh_bytes: l(&stats.mesh_bytes),
            ping_ms: stats.ping_avg_ms.load(Ordering::Relaxed) as f32,
        }
    }
}

/// When the reports leave, and the frame count of the previous one
/// (gSimFrames / gSimLastTime, for `sim_fps`).
#[derive(Debug, Clone, Default)]
pub struct Schedule {
    last_sent: Option<Instant>,
    sim_frames: u64,
    sim_last_time: f64,
}

impl Schedule {
    /// The first report as soon as the agent has arrived (the Legacy Viewer's
    /// timer starts elapsed and runs once its startup is over), then one
    /// every [`SEND_PERIOD`].
    pub fn due(&self, now: Instant, arrived: bool) -> bool {
        arrived && self.last_sent.is_none_or(|t| now.duration_since(t) >= SEND_PERIOD)
    }

    /// Note a report (whether it could be posted or not: the Legacy Viewer
    /// resets its timer either way) and return its `sim_fps`: the frames
    /// drawn per second since the previous one.
    pub fn sent(&mut self, now: Instant, frames: u64, render_time: f64) -> f64 {
        self.last_sent = Some(now);
        let elapsed = render_time - self.sim_last_time;
        let fps = if elapsed > 0.0 {
            frames.saturating_sub(self.sim_frames) as f64 / elapsed
        } else {
            0.0
        };
        self.sim_frames = frames;
        self.sim_last_time = render_time;
        fps
    }
}

fn int(v: u64) -> Llsd {
    Llsd::Integer(i32::try_from(v).unwrap_or(i32::MAX))
}

fn kb(bytes: u64) -> Llsd {
    Llsd::Real(bytes as f64 / 1024.0)
}

/// The report, without its `session_id`: the caller adds it after any
/// logging ("The session ID token must never appear in logs").
pub fn body(v: &ViewerSide, n: &NetCounters, sim_fps: f64, unix_now: i64, regions_visited: usize) -> Llsd {
    let run_time = if v.run_time < VALID_AFTER_SECS { 0.0 } else { v.run_time };
    let fps = if v.foreground_time > 0.0 {
        v.foreground_frames as f64 / v.foreground_time
    } else {
        0.0
    };
    // LLTranslate::asLLSD: Aurora has no chat translation service
    let translation = llsd_map! { "on" => false, "chars_seen" => 0 };
    let agent = llsd_map! {
        "start_time" => i32::try_from(unix_now - v.run_time as i64).unwrap_or(0),
        "run_time" => run_time,
        "foreground_time" => v.foreground_time,
        "fps" => fps,
        "normalized_session_jitter" => v.jitter.session,
        "normalized_frametime_variance" => v.jitter.frametime_variance,
        "normalized_period_jitter" => v.jitter.period,
        "version" => v.version.as_str(),
        "language" => v.language.as_str(),
        "sim_fps" => sim_fps,
        "agents_in_view" => v.agents_in_view,
        "ping" => n.ping_ms,
        "meters_traveled" => v.meters_traveled,
        "regions_visited" => int(regions_visited as u64),
        "mem_use" => v.mem_use_kb,
        "translation" => translation,
    };

    let s = &v.system;
    let g = &s.gpu;
    let vendor = g.vendor_short();
    // "%-6s Class %d " + LLGLManager::getRawGLString ("<vendor> <renderer>")
    let gpu_class = -1; // GPU_CLASS_UNKNOWN: Aurora runs no GPU benchmark
    let vendor6: String = vendor.chars().take(6).collect();
    let gpu = format!("{vendor6:<6} Class {gpu_class} {vendor} {}", g.name);
    let gl = llsd_map! {
        "gpu_vendor" => vendor,
        "gpu_version" => g.driver.as_str(),
        "opengl_version" => g.api.as_str(),
        "vram" => g.vram_mb,
        // OpenGL limits Vulkan has no counterpart for
        "max_samples" => 0,
        "num_texture_image_units" => 0,
        "max_sample_mask_words" => 0,
        "max_color_texture_samples" => 0,
        "max_depth_texture_samples" => 0,
        "max_integer_samples" => 0,
        "max_vertex_range" => 0,
        "max_index_range" => 0,
        "max_texture_size" => g.max_texture_size,
        "is_ati" => vendor == "AMD",
        "is_nvidia" => vendor == "NVIDIA",
        "is_intel" => vendor == "INTEL",
        "gl_renderer" => g.name.as_str(),
    };
    let system = llsd_map! {
        "ram" => s.ram_kb,
        "os" => s.os.as_str(),
        "cpu" => s.cpu.as_str(),
        "cpu_sse" => Llsd::Array(s.cpu_sse.iter().map(|v| Llsd::from(v.as_str())).collect()),
        "address_size" => 64,
        "os_bitness" => 64,
        "hardware_concurrency" => s.hardware_concurrency,
        "mac_address" => s.mac_address.as_str(),
        "serial_number" => s.serial_number.as_str(),
        "gpu" => gpu,
        "gpu_class" => gpu_class,
        "gpu_memory_bandwidth" => 0.0,
        "gpu_vendor" => vendor,
        "gpu_version" => g.driver.as_str(),
        "opengl_version" => g.api.as_str(),
        "gl" => gl,
        "shader_level" => v.shader_level,
    };

    let downloads = llsd_map! {
        // gTotalWorldData and gTotalObjectData are never added to in the
        // Legacy Viewer: its reports carry 0
        "world_kbytes" => 0.0,
        "object_kbytes" => 0.0,
        "texture_kbytes" => kb(n.texture_bytes),
        "mesh_kbytes" => kb(n.mesh_bytes),
    };

    let net_in = llsd_map! {
        "kbytes" => kb(n.bytes_in),
        "packets" => int(n.packets_in),
        "compressed_packets" => int(n.zero_coded_in),
        "savings" => kb(n.zero_decoded_bytes_in.saturating_sub(n.zero_coded_bytes_in)),
    };
    // (the outgoing compression counters are commented out in the Legacy
    // Viewer's message builder: always 0)
    let net_out = llsd_map! {
        "kbytes" => kb(n.bytes_out),
        "packets" => int(n.packets_out),
        "compressed_packets" => 0,
        "savings" => 0.0,
    };
    let failures = llsd_map! {
        "send_packet" => int(n.send_failures),
        "dropped" => int(n.dropped),
        "resent" => int(n.resent),
        "failed_resends" => int(n.failed_resends),
        "off_circuit" => int(n.off_circuit),
        "invalid" => int(n.invalid),
        // Aurora updates itself (the Legacy Viewer: true without Velopack)
        "missing_updater" => false,
    };
    let mut misc = Map::new();
    if cfg!(windows) {
        // the Vulkan probe of the report (Dave Houlton 05/2020): Aurora draws
        // with Vulkan, so its loader is there; the Legacy Viewer reads "1.0"
        // when it cannot ask the loader for more
        let vulkan = g.api == "Vulkan";
        misc.insert(
            "string_1".into(),
            Llsd::from(if vulkan {
                "Vulkan driver is detected"
            } else {
                "No Vulkan driver detected"
            }),
        );
        misc.insert("VulkanMaxApiVersion".into(), Llsd::from(if vulkan { "1.0" } else { "0.0" }));
    } else {
        misc.insert("string_1".into(), Llsd::from("Unused"));
    }
    misc.insert("string_2".into(), Llsd::from("Unused"));
    misc.insert("int_1".into(), Llsd::Integer(0));
    misc.insert("int_2".into(), Llsd::Integer(0));
    let stats = llsd_map! {
        "net" => llsd_map! { "in" => net_in, "out" => net_out },
        "failures" => failures,
        "misc" => Llsd::Map(misc),
    };

    let inventory = llsd_map! {
        "usable" => v.inventory_usable,
        // (Aurora runs no inventory validation pass: nothing to report)
        "validation_info" => Llsd::new_map(),
    };

    llsd_map! {
        "agent" => agent,
        "system" => system,
        "downloads" => downloads,
        "stats" => stats,
        "inventory" => inventory,
        "DisplayNamesEnabled" => v.display_names,
        "DisplayNamesShowUsername" => v.show_usernames,
        "MinimalSkin" => false,
        // LLViewerStats::addToMessage
        "misc" => llsd_map! { "Version" => true, "Vertex Buffers Enabled" => 1.0 },
        // (the Legacy Viewer sums the server corrections of our position
        // over half a metre; Aurora does not count them)
        "AgentPositionSnaps" => 0.0,
    }
}

/// The reports still being posted. The viewer waits for them, for a bounded
/// time, before it exits: the Legacy Viewer's last report finishes while it
/// waits for the LogoutReply, while Aurora's process would end under it
/// (`NetClient::wait_viewer_stats`).
#[derive(Debug, Default)]
pub struct PostsInFlight {
    count: parking_lot::Mutex<u32>,
    done: parking_lot::Condvar,
}

impl PostsInFlight {
    pub fn begin(&self) {
        *self.count.lock() += 1;
    }

    pub fn end(&self) {
        let mut count = self.count.lock();
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.done.notify_all();
        }
    }

    /// Wait until no report is being posted, `timeout` at most: true when
    /// they all finished. It returns as soon as the last one does.
    pub fn wait(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut count = self.count.lock();
        while *count > 0 {
            if self.done.wait_until(&mut count, deadline).timed_out() {
                return *count == 0;
            }
        }
        true
    }
}

/// Post a report (`messageHttpPost`, fire and forget): any 2xx answer is a
/// success whatever its body, a failure is only logged, and nothing is sent
/// again before the next report. The log never holds the capability's URL
/// (a secret), which reqwest's errors quote: only the kind of failure.
pub async fn post(http: &reqwest::Client, url: &str, body: &Llsd) -> bool {
    let sent = http
        .post(url)
        .header("Content-Type", "application/llsd+xml")
        .body(aurora_llsd::to_xml(body))
        .send()
        .await;
    match sent {
        Ok(reply) if reply.status().is_success() => {
            log::debug!("viewer statistics posted to the simulator");
            true
        }
        Ok(reply) => {
            log::debug!("viewer statistics not posted: status {}", reply.status().as_u16());
            false
        }
        Err(e) => {
            let kind = if e.is_timeout() {
                "timed out"
            } else if e.is_connect() {
                "no connection"
            } else {
                "request failed"
            };
            log::debug!("viewer statistics not posted: {kind}");
            false
        }
    }
}

#[cfg(test)]
mod tests;

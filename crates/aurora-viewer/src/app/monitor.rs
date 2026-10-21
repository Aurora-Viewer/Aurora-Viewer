//! The resource monitor's snapshots (ui/monitor): while Performance shows
//! its full view, the frame profile, the network probes and the system
//! sampler measure without a log line, and every half second the app
//! gathers what they and the scene say into one snapshot. Closed, or in
//! its compact view, nothing more is measured than without it.

use super::*;
use crate::disk_io::Origin;
use crate::ui::monitor::snapshot::{
    AvatarMode, AvatarRow, DiskInfo, Feature, FrameParts, GeometryInfo, NetInfo, ObjectRow, RegionHealth, RegionLink, SceneInfo, Snapshot,
    TextureInfo,
};
use crate::ui::monitor::{self, Action, MediaProcess, Page, Vram, model};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// How often a snapshot is taken.
const SAMPLE: Duration = Duration::from_millis(500);
/// How often the scene's avatars and objects are listed again.
const SCENE_EVERY: Duration = Duration::from_secs(2);
/// Rows of the scene's tables.
const SCENE_ROWS: usize = 40;
/// Object names asked of the simulators per second (RequestObjectPropertiesFamily).
const NAMES_PER_SECOND: usize = 5;

/// What the snapshots need of the previous ones.
#[derive(Default)]
pub(super) struct MonitorFeed {
    live: bool,
    last: Option<Instant>,
    /// Disk bytes read and written since the start by origin, at the last
    /// snapshot, and the rates (smoothed).
    disk: Option<(Instant, [(u64, u64); Origin::ALL.len()])>,
    disk_rates: [[f64; 2]; Origin::ALL.len()],
    page_faults: Option<(Instant, u32)>,
    /// Cache counters when the view opened: texture (found, missed,
    /// compressed), objects (found, missed); mesh reads counted since.
    texture_cache_base: [u64; 3],
    object_cache_base: (u64, u64),
    mesh_cache: [u64; 2],
    /// The probes' second last counted.
    probe_serial: u64,
    /// The demo's region statistics were asked.
    region_asked: bool,
    scene_at: Option<Instant>,
    scene: SceneInfo,
    names_asked: HashSet<Uuid>,
    /// Texture bytes sent to the GPU in the last frame (taken by the
    /// deep profile's frame sample).
    pub(super) upload_bytes: u64,
}

impl App {
    /// Each frame: turn the measures on or off with the view, and take a
    /// snapshot every half second while it shows (and is not paused).
    pub(super) fn monitor_frame(&mut self, renderer: &aurora_render::Renderer) {
        let live = self.panels.perf && ui::perf::is_full(&self.egui_ctx);
        if live != self.monitor_feed.live {
            self.monitor_feed.live = live;
            self.frame_profile.set_live(live);
            self.deep_profile.set_live(live);
            crate::sys_probe::set_live(live, crate::sys_probe::current_thread_id());
            if live {
                let counts = self.scene.textures.cache_counts();
                let net = self.net.stats.snapshot();
                self.monitor_feed = MonitorFeed {
                    live: true,
                    texture_cache_base: counts,
                    object_cache_base: (net.object_cache_hits, net.object_cache_misses),
                    ..MonitorFeed::default()
                };
            } else {
                self.monitor.history.clear();
            }
        }
        if !live {
            return;
        }
        // the demo's simulator sends its statistics when asked (a grid's
        // sends them every second)
        if self.demo
            && self.world.estate.stats.is_none()
            && !self.monitor_feed.region_asked
            && let Some(handle) = self.world.main_region
        {
            self.monitor_feed.region_asked = true;
            self.send(NetCommand::Estate(aurora_net::estate::EstateCommand::Refresh { handle }));
        }
        let media = self.media_processes();
        crate::sys_probe::watch_children(media.iter().map(|m| m.pid).collect());
        let now = Instant::now();
        if self.monitor.paused || self.monitor_feed.last.is_some_and(|t| now.duration_since(t) < SAMPLE) {
            return;
        }
        self.monitor_feed.last = Some(now);
        let snapshot = self.monitor_snapshot(renderer, &media);
        self.monitor.history.push(snapshot);
    }

    /// The media plugin processes running now: the parcel's and the faces'
    /// media, the web pages of profiles, search and the marketplace.
    fn media_processes(&self) -> Vec<MediaProcess> {
        let mut out = Vec::new();
        for m in &self.media.impls {
            let Some(pid) = m.process_id() else { continue };
            let label = match &m.key {
                crate::media::MediaKey::Parcel => aurora_i18n::tr!("monitor-media-parcel").to_owned(),
                _ => {
                    let what = if !m.title.is_empty() { m.title.clone() } else { m.url.clone() };
                    aurora_i18n::tr!("monitor-media-face", what = what)
                }
            };
            out.push(MediaProcess {
                pid,
                label,
                video: m.is_time_media(),
            });
        }
        let pages = [
            (self.search_ui.web_process_id(), aurora_i18n::tr!("monitor-media-search")),
            (self.marketplace_ui.web_process_id(), aurora_i18n::tr!("monitor-media-marketplace")),
        ];
        for (pid, label) in pages {
            if let Some(pid) = pid {
                out.push(MediaProcess {
                    pid,
                    label: label.to_owned(),
                    video: false,
                });
            }
        }
        for pid in self.profile_ui.web_process_ids() {
            out.push(MediaProcess {
                pid,
                label: aurora_i18n::tr!("monitor-media-profile").to_owned(),
                video: false,
            });
        }
        out
    }

    fn monitor_snapshot(&mut self, renderer: &aurora_render::Renderer, media: &[MediaProcess]) -> Snapshot {
        let now = Instant::now();
        let cpu = crate::sys_probe::live_snapshot();
        let frame = self.frame_profile.averages().cloned();
        let (serial, probes) = self
            .deep_profile
            .last_second()
            .map_or((0, None), |(serial, p)| (serial, Some(p.clone())));
        let memory = crate::sys_probe::process_memory();
        let video = renderer.video_memory.read(&renderer.device);
        let r = &self.last_render;
        let vram = Vram {
            pages: r.texture_page_bytes,
            geometry: r.geometry_bytes,
            allocated: video.map(|v| v.allocated),
            usage: video.map(|v| v.usage),
            budget: video.map(|v| v.budget),
        };

        // disk rates since the last snapshot, smoothed over about a second
        let mut totals = crate::disk_io::totals();
        let objects = Origin::Objects as usize;
        totals[objects].0 += aurora_net::objcache::DISK_READ.load(std::sync::atomic::Ordering::Relaxed);
        totals[objects].1 += aurora_net::objcache::DISK_WRITTEN.load(std::sync::atomic::Ordering::Relaxed);
        if let Some((at, before)) = self.monitor_feed.disk {
            let secs = now.duration_since(at).as_secs_f64().max(1e-3);
            for (i, rate) in self.monitor_feed.disk_rates.iter_mut().enumerate() {
                let read = totals[i].0.saturating_sub(before[i].0) as f64 / secs;
                let written = totals[i].1.saturating_sub(before[i].1) as f64 / secs;
                rate[0] += (read - rate[0]) * 0.5;
                rate[1] += (written - rate[1]) * 0.5;
            }
        }
        self.monitor_feed.disk = Some((now, totals));
        let page_faults_per_s = memory.and_then(|m| {
            let rate = self
                .monitor_feed
                .page_faults
                .map(|(at, n)| f64::from(m.page_faults.wrapping_sub(n)) / now.duration_since(at).as_secs_f64().max(1e-3));
            self.monitor_feed.page_faults = Some((now, m.page_faults));
            rate.map(|r| r as f32)
        });

        let (cpu_parts, gpu_parts) = self.perf.frame_parts();
        let net = self.net.stats.snapshot();
        let link = &self.net_profile.last;
        let net_info = NetInfo {
            in_kbps: self.perf.net_rate.2 + self.perf.http_rate_kbps,
            out_kbps: self.perf.net_rate.3,
            ping_ms: link.ping_ms,
            loss: crate::net_profile::loss_percent(link),
            packets_in: link.packets_in,
            packets_out: link.packets_out,
            resent: link.resent,
            given_up: link.given_up,
            unacked: link.unacked,
            input_delay_ms: self.net_profile.totals.input_delay_ms,
            loop_ms: link.loop_busy_ms,
            http_active: net.http_in_flight,
            http_queued: net.http_queued,
            corrections: (
                u64::from(self.net_profile.totals.corrections.count),
                self.net_profile.totals.corrections.largest,
            ),
            event_wait_ms: link.event_wait_ms,
            downloaded: net.http_bytes,
            http_failed: net.http_failed,
            throttle_asked: self.net.stats.throttle_asked_kbps.load(std::sync::atomic::Ordering::Relaxed),
            throttle_setting: bandwidth_kbps(&self.settings) as u32,
        };
        let inputs = model::Inputs {
            cpu: cpu.as_ref(),
            frame: frame.as_ref(),
            probes: probes.as_ref(),
            udp_kbps: [f64::from(self.perf.net_rate.2), f64::from(self.perf.net_rate.3)],
            disk: self.monitor_feed.disk_rates,
            working_set: memory.map(|m| m.working_set),
            vram,
            media,
        };
        let attribution = model::attribute(&inputs);

        let tex = &self.scene.textures.stats;
        let texture = TextureInfo {
            loaded: tex.loaded,
            total: tex.total,
            fetching: tex.fetching,
            decoding: tex.decoding,
            pending_upload: tex.pending_upload,
            uploads_frame: tex.uploads_frame,
            upload_bytes_frame: self.monitor_feed.upload_bytes,
            failing: tex.failing,
            target: tex.target_bytes,
            pressure: tex.pressure,
            max_pressure: crate::scene::texture_budget::MAX_PRESSURE,
            reduced: tex.reduced,
            budget_auto: self.settings.texture_budget_auto,
            jobs: self.scene.stats.jobs,
            meshes_fetching: self.scene.meshes.fetching,
        };
        let geometry = GeometryInfo {
            allocated: r.geometry_bytes,
            used: r.vertex_used * (std::mem::size_of::<aurora_render::Vertex>() + std::mem::size_of::<aurora_render::SkinVertex>()) as u64
                + r.index_used * 2,
            pending: self.scene.stats.geom_pending,
            records: r.records as usize,
            records_frame: r.records_uploaded,
            palettes_frame: r.palettes_uploaded,
            cull_frame: r.cull_uploaded,
            journal_frame: r.journal_bytes,
        };

        // the cache's hit rates since the view opened (each second of the
        // probes counted once: a snapshot comes every half second)
        if let Some(p) = probes.as_ref().filter(|_| serial != self.monitor_feed.probe_serial) {
            self.monitor_feed.probe_serial = serial;
            self.monitor_feed.mesh_cache[0] += p.mesh_cache[0];
            self.monitor_feed.mesh_cache[1] += p.mesh_cache[1];
        }
        let counts = self.scene.textures.cache_counts();
        let base = self.monitor_feed.texture_cache_base;
        let (hits, misses, bc7) = (
            counts[0].saturating_sub(base[0]),
            counts[1].saturating_sub(base[1]),
            counts[2].saturating_sub(base[2]),
        );
        let rate = |found: u64, total: u64| (total > 0).then(|| (found as f32 / total as f32 * 100.0, total));
        let (oh, om) = (
            net.object_cache_hits.saturating_sub(self.monitor_feed.object_cache_base.0),
            net.object_cache_misses.saturating_sub(self.monitor_feed.object_cache_base.1),
        );
        let mc = self.monitor_feed.mesh_cache;
        let dirs = self.disk_cache.usage_by_dir();
        let disk = DiskInfo {
            rates: self.monitor_feed.disk_rates,
            by_dir: crate::cache::DIRS.iter().copied().zip(dirs).collect(),
            used: self.disk_cache.usage(),
            limit: self.disk_cache.limit(),
            last_trim: self.disk_cache.last_trim(),
            hits: [
                rate(hits, hits + misses),
                rate(bc7, bc7 + hits + misses),
                rate(mc[0], mc[0] + mc[1]),
                rate(oh, oh + om),
            ],
        };

        if self.monitor_feed.scene_at.is_none_or(|t| now.duration_since(t) >= SCENE_EVERY) {
            self.monitor_feed.scene_at = Some(now);
            self.monitor_feed.scene = self.monitor_scene();
            self.ask_object_names();
        }
        let r = &self.last_render;
        let mut scene = self.monitor_feed.scene.clone();
        let st = &self.scene.stats;
        scene.object_count = st.objects;
        scene.visible = st.visible_objects;
        scene.triangles = r.triangles;
        scene.draws = r.draws as usize;
        scene.shadow_draws = r.shadow_draws as usize;
        scene.particles = r.particles as usize;
        scene.lights = r.lights as usize;
        scene.avatar_count = st.avatars.total;
        scene.full = st.avatars.full;
        scene.impostors = st.avatars.impostors;
        scene.silhouettes = st.avatars.silhouettes;
        scene.occluded = r.occluded;
        scene.gpu_cull = r.gpu_cull;
        let (known, ready) = self.probes.counts();
        // (the sky's own probe counts with the others, as in the compact view)
        scene.probes = (known + 1, ready);
        scene.sounds = self.scene.sounds.voices();
        scene.own_complexity = self.scene.avatar_complexity.get(&self.world.agent_id).copied();
        scene.reflections = (r.water_reflection, r.mirror);

        Snapshot {
            attribution,
            cpu,
            probes,
            fps: frame.as_ref().map_or(self.perf.fps, |f| f.fps),
            frame_ms: frame.as_ref().map_or(self.perf.frames.avg, |f| f.frame_ms),
            parts: FrameParts {
                cpu: cpu_parts,
                gpu: gpu_parts,
                render_thread: r.render_thread.then_some(self.perf.render_thread_shown),
            },
            memory,
            page_faults_per_s,
            vram,
            texture,
            geometry,
            net: net_info,
            regions: self.monitor_regions(),
            disk,
            scene,
            region: self.monitor_region(),
        }
    }

    /// The simulators the viewer has a circuit with, the agent's first.
    fn monitor_regions(&self) -> Vec<RegionLink> {
        let sims = self.net.stats.simulators.lock().clone();
        let main = self.world.main_region;
        let mut out: Vec<RegionLink> = sims
            .iter()
            .map(|s| {
                let name = s
                    .handle
                    .and_then(|h| self.world.regions.get(&h).map(|r| r.name.clone()))
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| aurora_i18n::tr!("monitor-region-unknown").to_owned());
                RegionLink {
                    name,
                    main: s.handle.is_some() && s.handle == main,
                    dilation: s.time_dilation,
                    packet_age: s.packet_age(),
                }
            })
            .collect();
        out.sort_by(|a, b| b.main.cmp(&a.main).then(a.name.cmp(&b.name)));
        out
    }

    /// The agent's simulator as its statistics last said (SimStats come
    /// from the main region only).
    fn monitor_region(&self) -> Option<RegionHealth> {
        use aurora_net::estate::sim_stat;
        let (handle, stats) = self.world.estate.stats.as_ref()?;
        if Some(*handle) != self.world.main_region {
            return None;
        }
        // (ids not named in sim_stat, from LLViewerStats' SimStat ids)
        const NETMS: u32 = 5;
        const SIMOTHERMS: u32 = 6;
        const AGENTMS: u32 = 8;
        const IMAGESMS: u32 = 9;
        Some(RegionHealth {
            name: self.world.region_name().to_owned(),
            dilation: stats.get(sim_stat::TIME_DILATION),
            fps: stats.get(sim_stat::FPS),
            physics_fps: stats.get(sim_stat::PHYSFPS),
            frame_ms: stats.get(sim_stat::FRAMEMS),
            physics_ms: stats.get(sim_stat::SIMPHYSICSMS),
            script_ms: stats.get(sim_stat::SCRIPTMS),
            net_ms: stats.get(NETMS),
            agent_ms: stats.get(AGENTMS),
            images_ms: stats.get(IMAGESMS),
            other_ms: stats.get(SIMOTHERMS),
            spare_ms: stats.get(sim_stat::SIMSPARETIME),
            scripts: stats.get(sim_stat::NUMSCRIPTSACTIVE),
            agents: stats.get(sim_stat::NUMAGENTMAIN),
            child_agents: stats.get(sim_stat::NUMAGENTCHILD),
            objects: stats.get(sim_stat::NUMTASKS),
            capacity: stats.object_capacity,
        })
    }

    /// Video memory of a set of textures at the size they are shown.
    fn shown_bytes(&self, textures: &HashSet<Uuid>) -> u64 {
        let t = &self.scene.textures;
        textures
            .iter()
            .filter_map(|id| {
                let (w, h) = t.full_size(id)?;
                let level = u32::from(t.decoded_level(id)?);
                Some(u64::from((w >> level).max(1)) * u64::from((h >> level).max(1)) * 4)
            })
            .sum()
    }

    /// The avatars and the objects around, their costs. A light pass over
    /// the scene's draw entries counts triangles and faces by avatar and by
    /// linkset; the textures and the features are only gathered for the
    /// avatars and the costliest linksets listed.
    fn monitor_scene(&self) -> SceneInfo {
        let scene = &self.scene;
        let world = &self.world;
        let eye = self.camera.position;
        let draw_distance = self.settings.draw_distance;
        #[derive(Default, Clone, Copy)]
        struct Count {
            prims: usize,
            tris: u64,
            faces: usize,
            attachments: usize,
        }
        let tris_of = |g: &crate::scene::ObjGpu| g.faces.iter().map(|f| u64::from(f.cmd.index_count / 3)).sum::<u64>();
        let mut avatars: HashMap<usize, (Count, HashSet<Uuid>)> = HashMap::new();
        let mut objects: HashMap<usize, Count> = HashMap::new();
        let mut rigged_total = 0u64;
        let mut attachments_total = 0usize;
        for (i, g) in scene.gpu.iter().enumerate() {
            if g.hud || (g.faces.is_empty() && !g.is_avatar) {
                continue;
            }
            let Some(o) = world.objects.get(i) else { continue };
            let tris = tris_of(g);
            if g.is_avatar || g.owner_avatar.is_some() {
                let owner = if g.is_avatar { i } else { g.owner_avatar.unwrap_or(i) };
                let (c, textures) = avatars.entry(owner).or_default();
                if !g.is_avatar {
                    // a worn object counts once, by its root
                    if world.objects.get(owner).is_some_and(|av| av.key.local_id == o.parent_id) {
                        c.attachments += 1;
                        attachments_total += 1;
                    }
                    if g.rigged {
                        rigged_total += tris;
                    }
                }
                c.prims += 1;
                c.tris += tris;
                c.faces += g.faces.len();
                for f in &g.faces {
                    textures.insert(f.tex_id);
                    textures.extend(f.aux_tex.iter().filter(|t| !t.is_nil()));
                }
                continue;
            }
            let root = if o.parent_id == 0 { i } else { crate::build::root_of(world, i) };
            let c = objects.entry(root).or_default();
            c.prims += 1;
            c.tris += tris;
            c.faces += g.faces.len();
        }

        let mut avatar_rows: Vec<AvatarRow> = avatars
            .into_iter()
            .filter_map(|(slot, (c, mut textures))| {
                let o = world.objects.get(slot)?;
                let g = scene.gpu.get(slot)?;
                if !g.is_avatar {
                    return None;
                }
                textures.remove(&Uuid::nil());
                let distance = g.center.distance(eye);
                let state = g.cull_state;
                use aurora_render::gpu_cull::avatar_state;
                let mode = if state & avatar_state::LOADING != 0 {
                    AvatarMode::Loading
                } else if state & avatar_state::TOO_COMPLEX != 0 {
                    AvatarMode::Silhouette
                } else if state & avatar_state::FULL != 0 {
                    AvatarMode::Full
                } else if distance < draw_distance {
                    AvatarMode::Impostor
                } else {
                    AvatarMode::Hidden
                };
                Some(AvatarRow {
                    id: o.full_id,
                    name: ui::people::radar_name(world, &o.full_id, self.settings.radar.name_format),
                    distance,
                    complexity: scene.avatar_complexity.get(&o.full_id).copied(),
                    triangles: c.tris,
                    attachments: c.attachments,
                    vram: self.shown_bytes(&textures),
                    mode,
                })
            })
            .collect();
        avatar_rows.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        avatar_rows.truncate(SCENE_ROWS * 2);

        // the costliest linksets by triangles, then their textures and
        // what else makes them dear
        let mut roots: Vec<(usize, Count)> = objects.into_iter().collect();
        roots.sort_by_key(|a| std::cmp::Reverse(a.1.tris));
        roots.truncate(SCENE_ROWS);
        let object_rows = roots
            .into_iter()
            .filter_map(|(root, c)| {
                let o = world.objects.get(root)?;
                let mut textures = HashSet::new();
                let mut features = Vec::new();
                let mut add = |f: Feature| {
                    if !features.contains(&f) {
                        features.push(f);
                    }
                };
                let mut center = None;
                for i in crate::build::family(world, root) {
                    let (Some(prim), Some(g)) = (world.objects.get(i), scene.gpu.get(i)) else {
                        continue;
                    };
                    center.get_or_insert(g.center);
                    for f in &g.faces {
                        textures.insert(f.tex_id);
                        textures.extend(f.aux_tex.iter().filter(|t| !t.is_nil()));
                        if f.pass == crate::scene::Pass::Blend {
                            add(Feature::Transparent);
                        }
                    }
                    if prim.particles.is_some() {
                        add(Feature::Particles);
                    }
                    if prim.extra.light.is_some() || g.is_light {
                        add(Feature::Light);
                    }
                    if !prim.media_url.is_empty() {
                        add(Feature::Media);
                    }
                    if !prim.sound.id.is_nil() {
                        add(Feature::Sound);
                    }
                    if prim.tex_anim.is_some_and(|a| a.mode & aurora_net::objects::TextureAnim::ON != 0) {
                        add(Feature::Animation);
                    }
                }
                textures.remove(&Uuid::nil());
                let tmem = textures.iter().map(|t| scene.texture_memory(t).0).sum();
                let name = self
                    .build
                    .props
                    .get(&o.full_id)
                    .or_else(|| self.interactions.props.get(&o.full_id))
                    .map(|p| p.name.clone())
                    .filter(|n| !n.is_empty());
                Some(ObjectRow {
                    id: o.full_id,
                    name,
                    prims: c.prims,
                    distance: center.map_or(f32::INFINITY, |p| p.distance(eye)),
                    triangles: c.tris,
                    faces: c.faces,
                    tmem,
                    vram: self.shown_bytes(&textures),
                    features,
                })
            })
            .collect();
        SceneInfo {
            avatars: avatar_rows,
            objects: object_rows,
            rigged_triangles: rigged_total,
            attachments: attachments_total,
            ..SceneInfo::default()
        }
    }

    /// The names of the listed objects not known yet, a few at a time
    /// (RequestObjectPropertiesFamily, as a hover asks for one).
    fn ask_object_names(&mut self) {
        let mut asked = 0;
        let wanted: Vec<Uuid> = self
            .monitor_feed
            .scene
            .objects
            .iter()
            .filter(|o| o.name.is_none())
            .map(|o| o.id)
            .collect();
        for id in wanted {
            if asked >= NAMES_PER_SECOND * SCENE_EVERY.as_secs() as usize || !self.monitor_feed.names_asked.insert(id) {
                continue;
            }
            let Some(handle) = self
                .world
                .objects
                .index_of_uuid(&id)
                .and_then(|i| self.world.objects.get(i))
                .map(|o| o.key.region)
            else {
                continue;
            };
            self.send(NetCommand::RequestObjectProperties { handle, object: id });
            asked += 1;
        }
    }

    /// What the monitor asked this frame.
    pub(super) fn monitor_actions(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Compact => {}
                Action::Export => self.export_monitor(),
                Action::Preferences(page) => {
                    ui::widgets::open_floater(&self.egui_ctx, ui::options::WINDOW, &mut self.panels.settings);
                    self.options_ui.open(
                        match page {
                            Page::Graphics => ui::options::Page::Graphics,
                            Page::Interface => ui::options::Page::Layout,
                            Page::Chat => ui::options::Page::Chat,
                            Page::Network => ui::options::Page::Network,
                            Page::Audio => ui::options::Page::Sound,
                        },
                        0,
                    );
                }
                Action::Inspect(id) => self.inspect_object(id),
                Action::InspectAvatar(id) => self.inspect_avatar(id),
                Action::RegionWindow => ui::region::request_open(&self.egui_ctx),
                Action::TopScripts => {
                    ui::region::request_open(&self.egui_ctx);
                    self.region_ui.demo_top(aurora_net::estate::land_stat::TOP_SCRIPTS);
                }
                Action::OpenCacheFolder => crate::snapshot::gallery::reveal(&crate::settings::cache_dir(), None),
                // the Network page's button, with its confirmation
                Action::ClearCache => {
                    ui::widgets::open_floater(&self.egui_ctx, ui::options::WINDOW, &mut self.panels.settings);
                    self.options_ui.open(ui::options::Page::Network, 0);
                }
            }
        }
    }

    /// The snapshot shown, as a CSV file in the logs' folder: the
    /// subsystems with their seven columns, then the threads.
    fn export_monitor(&mut self) {
        let Some(s) = &self.monitor.history.last else { return };
        let text = monitor::export_csv(s);
        let stamp = crate::snapshot::local_stamp();
        let dir = crate::logging::logs_dir();
        let file = dir.join(format!("resource-monitor_{stamp}.csv"));
        match crate::atomic_file::write(&file, text.as_bytes()) {
            Ok(()) => {
                log::info!("resource monitor report written to {}", file.display());
                self.monitor.exported = file.file_name().map(|n| n.to_string_lossy().into_owned());
                crate::snapshot::gallery::reveal(&dir, Some(&file));
            }
            Err(e) => log::warn!("resource monitor report not written: {e}"),
        }
    }
}

//! The per-frame media update (LLViewerMedia::updateMedia and the media
//! texture updates of LLViewerMediaImpl).

use super::*;
use std::collections::HashSet;

/// Upload budget per frame (bytes): a 1024² media is 4 MB.
const UPLOAD_BUDGET: usize = 16 << 20;

impl MediaManager {
    pub fn update(&mut self, world: &mut World, scene: &mut Scene, renderer: &mut Renderer, f: &MediaFrame) {
        let now = Instant::now();
        self.free_slots(renderer, &mut scene.textures, now);
        let _ = self.plugins(f.settings);
        self.on_parcel_messages(world, f.settings);
        self.scan_objects(world, scene, now);
        self.fetch_media_data(f);
        self.reconcile_prims(world, f);
        self.update_parcel(world, f);
        self.compute_interest(world, scene, f);
        self.assign_priorities(f);
        let navs = self.drive_plugins(f, now);
        self.send_navigates(world, f, navs);
        self.upload_textures(renderer, &mut scene.textures);
        self.publish(world);
        if !self.impls.is_empty() && now.duration_since(self.last_diag) > Duration::from_secs(20) {
            self.last_diag = now;
            for m in &self.impls {
                log::info!(
                    "media: {:?} {} [{}] wanted {} {:?} interest {:.0} px² at {:.0} m, {}{}",
                    m.key,
                    sanitize(&m.url),
                    m.mime,
                    m.wanted,
                    m.priority,
                    m.interest,
                    m.distance,
                    if m.is_loaded() {
                        format!("{:?}", m.status)
                    } else {
                        "not loaded".into()
                    },
                    m.failed.as_ref().map(|e| format!(", failed: {e}")).unwrap_or_default()
                );
            }
        }
    }

    fn free_slots(&mut self, renderer: &mut Renderer, textures: &mut TextureStreamer, now: Instant) {
        self.pending_free.retain(|(slot, id, at)| {
            if now.duration_since(*at) < FREE_DELAY {
                return true;
            }
            textures.unregister_local(id);
            renderer.free_texture(*slot);
            false
        });
    }

    /// Objects with media faces, their media version, and the objects
    /// showing the parcel placeholder (twice a second).
    fn scan_objects(&mut self, world: &World, scene: &Scene, now: Instant) {
        if now.duration_since(self.last_scan) < Duration::from_millis(500) {
            return;
        }
        self.last_scan = now;
        for o in self.objects.values_mut() {
            o.alive = false;
        }
        self.parcel_objects.clear();
        let parcel_tex = world.parcel.as_ref().map(|p| p.media.media_id).filter(|id| !id.is_nil());
        for (idx, o) in world.objects.iter() {
            let Some(te) = o.te.as_ref() else {
                continue;
            };
            if let Some(pt) = parcel_tex
                && te.faces.iter().any(|f| f.texture == pt)
            {
                self.parcel_objects.push(idx);
            }
            let local = self.local_entries.get(&o.full_id);
            if local.is_none() && !te.faces.iter().any(|f| f.media_flags & 1 != 0) {
                continue;
            }
            let on_other_avatar = attachment_avatar(world, idx).is_some_and(|a| a != world.agent_id);
            let e = self.objects.entry(o.full_id).or_default();
            e.alive = true;
            e.idx = idx;
            e.owner = o.owner_id;
            e.hud = scene.gpu.get(idx).is_some_and(|g| g.hud);
            e.on_other_avatar = on_other_avatar;
            e.changed_by = entry::media_version_agent(&o.media_url);
            if let Some(local) = local {
                if e.data.is_none() {
                    e.fetched_version = Some(local.version);
                    e.data = Some(local.clone());
                }
                continue;
            }
            // LLVOVolume::processUpdateMessage: fetch when the media version
            // grew (or never fetched)
            let version = entry::media_version(&o.media_url);
            let need = match (e.fetched_version, version) {
                (None, _) => true,
                (Some(have), Some(v)) => v > have,
                (Some(_), None) => false,
            };
            if need
                && !self.client.is_queued(&o.full_id)
                && let Some(cap) = world.regions.get(&o.key.region).and_then(|r| r.caps.get("ObjectMedia"))
            {
                self.client.request(o.full_id, cap);
            }
        }
        self.objects.retain(|_, o| o.alive);
        if std::env::var_os("AURORA_MEDIA_DEBUG").is_some() {
            log::info!(
                "media scan: {} objects, {} with media, {} local entries",
                world.objects.len(),
                self.objects.len(),
                self.local_entries.len()
            );
        }
    }

    fn fetch_media_data(&mut self, f: &MediaFrame) {
        let Some((rt, http)) = f.net.as_ref() else {
            return;
        };
        for got in self.client.update(rt, http) {
            let Some(e) = self.objects.get_mut(&got.object) else {
                continue;
            };
            match got.data {
                Ok(d) => {
                    // only newer data (updateObjectMediaData)
                    if e.fetched_version.is_none_or(|v| d.version >= v) {
                        log::debug!(
                            "media: {} has media on {} face(s) (version {})",
                            got.object,
                            d.faces.iter().filter(|x| x.is_some()).count(),
                            d.version
                        );
                        e.fetched_version = Some(d.version);
                        e.data = Some(d);
                    }
                }
                Err(err) => {
                    log::info!("media: ObjectMedia for {} failed: {err}", got.object);
                    // stop asking until the version changes
                    e.fetched_version = Some(e.fetched_version.unwrap_or(0));
                }
            }
        }
    }

    /// One impl per media face (LLViewerMedia::updateMediaImpl).
    fn reconcile_prims(&mut self, world: &World, f: &MediaFrame) {
        let mut desired: HashSet<MediaKey> = HashSet::new();
        let allowed = f.settings.enabled && f.settings.prim_media;
        let mut created = Vec::new();
        for (id, om) in &self.objects {
            let Some(data) = om.data.as_ref() else {
                continue;
            };
            if !allowed || (om.on_other_avatar && !f.settings.show_on_others) {
                continue;
            }
            let from_self = om.changed_by == world.agent_id && !om.changed_by.is_nil();
            for (fi, entry) in data.faces.iter().enumerate() {
                let Some(entry) = entry else {
                    continue;
                };
                if entry.url().is_empty() {
                    continue;
                }
                let key = MediaKey::Prim {
                    object: *id,
                    face: fi as u8,
                };
                desired.insert(key);
                if let Some(m) = self.impls.iter_mut().find(|m| m.key == key) {
                    if m.entry.as_ref() != Some(entry) {
                        let old_url = m.entry.as_ref().map(|e| e.url().to_string()).unwrap_or_default();
                        m.entry = Some(entry.clone());
                        m.width = entry.width_pixels as i32;
                        m.height = entry.height_pixels as i32;
                        m.auto_scale = entry.auto_scale;
                        m.looping = entry.auto_loop;
                        // someone else navigated: follow (not our own change)
                        let new_url = entry.url();
                        if new_url != old_url && new_url != m.url && !from_self {
                            m.navigate(new_url);
                        }
                    }
                    continue;
                }
                let mut m = MediaImpl::new(key, entry.url().to_string(), String::new());
                m.width = entry.width_pixels as i32;
                m.height = entry.height_pixels as i32;
                m.auto_scale = entry.auto_scale;
                m.looping = entry.auto_loop;
                // isAutoPlayable; HUD media plays with MediaAutoPlayHuds
                m.wanted = entry.auto_play
                    && ((f.autoplay && self.tentative_autoplay) || (om.hud && f.settings.autoplay_huds) || self.force_autoplay);
                m.entry = Some(entry.clone());
                log::info!(
                    "media: {} face {fi}: {} (auto play {}, {}×{})",
                    id,
                    sanitize(&m.url),
                    entry.auto_play,
                    entry.width_pixels,
                    entry.height_pixels
                );
                created.push(m);
            }
        }
        self.impls.extend(created);
        let stale: Vec<MediaKey> = self
            .impls
            .iter()
            .filter(|m| matches!(m.key, MediaKey::Prim { .. }) && !desired.contains(&m.key))
            .map(|m| m.key)
            .collect();
        for k in stale {
            self.remove(&k);
        }
    }

    /// Interest (pixels on screen, LLViewerMediaImpl::calculateInterest) and
    /// distance to the camera.
    fn compute_interest(&mut self, world: &World, scene: &Scene, f: &MediaFrame) {
        self.parcel_bounds.clear();
        for &idx in &self.parcel_objects {
            if let Some(g) = scene.gpu.get(idx)
                && g.radius > 0.0
                && !g.hud
            {
                self.parcel_bounds.push((g.center, g.radius));
            }
        }
        let parcel_interest = self.parcel_interest(f.view, world);
        for m in &mut self.impls {
            match m.key {
                MediaKey::Parcel => {
                    m.interest = parcel_interest;
                    m.distance = 0.0;
                }
                MediaKey::Prim { object, .. } => {
                    let Some(om) = self.objects.get(&object) else {
                        m.interest = 0.0;
                        continue;
                    };
                    if om.hud {
                        m.interest = 512.0 * 512.0;
                        m.distance = 0.0;
                        continue;
                    }
                    match scene.gpu.get(om.idx).filter(|g| g.radius > 0.0) {
                        Some(g) => {
                            m.distance = g.center.distance(f.camera);
                            m.interest = if f.view.sphere_visible(g.center, g.radius) {
                                let px = f.view.pixel_size(g.center, g.radius);
                                px * px
                            } else {
                                0.0
                            };
                        }
                        None => m.interest = 0.0,
                    }
                }
            }
        }
    }

    /// LLViewerMedia::updateMedia: sorted by priorityComparitor, then the
    /// instance limits.
    fn assign_priorities(&mut self, f: &MediaFrame) {
        let focus = self.focus;
        let mut order: Vec<usize> = (0..self.impls.len()).collect();
        let rank = |m: &MediaImpl| -> (bool, bool, bool, bool) {
            let off = !m.wanted || m.failed.is_some();
            (off, Some(m.key) != focus, m.key != MediaKey::Parcel, !m.wanted)
        };
        order.sort_by(|&a, &b| {
            let (ma, mb) = (&self.impls[a], &self.impls[b]);
            rank(ma)
                .cmp(&rank(mb))
                .then(mb.interest.partial_cmp(&ma.interest).unwrap_or(std::cmp::Ordering::Equal))
                .then(ma.distance.partial_cmp(&mb.distance).unwrap_or(std::cmp::Ordering::Equal))
        });
        let max_instances = f.settings.max_instances.max(1);
        let (mut total, mut normal, mut low) = (0usize, 0usize, 0usize);
        let mut cpu = 0.0f64;
        for i in order {
            let m = &mut self.impls[i];
            let is_parcel = m.key == MediaKey::Parcel;
            let small = {
                let (w, h) = m.plugin.as_ref().map(|p| p.full_size()).unwrap_or((1024, 1024));
                m.interest < (w.max(1) * h.max(1)) as f32 / 4.0
            };
            let mut p = if !m.wanted || m.failed.is_some() || !f.settings.enabled || total >= max_instances {
                Priority::Unloaded
            } else if !is_parcel && m.interest <= 0.0 && Some(m.key) != focus {
                // not visible
                Priority::Hidden
            } else if Some(m.key) == focus {
                Priority::High
            } else if is_parcel {
                Priority::Normal
            } else if cpu > INSTANCES_CPU_LIMIT {
                Priority::Slideshow
            } else if normal < INSTANCES_NORMAL && !small {
                normal += 1;
                Priority::Normal
            } else if normal + low < INSTANCES_NORMAL + INSTANCES_LOW {
                low += 1;
                Priority::Low
            } else {
                Priority::Slideshow
            };
            if p != Priority::Unloaded {
                total += 1;
                cpu += m.plugin.as_ref().map(|p| p.cpu_usage()).unwrap_or(0.0);
                // lost window focus: at most low priority
                if !f.window_focused && p > Priority::Low {
                    p = Priority::Low;
                }
            }
            m.priority = p;
        }
    }

    /// Launch / drop plugins, navigate, feed priorities and volumes, read
    /// their events. Returns the prim navigations to report.
    fn drive_plugins(&mut self, f: &MediaFrame, now: Instant) -> Vec<(Uuid, u8, String)> {
        let mut navs = Vec::new();
        let mut messages = Vec::new();
        let paths = self.paths.clone();
        let browser = browser_settings(f.cache_dir, f.language);
        for m in &mut self.impls {
            if m.priority == Priority::Unloaded {
                if m.plugin.take().is_some() {
                    log::info!("media: unloaded {}", sanitize(&m.url));
                    m.texture.ready = false;
                    m.status = MediaStatus::None;
                    m.nav_pending = true;
                }
                continue;
            }
            // media type (navigateInternal / mimeDiscoveryCoro)
            if m.mime.is_empty() {
                if m.probe.is_none() {
                    match aurora_media::mime_from_scheme(&m.url) {
                        Some(t) => m.mime = t,
                        None => match f.net.as_ref() {
                            Some((rt, http)) => m.probe = Some(probe_mime(rt, http.clone(), m.url.clone())),
                            None => m.mime = "text/html".into(),
                        },
                    }
                }
                if let Some(rx) = &m.probe {
                    match rx.try_recv() {
                        Ok(t) => {
                            log::info!("media: {} is {t}", sanitize(&m.url));
                            m.mime = t;
                            m.probe = None;
                        }
                        Err(crossbeam_channel::TryRecvError::Empty) => continue,
                        Err(_) => {
                            m.mime = "text/html".into();
                            m.probe = None;
                        }
                    }
                }
            }
            let plugin_name = aurora_media::plugin_for_mime(&m.mime);
            if m.plugin.as_ref().is_some_and(|p| p.plugin_name != plugin_name) {
                // another kind of media: another plugin
                m.plugin = None;
                m.texture.ready = false;
                m.nav_pending = true;
            }
            if m.plugin.is_none() {
                if m.failed.is_some() || now.duration_since(self.last_create) < CREATE_DELAY {
                    continue;
                }
                let Some(paths) = paths.as_ref() else {
                    continue;
                };
                self.last_create = now;
                match MediaPlugin::launch(paths, plugin_name, m.width, m.height, &browser, "") {
                    Ok(mut p) => {
                        p.set_auto_scale(m.auto_scale);
                        p.set_loop(m.looping);
                        m.plugin = Some(p);
                        m.nav_pending = true;
                        log::info!("media: {plugin_name} for {} ({})", sanitize(&m.url), m.mime);
                    }
                    Err(e) => {
                        log::warn!("media: cannot start {plugin_name}: {e}");
                        m.failed = Some(e.to_string());
                        continue;
                    }
                }
            }
            let volume = if m.muted {
                0.0
            } else {
                let mut v = f.channel_volume * m.volume;
                if matches!(m.key, MediaKey::Prim { .. }) {
                    // LLViewerMediaImpl::updateVolume roll-off
                    let d = m.distance;
                    if d > ROLLOFF_MAX {
                        v = 0.0;
                    } else if d > ROLLOFF_MIN {
                        let att = 1.0 + ROLLOFF_RATE * (d - ROLLOFF_MIN);
                        v *= (1.0 / (att * att)).min(1.0);
                    }
                }
                v
            };
            let Some(p) = m.plugin.as_mut() else {
                continue;
            };
            if m.nav_pending {
                m.nav_pending = false;
                m.status = MediaStatus::Loading;
                p.load_uri(&m.url);
                if m.paused {
                    p.pause();
                }
            }
            p.set_priority(m.priority);
            if matches!(m.priority, Priority::Low | Priority::Slideshow) {
                p.set_low_priority_size_limit(m.interest.sqrt() as i32);
            }
            p.set_volume(volume);
            p.idle();
            let mut close = false;
            for ev in p.take_events() {
                let complete = matches!(ev, MediaEvent::NavigateComplete { .. });
                match ev {
                    MediaEvent::StatusChanged(s) => m.status = s,
                    MediaEvent::NameChanged(t) => m.title = t,
                    MediaEvent::NavigateComplete { uri, .. } | MediaEvent::LocationChanged(uri) => {
                        // the browser plugin reports no "loaded" status
                        if complete && m.status == MediaStatus::Loading && p.plugin_name == "media_plugin_cef" {
                            m.status = MediaStatus::Loaded;
                        }
                        if uri.is_empty() || uri == m.location {
                            continue;
                        }
                        m.location = uri.clone();
                        // LLVOVolume::mediaNavigated
                        if let (MediaKey::Prim { object, face }, Some(entry)) = (m.key, m.entry.as_ref()) {
                            if !entry.check_url(&uri) {
                                log::info!("media: {} blocked by the whitelist", sanitize(&uri));
                                let back = entry.url().to_string();
                                if back != uri {
                                    m.url = back;
                                    m.nav_pending = true;
                                }
                            } else if m.user_nav && uri != entry.current_url {
                                m.user_nav = false;
                                navs.push((object, face, uri));
                            }
                        }
                    }
                    MediaEvent::ClickLinkHref { url, target } => {
                        if target == "_external" {
                            messages.push(format!("Lien externe : {url}"));
                        } else if !url.is_empty() && url != m.location {
                            // in this media (no browser floater here)
                            m.user_nav = true;
                        }
                    }
                    MediaEvent::CloseRequest => close = true,
                    MediaEvent::CursorChanged(c) => m.cursor = c,
                    MediaEvent::PluginFailed(e) => {
                        log::warn!("media: plugin failed for {}: {e}", sanitize(&m.url));
                        m.failed = Some(e);
                    }
                    _ => {}
                }
            }
            m.can_back = p.history_back;
            m.can_forward = p.history_forward;
            m.current_time = p.current_time;
            m.duration = p.duration;
            if m.failed.is_some() {
                m.plugin = None;
                m.texture.ready = false;
            }
            if close {
                m.wanted = false;
            }
        }
        self.messages.extend(messages);
        navs
    }

    /// ObjectMediaNavigate: tell the region the face shows another page
    /// (others follow it); refused navigations bounce back.
    fn send_navigates(&mut self, world: &World, f: &MediaFrame, navs: Vec<(Uuid, u8, String)>) {
        while let Ok((object, face, result)) = self.nav_rx.try_recv() {
            if let Err(e) = result {
                log::info!("media: navigation of {object} face {face} refused ({e})");
                if let Some(m) = self.find_mut(&MediaKey::Prim { object, face })
                    && let Some(back) = m.entry.as_ref().map(|e| e.url().to_string())
                {
                    m.navigate(&back);
                }
            }
        }
        let Some((rt, http)) = f.net.as_ref() else {
            return;
        };
        for (object, face, url) in navs {
            let Some(cap) = self
                .objects
                .get(&object)
                .and_then(|om| world.objects.get(om.idx))
                .and_then(|o| world.regions.get(&o.key.region))
                .and_then(|r| r.caps.get("ObjectMediaNavigate"))
                .cloned()
            else {
                continue;
            };
            let (tx, http) = (self.nav_tx.clone(), http.clone());
            rt.spawn(async move {
                let mut m = Map::new();
                m.insert("object_id".into(), Llsd::Uuid(object));
                m.insert("current_url".into(), Llsd::String(url));
                m.insert("texture_index".into(), Llsd::Integer(face as i32));
                let r = async {
                    let resp = http
                        .post(&cap)
                        .header("Content-Type", "application/llsd+xml")
                        .header("Accept", "application/llsd+xml")
                        .body(aurora_llsd::to_xml(&Llsd::Map(m)))
                        .timeout(Duration::from_secs(30))
                        .send()
                        .await
                        .map_err(|e| e.to_string())?;
                    if !resp.status().is_success() {
                        return Err(format!("HTTP {}", resp.status()));
                    }
                    let body = resp.bytes().await.map_err(|e| e.to_string())?;
                    match aurora_llsd::from_xml(&body) {
                        Ok(v) if v.has("error") => Err(format!("error {}", v.get("error").get("code").as_i32())),
                        _ => Ok(()),
                    }
                }
                .await;
                let _ = tx.send((object, face, r));
            });
        }
    }

    /// LLViewerMediaImpl::updateMediaImage: frames into the media textures
    /// (a power-of-two texture; the media fills its bottom-left part in GL
    /// terms, i.e. the bottom rows of our top-down textures).
    fn upload_textures(&mut self, renderer: &mut Renderer, textures: &mut TextureStreamer) {
        let mut budget = UPLOAD_BUDGET;
        for m in &mut self.impls {
            let Some(p) = m.plugin.as_mut() else {
                continue;
            };
            let Some(frame) = p.frame() else {
                continue;
            };
            let (tw, th) = p.texture_size();
            let (tw, th) = (tw.max(1) as u32, th.max(1) as u32);
            let fresh = m.texture.slot == 0 || m.texture.size != (tw, th);
            if !fresh && p.dirty().is_none() {
                continue;
            }
            let (mw, mh) = (frame.media_width.min(tw), frame.media_height.min(th));
            let stride = frame.texture_width as usize;
            let bottom_up = frame.bottom_up;
            let rect = match (fresh, p.dirty()) {
                (false, Some(r)) => (
                    (r.left.max(0) as u32).min(mw),
                    (r.right.max(0) as u32).min(mw),
                    (r.bottom.max(0) as u32).min(mh),
                    (r.top.max(0) as u32).min(mh),
                ),
                _ => (0, mw, 0, mh),
            };
            let (x0, x1, y0, y1) = rect;
            let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
            if !fresh && (w as usize * h as usize * 4) > budget {
                // over budget: next frame
                continue;
            }
            if fresh {
                let black: Vec<u8> = [0u8, 0, 0, 255].repeat((tw * th) as usize);
                let level = [MipLevel {
                    width: tw,
                    height: th,
                    data: &black,
                }];
                let slot = if m.texture.slot == 0 {
                    renderer.create_texture(&level)
                } else if renderer.replace_texture(m.texture.slot, &level) {
                    Some(m.texture.slot)
                } else {
                    None
                };
                let Some(slot) = slot else {
                    continue;
                };
                m.texture.slot = slot;
                m.texture.size = (tw, th);
                textures.register_local(m.texture.id, slot);
            }
            if w > 0 && h > 0 {
                let data = frame.data;
                let need = (w * h * 4) as usize;
                self.scratch.resize(need, 0);
                let mut ok = true;
                for k in 0..h {
                    // scratch row k = our texture row (th - y1 + k) when
                    // bottom-up, i.e. GL row (y1 - 1 - k)
                    let src_row = if bottom_up { y1 - 1 - k } else { y0 + k } as usize;
                    let s = (src_row * stride + x0 as usize) * 4;
                    let Some(src) = data.get(s..s + w as usize * 4) else {
                        ok = false;
                        break;
                    };
                    let dst = &mut self.scratch[(k * w * 4) as usize..((k + 1) * w * 4) as usize];
                    for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<4>().0) {
                        d[0] = s[2];
                        d[1] = s[1];
                        d[2] = s[0];
                        d[3] = 255;
                    }
                }
                if ok {
                    let y = if bottom_up { th - y1 } else { y0 };
                    renderer.update_texture_region(m.texture.slot, x0, y, w, h, &self.scratch[..need]);
                    budget = budget.saturating_sub(need);
                }
            }
            p.reset_dirty();
            m.texture.ready = true;
        }
    }

    /// Texture overrides for the scene; objects whose faces change are rebuilt.
    fn publish(&mut self, world: &mut World) {
        let mut by_texture = HashMap::new();
        let mut by_face = HashMap::new();
        for m in &self.impls {
            if !m.texture.ready || m.plugin.is_none() {
                continue;
            }
            match m.key {
                MediaKey::Parcel => {
                    if !m.media_id.is_nil() {
                        by_texture.insert(m.media_id, m.texture.id);
                    }
                }
                MediaKey::Prim { object, face } => {
                    by_face.insert((object, face), m.texture.id);
                }
            }
        }
        if by_texture == world.media.by_texture && by_face == world.media.by_face {
            return;
        }
        let mut dirty_tex: HashSet<Uuid> = HashSet::new();
        for (k, v) in by_texture.iter() {
            if world.media.by_texture.get(k) != Some(v) {
                dirty_tex.insert(*k);
            }
        }
        for k in world.media.by_texture.keys() {
            if !by_texture.contains_key(k) {
                dirty_tex.insert(*k);
            }
        }
        let mut dirty_obj: HashSet<Uuid> = HashSet::new();
        for (k, v) in by_face.iter() {
            if world.media.by_face.get(k) != Some(v) {
                dirty_obj.insert(k.0);
            }
        }
        for k in world.media.by_face.keys() {
            if !by_face.contains_key(k) {
                dirty_obj.insert(k.0);
            }
        }
        world.media.by_texture = by_texture;
        world.media.by_face = by_face;
        for o in world.objects.slots.iter_mut().flatten() {
            let uses_tex = !dirty_tex.is_empty()
                && o.te
                    .as_ref()
                    .is_some_and(|te| te.faces.iter().any(|f| dirty_tex.contains(&f.texture)));
            if uses_tex || dirty_obj.contains(&o.full_id) {
                o.material_dirty = true;
            }
        }
    }
}

/// Root avatar of an attachment.
fn attachment_avatar(world: &World, mut idx: usize) -> Option<Uuid> {
    for _ in 0..16 {
        let o = world.objects.get(idx)?;
        if o.is_avatar() {
            return Some(o.full_id);
        }
        idx = world.objects.parent_of(o)?;
    }
    None
}

/// mimeDiscoveryCoro: headers-only request following redirects; the type
/// up to ';', "text/html" when absent or on failure.
fn probe_mime(rt: &tokio::runtime::Handle, http: reqwest::Client, url: String) -> crossbeam_channel::Receiver<String> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let url = if url.contains("://") { url } else { format!("http://{url}") };
    rt.spawn(async move {
        let t = match http.head(&url).timeout(Duration::from_secs(15)).send().await {
            Ok(resp) => resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "text/html".into()),
            Err(e) => {
                log::info!("media: type probe of {} failed: {e}", sanitize(&url));
                "text/html".into()
            }
        };
        let _ = tx.send(t);
    });
    rx
}

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

    fn scan_objects(&mut self, world: &mut World, scene: &Scene, now: Instant) {
        self.track_objects(world, &|idx| scene.gpu.get(idx).is_some_and(|g| g.hud), now);
    }

    /// Objects with media faces, their media version, and the objects
    /// showing the parcel placeholder, kept up to date from the store's
    /// media change feed: only the objects changed, added or removed since
    /// the last frame are visited (a pass over every object twice a second
    /// made a slow frame every 500 ms in big regions). Every object is
    /// visited only at the start, after `clear`, and the parcel list is
    /// rebuilt when the parcel's placeholder texture changes.
    ///
    /// Twice a second the media objects alone are visited again: what they
    /// derive from other objects or from the region (attached to another
    /// avatar or a HUD once their parents arrive, the ObjectMedia
    /// capability once the region has it) is refreshed as with the old scan.
    fn track_objects(&mut self, world: &mut World, hud: &dyn Fn(usize) -> bool, now: Instant) {
        let mut changes = std::mem::take(&mut self.changes);
        world.objects.take_media_changes(&mut changes);
        let parcel_tex = world.parcel.as_ref().map(|p| p.media.media_id).filter(|id| !id.is_nil());
        let rescan = std::mem::take(&mut self.rescan);
        if rescan || parcel_tex != self.parcel_tex {
            self.parcel_tex = parcel_tex;
            self.parcel_objects = IndexSet::default();
            if let Some(pt) = parcel_tex {
                for (idx, o) in world.objects.iter() {
                    if o.te.as_ref().is_some_and(|te| te.faces.iter().any(|f| f.texture == pt)) {
                        self.parcel_objects.insert(idx);
                    }
                }
            }
        }
        if rescan {
            changes.extend(world.objects.iter().map(|(idx, _)| idx));
        }
        for id in self.local_dirty.drain(..) {
            changes.extend(world.objects.index_of_uuid(&id));
        }
        let refresh = now.duration_since(self.last_refresh) >= Duration::from_millis(500);
        if refresh {
            self.last_refresh = now;
            // media objects by id, wherever they are now (a safety net for
            // the slot links)
            let mut gone = Vec::new();
            for id in self.objects.keys() {
                match world.objects.index_of_uuid(id) {
                    Some(idx) => changes.push(idx),
                    None => gone.push(*id),
                }
            }
            for id in gone {
                if let Some(e) = self.objects.remove(&id) {
                    forget_slot(&mut self.slot_ids, e.idx, &id);
                }
            }
        }
        for &idx in &changes {
            self.visit(world, idx, hud);
        }
        changes.clear();
        self.changes = changes;
        if refresh && std::env::var_os("AURORA_MEDIA_DEBUG").is_some() {
            log::info!(
                "media objects: {} objects, {} with media, {} showing the parcel media, {} local entries",
                world.objects.len(),
                self.objects.len(),
                self.parcel_objects.len(),
                self.local_entries.len()
            );
        }
    }

    /// One slot of the store: what it held is forgotten if the object is
    /// gone, its media state is read again if it is (still) a media object.
    fn visit(&mut self, world: &World, idx: usize, hud: &dyn Fn(usize) -> bool) {
        let obj = world.objects.get(idx);
        if let Some(old) = self.slot_ids.get(&idx).copied()
            && obj.is_none_or(|o| o.full_id != old)
        {
            self.slot_ids.remove(&idx);
            // an object moved to another slot (region crossing) keeps its
            // media data: that slot is in the feed too
            match world.objects.index_of_uuid(&old) {
                Some(moved) => {
                    if let Some(e) = self.objects.get_mut(&old) {
                        e.idx = moved;
                    }
                }
                None => {
                    self.objects.remove(&old);
                }
            }
        }
        let shows_parcel = obj
            .and_then(|o| o.te.as_ref())
            .is_some_and(|te| self.parcel_tex.is_some_and(|pt| te.faces.iter().any(|f| f.texture == pt)));
        if shows_parcel {
            self.parcel_objects.insert(idx);
        } else {
            self.parcel_objects.remove(idx);
        }
        let Some(o) = obj else {
            return;
        };
        let local = self.local_entries.get(&o.full_id);
        let has_media =
            o.te.as_ref()
                .is_some_and(|te| local.is_some() || te.faces.iter().any(|f| f.media_flags & 1 != 0));
        if !has_media {
            if let Some(e) = self.objects.remove(&o.full_id) {
                forget_slot(&mut self.slot_ids, e.idx, &o.full_id);
            }
            forget_slot(&mut self.slot_ids, idx, &o.full_id);
            return;
        }
        let on_other_avatar = attachment_avatar(world, idx).is_some_and(|a| a != world.agent_id);
        let e = self.objects.entry(o.full_id).or_default();
        if e.idx != idx {
            forget_slot(&mut self.slot_ids, e.idx, &o.full_id);
            e.idx = idx;
        }
        self.slot_ids.insert(idx, o.full_id);
        e.owner = o.owner_id;
        e.hud = hud(idx);
        e.on_other_avatar = on_other_avatar;
        e.changed_by = entry::media_version_agent(&o.media_url);
        if let Some(local) = local {
            if e.data.is_none() {
                e.fetched_version = Some(local.version);
                e.data = Some(local.clone());
            }
            return;
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
            if !allowed || (om.on_other_avatar && (!f.settings.show_on_others || om.hud)) {
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
        for &idx in self.parcel_objects.as_slice() {
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
        if dirty_tex.is_empty() && dirty_obj.is_empty() {
            return;
        }
        // through `get_mut`: the scene sync visits the objects it marks; the
        // prim media objects by id, the users of a parcel placeholder (old
        // or new, so not only those of the parcel list) by a pass over every
        // object, done only when the parcel media starts, stops or changes
        let mut marked: Vec<usize> = dirty_obj.iter().filter_map(|id| world.objects.index_of_uuid(id)).collect();
        if !dirty_tex.is_empty() {
            marked.extend(
                world
                    .objects
                    .iter()
                    .filter(|(_, o)| {
                        o.te.as_ref()
                            .is_some_and(|te| te.faces.iter().any(|f| dirty_tex.contains(&f.texture)))
                    })
                    .map(|(i, _)| i),
            );
        }
        for i in marked {
            if let Some(o) = world.objects.get_mut(i) {
                o.material_dirty = true;
            }
        }
    }
}

/// Drop the slot → object link if the slot still names that object.
fn forget_slot(slot_ids: &mut HashMap<usize, Uuid>, idx: usize, id: &Uuid) {
    if slot_ids.get(&idx) == Some(id) {
        slot_ids.remove(&idx);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// The demo place, its media screen, and an ObjectMedia capability.
    fn world() -> (World, Uuid) {
        let mut w = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for ev in crate::demo::events() {
            w.apply(ev);
        }
        let (ev, screen, _) = crate::demo::media_screen();
        w.apply(ev);
        let main = w.main_region.expect("demo region");
        let r = w.regions.get_mut(&main).expect("demo region");
        Arc::make_mut(&mut r.caps).insert("ObjectMedia".into(), "http://example.invalid/media".into());
        (w, screen)
    }

    fn track(m: &mut MediaManager, w: &mut World) {
        m.track_objects(w, &|_| false, m.last_refresh);
    }

    /// A demo object without media faces.
    fn plain(w: &World) -> (usize, Uuid) {
        w.objects
            .iter()
            .find(|(_, o)| !o.is_avatar() && o.te.as_ref().is_some_and(|te| te.faces.iter().all(|f| f.media_flags & 1 == 0)))
            .map(|(i, o)| (i, o.full_id))
            .expect("plain object")
    }

    fn set_media_flag(w: &mut World, idx: usize, on: bool, tracked: bool) {
        let o = if tracked {
            w.objects.get_mut(idx)
        } else {
            w.objects.slots.get_mut(idx).and_then(|o| o.as_mut())
        }
        .expect("object");
        let te = Arc::make_mut(o.te.as_mut().expect("texture entry"));
        te.faces[0].media_flags = u8::from(on);
    }

    #[test]
    fn media_objects_follow_the_change_feed() {
        let (mut w, screen) = world();
        let mut m = MediaManager::default();
        // first update: every object, the screen's data is asked for
        track(&mut m, &mut w);
        assert!(m.objects.contains_key(&screen));
        assert!(m.client.is_queued(&screen));
        let (idx, id) = plain(&w);
        assert!(!m.objects.contains_key(&id));
        // an object gains media
        set_media_flag(&mut w, idx, true, true);
        track(&mut m, &mut w);
        assert_eq!(m.objects.get(&id).map(|e| e.idx), Some(idx));
        assert!(m.client.is_queued(&id));
        // and loses it
        set_media_flag(&mut w, idx, false, true);
        track(&mut m, &mut w);
        assert!(!m.objects.contains_key(&id));
        assert!(!m.slot_ids.contains_key(&idx));
        // a change behind the store's back is not visited: no full pass
        set_media_flag(&mut w, idx, true, false);
        track(&mut m, &mut w);
        assert!(!m.objects.contains_key(&id));
        // ...except after `clear`
        m.clear();
        track(&mut m, &mut w);
        assert!(m.objects.contains_key(&id) && m.objects.contains_key(&screen));
    }

    #[test]
    fn removed_media_objects_are_forgotten_even_when_the_slot_is_reused() {
        let (mut w, screen) = world();
        let mut m = MediaManager::default();
        track(&mut m, &mut w);
        let idx = w.objects.index_of_uuid(&screen).expect("screen");
        w.objects.remove_idx(idx);
        // the freed slot takes another object before the media update
        let (ev, _, _) = crate::demo::media_screen();
        let aurora_net::NetEvent::ObjectUpdates { handle, mut objects } = ev else {
            panic!("object update");
        };
        let mut other = objects.remove(0);
        other.full_id = Uuid::from_u128(0xfeed);
        other.local_id += 1000;
        if let Some(te) = other.texture_entry.as_mut() {
            Arc::make_mut(te).faces.iter_mut().for_each(|f| f.media_flags = 0);
        }
        let reused = w.objects.upsert(handle, other);
        assert_eq!(reused, idx);
        track(&mut m, &mut w);
        assert!(!m.objects.contains_key(&screen));
        assert!(!m.objects.contains_key(&Uuid::from_u128(0xfeed)));
        assert!(m.slot_ids.is_empty(), "{:?}", m.slot_ids);
    }

    #[test]
    fn fetch_follows_the_media_version() {
        let (mut w, screen) = world();
        let mut m = MediaManager::default();
        track(&mut m, &mut w);
        // as if the data of version 5 had arrived
        m.client.clear();
        if let Some(e) = m.objects.get_mut(&screen) {
            e.fetched_version = Some(5);
        }
        let idx = w.objects.index_of_uuid(&screen).expect("screen");
        let agent = Uuid::from_u128(7);
        let set_url = |w: &mut World, v: u32| {
            if let Some(o) = w.objects.get_mut(idx) {
                o.media_url = format!("x-mv:{v:010}/{agent}");
            }
        };
        set_url(&mut w, 5);
        track(&mut m, &mut w);
        assert!(!m.client.is_queued(&screen), "same version: no fetch");
        assert_eq!(m.objects.get(&screen).map(|e| e.changed_by), Some(agent));
        set_url(&mut w, 6);
        track(&mut m, &mut w);
        assert!(m.client.is_queued(&screen), "newer version: fetched");
    }

    #[test]
    fn fetch_waits_for_the_capability_without_a_full_pass() {
        let (mut w, screen) = world();
        let main = w.main_region.expect("demo region");
        if let Some(r) = w.regions.get_mut(&main) {
            r.caps = Arc::default();
        }
        let mut m = MediaManager::default();
        track(&mut m, &mut w);
        assert!(m.objects.contains_key(&screen) && !m.client.is_queued(&screen));
        if let Some(r) = w.regions.get_mut(&main) {
            Arc::make_mut(&mut r.caps).insert("ObjectMedia".into(), "http://example.invalid/media".into());
        }
        track(&mut m, &mut w);
        assert!(!m.client.is_queued(&screen), "not before the refresh");
        let later = m.last_refresh + Duration::from_millis(600);
        m.track_objects(&mut w, &|_| false, later);
        assert!(m.client.is_queued(&screen));
    }

    #[test]
    fn media_on_another_avatar_is_seen_once_its_parents_arrive() {
        let (mut w, screen) = world();
        let mut m = MediaManager::default();
        track(&mut m, &mut w);
        assert_eq!(m.objects.get(&screen).map(|e| e.on_other_avatar), Some(false));
        // the screen becomes the child of an avatar not known yet
        let idx = w.objects.index_of_uuid(&screen).expect("screen");
        let (ev, _, _) = crate::demo::media_screen();
        let aurora_net::NetEvent::ObjectUpdates { handle, mut objects } = ev else {
            panic!("object update");
        };
        let mut avatar = objects.remove(0);
        avatar.pcode = aurora_prim::params::LL_PCODE_LEGACY_AVATAR;
        avatar.texture_entry = None;
        avatar.full_id = Uuid::from_u128(0xa1);
        avatar.local_id = 0xa1a1;
        if let Some(o) = w.objects.get_mut(idx) {
            o.parent_id = avatar.local_id;
        }
        track(&mut m, &mut w);
        assert_eq!(m.objects.get(&screen).map(|e| e.on_other_avatar), Some(false));
        // the avatar arrives: seen at the next refresh, without the screen changing
        w.objects.upsert(handle, avatar);
        let later = m.last_refresh + Duration::from_millis(600);
        m.track_objects(&mut w, &|_| false, later);
        assert_eq!(m.objects.get(&screen).map(|e| e.on_other_avatar), Some(true));
    }

    #[test]
    fn parcel_list_follows_objects_and_the_parcel_texture() {
        let (mut w, _) = world();
        let mut m = MediaManager::default();
        let tex = crate::demo::parcel_media_texture();
        let users = |w: &World, t: Uuid| -> Vec<usize> {
            let mut v: Vec<usize> = w
                .objects
                .iter()
                .filter(|(_, o)| o.te.as_ref().is_some_and(|te| te.faces.iter().any(|f| f.texture == t)))
                .map(|(i, _)| i)
                .collect();
            v.sort();
            v
        };
        let listed = |m: &MediaManager| -> Vec<usize> {
            let mut v = m.parcel_objects.as_slice().to_vec();
            v.sort();
            v
        };
        Arc::make_mut(w.parcel.as_mut().expect("demo parcel")).media.media_id = Uuid::nil();
        track(&mut m, &mut w);
        assert!(listed(&m).is_empty(), "no parcel media");
        Arc::make_mut(w.parcel.as_mut().expect("demo parcel")).media.media_id = tex;
        track(&mut m, &mut w);
        assert!(!users(&w, tex).is_empty());
        assert_eq!(listed(&m), users(&w, tex));
        // an object starts showing the placeholder, another one is removed
        let (idx, _) = plain(&w);
        if let Some(o) = w.objects.get_mut(idx) {
            Arc::make_mut(o.te.as_mut().expect("texture entry")).faces[0].texture = tex;
        }
        let gone = users(&w, tex).into_iter().find(|&i| i != idx).expect("another user");
        w.objects.remove_idx(gone);
        track(&mut m, &mut w);
        assert!(listed(&m).contains(&idx) && !listed(&m).contains(&gone));
        assert_eq!(listed(&m), users(&w, tex));
        // another placeholder: the list is rebuilt
        let other = w
            .objects
            .get(idx)
            .and_then(|o| o.te.as_ref())
            .map(|te| te.faces[1].texture)
            .expect("face 1");
        Arc::make_mut(w.parcel.as_mut().expect("demo parcel")).media.media_id = other;
        track(&mut m, &mut w);
        assert_eq!(listed(&m), users(&w, other));
    }

    #[test]
    fn local_entries_count_as_media() {
        let (mut w, _) = world();
        let mut m = MediaManager::default();
        track(&mut m, &mut w);
        let (_, id) = plain(&w);
        m.demo_entry(id, 0, "https://example.invalid/page");
        track(&mut m, &mut w);
        let e = m.objects.get(&id).expect("local media object");
        assert_eq!(e.fetched_version, Some(1));
        assert!(e.data.as_ref().is_some_and(|d| d.faces[0].is_some()));
        assert!(!m.client.is_queued(&id), "local data is not fetched");
    }
}

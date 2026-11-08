//! Derived normals share the ordinary texture streamer's residency lifecycle.
//! No asset requests, parallel GPU cache, retained CPU images or face invalidation.

use super::*;

pub(super) struct LegacyBumpState {
    source: Uuid,
    mode: u8,
    observed: Option<(PreviewRevision, u32)>,
    prepared: Option<PreviewRevision>,
}

impl TextureStreamer {
    /// Reserve one stable normal slot per source/mode. Faces acquire and release
    /// this ordinary entry just like their other maps, including while it loads.
    pub fn request_legacy_bump(&mut self, renderer: &mut Renderer, source: Uuid, mode: u8) -> Option<Uuid> {
        self.reserve_legacy_bump(renderer, source, mode)
    }

    fn reserve_legacy_bump(&mut self, table: &mut impl SlotTable, source: Uuid, mode: u8) -> Option<Uuid> {
        if !(1..=17).contains(&mode) || source.is_nil() {
            return None;
        }
        if let Some(id) = self.legacy_bumps.get(&(source, mode)) {
            return Some(*id);
        }
        let slot = self.free_slot(table, true)?;
        // A local identity cannot collide with a predictable hash of a grid UUID.
        let mut id = Uuid::new_v4();
        while self.entries.contains_key(&id) || self.local_slots.contains_key(&id) {
            id = Uuid::new_v4();
        }
        let mut entry = Entry::new(slot, TexSource::Asset, self.max_resolution);
        entry.refs = 0; // Acquired by the face, after reserving its identity.
        entry.cache_state = 2;
        entry.complete = true;
        self.entries.insert(id, entry);
        if mode >= 3 && legacy_bump_diagnostics() {
            log::info!("legacy bump requested: mode={mode} slot={slot}");
        }
        self.legacy_bumps.insert((source, mode), id);
        self.legacy_bump_entries.insert(
            id,
            LegacyBumpState {
                source,
                mode,
                observed: None,
                prepared: None,
            },
        );
        set_slot_alpha(
            &mut self.alpha_by_slot,
            &mut self.generation,
            &mut self.alpha_changes,
            slot,
            AlphaKind::Opaque,
        );
        Some(id)
    }

    pub(super) fn forget_legacy_bump(&mut self, id: Uuid) {
        if let Some(state) = self.legacy_bump_entries.remove(&id) {
            self.legacy_bumps.remove(&(state.source, state.mode));
        }
    }

    pub(super) fn reset_legacy_bumps(&mut self) {
        for state in self.legacy_bump_entries.values_mut() {
            state.observed = None;
            state.prepared = None;
        }
    }

    fn note_bump_availability(&mut self, id: Uuid, source: Uuid) {
        let unavailable = self.entries.get(&source).is_some_and(Entry::unavailable);
        if let Some(entry) = self.entries.get_mut(&id) {
            // A missing preset must not keep photo quality waiting forever.
            // Its reserved normal stays flat; a recovered source clears this.
            entry.gone = unavailable;
        }
    }

    /// The regular streamer handles usage, reduction, upload ordering and
    /// eviction. Only the derived entries need their source revision polled.
    pub(super) fn update_legacy_bumps(&mut self, jobs: &Jobs) {
        let Some(pool) = self.staging.clone() else { return };
        let now = Instant::now();
        let mut candidates: Vec<_> = self
            .legacy_bump_entries
            .keys()
            .filter_map(|id| {
                let e = self.entries.get(id)?;
                (e.refs > 0).then_some((
                    UploadPriority {
                        first: e.decoded.is_none(),
                        pixels: e.px_now(),
                        seq: 0,
                    },
                    *id,
                ))
            })
            .collect();
        candidates.sort_unstable_by_key(|c| std::cmp::Reverse(c.0));
        // Results waiting for a page count too: neither staging pressure nor an
        // integrated GPU's smaller chunk can grow an unbounded CPU upload queue.
        let mut queued = self.legacy_bump_jobs + self.uploads.derived;
        for (_, id) in candidates {
            let state = &self.legacy_bump_entries[&id];
            let (source_id, mode) = (state.source, state.mode);
            self.note_bump_availability(id, source_id);
            let mut source = self.preview_source(&source_id);
            if !source.bake_ready(source_id) {
                continue;
            }
            let revision = source.bake_revision();
            let source_epoch = self.entries.get(&source_id).map_or(0, |e| e.epoch);
            let state = self.legacy_bump_entries.get_mut(&id).expect("derived entry");
            let e = self.entries.get_mut(&id).expect("derived texture");
            if state.observed != Some((revision, source_epoch)) {
                state.observed = Some((revision, source_epoch));
                state.prepared = None;
                e.epoch = e.epoch.wrapping_add(1);
                e.retry_at = None;
                e.failures = 0;
            }
            let size = source
                .local
                .as_ref()
                .map(|i| (i.size[0], i.size[1]))
                .or(source.size)
                .or_else(|| e.info.map(|i| (i.width, i.height)));
            if let Some((width, height)) = size {
                let fresh = e.info.is_none_or(|info| (info.width, info.height) != (width, height));
                e.info = Some(bump_info(width, height));
                if fresh {
                    e.want_for_its_size();
                }
                e.want_before_the_first_pass();
            }
            let discard = if size.is_some() {
                e.want.max(source.decoded.unwrap_or(0))
            } else {
                0
            };
            if e.decoding
                || e.retry_at.is_some_and(|t| t > now)
                || (state.prepared == Some(revision) && !e.needs_decode(discard))
                || self.uploads.covers(&id, e.epoch, discard)
                || queued >= 8
                || self.decodes >= self.max_decodes
            {
                continue;
            }
            e.decoding = true;
            self.decodes += 1;
            self.legacy_bump_jobs += 1;
            queued += 1;
            let epoch = e.epoch;
            source.decoded = Some(discard);
            let pool = pool.clone();
            jobs.spawn(move || {
                let image = if mode >= 3 {
                    source.standard_bump_image(source_id, mode)
                } else {
                    source.bake_image(source_id)
                };
                let size = size.or_else(|| image.as_ref().map(|i| (i.size[0], i.size[1])));
                let normal = image.and_then(|image| {
                    // Local/procedural pixels have no decoder discard level.
                    let target = size.map(|(w, h)| texture_cache::discard_size(w, h, discard));
                    let image = match target.filter(|&(w, h)| image.size != [w, h]) {
                        Some((w, h)) => {
                            let rgba = image::RgbaImage::from_raw(image.size[0], image.size[1], image.rgba.to_vec())?;
                            let rgba = image::imageops::resize(&rgba, w, h, image::imageops::FilterType::Triangle);
                            Arc::new(aurora_render::preview::Image {
                                size: [w, h],
                                rgba: rgba.into_raw().into(),
                            })
                        }
                        None => image,
                    };
                    legacy_bump_image(image, mode)
                });
                let levels = normal.map(|image| build_mips(image.size[0], image.size[1], image.rgba.to_vec(), 16));
                let mut staged = None;
                let mut mips = Vec::new();
                if let Some(levels) = levels {
                    let slices: Vec<_> = levels.iter().map(|(w, h, data)| (*w, *h, data.as_slice())).collect();
                    staged = StagedTexture::new(&pool, &slices);
                    // A 2K RGBA chain exceeds the integrated pool's 16 MiB
                    // chunk. Only that case uses the existing budgeted upload
                    // fallback; a temporarily full pool retries off-thread.
                    let (_, bytes) = aurora_render::upload::level_layout(TexelFormat::Rgba8, levels.iter().map(|m| (m.0, m.1)));
                    if staged.is_none() && bytes > pool.max_region_bytes() {
                        mips = levels;
                    }
                }
                JobResult::LegacyBump {
                    id,
                    epoch,
                    revision,
                    discard,
                    size,
                    staged,
                    mips,
                }
            });
            Self::settle(&mut self.todo, &mut self.counts, id, e);
        }
    }

    pub fn on_legacy_bump_job(&mut self, result: JobResult) {
        let JobResult::LegacyBump {
            id,
            epoch,
            revision,
            discard,
            size,
            staged,
            mips,
        } = result
        else {
            return;
        };
        self.legacy_bump_jobs = self.legacy_bump_jobs.saturating_sub(1);
        self.decodes = self.decodes.saturating_sub(1);
        if !self.legacy_bump_entries.contains_key(&id) || !self.entries.contains_key(&id) {
            for (_, _, data) in mips {
                self.trash.push(data);
            }
            return;
        }
        let Some(state) = self.legacy_bump_entries.get_mut(&id) else {
            return;
        };
        let Some(e) = self.entries.get_mut(&id) else { return };
        e.decoding = false;
        let stale_pool = staged
            .as_ref()
            .is_some_and(|s| self.staging.as_ref().is_none_or(|p| !s.data.is_from(p)));
        if epoch != e.epoch || e.refs == 0 || stale_pool {
            state.prepared = None;
            Self::settle(&mut self.todo, &mut self.counts, id, e);
            for (_, _, data) in mips {
                self.trash.push(data);
            }
            return;
        }
        if staged.is_none() && mips.is_empty() {
            e.failures = e.failures.saturating_add(1);
            e.retry_at = Some(Instant::now() + Duration::from_millis(100 * (1u64 << e.failures.min(6))));
            Self::settle(&mut self.todo, &mut self.counts, id, e);
            return;
        }
        if let Some((width, height)) = size {
            e.info = Some(bump_info(width, height));
        }
        state.prepared = Some(revision);
        e.retry_at = None;
        e.failures = 0;
        if state.mode >= 3 && legacy_bump_diagnostics() {
            let size = staged
                .as_ref()
                .and_then(|s| s.levels.first().map(|l| (l.width, l.height)))
                .or_else(|| mips.first().map(|m| (m.0, m.1)));
            log::info!(
                "legacy bump ready: mode={} size={size:?} slot={} discard={discard}",
                state.mode,
                e.slot
            );
        }
        // Reuse the ordinary upload path, including page preparation, device
        // ownership, per-frame time/byte limits and deferred CPU destruction.
        self.decodes += 1; // on_job releases this shared decode slot.
        self.on_job(JobResult::Texture {
            id,
            discard,
            mips,
            staged,
            alpha: AlphaKind::Opaque,
            alpha_channel: false,
            pick_mask: None,
            sculpt: None,
        });
    }
}

fn bump_info(width: u32, height: u32) -> J2kInfo {
    J2kInfo {
        width,
        height,
        components: 4,
        levels: (width.max(height).max(1).ilog2() + 1) as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Slots {
        free: Vec<u32>,
        normal_calls: usize,
    }
    impl SlotTable for Slots {
        fn placeholder(&mut self) -> Option<u32> {
            self.free.pop()
        }
        fn normal_placeholder(&mut self) -> Option<u32> {
            let slot = self.free.pop()?;
            self.normal_calls += 1;
            Some(slot)
        }
        fn free(&mut self, slot: u32) {
            self.free.push(slot);
        }
        fn show_missing(&mut self, _: u32) -> bool {
            true
        }
    }

    fn streamer() -> TextureStreamer {
        TextureStreamer::new(std::env::temp_dir().join(format!("aurora-bump-{}", Uuid::new_v4())))
    }

    #[test]
    fn bump_identity_survives_rebuilds_and_unrelated_source_releases() {
        let mut s = streamer();
        let mut slots = Slots {
            free: vec![11, 12, 13],
            ..Default::default()
        };
        let source = Uuid::new_v4();
        let normal = s.reserve_legacy_bump(&mut slots, source, 1).unwrap();
        s.entries.get_mut(&normal).unwrap().refs = 2;
        let dark = s.reserve_legacy_bump(&mut slots, source, 2).unwrap();
        assert_ne!(normal, dark);
        s.release(&source);
        s.release(&normal);
        assert_eq!(s.reserve_legacy_bump(&mut slots, source, 1), Some(normal));
        assert_eq!(s.entries[&normal].refs, 1);
        assert!(s.local_images.is_empty() && s.local_slots.is_empty());
        s.release(&normal);
        s.evict(&mut slots, vec![normal]);
        assert!(!s.legacy_bumps.contains_key(&(source, 1)));
        assert!(!s.legacy_bump_entries.contains_key(&normal));
        assert_ne!(s.reserve_legacy_bump(&mut slots, source, 1), Some(normal));
    }

    #[test]
    fn bump_residency_uses_the_regular_memory_reduction_and_eviction_rules() {
        let mut s = streamer();
        let mut slots = Slots {
            free: vec![11],
            ..Default::default()
        };
        let normal = s.reserve_legacy_bump(&mut slots, Uuid::new_v4(), 9).unwrap();
        let e = s.entries.get_mut(&normal).unwrap();
        e.refs = 1;
        e.info = Some(bump_info(2048, 2048));
        e.decoded = Some(0);
        e.gpu_size = (2048, 2048);
        e.need_px = 32.0;
        let now = Instant::now();
        s.pressure.hold_max(now);
        s.to_lower = u64::MAX;
        s.recompute_wants();
        assert!(s.reductions.iter().any(|&(id, keep, _)| id == normal && keep > 0));
        assert!(s.entries[&normal].want > 0);
        assert!(s.bytes_below < 2048 * 2048 * 4);
        s.release(&normal);
        let evict = eviction_round(
            &mut s.unused,
            now + Duration::from_secs(10),
            texture_budget::PRESSURE_EVICT,
            |id, since| evictable(&s.entries, id, since),
        );
        assert_eq!(evict, vec![normal]);
        s.evict(&mut slots, evict);
        assert!(s.entries.is_empty() && s.legacy_bumps.is_empty() && s.legacy_bump_entries.is_empty());
    }

    #[test]
    fn bump_old_worker_cannot_cross_source_refresh_or_recreate_evicted_entries() {
        let mut s = streamer();
        let mut slots = Slots {
            free: vec![11],
            ..Default::default()
        };
        let normal = s.reserve_legacy_bump(&mut slots, Uuid::new_v4(), 9).unwrap();
        let e = s.entries.get_mut(&normal).unwrap();
        e.refs = 1;
        e.epoch = 2;
        e.decoding = true;
        let job = || JobResult::LegacyBump {
            id: normal,
            epoch: 1,
            revision: (0, Some(0), Some((16, 16)), false),
            discard: 0,
            size: Some((16, 16)),
            staged: None,
            mips: vec![],
        };
        s.legacy_bump_jobs = 1;
        s.decodes = 1;
        s.on_legacy_bump_job(job());
        assert_eq!((s.decodes, s.legacy_bump_jobs, s.entries[&normal].failures), (0, 0, 0));
        assert!(!s.entries[&normal].decoding);
        assert_eq!(s.uploads.len(), 0);
        s.evict(&mut slots, vec![normal]);
        s.on_legacy_bump_job(job());
        assert!(s.entries.is_empty());
    }

    #[test]
    fn bump_waiting_for_a_slot_recovers_flat_without_fetching_its_local_identity() {
        let mut s = streamer();
        let mut slots = Slots {
            free: vec![11],
            ..Default::default()
        };
        let source = Uuid::new_v4();
        let normal = s.reserve_legacy_bump(&mut slots, source, 9).unwrap();
        s.entries.get_mut(&normal).unwrap().refs = 2;
        let mut replacement = Slots::default();
        s.gpu_reset_into(&mut replacement);
        assert_eq!(s.entries[&normal].slot, NO_SLOT);
        assert_eq!(s.slotless, 1);
        replacement.free.push(99);
        s.give_slots(&mut replacement, Instant::now() + RESLOT_EVERY);
        let e = &s.entries[&normal];
        assert_eq!((e.slot, e.refs, e.cache_state, e.decoded), (99, 2, 2, None));
        assert_eq!(replacement.normal_calls, 1);
        assert!(!e.fetching && e.fetch_key == 0 && e.data.is_empty());
        assert_eq!(s.slotless, 0);
        assert_eq!(s.reslotted, vec![normal]);
        assert_eq!(s.reserve_legacy_bump(&mut replacement, source, 9), Some(normal));
    }

    #[test]
    fn bump_gpu_reset_retains_identity_and_users_without_reading_a_fake_asset() {
        let mut s = streamer();
        let mut slots = Slots {
            free: vec![11],
            ..Default::default()
        };
        let source = Uuid::new_v4();
        let normal = s.reserve_legacy_bump(&mut slots, source, 9).unwrap();
        let e = s.entries.get_mut(&normal).unwrap();
        e.refs = 2;
        e.info = Some(bump_info(16, 16));
        e.decoded = Some(0);
        let mut replacement = Slots {
            free: vec![99],
            ..Default::default()
        };
        s.gpu_reset_into(&mut replacement);
        let e = &s.entries[&normal];
        assert_eq!((e.slot, e.refs, e.cache_state, e.decoded), (99, 2, 2, None));
        assert_eq!(s.reserve_legacy_bump(&mut replacement, source, 9), Some(normal));
        assert!(s.local_images.is_empty());
    }

    #[test]
    fn bump_missing_source_does_not_block_photo_quality_and_can_recover() {
        let mut s = streamer();
        let mut slots = Slots {
            free: vec![11],
            ..Default::default()
        };
        let source = Uuid::new_v4();
        let normal = s.reserve_legacy_bump(&mut slots, source, 9).unwrap();
        let mut height = Entry::new(12, TexSource::Asset, 2048);
        height.gone = true;
        s.entries.insert(source, height);
        let e = s.entries.get_mut(&normal).unwrap();
        e.refs = 1;
        e.info = Some(bump_info(16, 16));
        e.need_px = 16.0;
        s.note_bump_availability(normal, source);
        s.recompute_wants();
        assert_eq!(s.pending_in_view(), 0);
        s.entries.get_mut(&source).unwrap().gone = false;
        s.note_bump_availability(normal, source);
        s.entries.get_mut(&normal).unwrap().need_px = 16.0;
        s.recompute_wants();
        assert_eq!(s.pending_in_view(), 1);
    }
}

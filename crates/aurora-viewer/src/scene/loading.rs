//! Avatars still loading, after LLVOAvatar::updateIsFullyLoaded /
//! getRezzedStatus / processFullyLoadedChange of the Second Life / Firestorm
//! viewer (originally LGPL 2.1, Copyright (C) 2001-2024 Linden Research, Inc. and the
//! Firestorm project): an avatar shows once its bakes are downloaded and its
//! worn objects have arrived and been built, a little after it settles (3 to
//! 15 s the first time, 1 s later on); until then it is a particle cloud
//! (Firestorm cloud.xml) with a progress bar under its name.

use super::{GeomKey, Scene, avatar};
use crate::world::World;
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

/// processFullyLoadedChange: LOADED_DELAY, FIRST_APPEARANCE_CLOUD_MIN_DELAY.
const LOADED_DELAY: f32 = 1.0;
const FIRST_MIN_DELAY: f32 = 3.0;
/// MAX_TEXTURE_WAIT_TIME_SEC / MAX_ATTACHMENT_WAIT_TIME_SEC: shown anyway.
const MAX_WAIT: f32 = 60.0;
/// The worn objects count changed less than this long ago: more may come
/// (LL compares with the simulator's attachment list).
const ATTACHMENTS_SETTLE: f32 = 2.0;

#[derive(Debug, Clone, Copy)]
struct State {
    first_seen: Instant,
    /// Last time it was still loading.
    loading_at: Instant,
    /// Shown fully once (later loads use the short delay).
    shown_once: bool,
    attachments: usize,
    attachments_changed: Instant,
    /// "Still loading" already logged.
    reported: bool,
    /// Progress shown so far (it never goes back when more parts arrive).
    shown: f32,
}

#[derive(Default)]
pub struct Loading {
    states: HashMap<Uuid, State>,
    /// Avatars not fully loaded yet, with their progress (0..1).
    pub progress: HashMap<Uuid, f32>,
    /// Their object indices.
    pub indices: std::collections::HashSet<usize>,
}

impl Scene {
    /// What is loaded of an avatar: (done, total), what is still missing
    /// (empty when complete) and the textures to hurry.
    fn avatar_load_parts(&self, world: &World, idx: usize, hurry: &mut Vec<Uuid>) -> (u32, u32, Vec<String>) {
        let Some(av) = world.objects.get(idx) else {
            return (0, 1, vec!["avatar".into()]);
        };
        let (mut done, mut total, mut missing) = (0u32, 0u32, Vec::new());
        // bakes: defined (getHasMissingParts) and downloaded; only those
        // something shows count (bakes on mesh avatars define unused ones)
        for (te, name, _) in avatar::BAKES {
            let t = world.bake_texture(idx, te);
            let main = matches!(name, "head" | "upper" | "lower");
            match t {
                Some(t) if !t.is_nil() && t != avatar::IMG_DEFAULT_AVATAR && t != avatar::IMG_INVISIBLE => {
                    if !self.textures.is_wanted(&t) {
                        continue;
                    }
                    total += 1;
                    if self.textures.is_loaded(&t) {
                        done += 1;
                    } else {
                        hurry.push(t);
                        missing.push(format!("bake {name}"));
                    }
                }
                _ if main => {
                    // no appearance yet
                    total += 1;
                    missing.push("appearance".into());
                }
                _ => {}
            }
        }
        // worn objects: geometry built (meshes downloaded)
        for &att in world.objects.children_of(&av.key) {
            let Some(root) = world.objects.get(att) else {
                continue;
            };
            if (31..=38).contains(&root.attachment_point()) {
                continue; // HUD
            }
            let mut prims = vec![att];
            prims.extend_from_slice(world.objects.children_of(&root.key));
            for p in prims {
                let Some(g) = self.gpu.get(p) else {
                    continue;
                };
                total += 1;
                // geometry asked for and not there yet (a failed mesh is given up)
                let pending = g.wanted_geom.is_some_and(|k| Some(k) != g.geom) || g.sculpt_wait.is_some();
                let failed = matches!(g.wanted_geom, Some(GeomKey::Mesh { .. })) && {
                    let m = world.objects.get(p).and_then(|o| o.volume.sculpt).map(|s| s.texture);
                    m.is_some_and(|id| self.meshes.failed(&id))
                };
                if pending && !failed {
                    missing.push(format!("geometry of {}", world.objects.get(p).map(|o| o.key.local_id).unwrap_or(0)));
                } else {
                    done += 1;
                }
                // their textures first (the avatar is not drawn meanwhile)
                for f in &g.faces {
                    if !f.tex_id.is_nil() && !self.textures.is_loaded(&f.tex_id) {
                        hurry.push(f.tex_id);
                    }
                }
            }
        }
        (done, total.max(1), missing)
    }

    /// Refresh which avatars are still loading (every frame).
    pub fn update_loading(&mut self, world: &World, now: Instant) {
        let mut loading = std::mem::take(&mut self.loading);
        loading.progress.clear();
        loading.indices.clear();
        let mut seen = Vec::new();
        let mut hurry = Vec::new();
        for (idx, o) in world.objects.iter() {
            if !o.is_avatar() {
                continue;
            }
            // silhouettes are not clouded (LL: no cloud for too complex avatars)
            if self.too_complex.contains(&idx) {
                seen.push(o.full_id);
                continue;
            }
            let id = o.full_id;
            seen.push(id);
            let attachments = world.objects.children_of(&o.key).len();
            let s = loading.states.entry(id).or_insert(State {
                first_seen: now,
                loading_at: now,
                shown_once: false,
                attachments,
                attachments_changed: now,
                reported: false,
                shown: 0.0,
            });
            if s.attachments != attachments {
                s.attachments = attachments;
                s.attachments_changed = now;
            }
            let mut wanted = Vec::new();
            let (done, total, missing) = self.avatar_load_parts(world, idx, &mut wanted);
            let settling = now.duration_since(s.attachments_changed).as_secs_f32() < ATTACHMENTS_SETTLE;
            let waited = now.duration_since(s.first_seen).as_secs_f32();
            let still = (!missing.is_empty() || settling) && waited < MAX_WAIT;
            if !s.shown_once {
                hurry.extend(wanted);
                // diagnostic: still a cloud after 20 s
                if waited > 20.0 && !s.reported {
                    s.reported = true;
                    log::info!(
                        "avatar {id} still loading after {waited:.0} s: {done}/{total} parts, attachments settling {settling}, missing {:?}",
                        &missing[..missing.len().min(8)]
                    );
                }
            }
            if still {
                s.loading_at = now;
            }
            // processFullyLoadedChange: once nothing is missing, 3 s the first
            // time (1 s later on); LL's 3..15 s delay (x1.25 for impostors)
            // only applies while still loading, when the timer is reset anyway
            let delay = if s.shown_once { LOADED_DELAY } else { FIRST_MIN_DELAY };
            let loaded = !still && (now.duration_since(s.loading_at).as_secs_f32() > delay || waited > MAX_WAIT);
            if loaded {
                if !s.shown_once {
                    log::info!("avatar {id} shown after {waited:.1} s");
                }
                s.shown_once = true;
            } else if !s.shown_once {
                // parts ready over parts known; the last bit is the settling
                // delay (it fills as the avatar is about to show)
                let parts = done as f32 / total as f32;
                let p = if still {
                    parts * 0.9
                } else {
                    0.9 + 0.1 * (now.duration_since(s.loading_at).as_secs_f32() / delay).min(1.0)
                };
                s.shown = s.shown.max(p.min(0.99));
                loading.progress.insert(id, s.shown);
                loading.indices.insert(idx);
            }
        }
        loading.states.retain(|id, _| seen.contains(id));
        self.loading = loading;
        for t in hurry {
            self.textures.note_usage(&t, 512.0);
        }
    }

    /// Avatars to show as clouds (not fully loaded yet).
    pub fn loading_avatars(&self) -> &HashMap<Uuid, f32> {
        &self.loading.progress
    }
}

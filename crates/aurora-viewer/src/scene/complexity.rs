//! Avatar rendering complexity ("ARC") and the too-complex limit, ported from
//! the Second Life / Firestorm viewer sources (originally LGPL 2.1, Copyright (C)
//! 2010-2024 Linden Research, Inc. and the Firestorm project):
//! LLVOAvatar::calculateUpdateRenderComplexity /
//! accountRenderComplexityForObject, LLVOVolume::getRenderCost /
//! getTextureCost, LLMeshCostData::init / getRadiusWeightedTris and
//! LLVOAvatar::isTooComplex. As the source notes, the calculation must stay
//! the same in every viewer since it limits rendering.
//!
//! Also LLViewerObject::recursiveGetScaledSurfaceArea (the attachment surface
//! area limit, RenderAutoMuteSurfaceAreaLimit), RenderAvatarComplexityMode
//! (friends) and the per-avatar exceptions of LLRenderMuteList.

use std::collections::{HashMap, HashSet};

use aurora_prim::te::TextureFace;
use uuid::Uuid;

use super::{AlphaKind, Pass, Scene, avatar, classify};
use crate::world::World;

const COMPLEXITY_BODY_PART_COST: u32 = 200;
const MAX_ATTACHMENT_COMPLEXITY: f32 = 1.0e6;
const ARC_PARTICLE_COST: f32 = 1.0;
const ARC_PARTICLE_MAX: f32 = 2048.0;
const ARC_LIGHT_COST: f32 = 500.0;
const ARC_MEDIA_FACE_COST: f32 = 1500.0;
const ARC_GLOW_MULT: f32 = 1.5;
const ARC_BUMP_MULT: f32 = 1.25;
/// Frames the periodic complexity refresh is spread over.
const COMPLEXITY_SLICES: usize = 16;
const ARC_FLEXI_MULT: f32 = 5.0;
const ARC_SHINY_MULT: f32 = 1.6;
const ARC_WEIGHTED_MESH: f32 = 1.2;
const ARC_ANIM_TEX_COST: f32 = 4.0;
const ARC_ALPHA_COST: f32 = 4.0;
const ARC_TEXTURE_COST: f32 = 16.0;

/// RenderAvatarMaxComplexity slider (LLAvatarComplexityControls): positions
/// 1..=100 map exponentially onto 20 000..350 000, 101 is "no limit" (0).
pub mod slider {
    pub const OFF: u32 = 101;
    const MIN_LIMIT: f32 = 20_000.0;
    const MAX_LIMIT: f32 = 350_000.0;
    const MIN_INDIRECT: f32 = 1.0;
    const MAX_INDIRECT: f32 = (OFF - 1) as f32;

    fn scale() -> f32 {
        (MAX_LIMIT.ln() - MIN_LIMIT.ln()) / (MAX_INDIRECT - MIN_INDIRECT)
    }

    /// updateMax: slider position -> RenderAvatarMaxComplexity.
    pub fn to_limit(pos: u32) -> u32 {
        if pos >= OFF {
            return 0;
        }
        (MIN_LIMIT.ln() + scale() * (pos.max(1) as f32 - MIN_INDIRECT)).exp().round() as u32
    }

    /// setIndirectMaxArc: RenderAvatarMaxComplexity -> slider position.
    pub fn from_limit(limit: u32) -> u32 {
        if limit == 0 {
            return OFF;
        }
        (((limit as f32).ln() - MIN_LIMIT.ln()) / scale())
            .round()
            .clamp(0.0, MAX_INDIRECT - 1.0) as u32
            + 1
    }
}

/// LLMeshCostData::init + getRadiusWeightedTris: triangles estimated from the
/// byte size of each LOD (MeshMetaDataDiscount 384, MeshMinimumByteSize 16,
/// MeshBytesPerTriangle 16), weighted by the area over which each LOD shows.
pub fn radius_weighted_tris(lod_bytes: [u32; 4], radius: f32) -> f32 {
    let high = lod_bytes[3];
    let med = if lod_bytes[2] == 0 { high } else { lod_bytes[2] };
    let low = if lod_bytes[1] == 0 { med } else { lod_bytes[1] };
    let lowest = if lod_bytes[0] == 0 { low } else { lod_bytes[0] };
    let tris = |b: u32| (b as f32 - 384.0).max(16.0) / 16.0;
    let (t_lowest, t_low, t_mid, t_high) = (tris(lowest), tris(low), tris(med), tris(high));
    let max_distance = 512.0f32;
    let dlowest = (radius / 0.03).min(max_distance);
    let dlow = (radius / 0.06).min(max_distance);
    let dmid = (radius / 0.24).min(max_distance);
    let max_area = 102_944.0f32;
    let min_area = 1.0f32;
    let pi = std::f32::consts::PI;
    let mut high_area = (pi * dmid * dmid).min(max_area);
    let mut mid_area = (pi * dlow * dlow).min(max_area);
    let mut low_area = (pi * dlowest * dlowest).min(max_area);
    let mut lowest_area = max_area;
    lowest_area -= low_area;
    low_area -= mid_area;
    mid_area -= high_area;
    high_area = high_area.clamp(min_area, max_area);
    mid_area = mid_area.clamp(min_area, max_area);
    low_area = low_area.clamp(min_area, max_area);
    lowest_area = lowest_area.clamp(min_area, max_area);
    let total = high_area + mid_area + low_area + lowest_area;
    (t_high * high_area + t_mid * mid_area + t_low * low_area + t_lowest * lowest_area) / total
}

/// Shapes kept by `Scene::prim_triangle_counts` (then it starts over).
const PRIM_TRIANGLES_CACHED: usize = 16384;

/// What decides which avatars show as grey silhouettes.
pub struct Rules<'a> {
    /// RenderAvatarMaxComplexity (0 = no limit).
    pub max: u32,
    /// RenderAutoMuteSurfaceAreaLimit (m², 0 = none).
    pub max_area: f32,
    /// RenderAvatarComplexityMode: 0 everyone, 1 friends always full, 2 only friends.
    pub mode: u8,
    /// Avatar id -> 1 never render fully, 2 always render fully,
    /// 3 blocked (silhouette whatever the other rules, FIRE-11783).
    pub exceptions: &'a HashMap<Uuid, u8>,
    pub friends: &'a HashSet<Uuid>,
}

impl Rules<'_> {
    /// Changes when any rule does (recompute at once).
    fn key(&self) -> u64 {
        let mut h = self.max as u64 ^ ((self.max_area.to_bits() as u64) << 20) ^ ((self.mode as u64) << 60);
        for (id, m) in self.exceptions {
            h = h.wrapping_add((id.as_u128() as u64).wrapping_mul(*m as u64 + 7));
        }
        for id in self.friends {
            h = h.wrapping_add((id.as_u128() >> 64) as u64);
        }
        h
    }

    /// LLVOAvatar::isTooComplex (+ the "do not render" exception).
    pub fn too_complex(&self, id: &Uuid, own: bool, complexity: u32, area: f32) -> bool {
        let exception = self.exceptions.get(id).copied().unwrap_or(0);
        let friend = self.friends.contains(id);
        if exception == 3 && !own {
            return true;
        }
        if own || exception == 2 || (friend && self.mode >= 1) {
            return false;
        }
        if exception == 1 || self.mode == 2 {
            return true;
        }
        // an unlimited complexity disables the area limit too (LL)
        self.max > 0 && (complexity > self.max || (self.max_area > 0.0 && area > self.max_area))
    }
}

/// LLVOVolume::getTextureCost.
pub fn texture_cost(size: Option<(u32, u32)>) -> u32 {
    let (w, h) = size.unwrap_or((0, 0));
    256 + (ARC_TEXTURE_COST * (h as f32 / 128.0 + w as f32 / 128.0)) as u32
}

impl Scene {
    /// `lod_triangle_counts` of a prim shape, kept per shape: it builds the
    /// path and the profile of the four LODs (1–3 µs), and an avatar wearing
    /// a few hundred prims (flexi hair, old jewelry) made its refresh take
    /// 1–2 ms of a frame.
    fn prim_triangle_counts(&self, volume: &aurora_prim::VolumeParams) -> [u32; 4] {
        let key = volume.cache_key();
        let mut cache = self.prim_triangles.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(counts) = cache.get(&key) {
            return *counts;
        }
        if cache.len() >= PRIM_TRIANGLES_CACHED {
            cache.clear();
        }
        let counts = aurora_prim::lod_triangle_counts(volume);
        cache.insert(key, counts);
        counts
    }

    /// LLVOVolume::getRenderCost of one prim; its textures go into `textures`.
    fn prim_render_cost(&self, world: &World, idx: usize, textures: &mut HashSet<Uuid>) -> f32 {
        let Some(o) = world.objects.get(idx) else {
            return 0.0;
        };
        let Some(g) = self.gpu.get(idx) else {
            return 0.0;
        };
        let radius = o.scale.length() * 0.5;
        let mut weighted_mesh = false;
        let mut num_triangles = if o.volume.is_mesh() {
            let Some(meta) = o.volume.sculpt.and_then(|s| self.meshes.meta(&s.texture)) else {
                // mesh not loaded yet: no cost (LL returns 0)
                return 0.0;
            };
            weighted_mesh = meta.skin.is_some();
            radius_weighted_tris(meta.lod_bytes, radius)
        } else if o.volume.sculpt.is_some() {
            // sculpts: getCostData asks the mesh repository for the sculpt
            // map's id and finds nothing, so they count the minimum below
            0.0
        } else {
            // prims: each LOD's triangles estimated from the shape
            // (getLoDTriangleCounts), as LOD sizes of 10 bytes per triangle
            let counts = self.prim_triangle_counts(&o.volume);
            radius_weighted_tris(counts.map(|c| c.saturating_mul(10)), radius)
        };
        if num_triangles <= 0.0 {
            num_triangles = 4.0;
        }
        if let Some(s) = o.volume.sculpt.filter(|_| !o.volume.is_mesh()) {
            textures.insert(s.texture);
        }
        let te = o.te.as_ref();
        let alpha = &self.textures.alpha_by_slot;
        let (mut alpha_pool, mut bump, mut shiny, mut glow, mut media) = (false, false, false, false, 0u32);
        for (fi, f) in g.faces.iter().enumerate() {
            let tf: TextureFace = te.map(|t| *t.face(fi)).unwrap_or_default();
            textures.insert(tf.texture);
            let a = alpha.get(f.base_slot as usize).copied().unwrap_or(AlphaKind::Opaque);
            if classify(f, a) == Pass::Blend || tf.color[3] < 0.999 {
                alpha_pool = true;
            }
            bump |= tf.bump() != 0;
            shiny |= tf.shiny() != 0;
            glow |= tf.glow > 0.0;
            if tf.media_flags & 1 != 0 {
                media += 1;
            }
        }
        let mut shame = (num_triangles * 5.0).max(2.0);
        if o.tex_anim.is_some() {
            shame *= ARC_ANIM_TEX_COST;
        }
        if alpha_pool {
            shame *= ARC_ALPHA_COST;
        }
        if glow {
            shame *= ARC_GLOW_MULT;
        }
        if bump {
            shame *= ARC_BUMP_MULT;
        }
        if shiny {
            shame *= ARC_SHINY_MULT;
        }
        if weighted_mesh {
            shame *= ARC_WEIGHTED_MESH;
        }
        if o.extra.flexible.is_some() {
            shame *= ARC_FLEXI_MULT;
        }
        if let Some(d) = self.particles.source_data(&o.full_id) {
            let n = (d.burst_part_count as f32 * (d.part.max_age / d.burst_rate.max(1e-3)).ceil()).min(ARC_PARTICLE_MAX);
            let size = (d.part.start_scale.x.max(d.part.end_scale.x) + d.part.start_scale.y.max(d.part.end_scale.y)) / 2.0;
            shame += n * size * ARC_PARTICLE_COST;
        }
        if o.extra.light.is_some() {
            shame += ARC_LIGHT_COST;
        }
        shame += media as f32 * ARC_MEDIA_FACE_COST;
        shame
    }

    /// LLVolume::getSurfaceArea (sculpts: their grid's area, else 1) times
    /// the largest dimension: one prim of recursiveGetScaledSurfaceArea.
    fn prim_scaled_area(&self, world: &World, idx: usize) -> f32 {
        let Some(o) = world.objects.get(idx) else {
            return 0.0;
        };
        let area = match self.gpu.get(idx).and_then(|g| g.geom) {
            Some(k @ super::GeomKey::Sculpt { .. }) => self.sculpt_area.get(&k).copied().unwrap_or(1.0),
            _ => 1.0,
        };
        area * o.scale.x.max(o.scale.y).max(o.scale.z)
    }

    /// LLVOAvatar::calculateUpdateRenderComplexity for the avatar at `idx`,
    /// with the attachments' surface area (m²).
    pub fn avatar_complexity(&self, world: &World, idx: usize) -> (u32, f32) {
        let Some(av) = world.objects.get(idx) else {
            return (0, 0.0);
        };
        let mut cost = 0u32;
        let mut area = 0.0f32;
        // body parts: a defined, visible bake costs 200
        for (te, name, _) in avatar::BAKES {
            if let Some(t) = world.bake_texture(idx, te) {
                let skirt_missing = name == "skirt" && t == avatar::IMG_DEFAULT_AVATAR;
                if !t.is_nil() && t != avatar::IMG_INVISIBLE && !skirt_missing {
                    cost += COMPLEXITY_BODY_PART_COST;
                }
            }
        }
        for &att in world.objects.children_of(&av.key) {
            let Some(root) = world.objects.get(att) else {
                continue;
            };
            // HUD attachments (points 31..=38) do not count
            if (31..=38).contains(&root.attachment_point()) {
                continue;
            }
            let mut textures = HashSet::new();
            let mut total = self.prim_render_cost(world, att, &mut textures);
            area += self.prim_scaled_area(world, att);
            for &c in world.objects.children_of(&root.key) {
                total += self.prim_render_cost(world, c, &mut textures);
                area += self.prim_scaled_area(world, c);
            }
            for t in &textures {
                total += texture_cost(self.textures.full_size(t)) as f32;
            }
            cost += total.clamp(0.0, MAX_ATTACHMENT_COMPLEXITY) as u32;
        }
        (cost, area)
    }

    /// Refresh every avatar's complexity (every 2 s, at once when a rule
    /// changes) and the set of avatars shown as grey silhouettes
    /// (LLVOAvatar::isTooComplex).
    ///
    /// The periodic refresh is spread over [`COMPLEXITY_SLICES`] frames: with
    /// a crowd, walking every attachment prim of every avatar in one frame
    /// made a slow frame every 2 s. A rule change still refreshes them all
    /// at once (the silhouettes follow the setting immediately).
    pub fn update_complexity(&mut self, world: &World, rules: &Rules) {
        let key = rules.key();
        let rules_changed = self.complexity_rules != key;
        let due = self.complexity_at.is_none_or(|t| t.elapsed().as_secs_f32() >= 2.0);
        if rules_changed || (due && self.complexity_round.is_empty()) {
            self.complexity_at = Some(std::time::Instant::now());
            self.complexity_rules = key;
            self.complexity_max = rules.max;
            // the avatars of this round; those gone since the last one are dropped
            let round: Vec<(usize, Uuid)> = world
                .objects
                .iter()
                .filter(|(_, o)| o.is_avatar())
                .map(|(idx, o)| (idx, o.full_id))
                .collect();
            let ids: HashSet<Uuid> = round.iter().map(|&(_, id)| id).collect();
            let idxs: HashSet<usize> = round.iter().map(|&(idx, _)| idx).collect();
            self.avatar_complexity.retain(|id, _| ids.contains(id));
            self.avatar_area.retain(|id, _| ids.contains(id));
            self.too_complex.retain(|idx| idxs.contains(idx));
            self.complexity_round = round;
            // pop() takes from the end: keep the slab order
            self.complexity_round.reverse();
            self.complexity_slice = self.complexity_round.len().div_ceil(COMPLEXITY_SLICES);
        }
        let n = if rules_changed {
            self.complexity_round.len()
        } else {
            self.complexity_slice.min(self.complexity_round.len())
        };
        for _ in 0..n {
            let Some((idx, id)) = self.complexity_round.pop() else {
                break;
            };
            // the slot may hold another object since the round started
            if world.objects.get(idx).is_none_or(|o| o.full_id != id) {
                continue;
            }
            let (c, area) = self.avatar_complexity(world, idx);
            self.avatar_complexity.insert(id, c);
            self.avatar_area.insert(id, area);
            if rules.too_complex(&id, id == world.agent_id, c, area) {
                self.too_complex.insert(idx);
            } else {
                self.too_complex.remove(&idx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_matches_firestorm() {
        assert_eq!(slider::to_limit(slider::OFF), 0);
        assert_eq!(slider::to_limit(1), 20_000);
        assert_eq!(slider::to_limit(100), 350_000);
        assert_eq!(slider::from_limit(0), slider::OFF);
        assert_eq!(slider::from_limit(350_000), 100);
        assert_eq!(slider::from_limit(20_000), 1);
        for p in 1..=100 {
            assert_eq!(slider::from_limit(slider::to_limit(p)), p);
        }
    }

    #[test]
    fn prim_triangle_counts_are_kept_per_shape() {
        let world = World::new(std::sync::Arc::new(avatar::AvatarLibrary::load()));
        let scene = Scene::new(std::path::PathBuf::new(), world.avatar_lib.clone());
        let shape = |hollow: u16| {
            aurora_prim::RawShape {
                path_curve: aurora_prim::params::LL_PCODE_PATH_CIRCLE,
                profile_curve: aurora_prim::params::LL_PCODE_PROFILE_CIRCLE,
                path_scale_x: 100,
                path_scale_y: 50,
                profile_hollow: hollow,
                ..Default::default()
            }
            .to_params()
        };
        for hollow in [0, 20_000, 0, 20_000] {
            let v = shape(hollow);
            assert_eq!(scene.prim_triangle_counts(&v), aurora_prim::lod_triangle_counts(&v));
        }
        assert_eq!(scene.prim_triangles.lock().expect("cache").len(), 2);
    }

    #[test]
    fn periodic_refresh_is_spread_over_frames() {
        let mut world = World::new(std::sync::Arc::new(avatar::AvatarLibrary::load()));
        for ev in crate::demo::events().into_iter().chain(crate::demo::crowd_events(40, false)) {
            world.apply(ev);
        }
        let mut scene = Scene::new(std::path::PathBuf::new(), world.avatar_lib.clone());
        let (exceptions, friends) = (HashMap::new(), HashSet::new());
        let rules = Rules {
            max: 1,
            max_area: 0.0,
            mode: 0,
            exceptions: &exceptions,
            friends: &friends,
        };
        let avatars = world.objects.iter().filter(|(_, o)| o.is_avatar()).count();
        assert!(avatars >= 40);
        // first call (new rules): every avatar at once
        scene.update_complexity(&world, &rules);
        assert_eq!(scene.avatar_complexity.len(), avatars);
        assert!(scene.complexity_round.is_empty());
        let silhouettes = scene.too_complex.clone();
        assert!(!silhouettes.is_empty());
        // nothing until the next round is due
        scene.avatar_complexity.clear();
        scene.update_complexity(&world, &rules);
        assert!(scene.avatar_complexity.is_empty());
        // then a slice per frame
        scene.complexity_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(3));
        let slice = avatars.div_ceil(COMPLEXITY_SLICES);
        scene.update_complexity(&world, &rules);
        assert_eq!(scene.avatar_complexity.len(), slice);
        for _ in 1..COMPLEXITY_SLICES {
            scene.update_complexity(&world, &rules);
        }
        assert_eq!(scene.avatar_complexity.len(), avatars);
        assert!(scene.complexity_round.is_empty());
        assert_eq!(scene.too_complex, silhouettes);
    }

    #[test]
    fn weighted_tris_of_a_flat_mesh() {
        // same size for every LOD: the weighted average is that count
        let t = radius_weighted_tris([16_384; 4], 1.0);
        assert!((t - (16_384.0 - 384.0) / 16.0).abs() < 0.5, "{t}");
        // a missing lower LOD falls back on the next higher one
        assert!((radius_weighted_tris([0, 0, 0, 16_384], 1.0) - t).abs() < 0.5);
    }

    #[test]
    fn too_complex_rules() {
        let a = Uuid::from_u128(1);
        let friend = Uuid::from_u128(2);
        let mut exceptions = HashMap::new();
        let friends: HashSet<Uuid> = [friend].into_iter().collect();
        let rules = |exceptions: &HashMap<Uuid, u8>, mode: u8| {
            Rules {
                max: 100_000,
                max_area: 1000.0,
                mode,
                exceptions: &exceptions.clone(),
                friends: &friends.clone(),
            }
            .too_complex(&a, false, 150_000, 10.0)
        };
        assert!(rules(&exceptions, 0));
        exceptions.insert(a, 2);
        assert!(!rules(&exceptions, 0), "always render");
        exceptions.insert(a, 1);
        let r = Rules {
            max: 0,
            max_area: 0.0,
            mode: 0,
            exceptions: &exceptions,
            friends: &friends,
        };
        assert!(r.too_complex(&a, false, 1, 0.0), "never render, even without a limit");
        let none = HashMap::new();
        let r = Rules {
            max: 100_000,
            max_area: 1000.0,
            mode: 0,
            exceptions: &none,
            friends: &friends,
        };
        assert!(r.too_complex(&a, false, 10, 2000.0), "area over the limit");
        assert!(!r.too_complex(&a, true, 10_000_000, 0.0), "never ourselves");
        let r = Rules {
            max: 0,
            max_area: 1000.0,
            mode: 0,
            exceptions: &none,
            friends: &friends,
        };
        assert!(!r.too_complex(&a, false, 10, 2000.0), "no complexity limit: no area limit");
        let r = Rules {
            max: 100_000,
            max_area: 1000.0,
            mode: 2,
            exceptions: &none,
            friends: &friends,
        };
        assert!(r.too_complex(&a, false, 1, 0.0), "friends only");
        assert!(!r.too_complex(&friend, false, 10_000_000, 0.0), "friend shown");
        let r = Rules {
            max: 100_000,
            max_area: 1000.0,
            mode: 1,
            exceptions: &none,
            friends: &friends,
        };
        assert!(!r.too_complex(&friend, false, 10_000_000, 0.0), "friends always full");
    }

    #[test]
    fn texture_cost_of_a_1024_texture() {
        assert_eq!(texture_cost(Some((1024, 1024))), 256 + 256);
        assert_eq!(texture_cost(None), 256);
    }
}

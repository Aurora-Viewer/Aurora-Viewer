//! Per-frame morphs of the system body: lip sync's Aah / Ooh
//! (LLVoiceVisualizer::lipSyncOohAah / LLVOAvatar::idleUpdateLipSync,
//! indra/newview/llvoicevisualizer.cpp, llvoavatar.cpp, originally LGPL 2.1),
//! the hidden `*_Driven` parameters of the body physics (body_physics.rs),
//! the facial expressions and the hand poses (default_motions.rs), which the
//! Legacy Viewer applies to the base meshes as it does any morph
//! (LLPolyMorphTarget::apply).
//! Static morph deltas live once in the GPU palette buffer, sparse: per
//! vertex of a part, where its entries start and how many there are; per
//! entry, the position delta and the weight column of one morph, then its
//! normal delta. Each avatar only uploads its weights, one column per live
//! parameter, in the matrices after its anchors; no per-frame mesh rebuilds
//! or growing morph cache.

use super::{anim, avatar::AvatarLibrary};
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

/// First weight matrix of a palette (after the anchors).
pub const WEIGHTS: usize = aurora_render::LIVE_MORPH_WEIGHTS as usize;
/// Weight matrices per palette: sixteen columns each, a column per live
/// parameter from the first, one per body part from the last (not zero
/// while the part moves).
pub const WEIGHT_MATRICES: usize = 4;
const COLUMNS: usize = 16 * WEIGHT_MATRICES;
type Matrix = [[f32; 4]; 4];
pub type Weights = [Matrix; WEIGHT_MATRICES];

/// Aah and Ooh.
const LIP_PARAMS: [i32; 2] = [70, 71];
/// NORMAL_SOFTEN_FACTOR (llpolymorph.cpp).
const NORMAL_SOFTEN_FACTOR: f32 = 0.65;

pub struct LiveMorphs {
    pub matrices: Vec<Matrix>,
    /// Per body part: (vector of its first vertex in the bank, vertices,
    /// the column that says it moves).
    pub parts: Vec<Option<(u32, u32, u32)>>,
    pub slots: u32,
    pub uploaded: bool,
    /// The parameter of each weight column.
    columns: Vec<i32>,
    /// Per body part with a bank: the column that says it moves, and the
    /// columns of its morphs.
    moving: Vec<(usize, Vec<usize>)>,
    levels: HashMap<Uuid, (f32, Instant)>,
}

impl LiveMorphs {
    /// `live`: the other parameters animated every frame (the body
    /// physics' driven ones, the expressions, the hand poses).
    pub fn new(lib: &AvatarLibrary, live: &[i32]) -> Self {
        let mut columns: Vec<i32> = Vec::new();
        for p in LIP_PARAMS.iter().chain(live) {
            if !columns.contains(p) && lib.parts.iter().any(|part| part.morphs.iter().any(|m| m.param == *p)) {
                columns.push(*p);
            }
        }
        let room = COLUMNS - lib.parts.len();
        if columns.len() > room {
            log::warn!("system body: {} live morphs, only the first {room} move", columns.len());
            columns.truncate(room);
        }
        // the bank, as vectors: for each part its vertex table, then its
        // entries
        let mut bank: Vec<[f32; 4]> = Vec::new();
        let mut parts = Vec::new();
        let mut moving = Vec::new();
        for part in &lib.parts {
            let mut entries: Vec<Vec<[[f32; 4]; 2]>> = vec![Vec::new(); part.vertices.len()];
            let mut used = Vec::new();
            for m in &part.morphs {
                let Some(column) = columns.iter().position(|p| *p == m.param) else {
                    continue;
                };
                used.push(column);
                for (i, &vertex) in m.vertices.iter().enumerate() {
                    let (Some(list), Some(dp), Some(dn)) = (entries.get_mut(vertex as usize), m.d_positions.get(i), m.d_normals.get(i))
                    else {
                        continue;
                    };
                    let dn = *dn * NORMAL_SOFTEN_FACTOR;
                    list.push([[dp.x, dp.y, dp.z, column as f32], [dn.x, dn.y, dn.z, 0.0]]);
                }
            }
            if entries.iter().all(Vec::is_empty) {
                parts.push(None);
                continue;
            }
            let table = bank.len();
            let mut next = table + entries.len();
            for list in &entries {
                bank.push([next as f32, list.len() as f32, 0.0, 0.0]);
                next += 2 * list.len();
            }
            for list in entries {
                bank.extend(list.into_iter().flatten());
            }
            let flag = COLUMNS - 1 - moving.len();
            parts.push(Some((table as u32, part.vertices.len() as u32, flag as u32)));
            moving.push((flag, used));
        }
        let slots = bank.len().div_ceil(4 * anim::PALETTE_JOINTS) as u32;
        let mut matrices = vec![[[0.0; 4]; 4]; slots as usize * anim::PALETTE_JOINTS];
        for (i, v) in bank.into_iter().enumerate() {
            matrices[i / 4][i % 4] = v;
        }
        Self {
            matrices,
            parts,
            slots,
            uploaded: false,
            columns,
            moving,
            levels: HashMap::new(),
        }
    }

    pub fn set_levels(&mut self, levels: &HashMap<Uuid, (f32, bool)>, enabled: bool, now: Instant) {
        self.levels
            .retain(|id, _| enabled && levels.get(id).is_some_and(|(_, speaking)| *speaking));
        if enabled {
            for (&id, &(level, speaking)) in levels {
                if speaking && level.is_finite() {
                    self.levels
                        .entry(id)
                        .and_modify(|e| e.0 = level.clamp(0.0, 1.0))
                        .or_insert((level.clamp(0.0, 1.0), now));
                }
            }
        }
    }

    /// The weights of an avatar: its lip sync now, and the weights of the
    /// live parameters (`live`: parameter, weight as a difference from the
    /// weight baked into its shape; 0 when absent).
    pub fn weights(&self, id: &Uuid, now: Instant, live: &[(i32, f32)]) -> Weights {
        let mut lips = [0.0; 2];
        if let Some(&(level, start)) = self.levels.get(id) {
            let frame = (now.saturating_duration_since(start).as_secs_f64() * 24.0) as usize;
            lips[0] = amount(b"257998776531013446642343", b"0000123456789", level, frame);
            lips[1] = amount(b"1247898743223344444443200000", b"0012345566778899", level, frame);
        }
        let mut result = [[[0.0; 4]; 4]; WEIGHT_MATRICES];
        for (k, &p) in self.columns.iter().enumerate() {
            let w = match p {
                p if p == LIP_PARAMS[0] => lips[0],
                p if p == LIP_PARAMS[1] => lips[1],
                p => live.iter().find(|l| l.0 == p).map_or(0.0, |l| l.1),
            };
            result[k / 16][k / 4 % 4][k % 4] = if w.is_finite() { w } else { 0.0 };
        }
        // a part at rest skips its bank on the GPU
        for (flag, used) in &self.moving {
            if used.iter().any(|&k| result[k / 16][k / 4 % 4][k % 4] != 0.0) {
                result[flag / 16][flag / 4 % 4][flag % 4] = 1.0;
            }
        }
        result
    }
}

fn amount(cycle: &[u8], transfer: &[u8], level: f32, frame: usize) -> f32 {
    let i = ((transfer.len() as f32 * level) as usize).min(transfer.len() - 1);
    f32::from(transfer[i] - b'0') * f32::from(cycle[frame % cycle.len()] - b'0') * 0.11 * 0.11
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::default_motions;

    fn all_live() -> Vec<i32> {
        crate::scene::body_physics::driven_ids()
            .chain(default_motions::emote_params())
            .chain(default_motions::HAND_PARAMS[1..].iter().copied())
            .collect()
    }

    /// The weight of `param` in `w`.
    fn weight(bank: &LiveMorphs, w: &Weights, param: i32) -> f32 {
        let k = bank.columns.iter().position(|p| *p == param).expect("column");
        w[k / 16][k / 4 % 4][k % 4]
    }

    #[test]
    fn mouth_closes_when_speaking_stops_or_preference_is_disabled() {
        let lib = AvatarLibrary::load();
        let mut lips = LiveMorphs::new(&lib, &[]);
        assert!(lips.parts.iter().any(Option::is_some));
        let id = Uuid::from_u128(42);
        let now = Instant::now();
        let mouth = |lips: &LiveMorphs| {
            let w = lips.weights(&id, now, &[]);
            LIP_PARAMS.iter().map(|p| weight(lips, &w, *p)).sum::<f32>()
        };
        lips.set_levels(&HashMap::from([(id, (0.8, true))]), true, now);
        assert!(mouth(&lips) > 0.0);
        lips.set_levels(&HashMap::from([(id, (0.8, false))]), true, now);
        assert_eq!(mouth(&lips), 0.0);
        lips.set_levels(&HashMap::from([(id, (0.8, true))]), false, now);
        assert_eq!(mouth(&lips), 0.0);
    }

    #[test]
    fn every_live_morph_of_the_system_body_gets_its_weight() {
        let lib = AvatarLibrary::load();
        let live = all_live();
        let bank = LiveMorphs::new(&lib, &live);
        // all of them fit, each once
        for p in LIP_PARAMS.iter().chain(&live) {
            if lib.parts.iter().any(|part| part.morphs.iter().any(|m| m.param == *p)) {
                assert_eq!(bank.columns.iter().filter(|c| *c == p).count(), 1, "{p}");
            }
        }
        assert!(bank.columns.len() > 40, "{}", bank.columns.len());
        let id = Uuid::from_u128(7);
        let w = bank.weights(&id, Instant::now(), &[(1200, 1.5), (318, 0.25), (101, 1.0)]);
        assert_eq!(weight(&bank, &w, 1200), 1.5);
        assert_eq!(weight(&bank, &w, 318), 0.25);
        assert_eq!(weight(&bank, &w, 101), 1.0);
        let at = |w: &Weights, k: usize| w[k / 16][k / 4 % 4][k % 4];
        assert_eq!((0..bank.columns.len()).filter(|&k| at(&w, k) != 0.0).count(), 3);
        // the parts that move say so; at rest, none does
        let part = |name: &str| {
            let (_, _, flag) = bank.parts[lib.parts.iter().position(|p| p.name == name).expect("part")].expect("bank");
            flag as usize
        };
        let w = bank.weights(&id, Instant::now(), &[(318, 0.25)]);
        assert_eq!(
            (at(&w, part("head")), at(&w, part("eyelashes")), at(&w, part("upper"))),
            (1.0, 1.0, 0.0)
        );
        let rest = bank.weights(&id, Instant::now(), &[]);
        assert!(rest.iter().flatten().flatten().all(|v| *v == 0.0));
    }

    #[test]
    fn the_sparse_bank_holds_every_delta_of_a_vertex_once() {
        let lib = AvatarLibrary::load();
        let live = all_live();
        let bank = LiveMorphs::new(&lib, &live);
        let vector = |i: usize| bank.matrices[i / 4][i % 4];
        for (pi, part) in lib.parts.iter().enumerate() {
            let moved = part.morphs.iter().any(|m| bank.columns.contains(&m.param));
            let Some((table, count, _)) = bank.parts[pi] else {
                assert!(!moved, "{} has live morphs but no bank", part.name);
                continue;
            };
            assert_eq!(count as usize, part.vertices.len());
            // every delta of every live morph is found under its vertex
            for m in part.morphs.iter().filter(|m| bank.columns.contains(&m.param)) {
                let column = bank.columns.iter().position(|p| *p == m.param).expect("column") as f32;
                for (i, &v) in m.vertices.iter().enumerate() {
                    let [first, n, ..] = vector(table as usize + v as usize);
                    let found = (0..n as usize)
                        .map(|e| vector(first as usize + 2 * e))
                        .any(|d| d[3] == column && (glam::Vec3::new(d[0], d[1], d[2]) - m.d_positions[i]).length() < 1e-6);
                    assert!(found, "{} param {} vertex {v}", part.name, m.param);
                }
            }
        }
        // the hands sit on the upper body, the expressions on the head and
        // the eyelashes
        let banked = |name: &str| bank.parts[lib.parts.iter().position(|p| p.name == name).expect("part")].is_some();
        for name in ["head", "upper", "eyelashes"] {
            assert!(banked(name), "{name}");
        }
    }
}

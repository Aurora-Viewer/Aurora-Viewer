//! Per-frame morphs of the system body: lip sync's Aah / Ooh
//! (LLVoiceVisualizer::lipSyncOohAah / LLVOAvatar::idleUpdateLipSync,
//! indra/newview/llvoicevisualizer.cpp, llvoavatar.cpp, originally LGPL 2.1)
//! and the hidden `*_Driven` parameters of the body physics
//! (body_physics.rs), which the Legacy Viewer applies to the base meshes as
//! it does any morph (LLPolyMorphTarget::apply).
//! Static morph deltas live once in the GPU palette buffer: two matrices per
//! vertex of a part, the position and the normal deltas of up to four morphs
//! in their columns. Each avatar only uploads one row of four weights per
//! part, in the matrix after its anchors; no per-frame mesh rebuilds or
//! growing morph cache.

use super::{anim, avatar::AvatarLibrary};
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

pub const WEIGHTS: usize = 208;
type Matrix = [[f32; 4]; 4];

/// Aah and Ooh.
const LIP_PARAMS: [i32; 2] = [70, 71];
/// Morphs a part's bank holds: the columns of its matrices.
const COLUMNS: usize = 4;
/// Parts with a bank: the rows of the weight matrix.
const ROWS: usize = 4;
/// NORMAL_SOFTEN_FACTOR (llpolymorph.cpp).
const NORMAL_SOFTEN_FACTOR: f32 = 0.65;

pub struct LipSync {
    pub matrices: Vec<Matrix>,
    /// Per body part: (first bank matrix, vertices, weight row).
    pub parts: Vec<Option<(u32, u32, u32)>>,
    pub slots: u32,
    pub uploaded: bool,
    /// The parameter of each column of each weight row.
    rows: Vec<[Option<i32>; COLUMNS]>,
    levels: HashMap<Uuid, (f32, Instant)>,
}

impl LipSync {
    /// `live`: the other parameters animated every frame (the body
    /// physics' driven ones).
    pub fn new(lib: &AvatarLibrary, live: &[i32]) -> Self {
        let mut matrices = Vec::new();
        let mut parts = Vec::new();
        let mut rows = Vec::new();
        for part in &lib.parts {
            let mut columns = [None; COLUMNS];
            let mut used = 0;
            for m in &part.morphs {
                if used < COLUMNS && (LIP_PARAMS.contains(&m.param) || live.contains(&m.param)) && !columns.contains(&Some(m.param)) {
                    columns[used] = Some(m.param);
                    used += 1;
                }
            }
            if used == 0 || rows.len() == ROWS {
                if used > 0 {
                    log::warn!("system body part {}: no weight row left for its live morphs", part.name);
                }
                parts.push(None);
                continue;
            }
            let mut deltas = vec![[[0.0; 4]; 4]; 2 * part.vertices.len()];
            for (column, param) in columns.iter().enumerate() {
                let Some(m) = param.and_then(|p| part.morphs.iter().find(|m| m.param == p)) else {
                    continue;
                };
                for (i, &vertex) in m.vertices.iter().enumerate() {
                    let at = 2 * vertex as usize;
                    if at + 1 < deltas.len() {
                        deltas[at][column][..3].copy_from_slice(&m.d_positions[i].to_array());
                        deltas[at + 1][column][..3].copy_from_slice(&(m.d_normals[i] * NORMAL_SOFTEN_FACTOR).to_array());
                    }
                }
            }
            parts.push(Some((matrices.len() as u32, part.vertices.len() as u32, rows.len() as u32)));
            rows.push(columns);
            matrices.extend(deltas);
        }
        let slots = matrices.len().div_ceil(anim::PALETTE_JOINTS) as u32;
        matrices.resize(slots as usize * anim::PALETTE_JOINTS, [[0.0; 4]; 4]);
        Self {
            matrices,
            parts,
            slots,
            uploaded: false,
            rows,
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

    /// The weight rows of an avatar: its lip sync now, and the weights of
    /// the live parameters (`live`: parameter, weight; 0 when absent).
    pub fn weights(&self, id: &Uuid, now: Instant, live: &[(i32, f32)]) -> Matrix {
        let mut lips = [0.0; 2];
        if let Some(&(level, start)) = self.levels.get(id) {
            let frame = (now.saturating_duration_since(start).as_secs_f64() * 24.0) as usize;
            lips[0] = amount(b"257998776531013446642343", b"0000123456789", level, frame);
            lips[1] = amount(b"1247898743223344444443200000", b"0012345566778899", level, frame);
        }
        let mut result = [[0.0; 4]; 4];
        for (row, columns) in result.iter_mut().zip(&self.rows) {
            for (w, param) in row.iter_mut().zip(columns) {
                *w = match *param {
                    Some(p) if p == LIP_PARAMS[0] => lips[0],
                    Some(p) if p == LIP_PARAMS[1] => lips[1],
                    Some(p) => live.iter().find(|l| l.0 == p).map_or(0.0, |l| l.1),
                    None => 0.0,
                };
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
    #[test]
    fn mouth_closes_when_speaking_stops_or_preference_is_disabled() {
        let lib = AvatarLibrary::load();
        let mut lips = LipSync::new(&lib, &[]);
        assert!(lips.parts.iter().any(Option::is_some));
        let id = Uuid::from_u128(42);
        let now = Instant::now();
        let mouth = |lips: &LipSync| {
            let w = lips.weights(&id, now, &[]);
            lips.rows
                .iter()
                .zip(w)
                .flat_map(|(c, w)| c.iter().zip(w))
                .filter(|(p, _)| p.is_some_and(|p| LIP_PARAMS.contains(&p)))
                .map(|(_, w)| w)
                .sum::<f32>()
        };
        lips.set_levels(&HashMap::from([(id, (0.8, true))]), true, now);
        assert!(mouth(&lips) > 0.0);
        lips.set_levels(&HashMap::from([(id, (0.8, false))]), true, now);
        assert_eq!(mouth(&lips), 0.0);
        lips.set_levels(&HashMap::from([(id, (0.8, true))]), false, now);
        assert_eq!(mouth(&lips), 0.0);
    }

    #[test]
    fn the_body_physics_morphs_of_the_system_body_get_their_weights() {
        let lib = AvatarLibrary::load();
        let live: Vec<i32> = crate::scene::body_physics::driven_ids().collect();
        let bank = LipSync::new(&lib, &live);
        // every driven morph of the base meshes has a column
        for part in &lib.parts {
            for m in part.morphs.iter().filter(|m| live.contains(&m.param)) {
                let (_, _, row) = bank.parts[lib.parts.iter().position(|p| std::ptr::eq(p, part)).expect("part")].expect("bank");
                assert!(bank.rows[row as usize].contains(&Some(m.param)), "{} {}", part.name, m.param);
            }
        }
        let id = Uuid::from_u128(7);
        let w = bank.weights(&id, Instant::now(), &[(1200, 1.5)]);
        let (row, column) = bank
            .rows
            .iter()
            .enumerate()
            .find_map(|(r, c)| c.iter().position(|p| *p == Some(1200)).map(|k| (r, k)))
            .expect("breast bounce column");
        assert_eq!(w[row][column], 1.5);
        assert_eq!(w.iter().flatten().filter(|v| **v != 0.0).count(), 1);
    }
}

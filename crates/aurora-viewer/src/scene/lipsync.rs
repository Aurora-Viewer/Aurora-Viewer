//! LLVoiceVisualizer::lipSyncOohAah / LLVOAvatar::idleUpdateLipSync
//! (indra/newview/llvoicevisualizer.cpp, llvoavatar.cpp, originally LGPL 2.1).
//! Static morph deltas live once in the GPU palette buffer. Each avatar only
//! uploads two weights; no per-frame mesh rebuilds or growing morph cache.

use super::{anim, avatar::AvatarLibrary};
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

pub const WEIGHTS: usize = 208;
type Matrix = [[f32; 4]; 4];

pub struct LipSync {
    pub matrices: Vec<Matrix>,
    pub parts: Vec<Option<(u32, u32)>>,
    pub slots: u32,
    pub uploaded: bool,
    levels: HashMap<Uuid, (f32, Instant)>,
}

impl LipSync {
    pub fn new(lib: &AvatarLibrary) -> Self {
        let mut matrices = Vec::new();
        let mut parts = Vec::new();
        for part in &lib.parts {
            let mut deltas = vec![[[0.0; 4]; 4]; part.vertices.len()];
            let mut found = false;
            for (param, column) in [(70, 0), (71, 2)] {
                if let Some(m) = part.morphs.iter().find(|m| m.param == param) {
                    found = true;
                    for (i, &vertex) in m.vertices.iter().enumerate() {
                        if let Some(delta) = deltas.get_mut(vertex as usize) {
                            delta[column][..3].copy_from_slice(&m.d_positions[i].to_array());
                            delta[column + 1][..3].copy_from_slice(&(m.d_normals[i] * 0.65).to_array());
                        }
                    }
                }
            }
            parts.push(found.then_some((matrices.len() as u32, deltas.len() as u32)));
            if found {
                matrices.extend(deltas);
            }
        }
        let slots = matrices.len().div_ceil(anim::PALETTE_JOINTS) as u32;
        matrices.resize(slots as usize * anim::PALETTE_JOINTS, [[0.0; 4]; 4]);
        Self {
            matrices,
            parts,
            slots,
            uploaded: false,
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

    pub fn weights(&self, id: &Uuid, now: Instant) -> Matrix {
        let mut result = [[0.0; 4]; 4];
        if let Some(&(level, start)) = self.levels.get(id) {
            let frame = (now.saturating_duration_since(start).as_secs_f64() * 24.0) as usize;
            result[0][0] = amount(b"257998776531013446642343", b"0000123456789", level, frame);
            result[0][1] = amount(b"1247898743223344444443200000", b"0012345566778899", level, frame);
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
        let mut lips = LipSync::new(&lib);
        assert!(lips.parts.iter().any(Option::is_some));
        let id = Uuid::from_u128(42);
        let now = Instant::now();
        lips.set_levels(&HashMap::from([(id, (0.8, true))]), true, now);
        assert!(lips.weights(&id, now)[0][0] > 0.0);
        lips.set_levels(&HashMap::from([(id, (0.8, false))]), true, now);
        assert_eq!(lips.weights(&id, now)[0], [0.0; 4]);
        lips.set_levels(&HashMap::from([(id, (0.8, true))]), false, now);
        assert_eq!(lips.weights(&id, now)[0], [0.0; 4]);
    }
}

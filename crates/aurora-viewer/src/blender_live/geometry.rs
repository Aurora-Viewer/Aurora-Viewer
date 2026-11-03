//! Optional packed geometry: the same f32/u8 values as the ordinary JSON fields.
use super::protocol::{Corner, MAX_CORNERS};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackedGeometry {
    vertices: String,
    indices: String,
}

impl PackedGeometry {
    pub(super) fn decode(self) -> Result<(Vec<Corner>, Vec<u32>), String> {
        const STRIDE: usize = 52;
        let invalid = || aurora_i18n::tr!("blender-live-invalid-packed").to_owned();
        if self.vertices.len() > (MAX_CORNERS * STRIDE).div_ceil(3) * 4 || self.indices.len() > (MAX_CORNERS * 3 * 4).div_ceil(3) * 4 {
            return Err(invalid());
        }
        let raw = STANDARD.decode(self.vertices).map_err(|_| invalid())?;
        let indices = STANDARD.decode(self.indices).map_err(|_| invalid())?;
        if !raw.len().is_multiple_of(STRIDE) || !indices.len().is_multiple_of(4) {
            return Err(invalid());
        }
        let vertices = raw
            .as_chunks::<STRIDE>()
            .0
            .iter()
            .map(|record| {
                let float = |offset| f32::from_le_bytes([record[offset], record[offset + 1], record[offset + 2], record[offset + 3]]);
                Corner {
                    position: [float(0), float(4), float(8)],
                    normal: [float(12), float(16), float(20)],
                    uv: [float(24), float(28)],
                    joints: [record[32], record[33], record[34], record[35]],
                    weights: [float(36), float(40), float(44), float(48)],
                }
            })
            .collect();
        let indices = indices
            .as_chunks::<4>()
            .0
            .iter()
            .map(|value| u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
            .collect();
        Ok((vertices, indices))
    }
}

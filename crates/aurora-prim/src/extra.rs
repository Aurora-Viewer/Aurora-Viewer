//! Object "extra parameters" (`ExtraParams` blob), ported from
//! `llprimitive/llprimitive.cpp` (`LL*Params::unpack`).

use crate::params::SculptParams;
use glam::Vec3;
use uuid::Uuid;

pub const PARAMS_FLEXIBLE: u16 = 0x10;
pub const PARAMS_LIGHT: u16 = 0x20;
pub const PARAMS_SCULPT: u16 = 0x30;
pub const PARAMS_LIGHT_IMAGE: u16 = 0x40;
pub const PARAMS_MESH: u16 = 0x60;
pub const PARAMS_EXTENDED_MESH: u16 = 0x70;
pub const PARAMS_RENDER_MATERIAL: u16 = 0x80;
pub const PARAMS_REFLECTION_PROBE: u16 = 0x90;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightParams {
    /// Linear color (LLLightParams::setLinearColor); alpha is intensity.
    pub color: [f32; 4],
    pub radius: f32,
    pub cutoff: f32,
    pub falloff: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlexibleParams {
    pub softness: u8,
    pub tension: f32,
    pub air_friction: f32,
    pub gravity: f32,
    pub wind_sensitivity: f32,
    pub user_force: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightImageParams {
    pub texture: Uuid,
    /// (fov, focus, ambiance)
    pub params: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReflectionProbeParams {
    pub ambiance: f32,
    pub clip_distance: f32,
    pub flags: u8,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExtraParams {
    pub flexible: Option<FlexibleParams>,
    pub light: Option<LightParams>,
    pub sculpt: Option<SculptParams>,
    pub light_image: Option<LightImageParams>,
    pub extended_mesh_flags: Option<u32>,
    /// (face index, GLTF material asset id)
    pub render_materials: Vec<(u8, Uuid)>,
    pub reflection_probe: Option<ReflectionProbeParams>,
}

/// Extended mesh flag: object is an animated mesh (animesh).
pub const EXTENDED_MESH_ANIMATED: u32 = 0x1;

fn f32_at(d: &[u8], o: usize) -> Option<f32> {
    Some(f32::from_le_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

fn uuid_at(d: &[u8], o: usize) -> Option<Uuid> {
    Uuid::from_slice(d.get(o..o + 16)?).ok()
}

fn parse_entry(ep: &mut ExtraParams, ty: u16, d: &[u8]) -> Option<()> {
    match ty {
        PARAMS_FLEXIBLE => {
            let t = *d.first()?;
            let f = *d.get(1)?;
            let g = *d.get(2)?;
            let w = *d.get(3)?;
            let bit1 = (t >> 6) & 2;
            let bit2 = (f >> 7) & 1;
            let user_force = if d.len() >= 16 {
                Vec3::new(f32_at(d, 4)?, f32_at(d, 8)?, f32_at(d, 12)?)
            } else {
                Vec3::ZERO
            };
            ep.flexible = Some(FlexibleParams {
                softness: bit1 | bit2,
                tension: (t & 0x7f) as f32 / 10.0,
                air_friction: (f & 0x7f) as f32 / 10.0,
                gravity: g as f32 / 10.0 - 10.0,
                wind_sensitivity: w as f32 / 10.0,
                user_force,
            });
        }
        PARAMS_LIGHT => {
            let c = d.get(0..4)?;
            ep.light = Some(LightParams {
                color: [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, c[3] as f32 / 255.0],
                radius: f32_at(d, 4)?,
                cutoff: f32_at(d, 8)?,
                falloff: f32_at(d, 12)?,
            });
        }
        // the servers send meshes as PARAMS_MESH with the sculpt layout
        // (LLViewerObject::unpackParameterEntry maps it to PARAMS_SCULPT)
        PARAMS_SCULPT | PARAMS_MESH => {
            ep.sculpt = Some(SculptParams {
                texture: uuid_at(d, 0)?,
                sculpt_type: *d.get(16)?,
            });
        }
        PARAMS_LIGHT_IMAGE => {
            ep.light_image = Some(LightImageParams {
                texture: uuid_at(d, 0)?,
                params: Vec3::new(f32_at(d, 16)?, f32_at(d, 20)?, f32_at(d, 24)?),
            });
        }
        PARAMS_EXTENDED_MESH => {
            ep.extended_mesh_flags = Some(u32::from_le_bytes(d.get(0..4)?.try_into().ok()?));
        }
        PARAMS_RENDER_MATERIAL => {
            let n = *d.first()? as usize;
            let mut v = Vec::with_capacity(n);
            for i in 0..n {
                let o = 1 + i * 17;
                let te = *d.get(o)?;
                v.push((te, uuid_at(d, o + 1)?));
            }
            ep.render_materials = v;
        }
        PARAMS_REFLECTION_PROBE => {
            ep.reflection_probe = Some(ReflectionProbeParams {
                ambiance: f32_at(d, 0)?,
                clip_distance: f32_at(d, 4)?,
                flags: *d.get(8)?,
            });
        }
        _ => {}
    }
    Some(())
}

/// Parse an ExtraParams blob: `count u8, { type u16, size u32, data }*`.
/// Malformed entries are skipped; parsing stops at the first truncation.
pub fn parse_extra_params(data: &[u8]) -> ExtraParams {
    let mut ep = ExtraParams::default();
    let Some(&count) = data.first() else {
        return ep;
    };
    let mut pos = 1usize;
    for _ in 0..count {
        let Some(h) = data.get(pos..pos + 6) else {
            break;
        };
        let ty = u16::from_le_bytes([h[0], h[1]]);
        let size = u32::from_le_bytes([h[2], h[3], h[4], h[5]]) as usize;
        pos += 6;
        let Some(d) = data.get(pos..pos.saturating_add(size)) else {
            break;
        };
        pos += size;
        let _ = parse_entry(&mut ep, ty, d);
    }
    ep
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sculpt_and_light() {
        let mut v = vec![2u8];
        v.extend_from_slice(&PARAMS_SCULPT.to_le_bytes());
        v.extend_from_slice(&17u32.to_le_bytes());
        v.extend_from_slice(&[7u8; 16]);
        v.push(5);
        v.extend_from_slice(&PARAMS_LIGHT.to_le_bytes());
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&[255, 128, 0, 255]);
        v.extend_from_slice(&10f32.to_le_bytes());
        v.extend_from_slice(&0f32.to_le_bytes());
        v.extend_from_slice(&0.75f32.to_le_bytes());
        let ep = parse_extra_params(&v);
        assert!(ep.sculpt.unwrap().is_mesh());
        assert_eq!(ep.light.unwrap().radius, 10.0);
        for n in 0..v.len() {
            let _ = parse_extra_params(&v[..n]);
        }
    }
    #[test]
    fn mesh_params_are_sculpt_params() {
        // what the servers send for a mesh object: PARAMS_MESH (0x60)
        let mut v = vec![1u8];
        v.extend_from_slice(&PARAMS_MESH.to_le_bytes());
        v.extend_from_slice(&17u32.to_le_bytes());
        v.extend_from_slice(&[9u8; 16]);
        v.push(5);
        let ep = parse_extra_params(&v);
        let s = ep.sculpt.expect("mesh params");
        assert!(s.is_mesh());
        assert_eq!(s.texture, Uuid::from_bytes([9; 16]));
    }
}

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

// ---- packing (LL*Params::pack, for ObjectExtraParams)

fn push_f32(v: &mut Vec<u8>, f: f32) {
    v.extend_from_slice(&f.to_le_bytes());
}

/// LLFlexibleObjectData::pack: softness in the top bits of tension / drag,
/// values truncated after `* 10.01` like the C++ casts.
pub fn pack_flexible(f: &FlexibleParams) -> Vec<u8> {
    let bit1 = (f.softness & 2) << 6;
    let bit2 = (f.softness & 1) << 7;
    let byte = |x: f32| (x * 10.01).clamp(0.0, 255.0) as u8;
    let mut v = vec![
        (byte(f.tension) & 0x7f) + bit1,
        (byte(f.air_friction) & 0x7f) + bit2,
        byte(f.gravity + 10.0),
        byte(f.wind_sensitivity),
    ];
    for c in f.user_force.to_array() {
        push_f32(&mut v, c);
    }
    v
}

/// LLLightParams::pack: linear color as bytes (alpha = intensity).
pub fn pack_light(l: &LightParams) -> Vec<u8> {
    let mut v: Vec<u8> = l.color.iter().map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
    push_f32(&mut v, l.radius);
    push_f32(&mut v, l.cutoff);
    push_f32(&mut v, l.falloff);
    v
}

/// LLSculptParams::pack.
pub fn pack_sculpt(s: &SculptParams) -> Vec<u8> {
    let mut v = s.texture.as_bytes().to_vec();
    v.push(s.sculpt_type);
    v
}

/// LLLightImageParams::pack: texture then (fov, focus, ambiance).
pub fn pack_light_image(l: &LightImageParams) -> Vec<u8> {
    let mut v = l.texture.as_bytes().to_vec();
    for c in l.params.to_array() {
        push_f32(&mut v, c);
    }
    v
}

/// LLExtendedMeshParams::pack.
pub fn pack_extended_mesh(flags: u32) -> Vec<u8> {
    flags.to_le_bytes().to_vec()
}

/// LLReflectionProbeParams::pack.
pub fn pack_reflection_probe(r: &ReflectionProbeParams) -> Vec<u8> {
    let mut v = Vec::with_capacity(9);
    push_f32(&mut v, r.ambiance);
    push_f32(&mut v, r.clip_distance);
    v.push(r.flags);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(ty: u16, d: &[u8]) -> ExtraParams {
        let mut v = vec![1u8];
        v.extend_from_slice(&ty.to_le_bytes());
        v.extend_from_slice(&(d.len() as u32).to_le_bytes());
        v.extend_from_slice(d);
        parse_extra_params(&v)
    }

    #[test]
    fn packs_round_trip() {
        let f = FlexibleParams {
            softness: 3,
            tension: 1.5,
            air_friction: 2.0,
            gravity: 0.3,
            wind_sensitivity: 4.2,
            user_force: Vec3::new(1.0, -2.0, 0.5),
        };
        let back = entry(PARAMS_FLEXIBLE, &pack_flexible(&f)).flexible.expect("flexible");
        assert_eq!(back.softness, 3);
        assert!((back.tension - 1.5).abs() < 0.01);
        assert!((back.gravity - 0.3).abs() < 0.01);
        assert!((back.wind_sensitivity - 4.2).abs() < 0.01);
        assert_eq!(back.user_force, f.user_force);
        let l = LightParams {
            color: [1.0, 0.5, 0.0, 1.0],
            radius: 10.0,
            cutoff: 0.0,
            falloff: 0.75,
        };
        let back = entry(PARAMS_LIGHT, &pack_light(&l)).light.expect("light");
        assert_eq!(back.radius, 10.0);
        assert!((back.color[1] - 0.5).abs() < 0.003);
        let r = ReflectionProbeParams {
            ambiance: 0.5,
            clip_distance: 2.0,
            flags: 3,
        };
        assert_eq!(entry(PARAMS_REFLECTION_PROBE, &pack_reflection_probe(&r)).reflection_probe, Some(r));
        let li = LightImageParams {
            texture: Uuid::from_bytes([4; 16]),
            params: Vec3::new(1.0, 2.0, 0.5),
        };
        assert_eq!(entry(PARAMS_LIGHT_IMAGE, &pack_light_image(&li)).light_image, Some(li));
    }

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
    /// What a rigged glTF attachment carries: mesh, extended mesh and render
    /// material entries (LLRenderMaterialParams::pack: count, then per entry
    /// the face index and the material asset id).
    #[test]
    fn parses_render_materials() {
        let push = |v: &mut Vec<u8>, ty: u16, d: &[u8]| {
            v.extend_from_slice(&ty.to_le_bytes());
            v.extend_from_slice(&(d.len() as u32).to_le_bytes());
            v.extend_from_slice(d);
        };
        let mat_a = Uuid::from_u128(0x968cbad0_4dad_d64e_71b5_72bf13ad051a);
        let mat_b = Uuid::from_u128(0x1234);
        let mut rm = vec![2u8, 0];
        rm.extend_from_slice(mat_a.as_bytes());
        rm.push(3);
        rm.extend_from_slice(mat_b.as_bytes());
        let mut mesh = vec![9u8; 16];
        mesh.push(5);
        let mut v = vec![3u8];
        push(&mut v, PARAMS_MESH, &mesh);
        push(&mut v, PARAMS_EXTENDED_MESH, &0u32.to_le_bytes());
        push(&mut v, PARAMS_RENDER_MATERIAL, &rm);
        let ep = parse_extra_params(&v);
        assert!(ep.sculpt.expect("mesh").is_mesh());
        assert_eq!(ep.extended_mesh_flags, Some(0));
        assert_eq!(ep.render_materials, vec![(0, mat_a), (3, mat_b)]);
        // a truncated entry list leaves no half-read materials
        let mut short = vec![1u8];
        push(&mut short, PARAMS_RENDER_MATERIAL, &rm[..rm.len() - 1]);
        let ep = parse_extra_params(&short);
        assert!(ep.render_materials.is_empty());
        // no entries: no materials
        assert!(entry(PARAMS_RENDER_MATERIAL, &[0]).render_materials.is_empty());
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

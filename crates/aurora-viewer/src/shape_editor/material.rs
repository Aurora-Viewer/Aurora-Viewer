//! Resolve the same worn-face appearance as Scene::rebuild_faces, without
//! requesting assets. Texture entry / LLGLTFMaterial transforms follow
//! LLFace::getGeometryVolume (Firestorm, originally LGPL 2.1).

use crate::{
    scene::{Scene, avatar},
    world::{World, objects::Object},
};
use aurora_render::preview::Material;
use uuid::Uuid;

pub(super) struct Face {
    pub textures: [Uuid; 4],
    pub material: Material,
    pub alpha_explicit: bool,
}

/// UV input here is SL bottom-left, while CPU images are top-left.
fn uv(st: [f32; 4], rotation: f32, pbr: bool) -> [[f32; 4]; 2] {
    let (s, c) = rotation.sin_cos();
    if pbr {
        [
            [c * st[0], -s * st[1], s * st[1] + st[2], 0.0],
            [-s * st[0], -c * st[1], c * st[1] + st[3], 0.0],
        ]
    } else {
        // ll_xform rotates around (.5,.5), then applies repeats and offsets.
        let a = c * st[0];
        let b = s * st[0];
        let d = -s * st[1];
        let e = c * st[1];
        [[a, b, 0.5 + st[2] - 0.5 * (a + b), 0.0], [-d, -e, 0.5 - st[3] + 0.5 * (d + e), 0.0]]
    }
}

impl Face {
    pub fn capture(world: &World, scene: &Scene, object: &Object, index: usize, avatar_index: usize) -> Self {
        let te = object.visual_texture_face(index);
        let mut out = Self {
            textures: [te.texture, Uuid::nil(), Uuid::nil(), Uuid::nil()],
            material: Material {
                color: te.color,
                alpha_mode: 2,
                uv: [uv([te.scale_s, te.scale_t, te.offset_s, te.offset_t], te.rotation, false); 4],
                ..Default::default()
            },
            alpha_explicit: false,
        };
        if let Some(id) = object.visual_material(index) {
            let mut material = scene.materials.peek(&id).map(|m| (*m).clone()).unwrap_or_default();
            if object.local_appearance(index).is_none()
                && let Some(override_) = world
                    .gltf_overrides
                    .get(&object.key)
                    .and_then(|faces| faces.iter().rev().find(|(face, _)| *face as usize == index))
                    .map(|(_, override_)| override_)
            {
                material.apply_override(override_);
            }
            out.textures = [
                material.base_color_texture,
                material.normal_texture,
                material.metallic_roughness_texture,
                material.emissive_texture,
            ]
            .map(|id| id.unwrap_or_default());
            out.material = Material {
                color: material.base_color_factor,
                emissive: material.emissive_factor,
                metallic: material.metallic_factor,
                roughness: material.roughness_factor,
                alpha_cutoff: material.alpha_cutoff,
                alpha_mode: match material.alpha_mode {
                    aurora_assets::AlphaMode::Opaque => 0,
                    aurora_assets::AlphaMode::Blend => 1,
                    aurora_assets::AlphaMode::Mask => 2,
                },
                double_sided: material.double_sided,
                uv: material
                    .transforms
                    .map(|t| uv([t.scale[0], t.scale[1], t.offset[0], t.offset[1]], t.rotation, true)),
                ..Default::default()
            };
            out.alpha_explicit = true;
        } else if let Some(material) = scene.legacy_mats.peek(&te.material_id) {
            out.textures[1] = material.normal_map;
            out.material.uv[1] = uv(material.normal_st, material.normal_rot, false);
            out.material.alpha_cutoff = material.alpha_cutoff as f32 / 255.0;
            out.material.alpha_mode = match material.diffuse_alpha_mode {
                crate::scene::legacy_mat::alpha_mode::BLEND => 1,
                crate::scene::legacy_mat::alpha_mode::MASK => 2,
                _ => 0,
            };
            out.alpha_explicit = true;
        }
        for texture in &mut out.textures {
            if let Some((slot, _)) = avatar::bake_slot_for(texture) {
                *texture = world.bake_texture(avatar_index, slot).unwrap_or_default();
            }
        }
        out
    }
    /// What builds the hints again: the maps, how the face blends and
    /// whether it shows at all (a body HUD hides parts with alpha 0).
    pub fn hash(&self, hasher: &mut impl std::hash::Hasher) {
        use std::hash::Hash;
        self.textures.hash(hasher);
        self.alpha_explicit.hash(hasher);
        self.material.alpha_mode.hash(hasher);
        self.material.double_sided.hash(hasher);
        (self.material.color[3] < 0.004).hash(hasher);
    }

    /// Its tint, glow and texture placement, which scripts often change
    /// all the time (pulsing tattoos, scrolling textures): the hints take
    /// them up whenever they are built, never just for them.
    pub fn hash_look(&self, hasher: &mut impl std::hash::Hasher) {
        use std::hash::Hash;
        for value in self
            .material
            .color
            .into_iter()
            .chain(self.material.emissive)
            .chain([self.material.metallic, self.material.roughness, self.material.alpha_cutoff])
            .chain(self.material.uv.into_iter().flatten().flatten())
        {
            value.to_bits().hash(hasher);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn texture_entry_and_gltf_uvs_match_world_shader_conventions() {
        let sample = |rows: [[f32; 4]; 2], point: [f32; 2]| rows.map(|r| r[0] * point[0] + r[1] * point[1] + r[2]);
        let out = sample(uv([1.0, 1.0, 0.0, 0.0], 0.0, false), [0.2, 0.8]);
        assert!((out[0] - 0.2).abs() < 1e-6 && (out[1] - 0.2).abs() < 1e-6);
        let out = sample(uv([2.0, 3.0, 0.1, 0.2], 0.0, false), [0.5, 0.5]);
        assert!((out[0] - 0.6).abs() < 1e-6 && (out[1] - 0.3).abs() < 1e-6);
        let out = sample(uv([2.0, 3.0, 0.1, 0.2], 0.0, true), [0.5, 0.5]);
        assert!((out[0] - 1.1).abs() < 1e-6 && (out[1] - 1.7).abs() < 1e-6);
    }
    #[test]
    fn pbr_face_uses_material_and_override_maps_including_bom_redirection() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let mut object = super::super::demo::object(&world).expect("head");
        let material_id = Uuid::from_u128(122);
        object.extra.render_materials = vec![(0, material_id)];
        let index = world.objects.upsert(world.main_region.expect("region"), object);
        let mut scene = Scene::new(
            std::env::temp_dir().join(format!("aurora-hint-material-{}", Uuid::new_v4())),
            world.avatar_lib.clone(),
        );
        scene.materials.resolve(&material_id);
        scene.materials.on_material(
            material_id,
            Some(aurora_assets::PbrMaterial {
                base_color_texture: Some(avatar::BAKES[0].2),
                base_color_factor: [0.8, 0.7, 0.6, 0.5],
                alpha_mode: aurora_assets::AlphaMode::Blend,
                ..Default::default()
            }),
        );
        let object = world.objects.get(index).expect("object");
        let avatar = world.objects.index_of_uuid(&world.agent_id).expect("avatar");
        let face = Face::capture(&world, &scene, object, 0, avatar);
        assert_eq!(face.textures[0], world.bake_texture(avatar, 8).expect("head bake"));
        assert_eq!(face.material.color, [0.8, 0.7, 0.6, 0.5]);
        assert_eq!(face.material.alpha_mode, 1);
        let object_key = object.key;
        let override_texture = Uuid::from_u128(123);
        world.gltf_overrides.insert(
            object_key,
            vec![(
                0,
                aurora_assets::material::PbrOverride {
                    textures: [override_texture, Uuid::nil(), Uuid::nil(), Uuid::nil()],
                    ..Default::default()
                },
            )]
            .into(),
        );
        let face = Face::capture(&world, &scene, world.objects.get(index).expect("object"), 0, avatar);
        assert_eq!(face.textures[0], override_texture);
    }
}

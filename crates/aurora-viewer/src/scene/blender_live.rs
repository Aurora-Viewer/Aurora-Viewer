//! Eager retirement for frequently replaced live-preview assets. Ordinary
//! scene geometry keeps its normal reuse interval; a stream must not retain
//! ten seconds of obsolete meshes or release buffers still drawn by an object.
use super::*;

pub(super) struct Appearance {
    materials: Vec<Uuid>,
    images: Vec<Uuid>,
}
pub(super) struct ImageSlot {
    slot: u32,
    refs: usize,
}

impl Scene {
    /// Resolve local rez/drop support, including elevated floors. Reuse the
    /// scene's triangle picking; previews cannot become their own support.
    pub fn preview_floor(&self, world: &World, position: Vec3, ceiling: f32) -> Option<f32> {
        use rayon::prelude::*;
        let ray = (position.with_z(ceiling), -Vec3::Z);
        let mut candidates = Vec::new();
        self.pick.ray_candidates(ray, &mut candidates);
        let now = Instant::now();
        let object = candidates
            .par_iter()
            .filter_map(|&idx| {
                let object = world.objects.get(idx)?;
                let gpu = self.gpu.get(idx)?;
                if object.is_client_preview()
                    || object.pcode != aurora_prim::params::LL_PCODE_VOLUME
                    || object.update_flags & crate::build::edits::FLAGS_PHANTOM != 0
                    || gpu.hud
                    || gpu.rigged
                    || gpu.owner_avatar.is_some()
                {
                    return None;
                }
                let hit = self.face_hit(world, idx, ray, now, true)?;
                (hit.t.is_finite() && hit.t >= 0.0 && hit.normal.is_finite() && hit.normal.z > 0.1).then_some(ceiling - hit.t)
            })
            .max_by(f32::total_cmp);
        let terrain = world
            .ground_height(position)
            .filter(|height| height.is_finite() && *height <= ceiling);
        match (object, terrain) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn preview_appearance_counts(&self) -> (usize, usize) {
        (self.preview_appearances.len(), self.preview_images.len())
    }
    pub fn install_preview_images(&mut self, images: &[crate::blender_live::protocol::PreparedImage], renderer: &mut Renderer) -> bool {
        if images
            .iter()
            .any(|image| image.staged.as_ref().is_none_or(|staged| !staged.data.ready()))
        {
            return false;
        }
        for image in images {
            if self.preview_images.contains_key(&image.id) {
                continue;
            }
            let Some(staged) = image.staged.as_ref() else { return false };
            let Some(slot) = renderer.textures.create_placeholder() else {
                return false;
            };
            if !renderer.replace_texture_staged(slot, staged) {
                renderer.free_texture(slot);
                return false;
            }
            self.textures.register_local_image(image.id, slot, image.alpha, image.has_alpha);
            self.preview_images.insert(image.id, ImageSlot { slot, refs: 0 });
        }
        true
    }

    pub fn install_preview_materials(&mut self, mesh: Uuid, materials: &[Option<(Uuid, aurora_assets::material::PbrMaterial)>]) {
        if self.preview_appearances.contains_key(&mesh) {
            return;
        }
        let mut appearance = Appearance {
            materials: Vec::new(),
            images: Vec::new(),
        };
        for (id, material) in materials.iter().flatten() {
            appearance.materials.push(*id);
            self.materials.insert(*id, material.clone());
            for image in [
                material.base_color_texture,
                material.normal_texture,
                material.metallic_roughness_texture,
                material.emissive_texture,
            ]
            .into_iter()
            .flatten()
            {
                if !appearance.images.contains(&image) {
                    appearance.images.push(image);
                    if let Some(entry) = self.preview_images.get_mut(&image) {
                        entry.refs += 1;
                    }
                }
            }
        }
        self.preview_appearances.insert(mesh, appearance);
    }

    pub fn retire_preview_mesh(&mut self, id: Uuid) {
        self.retired_preview_meshes.insert(id);
    }

    pub(super) fn collect_preview_meshes(&mut self, renderer: &mut Renderer) {
        let ready: Vec<_> = self
            .retired_preview_meshes
            .iter()
            .copied()
            .filter(|id| {
                (0..4).all(|lod| {
                    self.geoms
                        .get(&GeomKey::Mesh { id: *id, lod })
                        .is_none_or(|entry| entry.refs == 0 && !matches!(entry.state, GeomState::Pending))
                })
            })
            .take(64)
            .collect();
        for id in ready {
            if let Some(appearance) = self.preview_appearances.remove(&id) {
                for material in appearance.materials {
                    self.materials.remove_local(&material);
                }
                for image in appearance.images {
                    if let Some(entry) = self.preview_images.get_mut(&image) {
                        entry.refs = entry.refs.saturating_sub(1);
                    }
                }
            }
            for lod in 0..4 {
                let key = GeomKey::Mesh { id, lod };
                if let Some(GeomEntry {
                    state: GeomState::Ready(geometry),
                    ..
                }) = self.geoms.remove(&key)
                {
                    for face in geometry.faces.iter().flatten() {
                        renderer.free_mesh(*face);
                    }
                    self.joint_boxes.release(&key);
                    self.geom_trash.push(geometry);
                }
            }
            self.skin_binds.release(&id);
            self.meshes.retire_local(&id, &self.jobs);
            self.retired_preview_meshes.remove(&id);
        }
        let unused: Vec<_> = self
            .preview_images
            .iter()
            .filter(|(_, entry)| entry.refs == 0)
            .map(|(id, _)| *id)
            .collect();
        for id in unused {
            if let Some(entry) = self.preview_images.remove(&id) {
                self.textures.unregister_local(&id);
                renderer.free_texture(entry.slot);
            }
        }
    }
}

#[cfg(test)]
mod floor_tests {
    use super::*;

    #[test]
    fn blender_live_floor_uses_platform_triangles_and_excludes_phantom_and_worn_supports() {
        let (mut world, mut scene, front, _) = super::super::picking::tests::scene_with_two_objects();
        let object = world.objects.get_mut(front).expect("platform");
        object.update_flags = 0;
        object.rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        scene.gpu[front].center = Vec3::new(3.0, 0.0, 50.0);
        scene.gpu[front].radius = 1.0;
        scene.pick.set(front, super::super::hover::PickEntry::of(&scene.gpu[front]));
        let point = Vec3::new(3.0, 0.0, 51.0);
        let floor = scene.preview_floor(&world, point, 52.0).expect("elevated floor");
        assert!((floor - 50.5).abs() < 0.001);
        world.objects.get_mut(front).expect("platform").update_flags = crate::build::edits::FLAGS_PHANTOM;
        assert!(scene.preview_floor(&world, point, 52.0).is_none_or(|height| height < 50.5));
        world.objects.get_mut(front).expect("platform").update_flags = 0;
        scene.gpu[front].owner_avatar = Some(0);
        assert!(scene.preview_floor(&world, point, 52.0).is_none_or(|height| height < 50.5));
    }
}

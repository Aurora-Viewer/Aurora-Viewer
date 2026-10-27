//! Render-only placement hologram (#854). Geometry is prepared once per
//! shape on rayon; pointer motion only updates one GPU placement record.
//! Uses the loading shader's own HOLOGRAM mode, independently of Rez::enabled.

use crate::build::{BuildSettings, MAX_PRIM_SCALE, MIN_PRIM_SCALE, shapes};
use crate::world::World;
use aurora_render::{DrawCmd, DrawLists, DrawRecord, MeshAlloc, Renderer, rez_mode};
use crossbeam_channel::{Receiver, TryRecvError};
use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone, Copy)]
pub struct Placement {
    shape: usize,
    position: Vec3,
    rotation: Quat,
    scale: Vec3,
    bottom: f32,
    height: f32,
}

impl Placement {
    /// LLToolPlacer's shape, orientation and size; the simulator still
    /// decides the final placement from ObjectAdd's unchanged surface ray.
    pub fn new(world: &World, settings: &BuildSettings, point: Vec3) -> Option<Self> {
        if !point.is_finite()
            || settings
                .select_limit()
                .is_some_and(|max| point.distance(world.agent.position) > max)
        {
            return None;
        }
        crate::build::BuildTool::region_at(world, point)?;
        let shape = shapes::shapes().into_iter().nth(settings.create_shape)?;
        let scale = Vec3::from_array(settings.new_prim_size).clamp(Vec3::splat(MIN_PRIM_SCALE), Vec3::splat(MAX_PRIM_SCALE));
        let rotation = shape.rotation();
        let axes = glam::Mat3::from_quat(rotation);
        let half_height = Vec3::new(axes.x_axis.z, axes.y_axis.z, axes.z_axis.z).abs().dot(scale) * 0.5;
        Some(Self {
            shape: settings.create_shape,
            position: point + Vec3::Z * half_height,
            rotation,
            scale,
            bottom: point.z,
            height: half_height * 2.0,
        })
    }

    fn record(self, start: u32) -> DrawRecord {
        DrawRecord {
            model: Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position).to_cols_array_2d(),
            base_color: [0.5, 0.5, 0.5, 1.0],
            // Own look: the loading effect's chosen fine wire grid, even
            // when the frame's rez effect or animation preference is off.
            rez: [start, rez_mode::HOLOGRAM | (3 << 20), self.bottom.to_bits(), self.height.to_bits()],
            ..DrawRecord::default()
        }
    }
}

type Prepared = Vec<super::jobs::FaceGeom>;

enum Geometry {
    Pending(Receiver<Prepared>),
    Ready(Vec<MeshAlloc>),
}

#[derive(Default)]
pub struct CreationPreview {
    shapes: HashMap<usize, Geometry>,
    record: Option<u32>,
    start: Option<u32>,
    active: Option<usize>,
    pub candidates: Vec<usize>,
}

impl CreationPreview {
    pub fn update(&mut self, renderer: &mut Renderer, placement: Option<Placement>) {
        self.active = None;
        let Some(placement) = placement else {
            self.start = None;
            return;
        };
        let start = *self.start.get_or_insert_with(|| renderer.anim_clock().ms(Instant::now()));
        let geometry = self.shapes.entry(placement.shape).or_insert_with(|| {
            let params = shapes::shapes()[placement.shape].params();
            let (send, receive) = crossbeam_channel::bounded(1);
            rayon::spawn(move || {
                let mesh = aurora_prim::generate_volume(&params, aurora_prim::volume::DETAIL_SCALES[2]);
                let (faces, _, _) = super::jobs::volume_to_faces(&mesh);
                let _ = send.send(faces.into_iter().flatten().collect());
            });
            Geometry::Pending(receive)
        });
        if let Geometry::Pending(receive) = geometry {
            match receive.try_recv() {
                Ok(faces) => {
                    let meshes: Vec<_> = faces.iter().filter_map(|f| renderer.upload_mesh(&f.vertices, &f.indices)).collect();
                    if meshes.len() != faces.len() {
                        for mesh in meshes {
                            renderer.free_mesh(mesh);
                        }
                        log::warn!("creation preview: geometry arena full");
                        *geometry = Geometry::Ready(Vec::new());
                        return;
                    }
                    *geometry = Geometry::Ready(meshes);
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    log::warn!("creation preview: geometry preparation failed");
                    *geometry = Geometry::Ready(Vec::new());
                    return;
                }
            }
        }
        let record = placement.record(start);
        if let Some(slot) = self.record {
            renderer.records.set(slot, record);
        } else {
            self.record = Some(renderer.records.alloc(record));
        }
        self.active = Some(placement.shape);
    }

    pub fn append(&self, lists: &mut DrawLists) {
        let Some(Geometry::Ready(meshes)) = self.active.and_then(|shape| self.shapes.get(&shape)) else {
            return;
        };
        let Some(record) = self.record else { return };
        lists.creation_preview.extend(meshes.iter().map(|m| DrawCmd {
            index_count: m.index_count,
            first_index: m.index_offset,
            base_vertex: m.vertex_offset as i32,
            record,
            bounds: [0.0; 4],
        }));
    }

    pub fn clear(&mut self, renderer: &mut Renderer) {
        for (_, geometry) in self.shapes.drain() {
            if let Geometry::Ready(meshes) = geometry {
                for mesh in meshes {
                    renderer.free_mesh(mesh);
                }
            }
        }
        if let Some(record) = self.record.take() {
            renderer.records.free(record);
        }
        self.active = None;
        self.start = None;
        self.candidates.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn invisible_mesh_selection_requests_and_releases_its_wireframe_records() {
        let (mut world, mut scene, idx, _) = super::super::picking::tests::scene_with_two_objects();
        let object = world.objects.get_mut(idx).unwrap();
        for face in &mut Arc::make_mut(object.te.as_mut().unwrap()).faces {
            face.color[3] = 0.0;
        }
        object.material_dirty = false;
        object.render.needs_records = false;
        scene.set_wire_selection(&mut world, [(idx, true)].into_iter().collect());
        assert!(
            world.objects.get(idx).unwrap().render.needs_records,
            "A selected invisible mesh needs wireframe draw records"
        );
        let object = world.objects.get_mut(idx).unwrap();
        object.material_dirty = false;
        object.render.needs_records = false;
        scene.set_wire_selection(&mut world, [(idx, true)].into_iter().collect());
        assert!(
            !world.objects.get(idx).unwrap().render.needs_records,
            "Steady selection must not rebuild each frame"
        );
        scene.set_wire_selection(&mut world, HashMap::new());
        assert!(
            world.objects.get(idx).unwrap().render.needs_records,
            "Deselection removes temporary invisible records"
        );
    }

    #[test]
    fn creation_hologram_uses_each_rezzed_shape_and_rejects_invalid_placement() {
        let mut world = World::new(Arc::new(super::super::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let point = world.agent.position + Vec3::new(4.0, 3.0, 0.0);
        for (i, shape) in shapes::shapes().iter().enumerate() {
            let settings = BuildSettings {
                create_shape: i,
                new_prim_size: [1.0, 2.0, 3.0],
                ..Default::default()
            };
            let placement = Placement::new(&world, &settings, point).expect("valid demo surface");
            assert_eq!(placement.rotation, shape.rotation());
            assert_eq!(placement.scale, Vec3::new(1.0, 2.0, 3.0));
            let record = placement.record(123);
            assert_eq!(record.rez[0], 123);
            assert_eq!(record.rez[1] & 0xff, rez_mode::HOLOGRAM);
            assert_eq!(record.rez[1] >> 20, 3, "Own loading style is independent of the global rez setting");
            let model = Mat4::from_cols_array_2d(&record.model);
            let lowest = [-1.0, 1.0]
                .into_iter()
                .flat_map(|x| {
                    [-1.0, 1.0]
                        .into_iter()
                        .flat_map(move |y| [-1.0, 1.0].map(move |z| Vec3::new(x, y, z) * 0.5))
                })
                .map(|p| model.transform_point3(p).z)
                .fold(f32::INFINITY, f32::min);
            assert!((lowest - point.z).abs() < 1e-4);
        }
        let settings = BuildSettings::default();
        assert!(Placement::new(&world, &settings, Vec3::NAN).is_none());
        // unlimited by default; LimitSelectDistance refuses a far surface
        let far = point + Vec3::X * (crate::build::MAX_SELECT_DISTANCE + 10.0);
        let limited = BuildSettings {
            limit_select_distance: true,
            ..settings.clone()
        };
        assert!(Placement::new(&world, &limited, far).is_none());
        let settings = BuildSettings {
            create_shape: shapes::shapes().len(),
            ..settings
        };
        assert!(
            Placement::new(&world, &settings, point).is_none(),
            "Plants have no primitive hologram"
        );
    }
}

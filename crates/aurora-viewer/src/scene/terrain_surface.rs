//! Four-layer terrain, following LLTerrainMaterials and LLDrawPoolTerrain
//! (indra/newview/llvlcomposition.cpp, lldrawpoolterrain.cpp, LGPL 2.1).
//! Geometry survives material edits: only the region's shared layer records
//! and each chunk's surface references change. Maps use the normal streamer.

use super::*;
use aurora_assets::PbrMaterial;
use aurora_net::estate::TerrainTransform;
use std::sync::Arc;

/// LLTerrainMaterials::boost, TERRAIN_DECODE_PRIORITY = 2048 * 2048.
pub(super) const DECODE_PIXELS: f32 = 2048.0;

#[derive(Clone, Copy, PartialEq)]
struct Input {
    ids: [Uuid; 4],
    materials: Option<bool>,
    transforms: [TerrainTransform; 4],
    paintmap: Option<Uuid>,
    size: [u32; 2],
}

#[derive(Default)]
pub(super) struct SurfaceGpu {
    input: Option<Input>,
    materials: [Option<Arc<PbrMaterial>>; 4],
    layers: Option<[u32; 4]>,
    pub texture_ids: Vec<Uuid>,
    slots: [u32; 4],
    pbr: bool,
    pub masked: bool,
    paint_slot: Option<(Uuid, u32)>,
}

impl SurfaceGpu {
    pub(super) fn free(self, renderer: &mut Renderer, textures: &mut textures::TextureStreamer) {
        if let Some((_, slot)) = self.paint_slot {
            renderer.free_texture(slot);
        }
        for id in self.texture_ids {
            textures.release(&id);
        }
        if let Some(layers) = self.layers {
            for layer in layers {
                renderer.records.free(layer);
            }
        }
    }

    fn sync(
        &mut self,
        input: Input,
        renderer: &mut Renderer,
        textures: &mut textures::TextureStreamer,
        materials: &mut meshes::MaterialStreamer,
        paintmap: Option<&mut terrain::PaintMap>,
    ) -> bool {
        if let Some(paintmap) = paintmap {
            if self.paint_slot.map(|v| v.0) != Some(paintmap.id) {
                let Some(staged) = paintmap.prepare(renderer.staging_pool()) else {
                    return false;
                };
                let Some(slot) = renderer.textures.create_placeholder() else {
                    return false;
                };
                if !renderer.replace_texture_staged(slot, staged) {
                    renderer.free_texture(slot);
                    return false;
                }
                if let Some((_, old)) = self.paint_slot.replace((paintmap.id, slot)) {
                    renderer.free_texture(old);
                }
                paintmap.staged = None;
            }
        } else if let Some((_, slot)) = self.paint_slot.take() {
            renderer.free_texture(slot);
        }
        let loaded = input.ids.map(|id| (!id.is_nil()).then(|| materials.get(&id)).flatten());
        if self.input == Some(input) && self.materials == loaded {
            return false;
        }
        let any_texture = input
            .ids
            .iter()
            .any(|id| !id.is_nil() && (textures.is_loaded(id) || textures.is_local(id)));
        self.pbr = input.materials.unwrap_or(!any_texture && loaded.iter().any(Option::is_some));
        self.masked = self.pbr && loaded.iter().flatten().any(|m| m.alpha_mode == aurora_assets::AlphaMode::Mask);
        let mut wanted = Vec::with_capacity(16);
        if self.pbr {
            let layers = *self
                .layers
                .get_or_insert_with(|| std::array::from_fn(|_| renderer.records.alloc(DrawRecord::default())));
            for (i, layer) in layers.into_iter().enumerate() {
                let material = loaded[i].as_deref().cloned().unwrap_or_default();
                let maps = [
                    material.base_color_texture,
                    material.normal_texture,
                    material.metallic_roughness_texture,
                    material.emissive_texture,
                ];
                let defaults = [
                    aurora_render::textures::WHITE,
                    aurora_render::textures::FLAT_NORMAL,
                    aurora_render::textures::WHITE,
                    aurora_render::textures::WHITE,
                ];
                let slots = std::array::from_fn(|map| match maps[map].filter(|id| !id.is_nil()) {
                    Some(id) => {
                        wanted.push(id);
                        textures.acquire(renderer, id, TexSource::Asset)
                    }
                    None => defaults[map],
                });
                renderer
                    .records
                    .set_if_changed(layer, layer_record(&material, input.transforms[i], slots));
            }
            self.slots = layers;
        } else {
            self.slots = input.ids.map(|id| {
                if !id.is_nil() {
                    wanted.push(id);
                }
                textures.acquire(renderer, id, TexSource::Asset)
            });
        }
        // Acquire first: a map shared by the old and new surfaces stays resident.
        for id in self.texture_ids.drain(..) {
            textures.release(&id);
        }
        self.texture_ids = wanted;
        self.input = Some(input);
        self.materials = loaded;
        true
    }

    fn record(&self, offset: Vec3) -> DrawRecord {
        DrawRecord {
            model: Mat4::from_translation(offset).to_cols_array_2d(),
            uv_st: [12.0, 12.0, 0.0, 0.0],
            tex: self.slots,
            // Terrain-only record fields: paintmap slot and region extent.
            mat_uv: self.input.map_or([1.0; 4], |i| [i.size[0] as f32, i.size[1] as f32, 0.0, 0.0]),
            flags: [
                flags::TERRAIN | if self.pbr { flags::PBR } else { 0 } | if self.masked { flags::ALPHA_MASK } else { 0 },
                0,
                self.paint_slot.map_or(0, |v| v.1),
                0,
            ],
            ..Default::default()
        }
    }
}

/// The region override replaces the asset's transform, as
/// LLTerrainMaterials::makeMaterialsReady does. Otherwise the base-color
/// transform is shared by all maps (LLDrawPoolTerrain::renderFullShaderPBR).
fn layer_record(material: &PbrMaterial, override_: TerrainTransform, slots: [u32; 4]) -> DrawRecord {
    let base = material.transforms[0];
    let transform = if override_ == TerrainTransform::default() {
        TerrainTransform {
            scale: base.scale,
            rotation: base.rotation,
            offset: base.offset,
        }
    } else {
        override_
    };
    DrawRecord {
        base_color: material.base_color_factor,
        emissive: [
            material.emissive_factor[0],
            material.emissive_factor[1],
            material.emissive_factor[2],
            0.0,
        ],
        uv_st: [
            transform.scale[0] / 8.0,
            transform.scale[1] / 8.0,
            transform.offset[0],
            transform.offset[1],
        ],
        params: [
            transform.rotation,
            material.metallic_factor,
            material.roughness_factor,
            if material.alpha_mode == aurora_assets::AlphaMode::Mask {
                material.alpha_cutoff
            } else {
                0.0
            },
        ],
        tex: slots,
        flags: [flags::PBR, 0, 0, 0],
        ..Default::default()
    }
}

impl Scene {
    pub(super) fn sync_terrain(&mut self, renderer: &mut Renderer, world: &mut World) {
        let stale: Vec<_> = self.regions.keys().filter(|h| !world.regions.contains_key(h)).copied().collect();
        for h in stale {
            if let Some(region) = self.regions.remove(&h) {
                for chunk in region.chunks.into_values() {
                    renderer.free_mesh(chunk.alloc);
                    renderer.records.free(chunk.record);
                }
                region.surface.free(renderer, &mut self.textures);
            }
        }
        let handles: Vec<_> = world.regions.keys().copied().collect();
        let mut built = 0;
        for handle in handles {
            let Some(offset) = world.region_offset(handle) else { continue };
            let Some(region) = world.regions.get_mut(&handle) else { continue };
            let Some(info) = region.info.clone() else { continue };
            let comp = Composition {
                start_height: info.terrain_start_height,
                height_range: info.terrain_height_range,
                origin_x: (handle >> 32) as f64,
                origin_y: (handle & 0xffff_ffff) as f64,
            };
            if region.terrain_surface.paint_busy() {
                region
                    .terrain_surface
                    .poll_paint(&region.heightmap, comp, renderer.staging_pool(), renderer.max_texture_dimension());
            }
            let local = region.terrain_surface.effective();
            let input = Input {
                ids: local.map_or(info.terrain_detail, |s| s.ids),
                materials: local.map(|s| s.materials),
                transforms: local.map_or(region.terrain_surface.transforms, |s| s.transforms),
                paintmap: region.terrain_surface.paintmap.as_ref().map(|p| p.id),
                size: [region.heightmap.size_x, region.heightmap.size_y],
            };
            let gpu = self.regions.entry(handle).or_insert_with(|| RegionGpu {
                chunks: HashMap::new(),
                surface: SurfaceGpu::default(),
                offset: Vec3::NAN,
            });
            // Surface changes never rebuild terrain geometry or pause streaming.
            let changed = gpu.surface.sync(
                input,
                renderer,
                &mut self.textures,
                &mut self.materials,
                region.terrain_surface.paintmap.as_mut(),
            );
            let record = gpu.surface.record(offset);
            if changed || gpu.offset != offset {
                for chunk in gpu.chunks.values() {
                    renderer.records.set_if_changed(chunk.record, record);
                }
                gpu.offset = offset;
            }
            if region.dirty_chunks.is_empty() || built > 6 {
                continue;
            }
            let chunks: Vec<_> = region.dirty_chunks.iter().copied().take(8).collect();
            for index in chunks {
                let (vertices, frames, indices) = terrain::build_chunk(&region.heightmap, &comp, index);
                // Terrain reuses the parallel frame bytes without enabling skinning.
                let Some(alloc) = renderer.upload_skinned_mesh(&vertices, &frames, &indices) else {
                    continue;
                };
                region.dirty_chunks.remove(&index);
                let (mut zmin, mut zmax) = (f32::MAX, f32::MIN);
                for vertex in &vertices {
                    zmin = zmin.min(vertex.pos[2]);
                    zmax = zmax.max(vertex.pos[2]);
                }
                let cx = region.heightmap.size_x / CHUNK_CELLS;
                let half = CHUNK_CELLS as f32 * 0.5;
                let center = offset
                    + Vec3::new(
                        (index % cx * CHUNK_CELLS) as f32 + half,
                        (index / cx * CHUNK_CELLS) as f32 + half,
                        (zmin + zmax) * 0.5,
                    );
                let radius = Vec3::new(half, half, (zmax - zmin) * 0.5).length();
                let record = renderer.records.alloc(record);
                if let Some(old) = gpu.chunks.insert(
                    index,
                    TerrainChunk {
                        alloc,
                        record,
                        center,
                        radius,
                    },
                ) {
                    renderer.free_mesh(old.alloc);
                    renderer.records.free(old.record);
                }
                built += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_layer_records_preserve_factors_and_reference_texture_transforms() {
        let mut material = PbrMaterial {
            base_color_factor: [0.2, 0.3, 0.4, 0.8],
            metallic_factor: 0.6,
            roughness_factor: 0.25,
            emissive_factor: [0.7, 0.2, 0.1],
            alpha_mode: aurora_assets::AlphaMode::Mask,
            alpha_cutoff: 0.4,
            ..Default::default()
        };
        material.transforms[0] = aurora_assets::material::TextureTransform {
            scale: [4.0, -2.0],
            rotation: 0.5,
            offset: [0.1, 0.2],
        };
        let record = layer_record(&material, Default::default(), [7, 8, 9, 10]);
        assert_eq!(record.base_color, material.base_color_factor);
        assert_eq!(record.tex, [7, 8, 9, 10]);
        assert_eq!(record.uv_st, [0.5, -0.25, 0.1, 0.2]);
        assert_eq!(record.params, [0.5, 0.6, 0.25, 0.4]);
        assert_eq!(&record.emissive[..3], &material.emissive_factor);
        let override_ = TerrainTransform {
            scale: [2.0, 3.0],
            rotation: 1.0,
            offset: [0.4, 0.6],
        };
        let record = layer_record(&material, override_, [7, 8, 9, 10]);
        assert_eq!(record.uv_st, [0.25, 0.375, 0.4, 0.6]);
        assert_eq!(record.params[0], 1.0);
        let parent = SurfaceGpu {
            pbr: true,
            slots: [21, 22, 23, 24],
            ..Default::default()
        }
        .record(Vec3::new(256.0, 0.0, 0.0));
        assert_eq!(parent.tex, [21, 22, 23, 24]);
        assert_eq!(parent.flags[0], flags::PBR | flags::TERRAIN);
    }
}

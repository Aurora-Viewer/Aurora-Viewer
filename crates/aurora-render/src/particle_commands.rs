//! Ordered direct-draw chunks, validated on worker threads and reused while
//! their command ranges and bound resources stay unchanged. Particle data can
//! change in place without invalidating commands. No indirect argument reads,
//! device-generated commands, new device features or GPU/CPU synchronization.

use crate::ParticleDrawStats;
use crate::particles::{Draw, Plan};
use crate::renderer::ParticlePipelines;
use rayon::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};

const CHUNK_DRAWS: usize = 512;

struct Chunk {
    draws: Vec<Draw>,
    bundle: wgpu::RenderBundle,
}

struct Context {
    groups: Vec<wgpu::BindGroup>,
    vertices: wgpu::Buffer,
    color: wgpu::RenderPipeline,
    glow: wgpu::RenderPipeline,
    format: wgpu::TextureFormat,
    depth: Option<wgpu::TextureFormat>,
    samples: u32,
}

pub(crate) struct Commands {
    pub plan: Plan,
    chunks: Vec<Option<Chunk>>,
    context: Option<Context>,
    workers: Option<rayon::ThreadPool>,
    workers_tried: bool,
    direct_requested: bool,
}

impl Default for Commands {
    fn default() -> Self {
        Self {
            plan: Plan::default(),
            chunks: Vec::new(),
            context: None,
            workers: None,
            workers_tried: false,
            direct_requested: std::env::var("AURORA_PARTICLE_COMMANDS").is_ok_and(|mode| mode == "direct"),
        }
    }
}

impl Commands {
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        groups: &[&wgpu::BindGroup],
        vertices: &wgpu::Buffer,
        pipelines: &ParticlePipelines,
        glow: &wgpu::RenderPipeline,
        format: wgpu::TextureFormat,
        depth: Option<wgpu::TextureFormat>,
        samples: u32,
    ) -> ParticleDrawStats {
        // Do this before the short-list/direct early return and before bundle
        // workers run. New slots do not invalidate chunks of existing pairs.
        for draw in &self.plan.draws {
            pipelines.ensure(device, draw.blend);
        }
        let changed = self.context.as_ref().is_none_or(|c| {
            c.vertices != *vertices
                // Pair 0 identifies the whole shader/layout/sample generation.
                || c.color != pipelines[0]
                || c.glow != *glow
                || c.format != format
                || c.depth != depth
                || c.samples != samples
                || c.groups.len() != groups.len()
                || c.groups.iter().zip(groups).any(|(a, b)| a != *b)
        });
        if changed {
            self.chunks.clear();
            self.context = Some(Context {
                groups: groups.iter().map(|g| (*g).clone()).collect(),
                vertices: vertices.clone(),
                color: pipelines[0].clone(),
                glow: glow.clone(),
                format,
                depth,
                samples,
            });
        }
        if self.direct_requested || self.plan.draws.len() < CHUNK_DRAWS {
            // Ordinary alpha effects keep the inexpensive original direct path.
            self.chunks.clear();
            return self.plan.stats;
        }
        if !self.workers_tried {
            self.workers_tried = true;
            self.workers = rayon::ThreadPoolBuilder::new()
                .num_threads(2)
                .thread_name(|i| format!("aurora-particle-encode-{i}"))
                .build()
                .map_err(|error| log::warn!("particle command workers unavailable: {error}; keeping direct draws"))
                .ok();
        }
        let Some(workers) = &self.workers else {
            self.chunks.clear();
            return self.plan.stats;
        };
        let count = self.plan.draws.len().div_ceil(CHUNK_DRAWS);
        self.chunks.resize_with(count, || None);
        self.chunks.truncate(count);
        let builds = AtomicU32::new(0);
        workers.install(|| {
            self.chunks
                .par_iter_mut()
                .zip(self.plan.draws.par_chunks(CHUNK_DRAWS))
                .for_each(|(slot, draws)| {
                    if slot.as_ref().is_some_and(|chunk| chunk.draws == draws) {
                        return;
                    }
                    // The encoder is created and finished on the same worker: wgpu's
                    // RenderBundleEncoder is deliberately !Send. Only the result moves.
                    let mut encoder = device.create_render_bundle_encoder(&wgpu::RenderBundleEncoderDescriptor {
                        label: Some("particle ordered chunk"),
                        color_formats: &[Some(format)],
                        depth_stencil: depth.map(|format| wgpu::RenderBundleDepthStencil {
                            format,
                            depth_read_only: true,
                            stencil_read_only: true,
                        }),
                        sample_count: samples,
                        multiview: None,
                    });
                    for (i, group) in groups.iter().enumerate() {
                        encoder.set_bind_group(i as u32, *group, &[]);
                    }
                    encoder.set_vertex_buffer(0, vertices.slice(..));
                    let mut bound = None;
                    for draw in draws {
                        if bound != Some(draw.blend) {
                            encoder.set_pipeline(&pipelines[draw.blend]);
                        }
                        encoder.draw(0..6, draw.color.clone());
                        if !draw.glow.is_empty() {
                            encoder.set_pipeline(glow);
                            encoder.draw(0..6, draw.glow.clone());
                            bound = None;
                        } else {
                            bound = Some(draw.blend);
                        }
                    }
                    *slot = Some(Chunk {
                        draws: draws.to_vec(),
                        bundle: encoder.finish(&wgpu::RenderBundleDescriptor {
                            label: Some("particle ordered chunk"),
                        }),
                    });
                    builds.fetch_add(1, Ordering::Relaxed);
                });
        });
        self.plan.stats.bundle_builds = builds.load(Ordering::Relaxed);
        self.plan.stats.bundles = count as u32;
        self.plan.stats
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, pipelines: &ParticlePipelines, glow: &wgpu::RenderPipeline) {
        let Some(context) = &self.context else { return };
        if !self.chunks.is_empty() {
            pass.execute_bundles(self.chunks.iter().filter_map(|chunk| chunk.as_ref().map(|chunk| &chunk.bundle)));
            return;
        }
        for (i, group) in context.groups.iter().enumerate() {
            pass.set_bind_group(i as u32, group, &[]);
        }
        pass.set_vertex_buffer(0, context.vertices.slice(..));
        let mut bound = None;
        for draw in &self.plan.draws {
            if bound != Some(draw.blend) {
                pass.set_pipeline(&pipelines[draw.blend]);
            }
            pass.draw(0..6, draw.color.clone());
            if !draw.glow.is_empty() {
                pass.set_pipeline(glow);
                pass.draw(0..6, draw.glow.clone());
                bound = None;
            } else {
                bound = Some(draw.blend);
            }
        }
    }
}

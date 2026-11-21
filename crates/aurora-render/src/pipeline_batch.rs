//! The scene's pipelines compiled together on worker threads
//! (`Backend::make_pipelines`). A new build's first start finds no pipeline
//! of its own in the cache (its identity names the build,
//! `pipeline_cache.rs`): one after another on the making thread, the driver
//! took 11 s for them on a fast PC, and over the 90 s an update's helper
//! waits for the first frame on slower ones, which then rolled the update
//! back. Drivers compile pipelines on several threads at once.
//!
//! Each `pipe` of the scene set is added to a [`Batch`], which gives back a
//! [`Slot`]; the batch is then run, and every slot taken from what it made
//! ([`Resolve`]). Every worker keeps the error scopes of the making thread's
//! [`crate::gpu_alloc::scoped_apart`] (`gpu_alloc::on_worker`): a pipeline
//! refused at the start still makes the renderer again.

use super::{PipeDesc, make_pipeline};
use std::cell::RefCell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// At most this many workers: drivers compile in parallel, but each
/// compile holds memory of its own.
const MAX_WORKERS: usize = 12;

/// A pipeline the batch will make.
pub(super) struct Slot(usize);

/// A [`PipeDesc`] whose texts outlive the closure that wrote it.
struct Owned {
    label: String,
    vs: String,
    fs: Option<String>,
    constants: Vec<(String, f64)>,
    /// The rest of the description (its texts left empty).
    rest: PipeDesc<'static>,
}

impl Owned {
    fn new(d: PipeDesc<'_>) -> Self {
        Self {
            label: d.label.to_owned(),
            vs: d.vs.to_owned(),
            fs: d.fs.map(str::to_owned),
            constants: d.constants.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
            rest: PipeDesc {
                label: "",
                vs: "",
                fs: None,
                constants: &[],
                ..d
            },
        }
    }
}

struct Job<'a> {
    module: &'a wgpu::ShaderModule,
    layout: &'a wgpu::PipelineLayout,
    desc: Owned,
}

impl Job<'_> {
    fn make(&self, device: &wgpu::Device) -> wgpu::RenderPipeline {
        let d = &self.desc;
        let constants: Vec<(&str, f64)> = d.constants.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        make_pipeline(
            device,
            self.module,
            self.layout,
            PipeDesc {
                label: &d.label,
                vs: &d.vs,
                fs: d.fs.as_deref(),
                constants: &constants,
                ..d.rest
            },
        )
    }
}

/// Pipelines to make together.
#[derive(Default)]
pub(super) struct Batch<'a> {
    jobs: RefCell<Vec<Job<'a>>>,
}

impl<'a> Batch<'a> {
    pub(super) fn add(&self, module: &'a wgpu::ShaderModule, layout: &'a wgpu::PipelineLayout, d: PipeDesc<'_>) -> Slot {
        let mut jobs = self.jobs.borrow_mut();
        jobs.push(Job {
            module,
            layout,
            desc: Owned::new(d),
        });
        Slot(jobs.len() - 1)
    }

    /// Every pipeline made, on up to [`MAX_WORKERS`] threads; those no
    /// worker could take (none could start) on this one.
    pub(super) fn run(self, device: &wgpu::Device) -> Done {
        let jobs = self.jobs.into_inner();
        let started = Instant::now();
        let made: Vec<OnceLock<wgpu::RenderPipeline>> = jobs.iter().map(|_| OnceLock::new()).collect();
        let workers = std::thread::available_parallelism()
            .map_or(4, std::num::NonZero::get)
            .min(MAX_WORKERS)
            .min(jobs.len());
        let parent = crate::gpu_alloc::enclosing();
        let next = AtomicUsize::new(0);
        let mut ran = 0;
        std::thread::scope(|s| {
            for n in 0..workers {
                let (jobs, made, next, parent) = (&jobs, &made, &next, parent.as_deref());
                let spawned = std::thread::Builder::new()
                    .name(format!("aurora-pipelines-{n}"))
                    .spawn_scoped(s, move || {
                        crate::gpu_alloc::on_worker(device, parent, || {
                            loop {
                                let i = next.fetch_add(1, Ordering::Relaxed);
                                let (Some(job), Some(slot)) = (jobs.get(i), made.get(i)) else {
                                    break;
                                };
                                let _ = slot.set(job.make(device));
                            }
                        });
                    });
                match spawned {
                    Ok(_) => ran += 1,
                    Err(e) => {
                        log::warn!("pipelines: cannot start worker {n}: {e}");
                        break;
                    }
                }
            }
        });
        for (job, slot) in jobs.iter().zip(&made) {
            if slot.get().is_none() {
                let _ = slot.set(job.make(device));
            }
        }
        log::info!(
            "pipelines: {} made on {ran} threads in {:.0} ms",
            jobs.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );
        Done(made.into_iter().map(OnceLock::into_inner).collect())
    }
}

/// What a batch made, each taken once by its slot.
pub(super) struct Done(Vec<Option<wgpu::RenderPipeline>>);

/// A slot, or a value holding slots, turned into what the batch made.
pub(super) trait Resolve {
    type Out;
    fn resolve(self, done: &mut Done) -> Self::Out;
}

impl Resolve for Slot {
    type Out = wgpu::RenderPipeline;
    fn resolve(self, done: &mut Done) -> wgpu::RenderPipeline {
        done.0[self.0].take().expect("a batch's slot is taken once, after its run")
    }
}

/// A pipeline made outside the batch (its layout or module lived only in
/// the closure that made it).
impl Resolve for wgpu::RenderPipeline {
    type Out = Self;
    fn resolve(self, _: &mut Done) -> Self {
        self
    }
}

impl<T: Resolve, const N: usize> Resolve for [T; N] {
    type Out = [T::Out; N];
    fn resolve(self, done: &mut Done) -> Self::Out {
        self.map(|t| t.resolve(done))
    }
}

impl<T: Resolve> Resolve for Option<T> {
    type Out = Option<T::Out>;
    fn resolve(self, done: &mut Done) -> Self::Out {
        self.map(|t| t.resolve(done))
    }
}

#[cfg(test)]
mod tests {
    use super::super::Vb;
    use super::*;

    #[test]
    fn a_description_keeps_its_texts_and_constants_once_owned() {
        // the constants of a closure's temporary, as `blend_pipe` writes them
        let owned = {
            let early = true;
            let constants = [("BLEND_EARLY_DISCARD", if early { 1.0 } else { 0.0 }), ("WATER_CLIP", 2.0)];
            Owned::new(PipeDesc {
                label: "blend",
                vs: "vs_main",
                fs: Some("fs_blend"),
                vb: Vb::Mesh,
                cull: Some(wgpu::Face::Back),
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
                depth_write: false,
                depth_compare: wgpu::CompareFunction::GreaterEqual,
                a2c: false,
                samples: 4,
                format: Some(wgpu::TextureFormat::Rgba16Float),
                gbuf: true,
                range: None,
                depth_format: Some(wgpu::TextureFormat::Depth32Float),
                depth_bias: Default::default(),
                constants: &constants,
                polygon_line: false,
                unclipped_depth: true,
            })
        };
        assert_eq!(owned.label, "blend");
        assert_eq!(owned.vs, "vs_main");
        assert_eq!(owned.fs.as_deref(), Some("fs_blend"));
        assert_eq!(
            owned.constants,
            vec![("BLEND_EARLY_DISCARD".to_owned(), 1.0), ("WATER_CLIP".to_owned(), 2.0)]
        );
        // the rest as written
        assert_eq!(owned.rest.samples, 4);
        assert!(owned.rest.gbuf && owned.rest.unclipped_depth && !owned.rest.depth_write);
        assert_eq!(owned.rest.cull, Some(wgpu::Face::Back));
        assert_eq!(owned.rest.format, Some(wgpu::TextureFormat::Rgba16Float));
    }
}

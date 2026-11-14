//! Exact blend order (`RiggedBlendDepth::Exact`): what the pass of the
//! blended faces and the pass over the screen share, the front and the haze
//! of each pixel.
//!
//! Sorting faces back to front is right only while no two of them cross,
//! no face folds over itself and every face is one layer; mesh hair, worn
//! layers, crossing panes and the triangles of one face break all three.
//! Here the blended faces are drawn once (`fs_blend_once`, object.wgsl):
//! each fragment the opaque scene leaves visible is lit and files where it
//! lies, its alpha and its light in the front of its pixel. A pass over the
//! screen (`fs_blend_over`) then sorts the front of each
//! pixel, gives each fragment the share of light the fragments in front of
//! it let through, and lays the sum over what was drawn. The result is the
//! one a sort of the fragments of each pixel would give. The glow of a
//! fragment is kept and dimmed with it (`fs_blend_glow`).
//!
//! The front of a pixel is every strong fragment that comes to it, each in
//! the slot of its rank of arrival: no fragment reads another, and none is
//! moved. A pixel has `SLOTS` of its own, and takes pages of as many from a
//! pool the view shares, as many as it has fragments. The pass over the
//! screen composes the nearest of them (`SLOTS` x (1 + `PAGES`) at most)
//! each in its place, and the others, in a pixel that has more, as one
//! layer behind those: which are kept is read from all of them, one pixel
//! at a time, and does not change from a view to the next. A fragment too
//! faint to take a slot, or for which the pool has no page left, is of the
//! haze of its pixel: it writes no word of the fronts, the blending of the
//! pass sums it into two targets (`Lists::haze`, `Lists::range`), and the
//! pass over the screen composes the haze as one layer in which its
//! fragments are mixed (see the shader). The faces are drawn nearest first,
//! so that most fragments come behind those that hide them.
//!
//! How faint a fragment is of the haze is the precision the user chose
//! (`BlendPrecision`, `Info::faint`): 3 % of alpha at the finest, where the
//! picture is the one of the sort, up to 25 %, which files a third fewer
//! fragments in a busy region. The pool starts at one page for four pixels
//! and follows what the views take of it (`pool_wanted`): doubled as soon
//! as three quarters are taken, halved after some ten seconds under a
//! quarter. Light that is not a number is kept out of the fronts and of the
//! sums (`blend_finite` of the shader): one such fragment would otherwise
//! blacken its pixel, or spread through the bloom.
//!
//! A pixel also has its cut, the place under which its fragments let
//! nothing through: between runs of faces it goes into the depth of the
//! pass, and what lies under it is left out before its shader.
//!
//! Before each view the heads of the pixels and the count of the pool are
//! emptied (`Lists::clear`), which a device does at the speed of its
//! memory. The slots keep what the view before left: the count of a pixel
//! says which are of this view.
//!
//! The pass needs the depth test before its shader, which is a device
//! feature (`EARLY_TEST`): only what shows may be filed. Without it the
//! faces are drawn sorted, as without the exact order.

/// The depth test before the fragment shader, where the device has it.
pub(crate) const EARLY_TEST: wgpu::Features = wgpu::Features::SHADER_EARLY_DEPTH_TEST;

/// Blending from two sources, where the device has it: the pass over the
/// screen is then one draw (`fs_blend_both`), light and glow together.
pub(crate) const TWO_SOURCES: wgpu::Features = wgpu::Features::DUAL_SOURCE_BLENDING;

/// `fs_blend_over` and `fs_blend_glow` of object.wgsl in one draw: the
/// light and the glow of the blended fragments of a pixel, over what is
/// drawn so far scaled by what they let through together (the second
/// source). Not in object.wgsl: a shader says first that it blends from two
/// sources, which only the devices that can may read.
pub(crate) const BOTH_SOURCE: &str = "
struct BlendBoth {
    @location(0) @blend_src(0) light: vec4<f32>,
    @location(0) @blend_src(1) through: vec4<f32>,
};

@fragment
fn fs_blend_both(@builtin(position) clip: vec4<f32>) -> BlendBoth {
    let composed = blend_compose(clip, true);
    if (composed.through >= 1.0 && composed.glow <= 0.0 && all(composed.light <= vec3<f32>(0.0))) {
        discard;
    }
    return BlendBoth(vec4<f32>(composed.light, composed.glow), vec4<f32>(composed.through));
}
";

/// The scene shader as the device takes it: without the attribute of the
/// early depth test where the device does not know it, and with the one
/// draw over the screen where the device blends from two sources.
pub(crate) fn scene_source(source: String, features: wgpu::Features) -> String {
    let source = if features.contains(EARLY_TEST) {
        source
    } else {
        source.replace("@early_depth_test(force)", "")
    };
    if features.contains(TWO_SOURCES) {
        // (a directive: before everything else)
        format!("enable dual_source_blending;\n{source}\n{BOTH_SOURCE}")
    } else {
        source
    }
}

/// The blend of the one draw over the screen: its first output over what
/// is drawn, scaled by its second, colour and alpha alike.
pub(crate) fn both_blend() -> wgpu::BlendState {
    let over = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::Src1,
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState {
        color: over,
        alpha: wgpu::BlendComponent {
            dst_factor: wgpu::BlendFactor::Src1Alpha,
            ..over
        },
    }
}

/// Slots of a pixel's own, and of a page (`BLEND_SLOTS` of object.wgsl).
/// Most pixels of most views stay under it: six layers of mesh hair, or a
/// window behind a window behind a sheer dress over a body.
pub(crate) const SLOTS: u32 = 6;

/// Pages a pixel may take from the pool (`BLEND_PAGES` of object.wgsl): the
/// blended fragments composed in their exact order are `SLOTS` x (1 +
/// `PAGES`) a pixel at most, those past it are mixed behind them.
pub(crate) const PAGES: u32 = 6;

/// Words of a pixel, in planes of four: its head (its count, its cut, the
/// glows of its slots and its pages), then two for each slot (`blend_slot`
/// of object.wgsl).
const PIXEL_WORDS: u32 = 4 + 2 * SLOTS;

/// Words of a pixel that are emptied before each view: its head.
const PIXEL_WORDS_EMPTIED: u32 = 4;

/// Words of a page of the pool (`BLEND_PAGE_WORDS` of object.wgsl): two for
/// each slot, two of glows, the pages before it, and one to make a cache
/// line. The count of the pool takes the room of one page before them.
const PAGE_WORDS: u32 = 2 * SLOTS + 4;

/// The most pages a pool has: a pixel names a page in so many bits
/// (`BLEND_LINK_BITS` of object.wgsl).
const POOL_MOST: u64 = (1 << 25) - 1;

/// The first target of the pass of the blended faces (`BlendOnce` of
/// object.wgsl): the light of the haze, and its glow.
pub(crate) const HAZE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Blending into a target of 32-bit floats, where the device has it: the
/// second target of the pass then holds the places of the haze whole.
pub(crate) const WIDE_RANGE: wgpu::Features = wgpu::Features::FLOAT32_BLENDABLE;

/// The second target of the pass (`BlendOnce::range`): the nearest and the
/// farthest place of the haze of a pixel, and what it stops.
pub(crate) fn range_format(features: wgpu::Features) -> wgpu::TextureFormat {
    if features.contains(WIDE_RANGE) {
        wgpu::TextureFormat::Rgba32Float
    } else {
        wgpu::TextureFormat::Rgba16Float
    }
}

/// The low bits a place of the haze goes without in that target
/// (`BlendOrderInfo::shift`): a place is 24 bits, and a 16-bit float holds
/// the whole numbers up to 2048.
pub(crate) fn range_shift(format: wgpu::TextureFormat) -> u32 {
    if format == wgpu::TextureFormat::Rgba32Float { 0 } else { 13 }
}

/// How the pass of the blended faces blends into its first target: the
/// haze is summed.
pub(crate) fn once_blend() -> wgpu::BlendState {
    let add = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState { color: add, alpha: add }
}

/// And into its second: the greatest of the places, the sum of what is
/// stopped.
pub(crate) fn range_blend() -> wgpu::BlendState {
    let component = |operation| wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation,
    };
    wgpu::BlendState {
        color: component(wgpu::BlendOperation::Max),
        alpha: component(wgpu::BlendOperation::Add),
    }
}

/// Pages of the pool of a view of `pixels` at most: two for each pixel, so
/// that every pixel may hold three times its own slots, or one in three all
/// that a pixel may hold.
pub(crate) fn pool_pages(pixels: u64) -> u64 {
    (pixels.max(1) * 2).min(POOL_MOST)
}

/// And at first: one for four pixels (16 bytes a pixel). Most views take
/// none or a few; the pool then follows what the views take
/// (`pool_wanted`).
pub(crate) fn pool_start(pixels: u64) -> u64 {
    pixels.div_ceil(4).clamp(1, POOL_MOST)
}

/// Views in a row that took less than a quarter of their pool before it is
/// halved (some ten seconds).
const POOL_QUIET: u32 = 600;

/// The pool a view should have instead of its `pages`, when the last view
/// took `taken` of them: twice as many once three quarters are taken (the
/// count stops at the pool: a view that found it empty says no more than
/// that), half as many after `quiet` views in a row under a quarter. A
/// halved pool is then under half taken: it does not grow back at once.
pub(crate) fn pool_wanted(pages: u64, taken: u64, quiet: u32, least: u64, most: u64) -> Option<u64> {
    if taken * 4 >= pages * 3 && pages < most {
        Some((pages * 2).min(most))
    } else if quiet >= POOL_QUIET && pages > least {
        Some((pages / 2).max(least))
    } else {
        None
    }
}

/// Bytes of the fronts of a view of `pixels` with a pool of `pages`, when
/// one binding of the device holds them: the planes of the pixels, the
/// count of the pool, the pages.
pub(crate) fn fronts_size(pixels: u64, pages: u64, max_binding: u64) -> Option<u64> {
    let size = pixels.max(1) * u64::from(PIXEL_WORDS) * 4 + (pages + 1) * u64::from(PAGE_WORDS) * 4;
    // (the shader counts its words in 32 bits)
    (size <= max_binding && size / 4 <= u64::from(u32::MAX) && pages <= POOL_MOST).then_some(size)
}

/// Times the cut of the pixels goes into the depth of the pass of the
/// blended faces. Each cut is a draw over the screen: three of them, each
/// run twice the one before, leave out most of what is hidden (2.6 ms of
/// 28 in a busy region) where a fourth gains nothing more.
const CUTS: u32 = 3;

/// Ends of the runs of `n` blended faces, nearest first, between which the
/// cut of the pixels goes into the depth of the pass: each run twice the
/// one before, the nearest faces being those that hide the most.
pub(crate) fn runs(n: u32) -> Vec<u32> {
    let mut ends: Vec<u32> = (1..=CUTS)
        .map(|k| n.checked_shr(CUTS + 1 - k).unwrap_or(0))
        .filter(|end| *end > 0 && *end < n)
        .collect();
    ends.push(n);
    ends.dedup();
    ends
}

/// `BlendOrderInfo` of object.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Info {
    width: u32,
    height: u32,
    /// The word of the count of the pool, after the planes of the pixels.
    pool: u32,
    pages: u32,
    /// The most strong fragments of a pixel composed each in its place.
    most: u32,
    /// `range_shift` of the second target of the pass.
    shift: u32,
    /// `BlendPrecision::faint`.
    faint: u32,
}

/// What a view took of its pool, read back a few frames later.
struct Watch {
    readback: wgpu::Buffer,
    ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The count was copied this frame; its mapping was asked for.
    copied: bool,
    pending: bool,
    /// Pages the last view read took, and the views in a row that took
    /// less than a quarter of the pool.
    taken: u64,
    quiet: u32,
    /// The memory refused a larger pool: this one stays.
    refused: bool,
}

fn buffer_entry(binding: u32, visibility: wgpu::ShaderStages, ty: wgpu::BufferBindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Bind group layouts of group 3: the pass of the blended faces writes the
/// fronts, the pass over the screen reads them and the two targets of the
/// first. Two layouts, so that the buffer changes usage between the two and
/// the writes of the first are over before the second reads.
pub(crate) fn layouts(device: &wgpu::Device) -> [wgpu::BindGroupLayout; 2] {
    let stage = wgpu::ShaderStages::FRAGMENT;
    let info = buffer_entry(8, stage, wgpu::BufferBindingType::Uniform);
    let haze = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: stage,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    [
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blend order fronts (written)"),
            entries: &[info, buffer_entry(9, stage, wgpu::BufferBindingType::Storage { read_only: false })],
        }),
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blend order fronts (read)"),
            entries: &[
                info,
                buffer_entry(10, stage, wgpu::BufferBindingType::Storage { read_only: true }),
                haze(11),
                haze(12),
            ],
        }),
    ]
}

/// The fronts and the haze of one view size.
pub(crate) struct Lists {
    fronts: wgpu::Buffer,
    info: wgpu::Buffer,
    /// `Info::faint` as the uniform holds it.
    faint: std::sync::atomic::AtomicU32,
    /// The view, its pool, and what the views take of it.
    size: (u32, u32),
    pages: u64,
    watch: std::sync::Mutex<Watch>,
    /// Bytes of the heads, emptied before each view, and where the count of
    /// the pool is.
    emptied: u64,
    pool: u64,
    /// The targets of the pass of the blended faces.
    pub haze: wgpu::TextureView,
    pub range: wgpu::TextureView,
    /// The depth of the pass of the blended faces: the scene's, then the
    /// cut of each pixel as it rises (`fs_blend_cut`).
    pub depth_tex: wgpu::Texture,
    pub depth: wgpu::TextureView,
    /// Group 3 of the pass of the blended faces.
    pub write: wgpu::BindGroup,
    /// Group 3 of the pass over the screen.
    pub read: wgpu::BindGroup,
}

impl Lists {
    /// The fronts of a view of `size`; `None` when the device has no binding
    /// that large or no depth test before the shader (the blended faces are
    /// then drawn sorted, as without the exact order).
    pub fn new(device: &wgpu::Device, size: (u32, u32), depth_format: wgpu::TextureFormat) -> Option<Lists> {
        let pixels = u64::from(size.0) * u64::from(size.1);
        let max = device.limits().max_storage_buffer_binding_size;
        // (a pixel of more layers than its own slots needs a page: no pool,
        // no fronts)
        let pages = pool_start(pixels);
        fronts_size(pixels, pages, max)?;
        Lists::with_pool(device, size, depth_format, pages)
    }

    /// The view these fronts are of.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The pool these fronts should have instead of theirs, from what the
    /// last views took of it (`pool_wanted`), within what one binding of
    /// the device holds.
    pub fn pool_wanted(&self, device: &wgpu::Device) -> Option<u64> {
        let watch = self.watch.lock().ok()?;
        if watch.refused {
            return None;
        }
        let pixels = u64::from(self.size.0) * u64::from(self.size.1);
        let max = device.limits().max_storage_buffer_binding_size;
        let most = [pool_pages(pixels), pool_pages(pixels) / 2, pool_pages(pixels) / 4]
            .into_iter()
            .find(|pages| fronts_size(pixels, *pages, max).is_some())
            .unwrap_or(self.pages);
        pool_wanted(self.pages, watch.taken, watch.quiet, pool_start(pixels).min(most), most).filter(|pages| *pages != self.pages)
    }

    /// The memory had no room for another pool: this one is kept.
    pub fn pool_refused(&self) {
        if let Ok(mut watch) = self.watch.lock() {
            watch.refused = true;
        }
    }

    /// After the pass of the blended faces: the count of the pages it took,
    /// to be read (`after_submit`).
    pub fn count_pool(&self, encoder: &mut wgpu::CommandEncoder) {
        let Ok(mut watch) = self.watch.lock() else { return };
        if watch.pending || watch.copied {
            return;
        }
        encoder.copy_buffer_to_buffer(&self.fronts, self.pool, &watch.readback, 0, 8);
        watch.copied = true;
    }

    /// Once the frame is sent: the count copied by `count_pool` is asked
    /// for, and the one of an earlier frame read if it came.
    pub fn after_submit(&self) {
        use std::sync::atomic::Ordering;
        let Ok(mut watch) = self.watch.lock() else { return };
        if watch.pending && watch.ready.load(Ordering::Acquire) {
            let taken = watch
                .readback
                .get_mapped_range(..)
                .ok()
                .map(|view| u64::from(u32::from_le_bytes([view[0], view[1], view[2], view[3]])));
            if let Some(taken) = taken {
                watch.taken = taken;
                watch.quiet = if taken * 4 < self.pages { watch.quiet.saturating_add(1) } else { 0 };
            }
            watch.readback.unmap();
            watch.pending = false;
            watch.ready.store(false, Ordering::Release);
        }
        if watch.copied {
            watch.copied = false;
            watch.pending = true;
            let ready = watch.ready.clone();
            watch.readback.map_async(wgpu::MapMode::Read, .., move |r| {
                if r.is_ok() {
                    ready.store(true, Ordering::Release);
                }
            });
        }
    }

    /// The faintest alpha that takes a slot in the views to come
    /// (`BlendPrecision::faint`).
    pub fn set_faint(&self, queue: &wgpu::Queue, faint: u32) {
        if self.faint.swap(faint, std::sync::atomic::Ordering::Relaxed) != faint {
            queue.write_buffer(&self.info, 24, bytemuck::bytes_of(&faint));
        }
    }

    /// As `new`, with a pool of `pages`.
    pub fn with_pool(device: &wgpu::Device, size: (u32, u32), depth_format: wgpu::TextureFormat, pages: u64) -> Option<Lists> {
        if !device.features().contains(EARLY_TEST) {
            return None;
        }
        let pixels = u64::from(size.0) * u64::from(size.1);
        let bytes = fronts_size(pixels, pages, device.limits().max_storage_buffer_binding_size)?;
        let emptied = pixels.max(1) * u64::from(PIXEL_WORDS_EMPTIED) * 4;
        let pool = pixels.max(1) * u64::from(PIXEL_WORDS) * 4;
        let range_format = range_format(device.features());
        let info = crate::gpu_alloc::buffer_init(
            device,
            "blend order info",
            bytemuck::bytes_of(&Info {
                width: size.0,
                height: size.1,
                pool: (pool / 4) as u32,
                pages: pages as u32,
                most: SLOTS * (1 + PAGES),
                shift: range_shift(range_format),
                faint: crate::BlendPrecision::default().faint(),
            }),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        // a new buffer is all zeros: every front empty
        let fronts = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blend order fronts"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let target = |label, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.0.max(1),
                        height: size.1.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let haze = target("blend order haze", HAZE_FORMAT);
        let range = target("blend order range", range_format);
        let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blend order depth"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: depth_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let depth = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let [write_layout, read_layout] = layouts(device);
        let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
        let write = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blend order fronts (written)"),
            layout: &write_layout,
            entries: &[entry(8, info.as_entire_binding()), entry(9, fronts.as_entire_binding())],
        });
        let read = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blend order fronts (read)"),
            layout: &read_layout,
            entries: &[
                entry(8, info.as_entire_binding()),
                entry(10, fronts.as_entire_binding()),
                entry(11, wgpu::BindingResource::TextureView(&haze)),
                entry(12, wgpu::BindingResource::TextureView(&range)),
            ],
        });
        let watch = Watch {
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("blend order pool count"),
                size: crate::readback::readback_size(8),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ready: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            copied: false,
            pending: false,
            taken: 0,
            quiet: 0,
            refused: false,
        };
        Some(Lists {
            fronts,
            info,
            faint: std::sync::atomic::AtomicU32::new(crate::BlendPrecision::default().faint()),
            size,
            pages,
            watch: std::sync::Mutex::new(watch),
            emptied,
            pool,
            haze,
            range,
            depth_tex,
            depth,
            write,
            read,
        })
    }

    /// Empty the fronts before the blended faces of a view: the heads of
    /// the pixels and the count of the pool, which the device does at the
    /// speed of its memory. (Emptying only the pixels the last view used,
    /// in a compute pass, costs twenty times that as soon as most of them
    /// were.)
    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.clear_buffer(&self.fronts, 0, Some(self.emptied));
        encoder.clear_buffer(&self.fronts, self.pool, Some(u64::from(PAGE_WORDS) * 4));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_early_test_leaves_the_shader_where_the_device_lacks_it() {
        let source = include_str!("shaders/object.wgsl");
        assert_eq!(
            source.matches("@early_depth_test(force)").count(),
            1,
            "the pass of the blended faces alone"
        );
        assert!(scene_source(source.to_string(), EARLY_TEST).contains("@early_depth_test"));
        assert!(!scene_source(source.to_string(), wgpu::Features::empty()).contains("@early_depth_test"));
        // the one draw over the screen, and what it needs of the device
        let two = scene_source(source.to_string(), TWO_SOURCES);
        assert!(two.starts_with("enable dual_source_blending;"));
        assert!(two.contains("fn fs_blend_both("));
        assert_eq!(scene_source(source.to_string(), EARLY_TEST), source);
        assert!(!source.contains("@blend_src"), "other shaders take object.wgsl as it is");
    }

    #[test]
    fn the_fronts_fit_one_binding_or_are_not_made() {
        // 1440p and 4K with the bindings of a desktop card: 64 bytes a
        // pixel, 64 a page, two pages a pixel, and the count of the pool
        let (qhd, uhd) = (2560 * 1440, 3840 * 2160);
        assert_eq!(fronts_size(qhd, pool_pages(qhd), 1 << 31), Some(qhd * 192 + 64));
        assert_eq!(fronts_size(uhd, pool_pages(uhd), 1 << 31), Some(uhd * 192 + 64));
        assert_eq!(fronts_size(uhd, 0, 1 << 31), Some(uhd * 64 + 64));
        // 4K with bindings of 128 MB
        assert_eq!(fronts_size(uhd, 0, 128 << 20), None);
        // a pixel names a page in 25 bits
        assert_eq!(pool_pages(1 << 30), POOL_MOST);
        assert_eq!(fronts_size(1, POOL_MOST + 1, u64::MAX), None);
        // at first 16 bytes of pool a pixel
        assert_eq!(fronts_size(qhd, pool_start(qhd), 1 << 31), Some(qhd * 80 + 64));
        assert_eq!(pool_start(0), 1);
    }

    #[test]
    fn the_pool_follows_what_the_views_take() {
        let (least, most) = (1000, 8000);
        // under three quarters: as it is
        assert_eq!(pool_wanted(1000, 0, 0, least, most), None);
        assert_eq!(pool_wanted(1000, 749, 0, least, most), None);
        // from three quarters: twice as many, up to the most
        assert_eq!(pool_wanted(1000, 750, 0, least, most), Some(2000));
        assert_eq!(pool_wanted(1000, 1000, 0, least, most), Some(2000));
        assert_eq!(pool_wanted(6000, 6000, 0, least, most), Some(8000));
        assert_eq!(pool_wanted(8000, 8000, 0, least, most), None);
        // quiet for long: half as many, down to the least, and not at once
        assert_eq!(pool_wanted(4000, 900, POOL_QUIET - 1, least, most), None);
        assert_eq!(pool_wanted(4000, 900, POOL_QUIET, least, most), Some(2000));
        assert_eq!(pool_wanted(1500, 0, POOL_QUIET, least, most), Some(1000));
        assert_eq!(pool_wanted(1000, 0, POOL_QUIET, least, most), None);
        // a halved pool does not grow back at once: under a quarter of the
        // old one is under half of the new one
        assert_eq!(pool_wanted(2000, 999, 0, least, most), None);
        // every level of precision keeps the faintest fragments out, and
        // each coarser one more of them
        use crate::BlendPrecision::{Balanced, Fast, Full, High};
        let faint: Vec<u32> = [Full, High, Balanced, Fast].into_iter().map(crate::BlendPrecision::faint).collect();
        assert!(faint.windows(2).all(|pair| pair[0] < pair[1]), "{faint:?}");
        let source = include_str!("shaders/object.wgsl");
        assert!(source.contains(&format!("const BLEND_FAINT: u32 = {}u;", Full.faint())));
    }

    #[test]
    fn the_shader_agrees_on_the_fronts() {
        let source = include_str!("shaders/object.wgsl");
        let at = source.find("struct BlendOrderInfo").expect("the struct");
        let body = &source[at..at + source[at..].find("};").expect("its end")];
        let fields: Vec<_> = body.lines().filter_map(|line| line.trim().strip_suffix(": u32,")).collect();
        assert_eq!(fields, ["width", "height", "pool", "pages", "most", "shift", "faint"]);
        assert_eq!(fields.len() * 4, std::mem::size_of::<Info>());
        assert!(source.contains(&format!("const BLEND_SLOTS: u32 = {SLOTS}u;")));
        assert!(source.contains(&format!("const BLEND_PAGES: u32 = {PAGES}u;")));
        assert!(source.contains(&format!("const BLEND_MOST: u32 = {}u;", SLOTS * (1 + PAGES))));
        assert!(source.contains(&format!("const BLEND_PAGE_WORDS: u32 = {PAGE_WORDS}u;")));
        assert!(source.contains(&format!("const BLEND_LINK_MASK: u32 = {POOL_MOST:#010x}u;")));
        // four planes of four words, the heads first: they alone are
        // emptied
        assert_eq!((PIXEL_WORDS, PIXEL_WORDS_EMPTIED), (16, 4));
        assert!(source.contains("let head = pixel * 4u;"));
        assert!(source.contains("return (pixels + pixel) * 4u + s;"));
        assert!(source.contains("return (2u * pixels + pixel) * 4u + s - 4u;"));
        // a place of the haze, whole or without its low bits, is a whole
        // number its target holds
        assert_eq!(range_shift(range_format(WIDE_RANGE)), 0);
        assert_eq!(range_format(wgpu::Features::empty()), HAZE_FORMAT);
        assert!(0x00ff_ffff_u32 >> range_shift(HAZE_FORMAT) < 2048);
    }
}

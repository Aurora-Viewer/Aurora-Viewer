//! Exact blend order (`RiggedBlendDepth::Exact`): what the two passes of
//! the blended faces share, the front of each pixel.
//!
//! Sorting faces back to front is right only while no two of them cross,
//! no face folds over itself and every face is one layer; mesh hair, worn
//! layers, crossing panes and the triangles of one face break all three.
//! Here nothing is sorted on the CPU. The visibility pass (`fs_blend_vis`,
//! object.wgsl) files every blended fragment the opaque scene leaves
//! visible in the front of its pixel, where it lies and its alpha; the
//! colour pass (`fs_blend_exact`) gives each fragment the share of light
//! the fragments in front of it let through, and adds it. The result is
//! the one a sort of the fragments of each pixel would give, in two passes
//! whatever the scene, and a fragment the layers in front of it hide
//! leaves before its material and its lighting.
//!
//! The front of a pixel is a fixed number of slots (`SLOTS`), its nearest
//! fragments in order, kept sorted by the fragments themselves with
//! `atomicMax` (see the shader): no list, no pool that a deep scene could
//! fill, no lock. The fragments under a full front are its tail, composed
//! as one layer of their mean colour under the front: two words per pixel.
//!
//! The first slot of every pixel comes first in the buffer: a pixel with
//! one blended layer touches that word alone, and the fronts are emptied
//! before each view only where it is set (`Clear`, blend_clear.wgsl), so
//! that a view with two panes of glass pays for the pixels of its two
//! panes. (A flag per tile of pixels, set by the fragments, costs more
//! than it saves: thousands of them share its word.)
//!
//! Only the visibility pass needs the depth test before its shader, which
//! is a device feature (`EARLY_TEST`): with it, only what shows is filed.
//! Without it (Metal) the hidden fragments are filed too, behind all that
//! shows: the result is the same while the front has slots left
//! (`scene_source`).

/// The depth test before the fragment shader, where the device has it.
pub(crate) const EARLY_TEST: wgpu::Features = wgpu::Features::SHADER_EARLY_DEPTH_TEST;

/// The scene shader as the device takes it: without the attribute of the
/// early depth test where the device does not know it.
pub(crate) fn scene_source(source: String, features: wgpu::Features) -> String {
    if features.contains(EARLY_TEST) {
        source
    } else {
        source.replace("@early_depth_test(force)", "")
    }
}

/// Slots of a pixel's front: the blended fragments composed in their exact
/// order. Eight layers of mesh hair, or a window behind a window behind a
/// sheer dress over a body, stay under it; past it the order is the tail's.
pub(crate) const SLOTS: u32 = 8;

/// Words of the tail of a pixel: the sum of its alphas, and of what it
/// stops.
const TAIL_WORDS: u32 = 2;

/// Pixels a group of threads of the clear covers, across and down
/// (blend_clear.wgsl: 8 x 8 threads, each on a run of 16 pixels).
const CLEAR_GROUP: (u32, u32) = (128, 8);

/// Slots a view of `pixels` gets: `SLOTS`, or what one binding can hold.
pub(crate) fn slots(pixels: u64, max_binding: u64) -> u32 {
    let words = max_binding / 4 / pixels.max(1);
    words.saturating_sub(u64::from(TAIL_WORDS)).clamp(1, u64::from(SLOTS)) as u32
}

/// `BlendOrderInfo` of object.wgsl and blend_clear.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Info {
    width: u32,
    height: u32,
    slots: u32,
    /// Words of a pixel after its first slot.
    rest: u32,
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

/// Bind group layouts of group 3: the visibility pass writes the fronts,
/// the colour pass reads them. Two layouts, so that the buffer changes
/// usage between the two passes and the writes of the first are over
/// before the second reads.
pub(crate) fn layouts(device: &wgpu::Device) -> [wgpu::BindGroupLayout; 2] {
    let stage = wgpu::ShaderStages::FRAGMENT;
    let info = buffer_entry(8, stage, wgpu::BufferBindingType::Uniform);
    [
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blend order fronts (written)"),
            entries: &[info, buffer_entry(9, stage, wgpu::BufferBindingType::Storage { read_only: false })],
        }),
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blend order fronts (read)"),
            entries: &[info, buffer_entry(10, stage, wgpu::BufferBindingType::Storage { read_only: true })],
        }),
    ]
}

/// The pass that empties the fronts (blend_clear.wgsl).
pub(crate) struct Clear {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}

pub(crate) const CLEAR_SHADER: &str = include_str!("shaders/blend_clear.wgsl");

impl Clear {
    pub fn new(device: &wgpu::Device) -> Clear {
        let stage = wgpu::ShaderStages::COMPUTE;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blend order clear"),
            entries: &[
                buffer_entry(0, stage, wgpu::BufferBindingType::Uniform),
                buffer_entry(1, stage, wgpu::BufferBindingType::Storage { read_only: false }),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blend order clear"),
            source: wgpu::ShaderSource::Wgsl(CLEAR_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blend order clear"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("blend order clear"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_blend_clear"),
            compilation_options: Default::default(),
            cache: None,
        });
        Clear { layout, pipeline }
    }
}

/// The fronts of one view size.
pub(crate) struct Lists {
    size: (u32, u32),
    clear: wgpu::BindGroup,
    /// Group 3 of the visibility pass.
    pub write: wgpu::BindGroup,
    /// Group 3 of the colour pass.
    pub read: wgpu::BindGroup,
}

impl Lists {
    pub fn new(device: &wgpu::Device, clear: &Clear, size: (u32, u32)) -> Lists {
        let pixels = (u64::from(size.0) * u64::from(size.1)).max(1);
        let slots = slots(pixels, u64::from(device.limits().max_storage_buffer_binding_size));
        let rest = slots - 1 + TAIL_WORDS;
        let info = crate::gpu_alloc::buffer_init(
            device,
            "blend order info",
            bytemuck::bytes_of(&Info {
                width: size.0,
                height: size.1,
                slots,
                rest,
            }),
            wgpu::BufferUsages::UNIFORM,
        );
        // a new buffer is all zeros: every front empty
        let fronts = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blend order fronts"),
            size: pixels * u64::from(1 + rest) * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let [write_layout, read_layout] = layouts(device);
        let group = |label, layout: &wgpu::BindGroupLayout, bindings: [u32; 2]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: bindings[0],
                        resource: info.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: bindings[1],
                        resource: fronts.as_entire_binding(),
                    },
                ],
            })
        };
        Lists {
            size,
            clear: group("blend order clear", &clear.layout, [0, 1]),
            write: group("blend order fronts (written)", &write_layout, [8, 9]),
            read: group("blend order fronts (read)", &read_layout, [8, 10]),
        }
    }

    /// Empty the fronts the last view filled, before the visibility pass
    /// of a view.
    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder, clear: &Clear) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("blend order clear"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&clear.pipeline);
        pass.set_bind_group(0, &self.clear, &[]);
        pass.dispatch_workgroups(
            self.size.0.div_ceil(CLEAR_GROUP.0).max(1),
            self.size.1.div_ceil(CLEAR_GROUP.1).max(1),
            1,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_early_test_leaves_the_shader_where_the_device_lacks_it() {
        let source = include_str!("shaders/object.wgsl");
        assert_eq!(source.matches("@early_depth_test(force)").count(), 1, "the visibility pass alone");
        assert!(scene_source(source.to_string(), EARLY_TEST).contains("@early_depth_test"));
        assert!(!scene_source(source.to_string(), wgpu::Features::empty()).contains("@early_depth_test"));
    }

    #[test]
    fn the_front_keeps_its_slots_within_the_binding() {
        // 1080p and 4K with the bindings of a desktop card
        assert_eq!(slots(1920 * 1080, 1 << 31), SLOTS);
        assert_eq!(slots(3840 * 2160, 1 << 31), SLOTS);
        // 4K with bindings of 128 MB: 4 words per pixel, 2 of them slots
        assert_eq!(slots(3840 * 2160, 128 << 20), 2);
        // never none, however small the binding
        assert_eq!(slots(3840 * 2160, 1 << 20), 1);
    }

    #[test]
    fn both_shaders_agree_on_the_info() {
        let fields = |source: &str| {
            let at = source.find("struct BlendOrderInfo").expect("the struct");
            let body = &source[at..at + source[at..].find("};").expect("its end")];
            body.lines()
                .filter_map(|line| line.trim().strip_suffix(": u32,").map(str::to_string))
                .collect::<Vec<_>>()
        };
        let object = fields(include_str!("shaders/object.wgsl"));
        assert_eq!(object, ["width", "height", "slots", "rest"]);
        assert_eq!(object, fields(CLEAR_SHADER));
        assert_eq!(object.len() * 4, std::mem::size_of::<Info>());
        assert!(CLEAR_SHADER.contains("@workgroup_size(8, 8)") && CLEAR_SHADER.contains("const RUN: u32 = 16u;"));
        assert_eq!(CLEAR_GROUP, (8 * 16, 8));
    }
}

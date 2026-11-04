//! Workaround for naga's SPIR-V `sample_mask` built-in, on Vulkan.
//!
//! naga 30.0.1 (gfx-rs/wgpu) writes a WGSL `@builtin(sample_mask)` input or
//! output of a fragment shader as a scalar `u32` variable decorated
//! `BuiltIn SampleMask`. Vulkan's SPIR-V environment wants that built-in as
//! an array of 32-bit integers (one `u32` per 32 samples; the Vulkan spec,
//! "Built-In Variables", `SampleMask`), and the validation layer reports
//! VUID-SampleMask-SampleMask-04359 ("BuiltIn SampleMask variable needs to
//! be a 32-bit int array") for every such variable at `vkCreateShaderModule`.
//! NVIDIA's driver tolerates the scalar; another may not. naga's HLSL and
//! MSL writers are right already (`SV_Coverage` and `[[sample_mask]]` are
//! scalars there; a test below checks them), so this only concerns Vulkan.
//!
//! Rather than vendoring naga, this module takes a shader that uses the
//! built-in through the route wgpu-hal would take, then fixes the words:
//!
//! 1. the WGSL is parsed and validated with naga, and written to SPIR-V with
//!    the options wgpu-hal 30.0.1's Vulkan backend uses (`writer_options`
//!    says which and why);
//! 2. [`rewrite`] turns each `SampleMask` variable into an `array<u32, 1>`
//!    (its length constant, array type, pointer type) and routes every
//!    `OpLoad` / `OpStore` of it through an `OpAccessChain` to element 0,
//!    the shape of GLSL's `gl_SampleMaskIn[0]` and `gl_SampleMask[0]`;
//! 3. the words go to the device through
//!    `Device::create_shader_module_passthrough`
//!    (`Features::PASSTHROUGH_SHADERS`); without the feature, or when a
//!    step fails, the plain `create_shader_module` is used and it is logged
//!    once.
//!
//! **Self-retiring.** When naga's output already declares the built-in as
//! an array (a naga with the fix), the words are left alone and an info
//! line says this module can be deleted. Delete it then: the callers go
//! back to `device.create_shader_module`, and `naga` and `spirv` leave
//! `Cargo.toml`.
//!
//! Its user is the stochastic transparency's depth pass
//! (`oit_stochastic.rs`, `Shared::mask_module`), which takes the sample
//! index route instead when no module can be made here.
//!
//! Upstream: the bug is in `naga/src/back/spv/writer.rs`
//! (`write_varying`, the `BuiltIn::SampleMask` arm takes the varying's
//! own type); a report draft sits with the task's notes.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use naga::back::spv;

/// `PASSTHROUGH_SHADERS` when the adapter has it: the feature the workaround
/// needs, to add to the device's requested features.
pub fn feature(adapter_features: wgpu::Features) -> wgpu::Features {
    adapter_features & wgpu::Features::PASSTHROUGH_SHADERS
}

/// What wgpu-hal's Vulkan SPIR-V options derive from, read once from the
/// device (`VulkanFacts::query`); a plain value so that the option mirror
/// is testable without a GPU.
#[derive(Clone, Debug)]
pub struct VulkanFacts {
    /// `VkPhysicalDeviceProperties::apiVersion`.
    pub device_api_version: u32,
    /// The Vulkan instance's version, as wgpu-hal created it.
    pub instance_api_version: u32,
    /// `VkPhysicalDeviceProperties::vendorID`.
    pub vendor_id: u32,
    /// The flags the instance was created with (`DEBUG` matters).
    pub instance_flags: wgpu::InstanceFlags,
    /// The features the device was opened with.
    pub features: wgpu::Features,
    /// The adapter's downlevel flags.
    pub downlevel: wgpu::DownlevelFlags,
    /// The limits the device was opened with (the mesh dispatch limits).
    pub max_mesh_workgroups_per_dimension: u32,
    pub max_mesh_workgroup_total_count: u32,
    /// `VK_KHR_shader_non_semantic_info` is enabled on the device.
    pub non_semantic_info: bool,
    /// `VkPhysicalDeviceRobustness2FeaturesEXT::robustBufferAccess2`.
    pub robust_buffer_access2: bool,
    /// `robustImageAccess2` (robustness2) or, without that extension,
    /// `VkPhysicalDeviceImageRobustnessFeatures::robustImageAccess`.
    pub robust_image_access: bool,
    /// `shaderZeroInitializeWorkgroupMemory` (Vulkan 1.3 or its extension).
    pub zero_initialize_workgroup_memory: bool,
    /// `VkPhysicalDevice16BitStorageFeatures::storageInputOutput16`.
    pub storage_input_output_16: bool,
    /// `shaderIntegerDotProduct` (Vulkan 1.3 or its extension).
    pub shader_integer_dot_product: bool,
    /// `VkPhysicalDeviceShaderFloat16Int8Features::shaderInt8`.
    pub shader_int8: bool,
}

impl VulkanFacts {
    /// Reads the facts from the device's Vulkan objects. `None` on another
    /// backend.
    #[cfg(not(target_os = "macos"))]
    pub fn query(adapter: &wgpu::Adapter, device: &wgpu::Device, instance_flags: wgpu::InstanceFlags) -> Option<Self> {
        use ash::vk;
        // SAFETY: the hal adapter and device are only borrowed for this
        // function; the physical device queries change no state.
        let hal_adapter = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }?;
        // SAFETY: as above.
        let hal_device = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }?;
        let shared = hal_adapter.shared_instance();
        let instance = shared.raw_instance();
        let pd = hal_adapter.raw_physical_device();
        // SAFETY: physical device queries, valid from any thread.
        let properties = unsafe { instance.get_physical_device_properties(pd) };
        let device_api_version = properties.api_version;
        let instance_api_version = shared.instance_api_version();
        // SAFETY: as above.
        let supported = unsafe { instance.enumerate_device_extension_properties(pd) }.unwrap_or_default();
        let supports = |name: &std::ffi::CStr| supported.iter().any(|e| e.extension_name_as_c_str() == Ok(name));
        let enabled = hal_device.enabled_device_extensions();

        // The feature structures wgpu-hal chains into its own query
        // (`PhysicalDeviceFeatures::from_extensions_and_requested_features`
        // and `Adapter::open`'s private capabilities), under the same
        // conditions: a structure of an extension the device does not have
        // may not be chained. vkGetPhysicalDeviceFeatures2 needs a 1.1
        // instance and device; a 1.0 one keeps every answer at "no", which
        // picks wgpu-hal's conservative choices.
        let mut robustness2 = vk::PhysicalDeviceRobustness2FeaturesEXT::default();
        let mut image_robustness = vk::PhysicalDeviceImageRobustnessFeatures::default();
        let mut zero_init = vk::PhysicalDeviceZeroInitializeWorkgroupMemoryFeatures::default();
        let mut storage_16 = vk::PhysicalDevice16BitStorageFeatures::default();
        let mut dot_product = vk::PhysicalDeviceShaderIntegerDotProductFeatures::default();
        let mut float16_int8 = vk::PhysicalDeviceShaderFloat16Int8Features::default();
        if instance_api_version >= vk::API_VERSION_1_1 && device_api_version >= vk::API_VERSION_1_1 {
            let mut features = vk::PhysicalDeviceFeatures2::default();
            let has_robustness2 = supports(ash::ext::robustness2::NAME);
            if has_robustness2 {
                features = features.push_next(&mut robustness2);
            }
            if supports(ash::ext::image_robustness::NAME) {
                features = features.push_next(&mut image_robustness);
            }
            if device_api_version >= vk::API_VERSION_1_3 || supports(ash::khr::zero_initialize_workgroup_memory::NAME) {
                features = features.push_next(&mut zero_init);
            }
            if supports(ash::khr::_16bit_storage::NAME) {
                features = features.push_next(&mut storage_16);
            }
            if device_api_version >= vk::API_VERSION_1_3 || supports(ash::khr::shader_integer_dot_product::NAME) {
                features = features.push_next(&mut dot_product);
            }
            if device_api_version >= vk::API_VERSION_1_2 || supports(ash::khr::shader_float16_int8::NAME) {
                features = features.push_next(&mut float16_int8);
            }
            // SAFETY: as above; the entry point exists (versions checked).
            unsafe { instance.get_physical_device_features2(pd, &mut features) };
            // wgpu-hal reads robustImageAccess2 when robustness2 is there,
            // the image robustness feature otherwise.
            robustness2.robust_image_access2 = if has_robustness2 {
                robustness2.robust_image_access2
            } else {
                image_robustness.robust_image_access
            };
        }
        Some(Self {
            device_api_version,
            instance_api_version,
            vendor_id: properties.vendor_id,
            instance_flags,
            features: device.features(),
            downlevel: adapter.get_downlevel_capabilities().flags,
            max_mesh_workgroups_per_dimension: device.limits().max_mesh_workgroups_per_dimension,
            max_mesh_workgroup_total_count: device.limits().max_mesh_workgroup_total_count,
            non_semantic_info: enabled.contains(&ash::khr::shader_non_semantic_info::NAME),
            robust_buffer_access2: robustness2.robust_buffer_access2 == vk::TRUE,
            robust_image_access: robustness2.robust_image_access2 == vk::TRUE,
            zero_initialize_workgroup_memory: zero_init.shader_zero_initialize_workgroup_memory == vk::TRUE,
            storage_input_output_16: storage_16.storage_input_output16 == vk::TRUE,
            shader_integer_dot_product: dot_product.shader_integer_dot_product == vk::TRUE,
            shader_int8: float16_int8.shader_int8 == vk::TRUE,
        })
    }

    /// macOS draws through Metal: nothing to fix there.
    #[cfg(target_os = "macos")]
    pub fn query(_adapter: &wgpu::Adapter, _device: &wgpu::Device, _instance_flags: wgpu::InstanceFlags) -> Option<Self> {
        None
    }
}

/// `VkPhysicalDeviceProperties::vendorID` of Qualcomm (wgpu-hal
/// `auxil::db::qualcomm::VENDOR`), whose driver dislikes named varyings.
const QUALCOMM: u32 = 0x5143;

/// `VK_API_VERSION_1_1`, `1_2`, `1_3`, as ash packs them (no variant bits).
const fn api_version(minor: u32) -> u32 {
    (1 << 22) | (minor << 12)
}

/// The SPIR-V options wgpu-hal 30.0.1's Vulkan backend writes shaders with,
/// built the way `Adapter::device_from_raw` (src/vulkan/adapter.rs,
/// `naga_options`), `Device::create_shader_module` and `compile_stage`
/// (src/vulkan/device.rs) do. Each field says where it comes from; the
/// values wgpu-hal takes from the shader descriptor are those of a plain
/// `create_shader_module` (`ShaderRuntimeChecks::checked()`,
/// `zero_initialize_workgroup_memory: true`).
///
/// wgpu-hal writes one SPIR-V module per entry point at pipeline creation
/// (`Workarounds::SEPARATE_ENTRY_POINTS`, unconditionally on in 30.0.1:
/// "TODO: only enable for particular devices"). This module writes all the
/// entry points of the WGSL at once (`pipeline_options: None`, the shape
/// of wgpu-hal's other path), since a passthrough module is one Vulkan
/// shader module whose entry point the pipeline names: naga keeps the
/// same options for either, and a module with several entry points is
/// ordinary SPIR-V. Pipeline-overridable constants cannot be specialised
/// through a passthrough module, so a WGSL with `override`s is refused.
pub fn writer_options<'a>(facts: &VulkanFacts, binding_map: spv::BindingMap, debug_info: Option<spv::DebugInfo<'a>>) -> spv::Options<'a> {
    use spv::Capability as C;
    let features = facts.features;
    // The capabilities every Vulkan device has (the Vulkan spec, "SPIR-V
    // environment, capabilities"), then those of the device's features, in
    // wgpu-hal's order: naga picks the first available one of a choice.
    let mut capabilities = vec![
        C::Shader,
        C::Matrix,
        C::Sampled1D,
        C::Image1D,
        C::ImageQuery,
        C::DerivativeControl,
        C::StorageImageExtendedFormats,
    ];
    if facts.downlevel.contains(wgpu::DownlevelFlags::CUBE_ARRAY_TEXTURES) {
        capabilities.push(C::SampledCubeArray);
    }
    if facts.downlevel.contains(wgpu::DownlevelFlags::MULTISAMPLED_SHADING) {
        capabilities.push(C::SampleRateShading);
    }
    if features.contains(wgpu::Features::MULTIVIEW) {
        capabilities.push(C::MultiView);
    }
    if features.contains(wgpu::Features::PRIMITIVE_INDEX) {
        capabilities.push(C::Geometry);
    }
    if features.intersects(wgpu::Features::SUBGROUP | wgpu::Features::SUBGROUP_VERTEX) {
        capabilities.extend([
            C::GroupNonUniform,
            C::GroupNonUniformVote,
            C::GroupNonUniformArithmetic,
            C::GroupNonUniformBallot,
            C::GroupNonUniformShuffle,
            C::GroupNonUniformShuffleRelative,
            C::GroupNonUniformQuad,
        ]);
    }
    if features.intersects(
        wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
            | wgpu::Features::STORAGE_TEXTURE_ARRAY_NON_UNIFORM_INDEXING
            | wgpu::Features::UNIFORM_BUFFER_BINDING_ARRAYS,
    ) {
        capabilities.push(C::ShaderNonUniform);
    }
    if features.contains(wgpu::Features::BGRA8UNORM_STORAGE) {
        capabilities.push(C::StorageImageWriteWithoutFormat);
    }
    if features.contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY) {
        capabilities.push(C::RayQueryKHR);
    }
    if features.contains(wgpu::Features::SHADER_INT64) {
        capabilities.push(C::Int64);
    }
    if features.contains(wgpu::Features::SHADER_F16) {
        capabilities.push(C::Float16);
    }
    if features.contains(wgpu::Features::SHADER_I16) {
        capabilities.push(C::Int16);
    }
    if features.intersects(
        wgpu::Features::SHADER_INT64_ATOMIC_ALL_OPS | wgpu::Features::SHADER_INT64_ATOMIC_MIN_MAX | wgpu::Features::TEXTURE_INT64_ATOMIC,
    ) {
        capabilities.push(C::Int64Atomics);
    }
    if features.intersects(wgpu::Features::TEXTURE_INT64_ATOMIC) {
        capabilities.push(C::Int64ImageEXT);
    }
    if features.contains(wgpu::Features::SHADER_FLOAT32_ATOMIC) {
        capabilities.push(C::AtomicFloat32AddEXT);
    }
    if features.contains(wgpu::Features::CLIP_DISTANCES) {
        capabilities.push(C::ClipDistance);
    }
    if features.intersects(wgpu::Features::SHADER_BARYCENTRICS | wgpu::Features::SHADER_PER_VERTEX) {
        capabilities.push(C::FragmentBarycentricKHR);
    }
    if features.contains(wgpu::Features::SHADER_DRAW_INDEX) {
        capabilities.push(C::DrawParameters);
    }
    // wgpu-hal pushes RayQueryKHR a second time here; a set absorbs it.
    if features.contains(wgpu::Features::EXPERIMENTAL_RAY_TRACING_PIPELINES) {
        capabilities.push(C::RayTracingKHR);
    }
    if features.contains(wgpu::Features::EXPERIMENTAL_RAY_HIT_VERTEX_RETURN) {
        capabilities.push(C::RayQueryPositionFetchKHR);
    }
    if features.contains(wgpu::Features::EXPERIMENTAL_MESH_SHADER) {
        capabilities.push(C::MeshShadingEXT);
    }
    if features.contains(wgpu::Features::EXPERIMENTAL_COOPERATIVE_MATRIX) {
        capabilities.extend([C::CooperativeMatrixKHR, C::VulkanMemoryModel]);
    }
    if facts.shader_integer_dot_product {
        capabilities.extend([
            C::DotProductInputAllKHR,
            C::DotProductInput4x8BitKHR,
            C::DotProductInput4x8BitPackedKHR,
            C::DotProductKHR,
        ]);
    }
    if facts.shader_int8 {
        capabilities.push(C::Int8);
    }

    let debug = facts.instance_flags.contains(wgpu::InstanceFlags::DEBUG);
    let mut flags = spv::WriterFlags::empty();
    // OpName and OpSource when the instance is a debugging one.
    flags.set(spv::WriterFlags::DEBUG, debug);
    // Named varyings, except on Qualcomm.
    flags.set(spv::WriterFlags::LABEL_VARYINGS, facts.vendor_id != QUALCOMM);
    // Always, since the topology is unknown when the module is written.
    flags.set(spv::WriterFlags::FORCE_POINT_SIZE, true);
    // Debug printf on ray tracing failures: a debugging instance that has
    // the non-semantic info extension (core from 1.3).
    flags.set(
        spv::WriterFlags::PRINT_ON_RAY_QUERY_INITIALIZATION_FAIL | spv::WriterFlags::PRINT_ON_TRACE_RAYS_FAIL,
        debug && (facts.instance_api_version >= api_version(3) || facts.non_semantic_info),
    );

    spv::Options {
        // The highest SPIR-V each Vulkan version accepts (the Vulkan spec,
        // appendix "SPIR-V environment").
        lang_version: match facts.device_api_version {
            v if v < api_version(1) => (1, 0),
            v if v < api_version(2) => (1, 3),
            v if v < api_version(3) => (1, 5),
            _ => (1, 6),
        },
        flags,
        capabilities: Some(capabilities.into_iter().collect()),
        // `ShaderRuntimeChecks::checked()`: the device's policies as they
        // are. Buffer and image loads are left to the driver's robustness
        // when it has it, clamped otherwise; binding arrays unchecked.
        bounds_check_policies: naga::proc::BoundsCheckPolicies {
            index: naga::proc::BoundsCheckPolicy::Restrict,
            buffer: if facts.robust_buffer_access2 {
                naga::proc::BoundsCheckPolicy::Unchecked
            } else {
                naga::proc::BoundsCheckPolicy::Restrict
            },
            image_load: if facts.robust_image_access {
                naga::proc::BoundsCheckPolicy::Unchecked
            } else {
                naga::proc::BoundsCheckPolicy::Restrict
            },
            binding_array: naga::proc::BoundsCheckPolicy::Unchecked,
        },
        // The driver's zero-initialisation when it has the feature, naga's
        // stores otherwise (`zero_initialize_workgroup_memory: true` of the
        // pipeline stage).
        zero_initialize_workgroup_memory: if facts.zero_initialize_workgroup_memory {
            spv::ZeroInitializeWorkgroupMemoryMode::Native
        } else {
            spv::ZeroInitializeWorkgroupMemoryMode::Polyfill
        },
        force_loop_bounding: true,
        ray_query_initialization_tracking: true,
        use_storage_input_output_16: features.contains(wgpu::Features::SHADER_F16) && facts.storage_input_output_16,
        // A resource missing from the map is an error, as for wgpu-hal.
        fake_missing_bindings: false,
        binding_map,
        // The WGSL as OpSource, on a debugging instance (wgpu-core sets
        // `debug_source` then, with the label as the file name).
        debug_info,
        task_dispatch_limits: Some(naga::back::TaskDispatchLimits {
            max_mesh_workgroups_per_dim: facts.max_mesh_workgroups_per_dimension,
            max_mesh_workgroups_total: facts.max_mesh_workgroup_total_count,
        }),
        mesh_shader_primitive_indices_clamp: true,
        trace_ray_argument_validation: true,
        emit_int_div_checks: true,
    }
}

/// The binding map wgpu-hal's Vulkan backend gives naga at pipeline
/// creation (`Device::create_bind_group_layout`, `create_pipeline_layout`,
/// src/vulkan/device.rs): the descriptor set is the group's index, and the
/// Vulkan binding of an entry is its rank among the layout's entries sorted
/// by binding (wgpu-core's `bgl::EntryMap` sorts them), not the WGSL
/// `@binding` number; a binding array carries its length. A passthrough
/// module skips that step, so the shader must be written with the layouts
/// of the pipeline it will serve: `groups[i]` is the entries of group `i`.
pub fn binding_map(groups: &[&[wgpu::BindGroupLayoutEntry]]) -> spv::BindingMap {
    let mut map = BTreeMap::new();
    for (group, entries) in groups.iter().enumerate() {
        let mut sorted: Vec<&wgpu::BindGroupLayoutEntry> = entries.iter().collect();
        sorted.sort_by_key(|e| e.binding);
        for (rank, entry) in sorted.iter().enumerate() {
            map.insert(
                naga::ResourceBinding {
                    group: group as u32,
                    binding: entry.binding,
                },
                spv::BindingInfo {
                    descriptor_set: group as u32,
                    binding: rank as u32,
                    binding_array_size: entry.count.map(std::num::NonZeroU32::get),
                },
            );
        }
    }
    map
}

/// Why a shader went through the plain `create_shader_module` instead.
#[derive(Debug, thiserror::Error)]
pub enum FixError {
    #[error("the device has no PASSTHROUGH_SHADERS feature")]
    NoPassthrough,
    #[error("the WGSL does not parse: {0}")]
    Parse(String),
    #[error("the WGSL does not validate: {0}")]
    Validate(String),
    #[error("the WGSL has pipeline-overridable constants, which a passthrough module cannot specialise")]
    Overrides,
    #[error("naga could not write the SPIR-V: {0}")]
    Write(String),
    #[error("the SPIR-V rewrite failed: {0}")]
    Rewrite(#[from] RewriteError),
}

/// The shaders that use `sample_mask`, created through the workaround on
/// Vulkan and through `create_shader_module` elsewhere. Made once per
/// device.
pub struct SampleMaskShaders {
    facts: Option<VulkanFacts>,
    passthrough: bool,
    /// the device is a Vulkan one (where naga's scalar is wrong)
    vulkan: bool,
}

/// The messages that are logged once per process.
static FALLBACK_LOGGED: AtomicBool = AtomicBool::new(false);
static RETIRE_LOGGED: AtomicBool = AtomicBool::new(false);

impl SampleMaskShaders {
    /// Reads what the device is; `instance_flags` are those the instance
    /// was created with.
    pub fn new(adapter: &wgpu::Adapter, device: &wgpu::Device, instance_flags: wgpu::InstanceFlags) -> Self {
        let vulkan = adapter.get_info().backend == wgpu::Backend::Vulkan;
        let facts = vulkan.then(|| VulkanFacts::query(adapter, device, instance_flags)).flatten();
        Self {
            facts,
            passthrough: device.features().contains(wgpu::Features::PASSTHROUGH_SHADERS),
            vulkan,
        }
    }

    /// Facts given by hand (tests).
    #[cfg(test)]
    pub fn with_facts(facts: Option<VulkanFacts>, passthrough: bool) -> Self {
        let vulkan = facts.is_some();
        Self {
            facts,
            passthrough,
            vulkan,
        }
    }

    /// [`SampleMaskShaders::create_shader_module_mapped`] that never falls
    /// back to naga's scalar on Vulkan: `None` when the words cannot be
    /// fixed there (no passthrough, a step failed; logged once), so that
    /// the caller takes a route without `sample_mask`. Elsewhere (DX12,
    /// Metal: naga's HLSL and MSL are right) the plain module.
    pub fn try_create_shader_module_mapped(
        &self,
        device: &wgpu::Device,
        label: &str,
        wgsl: &str,
        map: spv::BindingMap,
    ) -> Option<wgpu::ShaderModule> {
        if !self.vulkan {
            return Some(device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(wgsl)),
            }));
        }
        let Some(facts) = &self.facts else {
            log::warn!("sample_mask shader \"{label}\": the Vulkan device could not be queried; not created");
            return None;
        };
        match self.fixed_words_mapped(facts, label, wgsl, map) {
            Ok(Some(words)) => Some(self.passthrough_module(device, label, words)),
            // (no SampleMask in the words, or naga's array already: right as it is)
            Ok(None) => Some(device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(wgsl)),
            })),
            Err(err) => {
                log::warn!("sample_mask shader \"{label}\": {err}; not created");
                None
            }
        }
    }

    fn passthrough_module(&self, device: &wgpu::Device, label: &str, words: FixedWords) -> wgpu::ShaderModule {
        let entry_points: Vec<wgpu::PassthroughShaderEntryPoint<'static>> = words
            .entry_points
            .iter()
            .map(|(name, size)| wgpu::PassthroughShaderEntryPoint {
                name: Cow::Owned(name.clone()),
                workgroup_size: (size[0], size[1], size[2]),
            })
            .collect();
        // SAFETY: the SPIR-V comes from naga, from a WGSL that naga
        // validated, written with the options wgpu-hal itself uses; the
        // rewrite only retypes the SampleMask variables and reroutes their
        // loads and stores (`rewrite`), and `verify` checked the result
        // before this point. The tests run spirv-val on the shapes of shader
        // this serves.
        unsafe {
            device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                label: Some(label),
                entry_points: Cow::Owned(entry_points),
                spirv: Some(Cow::Owned(words.spirv)),
                ..Default::default()
            })
        }
    }

    /// The shader module of a WGSL that uses `sample_mask`, in place of
    /// `device.create_shader_module`. `groups` are the bind group layout
    /// entries of each group the shader's pipeline will use, in group order
    /// (see [`binding_map`]); a shader without resources passes `&[]`.
    pub fn create_shader_module(
        &self,
        device: &wgpu::Device,
        label: &str,
        wgsl: &str,
        groups: &[&[wgpu::BindGroupLayoutEntry]],
    ) -> wgpu::ShaderModule {
        self.create_shader_module_mapped(device, label, wgsl, binding_map(groups))
    }

    /// [`SampleMaskShaders::create_shader_module`] with the binding map
    /// given: a module that declares resources outside its pipeline's
    /// layouts (a large shared source) maps those too, naga refuses to
    /// write a resource without a mapping.
    pub fn create_shader_module_mapped(&self, device: &wgpu::Device, label: &str, wgsl: &str, map: spv::BindingMap) -> wgpu::ShaderModule {
        if let Some(facts) = &self.facts {
            match self.fixed_words_mapped(facts, label, wgsl, map) {
                Ok(Some(words)) => return self.passthrough_module(device, label, words),
                Ok(None) => {}
                Err(err) => {
                    if !FALLBACK_LOGGED.swap(true, Ordering::Relaxed) {
                        log::warn!(
                            "sample_mask shader \"{label}\": {err}; created through wgpu (the validation layer may report VUID-SampleMask-SampleMask-04359)"
                        );
                    }
                }
            }
        }
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(wgsl)),
        })
    }

    /// The rewritten SPIR-V, `None` when naga already writes the array.
    #[cfg(test)]
    fn fixed_words(
        &self,
        facts: &VulkanFacts,
        label: &str,
        wgsl: &str,
        groups: &[&[wgpu::BindGroupLayoutEntry]],
    ) -> Result<Option<FixedWords>, FixError> {
        self.fixed_words_mapped(facts, label, wgsl, binding_map(groups))
    }

    fn fixed_words_mapped(
        &self,
        facts: &VulkanFacts,
        label: &str,
        wgsl: &str,
        map: spv::BindingMap,
    ) -> Result<Option<FixedWords>, FixError> {
        if !self.passthrough {
            return Err(FixError::NoPassthrough);
        }
        let module = naga::front::wgsl::parse_str(wgsl).map_err(|e| FixError::Parse(e.emit_to_string(wgsl)))?;
        if !module.overrides.is_empty() {
            return Err(FixError::Overrides);
        }
        // Every naga capability: what the device lacks is refused by the
        // writer's capability set (from the device's features), and the
        // fallback through wgpu then reports it the usual way.
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .map_err(|e| FixError::Validate(e.emit_to_string(wgsl)))?;
        let debug_info = facts.instance_flags.contains(wgpu::InstanceFlags::DEBUG).then_some(spv::DebugInfo {
            source_code: wgsl,
            file_name: label,
            language: spv::SourceLanguage::WGSL,
        });
        let options = writer_options(facts, map, debug_info);
        let spirv = spv::write_vec(&module, &info, &options, None).map_err(|e| FixError::Write(e.to_string()))?;
        let spirv = match rewrite(&spirv)? {
            Outcome::Rewritten(words) => {
                verify(&words)?;
                words
            }
            Outcome::AlreadyArray => {
                if !RETIRE_LOGGED.swap(true, Ordering::Relaxed) {
                    log::info!("naga declares the SampleMask built-in as an array: the spirv_fix workaround can be removed");
                }
                return Ok(None);
            }
            Outcome::NoSampleMask => return Ok(None),
        };
        let entry_points = module.entry_points.iter().map(|ep| (ep.name.clone(), ep.workgroup_size)).collect();
        Ok(Some(FixedWords { spirv, entry_points }))
    }
}

struct FixedWords {
    spirv: Vec<u32>,
    entry_points: Vec<(String, [u32; 3])>,
}

/// What [`rewrite`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// No `SampleMask` built-in in the module.
    NoSampleMask,
    /// Every `SampleMask` variable is an array already: nothing to do.
    AlreadyArray,
    /// The fixed module.
    Rewritten(Vec<u32>),
}

/// A SPIR-V module the rewrite does not understand; it is left to wgpu.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum RewriteError {
    #[error("malformed SPIR-V: {0}")]
    Malformed(&'static str),
    #[error("SampleMask variable %{0} has no OpVariable")]
    NoVariable(u32),
    #[error("SampleMask variable %{0} has an unexpected type")]
    UnexpectedType(u32),
    #[error("SampleMask variables are arrays and scalars at once")]
    Mixed,
    #[error("SampleMask variable %{0} is used by an instruction that is not a load or a store")]
    UnexpectedUse(u32),
    #[error("the rewritten module is not as expected: {0}")]
    Verify(&'static str),
}

const HEADER_WORDS: usize = 5;

/// An instruction's place in the words.
#[derive(Clone, Copy)]
struct Instr {
    start: usize,
    len: usize,
    op: u16,
}

impl Instr {
    fn words<'w>(&self, words: &'w [u32]) -> &'w [u32] {
        &words[self.start..self.start + self.len]
    }
}

fn op(code: spirv::Op) -> u16 {
    code as u16
}

/// Splits the module after its header into instructions.
fn instructions(words: &[u32]) -> Result<Vec<Instr>, RewriteError> {
    if words.len() < HEADER_WORDS || words[0] != spirv::MAGIC_NUMBER {
        return Err(RewriteError::Malformed("no SPIR-V header"));
    }
    let mut out = Vec::new();
    let mut at = HEADER_WORDS;
    while at < words.len() {
        let len = (words[at] >> 16) as usize;
        let op = (words[at] & 0xffff) as u16;
        if len == 0 || at + len > words.len() {
            return Err(RewriteError::Malformed("instruction length"));
        }
        out.push(Instr { start: at, len, op });
        at += len;
    }
    Ok(out)
}

/// A `SampleMask` variable of the module, as naga wrote it.
#[derive(Clone, Copy)]
struct MaskVar {
    id: u32,
    /// `OpTypePointer <class> %int`, the variable's type before the fix.
    scalar_pointer: u32,
    storage_class: u32,
    /// The 32-bit integer type of the mask.
    int: u32,
}

/// The `SampleMask` variables and whether they are arrays already.
fn mask_variables(words: &[u32], instrs: &[Instr]) -> Result<(Vec<MaskVar>, Outcome), RewriteError> {
    let decorated: Vec<u32> = instrs
        .iter()
        .filter(|i| i.op == op(spirv::Op::Decorate) && i.len == 4)
        .map(|i| i.words(words))
        .filter(|w| w[2] == spirv::Decoration::BuiltIn as u32 && w[3] == spirv::BuiltIn::SampleMask as u32)
        .map(|w| w[1])
        .collect();
    if decorated.is_empty() {
        return Ok((Vec::new(), Outcome::NoSampleMask));
    }
    // Types have their result id at word 1, a variable at word 2.
    let type_of = |id: u32| {
        instrs.iter().find(|i| {
            i.len >= 2
                && i.words(words)[1] == id
                && matches!(i.op, o if o == op(spirv::Op::TypeInt) || o == op(spirv::Op::TypeArray) || o == op(spirv::Op::TypePointer))
        })
    };
    let mut vars = Vec::new();
    let mut arrays = 0;
    for id in decorated {
        let variable = instrs
            .iter()
            .find(|i| i.op == op(spirv::Op::Variable) && i.len >= 4 && i.words(words)[2] == id)
            .ok_or(RewriteError::NoVariable(id))?
            .words(words);
        let pointer = type_of(variable[1])
            .filter(|i| i.op == op(spirv::Op::TypePointer))
            .ok_or(RewriteError::UnexpectedType(id))?
            .words(words);
        if pointer.len() != 4 {
            return Err(RewriteError::UnexpectedType(id));
        }
        let pointee = type_of(pointer[3]).ok_or(RewriteError::UnexpectedType(id))?;
        let pointee_words = pointee.words(words);
        if pointee.op == op(spirv::Op::TypeArray) {
            arrays += 1;
            continue;
        }
        // OpTypeInt %id width signedness
        if pointee_words.len() != 4 || pointee_words[2] != 32 {
            return Err(RewriteError::UnexpectedType(id));
        }
        vars.push(MaskVar {
            id,
            scalar_pointer: pointer[1],
            storage_class: pointer[2],
            int: pointer[3],
        });
    }
    // The outcome only matters when there is nothing to rewrite.
    match (arrays, vars.is_empty()) {
        (0, _) => Ok((vars, Outcome::NoSampleMask)),
        (_, true) => Ok((vars, Outcome::AlreadyArray)),
        (_, false) => Err(RewriteError::Mixed),
    }
}

/// The id of an `OpConstant %int value` of the module, if any.
fn find_constant(words: &[u32], instrs: &[Instr], int: u32, value: u32) -> Option<u32> {
    instrs
        .iter()
        .filter(|i| i.op == op(spirv::Op::Constant) && i.len == 4)
        .map(|i| i.words(words))
        .find(|w| w[1] == int && w[3] == value)
        .map(|w| w[2])
}

fn instruction(code: spirv::Op, operands: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(operands.len() + 1);
    out.push(((operands.len() as u32 + 1) << 16) | code as u32);
    out.extend_from_slice(operands);
    out
}

/// Retypes the module's `SampleMask` variables as arrays of one 32-bit
/// integer and routes their loads and stores through element 0. The module
/// is naga's output (one `OpVariable` per variable, loaded and stored
/// directly, never passed around); anything else is an error and the
/// module goes to wgpu unchanged.
pub fn rewrite(words: &[u32]) -> Result<Outcome, RewriteError> {
    let instrs = instructions(words)?;
    let (vars, outcome) = mask_variables(words, &instrs)?;
    if vars.is_empty() {
        return Ok(outcome);
    }
    let mut bound = words[3];
    let mut next_id = || {
        let id = bound;
        bound += 1;
        id
    };
    // The new declarations, per integer type (naga has one `u32`, but a
    // signed mask would get its own) and per storage class.
    let mut declarations: Vec<u32> = Vec::new();
    let mut zero_of: BTreeMap<u32, u32> = BTreeMap::new();
    let mut array_of: BTreeMap<u32, u32> = BTreeMap::new();
    let mut array_pointer_of: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for var in &vars {
        if let std::collections::btree_map::Entry::Vacant(entry) = zero_of.entry(var.int) {
            let zero = match find_constant(words, &instrs, var.int, 0) {
                Some(id) => id,
                None => {
                    let id = next_id();
                    declarations.extend(instruction(spirv::Op::Constant, &[var.int, id, 0]));
                    id
                }
            };
            entry.insert(zero);
            let one = match find_constant(words, &instrs, var.int, 1) {
                Some(id) => id,
                None => {
                    let id = next_id();
                    declarations.extend(instruction(spirv::Op::Constant, &[var.int, id, 1]));
                    id
                }
            };
            let array = next_id();
            declarations.extend(instruction(spirv::Op::TypeArray, &[array, var.int, one]));
            array_of.insert(var.int, array);
        }
        if let std::collections::btree_map::Entry::Vacant(entry) = array_pointer_of.entry((var.storage_class, var.int)) {
            let pointer = next_id();
            declarations.extend(instruction(
                spirv::Op::TypePointer,
                &[pointer, var.storage_class, array_of[&var.int]],
            ));
            entry.insert(pointer);
        }
    }
    let var_of = |id: u32| vars.iter().find(|v| v.id == id).copied();
    // The declarations go right before the first SampleMask OpVariable:
    // after the integer type and the constants it reuses (declared before
    // the variable's pointer type, itself before the variable), before any
    // use of the new ids, and in the declarations section.
    let first_variable = instrs
        .iter()
        .position(|i| i.op == op(spirv::Op::Variable) && i.len >= 4 && var_of(i.words(words)[2]).is_some())
        .ok_or(RewriteError::Malformed("variable order"))?;
    let mut out: Vec<u32> = Vec::with_capacity(words.len() + declarations.len() + 8 * vars.len());
    out.extend_from_slice(&words[..HEADER_WORDS]);
    let mut in_function = false;
    for (index, instr) in instrs.iter().enumerate() {
        if index == first_variable {
            out.extend_from_slice(&declarations);
        }
        let w = instr.words(words);
        if instr.op == op(spirv::Op::Function) {
            in_function = true;
        }
        if instr.op == op(spirv::Op::Variable)
            && instr.len >= 4
            && let Some(var) = var_of(w[2])
        {
            let mut variable = w.to_vec();
            variable[1] = array_pointer_of[&(var.storage_class, var.int)];
            out.extend_from_slice(&variable);
            continue;
        }
        if in_function {
            // The pointer operand of a load (type, result, pointer, ...)
            // or a store (pointer, object, ...).
            let pointer_at = if instr.op == op(spirv::Op::Load) && instr.len >= 4 {
                Some(3)
            } else if instr.op == op(spirv::Op::Store) && instr.len >= 3 {
                Some(1)
            } else {
                None
            };
            if let Some(at) = pointer_at
                && let Some(var) = var_of(w[at])
            {
                let element = next_id();
                out.extend(instruction(
                    spirv::Op::AccessChain,
                    &[var.scalar_pointer, element, var.id, zero_of[&var.int]],
                ));
                let mut access = w.to_vec();
                access[at] = element;
                out.extend_from_slice(&access);
                continue;
            }
            if let Some(id) = other_use(instr, w, &vars) {
                return Err(RewriteError::UnexpectedUse(id));
            }
        }
        out.extend_from_slice(w);
    }
    out[3] = bound;
    Ok(Outcome::Rewritten(out))
}

/// A `SampleMask` variable given to an instruction of a function body that
/// takes pointers and is not a load or a store: naga never does it, and the
/// rewrite would miss it.
fn other_use(instr: &Instr, w: &[u32], vars: &[MaskVar]) -> Option<u32> {
    let is_var = |id: u32| vars.iter().any(|v| v.id == id);
    let pointer_operands: &[usize] = match instr.op {
        o if o == op(spirv::Op::AccessChain) || o == op(spirv::Op::InBoundsAccessChain) || o == op(spirv::Op::PtrAccessChain) => &[3],
        o if o == op(spirv::Op::CopyMemory) => &[1, 2],
        o if o == op(spirv::Op::CopyObject) || o == op(spirv::Op::ArrayLength) => &[3],
        o if o == op(spirv::Op::FunctionCall) => return w.get(4..).and_then(|args| args.iter().copied().find(|&a| is_var(a))),
        o if (op(spirv::Op::AtomicLoad)..=op(spirv::Op::AtomicXor)).contains(&o) => &[3],
        _ => &[],
    };
    pointer_operands.iter().filter_map(|&at| w.get(at).copied()).find(|&id| is_var(id))
}

/// Checks a rewritten module: each `SampleMask` variable is a pointer to an
/// array of one 32-bit integer, no load or store takes the variable itself,
/// and every access chain on it indexes element 0 with its only index.
pub fn verify(words: &[u32]) -> Result<(), RewriteError> {
    let instrs = instructions(words)?;
    let decorated: Vec<u32> = instrs
        .iter()
        .filter(|i| i.op == op(spirv::Op::Decorate) && i.len == 4)
        .map(|i| i.words(words))
        .filter(|w| w[2] == spirv::Decoration::BuiltIn as u32 && w[3] == spirv::BuiltIn::SampleMask as u32)
        .map(|w| w[1])
        .collect();
    if decorated.is_empty() {
        return Err(RewriteError::Verify("no SampleMask variable"));
    }
    // Only the type declarations: OpName and OpDecorate also carry the
    // id as their first operand, and come before the types.
    let types = [op(spirv::Op::TypePointer), op(spirv::Op::TypeArray), op(spirv::Op::TypeInt)];
    let by_result = |id: u32| {
        instrs
            .iter()
            .find(|i| types.contains(&i.op) && i.len >= 2 && i.words(words)[1] == id)
            .map(|i| (i.op, i.words(words)))
    };
    let constant_value = |id: u32| {
        instrs
            .iter()
            .filter(|i| i.op == op(spirv::Op::Constant) && i.len == 4)
            .map(|i| i.words(words))
            .find(|w| w[2] == id)
            .map(|w| w[3])
    };
    for id in &decorated {
        let Some(variable) = instrs
            .iter()
            .find(|i| i.op == op(spirv::Op::Variable) && i.len >= 4 && i.words(words)[2] == *id)
            .map(|i| i.words(words))
        else {
            return Err(RewriteError::Verify("variable missing"));
        };
        let Some((o, pointer)) = by_result(variable[1]) else {
            return Err(RewriteError::Verify("pointer type missing"));
        };
        if o != op(spirv::Op::TypePointer) || pointer.len() != 4 || pointer[2] != variable[3] {
            return Err(RewriteError::Verify("variable type is not a pointer of its class"));
        }
        let Some((o, array)) = by_result(pointer[3]) else {
            return Err(RewriteError::Verify("pointee missing"));
        };
        if o != op(spirv::Op::TypeArray) || array.len() != 4 || constant_value(array[3]) != Some(1) {
            return Err(RewriteError::Verify("variable is not an array of one"));
        }
        match by_result(array[2]) {
            Some((o, int)) if o == op(spirv::Op::TypeInt) && int.len() == 4 && int[2] == 32 => {}
            _ => return Err(RewriteError::Verify("array element is not a 32-bit integer")),
        }
    }
    let is_var = |id: u32| decorated.contains(&id);
    let mut in_function = false;
    for instr in &instrs {
        let w = instr.words(words);
        if instr.op == op(spirv::Op::Function) {
            in_function = true;
        }
        if !in_function {
            continue;
        }
        if instr.op == op(spirv::Op::Load) && instr.len >= 4 && is_var(w[3]) {
            return Err(RewriteError::Verify("a load takes the variable itself"));
        }
        if instr.op == op(spirv::Op::Store) && instr.len >= 3 && is_var(w[1]) {
            return Err(RewriteError::Verify("a store takes the variable itself"));
        }
        if instr.op == op(spirv::Op::AccessChain) && instr.len >= 4 && is_var(w[3]) && (instr.len != 5 || constant_value(w[4]) != Some(0)) {
            return Err(RewriteError::Verify("an access chain does not take element 0 alone"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

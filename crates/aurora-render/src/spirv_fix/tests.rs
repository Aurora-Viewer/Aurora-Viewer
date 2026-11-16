//! The proof of the sample-mask workaround: naga's words rewritten, checked
//! structurally (always) and by spirv-val (when the Vulkan SDK is there),
//! the other writers inspected, and an ignored test on the GPU under the
//! validation layer.

use std::path::PathBuf;
use std::process::Command;

use super::*;

/// A fragment that writes the mask only.
const OUTPUT_ONLY: &str = r#"
struct Out { @location(0) color: vec4<f32>, @builtin(sample_mask) mask: u32 }
@fragment fn fs() -> Out { return Out(vec4<f32>(1.0), 5u); }
"#;

/// A fragment that reads the mask only.
const INPUT_ONLY: &str = r#"
@fragment fn fs(@builtin(sample_mask) mask: u32) -> @location(0) vec4<f32> {
    return vec4<f32>(f32(countOneBits(mask)) / 4.0);
}
"#;

/// A fragment that reads and writes it.
const BOTH: &str = r#"
struct Out { @location(0) color: vec4<f32>, @builtin(sample_mask) mask: u32 }
@fragment fn fs(@builtin(sample_mask) mask_in: u32) -> Out {
    return Out(vec4<f32>(1.0), mask_in & 5u);
}
"#;

/// Two fragment entry points, each with its own mask variables, beside a
/// vertex entry point.
const TWO_ENTRY_POINTS: &str = r#"
struct Out { @location(0) color: vec4<f32>, @builtin(sample_mask) mask: u32 }
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i), 0.0, 0.0, 1.0);
}
@fragment fn fs_a(@builtin(sample_mask) mask_in: u32) -> Out { return Out(vec4<f32>(1.0), mask_in & 5u); }
@fragment fn fs_b(@builtin(sample_mask) mask_in: u32) -> Out { return Out(vec4<f32>(0.5), mask_in & 10u); }
"#;

/// The mask beside the other fragment built-ins, in and out.
const WITH_BUILTINS: &str = r#"
struct In {
    @builtin(position) position: vec4<f32>,
    @builtin(front_facing) front: bool,
    @builtin(sample_index) sample: u32,
    @builtin(sample_mask) mask: u32,
    @location(0) @interpolate(flat) id: u32,
}
struct Out {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
    @builtin(sample_mask) mask: u32,
}
@fragment fn fs(input: In) -> Out {
    var out: Out;
    out.color = vec4<f32>(input.position.xy / 100.0, f32(input.sample), 1.0);
    out.depth = select(0.25, 0.75, input.front);
    out.mask = input.mask & (1u << (input.id % 4u));
    return out;
}
"#;

/// A fragment with resources, for the binding map.
const WITH_RESOURCES: &str = r#"
enable wgpu_binding_array;
struct Frame { scale: f32 }
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(2) var tex: texture_2d<f32>;
@group(1) @binding(1) var pages: binding_array<texture_2d<f32>>;
struct Out { @location(0) color: vec4<f32>, @builtin(sample_mask) mask: u32 }
@fragment fn fs(@builtin(position) p: vec4<f32>) -> Out {
    let c = textureLoad(tex, vec2<i32>(p.xy), 0) + textureLoad(pages[0], vec2<i32>(p.xy), 0);
    return Out(c * frame.scale, 5u);
}
"#;

const ALL: [(&str, &str); 5] = [
    ("output only", OUTPUT_ONLY),
    ("input only", INPUT_ONLY),
    ("both", BOTH),
    ("two entry points", TWO_ENTRY_POINTS),
    ("with built-ins", WITH_BUILTINS),
];

/// Facts of an ordinary Vulkan 1.3 device, nothing optional on.
fn facts(device_api_version: u32) -> VulkanFacts {
    VulkanFacts {
        device_api_version,
        instance_api_version: api_version(3),
        vendor_id: 0x10DE,
        instance_flags: wgpu::InstanceFlags::empty(),
        features: wgpu::Features::empty(),
        downlevel: wgpu::DownlevelFlags::all(),
        max_mesh_workgroups_per_dimension: 0,
        max_mesh_workgroup_total_count: 0,
        non_semantic_info: false,
        robust_buffer_access2: false,
        robust_image_access: false,
        zero_initialize_workgroup_memory: false,
        storage_input_output_16: false,
        shader_integer_dot_product: false,
        shader_int8: false,
    }
}

fn parse(wgsl: &str) -> (naga::Module, naga::valid::ModuleInfo) {
    let module = naga::front::wgsl::parse_str(wgsl).expect("WGSL parses");
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .expect("WGSL validates");
    (module, info)
}

/// naga's SPIR-V for the WGSL, written as the module does it.
fn naga_spirv(wgsl: &str, facts: &VulkanFacts, map: spv::BindingMap) -> Vec<u32> {
    let (module, info) = parse(wgsl);
    spv::write_vec(&module, &info, &writer_options(facts, map, None), None).expect("naga writes SPIR-V")
}

/// Runs spirv-val on the words for the Vulkan version of `facts`.
/// `None` when spirv-val is not installed (CI).
fn spirv_val(words: &[u32], facts: &VulkanFacts) -> Option<Result<(), String>> {
    let exe = std::env::var_os("VULKAN_SDK")
        .map(|sdk| {
            PathBuf::from(sdk)
                .join("Bin")
                .join(if cfg!(windows) { "spirv-val.exe" } else { "spirv-val" })
        })
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("spirv-val"));
    static FILES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let file = std::env::temp_dir().join(format!(
        "aurora-spirv-fix-{}-{}.spv",
        std::process::id(),
        FILES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&file, bytemuck::cast_slice::<u32, u8>(words)).expect("write the SPIR-V");
    let target = match facts.device_api_version {
        v if v < api_version(1) => "vulkan1.0",
        v if v < api_version(2) => "vulkan1.1",
        v if v < api_version(3) => "vulkan1.2",
        _ => "vulkan1.3",
    };
    let output = Command::new(&exe).arg("--target-env").arg(target).arg(&file).output();
    let _ = std::fs::remove_file(&file);
    match output {
        Ok(out) if out.status.success() => Some(Ok(())),
        Ok(out) => Some(Err(
            String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout)
        )),
        Err(_) => {
            eprintln!(
                "spirv-val not found ({}): the SPIR-V validation of this test is skipped",
                exe.display()
            );
            None
        }
    }
}

/// `OpDecorate %id BuiltIn SampleMask` ids of a module.
fn mask_ids(words: &[u32]) -> Vec<u32> {
    let instrs = instructions(words).expect("instructions");
    instrs
        .iter()
        .filter(|i| i.op == op(spirv::Op::Decorate) && i.len == 4)
        .map(|i| i.words(words))
        .filter(|w| w[2] == spirv::Decoration::BuiltIn as u32 && w[3] == spirv::BuiltIn::SampleMask as u32)
        .map(|w| w[1])
        .collect()
}

/// Loads and stores in function bodies whose pointer is one of `ids`,
/// and the access chains on them.
fn uses(words: &[u32], ids: &[u32]) -> (usize, usize) {
    let instrs = instructions(words).expect("instructions");
    let mut in_function = false;
    let mut direct = 0;
    let mut chains = 0;
    for i in &instrs {
        let w = i.words(words);
        in_function |= i.op == op(spirv::Op::Function);
        if !in_function {
            continue;
        }
        if (i.op == op(spirv::Op::Load) && ids.contains(&w[3])) || (i.op == op(spirv::Op::Store) && ids.contains(&w[1])) {
            direct += 1;
        }
        if i.op == op(spirv::Op::AccessChain) && ids.contains(&w[3]) {
            chains += 1;
        }
    }
    (direct, chains)
}

#[test]
fn naga_writes_the_mask_as_a_scalar() {
    // The bug this module works around: when this fails, naga is fixed and
    // the module can go (the `AlreadyArray` path retires it meanwhile).
    let words = naga_spirv(BOTH, &facts(api_version(3)), BTreeMap::new());
    let ids = mask_ids(&words);
    assert_eq!(ids.len(), 2, "one input and one output variable");
    assert_eq!(uses(&words, &ids), (2, 0), "loaded and stored directly");
    assert!(verify(&words).is_err());
    if let Some(result) = spirv_val(&words, &facts(api_version(3))) {
        let message = result.expect_err("spirv-val rejects the scalar");
        assert!(message.contains("SampleMask"), "{message}");
    }
}

#[test]
fn rewrites_every_shape_of_shader() {
    for (name, wgsl) in ALL {
        for api in [api_version(1), api_version(3)] {
            let facts = facts(api);
            let words = naga_spirv(wgsl, &facts, BTreeMap::new());
            let fixed = match rewrite(&words).unwrap_or_else(|e| panic!("{name}: {e}")) {
                Outcome::Rewritten(fixed) => fixed,
                other => panic!("{name}: {other:?}"),
            };
            verify(&fixed).unwrap_or_else(|e| panic!("{name}: {e}"));
            let ids = mask_ids(&fixed);
            assert!(!ids.is_empty(), "{name}");
            let (direct, chains) = uses(&fixed, &ids);
            assert_eq!(direct, 0, "{name}: no load or store of the variable itself");
            assert_eq!(
                chains,
                uses(&words, &mask_ids(&words)).0,
                "{name}: one access chain per load or store"
            );
            assert_eq!(
                fixed[3],
                words[3] + new_ids(&words, &fixed),
                "{name}: the bound follows the new ids"
            );
            if let (Some(plain), Some(result)) = (spirv_val(&words, &facts), spirv_val(&fixed, &facts)) {
                // spirv-val stops at its first fault and may fault naga for
                // something else (the Offset decorations of a function-scope
                // struct, under 1.3): the fix removes the SampleMask fault
                // and adds nothing.
                let plain = faults(&plain.err().unwrap_or_default());
                let left = faults(&result.err().unwrap_or_default());
                assert!(!plain.is_empty(), "{name}: naga's output passes spirv-val: is naga fixed?");
                assert!(
                    left.iter().all(|l| !l.contains("SampleMask") && plain.contains(l)),
                    "{name}, Vulkan 1.{}:
{left:?}
against
{plain:?}",
                    (api >> 12) & 0x3ff
                );
                if left.is_empty() {
                    assert!(plain.iter().any(|l| l.contains("SampleMask")), "{name}: {plain:?}");
                } else {
                    eprintln!(
                        "{name}, Vulkan 1.{}: naga's output has other faults, unchanged by the fix:
{left:?}",
                        (api >> 12) & 0x3ff
                    );
                }
            }
            // The fix is idempotent: a module with the array is left alone.
            assert_eq!(rewrite(&fixed), Ok(Outcome::AlreadyArray), "{name}");
        }
    }
}

/// spirv-val's error lines, without their line numbers (the fix shifts
/// them).
fn faults(report: &str) -> Vec<String> {
    report
        .lines()
        .filter(|l| l.starts_with("error:"))
        .map(|l| match (l.find("line "), l.find(": [")) {
            (Some(from), Some(to)) if from < to => format!("{}{}", &l[..from], &l[to + 2..]),
            _ => l.to_string(),
        })
        .collect()
}

/// Ids the rewrite made: the declarations it added and one per access
/// chain.
fn new_ids(before: &[u32], after: &[u32]) -> u32 {
    let added = instructions(after).expect("instructions").len() - instructions(before).expect("instructions").len();
    added as u32
}

#[test]
fn a_debugging_instance_names_things_and_still_rewrites() {
    // OpName, OpSource and OpLine come before the types and name the same
    // ids: the lookups must only read the declarations (the GPU test
    // found a fallback here).
    let mut debugging = facts(api_version(3));
    debugging.instance_flags = wgpu::InstanceFlags::debugging();
    let (module, info) = parse(WITH_BUILTINS);
    let debug_info = Some(spv::DebugInfo {
        source_code: WITH_BUILTINS,
        file_name: "with built-ins",
        language: spv::SourceLanguage::WGSL,
    });
    let words = spv::write_vec(&module, &info, &writer_options(&debugging, BTreeMap::new(), debug_info), None).expect("naga writes SPIR-V");
    let instrs = instructions(&words).expect("instructions");
    assert!(instrs.iter().any(|i| i.op == op(spirv::Op::Name)) && instrs.iter().any(|i| i.op == op(spirv::Op::Source)));
    let Outcome::Rewritten(fixed) = rewrite(&words).expect("rewrite") else {
        panic!("not rewritten");
    };
    verify(&fixed).expect("verified");
    assert_eq!(rewrite(&fixed), Ok(Outcome::AlreadyArray));
    let shaders = SampleMaskShaders::with_facts(Some(debugging.clone()), true);
    assert!(
        shaders
            .fixed_words(&debugging, "with built-ins", WITH_BUILTINS, &[])
            .expect("fixed")
            .is_some()
    );
}

#[test]
fn two_entry_points_each_keep_their_variables() {
    let words = naga_spirv(TWO_ENTRY_POINTS, &facts(api_version(3)), BTreeMap::new());
    let Outcome::Rewritten(fixed) = rewrite(&words).expect("rewrite") else {
        panic!("not rewritten");
    };
    let ids = mask_ids(&fixed);
    assert_eq!(ids.len(), 4, "two inputs and two outputs");
    // Each variable is still in its entry point's interface.
    let instrs = instructions(&fixed).expect("instructions");
    let entry_points: Vec<&[u32]> = instrs
        .iter()
        .filter(|i| i.op == op(spirv::Op::EntryPoint))
        .map(|i| i.words(&fixed))
        .collect();
    assert_eq!(entry_points.len(), 3);
    let listed = entry_points.iter().flat_map(|w| w.iter()).filter(|id| ids.contains(id)).count();
    assert_eq!(listed, 4);
}

#[test]
fn a_module_without_the_mask_is_untouched() {
    let words = naga_spirv(
        "@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }",
        &facts(api_version(3)),
        BTreeMap::new(),
    );
    assert_eq!(rewrite(&words), Ok(Outcome::NoSampleMask));
}

/// A module written by hand: the words a fixed naga would give for
/// `@fragment fn main(@builtin(sample_mask) m: u32) -> @builtin(sample_mask) u32`.
fn hand_built(array: bool) -> Vec<u32> {
    use spirv::Op;
    let mut w = vec![spirv::MAGIC_NUMBER, 0x0001_0300, 0, 20, 0];
    let mut push = |code: Op, operands: &[u32]| w.extend(instruction(code, operands));
    push(Op::Capability, &[spirv::Capability::Shader as u32]);
    push(
        Op::MemoryModel,
        &[spirv::AddressingModel::Logical as u32, spirv::MemoryModel::GLSL450 as u32],
    );
    // "main\0\0\0\0" in two words
    push(
        Op::EntryPoint,
        &[spirv::ExecutionModel::Fragment as u32, 10, 0x6e69_616d, 0, 11, 12],
    );
    push(Op::ExecutionMode, &[10, spirv::ExecutionMode::OriginUpperLeft as u32]);
    push(
        Op::Decorate,
        &[11, spirv::Decoration::BuiltIn as u32, spirv::BuiltIn::SampleMask as u32],
    );
    push(Op::Decorate, &[11, spirv::Decoration::Flat as u32]);
    push(
        Op::Decorate,
        &[12, spirv::Decoration::BuiltIn as u32, spirv::BuiltIn::SampleMask as u32],
    );
    push(Op::TypeVoid, &[1]);
    push(Op::TypeFunction, &[2, 1]);
    push(Op::TypeInt, &[3, 32, 0]);
    push(Op::Constant, &[3, 4, 1]);
    push(Op::Constant, &[3, 5, 0]);
    let (input, output) = (spirv::StorageClass::Input as u32, spirv::StorageClass::Output as u32);
    if array {
        push(Op::TypeArray, &[6, 3, 4]);
        push(Op::TypePointer, &[7, input, 6]);
        push(Op::TypePointer, &[8, output, 6]);
        push(Op::TypePointer, &[13, input, 3]);
        push(Op::TypePointer, &[14, output, 3]);
    } else {
        push(Op::TypePointer, &[7, input, 3]);
        push(Op::TypePointer, &[8, output, 3]);
    }
    push(Op::Variable, &[7, 11, input]);
    push(Op::Variable, &[8, 12, output]);
    push(Op::Function, &[1, 10, spirv::FunctionControl::NONE.bits(), 2]);
    push(Op::Label, &[15]);
    if array {
        push(Op::AccessChain, &[13, 16, 11, 5]);
        push(Op::Load, &[3, 17, 16]);
        push(Op::AccessChain, &[14, 18, 12, 5]);
        push(Op::Store, &[18, 17]);
    } else {
        push(Op::Load, &[3, 17, 11]);
        push(Op::Store, &[12, 17]);
    }
    push(Op::Return, &[]);
    push(Op::FunctionEnd, &[]);
    w
}

#[test]
fn a_module_with_the_array_already_is_left_alone() {
    let fixed_by_naga = hand_built(true);
    verify(&fixed_by_naga).expect("the hand-built module has the expected shape");
    assert_eq!(rewrite(&fixed_by_naga), Ok(Outcome::AlreadyArray));
    if let Some(result) = spirv_val(&fixed_by_naga, &facts(api_version(1))) {
        result.expect("the hand-built array module validates");
    }
}

#[test]
fn a_hand_built_scalar_module_is_rewritten() {
    let scalar = hand_built(false);
    if let Some(result) = spirv_val(&scalar, &facts(api_version(1))) {
        result.expect_err("the hand-built scalar module fails spirv-val");
    }
    let Outcome::Rewritten(fixed) = rewrite(&scalar).expect("rewrite") else {
        panic!("not rewritten");
    };
    verify(&fixed).expect("verified");
    // Fresh constants (zero and one) are always declared: with the array
    // type and two pointers that is five ids, plus one per access chain.
    assert_eq!(fixed[3], 20 + 5 + 2);
    if let Some(result) = spirv_val(&fixed, &facts(api_version(1))) {
        result.expect("the rewritten module validates");
    }
}

#[test]
fn a_mix_of_arrays_and_scalars_is_refused() {
    let mut words = hand_built(false);
    // Turn the output's pointer into a pointer to an array of the input's
    // integer (the array type after the constants).
    let instrs = instructions(&words).expect("instructions");
    let pointer = instrs
        .iter()
        .find(|i| i.op == op(spirv::Op::TypePointer) && i.words(&words)[1] == 8)
        .expect("output pointer");
    words[pointer.start + 3] = 6;
    let at = instrs
        .iter()
        .find(|i| i.op == op(spirv::Op::TypePointer))
        .expect("first pointer")
        .start;
    words.splice(at..at, instruction(spirv::Op::TypeArray, &[6, 3, 4]));
    assert_eq!(rewrite(&words), Err(RewriteError::Mixed));
}

#[test]
fn an_unexpected_use_of_the_variable_is_refused() {
    let mut words = hand_built(false);
    // OpCopyObject %u32 %19 %11 before the load: a pointer copy the rewrite
    // cannot follow.
    let instrs = instructions(&words).expect("instructions");
    let load = instrs.iter().find(|i| i.op == op(spirv::Op::Load)).expect("load").start;
    words.splice(load..load, instruction(spirv::Op::CopyObject, &[7, 19, 11]));
    assert_eq!(rewrite(&words), Err(RewriteError::UnexpectedUse(11)));
}

#[test]
fn malformed_words_are_refused() {
    assert_eq!(rewrite(&[1, 2, 3]), Err(RewriteError::Malformed("no SPIR-V header")));
    let mut truncated = hand_built(false);
    // Into the last store (three words), past OpReturn and OpFunctionEnd.
    truncated.truncate(truncated.len() - 3);
    assert_eq!(rewrite(&truncated), Err(RewriteError::Malformed("instruction length")));
}

#[test]
fn the_binding_map_renumbers_like_wgpu_hal() {
    let group0 = [
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
    ];
    let group1 = [wgpu::BindGroupLayoutEntry {
        binding: 1,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: std::num::NonZeroU32::new(8),
    }];
    let map = binding_map(&[&group0, &group1]);
    let info = |group, binding| map[&naga::ResourceBinding { group, binding }];
    assert_eq!((info(0, 0).descriptor_set, info(0, 0).binding), (0, 0));
    assert_eq!((info(0, 2).descriptor_set, info(0, 2).binding), (0, 1), "the gap is closed");
    assert_eq!(
        (info(1, 1).descriptor_set, info(1, 1).binding, info(1, 1).binding_array_size),
        (1, 0, Some(8))
    );

    // The SPIR-V carries the Vulkan numbers, and the fix still applies.
    let facts = facts(api_version(3));
    let words = naga_spirv(WITH_RESOURCES, &facts, map);
    let instrs = instructions(&words).expect("instructions");
    let bindings: Vec<(u32, u32)> = instrs
        .iter()
        .filter(|i| i.op == op(spirv::Op::Decorate) && i.len == 4 && i.words(&words)[2] == spirv::Decoration::Binding as u32)
        .map(|i| (i.words(&words)[1], i.words(&words)[3]))
        .collect();
    let mut numbers: Vec<u32> = bindings.iter().map(|&(_, n)| n).collect();
    numbers.sort_unstable();
    assert_eq!(numbers, vec![0, 0, 1], "bindings 0 and 1 in set 0, 0 in set 1");
    let Outcome::Rewritten(fixed) = rewrite(&words).expect("rewrite") else {
        panic!("not rewritten");
    };
    verify(&fixed).expect("verified");
    if let Some(result) = spirv_val(&fixed, &facts) {
        result.expect("validates");
    }
}

#[test]
fn a_resource_missing_from_the_map_is_an_error_like_wgpu_hal() {
    let (module, info) = parse(WITH_RESOURCES);
    let options = writer_options(&facts(api_version(3)), BTreeMap::new(), None);
    assert!(spv::write_vec(&module, &info, &options, None).is_err());
}

#[test]
fn options_follow_the_device() {
    let v10 = facts(api_version(0));
    assert_eq!(writer_options(&v10, BTreeMap::new(), None).lang_version, (1, 0));
    assert_eq!(writer_options(&facts(api_version(1)), BTreeMap::new(), None).lang_version, (1, 3));
    assert_eq!(writer_options(&facts(api_version(2)), BTreeMap::new(), None).lang_version, (1, 5));
    let v13 = facts(api_version(3));
    let options = writer_options(&v13, BTreeMap::new(), None);
    assert_eq!(options.lang_version, (1, 6));
    assert!(
        options
            .flags
            .contains(spv::WriterFlags::LABEL_VARYINGS | spv::WriterFlags::FORCE_POINT_SIZE)
    );
    assert!(!options.flags.contains(spv::WriterFlags::DEBUG));
    assert_eq!(options.bounds_check_policies.buffer, naga::proc::BoundsCheckPolicy::Restrict);
    assert_eq!(
        options.zero_initialize_workgroup_memory,
        spv::ZeroInitializeWorkgroupMemoryMode::Polyfill
    );
    let capabilities = options.capabilities.as_ref().expect("a set");
    assert!(capabilities.contains(&spv::Capability::SampleRateShading));
    assert!(!capabilities.contains(&spv::Capability::Int8));

    let mut qualcomm = facts(api_version(3));
    qualcomm.vendor_id = QUALCOMM;
    qualcomm.instance_flags = wgpu::InstanceFlags::DEBUG;
    qualcomm.robust_buffer_access2 = true;
    qualcomm.robust_image_access = true;
    qualcomm.zero_initialize_workgroup_memory = true;
    qualcomm.shader_int8 = true;
    qualcomm.features = wgpu::Features::SHADER_F16;
    qualcomm.storage_input_output_16 = true;
    let options = writer_options(&qualcomm, BTreeMap::new(), None);
    assert!(!options.flags.contains(spv::WriterFlags::LABEL_VARYINGS));
    assert!(
        options
            .flags
            .contains(spv::WriterFlags::DEBUG | spv::WriterFlags::PRINT_ON_RAY_QUERY_INITIALIZATION_FAIL)
    );
    assert_eq!(options.bounds_check_policies.buffer, naga::proc::BoundsCheckPolicy::Unchecked);
    assert_eq!(options.bounds_check_policies.image_load, naga::proc::BoundsCheckPolicy::Unchecked);
    assert_eq!(options.bounds_check_policies.index, naga::proc::BoundsCheckPolicy::Restrict);
    assert_eq!(
        options.zero_initialize_workgroup_memory,
        spv::ZeroInitializeWorkgroupMemoryMode::Native
    );
    assert!(options.use_storage_input_output_16);
    let capabilities = options.capabilities.as_ref().expect("a set");
    assert!(capabilities.contains(&spv::Capability::Int8) && capabilities.contains(&spv::Capability::Float16));
}

#[test]
fn hlsl_and_msl_writers_are_right_already() {
    let (module, info) = parse(BOTH);
    let mut hlsl = String::new();
    naga::back::hlsl::Writer::new(
        &mut hlsl,
        &naga::back::hlsl::Options::default(),
        &naga::back::hlsl::PipelineOptions::default(),
    )
    .write(&module, &info, None)
    .expect("HLSL");
    // One input and one output coverage semantic, both scalar uint.
    let coverage: Vec<&str> = hlsl.lines().filter(|l| l.contains("SV_Coverage")).map(str::trim).collect();
    assert_eq!(coverage.len(), 2, "{hlsl}");
    assert!(coverage.iter().all(|l| l.starts_with("uint ")), "{hlsl}");
    assert!(
        coverage.iter().any(|l| l.contains("mask_in")) && coverage.iter().any(|l| l.starts_with("uint mask ")),
        "{hlsl}"
    );

    let (msl, _) = naga::back::msl::write_string(
        &module,
        &info,
        &naga::back::msl::Options::default(),
        &naga::back::msl::PipelineOptions::default(),
    )
    .expect("MSL");
    assert_eq!(msl.matches("[[sample_mask]]").count(), 2, "{msl}");
    assert!(msl.contains("uint mask_in [[sample_mask]]"), "{msl}");
    assert!(msl.contains("uint mask [[sample_mask]]"), "{msl}");
}

#[test]
fn a_shader_with_overrides_goes_to_wgpu() {
    let shaders = SampleMaskShaders::with_facts(Some(facts(api_version(3))), true);
    let wgsl = "override k: u32 = 5u;\nstruct Out { @location(0) c: vec4<f32>, @builtin(sample_mask) m: u32 }\n@fragment fn fs() -> Out { return Out(vec4<f32>(1.0), k); }";
    assert!(matches!(
        shaders.fixed_words(&facts(api_version(3)), "t", wgsl, &[]),
        Err(FixError::Overrides)
    ));
    let shaders = SampleMaskShaders::with_facts(Some(facts(api_version(3))), false);
    assert!(matches!(
        shaders.fixed_words(&facts(api_version(3)), "t", BOTH, &[]),
        Err(FixError::NoPassthrough)
    ));
    let shaders = SampleMaskShaders::with_facts(Some(facts(api_version(3))), true);
    assert!(
        shaders
            .fixed_words(&facts(api_version(3)), "t", BOTH, &[])
            .expect("fixed")
            .is_some()
    );
}

/// Counts the validation layer's SampleMask messages, which wgpu-hal logs.
struct VuidCounter;

static VUID_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static ANY_VALIDATION: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl log::Log for VuidCounter {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        let text = record.args().to_string();
        if record.level() <= log::Level::Warn && !text.contains("VUID-") {
            eprintln!("[{}] {text}", record.level());
        }
        if text.contains("VUID-") && text.contains(" (0x") {
            ANY_VALIDATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        // The layer's records read "VALIDATION [VUID-… (0x…)]"; the
        // module's own fallback warning names the VUID too, and that one
        // is printed above instead of counted.
        if text.contains("VUID-SampleMask-SampleMask-04359 (") {
            VUID_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    fn flush(&self) {}
}

/// The lock the agents measuring on this GPU share: made exclusively,
/// waited for otherwise, deleted at the end.
struct GpuBenchLock(PathBuf);

impl GpuBenchLock {
    fn take() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../.gpu-bench.lock");
        for _ in 0..40 {
            if let Ok(mut file) = std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                use std::io::Write;
                let _ = file.write_all(b"spirv-fix");
                return Self(path);
            }
            eprintln!("waiting for {}", path.display());
            std::thread::sleep(std::time::Duration::from_secs(30));
        }
        panic!("{} stayed taken", path.display());
    }
}

impl Drop for GpuBenchLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The full proof on the GPU, under the validation layer: a triangle drawn
/// into a 4x target with the mask written through the fixed module gives
/// no SampleMask message (the plain module gives some) and half coverage.
/// Ignored: it needs a Vulkan GPU and the validation layer. Run it alone,
/// with `-- --ignored --nocapture gpu_`.
#[test]
#[ignore]
fn gpu_passthrough_under_the_validation_layer() {
    let _bench = GpuBenchLock::take();
    let _gpu = crate::test_gpu::lock();
    let logger_set = log::set_logger(&VuidCounter).is_ok();
    log::set_max_level(log::LevelFilter::Warn);
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::VULKAN;
    desc.flags = wgpu::InstanceFlags::debugging();
    let instance = wgpu::Instance::new(desc);
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
        eprintln!("no Vulkan adapter: skipped");
        return;
    };
    let features = feature(adapter.features());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: features,
        ..Default::default()
    }))
    .expect("GPU device");
    let shaders = SampleMaskShaders::new(&adapter, &device, wgpu::InstanceFlags::debugging());
    assert!(shaders.passthrough, "the adapter has PASSTHROUGH_SHADERS");
    assert!(shaders.facts.is_some(), "the Vulkan facts were read");
    eprintln!("{:#?}", shaders.facts);

    let wgsl = r#"
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i) - 1) * 4.0;
    let y = f32(i32(i & 1u) * 2 - 1) * 4.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}
struct Out { @location(0) color: vec4<f32>, @builtin(sample_mask) mask: u32 }
@fragment fn fs(@builtin(sample_mask) mask_in: u32) -> Out {
    return Out(vec4<f32>(1.0), mask_in & 5u);
}
"#;
    let size = wgpu::Extent3d {
        width: 8,
        height: 8,
        depth_or_array_layers: 1,
    };
    let texture = |samples| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("spirv-fix target"),
            size,
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };
    let msaa = texture(4).create_view(&Default::default());
    let resolved = texture(1);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("spirv-fix readback"),
        size: crate::readback::readback_size(256 * 8),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor::default());

    let mut counts = Vec::new();
    for fixed in [false, true] {
        VUID_COUNT.store(0, std::sync::atomic::Ordering::Relaxed);
        let module = if fixed {
            shaders.create_shader_module(&device, "spirv-fix fixed", wgsl, &[])
        } else {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("spirv-fix plain"),
                source: wgpu::ShaderSource::Wgsl(wgsl.into()),
            })
        };
        use crate::pipeline_cache::CachedPipelines;
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: Some("spirv-fix"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 4,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spirv-fix"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &msaa,
                    depth_slice: None,
                    resolve_target: Some(&resolved.create_view(&Default::default())),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            resolved.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            size,
        );
        queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let pixel = {
            let data = slice.get_mapped_range().expect("mapped");
            let at = 4 * 256 + 4 * 4;
            [data[at], data[at + 1], data[at + 2], data[at + 3]]
        };
        readback.unmap();
        let count = VUID_COUNT.load(std::sync::atomic::Ordering::Relaxed);
        eprintln!(
            "{}: VUID-SampleMask-SampleMask-04359 x{count}, resolved pixel {pixel:?}",
            if fixed { "fixed module" } else { "plain module" }
        );
        // Two of the four samples covered: half of white (the alpha is 1
        // on both sides of the resolve).
        for channel in &pixel[..3] {
            assert!(
                (120..=136).contains(channel),
                "pixel {pixel:?}: the mask did not halve the coverage"
            );
        }
        assert_eq!(pixel[3], 255);
        counts.push(count);
    }
    if !logger_set {
        eprintln!("another logger was set: the messages could not be counted");
        return;
    }
    assert_eq!(counts[1], 0, "the fixed module gets no SampleMask message");
    if counts[0] == 0 {
        eprintln!(
            "the plain module got no message either ({} validation messages in all): is the validation layer installed?",
            ANY_VALIDATION.load(std::sync::atomic::Ordering::Relaxed)
        );
    }
}

/// The real thing: the scene module's `fs_st_depth_mask`, as `mask_module`
/// makes it (the overrides turned into constants), is rewritten and
/// verified, and spirv-val accepts it when the SDK is there.
#[test]
fn the_scene_module_of_the_stochastic_depth_pass_is_rewritten() {
    let object = include_str!("../shaders/object.wgsl").replace("\r\n", "\n");
    let src = format!(
        "{}{}\n{}\n{}",
        crate::oit_stochastic::enable_directive_with(true),
        crate::COMMON_SHADER,
        crate::oit_stochastic::st_object_source_with(&object, true),
        include_str!("../shaders/env.wgsl")
    );
    let wgsl = crate::oit_stochastic::constants_for_overrides(&src, &[("WATER_CLIP", "1u"), ("BLEND_EARLY_DISCARD", "true")]);
    // every declared resource gets a binding of its own (the real layouts
    // are the renderer's; the rewrite does not depend on them)
    let mut map = BTreeMap::new();
    for (i, (group, binding)) in crate::oit_stochastic::declared_bindings(&wgsl).into_iter().enumerate() {
        map.insert(
            naga::ResourceBinding { group, binding },
            spv::BindingInfo {
                descriptor_set: group,
                binding: 100 + i as u32,
                // (the texture pages are a binding array)
                binding_array_size: ((group, binding) == (2, 0)).then_some(64),
            },
        );
    }
    let mut facts = facts(api_version(3));
    // (the textures are a binding array indexed by a value that varies)
    facts.features = wgpu::Features::TEXTURE_BINDING_ARRAY
        | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
        | wgpu::Features::PRIMITIVE_INDEX;
    let shaders = SampleMaskShaders::with_facts(Some(facts.clone()), true);
    let words = shaders
        .fixed_words_mapped(&facts, "scene stochastic mask", &wgsl, map)
        .expect("the scene module is written and rewritten")
        .expect("the module has a SampleMask built-in to rewrite");
    verify(&words.spirv).expect("verified");
    assert_eq!(rewrite(&words.spirv), Ok(Outcome::AlreadyArray));
    assert!(words.entry_points.iter().any(|(name, _)| name == "fs_st_depth_mask"));
    // (the module declares many resources the entry point never uses: only
    // the faults about the mask matter here)
    if let Some(Err(report)) = spirv_val(&words.spirv, &facts) {
        let about_the_mask: Vec<String> = faults(&report).into_iter().filter(|f| f.contains("SampleMask")).collect();
        assert!(about_the_mask.is_empty(), "{about_the_mask:?}");
    }
}

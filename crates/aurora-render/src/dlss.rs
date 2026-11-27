//! NVIDIA DLSS super resolution in FSR's place, from files the user adds,
//! in a build with the `dlss` feature (off by default).
//!
//! The viewer speaks only NVIDIA Streamline's public interface (`sl`:
//! the names, values and layouts of Streamline's MIT-licensed headers),
//! loaded at run time from a folder of the user's (`AURORA_DLSS_DIR`, else
//! `$XDG_DATA_HOME/aurora-dlss` on Linux, `%LOCALAPPDATA%\aurora-dlss` on
//! Windows). Nothing of NVIDIA's is linked into the viewer or shipped with
//! it:
//!
//! - Windows (x86-64): NVIDIA's signed Streamline release, its `bin/x64`
//!   files (`sl.interposer.dll`, `sl.common.dll`, `sl.dlss.dll`,
//!   `nvngx_dlss.dll`), which the user downloads from NVIDIA. The Vulkan
//!   instance is made through `sl.interposer.dll`, so that Streamline sees
//!   the instance and device made.
//! - Linux: `libsl_ngx.so`, sl-ngx (MIT), which implements that interface
//!   for Vulkan on NVIDIA NGX, built by the user against their own copy of
//!   the DLSS SDK, beside the SDK's `libnvidia-ngx-dlss.so.*`. NVIDIA ships
//!   Streamline for Windows only; the viewer names the instance and device
//!   to it (`slSetVulkanInfo`, manual hooking).
//!
//! `scripts/dlss-setup.sh` and `scripts/dlss-setup.ps1` set the folder up:
//! docs/DLSS.md. Without the folder or its files the viewer says so in the
//! log and FSR draws.
//!
//! When the files are there the instance and the device are opened with
//! the DLSS extensions, and DLSS draws while the anti-aliasing is FSR and
//! Preferences › Graphics › FSR › DLSS is on (`RenderSettings::dlss`;
//! `AURORA_DLSS=1` or `0` overrides it for a test). FSR's render size,
//! jitter and texture bias stay as they are; DLSS takes the same inputs
//! (the HDR scene, its depth, the prepass motion) and writes the image the
//! post passes read instead of FSR's. Without the feature [`Dlss::new`]
//! gives nothing and the renderer runs as before.
//!
//! The wgpu side follows the `dlss_wgpu` crate (MIT OR Apache-2.0,
//! Copyright (c) the dlss_wgpu contributors).

/// What DLSS reads, all at the render size (the same views as FSR's).
#[cfg_attr(
    not(all(feature = "dlss", any(target_os = "linux", all(windows, target_arch = "x86_64")))),
    allow(dead_code)
)]
pub(crate) struct Inputs<'a> {
    pub color: &'a wgpu::TextureView,
    pub depth: &'a wgpu::TextureView,
    pub motion: &'a wgpu::TextureView,
    /// the scene pass's G-buffer and base colour, for Ray Reconstruction's
    /// guides (`RenderSettings::rr_guides`); None without them
    pub gbuf: Option<&'a wgpu::TextureView>,
    pub albedo: Option<&'a wgpu::TextureView>,
}

/// The frame's values (fsr.rs `FrameInputs`).
#[cfg_attr(
    not(all(feature = "dlss", any(target_os = "linux", all(windows, target_arch = "x86_64")))),
    allow(dead_code)
)]
pub(crate) struct Frame {
    pub motion_reproj: [[f32; 4]; 4],
    /// where the jitter moved the image, in render pixels (y down)
    pub jitter: glam::Vec2,
    pub reset: bool,
    pub motion_on: bool,
    /// the camera's projection, without the jitter
    pub proj: glam::Mat4,
    /// world to camera
    pub view: glam::Mat4,
    pub camera_pos: glam::Vec3,
    pub near: f32,
    pub far: f32,
    /// vertical field of view, radians
    pub fov_y: f32,
}

/// Streamline's render sizes already asked, of one start of Streamline: a new start (a
/// new device) asks again, and a size Streamline did not give is asked again
/// next time rather than kept.
#[cfg_attr(
    not(all(feature = "dlss", any(target_os = "linux", all(windows, target_arch = "x86_64")))),
    allow(dead_code)
)]
struct Answers<K> {
    /// the start of Streamline that gave `sizes` (0: none yet)
    start: u64,
    sizes: Vec<(K, (u32, u32))>,
}

#[cfg_attr(
    not(all(feature = "dlss", any(target_os = "linux", all(windows, target_arch = "x86_64")))),
    allow(dead_code)
)]
impl<K: PartialEq> Answers<K> {
    const fn new() -> Self {
        Self {
            start: 0,
            sizes: Vec::new(),
        }
    }

    /// The size kept for `key`, when Streamline is still the start that gave it.
    fn get(&self, start: Option<u64>, key: &K) -> Option<(u32, u32)> {
        if start != Some(self.start) {
            return None;
        }
        self.sizes.iter().find(|(k, _)| k == key).map(|(_, size)| *size)
    }

    /// What `start` answered for `key`; a start other than the kept one's
    /// forgets the others first.
    fn put(&mut self, start: u64, key: K, size: Option<(u32, u32)>) {
        if start != self.start {
            self.start = start;
            self.sizes.clear();
        }
        if let Some(size) = size {
            self.sizes.push((key, size));
        }
    }
}

/// Streamline refused to start on the device counted `opens`, when `refused_at` is
/// the count kept at the refusal (0: none). Kept until another device opens
/// or the user tries again (`imp::REFUSED`).
#[cfg_attr(
    not(all(feature = "dlss", any(target_os = "linux", all(windows, target_arch = "x86_64")))),
    allow(dead_code)
)]
fn refused(refused_at: u64, opens: u64) -> bool {
    refused_at != 0 && refused_at == opens
}

/// `lock`, taken within `wait` or not at all. For Streamline's lock where a
/// device loss may have left a thread inside a Streamline call that never
/// returns, holding it (`imp::let_go`): the device opened after the loss
/// must not wait for it for ever. A lock poisoned by a panic is taken (the
/// state behind it is plain values).
#[cfg_attr(
    not(all(feature = "dlss", any(target_os = "linux", all(windows, target_arch = "x86_64")))),
    allow(dead_code)
)]
fn lock_within<T>(lock: &std::sync::Mutex<T>, wait: std::time::Duration) -> Option<std::sync::MutexGuard<'_, T>> {
    let deadline = std::time::Instant::now() + wait;
    loop {
        match lock.try_lock() {
            Ok(held) => return Some(held),
            Err(std::sync::TryLockError::Poisoned(e)) => return Some(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err(std::sync::TryLockError::WouldBlock) => return None,
        }
    }
}

/// Streamline's interface (`sl.h`, `sl_dlss.h`, `sl_helpers_vk.h`, MIT,
/// Copyright (c) NVIDIA Corporation: NOTICE.md), opened at run time from
/// the user's folder: the functions the viewer calls and the structures
/// they take.
#[cfg(all(feature = "dlss", any(target_os = "linux", all(windows, target_arch = "x86_64"))))]
mod sl {
    use ash::vk;
    use std::ffi::{CStr, c_char, c_void};

    /// `sl::kSDKVersion` of the headers these layouts are from (2.14.16).
    pub const SDK_VERSION: u64 = (2 << 48) | (14 << 32) | (16 << 16) | 0xfedc;

    /// `sl::Feature`
    pub const FEATURE_DLSS: u32 = 0;
    pub const FEATURE_DLSS_RR: u32 = 1001;
    /// `kFeatureDLSS_G`: frame generation (`RenderSettings::dlss_fg`).
    /// sl-ngx has the caller evaluate it; Windows' Streamline makes the
    /// frames itself, at the window's present.
    pub const FEATURE_DLSS_G: u32 = 1000;
    /// `kFeatureReflex` and `kFeaturePCL`: frame generation in Windows'
    /// Streamline runs only with both (its markers name the frame each
    /// present shows).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub const FEATURE_REFLEX: u32 = 3;
    #[cfg_attr(not(windows), allow(dead_code))]
    pub const FEATURE_PCL: u32 = 4;

    /// `sl::Result`: `eOk` and the values the viewer tells apart.
    pub const OK: i32 = 0;
    pub const ERROR_DRIVER_OUT_OF_DATE: i32 = 2;
    pub const ERROR_INVALID_PARAMETER: i32 = 25;

    /// `sl::Result`'s name, for the log.
    pub fn result_name(r: i32) -> &'static str {
        const NAMES: [&str; 40] = [
            "eOk",
            "eErrorIO",
            "eErrorDriverOutOfDate",
            "eErrorOSOutOfDate",
            "eErrorOSDisabledHWS",
            "eErrorDeviceNotCreated",
            "eErrorNoSupportedAdapterFound",
            "eErrorAdapterNotSupported",
            "eErrorNoPlugins",
            "eErrorVulkanAPI",
            "eErrorDXGIAPI",
            "eErrorD3DAPI",
            "eErrorNRDAPI",
            "eErrorNVAPI",
            "eErrorReflexAPI",
            "eErrorNGXFailed",
            "eErrorJSONParsing",
            "eErrorMissingProxy",
            "eErrorMissingResourceState",
            "eErrorInvalidIntegration",
            "eErrorMissingInputParameter",
            "eErrorNotInitialized",
            "eErrorComputeFailed",
            "eErrorInitNotCalled",
            "eErrorExceptionHandler",
            "eErrorInvalidParameter",
            "eErrorMissingConstants",
            "eErrorDuplicatedConstants",
            "eErrorMissingOrInvalidAPI",
            "eErrorCommonConstantsMissing",
            "eErrorUnsupportedInterface",
            "eErrorFeatureMissing",
            "eErrorFeatureNotSupported",
            "eErrorFeatureMissingHooks",
            "eErrorFeatureFailedToLoad",
            "eErrorFeatureWrongPriority",
            "eErrorFeatureMissingDependency",
            "eErrorFeatureManagerInvalidState",
            "eErrorInvalidState",
            "eWarnOutOfVRAM",
        ];
        usize::try_from(r)
            .ok()
            .and_then(|i| NAMES.get(i))
            .copied()
            .unwrap_or("an unknown result")
    }

    /// `sl::Result` as an error the log can show.
    pub fn check(r: i32) -> Result<(), String> {
        if r == OK {
            Ok(())
        } else {
            Err(format!("{} ({r})", result_name(r)))
        }
    }

    /// `sl::DLSSMode`
    pub mod mode {
        pub const MAX_PERFORMANCE: u32 = 1;
        pub const BALANCED: u32 = 2;
        pub const MAX_QUALITY: u32 = 3;
        pub const ULTRA_PERFORMANCE: u32 = 4;
        pub const DLAA: u32 = 6;
    }

    /// `sl::BufferType`
    pub mod buffer {
        pub const DEPTH: u32 = 0;
        pub const MOTION_VECTORS: u32 = 1;
        pub const HUDLESS_COLOR: u32 = 2;
        pub const SCALING_INPUT_COLOR: u32 = 3;
        pub const SCALING_OUTPUT_COLOR: u32 = 4;
        pub const ALBEDO: u32 = 7;
        pub const SPECULAR_ALBEDO: u32 = 8;
        /// normals with the roughness in w (`DLSSDNormalRoughnessMode::ePacked`)
        pub const NORMAL_ROUGHNESS: u32 = 14;
    }

    /// `sl::DLSSDNormalRoughnessMode::ePacked`
    pub const ROUGHNESS_PACKED: u32 = 1;

    /// `sl::ResourceLifecycle::eValidUntilEvaluate`: the viewer's images
    /// stay as they are until `slEvaluateFeature` returns.
    pub const VALID_UNTIL_EVALUATE: u32 = 2;
    /// `sl::ResourceLifecycle::eValidUntilPresent`: the image stays as
    /// tagged until the frame is presented, and the library makes no copy
    /// of it (`ResourceTag::until_present`).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub const VALID_UNTIL_PRESENT: u32 = 1;
    /// `sl::ResourceType::eTex2d`
    pub const TEX_2D: u8 = 0;
    /// `sl::RenderAPI::eVulkan`
    pub const VULKAN: u32 = 2;
    /// `sl::EngineType::eCustom`
    pub const ENGINE_CUSTOM: u32 = 0;
    /// `sl::LogLevel::eDefault`
    pub const LOG_DEFAULT: u32 = 1;
    /// `sl::LogType`
    pub const LOG_WARN: u32 = 1;
    pub const LOG_ERROR: u32 = 2;
    /// `sl::PreferenceFlags`
    pub const DISABLE_CL_STATE_TRACKING: u64 = 1;
    pub const DISABLE_DEBUG_TEXT: u64 = 1 << 1;
    #[cfg_attr(windows, allow(dead_code))]
    pub const USE_MANUAL_HOOKING: u64 = 1 << 2;
    /// needed for `slSetTagForFrame` (NVIDIA's Streamline refuses it without)
    pub const USE_FRAME_BASED_RESOURCE_TAGGING: u64 = 1 << 7;

    /// `sl::Boolean`
    pub const FALSE: u8 = 0;
    pub const TRUE: u8 = 1;
    pub fn boolean(b: bool) -> u8 {
        if b { TRUE } else { FALSE }
    }

    /// A struct's type id: its GUID's 16 bytes as a C++ compiler lays
    /// them out on x86-64 (tests/layout.expected in sl-ngx).
    pub type Guid = [u8; 16];

    /// `sl::BaseStructure`: every struct the interface passes starts with
    /// it.
    #[repr(C)]
    pub struct Base {
        pub next: *mut Base,
        pub id: Guid,
        pub version: usize,
    }

    impl Base {
        pub const fn of(id: Guid, version: usize) -> Self {
            Self {
                next: std::ptr::null_mut(),
                id,
                version,
            }
        }
    }

    macro_rules! id {
        ($(#[$m:meta])* $name:ident, $version:expr, [$($b:literal)*]) => {
            $(#[$m])*
            pub const $name: (Guid, usize) = ([$($b),*], $version);
        };
    }
    id!(PREFERENCES, 1, [0x65 0x09 0xa1 0x1c 0x8e 0xbf 0x2b 0x43 0x8d 0xa1 0x67 0x16 0xd8 0x79 0xfb 0x14]);
    id!(#[cfg_attr(windows, allow(dead_code))] VULKAN_INFO, 3, [0xd5 0x6f 0xed 0x0e 0xcd 0x82 0xa9 0x43 0xbd 0xb5 0x47 0xa5 0xba 0x2f 0x45 0xd6]);
    id!(FEATURE_REQUIREMENTS, 2, [0x97 0x40 0x71 0x66 0x6d 0xac 0xc6 0x4b 0x89 0x15 0x1e 0x0f 0x55 0xa6 0xb6 0x1f]);
    id!(RESOURCE, 1, [0xcf 0x70 0x9d 0x3a 0x18 0x24 0x72 0x4b 0x83 0x91 0x13 0xf8 0x72 0x1c 0x72 0x61]);
    id!(RESOURCE_TAG, 1, [0xad 0x5a 0x6a 0x4c 0x45 0xb4 0x6c 0x49 0x87 0xff 0x1a 0xf3 0x84 0x5b 0xe6 0x53]);
    id!(CONSTANTS, 2, [0xd7 0x5a 0xd3 0xdc 0x4a 0x4e 0xad 0x4b 0xa9 0x0c 0xe0 0xc4 0x9e 0xb2 0x3a 0xfe]);
    id!(DLSS_OPTIONS, 3, [0xe4 0x26 0xc8 0x6a 0x61 0x4c 0x01 0x41 0xa9 0x2d 0x63 0x8d 0x42 0x10 0x57 0xb8]);
    id!(DLSS_OPTIMAL_SETTINGS, 1, [0x57 0x09 0x1d 0xef 0x58 0xfd 0xf7 0x4d 0xb5 0x04 0x8b 0x69 0xd8 0xaa 0x6b 0x76]);
    id!(DLSSD_OPTIONS, 3, [0x04 0x75 0xd8 0x0a 0x4e 0x77 0xf3 0x4b 0x96 0x33 0xa4 0x4d 0x1f 0x7f 0x9c 0xb8]);
    id!(DLSSD_OPTIMAL_SETTINGS, 1, [0x37 0xc6 0xd0 0xfb 0x8f 0xa2 0xf2 0x41 0xbc 0x91 0xb4 0x21 0xfa 0xee 0x8e 0x1e]);
    id!(VIEWPORT_HANDLE, 1, [0x35 0x64 0x1b 0x17 0x3c 0x9b 0xc8 0x4f 0x99 0x94 0xfb 0xe5 0x25 0x69 0xaa 0xa4]);
    id!(ADAPTER_INFO, 1, [0x5f 0x31 0x77 0x06 0x46 0xa7 0x92 0x44 0x9f 0x42 0xcb 0x61 0x42 0xc9 0xc3 0xd4]);
    id!(#[cfg_attr(not(windows), allow(dead_code))] DLSSG_OPTIONS, 5, [0xcb 0xf1 0xc5 0xfa 0xfd 0x2d 0x36 0x4f 0xa1 0xe6 0x3a 0x9e 0x86 0x52 0x56 0xc5]);
    id!(#[cfg_attr(not(windows), allow(dead_code))] DLSSG_STATE, 4, [0xe1 0xc8 0x8a 0xcc 0x79 0xa1 0xf5 0x44 0x97 0xfa 0xe7 0x41 0x12 0xf9 0xbc 0x61]);
    id!(#[cfg_attr(not(windows), allow(dead_code))] REFLEX_OPTIONS, 1, [0x1a 0xf8 0x3a 0xf0 0x0b 0x6d 0x02 0x49 0xa6 0x51 0xc4 0x96 0x5e 0x21 0x54 0x34]);

    const fn base((id, version): (Guid, usize)) -> Base {
        Base::of(id, version)
    }

    /// `sl::DLSSGMode`
    #[cfg_attr(not(windows), allow(dead_code))]
    pub mod dlssg_mode {
        pub const OFF: u32 = 0;
        pub const ON: u32 = 1;
    }

    /// `sl::DLSSGFlags::eRetainResourcesWhenOff`: a pause (a frame DLSS
    /// did not draw, a modal's blur) keeps the feature's memory.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub const DLSSG_RETAIN_RESOURCES_WHEN_OFF: u32 = 1 << 3;

    /// `sl::DLSSGOptions` (sl_dlss_g.h, version 5)
    #[repr(C)]
    #[cfg_attr(not(windows), allow(dead_code))]
    pub struct DlssgOptions {
        pub base: Base,
        pub mode: u32,
        pub num_frames_to_generate: u32,
        pub flags: u32,
        pub dynamic_res_width: u32,
        pub dynamic_res_height: u32,
        pub num_back_buffers: u32,
        pub mvec_depth_width: u32,
        pub mvec_depth_height: u32,
        pub color_width: u32,
        pub color_height: u32,
        pub color_buffer_format: u32,
        pub mvec_buffer_format: u32,
        pub depth_buffer_format: u32,
        pub hud_less_buffer_format: u32,
        pub ui_buffer_format: u32,
        pub on_error_callback: *const c_void,
        pub reserved_15: u8,
        pub queue_parallelism_mode: u32,
        pub enable_user_interface_recomposition: u8,
        pub dynamic_target_frame_rate: f32,
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    impl DlssgOptions {
        /// The header's defaults, with `mode` and `frames` generated per
        /// frame drawn (1 for 2x).
        pub fn new(mode: u32, frames: u32, flags: u32) -> Self {
            Self {
                base: base(DLSSG_OPTIONS),
                mode,
                num_frames_to_generate: frames,
                flags,
                dynamic_res_width: 0,
                dynamic_res_height: 0,
                num_back_buffers: 0,
                mvec_depth_width: 0,
                mvec_depth_height: 0,
                color_width: 0,
                color_height: 0,
                color_buffer_format: 0,
                mvec_buffer_format: 0,
                depth_buffer_format: 0,
                hud_less_buffer_format: 0,
                ui_buffer_format: 0,
                on_error_callback: std::ptr::null(),
                // `Boolean::eInvalid`, as the header leaves it
                reserved_15: 2,
                queue_parallelism_mode: 0,
                enable_user_interface_recomposition: FALSE,
                dynamic_target_frame_rate: 0.0,
            }
        }
    }

    /// `sl::DLSSGStatus` bits, for the log.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn dlssg_status(status: u32) -> String {
        if status == 0 {
            return String::from("ok");
        }
        let names = [
            (1, "resolution too low"),
            (2, "Reflex not detected"),
            (4, "HDR format not supported"),
            (8, "common constants invalid"),
            (16, "back buffer index not asked"),
        ];
        let mut out: Vec<String> = names
            .iter()
            .filter(|(bit, _)| status & bit != 0)
            .map(|(_, name)| String::from(*name))
            .collect();
        let rest = status & !31;
        if rest != 0 {
            out.push(format!("0x{rest:x}"));
        }
        out.join(", ")
    }

    /// `sl::DLSSGState` (sl_dlss_g.h, version 4)
    #[repr(C)]
    #[cfg_attr(not(windows), allow(dead_code))]
    pub struct DlssgState {
        pub base: Base,
        pub estimated_vram_usage_in_bytes: u64,
        pub status: u32,
        pub min_width_or_height: u32,
        pub num_frames_actually_presented: u32,
        pub num_frames_to_generate_max: u32,
        pub reserved_4: u8,
        pub is_vsync_support_available: u8,
        pub inputs_processing_completion_fence: *mut c_void,
        pub last_present_inputs_processing_completion_fence_value: u64,
        pub is_dynamic_mfg_supported: u8,
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    impl DlssgState {
        pub fn new() -> Self {
            Self {
                base: base(DLSSG_STATE),
                estimated_vram_usage_in_bytes: 0,
                status: 0,
                min_width_or_height: 0,
                num_frames_actually_presented: 0,
                num_frames_to_generate_max: 0,
                reserved_4: 2,
                is_vsync_support_available: FALSE,
                inputs_processing_completion_fence: std::ptr::null_mut(),
                last_present_inputs_processing_completion_fence_value: 0,
                is_dynamic_mfg_supported: FALSE,
            }
        }
    }

    /// `sl::ReflexMode::eLowLatency`
    #[cfg_attr(not(windows), allow(dead_code))]
    pub const REFLEX_LOW_LATENCY: u32 = 1;

    /// `sl::ReflexOptions` (sl_reflex.h, version 1)
    #[repr(C)]
    #[cfg_attr(not(windows), allow(dead_code))]
    pub struct ReflexOptions {
        pub base: Base,
        pub mode: u32,
        pub frame_limit_us: u32,
        pub use_markers_to_optimize: bool,
        pub virtual_key: u16,
        pub id_thread: u32,
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    impl ReflexOptions {
        pub fn new(mode: u32) -> Self {
            Self {
                base: base(REFLEX_OPTIONS),
                mode,
                frame_limit_us: 0,
                use_markers_to_optimize: false,
                virtual_key: 0,
                id_thread: 0,
            }
        }
    }

    /// `sl::PCLMarker`: the frame's stages, each told with its token.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub mod pcl {
        pub const SIMULATION_START: u32 = 0;
        pub const SIMULATION_END: u32 = 1;
        pub const RENDER_SUBMIT_START: u32 = 2;
        pub const RENDER_SUBMIT_END: u32 = 3;
        pub const PRESENT_START: u32 = 4;
        pub const PRESENT_END: u32 = 5;
    }

    /// `sl::Preferences`
    #[repr(C)]
    pub struct Preferences {
        pub base: Base,
        pub show_console: bool,
        pub log_level: u32,
        pub paths_to_plugins: *const *const Wide,
        pub num_paths_to_plugins: u32,
        pub path_to_logs_and_data: *const Wide,
        pub allocate_callback: *const c_void,
        pub release_callback: *const c_void,
        pub log_message_callback: Option<unsafe extern "C" fn(u32, *const c_char)>,
        pub flags: u64,
        pub features_to_load: *const u32,
        pub num_features_to_load: u32,
        pub application_id: u32,
        pub engine: u32,
        pub engine_version: *const c_char,
        pub project_id: *const c_char,
        pub render_api: u32,
    }

    impl Preferences {
        pub fn new() -> Self {
            Self {
                base: base(PREFERENCES),
                show_console: false,
                log_level: LOG_DEFAULT,
                paths_to_plugins: std::ptr::null(),
                num_paths_to_plugins: 0,
                path_to_logs_and_data: std::ptr::null(),
                allocate_callback: std::ptr::null(),
                release_callback: std::ptr::null(),
                log_message_callback: None,
                flags: 0,
                features_to_load: std::ptr::null(),
                num_features_to_load: 0,
                application_id: 0,
                engine: ENGINE_CUSTOM,
                engine_version: std::ptr::null(),
                project_id: std::ptr::null(),
                render_api: VULKAN,
            }
        }
    }

    /// `sl::VulkanInfo` (Linux: the viewer opens the device itself and
    /// tells the library which; Windows: the interposer sees them made).
    #[repr(C)]
    #[cfg_attr(windows, allow(dead_code))]
    pub struct VulkanInfo {
        pub base: Base,
        pub device: vk::Device,
        pub instance: vk::Instance,
        pub physical_device: vk::PhysicalDevice,
        pub compute_queue_index: u32,
        pub compute_queue_family: u32,
        pub graphics_queue_index: u32,
        pub graphics_queue_family: u32,
        pub optical_flow_queue_index: u32,
        pub optical_flow_queue_family: u32,
        pub use_native_optical_flow_mode: bool,
        pub compute_queue_create_flags: u32,
        pub graphics_queue_create_flags: u32,
        pub optical_flow_queue_create_flags: u32,
    }

    impl VulkanInfo {
        #[cfg_attr(windows, allow(dead_code))]
        pub fn new(instance: vk::Instance, physical_device: vk::PhysicalDevice, device: vk::Device) -> Self {
            Self {
                base: base(VULKAN_INFO),
                device,
                instance,
                physical_device,
                compute_queue_index: 0,
                compute_queue_family: 0,
                graphics_queue_index: 0,
                graphics_queue_family: 0,
                optical_flow_queue_index: 0,
                optical_flow_queue_family: 0,
                use_native_optical_flow_mode: false,
                compute_queue_create_flags: 0,
                graphics_queue_create_flags: 0,
                optical_flow_queue_create_flags: 0,
            }
        }
    }

    /// `sl::FeatureRequirements`: the strings it points to are the
    /// library's, kept until its next call.
    #[repr(C)]
    pub struct FeatureRequirements {
        pub base: Base,
        pub flags: u32,
        pub max_num_cpu_threads: u32,
        pub max_num_viewports: u32,
        pub num_required_tags: u32,
        pub required_tags: *const c_void,
        pub os_version_detected: [u32; 3],
        pub os_version_required: [u32; 3],
        pub driver_version_detected: [u32; 3],
        pub driver_version_required: [u32; 3],
        pub vk_num_compute_queues_required: u32,
        pub vk_num_graphics_queues_required: u32,
        pub vk_num_device_extensions: u32,
        pub vk_device_extensions: *const *const c_char,
        pub vk_num_instance_extensions: u32,
        pub vk_instance_extensions: *const *const c_char,
        pub vk_num_features12: u32,
        pub vk_features12: *const *const c_char,
        pub vk_num_features13: u32,
        pub vk_features13: *const *const c_char,
        pub vk_num_optical_flow_queues_required: u32,
    }

    impl FeatureRequirements {
        pub fn new() -> Self {
            // SAFETY: plain integers and pointers, for which all zeroes is
            // a value (null, 0)
            let mut r: Self = unsafe { std::mem::zeroed() };
            r.base = base(FEATURE_REQUIREMENTS);
            r
        }

        /// The names in `list` (`count` of them), copied.
        ///
        /// # Safety
        /// `list` is the library's, from the call that filled `self`, and
        /// no other call has been made since.
        pub unsafe fn names(list: *const *const c_char, count: u32) -> Vec<std::ffi::CString> {
            if list.is_null() {
                return Vec::new();
            }
            (0..count as usize)
                .filter_map(|i| {
                    // SAFETY: `count` entries, each a nul-ended string or
                    // null (the caller's contract)
                    let p = unsafe { *list.add(i) };
                    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_owned())
                })
                .collect()
        }
    }

    /// `sl::Resource`, as a Vulkan image (its view, image, layout, size and
    /// format).
    #[repr(C)]
    pub struct Resource {
        pub base: Base,
        pub kind: u8,
        pub native: *mut c_void,
        pub memory: *mut c_void,
        pub view: *mut c_void,
        /// the image's `VkImageLayout` as tagged
        pub state: u32,
        pub width: u32,
        pub height: u32,
        /// `VkFormat`
        pub native_format: u32,
        pub mip_levels: u32,
        pub array_layers: u32,
        pub gpu_virtual_address: u64,
        /// `VkImageCreateFlags`
        pub flags: u32,
        /// `VkImageUsageFlags`
        pub usage: u32,
        pub internal_flags: u32,
    }

    impl Resource {
        #[allow(clippy::too_many_arguments)]
        pub fn image(
            image: vk::Image,
            view: vk::ImageView,
            layout: vk::ImageLayout,
            format: vk::Format,
            usage: vk::ImageUsageFlags,
            width: u32,
            height: u32,
        ) -> Self {
            use ash::vk::Handle;
            Self {
                base: base(RESOURCE),
                kind: TEX_2D,
                native: image.as_raw() as *mut c_void,
                memory: std::ptr::null_mut(),
                view: view.as_raw() as *mut c_void,
                state: layout.as_raw() as u32,
                width,
                height,
                native_format: format.as_raw() as u32,
                mip_levels: 1,
                array_layers: 1,
                gpu_virtual_address: 0,
                flags: 0,
                usage: usage.as_raw(),
                internal_flags: 0,
            }
        }
    }

    /// `sl::Extent`
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct Extent {
        pub top: u32,
        pub left: u32,
        pub width: u32,
        pub height: u32,
    }

    /// `sl::ResourceTag`
    #[repr(C)]
    pub struct ResourceTag {
        pub base: Base,
        pub resource: *mut Resource,
        pub kind: u32,
        pub lifecycle: u32,
        pub extent: Extent,
    }

    impl ResourceTag {
        pub fn new(resource: &mut Resource, kind: u32, extent: Extent) -> Self {
            Self {
                base: base(RESOURCE_TAG),
                resource,
                kind,
                lifecycle: VALID_UNTIL_EVALUATE,
                extent,
            }
        }

        /// Valid until the present ([`VALID_UNTIL_PRESENT`]): Windows'
        /// frame generation's inputs, the viewer's own copies. A tag valid
        /// until the evaluate, or only now, of an input frame generation
        /// has read at a present is copied by the library in the tag's
        /// command buffer (Streamline 2.14.16,
        /// `ResourceTaggingForFrame::setTag`), which needs the image to be
        /// a copy source.
        #[cfg_attr(not(windows), allow(dead_code))]
        pub fn until_present(mut self) -> Self {
            self.lifecycle = VALID_UNTIL_PRESENT;
            self
        }
    }

    /// `sl::float4x4`: four rows (row major, as the header says).
    pub type Float4x4 = [[f32; 4]; 4];

    /// `sl::Constants`
    #[repr(C)]
    pub struct Constants {
        pub base: Base,
        pub camera_view_to_clip: Float4x4,
        pub clip_to_camera_view: Float4x4,
        pub clip_to_lens_clip: Float4x4,
        pub clip_to_prev_clip: Float4x4,
        pub prev_clip_to_clip: Float4x4,
        pub jitter_offset: [f32; 2],
        pub mvec_scale: [f32; 2],
        pub camera_pinhole_offset: [f32; 2],
        pub camera_pos: [f32; 3],
        pub camera_up: [f32; 3],
        pub camera_right: [f32; 3],
        pub camera_fwd: [f32; 3],
        pub camera_near: f32,
        pub camera_far: f32,
        pub camera_fov: f32,
        pub camera_aspect_ratio: f32,
        pub motion_vectors_invalid_value: f32,
        pub depth_inverted: u8,
        pub camera_motion_included: u8,
        pub motion_vectors_3d: u8,
        pub reset: u8,
        pub orthographic_projection: u8,
        pub motion_vectors_dilated: u8,
        pub motion_vectors_jittered: u8,
        pub min_relative_linear_depth_object_separation: f32,
    }

    // SAFETY: `base.next` is always null: nothing is chained to the
    // constants, so the struct is plain data
    unsafe impl Send for Constants {}
    // SAFETY: as above
    unsafe impl Sync for Constants {}

    /// `sl::INVALID_FLOAT`: a value the viewer has no answer for.
    pub const INVALID_FLOAT: f32 = f32::MAX;

    impl Constants {
        pub fn new() -> Self {
            Self {
                base: base(CONSTANTS),
                camera_view_to_clip: [[INVALID_FLOAT; 4]; 4],
                clip_to_camera_view: [[INVALID_FLOAT; 4]; 4],
                clip_to_lens_clip: [[INVALID_FLOAT; 4]; 4],
                clip_to_prev_clip: [[INVALID_FLOAT; 4]; 4],
                prev_clip_to_clip: [[INVALID_FLOAT; 4]; 4],
                jitter_offset: [INVALID_FLOAT; 2],
                mvec_scale: [INVALID_FLOAT; 2],
                camera_pinhole_offset: [INVALID_FLOAT; 2],
                camera_pos: [INVALID_FLOAT; 3],
                camera_up: [INVALID_FLOAT; 3],
                camera_right: [INVALID_FLOAT; 3],
                camera_fwd: [INVALID_FLOAT; 3],
                camera_near: INVALID_FLOAT,
                camera_far: INVALID_FLOAT,
                camera_fov: INVALID_FLOAT,
                camera_aspect_ratio: INVALID_FLOAT,
                motion_vectors_invalid_value: INVALID_FLOAT,
                depth_inverted: 2,
                camera_motion_included: 2,
                motion_vectors_3d: 2,
                reset: 2,
                orthographic_projection: FALSE,
                motion_vectors_dilated: FALSE,
                motion_vectors_jittered: FALSE,
                min_relative_linear_depth_object_separation: 40.0,
            }
        }
    }

    /// `sl::DLSSOptions`
    #[repr(C)]
    pub struct DlssOptions {
        pub base: Base,
        pub mode: u32,
        pub output_width: u32,
        pub output_height: u32,
        pub sharpness: f32,
        pub pre_exposure: f32,
        pub exposure_scale: f32,
        pub color_buffers_hdr: u8,
        pub indicator_invert_axis_x: u8,
        pub indicator_invert_axis_y: u8,
        pub dlaa_preset: u32,
        pub quality_preset: u32,
        pub balanced_preset: u32,
        pub performance_preset: u32,
        pub ultra_performance_preset: u32,
        pub ultra_quality_preset: u32,
        pub use_auto_exposure: u8,
        pub alpha_upscaling_enabled: u8,
    }

    impl DlssOptions {
        pub fn new(mode: u32, output: (u32, u32)) -> Self {
            Self {
                base: base(DLSS_OPTIONS),
                mode,
                output_width: output.0,
                output_height: output.1,
                sharpness: 0.0,
                pre_exposure: 1.0,
                exposure_scale: 1.0,
                color_buffers_hdr: TRUE,
                indicator_invert_axis_x: FALSE,
                indicator_invert_axis_y: FALSE,
                dlaa_preset: 0,
                quality_preset: 0,
                balanced_preset: 0,
                performance_preset: 0,
                ultra_performance_preset: 0,
                ultra_quality_preset: 0,
                use_auto_exposure: FALSE,
                alpha_upscaling_enabled: FALSE,
            }
        }

        /// The same model for every mode.
        pub fn preset(&mut self, preset: u32) {
            self.dlaa_preset = preset;
            self.quality_preset = preset;
            self.balanced_preset = preset;
            self.performance_preset = preset;
            self.ultra_performance_preset = preset;
            self.ultra_quality_preset = preset;
        }
    }

    /// `sl::DLSSOptimalSettings`
    #[repr(C)]
    pub struct DlssOptimalSettings {
        pub base: Base,
        pub optimal_render_width: u32,
        pub optimal_render_height: u32,
        pub optimal_sharpness: f32,
        pub render_width_min: u32,
        pub render_height_min: u32,
        pub render_width_max: u32,
        pub render_height_max: u32,
    }

    impl DlssOptimalSettings {
        pub fn new() -> Self {
            Self {
                base: base(DLSS_OPTIMAL_SETTINGS),
                optimal_render_width: 0,
                optimal_render_height: 0,
                optimal_sharpness: 0.0,
                render_width_min: 0,
                render_height_min: 0,
                render_width_max: 0,
                render_height_max: 0,
            }
        }
    }

    /// `sl::DLSSDOptions` (Ray Reconstruction's)
    #[repr(C)]
    pub struct DlssdOptions {
        pub base: Base,
        pub mode: u32,
        pub output_width: u32,
        pub output_height: u32,
        pub sharpness: f32,
        pub pre_exposure: f32,
        pub exposure_scale: f32,
        pub color_buffers_hdr: u8,
        pub indicator_invert_axis_x: u8,
        pub indicator_invert_axis_y: u8,
        pub normal_roughness_mode: u32,
        pub world_to_camera_view: Float4x4,
        pub camera_view_to_world: Float4x4,
        pub alpha_upscaling_enabled: u8,
        pub dlaa_preset: u32,
        pub quality_preset: u32,
        pub balanced_preset: u32,
        pub performance_preset: u32,
        pub ultra_performance_preset: u32,
        pub ultra_quality_preset: u32,
    }

    impl DlssdOptions {
        /// Roughness in the normals' w; the matrices are the identity
        /// until the frame's are set.
        pub fn new(mode: u32, output: (u32, u32)) -> Self {
            const IDENTITY: Float4x4 = [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ];
            Self {
                base: base(DLSSD_OPTIONS),
                mode,
                output_width: output.0,
                output_height: output.1,
                sharpness: 0.0,
                pre_exposure: 1.0,
                exposure_scale: 1.0,
                color_buffers_hdr: TRUE,
                indicator_invert_axis_x: FALSE,
                indicator_invert_axis_y: FALSE,
                normal_roughness_mode: ROUGHNESS_PACKED,
                world_to_camera_view: IDENTITY,
                camera_view_to_world: IDENTITY,
                alpha_upscaling_enabled: FALSE,
                dlaa_preset: 0,
                quality_preset: 0,
                balanced_preset: 0,
                performance_preset: 0,
                ultra_performance_preset: 0,
                ultra_quality_preset: 0,
            }
        }

        /// The same model for every mode.
        pub fn preset(&mut self, preset: u32) {
            self.dlaa_preset = preset;
            self.quality_preset = preset;
            self.balanced_preset = preset;
            self.performance_preset = preset;
            self.ultra_performance_preset = preset;
            self.ultra_quality_preset = preset;
        }
    }

    /// `sl::DLSSDOptimalSettings`: `sl::DLSSOptimalSettings`' fields under
    /// its own id.
    pub fn dlssd_optimal_settings() -> DlssOptimalSettings {
        let mut s = DlssOptimalSettings::new();
        s.base = base(DLSSD_OPTIMAL_SETTINGS);
        s
    }

    /// `sl::ViewportHandle`
    #[repr(C)]
    pub struct ViewportHandle {
        pub base: Base,
        pub value: u32,
    }

    impl ViewportHandle {
        pub fn new(value: u32) -> Self {
            Self {
                base: base(VIEWPORT_HANDLE),
                value,
            }
        }
    }

    /// `sl::AdapterInfo`, naming the adapter by its Vulkan handle.
    #[repr(C)]
    pub struct AdapterInfo {
        pub base: Base,
        pub device_luid: *const u8,
        pub device_luid_size_in_bytes: u32,
        pub vk_physical_device: vk::PhysicalDevice,
    }

    impl AdapterInfo {
        pub fn new(physical_device: vk::PhysicalDevice) -> Self {
            Self {
                base: base(ADAPTER_INFO),
                device_luid: std::ptr::null(),
                device_luid_size_in_bytes: 0,
                vk_physical_device: physical_device,
            }
        }
    }

    /// `sl::FrameToken`: the library's own object, known by its address.
    pub type FrameToken = *mut c_void;

    // sizes and offsets a C++ compiler gives Streamline's headers on x86-64
    // Linux (sl-ngx's tests/layout.expected); the Windows ABI lays these
    // structs out the same (no `long`, no `wchar_t` by value)
    const _: () = {
        use std::mem::{align_of, offset_of, size_of};
        assert!(size_of::<Base>() == 32 && offset_of!(Base, id) == 8 && offset_of!(Base, version) == 24);
        assert!(size_of::<Resource>() == 112);
        assert!(offset_of!(Resource, kind) == 32 && offset_of!(Resource, native) == 40);
        assert!(offset_of!(Resource, memory) == 48 && offset_of!(Resource, view) == 56);
        assert!(offset_of!(Resource, state) == 64 && offset_of!(Resource, width) == 68);
        assert!(offset_of!(Resource, height) == 72 && offset_of!(Resource, native_format) == 76);
        assert!(offset_of!(Resource, mip_levels) == 80 && offset_of!(Resource, array_layers) == 84);
        assert!(offset_of!(Resource, gpu_virtual_address) == 88 && offset_of!(Resource, flags) == 96);
        assert!(offset_of!(Resource, usage) == 100 && offset_of!(Resource, internal_flags) == 104);
        assert!(size_of::<ResourceTag>() == 64 && offset_of!(ResourceTag, resource) == 32);
        assert!(offset_of!(ResourceTag, kind) == 40 && offset_of!(ResourceTag, lifecycle) == 44);
        assert!(offset_of!(ResourceTag, extent) == 48);
        assert!(size_of::<Extent>() == 16 && align_of::<Extent>() == 4);
        assert!(size_of::<Constants>() == 456);
        assert!(offset_of!(Constants, camera_view_to_clip) == 32 && offset_of!(Constants, clip_to_camera_view) == 96);
        assert!(offset_of!(Constants, clip_to_lens_clip) == 160 && offset_of!(Constants, clip_to_prev_clip) == 224);
        assert!(offset_of!(Constants, prev_clip_to_clip) == 288 && offset_of!(Constants, jitter_offset) == 352);
        assert!(offset_of!(Constants, mvec_scale) == 360 && offset_of!(Constants, camera_pinhole_offset) == 368);
        assert!(offset_of!(Constants, camera_pos) == 376 && offset_of!(Constants, camera_up) == 388);
        assert!(offset_of!(Constants, camera_right) == 400 && offset_of!(Constants, camera_fwd) == 412);
        assert!(offset_of!(Constants, camera_near) == 424 && offset_of!(Constants, camera_far) == 428);
        assert!(offset_of!(Constants, camera_fov) == 432 && offset_of!(Constants, camera_aspect_ratio) == 436);
        assert!(offset_of!(Constants, motion_vectors_invalid_value) == 440);
        assert!(offset_of!(Constants, depth_inverted) == 444 && offset_of!(Constants, camera_motion_included) == 445);
        assert!(offset_of!(Constants, motion_vectors_3d) == 446 && offset_of!(Constants, reset) == 447);
        assert!(offset_of!(Constants, orthographic_projection) == 448);
        assert!(offset_of!(Constants, motion_vectors_dilated) == 449);
        assert!(offset_of!(Constants, motion_vectors_jittered) == 450);
        assert!(offset_of!(Constants, min_relative_linear_depth_object_separation) == 452);
        assert!(size_of::<DlssOptions>() == 88);
        assert!(offset_of!(DlssOptions, mode) == 32 && offset_of!(DlssOptions, output_width) == 36);
        assert!(offset_of!(DlssOptions, output_height) == 40 && offset_of!(DlssOptions, sharpness) == 44);
        assert!(offset_of!(DlssOptions, pre_exposure) == 48 && offset_of!(DlssOptions, exposure_scale) == 52);
        assert!(offset_of!(DlssOptions, color_buffers_hdr) == 56);
        assert!(offset_of!(DlssOptions, indicator_invert_axis_x) == 57);
        assert!(offset_of!(DlssOptions, indicator_invert_axis_y) == 58);
        assert!(offset_of!(DlssOptions, dlaa_preset) == 60 && offset_of!(DlssOptions, quality_preset) == 64);
        assert!(offset_of!(DlssOptions, balanced_preset) == 68 && offset_of!(DlssOptions, performance_preset) == 72);
        assert!(offset_of!(DlssOptions, ultra_performance_preset) == 76);
        assert!(offset_of!(DlssOptions, ultra_quality_preset) == 80);
        assert!(offset_of!(DlssOptions, use_auto_exposure) == 84);
        assert!(offset_of!(DlssOptions, alpha_upscaling_enabled) == 85);
        assert!(size_of::<DlssdOptions>() == 224 && align_of::<DlssdOptions>() == 8);
        assert!(offset_of!(DlssdOptions, mode) == 32 && offset_of!(DlssdOptions, output_width) == 36);
        assert!(offset_of!(DlssdOptions, output_height) == 40 && offset_of!(DlssdOptions, sharpness) == 44);
        assert!(offset_of!(DlssdOptions, pre_exposure) == 48 && offset_of!(DlssdOptions, exposure_scale) == 52);
        assert!(offset_of!(DlssdOptions, color_buffers_hdr) == 56);
        assert!(offset_of!(DlssdOptions, indicator_invert_axis_x) == 57);
        assert!(offset_of!(DlssdOptions, indicator_invert_axis_y) == 58);
        assert!(offset_of!(DlssdOptions, normal_roughness_mode) == 60);
        assert!(offset_of!(DlssdOptions, world_to_camera_view) == 64);
        assert!(offset_of!(DlssdOptions, camera_view_to_world) == 128);
        assert!(offset_of!(DlssdOptions, alpha_upscaling_enabled) == 192);
        assert!(offset_of!(DlssdOptions, dlaa_preset) == 196 && offset_of!(DlssdOptions, quality_preset) == 200);
        assert!(offset_of!(DlssdOptions, balanced_preset) == 204 && offset_of!(DlssdOptions, performance_preset) == 208);
        assert!(offset_of!(DlssdOptions, ultra_performance_preset) == 212);
        assert!(offset_of!(DlssdOptions, ultra_quality_preset) == 216);
        assert!(size_of::<DlssOptimalSettings>() == 64);
        assert!(offset_of!(DlssOptimalSettings, optimal_render_width) == 32);
        assert!(offset_of!(DlssOptimalSettings, optimal_render_height) == 36);
        assert!(offset_of!(DlssOptimalSettings, optimal_sharpness) == 40);
        assert!(offset_of!(DlssOptimalSettings, render_width_min) == 44);
        assert!(offset_of!(DlssOptimalSettings, render_height_min) == 48);
        assert!(offset_of!(DlssOptimalSettings, render_width_max) == 52);
        assert!(offset_of!(DlssOptimalSettings, render_height_max) == 56);
        assert!(size_of::<Preferences>() == 144);
        assert!(offset_of!(Preferences, show_console) == 32 && offset_of!(Preferences, log_level) == 36);
        assert!(offset_of!(Preferences, paths_to_plugins) == 40);
        assert!(offset_of!(Preferences, num_paths_to_plugins) == 48);
        assert!(offset_of!(Preferences, path_to_logs_and_data) == 56);
        assert!(offset_of!(Preferences, allocate_callback) == 64);
        assert!(offset_of!(Preferences, release_callback) == 72);
        assert!(offset_of!(Preferences, log_message_callback) == 80);
        assert!(offset_of!(Preferences, flags) == 88 && offset_of!(Preferences, features_to_load) == 96);
        assert!(offset_of!(Preferences, num_features_to_load) == 104);
        assert!(offset_of!(Preferences, application_id) == 108 && offset_of!(Preferences, engine) == 112);
        assert!(offset_of!(Preferences, engine_version) == 120 && offset_of!(Preferences, project_id) == 128);
        assert!(offset_of!(Preferences, render_api) == 136);
        assert!(size_of::<VulkanInfo>() == 96);
        assert!(offset_of!(VulkanInfo, device) == 32 && offset_of!(VulkanInfo, instance) == 40);
        assert!(offset_of!(VulkanInfo, physical_device) == 48);
        assert!(offset_of!(VulkanInfo, compute_queue_index) == 56);
        assert!(offset_of!(VulkanInfo, compute_queue_family) == 60);
        assert!(offset_of!(VulkanInfo, graphics_queue_index) == 64);
        assert!(offset_of!(VulkanInfo, graphics_queue_family) == 68);
        assert!(offset_of!(VulkanInfo, optical_flow_queue_index) == 72);
        assert!(offset_of!(VulkanInfo, optical_flow_queue_family) == 76);
        assert!(offset_of!(VulkanInfo, use_native_optical_flow_mode) == 80);
        assert!(offset_of!(VulkanInfo, compute_queue_create_flags) == 84);
        assert!(offset_of!(VulkanInfo, graphics_queue_create_flags) == 88);
        assert!(offset_of!(VulkanInfo, optical_flow_queue_create_flags) == 92);
        assert!(size_of::<FeatureRequirements>() == 184);
        assert!(offset_of!(FeatureRequirements, flags) == 32);
        assert!(offset_of!(FeatureRequirements, max_num_cpu_threads) == 36);
        assert!(offset_of!(FeatureRequirements, max_num_viewports) == 40);
        assert!(offset_of!(FeatureRequirements, num_required_tags) == 44);
        assert!(offset_of!(FeatureRequirements, required_tags) == 48);
        assert!(offset_of!(FeatureRequirements, os_version_detected) == 56);
        assert!(offset_of!(FeatureRequirements, os_version_required) == 68);
        assert!(offset_of!(FeatureRequirements, driver_version_detected) == 80);
        assert!(offset_of!(FeatureRequirements, driver_version_required) == 92);
        assert!(offset_of!(FeatureRequirements, vk_num_compute_queues_required) == 104);
        assert!(offset_of!(FeatureRequirements, vk_num_graphics_queues_required) == 108);
        assert!(offset_of!(FeatureRequirements, vk_num_device_extensions) == 112);
        assert!(offset_of!(FeatureRequirements, vk_device_extensions) == 120);
        assert!(offset_of!(FeatureRequirements, vk_num_instance_extensions) == 128);
        assert!(offset_of!(FeatureRequirements, vk_instance_extensions) == 136);
        assert!(offset_of!(FeatureRequirements, vk_num_features12) == 144);
        assert!(offset_of!(FeatureRequirements, vk_features12) == 152);
        assert!(offset_of!(FeatureRequirements, vk_num_features13) == 160);
        assert!(offset_of!(FeatureRequirements, vk_features13) == 168);
        assert!(offset_of!(FeatureRequirements, vk_num_optical_flow_queues_required) == 176);
        assert!(size_of::<AdapterInfo>() == 56 && offset_of!(AdapterInfo, device_luid) == 32);
        assert!(offset_of!(AdapterInfo, device_luid_size_in_bytes) == 40);
        assert!(offset_of!(AdapterInfo, vk_physical_device) == 48);
        assert!(size_of::<ViewportHandle>() == 40 && offset_of!(ViewportHandle, value) == 32);
        // (sl_dlss_g.h and sl_reflex.h of 2.14.16, measured with clang)
        assert!(size_of::<DlssgOptions>() == 120 && offset_of!(DlssgOptions, mode) == 32);
        assert!(offset_of!(DlssgOptions, num_back_buffers) == 52 && offset_of!(DlssgOptions, mvec_depth_width) == 56);
        assert!(offset_of!(DlssgOptions, color_buffer_format) == 72 && offset_of!(DlssgOptions, ui_buffer_format) == 88);
        assert!(offset_of!(DlssgOptions, on_error_callback) == 96 && offset_of!(DlssgOptions, reserved_15) == 104);
        assert!(offset_of!(DlssgOptions, queue_parallelism_mode) == 108);
        assert!(offset_of!(DlssgOptions, enable_user_interface_recomposition) == 112);
        assert!(offset_of!(DlssgOptions, dynamic_target_frame_rate) == 116);
        assert!(size_of::<DlssgState>() == 88 && offset_of!(DlssgState, status) == 40);
        assert!(offset_of!(DlssgState, num_frames_actually_presented) == 48);
        assert!(offset_of!(DlssgState, num_frames_to_generate_max) == 52 && offset_of!(DlssgState, reserved_4) == 56);
        assert!(offset_of!(DlssgState, inputs_processing_completion_fence) == 64);
        assert!(offset_of!(DlssgState, is_dynamic_mfg_supported) == 80);
        assert!(size_of::<ReflexOptions>() == 48 && offset_of!(ReflexOptions, use_markers_to_optimize) == 40);
        assert!(offset_of!(ReflexOptions, virtual_key) == 42 && offset_of!(ReflexOptions, id_thread) == 44);
        // sl-ngx's presenter (include/sl_ngx/present.h), Linux only
        #[cfg(target_os = "linux")]
        {
            use present::{Desc, Frame, Image, Stats};
            assert!(size_of::<Desc>() == 48 && align_of::<Desc>() == 8);
            assert!(offset_of!(Desc, surface) == 8 && offset_of!(Desc, queue_family) == 16);
            assert!(offset_of!(Desc, device_bits) == 24 && offset_of!(Desc, width) == 28);
            assert!(offset_of!(Desc, format) == 36 && offset_of!(Desc, color_space) == 40);
            assert!(offset_of!(Desc, min_image_count) == 44);
            assert!(size_of::<Image>() == 24 && offset_of!(Image, layout) == 8 && offset_of!(Image, height) == 16);
            assert!(size_of::<Frame>() == 88 && offset_of!(Frame, frame) == 4 && offset_of!(Frame, generated) == 8);
            assert!(offset_of!(Frame, real) == 32 && offset_of!(Frame, ready) == 56);
            assert!(offset_of!(Frame, ready_value) == 64 && offset_of!(Frame, released) == 72);
            assert!(offset_of!(Frame, released_value) == 80);
            assert!(size_of::<Stats>() == 104 && offset_of!(Stats, frames_accepted) == 8);
            assert!(offset_of!(Stats, present_timing) == 56 && offset_of!(Stats, swapchains_made) == 68);
            assert!(offset_of!(Stats, copy_gpu_ms) == 80 && offset_of!(Stats, last_error) == 92);
            assert!(offset_of!(Stats, device_lost) == 96);
        }
        assert!(size_of::<ReflexOptions>() == 48 && offset_of!(ReflexOptions, mode) == 32);
        assert!(offset_of!(ReflexOptions, frame_limit_us) == 36 && offset_of!(ReflexOptions, use_markers_to_optimize) == 40);
        assert!(offset_of!(ReflexOptions, virtual_key) == 42 && offset_of!(ReflexOptions, id_thread) == 44);
    };

    /// `wchar_t`: one UTF-32 unit on Linux, one UTF-16 unit on Windows.
    #[cfg(target_os = "linux")]
    pub type Wide = u32;
    #[cfg(windows)]
    pub type Wide = u16;

    /// `s` as a `wchar_t` string, ended by a nul.
    pub fn wide(s: &str) -> Vec<Wide> {
        #[cfg(target_os = "linux")]
        return s.chars().map(|c| c as Wide).chain([0]).collect();
        #[cfg(windows)]
        return s.encode_utf16().chain([0]).collect();
    }

    /// The functions this module calls, with the arguments `sl.h`,
    /// `sl_dlss.h` and `sl_helpers_vk.h` give them (a C++ reference is a
    /// pointer).
    pub struct Lib {
        pub init: unsafe extern "C" fn(*const Preferences, u64) -> i32,
        pub shutdown: unsafe extern "C" fn() -> i32,
        pub is_feature_supported: unsafe extern "C" fn(u32, *const AdapterInfo) -> i32,
        pub feature_requirements: unsafe extern "C" fn(u32, *mut FeatureRequirements) -> i32,
        /// (Linux only: Windows' interposer sees the instance and device)
        #[cfg_attr(windows, allow(dead_code))]
        pub set_vulkan_info: unsafe extern "C" fn(*const VulkanInfo) -> i32,
        pub feature_function: unsafe extern "C" fn(u32, *const c_char, *mut *mut c_void) -> i32,
        pub new_frame_token: unsafe extern "C" fn(*mut FrameToken, *const u32) -> i32,
        pub set_constants: unsafe extern "C" fn(*const Constants, FrameToken, *const ViewportHandle) -> i32,
        pub set_tag_for_frame: unsafe extern "C" fn(FrameToken, *const ViewportHandle, *const ResourceTag, u32, vk::CommandBuffer) -> i32,
        pub evaluate: unsafe extern "C" fn(u32, FrameToken, *const *const Base, u32, vk::CommandBuffer) -> i32,
        pub allocate: unsafe extern "C" fn(vk::CommandBuffer, u32, *const ViewportHandle) -> i32,
        pub free: unsafe extern "C" fn(u32, *const ViewportHandle) -> i32,
        /// where the library is, for `pathsToPlugins` and its log
        pub dir: std::path::PathBuf,
        /// Windows: the interposer's `vkGetInstanceProcAddr`, the Vulkan
        /// loader the viewer's instance and device are made through, so
        /// that the library sees them made and their swapchains
        #[cfg(windows)]
        pub get_instance_proc_addr: vk::PFN_vkGetInstanceProcAddr,
        /// Linux: sl-ngx's presenter, when this sl-ngx has it
        #[cfg(target_os = "linux")]
        pub presenter: Option<present::Fns>,
        /// the library's own module, kept open for the process
        #[allow(dead_code)]
        pub handle: *mut c_void,
    }

    // SAFETY: the handle and function addresses are the process's for its
    // life (never closed); calls go through Streamline's lock in `imp`
    unsafe impl Send for Lib {}
    unsafe impl Sync for Lib {}

    /// The DLSS functions, from `slGetFeatureFunction`.
    pub struct Dlss {
        pub optimal_settings: unsafe extern "C" fn(*const DlssOptions, *mut DlssOptimalSettings) -> i32,
        pub set_options: unsafe extern "C" fn(*const ViewportHandle, *const DlssOptions) -> i32,
    }

    /// Ray Reconstruction's functions, from `slGetFeatureFunction`.
    pub struct Dlssd {
        pub optimal_settings: unsafe extern "C" fn(*const DlssdOptions, *mut DlssOptimalSettings) -> i32,
        pub set_options: unsafe extern "C" fn(*const ViewportHandle, *const DlssdOptions) -> i32,
    }

    /// Windows' frame generation and the Reflex and PCL functions it runs
    /// with, from `slGetFeatureFunction` (sl_dlss_g.h, sl_reflex.h,
    /// sl_pcl.h; a `const FrameToken&` is the token's address, as for
    /// `slSetConstants`).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub struct Dlssg {
        pub set_options: unsafe extern "C" fn(*const ViewportHandle, *const DlssgOptions) -> i32,
        pub get_state: unsafe extern "C" fn(*const ViewportHandle, *mut DlssgState, *const DlssgOptions) -> i32,
        pub reflex_set_options: unsafe extern "C" fn(*const ReflexOptions) -> i32,
        pub reflex_sleep: unsafe extern "C" fn(FrameToken) -> i32,
        pub pcl_set_marker: unsafe extern "C" fn(u32, FrameToken) -> i32,
    }

    /// Linux: Reflex's and PCL's functions alone, from `slGetFeatureFunction`
    /// (sl-ngx has no `slDLSSG*`: its frame generation is evaluated, and
    /// shown by its presenter, [`present`]).
    #[cfg(target_os = "linux")]
    pub struct Latency {
        pub reflex_set_options: unsafe extern "C" fn(*const ReflexOptions) -> i32,
        pub reflex_sleep: unsafe extern "C" fn(FrameToken) -> i32,
        pub pcl_set_marker: unsafe extern "C" fn(u32, FrameToken) -> i32,
    }

    /// sl-ngx's presenter (`include/sl_ngx/present.h`, MIT, sl-ngx's
    /// own, not Streamline's): frame generation shown without swapchain
    /// hooks. The application hands it a surface it does not present to,
    /// and once per rendered frame the generated frame and the rendered
    /// one; a thread of the presenter, on a queue of its own, copies and
    /// presents both, half a frame apart, with VK_NV_low_latency2 on its
    /// swapchain so that Reflex works. Found by name ([`Lib::presenter`]):
    /// an sl-ngx without it presents the prototype's way (dlss_fg.rs).
    #[cfg(target_os = "linux")]
    pub mod present {
        use ash::vk;
        use std::ffi::c_void;

        /// `SLNGX_PRESENTER_VERSION`
        pub const VERSION: u32 = 1;

        /// `SlngxPresenterDesc`
        #[repr(C)]
        pub struct Desc {
            pub version: u32,
            pub surface: vk::SurfaceKHR,
            pub queue_family: u32,
            pub queue_index: u32,
            pub device_bits: u32,
            pub width: u32,
            pub height: u32,
            pub format: vk::Format,
            pub color_space: vk::ColorSpaceKHR,
            pub min_image_count: u32,
        }

        /// `SlngxPresentImage`
        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct Image {
            pub image: vk::Image,
            pub layout: vk::ImageLayout,
            pub width: u32,
            pub height: u32,
        }

        /// `SlngxPresentFrame`
        #[repr(C)]
        pub struct Frame {
            pub version: u32,
            pub frame: u32,
            pub generated: Image,
            pub real: Image,
            pub ready: vk::Semaphore,
            pub ready_value: u64,
            pub released: vk::Semaphore,
            pub released_value: u64,
        }

        /// `SlngxPresenterStats`
        #[repr(C)]
        #[derive(Clone, Copy, Default, Debug)]
        pub struct Stats {
            pub version: u32,
            pub frames_accepted: u64,
            pub frames_skipped: u64,
            pub generated_presented: u64,
            pub real_presented: u64,
            pub generated_shown: u64,
            pub real_shown: u64,
            pub present_timing: u32,
            pub low_latency: u32,
            pub latency_reports: u32,
            pub swapchains_made: u32,
            pub width: u32,
            pub height: u32,
            pub copy_gpu_ms: f32,
            pub interval_ms: f32,
            pub present_half_ms: f32,
            pub last_error: i32,
            pub device_lost: u32,
        }

        /// The presenter's functions (`PFN_slngx*`).
        pub struct Fns {
            pub create: unsafe extern "C" fn(*const Desc, *mut *mut c_void) -> vk::Result,
            pub present: unsafe extern "C" fn(*mut c_void, *const Frame, u64) -> vk::Result,
            pub resize: unsafe extern "C" fn(*mut c_void, u32, u32),
            pub pause: unsafe extern "C" fn(*mut c_void) -> vk::Result,
            pub resume: unsafe extern "C" fn(*mut c_void),
            pub destroy: unsafe extern "C" fn(*mut c_void),
            pub get_stats: unsafe extern "C" fn(*mut c_void, *mut Stats) -> vk::Result,
        }
    }

    /// The library the user put in the folder: sl-ngx built on their
    /// machine (Linux), NVIDIA's signed Streamline (Windows).
    #[cfg(target_os = "linux")]
    pub const SHIM: &str = "libsl_ngx.so";
    #[cfg(windows)]
    pub const SHIM: &str = "sl.interposer.dll";

    /// What the user runs to set DLSS up (docs/DLSS.md).
    pub const SETUP: &str = crate::DLSS_SETUP;

    /// Opens the library at `path`, or says why not.
    ///
    /// # Safety
    /// Loading runs the library's initialisers: `path` is the user's
    /// Streamline library (`SHIM`), nothing else.
    #[cfg(target_os = "linux")]
    unsafe fn open(path: &std::path::Path) -> Result<*mut c_void, String> {
        let file = std::ffi::CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        // SAFETY: `file` is a nul-ended path; the library is the user's
        // own, opened once and never closed.
        let handle = unsafe { libc::dlopen(file.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if !handle.is_null() {
            return Ok(handle);
        }
        // SAFETY: dlerror's string, when there is one, lives until the next
        // dl call on this thread; it is copied at once.
        let why = unsafe { libc::dlerror() };
        Err(if why.is_null() {
            String::from("no reason given")
        } else {
            // SAFETY: non-null, nul-ended (above)
            unsafe { CStr::from_ptr(why) }.to_string_lossy().into_owned()
        })
    }

    /// # Safety
    /// Loading runs the library's initialisers: `path` is the user's
    /// Streamline library (`SHIM`), nothing else.
    #[cfg(windows)]
    unsafe fn open(path: &std::path::Path) -> Result<*mut c_void, String> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::System::LibraryLoader::{LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW};
        // the altered search path is defined for an absolute path only
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        let file: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: `file` is a nul-ended UTF-16 path; the altered search path
        // finds the library's own plugins beside it. Opened once, never
        // freed.
        let handle = unsafe { LoadLibraryExW(file.as_ptr(), std::ptr::null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH) };
        if handle.is_null() {
            Err(std::io::Error::last_os_error().to_string())
        } else {
            Ok(handle)
        }
    }

    /// The address of `name` in the library `open` gave, or null.
    ///
    /// # Safety
    /// `handle` is a library `open` gave, never closed.
    pub unsafe fn symbol(handle: *mut c_void, name: &CStr) -> *mut c_void {
        #[cfg(target_os = "linux")]
        // SAFETY: `handle` is a live dlopen handle, `name` nul-ended
        return unsafe { libc::dlsym(handle, name.as_ptr()) };
        #[cfg(windows)]
        // SAFETY: `handle` is a live module handle, `name` nul-ended ASCII
        return unsafe { windows_sys::Win32::System::LibraryLoader::GetProcAddress(handle, name.as_ptr().cast()) }
            .map_or(std::ptr::null_mut(), |f| f as *mut c_void);
    }

    /// `p`, an address the library gave for a function, as the pointer type
    /// `F` its header gives that function, or `None` for null.
    ///
    /// # Safety
    /// `F` is that function pointer type.
    unsafe fn as_fn<F: Copy>(p: *mut c_void) -> Option<F> {
        // (a function pointer type is always an address wide: this only
        // refuses a wrong `F` instead of reading past `p`)
        if p.is_null() || std::mem::size_of::<F>() != std::mem::size_of::<usize>() {
            return None;
        }
        // SAFETY: a non-null address read as the pointer type the header
        // gives it (the caller's contract), of the same size (checked)
        Some(unsafe { std::mem::transmute_copy(&p) })
    }

    static LIB: std::sync::OnceLock<Option<Lib>> = std::sync::OnceLock::new();

    /// The library, when [`lib`] has loaded it.
    pub fn loaded() -> Option<&'static Lib> {
        LIB.get().and_then(Option::as_ref)
    }

    /// The library in the user's folder, opened once and kept for the life
    /// of the process.
    pub fn lib(dir: &str) -> Option<&'static Lib> {
        // SAFETY: the library is opened once and never closed (`open`), so
        // the handle and every address taken from it live for the process
        LIB.get_or_init(|| unsafe {
            let dir = std::path::absolute(dir).unwrap_or_else(|_| std::path::PathBuf::from(dir));
            // Linux: this viewer's sl-ngx by its commit, else the one the
            // install left (older: what it lacks falls back, `Lib`'s
            // optional parts)
            #[cfg(target_os = "linux")]
            let path = Some(dir.join(super::versioned_shim()))
                .filter(|p| p.is_file())
                .unwrap_or_else(|| dir.join(SHIM));
            #[cfg(not(target_os = "linux"))]
            let path = dir.join(SHIM);
            let handle = match open(&path) {
                Ok(handle) => handle,
                Err(why) => {
                    log::error!(
                        "DLSS: {} did not load ({why}). Run {SETUP} to set it up \
                         (docs/DLSS.md): FSR draws",
                        path.display()
                    );
                    return None;
                }
            };
            /// # Safety
            /// `F` is the function pointer type the headers give `name`;
            /// `handle` as `symbol` asks.
            unsafe fn sym<F: Copy>(handle: *mut c_void, name: &CStr) -> Option<F> {
                // SAFETY: as the caller's contract says
                let f = unsafe { as_fn(symbol(handle, name)) };
                if f.is_none() {
                    log::error!("DLSS: {SHIM} has no {name:?}: FSR draws");
                }
                f
            }
            log::info!("DLSS: the user's {} is loaded", path.display());
            Some(Lib {
                init: sym(handle, c"slInit")?,
                shutdown: sym(handle, c"slShutdown")?,
                is_feature_supported: sym(handle, c"slIsFeatureSupported")?,
                feature_requirements: sym(handle, c"slGetFeatureRequirements")?,
                set_vulkan_info: sym(handle, c"slSetVulkanInfo")?,
                feature_function: sym(handle, c"slGetFeatureFunction")?,
                new_frame_token: sym(handle, c"slGetNewFrameToken")?,
                set_constants: sym(handle, c"slSetConstants")?,
                set_tag_for_frame: sym(handle, c"slSetTagForFrame")?,
                evaluate: sym(handle, c"slEvaluateFeature")?,
                allocate: sym(handle, c"slAllocateResources")?,
                free: sym(handle, c"slFreeResources")?,
                #[cfg(windows)]
                get_instance_proc_addr: sym(handle, c"vkGetInstanceProcAddr")?,
                // SAFETY: the types present.h gives these names
                #[cfg(target_os = "linux")]
                presenter: (|| {
                    let fns = present::Fns {
                        create: as_fn(symbol(handle, c"slngxPresenterCreate"))?,
                        present: as_fn(symbol(handle, c"slngxPresent"))?,
                        resize: as_fn(symbol(handle, c"slngxPresenterResize"))?,
                        pause: as_fn(symbol(handle, c"slngxPresenterPause"))?,
                        resume: as_fn(symbol(handle, c"slngxPresenterResume"))?,
                        destroy: as_fn(symbol(handle, c"slngxPresenterDestroy"))?,
                        get_stats: as_fn(symbol(handle, c"slngxPresenterGetStats"))?,
                    };
                    Some(fns)
                })()
                .or_else(|| {
                    log::info!(
                        "DLSS: this {SHIM} has no presenter: frame generation presents the prototype's way (rebuild it with {SETUP})"
                    );
                    None
                }),
                dir,
                handle,
            })
        })
        .as_ref()
    }

    impl Lib {
        /// The DLSS functions, asked of the library each time (its answer
        /// can change once a device is set).
        ///
        /// # Safety
        /// Called under Streamline's lock, after `slInit`.
        pub unsafe fn dlss(&self) -> Result<Dlss, String> {
            // SAFETY: the types sl_dlss.h gives these two names; the
            // caller's contract
            unsafe {
                Ok(Dlss {
                    optimal_settings: self.function(FEATURE_DLSS, c"slDLSSGetOptimalSettings")?,
                    set_options: self.function(FEATURE_DLSS, c"slDLSSSetOptions")?,
                })
            }
        }

        /// Ray Reconstruction's functions, as [`Lib::dlss`].
        ///
        /// # Safety
        /// As [`Lib::dlss`].
        pub unsafe fn dlssd(&self) -> Result<Dlssd, String> {
            // SAFETY: the types sl_dlss_d.h gives these two names; the
            // caller's contract
            unsafe {
                Ok(Dlssd {
                    optimal_settings: self.function(FEATURE_DLSS_RR, c"slDLSSDGetOptimalSettings")?,
                    set_options: self.function(FEATURE_DLSS_RR, c"slDLSSDSetOptions")?,
                })
            }
        }

        /// Windows' frame generation functions, with Reflex's and PCL's,
        /// as [`Lib::dlss`].
        ///
        /// # Safety
        /// As [`Lib::dlss`].
        #[cfg_attr(not(windows), allow(dead_code))]
        pub unsafe fn dlssg(&self) -> Result<Dlssg, String> {
            // SAFETY: the types the three headers give these names; the
            // caller's contract
            unsafe {
                Ok(Dlssg {
                    set_options: self.function(FEATURE_DLSS_G, c"slDLSSGSetOptions")?,
                    get_state: self.function(FEATURE_DLSS_G, c"slDLSSGGetState")?,
                    reflex_set_options: self.function(FEATURE_REFLEX, c"slReflexSetOptions")?,
                    reflex_sleep: self.function(FEATURE_REFLEX, c"slReflexSleep")?,
                    pcl_set_marker: self.function(FEATURE_PCL, c"slPCLSetMarker")?,
                })
            }
        }

        /// Linux: Reflex's and PCL's functions, as [`Lib::dlss`].
        ///
        /// # Safety
        /// As [`Lib::dlss`].
        #[cfg(target_os = "linux")]
        pub unsafe fn latency(&self) -> Result<Latency, String> {
            // SAFETY: the types sl_reflex.h and sl_pcl.h give these
            // names; the caller's contract
            unsafe {
                Ok(Latency {
                    reflex_set_options: self.function(FEATURE_REFLEX, c"slReflexSetOptions")?,
                    reflex_sleep: self.function(FEATURE_REFLEX, c"slReflexSleep")?,
                    pcl_set_marker: self.function(FEATURE_PCL, c"slPCLSetMarker")?,
                })
            }
        }

        /// `feature`'s function `name`, as the pointer type `F`.
        ///
        /// # Safety
        /// `F` is the type the headers give `name`; under Streamline's
        /// lock, after `slInit`.
        unsafe fn function<F: Copy>(&self, feature: u32, name: &CStr) -> Result<F, String> {
            let mut f = std::ptr::null_mut();
            // SAFETY: `name` nul-ended, `f` lives through the call
            check(unsafe { (self.feature_function)(feature, name.as_ptr(), &mut f) })
                .map_err(|e| format!("slGetFeatureFunction({name:?}): {e}"))?;
            // SAFETY: the caller's contract
            unsafe { as_fn(f) }.ok_or_else(|| format!("{name:?} came back null"))
        }
    }

    /// The library's log, into the viewer's.
    pub unsafe extern "C" fn log_message(kind: u32, msg: *const c_char) {
        if msg.is_null() {
            return;
        }
        // SAFETY: the library passes a nul-ended message
        let text = unsafe { CStr::from_ptr(msg) }.to_string_lossy();
        let text = text.trim_end();
        match kind {
            LOG_ERROR => log::error!("DLSS (Streamline): {text}"),
            LOG_WARN => log::warn!("DLSS (Streamline): {text}"),
            _ => log::info!("DLSS (Streamline): {text}"),
        }
    }
}

#[cfg(all(feature = "dlss", any(target_os = "linux", all(windows, target_arch = "x86_64"))))]
mod imp {
    use super::sl;
    use super::{Frame, Inputs};
    use crate::device_lost::DeviceLost;
    use ash::vk;
    use std::ffi::{CStr, CString};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    use wgpu::hal::api::Vulkan;

    /// Streamline application id of this local build (any fixed UUID).
    const PROJECT_ID: &CStr = c"5b0c4a8e-6f1d-4d7a-9a52-3e1f0c2d7b91";
    const MOTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
    const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
    /// Ray Reconstruction's albedo guides, and its normals with roughness
    const GUIDE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
    const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
    /// The file the user downloads starts with this (a version follows on
    /// Linux).
    #[cfg(target_os = "linux")]
    const SNIPPET: &str = "libnvidia-ngx-dlss.so";
    #[cfg(windows)]
    const SNIPPET: &str = "nvngx_dlss.dll";

    /// The last instance was made with the DLSS extensions.
    static INSTANCE: AtomicBool = AtomicBool::new(false);
    /// The last instance was made with Ray Reconstruction's instance
    /// extensions too ([`rr_instance_extensions`]): without them, its
    /// devices open for super resolution alone ([`request_device`]).
    static RR_INSTANCE: AtomicBool = AtomicBool::new(false);
    /// The last device opened with the DLSS extensions, and its adapter (Streamline
    /// takes the textures through it). A renderer made after a device loss
    /// opens another: one opened the usual way empties this.
    static OPENED: Mutex<Option<(wgpu::Adapter, wgpu::Device)>> = Mutex::new(None);
    /// Streamline, started on one device at a time, held while it is called.
    static STARTED: Mutex<Option<Started>> = Mutex::new(None);
    /// Counts Streamline's starts: a feature belongs to the start that made it.
    static STARTS: AtomicU64 = AtomicU64::new(0);
    /// DLSS was wanted and FSR draws instead: Streamline would not start, make
    /// the feature or evaluate it (the log says why). Cleared by the next
    /// feature made, and when a device opens (its own tries are what
    /// counts). What Preferences shows ([`support`]).
    static FAILED: AtomicBool = AtomicBool::new(false);
    /// [`FAILED`] stands for DLSS's refusal to start ([`refusal_shown`]),
    /// the device's, not a view's: then any view that makes a feature or
    /// draws clears it (with VR drawing, only the eyes do, and they do not
    /// report their own failures), not only the main view. Over when Streamline
    /// starts, or when the main view reports a failure of its own
    /// ([`failed`]): a view's failure is cleared only by that view.
    static REFUSAL: AtomicBool = AtomicBool::new(false);
    /// The device open now was opened without Streamline: Streamline's lock was still
    /// held on the device before it, by a thread in a Streamline call that did
    /// not return after a device loss ([`let_go`]), or a render thread
    /// was left behind ([`LEFT_BEHIND`]). Nothing of Streamline is touched until
    /// another device opens.
    static BUSY: AtomicBool = AtomicBool::new(false);
    /// A render thread was left behind, still in a GPU or OpenXR call,
    /// when its renderer closed ([`left_behind`]): its backend, and the
    /// DLSS in it, are not dropped while it is out, and it may still record and submit
    /// an evaluate when the call returns. Streamline's start on a device that
    /// works is left as it is, never ended ([`let_go`]): ending it releases
    /// the features that thread may still use (`end` waits only for the
    /// queue, which can be idle while the thread is blocked elsewhere).
    /// Cleared when that thread returns after all and its renderer closes
    /// ([`closing`], which ends the start then).
    /// Every device opened beside that start opens without Streamline ([`BUSY`]),
    /// and no new instance asks Streamline for its extensions ([`idle`]). Nothing
    /// started, or a start on a lost device (`with_started` refuses that
    /// thread's calls there, and it is abandoned as after any loss), keeps
    /// nothing from the next device, and the flag is cleared then
    /// ([`let_go`]): Streamline starts there as usual, never on
    /// the device of the thread left behind (`with_started` starts Streamline only on
    /// the device opened now).
    static LEFT_BEHIND: AtomicBool = AtomicBool::new(false);
    /// The probe of the window's instance ([`idle`] in [`instance`]) found
    /// Streamline's lock held: a device opened from that instance shows [`BUSY`]
    /// too, even when the stuck call returned before the device opened
    /// (that instance has no DLSS extensions, so DLSS would read as the
    /// driver's refusal).
    static PROBE_BUSY: AtomicBool = AtomicBool::new(false);
    /// The same for the VR headset's instance ([`xr_instance_extensions`]).
    /// Apart from the window's: the two instances live side by side (the
    /// window's stays while VR runs), so a headset probe that found Streamline
    /// busy must not mark the window's next device, made from an instance
    /// that has the DLSS extensions, and the reverse.
    static XR_PROBE_BUSY: AtomicBool = AtomicBool::new(false);
    /// Counts the devices opened (by [`request_device`], or for a headset,
    /// [`opened_for_xr`]): what a refused start is kept for.
    static OPENS: AtomicU64 = AtomicU64::new(0);
    /// The [`OPENS`] at which Streamline refused to start (0: none). The same
    /// device refuses again, so asking at every target rebuild would only
    /// cost a start and an error in the log each time; a new device, or
    /// [`retry`], asks again.
    static REFUSED: AtomicU64 = AtomicU64::new(0);
    /// The last device was opened for a VR headset (OpenXR picks its
    /// extensions, not Streamline): DLSS is not offered until another opens.
    static XR: AtomicBool = AtomicBool::new(false);
    /// Counts the user's asks to try DLSS again (Preferences' button):
    /// each renderer compares it with the one it last saw
    /// ([`retry_asked`]).
    static RETRIES: AtomicU64 = AtomicU64::new(0);

    /// Streamline refused to start on the device open now: [`FAILED`] for
    /// Preferences whichever view met it ([`REFUSAL`]).
    fn refusal_shown() {
        REFUSAL.store(true, Ordering::Relaxed);
        FAILED.store(true, Ordering::Relaxed);
    }

    /// A feature made or a frame drawn by a view (`report`: the main
    /// view's): what Preferences shows is no failure, unless the failure
    /// shown is another view's own ([`REFUSAL`]).
    fn working(report: bool) {
        if REFUSAL.swap(false, Ordering::Relaxed) || report {
            FAILED.store(false, Ordering::Relaxed);
        }
    }

    /// DLSS failed in a view, after Streamline started (no feature, an evaluate
    /// that failed, the video memory): logged, and with `report` (the
    /// main view) [`FAILED`] for Preferences, a failure of its own, not
    /// Streamline's refusal any more ([`REFUSAL`]). Without `report` (an external
    /// renderer, the second camera's or an eye of VR, or a snapshot's
    /// targets) only the log: that renderer draws FSR alone, and what
    /// Preferences shows stays the main view's.
    fn failed(e: &str, report: bool) {
        log::error!("DLSS: {e}: FSR draws");
        if report {
            REFUSAL.store(false, Ordering::Relaxed);
            FAILED.store(true, Ordering::Relaxed);
        }
    }

    /// The video memory refused DLSS's resources `when` (renderer.rs: a
    /// pipeline, buffer or bind group of [`Dlss::new`] or of the post
    /// pass): FSR draws, and with `report` (not an external renderer)
    /// Preferences says DLSS failed, as for any failure of the view.
    pub(crate) fn refused_memory(when: &str, report: bool) {
        failed(&format!("refused by the video memory {when}"), report);
    }

    /// Ray Reconstruction could not be made, or the video memory refused
    /// its guides: [`rr_ready`] turns false, the renderer builds its
    /// pipelines again without the guides, and super resolution draws (at
    /// once when only the guides were refused, `Dlss::new`; FSR until the
    /// rebuild when Streamline refused the feature). Kept until the next
    /// device ([`opening_device`]), Preferences' retry ([`retry`]) or
    /// another model ([`rr_retry`]), so a resize does not try it again.
    /// Without `report` (an external renderer, which never draws Ray
    /// Reconstruction: it traces nothing, `rt_shadows::traced`) only the
    /// log, and the main view's [`rr_ready`] stays as it was.
    fn rr_refused(e: &str, report: bool) {
        if report {
            RR_REFUSED.store(true, Ordering::Relaxed);
        }
        log::error!("DLSS: Ray Reconstruction: {e}: super resolution draws instead");
    }

    struct Started {
        device: wgpu::Device,
        /// which start of DLSS this is ([`STARTS`])
        start: u64,
        /// the viewports made on it and not yet freed, one per feature,
        /// each with its feature (super resolution's or Ray
        /// Reconstruction's)
        viewports: Vec<(u32, u32)>,
        /// the loss of the device, as its renderer sees it
        /// (device_lost.rs): set by the device-lost callback itself,
        /// without the lock, so a renderer stuck in a Streamline call or
        /// left behind still has it; nothing more runs on a lost device
        lost: DeviceLost,
        /// Linux: sl-ngx's presenter running on this start ([`Presenter`]),
        /// and how many pauses hold it ([`pause_presenter`]). slShutdown,
        /// and slSetVulkanInfo with another device, end it under the lock.
        #[cfg(target_os = "linux")]
        presenter: Option<usize>,
        #[cfg(target_os = "linux")]
        presenter_pauses: u32,
    }

    /// The lock, also after a panic under it (the state is plain values).
    fn started() -> std::sync::MutexGuard<'static, Option<Started>> {
        STARTED.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn opened() -> std::sync::MutexGuard<'static, Option<(wgpu::Adapter, wgpu::Device)>> {
        OPENED.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `AURORA_DLSS` for a test: `0` or `off` keeps DLSS and Streamline out
    /// entirely, anything else turns DLSS on whatever the setting; unset or
    /// empty, None.
    fn forced() -> Option<bool> {
        static ON: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
        *ON.get_or_init(|| {
            let v = std::env::var("AURORA_DLSS").ok()?;
            let v = v.trim();
            (!v.is_empty()).then_some(!matches!(v, "0" | "off"))
        })
    }

    /// DLSS draws the FSR frames: the setting (`RenderSettings::dlss`),
    /// unless `AURORA_DLSS` says otherwise.
    pub(crate) fn wanted(setting: bool) -> bool {
        forced().unwrap_or(setting)
    }

    /// DLSS frame generation is asked for: the setting
    /// (`RenderSettings::dlss_fg`), unless `AURORA_DLSS_FG` says otherwise.
    /// Linux's sl-ngx (a prototype) and Windows' Streamline.
    pub(crate) fn fg_wanted(setting: bool) -> bool {
        cfg!(any(target_os = "linux", windows)) && fg_forced().unwrap_or(setting)
    }

    /// Windows: frame generation is wanted at the next `slInit` (the
    /// setting as the renderer is made, [`set_fg_at_start`]). Streamline
    /// loads its plugin then or not at all, and while it is loaded every
    /// window's swapchain goes through its proxy, which costs a copy and a
    /// queue even with frame generation off (DLSS-G guide §18).
    #[cfg(windows)]
    static FG_AT_START: AtomicBool = AtomicBool::new(false);

    /// Windows: the `slInit` in effect loaded frame generation
    /// ([`FG_AT_START`] then).
    #[cfg(windows)]
    static FG_LOADED: AtomicBool = AtomicBool::new(false);

    /// The frame generation setting as the renderer is made: on Windows,
    /// whether Streamline loads frame generation at its start.
    pub(crate) fn set_fg_at_start(setting: bool) {
        #[cfg(windows)]
        FG_AT_START.store(fg_wanted(setting), Ordering::Relaxed);
        #[cfg(not(windows))]
        let _ = setting;
    }

    /// The setting `setting` asks for what only a restart gives: on
    /// Windows, frame generation loaded or let go (Streamline loads it at
    /// its start).
    pub(crate) fn fg_restart(setting: bool) -> bool {
        #[cfg(windows)]
        {
            INITED.load(Ordering::Relaxed) && fg_wanted(setting) != FG_LOADED.load(Ordering::Relaxed)
        }
        #[cfg(not(windows))]
        {
            let _ = setting;
            false
        }
    }

    /// Windows: frames generated per frame drawn, `AURORA_DLSS_FG_FRAMES`
    /// for a test (1 for 2x, 3 for 4x; the GPU's most at most), else 1.
    #[cfg(windows)]
    fn fg_frames() -> u32 {
        static N: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        *N.get_or_init(|| {
            std::env::var("AURORA_DLSS_FG_FRAMES")
                .ok()
                .and_then(|v| v.trim().parse().ok())
                .filter(|n| *n >= 1)
                .unwrap_or(1)
        })
    }

    /// `AURORA_DLSS_FG` for a test: `1` turns frame generation on whatever
    /// the setting, `0` or `off` keeps it out; unset or empty, None.
    fn fg_forced() -> Option<bool> {
        static ON: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
        *ON.get_or_init(|| {
            let v = std::env::var("AURORA_DLSS_FG").ok()?;
            let v = v.trim();
            (!v.is_empty()).then_some(!matches!(v, "0" | "off"))
        })
    }

    /// Streamline, started on this device, said frame generation is
    /// supported there (`slIsFeatureSupported`; an sl-ngx without it says
    /// no). Asked at each start, forgotten with the device.
    static FG_SUPPORTED: AtomicBool = AtomicBool::new(false);

    /// DLSS frame generation can draw on the device DLSS started on: what
    /// Preferences shows, and what the renderer waits for.
    pub(crate) fn fg_ready() -> bool {
        FG_SUPPORTED.load(Ordering::Relaxed)
    }

    /// `AURORA_DLSS_RR` for a test: `1` turns Ray Reconstruction on
    /// whatever the setting, `0` or `off` keeps it out, the device's
    /// extensions too; unset or empty, None.
    fn rr_forced() -> Option<bool> {
        static ON: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
        *ON.get_or_init(|| {
            let v = std::env::var("AURORA_DLSS_RR").ok()?;
            let v = v.trim();
            (!v.is_empty()).then_some(!matches!(v, "0" | "off"))
        })
    }

    /// The device was opened with Ray Reconstruction's extensions too.
    static RR_DEVICE: AtomicBool = AtomicBool::new(false);

    /// Linux: the device was opened with a second queue in family 0, for
    /// sl-ngx's presenter of frame generation ([`PRESENTER_QUEUE`]), with
    /// what was enabled for it (`SlngxPresenterDeviceBits`) below that bit;
    /// 0 when it opened without the queue. Set once the device is open.
    #[cfg(target_os = "linux")]
    static PRESENTER_DEVICE: AtomicU32 = AtomicU32::new(0);
    /// [`PRESENTER_DEVICE`]: the queue is there.
    #[cfg(target_os = "linux")]
    const PRESENTER_QUEUE: u32 = 1 << 31;
    /// `SLNGX_DEVICE_LOW_LATENCY2` (sl_ngx/present.h): VK_NV_low_latency2
    /// and VK_KHR_present_id, with `presentId`, so that Reflex works on the
    /// presenter's swapchain.
    #[cfg(target_os = "linux")]
    const PRESENTER_LOW_LATENCY2: u32 = 1 << 0;

    /// Linux: what the device opened with for sl-ngx's presenter: None
    /// without its queue, else the `SlngxPresenterDeviceBits` enabled.
    #[cfg(target_os = "linux")]
    pub(crate) fn presenter_device() -> Option<u32> {
        let bits = PRESENTER_DEVICE.load(Ordering::Relaxed);
        (bits & PRESENTER_QUEUE != 0 && opened().is_some()).then_some(bits & !PRESENTER_QUEUE)
    }
    /// Streamline, started on this device, said whether Ray Reconstruction
    /// is supported there ([`RR_UNAVAILABLE`]): until then the renderer
    /// draws no guides ([`rr_ready_on`], [`rr_probe`]).
    static RR_KNOWN: AtomicBool = AtomicBool::new(false);
    /// Streamline's start on this device said Ray Reconstruction is not
    /// supported (`slIsFeatureSupported`): asked again only by the next
    /// start, and a retry cannot change it.
    static RR_UNAVAILABLE: AtomicBool = AtomicBool::new(false);
    /// Ray Reconstruction refused on this device after Streamline offered
    /// it: its feature, its evaluate or the video memory for its textures
    /// ([`rr_refused`]). Kept through Streamline's starts on the device (a
    /// start says only [`RR_UNAVAILABLE`]), until the next device, a retry
    /// or another model.
    static RR_REFUSED: AtomicBool = AtomicBool::new(false);
    /// Ray Reconstruction refused for a photo or film frame, whatever
    /// refused it (its render targets, its guides' textures or objects,
    /// Streamline's feature, its evaluate): from there every capture, of
    /// any size, draws super resolution, until the next device, a retry or
    /// another model; the window's Ray Reconstruction is left as it is
    /// ([`snapshot_rr_refused`]).
    static SNAPSHOT_RR_REFUSED: AtomicBool = AtomicBool::new(false);

    /// The device was opened with Ray Reconstruction's extensions: what
    /// the renderer draws its guides by (`RenderSettings::rr_guides`), set
    /// once the device is open and fixed with it.
    pub(crate) fn rr_device() -> bool {
        RR_DEVICE.load(Ordering::Relaxed) && opened().is_some()
    }

    /// Ray Reconstruction can draw: [`rr_device`], and Streamline did not
    /// refuse it, or has not been asked yet (it is asked when DLSS
    /// starts). What Preferences shows; a renderer asks [`rr_ready_on`] of
    /// its own device.
    pub(crate) fn rr_ready() -> bool {
        rr_device() && !RR_UNAVAILABLE.load(Ordering::Relaxed) && !RR_REFUSED.load(Ordering::Relaxed)
    }

    /// [`rr_device`], and Ray Reconstruction refused after Streamline
    /// offered it ([`RR_REFUSED`]): what [`retry`] may change, unlike
    /// Streamline saying it is not supported ([`RR_UNAVAILABLE`]).
    pub(crate) fn rr_refused_here() -> bool {
        rr_device() && !RR_UNAVAILABLE.load(Ordering::Relaxed) && RR_REFUSED.load(Ordering::Relaxed)
    }

    /// [`rr_ready`] for a renderer on `device`: false when `device` is not
    /// the one opened with Ray Reconstruction's extensions (a renderer still
    /// on a lost device, or on another adapter's), so that one never builds
    /// the guides for a device without them.
    /// Only once Streamline said Ray Reconstruction is supported there
    /// ([`RR_KNOWN`]): guides compiled before that answer were built again
    /// at once wherever it says no.
    pub(crate) fn rr_ready_on(device: &wgpu::Device) -> bool {
        RR_KNOWN.load(Ordering::Relaxed) && rr_ready() && opened().as_ref().is_some_and(|(_, d)| same(d, device))
    }

    /// Starts DLSS on `device` when it opened with Ray Reconstruction's
    /// extensions and whether Streamline supports it there is not known yet
    /// ([`RR_KNOWN`]): the renderer asks before it compiles the guides'
    /// pipelines (renderer.rs, `rr_wanted`), so that a device without Ray
    /// Reconstruction never compiles them. Nothing on a lost device, one
    /// where Streamline refused to start, or another device than the one
    /// opened (a start refused here is shown as [`with_started`] shows it).
    pub(crate) fn rr_probe(device: &wgpu::Device, lost: &DeviceLost) {
        if RR_KNOWN.load(Ordering::Relaxed)
            || !RR_DEVICE.load(Ordering::Relaxed)
            || lost.get()
            || super::refused(REFUSED.load(Ordering::Relaxed), OPENS.load(Ordering::Relaxed))
            || !opened().as_ref().is_some_and(|(_, d)| same(d, device))
        {
            return;
        }
        if let Some(lib) = sl::loaded() {
            let _ = with_started(lib, device, lost, true, |_| ());
        }
    }

    /// Ray Reconstruction was refused for a photo or film frame on the
    /// device open now: every capture's pipelines, targets and DLSS take
    /// super resolution ([`SNAPSHOT_RR_REFUSED`]).
    pub(crate) fn snapshot_rr_refused() -> bool {
        SNAPSHOT_RR_REFUSED.load(Ordering::Relaxed)
    }

    /// [`rr_device`], and [`snapshot_rr_refused`]: what Preferences offers
    /// to [`retry`], as for the window's refusal ([`rr_refused_here`]).
    pub(crate) fn snapshot_rr_refused_here() -> bool {
        rr_device() && SNAPSHOT_RR_REFUSED.load(Ordering::Relaxed)
    }

    /// The video memory refused the render targets only Ray
    /// Reconstruction reads (renderer.rs, `make_targets`): refused as
    /// Streamline refusing it would be ([`rr_refused`]), so the targets are
    /// made again without them and super resolution draws.
    pub(crate) fn rr_targets_refused(report: bool) {
        rr_refused("the video memory refused its render targets", report);
    }

    /// The same for a photo or film frame's targets: remembered for the
    /// captures alone ([`SNAPSHOT_RR_REFUSED`]), the window's Ray
    /// Reconstruction left as it was.
    pub(crate) fn snapshot_rr_targets_refused() {
        snapshot_refusal("the video memory refused its render targets");
    }

    /// Ray Reconstruction draws with DLSS: the setting
    /// (`RenderSettings::dlss_rr`), unless `AURORA_DLSS_RR` says otherwise.
    pub(crate) fn rr_wanted(setting: bool) -> bool {
        rr_forced().unwrap_or(setting)
    }

    /// What Preferences shows: the device opened with the DLSS extensions, or
    /// why not.
    pub(crate) fn support() -> crate::DlssSupport {
        support_of(opened().is_some())
    }

    /// [`support`] for `device`, the one just opened: Ready only when it is
    /// the device opened with the DLSS extensions, not merely when one was
    /// (a VR headset's device opened as before has none).
    pub(crate) fn support_on(device: &wgpu::Device) -> crate::DlssSupport {
        support_of(opened().as_ref().is_some_and(|(_, d)| same(d, device)))
    }

    fn support_of(ours: bool) -> crate::DlssSupport {
        use crate::DlssSupport as S;
        // (VR's reason only with the files there: without, setting them up
        // is what Preferences says)
        let files = || folder().is_some_and(|dir| std::path::Path::new(dir).join(sl::SHIM).is_file());
        // (BUSY first: a VR headset's device opened without Streamline for that
        // reason is not the headset's doing)
        if BUSY.load(Ordering::Relaxed) && forced() != Some(false) && files() {
            S::Busy
        } else if XR.load(Ordering::Relaxed) && forced() != Some(false) && files() {
            S::Vr
        } else if ours {
            if FAILED.load(Ordering::Relaxed) { S::Failed } else { S::Ready }
        } else if forced() == Some(false) {
            S::Off
        } else if files() {
            S::NoDevice
        } else {
            S::NoFiles
        }
    }

    /// The folder the viewer looks in for the user's two files.
    /// `AURORA_DLSS_DIR`, else `$XDG_DATA_HOME/aurora-dlss` (Linux) or
    /// `%LOCALAPPDATA%\aurora-dlss` (Windows).
    pub(crate) fn folder_named() -> Option<String> {
        std::env::var("AURORA_DLSS_DIR").ok().filter(|d| !d.is_empty()).or_else(|| {
            #[cfg(target_os = "linux")]
            let data = std::env::var("XDG_DATA_HOME")
                .ok()
                .filter(|d| !d.is_empty())
                .or_else(|| Some(format!("{}/.local/share", std::env::var("HOME").ok()?)))?;
            #[cfg(windows)]
            let data = std::env::var("LOCALAPPDATA").ok().filter(|d| !d.is_empty())?;
            Some(std::path::Path::new(&data).join("aurora-dlss").to_string_lossy().into_owned())
        })
    }

    /// The jitter's sign given to DLSS: where the image moved, as FSR takes
    /// it (the opposite drew a light rim on every silhouette).
    /// `AURORA_DLSS_JITTER=x|y|xy` flips the axes named, to debug it (a
    /// still scene must resolve sharp).
    fn sign() -> [f32; 2] {
        static SIGN: std::sync::OnceLock<[f32; 2]> = std::sync::OnceLock::new();
        *SIGN.get_or_init(|| {
            let flip = std::env::var("AURORA_DLSS_JITTER").unwrap_or_default();
            [
                if flip.contains('x') { -1.0 } else { 1.0 },
                if flip.contains('y') { -1.0 } else { 1.0 },
            ]
        })
    }

    /// The folder with the DLSS library the user downloaded, when the file
    /// is there.
    /// Linux: sl-ngx brought up to this viewer's commit at start
    /// ([`super::SL_NGX_COMMIT`]): when the DLSS folder has no
    /// `libsl_ngx.<commit>.so`, the setup script beside the viewer runs
    /// with `--update`, off the main thread. It asks nothing, and installs
    /// nothing when NVIDIA's licence changed since the install; the
    /// library it builds is loaded from the next start, the one there
    /// until then. Its output is `update.log` in the folder.
    #[cfg(target_os = "linux")]
    pub(crate) mod update {
        use std::sync::Mutex;

        /// What the update did, for Preferences.
        #[derive(Clone, Debug, PartialEq)]
        pub(crate) enum State {
            /// not looked at (no DLSS folder)
            Unknown,
            /// the folder has this viewer's sl-ngx
            Current,
            Running,
            /// built: loaded from the next start (the script's last line)
            Done(String),
            /// NVIDIA's licence changed: the user runs the script
            Licence,
            /// the script's last line, or why it did not run
            Failed(String),
        }

        static STATE: Mutex<State> = Mutex::new(State::Unknown);

        pub(crate) fn state() -> State {
            STATE.lock().map(|s| s.clone()).unwrap_or(State::Unknown)
        }

        fn set(state: State) {
            if let Ok(mut s) = STATE.lock() {
                *s = state;
            }
        }

        /// The setup script: beside the viewer, as a release ships it or
        /// as a package names it, or in the source checkout of a build.
        fn script() -> Option<std::path::PathBuf> {
            let exe = std::env::current_exe().ok()?;
            let here = exe.parent()?;
            [
                here.join(crate::DLSS_SETUP),
                here.join("aurora-viewer-dlss-setup"),
                here.join("../../scripts").join(crate::DLSS_SETUP),
            ]
            .into_iter()
            .find(|p| p.is_file())
        }

        pub(super) fn begin(dir: &str) {
            let dir = std::path::PathBuf::from(dir);
            if dir.join(super::super::versioned_shim()).is_file() {
                set(State::Current);
                return;
            }
            let Some(script) = script() else {
                log::warn!(
                    "DLSS: {} has no sl-ngx {} and {} is not beside the viewer: run it with --update",
                    dir.display(),
                    &super::super::SL_NGX_COMMIT[..12],
                    crate::DLSS_SETUP
                );
                set(State::Failed(format!("{} not found beside the viewer", crate::DLSS_SETUP)));
                return;
            };
            set(State::Running);
            let spawned = std::thread::Builder::new().name("dlss-update".into()).spawn(move || {
                log::info!(
                    "DLSS: building sl-ngx {} for this viewer ({} --update; its output: {})",
                    &super::super::SL_NGX_COMMIT[..12],
                    script.display(),
                    dir.join("update.log").display()
                );
                let log_path = dir.join("update.log");
                let run = std::fs::File::create(&log_path).and_then(|out| {
                    let err = out.try_clone()?;
                    std::process::Command::new("bash")
                        .arg(&script)
                        .arg("--update")
                        .stdin(std::process::Stdio::null())
                        .stdout(out)
                        .stderr(err)
                        .status()
                });
                let last = std::fs::read_to_string(&log_path)
                    .ok()
                    .and_then(|text| text.lines().rev().find(|l| !l.trim().is_empty()).map(str::to_owned))
                    .unwrap_or_default();
                let state = match run {
                    Ok(status) if status.success() => State::Done(last),
                    Ok(status) if status.code() == Some(3) => State::Licence,
                    Ok(_) => State::Failed(last),
                    Err(e) => State::Failed(format!("bash {}: {e}", script.display())),
                };
                match &state {
                    State::Done(line) => log::info!("DLSS: {line}"),
                    State::Licence => {
                        log::warn!(
                            "DLSS: NVIDIA's licence changed since the install: sl-ngx not updated; run {} to read it",
                            crate::DLSS_SETUP
                        )
                    }
                    other => log::error!("DLSS: sl-ngx was not updated: {other:?}"),
                }
                set(state);
            });
            if let Err(e) = spawned {
                set(State::Failed(format!("no thread: {e}")));
            }
        }
    }

    fn folder() -> Option<&'static str> {
        static DIR: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        DIR.get_or_init(|| {
            let dir = folder_named()?;
            // Linux: the name carries a version after the prefix
            #[cfg(target_os = "linux")]
            let found = std::fs::read_dir(&dir).ok().and_then(|entries| {
                entries
                    .flatten()
                    .map(|e| e.path())
                    .find(|p| p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(SNIPPET)))
            });
            // Windows: the one name, in any case (the file system ignores it)
            #[cfg(windows)]
            let found = Some(std::path::Path::new(&dir).join(SNIPPET)).filter(|p| p.is_file());
            match found {
                Some(file) => {
                    log::info!("DLSS: the user's library is {}", file.display());
                    #[cfg(target_os = "linux")]
                    update::begin(&dir);
                    Some(dir)
                }
                None => {
                    // the usual case: DLSS was never set up (docs/DLSS.md)
                    log::info!("DLSS: no {SNIPPET}* in {dir} ({} puts it there): not offered", sl::SETUP);
                    None
                }
            }
        })
        .as_deref()
    }

    /// `slInit` was called and `slShutdown` not since; read and changed
    /// under the lock ([`STARTED`]).
    static INITED: AtomicBool = AtomicBool::new(false);

    /// The last start was left on a lost device ([`abandon`]) and the
    /// library still holds it (sl-ngx lets it go at the next device it is
    /// told of): until then nothing may shut the library down, which would
    /// call Streamline on that device. Under the lock.
    #[cfg(target_os = "linux")]
    static ABANDONED: AtomicBool = AtomicBool::new(false);

    /// Windows: a start was left on a lost device ([`abandon`]), which
    /// Streamline keeps until `slShutdown`, and a shutdown would call into
    /// it: DLSS stays off until the viewer restarts ([`fresh`]).
    #[cfg(windows)]
    static GIVEN_UP: AtomicBool = AtomicBool::new(false);

    /// The viewport of the next feature: each feature its own, never
    /// reused in the process (a retired one is freed later, beside the
    /// next).
    static VIEWPORTS: AtomicU32 = AtomicU32::new(1);

    /// `slInit`, unless it is in effect already. Under the lock.
    fn ensure_init(lib: &sl::Lib) -> Result<(), String> {
        if INITED.load(Ordering::Relaxed) {
            return Ok(());
        }
        // the viewer's own number, from its Git tag (AGENTS.md §13: never
        // the crate's)
        let version = CString::new(aurora_version::version()).unwrap_or_default();
        let dir = sl::wide(&lib.dir.to_string_lossy());
        let dirs = [dir.as_ptr()];
        let data = sl::wide(&std::env::temp_dir().to_string_lossy());
        // Ray Reconstruction's plugin too: Streamline loads what it finds
        // (on Windows `sl.dlss_d.dll`, which the user may not have copied),
        // and says per feature whether it runs ([`start`])
        // Windows: frame generation's too, with the Reflex and PCL it
        // needs, only when it is wanted at the start ([`FG_AT_START`])
        let all = [
            sl::FEATURE_DLSS,
            sl::FEATURE_DLSS_RR,
            sl::FEATURE_DLSS_G,
            sl::FEATURE_REFLEX,
            sl::FEATURE_PCL,
        ];
        #[cfg(windows)]
        let fg = {
            let fg = FG_AT_START.load(Ordering::Relaxed);
            FG_LOADED.store(fg, Ordering::Relaxed);
            fg
        };
        #[cfg(not(windows))]
        let fg = false;
        let features = &all[..if fg { all.len() } else { 2 }];
        let mut p = sl::Preferences::new();
        p.paths_to_plugins = dirs.as_ptr();
        p.num_paths_to_plugins = 1;
        p.path_to_logs_and_data = data.as_ptr();
        p.log_message_callback = Some(sl::log_message);
        // no over-the-air updates, and Streamline's command list state
        // tracking off (the viewer restores nothing it would track)
        p.flags = sl::DISABLE_CL_STATE_TRACKING | sl::DISABLE_DEBUG_TEXT | sl::USE_FRAME_BASED_RESOURCE_TAGGING;
        // Linux: the viewer makes the instance and device itself and
        // tells the library which (`slSetVulkanInfo`)
        #[cfg(target_os = "linux")]
        {
            p.flags |= sl::USE_MANUAL_HOOKING;
        }
        p.features_to_load = features.as_ptr();
        p.num_features_to_load = features.len() as u32;
        p.engine_version = version.as_ptr();
        p.project_id = PROJECT_ID.as_ptr();
        // SAFETY: as sl.h declares it; every pointer in `p` lives through
        // the call, and the library copies what it keeps
        sl::check(unsafe { (lib.init)(&p, sl::SDK_VERSION) }).map_err(|e| format!("slInit: {e}"))?;
        INITED.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// `slShutdown`, when `slInit` is in effect: the next call that needs
    /// the library initialises it again ([`ensure_init`]). Under the lock.
    fn shut_down(lib: &sl::Lib) {
        if INITED.swap(false, Ordering::Relaxed) {
            // SAFETY: as sl.h declares it, under the lock
            if let Err(e) = sl::check(unsafe { (lib.shutdown)() }) {
                log::warn!("DLSS: slShutdown: {e}");
            }
        }
    }

    /// `name` for the life of the process, as wgpu-hal keeps the extension
    /// names it is given: each name kept once.
    fn interned(name: CString) -> &'static CStr {
        static NAMES: Mutex<Vec<&'static CStr>> = Mutex::new(Vec::new());
        let mut names = NAMES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(kept) = names.iter().find(|n| **n == name.as_c_str()) {
            return kept;
        }
        let kept: &'static CStr = Box::leak(name.into_boxed_c_str());
        names.push(kept);
        kept
    }

    /// The Vulkan extensions the library asks for `feature`: the
    /// instance's, then the device's (those once `slSetVulkanInfo` named
    /// an instance and physical device). Under the lock, after
    /// [`ensure_init`].
    fn requirements(lib: &sl::Lib, feature: u32) -> Result<(Vec<&'static CStr>, Vec<&'static CStr>), String> {
        let mut req = sl::FeatureRequirements::new();
        // SAFETY: as sl.h declares it; `req` lives through the call
        sl::check(unsafe { (lib.feature_requirements)(feature, &mut req) }).map_err(|e| format!("slGetFeatureRequirements: {e}"))?;
        // SAFETY: the lists the call above filled, read before any other
        // call
        let (instance, device) = unsafe {
            (
                sl::FeatureRequirements::names(req.vk_instance_extensions, req.vk_num_instance_extensions),
                sl::FeatureRequirements::names(req.vk_device_extensions, req.vk_num_device_extensions),
            )
        };
        Ok((
            instance.into_iter().map(interned).collect(),
            device.into_iter().map(interned).collect(),
        ))
    }

    /// The library's instance extensions for DLSS, each checked to be one
    /// the Vulkan loader of `entry` has. Under the lock (`held`, whose
    /// state says whether a start is running).
    fn instance_extensions(lib: &sl::Lib, held: &Option<Started>, entry: &ash::Entry) -> Result<Vec<&'static CStr>, String> {
        // With nothing started, the library is initialised afresh: the
        // instance and physical device it was last told of may be gone.
        // Not after a start left on a lost device, which a shutdown would
        // call Streamline on ([`ABANDONED`]).
        #[cfg(target_os = "linux")]
        if held.is_none() && !ABANDONED.load(Ordering::Relaxed) {
            shut_down(lib);
        }
        // Windows: only for an instance made through the interposer, the
        // library made afresh by its caller ([`fresh`])
        #[cfg(windows)]
        {
            let _ = held;
            if entry.static_fn().get_instance_proc_addr as usize != lib.get_instance_proc_addr as usize {
                return Err(String::from("the instance is not made through sl.interposer.dll"));
            }
        }
        ensure_init(lib)?;
        let (wanted, _) = requirements(lib, sl::FEATURE_DLSS)?;
        // SAFETY: a query of the loader, no layer named
        let have = unsafe { entry.enumerate_instance_extension_properties(None) }.map_err(|e| e.to_string())?;
        for name in &wanted {
            if !have.iter().any(|e| e.extension_name_as_c_str() == Ok(*name)) {
                return Err(format!("no instance extension {name:?}"));
            }
        }
        Ok(wanted)
    }

    /// The library's device extensions for DLSS on `adapter`, each checked
    /// to be one its device supports. Under the lock.
    fn device_extensions(lib: &sl::Lib, adapter: &wgpu::hal::vulkan::Adapter) -> Result<Vec<&'static CStr>, String> {
        ensure_init(lib)?;
        // Linux: the adapter named first, without a device, so that the
        // library asks Streamline about this one (on Windows the interposer sees
        // the instance and device being made)
        #[cfg(target_os = "linux")]
        {
            let info = sl::VulkanInfo::new(
                adapter.shared_instance().raw_instance().handle(),
                adapter.raw_physical_device(),
                vk::Device::null(),
            );
            // SAFETY: as sl_helpers_vk.h declares it, with the adapter's
            // live instance and physical device
            sl::check(unsafe { (lib.set_vulkan_info)(&info) }).map_err(|e| format!("slSetVulkanInfo: {e}"))?;
        }
        let (_, wanted) = requirements(lib, sl::FEATURE_DLSS)?;
        for name in &wanted {
            if !adapter.physical_device_capabilities().supports_extension(name) {
                return Err(format!("no device extension {name:?}"));
            }
        }
        Ok(wanted)
    }

    /// Ray Reconstruction's instance extensions beyond `wanted` (super
    /// resolution's), when the library names them and the Vulkan loader of
    /// `entry` has them all; None, with the reason in the log, and the
    /// instance is made for super resolution alone. After
    /// [`instance_extensions`], under the lock.
    fn rr_instance_extensions(lib: &sl::Lib, entry: &ash::Entry, wanted: &[&'static CStr]) -> Option<Vec<&'static CStr>> {
        let (all, _) = requirements(lib, sl::FEATURE_DLSS_RR)
            .map_err(|e| log::info!("DLSS: no Ray Reconstruction on this instance ({e})"))
            .ok()?;
        let extra: Vec<_> = all.into_iter().filter(|n| !wanted.contains(n)).collect();
        if extra.is_empty() {
            return Some(extra);
        }
        // SAFETY: a query of the loader, no layer named
        let have = match unsafe { entry.enumerate_instance_extension_properties(None) } {
            Ok(have) => have,
            Err(e) => {
                log::info!("DLSS: no Ray Reconstruction on this instance (the loader's extensions: {e})");
                return None;
            }
        };
        let missing: Vec<_> = extra
            .iter()
            .filter(|n| !have.iter().any(|e| e.extension_name_as_c_str() == Ok(**n)))
            .collect();
        if !missing.is_empty() {
            log::info!("DLSS: no Ray Reconstruction on this instance (missing {missing:?})");
            return None;
        }
        log::info!("DLSS: Ray Reconstruction's instance extensions beyond super resolution's: {extra:?}");
        Some(extra)
    }

    /// Ray Reconstruction's device extensions beyond `wanted` (super
    /// resolution's) on `adapter`, when the library names them and the
    /// device supports them all; None, with the reason in the log, and the
    /// device opens for super resolution alone. After
    /// [`device_extensions`] (which named the adapter), under the lock.
    fn rr_device_extensions(lib: &sl::Lib, adapter: &wgpu::hal::vulkan::Adapter, wanted: &[&'static CStr]) -> Option<Vec<&'static CStr>> {
        let (_, all) = requirements(lib, sl::FEATURE_DLSS_RR)
            .map_err(|e| log::info!("DLSS: no Ray Reconstruction on this device ({e})"))
            .ok()?;
        let extra: Vec<_> = all.into_iter().filter(|n| !wanted.contains(n)).collect();
        let caps = adapter.physical_device_capabilities();
        let missing: Vec<_> = extra.iter().filter(|n| !caps.supports_extension(n)).collect();
        if !missing.is_empty() {
            log::info!("DLSS: no Ray Reconstruction on this device (missing {missing:?})");
            return None;
        }
        log::info!("DLSS: Ray Reconstruction's device extensions beyond super resolution's: {extra:?}");
        Some(extra)
    }

    /// Linux: what the device opens with for sl-ngx's presenter of frame
    /// generation (sl_ngx/present.h), beyond `wanted`: a second queue in
    /// family 0, the one wgpu takes its queue from, and with it, where
    /// the library names them and the device has them with `presentId`,
    /// VK_KHR_present_id and VK_NV_low_latency2 (Reflex on the presenter's
    /// swapchain): those extensions and whether they are there. None,
    /// with the reason in the log, without the second queue: frame
    /// generation then presents the prototype's way (dlss_fg.rs). The
    /// library's other presenter extensions (present_id2, present_wait2,
    /// present_timing) want feature structures this ash does not have:
    /// the presenter runs without them, its shown counts unmeasured.
    /// After [`device_extensions`] (which named the adapter), under the
    /// lock.
    #[cfg(target_os = "linux")]
    fn presenter_device_extensions(
        lib: &sl::Lib,
        adapter: &wgpu::hal::vulkan::Adapter,
        wanted: &[&'static CStr],
    ) -> Option<(Vec<&'static CStr>, bool)> {
        let instance = adapter.shared_instance().raw_instance();
        let physical = adapter.raw_physical_device();
        // SAFETY: a query of the adapter's live physical device
        let families = unsafe { instance.get_physical_device_queue_family_properties(physical) };
        let count = families.first().map_or(0, |f| f.queue_count);
        if count < 2 {
            log::info!(
                "DLSS: frame generation presents the prototype's way: queue family 0 has {count} queue(s), the presenter wants a second"
            );
            return None;
        }
        let names = [vk::KHR_PRESENT_ID_NAME, vk::NV_LOW_LATENCY2_NAME];
        let listed = requirements(lib, sl::FEATURE_DLSS_G).map(|(_, device)| device).unwrap_or_default();
        let caps = adapter.physical_device_capabilities();
        let mut present_id = vk::PhysicalDevicePresentIdFeaturesKHR::default();
        let mut features = vk::PhysicalDeviceFeatures2::default().push_next(&mut present_id);
        // SAFETY: as above; `features` and its chain live through the call
        unsafe { instance.get_physical_device_features2(physical, &mut features) };
        let low_latency = present_id.present_id == vk::TRUE && names.iter().all(|n| listed.contains(n) && caps.supports_extension(n));
        if !low_latency {
            log::info!("DLSS: frame generation's presenter without Reflex: no VK_NV_low_latency2 with VK_KHR_present_id here");
        }
        let extra = if low_latency {
            names.into_iter().filter(|n| !wanted.contains(n)).collect()
        } else {
            Vec::new()
        };
        Some((extra, low_latency))
    }

    /// The library's instance extensions, for the instance OpenXR makes
    /// for a VR headset (xr_device.rs): None when DLSS is not set up or
    /// the query failed (the log says why), and the instance is made as
    /// before.
    pub(crate) fn xr_instance_extensions(entry: &ash::Entry) -> Option<Vec<&'static CStr>> {
        if forced() == Some(false) {
            return None;
        }
        let held = idle(&XR_PROBE_BUSY)?;
        let dir = folder()?;
        let lib = sl::lib(dir)?;
        instance_extensions(lib, &held, entry)
            .map_err(|e| log::error!("DLSS: {e}, for the VR headset's instance: FSR draws"))
            .ok()
    }

    /// Streamline's lock, when Streamline may be called for a new instance's
    /// extensions: held by the caller across the query, so that no other
    /// thread enters Streamline beside it. None when a thread holds it longer
    /// than [`END_WAIT`] (one inside a Streamline call on a device lost
    /// meanwhile, [`let_go`]), or when Streamline is started on a live device for
    /// a render thread left behind ([`kept_for_left_behind`]): Streamline is not
    /// called beside either, and the instance is made without it (FSR
    /// draws on its devices). `probe` keeps the answer for the devices of
    /// that instance ([`PROBE_BUSY`], [`XR_PROBE_BUSY`]).
    fn idle(probe: &AtomicBool) -> Option<std::sync::MutexGuard<'static, Option<Started>>> {
        static SAID: AtomicBool = AtomicBool::new(false);
        let Some(held) = super::lock_within(&STARTED, END_WAIT) else {
            probe.store(true, Ordering::Relaxed);
            log::error!(
                "DLSS: Streamline still busy after {END_WAIT:?} (a call that has not returned): the instance is made without it, FSR draws"
            );
            return None;
        };
        let kept = kept_for_left_behind(&held);
        probe.store(kept, Ordering::Relaxed);
        if kept {
            // (once: every instance made while that thread is out says the same)
            if !SAID.swap(true, Ordering::Relaxed) {
                log::error!("DLSS: Streamline kept for a render thread left behind: instances are made without it, FSR draws");
            }
            return None;
        }
        Some(held)
    }

    /// Streamline as `held` is kept for a render thread left behind
    /// ([`LEFT_BEHIND`]): started on a device that still works, where that
    /// thread may still evaluate its features. Nothing started, or a start
    /// on a lost device (whose calls `with_started` refuses, that thread's
    /// too), keeps nothing from the next device.
    fn kept_for_left_behind(held: &Option<Started>) -> bool {
        LEFT_BEHIND.load(Ordering::Relaxed) && held.as_ref().is_some_and(|st| !st.lost.get())
    }

    /// The library's device extensions, for the device OpenXR opens for a
    /// VR headset (xr_device.rs), whose instance has its extensions: None
    /// when the lock is busy or the query failed (the log says why), and
    /// the device is opened as before.
    pub(crate) fn xr_device_extensions(adapter: &wgpu::hal::vulkan::Adapter) -> Option<Vec<&'static CStr>> {
        let dir = folder()?;
        let lib = sl::lib(dir)?;
        let Some(_held) = super::lock_within(&STARTED, END_WAIT) else {
            log::error!("DLSS: Streamline still busy after {END_WAIT:?}, for the VR headset's device: FSR draws");
            return None;
        };
        device_extensions(lib, adapter)
            .map_err(|e| log::error!("DLSS: {e}, for the VR headset's device: FSR draws"))
            .ok()
    }

    /// A device is being opened: the one opened for DLSS before is
    /// forgotten until the new one is known ([`request_device`],
    /// [`opened_for_xr`]), and a start Streamline refused, or a failure shown,
    /// is the new device's to find again. Streamline started on the previous
    /// device is let go ([`let_go`]): its renderer is gone (gpu_recovery.rs
    /// drops it first), and a device left while it works, VR's after the
    /// runtime closed, is not kept open by Streamline until DLSS starts again,
    /// which may be never. False when Streamline's lock could not be taken: the
    /// new device then opens without Streamline ([`BUSY`]), so that the recovery
    /// finishes. `xr`: the VR headset's device, from its own instance.
    pub(crate) fn opening_device(xr: bool) -> bool {
        let free = let_go(Ending::Moving);
        *opened() = None;
        XR.store(false, Ordering::Relaxed);
        // Ray Reconstruction is forgotten with it: only [`request_device`]
        // opens a device with its extensions (a VR headset's has OpenXR's
        // and super resolution's), and a refusal belongs to the device
        // Streamline gave it
        RR_DEVICE.store(false, Ordering::Relaxed);
        RR_KNOWN.store(false, Ordering::Relaxed);
        RR_UNAVAILABLE.store(false, Ordering::Relaxed);
        RR_REFUSED.store(false, Ordering::Relaxed);
        FG_SUPPORTED.store(false, Ordering::Relaxed);
        SNAPSHOT_RR_REFUSED.store(false, Ordering::Relaxed);
        FAILED.store(false, Ordering::Relaxed);
        REFUSAL.store(false, Ordering::Relaxed);
        let probe = if xr { &XR_PROBE_BUSY } else { &PROBE_BUSY };
        BUSY.store(!free || probe.load(Ordering::Relaxed), Ordering::Relaxed);
        OPENS.fetch_add(1, Ordering::Relaxed);
        free
    }

    /// The render thread of a renderer that closed did not leave within
    /// its wait (render_thread.rs, `Host`'s drop) and was left behind with
    /// its backend: a start of Streamline on a device that still works is kept
    /// for it, not ended until it returns ([`closing`]), and the devices opened beside it run without
    /// DLSS ([`LEFT_BEHIND`]). Waits for nothing (called at exit too).
    pub(crate) fn left_behind() {
        if !LEFT_BEHIND.swap(true, Ordering::Relaxed) {
            log::error!(
                "DLSS: a render thread was left behind in a GPU call: Streamline, if started on a device that works, stays as it is \
                 for it, and the next devices open without DLSS while it does (FSR draws)"
            );
        }
    }

    /// The main renderer of `device` is closing for good on a device that
    /// still works (renderer.rs, `Backend`'s drop: the viewer's exit, or a
    /// switch to or from VR): Streamline is let go there now, as when the next
    /// device opens, and the device is no longer held here, so the Vulkan
    /// device is destroyed with its renderer and Streamline shut down before it.
    /// Bounded as [`let_go`] is: an exit never waits on Streamline for longer.
    ///
    /// Only `device`'s own: this may come late, from a render thread left
    /// behind that returned at last (its detached result, the backend, is
    /// dropped on it when it ends), after the next device opened. So the
    /// check and the clearing are done under Streamline's lock, which
    /// [`opening_device`]'s [`let_go`] and `with_started` take too, and touch
    /// only what is `device`'s: the next device's opening and start stay.
    /// A start kept for that thread ([`LEFT_BEHIND`]) is ended here, its
    /// renderer gone, and the flag with it: later devices open with Streamline.
    pub(crate) fn closing(device: &wgpu::Device) {
        let Some(mut held) = super::lock_within(&STARTED, END_WAIT) else {
            // (Streamline left to the thread in it, as `let_go` does; the device
            // is still not held here, whatever opens next)
            let mut opened = opened();
            if opened.as_ref().is_some_and(|(_, d)| same(d, device)) {
                *opened = None;
            }
            log::error!("DLSS: Streamline still busy after {END_WAIT:?} (a call that has not returned): not shut down with its device");
            return;
        };
        {
            let mut opened = opened();
            if opened.as_ref().is_some_and(|(_, d)| same(d, device)) {
                *opened = None;
            }
        }
        if !held.as_ref().is_some_and(|st| same(&st.device, device)) {
            return;
        }
        let Some(old) = held.take() else {
            return;
        };
        LEFT_BEHIND.store(false, Ordering::Relaxed);
        match sl::loaded() {
            Some(lib) => drop(end(lib, old, held, Ending::Closing)),
            None => abandon(old),
        }
    }

    /// Streamline's state taken from wherever it was started and ended there
    /// ([`end`]), or left ([`abandon`]) on a lost device. Streamline's lock is
    /// taken within [`END_WAIT`] or not at all: a thread inside a Streamline
    /// call on a device lost meanwhile may never return, with the lock
    /// held, and the main thread opening the next device would wait for
    /// it for ever (`Host`'s drop leaves such a render thread behind,
    /// render_thread.rs). Then nothing of Streamline is touched (that thread
    /// owns it), and false.
    fn let_go(why: Ending) -> bool {
        let Some(mut held) = super::lock_within(&STARTED, END_WAIT) else {
            log::error!(
                "DLSS: Streamline still busy on the previous device after {END_WAIT:?} (a call that has not returned): \
                 left to it, the new device opens without DLSS and FSR draws"
            );
            return false;
        };
        // (a render thread left behind may still evaluate there: nothing
        // of it is ended; a start on a lost device is abandoned as always)
        if kept_for_left_behind(&held) {
            log::warn!("DLSS: Streamline kept as it is for a render thread left behind: the new device opens without DLSS, FSR draws");
            return false;
        }
        // Nothing is kept for a thread left behind, if one was: the device
        // open now is let go under Streamline's lock, which `with_started` reads it
        // under, so that thread cannot start Streamline on its device from here,
        // and the flag goes, so that the next device's own start is not
        // taken for that thread's (it is ended as any other).
        *opened() = None;
        LEFT_BEHIND.store(false, Ordering::Relaxed);
        let Some(old) = held.take() else {
            return true;
        };
        match sl::loaded() {
            Some(lib) => end(lib, old, held, why).is_some(),
            None => {
                abandon(old);
                true
            }
        }
    }

    /// OpenXR opened `device` for a VR headset (xr_device.rs). `with_dlss`: with
    /// the DLSS extensions on its instance and device, and then DLSS draws
    /// there as on a device of [`request_device`]; without, DLSS is not
    /// offered while the headset owns the device, and Preferences says why.
    pub(crate) fn opened_for_xr(adapter: &wgpu::Adapter, device: &wgpu::Device, with_dlss: bool) {
        if with_dlss {
            *opened() = Some((adapter.clone(), device.clone()));
            log::info!(
                "DLSS: VR headset's device opened with the DLSS extensions on {}",
                adapter.get_info().name
            );
        } else {
            *opened() = None;
            XR.store(true, Ordering::Relaxed);
            log::info!("DLSS: VR headset's device opened without the DLSS extensions: FSR draws");
        }
    }

    /// Streamline refused to start on the device open now ([`REFUSED`]): nothing
    /// but [`retry`] or another device asks it again. A refusal met here
    /// is [`FAILED`] for Preferences, with its Try again button, Streamline not
    /// asked, whichever view meets it: a refusal is the device's, not a
    /// view's (with VR drawing, only the eyes ask, and neither reports
    /// its own failures), and DLAA asks Streamline nothing before its feature,
    /// so no other path would show it.
    pub(crate) fn refused_here() -> bool {
        let refused = super::refused(REFUSED.load(Ordering::Relaxed), OPENS.load(Ordering::Relaxed));
        if refused {
            refusal_shown();
        }
        refused
    }

    /// The user asked to try DLSS again (Preferences): a start Streamline
    /// refused is asked again, Ray Reconstruction too when it was refused,
    /// and every renderer makes its feature anew ([`retry_asked`]).
    /// [`FAILED`] stays until a feature is made.
    pub(crate) fn retry() {
        REFUSED.store(0, Ordering::Relaxed);
        // Ray Reconstruction's refusal too: a feature refused again sets it
        // back (`rr_refused`), so a retry costs at most one more pipeline
        // rebuild
        RR_REFUSED.store(false, Ordering::Relaxed);
        SNAPSHOT_RR_REFUSED.store(false, Ordering::Relaxed);
        RETRIES.fetch_add(1, Ordering::Relaxed);
        log::info!("DLSS: trying again, as asked");
    }

    /// Another Ray Reconstruction model was picked (Preferences): a
    /// refusal is forgotten, so the renderer builds the guides again and
    /// asks Streamline with that model; refused again, it is set back
    /// (`rr_refused`).
    pub(crate) fn rr_retry() {
        let snapshot = SNAPSHOT_RR_REFUSED.swap(false, Ordering::Relaxed);
        if RR_REFUSED.swap(false, Ordering::Relaxed) || snapshot {
            log::info!("DLSS: Ray Reconstruction tried again, with another model");
        }
    }

    /// Ray Reconstruction refused for a photo or film frame
    /// ([`snapshot_rr_refused`]).
    fn snapshot_refusal(why: &str) {
        if !SNAPSHOT_RR_REFUSED.swap(true, Ordering::Relaxed) {
            log::warn!("DLSS: Ray Reconstruction for a snapshot: {why}: super resolution draws every snapshot from here, until Try again");
        }
    }

    /// A [`retry`] came since `seen`, which is moved to it.
    pub(crate) fn retry_asked(seen: &mut u64) -> bool {
        let now = RETRIES.load(Ordering::Relaxed);
        std::mem::replace(seen, now) != now
    }

    /// The instance made by [`instance`] was replaced by a plain one (the
    /// Windows adapter retry): a device from it has no DLSS extensions.
    #[cfg(windows)]
    pub(crate) fn forget_instance() {
        INSTANCE.store(false, Ordering::Relaxed);
    }

    /// A Vulkan instance with the DLSS extensions, when DLSS is wanted.
    pub(crate) fn instance(desc: &wgpu::InstanceDescriptor) -> Option<wgpu::Instance> {
        INSTANCE.store(false, Ordering::Relaxed);
        RR_INSTANCE.store(false, Ordering::Relaxed);
        // opened with Streamline whenever the user's files are there, so that the
        // setting turns DLSS on and off without a new device
        if forced() == Some(false) {
            return None;
        }
        let dir = folder()?;
        // Windows: the previous device's start ended first, while that
        // device lives (the library shuts down before it goes, [`fresh`])
        #[cfg(windows)]
        if !let_go(Ending::Moving) {
            return None;
        }
        // (held across the instance's making, which asks the library; no
        // wgpu resource is touched under it)
        let held = idle(&PROBE_BUSY)?;
        let lib = sl::lib(dir)?;
        // Ray Reconstruction's extensions too, when the loader has them all
        // (`rr_instance_extensions`); refused with them, super resolution's
        // alone, so DLSS is not lost for it
        let with_rr = rr_forced() != Some(false);
        let mut raw = made(lib, &held, desc, with_rr);
        if with_rr && let Err(e) = &raw {
            log::error!("DLSS: {e}: making the instance with super resolution's extensions alone");
            raw = made(lib, &held, desc, false);
        }
        drop(held);
        match raw {
            Ok((raw, rr)) => {
                INSTANCE.store(true, Ordering::Relaxed);
                RR_INSTANCE.store(rr, Ordering::Relaxed);
                // SAFETY: `raw` is a live Vulkan instance made with wgpu's
                // own instance flags and extensions, handed over whole
                Some(unsafe { wgpu::Instance::from_hal::<Vulkan>(raw) })
            }
            Err(e) => {
                log::error!("DLSS: {e}: FSR draws");
                None
            }
        }
    }

    /// Linux: wgpu-hal's own instance, the library's extensions added as
    /// it is made, Ray Reconstruction's too with `with_rr` when the loader
    /// has them all (the instance says whether they were). Under the lock
    /// (`held`).
    #[cfg(target_os = "linux")]
    fn made(
        lib: &sl::Lib,
        held: &Option<Started>,
        desc: &wgpu::InstanceDescriptor,
        with_rr: bool,
    ) -> Result<(wgpu::hal::vulkan::Instance, bool), String> {
        let mut added = Err(String::from("the instance was made without asking"));
        let mut rr = false;
        // SAFETY: the descriptor is wgpu's own, copied field by field; the
        // callback only reads the entry and pushes names kept for the life
        // of the process (`interned`) onto the extensions asked for
        let raw = unsafe {
            wgpu::hal::vulkan::Instance::init_with_callback(
                &wgpu::hal::InstanceDescriptor {
                    name: "wgpu",
                    flags: desc.flags,
                    memory_budget_thresholds: desc.memory_budget_thresholds,
                    backend_options: desc.backend_options.clone(),
                    telemetry: None,
                    display: None,
                },
                Some(Box::new(|args| {
                    added = instance_extensions(lib, held, args.entry).map(|mut wanted| {
                        if with_rr && let Some(extra) = rr_instance_extensions(lib, args.entry, &wanted) {
                            wanted.extend(extra);
                            rr = true;
                        }
                        for name in wanted {
                            if !args.extensions.contains(&name) {
                                args.extensions.push(name);
                            }
                        }
                    });
                })),
            )
        }
        .map_err(|e| format!("instance with the DLSS extensions failed: {e}"))?;
        added.map(|()| (raw, rr))
    }

    /// Windows: the instance made through the interposer's
    /// `vkGetInstanceProcAddr`, so that Streamline sees it made, and the
    /// device and swapchains made from it (wgpu-hal loads every function
    /// through the instance). Made as wgpu-hal's `init_with_callback`
    /// makes it, without the validation layer and its messenger. Under the
    /// lock (`held`).
    #[cfg(windows)]
    fn made(
        lib: &sl::Lib,
        held: &Option<Started>,
        desc: &wgpu::InstanceDescriptor,
        with_rr: bool,
    ) -> Result<(wgpu::hal::vulkan::Instance, bool), String> {
        // slInit before any Vulkan call through the interposer (making
        // the entry already asks it for the global functions)
        fresh(lib, held)?;
        ensure_init(lib)?;
        let entry = interposer_entry(lib);
        let mut wanted = instance_extensions(lib, held, &entry)?;
        // Ray Reconstruction's too, when the loader has them all
        let mut rr = false;
        if with_rr && let Some(extra) = rr_instance_extensions(lib, &entry, &wanted) {
            wanted.extend(extra);
            rr = true;
        }
        // SAFETY: queries of the interposer's loader
        let version = unsafe { entry.try_enumerate_instance_version() }
            .map_err(|e| format!("vkEnumerateInstanceVersion: {e}"))?
            .unwrap_or(vk::API_VERSION_1_0);
        // (wgpu-hal's choice: 1.0 for a 1.0 loader, else the 1.3 it targets)
        let api = if version < vk::API_VERSION_1_1 {
            vk::API_VERSION_1_0
        } else {
            vk::API_VERSION_1_3
        };
        let mut extensions = wgpu::hal::vulkan::Instance::desired_extensions(&entry, version, desc.flags)
            .map_err(|e| format!("the instance's extensions: {e}"))?;
        for name in wanted {
            if !extensions.contains(&name) {
                extensions.push(name);
            }
        }
        // SAFETY: as above
        let optimus = unsafe { entry.enumerate_instance_layer_properties() }
            .is_ok_and(|layers| layers.iter().any(|l| l.layer_name_as_c_str() == Ok(c"VK_LAYER_NV_optimus")));
        let pointers: Vec<_> = extensions.iter().map(|name| name.as_ptr()).collect();
        let app = vk::ApplicationInfo::default()
            .application_name(c"wgpu")
            .application_version(1)
            .engine_name(c"wgpu-hal")
            .engine_version(2)
            .api_version(api);
        let create = vk::InstanceCreateInfo::default()
            .application_info(&app)
            .enabled_extension_names(&pointers);
        // SAFETY: the create info and its pointees live through the call;
        // every extension is the loader's (wgpu-hal's list keeps only
        // those, `instance_extensions` checked the library's)
        let raw = unsafe { entry.create_instance(&create, None) }.map_err(|e| format!("vkCreateInstance: {e}"))?;
        // SAFETY: `raw` was made from `entry` with `api`, wgpu's flags and
        // a superset of `desired_extensions`; wgpu-hal destroys it (no
        // callback), through the interposer, which only passes that on
        unsafe {
            wgpu::hal::vulkan::Instance::from_raw(
                entry,
                raw,
                version,
                0,
                None,
                extensions,
                desc.flags,
                desc.memory_budget_thresholds,
                optimus,
                None,
            )
        }
        .map(|raw| (raw, rr))
        .map_err(|e| format!("the instance through sl.interposer.dll: {e}"))
    }

    /// The interposer as the Vulkan loader. After [`ensure_init`]: making
    /// it already calls the interposer.
    #[cfg(windows)]
    fn interposer_entry(lib: &sl::Lib) -> ash::Entry {
        // SAFETY: the interposer's own `vkGetInstanceProcAddr`, kept loaded
        // for the process (`sl::lib`), with the type vulkan_core.h gives it
        unsafe {
            ash::Entry::from_static_fn(ash::StaticFn {
                get_instance_proc_addr: lib.get_instance_proc_addr,
            })
        }
    }

    /// Windows: the library afresh for a new instance. Streamline attaches
    /// to the first device made through the interposer after `slInit`, and
    /// to no other until `slShutdown`, which must come while the device it
    /// holds lives (its plugins free their objects there). So the shutdown
    /// comes when a start ends, as on Linux ([`end`]), with the device,
    /// and again here when nothing is started. A start left on a lost
    /// device ([`abandon`]) cannot be shut down safely, nor let go, as
    /// sl-ngx does: DLSS stays off until the viewer restarts
    /// ([`GIVEN_UP`]). Under the lock (`held`).
    #[cfg(windows)]
    fn fresh(lib: &sl::Lib, held: &Option<Started>) -> Result<(), String> {
        if GIVEN_UP.load(Ordering::Relaxed) {
            return Err(String::from(
                "Streamline still holds the device lost before: DLSS is off until the viewer restarts",
            ));
        }
        if held.is_some() {
            return Err(String::from("DLSS still runs on the previous device"));
        }
        shut_down(lib);
        Ok(())
    }

    /// Windows: the Vulkan loader for the instance OpenXR makes for a VR
    /// headset (xr_device.rs): the interposer, the library made ready, so
    /// that Streamline sees that instance and its device made. None for
    /// the system's loader (no DLSS in VR then: the log says why).
    #[cfg(windows)]
    pub(crate) fn xr_entry() -> Option<ash::Entry> {
        if forced() == Some(false) {
            return None;
        }
        let dir = folder()?;
        if !let_go(Ending::Moving) {
            return None;
        }
        let held = idle(&XR_PROBE_BUSY)?;
        let lib = sl::lib(dir)?;
        let ready = fresh(lib, &held).and_then(|()| ensure_init(lib));
        drop(held);
        match ready {
            Ok(()) => Some(interposer_entry(lib)),
            Err(e) => {
                log::error!("DLSS: {e}, for the VR headset: FSR draws");
                None
            }
        }
    }

    /// The device opened with the DLSS extensions, when [`instance`] made the
    /// instance; `None` to open it the usual way (also when Streamline's way fails).
    pub(crate) fn request_device(
        adapter: &wgpu::Adapter,
        desc: &wgpu::DeviceDescriptor,
        limits: wgpu::Limits,
    ) -> Option<(wgpu::Device, wgpu::Queue)> {
        // (what the last device had for the presenter is not this one's)
        #[cfg(target_os = "linux")]
        PRESENTER_DEVICE.store(0, Ordering::Relaxed);
        if !opening_device(false) || !INSTANCE.load(Ordering::Relaxed) {
            return None;
        }
        let dir = folder()?;
        let lib = sl::lib(dir)?;
        // SAFETY: the hal adapter is only read here, while `adapter` lives
        let raw_adapter = unsafe { adapter.as_hal::<Vulkan>() }?;
        // (held across the opening, whose callback asks the library)
        let Some(held) = super::lock_within(&STARTED, END_WAIT) else {
            log::error!("DLSS: Streamline still busy after {END_WAIT:?}: the device opens the usual way, FSR draws");
            return None;
        };
        // `with_rr`: Ray Reconstruction's extensions too, when the device
        // has them all (it is switched on and off with the device open).
        // Whether they were added is known for sure only once it is open,
        // so RR_DEVICE is set there.
        // `with_fg` (Linux): sl-ngx's presenter's queue and extensions too
        // ([`presenter_device_extensions`]); `fg` comes back with what was
        // added for it ([`PRESENTER_DEVICE`]).
        let open = |with_rr: bool, with_fg: bool| {
            let mut added = Err(String::from("the device was opened without asking"));
            let mut rr = false;
            let mut fg = 0u32;
            // (the presenter's two queues share family 0, as wgpu's does)
            #[cfg(target_os = "linux")]
            static PRIORITIES: [f32; 2] = [1.0, 1.0];
            #[cfg(target_os = "linux")]
            let mut present_id = vk::PhysicalDevicePresentIdFeaturesKHR::default().present_id(true);
            #[cfg(target_os = "linux")]
            let present_id = &mut present_id;
            #[cfg(not(target_os = "linux"))]
            let _ = with_fg;
            // SAFETY: features, limits and hints are the descriptor's, as
            // wgpu's own `request_device` passes them; the callback asks
            // the library with this adapter's live instance and physical
            // device, and only adds extensions, queues and features the
            // device supports, the feature structure outliving the call
            let opened = unsafe {
                raw_adapter.open_with_callback(
                    desc.required_features,
                    &limits,
                    &desc.memory_hints,
                    Some(Box::new(|args| {
                        added = device_extensions(lib, &raw_adapter).map(|mut wanted| {
                            if with_rr && let Some(extra) = rr_device_extensions(lib, &raw_adapter, &wanted) {
                                wanted.extend(extra);
                                rr = true;
                            }
                            #[cfg(target_os = "linux")]
                            if with_fg
                                && let Some((extra, low_latency)) = presenter_device_extensions(lib, &raw_adapter, &wanted)
                                && let Some(family) = args.queue_create_infos.first_mut()
                            {
                                *family = family.queue_priorities(&PRIORITIES);
                                wanted.extend(extra);
                                fg = PRESENTER_QUEUE;
                                if low_latency {
                                    let info = std::mem::take(args.create_info);
                                    *args.create_info = info.push_next(present_id);
                                    fg |= PRESENTER_LOW_LATENCY2;
                                }
                            }
                            for name in wanted {
                                if !args.extensions.contains(&name) {
                                    args.extensions.push(name);
                                }
                            }
                        });
                    })),
                )
            };
            (opened, added, rr, fg)
        };
        // (an instance without Ray Reconstruction's extensions opens its
        // devices for super resolution alone: `RR_INSTANCE`)
        let with_rr = rr_forced() != Some(false) && RR_INSTANCE.load(Ordering::Relaxed);
        let with_fg = cfg!(target_os = "linux");
        let (mut opened, mut added, mut rr, mut fg) = open(with_rr, with_fg);
        // refused with the presenter's queue and extensions: without them,
        // and frame generation presents the prototype's way
        if fg != 0
            && let Err(e) = &opened
        {
            log::error!("DLSS: device with frame generation's presenter queue failed: {e}: opening it without");
            (opened, added, rr, fg) = open(with_rr, false);
        }
        // refused with Ray Reconstruction's extensions: super resolution's
        // alone before the usual way, so DLSS is not lost for it
        if rr && let Err(e) = &opened {
            log::error!("DLSS: device with Ray Reconstruction's extensions failed: {e}: opening it with super resolution's alone");
            (opened, added, rr, fg) = open(false, false);
        }
        drop(held);
        let opened = match (opened, added) {
            (Ok(opened), Ok(())) => opened,
            (Err(e), _) => {
                log::error!("DLSS: device with the DLSS extensions failed: {e}: opened the usual way, FSR draws");
                return None;
            }
            (Ok(opened), Err(e)) => {
                // the device is good, only without what DLSS needs
                log::error!("DLSS: {e} on {}: FSR draws", adapter.get_info().name);
                // SAFETY: `opened` was opened from this adapter with the
                // descriptor's features and limits
                return unsafe { adapter.create_device_from_hal::<Vulkan>(opened, desc) }.ok();
            }
        };
        // SAFETY: as above
        match unsafe { adapter.create_device_from_hal::<Vulkan>(opened, desc) } {
            Ok(pair) => {
                *self::opened() = Some((adapter.clone(), pair.0.clone()));
                RR_DEVICE.store(rr, Ordering::Relaxed);
                #[cfg(target_os = "linux")]
                PRESENTER_DEVICE.store(fg, Ordering::Relaxed);
                #[cfg(not(target_os = "linux"))]
                let _ = fg;
                log::info!(
                    "DLSS: device opened with the DLSS extensions on {}{}{}",
                    adapter.get_info().name,
                    if rr { ", Ray Reconstruction's included" } else { "" },
                    match fg {
                        0 => "",
                        f if f & 1 != 0 => ", with frame generation's presenter queue and Reflex",
                        _ => ", with frame generation's presenter queue",
                    }
                );
                Some(pair)
            }
            Err(e) => {
                log::error!("DLSS: device with the DLSS extensions failed: {e}: opened the usual way, FSR draws");
                None
            }
        }
    }

    /// Whether two devices are one Vulkan device. Not `==`: wgpu compares
    /// the identifier its instance gave, and the instance made after a
    /// device loss gives the same ones again (seen: Streamline stayed on the lost
    /// device, and the new one was lost at the first evaluate).
    fn same(a: &wgpu::Device, b: &wgpu::Device) -> bool {
        // SAFETY: the handle is only read and compared, never used
        let raw = |d: &wgpu::Device| unsafe { d.as_hal::<Vulkan>() }.map(|hal| hal.raw_device().handle());
        matches!((raw(a), raw(b)), (Some(a), Some(b)) if a == b)
    }

    /// DLSS started on `device`: the library told of the device (Linux),
    /// then asked whether DLSS runs on its adapter. Under the lock.
    fn start(lib: &sl::Lib, device: &wgpu::Device) -> Result<(), String> {
        ensure_init(lib)?;
        // SAFETY: the handles are only named to the library, which uses
        // them while the device lives (the clone in `Started`, or kept for
        // ever by `abandon`)
        let hal = unsafe { device.as_hal::<Vulkan>() }.ok_or("not a Vulkan device")?;
        let physical = hal.raw_physical_device();
        #[cfg(target_os = "linux")]
        {
            let info = sl::VulkanInfo::new(hal.shared_instance().raw_instance().handle(), physical, hal.raw_device().handle());
            // SAFETY: as sl_helpers_vk.h declares it, with this device's
            // live handles
            let r = sl::check(unsafe { (lib.set_vulkan_info)(&info) });
            // (a start left on a lost device was let go by that call, even
            // when the new start failed)
            ABANDONED.store(false, Ordering::Relaxed);
            if let Err(e) = r {
                shut_down(lib);
                return Err(format!("DLSS did not start: slSetVulkanInfo: {e}"));
            }
        }
        let adapter = sl::AdapterInfo::new(physical);
        // SAFETY: as sl.h declares it; `adapter` lives through the call
        let r = unsafe { (lib.is_feature_supported)(sl::FEATURE_DLSS, &adapter) };
        if r == sl::OK {
            // frame generation: Linux's sl-ngx, and Windows' Streamline
            // when it loaded the plugin ([`FG_AT_START`])
            #[cfg(windows)]
            let asked = FG_LOADED.load(Ordering::Relaxed);
            #[cfg(not(windows))]
            let asked = true;
            if asked {
                // SAFETY: as above
                let fg = unsafe { (lib.is_feature_supported)(sl::FEATURE_DLSS_G, &adapter) };
                #[allow(unused_mut)] // Windows: Reflex too
                let mut ok = fg == sl::OK;
                if !ok {
                    log::info!("DLSS: no frame generation here ({})", sl::result_name(fg));
                }
                // Windows: Reflex in low-latency mode, which frame
                // generation needs
                #[cfg(windows)]
                if ok {
                    // SAFETY: under the lock, after slInit; `options`
                    // lives through the call
                    let reflex = unsafe {
                        lib.dlssg().and_then(|f| {
                            let options = sl::ReflexOptions::new(sl::REFLEX_LOW_LATENCY);
                            sl::check((f.reflex_set_options)(&options)).map_err(|e| format!("slReflexSetOptions: {e}"))
                        })
                    };
                    if let Err(e) = reflex {
                        log::error!("DLSS: no frame generation here: {e}");
                        ok = false;
                    }
                }
                FG_SUPPORTED.store(ok, Ordering::Relaxed);
                if ok {
                    log::info!("DLSS: frame generation supported here");
                }
            }
            if RR_DEVICE.load(Ordering::Relaxed) {
                // (Streamline's answer alone: a refusal of the feature on
                // this device, RR_REFUSED, stays through a new start)
                // SAFETY: as above
                let rr = unsafe { (lib.is_feature_supported)(sl::FEATURE_DLSS_RR, &adapter) };
                RR_UNAVAILABLE.store(rr != sl::OK, Ordering::Relaxed);
                RR_KNOWN.store(true, Ordering::Relaxed);
                if rr == sl::OK {
                    log::info!("DLSS: Ray Reconstruction supported here");
                } else {
                    log::info!("DLSS: no Ray Reconstruction here ({})", sl::result_name(rr));
                }
            }
            return Ok(());
        }
        let e = if r == sl::ERROR_DRIVER_OUT_OF_DATE {
            String::from("DLSS needs a newer NVIDIA driver")
        } else {
            format!(
                "no DLSS on this device ({}): is the {SNIPPET} in {} one this driver takes?",
                sl::result_name(r),
                folder().unwrap_or("the DLSS folder")
            )
        };
        // a start that goes no further gives back what it took (it is
        // tried again at the next size)
        #[cfg(target_os = "linux")]
        shut_down(lib);
        Err(e)
    }

    /// How long ending Streamline on a device left for good waits for that
    /// device's last frames ([`end`]), and a thread that is not Streamline's own
    /// for DLSS's lock ([`let_go`]).
    const END_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

    /// Ends DLSS on a device the renderer left for good while it still
    /// works (VR's, after the runtime closed), when the next device opens
    /// ([`opening_device`]) or else at the next start: its viewports freed
    /// once its last frames are done, then `slShutdown` (the library is
    /// initialised again when next needed). The wait is bounded
    /// ([`END_WAIT`]); a device that does not finish in time, or is lost,
    /// is [`abandon`]ed instead. The lock is not held during the wait (the
    /// frames' callbacks take it, [`release`]), only for the library's
    /// calls after it: `held` is given up and taken again within
    /// [`END_WAIT`], returned; None when it could not be (`old` is then
    /// abandoned).
    fn end(
        lib: &sl::Lib,
        old: Started,
        held: std::sync::MutexGuard<'static, Option<Started>>,
        why: Ending,
    ) -> Option<std::sync::MutexGuard<'static, Option<Started>>> {
        if old.lost.get() {
            abandon(old);
            return Some(held);
        }
        // the lock let go for the wait, the old start kept outside it: the
        // callbacks find no start of theirs and leave their viewports,
        // freed below
        drop(held);
        let done = crate::device_lost::poll_guarded(
            &old.device,
            wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(END_WAIT),
            },
        );
        let Some(held) = super::lock_within(&STARTED, END_WAIT) else {
            abandon(old);
            return None;
        };
        // Another start was made during the wait, on another device (a
        // start needs a device just opened, and `old`'s is not): the
        // library let `old` go when told of that device, so nothing more
        // is called for it, and a shutdown would end the new start.
        if held.is_some() {
            log::info!("DLSS: the previous device's start was let go by the next start");
            std::mem::forget(old);
            return Some(held);
        }
        // (a loss during the wait is seen by the callback, or by the wait)
        if !matches!(done, Some(Ok(_))) || old.lost.get() {
            abandon(old);
            return Some(held);
        }
        for (id, feature) in &old.viewports {
            let viewport = sl::ViewportHandle::new(*id);
            // SAFETY: the device finished every frame (the wait above), so
            // no feature is in use; the viewport is one this start made and
            // did not free (still in its list); under the lock (`held`)
            if let Err(e) = sl::check(unsafe { (lib.free)(*feature, &viewport) }) {
                log::warn!("DLSS: slFreeResources: {e}");
            }
        }
        // (on the device the start was on, still alive: the clone in `old`;
        // Streamline wants its shutdown before the device goes)
        shut_down(lib);
        match why {
            Ending::Closing => log::info!(
                "DLSS shut down with its device, at exit or a switch to or from VR ({} features freed)",
                old.viewports.len()
            ),
            Ending::Moving => log::info!("DLSS ended on the previous device ({} features freed)", old.viewports.len()),
        }
        Some(held)
    }

    /// Why Streamline is [`end`]ed, for the log.
    #[derive(Clone, Copy)]
    enum Ending {
        /// its device's main renderer closed for good ([`closing`])
        Closing,
        /// another device took over ([`opening_device`], [`with_started`])
        Moving,
    }

    /// Leaves DLSS started on the device it was started on, for good: the
    /// renderer moved to another device after a device loss (or one that
    /// did not finish its frames in time, [`end`]).
    ///
    /// Not ended there. Waiting for a lost device is not bounded (a hung
    /// GPU never idles, and recovery stopped here with the lock held), and
    /// freeing on it runs commands on a device that runs none. On Linux
    /// nor needed: sl-ngx starts again beside the old start on the next
    /// device (`slSetVulkanInfo` with a new device leaves the old start
    /// without a call there). On Windows Streamline keeps the lost device
    /// until a shutdown, which would call into it: DLSS stays off until
    /// the viewer restarts ([`fresh`]). The device is kept with it (not
    /// dropped), so the Vulkan device the library holds objects on is never
    /// destroyed under it; one per device loss, the cost of not hanging.
    fn abandon(old: Started) {
        log::warn!(
            "DLSS left started on the previous device ({} features kept there): started on the new one",
            old.viewports.len()
        );
        #[cfg(target_os = "linux")]
        ABANDONED.store(true, Ordering::Relaxed);
        #[cfg(windows)]
        GIVEN_UP.store(true, Ordering::Relaxed);
        std::mem::forget(old);
    }

    /// Runs `f` with Streamline as started on `device`, whose loss its renderer
    /// sees in `lost`. With `start`, Streamline is started when it is not, and
    /// started anew when it was started on another device (a renderer
    /// made after a device loss, [`abandon`]); without, a feature of
    /// another device gets nothing. A start refused is [`FAILED`] for
    /// Preferences, whichever view asked (it is the device's refusal,
    /// [`refused_here`]); what fails after is the caller's to report. A start
    /// refused on this device is not asked again ([`REFUSED`]).
    ///
    /// Nothing on a lost device: asked before Streamline's lock is taken (a
    /// renderer left behind by the loss does not queue for the lock the
    /// next device's needs) and again once it is held (the loss may have
    /// come while it waited; `opening_device` may have taken the state).
    fn with_started<T>(
        lib: &sl::Lib,
        device: &wgpu::Device,
        lost: &DeviceLost,
        start: bool,
        f: impl FnOnce(&mut Started) -> T,
    ) -> Option<T> {
        if lost.get() {
            return None;
        }
        let mut held = started();
        if lost.get() || held.as_ref().is_some_and(|st| st.lost.get() && same(&st.device, device)) {
            return None;
        }
        // (Streamline is started only on the device opened now, read under Streamline's
        // lock: a renderer of a device closed meanwhile neither ends
        // another device's Streamline nor starts it on its own)
        let current = || opened().as_ref().is_some_and(|(_, d)| same(d, device));
        if held.as_ref().is_some_and(|st| !same(&st.device, device)) {
            if !start || !current() {
                return None;
            }
            if let Some(old) = held.take() {
                held = end(lib, old, held, Ending::Moving)?;
            }
            // (started again during the wait, on another device: not ours)
            if held.as_ref().is_some_and(|st| !same(&st.device, device)) {
                return None;
            }
        }
        if held.is_none() {
            if !start || !current() {
                return None;
            }
            let opens = OPENS.load(Ordering::Relaxed);
            if super::refused(REFUSED.load(Ordering::Relaxed), opens) {
                // (the reason is in the log already, from the refusal)
                refusal_shown();
                return None;
            }
            match self::start(lib, device) {
                Ok(()) => {
                    // (a refusal shown before, of this device before a
                    // retry, is over: DLSS runs, and Preferences no longer
                    // says it failed, whichever view started it)
                    working(false);
                    *held = Some(Started {
                        device: device.clone(),
                        start: STARTS.fetch_add(1, Ordering::Relaxed) + 1,
                        viewports: Vec::new(),
                        lost: lost.clone(),
                        #[cfg(target_os = "linux")]
                        presenter: None,
                        #[cfg(target_os = "linux")]
                        presenter_pauses: 0,
                    });
                }
                Err(e) => {
                    REFUSED.store(opens, Ordering::Relaxed);
                    log::error!("DLSS: {e}: FSR draws");
                    refusal_shown();
                    return None;
                }
            }
        }
        held.as_mut().map(f)
    }

    /// Linux: sl-ngx's presenter of frame generation on the window's
    /// surface (`sl::present`), with the two timelines its frames are
    /// handed over by: `ready`, signalled on the viewer's queue once a
    /// frame's images are written, and `released`, signalled by the
    /// presenter once it has read them. Every call is made under
    /// Streamline's lock, for the start it was made on: slShutdown, and
    /// slSetVulkanInfo with another device, end the presenter under that
    /// lock, after which its handle is never called again ([`Presenter::call`]).
    #[cfg(target_os = "linux")]
    pub(crate) struct Presenter {
        raw: usize,
        lib: &'static sl::Lib,
        device: wgpu::Device,
        lost: DeviceLost,
        start: u64,
        vk: ash::Device,
        ready: vk::Semaphore,
        released: vk::Semaphore,
        /// the last value given to `ready` (and `released`)
        value: u64,
        /// Reflex in low-latency mode on the presenter's swapchain
        reflex: bool,
        /// this frame's token, from [`Presenter::begin`], and its value
        token: Option<(usize, u32)>,
        /// a Reflex or PCL call failed: said once, then they stop
        latency_failed: bool,
    }

    #[cfg(target_os = "linux")]
    impl Presenter {
        /// The presenter on `surface`, which no swapchain uses (renderer.rs
        /// drops the window's configured surface and makes it afresh), at
        /// `config`'s size and format. Err with the reason: frame
        /// generation then presents the prototype's way.
        pub(crate) fn new(
            device: &wgpu::Device,
            lost: &DeviceLost,
            surface: &wgpu::Surface<'_>,
            config: &wgpu::SurfaceConfiguration,
        ) -> Result<Self, crate::dlss_fg::FgRefusal> {
            use crate::dlss_fg::FgRefusal;
            let lib = sl::loaded().ok_or_else(|| FgRefusal::Failed(String::from("no sl-ngx")))?;
            let fns = lib.presenter.as_ref().ok_or(FgRefusal::OldSlNgx)?;
            let bits = presenter_device().ok_or(FgRefusal::NoQueue)?;
            Self::make(device, lost, surface, config, lib, fns, bits).map_err(FgRefusal::Failed)
        }

        fn make(
            device: &wgpu::Device,
            lost: &DeviceLost,
            surface: &wgpu::Surface<'_>,
            config: &wgpu::SurfaceConfiguration,
            lib: &'static sl::Lib,
            fns: &'static sl::present::Fns,
            bits: u32,
        ) -> Result<Self, String> {
            let format = match config.format {
                wgpu::TextureFormat::Bgra8Unorm => vk::Format::B8G8R8A8_UNORM,
                wgpu::TextureFormat::Bgra8UnormSrgb => vk::Format::B8G8R8A8_SRGB,
                wgpu::TextureFormat::Rgba8Unorm => vk::Format::R8G8B8A8_UNORM,
                wgpu::TextureFormat::Rgba8UnormSrgb => vk::Format::R8G8B8A8_SRGB,
                other => return Err(format!("the window's format {other:?} is not one the presenter is given")),
            };
            // SAFETY: only the handle is read, while `surface` lives (the
            // renderer destroys the presenter before it lets the surface go)
            let raw_surface = unsafe { surface.as_hal::<Vulkan>() }
                .and_then(|s| unsafe { s.raw_native_handle() })
                .ok_or("not a Vulkan surface")?;
            // SAFETY: the device's handle, cloned: it lives as long as
            // `device`, which this holds
            let vk_device = unsafe { device.as_hal::<Vulkan>() }
                .map(|hal| hal.raw_device().clone())
                .ok_or("not a Vulkan device")?;
            let timeline = || {
                let mut kind = vk::SemaphoreTypeCreateInfo::default()
                    .semaphore_type(vk::SemaphoreType::TIMELINE)
                    .initial_value(0);
                let info = vk::SemaphoreCreateInfo::default().push_next(&mut kind);
                // SAFETY: a plain timeline semaphore on the live device
                unsafe { vk_device.create_semaphore(&info, None) }.map_err(|e| format!("a timeline semaphore: {e}"))
            };
            let ready = timeline()?;
            let released = match timeline() {
                Ok(s) => s,
                Err(e) => {
                    // SAFETY: just made, never used
                    unsafe { vk_device.destroy_semaphore(ready, None) };
                    return Err(e);
                }
            };
            let desc = sl::present::Desc {
                version: sl::present::VERSION,
                surface: raw_surface,
                queue_family: 0,
                queue_index: 1,
                device_bits: bits,
                width: config.width,
                height: config.height,
                format,
                color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
                min_image_count: 0,
            };
            let reflex_wanted = bits & PRESENTER_LOW_LATENCY2 != 0;
            // SAFETY: under the lock, on the start running on this device;
            // `desc` lives through the call; the surface has no swapchain,
            // the queue (family 0, index 1) is used by nothing else, the
            // device was opened with what `bits` says
            let made = with_started(lib, device, lost, false, |st| unsafe {
                let mut raw = std::ptr::null_mut();
                let r = (fns.create)(&desc, &mut raw);
                if r != vk::Result::SUCCESS || raw.is_null() {
                    return Err(format!("slngxPresenterCreate: {r}"));
                }
                st.presenter = Some(raw as usize);
                st.presenter_pauses = 0;
                // Reflex in low-latency mode, without a frame limit: the
                // presenter's Fifo already holds the rendered frames to
                // half the refresh rate, and a limit there drops
                // generated frames (sl-ngx's cap test, 2026-11-27)
                let reflex = reflex_wanted
                    && lib
                        .latency()
                        .and_then(|f| {
                            let options = sl::ReflexOptions::new(sl::REFLEX_LOW_LATENCY);
                            sl::check((f.reflex_set_options)(&options)).map_err(|e| format!("slReflexSetOptions: {e}"))
                        })
                        .map_err(|e| log::error!("DLSS frame generation: Reflex stays off: {e}"))
                        .is_ok();
                Ok((raw as usize, st.start, reflex))
            });
            let (raw, start, reflex) = match made {
                Some(Ok(made)) => made,
                other => {
                    // SAFETY: never handed to the presenter
                    unsafe {
                        vk_device.destroy_semaphore(ready, None);
                        vk_device.destroy_semaphore(released, None);
                    }
                    return Err(match other {
                        Some(Err(e)) => e,
                        _ => String::from("DLSS is not started on this device"),
                    });
                }
            };
            log::info!(
                "DLSS frame generation: sl-ngx's presenter runs at {}x{}, {:?}{}",
                config.width,
                config.height,
                format,
                if reflex {
                    ", Reflex in low-latency mode"
                } else {
                    ", without Reflex"
                }
            );
            Ok(Self {
                raw,
                lib,
                device: device.clone(),
                lost: lost.clone(),
                start,
                vk: vk_device,
                ready,
                released,
                value: 0,
                reflex,
                token: None,
                latency_failed: false,
            })
        }

        /// `f` with the presenter's functions and handle, under the lock,
        /// while the start it was made on runs with it: None once that
        /// start ended it (slShutdown, another device), or the device is
        /// lost.
        fn call<T>(&self, f: impl FnOnce(&sl::present::Fns, *mut std::ffi::c_void) -> T) -> Option<T> {
            let fns = self.lib.presenter.as_ref()?;
            with_started(self.lib, &self.device, &self.lost, false, |st| {
                (st.start == self.start && st.presenter == Some(self.raw)).then(|| f(fns, self.raw as *mut std::ffi::c_void))
            })
            .flatten()
        }

        /// Reflex in low-latency mode on the presenter's swapchain.
        pub(crate) fn reflex(&self) -> bool {
            self.reflex && !self.latency_failed
        }

        /// The frame's start, before its input is read: its token, with
        /// `index` as its value, Reflex's sleep, and the PCL markers up to
        /// the submit's start. The token is the frame's for DLSS too
        /// ([`Presenter::token`]).
        pub(crate) fn begin(&mut self, index: u64) {
            self.token = None;
            let lib = self.lib;
            let frame = index as u32;
            let reflex = self.reflex();
            // SAFETY: under the lock, after slInit, on this presenter's
            // start; the token is the library's own, kept by it for some
            // frames
            let r = with_started(lib, &self.device, &self.lost, false, |st| unsafe {
                if st.start != self.start {
                    return None;
                }
                Some((|| {
                    let mut token: sl::FrameToken = std::ptr::null_mut();
                    sl::check((lib.new_frame_token)(&mut token, &frame)).map_err(|e| format!("slGetNewFrameToken: {e}"))?;
                    if reflex {
                        let f = lib.latency()?;
                        sl::check((f.reflex_sleep)(token)).map_err(|e| format!("slReflexSleep: {e}"))?;
                        for marker in [sl::pcl::SIMULATION_START, sl::pcl::SIMULATION_END, sl::pcl::RENDER_SUBMIT_START] {
                            sl::check((f.pcl_set_marker)(marker, token)).map_err(|e| format!("slPCLSetMarker({marker}): {e}"))?;
                        }
                    }
                    Ok::<_, String>(token as usize)
                })())
            })
            .flatten();
            match r {
                Some(Ok(token)) => self.token = Some((token, frame)),
                Some(Err(e)) => self.latency_error(&e),
                None => {}
            }
        }

        /// This frame's token, for DLSS's evaluate: the one Reflex's
        /// markers and the presenter's frame name.
        pub(crate) fn token(&self) -> Option<usize> {
            self.token.map(|(t, _)| t)
        }

        fn latency_error(&mut self, e: &str) {
            if !self.latency_failed {
                self.latency_failed = true;
                log::error!("DLSS frame generation: Reflex stops: {e}");
            }
        }

        /// A PCL marker for this frame's token.
        fn mark(&mut self, marker: u32) {
            let Some((token, _)) = self.token.filter(|_| self.reflex()) else {
                return;
            };
            let lib = self.lib;
            // SAFETY: as in `begin`
            let r = with_started(lib, &self.device, &self.lost, false, |st| unsafe {
                (st.start == self.start).then(|| {
                    lib.latency().and_then(|f| {
                        sl::check((f.pcl_set_marker)(marker, token as sl::FrameToken)).map_err(|e| format!("slPCLSetMarker({marker}): {e}"))
                    })
                })
            })
            .flatten();
            if let Some(Err(e)) = r {
                self.latency_error(&e);
            }
        }

        /// Waits until the presenter has read the frame handed over with
        /// `value` (its images may be written again), at most 1 s. False
        /// past that, or on a lost device.
        pub(crate) fn wait_released(&self, value: u64) -> bool {
            let semaphores = [self.released];
            let values = [value];
            let info = vk::SemaphoreWaitInfo::default().semaphores(&semaphores).values(&values);
            // SAFETY: the timeline lives as long as `self`
            unsafe { self.vk.wait_semaphores(&info, 1_000_000_000) }.is_ok()
        }

        /// Hands this frame over, once its commands are submitted:
        /// `generated` (none: the rendered frame alone) and `real`, both
        /// left by the frame in the transfer source layout. `ready` is
        /// signalled after everything submitted so far. Ok(Some(value)):
        /// taken, the images are the presenter's until `released` reaches
        /// `value` ([`Presenter::wait_released`]); Ok(None): not taken
        /// (the presenter was paused, or still had a frame waiting after
        /// `wait`), the images stay the viewer's. Err: the presenter is
        /// gone or failed, frame generation stops presenting through it.
        pub(crate) fn hand_over(
            &mut self,
            queue: &wgpu::Queue,
            generated: Option<(&wgpu::Texture, (u32, u32))>,
            real: (&wgpu::Texture, (u32, u32)),
            wait: std::time::Duration,
        ) -> Result<Option<u64>, String> {
            self.mark(sl::pcl::RENDER_SUBMIT_END);
            let image = |texture: &wgpu::Texture, (width, height): (u32, u32)| {
                // SAFETY: only the handle is read, while the texture lives
                // (the caller keeps it until the frame is released)
                unsafe { texture.as_hal::<Vulkan>() }.map(|t| sl::present::Image {
                    image: unsafe { t.raw_handle() },
                    layout: vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    width,
                    height,
                })
            };
            let none = sl::present::Image {
                image: vk::Image::null(),
                layout: vk::ImageLayout::UNDEFINED,
                width: 0,
                height: 0,
            };
            let real = image(real.0, real.1).ok_or("not a Vulkan texture")?;
            let generated = match generated {
                Some((texture, size)) => image(texture, size).ok_or("not a Vulkan texture")?,
                None => none,
            };
            self.value += 1;
            let value = self.value;
            // `ready` signalled by the next submission, which comes after
            // the frame's: an empty one, made now (another thread's
            // submission taking it first is later than the frame's too)
            // SAFETY: the timeline lives as long as `self`, and is waited
            // for in `Drop` before it goes
            match unsafe { queue.as_hal::<Vulkan>() } {
                Some(hal) => hal.add_signal_semaphore(self.ready, Some(value)),
                None => return Err(String::from("not a Vulkan queue")),
            }
            queue.submit(std::iter::empty());
            let frame = sl::present::Frame {
                version: sl::present::VERSION,
                frame: self.token.map_or(0, |(_, f)| f),
                generated,
                real,
                ready: self.ready,
                ready_value: value,
                released: self.released,
                released_value: value,
            };
            self.mark(sl::pcl::PRESENT_START);
            let wait_ns = u64::try_from(wait.as_nanos()).unwrap_or(u64::MAX);
            // SAFETY: under the lock, the presenter live on its start;
            // `frame` lives through the call, its images and timelines as
            // its contract asks (the caller's and `self`'s)
            let r = self.call(|fns, raw| unsafe { (fns.present)(raw, &frame, wait_ns) });
            self.mark(sl::pcl::PRESENT_END);
            self.token = None;
            match r {
                Some(vk::Result::SUCCESS) => Ok(Some(value)),
                Some(vk::Result::TIMEOUT | vk::Result::NOT_READY) => Ok(None),
                Some(r) => Err(format!("slngxPresent: {r}")),
                None => Err(String::from("the presenter was ended (Streamline shut down, or the device lost)")),
            }
        }

        /// The swapchain's size from the next frame on.
        pub(crate) fn resize(&self, width: u32, height: u32) {
            // SAFETY: under the lock, the presenter live on its start
            self.call(|fns, raw| unsafe { (fns.resize)(raw, width, height) });
        }

        /// Every frame handed over released, and nothing submitted by the
        /// presenter until the guard goes: before images handed over are
        /// let go ([`pause_presenter`] does the same for the device).
        pub(crate) fn pause(&self) -> PresenterPause {
            pause_presenter()
        }

        /// The presenter's numbers, None once it is gone.
        pub(crate) fn stats(&self) -> Option<sl::present::Stats> {
            let mut stats = sl::present::Stats {
                version: sl::present::VERSION,
                ..Default::default()
            };
            // SAFETY: under the lock, the presenter live on its start;
            // `stats` lives through the call
            let r = self.call(|fns, raw| unsafe { (fns.get_stats)(raw, &mut stats) })?;
            (r == vk::Result::SUCCESS).then_some(stats)
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for Presenter {
        fn drop(&mut self) {
            // the presenter destroyed first (its swapchain, before the
            // renderer lets the surface go), also on a lost device; then
            // the timelines, once the viewer's queue is through the last
            // signal of `ready`
            if let Some(mut held) = super::lock_within(&STARTED, END_WAIT) {
                if let Some(st) = held.as_mut().filter(|st| st.start == self.start && st.presenter == Some(self.raw)) {
                    st.presenter = None;
                    st.presenter_pauses = 0;
                    if let Some(fns) = self.lib.presenter.as_ref() {
                        // SAFETY: under the lock, the presenter live on
                        // its start, taken out of it above
                        unsafe { (fns.destroy)(self.raw as *mut std::ffi::c_void) };
                    }
                }
            } else {
                log::error!("DLSS frame generation: Streamline still busy: the presenter is left to it");
            }
            let semaphores = [self.ready];
            let values = [self.value];
            let info = vk::SemaphoreWaitInfo::default().semaphores(&semaphores).values(&values);
            // SAFETY: the timelines are this presenter's own; destroyed
            // only once no signal of theirs is pending (the presenter is
            // gone, the viewer's last `ready` reached)
            unsafe {
                if self.lost.get() || self.vk.wait_semaphores(&info, END_WAIT.as_nanos() as u64).is_err() {
                    log::warn!("DLSS frame generation: the presenter's timelines are left (the device did not finish)");
                    return;
                }
                self.vk.destroy_semaphore(self.ready, None);
                self.vk.destroy_semaphore(self.released, None);
            }
            log::info!("DLSS frame generation: sl-ngx's presenter stopped");
        }
    }

    /// Linux: sl-ngx's presenter held paused ([`pause_presenter`]),
    /// resumed when this goes.
    #[cfg(target_os = "linux")]
    pub(crate) struct PresenterPause(Option<(u64, usize)>);

    /// Linux: sl-ngx's presenter, if one runs, paused until the guard
    /// goes: its queue idle and every frame handed over released, nothing
    /// submitted meanwhile. Around anything that waits for the whole
    /// device (a wgpu surface configured or let go: vkDeviceWaitIdle,
    /// which no other queue may be used beside) and before images handed
    /// over are let go.
    #[cfg(target_os = "linux")]
    pub(crate) fn pause_presenter() -> PresenterPause {
        let Some(lib) = sl::loaded() else {
            return PresenterPause(None);
        };
        let Some(fns) = lib.presenter.as_ref() else {
            return PresenterPause(None);
        };
        let Some(mut held) = super::lock_within(&STARTED, END_WAIT) else {
            return PresenterPause(None);
        };
        let Some(st) = held.as_mut() else {
            return PresenterPause(None);
        };
        let Some(raw) = st.presenter else {
            return PresenterPause(None);
        };
        if st.presenter_pauses == 0 {
            // SAFETY: under the lock, the presenter live on this start
            let r = unsafe { (fns.pause)(raw as *mut std::ffi::c_void) };
            if r != vk::Result::SUCCESS {
                log::warn!("DLSS frame generation: slngxPresenterPause: {r}");
            }
        }
        st.presenter_pauses += 1;
        PresenterPause(Some((st.start, raw)))
    }

    #[cfg(target_os = "linux")]
    impl Drop for PresenterPause {
        fn drop(&mut self) {
            let Some((start, raw)) = self.0 else {
                return;
            };
            let (Some(lib), Some(mut held)) = (sl::loaded(), super::lock_within(&STARTED, END_WAIT)) else {
                return;
            };
            let Some(st) = held.as_mut().filter(|st| st.start == start && st.presenter == Some(raw)) else {
                return;
            };
            st.presenter_pauses = st.presenter_pauses.saturating_sub(1);
            if st.presenter_pauses == 0
                && let Some(fns) = lib.presenter.as_ref()
            {
                // SAFETY: under the lock, the presenter live on this start
                unsafe { (fns.resume)(raw as *mut std::ffi::c_void) };
            }
        }
    }

    /// Frees `viewport`'s feature, when the start that made it is still
    /// the one running and its device not lost (a feature of a lost device
    /// or of an abandoned start stays with it, [`abandon`]). `lost` is
    /// asked before the lock is taken, as in [`with_started`].
    fn release(lib: &sl::Lib, lost: &DeviceLost, start: u64, viewport: u32) {
        if lost.get() {
            return;
        }
        let mut held = started();
        let Some(st) = held.as_mut().filter(|st| st.start == start && !st.lost.get()) else {
            return;
        };
        let Some(at) = st.viewports.iter().position(|(v, _)| *v == viewport) else {
            return;
        };
        let (_, feature) = st.viewports.swap_remove(at);
        let handle = sl::ViewportHandle::new(viewport);
        // frame generation's feature on the viewport, if it made one (none
        // is freed as it should: sl-ngx frees what it has): the frames
        // that used it are done (`retire`)
        if fg_ready() {
            // SAFETY: as below
            if let Err(e) = sl::check(unsafe { (lib.free)(sl::FEATURE_DLSS_G, &handle) }) {
                log::error!("DLSS: freeing frame generation: {e}");
            }
        }
        // SAFETY: a viewport the running start made and has not freed (it
        // was still in its list), under the lock
        if let Err(e) = sl::check(unsafe { (lib.free)(feature, &handle) }) {
            log::error!("DLSS: freeing the feature: {e}");
        }
    }

    /// `sl::DLSSMode`
    #[derive(Clone, Copy, Debug, PartialEq)]
    #[repr(u32)]
    enum Mode {
        Performance = sl::mode::MAX_PERFORMANCE,
        Balanced = sl::mode::BALANCED,
        Quality = sl::mode::MAX_QUALITY,
        UltraPerformance = sl::mode::ULTRA_PERFORMANCE,
        Dlaa = sl::mode::DLAA,
    }

    impl Mode {
        /// The mode of a display size over a render size.
        fn of(ratio: f32) -> Mode {
            if ratio <= 1.0 {
                Mode::Dlaa
            } else if ratio <= 1.55 {
                Mode::Quality
            } else if ratio <= 1.85 {
                Mode::Balanced
            } else if ratio <= 2.5 {
                Mode::Performance
            } else {
                Mode::UltraPerformance
            }
        }
    }

    /// The render size Streamline picks for the mode of `ratio` at this display
    /// size: the guide (3.2.2) has the feature made with it "exactly as
    /// given", and Ultra Performance takes that one size alone. None when
    /// this device has no Streamline (FSR draws), is lost (`lost`, its renderer's)
    /// or Streamline gives no answer; an answer is kept per display size and
    /// mode, for the start of Streamline that gave it ([`super::Answers`]).
    pub(crate) fn optimal_size(device: &wgpu::Device, lost: &DeviceLost, display: (u32, u32), ratio: f32, rr: bool) -> Option<(u32, u32)> {
        type Key = ((u32, u32), i32, bool);
        static ASKED: Mutex<super::Answers<Key>> = Mutex::new(super::Answers::new());
        let mode = Mode::of(ratio);
        if mode == Mode::Dlaa {
            return Some(display);
        }
        // (before any lock: a device opened without Streamline, BUSY's after a
        // loss among them, never waits for DLSS's)
        if lost.get() || !opened().as_ref().is_some_and(|(_, d)| same(d, device)) {
            return None;
        }
        let key = (display, mode as i32, rr);
        // (Streamline's lock let go before ASKED's is taken: ASKED, then Streamline)
        let running = started().as_ref().filter(|st| same(&st.device, device)).map(|st| st.start);
        let mut asked = ASKED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(size) = asked.get(running, &key) {
            return Some(size);
        }
        let lib = sl::loaded()?;
        // (a start Streamline refused gives None here and is not asked again on
        // this device: with_started keeps the refusal)
        let (start, size) = with_started(lib, device, lost, true, |st| {
            // SAFETY: DLSS started on this device, under the lock
            (st.start, unsafe { sizes(lib, display, mode, rr) })
        })?;
        let size = size
            .map_err(|e| log::error!("DLSS: {e}"))
            .ok()
            .map(|[optimal, _, _]| optimal)
            .filter(|s| s.0 > 0 && s.1 > 0 && s.0 <= display.0 && s.1 <= display.1);
        log::info!("DLSS: the {mode:?} size for {}x{}: {size:?}", display.0, display.1);
        asked.put(start, key, size);
        size
    }

    /// What DLSS takes for `mode` at this display size: the render size it
    /// would pick, then the smallest and the largest. `rr`: Ray
    /// Reconstruction's, which takes its one size.
    ///
    /// # Safety
    /// Under the lock, with DLSS started.
    unsafe fn sizes(lib: &sl::Lib, display: (u32, u32), mode: Mode, rr: bool) -> Result<[(u32, u32); 3], String> {
        if rr {
            // SAFETY: the caller's contract
            let dlssd = unsafe { lib.dlssd() }?;
            let options = sl::DlssdOptions::new(mode as u32, display);
            let mut s = sl::dlssd_optimal_settings();
            // SAFETY: as sl_dlss_d.h declares it; both structs live through
            // the call
            sl::check(unsafe { (dlssd.optimal_settings)(&options, &mut s) }).map_err(|e| format!("slDLSSDGetOptimalSettings: {e}"))?;
            // RR takes no dynamic resolution: its one size
            return Ok([(s.optimal_render_width, s.optimal_render_height); 3]);
        }
        // SAFETY: the caller's contract
        let dlss = unsafe { lib.dlss() }?;
        let options = sl::DlssOptions::new(mode as u32, display);
        let mut s = sl::DlssOptimalSettings::new();
        // SAFETY: as sl_dlss.h declares it; both structs live through the
        // call
        sl::check(unsafe { (dlss.optimal_settings)(&options, &mut s) }).map_err(|e| format!("slDLSSGetOptimalSettings: {e}"))?;
        let optimal = (s.optimal_render_width, s.optimal_render_height);
        // the dynamic range defaults to the optimal size when none is
        // given, as NVIDIA's own helper does (older DLSS libraries may not
        // set it)
        let or_optimal = |size: (u32, u32)| if size.0 == 0 || size.1 == 0 { optimal } else { size };
        Ok([
            optimal,
            or_optimal((s.render_width_min, s.render_height_min)),
            or_optimal((s.render_width_max, s.render_height_max)),
        ])
    }

    /// The frame's camera as the library takes it; `None`, a neutral one
    /// for the feature's making (only its flags are read then).
    fn constants(f: Option<&Frame>, reset: bool) -> sl::Constants {
        let mut c = sl::Constants::new();
        c.depth_inverted = sl::TRUE;
        c.camera_motion_included = sl::TRUE;
        c.motion_vectors_3d = sl::FALSE;
        // the inputs' pass takes the jitter out of the vectors
        c.motion_vectors_jittered = sl::FALSE;
        c.orthographic_projection = sl::FALSE;
        c.motion_vectors_dilated = sl::FALSE;
        c.reset = sl::boolean(reset);
        // the vectors are in uv: one is the whole render size, which the
        // library multiplies them by
        c.mvec_scale = [1.0, 1.0];
        c.camera_pinhole_offset = [0.0, 0.0];
        let identity = glam::Mat4::IDENTITY.to_cols_array_2d();
        c.clip_to_lens_clip = identity;
        let Some(f) = f else {
            c.jitter_offset = [0.0, 0.0];
            c.camera_view_to_clip = identity;
            c.clip_to_camera_view = identity;
            c.clip_to_prev_clip = identity;
            c.prev_clip_to_clip = identity;
            c.camera_pos = [0.0; 3];
            c.camera_right = [1.0, 0.0, 0.0];
            c.camera_up = [0.0, 1.0, 0.0];
            c.camera_fwd = [0.0, 0.0, -1.0];
            (c.camera_near, c.camera_far, c.camera_fov, c.camera_aspect_ratio) = (0.1, 1000.0, 1.0, 1.0);
            return c;
        };
        let [sx, sy] = sign();
        c.jitter_offset = [f.jitter.x * sx, f.jitter.y * sy];
        // The header's matrices are rows that multiply a row vector: the
        // transpose of glam's, whose columns are those rows as they are.
        c.camera_view_to_clip = f.proj.to_cols_array_2d();
        c.clip_to_camera_view = f.proj.inverse().to_cols_array_2d();
        c.clip_to_prev_clip = f.motion_reproj;
        c.prev_clip_to_clip = glam::Mat4::from_cols_array_2d(&f.motion_reproj)
            .as_dmat4()
            .inverse()
            .as_mat4()
            .to_cols_array_2d();
        let world = f.view.inverse();
        c.camera_pos = f.camera_pos.to_array();
        c.camera_right = world.x_axis.truncate().normalize_or_zero().to_array();
        c.camera_up = world.y_axis.truncate().normalize_or_zero().to_array();
        c.camera_fwd = (-world.z_axis.truncate()).normalize_or_zero().to_array();
        c.camera_near = f.near;
        c.camera_far = f.far;
        c.camera_fov = f.fov_y;
        c.camera_aspect_ratio = f.proj.y_axis.y / f.proj.x_axis.x;
        c
    }

    #[repr(C)]
    #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    struct MotionU {
        reproj: [[f32; 4]; 4],
        jitter_uv: [f32; 2],
        motion_on: f32,
        pad: f32,
    }

    /// What a feature is made with, kept to make it again.
    #[derive(Clone, Copy, PartialEq)]
    struct Shape {
        /// `sl::FEATURE_DLSS`, or `FEATURE_DLSS_RR`
        feature: u32,
        mode: Mode,
        render: (u32, u32),
        display: (u32, u32),
        /// DLSS's own exposure (`AURORA_DLSS_AUTOEXP=0` drops it)
        auto_exposure: bool,
        /// DLSS upscales the alpha too (`AURORA_DLSS_ALPHA=1`)
        alpha: bool,
        /// `sl::DLSSPreset`, 0 for NVIDIA's own (and for Ray
        /// Reconstruction, which does not read it)
        preset: u32,
        /// Ray Reconstruction's, 0 for NVIDIA's own (and for super
        /// resolution)
        rr_preset: u32,
    }

    /// A DLSS feature: the viewport it was made for, the start that made
    /// it, and the render sizes DLSS takes.
    struct Made {
        viewport: u32,
        start: u64,
        lo: (u32, u32),
        hi: (u32, u32),
    }

    /// Makes the DLSS feature of `shape`, starting DLSS on `device` when
    /// it is not started there. `report`: the main view's, whose failure
    /// or success is what Preferences shows ([`failed`]).
    fn create(lib: &sl::Lib, device: &wgpu::Device, lost: &DeviceLost, queue: &wgpu::Queue, shape: &Shape, report: bool) -> Option<Made> {
        let Shape {
            feature: kind,
            mode,
            render,
            display,
            auto_exposure,
            alpha,
            preset,
            rr_preset,
        } = *shape;
        let rr = kind == sl::FEATURE_DLSS_RR;
        // Debug: AURORA_DLSS_RR_FAIL=1 has Ray Reconstruction refused here,
        // to see super resolution draw in its place
        if rr && is("AURORA_DLSS_RR_FAIL", "1") {
            rr_refused("refused for a test (AURORA_DLSS_RR_FAIL=1)", report);
            return None;
        }
        let begun = std::time::Instant::now();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("DLSS feature"),
        });
        // SAFETY: under the lock, with DLSS started on this very device
        // (with_started); every struct lives through the call it is passed to;
        // the command buffer is the encoder's open one, submitted below
        // once the library recorded into it; the viewport's feature is the
        // library's until freed
        let made = with_started(lib, device, lost, true, |st| unsafe {
            let [_, mut lo, mut hi] = sizes(lib, display, mode, rr)?;
            if rr {
                // one size, the one the frames are drawn at
                (lo, hi) = (render, render);
            }
            if mode == Mode::Dlaa {
                (lo, hi) = (display, display);
            }
            if render.0 < lo.0 || render.1 < lo.1 || render.0 > hi.0 || render.1 > hi.1 {
                return Err(format!(
                    "render size {}x{} outside {mode:?}'s {}x{}..{}x{}",
                    render.0, render.1, lo.0, lo.1, hi.0, hi.1
                ));
            }
            let viewport = VIEWPORTS.fetch_add(1, Ordering::Relaxed);
            let handle = sl::ViewportHandle::new(viewport);
            // the library keeps the viewport from the options on, so every
            // failure after them frees it
            let made = (|| {
                // the model, read when the feature is made: one per mode,
                // as DLSS picks the one of the mode it runs
                if rr {
                    let dlssd = lib.dlssd()?;
                    let mut options = sl::DlssdOptions::new(mode as u32, display);
                    options.preset(rr_preset);
                    options.alpha_upscaling_enabled = sl::boolean(alpha);
                    sl::check((dlssd.set_options)(&handle, &options)).map_err(|e| format!("slDLSSDSetOptions: {e}"))?;
                } else {
                    let dlss = lib.dlss()?;
                    let mut options = sl::DlssOptions::new(mode as u32, display);
                    options.preset(preset);
                    options.use_auto_exposure = sl::boolean(auto_exposure);
                    options.alpha_upscaling_enabled = sl::boolean(alpha);
                    sl::check((dlss.set_options)(&handle, &options)).map_err(|e| format!("slDLSSSetOptions: {e}"))?;
                }
                let mut token: sl::FrameToken = std::ptr::null_mut();
                sl::check((lib.new_frame_token)(&mut token, std::ptr::null())).map_err(|e| format!("slGetNewFrameToken: {e}"))?;
                sl::check((lib.set_constants)(&constants(None, true), token, &handle)).map_err(|e| format!("slSetConstants: {e}"))?;
                // NVIDIA's Streamline (Windows) makes the feature only from
                // tagged inputs (sl.dlss's dlssBeginEvent), which exist at
                // the first evaluate, not here: slAllocateResources without
                // them leaves no feature and answers eErrorInvalidParameter.
                // The feature is then made by that first evaluate.
                let r = if cfg!(windows) {
                    0
                } else {
                    encoder.as_hal_mut::<Vulkan, _, _>(|e| match e {
                        Some(e) => (lib.allocate)(e.raw_handle(), kind, &handle),
                        None => sl::ERROR_INVALID_PARAMETER,
                    })
                };
                let what = if rr { "Ray Reconstruction" } else { "DLSS" };
                sl::check(r).map_err(|e| format!("the {what} feature for {}x{}: {e}", display.0, display.1))
            })();
            if let Err(e) = made {
                let _ = (lib.free)(kind, &handle);
                return Err(e);
            }
            st.viewports.push((viewport, kind));
            Ok(Made {
                viewport,
                start: st.start,
                lo,
                hi,
            })
        })?;
        match made {
            Ok(made) => {
                // outside the lock: a submit may run the callbacks that
                // free a feature (Dlss::retire), under that lock; noted for
                // the line of a device loss like the frame's (the feature's
                // creation work is a GPU job of its own)
                crate::device_lost::submit_tracked(lost, queue, lost.last_frame(), [encoder.finish()]);
                working(report);
                log::info!(
                    "DLSS: {}feature made in {:.1} ms",
                    if rr { "Ray Reconstruction " } else { "" },
                    begun.elapsed().as_secs_f64() * 1000.0
                );
                Some(made)
            }
            Err(e) if rr => {
                rr_refused(&e, report);
                None
            }
            Err(e) => {
                failed(&e, report);
                None
            }
        }
    }

    /// Debug: `AURORA_DLSS_FAIL=<n>[,<n>]` fails those evaluates, counted
    /// from the program's first, to see DLSS made again and then given up.
    fn fails(evaluate: u64) -> bool {
        static AT: std::sync::OnceLock<Vec<u64>> = std::sync::OnceLock::new();
        AT.get_or_init(|| {
            let list = std::env::var("AURORA_DLSS_FAIL").unwrap_or_default();
            list.split(',').filter_map(|n| n.trim().parse().ok()).collect()
        })
        .contains(&evaluate)
    }

    /// Debug: `AURORA_DLSS_RR_TRANSPOSED=1` hands Ray Reconstruction its
    /// matrices transposed ([`Dlss::evaluate`]), read once.
    fn rr_transposed() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| is("AURORA_DLSS_RR_TRANSPOSED", "1"))
    }

    /// How long a new display size must hold before its feature is made.
    const SETTLE: std::time::Duration = std::time::Duration::from_millis(250);

    /// Debug: `AURORA_DLSS_SETTLE=0` makes the feature at every size of a
    /// resize, as before, to time what waiting for the size saves.
    fn settles() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| !std::env::var("AURORA_DLSS_SETTLE").is_ok_and(|v| v.trim() == "0"))
    }

    /// A window being resized has a new size at every frame, and a feature
    /// takes ~35 ms to make (measured: 1.2 s over 30 sizes): asked before
    /// [`Dlss::new`], for a display size that is not the renderer's last
    /// (`last`, each renderer its own: a second camera's window is resized
    /// too; a snapshot's sizes are not asked, they would read as a resize
    /// at every frame). Some(when to try again) while the size moves:
    /// nothing of DLSS is made meanwhile (no textures, no pipelines, no
    /// feature), FSR draws, and the renderer makes DLSS once the size has
    /// held that long.
    pub(crate) fn resizing(display: (u32, u32), last: &std::cell::Cell<Option<(u32, u32)>>) -> Option<std::time::Instant> {
        let moved = last.get().is_some_and(|d| d != display) && settles();
        last.set(Some(display));
        moved.then(|| {
            log::info!("DLSS: the size changed to {}x{}, FSR draws until it holds", display.0, display.1);
            std::time::Instant::now() + SETTLE
        })
    }

    fn active(shape: &Shape, made: &Made) {
        let (Shape { mode, render, display, .. }, Made { lo, hi, .. }) = (shape, made);
        log::info!(
            "DLSS: active, {mode:?} from {}x{} to {}x{} (Streamline takes {}x{}..{}x{})",
            render.0,
            render.1,
            display.0,
            display.1,
            lo.0,
            lo.1,
            hi.0,
            hi.1
        );
    }

    /// Evaluates of every feature since the program started.
    static EVALUATES: AtomicU64 = AtomicU64::new(0);

    #[repr(C)]
    #[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
    struct GuidesU {
        inv_view_proj: [[f32; 4]; 4],
        eye: [f32; 4],
    }

    /// Ray Reconstruction's guides (`RenderSettings::rr_guides`), at
    /// the render size, drawn from the G-buffer (shaders/dlss_guides.wgsl).
    struct Rr {
        diffuse: wgpu::TextureView,
        specular: wgpu::TextureView,
        normals: wgpu::TextureView,
        pipeline: wgpu::RenderPipeline,
        group: wgpu::BindGroup,
        buffer: wgpu::Buffer,
        /// Debug, `AURORA_DLSS_RR_SHOW=diffuse|specular|normals|roughness`:
        /// that guide drawn over DLSS's image
        show: Option<(wgpu::RenderPipeline, wgpu::BindGroup)>,
        /// `show` reads the renderer's base colour, made again with the
        /// targets ([`Dlss::rebind`])
        shows_albedo: bool,
    }

    pub(crate) struct Dlss {
        lib: &'static sl::Lib,
        /// the viewport the feature was made for (one per feature)
        viewport: u32,
        /// the start of DLSS that made it ([`STARTS`])
        start: u64,
        /// the feature exists; not after a failure retired it (nor while
        /// `remodel` makes it again)
        made: bool,
        shape: Shape,
        device: wgpu::Device,
        queue: wgpu::Queue,
        adapter: wgpu::Adapter,
        /// the scene, for its alpha
        color: wgpu::TextureView,
        /// the scene's colour made safe, what DLSS reads
        safe: wgpu::TextureView,
        depth: wgpu::TextureView,
        motion: wgpu::TextureView,
        output: wgpu::TextureView,
        pipeline: wgpu::RenderPipeline,
        group: wgpu::BindGroup,
        /// the pass that writes the scene's alpha over DLSS's; None when
        /// Streamline upscales it (`AURORA_DLSS_ALPHA=1`)
        alpha: Option<(wgpu::RenderPipeline, wgpu::BindGroup)>,
        buffer: wgpu::Buffer,
        /// frames DLSS drew since it was made
        drawn: u64,
        /// the next frame starts DLSS's history afresh
        reset: bool,
        /// the feature was made again after a failure: the next one ends DLSS
        remade: bool,
        failed: bool,
        /// the main view's: its failures are what Preferences shows
        /// ([`failed`])
        report: bool,
        /// made for a snapshot's targets ([`Dlss::new`]), or kept for them
        /// since ([`Dlss::serving`])
        snapshot: bool,
        /// its renderer's view of the device's loss: nothing of Streamline is
        /// called once it is set ([`with_started`])
        lost: DeviceLost,
        /// the feature is Ray Reconstruction's, with its guides
        rr: Option<Rr>,
        /// this frame's token and constants, taken by frame generation
        /// ([`Dlss::generate`])
        fg_frame: Option<(usize, sl::Constants)>,
        /// Windows' frame generation, made at the present ([`Dlss::fg_begin`])
        #[cfg(windows)]
        fg_win: FgWin,
        /// Linux: this frame's token from sl-ngx's presenter
        /// ([`Presenter::begin`]), the one Reflex's markers name
        #[cfg(target_os = "linux")]
        frame_token: Option<usize>,
    }

    /// Windows' frame generation, for the main window's DLSS: this
    /// frame's token from [`Dlss::fg_begin`] to the present's end marker,
    /// and what the log says of it.
    #[cfg(windows)]
    #[derive(Default)]
    struct FgWin {
        /// the frame's token (its address), while the frame runs
        token: Option<usize>,
        /// the options asked for the frame's present: frames generated
        /// per frame drawn, 0 for off
        asked: u32,
        /// presents with frame generation asked, and the state's last
        /// status, for the log
        presents: u64,
        status: Option<u32>,
        /// frames presented per frame drawn, as the last state read said
        /// (`numFramesActuallyPresented`), for Preferences
        per_drawn: u32,
        /// frames the GPU can generate per frame drawn
        /// (`numFramesToGenerateMax`) and the smallest side the window
        /// may have (`minWidthOrHeight`), read before the first ask
        /// (`state_read`)
        max: u32,
        min: u32,
        state_read: bool,
        /// the depth and the motion this frame's DLSS read, copied as it
        /// read them ([`Dlss::fg_copy_inputs`]): frame generation reads
        /// them at the present, after later passes have changed the
        /// originals' layouts
        inputs: Option<[wgpu::TextureView; 2]>,
        /// the video memory refused those copies: off for this DLSS
        inputs_refused: bool,
        /// the window's size at the last present asked on, and the size a
        /// status other than OK was read at: off there, until the window
        /// takes another size (DLSS-G guide §14.3: on in an invalid state,
        /// it paints the frame pink)
        size: Option<(u32, u32)>,
        bad: Option<(u32, u32)>,
        /// presents in a row read with a status other than OK
        bad_run: u32,
        /// asked on once for this viewport: asked off, without its memory
        /// kept, as the feature goes ([`Dlss::retire`])
        used: bool,
        /// a failure was logged (one line, not one a frame)
        failed: bool,
    }

    /// `view` as the library takes it, in the layout wgpu-hal leaves it
    /// in: `storage` for the one DLSS writes, else read by a shader.
    fn resource(view: &wgpu::TextureView, adapter: &wgpu::Adapter, storage: bool) -> Option<sl::Resource> {
        let texture = view.texture();
        // One guard at a time, each dropped at the end of its statement: a
        // texture's or a view's guard holds the device's snatch lock for
        // reading (wgpu-core as_hal.rs, `SnatchableResourceGuard::new`) and
        // forgets its lock trace, so a second one alive beside it panics
        // under debug assertions ("attempted to acquire a snatch lock
        // recursively"), and without them is a recursive read that can wait
        // for ever behind a writer queued in between.
        // SAFETY: the raw handle is only read, for the library to use while
        // `view` lives (the Dlss holding it outlives the evaluate, and
        // retire keeps it until its frames are done)
        let view_raw = unsafe { view.as_hal::<Vulkan>()?.raw_handle() };
        // SAFETY: as above, for the view's texture
        let image_raw = unsafe { texture.as_hal::<Vulkan>()?.raw_handle() };
        // SAFETY: only a format lookup on the adapter (not a snatchable
        // resource: its guard takes no lock)
        let format = unsafe { adapter.as_hal::<Vulkan>()?.texture_format_as_raw(texture.format()) };
        let color = texture.format().has_color_aspect();
        // (wgpu-hal's layouts: conv.rs, `derive_image_layout`)
        let layout = if storage {
            vk::ImageLayout::GENERAL
        } else if color {
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
        } else {
            vk::ImageLayout::DEPTH_STENCIL_READ_ONLY_OPTIMAL
        };
        let mut usage = vk::ImageUsageFlags::empty();
        for (ours, theirs) in [
            (wgpu::TextureUsages::COPY_SRC, vk::ImageUsageFlags::TRANSFER_SRC),
            (wgpu::TextureUsages::COPY_DST, vk::ImageUsageFlags::TRANSFER_DST),
            (wgpu::TextureUsages::TEXTURE_BINDING, vk::ImageUsageFlags::SAMPLED),
            (wgpu::TextureUsages::STORAGE_BINDING, vk::ImageUsageFlags::STORAGE),
            (
                wgpu::TextureUsages::RENDER_ATTACHMENT,
                if color {
                    vk::ImageUsageFlags::COLOR_ATTACHMENT
                } else {
                    vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT
                },
            ),
        ] {
            if texture.usage().contains(ours) {
                usage |= theirs;
            }
        }
        Some(sl::Resource::image(
            image_raw,
            view_raw,
            layout,
            format,
            usage,
            texture.width(),
            texture.height(),
        ))
    }

    /// The feature `render` to `display` with the models would be made
    /// with: super resolution, or Ray Reconstruction when `rr` (its guides
    /// drawn, [`guides`]), its mode, its options and the models' presets.
    fn shape(render: (u32, u32), display: (u32, u32), (model, rr_model): (crate::DlssModel, crate::DlssRrModel), rr: bool) -> Shape {
        let mode = if render == display {
            Mode::Dlaa
        } else {
            Mode::of(display.0 as f32 / render.0.max(1) as f32)
        };
        // Debug: AURORA_DLSS_PRESET=<n> sets NVIDIA's preset number
        // whatever the setting (11 K, 12 L, 13 M; 0 NVIDIA's own)
        // (0 for Ray Reconstruction, which reads its own: a change of the
        // super resolution model then leaves its shape, and its feature, as
        // they are)
        let preset = std::env::var("AURORA_DLSS_PRESET")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(model.preset())
            * u32::from(!rr);
        // Debug: AURORA_DLSS_RR_PRESET=<n> likewise for Ray
        // Reconstruction (4 D, 5 E, 6 F; 0 NVIDIA's own)
        // (0 for super resolution, which never reads it: a change of the Ray
        // Reconstruction model then leaves its shape, and its feature, as
        // they are)
        let rr_preset = std::env::var("AURORA_DLSS_RR_PRESET")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(rr_model.preset())
            * u32::from(rr);
        Shape {
            feature: if rr { sl::FEATURE_DLSS_RR } else { sl::FEATURE_DLSS },
            mode,
            render,
            display,
            // RR takes no automatic exposure (NVIDIA's RR guide)
            auto_exposure: !is("AURORA_DLSS_AUTOEXP", "0") && !rr,
            alpha: is("AURORA_DLSS_ALPHA", "1"),
            preset,
            rr_preset,
        }
    }

    /// The G-buffer and base colour Ray Reconstruction's guides are drawn
    /// from: the renderer drew them and Streamline takes RR on `device`.
    fn guides<'a>(device: &wgpu::Device, inputs: &Inputs<'a>) -> Option<(&'a wgpu::TextureView, &'a wgpu::TextureView)> {
        match (inputs.gbuf, inputs.albedo) {
            (Some(g), Some(a)) if rr_ready_on(device) => Some((g, a)),
            _ => None,
        }
    }

    /// The guides' pass reads the depth, the G-buffer and the base colour,
    /// and the motion and the colour for the blended faces the renderer
    /// marked there (renderer.rs `RrInputs`).
    fn guides_group(
        device: &wgpu::Device,
        pipeline: &wgpu::RenderPipeline,
        buffer: &wgpu::Buffer,
        inputs: &Inputs,
        gbuf: &wgpu::TextureView,
        albedo: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let view = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("DLSS guides"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                view(0, inputs.depth),
                view(1, gbuf),
                view(2, albedo),
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffer.as_entire_binding(),
                },
                view(5, inputs.motion),
                view(6, inputs.color),
            ],
        })
    }

    /// The debug view of one guide (`AURORA_DLSS_RR_SHOW`).
    fn show_group(device: &wgpu::Device, pipeline: &wgpu::RenderPipeline, tex: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("DLSS RR show"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(tex),
            }],
        })
    }

    /// The variable `name` holds `value`, spaces aside.
    fn is(name: &str, value: &str) -> bool {
        std::env::var(name).is_ok_and(|v| v.trim() == value)
    }

    /// The inputs' pass's bind group: what it reads of the scene.
    fn inputs_group(device: &wgpu::Device, pipeline: &wgpu::RenderPipeline, buffer: &wgpu::Buffer, inputs: &Inputs) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("DLSS inputs"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(inputs.depth),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(inputs.motion),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(inputs.color),
                },
            ],
        })
    }

    /// The alpha pass's bind group: the scene, for its alpha.
    fn alpha_group(device: &wgpu::Device, pipeline: &wgpu::RenderPipeline, color: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("DLSS alpha"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(color),
            }],
        })
    }

    impl Dlss {
        /// `snapshot`: the targets of a photo or film frame, whose
        /// failures do not change what Preferences shows. `report`: the
        /// main view's, whose failures Preferences shows ([`failed`]); an
        /// external renderer's (the second camera's, an eye of VR) falls
        /// back to FSR alone, with the reason in the log. `lost`: the
        /// renderer's view of the device's loss. Made with its feature or
        /// not at all: a resize asks [`resizing`] first, and nothing is
        /// made while the size moves.
        pub fn new(
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            lost: &DeviceLost,
            render: (u32, u32),
            display: (u32, u32),
            (model, rr_model): (crate::DlssModel, crate::DlssRrModel),
            inputs: &Inputs,
            snapshot: bool,
            report: bool,
        ) -> Option<Self> {
            // the caller asked `wanted`
            let report = report && !snapshot;
            if lost.get() {
                return None;
            }
            // the adapter of this very device: a renderer made after a
            // device loss has another
            let adapter = opened().as_ref().filter(|(_, d)| same(d, device)).map(|(a, _)| a.clone());
            let Some(adapter) = adapter else {
                log::info!("DLSS: wanted, but the device has no DLSS extensions: FSR draws");
                return None;
            };
            let lib = sl::loaded()?;
            // The scene's alpha is the glow amount, per frame: DLSS would
            // keep it in its history and trail it, so a pass of our own
            // copies it as FSR does (encode_alpha). Debug:
            // AURORA_DLSS_ALPHA=1 has Streamline upscale it instead, =0 leaves
            // whatever DLSS wrote (the reference of the fault),
            // AURORA_DLSS_AUTOEXP=0 drops the automatic exposure.
            let ngx_alpha = is("AURORA_DLSS_ALPHA", "1");
            // Ray Reconstruction: the renderer drew its guides
            // (`RenderSettings::rr_guides`)
            let rr_inputs = guides(device, inputs);
            let texture = |label, size: (u32, u32), format, usage| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width: size.0,
                            height: size.1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage,
                        view_formats: &[],
                    })
                    .create_view(&Default::default())
            };
            // DLSS's own textures, checked apart from the render targets
            // they are made with: refused, FSR draws and the targets stand
            let drawn_and_read = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
            let ((motion, safe, output), refused) = crate::gpu_alloc::scoped(device, || {
                (
                    // (Windows: copied for frame generation, `fg_copy_inputs`)
                    texture(
                        "DLSS motion vectors",
                        render,
                        MOTION_FORMAT,
                        if cfg!(windows) {
                            drawn_and_read | wgpu::TextureUsages::COPY_SRC
                        } else {
                            drawn_and_read
                        },
                    ),
                    texture("DLSS colour", render, OUTPUT_FORMAT, drawn_and_read),
                    texture(
                        "DLSS output",
                        display,
                        OUTPUT_FORMAT,
                        wgpu::TextureUsages::STORAGE_BINDING | drawn_and_read,
                    ),
                )
            });
            if refused {
                failed("the video memory refused its textures", report);
                return None;
            }
            // Ray Reconstruction's guides, their textures, pipelines and
            // bind groups checked apart: refused, Ray Reconstruction is
            // refused (`rr_refused`), not DLSS. FSR draws until the targets
            // are made again for super resolution, at its own size
            // (`render` is Ray Reconstruction's, which super resolution may
            // not take). A snapshot's draws super resolution at once
            // instead (`snapshot_refusal`)
            let mut rr = match rr_inputs {
                Some((gbuf, albedo)) => {
                    let (rr, refused) = crate::gpu_alloc::scoped(device, || {
                        let diffuse = texture("DLSS RR diffuse albedo", render, GUIDE_FORMAT, drawn_and_read);
                        let specular = texture("DLSS RR specular albedo", render, GUIDE_FORMAT, drawn_and_read);
                        let normals = texture("DLSS RR normals", render, NORMAL_FORMAT, drawn_and_read);
                        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                            label: Some("DLSS guides"),
                            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/dlss_guides.wgsl").into()),
                        });
                        let make = |label, entry_point, constants: &[(&str, f64)], targets: &[Option<wgpu::ColorTargetState>]| {
                            use crate::pipeline_cache::CachedPipelines as _;
                            device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
                                label: Some(label),
                                layout: None,
                                vertex: wgpu::VertexState {
                                    module: &module,
                                    entry_point: Some("vs_main"),
                                    compilation_options: Default::default(),
                                    buffers: &[],
                                },
                                fragment: Some(wgpu::FragmentState {
                                    module: &module,
                                    entry_point: Some(entry_point),
                                    compilation_options: wgpu::PipelineCompilationOptions {
                                        constants,
                                        ..Default::default()
                                    },
                                    targets,
                                }),
                                primitive: Default::default(),
                                depth_stencil: None,
                                multisample: Default::default(),
                                multiview_mask: None,
                                cache: None,
                            })
                        };
                        let pipeline = make(
                            "DLSS guides",
                            "fs_main",
                            &[],
                            &[Some(GUIDE_FORMAT.into()), Some(GUIDE_FORMAT.into()), Some(NORMAL_FORMAT.into())],
                        );
                        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("DLSS guides"),
                            size: std::mem::size_of::<GuidesU>() as u64,
                            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        });
                        let group = guides_group(device, &pipeline, &buffer, inputs, gbuf, albedo);
                        let shown = std::env::var("AURORA_DLSS_RR_SHOW").ok();
                        let show = shown.as_deref().map(str::trim).and_then(|name| {
                            let (tex, mode) = match name {
                                "diffuse" => (&diffuse, 0.0),
                                "specular" => (&specular, 0.0),
                                "normals" => (&normals, 1.0),
                                "roughness" => (&normals, 2.0),
                                "albedo" => (albedo, 0.0),
                                _ => return None,
                            };
                            let scale = std::env::var("AURORA_DLSS_RR_SHOW_SCALE")
                                .ok()
                                .and_then(|v| v.trim().parse::<f64>().ok())
                                .unwrap_or(0.1);
                            let pipeline = make(
                                "DLSS RR show",
                                "fs_show",
                                &[("show_mode", mode), ("show_scale", scale)],
                                &[Some(OUTPUT_FORMAT.into())],
                            );
                            let group = show_group(device, &pipeline, tex);
                            log::info!("DLSS RR: showing the {name} guide");
                            Some((pipeline, group))
                        });
                        let shows_albedo = show.is_some() && shown.as_deref().map(str::trim) == Some("albedo");
                        Rr {
                            diffuse,
                            specular,
                            normals,
                            pipeline,
                            group,
                            buffer,
                            show,
                            shows_albedo,
                        }
                    });
                    match refused {
                        false => Some(rr),
                        true if snapshot => {
                            snapshot_refusal("its guides' textures or objects were refused");
                            None
                        }
                        true => {
                            rr_refused("its guides' textures or objects were refused", report);
                            return None;
                        }
                    }
                }
                None => None,
            };
            let mut shape = shape(render, display, (model, rr_model), rr.is_some());
            if rr.is_some() {
                log::info!(
                    "DLSS: Ray Reconstruction, alpha upscaled {}, preset {}",
                    shape.alpha,
                    shape.rr_preset
                );
            } else {
                log::info!(
                    "DLSS: automatic exposure {}, alpha upscaled {}, preset {}",
                    shape.auto_exposure,
                    shape.alpha,
                    shape.preset
                );
            }
            let made = match create(lib, device, lost, queue, &shape, report) {
                Some(made) => made,
                // a photo or film frame whose Ray Reconstruction Streamline
                // refused: super resolution at once, from the same targets
                // (a snapshot's is DLAA, whose size both take), and for
                // every capture from here, the window's Ray Reconstruction
                // left as it is; otherwise an offline film would ask
                // Streamline again at every frame, each drawn by FSR
                None if snapshot && rr.is_some() && !lost.get() => {
                    snapshot_refusal("Streamline refused its feature");
                    rr = None;
                    shape = self::shape(render, display, (model, rr_model), false);
                    create(lib, device, lost, queue, &shape, report)?
                }
                None => return None,
            };
            // Only Streamline writes the output, in commands wgpu does not see, so
            // wgpu would count it unwritten and clear it before its first
            // use, after DLSS drew into it: one black frame per feature.
            // A clear wgpu tracks, once, marks it written.
            {
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("DLSS output clear"),
                });
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("DLSS output clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &output,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                // (noted for the line of a device loss, device_lost.rs)
                crate::device_lost::submit_tracked(lost, queue, lost.last_frame(), [encoder.finish()]);
            }
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("DLSS inputs"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/dlss_motion.wgsl").into()),
            });
            let pass = |label, entry_point, targets: &[Option<wgpu::ColorTargetState>]| {
                use crate::pipeline_cache::CachedPipelines as _;
                device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(entry_point),
                        compilation_options: Default::default(),
                        targets,
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                })
            };
            let pipeline = pass("DLSS inputs", "fs_main", &[Some(MOTION_FORMAT.into()), Some(OUTPUT_FORMAT.into())]);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("DLSS inputs"),
                size: std::mem::size_of::<MotionU>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let group = inputs_group(device, &pipeline, &buffer, inputs);
            let alpha = (!ngx_alpha && !is("AURORA_DLSS_ALPHA", "0")).then(|| {
                let pipeline = pass(
                    "DLSS alpha",
                    "fs_alpha",
                    &[Some(wgpu::ColorTargetState {
                        format: OUTPUT_FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALPHA,
                    })],
                );
                let group = alpha_group(device, &pipeline, inputs.color);
                (pipeline, group)
            });
            active(&shape, &made);
            Some(Self {
                lib,
                viewport: made.viewport,
                start: made.start,
                made: true,
                shape,
                device: device.clone(),
                queue: queue.clone(),
                adapter,
                color: inputs.color.clone(),
                safe,
                depth: inputs.depth.clone(),
                motion,
                output,
                pipeline,
                group,
                alpha,
                buffer,
                drawn: 0,
                reset: true,
                remade: false,
                failed: false,
                report,
                snapshot,
                lost: lost.clone(),
                rr,
                fg_frame: None,
                #[cfg(windows)]
                fg_win: FgWin::default(),
                #[cfg(target_os = "linux")]
                frame_token: None,
            })
        }

        /// The upscaled image, HDR, at the display size.
        pub fn output(&self) -> &wgpu::TextureView {
            &self.output
        }

        /// This DLSS would be made again as it is from `inputs`: same
        /// device, sizes, models and kind (Ray Reconstruction's guides or
        /// none), and not failed. The feature keys on those alone
        /// (Streamline takes the textures at each evaluate), so the targets'
        /// remake keeps it ([`Dlss::rebind`]) instead of making it again
        /// (~35 ms).
        pub fn fits(
            &self,
            device: &wgpu::Device,
            render: (u32, u32),
            display: (u32, u32),
            models: (crate::DlssModel, crate::DlssRrModel),
            inputs: &Inputs,
        ) -> bool {
            !self.failed && same(&self.device, device) && self.shape == shape(render, display, models, guides(device, inputs).is_some())
        }

        /// Reads the scene from `inputs` (the targets made again at the same
        /// size), keeping the feature and DLSS's own textures. The views
        /// let go are held until the frames already submitted are done: the
        /// last evaluate reads the depth through Streamline, which wgpu does not
        /// see.
        pub fn rebind(&mut self, inputs: &Inputs) {
            self.group = inputs_group(&self.device, &self.pipeline, &self.buffer, inputs);
            if let Some((pipeline, group)) = &mut self.alpha {
                *group = alpha_group(&self.device, pipeline, inputs.color);
            }
            // `fits` kept the kind: guides drawn exactly when RR is
            if let (Some(rr), Some((gbuf, albedo))) = (&mut self.rr, guides(&self.device, inputs)) {
                rr.group = guides_group(&self.device, &rr.pipeline, &rr.buffer, inputs, gbuf, albedo);
                if let (Some((pipeline, group)), true) = (&mut rr.show, rr.shows_albedo) {
                    *group = show_group(&self.device, pipeline, albedo);
                }
            }
            let held = (
                std::mem::replace(&mut self.color, inputs.color.clone()),
                std::mem::replace(&mut self.depth, inputs.depth.clone()),
            );
            self.queue.on_submitted_work_done(move || drop(held));
            self.reset = true;
        }

        /// The bytes of DLSS's own textures, Ray Reconstruction's guides
        /// included (wgpu's counters measure the TAA history without a DLSS
        /// kept, `Backend::remake_taa`).
        pub fn bytes(&self) -> u64 {
            let guides = self.rr.as_ref().map(|rr| [&rr.diffuse, &rr.specular, &rr.normals]);
            [&self.motion, &self.safe, &self.output]
                .into_iter()
                .chain(guides.into_iter().flatten())
                .map(|view| {
                    let t = view.texture();
                    u64::from(t.width()) * u64::from(t.height()) * u64::from(t.format().block_copy_size(None).unwrap_or(0))
                })
                .sum()
        }

        /// Ray Reconstruction draws the frames, on top of the
        /// renderer's own ray-tracing denoisers.
        pub fn rr_on(&self) -> bool {
            self.rr.is_some() && self.on()
        }

        /// DLSS draws the frames: it has a feature and has not failed.
        pub fn on(&self) -> bool {
            self.made && !self.failed
        }

        /// Made at the display size: DLAA, not super resolution (what the
        /// reports name, `RenderStats::dlss_drawing`).
        pub fn dlaa(&self) -> bool {
            self.shape.mode == Mode::Dlaa
        }

        /// Given up for its size: its renderer lets it go, and with it the
        /// feature and DLSS's textures ([`Dlss::retire`] once its frames
        /// are done), FSR drawing from here.
        pub fn failed(&self) -> bool {
            self.failed
        }

        /// What a DLSS kept by a remake of the targets serves now
        /// ([`Dlss::new`]'s `snapshot` and `report`): one made for a
        /// snapshot's targets and kept for the window's reports again, and
        /// the reverse.
        pub fn serving(&mut self, snapshot: bool, report: bool) {
            (self.snapshot, self.report) = (snapshot, report && !snapshot);
        }

        /// Made for, or kept for, a snapshot's targets.
        pub fn snapshot(&self) -> bool {
            self.snapshot
        }

        /// The device this DLSS was made on is still the one opened for
        /// Streamline: a renderer left behind by a device loss must not take Streamline
        /// back from the new one ([`with_started`] starts Streamline anew on another
        /// device).
        fn ours(&self) -> bool {
            opened().as_ref().is_some_and(|(_, d)| same(d, &self.device))
        }

        /// The feature made again with `models` (Preferences' model lists):
        /// the sizes, DLSS's textures and the targets stay, as the model
        /// is read only when the feature is made (guide 3.12). Only the
        /// kind drawn counts: super resolution's model for super
        /// resolution, Ray Reconstruction's for Ray Reconstruction. A
        /// failed DLSS is tried once more. The old feature is released once
        /// its frames are done ([`Dlss::retire`]).
        pub fn remodel(&mut self, models: (crate::DlssModel, crate::DlssRrModel)) {
            let shape = shape(self.shape.render, self.shape.display, models, self.rr.is_some());
            // (on a lost device nothing is made again: the renderer goes)
            if (shape == self.shape && !self.failed) || self.lost.get() {
                return;
            }
            if !self.ours() {
                self.failed = true;
                return;
            }
            self.retire();
            // (not released a second time when dropped)
            self.made = false;
            (self.shape, self.failed, self.remade) = (shape, false, false);
            match create(self.lib, &self.device, &self.lost, &self.queue, &self.shape, self.report) {
                Some(made) => {
                    active(&self.shape, &made);
                    (self.viewport, self.start, self.made, self.reset) = (made.viewport, made.start, true, true);
                }
                None => self.failed = true,
            }
        }

        /// The inputs' pass into `encoder`, then DLSS's own commands, to be
        /// submitted right after `encoder`'s, nothing between (dlss_wgpu:
        /// anything else is undefined behaviour). None on a frame DLSS
        /// failed: the feature is made again once, for the next frame, and
        /// a second failure ends DLSS for this size. None on a lost device,
        /// with nothing retired or made again on it (nor any failure for
        /// Preferences): the renderer goes with the device.
        pub fn encode(&mut self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, f: &Frame) -> Option<wgpu::CommandBuffer> {
            if !self.on() || self.lost.get() {
                return None;
            }
            let size = glam::Vec2::new(self.render().0 as f32, self.render().1 as f32);
            let u = MotionU {
                reproj: f.motion_reproj,
                jitter_uv: (f.jitter / size).to_array(),
                motion_on: if f.motion_on { 1.0 } else { 0.0 },
                pad: 0.0,
            };
            queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&u));
            {
                let target = |view| {
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })
                };
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("DLSS inputs"),
                    color_attachments: &[target(&self.motion), target(&self.safe)],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.group, &[]);
                pass.draw(0..3, 0..1);
            }
            if let Some(rr) = &self.rr {
                let u = GuidesU {
                    inv_view_proj: (f.proj * f.view).inverse().to_cols_array_2d(),
                    eye: f.camera_pos.extend(1.0).to_array(),
                };
                queue.write_buffer(&rr.buffer, 0, bytemuck::bytes_of(&u));
                let target = |view| {
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })
                };
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("DLSS guides"),
                    color_attachments: &[target(&rr.diffuse), target(&rr.specular), target(&rr.normals)],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&rr.pipeline);
                pass.set_bind_group(0, &rr.group, &[]);
                pass.draw(0..3, 0..1);
            }
            self.fg_frame = None;
            #[cfg(windows)]
            self.fg_copy_inputs(encoder);
            let commands = self.evaluate(encoder, f);
            match commands {
                Ok((commands, token, constants)) => {
                    self.fg_frame = Some((token, constants));
                    // drawing: what Preferences shows is no failure (one of
                    // a DLSS before this, or of a snapshot's, is over)
                    working(self.report);
                    self.reset = false;
                    self.drawn += 1;
                    if self.drawn == 1 || self.drawn.is_multiple_of(600) {
                        log::info!("DLSS: {} frames drawn", self.drawn);
                    }
                    Some(commands)
                }
                Err(e) => {
                    // (the loss found by this very evaluate)
                    if self.lost.get() {
                        return None;
                    }
                    // Ray Reconstruction failing again: super resolution
                    // draws instead (RR_REFUSED makes the renderer remake
                    // DLSS without it). A snapshot's fails at once, its
                    // first failure a refusal for every capture, as a
                    // refused feature is: its DLSS is made per capture, so
                    // a second failure would never come, and a film would
                    // make it again at every frame
                    if (self.remade || self.snapshot) && self.rr.is_some() && self.ours() {
                        let why = format!("evaluate failed after {} frames: {e}", self.drawn);
                        if self.snapshot {
                            snapshot_refusal(&why);
                        } else {
                            rr_refused(&why, self.report);
                        }
                        self.failed = true;
                        return None;
                    }
                    if self.remade || !self.ours() {
                        failed(&format!("evaluate failed after {} frames: {e}, from here", self.drawn), self.report);
                        self.failed = true;
                        return None;
                    }
                    log::error!(
                        "DLSS: evaluate failed after {} frames: {e}: FSR draws this frame, the feature is made again",
                        self.drawn
                    );
                    self.remade = true;
                    self.retire();
                    // (not released a second time when dropped)
                    self.made = false;
                    match create(self.lib, &self.device, &self.lost, &self.queue, &self.shape, self.report) {
                        Some(made) => {
                            (self.viewport, self.start, self.made, self.reset) = (made.viewport, made.start, true, true);
                            // (a new viewport: frame generation off on the
                            // old one, `retire`, and not yet asked here)
                            #[cfg(windows)]
                            {
                                (self.fg_win.asked, self.fg_win.used, self.fg_win.bad) = (0, false, None);
                            }
                        }
                        None => self.failed = true,
                    }
                    None
                }
            }
        }

        /// The scene's alpha of this frame over the one DLSS wrote, into
        /// the encoder submitted after DLSS's commands.
        pub fn encode_alpha(&self, encoder: &mut wgpu::CommandEncoder) {
            let show = self.rr.as_ref().and_then(|rr| rr.show.as_ref());
            let Some((pipeline, group)) = show.or(self.alpha.as_ref()) else {
                return;
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("DLSS alpha"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.output,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }

        fn render(&self) -> (u32, u32) {
            self.shape.render
        }

        fn evaluate(&self, encoder: &mut wgpu::CommandEncoder, f: &Frame) -> Result<(wgpu::CommandBuffer, usize, sl::Constants), String> {
            if fails(EVALUATES.fetch_add(1, Ordering::Relaxed) + 1) {
                return Err(String::from("AURORA_DLSS_FAIL"));
            }
            let lost = "a texture is not Vulkan's";
            // the library reads these through the tags' pointers, during
            // the tag call
            let mut color = resource(&self.safe, &self.adapter, false).ok_or(lost)?;
            let mut depth = resource(&self.depth, &self.adapter, false).ok_or(lost)?;
            let mut motion = resource(&self.motion, &self.adapter, false).ok_or(lost)?;
            let mut output = resource(&self.output, &self.adapter, true).ok_or(lost)?;
            let mut guides = match &self.rr {
                Some(rr) => Some((
                    resource(&rr.diffuse, &self.adapter, false).ok_or(lost)?,
                    resource(&rr.specular, &self.adapter, false).ok_or(lost)?,
                    resource(&rr.normals, &self.adapter, false).ok_or(lost)?,
                )),
                None => None,
            };
            // NVIDIA's RR guide (3.4.9): "All matrices are Row Major Order
            // and use left multiplication", a row vector times the matrix
            // (Streamline's `float4x4`). That matrix is the transpose of
            // glam's, which takes a column vector, and its rows are glam's
            // columns: glam's own column array, as `constants` hands the
            // projection. Without jitter (`f.proj` has none). Debug:
            // AURORA_DLSS_RR_TRANSPOSED=1 hands the transpose instead
            let major = |m: glam::Mat4| {
                if rr_transposed() {
                    m.transpose().to_cols_array_2d()
                } else {
                    m.to_cols_array_2d()
                }
            };
            fn read(view: &wgpu::TextureView) -> wgpu::TextureTransition<&wgpu::Texture> {
                wgpu::TextureTransition {
                    texture: view.texture(),
                    selector: None,
                    state: wgpu::TextureUses::RESOURCE,
                }
            }
            encoder.transition_resources(
                std::iter::empty(),
                [
                    read(&self.safe),
                    read(&self.depth),
                    read(&self.motion),
                    wgpu::TextureTransition {
                        texture: self.output.texture(),
                        selector: None,
                        state: wgpu::TextureUses::STORAGE_READ_WRITE,
                    },
                ]
                .into_iter()
                .chain(
                    self.rr
                        .iter()
                        .flat_map(|rr| [read(&rr.diffuse), read(&rr.specular), read(&rr.normals)]),
                ),
            );
            let mut own = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("DLSS") });
            let (render, display) = (self.render(), self.shape.display);
            let extent = |(width, height): (u32, u32)| sl::Extent {
                top: 0,
                left: 0,
                width,
                height,
            };
            let mut constants = constants(Some(f), f.reset || self.reset);
            // Ray Reconstruction reads the camera from the options, set
            // each frame with the shape's own values (the library makes the
            // feature again only when those change)
            let rr_options = self.rr.as_ref().map(|_| {
                if rr_transposed() {
                    constants.camera_view_to_clip = major(f.proj);
                }
                let mut options = sl::DlssdOptions::new(self.shape.mode as u32, display);
                options.preset(self.shape.rr_preset);
                options.alpha_upscaling_enabled = sl::boolean(self.shape.alpha);
                options.world_to_camera_view = major(f.view);
                options.camera_view_to_world = major(f.view.inverse());
                options
            });
            let handle = sl::ViewportHandle::new(self.viewport);
            let lib = self.lib;
            let mut frame: sl::FrameToken = std::ptr::null_mut();
            // SAFETY: under the lock, only when the start running is the
            // one that made this feature (so its viewport is live); every
            // struct and the resources the tags point at are locals that
            // outlive the calls, whose views the Dlss holds; the command
            // buffer is `own`'s open one, which is submitted right after
            // the encoder's (dlss_wgpu's order)
            let r = with_started(lib, &self.device, &self.lost, false, |st| unsafe {
                if st.start != self.start {
                    return None;
                }
                Some(own.as_hal_mut::<Vulkan, _, _>(|e| {
                    let cmd = e.ok_or("not a Vulkan encoder")?.raw_handle();
                    // Windows' frame generation: the frame's own token,
                    // which its present markers name ([`Dlss::fg_begin`])
                    #[cfg(windows)]
                    let given = self.fg_win.token;
                    // Linux: sl-ngx's presenter's, for the same reason
                    // ([`Dlss::set_frame_token`])
                    #[cfg(target_os = "linux")]
                    let given = self.frame_token;
                    #[cfg(not(any(windows, target_os = "linux")))]
                    let given: Option<usize> = None;
                    let token = match given {
                        Some(token) => token as sl::FrameToken,
                        None => {
                            let mut token: sl::FrameToken = std::ptr::null_mut();
                            sl::check((lib.new_frame_token)(&mut token, std::ptr::null()))
                                .map_err(|e| format!("slGetNewFrameToken: {e}"))?;
                            token
                        }
                    };
                    frame = token;
                    sl::check((lib.set_constants)(&constants, token, &handle)).map_err(|e| format!("slSetConstants: {e}"))?;
                    if let Some(options) = &rr_options {
                        let dlssd = lib.dlssd()?;
                        sl::check((dlssd.set_options)(&handle, options)).map_err(|e| format!("slDLSSDSetOptions: {e}"))?;
                    }
                    // (with Windows' frame generation loaded, valid until
                    // the present: the library would copy them otherwise
                    // once frame generation has read them at a present;
                    // its own are the copies `fg_present` tags after)
                    let fg_copy = |tag: sl::ResourceTag| {
                        if cfg!(windows) && given.is_some() {
                            tag.until_present()
                        } else {
                            tag
                        }
                    };
                    let mut tags = vec![
                        sl::ResourceTag::new(&mut color, sl::buffer::SCALING_INPUT_COLOR, extent(render)),
                        fg_copy(sl::ResourceTag::new(&mut depth, sl::buffer::DEPTH, extent(render))),
                        fg_copy(sl::ResourceTag::new(&mut motion, sl::buffer::MOTION_VECTORS, extent(render))),
                        sl::ResourceTag::new(&mut output, sl::buffer::SCALING_OUTPUT_COLOR, extent(display)),
                    ];
                    if let Some((diffuse, specular, normals)) = &mut guides {
                        tags.extend([
                            sl::ResourceTag::new(diffuse, sl::buffer::ALBEDO, extent(render)),
                            sl::ResourceTag::new(specular, sl::buffer::SPECULAR_ALBEDO, extent(render)),
                            sl::ResourceTag::new(normals, sl::buffer::NORMAL_ROUGHNESS, extent(render)),
                        ]);
                    }
                    sl::check((lib.set_tag_for_frame)(token, &handle, tags.as_ptr(), tags.len() as u32, cmd))
                        .map_err(|e| format!("slSetTagForFrame: {e}"))?;
                    let inputs = [(&raw const handle).cast::<sl::Base>()];
                    sl::check((lib.evaluate)(self.shape.feature, token, inputs.as_ptr(), inputs.len() as u32, cmd))
                        .map_err(|e| format!("slEvaluateFeature: {e}"))
                }))
            })
            .flatten()
            .ok_or("DLSS is no longer started for this feature")?;
            r?;
            Ok((own.finish(), frame as usize, constants))
        }

        /// Windows' frame generation, at the frame's start (renderer.rs,
        /// before the window image is acquired): the frame's token,
        /// Reflex's sleep, and the PCL markers up to the submit's start.
        /// Linux: this frame's token from sl-ngx's presenter
        /// ([`Presenter::token`]), for `evaluate`, or None without one.
        #[cfg(target_os = "linux")]
        pub fn set_frame_token(&mut self, token: Option<usize>) {
            self.frame_token = token;
        }

        /// Every frame while Streamline has frame generation loaded
        /// ([`fg_ready`]), on or off: its markers name the frame each
        /// present shows, and `evaluate` uses the token.
        #[cfg(windows)]
        pub fn fg_begin(&mut self) {
            self.fg_win.token = None;
            if !fg_ready() || !self.on() || self.lost.get() {
                return;
            }
            let lib = self.lib;
            // SAFETY: under the lock, for the start that made the viewport;
            // the token is the library's own, kept by it for some frames
            let r = with_started(lib, &self.device, &self.lost, false, |st| unsafe {
                if st.start != self.start {
                    return None;
                }
                Some((|| {
                    let f = lib.dlssg()?;
                    let mut token: sl::FrameToken = std::ptr::null_mut();
                    sl::check((lib.new_frame_token)(&mut token, std::ptr::null())).map_err(|e| format!("slGetNewFrameToken: {e}"))?;
                    sl::check((f.reflex_sleep)(token)).map_err(|e| format!("slReflexSleep: {e}"))?;
                    for marker in [sl::pcl::SIMULATION_START, sl::pcl::SIMULATION_END, sl::pcl::RENDER_SUBMIT_START] {
                        sl::check((f.pcl_set_marker)(marker, token)).map_err(|e| format!("slPCLSetMarker({marker}): {e}"))?;
                    }
                    Ok::<_, String>(token as usize)
                })())
            })
            .flatten();
            match r {
                Some(Ok(token)) => self.fg_win.token = Some(token),
                Some(Err(e)) => self.fg_failed(&e),
                None => {}
            }
        }

        /// Windows' frame generation, before DLSS's evaluate: the depth and
        /// the motion it reads, copied into frame generation's own
        /// textures, which `fg_present` tags valid until the present.
        /// While Streamline has frame generation loaded (a token from
        /// [`Dlss::fg_begin`]); the copies are made the first time, at the
        /// originals' size and format, and refused, frame generation stays
        /// off for this DLSS.
        #[cfg(windows)]
        fn fg_copy_inputs(&mut self, encoder: &mut wgpu::CommandEncoder) {
            if self.fg_win.token.is_none() || self.fg_win.inputs_refused {
                return;
            }
            let (depth, motion) = (self.depth.texture(), self.motion.texture());
            let fits =
                |copy: &wgpu::TextureView, of: &wgpu::Texture| copy.texture().size() == of.size() && copy.texture().format() == of.format();
            if !self.fg_win.inputs.as_ref().is_some_and(|[d, m]| fits(d, depth) && fits(m, motion)) {
                self.fg_win.inputs = None;
                if !(depth.usage() & motion.usage()).contains(wgpu::TextureUsages::COPY_SRC) {
                    self.fg_win.inputs_refused = true;
                    self.fg_failed("the depth or the motion DLSS reads cannot be copied");
                    return;
                }
                let device = &self.device;
                let copy = |of: &wgpu::Texture, label| {
                    device
                        .create_texture(&wgpu::TextureDescriptor {
                            label: Some(label),
                            size: of.size(),
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: of.format(),
                            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                            view_formats: &[],
                        })
                        .create_view(&Default::default())
                };
                let (made, refused) = crate::gpu_alloc::scoped(device, || [copy(depth, "DLSS FG depth"), copy(motion, "DLSS FG motion")]);
                if refused {
                    self.fg_win.inputs_refused = true;
                    log::warn!("DLSS frame generation: the video memory refused its copies of the depth and the motion: off");
                    return;
                }
                self.fg_win.inputs = Some(made);
            }
            if let Some([d, m]) = &self.fg_win.inputs {
                encoder.copy_texture_to_texture(depth.as_image_copy(), d.texture().as_image_copy(), depth.size());
                encoder.copy_texture_to_texture(motion.as_image_copy(), m.texture().as_image_copy(), motion.size());
            }
        }

        /// Windows' frame generation, before the frame's last submit: the
        /// window's image before the interface (`hudless`, at the window's
        /// size, in its format) and the copies of the depth and the motion
        /// ([`Dlss::fg_copy_inputs`]) tagged with the frame's token, and
        /// frame generation asked on for the present, [`fg_frames`]
        /// generated per frame drawn (1 for 2x, at most what the GPU says).
        /// Off with `hudless` None (the setting off, a modal's blur, a
        /// resize to come), for a frame DLSS did not draw, for a window
        /// under the smallest size it takes, and at a size its status was
        /// not OK at; `keep` keeps the feature's memory through an off of a
        /// moment. The commands go after the encoder's; None when there
        /// are none to run.
        #[cfg(windows)]
        pub fn fg_present(
            &mut self,
            encoder: &mut wgpu::CommandEncoder,
            hudless: Option<&wgpu::TextureView>,
            keep: bool,
        ) -> Option<wgpu::CommandBuffer> {
            let token = self.fg_win.token?;
            // a frame DLSS drew, its inputs copied: `evaluate` set its
            // constants with the token
            let drew = self.fg_win.inputs.is_some() && self.fg_frame.as_ref().is_some_and(|(t, _)| *t == token);
            let hudless = hudless.filter(|_| drew);
            let size = hudless.map(|view| {
                let size = view.texture().size();
                (size.width, size.height)
            });
            // the GPU's most frames and the smallest size, read before the
            // first ask: more frames than the GPU makes are refused (DLSS-G
            // guide §6.2), a smaller window must have it off (§17)
            if size.is_some() && !self.fg_win.state_read {
                self.fg_win.state_read = true;
                if let Some(state) = self.fg_state() {
                    (self.fg_win.max, self.fg_win.min) = (state.num_frames_to_generate_max, state.min_width_or_height);
                    log::info!(
                        "DLSS frame generation: up to {}x here, windows from {} pixels a side",
                        state.num_frames_to_generate_max + 1,
                        state.min_width_or_height
                    );
                }
            }
            if let (Some(bad), Some(size)) = (self.fg_win.bad, size)
                && bad != size
            {
                self.fg_win.bad = None;
            }
            let small = size.is_some_and(|(w, h)| w.min(h) < self.fg_win.min);
            let hudless = hudless.filter(|_| !small && self.fg_win.bad.is_none());
            let mut tagged = match (hudless, &self.fg_win.inputs) {
                (Some(hudless), Some([depth, motion])) => (|| {
                    Some([
                        resource(hudless, &self.adapter, false)?,
                        resource(depth, &self.adapter, false)?,
                        resource(motion, &self.adapter, false)?,
                    ])
                })(),
                _ => None,
            };
            if tagged.is_some()
                && let (Some(hudless), Some([depth, motion])) = (hudless, &self.fg_win.inputs)
            {
                fn read(view: &wgpu::TextureView) -> wgpu::TextureTransition<&wgpu::Texture> {
                    wgpu::TextureTransition {
                        texture: view.texture(),
                        selector: None,
                        state: wgpu::TextureUses::RESOURCE,
                    }
                }
                encoder.transition_resources(std::iter::empty(), [read(hudless), read(depth), read(motion)].into_iter());
            }
            let on = tagged.is_some();
            let frames = match self.fg_win.max {
                0 => 1,
                max => fg_frames().min(max),
            };
            let options = sl::DlssgOptions::new(
                if on { sl::dlssg_mode::ON } else { sl::dlssg_mode::OFF },
                frames,
                if on || keep { sl::DLSSG_RETAIN_RESOURCES_WHEN_OFF } else { 0 },
            );
            let extent = |(width, height): (u32, u32)| sl::Extent {
                top: 0,
                left: 0,
                width,
                height,
            };
            let (display, render) = (size.map(extent), extent(self.render()));
            let mut own = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("DLSS frame generation inputs"),
            });
            let handle = sl::ViewportHandle::new(self.viewport);
            let lib = self.lib;
            // SAFETY: as in `evaluate`: under the lock, for the start that
            // made the viewport; the token is this frame's ([`fg_begin`]);
            // the structs are locals that outlive the calls; the images are
            // valid until the present (frame generation's own, written by
            // nothing until the next frame's copies, which go after the
            // present), and the Dlss keeps them until its last present
            // (`retire`); no copy is asked, so `own` stays empty
            let r = with_started(lib, &self.device, &self.lost, false, |st| unsafe {
                if st.start != self.start {
                    return None;
                }
                Some(own.as_hal_mut::<Vulkan, _, _>(|e| {
                    let cmd = e.ok_or("not a Vulkan encoder")?.raw_handle();
                    let f = lib.dlssg()?;
                    if let (Some([hudless, depth, motion]), Some(display)) = (&mut tagged, display) {
                        let tags = [
                            sl::ResourceTag::new(hudless, sl::buffer::HUDLESS_COLOR, display).until_present(),
                            sl::ResourceTag::new(depth, sl::buffer::DEPTH, render).until_present(),
                            sl::ResourceTag::new(motion, sl::buffer::MOTION_VECTORS, render).until_present(),
                        ];
                        sl::check((lib.set_tag_for_frame)(
                            token as sl::FrameToken,
                            &handle,
                            tags.as_ptr(),
                            tags.len() as u32,
                            cmd,
                        ))
                        .map_err(|e| format!("slSetTagForFrame (frame generation): {e}"))?;
                    }
                    sl::check((f.set_options)(&handle, &options)).map_err(|e| format!("slDLSSGSetOptions: {e}"))
                }))
            })
            .flatten();
            match r {
                Some(Ok(())) => {
                    if on && self.fg_win.asked == 0 {
                        log::info!("DLSS frame generation: on, {} frame(s) generated per frame drawn", frames);
                    } else if !on && self.fg_win.asked != 0 {
                        log::info!("DLSS frame generation: off for now");
                    }
                    self.fg_win.asked = if on { frames } else { 0 };
                    self.fg_win.used |= on;
                    if on {
                        self.fg_win.size = size;
                    }
                    Some(own.finish())
                }
                Some(Err(e)) => {
                    self.fg_failed(&e);
                    None
                }
                None => None,
            }
        }

        /// Windows' frame generation asked off now, its memory kept: before
        /// the swapchain is made again outside a present asked off (an
        /// image the window could not give, a hold that did not end), and
        /// for a present that is not the frame's own (DLSS-G guide §17).
        #[cfg(windows)]
        pub fn fg_off(&mut self) {
            if self.fg_win.asked == 0 {
                return;
            }
            match self.fg_set_off(true) {
                Ok(()) => {
                    self.fg_win.asked = 0;
                    log::info!("DLSS frame generation: off for now");
                }
                Err(e) => self.fg_failed(&e),
            }
        }

        /// Windows' frame generation asked off for the viewport, its memory
        /// kept (`keep`) or let go at the next present.
        #[cfg(windows)]
        fn fg_set_off(&self, keep: bool) -> Result<(), String> {
            let options = sl::DlssgOptions::new(sl::dlssg_mode::OFF, 1, if keep { sl::DLSSG_RETAIN_RESOURCES_WHEN_OFF } else { 0 });
            let handle = sl::ViewportHandle::new(self.viewport);
            let lib = self.lib;
            // SAFETY: as in `fg_begin`; `options` lives through the call
            with_started(lib, &self.device, &self.lost, false, |st| unsafe {
                if st.start != self.start {
                    return None;
                }
                Some(
                    lib.dlssg()
                        .and_then(|f| sl::check((f.set_options)(&handle, &options)).map_err(|e| format!("slDLSSGSetOptions: {e}"))),
                )
            })
            .flatten()
            .unwrap_or(Ok(()))
        }

        /// Windows' frame generation's state for the viewport (no options
        /// passed: no VRAM estimate, the cheap call); None when it cannot
        /// be read.
        #[cfg(windows)]
        fn fg_state(&mut self) -> Option<sl::DlssgState> {
            let handle = sl::ViewportHandle::new(self.viewport);
            let lib = self.lib;
            let mut state = sl::DlssgState::new();
            // SAFETY: as in `fg_begin`; `state` lives through the call
            let r = with_started(lib, &self.device, &self.lost, false, |st| unsafe {
                if st.start != self.start {
                    return None;
                }
                Some(lib.dlssg().and_then(|f| {
                    sl::check((f.get_state)(&handle, &mut state, std::ptr::null())).map_err(|e| format!("slDLSSGGetState: {e}"))
                }))
            })
            .flatten();
            match r {
                Some(Ok(())) => Some(state),
                Some(Err(e)) => {
                    self.fg_failed(&e);
                    None
                }
                None => None,
            }
        }

        /// Windows' frame generation for Preferences: frames presented per
        /// frame drawn (0 while it is not asked on), and Streamline's
        /// status when it turned it off at this window size.
        #[cfg(windows)]
        pub fn fg_shown(&self) -> (u32, Option<String>) {
            let refused = self.fg_win.bad.filter(|b| Some(*b) == self.fg_win.size).and(self.fg_win.status);
            let per_drawn = if self.fg_win.asked == 0 { 0 } else { self.fg_win.per_drawn };
            (per_drawn, refused.map(sl::dlssg_status))
        }

        /// Windows' frame generation: the frame's commands are submitted
        /// (its PCL marker).
        #[cfg(windows)]
        pub fn fg_submitted(&mut self) {
            self.fg_mark(sl::pcl::RENDER_SUBMIT_END);
        }

        /// Windows' frame generation: the window's image is about to be
        /// presented (its PCL marker).
        #[cfg(windows)]
        pub fn fg_presenting(&mut self) {
            self.fg_mark(sl::pcl::PRESENT_START);
        }

        /// Windows' frame generation was asked on for the last present: a
        /// swapchain made again now would be made under it, which the
        /// DLSS-G guide (§17) says to avoid (off for a present first).
        #[cfg(windows)]
        pub fn fg_asked(&self) -> bool {
            self.fg_win.asked != 0
        }

        /// Windows' frame generation: the PCL marker `marker` (a stage of
        /// the submit or the present, `sl::pcl`) for the frame's token.
        #[cfg(windows)]
        fn fg_mark(&mut self, marker: u32) {
            let Some(token) = self.fg_win.token else {
                return;
            };
            let lib = self.lib;
            // SAFETY: as in `fg_begin`
            let r = with_started(lib, &self.device, &self.lost, false, |st| unsafe {
                if st.start != self.start {
                    return None;
                }
                Some(lib.dlssg().and_then(|f| {
                    sl::check((f.pcl_set_marker)(marker, token as sl::FrameToken)).map_err(|e| format!("slPCLSetMarker({marker}): {e}"))
                }))
            })
            .flatten();
            if let Some(Err(e)) = r {
                self.fg_failed(&e);
            }
        }

        /// Windows' frame generation, once the window's image is presented:
        /// the present's end marker, and the frame's token let go. With
        /// frame generation asked, its state, logged at the first present,
        /// each 600th and when its status changes; a status other than OK
        /// asks it off at this window size (`bad`).
        #[cfg(windows)]
        pub fn fg_end(&mut self) {
            self.fg_mark(sl::pcl::PRESENT_END);
            self.fg_win.token = None;
            if self.fg_win.asked == 0 {
                return;
            }
            self.fg_win.presents += 1;
            let Some(state) = self.fg_state() else {
                return;
            };
            (self.fg_win.max, self.fg_win.min) = (state.num_frames_to_generate_max, state.min_width_or_height);
            self.fg_win.per_drawn = state.num_frames_actually_presented;
            let n = self.fg_win.presents;
            if n == 1 || n.is_multiple_of(600) || self.fg_win.status != Some(state.status) {
                let level = if state.status == 0 { log::Level::Info } else { log::Level::Warn };
                log::log!(
                    level,
                    "DLSS frame generation: {n} presents, status {}; {} frame(s) presented per frame drawn, up to {}x here (minimum size {})",
                    sl::dlssg_status(state.status),
                    state.num_frames_actually_presented,
                    state.num_frames_to_generate_max + 1,
                    state.min_width_or_height,
                );
            }
            self.fg_win.status = Some(state.status);
            // (DLSS-G guide §14.3: "If not, disable DLSS-G"; three presents
            // in a row, so one of a moment, as Reflex starts, does not keep
            // it off)
            self.fg_win.bad_run = if state.status == 0 { 0 } else { self.fg_win.bad_run + 1 };
            if self.fg_win.bad_run >= 3 {
                log::warn!("DLSS frame generation: off at this window size, on again at another");
                (self.fg_win.bad, self.fg_win.bad_run) = (self.fg_win.size, 0);
                self.fg_off();
            }
        }

        /// Windows' frame generation failed: one line in the log a DLSS.
        #[cfg(windows)]
        fn fg_failed(&mut self, e: &str) {
            if !self.fg_win.failed {
                self.fg_win.failed = true;
                log::error!("DLSS frame generation: {e}");
            }
        }

        /// Frame generation (prototype, `RenderSettings::dlss_fg`, Linux's sl-ngx
        /// only): the frame halfway between the last frame and this one,
        /// without the interface, written to `out`. `hudless` is this frame
        /// before the interface was drawn; depth and motion are the ones
        /// this frame's DLSS evaluate tagged. Both images at the display
        /// size, in one format. The frame as shown is not handed to NGX: its
        /// interface, taken from where the two differ, smeared the dock
        /// and chat bar at some window sizes (1226x2055, not 1226x2050), so
        /// the caller lays the real frame's interface over `out`
        /// (`dlss_fg::FrameGen::present_into`). `reset` starts the history
        /// afresh (a frame went by without generation). The commands go
        /// after the encoder's; the flag says whether `out` holds a
        /// generated frame (not after a reset: there is no frame before).
        pub fn generate(
            &mut self,
            encoder: &mut wgpu::CommandEncoder,
            hudless: &wgpu::TextureView,
            out: &wgpu::TextureView,
            reset: bool,
        ) -> Result<(wgpu::CommandBuffer, bool), String> {
            let (token, mut constants) = self.fg_frame.take().ok_or("no DLSS evaluate this frame")?;
            if !self.on() || self.lost.get() {
                return Err(String::from("DLSS is off"));
            }
            let lost = "a texture is not Vulkan's";
            let mut hudless_r = resource(hudless, &self.adapter, false).ok_or(lost)?;
            let mut out_r = resource(out, &self.adapter, true).ok_or(lost)?;
            let read = |view: &wgpu::TextureView| -> wgpu::TextureTransition<wgpu::Texture> {
                wgpu::TextureTransition {
                    texture: view.texture().clone(),
                    selector: None,
                    state: wgpu::TextureUses::RESOURCE,
                }
            };
            let transitions = [
                read(hudless),
                read(&self.depth),
                read(&self.motion),
                wgpu::TextureTransition {
                    texture: out.texture().clone(),
                    selector: None,
                    state: wgpu::TextureUses::STORAGE_READ_WRITE,
                },
            ];
            encoder.transition_resources(
                std::iter::empty(),
                transitions.iter().map(|t| wgpu::TextureTransition {
                    texture: &t.texture,
                    selector: None,
                    state: t.state,
                }),
            );
            let mut own = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("DLSS frame generation"),
            });
            let reset = reset || constants.reset == sl::TRUE;
            constants.reset = sl::boolean(reset);
            let size = hudless.texture().size();
            let extent = sl::Extent {
                top: 0,
                left: 0,
                width: size.width,
                height: size.height,
            };
            let handle = sl::ViewportHandle::new(self.viewport);
            let lib = self.lib;
            // SAFETY: as in `evaluate`: under the lock, for the start that
            // made the viewport; the token is this frame's, from
            // `evaluate`, whose frame record the library keeps for some
            // frames; the structs are locals that outlive the calls
            let r = with_started(lib, &self.device, &self.lost, false, |st| unsafe {
                if st.start != self.start {
                    return None;
                }
                Some(own.as_hal_mut::<Vulkan, _, _>(|e| {
                    let cmd = e.ok_or("not a Vulkan encoder")?.raw_handle();
                    let token = token as sl::FrameToken;
                    sl::check((lib.set_constants)(&constants, token, &handle)).map_err(|e| format!("slSetConstants: {e}"))?;
                    let tags = [
                        sl::ResourceTag::new(&mut hudless_r, sl::buffer::HUDLESS_COLOR, extent),
                        sl::ResourceTag::new(&mut out_r, sl::buffer::SCALING_OUTPUT_COLOR, extent),
                    ];
                    let inputs = [
                        (&raw const handle).cast::<sl::Base>(),
                        (&raw const tags[0]).cast::<sl::Base>(),
                        (&raw const tags[1]).cast::<sl::Base>(),
                    ];
                    sl::check((lib.evaluate)(sl::FEATURE_DLSS_G, token, inputs.as_ptr(), inputs.len() as u32, cmd))
                        .map_err(|e| format!("slEvaluateFeature (frame generation): {e}"))
                }))
            })
            .flatten()
            .ok_or("DLSS is no longer started for this feature")?;
            r?;
            Ok((own.finish(), !reset))
        }

        /// Gives the feature back to Streamline once the frames submitted with it
        /// are done, without waiting for them here: a resize makes a
        /// feature per size, and waiting for the device at each stalled the
        /// frame. The callback runs where the device is polled, under
        /// Streamline's lock, and keeps the textures the feature read and wrote.
        /// A lost device's feature stays with its start of Streamline ([`release`],
        /// [`abandon`]).
        fn retire(&self) {
            if !self.made {
                return;
            }
            // Windows' frame generation asked off, without its memory kept,
            // before its feature is freed (DLSS-G guide §6.4: off, then
            // `slFreeResources`): the frames after this one have no DLSS
            // to ask it off
            #[cfg(windows)]
            if self.fg_win.used
                && let Err(e) = self.fg_set_off(false)
            {
                log::error!("DLSS frame generation: off as the feature goes: {e}");
            }
            let (lib, lost, start, viewport) = (self.lib, self.lost.clone(), self.start, self.viewport);
            // (the guides too: the feature reads them until its last
            // evaluate has run; and frame generation's copies, until its
            // last present)
            let guides = self
                .rr
                .as_ref()
                .map(|rr| (rr.diffuse.clone(), rr.specular.clone(), rr.normals.clone()));
            #[cfg(windows)]
            let fg_inputs = self.fg_win.inputs.clone();
            #[cfg(not(windows))]
            let fg_inputs: Option<()> = None;
            let held = (
                self.color.clone(),
                self.safe.clone(),
                self.depth.clone(),
                self.motion.clone(),
                self.output.clone(),
                guides,
                fg_inputs,
            );
            let begun = std::time::Instant::now();
            self.queue.on_submitted_work_done(move || {
                release(lib, &lost, start, viewport);
                drop(held);
                log::info!(
                    "DLSS: feature released {:.1} ms after it was retired, nothing waited",
                    begun.elapsed().as_secs_f64() * 1000.0
                );
            });
        }
    }

    impl Drop for Dlss {
        fn drop(&mut self) {
            self.retire();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::shape;
        use crate::{DlssModel, DlssRrModel};

        #[test]
        fn a_ray_reconstruction_model_leaves_super_resolution_as_it_is() {
            let sr = |rr_model| shape((960, 540), (1920, 1080), (DlssModel::default(), rr_model), false);
            assert!(sr(DlssRrModel::D) == sr(DlssRrModel::E));
            assert!(sr(DlssRrModel::Default) == sr(DlssRrModel::F));
            // Ray Reconstruction itself is made again for its new model,
            // unless a debug preset overrides both
            if std::env::var_os("AURORA_DLSS_RR_PRESET").is_none() {
                let rr = |rr_model| shape((960, 540), (1920, 1080), (DlssModel::default(), rr_model), true);
                assert!(rr(DlssRrModel::D) != rr(DlssRrModel::E));
            }
        }

        #[test]
        fn a_super_resolution_model_leaves_ray_reconstruction_as_it_is() {
            let rr = |model| shape((960, 540), (1920, 1080), (model, DlssRrModel::default()), true);
            assert!(rr(DlssModel::K) == rr(DlssModel::L));
            assert!(rr(DlssModel::Default) == rr(DlssModel::M));
            // super resolution itself is made again for its new model,
            // unless a debug preset overrides both
            if std::env::var_os("AURORA_DLSS_PRESET").is_none() {
                let sr = |model| shape((960, 540), (1920, 1080), (model, DlssRrModel::default()), false);
                assert!(sr(DlssModel::K) != sr(DlssModel::L));
            }
        }
    }
}

#[cfg(not(all(feature = "dlss", any(target_os = "linux", all(windows, target_arch = "x86_64")))))]
mod imp {
    use super::{Frame, Inputs};

    pub(crate) fn fg_wanted(_: bool) -> bool {
        false
    }

    pub(crate) fn fg_ready() -> bool {
        false
    }

    pub(crate) fn set_fg_at_start(_: bool) {}

    pub(crate) fn fg_restart(_: bool) -> bool {
        false
    }

    pub(crate) fn wanted(_: bool) -> bool {
        false
    }

    pub(crate) fn support() -> crate::DlssSupport {
        crate::DlssSupport::NotBuilt
    }

    pub(crate) fn support_on(_: &wgpu::Device) -> crate::DlssSupport {
        crate::DlssSupport::NotBuilt
    }

    pub(crate) fn retry() {}

    pub(crate) fn rr_retry() {}

    pub(crate) fn retry_asked(_: &mut u64) -> bool {
        false
    }

    pub(crate) fn refused_here() -> bool {
        false
    }

    pub(crate) fn refused_memory(_: &str, _: bool) {}

    pub(crate) fn closing(_: &wgpu::Device) {}

    pub(crate) fn left_behind() {}

    #[cfg(windows)]
    pub(crate) fn xr_entry() -> Option<ash::Entry> {
        None
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    pub(crate) fn xr_instance_extensions(_: &ash::Entry) -> Option<Vec<&'static std::ffi::CStr>> {
        None
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    pub(crate) fn xr_device_extensions(_: &wgpu::hal::vulkan::Adapter) -> Option<Vec<&'static std::ffi::CStr>> {
        None
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    pub(crate) fn opening_device(_: bool) -> bool {
        true
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    pub(crate) fn opened_for_xr(_: &wgpu::Adapter, _: &wgpu::Device, _: bool) {}

    pub(crate) fn folder_named() -> Option<String> {
        None
    }

    pub(crate) fn instance(_: &wgpu::InstanceDescriptor) -> Option<wgpu::Instance> {
        None
    }

    #[cfg(windows)]
    pub(crate) fn forget_instance() {}

    pub(crate) fn request_device(_: &wgpu::Adapter, _: &wgpu::DeviceDescriptor, _: wgpu::Limits) -> Option<(wgpu::Device, wgpu::Queue)> {
        None
    }

    pub(crate) fn optimal_size(_: &wgpu::Device, _: &crate::device_lost::DeviceLost, _: (u32, u32), _: f32, _: bool) -> Option<(u32, u32)> {
        None
    }

    pub(crate) fn rr_device() -> bool {
        false
    }

    pub(crate) fn rr_ready() -> bool {
        false
    }

    pub(crate) fn rr_ready_on(_: &wgpu::Device) -> bool {
        false
    }

    pub(crate) fn rr_refused_here() -> bool {
        false
    }

    pub(crate) fn rr_probe(_: &wgpu::Device, _: &crate::device_lost::DeviceLost) {}

    pub(crate) fn snapshot_rr_refused() -> bool {
        false
    }

    pub(crate) fn snapshot_rr_refused_here() -> bool {
        false
    }

    pub(crate) fn rr_targets_refused(_: bool) {}

    pub(crate) fn snapshot_rr_targets_refused() {}

    pub(crate) fn rr_wanted(_: bool) -> bool {
        false
    }

    pub(crate) fn resizing(_: (u32, u32), _: &std::cell::Cell<Option<(u32, u32)>>) -> Option<std::time::Instant> {
        None
    }

    /// Never made without the feature.
    pub(crate) struct Dlss(std::convert::Infallible);

    impl Dlss {
        pub fn new(
            _: &wgpu::Device,
            _: &wgpu::Queue,
            _: &crate::device_lost::DeviceLost,
            _: (u32, u32),
            _: (u32, u32),
            _: (crate::DlssModel, crate::DlssRrModel),
            _: &Inputs,
            _: bool,
            _: bool,
        ) -> Option<Self> {
            None
        }

        pub fn output(&self) -> &wgpu::TextureView {
            match self.0 {}
        }

        pub fn fits(&self, _: &wgpu::Device, _: (u32, u32), _: (u32, u32), _: (crate::DlssModel, crate::DlssRrModel), _: &Inputs) -> bool {
            match self.0 {}
        }

        pub fn remodel(&mut self, _: (crate::DlssModel, crate::DlssRrModel)) {
            match self.0 {}
        }

        pub fn rebind(&mut self, _: &Inputs) {
            match self.0 {}
        }

        pub fn bytes(&self) -> u64 {
            match self.0 {}
        }

        pub fn on(&self) -> bool {
            match self.0 {}
        }

        pub fn dlaa(&self) -> bool {
            match self.0 {}
        }

        pub fn failed(&self) -> bool {
            match self.0 {}
        }

        pub fn serving(&mut self, _: bool, _: bool) {
            match self.0 {}
        }

        pub fn snapshot(&self) -> bool {
            match self.0 {}
        }

        pub fn rr_on(&self) -> bool {
            match self.0 {}
        }

        pub fn encode(&mut self, _: &wgpu::Queue, _: &mut wgpu::CommandEncoder, _: &Frame) -> Option<wgpu::CommandBuffer> {
            match self.0 {}
        }

        pub fn encode_alpha(&self, _: &mut wgpu::CommandEncoder) {
            match self.0 {}
        }

        pub fn generate(
            &mut self,
            _: &mut wgpu::CommandEncoder,
            _: &wgpu::TextureView,
            _: &wgpu::TextureView,
            _: bool,
        ) -> Result<(wgpu::CommandBuffer, bool), String> {
            match self.0 {}
        }

        #[cfg(windows)]
        pub fn fg_begin(&mut self) {
            match self.0 {}
        }

        #[cfg(windows)]
        pub fn fg_present(&mut self, _: &mut wgpu::CommandEncoder, _: Option<&wgpu::TextureView>, _: bool) -> Option<wgpu::CommandBuffer> {
            match self.0 {}
        }

        #[cfg(windows)]
        pub fn fg_submitted(&mut self) {
            match self.0 {}
        }

        #[cfg(windows)]
        pub fn fg_presenting(&mut self) {
            match self.0 {}
        }

        #[cfg(windows)]
        pub fn fg_asked(&self) -> bool {
            match self.0 {}
        }

        #[cfg(windows)]
        pub fn fg_off(&mut self) {
            match self.0 {}
        }

        #[cfg(windows)]
        pub fn fg_end(&mut self) {
            match self.0 {}
        }
    }
}

pub(crate) use imp::{
    Dlss, closing, fg_ready, fg_restart, fg_wanted, folder_named, instance, left_behind, optimal_size, refused_here, refused_memory,
    request_device, resizing, retry, retry_asked, rr_device, rr_probe, rr_ready, rr_ready_on, rr_refused_here, rr_retry,
    rr_targets_refused, rr_wanted, set_fg_at_start, snapshot_rr_refused, snapshot_rr_refused_here, snapshot_rr_targets_refused, support,
    support_on, wanted,
};
#[cfg(all(feature = "dlss", target_os = "linux"))]
pub(crate) use imp::{Presenter, pause_presenter, update};
#[cfg(windows)]
pub(crate) use imp::{forget_instance, xr_entry};
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) use imp::{opened_for_xr, opening_device, xr_device_extensions, xr_instance_extensions};

/// The sl-ngx commit this viewer speaks to (Linux): `SL_NGX_COMMIT` in
/// scripts/dlss-setup.sh, which a test keeps the same. Its library is
/// `libsl_ngx.<first 12 digits>.so` in the DLSS folder, loaded before the
/// plain `libsl_ngx.so`, and built at start when missing ([`update`]).
#[cfg_attr(not(all(feature = "dlss", target_os = "linux")), allow(dead_code, reason = "Linux's DLSS only"))]
pub(crate) const SL_NGX_COMMIT: &str = "58cef3a222b7f248e7c639a5a196fcdba2abebf7";

/// `libsl_ngx.<commit>.so`: [`SL_NGX_COMMIT`]'s library.
#[cfg_attr(not(all(feature = "dlss", target_os = "linux")), allow(dead_code, reason = "Linux's DLSS only"))]
fn versioned_shim() -> String {
    format!("libsl_ngx.{}.so", &SL_NGX_COMMIT[..12])
}

/// The world's size under DLSS: the display size divided by the mode's
/// ratio, rounded up. Streamline's smallest accepted size is the display size over
/// the ratio rounded up (half of 2055 is 1028 for Performance), so FSR's
/// rounding down (`fsr::render_size`) is one pixel short at an odd size and
/// DLSS refuses it.
pub(crate) fn render_size(display: (u32, u32), ratio: f32) -> (u32, u32) {
    let ratio = ratio.max(1.0);
    let side = |d: u32| ((d as f32 / ratio).ceil() as u32).clamp(1, d.max(1));
    (side(display.0), side(display.1))
}

#[cfg(test)]
mod tests {
    use super::{Answers, SL_NGX_COMMIT, lock_within, refused, render_size};

    #[test]
    fn the_viewer_and_the_setup_script_name_one_sl_ngx() {
        // a viewer update brings its sl-ngx along only if the script
        // builds the commit the viewer loads
        let script = include_str!("../../../scripts/dlss-setup.sh");
        assert!(script.contains(&format!("\nSL_NGX_COMMIT={SL_NGX_COMMIT}\n")));
    }
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    #[test]
    fn a_lock_held_elsewhere_is_given_up_after_the_wait() {
        // the render thread left behind inside a Streamline call holds the
        // lock for good: opening the next device must not wait on it
        let lock = Arc::new(Mutex::new(()));
        let (took, held) = mpsc::channel();
        let (done, release) = mpsc::channel::<()>();
        let other = Arc::clone(&lock);
        let holder = std::thread::spawn(move || {
            let _guard = other.lock();
            let _ = took.send(());
            let _ = release.recv();
        });
        held.recv().expect("the holder took the lock");
        let start = Instant::now();
        assert!(lock_within(&lock, Duration::from_millis(30)).is_none());
        assert!(start.elapsed() >= Duration::from_millis(30));
        let _ = done.send(());
        holder.join().expect("the holder ended");
        // free again: taken at once
        assert!(lock_within(&lock, Duration::ZERO).is_some());
    }

    #[cfg(all(feature = "dlss", any(target_os = "linux", all(windows, target_arch = "x86_64"))))]
    #[test]
    fn only_a_size_that_moved_puts_dlss_off() {
        // a renderer's first size is not a resize; a new one is, until
        // it is asked again unchanged
        let last = std::cell::Cell::new(None);
        assert!(super::resizing((800, 600), &last).is_none());
        assert!(super::resizing((800, 600), &last).is_none());
        let due = super::resizing((801, 600), &last).expect("a new size waits");
        assert!(due > Instant::now());
        assert!(super::resizing((801, 600), &last).is_none());
    }

    #[test]
    fn a_poisoned_lock_is_still_taken() {
        // a thread that panicked holding it left the state as it was,
        // which end() can still release
        let lock = Arc::new(Mutex::new(7));
        let other = Arc::clone(&lock);
        let _ = std::thread::spawn(move || {
            let _guard = other.lock();
            panic!("poison the lock");
        })
        .join();
        assert!(lock.is_poisoned());
        assert_eq!(lock_within(&lock, Duration::ZERO).map(|held| *held), Some(7));
    }

    #[test]
    fn a_refused_start_is_kept_for_its_device_alone() {
        // nothing refused yet, whatever the device
        assert!(!refused(0, 0));
        assert!(!refused(0, 3));
        // refused on the third device opened: kept for it
        assert!(refused(3, 3));
        // a fourth device (a device loss, the headset's) asks again
        assert!(!refused(3, 4));
    }

    #[test]
    fn answers_keep_only_sizes_of_the_running_start() {
        let mut a = Answers::new();
        assert_eq!(a.get(None, &1), None);
        // a size Streamline did not give is asked again
        a.put(1, 1, None);
        assert_eq!(a.get(Some(1), &1), None);
        a.put(1, 1, Some((960, 540)));
        a.put(1, 2, Some((640, 360)));
        assert_eq!(a.get(Some(1), &1), Some((960, 540)));
        assert_eq!(a.get(Some(1), &2), Some((640, 360)));
        // Streamline not running, or started again (a new device): asked again
        assert_eq!(a.get(None, &1), None);
        assert_eq!(a.get(Some(2), &1), None);
        // the new start's answer forgets the old start's
        a.put(2, 1, Some((1280, 720)));
        assert_eq!(a.get(Some(2), &1), Some((1280, 720)));
        assert_eq!(a.get(Some(2), &2), None);
        assert_eq!(a.get(Some(1), &1), None);
    }

    #[test]
    fn an_odd_display_rounds_up_to_the_dlss_minimum() {
        assert_eq!(render_size((3768, 2055), 2.0), (1884, 1028));
        assert_eq!(render_size((918, 1581), 2.0), (459, 791));
        assert_eq!(render_size((3768, 2055), 1.0), (3768, 2055));
        assert_eq!(render_size((1, 1), 3.0), (1, 1));
    }
}

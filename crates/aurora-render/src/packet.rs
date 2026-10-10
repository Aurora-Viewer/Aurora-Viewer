//! What crosses from the main thread to the render thread, and back.
//!
//! A [`FramePacket`] is everything one frame needs, owned: the render
//! thread never reads the scene's stores, which the main thread is already
//! changing for the next frame. It holds
//!
//! - the frame's write journal (writes.rs): dirty ranges of the draw
//!   records, face and object tables, joint palettes, skin bindings,
//!   geometry and texel writes, texture locations; and the streamed copies
//!   recorded for it (upload.rs), with the staging chunks they free;
//! - the scene's GPU buffers and bind group as they are for this frame
//!   ([`SceneGpu`]): the main thread may replace them (a table that grows,
//!   a new texture page) while the frame is still being drawn;
//! - camera, environment, lights and probes ([`FrameParams`]), the draw
//!   lists still built on the CPU ([`DrawLists`]: terrain, water, blended
//!   faces, glow, debug, selection, impostors, particles) and the view of
//!   the GPU culling;
//! - the interface: egui primitives and texture changes;
//! - the window surface state and the render settings to apply;
//! - the readbacks asked for: the pixel under the cursor, a capture.
//!
//! A [`FrameResult`] comes back once the frame is presented: its
//! statistics, the readbacks that arrived, and the packet's buffers to
//! fill again.

use crate::gpu_cull::CullCounts;
use crate::types::{DrawLists, FrameParams, RenderSettings, RenderStats};
use crate::upload::FrameUploads;
use crate::writes::GpuWrites;
use glam::Vec3;

/// The interface of a frame, as egui tessellated it.
pub struct EguiFrame {
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub textures_delta: egui::TexturesDelta,
    pub pixels_per_point: f32,
}

/// Capture of the presented image asked with a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Capture {
    /// The 3D view, before the interface is drawn over it.
    Scene,
    /// The whole frame.
    Full,
}

/// Which capture a frame carries: the scene alone wins over the whole
/// frame (one answer resets both requests), none when the surface cannot be
/// read back.
pub(crate) fn capture_for(scene: bool, full: bool, can_capture: bool) -> Option<Capture> {
    match (can_capture, scene, full) {
        (false, _, _) | (_, false, false) => None,
        (_, true, _) => Some(Capture::Scene),
        (_, false, true) => Some(Capture::Full),
    }
}

/// Window surface as the main thread wants it for a frame; the render
/// thread reconfigures the swapchain when it differs from what it has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SurfaceState {
    pub width: u32,
    pub height: u32,
    pub vsync: bool,
}

/// The scene's data on the GPU for one frame: handles (reference counted)
/// taken when the frame is handed over.
#[derive(Clone)]
pub(crate) struct SceneGpu {
    /// Draw records, joint palettes and skin bindings (one bind group).
    pub records: wgpu::Buffer,
    pub palettes: wgpu::Buffer,
    pub skin_binds: wgpu::Buffer,
    /// Changes when one of the three buffers above was reallocated.
    pub records_key: (u64, u64),
    /// Face table (one entry per draw record) and object table of the GPU
    /// culling, with their sizes.
    pub faces: wgpu::Buffer,
    pub face_count: usize,
    pub objects: wgpu::Buffer,
    pub object_count: usize,
    /// Record ids handed out so far (sizes the occlusion's visibility flags).
    pub record_slots: usize,
    /// Texture pages, sampler and locations.
    pub textures: wgpu::BindGroup,
    pub vertices: wgpu::Buffer,
    pub skin: wgpu::Buffer,
    pub indices: wgpu::Buffer,
}

/// Buffers of a packet that keep their memory from frame to frame.
#[derive(Default)]
pub(crate) struct Spent {
    pub writes: GpuWrites,
    pub lists: DrawLists,
}

pub(crate) struct FramePacket {
    /// Frames handed over before this one.
    pub index: u64,
    pub writes: GpuWrites,
    pub uploads: FrameUploads,
    pub scene: SceneGpu,
    pub params: FrameParams,
    pub lists: DrawLists,
    pub ui: Option<EguiFrame>,
    pub surface: SurfaceState,
    pub settings: RenderSettings,
    /// Pixel whose depth to read back (hover; answered a few frames later).
    pub hover: Option<(u32, u32)>,
    pub capture: Option<Capture>,
}

pub(crate) struct FrameResult {
    pub index: u64,
    /// The renderer's side of the statistics (the scene's stores add theirs).
    pub stats: RenderStats,
    /// Last hover answer: world position under the requested pixel.
    pub hover: Option<Vec3>,
    /// Last GPU culling counts read back.
    pub cull: CullCounts,
    /// The capture asked with this frame (width, height, RGBA8); None when
    /// the frame could not be drawn (no swapchain image).
    pub captured: Option<(u32, u32, Vec<u8>)>,
    pub spent: Spent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_capture_wins_and_needs_a_readable_surface() {
        assert_eq!(capture_for(false, false, true), None);
        assert_eq!(capture_for(true, false, true), Some(Capture::Scene));
        assert_eq!(capture_for(false, true, true), Some(Capture::Full));
        // both asked: the 3D view alone, as before (one answer for both)
        assert_eq!(capture_for(true, true, true), Some(Capture::Scene));
        // a surface without COPY_SRC never answers: no capture is carried,
        // so the frame is not made to wait for one
        assert_eq!(capture_for(true, true, false), None);
    }
}

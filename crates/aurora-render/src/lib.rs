//! Aurora Viewer renderer: wgpu on the Vulkan backend, bindless textures,
//! sub-allocated geometry and multi-draw-indirect submission.

pub mod arena;
mod occlusion;
pub mod probes;
pub mod renderer;
pub mod textures;
pub mod types;

pub use arena::MeshAlloc;
pub use renderer::{CASCADES, EguiFrame, GpuInfo, RenderError, Renderer};
pub use textures::{MipLevel, build_mips};
pub use types::*;

pub use egui;
pub use egui_wgpu;
pub use wgpu;

#[cfg(test)]
mod shader_tests {
    fn validate(src: &str) {
        let module = match naga::front::wgsl::parse_str(src) {
            Ok(m) => m,
            Err(e) => panic!("{}", e.emit_to_string(src)),
        };
        let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
        if let Err(e) = v.validate(&module) {
            panic!("{}", e.emit_to_string(src));
        }
    }

    #[test]
    fn scene_shader_is_valid() {
        let src = format!(
            "{}\n{}\n{}",
            include_str!("shaders/common.wgsl"),
            include_str!("shaders/object.wgsl"),
            include_str!("shaders/env.wgsl")
        );
        validate(&src);
    }

    #[test]
    fn ssao_shader_is_valid() {
        validate(include_str!("shaders/ssao.wgsl"));
    }

    #[test]
    fn ssr_shader_is_valid() {
        validate(&format!(
            "{}\n{}",
            include_str!("shaders/common.wgsl"),
            include_str!("shaders/ssr.wgsl")
        ));
    }

    #[test]
    fn post_shader_is_valid() {
        validate(include_str!("shaders/post.wgsl"));
    }

    #[test]
    fn probe_filter_shader_is_valid() {
        validate(include_str!("shaders/probe_filter.wgsl"));
    }

    #[test]
    fn smaa_shader_is_valid() {
        validate(include_str!("shaders/smaa.wgsl"));
    }

    #[test]
    fn occlusion_shader_is_valid() {
        validate(include_str!("shaders/occlusion.wgsl"));
    }

    #[test]
    fn glow_shader_is_valid() {
        validate(include_str!("shaders/glow.wgsl"));
    }

    #[test]
    fn taa_shader_is_valid() {
        validate(include_str!("shaders/taa.wgsl"));
    }
}

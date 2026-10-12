//! Aurora Viewer renderer: wgpu on the Vulkan backend, bindless textures,
//! sub-allocated geometry and multi-draw-indirect submission.
//!
//! Two halves: `main_thread` (the `Renderer` the scene uses: its stores,
//! the frame packet) and `renderer` (the passes, run by the render thread
//! of `render_thread`); see main_thread.rs for who owns what.

pub mod arena;
pub mod bc7;
mod drawn_pick;
pub mod gpu_cull;
mod helpers;
pub mod hud;
pub mod light_tiles;
pub mod main_thread;
mod occlusion;
mod packet;
mod pick;
pub mod present;
pub mod probes;
mod render_thread;
mod renderer;
mod selection;
pub mod shadow_cull;
pub mod tex_anim;
pub mod textures;
pub mod thread_cpu;
pub mod types;
mod ui_blur;
mod ui_frame;
mod ui_glass;
pub mod upload;
pub mod vram;
pub mod writes;

pub use arena::MeshAlloc;
pub use drawn_pick::{Drawn, DrawnFace};
pub use gpu_cull::set_whole_chunk_uploads;
pub use hud::HudView;
pub use main_thread::{GpuInfo, Renderer};
pub use renderer::{CASCADES, RenderError};
pub use textures::{MipLevel, build_mips};
pub use types::*;
pub use ui_blur::blurred_backdrop;
pub use ui_frame::{EguiFrame, EguiHandOver};
pub use ui_glass::{glass_panel, glass_panel_faded, glass_region};
pub use upload::{StagedMesh, StagedTexture, StagingPool, TexelFormat};

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

    /// The statements of vertex entry point `entry` that lead to its
    /// position: the model matrix, the placed vertex and the clip position.
    fn position_steps<'a>(src: &'a str, entry: &str) -> Vec<&'a str> {
        let start = src.find(&format!("fn {entry}(")).unwrap_or_else(|| panic!("{entry} not found"));
        let body = &src[start..];
        let end = body.find("\n}").unwrap_or(body.len());
        body[..end]
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("let model =") || l.starts_with("let ep =") || l.starts_with("out.clip ="))
            .collect()
    }

    /// The depth prepass draws the terrain with `vs_depth`, and without
    /// multisampling the scene pass tests `vs_terrain` against that depth
    /// (`GreaterEqual`): both must reach the position by the same
    /// operations. When only the prepass placed its vertices relative to
    /// the eye, patches of terrain failed the test and showed the sky.
    #[test]
    fn terrain_position_is_the_depth_prepass_position() {
        let prepass = position_steps(include_str!("shaders/object.wgsl"), "vs_depth");
        let scene = position_steps(include_str!("shaders/env.wgsl"), "vs_terrain");
        assert_eq!(prepass.len(), 3, "{prepass:?}");
        assert_eq!(prepass, scene);
    }

    /// The same for the objects: the depth prepasses draw the opaque faces
    /// with `vs_depth` (the single-sample one, and with MSAA the one of
    /// the scene pass's own depth) and the scene pass tests `vs_main`
    /// against that depth. Plain, worn, rigged and anchored faces all go
    /// through `eye_model` in both.
    #[test]
    fn object_position_is_the_depth_prepass_position() {
        let src = include_str!("shaders/object.wgsl");
        let prepass = position_steps(src, "vs_depth");
        let scene = position_steps(src, "vs_main");
        assert_eq!(prepass.len(), 3, "{prepass:?}");
        assert_eq!(prepass, scene);
    }

    /// Equal operations give equal depths only if the compiler may not
    /// rearrange them differently in each shader: the three positions are
    /// declared invariant.
    #[test]
    fn prepass_and_scene_positions_are_invariant() {
        let object = include_str!("shaders/object.wgsl");
        let env = include_str!("shaders/env.wgsl");
        for (src, name) in [(object, "DepthOut"), (object, "VsOut"), (env, "TerrainOut")] {
            let start = src.find(&format!("struct {name} {{")).unwrap_or_else(|| panic!("{name} not found"));
            let body = &src[start..];
            let body = &body[..body.find('}').unwrap_or(body.len())];
            assert!(body.contains("@invariant @builtin(position) clip: vec4<f32>"), "{name}: {body}");
        }
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
    fn selection_overlay_shader_is_valid() {
        validate(include_str!("shaders/selection.wgsl"));
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
        validate(&format!(
            "{}
{}",
            include_str!("shaders/hiz_test.wgsl"),
            include_str!("shaders/occlusion.wgsl")
        ));
    }

    #[test]
    fn cull_shader_is_valid() {
        validate(&format!(
            "{}
{}",
            include_str!("shaders/hiz_test.wgsl"),
            include_str!("shaders/cull.wgsl")
        ));
    }

    #[test]
    fn glow_shader_is_valid() {
        validate(include_str!("shaders/glow.wgsl"));
    }

    #[test]
    fn ui_blur_shader_is_valid() {
        validate(include_str!("shaders/ui_blur.wgsl"));
    }

    #[test]
    fn taa_shader_is_valid() {
        validate(include_str!("shaders/taa.wgsl"));
    }
}

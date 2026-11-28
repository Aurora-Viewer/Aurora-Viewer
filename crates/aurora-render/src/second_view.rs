//! The second camera: a small window that keeps our avatar in view (the
//! face, the whole body, the back) while the main camera looks elsewhere.
//! Aurora's own: the Legacy Viewer has no such view. The idea comes from
//! SnowPastel's multi-view-viewer (an experimental viewer built on the
//! Second Life viewer, LGPL 2.1); none of its code is used here.
//!
//! The view is drawn by a second, external `Backend`, as an eye of VR is
//! (`render_xr`): it shares the scene's buffers and textures and the frame's
//! uploads, and owns its depth, shadow cascades, exposure and temporal
//! history, so its shadows and its anti-aliasing are those of its own
//! camera. It draws right after the window's frame, from a copy of the
//! frame's draw lists: the GPU culling gets the second camera's frustum, the
//! blended faces are put back to front from that camera (`resort_blend`),
//! and what belongs to the window alone is left out (HUDs, selection,
//! debug shapes, depth of field, the impostor cards of the window's own
//! atlas). The picture lands in a texture of its own that egui shows
//! through a paint callback (`paint`), with no readback and no wait: the
//! window shows the picture drawn after the frame before.

use crate::packet::{FramePacket, SurfaceState};
use crate::types::{DrawCmd, DrawLists};
use glam::{Mat4, Vec3};
use std::sync::Arc;

/// What the viewer asks the second camera to draw this frame: where it
/// looks from, its projection (reverse-Z infinite, as the window's) and
/// the size of the picture in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SecondViewJob {
    pub view: Mat4,
    pub proj: Mat4,
    pub eye: Vec3,
    pub fov_y: f32,
    pub size: [u32; 2],
}

/// Longest side of the picture, in pixels: a large window is drawn smaller
/// and stretched by egui, which keeps the cost of the second view bounded.
pub const MAX_SIDE: u32 = 1024;

/// The pixel size of a picture shown over `size` points at `scale` pixels
/// per point: both sides shrink together past `MAX_SIDE`, so the picture
/// keeps the shape of its rectangle.
pub fn target_size(size: egui::Vec2, scale: f32) -> [u32; 2] {
    let pixels = size * scale;
    let max = MAX_SIDE as f32;
    let fit = (max / pixels.max_elem().max(1.0)).min(1.0);
    [
        (pixels.x * fit).round().clamp(1.0, max) as u32,
        (pixels.y * fit).round().clamp(1.0, max) as u32,
    ]
}

/// The paint callback's marker: the render thread replaces it with a mesh
/// of the second view's texture.
struct Region;

/// Show the second view's picture over `rect`. Before the first picture is
/// drawn (and while the view cannot be drawn) nothing is painted there.
pub fn paint(ui: &egui::Ui, rect: egui::Rect) {
    ui.painter().add(egui::Shape::Callback(egui::PaintCallback {
        rect,
        callback: Arc::new(Region),
    }));
}

/// Whether `callback` is the second view's: a picture that changes at
/// every frame, whatever its shapes say.
pub(crate) fn is(callback: &egui::PaintCallback) -> bool {
    callback.callback.downcast_ref::<Region>().is_some()
}

/// Replace the second view's callbacks with meshes of its texture, or with
/// nothing while it has none (egui-wgpu only knows meshes).
pub(crate) fn replace(primitives: &mut [egui::ClippedPrimitive], texture: Option<egui::TextureId>) {
    for primitive in primitives {
        let egui::epaint::Primitive::Callback(callback) = &primitive.primitive else {
            continue;
        };
        if callback.callback.downcast_ref::<Region>().is_none() {
            continue;
        }
        let rect = callback.rect;
        match texture {
            Some(texture) if primitive.clip_rect.intersects(rect) => {
                let mut mesh = egui::Mesh::with_texture(texture);
                mesh.add_rect_with_uv(
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                primitive.primitive = egui::epaint::Primitive::Mesh(mesh);
            }
            _ => {
                primitive.primitive = egui::epaint::Primitive::Mesh(egui::Mesh::default());
                // an empty mesh with an empty clip: egui-wgpu binds the
                // zero-length buffer of an empty mesh otherwise (preview.rs)
                primitive.clip_rect = egui::Rect::NOTHING;
            }
        }
    }
}

/// The blended faces back to front from `eye`, past the first
/// `lists.blend_depth` (the rigged runs, whose order is that of their
/// attachment points, not a distance), with the per-entry flags that go
/// with them. The window's lists are sorted from its own camera: a face
/// before our avatar for it is behind it for a camera that faces us. Equal
/// distances keep their order.
pub(crate) fn resort_blend(lists: &mut DrawLists, eye: Vec3) {
    let start = lists.blend_depth.min(lists.blend.len());
    let n = lists.blend.len();
    if n - start < 2 {
        return;
    }
    let distance = |c: &DrawCmd| eye.distance_squared(Vec3::new(c.bounds[0], c.bounds[1], c.bounds[2]));
    let mut order: Vec<usize> = (start..n).collect();
    order.sort_by(|&a, &b| distance(&lists.blend[b]).total_cmp(&distance(&lists.blend[a])));
    let permute = |v: &mut Vec<bool>| {
        if v.len() == n {
            let sorted: Vec<bool> = order.iter().map(|&i| v[i]).collect();
            v[start..].copy_from_slice(&sorted);
        }
    };
    permute(&mut lists.blend_glow);
    permute(&mut lists.blend_double_sided);
    let sorted: Vec<DrawCmd> = order.iter().map(|&i| lists.blend[i]).collect();
    lists.blend[start..].copy_from_slice(&sorted);
}

/// The window's packet made over for the second camera, before the window
/// draws it (the copy shares the scene; the window's packet keeps the
/// frame's writes and uploads, which it applies first).
pub(crate) fn packet(main: &FramePacket, job: &SecondViewJob) -> FramePacket {
    let mut lists = DrawLists::default();
    lists.copy_from(&main.lists);
    lists.strip_viewer_overlays();
    // the cards face the window's camera and come from its atlas
    lists.impostor_sprites.clear();
    lists.impostor_captures.clear();
    resort_blend(&mut lists, job.eye);
    let [width, height] = job.size;
    if let Some(gpu) = &mut lists.gpu {
        gpu.planes = crate::gpu_cull::view_planes(job.proj * job.view);
        gpu.eye = job.eye;
        gpu.pixel_scale = height as f32 * job.proj.y_axis.y;
        // the whole avatar: the second camera is not in mouselook
        gpu.first_person = None;
    }
    let mut params = main.params.clone();
    params.view = job.view;
    params.proj = job.proj;
    params.camera_pos = job.eye;
    params.fov_y = job.fov_y;
    params.photo_tile = Mat4::IDENTITY;
    params.photo_scale = 1.0;
    params.photo_context = crate::types::PhotoContext::None;
    params.part_pick = None;
    // focused for the window's camera
    params.dof = None;
    // the probes are the window's (`Probes::share_cubes`): its renderer
    // captures them
    params.probes.capture = None;
    FramePacket {
        output: None,
        index: main.index,
        writes: Default::default(),
        uploads: main.uploads.empty_sibling(),
        scene: main.scene.clone(),
        params,
        lists,
        ui: None,
        outside: Default::default(),
        surface: SurfaceState {
            width,
            height,
            vsync: main.surface.vsync,
            // its own window: 0 x 0 is minimised, as the main window
            minimised: width == 0 || height == 0,
        },
        settings: main.settings,
        hover: None,
        picks: Vec::new(),
        drawn: Vec::new(),
        drawn_forgotten: Vec::new(),
        capture: None,
        equirect: None,
        histogram: None,
        thumbnail: None,
        second_view: None,
        motion_cut: main.motion_cut,
        // (each renderer moves its weather memory once, the window's from
        // its own packet)
        world_shift: main.world_shift,
        // (traced by the window's renderer only: `rt_shadows::traced`)
        rt: None,
        input_at: None,
    }
}

/// The picture's texture and the egui id it is shown by.
pub(crate) struct Target {
    pub texture: wgpu::Texture,
    pub id: egui::TextureId,
    pub size: [u32; 2],
}

impl Target {
    /// A texture of the renderer's output `format` that the second backend
    /// draws into, shown by egui through its plain view, as the previews
    /// are (preview.rs): the post pass writes display-encoded values, which
    /// egui puts on the window as they are. An sRGB view would decode them
    /// once more and darken the picture (a 0.35 grey shown as 0.10).
    pub fn new(device: &wgpu::Device, egui: &mut egui_wgpu::Renderer, format: wgpu::TextureFormat, size: [u32; 2]) -> Target {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("second camera"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let id = egui.register_native_texture(device, &view, wgpu::FilterMode::Linear);
        Target { texture, id, size }
    }
}

/// Frames without a request after which the second backend is let go
/// (its targets and shadow atlas): a window closed, not one that skipped a
/// frame (a photo, a minimised window).
pub(crate) const IDLE_FRAMES: u32 = 120;

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(record: u32, x: f32, radius: f32) -> DrawCmd {
        DrawCmd {
            index_count: 3,
            first_index: 0,
            base_vertex: 0,
            record,
            bounds: [x, 0.0, 0.0, radius],
        }
    }

    #[test]
    fn blended_faces_go_back_to_front_from_the_second_camera() {
        // two rigged runs first, then three faces sorted for a camera at
        // x = -10 (the far one first)
        let mut lists = DrawLists {
            blend: vec![
                cmd(1, 0.0, 1.0),
                cmd(2, 0.0, 1.0),
                cmd(3, 5.0, 1.0),
                cmd(4, 1.0, 1.0),
                cmd(5, -2.0, 1.0),
            ],
            blend_depth: 2,
            blend_glow: vec![false, false, true, false, false],
            blend_double_sided: vec![false, true, false, false, true],
            ..Default::default()
        };
        // a camera at x = +10, facing the other way
        resort_blend(&mut lists, Vec3::new(10.0, 0.0, 0.0));
        let records: Vec<u32> = lists.blend.iter().map(|c| c.record).collect();
        assert_eq!(records, [1, 2, 5, 4, 3]);
        // the flags follow their faces
        assert_eq!(lists.blend_glow, [false, false, false, false, true]);
        assert_eq!(lists.blend_double_sided, [false, true, true, false, false]);
    }

    #[test]
    fn equal_distances_and_short_lists_keep_their_order() {
        let mut lists = DrawLists {
            blend: vec![cmd(1, 3.0, 1.0), cmd(2, -3.0, 1.0)],
            ..Default::default()
        };
        resort_blend(&mut lists, Vec3::ZERO);
        assert_eq!(lists.blend.iter().map(|c| c.record).collect::<Vec<_>>(), [1, 2]);
        lists.blend = vec![cmd(7, 1.0, 1.0)];
        resort_blend(&mut lists, Vec3::new(9.0, 0.0, 0.0));
        assert_eq!(lists.blend[0].record, 7);
        // every face rigged: nothing to sort
        lists.blend = vec![cmd(1, 1.0, 1.0), cmd(2, 9.0, 1.0)];
        lists.blend_depth = 5;
        resort_blend(&mut lists, Vec3::ZERO);
        assert_eq!(lists.blend.iter().map(|c| c.record).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn flags_of_another_length_are_left_alone() {
        let mut lists = DrawLists {
            blend: vec![cmd(1, 1.0, 1.0), cmd(2, 9.0, 1.0)],
            blend_glow: vec![true],
            ..Default::default()
        };
        resort_blend(&mut lists, Vec3::ZERO);
        assert_eq!(lists.blend.iter().map(|c| c.record).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(lists.blend_glow, [true]);
    }

    #[test]
    fn large_pictures_shrink_and_keep_their_shape() {
        assert_eq!(target_size(egui::vec2(300.0, 400.0), 1.0), [300, 400]);
        assert_eq!(target_size(egui::vec2(300.0, 400.0), 2.0), [600, 800]);
        let [w, h] = target_size(egui::vec2(1600.0, 900.0), 1.5);
        assert_eq!(w, MAX_SIDE);
        assert!(((w as f32 / h as f32) - 16.0 / 9.0).abs() < 0.01, "{w}×{h}");
        assert_eq!(target_size(egui::Vec2::ZERO, 1.0), [1, 1]);
    }
}

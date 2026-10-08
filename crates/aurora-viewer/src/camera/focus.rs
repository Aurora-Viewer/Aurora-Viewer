//! Focus geometry of the alt-camera on an object: where the focus point goes
//! inside the object, and how close the camera may come before the field of
//! view narrows instead.
//!
//! Port of LLAgentCamera::calcFocusOffset and calcCameraMinDistance
//! (indra/newview/llagentcamera.cpp), originally LGPL 2.1.

use glam::{Quat, Vec3};

/// The object the camera is focused on, in render space.
#[derive(Debug, Clone, Copy)]
pub struct FocusObject {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub avatar: bool,
    pub mesh: bool,
}

const AVATAR_ZOOM_MIN: Vec3 = Vec3::new(0.55, 0.7, 1.15);
const OBJECT_EXTENTS_PADDING: f32 = 0.5;

fn argmax(v: Vec3) -> usize {
    if v.x > v.y && v.x > v.z {
        0
    } else if v.y > v.z {
        1
    } else {
        2
    }
}

/// Point where a ray meets a plane (LLViewerWindow::mousePointOnPlaneGlobal).
fn ray_on_plane(origin: Vec3, dir: Vec3, point: Vec3, normal: Vec3) -> Option<Vec3> {
    let d = dir.dot(normal);
    if d.abs() < 1e-6 {
        return None;
    }
    let t = (point - origin).dot(normal) / d;
    (t.is_finite() && t >= 0.0).then(|| origin + dir * t)
}

/// Focus offset from the object's position for a click at `point` along the
/// cursor ray: a point near the middle of the object so that orbiting
/// tumbles it, pushed toward the clicked surface when the camera is close.
pub fn calc_focus_offset(obj: &FocusObject, point: Vec3, ray: (Vec3, Vec3), camera: Vec3, camera_at: Vec3, virtual_camera: Vec3) -> Vec3 {
    if obj.avatar {
        // DEV-30589: no heuristics on avatars
        return point - obj.position;
    }
    let inv = obj.rotation.inverse();
    let extents = obj.scale.max(Vec3::splat(0.001));
    let to_cam = (inv * (obj.position - camera)).normalize_or_zero();
    // the object's axial plane most facing the camera
    let axis = argmax((to_cam / extents).abs());
    let normal = obj.rotation * Vec3::AXES[axis];
    let focus_pt = ray_on_plane(ray.0, ray.1, obj.position, normal).unwrap_or(point);
    let camera_to_focus = inv * (focus_pt - camera);
    let mut offset = inv * (focus_pt - obj.position);
    // back inside the bounding box along the camera -> focus line
    let mut clip = Vec3::ZERO;
    for i in 0..3 {
        let out = if offset[i] > 0.0 {
            (offset[i] - extents[i] * 0.5).max(0.0)
        } else {
            (offset[i] + extents[i] * 0.5).min(0.0)
        };
        clip[i] = if camera_to_focus[i].abs() < 0.0001 {
            0.0
        } else {
            out / camera_to_focus[i]
        };
    }
    offset -= clip[argmax(clip.abs())] * camera_to_focus;
    let offset = obj.rotation * offset;
    // close to the object: toward the clicked surface; far: its middle
    let rel = point - obj.position;
    let rel_dist = rel.dot(camera_at).abs();
    let view_dist = (obj.position + rel).distance(camera);
    let local_cam = inv * (virtual_camera - obj.position);
    let inside = (0..3).all(|i| local_cam[i].abs() <= obj.scale[i] * 0.5);
    if inside || view_dist <= 0.0 {
        return rel;
    }
    let bias = ((rel_dist / view_dist - 0.1) / (0.7 - 0.1)).clamp(0.0, 1.0);
    offset.lerp(rel, bias)
}

/// Closest the camera may come to the focused object before the field of
/// view zooms instead (LLAgentCamera::calcCameraMinDistance). The flag is
/// false when the focus left the object on the camera's side.
pub fn calc_camera_min_distance(obj: &FocusObject, focus_object_offset: Vec3, camera: Vec3, focus_target: Vec3, near: f32) -> (f32, bool) {
    if obj.mesh {
        return (0.0, true);
    }
    let inv = obj.rotation.inverse();
    let target_offset = inv * focus_object_offset;
    let camera_offset = inv * (camera - focus_target);
    let (mut extents, soft) = if obj.avatar {
        (obj.scale * AVATAR_ZOOM_MIN, true)
    } else {
        (obj.scale, false)
    };
    let mut outside = false;
    for i in 0..3 {
        if target_offset[i].abs() * 2.0 > extents[i] + OBJECT_EXTENTS_PADDING {
            outside = true;
        }
        if camera_offset[i] > 0.0 {
            extents[i] -= target_offset[i] * 2.0;
        } else {
            extents[i] += target_offset[i] * 2.0;
        }
    }
    let extents = extents.max(Vec3::splat(0.001));
    let cam_dir = camera_offset.abs().max(Vec3::splat(0.001)).normalize();
    let i = argmax(cam_dir / extents);
    let mut min = if cam_dir[i] < 0.001 {
        extents[i] * 0.5
    } else {
        extents[i] * 0.5 / cam_dir[i]
    };
    let split = Vec3::AXES[argmax(target_offset.abs().normalize_or_zero() / extents)];
    let camera_clip = (camera - obj.position).dot(split);
    let target_clip = target_offset.dot(split);
    if outside && ((camera_clip > 0.0 && target_clip > 0.0) || (camera_clip < 0.0 && target_clip < 0.0)) {
        return (min, false);
    }
    // at most the diagonal of a 10 m cube
    min = min.min(10.0 * 3f32.sqrt());
    (min + near + if soft { 0.1 } else { 0.2 }, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(size: f32) -> FocusObject {
        FocusObject {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::splat(size),
            avatar: false,
            mesh: false,
        }
    }

    #[test]
    fn min_distance_is_half_the_extent_plus_near() {
        let (d, ok) = calc_camera_min_distance(&cube(2.0), Vec3::ZERO, Vec3::new(-5.0, 0.0, 0.0), Vec3::ZERO, 0.1);
        assert!(ok);
        assert!((d - (1.0 + 0.1 + 0.2)).abs() < 1e-4, "{d}");
    }

    #[test]
    fn mesh_objects_have_no_min_distance() {
        let mut m = cube(2.0);
        m.mesh = true;
        assert_eq!(
            calc_camera_min_distance(&m, Vec3::ZERO, Vec3::X * 5.0, Vec3::ZERO, 0.1),
            (0.0, true)
        );
    }

    #[test]
    fn far_click_focuses_inside_the_object() {
        let c = cube(2.0);
        let camera = Vec3::new(-20.0, 0.0, 0.0);
        let point = Vec3::new(-1.0, 0.5, 0.0);
        let dir = (point - camera).normalize();
        let off = calc_focus_offset(&c, point, (camera, dir), camera, Vec3::X, camera);
        // far away: the focus is pulled to the middle depth of the cube
        assert!(off.x.abs() < 0.2, "{off:?}");
        assert!(off.length() <= 1.8);
    }

    #[test]
    fn avatars_keep_the_clicked_point() {
        let mut a = cube(1.0);
        a.avatar = true;
        let p = Vec3::new(0.2, 0.1, 0.6);
        assert_eq!(
            calc_focus_offset(&a, p, (Vec3::X * -4.0, Vec3::X), Vec3::X * -4.0, Vec3::X, Vec3::X * -4.0),
            p
        );
    }
}

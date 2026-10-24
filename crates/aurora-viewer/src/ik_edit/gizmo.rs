//! Bone-only manipulators following LLManipTranslate/LLManipRotate's axes,
//! planes and constrained rings (indra/newview/llmanip{translate,rotate}.cpp).
//! Reuse the build tools' projection, geometry and theme drawing; no object edits.
use super::Projector;
use crate::{
    build::{
        draw::{Painter3d, axis_color, fade},
        geom::{Cam, nearest_on_line, ray_plane, seg_distance},
    },
    theme::Palette,
};
use egui::Pos2;
use glam::{Quat, Vec3, Vec4};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Tool {
    #[default]
    Move,
    Rotate,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Space {
    #[default]
    World,
    Local,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Axis(usize),
    Plane(usize),
    Ring(usize),
}

#[derive(Clone, Copy)]
pub(super) struct Pose {
    pub position: Vec3,
    pub rotation: Quat,
}

#[derive(Clone, Copy)]
pub(super) struct Geometry {
    cam: Cam,
    pivot: Vec3,
    basis: Quat,
    length: f32,
    tool: Tool,
}

/// Recover the current camera, including roll, from the rendered projection.
pub(super) fn camera(proj: &Projector) -> Option<Cam> {
    let inverse = proj.view_proj.inverse();
    let unproject = |x, y, z, w| {
        let p = inverse * Vec4::new(x, y, z, w);
        p.truncate() / p.w
    };
    // A perspective camera's origin projects onto the clip Z axis.
    let eye = unproject(0.0, 0.0, 1.0, 0.0);
    let near = unproject(0.0, 0.0, 1.0, 1.0);
    let up_span = unproject(0.0, 1.0, 1.0, 1.0) - unproject(0.0, -1.0, 1.0, 1.0);
    let fov = 2.0 * (up_span.length() * 0.5 / near.distance(eye)).atan();
    if !eye.is_finite() || !fov.is_finite() || !(0.01..3.13).contains(&fov) {
        return None;
    }
    let mut cam = Cam::new(eye, near, proj.view_proj, fov, proj.width, proj.height, proj.ppp);
    // Keep a rolled camera's actual up/right, too.
    cam.up = up_span.try_normalize()?;
    cam.left = cam.up.cross(cam.at).try_normalize()?;
    Some(cam)
}

impl Geometry {
    pub fn new(proj: &Projector, pivot: Vec3, basis: Quat, tool: Tool) -> Option<Self> {
        let cam = camera(proj)?;
        if cam.depth(pivot) <= 0.05 {
            return None;
        }
        let pixels = if tool == Tool::Move { 50.0 } else { 100.0 } * proj.ppp;
        let length = cam.depth(pivot) * (pixels / cam.height * cam.fov_y).tan();
        Some(Self {
            cam,
            pivot,
            basis,
            length,
            tool,
        })
    }

    fn axis(&self, axis: usize) -> Vec3 {
        self.basis * Vec3::AXES[axis]
    }

    fn plane(&self, axis: usize) -> [Vec3; 4] {
        let signs = (self.basis.inverse() * self.cam.at).signum();
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        let at = |a, b| self.pivot + (self.axis(u) * (a * signs[u]) + self.axis(v) * (b * signs[v])) * self.length;
        [at(0.37, 0.37), at(0.67, 0.37), at(0.67, 0.67), at(0.37, 0.67)]
    }

    fn ring(&self, axis: usize, t: f32) -> Vec3 {
        let a = self.axis(axis);
        let u = a.any_orthonormal_vector();
        self.pivot + (u * t.cos() + a.cross(u) * t.sin()) * self.length
    }

    fn pick(&self, pointer: Pos2) -> Option<Part> {
        let mut best = (8.0, None);
        for axis in 0..3 {
            if self.tool == Tool::Move {
                if self.cam.direction_to(self.pivot).dot(self.axis(axis)).abs() > 0.15 {
                    let q = self.plane(axis).map(|p| self.cam.project(p));
                    if let [Some(a), Some(b), Some(c), Some(d)] = q {
                        let side = |a: Pos2, b: Pos2| (b - a).rot90().dot(pointer - a);
                        let sides = [side(a, b), side(b, c), side(c, d), side(d, a)];
                        if sides.iter().all(|s| *s >= 0.0) || sides.iter().all(|s| *s <= 0.0) {
                            return Some(Part::Plane(axis));
                        }
                    }
                }
                if let (Some(a), Some(b)) = (
                    self.cam.project(self.pivot + self.axis(axis) * self.length * 0.2),
                    self.cam.project(self.pivot + self.axis(axis) * self.length * 1.2),
                ) && a.distance(b) > 12.0
                {
                    let distance = seg_distance(pointer, a, b).0;
                    if distance < best.0 {
                        best = (distance, Some(Part::Axis(axis)));
                    }
                }
            } else {
                for k in 0..96 {
                    let t = std::f32::consts::TAU / 96.0;
                    if let (Some(a), Some(b)) = (
                        self.cam.project(self.ring(axis, k as f32 * t)),
                        self.cam.project(self.ring(axis, (k + 1) as f32 * t)),
                    ) {
                        let distance = seg_distance(pointer, a, b).0;
                        if distance < best.0 {
                            best = (distance, Some(Part::Ring(axis)));
                        }
                    }
                }
            }
        }
        best.1
    }

    fn draw(&self, ctx: &egui::Context, palette: &Palette, active: Option<Part>) {
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("ik-gizmo")));
        let mut p = Painter3d::new(painter, &self.cam, *palette);
        for axis in 0..3 {
            let part = if self.tool == Tool::Move {
                Part::Axis(axis)
            } else {
                Part::Ring(axis)
            };
            let color = if active == Some(part) {
                palette.violet_light
            } else {
                axis_color(palette, axis)
            };
            if self.tool == Tool::Move {
                let dir = self.axis(axis);
                let base = self.pivot + dir * self.length * 0.87;
                p.line_cased(self.pivot, base, color, 4.0);
                p.cone(base, dir, self.length * 0.12, self.length * 0.26, color);
                p.flush();
                p.text(self.pivot + dir * self.length * 1.45, ["X", "Y", "Z"][axis], 12.0, color);
                if self.cam.direction_to(self.pivot).dot(dir).abs() > 0.15 {
                    let quad = self.plane(axis);
                    let color = if active == Some(Part::Plane(axis)) {
                        palette.violet_light
                    } else {
                        axis_color(palette, axis)
                    };
                    p.quad(quad, fade(color, 0.3));
                    p.flush();
                    for k in 0..4 {
                        p.line(quad[k], quad[(k + 1) % 4], color, 2.0);
                    }
                }
            } else {
                for k in 0..96 {
                    let t = std::f32::consts::TAU / 96.0;
                    let a = self.ring(axis, k as f32 * t);
                    let b = self.ring(axis, (k + 1) as f32 * t);
                    let front = ((a + b) * 0.5 - self.pivot).dot(-self.cam.direction_to(self.pivot)) >= 0.0;
                    p.line_cased(
                        a,
                        b,
                        fade(color, if front { 1.0 } else { 0.35 }),
                        if active == Some(part) { 6.0 } else { 4.0 },
                    );
                }
            }
        }
    }
}

enum Motion {
    Axis(f32),
    Plane(Vec3),
    Ring { start: Vec3, last: f32, total: f32 },
    Edge { origin: Pos2, tangent: egui::Vec2, radius: f32 },
}

struct Drag {
    geometry: Geometry,
    part: Part,
    pose: Pose,
    motion: Motion,
}

impl Drag {
    fn new(geometry: Geometry, part: Part, pointer: Pos2, pose: Pose) -> Option<Self> {
        let (o, d) = geometry.cam.ray(pointer.x * geometry.cam.ppp, pointer.y * geometry.cam.ppp);
        let axis = geometry.axis(match part {
            Part::Axis(i) | Part::Plane(i) | Part::Ring(i) => i,
        });
        let pivot = geometry.pivot;
        let motion = match part {
            Part::Axis(_) => Motion::Axis(nearest_on_line(o, d, pivot, pivot + axis)?),
            Part::Plane(_) => Motion::Plane(ray_plane(o, d, pivot, axis)?),
            Part::Ring(_) if axis.dot(d).abs() > 0.15 => Motion::Ring {
                start: (ray_plane(o, d, pivot, axis)? - pivot).try_normalize()?,
                last: 0.0,
                total: 0.0,
            },
            Part::Ring(i) => {
                // Like LLManipRotate's edge-on fallback, use the projected
                // tangent when the mouse ray cannot intersect the ring plane.
                let mut nearest = (f32::MAX, Vec3::ZERO);
                for k in 0..96 {
                    let point = geometry.ring(i, k as f32 * std::f32::consts::TAU / 96.0);
                    if let Some(screen) = geometry.cam.project(point) {
                        let distance = screen.distance(pointer);
                        if distance < nearest.0 {
                            nearest = (distance, point);
                        }
                    }
                }
                let radial = (nearest.1 - pivot).try_normalize()?;
                let screen = geometry.cam.project(nearest.1)?;
                let tangent = geometry.cam.project(nearest.1 + axis.cross(radial) * geometry.length)? - screen;
                let radius = tangent.length();
                if radius < 2.0 {
                    return None;
                }
                Motion::Edge {
                    origin: pointer,
                    tangent: tangent / radius,
                    radius,
                }
            }
        };
        Some(Self {
            geometry,
            part,
            pose,
            motion,
        })
    }

    fn update(&mut self, pointer: Pos2) -> Option<Pose> {
        let g = &self.geometry;
        let (o, d) = g.cam.ray(pointer.x * g.cam.ppp, pointer.y * g.cam.ppp);
        let axis = g.axis(match self.part {
            Part::Axis(i) | Part::Plane(i) | Part::Ring(i) => i,
        });
        let mut pose = self.pose;
        let angle = match &mut self.motion {
            Motion::Axis(start) => {
                pose.position += axis * (nearest_on_line(o, d, g.pivot, g.pivot + axis)? - *start);
                return Some(pose);
            }
            Motion::Plane(start) => {
                pose.position += ray_plane(o, d, g.pivot, axis)? - *start;
                return Some(pose);
            }
            Motion::Ring { start, last, total } => {
                let current = (ray_plane(o, d, g.pivot, axis)? - g.pivot).try_normalize()?;
                let angle = axis.dot(start.cross(current)).atan2(start.dot(current));
                let delta = angle - *last;
                *total += delta.sin().atan2(delta.cos());
                *last = angle;
                *total
            }
            Motion::Edge { origin, tangent, radius } => (pointer - *origin).dot(*tangent) / *radius,
        };
        pose.rotation = (Quat::from_axis_angle(axis, angle) * pose.rotation).normalize();
        Some(pose)
    }
}

#[derive(Default)]
pub(super) struct Gizmo {
    drag: Option<Drag>,
}

impl Gizmo {
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }
    pub fn cancel(&mut self) {
        self.drag = None;
    }

    /// Returns ownership of the pointer and an optional world-space edit.
    /// Only begin over the scene: Middle/Foreground windows own covered pixels.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        palette: &Palette,
        geometry: Geometry,
        pose: Pose,
        allow_hover: bool,
    ) -> (bool, Option<(Pose, Tool)>) {
        let (pointer, pressed, down, released, alt) = ctx.input(|i| {
            (
                i.pointer.interact_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.modifiers.alt,
            )
        });
        if alt {
            self.cancel();
        }
        let hover = pointer
            .filter(|p| {
                allow_hover && !alt && ctx.layer_id_at(*p).is_none_or(|l| l.order <= egui::Order::Background)
                // Edge-on rings can cross the pivot: reserve its hit for the
                // selected bone/pole handle, including Ctrl free rotation.
                && geometry.cam.project(geometry.pivot).is_none_or(|c| c.distance(*p) > 9.0)
            })
            .and_then(|p| geometry.pick(p));
        if pressed && let (Some(part), Some(pointer)) = (hover, pointer) {
            self.drag = Drag::new(geometry, part, pointer, pose);
        } else if self.drag.is_none() {
            geometry.draw(ctx, palette, hover);
        }
        let mut change = None;
        let mut owns = hover.is_some();
        if let Some(drag) = &mut self.drag {
            owns = true;
            if (down || released)
                && let Some(pointer) = pointer
            {
                change = drag.update(pointer).map(|pose| (pose, drag.geometry.tool));
            }
            let mut visual = drag.geometry;
            if let Some((pose, Tool::Move)) = change {
                visual.pivot += pose.position - drag.pose.position;
            }
            visual.draw(ctx, palette, Some(drag.part));
            if released || !down {
                self.cancel();
            }
        }
        if owns {
            ctx.set_cursor_icon(if down { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        (owns, change)
    }
}

#[cfg(test)]
mod tests;

//! Local lights of a frame: the prim lights around the camera, as
//! LLPipeline::calcNearbyLights and renderDeferredLighting collect them
//! (indra/newview/pipeline.cpp, originally LGPL 2.1, Linden Research). A
//! light with a light texture is a projector (LLVOVolume::isLightSpotlight):
//! it lights the frustum that leaves its prim, through that texture. Shadow
//! slots follow LLPipeline::generateSunShadow (mShadowSpotLight /
//! mTargetShadowSpotLight), extended to an adjustable budget in Aurora.
//!
//! The Legacy Viewer keeps the 256 nearest lights within the draw distance
//! (RenderLocalLightCount) and draws those in the camera's frustum. The
//! shaders here hold 64 (one bit each in the light sets of `light_tiles`):
//! the lights whose sphere reaches the frustum come first, nearest first,
//! then the others while room is left. Nearest by distance alone, the worn
//! lights of a crowd around and behind the camera filled the list and the
//! lights in front of it went out as it turned (#1032).

use super::{CullView, Scene};
use crate::world::World;
use aurora_render::projector_shadows::{MAX_POINT_SHADOWS, MAX_PROJECTOR_SHADOWS};
use aurora_render::{PointLight, Projector, Renderer, SpotShadow};
use glam::Vec3;
use std::time::Instant;
use uuid::Uuid;

/// Size of the frame's light list (`MAX_LIGHTS` of the shaders), and the
/// top of Preferences › Graphics › "Maximum nearby lights".
pub(crate) const MAX_LIGHTS: usize = 64;
/// Pixels asked of a projected texture: LL keeps it at full size while its
/// light is drawn; the beam of a light spreads it over whole walls.
const PROJECTED_PIXELS: f32 = 512.0;
/// Retain a shadow map until a challenger covers 20% more screen area.
const SHADOW_AREA_RETENTION: f32 = 1.2;
/// A fast camera still leaves at least a quarter-second for each fade.
const MAX_SHADOW_FADE_SPEED: f32 = 4.0;
/// Point lights: a camera turning this fast (radians a second) fades their
/// shadows as fast as one moving `MAX_SHADOW_FADE_SPEED` metres a second.
/// Their ranking turns with the camera, so a glance aside can hand a map to
/// another light, and a one-second fade out then in left the light shining
/// through walls for up to two seconds after the camera turned back.
const TURN_FADE_RATE: f32 = 1.0;
/// Seconds the turn's fade speed holds after the camera stops turning, so
/// the hand-over a turn starts ends quickly too.
const TURN_FADE_HOLD: f32 = 0.5;
/// Point lights: with the camera within this distance of our avatar, their
/// shadows are ranked from the avatar, not from the camera, so orbiting,
/// zooming or turning the camera hands no map from light to light.
const POINT_ANCHOR_NEAR: f32 = 6.0;
/// Point lights: with the camera this far from our avatar or more, they are
/// ranked from the camera, as the camera is then looking at another place.
/// Between the two, the ranking point slides from the avatar to the camera,
/// so zooming out moves it smoothly instead of re-ranking every light at once.
const POINT_ANCHOR_FAR: f32 = 10.0;

/// The camera of a frame, for its lights.
#[derive(Clone, Copy)]
pub struct LightView {
    pub eye: Vec3,
    pub forward: Vec3,
    /// The main view's frustum: the lights that reach it come first.
    pub frustum: CullView,
    /// The draw distance: lights whose sphere starts beyond it are left out
    /// (calcNearbyLights' max_dist, RenderFarClip).
    pub range: f32,
    /// Seconds since the previous frame.
    pub dt: f32,
    /// Projectors cast shadows (the setting, with the sun's shadows on).
    pub projector_shadows: bool,
    pub projector_shadow_limit: u8,
    /// Point lights that may cast shadows (0: none).
    pub point_shadow_limit: u8,
    /// Where our avatar is (render space), which point-light shadows are
    /// ranked from while the camera stays near it (`point_anchor`); None
    /// before it is loaded.
    pub avatar: Option<Vec3>,
    /// Keep lights worn by other residents (Preferences › Graphics).
    pub others_attached_lights: bool,
    /// Lights the frame keeps at most (Preferences › Graphics, "Maximum
    /// nearby lights": RenderLocalLightCount); 0, none.
    pub max_lights: usize,
}

/// A light that may enter the frame's list.
#[derive(Debug, Clone, Copy)]
struct Candidate {
    /// Its sphere reaches the main view's frustum.
    in_view: bool,
    /// From the camera to its sphere (calc_light_dist).
    dist: f32,
    /// Object index.
    idx: usize,
    light: PointLight,
    /// A projector's texture, given a table slot once the light is kept.
    texture: Option<Uuid>,
    /// Worn: a face light sits inside its avatar, which would swallow it.
    worn: bool,
}

/// LLPipeline's calc_light_dist (pipeline.cpp): from the camera to the
/// light's sphere, 0 inside it.
fn light_dist(center: Vec3, radius: f32, eye: Vec3) -> f32 {
    (center.distance(eye) - radius).max(0.0)
}

/// Whether a light gives any light: calc_light_dist leaves out an intensity
/// under 0.001, renderDeferredLighting a colour (times intensity) whose
/// squared length is under 0.001 and a reach of 1 mm or less.
fn gives_light(color: [f32; 4], radius: f32) -> bool {
    let linear = Vec3::new(color[0], color[1], color[2]) * color[3];
    color[3] >= 0.001 && linear.length_squared() >= 0.001 && radius * 1.5 > 0.001
}

/// Keep the frame's lights: those that reach the view first, then the
/// nearest (equal distances in object order, the order they came in), `max`
/// of them at most. The Legacy Viewer's RenderLocalLightCount stops
/// renderDeferredLighting after that many of mNearbyLights, nearest first
/// (indra/newview/pipeline.cpp, originally LGPL 2.1, Linden Research); the
/// list here keeps the lights in view first (#1032), so a small limit keeps
/// the nearest that are seen rather than one behind the camera.
fn select(lights: &mut Vec<Candidate>, max: usize) {
    // (unstable: a stable sort allocates a buffer as long as the list)
    lights.sort_unstable_by(|a, b| b.in_view.cmp(&a.in_view).then(a.dist.total_cmp(&b.dist)).then(a.idx.cmp(&b.idx)));
    lights.truncate(max.min(MAX_LIGHTS));
}

/// The light list's working buffers, kept between frames for their room.
#[derive(Default)]
pub struct Scratch {
    lights: Vec<Candidate>,
    /// Objects whose projector is in the list.
    projecting: Vec<usize>,
    /// `Scene::light_objects`, copied: the loop borrows the scene.
    objects: Vec<usize>,
    shadow_map_count: usize,
    point_map_count: usize,
    /// Point lights in view at the last frame, sorted (`show_entered`).
    point_in_view: Vec<usize>,
}

/// Projectors that cast shadows and how far each has faded in:
/// LLPipeline::generateSunShadow's update of mShadowSpotLight. A projector
/// keeps its shadow map while it is among the `targets`; otherwise its
/// shadows fade out, then the map goes to the target that has none.
#[derive(Debug, Default)]
pub struct SpotShadowSlots {
    /// Object index of the projector of each shadow map.
    pub lights: Vec<Option<usize>>,
    pub fade: Vec<f32>,
    last_eye: Option<Vec3>,
    /// Targets that came into view and wait for a map (`show_entered`).
    entering: Vec<usize>,
    /// Point lights: the camera's last direction, the fade speed of its
    /// turn, and how long that speed still holds (`turn_speed`).
    last_forward: Option<Vec3>,
    turn_speed: f32,
    turn_hold: f32,
}

impl SpotShadowSlots {
    /// LLPipeline ranks the apparent area of projectors. Keep that ranking,
    /// with a small retention margin for the maps already drawn: nearby
    /// lights crossing in priority must not repeatedly trade visible shadows.
    fn targets(&self, ranked: &mut [(f32, usize)], limit: usize) -> [Option<usize>; MAX_PROJECTOR_SHADOWS] {
        let area = |(angle, idx): &(f32, usize)| {
            let retention = if self.lights.contains(&Some(*idx)) {
                SHADOW_AREA_RETENTION
            } else {
                1.0
            };
            angle * angle * retention
        };
        ranked.sort_by(|a, b| area(b).total_cmp(&area(a)).then(a.1.cmp(&b.1)));
        std::array::from_fn(|k| ranked.get(k).filter(|_| k < limit).map(|r| r.1))
    }

    /// `targets`: the projectors that weigh most on screen within the budget.
    /// The fade lasts a second, less as the camera moves faster.
    pub fn update(&mut self, targets: &[Option<usize>], eye: Vec3, dt: f32) {
        self.update_at_least(targets, eye, dt, 1.0);
    }

    /// `update`, the fade going at least as fast as for a camera moving at
    /// `min_speed` metres a second.
    fn update_at_least(&mut self, targets: &[Option<usize>], eye: Vec3, dt: f32, min_speed: f32) {
        let limit = targets.len().min(MAX_PROJECTOR_SHADOWS);
        self.lights.resize(limit, None);
        self.fade.resize(limit, 0.0);
        let speed = self.last_eye.map_or(0.0, |e| e.distance(eye) / dt.max(1e-4));
        self.last_eye = Some(eye);
        let amount = dt * speed.clamp(min_speed, MAX_SHADOW_FADE_SPEED);
        // Fade every old owner first, then fill free slots. Assigning while
        // visiting a slot can duplicate an owner which is still fading out.
        for i in 0..limit {
            if self.lights[i].is_some() && targets.contains(&self.lights[i]) {
                self.fade[i] = (self.fade[i] + amount).min(1.0);
            } else {
                self.fade[i] = (self.fade[i] - amount).max(0.0);
                if self.fade[i] == 0.0 || self.lights[i].is_none() {
                    self.lights[i] = None;
                }
            }
        }
        for target in targets.iter().flatten() {
            if self.lights.contains(&Some(*target)) {
                continue;
            }
            if let Some(slot) = self.lights.iter().position(Option::is_none) {
                self.lights[slot] = Some(*target);
                self.fade[slot] = 0.0;
            }
        }
    }

    /// Point lights: a target that `entered` the view since the last frame
    /// casts its full shadow at once, as soon as it has a map. Its light was
    /// off screen, so there is no unshadowed image to fade from, and a fade
    /// would let it light through walls as the camera turns to it. One that
    /// waits for a map keeps that right until it gets one, while it stays a
    /// target.
    fn show_entered(&mut self, targets: &[Option<usize>], entered: impl Fn(usize) -> bool) {
        self.entering.retain(|idx| targets.contains(&Some(*idx)));
        for idx in targets.iter().flatten().copied() {
            if entered(idx) && !self.entering.contains(&idx) {
                self.entering.push(idx);
            }
        }
        for (owner, fade) in self.lights.iter().zip(self.fade.iter_mut()) {
            if let Some(idx) = *owner
                && let Some(k) = self.entering.iter().position(|e| *e == idx)
            {
                *fade = 1.0;
                self.entering.swap_remove(k);
            }
        }
    }

    /// Point lights: the fade speed of the camera's turn, as `update_at_least`
    /// takes it. A turn fades as fast as a moving camera, up to the same
    /// quarter-second, and keeps that speed for `TURN_FADE_HOLD` after it
    /// stops; a still camera fades in the usual second.
    fn turn_speed(&mut self, forward: Vec3, dt: f32) -> f32 {
        let dt = dt.max(1e-4);
        let angle = self.last_forward.map_or(0.0, |f| f.angle_between(forward));
        self.last_forward = Some(forward);
        let rate = if angle.is_finite() { angle / dt } else { 0.0 };
        let speed = (rate / TURN_FADE_RATE * MAX_SHADOW_FADE_SPEED).min(MAX_SHADOW_FADE_SPEED);
        if speed >= self.turn_speed {
            self.turn_speed = speed;
            self.turn_hold = TURN_FADE_HOLD;
        } else if self.turn_hold > 0.0 {
            self.turn_hold -= dt;
        } else {
            self.turn_speed = speed;
        }
        self.turn_speed.max(1.0)
    }

    fn clear(&mut self) {
        *self = SpotShadowSlots::default();
    }
}

/// LLPipeline::calcPixelArea's angle of a sphere seen from `from` (only the
/// order matters).
fn apparent_size(center: Vec3, radius: f32, from: Vec3) -> f32 {
    let mut dist = center.distance(from);
    if dist < 16.0 {
        // calcPixelArea ramps the distance down for nearby objects
        dist = (dist / 16.0).powi(2) * 16.0;
    }
    (radius * 3f32.sqrt() / dist.max(1e-4)).atan()
}

/// LLVOVolume::updateSpotLightPriority: apparent size of a sphere of the
/// light's radius, one radius down its beam and one radius towards the
/// camera.
fn spot_priority(position: Vec3, direction: Vec3, radius: f32, view: &LightView) -> f32 {
    apparent_size(position + direction * radius - view.forward * radius, radius, view.eye)
}

/// Where point-light shadows are ranked from: (the point, the direction
/// its sphere is moved towards, whether that point is near our avatar).
/// The avatar while the camera is within `POINT_ANCHOR_NEAR` of it, with no
/// direction, as the camera turns around it; the camera from
/// `POINT_ANCHOR_FAR` on, as projectors are; in between a point that slides
/// from one to the other. The camera's own ranking turns with it, and each
/// turn or orbit around the avatar handed maps from light to light: the
/// shadows went out, then came back, as the camera moved.
fn point_anchor(view: &LightView) -> (Vec3, Vec3, bool) {
    let Some(avatar) = view.avatar.filter(|a| a.is_finite()) else {
        return (view.eye, view.forward, false);
    };
    let t = (avatar.distance(view.eye) - POINT_ANCHOR_NEAR) / (POINT_ANCHOR_FAR - POINT_ANCHOR_NEAR);
    if t.is_nan() || t >= 1.0 {
        return (view.eye, view.forward, false);
    }
    let t = t.max(0.0);
    let t = t * t * (3.0 - 2.0 * t);
    (avatar.lerp(view.eye, t), view.forward * t, true)
}

/// Whether point-light shadows may be ranked from our avatar
/// (`point_anchor`): `AURORA_POINT_SHADOW_ANCHOR=0` ranks them from the
/// camera alone, as before, to compare.
pub fn point_anchor_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("AURORA_POINT_SHADOW_ANCHOR").as_deref() != Ok("0"))
}

/// Weight of a point light on the image: its apparent size from `from`, its
/// sphere moved one radius towards `forward` as `spot_priority` does, times
/// the square root of its brightness (`targets` squares this, so a light
/// counts by the light it sends).
fn point_priority(light: &PointLight, from: Vec3, forward: Vec3) -> f32 {
    let radius = light.radius / 1.5;
    apparent_size(light.position - forward * radius, radius, from) * light.color.length().sqrt()
}

/// The point lights that may take a shadow map, with their weight
/// (`point_priority` from `point_anchor`): those whose light reaches the
/// view, and, ranked from near our avatar, those that hold a map (`held`)
/// even out of it. The map of a light the camera turned away from then
/// waits for it, and its shadow is whole when the camera turns back; the
/// faces of a light out of view are not drawn (`point_shadow_faces`), so
/// the map costs nothing meanwhile.
fn point_ranking(lights: &[Candidate], view: &LightView, held: &[Option<usize>]) -> Vec<(f32, usize)> {
    let (from, forward, anchored) = point_anchor(view);
    lights
        .iter()
        .filter(|c| !c.worn && c.light.projector.is_none())
        .filter(|c| c.in_view || (anchored && held.contains(&Some(c.idx))))
        .map(|c| (point_priority(&c.light, from, forward), c.idx))
        .collect()
}

/// The avatar that wears the light, if any: a parent up its link is one
/// (the attachment's root, or the prim itself; LLViewerObject::isAttachment).
/// A seat's linkset and animesh are not worn: their avatars are children of
/// the prims, not parents.
fn worn(world: &World, o: &crate::world::objects::Object) -> Option<Uuid> {
    let mut current = o;
    for _ in 0..4 {
        let parent = world.objects.parent_of(current).and_then(|i| world.objects.get(i))?;
        if parent.is_avatar() {
            return Some(parent.full_id);
        }
        current = parent;
    }
    None
}

/// Whether a light enters the frame's list. The photo window's switch
/// (`Scene::attached_lights`) drops every worn light, as the Legacy Viewer's
/// RenderAttachedLights does; the Graphics preference (#2474) drops only
/// those of other avatars and keeps ours, beyond the Legacy Viewer's
/// all-or-nothing setting.
fn light_visible(wearer: Option<Uuid>, agent: Uuid, photo_attached: bool, others_attached: bool) -> bool {
    wearer.is_none() || (photo_attached && (others_attached || wearer == Some(agent)))
}

impl Scene {
    /// The lights that reach the view, nearest first, then the other
    /// nearest ones (`select`), and those of them that cast shadows.
    pub fn local_lights(
        &mut self,
        world: &World,
        renderer: &mut Renderer,
        view: &LightView,
    ) -> (
        Vec<PointLight>,
        [Option<SpotShadow>; MAX_PROJECTOR_SHADOWS],
        [Option<SpotShadow>; MAX_POINT_SHADOWS],
    ) {
        let eye = view.eye;
        let now = Instant::now();
        let Scratch {
            mut lights,
            mut projecting,
            mut objects,
            shadow_map_count,
            point_map_count,
            mut point_in_view,
        } = std::mem::take(&mut self.light_scratch);
        // with no local light allowed, none is even looked at (the Legacy
        // Viewer's calcNearbyLights and renderDeferredLighting skip them all
        // at RenderLocalLightCount 0)
        if view.max_lights > 0 {
            objects.extend(self.light_objects.iter().copied());
        }
        for idx in objects.drain(..) {
            if !world.parcel_object_visible(idx) {
                continue;
            }
            let Some(o) = world.objects.get(idx) else {
                continue;
            };
            let Some(l) = o.extra.light else {
                continue;
            };
            let Some((center, hud)) = self.gpu.get(idx).map(|g| (g.center, g.hud)) else {
                continue;
            };
            if hud || !gives_light(l.color, l.radius) {
                continue;
            }
            // the wearer is looked up only while a switch drops worn lights
            // or point lights cast shadows (with both on and no point
            // shadows, every light is kept: no walk up its links)
            // (the 360° snapshot's "Hide all avatars" also turns the worn
            // lights off, LLFloater360Capture's sRenderAttachedLights)
            let photo_attached = self.attached_lights && !world.photo_isolation.hide_avatars;
            let filtered = !photo_attached || !view.others_attached_lights;
            let wearer = if filtered || view.point_shadow_limit > 0 {
                worn(world, o)
            } else {
                None
            };
            let is_worn = wearer.is_some();
            if filtered && !light_visible(wearer, world.agent_id, photo_attached, view.others_attached_lights) {
                continue;
            }
            let dist = light_dist(center, l.radius, eye);
            if dist >= view.range {
                continue;
            }
            // deferred SL lights reach 1.5x their radius with half the
            // falloff (pipeline.cpp: getLightRadius() * 1.5,
            // getLightFalloff(DEFERRED_LIGHT_FALLOFF = 0.5))
            let radius = (l.radius * 1.5).clamp(0.1, 30.0);
            let mut position = center;
            let mut projector = None;
            let mut texture = None;
            if let Some(image) = o.visual_light_image().filter(|i| !i.texture.is_nil())
                && let Some((pos, rot, _)) = Self::object_transform(world, idx, now, 0)
            {
                // the frustum leaves the prim itself, not its drawn bounds;
                // its texture slot is taken once the light is kept
                position = pos;
                texture = Some(image.texture);
                projector = Some(Projector::new(pos, rot, o.visual_scale(), radius, image.params, 0));
            }
            // the packed light color is already linear (LLLightParams::unpack ->
            // setLinearColor; pipeline.cpp sends getLightLinearColor() as is),
            // times 3.25 for the PBR punctual intensity (multiPointLightF)
            let color = Vec3::new(l.color[0], l.color[1], l.color[2]) * l.color[3] * 3.25;
            lights.push(Candidate {
                in_view: view.frustum.sphere_visible(position, radius),
                dist,
                idx,
                light: PointLight {
                    position,
                    radius,
                    color,
                    falloff: l.falloff * 0.5,
                    extent: o.visual_scale().length() * 0.5,
                    projector,
                },
                texture,
                worn: is_worn,
            });
        }
        select(&mut lights, view.max_lights);
        for c in &mut lights {
            if let (Some(id), Some(p)) = (c.texture, c.light.projector.as_mut()) {
                p.texture = self.projected_texture(renderer, c.idx, id);
                projecting.push(c.idx);
            }
        }
        // textures of the projectors that left the list
        let textures = &mut self.textures;
        self.light_textures.retain(|idx, (id, _)| {
            let kept = projecting.contains(idx);
            if !kept {
                textures.release(id);
            }
            kept
        });
        // the projectors of the list that weigh most on screen, among
        // those that reach into the half-space in front of the camera
        if view.projector_shadows {
            let mut ranked: Vec<(f32, usize)> = lights
                .iter()
                .filter_map(|Candidate { idx, light: l, .. }| {
                    let p = l.projector?;
                    let ahead = view.forward.dot(l.position - eye) > -l.radius;
                    ahead.then(|| (spot_priority(l.position, p.direction, l.radius / 1.5, view), *idx))
                })
                .collect();
            let limit = usize::from(view.projector_shadow_limit).min(MAX_PROJECTOR_SHADOWS);
            let targets = self.spot_shadows.targets(&mut ranked, limit);
            self.spot_shadows.update(&targets[..limit], eye, view.dt);
            if std::env::var_os("AURORA_PROJECTOR_TRACE").is_some() {
                log::info!(
                    "projector trace: eye={eye:?} targets={targets:?} slots={:?} fade={:?} ranked={ranked:?}",
                    self.spot_shadows.lights,
                    self.spot_shadows.fade
                );
            }
        } else {
            self.spot_shadows.clear();
        }
        let slots = &self.spot_shadows;
        let shadows = std::array::from_fn(|k| {
            let idx = slots.lights.get(k).copied().flatten()?;
            let light = lights.iter().position(|c| c.idx == idx && c.light.projector.is_some())?;
            Some(SpotShadow {
                light: light as u32,
                fade: slots.fade[k],
            })
        });
        // the point lights that weigh most around our avatar, or on what the
        // camera sees when it is away from the avatar (`point_anchor`)
        if view.point_shadow_limit > 0 {
            let mut ranked = point_ranking(&lights, view, &self.point_shadows.lights);
            let limit = usize::from(view.point_shadow_limit).min(MAX_POINT_SHADOWS);
            let targets = self.point_shadows.targets(&mut ranked, limit);
            let turn = self.point_shadows.turn_speed(view.forward, view.dt);
            self.point_shadows.update_at_least(&targets[..limit], eye, view.dt, turn);
            self.point_shadows
                .show_entered(&targets[..limit], |idx| point_in_view.binary_search(&idx).is_err());
            if std::env::var_os("AURORA_PROJECTOR_TRACE").is_some() {
                log::info!(
                    "point trace: eye={eye:?} avatar={:?} targets={:?} slots={:?} fade={:?}",
                    view.avatar,
                    &targets[..limit],
                    self.point_shadows.lights,
                    self.point_shadows.fade
                );
            }
        } else {
            self.point_shadows.clear();
        }
        point_in_view.clear();
        point_in_view.extend(lights.iter().filter(|c| c.in_view && c.light.projector.is_none()).map(|c| c.idx));
        point_in_view.sort_unstable();
        let slots = &self.point_shadows;
        let point_shadows = std::array::from_fn(|k| {
            let idx = slots.lights.get(k).copied().flatten()?;
            // a light just put on keeps no map while its slot fades out
            let light = lights.iter().position(|c| c.idx == idx && !c.worn && c.light.projector.is_none())?;
            Some(SpotShadow {
                light: light as u32,
                fade: slots.fade[k],
            })
        });
        let point_active = point_shadows.iter().flatten().count();
        if point_active != point_map_count {
            log::info!("point light shadow maps active: {point_active}, limit {}", view.point_shadow_limit);
        }
        let active = shadows.iter().flatten().count();
        if active != shadow_map_count {
            log::info!("projector shadow maps active: {active}, limit {}", view.projector_shadow_limit);
        }
        let list = lights.iter().map(|c| c.light).collect();
        lights.clear();
        projecting.clear();
        self.light_scratch = Scratch {
            lights,
            projecting,
            objects,
            shadow_map_count: active,
            point_map_count: point_active,
            point_in_view,
        };
        (list, shadows, point_shadows)
    }

    /// Texture table slot of the texture object `idx` projects, held while
    /// the object stays in the light list.
    fn projected_texture(&mut self, renderer: &mut Renderer, idx: usize, id: Uuid) -> u32 {
        let slot = match self.light_textures.get(&idx) {
            Some((held, slot)) if *held == id => *slot,
            _ => {
                let slot = self.textures.acquire(renderer, id, super::textures::TexSource::Asset);
                if let Some((old, _)) = self.light_textures.insert(idx, (id, slot)) {
                    self.textures.release(&old);
                }
                slot
            }
        };
        self.textures.note_usage(&id, PROJECTED_PIXELS);
        slot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_avatar_lights_can_be_hidden_without_hiding_our_own() {
        let agent = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        // preference off: ours kept, others dropped, unworn kept
        assert!(light_visible(Some(agent), agent, true, false));
        assert!(!light_visible(Some(other), agent, true, false));
        assert!(light_visible(None, agent, true, false));
        assert!(light_visible(Some(other), agent, true, true));
        // photo switch off: every worn light dropped, whatever the preference
        for others in [true, false] {
            assert!(!light_visible(Some(agent), agent, false, others));
            assert!(!light_visible(Some(other), agent, false, others));
            assert!(light_visible(None, agent, false, others));
        }
    }

    /// #2474: the wearer is found from a child prim of an attachment, ours
    /// by the agent id; a seat's linkset is not worn by its sitter.
    #[test]
    fn the_wearer_of_a_light_is_the_avatar_above_its_linkset() {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for ev in crate::demo::events().into_iter().chain(crate::demo::action_events()) {
            w.apply(ev);
        }
        let own = w.objects.index_of_uuid(&w.agent_id).expect("our avatar");
        let own_local = w.objects.get(own).unwrap().key.local_id;
        let (other, other_id) = w
            .objects
            .avatars()
            .find(|(_, o)| o.full_id != w.agent_id)
            .map(|(_, o)| (o.key.local_id, o.full_id))
            .expect("another demo resident");
        let root = w.objects.index_of_uuid(&crate::demo::action_id(971)).unwrap();
        let child = w.objects.index_of_uuid(&crate::demo::action_id(974)).unwrap();
        let wearer = |w: &World, idx| worn(w, w.objects.get(idx).unwrap());
        assert_eq!((wearer(&w, root), wearer(&w, child)), (None, None));
        // we sit on it: the avatar is the child, the prims stay unworn
        let root_local = w.objects.get(root).unwrap().key.local_id;
        w.objects.get_mut(own).unwrap().parent_id = root_local;
        assert_eq!((wearer(&w, root), wearer(&w, child)), (None, None));
        w.objects.get_mut(own).unwrap().parent_id = 0;
        w.objects.get_mut(root).unwrap().parent_id = other;
        assert_eq!((wearer(&w, root), wearer(&w, child)), (Some(other_id), Some(other_id)));
        w.objects.get_mut(root).unwrap().parent_id = own_local;
        assert_eq!(wearer(&w, child), Some(w.agent_id));
    }

    #[test]
    fn a_projector_keeps_its_shadow_map_while_it_is_a_target() {
        let mut slots = SpotShadowSlots::default();
        let eye = Vec3::ZERO;
        // two targets take the two maps, then fade in over a second
        slots.update(&[Some(7), Some(9)], eye, 0.1);
        assert_eq!(slots.lights, [Some(7), Some(9)]);
        assert_eq!(slots.fade, [0.0, 0.0]);
        for _ in 0..5 {
            slots.update(&[Some(9), Some(7)], eye, 0.1);
        }
        // (whatever their order among the targets)
        assert_eq!(slots.lights, [Some(7), Some(9)]);
        assert!((slots.fade[0] - 0.5).abs() < 1e-5 && (slots.fade[1] - 0.5).abs() < 1e-5);
        for _ in 0..10 {
            slots.update(&[Some(7), Some(9)], eye, 0.1);
        }
        assert_eq!(slots.fade, [1.0, 1.0]);
    }

    #[test]
    fn a_new_target_waits_for_the_fade_out_of_the_one_it_replaces() {
        let mut slots = SpotShadowSlots::default();
        let eye = Vec3::ZERO;
        for _ in 0..12 {
            slots.update(&[Some(1), Some(2)], eye, 0.1);
        }
        // 3 outranks 2: 1 keeps its map, 2 fades out before 3 takes its
        slots.update(&[Some(1), Some(3)], eye, 0.25);
        assert_eq!(slots.lights, [Some(1), Some(2)]);
        assert_eq!(slots.fade, [1.0, 0.75]);
        for _ in 0..3 {
            slots.update(&[Some(1), Some(3)], eye, 0.25);
        }
        assert_eq!(slots.lights, [Some(1), Some(3)]);
        assert_eq!(slots.fade, [1.0, 0.0]);
        slots.update(&[Some(1), Some(3)], eye, 0.25);
        assert_eq!(slots.fade, [1.0, 0.25]);
        // a moving camera shortens the fade (2 m in 0.25 s: 8 m/s)
        slots.update(&[Some(1), Some(3)], Vec3::new(2.0, 0.0, 0.0), 0.25);
        assert_eq!(slots.fade, [1.0, 1.0]);
        // a single target: the other map is given up
        for _ in 0..5 {
            slots.update(&[Some(1), None], Vec3::new(2.0, 0.0, 0.0), 0.25);
        }
        assert_eq!(slots.lights, [Some(1), None]);
    }

    #[test]
    fn a_camera_jump_cannot_remove_a_full_projector_shadow_in_one_frame() {
        let mut slots = SpotShadowSlots::default();
        for _ in 0..70 {
            slots.update(&[Some(1), Some(2)], Vec3::ZERO, 0.016);
        }
        assert_eq!(slots.fade, [1.0, 1.0]);
        slots.update(&[Some(1), Some(3)], Vec3::new(8.0, 0.0, 0.0), 0.016);
        assert_eq!(slots.lights, [Some(1), Some(2)]);
        assert!((slots.fade[1] - 0.936).abs() < 1e-5);
        // Normal stationary-camera fades still take one second.
        slots.update(&[Some(1), Some(3)], Vec3::new(8.0, 0.0, 0.0), 0.016);
        assert!((slots.fade[1] - 0.920).abs() < 1e-5);
    }

    #[test]
    fn nearby_projectors_keep_their_shadow_maps_until_a_clear_priority_change() {
        let slots = SpotShadowSlots {
            lights: vec![Some(1), Some(2)],
            fade: vec![1.0, 1.0],
            ..Default::default()
        };
        for incoming in [0.98, 1.01, 1.04, 1.01, 0.98] {
            let mut ranked = [(incoming, 3), (1.0, 1), (0.99, 2)];
            let targets = slots.targets(&mut ranked, 2);
            assert!(targets.contains(&Some(1)) && targets.contains(&Some(2)));
        }
        let mut ranked = [(1.11, 3), (1.0, 1), (0.99, 2)];
        let targets = slots.targets(&mut ranked, 2);
        assert!(targets.contains(&Some(1)) && targets.contains(&Some(3)));
    }

    #[test]
    fn projector_shadow_retention_never_keeps_a_missing_candidate() {
        let slots = SpotShadowSlots {
            lights: vec![Some(1), Some(2)],
            ..Default::default()
        };
        assert_eq!(slots.targets(&mut [(0.3, 3)], 2)[..2], [Some(3), None]);
        assert_eq!(slots.targets(&mut [], 2), [None; MAX_PROJECTOR_SHADOWS]);
    }

    #[test]
    fn ten_projector_shadows_keep_distinct_slots_and_follow_budget_changes() {
        let targets: Vec<_> = (0..10).map(Some).collect();
        let mut slots = SpotShadowSlots::default();
        for _ in 0..12 {
            slots.update(&targets, Vec3::ZERO, 0.1);
        }
        assert_eq!(slots.lights, targets);
        assert!(slots.fade.iter().all(|&fade| fade == 1.0));
        let reversed: Vec<_> = targets.iter().rev().copied().collect();
        slots.update(&reversed, Vec3::ZERO, 0.1);
        assert_eq!(slots.lights, targets, "ranking changes cannot move retained maps");
        slots.update(&targets[..2], Vec3::ZERO, 0.1);
        assert_eq!(slots.lights, targets[..2]);
        slots.update(&targets, Vec3::ZERO, 0.1);
        assert_eq!(slots.lights, targets);
        assert_eq!(slots.fade[..2], [1.0, 1.0]);
        assert!(slots.fade[2..].iter().all(|&fade| fade == 0.0));
    }

    /// A camera at `eye` looking along `forward`, 60° of vertical field, 16:9.
    fn camera(eye: Vec3, forward: Vec3) -> LightView {
        let fov = 60f32.to_radians();
        let view = glam::camera::rh::view::look_to_mat4(eye, forward, Vec3::Z);
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(fov, 16.0 / 9.0, 0.1);
        LightView {
            eye,
            forward,
            frustum: CullView::new(proj * view, eye, 900.0, fov),
            range: 128.0,
            dt: 0.016,
            projector_shadows: true,
            projector_shadow_limit: 2,
            point_shadow_limit: 0,
            avatar: None,
            others_attached_lights: true,
            max_lights: MAX_LIGHTS,
        }
    }

    fn candidate(view: &LightView, idx: usize, position: Vec3, radius: f32) -> Candidate {
        Candidate {
            in_view: view.frustum.sphere_visible(position, radius * 1.5),
            dist: light_dist(position, radius, view.eye),
            idx,
            light: PointLight {
                extent: 0.0,
                position,
                radius: radius * 1.5,
                color: Vec3::ONE,
                falloff: 0.0,
                projector: None,
            },
            texture: None,
            worn: false,
        }
    }

    /// #1032: a crowd's worn lights around and behind the camera no longer
    /// push out the lights it looks at, whichever way it turns.
    #[test]
    fn the_lights_in_view_come_before_nearer_ones_out_of_it() {
        for forward in [Vec3::X, Vec3::NEG_Y, Vec3::new(1.0, 0.0, -1.0).normalize()] {
            let view = camera(Vec3::new(128.0, 128.0, 25.0), forward);
            let side = forward.cross(Vec3::Z).normalize_or(Vec3::Y);
            let mut lights = Vec::new();
            // 80 small worn lights within 4 m behind the camera
            for i in 0..80 {
                let p = view.eye - forward * (3.0 + (i % 8) as f32 * 0.1) + side * ((i / 8) as f32 * 0.3 - 1.5);
                lights.push(candidate(&view, i, p, 0.5));
            }
            // 10 lights 5 to 20 m in front of it
            for i in 0..10 {
                let p = view.eye + forward * (5.0 + i as f32 * 1.5) + side * (i as f32 - 5.0) * 0.5;
                lights.push(candidate(&view, 100 + i, p, 3.0));
            }
            select(&mut lights, MAX_LIGHTS);
            assert_eq!(lights.len(), MAX_LIGHTS);
            let kept: Vec<usize> = lights.iter().map(|c| c.idx).collect();
            assert_eq!(kept[..10], (100..110).collect::<Vec<_>>(), "looking along {forward}");
            // the room left goes to the nearest of the others
            assert!(lights[10..].iter().all(|c| !c.in_view && c.idx < 100));
            assert!(lights[10..].windows(2).all(|w| w[0].dist <= w[1].dist));
        }
    }

    /// "Maximum nearby lights" (RenderLocalLightCount): the list stops at
    /// the limit, the lights in view and nearest kept, none at 0, and never
    /// more than the shaders hold.
    #[test]
    fn the_light_limit_keeps_the_nearest_lights_in_view() {
        let view = camera(Vec3::new(128.0, 128.0, 25.0), Vec3::X);
        let all = || {
            let mut lights: Vec<Candidate> = (0..10)
                .map(|i| candidate(&view, i, view.eye + Vec3::X * (20.0 - i as f32), 1.0))
                .collect();
            // a nearer light behind the camera, out of the view
            lights.push(candidate(&view, 50, view.eye - Vec3::X * 4.0, 0.5));
            // and more than the shaders hold beyond them
            lights.extend((0..80).map(|i| candidate(&view, 100 + i, view.eye + Vec3::X * (40.0 + i as f32), 1.0)));
            lights
        };
        let mut lights = all();
        select(&mut lights, 3);
        let kept: Vec<usize> = lights.iter().map(|c| c.idx).collect();
        assert_eq!(kept, [9, 8, 7], "the three nearest in view, not the one behind");
        let mut lights = all();
        select(&mut lights, 0);
        assert!(lights.is_empty());
        let mut lights = all();
        select(&mut lights, 1000);
        assert_eq!(lights.len(), MAX_LIGHTS);
    }

    #[test]
    fn a_light_behind_the_camera_that_reaches_the_view_is_in_view() {
        let view = camera(Vec3::ZERO, Vec3::X);
        // 2 m behind, 3 m of reach: it lights what is just in front
        assert!(candidate(&view, 0, Vec3::new(-2.0, 0.0, 0.0), 2.0).in_view);
        assert!(!candidate(&view, 0, Vec3::new(-5.0, 0.0, 0.0), 2.0).in_view);
        // beside the field, its sphere crossing the side plane
        assert!(candidate(&view, 0, Vec3::new(4.0, 6.0, 0.0), 3.0).in_view);
    }

    #[test]
    fn distance_to_the_sphere_like_calc_light_dist() {
        assert_eq!(light_dist(Vec3::new(10.0, 0.0, 0.0), 4.0, Vec3::ZERO), 6.0);
        assert_eq!(light_dist(Vec3::new(1.0, 0.0, 0.0), 4.0, Vec3::ZERO), 0.0);
    }

    #[test]
    fn lights_that_give_no_light_are_left_out() {
        assert!(gives_light([1.0, 0.8, 0.5, 1.0], 5.0));
        // no intensity, black, or no reach: they took a place in the list
        assert!(!gives_light([1.0, 1.0, 1.0, 0.0], 5.0));
        assert!(!gives_light([0.0, 0.0, 0.0, 1.0], 5.0));
        assert!(!gives_light([0.01, 0.01, 0.01, 1.0], 5.0));
        assert!(!gives_light([1.0, 1.0, 1.0, 1.0], 0.0));
    }

    #[test]
    fn the_projector_that_weighs_most_on_screen_comes_first() {
        let view = camera(Vec3::ZERO, Vec3::X);
        let down = Vec3::NEG_Z;
        let near = spot_priority(Vec3::new(10.0, 0.0, 3.0), down, 10.0, &view);
        let far = spot_priority(Vec3::new(40.0, 0.0, 3.0), down, 10.0, &view);
        let small = spot_priority(Vec3::new(10.0, 0.0, 3.0), down, 2.0, &view);
        assert!(near > far && near > small, "{near} {far} {small}");
    }

    #[test]
    fn a_point_light_that_enters_the_view_casts_its_full_shadow_at_once() {
        let mut slots = SpotShadowSlots::default();
        for _ in 0..70 {
            slots.update(&[Some(1), None], Vec3::ZERO, 0.016);
        }
        // 2 was off screen, 3 in view without a map: 2 shows at once, 3
        // (given the other map later) fades in
        let targets = [Some(1), Some(2)];
        slots.update(&targets, Vec3::ZERO, 0.016);
        slots.show_entered(&targets, |idx| idx == 2);
        assert_eq!(slots.lights, [Some(1), Some(2)]);
        assert_eq!(slots.fade, [1.0, 1.0]);
        let mut slots = SpotShadowSlots::default();
        for _ in 0..70 {
            slots.update(&[Some(1), None], Vec3::ZERO, 0.016);
        }
        slots.update(&[Some(1), Some(3)], Vec3::ZERO, 0.016);
        slots.show_entered(&[Some(1), Some(3)], |idx| idx == 2);
        assert_eq!(slots.fade, [1.0, 0.0]);
        // a light that left the view is no target: its fade-out goes on
        // until it comes back, then it is whole again
        slots.update(&[Some(3), None], Vec3::ZERO, 0.25);
        slots.show_entered(&[Some(3), None], |idx| idx == 1);
        assert_eq!(slots.fade, [0.75, 0.25]);
        slots.update(&[Some(3), Some(1)], Vec3::ZERO, 0.016);
        slots.show_entered(&[Some(3), Some(1)], |idx| idx == 1);
        assert_eq!(slots.lights, [Some(1), Some(3)]);
        assert_eq!(slots.fade[0], 1.0);
    }

    #[test]
    fn a_light_that_enters_the_view_while_the_maps_are_taken_shows_at_once_when_it_gets_one() {
        let mut slots = SpotShadowSlots::default();
        for _ in 0..70 {
            slots.update(&[Some(1)], Vec3::ZERO, 0.016);
        }
        // 2 comes into view and outranks 1, whose map fades out first
        let targets = [Some(2)];
        slots.update(&targets, Vec3::ZERO, 0.5);
        slots.show_entered(&targets, |idx| idx == 2);
        assert_eq!(slots.lights, [Some(1)]);
        slots.update(&targets, Vec3::ZERO, 0.5);
        slots.show_entered(&targets, |_| false);
        assert_eq!(slots.lights, [Some(2)]);
        assert_eq!(slots.fade, [1.0]);
        // one that stops being a target while it waits loses that right
        let mut slots = SpotShadowSlots::default();
        for _ in 0..70 {
            slots.update(&[Some(1)], Vec3::ZERO, 0.016);
        }
        slots.update(&[Some(2)], Vec3::ZERO, 0.5);
        slots.show_entered(&[Some(2)], |idx| idx == 2);
        slots.update(&[Some(3)], Vec3::ZERO, 0.5);
        slots.show_entered(&[Some(3)], |_| false);
        assert_eq!(slots.lights, [Some(3)]);
        slots.update(&[Some(2)], Vec3::ZERO, 1.0);
        slots.update(&[Some(2)], Vec3::ZERO, 0.016);
        slots.show_entered(&[Some(2)], |_| false);
        assert_eq!(slots.lights, [Some(2)]);
        assert!(slots.fade[0] < 0.1, "{:?}", slots.fade);
    }

    /// The owners a point light's map has, in turn, as the camera circles
    /// our avatar twice at `radius` metres, among three lamps around it;
    /// ranked from the camera alone without `avatar`.
    fn point_map_owners_around(avatar: bool, radius: f32) -> Vec<Option<usize>> {
        let at = Vec3::new(128.0, 128.0, 25.0);
        let lamps = [
            (1, at + Vec3::new(3.0, 0.0, 0.5)),
            (2, at + Vec3::new(-2.5, 2.0, 0.5)),
            (3, at + Vec3::new(0.5, -3.5, 0.5)),
        ];
        let mut slots = SpotShadowSlots::default();
        let mut owners = Vec::new();
        for step in 0..=720u16 {
            let angle = f32::from(step).to_radians();
            let eye = at + Vec3::new(angle.cos() * radius, angle.sin() * radius, 1.5);
            let mut view = camera(eye, (at - eye).normalize());
            view.avatar = avatar.then_some(at);
            let lights: Vec<_> = lamps.iter().map(|&(idx, p)| candidate(&view, idx, p, 3.0)).collect();
            let mut ranked = point_ranking(&lights, &view, &slots.lights);
            let targets = slots.targets(&mut ranked, 1);
            slots.update(&targets[..1], eye, 1.0 / 60.0);
            if owners.last() != Some(&slots.lights[0]) {
                owners.push(slots.lights[0]);
            }
        }
        owners
    }

    #[test]
    fn the_camera_circling_our_avatar_hands_no_point_light_map_over() {
        assert_eq!(point_map_owners_around(true, 4.0), [Some(1)]);
        // ranked from the camera, as before, the map follows the nearest
        // lamp around the circle
        assert!(point_map_owners_around(false, 4.0).len() > 2);
        // a camera far from the avatar looks at another place: ranked from it
        assert!(point_map_owners_around(true, 12.0).len() > 2);
    }

    #[test]
    fn point_shadows_are_ranked_from_the_avatar_then_from_the_camera_as_it_moves_away() {
        let avatar = Vec3::new(128.0, 128.0, 25.0);
        let at = |d: f32| {
            let mut view = camera(avatar + Vec3::new(-d, 0.0, 0.0), Vec3::X);
            view.avatar = Some(avatar);
            point_anchor(&view)
        };
        assert_eq!(at(3.0), (avatar, Vec3::ZERO, true));
        assert_eq!(at(POINT_ANCHOR_NEAR), (avatar, Vec3::ZERO, true));
        let far = at(POINT_ANCHOR_FAR);
        assert_eq!(far, (avatar - Vec3::new(POINT_ANCHOR_FAR, 0.0, 0.0), Vec3::X, false));
        // no jump between the two: the ranking point slides with the camera
        let mut last = at(POINT_ANCHOR_NEAR).0;
        let mut d = POINT_ANCHOR_NEAR;
        while d < POINT_ANCHOR_FAR + 0.5 {
            d += 0.01;
            let (from, forward, _) = at(d);
            assert!(from.distance(last) < 0.05, "{d}: {from} after {last}");
            assert!(forward.length() <= 1.0 + 1e-6);
            last = from;
        }
        // no avatar yet, or a broken position: the camera
        let mut view = camera(avatar, Vec3::X);
        assert_eq!(point_anchor(&view), (avatar, Vec3::X, false));
        view.avatar = Some(Vec3::NAN);
        assert_eq!(point_anchor(&view), (avatar, Vec3::X, false));
    }

    #[test]
    fn near_our_avatar_a_point_light_out_of_view_keeps_its_map_but_takes_none() {
        let avatar = Vec3::new(128.0, 128.0, 25.0);
        // the camera looks away from a lamp 10 m behind it
        let mut view = camera(avatar - Vec3::new(3.0, 0.0, 0.0), Vec3::X);
        let behind = candidate(&view, 7, avatar - Vec3::new(13.0, 0.0, 0.0), 2.0);
        assert!(!behind.in_view);
        view.avatar = Some(avatar);
        assert!(point_ranking(&[behind], &view, &[None]).is_empty());
        assert_eq!(point_ranking(&[behind], &view, &[Some(7)]).len(), 1);
        // ranked from the camera it gives its map up, as before
        view.avatar = None;
        assert!(point_ranking(&[behind], &view, &[Some(7)]).is_empty());
    }

    /// Frames a point light's map takes to go from light 1 to light 2, the
    /// camera turning by `turn(frame)` radians at each frame.
    fn point_hand_over_frames(turn: impl Fn(u32) -> f32) -> u32 {
        let mut slots = SpotShadowSlots::default();
        let dt = 1.0 / 60.0;
        let mut yaw = 0.0f32;
        let mut step = |slots: &mut SpotShadowSlots, targets: &[Option<usize>], frame: u32| {
            yaw += turn(frame);
            let speed = slots.turn_speed(Vec3::new(yaw.cos(), yaw.sin(), 0.0), dt);
            slots.update_at_least(targets, Vec3::ZERO, dt, speed);
        };
        for frame in 0..100 {
            step(&mut slots, &[Some(1)], frame);
        }
        assert_eq!(slots.fade, [1.0]);
        let mut frames = 0;
        while slots.lights != [Some(2)] || slots.fade[0] < 1.0 {
            step(&mut slots, &[Some(2)], 100 + frames);
            frames += 1;
            assert!(frames < 200);
        }
        frames
    }

    #[test]
    fn a_camera_turn_hands_a_point_light_map_over_in_half_a_second() {
        // still: a second out, a second in
        let still = point_hand_over_frames(|_| 0.0);
        assert!((119..=122).contains(&still), "{still}");
        // a flick at the frame the ranking changes, then still: the speed
        // of the turn holds long enough for the whole hand-over
        let flick = point_hand_over_frames(|frame| if frame == 100 { 1.5 } else { 0.0 });
        assert!((29..=32).contains(&flick), "{flick}");
        // a steady pan of 60 degrees a second
        let pan = point_hand_over_frames(|_| 1.0 / 60.0);
        assert!((29..=32).contains(&pan), "{pan}");
        // a slow drift of 6 degrees a second barely counts
        let drift = point_hand_over_frames(|_| 0.1 / 60.0);
        assert!(drift > 100, "{drift}");
        // the speed of a flick before the change has run out by then
        let early = point_hand_over_frames(|frame| if frame == 40 { 1.5 } else { 0.0 });
        assert_eq!(early, still);
    }
}

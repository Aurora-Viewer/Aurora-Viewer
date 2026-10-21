//! Session-local visual modifiers, port of RlvOverlayEffect/RlvSphereEffect
//! (indra/newview/rlveffects.cpp, originally LGPL 2.1, Kitty Barnett).
use super::{Command, Param};
use std::collections::BTreeMap;
use std::time::Instant;
use uuid::Uuid;

pub const MODIFIERS: &[&str] = &[
    "setoverlay_alpha",
    "setoverlay_texture",
    "setoverlay_tint",
    "setoverlay_tween",
    "setsphere_mode",
    "setsphere_origin",
    "setsphere_color",
    "setsphere_distmin",
    "setsphere_distmax",
    "setsphere_distextend",
    "setsphere_param",
    "setsphere_tween",
    "setsphere_valuemin",
    "setsphere_valuemax",
];

#[derive(Clone)]
struct Tween<const N: usize> {
    from: [f32; N],
    to: [f32; N],
    start: Instant,
    duration: f32,
}

impl<const N: usize> Tween<N> {
    fn new(value: [f32; N]) -> Self {
        Self {
            from: value,
            to: value,
            start: Instant::now(),
            duration: 0.0,
        }
    }
    fn get(&self, now: Instant) -> [f32; N] {
        let t = if self.duration > 0.0 {
            (now.saturating_duration_since(self.start).as_secs_f32() / self.duration).min(1.0)
        } else {
            1.0
        };
        std::array::from_fn(|i| self.from[i] + (self.to[i] - self.from[i]) * t)
    }
    fn set(&mut self, value: [f32; N], duration: f32, now: Instant) {
        self.from = self.get(now);
        self.to = value;
        self.start = now;
        self.duration = duration.max(0.0);
    }
}

pub struct Overlay {
    pub texture: Uuid,
    alpha: Tween<1>,
    tint: Tween<3>,
}

pub struct OverlayFrame {
    pub texture: Uuid,
    pub color: [f32; 4],
    pub blocks_touch: bool,
}

/// Same two-pixel alpha mask and >32 threshold as LLImageGL::updatePickMask.
pub struct AlphaMask {
    width: usize,
    height: usize,
    bits: Vec<u8>,
}

impl AlphaMask {
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Self> {
        let (w, h) = (width as usize, height as usize);
        if w == 0 || h == 0 || rgba.len() != w.checked_mul(h)?.checked_mul(4)? {
            return None;
        }
        let (width, height) = (w.div_ceil(2), h.div_ceil(2));
        let mut bits = vec![0; (width * height).div_ceil(8)];
        for y in 0..height {
            for x in 0..width {
                if rgba[(y * 2 * w + x * 2) * 4 + 3] > 32 {
                    let i = y * width + x;
                    bits[i / 8] |= 1 << (i % 8);
                }
            }
        }
        Some(Self { width, height, bits })
    }

    pub fn hit(&self, u: f32, v: f32) -> bool {
        if !u.is_finite() || !v.is_finite() {
            return true;
        }
        let x = (u.clamp(0.0, 0.999999) * self.width as f32) as usize;
        let y = (v.clamp(0.0, 0.999999) * self.height as f32) as usize;
        let i = y * self.width + x;
        self.bits[i / 8] & (1 << (i % 8)) != 0
    }
}

impl Default for Overlay {
    fn default() -> Self {
        Self {
            texture: Uuid::nil(),
            alpha: Tween::new([1.0]),
            tint: Tween::new([1.0; 3]),
        }
    }
}

impl Overlay {
    pub fn color(&self, now: Instant) -> [f32; 4] {
        let [r, g, b] = self.tint.get(now);
        [r, g, b, self.alpha.get(now)[0].clamp(0.0, 1.0)]
    }
}

pub struct Sphere {
    pub mode: u32,
    pub origin: u32,
    pub extend: u32,
    params: Tween<4>,
    distance: [Tween<1>; 2],
    value: [Tween<1>; 2],
    duration: f32,
}

impl Default for Sphere {
    fn default() -> Self {
        Self {
            mode: 0,
            origin: 0,
            extend: 1,
            params: Tween::new([0.0; 4]),
            distance: [Tween::new([0.0]), Tween::new([0.0])],
            value: [Tween::new([1.0]), Tween::new([1.0])],
            duration: 0.0,
        }
    }
}

impl Sphere {
    pub fn sample(&self, now: Instant) -> ([f32; 4], [f32; 4]) {
        let min = self.distance[0].get(now)[0];
        let max = self.distance[1].get(now)[0].max(min);
        (
            self.params.get(now),
            [self.value[0].get(now)[0], min, self.value[1].get(now)[0], max],
        )
    }
}

#[derive(Default)]
pub struct Visuals {
    pub overlays: BTreeMap<Uuid, Overlay>,
    pub spheres: BTreeMap<Uuid, Sphere>,
    /// Order of activation is independent of UUID sort order.
    pub overlay_order: Vec<Uuid>,
    pub sphere_order: Vec<Uuid>,
}

fn scalar(text: &str) -> Option<f32> {
    text.parse::<f32>().ok().filter(|v| v.is_finite())
}

fn vector<const N: usize>(text: &str) -> Option<[f32; N]> {
    let values = text.split('/').map(scalar).collect::<Option<Vec<_>>>()?;
    values.try_into().ok()
}

fn format_values(values: &[f32]) -> String {
    values.iter().map(|v| format!("{v:.6}")).collect::<Vec<_>>().join("/")
}

impl Visuals {
    pub fn forget(&mut self, source: Uuid) {
        self.overlays.remove(&source);
        self.spheres.remove(&source);
        self.overlay_order.retain(|id| *id != source);
        self.sphere_order.retain(|id| *id != source);
    }

    pub fn activate(&mut self, source: Uuid, name: &str, active: bool) {
        let order = match name {
            "setoverlay" => &mut self.overlay_order,
            "setsphere" => &mut self.sphere_order,
            _ => return,
        };
        if !active {
            order.retain(|id| *id != source);
            if name == "setoverlay" {
                self.overlays.remove(&source);
            } else {
                self.spheres.remove(&source);
            }
        } else if !order.contains(&source) {
            order.push(source);
            if name == "setoverlay" {
                self.overlays.entry(source).or_default();
            } else {
                self.spheres.entry(source).or_default();
            }
        }
    }

    /// Local modifiers require an existing issuing object. Values and tween
    /// targets are per issuer; no script can mutate a different object's effect.
    pub fn command(&mut self, source: Uuid, c: &Command, now: Instant) -> Option<String> {
        let query = matches!(c.param, Param::Reply(_));
        if !query && c.param != Param::Force {
            return None;
        }
        if c.behaviour == "setoverlay_tween" {
            if query || !self.overlay_order.contains(&source) {
                return None;
            }
            let parts: Vec<_> = c.option.split(';').collect();
            if let [alpha, tint, duration] = parts.as_slice()
                && let Some(duration) = scalar(duration)
            {
                let overlay = self.overlays.get_mut(&source)?;
                if let Some(alpha) = scalar(alpha) {
                    overlay.alpha.set([alpha], duration, now);
                }
                if let Some(tint) = vector(tint) {
                    overlay.tint.set(tint, duration, now);
                }
            }
            return None;
        }
        if let Some(modifier) = c.behaviour.strip_prefix("setoverlay_") {
            let effect = self.overlays.entry(source).or_default();
            match modifier {
                "texture" => {
                    if query {
                        return Some(effect.texture.to_string());
                    }
                    if let Ok(texture) = Uuid::parse_str(&c.option) {
                        effect.texture = texture;
                    }
                }
                "alpha" => {
                    if query {
                        return Some(format_values(&effect.alpha.to));
                    }
                    if let Some(value) = scalar(&c.option) {
                        effect.alpha.set([value], 0.0, now);
                    }
                }
                "tint" => {
                    if query {
                        return Some(format_values(&effect.tint.to));
                    }
                    if let Some(value) = vector(&c.option) {
                        effect.tint.set(value, 0.0, now);
                    }
                }
                _ => {}
            }
            return None;
        }
        let modifier = c.behaviour.strip_prefix("setsphere_")?;
        let effect = self.spheres.entry(source).or_default();
        match modifier {
            "mode" | "origin" | "distextend" => {
                let (value, max) = match modifier {
                    "mode" => (&mut effect.mode, 4),
                    "origin" => (&mut effect.origin, 1),
                    _ => (&mut effect.extend, 3),
                };
                if query {
                    return Some(value.to_string());
                }
                if let Ok(new) = c.option.parse::<u32>()
                    && new <= max
                {
                    *value = new;
                }
            }
            "color" | "param" => {
                if query {
                    return Some(format_values(&effect.params.to[..if modifier == "color" { 3 } else { 4 }]));
                }
                let value = if modifier == "color" {
                    vector::<3>(&c.option).map(|[r, g, b]| [r, g, b, 0.0])
                } else {
                    vector(&c.option)
                };
                if let Some(value) = value {
                    effect.params.set(value, effect.duration, now);
                }
            }
            "distmin" | "distmax" | "valuemin" | "valuemax" => {
                let i = usize::from(modifier.ends_with("max"));
                let value = if modifier.starts_with("dist") {
                    &mut effect.distance[i]
                } else {
                    &mut effect.value[i]
                };
                if query {
                    return Some(format_values(&value.to));
                }
                if let Some(new) = scalar(&c.option) {
                    value.set([new], effect.duration, now);
                }
            }
            "tween" => {
                if query {
                    return Some(format_values(&[effect.duration]));
                }
                if let Some(value) = scalar(&c.option) {
                    effect.duration = value.max(0.0);
                }
            }
            _ => {}
        }
        None
    }
}

impl super::super::World {
    fn rlva_overlay_root(&self, source: Uuid) -> Option<(Uuid, usize)> {
        let index = self.objects.index_of_uuid(&source)?;
        let mut root = index;
        for _ in 0..64 {
            let object = self.objects.get(root)?;
            let Some(parent) = self.objects.parent_of(object) else {
                break;
            };
            if self.objects.get(parent)?.is_avatar() {
                break;
            }
            root = parent;
        }
        let object = self.objects.get(root)?;
        let link = if index == root {
            1
        } else {
            self.objects
                .children_of(&object.key)
                .iter()
                .position(|i| *i == index)
                .map_or(256, |i| i + 2)
        };
        Some((object.full_id, link))
    }

    pub fn rlva_overlay_frames(&self, now: Instant) -> Vec<OverlayFrame> {
        if !self.rlva.enabled() {
            return Vec::new();
        }
        let Some(first) = self.rlva.visuals.overlay_order.first() else {
            return Vec::new();
        };
        let active_root = self.rlva_overlay_root(*first).map(|(root, _)| root);
        let mut overlays: Vec<_> = self
            .rlva
            .visuals
            .overlay_order
            .iter()
            .filter_map(|id| {
                let root = self.rlva_overlay_root(*id);
                if root.map(|(root, _)| root) != active_root || (active_root.is_none() && id != first) {
                    return None;
                }
                let overlay = self.rlva.visuals.overlays.get(id)?;
                (!overlay.texture.is_nil()).then(|| {
                    (
                        root.map_or(1, |(_, link)| link),
                        OverlayFrame {
                            texture: overlay.texture,
                            color: overlay.color(now),
                            blocks_touch: self
                                .rlva
                                .rules_for("setoverlay_touch", Uuid::nil())
                                .any(|(source, _)| source == *id),
                        },
                    )
                })
            })
            .collect();
        overlays.sort_by_key(|(link, _)| std::cmp::Reverse(*link));
        overlays.into_iter().map(|(_, frame)| frame).collect()
    }

    pub fn rlva_sphere_frames(&self, camera: glam::Vec3, now: Instant) -> Vec<aurora_render::SphereEffect> {
        if !self.rlva.enabled() {
            return Vec::new();
        }
        self.rlva
            .visuals
            .sphere_order
            .iter()
            .filter_map(|id| {
                let sphere = self.rlva.visuals.spheres.get(id)?;
                let (params, range) = sphere.sample(now);
                Some(aurora_render::SphereEffect {
                    origin: if sphere.origin == 1 { camera } else { self.agent.position },
                    mode: sphere.mode,
                    extend: sphere.extend,
                    params,
                    range,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn rlva_visual_modifiers_are_isolated_finite_and_tween_from_the_current_value() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let mut visuals = Visuals::default();
        let now = Instant::now();
        visuals.activate(a, "setoverlay", true);
        visuals.activate(b, "setoverlay", true);
        visuals.command(a, &Command::parse("setoverlay_tween:0;0/0/0;10=force").unwrap(), now);
        assert_eq!(visuals.overlays[&a].color(now + Duration::from_secs(5)), [0.5; 4]);
        assert_eq!(visuals.overlays[&b].color(now), [1.0; 4]);
        visuals.command(a, &Command::parse("setoverlay_alpha:NaN=force").unwrap(), now);
        assert_eq!(visuals.overlays[&a].color(now + Duration::from_secs(10)), [0.0; 4]);
        visuals.command(a, &Command::parse("setsphere_tween:10=force").unwrap(), now);
        visuals.command(a, &Command::parse("setsphere_distmin:8=force").unwrap(), now);
        visuals.command(a, &Command::parse("setsphere_distmax:4=force").unwrap(), now);
        assert_eq!(visuals.spheres[&a].sample(now + Duration::from_secs(10)).1, [1.0, 8.0, 1.0, 8.0]);
        visuals.forget(a);
        assert!(visuals.overlays.contains_key(&b));
        assert!(!visuals.spheres.contains_key(&a));
    }
}

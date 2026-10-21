//! RlvEnvironment sky commands. Values use the protocol's slider scales,
//! including legacy colour components and intensity-preserving edits.
use super::{
    super::{World, eep::SkyFrame},
    Command, Param,
};
use glam::{Quat, Vec3};
use std::f32::consts::{FRAC_PI_2, PI, TAU};
use uuid::Uuid;

pub const SCALARS: &[&str] = &[
    "densitymultiplier",
    "distancemultiplier",
    "dropletradius",
    "hazedensity",
    "hazehorizon",
    "icelevel",
    "maxaltitude",
    "moisturelevel",
    "scenegamma",
    "cloudcoverage",
    "cloudscale",
    "cloudvariance",
    "moonbrightness",
    "moonscale",
    "sunglowsize",
    "sunglowfocus",
    "sunscale",
    "starbrightness",
    "sunazimuth",
    "sunelevation",
    "moonazimuth",
    "moonelevation",
    "eastangle",
    "sunmoonposition",
];
pub const COLORS: &[&str] = &[
    "ambient",
    "bluedensity",
    "bluehorizon",
    "cloudcolor",
    "clouddensity",
    "clouddetail",
    "sunlightcolor",
];
pub const TEXTURES: &[&str] = &["cloudtexture", "moontexture", "suntexture"];
const LEGACY: &[&str] = &[
    "ambient",
    "bluedensity",
    "bluehorizon",
    "cloudcolor",
    "cloud",
    "clouddetail",
    "sunmooncolor",
    "cloudscroll",
];

fn component(name: &str) -> Option<(&str, usize)> {
    let (base, suffix) = name.split_at_checked(name.len().checked_sub(1)?)?;
    let index = match suffix {
        "r" | "x" => 0,
        "g" | "y" => 1,
        "b" | "z" => 2,
        "i" => 3,
        _ => return None,
    };
    (LEGACY.contains(&base) && (base != "cloudscroll" || index < 2)).then_some((base, index))
}

pub fn supports(behaviour: &str, set: bool) -> bool {
    behaviour.strip_prefix(if set { "setenv_" } else { "getenv_" }).is_some_and(|n| {
        SCALARS.contains(&n)
            || COLORS.contains(&n)
            || TEXTURES.contains(&n)
            || matches!(n, "cloudscroll" | "daytime")
            || (set && matches!(n, "asset" | "preset" | "daycycle"))
            || component(n).is_some()
    })
}

fn number(text: &str) -> Option<f32> {
    text.parse::<f32>().ok().filter(|v| v.is_finite())
}
fn color<'a>(s: &'a mut SkyFrame, name: &str) -> Option<(&'a mut Vec3, f32)> {
    Some(match name {
        "ambient" => (&mut s.ambient, 3.0),
        "bluedensity" => (&mut s.blue_density, 2.0),
        "bluehorizon" => (&mut s.blue_horizon, 2.0),
        "cloudcolor" => (&mut s.cloud_color, 1.0),
        "clouddensity" | "cloud" => (&mut s.cloud_pos_density1, 1.0),
        "clouddetail" => (&mut s.cloud_pos_density2, 1.0),
        "sunlightcolor" | "sunmooncolor" => (&mut s.sunlight, 3.0),
        _ => return None,
    })
}
fn scalar<'a>(s: &'a mut SkyFrame, name: &str) -> Option<(&'a mut f32, f32)> {
    Some(match name {
        "densitymultiplier" => (&mut s.density_multiplier, 0.001),
        "distancemultiplier" => (&mut s.distance_multiplier, 1.0),
        "dropletradius" => (&mut s.droplet_radius, 1.0),
        "hazedensity" => (&mut s.haze_density, 1.0),
        "hazehorizon" => (&mut s.haze_horizon, 1.0),
        "icelevel" => (&mut s.ice_level, 1.0),
        "maxaltitude" => (&mut s.max_y, 1.0),
        "moisturelevel" => (&mut s.moisture_level, 1.0),
        "scenegamma" => (&mut s.gamma, 1.0),
        "cloudcoverage" => (&mut s.cloud_shadow, 1.0),
        "cloudscale" => (&mut s.cloud_scale, 1.0),
        "cloudvariance" => (&mut s.cloud_variance, 1.0),
        "moonbrightness" => (&mut s.moon_brightness, 1.0),
        "moonscale" => (&mut s.moon_scale, 1.0),
        "sunglowfocus" => (&mut s.glow.z, -5.0),
        "sunscale" => (&mut s.sun_scale, 1.0),
        "starbrightness" => (&mut s.star_brightness, 1.0),
        _ => return None,
    })
}

/// Applies to a copy; malformed or overflowing input never reaches rendering.
pub fn property(s: &mut SkyFrame, name: &str, value: Option<&str>) -> Option<String> {
    if let Some((field, scale)) = scalar(s, name) {
        if let Some(v) = value {
            let v = number(v)? * scale;
            if !v.is_finite() {
                return None;
            }
            *field = v;
        }
        return Some(format!("{:.6}", *field / scale));
    }
    if let Some((base, index)) = component(name) {
        if base == "cloudscroll" {
            if let Some(v) = value {
                s.cloud_scroll_rate[index] = number(v)?;
            }
            return Some(format!("{:.6}", s.cloud_scroll_rate[index]));
        }
        let (field, scale) = color(s, base)?;
        let mut c = *field / scale;
        if let Some(v) = value {
            let v = number(v)?;
            if index < 3 {
                c[index] = v;
            } else {
                let max = c.max_element();
                c = if max == 0.0 || v == 0.0 { Vec3::splat(v) } else { c * (v / max) };
            }
            if !(c * scale).is_finite() {
                return None;
            }
            *field = c * scale;
        }
        return Some(format!("{:.6}", if index < 3 { c[index] } else { c.max_element() }));
    }
    if let Some((field, scale)) = color(s, name) {
        if let Some(v) = value {
            let c = super::camera::vector(v)? * scale;
            if !c.is_finite() {
                return None;
            }
            *field = c;
        }
        let c = *field / scale;
        return Some(format!("{:.6}/{:.6}/{:.6}", c.x, c.y, c.z));
    }
    if name == "cloudscroll" {
        if let Some(v) = value {
            let (x, y) = v.split_once('/')?;
            s.cloud_scroll_rate = glam::Vec2::new(number(x)?, number(y)?);
        }
        return Some(format!("{:.6}/{:.6}", s.cloud_scroll_rate.x, s.cloud_scroll_rate.y));
    }
    let texture = match name {
        "cloudtexture" => Some(&mut s.cloud_id),
        "moontexture" => Some(&mut s.moon_id),
        "suntexture" => Some(&mut s.sun_id),
        _ => None,
    };
    if let Some(field) = texture {
        if let Some(v) = value {
            *field = Uuid::parse_str(v).ok()?;
        }
        return Some(field.to_string());
    }
    if name == "sunglowsize" {
        if let Some(v) = value {
            let g = (2.0 - number(v)?) * 20.0;
            if !g.is_finite() {
                return None;
            }
            s.glow.x = g;
        }
        return Some(format!("{:.6}", 2.0 - s.glow.x / 20.0));
    }
    if !matches!(
        name,
        "sunazimuth" | "sunelevation" | "moonazimuth" | "moonelevation" | "eastangle" | "sunmoonposition"
    ) {
        return None;
    }
    let moon = name.starts_with("moon");
    let dir = (if moon { s.moon_rotation } else { s.sun_rotation }) * Vec3::X;
    let mut azimuth = dir.y.atan2(dir.x);
    let mut elevation = dir.z.clamp(-1.0, 1.0).asin();
    if let Some(v) = value {
        let v = number(v)?;
        match name {
            "eastangle" => azimuth = -v * TAU,
            "sunmoonposition" => elevation = v * TAU,
            n if n.ends_with("azimuth") => azimuth = v,
            _ => elevation = v.clamp(-FRAC_PI_2, FRAC_PI_2),
        }
        if !azimuth.is_finite() || !elevation.is_finite() {
            return None;
        }
        let rotation = Quat::from_rotation_z(azimuth) * Quat::from_rotation_y(-elevation);
        if moon {
            s.moon_rotation = rotation;
        } else {
            s.sun_rotation = rotation;
        }
        if matches!(name, "eastangle" | "sunmoonposition") {
            s.moon_rotation = Quat::from_rotation_z(azimuth + PI) * Quat::from_rotation_y(elevation);
        }
    }
    Some(format!(
        "{:.6}",
        match name {
            "eastangle" => (-azimuth).rem_euclid(TAU) / TAU,
            "sunmoonposition" => elevation / TAU,
            n if n.ends_with("azimuth") => azimuth,
            _ => elevation,
        }
    ))
}

impl World {
    pub fn rlva_environment_command(&mut self, source: Uuid, command: &Command) -> bool {
        let set = command.param == Param::Force;
        let Some(name) = command.behaviour.strip_prefix(if set { "setenv_" } else { "getenv_" }) else {
            return false;
        };
        if set && self.rlva.has_except("setenv", source) {
            return true;
        }
        if set && matches!(name, "asset" | "preset" | "daycycle") {
            self.rlva.environment_request = Some((source, command.clone()));
            return true;
        }
        if name == "daytime" {
            if let Param::Reply(channel) = command.param {
                self.rlva.reply(
                    channel,
                    if self.rlva.environment_day.is_some() || (self.rlva.environment.is_none() && !self.eep.has_fixed_sky()) {
                        "-1.000000"
                    } else {
                        "2.000000"
                    }
                    .to_owned(),
                );
            } else if let Some(value) = number(&command.option) {
                if value == -1.0 {
                    self.rlva.clear_environment();
                } else if (0.0..=1.0).contains(&value) {
                    let sky = self
                        .rlva
                        .environment_day
                        .as_ref()
                        .map(|d| d.sky_at(value, 1))
                        .or_else(|| self.eep.day_sky_at(value, self.agent.position.z));
                    if let Some(sky) = sky {
                        self.rlva.environment = Some((source, sky));
                        self.rlva.environment_day = None;
                    }
                }
            }
            return true;
        }
        let mut sky = self.rlva_environment_frames(self.eep.current()).0;
        let result = property(&mut sky, name, set.then_some(command.option.as_str()));
        if set {
            if result.is_some() {
                self.rlva.environment = Some((source, sky));
                self.rlva.environment_day = None;
            }
        } else if let Param::Reply(channel) = command.param {
            self.rlva.reply(channel, result.unwrap_or_default());
        }
        true
    }

    pub fn rlva_environment_frames(
        &self,
        base: (super::super::eep::SkyFrame, super::super::eep::WaterFrame),
    ) -> (super::super::eep::SkyFrame, super::super::eep::WaterFrame) {
        if !self.rlva.enabled() {
            return base;
        }
        if let Some(day) = &self.rlva.environment_day {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0.0, |d| d.as_secs_f64());
            let length = if day.length > 0.0 {
                day.length
            } else {
                super::super::eep::DEFAULT_DAY_LENGTH
            };
            let pos = super::super::eep::cycle_position(now, length, day.offset);
            return (
                day.sky_at(pos, day.track_for_altitude(&self.eep.altitudes(), self.agent.position.z)),
                self.rlva.environment_water.unwrap_or_else(|| day.water_at(pos)),
            );
        }
        (
            self.rlva.environment.map_or(base.0, |(_, sky)| sky),
            self.rlva.environment_water.unwrap_or(base.1),
        )
    }
}

impl super::Rlva {
    pub fn clear_environment(&mut self) {
        self.environment = None;
        self.environment_water = None;
        self.environment_day = None;
        self.environment_request = None;
    }
}

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

/// rlvGetColorComponentFromCharacter (rlvenvironment.cpp): r/x, g/y, b/d
/// (the pre-EEP cloud "density" component) and i; `z` is not one.
fn component(name: &str) -> Option<(&str, usize)> {
    let (base, suffix) = name.split_at_checked(name.len().checked_sub(1)?)?;
    let index = match suffix {
        "r" | "x" => 0,
        "g" | "y" => 1,
        "b" | "d" => 2,
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

/// Discovery uses the same property table as command dispatch, including
/// legacy components and the environment-selection write extensions.
pub fn command_names(set: bool) -> Vec<String> {
    let prefix = if set { "setenv_" } else { "getenv_" };
    let mut names: Vec<_> = SCALARS
        .iter()
        .chain(COLORS)
        .chain(TEXTURES)
        .copied()
        .chain(["cloudscroll", "daytime"])
        .map(|name| format!("{prefix}{name}"))
        .collect();
    for base in LEGACY {
        for suffix in ["r", "g", "b", "x", "y", "d", "i"] {
            let name = format!("{prefix}{base}{suffix}");
            if supports(&name, set) {
                names.push(name);
            }
        }
    }
    if set {
        names.extend(["asset", "preset", "daycycle"].map(|name| format!("{prefix}{name}")));
    }
    names
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
    if matches!(name, "sunglowsize" | "sunglowfocus") {
        if let Some(v) = value {
            let mut glow = s.glow;
            if name == "sunglowsize" {
                glow.x = (2.0 - number(v)?) * 20.0;
            } else {
                glow.z = number(v)? * -5.0;
            }
            glow.y = 0.0;
            if !glow.is_finite() {
                return None;
            }
            s.glow = glow;
        }
        return Some(format!(
            "{:.6}",
            if name == "sunglowsize" {
                2.0 - s.glow.x / 20.0
            } else {
                s.glow.z / -5.0
            }
        ));
    }
    if !matches!(
        name,
        "sunazimuth" | "sunelevation" | "moonazimuth" | "moonelevation" | "eastangle" | "sunmoonposition"
    ) {
        return None;
    }
    let moon = name.starts_with("moon");
    let dir = (if moon { s.moon_rotation } else { s.sun_rotation }) * Vec3::X;
    let (mut azimuth, mut elevation) = angles(dir);
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
        let rotation = azimuth_altitude_quat(azimuth, elevation);
        if moon {
            s.moon_rotation = rotation;
        } else {
            s.sun_rotation = rotation;
        }
        if matches!(name, "eastangle" | "sunmoonposition") {
            s.moon_rotation = azimuth_altitude_quat(azimuth + PI, -elevation);
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

/// convert_azimuth_and_altitude_to_quat (indra/llinventory/llsettingssky.cpp):
/// the shortest rotation from +X to the direction, so that the sun and moon
/// textures keep the Legacy Viewer's roll. Opposite +X its axis is zero and
/// the Legacy Viewer's rotation degenerates; glam picks an axis instead.
fn azimuth_altitude_quat(azimuth: f32, altitude: f32) -> Quat {
    let dir = Vec3::new(azimuth.cos() * altitude.cos(), azimuth.sin() * altitude.cos(), altitude.sin());
    Quat::from_rotation_arc(Vec3::X, dir.normalize_or(Vec3::X))
}

/// rlvGetAzimuth/ElevationFromDirectionVector uses an exact zero test and
/// positive angles, including the legacy axis cases. Preserve those query
/// values even when several angle pairs describe the same direction.
fn angles(direction: Vec3) -> (f32, f32) {
    let positive = |angle: f32| if angle >= 0.0 { angle } else { angle + TAU };
    let azimuth = if direction.y == 0.0 {
        0.0
    } else if direction.x == 0.0 {
        FRAC_PI_2
    } else {
        positive(direction.y.atan2(direction.x))
    };
    let elevation = if direction.z == 0.0 {
        0.0
    } else if direction.x == 0.0 && direction.y != 0.0 {
        positive(direction.z.atan2(direction.y))
    } else if direction.x != 0.0 && direction.y == 0.0 {
        positive(direction.z.atan2(direction.x))
    } else {
        positive(direction.z.clamp(-1.0, 1.0).asin())
    };
    (azimuth, elevation)
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
        // getTargetEnvironment: ENV_EDIT while `@setenv=n` was held when the
        // command came, the user's ENV_LOCAL otherwise
        let locked = command.environment_locked;
        if set && matches!(name, "asset" | "preset" | "daycycle") {
            self.rlva.environment_request = Some((source, command.clone()));
            return true;
        }
        if name == "daytime" {
            if let Param::Reply(channel) = command.param {
                // registerGetEnvFn("daytime"): -1 when the target
                // environment has no instance (getEnvironmentFixedSky is
                // null), 2 otherwise, a day cycle included
                let exists = if self.rlva.has("setenv") {
                    self.rlva.environment.is_some() || self.rlva.environment_day.is_some()
                } else {
                    self.eep.has_local()
                };
                self.rlva.reply(
                    channel,
                    &command.behaviour,
                    if exists { "2.000000" } else { "-1.000000" }.to_owned(),
                );
            } else if let Some(value) = number(&command.option) {
                if value == -1.0 {
                    self.rlva_reset_environment(locked);
                } else if (0.0..=1.0).contains(&value) {
                    // registerSetEnvFn("daytime"): ENV_EDIT's own day only
                    // while locked, the nearest day otherwise; nothing
                    // happens without one
                    let sky = if locked {
                        self.rlva.environment_day.as_ref().map(|d| d.sky_at(value, 0))
                    } else {
                        self.eep.rlva_daytime_sky(value)
                    };
                    if let Some(sky) = sky {
                        self.rlva_set_sky(source, sky, locked);
                    }
                }
            }
            return true;
        }
        // a force reads the target it writes (getTargetSky), a reply the
        // target of the moment it is asked
        let target_locked = if set { locked } else { self.rlva.has("setenv") };
        let mut sky = if target_locked {
            self.rlva_environment_frames(self.eep.current()).0
        } else {
            self.eep.rlva_target_sky()
        };
        let result = property(&mut sky, name, set.then_some(command.option.as_str()));
        if set {
            if result.is_some() {
                self.rlva_set_sky(source, sky, locked);
            }
        } else if let Param::Reply(channel) = command.param {
            self.rlva.reply(channel, &command.behaviour, result.unwrap_or_default());
        }
        true
    }

    /// A fixed sky for the target environment: ENV_EDIT while locked (over
    /// its day, which stays for the next `daytime`), the user's ENV_LOCAL
    /// otherwise (getTargetSky(true) and setEnvironment(fixedEnvironment_t),
    /// rlvenvironment.cpp, llenvironment.cpp).
    fn rlva_set_sky(&mut self, source: Uuid, sky: SkyFrame, locked: bool) {
        if locked {
            self.rlva.environment = Some((source, sky));
            self.rlva.environment_day_sky_fixed = self.rlva.environment_day.is_some();
        } else {
            self.eep.rlva_local_sky(sky);
        }
    }

    /// RlvEnvironment's `daytime` -1: back to the shared environment, or
    /// only RLVa's sky while `@setenv=n` is held (getTargetEnvironment).
    fn rlva_reset_environment(&mut self, locked: bool) {
        if locked {
            self.rlva.clear_environment();
        } else {
            // a later `@setenv=n` in the same message already locked it
            let held = std::mem::replace(&mut self.eep.rlva_locked, false);
            self.eep.clear_local();
            self.eep.rlva_locked = held;
        }
    }

    /// RlvBehaviourToggleHandler<RLV_BHVR_SETENV> (rlvhandler.cpp): the
    /// first `@setenv=n` makes ENV_EDIT a fixed copy of the sky shown, and
    /// setEnvironment(fixedEnvironment_t) freezes the water shown with it.
    /// Called whenever rules may have changed: on the toggle to locked only
    /// (onCommandToggle runs for the first holder), never again while held.
    pub fn rlva_sync_environment_lock(&mut self) {
        let was = self.eep.rlva_locked;
        self.eep.rlva_locked = self.rlva.has("setenv");
        if was || !self.eep.rlva_locked {
            return;
        }
        let Some(holder) = self.rlva.rules_for("setenv", Uuid::nil()).next().map(|(id, _)| id) else {
            return;
        };
        let (sky, water) = self.rlva_environment_frames(self.eep.current());
        self.rlva.environment = Some((holder, sky));
        self.rlva.environment_water = Some(water);
        self.rlva.environment_day = None;
        self.rlva.environment_day_sky_fixed = false;
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
            let sky = match self.rlva.environment {
                Some((_, sky)) if self.rlva.environment_day_sky_fixed => sky,
                _ => day.sky_at(pos, day.track_for_altitude(&self.eep.altitudes(), self.agent.position.z)),
            };
            return (sky, self.rlva.environment_water.unwrap_or_else(|| day.water_at(pos)));
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
        self.environment_day_sky_fixed = false;
        self.environment_request = None;
    }
}

#[cfg(test)]
mod conformance_tests {
    use super::*;

    /// RlvEnvironment::getTargetEnvironment (rlvenvironment.cpp): without
    /// `@setenv=n` a force writes the user's own local environment (ENV_LOCAL),
    /// which the user's next choice replaces and which outlives the object;
    /// under the lock it writes ENV_EDIT, a fixed copy of the sky shown at
    /// the toggle, that the release drops.
    #[test]
    fn rlva_environment_forces_target_the_local_environment_unless_locked() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.rlva.configure(true);
        let say = |world: &mut World, text: &str| {
            world.on_chat(aurora_net::ChatMessage {
                from_name: "RLVa test object".to_owned(),
                source_id: Uuid::from_u128(1),
                owner_id: Uuid::from_u128(3),
                source_type: aurora_net::ChatSourceType::Object,
                chat_type: aurora_net::ChatType::OwnerSay,
                region: None,
                position: Vec3::ZERO,
                message: text.to_owned(),
            });
            world.rlva_pending();
        };
        // unlocked: the user's local environment, no RLVa sky of its own
        say(&mut world, "@setenv_ambient:0.2/0.4/0.6=force");
        assert!(world.rlva.environment.is_none() && world.eep.has_local());
        // the user's next choice replaces it, the shared environment included
        world.eep.clear_local();
        assert!(!world.eep.has_local());
        // RLVa's reset goes back to the shared environment too
        say(&mut world, "@setenv_ambient:0.2/0.4/0.6=force");
        say(&mut world, "@setenv_daytime:-1=force");
        assert!(!world.eep.has_local());
        // the lock copies the sky and freezes the water shown
        say(&mut world, "@setenv=n");
        let copy = world.rlva.environment.map(|(_, sky)| sky).expect("ENV_EDIT at the toggle");
        assert!(world.rlva.environment_water.is_some());
        say(&mut world, "@setenv_ambient:0.2/0.4/0.6=force");
        let forced = world.rlva.environment.map(|(_, sky)| sky).expect("ENV_EDIT kept");
        assert_ne!(forced.ambient, copy.ambient);
        assert!(!world.eep.has_local(), "the user's environment is not touched under the lock");
        // the user's choices are refused while held
        world
            .eep
            .apply_local_sky(SkyFrame::default(), crate::world::eep_env::SourceRights::Flags(0));
        assert!(!world.eep.has_local());
        // the release drops ENV_EDIT
        say(&mut world, "@setenv=y");
        assert!(world.rlva.environment.is_none() && world.rlva.environment_water.is_none());
    }

    #[test]
    fn rlva_environment_angles_keep_native_positive_domains_and_legacy_axis_cases() {
        assert_eq!(angles(Vec3::NEG_X), (0.0, 0.0));
        assert_eq!(angles(Vec3::NEG_Y), (FRAC_PI_2, 0.0));
        assert_eq!(angles(Vec3::NEG_Z), (0.0, 3.0 * FRAC_PI_2));
        let (azimuth, elevation) = angles(Vec3::new(0.5, -0.5, -0.5));
        assert!((azimuth - (TAU - PI / 4.0)).abs() < 1e-6);
        assert!((elevation - (TAU - PI / 6.0)).abs() < 1e-6);
        let mut sky = SkyFrame {
            sun_rotation: Quat::from_rotation_z(-1.0) * Quat::from_rotation_y(0.2),
            ..Default::default()
        };
        let azimuth: f32 = property(&mut sky, "sunazimuth", None).unwrap().parse().unwrap();
        let elevation: f32 = property(&mut sky, "sunelevation", None).unwrap().parse().unwrap();
        assert!((azimuth - (TAU - 1.0)).abs() < 1e-5);
        assert!((elevation - (TAU - 0.2)).abs() < 1e-5);
        assert_eq!(property(&mut sky, "sunglowsize", Some("1.5")), Some("1.500000".into()));
        sky.glow.y = 12.0;
        assert_eq!(property(&mut sky, "sunglowfocus", Some("0.25")), Some("0.250000".into()));
        assert_eq!(sky.glow, Vec3::new(10.0, 0.0, -1.25));
    }

    #[test]
    fn rlva_environment_registered_properties_round_trip_without_mutating_on_invalid_input() {
        let mut sky = SkyFrame::default();
        for name in SCALARS.iter().copied().filter(|name| {
            !matches!(
                *name,
                "sunazimuth" | "sunelevation" | "moonazimuth" | "moonelevation" | "eastangle" | "sunmoonposition"
            )
        }) {
            let expected: f32 = 0.125;
            assert!(property(&mut sky, name, Some("0.125")).is_some(), "{name}");
            let queried: f32 = property(&mut sky, name, None).unwrap().parse().unwrap();
            assert!((queried - expected).abs() < 1e-5, "{name}: {queried}");
            let before = property(&mut sky, name, None);
            assert!(property(&mut sky, name, Some("nan")).is_none());
            assert_eq!(property(&mut sky, name, None), before);
        }
        for name in COLORS {
            assert_eq!(
                property(&mut sky, name, Some("0.25/0.5/0.75")),
                Some("0.250000/0.500000/0.750000".into()),
                "{name}"
            );
            assert_eq!(property(&mut sky, name, None), Some("0.250000/0.500000/0.750000".into()), "{name}");
        }
        for name in TEXTURES {
            let texture = Uuid::from_u128(42).to_string();
            assert_eq!(property(&mut sky, name, Some(&texture)), Some(texture.clone()));
            assert_eq!(property(&mut sky, name, None), Some(texture));
        }
        for name in LEGACY {
            for (suffix, value) in [("r", "0.25"), ("g", "0.5")] {
                let component = format!("{name}{suffix}");
                assert!(property(&mut sky, &component, Some(value)).is_some());
                let actual: f32 = property(&mut sky, &component, None).unwrap().parse().unwrap();
                assert!((actual - value.parse::<f32>().unwrap()).abs() < 1e-5, "{component}");
            }
        }
    }

    /// convert_azimuth_and_altitude_to_quat (llsettingssky.cpp): the
    /// shortest arc from +X, so no twist about the X axis rolls the sun's or
    /// the moon's texture.
    #[test]
    fn rlva_sun_and_moon_angles_turn_by_the_shortest_arc() {
        for (azimuth, altitude) in [(0.0, 0.0), (FRAC_PI_2, PI / 4.0), (2.5, -0.7), (4.0, 1.2)] {
            let q = azimuth_altitude_quat(azimuth, altitude);
            let dir = Vec3::new(azimuth.cos() * altitude.cos(), azimuth.sin() * altitude.cos(), altitude.sin());
            assert!((q * Vec3::X - dir).length() < 1e-5, "{azimuth} {altitude}");
            // the arc's axis is X × dir: nothing along X
            assert!(q.xyz().dot(Vec3::X).abs() < 1e-5, "{azimuth} {altitude}: a roll");
        }
        let mut sky = SkyFrame::default();
        property(&mut sky, "sunazimuth", Some("1.5707963"));
        property(&mut sky, "sunelevation", Some("0.7853982"));
        assert!(sky.sun_rotation.xyz().dot(Vec3::X).abs() < 1e-4);
    }
}

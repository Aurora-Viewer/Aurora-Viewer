//! Build › Edit particles: the parameters of a particle system tuned on a
//! selected prim, the live system they make and the LSL script that sets
//! them. Port of ParticleEditor::onParameterChange, updateUI and
//! createScript (indra/newview/particleeditor.cpp, originally LGPL 2.1), the
//! ranges and defaults of floater_particle_editor.xml.
//!
//! The window lives in `build/ui/particles.rs`; the preview source in
//! `scene/particles.rs` (`ParticleManager::set_preview`); Inject in
//! `app/particle_editor.rs`.

use aurora_prim::particles::{self as ps, PartSysData};
use glam::{Vec2, Vec3};
use uuid::Uuid;

/// Name of the injected script's inventory item (PARTICLE_SCRIPT_NAME).
pub fn script_name() -> &'static str {
    aurora_i18n::tr!("particle-editor-script-name")
}

/// PSYS_SRC_PATTERN_*, in the order of the editor's pattern list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    Explode,
    Angle,
    AngleCone,
    AngleConeEmpty,
    Drop,
}

impl Pattern {
    pub const ALL: [Self; 5] = [Self::Explode, Self::Angle, Self::AngleCone, Self::AngleConeEmpty, Self::Drop];

    fn wire(self) -> u8 {
        match self {
            Self::Explode => ps::LL_PART_SRC_PATTERN_EXPLODE,
            Self::Angle => ps::LL_PART_SRC_PATTERN_ANGLE,
            Self::AngleCone => ps::LL_PART_SRC_PATTERN_ANGLE_CONE,
            Self::AngleConeEmpty => ps::LL_PART_SRC_PATTERN_ANGLE_CONE_EMPTY,
            Self::Drop => ps::LL_PART_SRC_PATTERN_DROP,
        }
    }

    fn lsl(self) -> &'static str {
        match self {
            Self::Explode => "PSYS_SRC_PATTERN_EXPLODE",
            Self::Angle => "PSYS_SRC_PATTERN_ANGLE",
            Self::AngleCone => "PSYS_SRC_PATTERN_ANGLE_CONE",
            Self::AngleConeEmpty => "PSYS_SRC_PATTERN_ANGLE_CONE_EMPTY",
            Self::Drop => "PSYS_SRC_PATTERN_DROP",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Explode => aurora_i18n::tr!("particle-editor-pattern-explode"),
            Self::Angle => aurora_i18n::tr!("particle-editor-pattern-angle"),
            Self::AngleCone => aurora_i18n::tr!("particle-editor-pattern-angle-cone"),
            Self::AngleConeEmpty => aurora_i18n::tr!("particle-editor-pattern-angle-cone-empty"),
            Self::Drop => aurora_i18n::tr!("particle-editor-pattern-drop"),
        }
    }
}

/// PSYS_PART_BF_*, in the order of the editor's blend lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    One,
    Zero,
    DestColor,
    SourceColor,
    OneMinusDestColor,
    OneMinusSourceColor,
    SourceAlpha,
    OneMinusSourceAlpha,
}

impl Blend {
    pub const ALL: [Self; 8] = [
        Self::One,
        Self::Zero,
        Self::DestColor,
        Self::SourceColor,
        Self::OneMinusDestColor,
        Self::OneMinusSourceColor,
        Self::SourceAlpha,
        Self::OneMinusSourceAlpha,
    ];

    fn wire(self) -> u8 {
        match self {
            Self::One => ps::LL_PART_BF_ONE,
            Self::Zero => ps::LL_PART_BF_ZERO,
            Self::DestColor => ps::LL_PART_BF_DEST_COLOR,
            Self::SourceColor => ps::LL_PART_BF_SOURCE_COLOR,
            Self::OneMinusDestColor => ps::LL_PART_BF_ONE_MINUS_DEST_COLOR,
            Self::OneMinusSourceColor => ps::LL_PART_BF_ONE_MINUS_SOURCE_COLOR,
            Self::SourceAlpha => ps::LL_PART_BF_SOURCE_ALPHA,
            Self::OneMinusSourceAlpha => ps::LL_PART_BF_ONE_MINUS_SOURCE_ALPHA,
        }
    }

    fn lsl(self) -> &'static str {
        match self {
            Self::One => "PSYS_PART_BF_ONE",
            Self::Zero => "PSYS_PART_BF_ZERO",
            Self::DestColor => "PSYS_PART_BF_DEST_COLOR",
            Self::SourceColor => "PSYS_PART_BF_SOURCE_COLOR",
            Self::OneMinusDestColor => "PSYS_PART_BF_ONE_MINUS_DEST_COLOR",
            Self::OneMinusSourceColor => "PSYS_PART_BF_ONE_MINUS_SOURCE_COLOR",
            Self::SourceAlpha => "PSYS_PART_BF_SOURCE_ALPHA",
            Self::OneMinusSourceAlpha => "PSYS_PART_BF_ONE_MINUS_SOURCE_ALPHA",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::One => aurora_i18n::tr!("particle-editor-blend-one"),
            Self::Zero => aurora_i18n::tr!("particle-editor-blend-zero"),
            Self::DestColor => aurora_i18n::tr!("particle-editor-blend-dest-color"),
            Self::SourceColor => aurora_i18n::tr!("particle-editor-blend-src-color"),
            Self::OneMinusDestColor => aurora_i18n::tr!("particle-editor-blend-one-minus-dest-color"),
            Self::OneMinusSourceColor => aurora_i18n::tr!("particle-editor-blend-one-minus-src-color"),
            Self::SourceAlpha => aurora_i18n::tr!("particle-editor-blend-src-alpha"),
            Self::OneMinusSourceAlpha => aurora_i18n::tr!("particle-editor-blend-one-minus-src-alpha"),
        }
    }
}

/// One PSYS_PART_FLAGS switch of the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flag {
    pub mask: u32,
    lsl: &'static str,
}

impl Flag {
    pub fn label(self) -> &'static str {
        match self.mask {
            ps::LL_PART_BOUNCE_MASK => aurora_i18n::tr!("particle-editor-flag-bounce"),
            ps::LL_PART_EMISSIVE_MASK => aurora_i18n::tr!("particle-editor-flag-emissive"),
            ps::LL_PART_FOLLOW_SRC_MASK => aurora_i18n::tr!("particle-editor-flag-follow-source"),
            ps::LL_PART_FOLLOW_VELOCITY_MASK => aurora_i18n::tr!("particle-editor-flag-follow-velocity"),
            ps::LL_PART_INTERP_COLOR_MASK => aurora_i18n::tr!("particle-editor-flag-interpolate-color"),
            ps::LL_PART_INTERP_SCALE_MASK => aurora_i18n::tr!("particle-editor-flag-interpolate-scale"),
            ps::LL_PART_TARGET_POS_MASK => aurora_i18n::tr!("particle-editor-flag-target-position"),
            ps::LL_PART_TARGET_LINEAR_MASK => aurora_i18n::tr!("particle-editor-flag-target-linear"),
            ps::LL_PART_WIND_MASK => aurora_i18n::tr!("particle-editor-flag-wind"),
            _ => aurora_i18n::tr!("particle-editor-flag-ribbon"),
        }
    }
}

const fn flag(mask: u32, lsl: &'static str) -> Flag {
    Flag { mask, lsl }
}

/// The switches in the order of the window (floater_particle_editor.xml).
pub const FLAGS: [Flag; 10] = [
    flag(ps::LL_PART_BOUNCE_MASK, "PSYS_PART_BOUNCE_MASK"),
    flag(ps::LL_PART_EMISSIVE_MASK, "PSYS_PART_EMISSIVE_MASK"),
    flag(ps::LL_PART_FOLLOW_SRC_MASK, "PSYS_PART_FOLLOW_SRC_MASK"),
    flag(ps::LL_PART_FOLLOW_VELOCITY_MASK, "PSYS_PART_FOLLOW_VELOCITY_MASK"),
    flag(ps::LL_PART_INTERP_COLOR_MASK, "PSYS_PART_INTERP_COLOR_MASK"),
    flag(ps::LL_PART_INTERP_SCALE_MASK, "PSYS_PART_INTERP_SCALE_MASK"),
    flag(ps::LL_PART_TARGET_POS_MASK, "PSYS_PART_TARGET_POS_MASK"),
    flag(ps::LL_PART_TARGET_LINEAR_MASK, "PSYS_PART_TARGET_LINEAR_MASK"),
    flag(ps::LL_PART_WIND_MASK, "PSYS_PART_WIND_MASK"),
    flag(ps::LL_PART_RIBBON_MASK, "PSYS_PART_RIBBON_MASK"),
];

/// The order createScript writes the flags in (target linear before target
/// position, unlike the window).
const SCRIPT_FLAG_ORDER: [u32; 10] = [
    ps::LL_PART_BOUNCE_MASK,
    ps::LL_PART_EMISSIVE_MASK,
    ps::LL_PART_FOLLOW_SRC_MASK,
    ps::LL_PART_FOLLOW_VELOCITY_MASK,
    ps::LL_PART_INTERP_COLOR_MASK,
    ps::LL_PART_INTERP_SCALE_MASK,
    ps::LL_PART_TARGET_LINEAR_MASK,
    ps::LL_PART_TARGET_POS_MASK,
    ps::LL_PART_WIND_MASK,
    ps::LL_PART_RIBBON_MASK,
];

/// Range of a numeric field: (min, max) of its spinner.
pub mod range {
    pub const BURST_RATE: (f32, f32) = (0.0, 1000.0);
    pub const BURST_COUNT: (u32, u32) = (1, 1000);
    pub const BURST_RADIUS: (f32, f32) = (0.0, 50.0);
    // the spinners' own limits, not π
    #[allow(clippy::approx_constant)]
    pub const ANGLE: (f32, f32) = (-3.14, 3.14);
    pub const SPEED: (f32, f32) = (0.0, 100.0);
    pub const SOURCE_MAX_AGE: (f32, f32) = (0.0, 1000.0);
    pub const PART_MAX_AGE: (f32, f32) = (0.0, 30.0);
    pub const UNIT: (f32, f32) = (0.0, 1.0);
    pub const SCALE: (f32, f32) = (0.03125, 4.0);
    pub const VECTOR: (f32, f32) = (-100.0, 100.0);
}

/// Every field of the editor, with the window's initial values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub pattern: Pattern,
    /// Particle texture; nil is the default particle image.
    pub texture: Uuid,
    pub burst_rate: f32,
    pub burst_count: u32,
    pub burst_radius: f32,
    pub angle_begin: f32,
    pub angle_end: f32,
    pub speed_min: f32,
    pub speed_max: f32,
    pub source_max_age: f32,
    pub part_max_age: f32,
    pub start_alpha: f32,
    pub end_alpha: f32,
    pub start_glow: f32,
    pub end_glow: f32,
    pub blend_source: Blend,
    pub blend_dest: Blend,
    pub start_scale: Vec2,
    pub end_scale: Vec2,
    /// `LL_PART_*` masks of [`FLAGS`].
    pub flags: u32,
    /// Particle target; nil (or the prim itself) is the prim.
    pub target: Uuid,
    pub acceleration: Vec3,
    pub omega: Vec3,
    /// RGB in [0, 1] as the colour swatches show it (sRGB, as LSL takes it).
    pub start_color: [f32; 3],
    pub end_color: [f32; 3],
}

impl Default for Params {
    /// floater_particle_editor.xml: the first pattern of the list, the
    /// spinners' initial values and the alpha blend set by postBuild.
    fn default() -> Self {
        Self {
            pattern: Pattern::Explode,
            texture: Uuid::nil(),
            burst_rate: 0.3,
            burst_count: 5,
            burst_radius: 0.0,
            angle_begin: 0.0,
            angle_end: 0.0,
            speed_min: 0.5,
            speed_max: 0.5,
            source_max_age: 0.0,
            part_max_age: 5.0,
            start_alpha: 1.0,
            end_alpha: 1.0,
            start_glow: 0.0,
            end_glow: 0.0,
            blend_source: Blend::SourceAlpha,
            blend_dest: Blend::OneMinusSourceAlpha,
            start_scale: Vec2::splat(0.5),
            end_scale: Vec2::splat(0.5),
            flags: 0,
            target: Uuid::nil(),
            acceleration: Vec3::ZERO,
            omega: Vec3::ZERO,
            start_color: [1.0; 3],
            end_color: [1.0; 3],
        }
    }
}

/// Which fields apply to the current settings (ParticleEditor::updateUI):
/// the others are greyed out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Enabled {
    pub burst_radius: bool,
    pub speed: bool,
    pub angles: bool,
    pub end_color: bool,
    pub end_scale: bool,
    pub target_position: bool,
    pub target: bool,
    pub motion: bool,
}

impl Params {
    pub fn has(&self, mask: u32) -> bool {
        self.flags & mask != 0
    }

    pub fn set(&mut self, mask: u32, on: bool) {
        if on {
            self.flags |= mask;
        } else {
            self.flags &= !mask;
        }
    }

    pub fn enabled(&self) -> Enabled {
        let drop = self.pattern == Pattern::Drop;
        let explode = self.pattern == Pattern::Explode;
        let linear = self.has(ps::LL_PART_TARGET_LINEAR_MASK);
        Enabled {
            burst_radius: !linear && !self.has(ps::LL_PART_FOLLOW_SRC_MASK) && !drop,
            speed: !linear && !drop,
            angles: !explode && !drop,
            end_color: self.has(ps::LL_PART_INTERP_COLOR_MASK),
            end_scale: self.has(ps::LL_PART_INTERP_SCALE_MASK),
            target_position: !linear,
            target: linear || self.has(ps::LL_PART_TARGET_POS_MASK),
            motion: !linear,
        }
    }

    /// The system shown on the prim (onParameterChange). The burst rate is
    /// at least 0.01 s, as the simulator's, while the script keeps the
    /// value typed.
    pub fn system(&self) -> PartSysData {
        let mut d = PartSysData {
            flags: ps::LL_PART_USE_NEW_ANGLE,
            pattern: self.pattern.wire(),
            max_age: self.source_max_age,
            inner_angle: self.angle_begin,
            outer_angle: self.angle_end,
            burst_rate: self.burst_rate.max(ps::MIN_BURST_RATE),
            burst_part_count: self.burst_count.min(u32::from(u8::MAX)) as u8,
            burst_radius: self.burst_radius,
            burst_speed_min: self.speed_min,
            burst_speed_max: self.speed_max,
            angular_velocity: self.omega,
            part_accel: self.acceleration,
            part_image_id: self.texture,
            target_id: self.target,
            ..Default::default()
        };
        let [r, g, b] = self.start_color;
        d.part.start_color = [r, g, b, self.start_alpha];
        let [r, g, b] = self.end_color;
        d.part.end_color = [r, g, b, self.end_alpha];
        d.part.start_scale = self.start_scale;
        d.part.end_scale = self.end_scale;
        d.part.max_age = self.part_max_age;
        d.part.start_glow = self.start_glow;
        d.part.end_glow = self.end_glow;
        d.part.blend_func_source = self.blend_source.wire();
        d.part.blend_func_dest = self.blend_dest.wire();
        d.part.flags = self.flags;
        // a non-nil CRC: a system, not the "no particles" block
        d.crc = 1;
        d
    }

    /// The llParticleSystem script of these settings for the prim `object`
    /// (createScript): the target is llGetKey() unless another object or
    /// avatar is set, the texture empty for the default image.
    pub fn script(&self, object: Uuid) -> String {
        let target = if self.target.is_nil() || self.target == object {
            "llGetKey()".to_owned()
        } else {
            format!("(key) \"{}\"", self.target)
        };
        let texture = if self.texture.is_nil() {
            String::new()
        } else {
            self.texture.to_string()
        };
        let flags: String = SCRIPT_FLAG_ORDER
            .iter()
            .filter(|m| self.has(**m))
            .filter_map(|m| FLAGS.iter().find(|f| f.mask == *m))
            .map(|f| format!(" |\n                {}", f.lsl))
            .collect();
        let rgb = |c: [f32; 3]| vector(c[0], c[1], c[2]);
        let v = |v: Vec3| vector(v.x, v.y, v.z);
        format!(
            "default\n\
             {{\n\
             \x20   state_entry()\n\
             \x20   {{\n\
             \x20       llParticleSystem(\n\
             \x20       [\n\
             \x20           PSYS_SRC_PATTERN,{pattern},\n\
             \x20           PSYS_SRC_BURST_RADIUS,{radius},\n\
             \x20           PSYS_SRC_ANGLE_BEGIN,{angle_begin},\n\
             \x20           PSYS_SRC_ANGLE_END,{angle_end},\n\
             \x20           PSYS_SRC_TARGET_KEY,{target},\n\
             \x20           PSYS_PART_START_COLOR,{start_color},\n\
             \x20           PSYS_PART_END_COLOR,{end_color},\n\
             \x20           PSYS_PART_START_ALPHA,{start_alpha},\n\
             \x20           PSYS_PART_END_ALPHA,{end_alpha},\n\
             \x20           PSYS_PART_START_GLOW,{start_glow},\n\
             \x20           PSYS_PART_END_GLOW,{end_glow},\n\
             \x20           PSYS_PART_BLEND_FUNC_SOURCE,{blend_source},\n\
             \x20           PSYS_PART_BLEND_FUNC_DEST,{blend_dest},\n\
             \x20           PSYS_PART_START_SCALE,{start_scale},\n\
             \x20           PSYS_PART_END_SCALE,{end_scale},\n\
             \x20           PSYS_SRC_TEXTURE,\"{texture}\",\n\
             \x20           PSYS_SRC_MAX_AGE,{source_max_age},\n\
             \x20           PSYS_PART_MAX_AGE,{part_max_age},\n\
             \x20           PSYS_SRC_BURST_RATE,{burst_rate},\n\
             \x20           PSYS_SRC_BURST_PART_COUNT,{burst_count},\n\
             \x20           PSYS_SRC_ACCEL,{acceleration},\n\
             \x20           PSYS_SRC_OMEGA,{omega},\n\
             \x20           PSYS_SRC_BURST_SPEED_MIN,{speed_min},\n\
             \x20           PSYS_SRC_BURST_SPEED_MAX,{speed_max},\n\
             \x20           PSYS_PART_FLAGS,\n\
             \x20               0{flags}\n\
             \x20       ]);\n\
             \x20   }}\n\
             }}\n",
            pattern = self.pattern.lsl(),
            radius = number(self.burst_radius),
            angle_begin = number(self.angle_begin),
            angle_end = number(self.angle_end),
            start_color = rgb(self.start_color),
            end_color = rgb(self.end_color),
            start_alpha = number(self.start_alpha),
            end_alpha = number(self.end_alpha),
            start_glow = number(self.start_glow),
            end_glow = number(self.end_glow),
            blend_source = self.blend_source.lsl(),
            blend_dest = self.blend_dest.lsl(),
            start_scale = vector(self.start_scale.x, self.start_scale.y, 0.0),
            end_scale = vector(self.end_scale.x, self.end_scale.y, 0.0),
            source_max_age = number(self.source_max_age),
            part_max_age = number(self.part_max_age),
            burst_rate = number(self.burst_rate),
            burst_count = self.burst_count,
            acceleration = v(self.acceleration),
            omega = v(self.omega),
            speed_min = number(self.speed_min),
            speed_max = number(self.speed_max),
        )
    }
}

/// The demo's fountain (AURORA_DEMO_PARTICLE_EDITOR=fountain): a glowing
/// violet to turquoise cone falling back, every interpolation on.
pub fn demo_fountain() -> Params {
    let mut p = Params {
        pattern: Pattern::AngleCone,
        burst_rate: 0.05,
        burst_count: 6,
        burst_radius: 0.1,
        angle_begin: 0.0,
        angle_end: 0.35,
        speed_min: 3.0,
        speed_max: 3.5,
        part_max_age: 2.5,
        start_alpha: 1.0,
        end_alpha: 0.0,
        start_glow: 0.3,
        end_glow: 0.0,
        start_scale: Vec2::splat(0.25),
        end_scale: Vec2::splat(0.05),
        acceleration: Vec3::new(0.0, 0.0, -2.5),
        start_color: [0.545, 0.361, 0.965],
        end_color: [0.369, 0.918, 0.831],
        ..Default::default()
    };
    for mask in [
        ps::LL_PART_EMISSIVE_MASK,
        ps::LL_PART_INTERP_COLOR_MASK,
        ps::LL_PART_INTERP_SCALE_MASK,
    ] {
        p.set(mask, true);
    }
    p
}

/// A spinner's value as LLSD writes a real (`%lg`: six significant digits,
/// no trailing zeros).
fn number(value: f32) -> String {
    if !value.is_finite() || value == 0.0 {
        return "0".to_owned();
    }
    let rounded: f64 = format!("{:.5e}", f64::from(value)).parse().unwrap_or(0.0);
    format!("{rounded}")
}

/// lslVector: `<%f,%f,%f>`.
fn vector(x: f32, y: f32, z: f32) -> String {
    format!("<{x:.6},{y:.6},{z:.6}>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_written_like_llsd_reals() {
        assert_eq!(number(0.3), "0.3");
        assert_eq!(number(5.0), "5");
        assert_eq!(number(0.0), "0");
        assert_eq!(number(-0.0), "0");
        assert_eq!(number(-2.75), "-2.75");
        assert_eq!(number(0.123_456_7), "0.123457");
        assert_eq!(number(0.03125), "0.03125");
        assert_eq!(number(1000.0), "1000");
        assert_eq!(vector(1.0, 0.5, 0.0), "<1.000000,0.500000,0.000000>");
    }

    #[test]
    fn the_default_script_matches_the_legacy_template() {
        let script = Params::default().script(Uuid::from_u128(7));
        let expected = "default\n{\n    state_entry()\n    {\n        llParticleSystem(\n        [\n\
            \x20           PSYS_SRC_PATTERN,PSYS_SRC_PATTERN_EXPLODE,\n\
            \x20           PSYS_SRC_BURST_RADIUS,0,\n\
            \x20           PSYS_SRC_ANGLE_BEGIN,0,\n\
            \x20           PSYS_SRC_ANGLE_END,0,\n\
            \x20           PSYS_SRC_TARGET_KEY,llGetKey(),\n\
            \x20           PSYS_PART_START_COLOR,<1.000000,1.000000,1.000000>,\n\
            \x20           PSYS_PART_END_COLOR,<1.000000,1.000000,1.000000>,\n\
            \x20           PSYS_PART_START_ALPHA,1,\n\
            \x20           PSYS_PART_END_ALPHA,1,\n\
            \x20           PSYS_PART_START_GLOW,0,\n\
            \x20           PSYS_PART_END_GLOW,0,\n\
            \x20           PSYS_PART_BLEND_FUNC_SOURCE,PSYS_PART_BF_SOURCE_ALPHA,\n\
            \x20           PSYS_PART_BLEND_FUNC_DEST,PSYS_PART_BF_ONE_MINUS_SOURCE_ALPHA,\n\
            \x20           PSYS_PART_START_SCALE,<0.500000,0.500000,0.000000>,\n\
            \x20           PSYS_PART_END_SCALE,<0.500000,0.500000,0.000000>,\n\
            \x20           PSYS_SRC_TEXTURE,\"\",\n\
            \x20           PSYS_SRC_MAX_AGE,0,\n\
            \x20           PSYS_PART_MAX_AGE,5,\n\
            \x20           PSYS_SRC_BURST_RATE,0.3,\n\
            \x20           PSYS_SRC_BURST_PART_COUNT,5,\n\
            \x20           PSYS_SRC_ACCEL,<0.000000,0.000000,0.000000>,\n\
            \x20           PSYS_SRC_OMEGA,<0.000000,0.000000,0.000000>,\n\
            \x20           PSYS_SRC_BURST_SPEED_MIN,0.5,\n\
            \x20           PSYS_SRC_BURST_SPEED_MAX,0.5,\n\
            \x20           PSYS_PART_FLAGS,\n\
            \x20               0\n\
            \x20       ]);\n    }\n}\n";
        assert_eq!(script, expected);
    }

    #[test]
    fn the_script_names_the_target_texture_and_flags() {
        let object = Uuid::from_u128(7);
        let target = Uuid::from_u128(9);
        let texture = Uuid::from_u128(11);
        let mut p = Params {
            pattern: Pattern::AngleCone,
            texture,
            target,
            blend_dest: Blend::One,
            ..Default::default()
        };
        for mask in [
            ps::LL_PART_RIBBON_MASK,
            ps::LL_PART_TARGET_POS_MASK,
            ps::LL_PART_TARGET_LINEAR_MASK,
            ps::LL_PART_BOUNCE_MASK,
        ] {
            p.set(mask, true);
        }
        let script = p.script(object);
        assert!(script.contains("PSYS_SRC_PATTERN,PSYS_SRC_PATTERN_ANGLE_CONE,\n"));
        assert!(script.contains(&format!("PSYS_SRC_TARGET_KEY,(key) \"{target}\",\n")));
        assert!(script.contains(&format!("PSYS_SRC_TEXTURE,\"{texture}\",\n")));
        assert!(script.contains("PSYS_PART_BLEND_FUNC_DEST,PSYS_PART_BF_ONE,\n"));
        // createScript's order: linear before position, ribbon last
        assert!(script.contains(
            "0 |\n                PSYS_PART_BOUNCE_MASK |\n                PSYS_PART_TARGET_LINEAR_MASK |\n                PSYS_PART_TARGET_POS_MASK |\n                PSYS_PART_RIBBON_MASK\n"
        ));
        // the prim itself as target is llGetKey()
        p.target = object;
        assert!(p.script(object).contains("PSYS_SRC_TARGET_KEY,llGetKey(),\n"));
    }

    #[test]
    fn the_preview_system_carries_every_field() {
        let mut p = Params {
            pattern: Pattern::Angle,
            burst_rate: 0.0,
            burst_count: 600,
            angle_begin: 0.5,
            angle_end: 1.0,
            start_alpha: 0.25,
            end_alpha: 0.75,
            start_glow: 0.2,
            blend_source: Blend::One,
            start_color: [1.0, 0.0, 0.5],
            acceleration: Vec3::new(0.0, 0.0, -1.0),
            omega: Vec3::new(0.0, 0.0, 2.0),
            ..Default::default()
        };
        p.set(ps::LL_PART_EMISSIVE_MASK, true);
        let d = p.system();
        assert_eq!(d.pattern, ps::LL_PART_SRC_PATTERN_ANGLE);
        assert_eq!(d.flags, ps::LL_PART_USE_NEW_ANGLE);
        assert_ne!(d.crc, 0);
        // the preview keeps the simulator's minimum and the U8 count
        assert_eq!(d.burst_rate, ps::MIN_BURST_RATE);
        assert_eq!(d.burst_part_count, 255);
        assert_eq!((d.inner_angle, d.outer_angle), (0.5, 1.0));
        assert_eq!(d.part.start_color, [1.0, 0.0, 0.5, 0.25]);
        assert_eq!(d.part.end_color[3], 0.75);
        assert_eq!(d.part.start_glow, 0.2);
        assert_eq!(d.part.blend_func_source, ps::LL_PART_BF_ONE);
        assert_eq!(d.part.flags, ps::LL_PART_EMISSIVE_MASK);
        assert_eq!(d.part.max_age, 5.0);
        assert_eq!(d.part_accel, Vec3::new(0.0, 0.0, -1.0));
        assert_eq!(d.angular_velocity, Vec3::new(0.0, 0.0, 2.0));
        // the script keeps the value typed
        assert!(p.script(Uuid::nil()).contains("PSYS_SRC_BURST_RATE,0,\n"));
        assert!(p.script(Uuid::nil()).contains("PSYS_SRC_BURST_PART_COUNT,600,\n"));
    }

    #[test]
    fn fields_grey_out_like_update_ui() {
        let mut p = Params::default();
        let e = p.enabled();
        assert!(!e.angles && e.speed && e.burst_radius && !e.end_color && !e.end_scale && !e.target && e.motion);
        p.pattern = Pattern::Drop;
        let e = p.enabled();
        assert!(!e.burst_radius && !e.speed && !e.angles);
        p.pattern = Pattern::AngleCone;
        p.set(ps::LL_PART_FOLLOW_SRC_MASK, true);
        p.set(ps::LL_PART_INTERP_COLOR_MASK, true);
        p.set(ps::LL_PART_INTERP_SCALE_MASK, true);
        let e = p.enabled();
        assert!(e.angles && !e.burst_radius && e.speed && e.end_color && e.end_scale);
        p.set(ps::LL_PART_TARGET_POS_MASK, true);
        assert!(p.enabled().target && p.enabled().target_position);
        p.set(ps::LL_PART_TARGET_POS_MASK, false);
        p.set(ps::LL_PART_TARGET_LINEAR_MASK, true);
        let e = p.enabled();
        assert!(e.target && !e.target_position && !e.motion && !e.speed);
    }
}

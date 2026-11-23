//! Projector shadow controls and compatibility with the beta shadow settings.

/// Radius of the light of a projector at a softness of 1, in metres: a
/// lamp has no size of its own, a small bulb gives the projectors' soft
/// shadows (`projector_soft` in common.wgsl), scaled by the softness.
pub(crate) const PROJECTOR_LIGHT_RADIUS: f32 = 0.02;

/// Projectors use P3's five-tap filter unless the startup override opts in.
fn projector_filter_requested(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.trim() == "1")
}

pub(crate) fn projectors_soft() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| projector_filter_requested(std::env::var("AURORA_SOFT_PROJECTORS").ok().as_deref()))
}

/// Directional filtering samples light-space depth directly, including tiled photos.
pub fn tile_reach(_enabled: bool) -> u32 {
    0
}

/// `AURORA_SOFT_SHADOWS`: the setting forced for a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Override {
    None,
    Off,
    On,
    /// Off and on by turns, one second each (on in odd seconds).
    Alternate,
}

impl Override {
    pub(crate) fn parse(v: Option<&str>) -> Self {
        match v.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("0" | "off") => Override::Off,
            Some("1" | "on") => Override::On,
            Some("alt") => Override::Alternate,
            _ => Override::None,
        }
    }

    pub(crate) fn from_env() -> Self {
        static MODE: std::sync::OnceLock<Override> = std::sync::OnceLock::new();
        *MODE.get_or_init(|| Override::parse(std::env::var("AURORA_SOFT_SHADOWS").ok().as_deref()))
    }

    /// Whether the projector-filter variant is needed for the setting `on`.
    pub(crate) fn needed(self, on: bool) -> bool {
        match self {
            Override::None => on,
            Override::Off => false,
            Override::On | Override::Alternate => true,
        }
    }

    /// Whether the frame drawn `secs` seconds into the session has them.
    pub(crate) fn on_now(self, on: bool, secs: u64) -> bool {
        match self {
            Override::Alternate => secs % 2 == 1,
            _ => self.needed(on),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graded_lookup_loop_preserves_invalid_map_and_band_rules() {
        // Exhaust every valid/invalid map combination, both band endpoints,
        // and portrait coverage. The values distinguish every map's result.
        for level in 0..4 {
            for mask in 0..16 {
                for band in [0.0_f32, 0.1, 0.5, 1.0] {
                    if level == 3 && band > 0.0 {
                        continue;
                    }
                    for close in [0.0_f32, 0.3, 1.0] {
                        let values: [f32; 4] = std::array::from_fn(|k| if mask & (1 << k) == 0 { -1.0 } else { 0.1 + k as f32 * 0.2 });
                        let mix = |a: f32, b: f32, w: f32| a * (1.0 - w) + b * w;
                        let mut old = values[level];
                        let mut last = level;
                        if band > 0.0 {
                            last += 1;
                            if values[last] >= 0.0 {
                                old = mix(old.max(0.0), values[last], if old >= 0.0 { band } else { 1.0 });
                            }
                        } else if close >= 1.0 {
                            old = 0.4;
                        } else if close > 0.0 && old >= 0.0 {
                            old = mix(old, 0.4, close);
                        }
                        for value in values.iter().skip(last + 1) {
                            if old >= 0.0 {
                                break;
                            }
                            old = *value;
                        }
                        let mut new = -1.0_f32;
                        for (k, value) in values.iter().enumerate().skip(level) {
                            let sample = if k == level && band <= 0.0 && close >= 1.0 { 0.4 } else { *value };
                            if k == level {
                                new = sample;
                                if band <= 0.0 && close > 0.0 && new >= 0.0 {
                                    new = mix(new, 0.4, close);
                                }
                                if band > 0.0 {
                                    continue;
                                }
                            } else if band > 0.0 && k == level + 1 {
                                if sample >= 0.0 {
                                    new = mix(new.max(0.0), sample, if new >= 0.0 { band } else { 1.0 });
                                }
                            } else {
                                new = sample;
                            }
                            if new >= 0.0 {
                                break;
                            }
                        }
                        assert_eq!(
                            old.to_bits(),
                            new.to_bits(),
                            "level={level} mask={mask} band={band} portrait={close}"
                        );
                    }
                }
            }
        }
        let common = include_str!("shaders/common.wgsl");
        let body = &common[common.find("fn graded_shadow(").expect("graded")..common.find("fn graded_distance(").expect("end")];
        assert_eq!(body.matches("graded_lookup(").count(), 1);
    }

    #[test]
    fn directional_disk_patterns_preserve_weights_and_order() {
        fn smooth(a: f32, b: f32, x: f32) -> f32 {
            let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        }
        for step in 0..=65536 {
            let radius = step as f32 / 256.0;
            let middle = smooth(5.0, 12.0, radius);
            let wide = smooth(16.0, 32.0, radius);
            let mut original = vec![(0, 1.0_f32)];
            if middle > 0.0 {
                original.push((1, middle));
            }
            if wide > 0.0 {
                original.push((2, wide));
            }
            let patterns = 1 + u32::from(middle > 0.0) + u32::from(wide > 0.0);
            let dynamic: Vec<_> = (0..patterns)
                .map(|pattern| (pattern, [1.0, middle, wide][pattern as usize]))
                .collect();
            assert_eq!(original, dynamic, "radius {radius}");
        }
        let common = include_str!("shaders/common.wgsl").replace("\r\n", "\n");
        let body = &common[common.find("fn directional_pcss(").expect("PCSS")..common.find("struct DistantShadow").expect("end of PCSS")];
        assert_eq!(body.matches("pcss_disk(").count(), 1, "one static disk callsite");
        assert!(body.contains("pattern < patterns"));
    }

    #[test]
    fn five_tap_projectors_are_stock_and_soft_projectors_require_opt_in() {
        for input in [None, Some("0"), Some(""), Some("invalid")] {
            assert!(!projector_filter_requested(input));
        }
        assert!(projector_filter_requested(Some(" 1 ")));
    }

    /// `classic_shadowed` and `shadow_lift` of common.wgsl, `lift` being
    /// `frame.soft.y` (1 - the shadow intensity).
    fn classic_shadowed(nl: f32, shadow: f32, lift: f32) -> f32 {
        let lit = nl.min(shadow);
        lit + (nl - lit) * lift
    }

    fn shadow_lift(s: f32, lift: f32) -> f32 {
        s + (1.0 - s) * lift
    }

    #[test]
    fn the_shadow_intensity_lifts_the_shadow_and_leaves_full_intensity_untouched() {
        for nl in [0.0_f32, 0.2, 0.57, 1.0] {
            for shadow in [0.0_f32, 0.1, 0.5, 0.9, 1.0] {
                // full intensity: the Legacy min(N.L, shadow), to the bit
                assert_eq!(classic_shadowed(nl, shadow, 0.0).to_bits(), nl.min(shadow).to_bits());
                assert_eq!(shadow_lift(shadow, 0.0).to_bits(), shadow.to_bits());
                // no intensity: no shadow at all
                assert_eq!(classic_shadowed(nl, shadow, 1.0), nl);
                assert_eq!(shadow_lift(shadow, 1.0), 1.0);
                // linear between
                let (a, b) = (classic_shadowed(nl, shadow, 0.25), classic_shadowed(nl, shadow, 0.75));
                let (lit, lifted) = (nl.min(shadow), nl);
                assert!((a - (lit + 0.25 * (lifted - lit))).abs() < 1e-6 && (b - (lit + 0.75 * (lifted - lit))).abs() < 1e-6);
                assert!((shadow_lift(shadow, 0.5) - (shadow + 0.5 * (1.0 - shadow))).abs() < 1e-6);
            }
        }
        // the shader holds the same formulas, and every consumer of a sun
        // shadow takes the lift: classic PBR, classic highlights, water
        let common = include_str!("shaders/common.wgsl");
        assert!(common.contains("return lit + (nl - lit) * frame.soft.y;"));
        assert!(common.contains("return s + (1.0 - s) * frame.soft.y;"));
        assert!(common.contains("* shadow_lift(shadow), vec3<f32>(0.0), vec3<f32>(10.0)"));
        assert!(common.contains("shadow_lift(shadow_factor(pos,normal"));
        assert!(include_str!("shaders/env.wgsl").contains("shadow_lift(shadow_factor(in.world_pos"));
        assert!(include_str!("shaders/object.wgsl").contains("shadow_lift(sh), ll_sun_linear(a)"));
    }

    #[test]
    fn terrain_water_and_snow_follow_the_shadow_intensity() {
        // the terrain's wet glint takes the lift on a classic sky only (the
        // others have it from `sun_shadow`)
        assert!(include_str!("shaders/env.wgsl").contains("*select(sh, shadow_lift(sh), is_classic_sky()))*wet;"));
        // the snow cover takes the lift too: its glints and sheen under
        // every sky, its PBR shading (the classic one lifts its own)
        let env = include_str!("shaders/env.wgsl");
        assert!(env.contains("let lifted = shadow_lift(shade);"));
        assert!(env.contains("* sunlit * lifted * 1.5;"));
        assert!(env.contains("sheen = sunlit * lifted * pow("));
        assert!(env.contains("sn, view, world, lifted, clip, false) + sheen + glint, world);"));
        assert!(env.contains("ll_classic_diffuse(base, sn, world, shade, clip, a) + sheen + glint, a);"));
    }

    #[test]
    fn the_override_follows_the_variable() {
        assert_eq!(Override::parse(None), Override::None);
        assert_eq!(Override::parse(Some("1")), Override::On);
        assert_eq!(Override::parse(Some("OFF")), Override::Off);
        assert_eq!(Override::parse(Some("alt")), Override::Alternate);
        assert!(Override::None.needed(true) && !Override::None.needed(false));
        assert!(!Override::Off.needed(true));
        assert!(Override::On.needed(false));
        assert!(Override::Alternate.on_now(false, 1) && !Override::Alternate.on_now(true, 2));
    }
}

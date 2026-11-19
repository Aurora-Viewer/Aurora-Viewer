//! Viewer-local surface effects stored in the object description.

const MARKER: &str = "[AURORA_WATER";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum Effect {
    Water = 0,
    Lava = 1,
    Energy = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Surface {
    pub face: u8,
    pub effect: Effect,
    pub tint: [u8; 3],
    pub speed: u8,
    pub volume: bool,
}

impl Surface {
    pub fn calm(face: u8) -> Self {
        Self::preset(face, Effect::Water)
    }

    pub fn preset(face: u8, effect: Effect) -> Self {
        Self {
            face,
            effect,
            tint: match effect {
                Effect::Water => [44, 97, 115],
                Effect::Lava => [255, 99, 26],
                Effect::Energy => [106, 170, 255],
            },
            speed: 100,
            volume: false,
        }
    }

    pub fn packed_tint(self) -> u32 {
        u32::from_be_bytes([0, self.tint[0], self.tint[1], self.tint[2]])
    }
}

/// The first bounded volume uses the standard closed sphere, including scaled ellipsoids.
pub(crate) fn supports_volume(shape: &aurora_prim::VolumeParams) -> bool {
    use aurora_prim::params::{LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, PathParams, ProfileParams};
    shape.sculpt.is_none()
        && shape.profile
            == (ProfileParams {
                curve_type: LL_PCODE_PROFILE_CIRCLE_HALF,
                ..Default::default()
            })
        && shape.path
            == (PathParams {
                curve_type: LL_PCODE_PATH_CIRCLE,
                ..Default::default()
            })
}

/// A single directive can stand alone or follow the creator's description.
pub(crate) fn parse(description: &str) -> Option<Surface> {
    let (text, directive) = description.trim().split_once(MARKER)?;
    if (!text.is_empty() && !text.ends_with(char::is_whitespace)) || directive.contains(MARKER) {
        return None;
    }
    let directive = directive.strip_suffix(']')?;
    if directive.is_empty() {
        return Some(Surface::calm(0));
    }
    let mut fields = directive.strip_prefix(":face=")?.split(',');
    let face = fields.next()?;
    if !face.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut surface = Surface::calm(face.parse().ok()?);
    let mut seen = 0;
    for field in fields {
        let (key, value) = field.split_once('=')?;
        let bit = match key {
            "tint" if value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) => {
                let color = u32::from_str_radix(value, 16).ok()?.to_be_bytes();
                surface.tint = [color[1], color[2], color[3]];
                1
            }
            "speed" if value.bytes().all(|byte| byte.is_ascii_digit()) => {
                surface.speed = value.parse().ok()?;
                if surface.speed > 200 {
                    return None;
                }
                2
            }
            "volume" if value == "1" => {
                surface.volume = true;
                4
            }
            "effect" => {
                surface.effect = match value {
                    "lava" => Effect::Lava,
                    "energy" => Effect::Energy,
                    _ => return None,
                };
                16
            }
            // Read markers from the earlier offline flow experiment as calm water.
            "flow" if value == "downhill" => 8,
            _ => return None,
        };
        if seen & bit != 0 {
            return None;
        }
        seen |= bit;
    }
    if seen & 1 == 0 {
        surface.tint = Surface::preset(surface.face, surface.effect).tint;
    }
    if surface.volume && surface.effect != Effect::Water {
        return None;
    }
    Some(surface)
}

/// Replace only a recognized suffix and preserve the creator's text.
pub(crate) fn with_face(description: &str, selected: Option<Surface>) -> Result<String, &'static str> {
    use std::fmt::Write as _;
    let text = if parse(description).is_some() {
        let (text, _) = description
            .split_once(MARKER)
            .ok_or_else(|| aurora_i18n::tr!("build-surface-water-invalid-marker"))?;
        text.strip_suffix(char::is_whitespace).unwrap_or(text)
    } else if description.contains(MARKER) {
        return Err(aurora_i18n::tr!("build-surface-water-invalid-marker"));
    } else {
        description
    };

    let Some(selected) = selected else {
        return Ok(text.to_owned());
    };
    let separator = if text.is_empty() { "" } else { " " };
    let mut updated = format!("{text}{separator}{MARKER}:face={}", selected.face);
    match selected.effect {
        Effect::Water => {}
        Effect::Lava => updated.push_str(",effect=lava"),
        Effect::Energy => updated.push_str(",effect=energy"),
    }
    let defaults = Surface::preset(selected.face, selected.effect);
    if selected.tint != defaults.tint {
        let _ = write!(updated, ",tint={:06X}", selected.packed_tint());
    }
    if selected.speed != defaults.speed {
        let _ = write!(updated, ",speed={}", selected.speed);
    }
    if selected.volume {
        updated.push_str(",volume=1");
    }
    updated.push(']');
    if updated.len() > 127 {
        return Err(aurora_i18n::tr!("build-surface-water-description-full"));
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::{Effect, Surface, parse, with_face};

    #[test]
    fn switches_effects_without_replacing_creator_text() {
        let water = with_face("Test surface", Some(Surface::calm(2))).unwrap();
        for effect in [Effect::Lava, Effect::Energy] {
            let preset = Surface::preset(2, effect);
            let updated = with_face(&water, Some(preset)).unwrap();
            assert_eq!(parse(&updated), Some(preset));
            assert_eq!(with_face(&updated, None).unwrap(), "Test surface");
        }
        assert!(parse("[AURORA_WATER:face=0,effect=lava,volume=1]").is_none());
    }

    fn face(text: &str) -> Option<u8> {
        parse(text).map(|surface| surface.face)
    }

    #[test]
    fn appearance_and_volume_round_trip() {
        let water = Surface {
            face: 2,
            effect: Effect::Water,
            tint: [13, 52, 79],
            speed: 45,
            volume: true,
        };
        let encoded = with_face("Floating pool", Some(water)).unwrap();
        assert_eq!(parse(&encoded), Some(water));
        assert_eq!(with_face(&encoded, None).unwrap(), "Floating pool");
        assert_eq!(parse("[AURORA_WATER:face=2,flow=downhill]"), Some(Surface::calm(2)));
        assert!(parse("[AURORA_WATER:face=2,speed=201]").is_none());
        assert!(parse("[AURORA_WATER:face=2,tint=112233,tint=AABBCC]").is_none());
    }

    #[test]
    fn editing_preserves_description_and_checks_capacity() {
        let description = "  Courtyard fountain  ";
        let marked = with_face(description, Some(Surface::calm(2))).unwrap();
        assert_eq!(face(&marked), Some(2));
        let moved = with_face(&marked, Some(Surface::calm(5))).unwrap();
        assert_eq!(face(&moved), Some(5));
        assert_eq!(with_face(&moved, None).unwrap(), description);
        let exact = with_face(&"a".repeat(105), Some(Surface::calm(0))).unwrap();
        assert_eq!(exact.len(), 127);
        assert!(with_face(&"a".repeat(106), Some(Surface::calm(0))).is_err());
        assert!(with_face("[AURORA_WATER:face=-1]", Some(Surface::calm(0))).is_err());
    }

    #[test]
    fn accepts_default_and_explicit_faces() {
        assert_eq!(face("[AURORA_WATER]"), Some(0));
        assert_eq!(face("[AURORA_WATER:face=0]"), Some(0));
        assert_eq!(face(" [AURORA_WATER:face=5] "), Some(5));
        assert_eq!(face("Courtyard fountain [AURORA_WATER:face=2]"), Some(2));
        assert_eq!(face("Pool [AURORA_WATER]"), Some(0));
    }

    #[test]
    fn rejects_invalid_and_unrelated_descriptions() {
        for description in [
            "",
            "Water fountain",
            "Pool[AURORA_WATER]",
            "Pool [AURORA_WATER] decorated",
            "[AURORA_WATER] [AURORA_WATER:face=2]",
            "[AURORA_WATER:face=]",
            "[AURORA_WATER:face=-1]",
            "[AURORA_WATER:face=+1]",
            "[AURORA_WATER:face=256]",
            "[AURORA_WATER:face=0,type=volume]",
            "[AURORA_WATER][AURORA_WATER]",
        ] {
            assert_eq!(face(description), None, "{description}");
        }
    }
}

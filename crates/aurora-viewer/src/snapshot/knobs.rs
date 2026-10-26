//! The settings of Aurora's looks (issue #969): for each look, the few
//! numbers worth changing, as sliders. A setting names its operation in
//! the look's file and the numbers of that operation it changes
//! (`LookOp::set_param`); its values are kept per look in the photo's
//! settings, the look's own as defaults. The Legacy Viewer's filters and
//! the tables have none: only their strength.

use aurora_render::looks::LookOp;

/// How a setting shows and what it holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// A number from `min` to `max`, shown from 0 to 100 %.
    Range { min: f32, max: f32 },
    /// A distance in metres.
    Meters { min: f32, max: f32 },
    /// A width over height ratio (2.39:1).
    Ratio { min: f32, max: f32 },
    /// A hue, in degrees.
    Hue,
    /// A whole number that draws another variant.
    Seed,
    /// A colour: three numbers.
    Color,
}

/// One setting of a look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Knob {
    /// Its name's message.
    pub label: &'static str,
    /// The operation of the look it changes.
    pub op: usize,
    /// The numbers of the operation it sets: all to its value, or the
    /// three of a colour.
    pub params: &'static [usize],
    pub kind: Kind,
    /// Shown as 1 - the number (a vignette's edge brightness shown as its
    /// strength).
    pub invert: bool,
}

const fn range(label: &'static str, op: usize, params: &'static [usize], min: f32, max: f32) -> Knob {
    Knob {
        label,
        op,
        params,
        kind: Kind::Range { min, max },
        invert: false,
    }
}

const fn color(label: &'static str, op: usize, params: &'static [usize]) -> Knob {
    Knob {
        label,
        op,
        params,
        kind: Kind::Color,
        invert: false,
    }
}

const fn vignette(op: usize) -> Knob {
    Knob {
        label: "photo-knob-vignette",
        op,
        params: &[0],
        kind: Kind::Range { min: 0.0, max: 1.0 },
        invert: true,
    }
}

const ALPHA: &[usize] = &[3, 4, 5];

/// The settings of one of Aurora's looks (the operations of its file in
/// `aurora-render/src/looks/aurora/`).
pub fn knobs(id: &str) -> &'static [Knob] {
    const SATURATION: &str = "photo-knob-saturation";
    const CONTRAST: &str = "photo-knob-contrast";
    match id {
        "fine-grain" => {
            const K: &[Knob] = &[
                range("photo-knob-amount", 0, &[0], 0.0, 0.15),
                range("photo-knob-grain-size", 0, &[1], 1.0, 4.0),
            ];
            K
        }
        "silver-grain" => {
            const K: &[Knob] = &[
                range(CONTRAST, 1, &[0], 0.8, 1.8),
                range("photo-knob-grain", 2, &[0], 0.0, 0.2),
                range("photo-knob-grain-size", 2, &[1], 1.0, 4.0),
            ];
            K
        }
        "teal-orange" => {
            const K: &[Knob] = &[
                range("photo-knob-strength", 0, &[0], 0.0, 1.0),
                color("photo-knob-shadows", 0, &[1, 2, 3]),
                color("photo-knob-lights", 0, &[4, 5, 6]),
                range(SATURATION, 1, &[0], 0.5, 1.8),
            ];
            K
        }
        "bleach-bypass" => {
            const K: &[Knob] = &[
                range(SATURATION, 0, &[0], 0.0, 1.0),
                range(CONTRAST, 1, &[0], 1.0, 2.0),
                range("photo-knob-grain", 2, &[0], 0.0, 0.1),
            ];
            K
        }
        "cross-process" => {
            const K: &[Knob] = &[
                range("photo-knob-strength", 0, &[0], 0.0, 1.0),
                range(CONTRAST, 1, &[0], 1.0, 1.6),
                range(SATURATION, 2, &[0], 0.5, 1.8),
            ];
            K
        }
        "instant" => {
            const K: &[Knob] = &[
                range("photo-knob-lift", 0, ALPHA, 0.0, 0.4),
                range("photo-knob-warmth", 1, ALPHA, 0.0, 0.3),
                range(SATURATION, 2, &[0], 0.3, 1.5),
                vignette(3),
                range("photo-knob-border", 6, &[0], 0.0, 0.12),
                range("photo-knob-border-bottom", 6, &[1], 0.0, 0.3),
                color("photo-knob-border-color", 6, &[2, 3, 4]),
            ];
            K
        }
        "chrome" => {
            const K: &[Knob] = &[range(SATURATION, 0, &[0], 1.0, 2.0), range(CONTRAST, 1, &[0], 1.0, 1.5)];
            K
        }
        "halation" => {
            const K: &[Knob] = &[
                range("photo-knob-strength", 0, &[0], 0.0, 2.0),
                range("photo-knob-radius", 0, &[1], 0.002, 0.04),
                range("photo-knob-threshold", 0, &[2], 0.3, 0.95),
                color("photo-knob-tint", 0, &[3, 4, 5]),
                range("photo-knob-grain", 2, &[0], 0.0, 0.1),
            ];
            K
        }
        "cyanotype" | "duotone" | "sunset" => {
            const K: &[Knob] = &[
                color("photo-knob-shadows", 0, &[0, 1, 2]),
                color("photo-knob-middle", 0, &[3, 4, 5]),
                color("photo-knob-lights", 0, &[6, 7, 8]),
            ];
            K
        }
        "infrared" => {
            const K: &[Knob] = &[range(CONTRAST, 1, &[0], 0.8, 1.6), range(SATURATION, 2, &[0], 0.5, 2.0)];
            K
        }
        "vivid" => {
            const K: &[Knob] = &[range(SATURATION, 0, &[0], 1.0, 2.0), range(CONTRAST, 1, &[0], 1.0, 1.4)];
            K
        }
        "faded" => {
            const K: &[Knob] = &[range("photo-knob-lift", 0, ALPHA, 0.0, 0.4), range(SATURATION, 1, &[0], 0.0, 1.0)];
            K
        }
        "keep-red" | "keep-yellow" | "keep-green" | "keep-blue" => {
            const K: &[Knob] = &[
                Knob {
                    label: "photo-knob-hue",
                    op: 0,
                    params: &[0],
                    kind: Kind::Hue,
                    invert: false,
                },
                range("photo-knob-tolerance", 0, &[1], 5.0, 90.0),
            ];
            K
        }
        "night-vision" => {
            const K: &[Knob] = &[
                color("photo-knob-color", 1, &[3, 4, 5]),
                range("photo-knob-grain", 2, &[0], 0.0, 0.2),
                range("photo-knob-lines", 3, &[1], 0.0, 0.8),
                vignette(5),
            ];
            K
        }
        "chromatic" => {
            const K: &[Knob] = &[range("photo-knob-strength", 0, &[0], 0.0, 0.03)];
            K
        }
        "fisheye" => {
            const K: &[Knob] = &[range("photo-knob-curvature", 0, &[0], -0.5, 0.9)];
            K
        }
        "old-lens" => {
            const K: &[Knob] = &[
                range("photo-knob-distortion", 0, &[0], -0.3, 0.5),
                range("photo-knob-fringes", 1, &[0], 0.0, 0.02),
                range("photo-knob-glow", 2, &[0], 0.0, 1.5),
                vignette(3),
            ];
            K
        }
        "scope" | "widescreen" => {
            const K: &[Knob] = &[
                Knob {
                    label: "photo-knob-format",
                    op: 0,
                    params: &[0],
                    kind: Kind::Ratio { min: 1.33, max: 2.76 },
                    invert: false,
                },
                color("photo-knob-bar-color", 0, &[1, 2, 3]),
            ];
            K
        }
        "dreamy" => {
            const K: &[Knob] = &[
                range("photo-knob-strength", 0, &[0], 0.0, 2.0),
                range("photo-knob-radius", 0, &[1], 0.005, 0.05),
                range("photo-knob-threshold", 0, &[2], 0.2, 0.95),
            ];
            K
        }
        "pixel" => {
            const K: &[Knob] = &[range("photo-knob-pixel-size", 0, &[0], 0.002, 0.03)];
            K
        }
        "vhs" => {
            const K: &[Knob] = &[
                range("photo-knob-tears", 0, &[0], 0.0, 0.02),
                range("photo-knob-fringes", 1, &[0], 0.0, 0.015),
                range("photo-knob-noise", 3, &[0], 0.0, 0.2),
                range("photo-knob-lines", 4, &[1], 0.0, 0.8),
                Knob {
                    label: "photo-knob-variant",
                    op: 0,
                    params: &[2],
                    kind: Kind::Seed,
                    invert: false,
                },
            ];
            K
        }
        "glitch" => {
            const K: &[Knob] = &[
                range("photo-knob-amplitude", 0, &[0], 0.0, 0.08),
                range("photo-knob-bands", 0, &[1], 0.005, 0.1),
                Knob {
                    label: "photo-knob-variant",
                    op: 0,
                    params: &[2],
                    kind: Kind::Seed,
                    invert: false,
                },
            ];
            K
        }
        "oil-paint" => {
            const K: &[Knob] = &[
                range("photo-knob-radius", 0, &[0], 0.001, 0.006),
                range(SATURATION, 1, &[0], 0.8, 1.6),
            ];
            K
        }
        "sketch" => {
            const K: &[Knob] = &[range(CONTRAST, 3, &[0], 1.0, 3.0)];
            K
        }
        "clarity" => {
            const K: &[Knob] = &[
                range("photo-knob-strength", 0, &[0], 0.0, 2.0),
                range("photo-knob-radius", 0, &[1], 0.005, 0.05),
            ];
            K
        }
        "haze" | "golden-haze" => {
            const K: &[Knob] = &[
                Knob {
                    label: "photo-knob-start",
                    op: 0,
                    params: &[0],
                    kind: Kind::Meters { min: 0.0, max: 200.0 },
                    invert: false,
                },
                Knob {
                    label: "photo-knob-end",
                    op: 0,
                    params: &[1],
                    kind: Kind::Meters { min: 10.0, max: 600.0 },
                    invert: false,
                },
                range("photo-knob-density", 0, &[2], 0.0, 1.0),
                color("photo-knob-color", 0, &[3, 4, 5]),
            ];
            K
        }
        "miniature-3d" => {
            const K: &[Knob] = &[
                range("photo-knob-blur", 0, &[0], 0.0, 0.03),
                range("photo-knob-sharp-zone", 0, &[1], 0.05, 1.0),
                range(SATURATION, 1, &[0], 1.0, 2.0),
            ];
            K
        }
        "background-blur" => {
            const K: &[Knob] = &[
                range("photo-knob-blur", 0, &[0], 0.0, 0.03),
                range("photo-knob-sharp-zone", 0, &[1], 0.05, 1.0),
            ];
            K
        }
        "subject" => {
            const K: &[Knob] = &[
                range("photo-knob-desaturate", 0, &[0], 0.0, 1.0),
                range("photo-knob-darken", 0, &[1], 0.0, 1.0),
                range("photo-knob-subject-zone", 0, &[2], 0.05, 1.0),
            ];
            K
        }
        _ => &[],
    }
}

impl Knob {
    /// The numbers it holds: three for a colour, one otherwise.
    pub fn width(&self) -> usize {
        if self.kind == Kind::Color { 3 } else { 1 }
    }

    /// Its values in a look's operations.
    pub fn read(&self, ops: &[LookOp]) -> Vec<f32> {
        let Some(op) = ops.get(self.op) else {
            return vec![0.0; self.width()];
        };
        let values = self.params.iter().take(self.width()).map(|&i| op.param(i).unwrap_or(0.0));
        values.map(|v| if self.invert { 1.0 - v } else { v }).collect()
    }

    /// Sets its values in a look's operations.
    pub fn write(&self, ops: &mut [LookOp], values: &[f32]) {
        let Some(op) = ops.get_mut(self.op) else { return };
        for (k, &param) in self.params.iter().enumerate() {
            let v = if self.width() == 3 { values.get(k) } else { values.first() };
            if let Some(&v) = v {
                op.set_param(param, if self.invert { 1.0 - v } else { v });
            }
        }
    }
}

/// The values of every setting of a look, in order (the look's own).
pub fn defaults(knobs: &[Knob], ops: &[LookOp]) -> Vec<f32> {
    knobs.iter().flat_map(|k| k.read(ops)).collect()
}

/// A look's operations with its settings' values (the defaults where the
/// values do not match its settings, saved by another version).
pub fn apply(knobs: &[Knob], ops: &[LookOp], values: &[f32]) -> Vec<LookOp> {
    let mut ops = ops.to_vec();
    if values.len() != knobs.iter().map(Knob::width).sum::<usize>() {
        return ops;
    }
    let mut at = 0;
    for knob in knobs {
        let width = knob.width();
        knob.write(&mut ops, &values[at..at + width]);
        at += width;
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_render::looks::aurora_looks;

    #[test]
    fn every_setting_names_a_number_of_its_look() {
        for look in aurora_looks() {
            let knobs = knobs(look.id);
            assert!(!knobs.is_empty(), "{} has no setting", look.id);
            for knob in knobs {
                let op = look
                    .ops
                    .get(knob.op)
                    .unwrap_or_else(|| panic!("{}: no operation {}", look.id, knob.op));
                for &param in knob.params {
                    assert!(op.param(param).is_some(), "{}: {:?} has no number {param}", look.id, op);
                }
                assert!(knob.params.len() == 3 || knob.kind != Kind::Color, "{}", look.id);
                let label = aurora_i18n::text(knob.label);
                assert_ne!(label, knob.label, "{} has no text", knob.label);
            }
            // the defaults are the look's own: applied, nothing changes
            let values = defaults(knobs, &look.ops);
            assert_eq!(apply(knobs, &look.ops, &values), look.ops.to_vec(), "{}", look.id);
        }
    }

    #[test]
    fn a_setting_changes_its_numbers() {
        let look = aurora_render::looks::aurora_look("instant").expect("instant");
        let knobs = knobs("instant");
        let mut values = defaults(knobs, &look.ops);
        // the warmth sets the three alphas of its colorize
        values[1] = 0.25;
        // the vignette is shown as its strength
        values[3] = 0.8;
        let ops = apply(knobs, &look.ops, &values);
        assert_eq!(ops[1].param(3), Some(0.25));
        assert_eq!(ops[1].param(5), Some(0.25));
        assert!((ops[3].param(0).unwrap_or(0.0) - 0.2).abs() < 1e-6);
        // values of another length are left out
        assert_eq!(apply(knobs, &look.ops, &[1.0]), look.ops.to_vec());
    }
}

//! The looks' files: the Legacy Viewer's filters
//! (indra/newview/app_settings/filters/*.xml, unchanged in `legacy/`), an
//! LLSD array of operations, each an array of its name and parameters. Read
//! like LLImageFilter::executeFilter (indra/llimage/llimagefilter.cpp,
//! originally LGPL 2.1): a missing parameter reads 0, an unknown shape,
//! blend mode or screen takes the first one, an unknown operation is left
//! out with a warning. Aurora's looks (`aurora/`) are files of the same
//! form, with the names of Aurora's operations besides.

use super::{LookOp, MAX_OPS, ScreenMode, StencilMode, StencilShape};
use aurora_llsd::Llsd;
use std::sync::{Arc, OnceLock};

/// The files of the Legacy Viewer, by their name (the file's, untranslated
/// there; the interface names them).
const LEGACY: [(&str, &str); 30] = [
    ("Antique", include_str!("legacy/Antique.xml")),
    ("Autocontrast", include_str!("legacy/Autocontrast.xml")),
    ("Badtrip", include_str!("legacy/Badtrip.xml")),
    ("BlackAndWhite", include_str!("legacy/BlackAndWhite.xml")),
    ("Blownhighlights", include_str!("legacy/Blownhighlights.xml")),
    ("Brighten", include_str!("legacy/Brighten.xml")),
    ("Cartoon", include_str!("legacy/Cartoon.xml")),
    ("Colors1970", include_str!("legacy/Colors1970.xml")),
    ("Darken", include_str!("legacy/Darken.xml")),
    ("Edges", include_str!("legacy/Edges.xml")),
    ("Focus", include_str!("legacy/Focus.xml")),
    ("Heatwave", include_str!("legacy/Heatwave.xml")),
    ("Intense", include_str!("legacy/Intense.xml")),
    ("Julesverne", include_str!("legacy/Julesverne.xml")),
    ("LensFlare", include_str!("legacy/LensFlare.xml")),
    ("Lightleak", include_str!("legacy/Lightleak.xml")),
    ("Linearize", include_str!("legacy/Linearize.xml")),
    ("Miniature", include_str!("legacy/Miniature.xml")),
    ("Negative", include_str!("legacy/Negative.xml")),
    ("Newspaper", include_str!("legacy/Newspaper.xml")),
    ("Overcast", include_str!("legacy/Overcast.xml")),
    ("Posterize", include_str!("legacy/Posterize.xml")),
    ("Rotatecolors180", include_str!("legacy/Rotatecolors180.xml")),
    ("Sepia", include_str!("legacy/Sepia.xml")),
    ("Sharpen", include_str!("legacy/Sharpen.xml")),
    ("Softfocus", include_str!("legacy/Softfocus.xml")),
    ("Spotlight", include_str!("legacy/Spotlight.xml")),
    ("Thematrix", include_str!("legacy/Thematrix.xml")),
    ("Toycamera", include_str!("legacy/Toycamera.xml")),
    ("Video", include_str!("legacy/Video.xml")),
];

/// The sections of Aurora's looks, in the order the interface shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuroraSection {
    Film,
    Color,
    Lens,
    Creative,
    Depth,
}

/// One of Aurora's looks.
#[derive(Debug, Clone)]
pub struct AuroraLook {
    pub id: &'static str,
    pub section: AuroraSection,
    pub ops: Arc<[LookOp]>,
}

macro_rules! aurora_files {
    ($($section:ident: [$($id:literal),* $(,)?]),* $(,)?) => {
        [$($((AuroraSection::$section, $id, include_str!(concat!("aurora/", $id, ".xml")))),*),*]
    };
}

/// Aurora's looks, by section.
const AURORA: [(AuroraSection, &str, &str); 36] = aurora_files!(
    Film: ["fine-grain", "silver-grain", "teal-orange", "bleach-bypass", "cross-process", "instant", "chrome", "halation", "cyanotype", "infrared"],
    Color: ["vivid", "faded", "keep-red", "keep-yellow", "keep-green", "keep-blue", "duotone", "sunset", "night-vision"],
    Lens: ["chromatic", "fisheye", "old-lens", "scope", "widescreen", "dreamy"],
    Creative: ["pixel", "vhs", "glitch", "oil-paint", "sketch", "clarity"],
    Depth: ["haze", "golden-haze", "miniature-3d", "background-blur", "subject"],
);

/// Aurora's looks, in their sections' order, read once.
pub fn aurora_looks() -> &'static [AuroraLook] {
    static LOOKS: OnceLock<Vec<AuroraLook>> = OnceLock::new();
    LOOKS.get_or_init(|| {
        AURORA
            .iter()
            .filter_map(|&(section, id, xml)| match parse_look(xml.as_bytes()) {
                Ok(ops) => Some(AuroraLook {
                    id,
                    section,
                    ops: ops.into(),
                }),
                Err(e) => {
                    log::warn!("photo look {id}: {e}");
                    None
                }
            })
            .collect()
    })
}

/// One of Aurora's looks by its id.
pub fn aurora_look(id: &str) -> Option<&'static AuroraLook> {
    aurora_looks().iter().find(|look| look.id == id)
}

/// The Legacy Viewer's looks, by name, read once.
pub fn legacy_looks() -> &'static [(&'static str, Arc<[LookOp]>)] {
    static LOOKS: OnceLock<Vec<(&'static str, Arc<[LookOp]>)>> = OnceLock::new();
    LOOKS.get_or_init(|| {
        LEGACY
            .iter()
            .filter_map(|&(name, xml)| match parse_look(xml.as_bytes()) {
                Ok(ops) => Some((name, ops.into())),
                Err(e) => {
                    log::warn!("photo look {name}: {e}");
                    None
                }
            })
            .collect()
    })
}

/// A Legacy Viewer's look by its name.
pub fn legacy_look(name: &str) -> Option<Arc<[LookOp]>> {
    legacy_looks().iter().find(|(n, _)| *n == name).map(|(_, ops)| ops.clone())
}

#[derive(Debug, thiserror::Error)]
pub enum LookError {
    #[error("not an LLSD file: {0}")]
    Llsd(#[from] aurora_llsd::LlsdError),
    #[error("not an array of operations")]
    NotArray,
    #[error("more than {MAX_OPS} operations")]
    TooLong,
}

fn rgb(v: &Llsd, first: usize) -> [f32; 3] {
    [v.at(first).as_f32(), v.at(first + 1).as_f32(), v.at(first + 2).as_f32()]
}

/// The operations of a filter file.
pub fn parse_look(xml: &[u8]) -> Result<Vec<LookOp>, LookError> {
    let doc = aurora_llsd::from_xml(xml)?;
    if !doc.is_array() {
        return Err(LookError::NotArray);
    }
    if doc.len() > MAX_OPS {
        return Err(LookError::TooLong);
    }
    let mut ops = Vec::with_capacity(doc.len());
    for v in doc.as_array() {
        let op = match v.at(0).as_str() {
            "stencil" => {
                let shape = match v.at(1).as_str() {
                    "gradient" => StencilShape::Gradient,
                    "vignette" => StencilShape::Vignette,
                    "scanlines" => StencilShape::ScanLines,
                    _ => StencilShape::Uniform,
                };
                let mode = match v.at(2).as_str() {
                    "add" => StencilMode::Add,
                    "add_back" => StencilMode::AddBack,
                    "fade" => StencilMode::Fade,
                    _ => StencilMode::Blend,
                };
                LookOp::Stencil {
                    shape,
                    mode,
                    min: v.at(3).as_f32(),
                    max: v.at(4).as_f32(),
                    params: [v.at(5).as_f32(), v.at(6).as_f32(), v.at(7).as_f32(), v.at(8).as_f32()],
                }
            }
            "sepia" => LookOp::Sepia,
            "grayscale" => LookOp::GrayScale,
            "saturate" => LookOp::Saturate(v.at(1).as_f32()),
            "rotate" => LookOp::Rotate(v.at(1).as_f32()),
            "gamma" => LookOp::Gamma {
                gamma: v.at(1).as_f32(),
                alpha: rgb(v, 2),
            },
            "colorize" => LookOp::Colorize {
                color: rgb(v, 1),
                alpha: rgb(v, 4),
            },
            "contrast" => LookOp::Contrast {
                slope: v.at(1).as_f32(),
                alpha: rgb(v, 2),
            },
            "brighten" => LookOp::Brightness {
                step: v.at(1).as_f32(),
                alpha: rgb(v, 2),
            },
            "darken" => LookOp::Brightness {
                step: -v.at(1).as_f32(),
                alpha: rgb(v, 2),
            },
            "linearize" => LookOp::Linearize {
                tail: v.at(1).as_f32(),
                alpha: rgb(v, 2),
            },
            "posterize" => LookOp::Posterize {
                // (S32) there
                classes: v.at(1).as_f64().trunc() as f32,
                alpha: rgb(v, 2),
            },
            "screen" => LookOp::Screen {
                mode: if v.at(1).as_str() == "line" {
                    ScreenMode::Line
                } else {
                    ScreenMode::Sine2d
                },
                wavelength: v.at(2).as_f32(),
                angle: v.at(3).as_f32(),
            },
            "blur" => LookOp::blur(),
            "sharpen" => LookOp::sharpen(),
            "gradient" => LookOp::gradient(),
            "convolve" => {
                let mut kernel = [[0.0; 3]; 3];
                for (k, value) in kernel.as_flattened_mut().iter_mut().enumerate() {
                    *value = v.at(3 + k).as_f32();
                }
                LookOp::Convolve {
                    kernel,
                    normalize: v.at(1).as_f64() > 0.0,
                    abs: v.at(2).as_f64() > 0.0,
                }
            }
            "grain" => LookOp::Grain {
                amount: v.at(1).as_f32(),
                size: v.at(2).as_f32(),
                mono: v.at(3).as_f64() > 0.0,
            },
            "chroma" => LookOp::Chroma(v.at(1).as_f32()),
            "distort" => LookOp::Distort(v.at(1).as_f32()),
            "bars" => LookOp::Bars {
                ratio: v.at(1).as_f32(),
                color: rgb(v, 2),
            },
            "frame" => LookOp::Frame {
                side: v.at(1).as_f32(),
                bottom: v.at(2).as_f32(),
                color: rgb(v, 3),
            },
            "selective" => LookOp::Selective {
                hue: v.at(1).as_f32(),
                width: v.at(2).as_f32(),
                soft: v.at(3).as_f32(),
            },
            "gradientmap" => LookOp::GradientMap([rgb(v, 1), rgb(v, 4), rgb(v, 7)]),
            "split" => LookOp::Split {
                amount: v.at(1).as_f32(),
                shadows: rgb(v, 2),
                lights: rgb(v, 5),
            },
            "pixelate" => LookOp::Pixelate(v.at(1).as_f32()),
            "glitch" => LookOp::Glitch {
                amount: v.at(1).as_f32(),
                band: v.at(2).as_f32(),
                seed: v.at(3).as_f32(),
            },
            "paint" => LookOp::Paint(v.at(1).as_f32()),
            "clarity" => LookOp::Clarity {
                amount: v.at(1).as_f32(),
                radius: v.at(2).as_f32(),
            },
            "glow" => LookOp::Glow {
                amount: v.at(1).as_f32(),
                radius: v.at(2).as_f32(),
                threshold: v.at(3).as_f32(),
                tint: rgb(v, 4),
            },
            "fog" => LookOp::Fog {
                start: v.at(1).as_f32(),
                end: v.at(2).as_f32(),
                max: v.at(3).as_f32(),
                color: rgb(v, 4),
            },
            "depthblur" => LookOp::DepthBlur {
                radius: v.at(1).as_f32(),
                range: v.at(2).as_f32(),
                behind: v.at(3).as_f64() > 0.0,
            },
            "subject" => LookOp::Subject {
                desaturate: v.at(1).as_f32(),
                darken: v.at(2).as_f32(),
                tolerance: v.at(3).as_f32(),
            },
            "colortransform" => {
                let mut m = [[0.0; 3]; 3];
                for (k, value) in m.as_flattened_mut().iter_mut().enumerate() {
                    *value = v.at(1 + k).as_f32();
                }
                LookOp::ColorTransform(m)
            }
            other => {
                log::warn!("photo look: unknown operation {other:?} left out");
                continue;
            }
        };
        ops.push(op);
    }
    Ok(ops)
}

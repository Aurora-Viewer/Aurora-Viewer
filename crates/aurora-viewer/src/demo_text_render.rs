//! AURORA_DEMO_TEXT_RENDER: the ways epaint can rasterize the interface
//! text, to compare them live in the same window. epaint draws glyphs in
//! grayscale coverage (skrifa outlines, one alpha channel in the font
//! atlas): what can change is whether the outlines are hinted and how
//! (`FontTweak::hinting_target`), whether a glyph is drawn at four
//! horizontal sub-pixel offsets (`TextOptions::subpixel_binning`), and the
//! curve from coverage to alpha (`TextOptions::color_transfer_function`),
//! which makes strokes lighter or heavier. A temporary comparison: once a
//! rendering is chosen it moves to the theme, and this goes.
//!
//! `AURORA_DEMO_TEXT_RENDER=<variant>|cycle[,scale=<zoom>][,font=<name>]`:
//! one variant of [`VARIANTS`], or each in turn every [`CYCLE_SECONDS`];
//! `scale` is the interface zoom of the launch (Preferences' "UI scale",
//! not saved), `font` an interface font of the launch (`Inter`, `Noto
//! Sans`, `Roboto`). The variant's name stays in the bottom left corner,
//! over a specimen of light text on glass and dark text on a light surface
//! at 11 to 14 points.

use crate::theme::Palette;
use crate::ui::fonts;
use egui::epaint::text::{HintingTarget, SmoothHinting};
use egui::epaint::FontColorTransferFunction;
use egui::{FontId, RichText, Vec2};
use std::time::{Duration, Instant};

/// How long each variant stays in `cycle` mode.
pub const CYCLE_SECONDS: u64 = 4;

/// One way of rasterizing the text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Variant {
    pub name: &'static str,
    /// `TextOptions::subpixel_binning`.
    pub binning: bool,
    /// `TextOptions::font_hinting`.
    pub hinting: bool,
    /// `FontTweak::hinting_target` of every face; None leaves epaint's.
    pub target: Option<HintingTarget>,
    /// `TextOptions::color_transfer_function`.
    pub transfer: FontColorTransferFunction,
}

/// What egui's dark theme does today (`Visuals::dark`).
const DEFAULT: Variant = Variant {
    name: "default",
    binning: true,
    hinting: true,
    target: None,
    transfer: FontColorTransferFunction::DARK_MODE_DEFAULT,
};

const fn smooth(light: bool, symmetric_rendering: bool, preserve_linear_metrics: bool) -> Option<HintingTarget> {
    Some(HintingTarget::Smooth(SmoothHinting {
        light,
        symmetric_rendering,
        preserve_linear_metrics,
    }))
}

pub const VARIANTS: &[Variant] = &[
    DEFAULT,
    // each glyph at a whole pixel: sharper, spacing a little less even
    Variant {
        name: "nobin",
        binning: false,
        ..DEFAULT
    },
    // FreeType's "light": stems snapped vertically only
    Variant {
        name: "light",
        target: smooth(true, true, true),
        ..DEFAULT
    },
    // horizontal grid-fitting too, rendering tied to the pixel position
    Variant {
        name: "crisp",
        target: smooth(false, false, false),
        ..DEFAULT
    },
    Variant {
        name: "crisp-nobin",
        binning: false,
        target: smooth(false, false, false),
        ..DEFAULT
    },
    // hinting made for 1-bit rendering, drawn anti-aliased
    Variant {
        name: "mono",
        target: Some(HintingTarget::Mono),
        ..DEFAULT
    },
    Variant {
        name: "nohint",
        hinting: false,
        ..DEFAULT
    },
    // coverage → alpha: 2c - c² is close to c^0.5; a higher exponent
    // makes the strokes lighter, a lower one heavier
    Variant {
        name: "gamma-0.75",
        transfer: FontColorTransferFunction::Gamma(0.75),
        ..DEFAULT
    },
    Variant {
        name: "linear",
        transfer: FontColorTransferFunction::Off,
        ..DEFAULT
    },
    Variant {
        name: "gamma-0.4",
        transfer: FontColorTransferFunction::Gamma(0.4),
        ..DEFAULT
    },
    Variant {
        name: "nobin-gamma-0.75",
        binning: false,
        transfer: FontColorTransferFunction::Gamma(0.75),
        ..DEFAULT
    },
];

/// The parsed switch.
#[derive(Debug, Clone, PartialEq)]
pub struct Switch {
    /// None: every variant in turn.
    pub variant: Option<usize>,
    pub scale: Option<f32>,
    pub font: Option<String>,
}

impl Switch {
    pub fn parse(value: &str) -> Option<Switch> {
        let mut parts = value.split(',').map(str::trim);
        let variant = match parts.next()? {
            "cycle" => None,
            name => Some(VARIANTS.iter().position(|v| v.name == name)?),
        };
        let mut switch = Switch {
            variant,
            scale: None,
            font: None,
        };
        for part in parts {
            match part.split_once('=') {
                Some(("scale", v)) => switch.scale = v.parse::<f32>().ok().filter(|s| (0.5..=3.0).contains(s)),
                Some(("font", v)) => switch.font = fonts::builtin(v).map(|b| b.name.to_owned()),
                _ => log::warn!("AURORA_DEMO_TEXT_RENDER: '{part}' ignored"),
            }
        }
        Some(switch)
    }

    fn from_env() -> Option<Switch> {
        let value = std::env::var("AURORA_DEMO_TEXT_RENDER").ok()?;
        let switch = Switch::parse(&value);
        if switch.is_none() {
            let names: Vec<&str> = VARIANTS.iter().map(|v| v.name).collect();
            log::warn!("AURORA_DEMO_TEXT_RENDER={value}: unknown variant (cycle, {})", names.join(", "));
        }
        switch
    }
}

/// The interface font and zoom of the launch, in the settings in memory
/// (demo mode only; nothing is saved by it).
pub fn override_settings(settings: &mut crate::settings::Settings) {
    let Some(switch) = Switch::from_env() else { return };
    if let Some(font) = switch.font {
        settings.font = font;
    }
    if let Some(scale) = switch.scale {
        settings.ui_scale = scale;
    }
}

/// The comparison running in the viewer.
pub struct TextRender {
    switch: Switch,
    started: Instant,
    /// The variant installed, by index.
    applied: Option<usize>,
}

impl TextRender {
    pub fn from_env() -> Option<TextRender> {
        let switch = Switch::from_env()?;
        log::info!("text rendering comparison: {switch:?}");
        Some(TextRender {
            switch,
            started: Instant::now(),
            applied: None,
        })
    }

    fn current(&self) -> usize {
        self.switch.variant.unwrap_or_else(|| {
            let step = self.started.elapsed().as_secs() / CYCLE_SECONDS;
            (step % VARIANTS.len() as u64) as usize
        })
    }

    /// Before the pass: the variant's text options (the theme puts egui's
    /// back whenever it is applied), its faces when the hinting target
    /// changes, and a repaint at the next change of `cycle`.
    pub fn before_pass(&mut self, ctx: &egui::Context, font: &str) {
        let index = self.current();
        let variant = VARIANTS[index];
        ctx.all_styles_mut(|style| {
            let options = &mut style.visuals.text_options;
            options.subpixel_binning = variant.binning;
            options.font_hinting = variant.hinting;
            options.color_transfer_function = variant.transfer;
        });
        let target = |i: usize| VARIANTS[i].target;
        if self.applied.is_none_or(|before| target(before) != variant.target) {
            fonts::set_hinting_target(variant.target);
            fonts::apply(ctx, font);
        }
        if self.applied != Some(index) {
            log::info!("text rendering: {}", variant.name);
            self.applied = Some(index);
        }
        if let Some(scale) = self.switch.scale {
            ctx.set_zoom_factor(scale);
        }
        if self.switch.variant.is_none() {
            let next = (self.started.elapsed().as_secs() / CYCLE_SECONDS + 1) * CYCLE_SECONDS;
            ctx.request_repaint_after(Duration::from_secs(next).saturating_sub(self.started.elapsed()));
        }
    }

    /// The variant's name in the bottom left corner, over the specimen.
    pub fn paint(&self, ctx: &egui::Context, p: &Palette) {
        let variant = VARIANTS[self.current()];
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("demo_text_render"))
            .fade_in(false)
            .order(egui::Order::Tooltip)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(screen.left_bottom() + Vec2::new(16.0, -16.0))
            .interactable(false)
            .show(ctx, |ui| {
                crate::ui::widgets::overlay(ui, p, egui::Margin::same(12), |ui| specimen(ui, p, variant.name));
            });
    }
}

const SIZES: [f32; 4] = [11.0, 12.0, 13.0, 14.0];

fn specimen(ui: &mut egui::Ui, p: &Palette, name: &str) {
    ui.spacing_mut().item_spacing.y = 3.0;
    let semibold = fonts::semibold(ui.ctx());
    let pangram = aurora_i18n::tr!("demo-text-render-pangram");
    let sample = aurora_i18n::tr!("font-sample");
    ui.label(
        RichText::new(aurora_i18n::tr!("demo-text-render-title", variant = name.to_string()))
            .font(FontId::new(14.0, semibold.clone()))
            .color(p.ink),
    );
    ui.add_space(4.0);
    ui.label(RichText::new(aurora_i18n::tr!("demo-text-render-on-glass")).size(11.0).color(p.muted));
    for size in SIZES {
        ui.label(RichText::new(format!("{size:.0}  {pangram}")).size(size).color(p.ink));
    }
    for size in [11.0, 12.5] {
        ui.label(RichText::new(sample).size(size).color(p.muted));
        ui.label(RichText::new(pangram).font(FontId::new(size, semibold.clone())).color(p.ink));
    }
    ui.add_space(6.0);
    for (fill, ink) in [(p.ink, p.bar), (p.violet, p.on_accent)] {
        egui::Frame::new()
            .fill(fill)
            .corner_radius(ui.visuals().window_corner_radius)
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                if fill == p.ink {
                    ui.label(RichText::new(aurora_i18n::tr!("demo-text-render-on-light")).size(11.0).color(ink));
                }
                for size in SIZES {
                    ui.label(RichText::new(format!("{size:.0}  {pangram}")).size(size).color(ink));
                }
                ui.label(RichText::new(pangram).font(FontId::new(12.5, semibold.clone())).color(ink));
            });
        ui.add_space(4.0);
    }
    // the variant's name alone, as large as a window title, for a glance
    ui.label(RichText::new(name).font(FontId::new(16.0, semibold)).color(p.violet_light));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_names_a_variant_or_cycles() {
        assert_eq!(
            Switch::parse("nobin"),
            Some(Switch {
                variant: Some(1),
                scale: None,
                font: None
            })
        );
        let cycle = Switch::parse("cycle, scale=1.25, font=Noto Sans").expect("cycle");
        assert_eq!(cycle.variant, None);
        assert_eq!(cycle.scale, Some(1.25));
        assert_eq!(cycle.font.as_deref(), Some("Noto Sans"));
        assert_eq!(Switch::parse("sharpest"), None);
        // a scale or a font out of reach is left out, not the variant
        let odd = Switch::parse("default,scale=9,font=Comic").expect("default");
        assert_eq!((odd.variant, odd.scale, odd.font), (Some(0), None, None));
    }

    #[test]
    fn the_variants_have_distinct_names_and_the_first_is_egui_s() {
        let mut names: Vec<&str> = VARIANTS.iter().map(|v| v.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), VARIANTS.len());
        let egui = egui::Visuals::dark().text_options;
        let first = VARIANTS[0];
        assert_eq!(
            (first.binning, first.hinting, first.transfer, first.target),
            (egui.subpixel_binning, egui.font_hinting, egui.color_transfer_function, None)
        );
    }
}

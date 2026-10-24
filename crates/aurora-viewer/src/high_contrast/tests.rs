use super::*;
use crate::colorblind::{Deficiency, Vision};
use egui::Color32;

/// Relative luminance (WCAG 2), of an opaque colour.
fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// Contrast ratio (WCAG 2) between two opaque colours, 1 to 21.
fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// WCAG 2 AAA for text, and the non-text minimum for borders and faces.
const TEXT: f32 = 7.0;
const UI: f32 = 3.0;

/// The hovered button's face, as `Theme::apply` and `action_button` draw it.
fn hot(p: &Palette) -> Color32 {
    p.raised.lerp_to_gamma(p.ink, 0.16)
}

fn assert_ratio(name: &str, a: Color32, b: Color32, min: f32) {
    let r = contrast(a, b);
    assert!(r >= min, "{name}: {r:.2}:1 < {min}:1");
}

#[test]
fn contrast_matches_wcag_reference_values() {
    assert!((contrast(hex("#000000"), hex("#FFFFFF")) - 21.0).abs() < 0.01);
    assert!((contrast(hex("#777777"), hex("#FFFFFF")) - 4.48).abs() < 0.01);
    assert!((contrast(hex("#595959"), hex("#000000")) - 3.0).abs() < 0.01);
}

#[test]
fn text_meets_aaa_on_every_surface() {
    let p = palette();
    for (surface, ground) in [
        ("panel", p.panel),
        ("field", p.field),
        ("bar", p.bar),
        ("raised", p.raised),
        ("hovered", hot(&p)),
    ] {
        for (text, ink) in [("ink", p.ink), ("muted", p.muted), ("muted_dim", p.muted_dim)] {
            assert_ratio(&format!("{text} on {surface}"), ink, ground, TEXT);
        }
    }
    // coloured text on the panels: links, states, warnings
    for (name, c) in [
        ("teal", p.teal),
        ("violet_light", p.violet_light),
        ("danger", p.danger),
        ("success", p.success),
        ("warn", p.warn),
        ("amber", p.amber),
        ("rose", p.rose),
    ] {
        assert_ratio(&format!("{name} on panel"), c, p.panel, TEXT);
    }
    // text on the accent's and the states' faces (primary, danger, caution
    // buttons: `on_accent`, then `navy`)
    for (face, c) in [("violet", p.violet), ("violet_light", p.violet_light), ("indigo", p.indigo)] {
        assert_ratio(&format!("on_accent on {face}"), p.on_accent, c, TEXT);
    }
    for (face, c) in [("danger", p.danger), ("amber", p.amber)] {
        assert_ratio(&format!("navy on {face}"), p.navy, c, TEXT);
    }
}

#[test]
fn borders_and_accent_faces_stand_out() {
    let p = palette();
    // the border of buttons, fields, windows and dividers in this mode
    assert_ratio("border (muted_dim) on panel", p.muted_dim, p.panel, UI);
    assert_ratio("border (muted_dim) on raised", p.muted_dim, p.raised, UI);
    assert_ratio("accent face on panel", p.violet, p.panel, UI);
    assert_ratio("selection (violet) on panel", p.violet, p.panel, UI);
    assert_eq!(p.panel_alpha, 255, "no glass: the world never shows through");
    assert!(p.high_contrast);
}

#[test]
fn states_differ_by_lightness() {
    let p = palette();
    // danger darkest, then the warning, then success: apart without hue
    assert!(luminance(p.danger) < luminance(p.warn) && luminance(p.warn) < luminance(p.success));
    assert_ratio("warn against danger", p.warn, p.danger, 1.3);
    assert_ratio("success against warn", p.success, p.warn, 1.3);
}

#[test]
fn colour_blindness_keeps_the_text_contrast() {
    for deficiency in Deficiency::ALL {
        let p = crate::colorblind::palette(palette(), Vision::new(deficiency, 1.0));
        for (text, ink) in [("ink", p.ink), ("muted", p.muted), ("muted_dim", p.muted_dim)] {
            assert_ratio(&format!("{deficiency:?}: {text} on panel"), ink, p.panel, TEXT);
        }
        assert_ratio(&format!("{deficiency:?}: on_accent on violet"), p.on_accent, p.violet, TEXT);
        assert!(p.high_contrast && p.panel_alpha == 255);
    }
}

#[test]
fn off_by_default() {
    assert!(!crate::settings::Settings::default().high_contrast);
}

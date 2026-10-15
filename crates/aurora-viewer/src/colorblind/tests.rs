use super::*;
use crate::theme::Theme;

fn full(d: Deficiency) -> Vision {
    Vision::new(d, 1.0)
}

/// CIE L*a*b* of a colour (D65), for the distances between colours.
fn lab(c: Color32) -> [f32; 3] {
    let [r, g, b] = linear(c);
    let x = 0.4124 * r + 0.3576 * g + 0.1805 * b;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = 0.0193 * r + 0.1192 * g + 0.9505 * b;
    let f = |t: f32| if t > 0.008_856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x / 0.950_47), f(y), f(z / 1.088_83));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// How far apart two colours look to someone with the deficiency (ΔE 76).
fn seen_apart(a: Color32, b: Color32, d: Deficiency) -> f32 {
    let (a, b) = (lab(simulate(a, d)), lab(simulate(b, d)));
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(linear(a)), luminance(linear(b)));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The pairs that say different things by their colour: the status dots
/// (online, away, do not disturb, offline) and success / warning / error.
fn meaningful(p: &Palette) -> [(&'static str, Color32, Color32); 8] {
    [
        ("success/amber", p.success, p.amber),
        ("success/danger", p.success, p.danger),
        ("amber/danger", p.amber, p.danger),
        ("danger/warn", p.danger, p.warn),
        ("success/warn", p.success, p.warn),
        ("success/muted_dim", p.success, p.muted_dim),
        ("amber/muted_dim", p.amber, p.muted_dim),
        ("danger/muted_dim", p.danger, p.muted_dim),
    ]
}

#[test]
fn off_or_no_strength_changes_nothing() {
    let p = Theme::default().palette_for(Vision::OFF);
    for v in [
        Vision::OFF,
        Vision::new(Deficiency::Deutan, 0.0),
        Vision::new(Deficiency::Mono, 0.0),
    ] {
        let q = palette(p, v);
        assert_eq!(format!("{q:?}"), format!("{p:?}"), "{v:?}");
        assert_eq!(adapt(p.success, v), p.success);
    }
}

#[test]
fn the_simulation_follows_machado() {
    // pure red for a protan: Machado's first column, back in sRGB
    let red = Color32::from_rgb(255, 0, 0);
    assert_eq!(simulate(red, Deficiency::Protan), Color32::from_rgb(109, 95, 0));
    assert_eq!(simulate(red, Deficiency::Deutan), Color32::from_rgb(163, 144, 0));
    assert_eq!(simulate(red, Deficiency::Tritan), Color32::from_rgb(255, 0, 15));
    // a grey stays grey for everyone
    let grey = Color32::from_rgb(128, 128, 128);
    for d in Deficiency::ALL {
        let s = simulate(grey, d);
        assert!(
            s.r().abs_diff(128) <= 1 && s.g().abs_diff(128) <= 1 && s.b().abs_diff(128) <= 1,
            "{d:?} {s:?}"
        );
    }
}

#[test]
fn monochrome_keeps_the_lightness() {
    for c in [
        Color32::from_rgb(0x4A, 0xDE, 0x80),
        Color32::from_rgb(0xF8, 0x71, 0x71),
        Color32::from_rgb(0x8B, 0x5C, 0xF6),
    ] {
        let grey = adapt(c, full(Deficiency::Mono));
        assert!(grey.r() == grey.g() && grey.g() == grey.b(), "{grey:?}");
        let (before, after) = (luminance(linear(c)), luminance(linear(grey)));
        assert!((before - after).abs() < 0.005, "{c:?}: {before} -> {after}");
    }
}

#[test]
fn the_meaningful_colours_part_for_every_deficiency() {
    let base = Theme::default().palette_for(Vision::OFF);
    // the problem of #418: online green and do-not-disturb red look alike
    // to a deuteranope
    assert!(seen_apart(base.success, base.danger, Deficiency::Deutan) < 12.0);
    for d in [Deficiency::Protan, Deficiency::Deutan, Deficiency::Tritan, Deficiency::Mono] {
        let p = palette(base, full(d));
        let least = if d == Deficiency::Mono { 10.0 } else { 20.0 };
        for (pair, a, b) in meaningful(&p) {
            let apart = seen_apart(a, b, d);
            assert!(apart >= least, "{d:?} {pair}: ΔE {apart:.1}");
        }
    }
}

#[test]
fn the_text_keeps_its_contrast() {
    let base = Theme::default().palette_for(Vision::OFF);
    for d in Deficiency::ALL {
        for strength in [0.5, 1.0] {
            let p = palette(base, Vision::new(d, strength));
            for surface in [p.panel, p.bar, p.field] {
                assert!(contrast(p.ink, surface) >= 4.5, "{d:?} ink");
                assert!(contrast(p.muted, surface) >= 4.5, "{d:?} muted");
                // the meaningful colours are drawn as text and dots
                for (name, c) in [("success", p.success), ("amber", p.amber), ("danger", p.danger), ("warn", p.warn)] {
                    assert!(contrast(c, surface) >= 4.5, "{d:?} {name}: {:.2}", contrast(c, surface));
                }
            }
            assert!(contrast(p.on_accent, p.violet) >= 3.0, "{d:?} on_accent");
        }
    }
}

#[test]
fn the_user_colours_are_daltonised() {
    // a red and a green marker apart again for a deuteranope
    let (red, green) = (Color32::from_rgb(220, 60, 60), Color32::from_rgb(60, 180, 60));
    let d = Deficiency::Deutan;
    let before = seen_apart(red, green, d);
    let after = seen_apart(adapt(red, full(d)), adapt(green, full(d)), d);
    assert!(after > before + 10.0, "{before:.1} -> {after:.1}");
    // half the strength, half the way
    let half = adapt(red, Vision::new(d, 0.5));
    assert!(half != red && half != adapt(red, full(d)));
    // the alpha is kept
    let faint = Color32::from_rgba_unmultiplied(220, 60, 60, 90);
    assert_eq!(adapt(faint, full(d)).a(), 90);
}

#[test]
fn the_choice_is_stored_and_read_back() {
    for d in Deficiency::ALL {
        for strength in [0.0, 0.25, 1.0] {
            let v = Vision::new(d, strength);
            assert_eq!(decode(encode(v)), v);
        }
    }
    // out of range strengths are clamped, not lost
    assert_eq!(decode(encode(Vision::new(Deficiency::Tritan, 7.0))).strength, 1.0);
    assert_eq!(Vision::new(Deficiency::Mono, f32::NAN).strength, 1.0);
    for (s, d) in [
        ("protan", Deficiency::Protan),
        ("DEUTAN", Deficiency::Deutan),
        (" mono ", Deficiency::Mono),
    ] {
        assert_eq!(Deficiency::parse(s), Some(d));
    }
    assert_eq!(Deficiency::parse("purple"), None);
    let json = serde_json::to_string(&Deficiency::Tritan).unwrap();
    assert_eq!(serde_json::from_str::<Deficiency>(&json).unwrap(), Deficiency::Tritan);
}

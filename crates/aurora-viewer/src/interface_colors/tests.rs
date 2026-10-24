use super::*;
use crate::theme::{Theme, hex};

fn skin() -> Palette {
    Theme::default().skin_palette()
}

fn rgba(c: &str) -> Option<Rgba> {
    let c = hex(c);
    Some([c.r(), c.g(), c.b(), 255])
}

/// Every colour of a palette, by name, for the comparisons.
fn colors(p: &Palette) -> Vec<(&'static str, Color32)> {
    vec![
        ("violet", p.violet),
        ("indigo", p.indigo),
        ("teal", p.teal),
        ("navy", p.navy),
        ("panel", p.panel),
        ("bar", p.bar),
        ("field", p.field),
        ("raised", p.raised),
        ("violet_light", p.violet_light),
        ("violet_pale", p.violet_pale),
        ("ink", p.ink),
        ("muted", p.muted),
        ("muted_dim", p.muted_dim),
        ("amber", p.amber),
        ("rose", p.rose),
        ("danger", p.danger),
        ("success", p.success),
        ("warn", p.warn),
        ("danger_dim", p.danger_dim),
        ("indigo_light", p.indigo_light),
        ("on_accent", p.on_accent),
    ]
}

fn assert_same(a: &Palette, b: &Palette) {
    for ((name, x), (_, y)) in colors(a).into_iter().zip(colors(b)) {
        assert_eq!(x, y, "{name}");
    }
    assert_eq!(a.panel_alpha, b.panel_alpha);
}

/// A grid of colours over the sRGB cube: hues, and dark to light.
fn sweep() -> Vec<Color32> {
    let mut out = Vec::new();
    for r in (0..=255).step_by(51) {
        for g in (0..=255).step_by(51) {
            for b in (0..=255).step_by(51) {
                out.push(Color32::from_rgb(r as u8, g as u8, b as u8));
            }
        }
    }
    out
}

fn some(c: Color32) -> Option<Rgba> {
    Some([c.r(), c.g(), c.b(), 255])
}

#[test]
fn nothing_chosen_is_the_skin() {
    let (p, adjusted) = derive(skin(), &InterfaceColors::default());
    assert_same(&p, &skin());
    assert_eq!(adjusted, Adjusted::default());
}

/// The skin's own bases, chosen, give back today's exact palette: every
/// derived colour comes from the skin's relations.
#[test]
fn the_skin_bases_give_back_the_skin() {
    let s = skin();
    let all = InterfaceColors {
        accent: some(s.violet),
        surface: some(s.panel),
        text: some(s.ink),
    };
    let (p, adjusted) = derive(s, &all);
    assert_same(&p, &s);
    assert_eq!(adjusted, Adjusted::default());
    for one in [
        InterfaceColors {
            accent: some(s.violet),
            ..Default::default()
        },
        InterfaceColors {
            surface: some(s.panel),
            ..Default::default()
        },
        InterfaceColors {
            text: some(s.ink),
            ..Default::default()
        },
    ] {
        assert_same(&derive(s, &one).0, &s);
    }
}

#[test]
fn the_colour_space_round_trips() {
    for c in sweep() {
        assert_eq!(rgb(lab(c)), c, "{c:?}");
    }
}

#[test]
fn an_accent_moves_its_family_only() {
    let s = skin();
    let (p, _) = derive(
        s,
        &InterfaceColors {
            accent: rgba("#14B8A6"),
            ..Default::default()
        },
    );
    assert_eq!(p.violet, hex("#14B8A6"));
    for (name, c, was) in [
        ("indigo", p.indigo, s.indigo),
        ("violet_light", p.violet_light, s.violet_light),
        ("violet_pale", p.violet_pale, s.violet_pale),
        ("indigo_light", p.indigo_light, s.indigo_light),
    ] {
        assert_ne!(c, was, "{name} follows the accent");
        let ([_, _, h], [_, _, ha]) = (lch(lab(c)), lch(lab(p.violet)));
        let turn = (h - ha).sin().abs();
        assert!(turn < 0.6, "{name} stays near the accent's hue");
    }
    // the light ones stay lighter, the deep one deeper
    assert!(luminance(p.violet_light) > luminance(p.violet));
    assert!(luminance(p.violet_pale) > luminance(p.violet_light));
    assert!(luminance(p.indigo) < luminance(p.violet));
    for (name, c) in colors(&p) {
        if !["violet", "indigo", "violet_light", "violet_pale", "indigo_light", "on_accent"].contains(&name) {
            assert_eq!(
                c,
                colors(&s).into_iter().find(|(n, _)| *n == name).map(|(_, c)| c).unwrap_or_default(),
                "{name}"
            );
        }
    }
}

/// A pink theme: the surfaces take the panel's hue, the muted texts the
/// text's, the states stay.
#[test]
fn a_tinted_theme_tints_its_surfaces_and_texts() {
    let s = skin();
    let (p, adjusted) = derive(
        s,
        &InterfaceColors {
            accent: rgba("#EC4899"),
            surface: rgba("#3A2433"),
            text: rgba("#FCE7F3"),
        },
    );
    assert_eq!(adjusted, Adjusted::default());
    let hue = |c: Color32| lch(lab(c))[2];
    for (name, c) in [("bar", p.bar), ("field", p.field), ("raised", p.raised), ("navy", p.navy)] {
        assert!((hue(c) - hue(p.panel)).sin().abs() < 0.35, "{name} {c:?} in the panel's hue");
    }
    for (name, c) in [("muted", p.muted), ("muted_dim", p.muted_dim)] {
        assert!(lch(lab(c))[1] > 0.02, "{name} {c:?} keeps a tint");
    }
    // the same order of lightness as the skin's
    assert!(luminance(p.field) < luminance(p.bar) && luminance(p.bar) < luminance(p.panel));
    assert!(luminance(p.raised) > luminance(p.panel));
    assert!(luminance(p.muted_dim) < luminance(p.muted) && luminance(p.muted) < luminance(p.ink));
    assert_eq!((p.danger, p.success, p.teal), (s.danger, s.success, s.teal));
}

/// On a light background the button faces go toward the (dark) text, the
/// muted texts toward the background: nothing white on white.
#[test]
fn a_light_background_keeps_its_faces_and_texts_apart() {
    let (p, _) = derive(
        skin(),
        &InterfaceColors {
            surface: rgba("#F4F4F8"),
            text: rgba("#1B1E26"),
            ..Default::default()
        },
    );
    assert!(luminance(p.raised) < luminance(p.panel), "{:?}", p.raised);
    assert!(contrast(p.raised, p.panel) > 1.2);
    assert!(luminance(p.muted) > luminance(p.ink) && luminance(p.muted) < luminance(p.panel));
}

/// Whatever the background and the text, the text and the muted text read
/// 4.5:1 on the panel (WCAG AA), the dim text 3:1, and the page is told
/// when the text had to move.
#[test]
fn texts_stay_readable_on_any_background() {
    let s = skin();
    for panel in sweep() {
        for text in [panel, s.ink, Color32::from_rgb(128, 128, 128), Color32::from_rgb(40, 40, 40)] {
            let chosen = InterfaceColors {
                surface: some(panel),
                text: some(text),
                ..Default::default()
            };
            let (p, adjusted) = derive(s, &chosen);
            assert_eq!(p.panel, panel);
            assert!(contrast(p.ink, panel) >= TEXT, "ink {:?} on {panel:?}", p.ink);
            assert!(contrast(p.muted, panel) >= TEXT, "muted {:?} on {panel:?}", p.muted);
            assert!(contrast(p.muted_dim, panel) >= UI, "muted_dim {:?} on {panel:?}", p.muted_dim);
            assert_eq!(adjusted.text, contrast(text, panel) < TEXT, "{text:?} on {panel:?}");
        }
    }
}

/// Whatever the accent, it shows against the panel (3:1) and the text on
/// it reads at least as well as the skin's white on its violet, 4.5:1 as
/// soon as the skin's pair reaches it.
#[test]
fn any_accent_keeps_its_text_and_shows_on_the_panel() {
    let s = skin();
    let need = TEXT.min(contrast(s.on_accent, s.violet));
    for accent in sweep() {
        let (p, adjusted) = derive(
            s,
            &InterfaceColors {
                accent: some(accent),
                ..Default::default()
            },
        );
        assert!(contrast(p.violet, p.panel) >= UI, "{accent:?} → {:?}", p.violet);
        assert!(contrast(p.on_accent, p.violet) >= need, "on {:?}: {:?}", p.violet, p.on_accent);
        assert!([s.on_accent, s.navy].contains(&p.on_accent));
        assert_eq!(adjusted.accent, contrast(accent, s.panel) < UI, "{accent:?}");
    }
    // the skin's violet keeps its white text, a bright yellow takes navy
    assert_eq!(
        derive(
            s,
            &InterfaceColors {
                accent: some(s.violet),
                ..Default::default()
            }
        )
        .0
        .on_accent,
        s.on_accent
    );
    assert_eq!(
        derive(
            s,
            &InterfaceColors {
                accent: rgba("#FACC15"),
                ..Default::default()
            }
        )
        .0
        .on_accent,
        s.navy
    );
}

#[test]
fn the_demo_variable_reads_the_three_colours() {
    let c = parse("accent:#14B8A6, surface:#3A2433,text:FCE7F3,other:#000000");
    assert_eq!(c.accent, rgba("#14B8A6"));
    assert_eq!(c.surface, rgba("#3A2433"));
    assert_eq!(c.text, rgba("#FCE7F3"));
    assert_eq!(parse("accent:#14B8A6").surface, None);
}

/// On any chosen background, light or dark, the states and the links read
/// 4.5:1 (they colour text too) and keep their hue; on the skin's they are
/// the skin's (`the_skin_bases_give_back_the_skin`).
#[test]
fn states_and_links_read_on_any_background() {
    let s = skin();
    for panel in sweep() {
        let (p, _) = derive(
            s,
            &InterfaceColors {
                surface: some(panel),
                ..Default::default()
            },
        );
        for (name, c, was) in [
            ("danger", p.danger, s.danger),
            ("success", p.success, s.success),
            ("warn", p.warn, s.warn),
            ("amber", p.amber, s.amber),
            ("rose", p.rose, s.rose),
            ("teal", p.teal, s.teal),
        ] {
            assert!(contrast(c, panel) >= TEXT, "{name} {c:?} on {panel:?}");
            let ([_, cc, h], [_, _, hw]) = (lch(lab(c)), lch(lab(was)));
            if cc > 0.03 {
                assert!((h - hw).sin().abs() < 0.15, "{name} {c:?} keeps the hue of {was:?}");
            }
        }
    }
    // a pale pink background darkens them, a near-black one keeps them
    let (light, _) = derive(
        s,
        &InterfaceColors {
            surface: rgba("#FCE7F3"),
            ..Default::default()
        },
    );
    assert!(luminance(light.danger) < luminance(s.danger) && luminance(light.teal) < luminance(s.teal));
    let (dark, _) = derive(
        s,
        &InterfaceColors {
            surface: rgba("#101014"),
            ..Default::default()
        },
    );
    assert_eq!((dark.danger, dark.success, dark.teal), (s.danger, s.success, s.teal));
}

/// A chat colour moved onto a light background (`on_panel`'s rule) keeps
/// its hue and reads.
#[test]
fn chat_colours_move_onto_a_light_background() {
    let ground = hex("#FCE7F3");
    for c in ["#5EEAD4", "#E8EDFB", "#9AA6C6", "#4ADE80", "#FCD34D"] {
        let moved = readable(hex(c), ground, TEXT);
        assert!(contrast(moved, ground) >= TEXT, "{c} → {moved:?}");
    }
    assert_eq!(readable(hex("#5EEAD4"), skin().panel, TEXT), hex("#5EEAD4"));
}

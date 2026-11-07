use super::*;

const PICKED: Rgba = [1, 2, 3, 255];

fn light(accent: Option<[u8; 3]>) -> Look {
    Look {
        scheme: Scheme::Light,
        accent,
    }
}

#[test]
fn a_dark_or_undecided_system_keeps_the_skins_background_and_text() {
    for scheme in [Scheme::Dark, Scheme::NoPreference] {
        let c = colors(Look { scheme, accent: None }, &InterfaceColors::default());
        assert_eq!(c, InterfaceColors::default());
    }
}

#[test]
fn a_light_system_brings_its_background_and_text() {
    let c = colors(light(None), &InterfaceColors::default());
    assert_eq!((c.surface, c.text), (Some(LIGHT_SURFACE), Some(LIGHT_TEXT)));
    assert_eq!(c.accent, None);
}

#[test]
fn the_systems_accent_is_opaque() {
    let c = colors(light(Some([10, 20, 30])), &InterfaceColors::default());
    assert_eq!(c.accent, Some([10, 20, 30, 255]));
}

#[test]
fn a_colour_the_user_chose_wins() {
    let chosen = InterfaceColors {
        accent: Some(PICKED),
        ..Default::default()
    };
    assert_eq!(colors(light(Some([10, 20, 30])), &chosen).accent, Some(PICKED));
}

/// A chosen dark background under the system's dark text would be unreadable.
#[test]
fn a_chosen_background_or_text_keeps_the_pair_off_the_system() {
    let surface = InterfaceColors {
        surface: Some(PICKED),
        ..Default::default()
    };
    assert_eq!(colors(light(None), &surface), surface);
    let text = InterfaceColors {
        text: Some(PICKED),
        ..Default::default()
    };
    assert_eq!(colors(light(None), &text), text);
}

/// The light pair goes through the same rules as a background of one's own.
#[test]
fn the_light_pair_needs_no_readability_adjustment() {
    let c = colors(light(None), &InterfaceColors::default());
    let adjusted = crate::interface_colors::adjusted(&c);
    assert!(!adjusted.text, "{adjusted:?}");
}

#[test]
fn the_portals_scheme_values() {
    assert_eq!(scheme_from_portal(0), Scheme::NoPreference);
    assert_eq!(scheme_from_portal(1), Scheme::Dark);
    assert_eq!(scheme_from_portal(2), Scheme::Light);
    assert_eq!(scheme_from_portal(3), Scheme::NoPreference);
}

#[test]
fn an_accent_out_of_range_is_no_accent() {
    assert_eq!(accent_from_unit(0.0, 0.5, 1.0), Some([0, 128, 255]));
    assert_eq!(accent_from_unit(-1.0, 0.5, 1.0), None);
    assert_eq!(accent_from_unit(0.0, 1.5, 1.0), None);
    assert_eq!(accent_from_unit(f64::NAN, 0.5, 1.0), None);
}

#[test]
fn a_demo_scenario_lists_its_looks_and_pace() {
    let demo = parse_demo("light,accent:#2563EB;dark;secs=3");
    assert_eq!(demo.every, Duration::from_secs(3));
    assert_eq!(
        demo.looks,
        vec![
            light(Some([0x25, 0x63, 0xEB])),
            Look {
                scheme: Scheme::Dark,
                accent: None
            }
        ]
    );
    assert_eq!(parse_demo("dark").every, Duration::from_secs(4));
    // a stray separator is no look of its own
    assert_eq!(parse_demo("dark;").looks, parse_demo("dark").looks);
    assert_eq!(parse_demo("dark; ;light").looks.len(), 2);
}

/// Counts, selected rows and initials are text in the lighter accents.
#[test]
fn the_lighter_accents_read_on_the_light_pair() {
    let skin = crate::theme::Theme::default().skin_palette();
    let chosen = colors(light(Some([0x25, 0x63, 0xEB])), &InterfaceColors::default());
    let (p, _) = crate::interface_colors::derive(skin, &chosen);
    for c in [p.violet_light, p.violet_pale, p.indigo_light] {
        assert!(crate::interface_colors::contrast(c, p.panel) >= 4.5, "{c:?}");
    }
}

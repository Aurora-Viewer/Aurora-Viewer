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
    let skin = crate::theme::Theme::default().skin_palette();
    let c = colors(light(None), &InterfaceColors::default());
    let (_, adjusted) = crate::interface_colors::derive(skin, &c);
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

/// A period no `Duration` holds keeps the default instead of stopping the run.
#[test]
fn a_demo_period_out_of_range_is_ignored() {
    for raw in ["dark;secs=inf", "dark;secs=1e30", "dark;secs=NaN", "dark;secs=0"] {
        assert_eq!(parse_demo(raw).every, Duration::from_secs(4), "{raw}");
    }
}

/// The light accent is text on the panels and the hovered face under
/// `on_accent`: every accent keeps the face, on the light pair the text too.
#[test]
fn the_hovered_accent_face_keeps_its_label() {
    use crate::interface_colors::{contrast, derive};
    let skin = crate::theme::Theme::default().skin_palette();
    let grey = InterfaceColors {
        accent: Some([0xFF, 0xFF, 0xFF, 255]),
        surface: Some([0x78, 0x78, 0x78, 255]),
        ..InterfaceColors::default()
    };
    let (p, _) = derive(skin, &grey);
    assert!(contrast(p.violet_light, p.on_accent) >= 3.0, "{p:?}");
    for rgb in [
        [0xFF, 0xE0, 0x00],
        [0x16, 0xA3, 0x4A],
        [0x3D, 0xAE, 0xE9],
        [0xE1, 0x1D, 0x8F],
        [0xFF, 0xFF, 0xFF],
        [0x10, 0x10, 0x40],
    ] {
        let (p, _) = derive(skin, &colors(light(Some(rgb)), &InterfaceColors::default()));
        assert!(contrast(p.violet_light, p.on_accent) >= 3.0, "{rgb:?}: {p:?}");
        assert!(contrast(p.violet_light, p.panel) >= 4.5, "{rgb:?}: {p:?}");
    }
}

#[test]
fn the_repeated_read_stops_with_the_option() {
    set_active(true);
    assert!(active());
    set_active(false);
    assert!(!active());
}

/// A desktop with less than both is said in Preferences, once it is known.
#[test]
fn what_the_system_gives() {
    assert_eq!(given_by(false, false, false), Given::Everything);
    assert_eq!(given_by(true, true, true), Given::Everything);
    assert_eq!(given_by(true, false, true), Given::Everything);
    assert_eq!(given_by(true, true, false), Given::ModeOnly);
    assert_eq!(given_by(true, false, false), Given::Nothing);
}

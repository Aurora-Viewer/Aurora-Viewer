//! Preferences › Layout: the bars and the windows (panel_preferences_UI.xml
//! and Aurora's islands), the hover tips of the world ("Hover Tips":
//! ShowHoverTips and what it enables) and what opens at startup.

use super::super::cx::{Cx, field, fields};

pub fn page(cx: &mut Cx, tab: usize) {
    use aurora_i18n::tr;
    match tab {
        0 => {
            cx.group(tr!("ui-options-bars"), |cx| {
                cx.toggle(tr!("bug-toolbar"), tr!("bug-toolbar-help"), &field!(show_bug_report));
                // the Legacy Viewer's Stand / Stop flying panel
                cx.toggle(
                    tr!("ui-options-stand-fly-panel"),
                    tr!("ui-options-stand-fly-panel-help"),
                    &field!(show_stand_fly_panel),
                );
                // the place island of the Aurora bars (also in its right-click menu)
                cx.toggle(
                    tr!("ui-options-place-bar-stretch"),
                    tr!("ui-options-place-bar-stretch-help"),
                    &field!(place_bar_stretch),
                );
                cx.toggle(
                    tr!("ui-options-place-bar-rights-text"),
                    tr!("ui-options-place-bar-rights-text-help"),
                    &field!(place_bar_rights_text),
                );
                // also in the favourites' own menus
                use crate::settings::FavoritesAccess;
                cx.choice(
                    tr!("ui-options-favorites-access"),
                    tr!("ui-options-favorites-access-help"),
                    &field!(favorites_access),
                    &[
                        (FavoritesAccess::Menu, tr!("favorites-access-menu"), false),
                        (FavoritesAccess::Row, tr!("favorites-access-row"), false),
                        (FavoritesAccess::Dock, tr!("favorites-access-dock"), false),
                    ],
                );
                // also in the system island's right click
                cx.toggle(
                    tr!("ui-options-traffic-indicator"),
                    tr!("ui-options-traffic-indicator-help"),
                    &field!(show_net_stats),
                );
                // panel_preferences_UI.xml "Show currency balance"; also the
                // balance's right click and the account island's
                cx.toggle(
                    tr!("ui-options-show-balance"),
                    tr!("ui-options-show-balance-help"),
                    &field!(show_balance),
                );
                // the L$ received and spent under it (#2675), each on its
                // own; also the balance's right click and the account island's
                cx.toggle(
                    tr!("ui-options-balance-received"),
                    tr!("ui-options-balance-received-hint"),
                    &field!(balance_shows_received),
                );
                cx.toggle(
                    tr!("ui-options-balance-spent"),
                    tr!("ui-options-balance-spent-hint"),
                    &field!(balance_shows_spent),
                );
                // also a left click on the clock, and the island's right click
                use crate::settings::ClockZone;
                cx.choice(
                    tr!("ui-options-clock-zone"),
                    tr!("ui-options-clock-zone-help"),
                    &field!(system_island.clock_zone),
                    &[
                        (ClockZone::Slt, tr!("ui-options-clock-zone-slt"), false),
                        (ClockZone::Local, tr!("ui-options-clock-zone-local"), false),
                    ],
                );
                cx.toggle(
                    tr!("ui-options-clock-seconds"),
                    tr!("ui-options-clock-seconds-help"),
                    &field!(system_island.clock_seconds),
                );
                use crate::settings::ClockFormat;
                cx.choice(
                    tr!("ui-options-clock-format"),
                    tr!("ui-options-clock-format-help"),
                    &field!(system_island.clock_format),
                    &[
                        (ClockFormat::Auto, tr!("ui-options-clock-format-auto"), false),
                        (ClockFormat::H12, tr!("ui-options-clock-format-12"), false),
                        (ClockFormat::H24, tr!("ui-options-clock-format-24"), false),
                    ],
                );
            });
            cx.group(tr!("common-windows"), |cx| {
                // the interface's zoom has its own path, each frame
                cx.wide().row_with(
                    tr!("ui-options-interface-scale"),
                    tr!("ui-options-interface-scale-help"),
                    &field!(ui_scale),
                    |ui, s, st, r| {
                        r.ui_scale_changed |= st.interface_scale.show(ui, &mut s.ui_scale);
                        false
                    },
                );
                cx.toggle(
                    tr!("ui-options-snap-windows"),
                    tr!("ui-options-snap-windows-help"),
                    &field!(snap_windows),
                );
                let p = cx.p;
                cx.row(
                    tr!("ui-options-windows-stop-at-edges"),
                    tr!("ui-options-windows-stop-at-edges-help"),
                    &fields!(windows_stop_left, windows_stop_right, windows_stop_bottom),
                    |ui, s| {
                        use crate::ui::widgets::checkbox;
                        checkbox(ui, &p, &mut s.windows_stop_left, tr!("ui-options-edge-left")).changed()
                            | checkbox(ui, &p, &mut s.windows_stop_right, tr!("ui-options-edge-right")).changed()
                            | checkbox(ui, &p, &mut s.windows_stop_bottom, tr!("ui-options-edge-bottom")).changed()
                    },
                );
                cx.combo(
                    tr!("ui-options-context-menu"),
                    tr!("ui-options-context-menu-help"),
                    &field!(use_pie_menu),
                    &[(false, tr!("ui-options-list-menu")), (true, tr!("ui-options-pie-menu"))],
                );
                cx.toggle(
                    tr!("ui-options-loading-backdrop"),
                    tr!("ui-options-loading-and-teleport-screens-show-your-last-view-blurred"),
                    &field!(loading_backdrop),
                );
                // panel_preferences_UI.xml "Use stand-alone windows for:"
                cx.toggle(
                    tr!("ui-options-landmarks-and-place-profiles"),
                    tr!("ui-options-separate-windows-for-landmarks-teleport-history-details-place-profiles"),
                    &field!(standalone_place_details),
                );
            });
        }
        1 => {
            cx.group(tr!("ui-options-hover-tips"), |cx| {
                cx.toggle(
                    tr!("ui-options-hover-tips-show"),
                    tr!("ui-options-hover-tips-show-help"),
                    &field!(hover_tips.enabled),
                );
                // the one exception to the main switch (#2475): it only acts
                // while world tooltips are off, so it is greyed while they are on
                cx.greyed(cx.s.hover_tips.enabled);
                cx.toggle(
                    tr!("ui-options-hover-tips-avatars"),
                    tr!("ui-options-hover-tips-avatars-help"),
                    &field!(hover_tips.avatars),
                );
                // form, group titles and delay serve the avatar-only tips too
                let any = crate::hover_tips::any_enabled(&cx.s.hover_tips);
                cx.greyed(!any);
                use crate::settings::HoverTipForm;
                let forms = [tr!("hover-tips-bubble"), tr!("hover-tips-card"), tr!("hover-tips-readout")];
                let choices: Vec<(HoverTipForm, &str, bool)> = HoverTipForm::ALL.iter().zip(forms).map(|(f, l)| (*f, l, true)).collect();
                cx.choice(
                    tr!("ui-options-hover-tips-form"),
                    tr!("ui-options-hover-tips-form-help"),
                    &field!(hover_tips.form),
                    &choices,
                );
                cx.greyed(!cx.s.hover_tips.enabled);
                cx.choice(
                    tr!("ui-options-hover-tips-objects"),
                    tr!("ui-options-hover-tips-objects-help"),
                    &field!(hover_tips.all_objects),
                    &[
                        (false, tr!("ui-options-hover-tips-interactive"), true),
                        (true, tr!("ui-options-hover-tips-all"), true),
                    ],
                );
                cx.toggle(
                    tr!("ui-options-hover-tips-land"),
                    tr!("ui-options-hover-tips-land-help"),
                    &field!(hover_tips.land),
                );
                cx.greyed(!any);
                cx.toggle(tr!("ui-options-hover-tips-group-titles"), "", &field!(hover_tips.group_titles));
                cx.slider(
                    tr!("ui-options-hover-tips-delay"),
                    "",
                    &field!(hover_tips.delay),
                    0.0..=crate::settings::MAX_HOVER_TIP_DELAY,
                    " s",
                );
            });
            floating_text(cx);
        }
        _ => cx.group(tr!("ui-options-at-startup"), |cx| {
            cx.toggle(tr!("ui-options-conversations-at-startup"), "", &field!(show_chat));
            cx.toggle(tr!("ui-options-performance-at-startup"), "", &field!(show_perf));
            cx.toggle(tr!("ui-options-mini-map-at-startup"), "", &field!(show_minimap));
        }),
    }
}

/// The objects' floating texts (llSetText): the Legacy Viewer's options of
/// panel_preferences_UI.xml (FSHudTextFadeDistance, FSHudTextFadeRange,
/// FSHudTextUseHoverHighlight, FSHudTextShowBackground) and
/// panel_preferences_colors.xml (FSHudTextBackgroundOpacity), with its
/// ranges. They sit by the hover tips, as in its 3D World tab.
fn floating_text(cx: &mut Cx) {
    use crate::settings::FloatingTextBackground as Bg;
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-floating-text"), |cx| {
        cx.slider(
            tr!("ui-options-floating-text-fade-distance"),
            tr!("ui-options-floating-text-fade-distance-help"),
            &field!(floating_text_fade_distance),
            0.0..=64.0,
            " m",
        );
        cx.slider(
            tr!("ui-options-floating-text-fade-range"),
            tr!("ui-options-floating-text-fade-range-help"),
            &field!(floating_text_fade_range),
            0.25..=32.0,
            " m",
        );
        cx.toggle(
            tr!("ui-options-floating-text-highlight"),
            tr!("ui-options-floating-text-highlight-help"),
            &field!(floating_text_hover_highlight),
        );
        // the hovered object's background needs the highlight (LLHUDText::renderText)
        let why = (!cx.s.floating_text_hover_highlight).then(|| tr!("ui-options-floating-text-background-hovered-why"));
        cx.combo_blocked(
            tr!("ui-options-floating-text-background"),
            tr!("ui-options-floating-text-background-help"),
            &field!(floating_text_background),
            &[
                (Bg::Off, tr!("ui-options-floating-text-background-off"), None),
                (Bg::Hovered, tr!("ui-options-floating-text-background-hovered"), why),
                (Bg::All, tr!("ui-options-floating-text-background-all"), None),
            ],
        );
        let off = cx.s.floating_text_background == Bg::Off;
        cx.subs(off, |cx| {
            cx.slider(
                tr!("ui-options-floating-text-background-opacity"),
                "",
                &field!(floating_text_background_opacity),
                0.0..=1.0,
                "",
            );
        });
    });
}

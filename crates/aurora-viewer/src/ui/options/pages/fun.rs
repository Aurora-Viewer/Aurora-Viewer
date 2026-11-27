//! Preferences › Just for fun (Aurora's own page, #1112 and #1114): little
//! things to play with, all off by default. The name plates wobble like a
//! jelly under the pointer (`ui/nameplate/fidget.rs`), and a click combo
//! counts the clicks on each plate (`ui/nameplate/combo.rs`). A sample
//! plate under them tries both as they are set.

use super::super::cx::{Cx, Nothing, field};

/// The constellations over the real sky (world/constellations.rs): one row
/// whose button opens their own window (ui/constellations_window.rs), where
/// every setting of theirs is; the window says when the night sky is not
/// the real one.
fn constellation_rows(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("ui-options-fun-sky"), |cx| {
        cx.keywords("constellations stars figures names zodiac drawings lines sky")
            .row_with(
                tr!("ui-options-constellations"),
                tr!("ui-options-constellations-help"),
                &Nothing,
                |ui, _, _, r| {
                    use crate::ui::widgets::{ActionKind, action_button};
                    if action_button(
                        ui,
                        &p,
                        "sparkles-filled",
                        tr!("ui-options-constellations-open"),
                        ActionKind::Flat,
                        egui::Vec2::new(0.0, 24.0),
                    )
                    .clicked()
                    {
                        r.open_constellations = true;
                    }
                    false
                },
            );
    });
}

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("ui-options-fun-plates"), |cx| {
        if cx.s.classic_name_tags {
            cx.note(tr!("ui-options-fun-classic-tags"));
        }
        if cx.s.reduce_motion {
            cx.note(tr!("ui-options-fun-reduced-motion"));
        }
        cx.toggle(
            tr!("ui-options-fun-jiggle"),
            tr!("ui-options-fun-jiggle-help"),
            &field!(plates_jiggle),
        );
        cx.toggle(
            tr!("ui-options-fun-click-combo"),
            tr!("ui-options-fun-click-combo-help"),
            &field!(plates_click_combo),
        );
        cx.wide()
            .row(tr!("ui-options-fun-try"), tr!("ui-options-fun-try-help"), &Nothing, |ui, s| {
                crate::ui::nameplate::playground::show(ui, &p, s.plates_jiggle, s.plates_click_combo, s.reduce_motion);
                false
            });
    });
    constellation_rows(cx);
}

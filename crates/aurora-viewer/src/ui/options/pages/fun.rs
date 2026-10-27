//! Preferences › Just for fun (Aurora's own page, #1112 and #1114): little
//! things to play with, all off by default. The name plates wobble like a
//! jelly under the pointer (`ui/nameplate/fidget.rs`), and a click combo
//! counts the clicks on each plate (`ui/nameplate/combo.rs`). A sample
//! plate under them tries both as they are set.

use super::super::cx::{Cx, Nothing, field};

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
}

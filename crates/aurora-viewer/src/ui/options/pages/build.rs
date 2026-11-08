//! Preferences › Build: what a prim made with the Create tool starts as,
//! after the Legacy Viewer's Build preferences (panel_preferences_firestorm.xml,
//! the "Build 1" tab: size, phantom, physical, temporary, material, color,
//! alpha, glow, full bright, shininess). The look and flags are given when
//! the prim arrives (`build::new_defaults`).

use super::super::cx::{Cx, field};

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("ui-options-build-new-objects"), |cx| {
        cx.note(tr!("ui-options-build-new-objects-note"));
        // FSBuildPrefs_X/Y/Zsize: 0.01 to 10 m, as the Legacy spinners
        cx.keywords("FSBuildPrefs_Xsize FSBuildPrefs_Ysize FSBuildPrefs_Zsize").row(
            tr!("common-size"),
            tr!("ui-options-build-size-hint"),
            &field!(build.new_prim_size),
            |ui, s| {
                let mut changed = false;
                for v in &mut s.build.new_prim_size {
                    changed |= ui
                        .add(
                            egui::DragValue::new(v)
                                .range(0.01..=10.0)
                                .speed(0.05)
                                .max_decimals(3)
                                .custom_parser(aurora_i18n::parse_number),
                        )
                        .changed();
                }
                changed
            },
        );
        let materials = [
            (0u8, tr!("build-ui-features-stone")),
            (1, tr!("build-ui-features-metal")),
            (2, tr!("build-ui-features-glass")),
            (3, tr!("build-ui-features-wood")),
            (4, tr!("ui-options-build-flesh")),
            (5, tr!("build-ui-features-plastic")),
            (6, tr!("build-ui-features-rubber")),
        ];
        cx.keywords("FSBuildPrefs_Material")
            .combo(tr!("ui-options-build-material"), "", &field!(build.new_prim_material), &materials);
        cx.keywords("FSBuildPrefs_Physical")
            .toggle(tr!("ui-options-build-physical"), "", &field!(build.new_prim_physical));
        cx.keywords("FSBuildPrefs_Temporary")
            .toggle(tr!("ui-options-build-temporary"), "", &field!(build.new_prim_temporary));
        cx.keywords("FSBuildPrefs_Phantom")
            .toggle(tr!("build-ui-object-phantom"), "", &field!(build.new_prim_phantom));
    });
    cx.group(tr!("ui-options-build-new-faces"), |cx| {
        cx.keywords("FSBuildPrefs_Color")
            .color(tr!("common-color"), "", &field!(build.new_prim_color));
        cx.keywords("FSBuildPrefs_Alpha").slider_whole(
            tr!("build-ui-texture-transparency"),
            "",
            &field!(build.new_prim_transparency),
            0.0..=100.0,
            " %",
        );
        cx.keywords("FSBuildPrefs_Glow")
            .slider(tr!("build-ui-texture-glow"), "", &field!(build.new_prim_glow), 0.0..=1.0, "");
        cx.keywords("FSBuildPrefs_FullBright")
            .toggle(tr!("build-ui-texture-fullbright"), "", &field!(build.new_prim_fullbright));
        let shines = [
            (0u8, tr!("common-none-2")),
            (1, tr!("build-ui-texture-low")),
            (2, tr!("common-medium")),
            (3, tr!("build-ui-texture-high")),
        ];
        cx.keywords("FSBuildPrefs_Shiny")
            .combo(tr!("build-ui-texture-shininess"), "", &field!(build.new_prim_shiny), &shines);
    });
}

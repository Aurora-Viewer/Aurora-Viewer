//! Headset controls and per-eye resolution preferences.
use super::super::cx::{Cx, field, fields};
use aurora_i18n::tr;

pub fn page(cx: &mut Cx) {
    let p = cx.p;
    cx.group(tr!("vr-preferences-title"), |cx| {
        cx.row(
            tr!("vr-mouse-look"),
            tr!("vr-discomfort"),
            &field!(camera.vr_mouse_look),
            |ui, s| {
                let changed = crate::ui::widgets::switch(ui, &p, &mut s.camera.vr_mouse_look).changed();
                crate::ui::widgets::glyph(ui, "alert-triangle-filled", p.amber, 13.0);
                ui.interact(ui.min_rect(), ui.scope_id().with("discomfort"), egui::Sense::hover())
                    .on_hover_text(tr!("vr-discomfort"));
                changed
            },
        );
        cx.slider(tr!("vr-gamma"), tr!("vr-gamma-help"), &field!(camera.vr_gamma), 0.5..=2.5, "");
        cx.slider(
            tr!("vr-vibrancy"),
            tr!("vr-vibrancy-help"),
            &field!(camera.vr_vibrancy),
            0.0..=2.0,
            "",
        );
        cx.row(
            tr!("vr-resolution"),
            tr!("vr-resolution-help"),
            &fields!(camera.vr_resolution, camera.vr_resolution_custom),
            |ui, s| {
                let mut changed = false;
                let mut preset = if s.camera.vr_resolution_custom {
                    0
                } else {
                    match s.camera.vr_resolution {
                        [1600, 1600] => 1600,
                        [2160, 2160] => 2160,
                        [2560, 2560] => 2560,
                        _ => 0,
                    }
                };
                egui::ComboBox::from_id_salt(ui.scope_id().with("vr-resolution"))
                    .selected_text(if preset == 0 {
                        tr!("vr-custom").to_owned()
                    } else {
                        format!("{preset} × {preset}")
                    })
                    .show_ui(ui, |ui| {
                        for n in [1600, 2160, 2560, 0] {
                            let label = if n == 0 {
                                tr!("vr-custom").to_owned()
                            } else {
                                format!("{n} × {n}")
                            };
                            if ui.selectable_value(&mut preset, n, label).changed() {
                                s.camera.vr_resolution_custom = n == 0;
                                if n != 0 {
                                    s.camera.vr_resolution = [n, n];
                                }
                                changed = true;
                            }
                        }
                    });
                if preset == 0 {
                    for dimension in &mut s.camera.vr_resolution {
                        changed |= ui.add(egui::DragValue::new(dimension).range(256..=8192)).changed();
                    }
                }
                changed
            },
        );
    });
}

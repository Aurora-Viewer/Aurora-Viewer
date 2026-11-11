//! Headset controls and per-eye resolution preferences.
use super::super::cx::{Cx, field, fields};
use aurora_i18n::tr;

pub fn page(cx: &mut Cx) {
    let p = cx.p;
    cx.group(tr!("vr-preferences-title"), |cx| {
        cx.row(
            tr!("vr-mouse-look"),
            tr!("vr-discomfort"),
            &field!(camera.vr.mouse_look),
            |ui, s| {
                let changed = crate::ui::widgets::switch(ui, &p, &mut s.camera.vr.mouse_look).changed();
                crate::ui::widgets::glyph(ui, "alert-triangle-filled", p.amber, 13.0);
                ui.interact(ui.min_rect(), ui.scope_id().with("discomfort"), egui::Sense::hover())
                    .on_hover_text(tr!("vr-discomfort"));
                changed
            },
        );
        cx.slider(tr!("vr-gamma"), tr!("vr-gamma-help"), &field!(camera.vr.gamma), 0.5..=2.5, "");
        cx.slider(
            tr!("vr-vibrancy"),
            tr!("vr-vibrancy-help"),
            &field!(camera.vr.vibrancy),
            0.0..=2.0,
            "",
        );
        cx.row(
            tr!("vr-resolution"),
            tr!("vr-resolution-help"),
            &fields!(camera.vr.resolution, camera.vr.resolution_custom),
            |ui, s| {
                let mut changed = false;
                let mut preset = match (s.camera.vr.resolution, s.camera.vr.resolution_custom) {
                    (None, _) => None,
                    (Some([n, m]), false) if n == m && [1600, 2160, 2560].contains(&n) => Some(n),
                    _ => Some(0),
                };
                let label = |choice| match choice {
                    None => tr!("vr-resolution-recommended").to_owned(),
                    Some(0) => tr!("vr-custom").to_owned(),
                    Some(n) => format!("{n} × {n}"),
                };
                egui::ComboBox::from_id_salt(ui.scope_id().with("vr-resolution"))
                    .selected_text(label(preset))
                    .show_ui(ui, |ui| {
                        for choice in [None, Some(1600), Some(2160), Some(2560), Some(0)] {
                            if ui.selectable_value(&mut preset, choice, label(choice)).changed() {
                                s.camera.vr.resolution_custom = choice == Some(0);
                                s.camera.vr.resolution = choice.map(|n| {
                                    if n == 0 {
                                        s.camera.vr.resolution.unwrap_or([1600, 1600])
                                    } else {
                                        [n, n]
                                    }
                                });
                                changed = true;
                            }
                        }
                    });
                if preset == Some(0)
                    && let Some(size) = &mut s.camera.vr.resolution
                {
                    for dimension in size {
                        changed |= ui.add(egui::DragValue::new(dimension).range(256..=8192)).changed();
                    }
                }
                changed
            },
        );
    });
    cx.track(&field!(camera.vr));
}

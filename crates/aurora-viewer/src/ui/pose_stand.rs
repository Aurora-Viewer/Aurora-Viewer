//! The pose stand's window (toolbar `pose_stand`): the list of poses and
//! the ground lock, as floater_fs_posestand.xml (a combo box, then the
//! FSPoseStandLock check box with its lock icon). The rules are in
//! `pose_stand.rs`, their effects in `app/pose_stand.rs`.

use super::widgets::{self, Floater};
use crate::pose_stand;
use crate::theme::Palette;
use egui::Vec2;
use uuid::Uuid;

/// The window's id (Floater.Toggle "fs_posestand").
pub const WINDOW: &str = "pose_stand";

/// What the window asks of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// A pose chosen in the list (onCommitCombo).
    Pose(Uuid),
    /// The lock switched (handleSetPoseStandLock).
    Lock(bool),
}

/// Draw the window while `open`; `current` is the pose playing.
pub fn show(ctx: &egui::Context, p: &Palette, open: &mut bool, current: Option<Uuid>, lock: bool) -> Option<Action> {
    if !*open {
        return None;
    }
    let mut action = None;
    let mut floater = Floater::new(
        WINDOW,
        aurora_i18n::tr!("toolbar-command-pose-stand"),
        egui::pos2(80.0, 140.0),
        Vec2::new(330.0, 72.0),
    )
    .fixed();
    floater.min_size = Vec2::new(300.0, 72.0);
    floater.show(ctx, p, open, |ui| {
        ui.horizontal(|ui| {
            let poses = pose_stand::sorted();
            let shown = current.and_then(|id| poses.iter().find(|p| p.id == id)).map_or("", |p| p.label());
            egui::ComboBox::from_id_salt("pose_stand_pose")
                .selected_text(shown)
                // the Legacy combo's width; following the window's would
                // let a fixed window grow with its content
                .width(240.0)
                .show_ui(ui, |ui| {
                    for pose in &poses {
                        if ui.selectable_label(current == Some(pose.id), pose.label()).clicked() {
                            action = Some(Action::Pose(pose.id));
                        }
                    }
                });
            let tip = aurora_i18n::tr!("pose-stand-lock");
            let mut on = lock;
            let check = widgets::checkbox(ui, p, &mut on, "");
            // no visible label (the lock icon stands for it): screen readers get the text
            check.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, tip));
            if check.on_hover_text(tip).changed() {
                action = Some(Action::Lock(on));
            }
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::hover());
            resp.on_hover_text(tip);
            if let Some(t) = super::icons::global("lock-filled") {
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(t.id(), rect, uv, if lock { p.ink } else { p.muted });
            }
        });
    });
    action
}

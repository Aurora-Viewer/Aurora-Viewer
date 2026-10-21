//! The progress window of a script action on a selection (floater_script_queue.xml,
//! LLFloaterScriptQueue): one line per step, "Close" once it is done. The
//! queue itself is `build::script_queue`.

use super::widgets;
use crate::build::script_queue::ScriptQueue;
use crate::theme::Palette;
use egui::{RichText, Vec2};

/// Draw the window; false once it is closed (the queue stops there, like
/// the coroutine of a closed floater).
pub fn show(ctx: &egui::Context, p: &Palette, queue: &ScriptQueue) -> bool {
    let mut open = true;
    let mut close = false;
    widgets::Floater::new(
        "script_queue",
        queue.kind.title(),
        egui::pos2(420.0, 120.0),
        Vec2::new(300.0, 400.0),
    )
    .show(ctx, p, &mut open, |ui| {
        let (mut list, foot) = widgets::above_footer(ui);
        egui::ScrollArea::vertical()
            .max_height(list.available_height())
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(&mut list, |ui| {
                for line in &queue.lines {
                    ui.add(egui::Label::new(RichText::new(line).size(12.5).color(p.ink)).wrap());
                }
            });
        foot.show(ui, |ui| {
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let size = Vec2::new(0.0, 26.0);
                let label = aurora_i18n::tr!("common-close");
                if widgets::action_button_if(ui, p, "x", label, widgets::ActionKind::Flat, size, queue.done).clicked() {
                    close = true;
                }
            });
        });
    });
    open && !close
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::script_queue::QueueKind;

    #[test]
    fn the_window_keeps_its_size() {
        crate::ui::widgets::steady_tests::assert_window_steady(
            "script_queue",
            &[Vec2::new(300.0, 400.0), Vec2::new(240.0, 220.0)],
            aurora_i18n::tr!("common-close"),
            || {
                let objects = (0..30).map(|i| (uuid::Uuid::from_u128(i), format!("Box {i}"))).collect();
                let mut queue = ScriptQueue::new(QueueKind::Reset, objects);
                queue.lines.extend((0..30).map(|i| format!("Loading inventory for: Box {i}")));
                queue
            },
            |queue, ctx, p| {
                show(ctx, p, queue);
            },
        );
    }
}

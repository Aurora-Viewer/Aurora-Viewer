//! Advanced › Region console: the simulator console of estate managers.
//!
//! Port of LLFloaterRegionDebugConsole (indra/newview/
//! llfloaterregiondebugconsole.cpp, floater_region_debug_console.xml,
//! originally LGPL 2.1): the output above, one command line below with its
//! history on the arrow keys. The text lives in `world::sim_console`.

use crate::theme::Palette;
use crate::ui::widgets::Floater;
use crate::world::sim_console::{Line, SimConsole};
use egui::{RichText, Vec2};

pub const WINDOW: &str = "sim_console";

/// The command being typed and where the arrow keys are in the history.
#[derive(Default)]
pub struct ConsoleInput {
    text: String,
    /// Index in the history while browsing it; None on a new line.
    browsing: Option<usize>,
}

impl ConsoleInput {
    /// Up goes back in the history, Down forward, past the end to an empty
    /// line (LLLineEditor's line history).
    fn browse(&mut self, history: &[String], up: bool) {
        if history.is_empty() {
            return;
        }
        let next = match (self.browsing, up) {
            (None, true) => Some(history.len() - 1),
            (None, false) => None,
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) => (i + 1 < history.len()).then_some(i + 1),
        };
        self.browsing = next;
        self.text = next.map(|i| history[i].clone()).unwrap_or_default();
    }
}

/// `region` names the window; `available` is false where the region has no
/// SimConsoleAsync capability (CONSOLE_NOT_SUPPORTED).
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    open: &mut bool,
    region: &str,
    available: bool,
    console: &mut SimConsole,
    input: &mut ConsoleInput,
) {
    if !*open {
        return;
    }
    let title = aurora_i18n::tr!("sim-console-title", region = region);
    Floater::new(
        WINDOW,
        title,
        ctx.content_rect().center() - Vec2::new(300.0, 200.0),
        Vec2::new(600.0, 400.0),
    )
    .show(ctx, p, open, |ui| {
        let line_height = ui.text_style_height(&egui::TextStyle::Monospace);
        let input_height = line_height + ui.spacing().interact_size.y;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .max_height((ui.available_height() - input_height).max(line_height))
            .show(ui, |ui| {
                if !available {
                    ui.label(RichText::new(aurora_i18n::tr!("sim-console-unsupported")).color(p.muted));
                }
                for line in console.lines() {
                    let text = match line {
                        Line::Command(c) => RichText::new(format!("> {c}")).monospace().color(p.ink),
                        Line::Output(o) => RichText::new(o).monospace().color(p.muted),
                        Line::NotSent => RichText::new(aurora_i18n::tr!("sim-console-not-sent")).color(p.danger),
                    };
                    ui.add(egui::Label::new(text).selectable(true).wrap());
                }
            });
        let edit = egui::TextEdit::singleline(&mut input.text)
            .font(egui::TextStyle::Monospace)
            .hint_text(aurora_i18n::tr!("sim-console-hint"))
            .interactive(available)
            .desired_width(f32::INFINITY);
        let r = ui.add(edit);
        if r.has_focus() {
            let (up, down) = ui.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                )
            });
            if up || down {
                input.browse(console.history(), up);
            }
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            console.send(&input.text);
            input.text.clear();
            input.browsing = None;
            r.request_focus();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window keeps its size with a long transcript over its field.
    #[test]
    fn the_window_keeps_its_size() {
        crate::ui::widgets::steady_tests::assert_window_steady(
            WINDOW,
            &[Vec2::new(600.0, 400.0), Vec2::new(300.0, 200.0)],
            aurora_i18n::tr!("sim-console-hint"),
            || {
                let mut console = SimConsole::default();
                for i in 0..20 {
                    console.send(&format!("stats {i}"));
                    console.reply(Ok(format!("Line {i} of the simulator's answer")));
                }
                (console, ConsoleInput::default())
            },
            |(console, input), ctx, p| {
                show(ctx, p, &mut true, "Aurora", true, console, input);
            },
        );
    }

    #[test]
    fn the_arrow_keys_walk_the_history() {
        let history = ["help".to_owned(), "stats".to_owned()];
        let mut input = ConsoleInput::default();
        input.browse(&history, true);
        assert_eq!(input.text, "stats");
        input.browse(&history, true);
        input.browse(&history, true);
        assert_eq!(input.text, "help");
        input.browse(&history, false);
        assert_eq!(input.text, "stats");
        input.browse(&history, false);
        assert_eq!((input.text.as_str(), input.browsing), ("", None));
        input.browse(&[], true);
        assert_eq!(input.text, "");
    }
}

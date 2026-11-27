//! Visual brace folding. The galley keeps every source character and native
//! cursor/undo indices; hidden rows have zero height instead of a substituted
//! text buffer. Saving, copying and compilation always use the complete source.

use super::Lang;

use super::{editing, syntax};
use crate::theme::Palette;
use egui::{
    Id, Rect, Vec2,
    text::{CCursor, LayoutJob, LayoutSection},
};
use std::{collections::BTreeSet, ops::Range};

struct Block {
    open: usize,
    hidden: Range<usize>,
    lines: usize,
}

#[derive(Default)]
pub(super) struct State {
    source: String,
    blocks: Vec<Block>,
    collapsed: BTreeSet<usize>,
    pub controls: Vec<(usize, Rect)>,
}

impl State {
    /// The folds of `source`, read as Lua when `lua`: its words open and close
    /// its blocks (`function … end`, `if … end`, `repeat … until`), where an
    /// LSL block is a pair of braces; a Lua table's braces fold as well.
    pub fn sync(&mut self, source: &str, lang: Lang) {
        let lua = lang.is_lua();
        if self.source == source {
            return;
        }
        let mut prefix = self.source.bytes().zip(source.bytes()).take_while(|(a, b)| a == b).count();
        while !source.is_char_boundary(prefix) {
            prefix -= 1;
        }
        let mut suffix = self.source[prefix..]
            .bytes()
            .rev()
            .zip(source[prefix..].bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();
        while !source.is_char_boundary(source.len() - suffix) {
            suffix -= 1;
        }
        let old_end = self.source.len() - suffix;
        let shift = source.len() as isize - self.source.len() as isize;
        self.collapsed = self
            .collapsed
            .iter()
            .filter_map(|&open| {
                if open < prefix {
                    Some(open)
                } else if open >= old_end {
                    open.checked_add_signed(shift)
                } else {
                    None
                }
            })
            .collect();
        let starts: Vec<_> = std::iter::once(0).chain(source.match_indices('\n').map(|(i, _)| i + 1)).collect();
        let mut pairs: Vec<(usize, usize)> = syntax::brace_pairs(source);
        if lua {
            pairs.extend(
                syntax::lua_block_pairs(source)
                    .into_iter()
                    .map(|(open, close)| (open.start, close.start)),
            );
            pairs.sort_unstable();
        }
        self.blocks = pairs
            .into_iter()
            .filter_map(|(open, close)| {
                let first = starts.partition_point(|&start| start <= open) - 1;
                let last = starts.partition_point(|&start| start <= close) - 1;
                (last > first + 1).then(|| Block {
                    open,
                    hidden: starts[first + 1]..starts[last],
                    lines: last - first - 1,
                })
            })
            .collect();
        self.collapsed.retain(|open| self.blocks.iter().any(|b| b.open == *open));
        self.source = source.to_owned();
    }

    pub fn reveal(&mut self, cursor: CCursor) -> bool {
        let at = self.source.char_indices().nth(cursor.index.0).map_or(self.source.len(), |(i, _)| i);
        let before = self.collapsed.len();
        self.collapsed
            .retain(|open| !self.blocks.iter().any(|b| b.open == *open && b.hidden.contains(&at)));
        self.collapsed.len() != before
    }

    pub fn layout(&self, job: &mut LayoutJob) {
        let mut ranges = Vec::<Range<usize>>::new();
        for block in self.blocks.iter().filter(|b| self.collapsed.contains(&b.open)) {
            if ranges.last().is_none_or(|r| r.end <= block.hidden.start) {
                ranges.push(block.hidden.clone());
            }
        }
        if ranges.is_empty() {
            return;
        }
        let mut ranges = ranges.into_iter().peekable();
        let mut sections = Vec::new();
        for section in job.sections.drain(..) {
            let mut start = section.byte_range.start.0;
            let end = section.byte_range.end.0;
            while start < end {
                while ranges.peek().is_some_and(|r| r.end <= start) {
                    ranges.next();
                }
                let hidden = ranges.peek().is_some_and(|r| r.contains(&start));
                let limit = ranges.peek().map_or(end, |r| end.min(if hidden { r.end } else { r.start }));
                let mut format = section.format.clone();
                if hidden {
                    // epaint requires a strictly positive glyph scale, even
                    // when the row is transparent and has no layout height.
                    format.font_id.size = f32::EPSILON;
                    format.line_height = Some(0.0);
                    format.color = egui::Color32::TRANSPARENT;
                    format.background = egui::Color32::TRANSPARENT;
                    format.underline = egui::Stroke::NONE;
                    format.strikethrough = egui::Stroke::NONE;
                    format.extra_letter_spacing = 0.0;
                }
                sections.push(LayoutSection {
                    leading_space: if !hidden && start == section.byte_range.start.0 {
                        section.leading_space
                    } else {
                        0.0
                    },
                    byte_range: start.into()..limit.into(),
                    format,
                });
                start = limit;
            }
        }
        job.sections = sections;
    }

    pub fn expand_all(&mut self) {
        self.collapsed.clear();
    }
    pub fn collapse_all(&mut self) {
        self.collapsed = self.blocks.iter().map(|b| b.open).collect();
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, p: &Palette, id: Id, output: &egui::text_edit::TextEditOutput, left: f32) {
        self.controls.clear();
        let mut clicked = None;
        let mut previous = 0;
        let mut chars = 0;
        let mut previous_row = None;
        for block in &self.blocks {
            chars += self.source[previous..block.open].chars().count();
            previous = block.open;
            let cursor = output
                .galley
                .pos_from_cursor(CCursor::new(chars))
                .translate(output.galley_pos.to_vec2());
            if cursor.height() < 1.0 {
                continue;
            }
            if previous_row == Some(cursor.top()) {
                continue;
            }
            previous_row = Some(cursor.top());
            let rect = Rect::from_center_size(egui::pos2(left + 9.0, cursor.center().y), Vec2::splat(14.0));
            if !ui.clip_rect().intersects(rect) {
                continue;
            }
            self.controls.push((block.open, rect));
            let collapsed = self.collapsed.contains(&block.open);
            let response = ui.interact(rect, id.with(("fold", block.open)), egui::Sense::click());
            if let Some(icon) = crate::ui::icons::global(if collapsed { "caret-right-filled" } else { "caret-down-filled" }) {
                ui.painter().image(
                    icon.id(),
                    rect,
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    if response.hovered() { p.ink } else { p.muted },
                );
            } else {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    if collapsed { "▸" } else { "▾" },
                    egui::FontId::proportional(13.0),
                    p.muted,
                );
            }
            if response
                .on_hover_text(if collapsed {
                    aurora_i18n::tr!("script-fold-expand")
                } else {
                    aurora_i18n::tr!("script-fold-collapse")
                })
                .clicked()
            {
                clicked = Some(block.open);
            }
            if collapsed {
                let newline = self.source[block.open..].find('\n').map_or(self.source.len(), |n| block.open + n);
                let end = self.source[..newline].chars().count();
                let anchor = output
                    .galley
                    .pos_from_cursor(CCursor::new(end))
                    .translate(output.galley_pos.to_vec2());
                let label = format!("⋯ {}", aurora_i18n::tr!("script-fold-line-count", count = block.lines));
                let galley = ui.painter().layout_no_wrap(label, egui::FontId::proportional(11.0), p.muted);
                let label_rect = Rect::from_min_size(egui::pos2(anchor.left() + 8.0, anchor.top()), galley.size());
                ui.painter().galley(label_rect.min, galley, p.muted);
                if ui
                    .interact(label_rect, id.with(("fold_label", block.open)), egui::Sense::click())
                    .clicked()
                {
                    clicked = Some(block.open);
                }
            }
        }
        if let Some(open) = clicked {
            if !self.collapsed.remove(&open) {
                self.collapsed.insert(open);
                let cursor = editing::cursor(ui.ctx(), id);
                let at = self
                    .source
                    .char_indices()
                    .nth(cursor.primary.index.0)
                    .map_or(self.source.len(), |(i, _)| i);
                if self.blocks.iter().any(|b| b.open == open && b.hidden.contains(&at)) {
                    let at = self.source[..open + 1].chars().count();
                    editing::select(ui.ctx(), id, at..at, false);
                }
            }
            ui.ctx().request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folded_label_follows_the_end_of_a_wrapped_header() {
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let source = format!("default {{ // {}\n    llOwnerSay(\"hidden\");\n}}", "long header ".repeat(15));
        let mut state = State::default();
        state.sync(&source, Lang::Lsl);
        state.collapse_all();
        let mut expected_y = 0.0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _: f32| {
                let mut job = LayoutJob::simple(text.as_str().into(), egui::FontId::monospace(13.0), p.ink, 200.0);
                state.layout(&mut job);
                ui.fonts_mut(|fonts| fonts.layout_job(job))
            };
            let mut text = source.as_str();
            let output = egui::TextEdit::multiline(&mut text).layouter(&mut layouter).show(ui);
            let end = source[..source.find('\n').expect("newline")].chars().count();
            let anchor = output.galley.pos_from_cursor(CCursor::new(end));
            assert!(anchor.top() > output.galley.rows[0].pos.y + 30.0, "header must wrap");
            expected_y = output.galley_pos.y + anchor.top();
            state.draw(ui, &p, Id::unique("wrapped_fold"), &output, 0.0);
        });
        let label = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.job.text.starts_with('⋯') => Some(text),
                _ => None,
            })
            .expect("folded label");
        assert!((label.pos.y - expected_y).abs() < 1.0);
        output.textures_delta.clear();
    }

    /// A Lua script folds on its words: a `function` to its `end`, an `if`
    /// to its `end`, a `repeat` to its `until`, a table's braces too, and no
    /// word of a comment or a string opens a block.
    #[test]
    fn lua_blocks_fold_on_their_words() {
        let source = "\
local function f(x)
    if x then
        do
            print(x)
        end
    end
    repeat
        x = x - 1
    until x == 0
    local t = {
        1,
        2,
    }
    -- function and do
    local s = [[ if then end ]]
end
";
        let mut state = State::default();
        state.sync(source, Lang::Lua);
        // the function, the if, the do it holds, the repeat and the table
        assert_eq!(
            state.blocks.len(),
            5,
            "{:?}",
            state.blocks.iter().map(|b| b.open).collect::<Vec<_>>()
        );
        // the function's fold hides every line of its body, its own `end` kept
        let function = state.blocks.iter().find(|b| b.open == 6).expect("the function's block");
        assert_eq!(function.lines, 14, "every line of its body, its own `end` kept");
        assert_eq!(
            &source[function.hidden.clone()],
            &source[source.find("    if x then").unwrap()..source.rfind("\nend").unwrap() + 1]
        );
        // colliding ranges fold nothing, as the braces' do
        state.collapsed.insert(6);
        let mut job = LayoutJob::simple(source.into(), egui::FontId::monospace(13.0), egui::Color32::WHITE, f32::INFINITY);
        state.layout(&mut job);
        assert_eq!(job.text, source);
        assert!(job.sections.iter().any(|section| section.format.font_id.size == f32::EPSILON));
        // read as LSL, the same text has no block to fold but the table's
        let mut lsl = State::default();
        lsl.sync(source, Lang::Lsl);
        assert_eq!(lsl.blocks.len(), 1, "a table's braces alone");
    }

    #[test]
    fn hidden_rows_have_zero_height_but_keep_all_source_and_character_indices() {
        let ctx = egui::Context::default();
        let source = "default {\n    event() {\n        // éあ\n        llSay(0, \"{}\");\n    }\n}\n";
        let mut state = State::default();
        state.sync(source, Lang::Lsl);
        assert_eq!(state.blocks.len(), 2);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let job = LayoutJob::simple(source.into(), egui::FontId::monospace(14.0), egui::Color32::WHITE, f32::INFINITY);
            let original = ui.fonts_mut(|f| f.layout_job(job.clone()));
            state.collapsed.insert(source.find('{').expect("brace"));
            let mut folded = job;
            state.layout(&mut folded);
            assert_eq!(folded.text, source);
            let folded = ui.fonts_mut(|f| f.layout_job(folded));
            assert!(
                folded.size().y < original.size().y * 0.6,
                "{} vs {}",
                folded.size().y,
                original.size().y
            );
            assert_eq!(folded.rows.len(), original.rows.len());
            assert_eq!(
                folded.rows.iter().map(|r| r.glyphs.len()).sum::<usize>(),
                source.chars().filter(|c| *c != '\n').count()
            );
            assert!(folded.rows.iter().filter(|r| r.height() < 1.0).count() >= 4);
        });
        output.textures_delta.clear();
    }

    #[test]
    fn clicking_fold_arrows_keeps_editable_and_readonly_documents_clean() {
        use super::super::{Editors, settings::ScriptSettings};
        use crate::{theme::Theme, ui::color_picker::Preferences};
        use aurora_net::scripts::ScriptTarget;
        use uuid::Uuid;
        for editable in [false, true] {
            let ctx = egui::Context::default();
            Theme::default().apply(&ctx, 1.0);
            let mut editors = Editors::default();
            let target = ScriptTarget {
                object: Uuid::nil(),
                item: Uuid::from_u128(42),
            };
            let source = "default {\n    event() {\n        code;\n    }\n}";
            let request = editors.open(target, "Folding".into(), true, editable, 0).expect("load");
            editors.loaded(request, target, Ok(source.into()));
            let frame = |editors: &mut Editors, events, time| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 800.0))),
                        events,
                        time: Some(time),
                        focused: true,
                        ..Default::default()
                    },
                    |ui| {
                        editors.show(
                            ui.ctx(),
                            &Theme::default().palette(),
                            &mut ScriptSettings::default(),
                            &mut Preferences::default(),
                            &[],
                        );
                    },
                );
                output.textures_delta.clear();
            };
            for n in 0..4 {
                frame(&mut editors, vec![], n as f64 * 0.02);
            }
            let (_, rect) = editors.documents[0].folding.controls[0];
            let pos = rect.center();
            for (pressed, time) in [(true, 0.1), (false, 0.12)] {
                frame(
                    &mut editors,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    time,
                );
            }
            frame(&mut editors, vec![], 0.14);
            assert_eq!(editors.documents[0].folding.collapsed.len(), 1);
            assert_eq!(editors.documents[0].source, source);
            assert!(!editors.documents[0].dirty());
            assert!(!editors.documents[0].can_save());
        }
    }

    #[test]
    fn fold_positions_follow_edits_and_navigation_reveals_the_destination() {
        let original = "// {\ndefault {\n    event() {\n        code;\n    }\n}";
        let mut state = State::default();
        state.sync(original, Lang::Lsl);
        assert_eq!(state.blocks.len(), 2);
        state.collapse_all();
        let updated = format!("// éあ\n{original}");
        state.sync(&updated, Lang::Lsl);
        assert_eq!(state.collapsed.len(), 2);
        let at = updated[..updated.find("code").expect("code")].chars().count();
        assert!(state.reveal(CCursor::new(at)));
        assert!(state.collapsed.is_empty());
        state.collapse_all();
        state.expand_all();
        assert!(state.collapsed.is_empty());
    }
}

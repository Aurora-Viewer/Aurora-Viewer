//! Inline LSL color literals, using the shared Aurora color picker. This is
//! an intentional addition to Firestorm's script editor: numeric unit vectors
//! are treated as potential colors without guessing from variable names.

use super::{
    editing,
    syntax::{COLOR_SPACE, ColorLiteral},
};
use crate::{
    theme::Palette,
    ui::color_picker::{ColorPicker, Event, Preferences, Rgb},
};
use egui::{
    Id, Rect, Vec2,
    text::{CCursor, CCursorRange},
};

struct Edit {
    index: usize,
    literal: ColorLiteral,
    original: String,
    expected: String,
    cursor: CCursorRange,
    id: Id,
}

pub(super) struct State {
    pub picker: ColorPicker,
    edit: Option<Edit>,
    pub clicked: Option<usize>,
    pub swatches: Vec<Rect>,
}

impl State {
    pub fn new(id: String) -> Self {
        Self {
            picker: ColorPicker::named(id),
            edit: None,
            clicked: None,
            swatches: Vec::new(),
        }
    }

    pub fn active(&self) -> bool {
        self.edit.is_some()
    }

    pub fn cancel(&mut self, source: &mut String) {
        if let Some(edit) = self.edit.take()
            && *source == edit.expected
        {
            source.clone_from(&edit.original);
        }
        self.picker.close();
    }

    pub fn open(&mut self, source: &mut String, ctx: &egui::Context, id: Id, parent: Id, index: usize) {
        let same = self.edit.as_ref().is_some_and(|edit| edit.index == index);
        self.cancel(source);
        if same {
            return;
        }
        // Reparse after restoring the previous preview: its component lengths
        // may have shifted all later literals, but their order is unchanged.
        let Some(literal) = super::syntax::colors(source).into_iter().nth(index) else {
            return;
        };
        self.edit = Some(Edit {
            index,
            literal: literal.clone(),
            original: source.clone(),
            expected: source.clone(),
            cursor: editing::cursor(ctx, id),
            id,
        });
        self.picker.open_from(literal.rgb, false, parent);
        if std::env::var("AURORA_DEMO_SCRIPTS").is_ok_and(|mode| matches!(mode.as_str(), "color-picker" | "color-accept" | "color-cancel"))
        {
            self.picker.track_controls();
        }
    }

    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        source: &str,
        output: &egui::text_edit::TextEditOutput,
        colors: &[ColorLiteral],
        editable: bool,
    ) {
        self.swatches.clear();
        let mut previous = 0;
        let mut chars = 0;
        for (index, literal) in colors.iter().enumerate() {
            chars += source[previous..literal.bytes.start].chars().count();
            previous = literal.bytes.start;
            let cursor = output
                .galley
                .pos_from_cursor(CCursor::new(chars))
                .translate(output.galley_pos.to_vec2());
            if cursor.height() < 1.0 {
                continue;
            }
            let rect = Rect::from_center_size(cursor.center() - Vec2::new(COLOR_SPACE * 0.5, 0.0), Vec2::splat(14.0));
            self.swatches.push(rect);
            if !ui.clip_rect().intersects(rect) {
                continue;
            }
            let response = ui.interact(
                rect,
                output.response.id.with(("color", literal.bytes.start)),
                if editable { egui::Sense::click() } else { egui::Sense::hover() },
            );
            ui.painter()
                .circle_filled(rect.center(), 6.0, crate::ui::color_picker::color(literal.rgb));
            ui.painter().circle_stroke(
                rect.center(),
                6.0,
                egui::Stroke::new(1.0, if response.hovered() { p.ink } else { p.muted_dim }),
            );
            if response.on_hover_text(aurora_i18n::tr!("script-color-hint")).clicked() {
                self.clicked = Some(index);
            }
            if self
                .edit
                .as_ref()
                .is_some_and(|edit| edit.literal.bytes.start == literal.bytes.start)
            {
                self.picker.set_origin(rect.intersect(ui.clip_rect()));
            }
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, prefs: &mut Preferences, source: &mut String, editable: bool) {
        if self.edit.as_ref().is_some_and(|edit| *source != edit.expected) || !editable {
            self.cancel(source);
            return;
        }
        if let Some(event) = self.picker.show(ctx, p, prefs) {
            self.event(ctx, source, event);
        }
    }

    fn event(&mut self, ctx: &egui::Context, source: &mut String, event: Event) {
        let Some(edit) = self.edit.as_mut() else {
            return;
        };
        if *source != edit.expected {
            self.cancel(source);
            return;
        }
        match event {
            Event::Preview(rgb) => {
                *source = replacement(edit, rgb);
                edit.expected.clone_from(source);
            }
            Event::Accept(rgb) => {
                let updated = replacement(edit, rgb);
                source.clone_from(&edit.original);
                editing::select(
                    ctx,
                    edit.id,
                    edit.cursor.as_sorted_char_range().start.0..edit.cursor.as_sorted_char_range().end.0,
                    false,
                );
                if updated != *source {
                    let start = source[..edit.literal.bytes.start].chars().count();
                    let end = source[..edit.literal.bytes.end].chars().count();
                    let new_end = updated.len() - (edit.original.len() - edit.literal.bytes.end);
                    editing::replace(ctx, edit.id, source, start..end, &updated[edit.literal.bytes.start..new_end]);
                }
                ctx.memory_mut(|m| m.request_focus(edit.id));
                self.edit = None;
            }
            Event::Cancel => {
                let id = edit.id;
                let cursor = edit.cursor.as_sorted_char_range();
                self.cancel(source);
                editing::select(ctx, id, cursor.start.0..cursor.end.0, true);
            }
        }
        ctx.request_repaint();
    }
}

fn replacement(edit: &Edit, rgb: Rgb) -> String {
    let mut result = edit.original.clone();
    if rgb == edit.literal.rgb {
        return result;
    }
    for ((range, value), original) in edit.literal.components.iter().zip(rgb).zip(edit.literal.rgb).rev() {
        if value != original {
            result.replace_range(range.clone(), &format!("{value:.3}"));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::super::{Editors, settings::ScriptSettings, syntax::Highlighter};
    use super::*;
    use crate::theme::Theme;
    use crate::ui::color_picker::Control;
    use aurora_net::scripts::ScriptTarget;
    use egui::{Event as Input, Modifiers, PointerButton, Pos2};
    use uuid::Uuid;

    fn literal(source: &str) -> ColorLiteral {
        let mut highlighter = Highlighter::default();
        highlighter.layout(source, &Theme::default().palette(), egui::FontId::monospace(14.0));
        highlighter.colors.remove(0)
    }

    #[test]
    fn switching_swatches_restores_the_previous_preview_before_locating_the_next_vector() {
        let ctx = egui::Context::default();
        let original = "vector a = <0,0,0>; vector b = <1,0,0>;";
        let mut source = original.to_owned();
        let mut state = State::new("switch".into());
        state.open(&mut source, &ctx, Id::unique("source"), Id::unique("parent"), 0);
        state.event(&ctx, &mut source, Event::Preview([0.5; 3]));
        state.open(&mut source, &ctx, Id::unique("source"), Id::unique("parent"), 1);
        assert_eq!(source, original);
        state.event(&ctx, &mut source, Event::Accept([0.0, 1.0, 0.0]));
        assert_eq!(source, "vector a = <0,0,0>; vector b = <0.000,1.000,0>;");
    }

    #[test]
    fn unchanged_channels_and_noop_accept_keep_the_original_numeric_precision() {
        let ctx = egui::Context::default();
        let original = "vector color = <0.1234567, .25, 1e-1>;";
        let mut source = original.to_owned();
        let mut state = State::new("precise".into());
        let color = literal(original);
        let rgb = color.rgb;
        state.open(&mut source, &ctx, Id::unique("source"), Id::unique("parent"), 0);
        state.event(&ctx, &mut source, Event::Accept(rgb));
        assert_eq!(source, original);
        state.open(&mut source, &ctx, Id::unique("source"), Id::unique("parent"), 0);
        state.event(&ctx, &mut source, Event::Preview([rgb[0], 0.5, rgb[2]]));
        assert_eq!(source, "vector color = <0.1234567, 0.500, 1e-1>;");
        state.event(&ctx, &mut source, Event::Cancel);
        assert_eq!(source, original);
    }

    #[test]
    fn preview_cancel_preserves_exact_text_accept_is_one_native_undo_and_external_edits_are_kept() {
        let ctx = egui::Context::default();
        let id = Id::unique("source");
        let original = "// éあ\nvector c = <.1, /*green*/\t0.2,\n +3e-1>;";
        let mut source = original.to_owned();
        let mut state = State::new("color_test".into());
        state.open(&mut source, &ctx, id, Id::unique("parent"), 0);
        state.event(&ctx, &mut source, Event::Preview([1.0, 0.0, 0.5]));
        assert_eq!(source, "// éあ\nvector c = <1.000, /*green*/\t0.000,\n 0.500>;");
        state.event(&ctx, &mut source, Event::Cancel);
        assert_eq!(source, original);
        state.open(&mut source, &ctx, id, Id::unique("parent"), 0);
        state.event(&ctx, &mut source, Event::Preview([1.0, 0.0, 0.5]));
        state.event(&ctx, &mut source, Event::Preview([0.0, 1.0, 0.25]));
        state.event(&ctx, &mut source, Event::Accept([0.0, 1.0, 0.25]));
        let accepted = source.clone();
        editing::undo(&ctx, id, &mut source, false);
        assert_eq!(source, original);
        editing::undo(&ctx, id, &mut source, true);
        assert_eq!(source, accepted);
        state.open(&mut source, &ctx, id, Id::unique("parent"), 0);
        state.event(&ctx, &mut source, Event::Preview([0.0; 3]));
        source.push_str("\n// external change");
        let external = source.clone();
        state.event(&ctx, &mut source, Event::Cancel);
        assert_eq!(source, external);
    }

    fn frame(ctx: &egui::Context, editors: &mut Editors, prefs: &mut Preferences, events: Vec<Input>, time: f64) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 900.0))),
                focused: true,
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                editors.show(ui.ctx(), &Theme::default().palette(), &mut ScriptSettings::default(), prefs);
            },
        );
        output.textures_delta.clear();
    }

    fn click(ctx: &egui::Context, editors: &mut Editors, prefs: &mut Preferences, pos: Pos2, time: f64) {
        frame(
            ctx,
            editors,
            prefs,
            vec![
                Input::PointerMoved(pos),
                Input::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: Modifiers::NONE,
                },
            ],
            time,
        );
        frame(
            ctx,
            editors,
            prefs,
            vec![Input::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
            time + 0.01,
        );
    }

    #[test]
    fn real_inline_click_opens_shared_palette_and_readonly_swatch_cannot_edit() {
        for editable in [false, true] {
            let ctx = egui::Context::default();
            Theme::default().apply(&ctx, 1.0);
            let mut editors = Editors::default();
            let mut prefs = Preferences::default();
            let target = ScriptTarget {
                object: Uuid::nil(),
                item: Uuid::from_u128(1),
            };
            let original = "vector c = <0.1,0.2,0.3>; vector d = <1,0,0>;";
            let request = editors.open(target, "Colors".into(), true, editable).expect("load");
            editors.loaded(request, target, Ok(original.into()));
            for n in 0..4 {
                frame(&ctx, &mut editors, &mut prefs, vec![], n as f64 * 0.02);
            }
            let swatches = editors.documents[0].colors.swatches.clone();
            assert_eq!(swatches.len(), 2);
            assert!(swatches[0].right() < swatches[1].left());
            click(&ctx, &mut editors, &mut prefs, swatches[0].center(), 0.1);
            assert_eq!(editors.color_open(), editable);
            assert_eq!(editors.documents[0].source, original);
            if editable {
                for n in 0..3 {
                    frame(&ctx, &mut editors, &mut prefs, vec![], 0.2 + n as f64 * 0.02);
                }
                let slot = editors.documents[0].colors.picker.controls[&Control::Slot(20)].center();
                click(&ctx, &mut editors, &mut prefs, slot, 0.3);
                assert!(editors.documents[0].source.contains("<0.000,1.000,0.000>"));
                assert!(!editors.documents[0].can_save());
                let cancel = editors.documents[0].colors.picker.controls[&Control::Cancel].center();
                click(&ctx, &mut editors, &mut prefs, cancel, 0.4);
                assert_eq!(editors.documents[0].source, original);
                assert!(!editors.documents[0].dirty());
            }
        }
    }
}

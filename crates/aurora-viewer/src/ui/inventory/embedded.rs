//! Inline inventory segments, following LLEmbeddedItemSegment and
//! LLViewerTextEditor::handleDragAndDrop (Firestorm, LGPL 2.1).
//! Each attachment occupies one character, so selection, undo and deletion
//! use egui's editor while the icon and name reserve their actual width.

use super::*;
use aurora_net::inventory::notecard::Notecard;
use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};

/// The native key handler must leave Ctrl+S to the focused document editor.
pub fn save_focused(ctx: &egui::Context) -> bool {
    ctx.memory(|m| m.focused()).is_some_and(|id| {
        ctx.data(|d| d.get_temp::<u64>(id.with("document_save_frame")))
            .is_some_and(|frame| frame + 1 >= ctx.cumulative_frame_nr())
    })
}

#[derive(Default)]
pub(super) struct EditorResult {
    pub changed: bool,
    pub save: bool,
    pub open: Option<char>,
    pub copy: Option<char>,
    pub drop: Option<(usize, Uuid)>,
    #[cfg(test)]
    galley: Option<std::sync::Arc<egui::Galley>>,
    #[cfg(test)]
    text_clip_rect: Option<egui::Rect>,
}

#[derive(Clone)]
struct Clipboard {
    text: String,
    card: Notecard,
}

#[derive(Default)]
pub(super) enum Appearance<'a> {
    #[default]
    Classic,
    Source,
    CodeSource(
        Option<&'a super::markdown::Rendered>,
        &'a super::super::scripts::settings::ScriptSettings,
    ),
    Markdown(&'a super::markdown::Rendered, &'a super::super::scripts::settings::ScriptSettings),
}

pub(super) fn clipboard(ctx: &egui::Context, id: egui::Id, source: &mut Notecard, text: &str, editable: bool) {
    if !ctx.memory(|m| m.has_focus(id)) {
        return;
    }
    let key = egui::Id::unique("notecard_attachment_clipboard");
    let copying = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy | egui::Event::Cut)));
    let pasting = editable && ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))));
    if !copying && !pasting {
        return;
    }
    if copying
        && let Some(state) = egui::TextEdit::load_state(ctx, id)
        && let Some(range) = state.cursor.char_range()
    {
        let [start, end] = range.sorted_cursors();
        let selected: String = text.chars().skip(start.index.0).take(end.index.0 - start.index.0).collect();
        if !selected.is_empty() {
            ctx.data_mut(|data| {
                data.insert_temp(
                    key,
                    Clipboard {
                        text: selected,
                        card: source.clone(),
                    },
                )
            });
        }
    }
    let clip = ctx.data_mut(|data| data.get_temp::<Clipboard>(key));
    ctx.input_mut(|input| {
        for event in &mut input.events {
            if editable
                && let egui::Event::Paste(pasted) = event
                && pasted
                    .chars()
                    .any(|c| c as u32 >= aurora_net::inventory::notecard::FIRST_EMBEDDED_CHAR)
            {
                if let Some(clip) = clip.as_ref().filter(|clip| clip.text == *pasted) {
                    *pasted = source.paste(pasted, &clip.card).unwrap_or_default();
                } else {
                    // Unverified system clipboard markers must never point to an
                    // unrelated item of this card, even when indices coincide.
                    pasted.retain(|c| (c as u32) < aurora_net::inventory::notecard::FIRST_EMBEDDED_CHAR);
                }
            }
        }
    });
}

pub(super) fn editor(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    id: egui::Id,
    text: &mut String,
    source: Option<&Notecard>,
    editable: bool,
    size: Vec2,
    appearance: Appearance<'_>,
) -> EditorResult {
    let style = if matches!(appearance, Appearance::Source | Appearance::CodeSource(..)) {
        egui::TextStyle::Monospace
    } else {
        egui::TextStyle::Body
    };
    let font = style.resolve(ui.style());
    let markdown = if let Appearance::Markdown(rendered, _) = &appearance {
        Some(*rendered)
    } else {
        None
    };
    let editable = editable && markdown.is_none();
    // Reserve a layer behind the immutable editor's transparent frame and text.
    // Native selection and cursor painting remain above the code surfaces.
    let code_background = markdown
        .filter(|rendered| !rendered.code_blocks.is_empty())
        .map(|_| ui.painter().add(egui::Shape::Noop));
    let label_font = egui::TextStyle::Body.resolve(ui.style());
    let mut labels = std::collections::HashMap::new();
    if let Some(source) = source {
        for marker in markdown.map_or(text.as_str(), |rendered| rendered.text.as_str()).chars() {
            if labels.contains_key(&marker) {
                continue;
            }
            if let Some(attachment) = source.attachment(marker) {
                let label = ui.fonts_mut(|fonts| fonts.layout_no_wrap(attachment.item.name.clone(), label_font.clone(), p.teal));
                labels.insert(marker, label);
            }
        }
    }
    // Braille blank is invisible but is not whitespace: egui must neither
    // trim an attachment at the beginning of a row nor split its width.
    let font_scale = super::super::fonts::scale(ui.ctx());
    let (blank, unit_width, line_height) = ui.fonts_mut(|fonts| {
        let blank = if fonts.has_glyph(&font, '\u{2800}') { '\u{2800}' } else { 'M' };
        let unit = egui::FontId::new(1.0, font.family.clone());
        // egui's glyph_width uses the nominal size, unlike shaped advances.
        (
            blank,
            fonts.glyph_width(&unit, blank) * font_scale,
            fonts.row_height(&font).max(fonts.row_height(&label_font)),
        )
    });
    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
        let normal = TextFormat {
            font_id: font.clone(),
            color: p.ink,
            ..Default::default()
        };
        let mut job = LayoutJob::default();
        job.wrap.max_width = wrap_width;
        let mut append = |job: &mut LayoutJob, segment: &str, normal: TextFormat| {
            let mut start = 0;
            for (byte, marker) in segment.char_indices() {
                if let Some(label) = labels.get(&marker) {
                    job.append(&segment[start..byte], 0.0, normal.clone());
                    // One wide glyph keeps egui's character indices, wrapping and
                    // selection aligned with the one-character asset marker.
                    job.append(
                        &blank.to_string(),
                        0.0,
                        TextFormat {
                            font_id: egui::FontId::new((label.size().x + 24.0) / unit_width, font.family.clone()),
                            color: egui::Color32::TRANSPARENT,
                            line_height: Some(line_height.max(normal.font_id.size)),
                            ..normal.clone()
                        },
                    );
                    start = byte + marker.len_utf8();
                }
            }
            job.append(&segment[start..], 0.0, normal);
        };
        if let Appearance::Markdown(rendered, settings) = &appearance {
            rendered.layout(ui, p, settings, &mut append, &mut job);
        } else if let Appearance::CodeSource(Some(rendered), settings) = &appearance
            && rendered.source == buffer.as_str()
        {
            super::markdown::code::source_layout(rendered, p, settings, normal, &mut append, &mut job);
        } else {
            append(&mut job, buffer.as_str(), normal);
        }
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let mut show = |ui: &mut egui::Ui, buffer: &mut dyn egui::TextBuffer| {
        egui::TextEdit::multiline(buffer)
            .id(id)
            .desired_width(size.x)
            .min_size(size)
            .desired_rows(1)
            .font(style.clone())
            .lock_focus(editable && matches!(appearance, Appearance::CodeSource(..)))
            .layouter(&mut layouter)
            .show(ui)
    };
    let mut output = if editable {
        if matches!(appearance, Appearance::CodeSource(..)) {
            let (mut output, changed) = super::super::scripts::edit_code(ui, id, text, &super::markdown::code::scope, show);
            if changed {
                output.response.mark_changed();
            }
            output
        } else {
            show(ui, text)
        }
    } else {
        show(ui, &mut markdown.map_or(text.as_str(), |rendered| rendered.text.as_str()))
    };
    // the Legacy notecard editor is spell checked and auto-replaced; not
    // the code view
    if editable && matches!(appearance, Appearance::Classic | Appearance::Source) {
        super::super::spelling::field(ui, p, &mut output, text, true);
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(id.with("demo_rect"), output.response.rect));
    if let (Some(rendered), Some(slot)) = (markdown, code_background) {
        let rects = rendered.code_rects(
            &output.galley,
            output.galley_pos,
            output.text_clip_rect,
            ui.spacing().item_spacing.y * 0.5,
        );
        ui.painter().set(
            slot,
            egui::Shape::Vec(
                rects
                    .into_iter()
                    .map(|rect| egui::Shape::rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.field))
                    .collect(),
            ),
        );
    }
    let mut result = EditorResult {
        changed: output.response.changed(),
        #[cfg(test)]
        galley: Some(output.galley.clone()),
        #[cfg(test)]
        text_clip_rect: Some(output.text_clip_rect),
        ..Default::default()
    };
    if output.response.has_focus() {
        let frame = ui.ctx().cumulative_frame_nr();
        ui.ctx().data_mut(|data| data.insert_temp(id.with("document_save_frame"), frame));
        result.save = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S));
    }
    if let Some(rendered) = markdown
        && let Some(pos) = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|pos| output.text_clip_rect.contains(*pos))
    {
        let cursor = output.galley.cursor_from_pos(pos - output.galley_pos);
        let cursor_rect = output.galley.pos_from_cursor(cursor).translate(output.galley_pos.to_vec2());
        if (cursor_rect.min.x - pos.x).abs() < font.size
            && let Some((_, url)) = rendered.links.iter().find(|(range, _)| range.contains(&cursor.index.0))
            && (url.starts_with("https://") || url.starts_with("http://"))
        {
            egui::Response::clone(&output.response)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(url.as_ref());
            if output.response.clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url.as_ref()));
            }
        }
    }
    let painter = ui.painter().with_clip_rect(output.text_clip_rect);
    if let Some(source) = source {
        for (index, marker) in markdown
            .map_or(text.as_str(), |rendered| rendered.text.as_str())
            .chars()
            .enumerate()
        {
            let Some(attachment) = source.attachment(marker) else { continue };
            let Some(label) = labels.get(&marker) else { continue };
            let mut cursor = CCursor::new(index);
            cursor.prefer_next_row = true;
            let pos = output.galley.pos_from_cursor(cursor).translate(output.galley_pos.to_vec2()).min;
            let rect = egui::Rect::from_min_size(pos, egui::vec2(label.size().x + 24.0, label.size().y));
            ui.ctx()
                .data_mut(|data| data.insert_temp(id.with(("demo_attachment", marker)), rect));
            if !output.text_clip_rect.intersects(rect) {
                continue;
            }
            if let Some(icon) = icons.get(item_icon(
                attachment.item.asset_type,
                attachment.item.inv_type,
                attachment.item.flags,
            )) {
                painter.image(
                    icon.id(),
                    egui::Rect::from_min_size(pos, Vec2::splat(16.0)),
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    p.teal,
                );
            }
            painter.galley(pos + egui::vec2(20.0, 0.0), label.clone(), p.teal);
            painter.hline(pos.x + 20.0..=rect.right() - 4.0, rect.bottom(), egui::Stroke::new(0.5, p.teal));
            let response = ui.interact(
                rect.intersect(output.text_clip_rect),
                id.with(("attachment", index)),
                egui::Sense::click(),
            );
            // LLViewerTextEditor::handleMouseUp opens items on a single click.
            if response.clicked() {
                result.open = Some(marker);
            }
            response
                .clone()
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!("{}\n{}", attachment.item.name, attachment.item.desc));
            super::super::menu::context_menu(&response, p, |ui| {
                let open = ui.scope(|ui| super::super::menu::item(ui, p, "folder-open-filled", aurora_i18n::tr!("common-open")));
                ui.ctx()
                    .data_mut(|data| data.insert_temp(id.with(("demo_attachment_open", marker)), open.response.rect));
                if open.inner {
                    result.open = Some(marker);
                }
                let copy = ui.scope(|ui| super::super::menu::item(ui, p, "copy-filled", aurora_i18n::tr!("notecard-copy-inventory")));
                ui.ctx()
                    .data_mut(|data| data.insert_temp(id.with(("demo_attachment_copy", marker)), copy.response.rect));
                if copy.inner {
                    result.copy = Some(marker);
                }
            });
        }
        if editable
            && output
                .response
                .rect
                .contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or(egui::Pos2::ZERO))
            && let Some(payload) = output.response.dnd_release_payload::<InvDrag>()
        {
            let pos = ui.input(|i| i.pointer.interact_pos()).unwrap_or(output.galley_pos);
            let cursor = output.galley.cursor_from_pos(pos - output.galley_pos);
            result.drop = Some((cursor.index.0, payload.0));
            let mut state = output.state;
            state.cursor.set_char_range(Some(CCursorRange::one(cursor + 1)));
            state.store(ui.ctx(), id);
            ui.ctx().memory_mut(|m| m.request_focus(id));
        }
    }
    result
}

pub(super) fn preview(
    ctx: &egui::Context,
    p: &Palette,
    st: &mut InventoryUi,
    images: &std::collections::HashMap<Uuid, egui::TextureHandle>,
) {
    let Some(item) = &st.attachment_preview else { return };
    let mut open = true;
    super::super::widgets::Floater::new(
        &format!("notecard_attachment_{}", st.ui_id),
        item.name.clone(),
        ctx.content_rect().center() - egui::vec2(180.0, 180.0),
        Vec2::splat(360.0),
    )
    .show(ctx, p, &mut open, |ui| {
        st.wanted_images.insert(item.asset_id);
        if let Some(image) = images.get(&item.asset_id) {
            ui.add(egui::Image::new(image).max_size(ui.available_size()).fit_to_original_size(1.0));
        } else {
            ui.spinner();
            ui.label(aurora_i18n::tr!("ui-inventory-dialogs-loading-image"));
        }
        ui.label(&item.desc);
    });
    if !open {
        st.attachment_preview = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, source: &mut Notecard, text: &mut String, editable: bool, events: Vec<egui::Event>) -> EditorResult {
        let mut result = EditorResult::default();
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(700.0, 400.0))),
                time: Some(ctx.cumulative_frame_nr() as f64 * 0.1),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                clipboard(ctx, egui::Id::unique("note"), source, text, editable);
                result = editor(
                    ui,
                    &crate::theme::Theme::default().palette(),
                    &Icons::default(),
                    egui::Id::unique("note"),
                    text,
                    Some(source),
                    editable,
                    egui::vec2(600.0, 300.0),
                    Appearance::Classic,
                );
            },
        );
        let _ = headless_output(output);
        result
    }

    fn fixture() -> (Notecard, String) {
        let source = Notecard::decode(&crate::world::inventory::demo::note_asset(true)).expect("fixture");
        let text = source.text.clone();
        (source, text)
    }

    #[test]
    fn markdown_code_backgrounds_cover_whole_blocks_below_selectable_text() {
        fn flatten<'a>(shape: &'a egui::Shape, shapes: &mut Vec<&'a egui::Shape>) {
            if let egui::Shape::Vec(children) = shape {
                for child in children {
                    flatten(child, shapes);
                }
            } else {
                shapes.push(shape);
            }
        }
        let raw = "Before `inline` 🐺\n\n```lsl\ndefault {\n\n    llOwnerSay(\"A long line that wraps in a narrow preview\");\n}\n```\n\nBetween\n\n```text\nx\n```\n\n```\n```\n\nAfter";
        let rendered = super::super::markdown::Rendered::parse(raw);
        for width in [160.0, 480.0] {
            let ctx = egui::Context::default();
            let p = crate::theme::Theme::default().palette();
            let mut text = raw.to_owned();
            let mut result = None;
            let output = headless_output(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 1000.0))),
                    ..Default::default()
                },
                |ui| {
                    result = Some(editor(
                        ui,
                        &p,
                        &Icons::default(),
                        egui::Id::unique("code_background"),
                        &mut text,
                        None,
                        false,
                        egui::vec2(width, 900.0),
                        Appearance::Markdown(&rendered, &Default::default()),
                    ));
                },
            ));
            assert_eq!(text, raw, "painting must not change the saved source");
            let result = result.expect("editor");
            let clip = result.text_clip_rect.expect("visible text bounds");
            let galley = result.galley.expect("text layout");
            let highlights: Vec<_> = galley
                .job
                .sections
                .iter()
                .filter(|section| section.format.background == p.field)
                .collect();
            assert_eq!(highlights.len(), 1, "only inline code keeps a glyph background");
            let range = &highlights[0].byte_range;
            assert_eq!(&galley.job.text[range.start.0..range.end.0], "inline");
            let mut shapes = Vec::new();
            for shape in &output.shapes {
                flatten(&shape.shape, &mut shapes);
            }
            let backgrounds: Vec<_> = shapes
                .iter()
                .enumerate()
                .filter_map(|(index, shape)| {
                    if let egui::Shape::Rect(rect) = shape
                        && rect.fill == p.field
                    {
                        Some((index, rect.rect))
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(backgrounds.len(), 3, "one rectangle for LSL, plain and empty blocks");
            let first = backgrounds[0].1;
            assert!(
                first.height() > 4.0 * egui::TextStyle::Monospace.resolve(&ctx.global_style()).size,
                "the surface must also cover blank and wrapped rows"
            );
            for (_, rect) in &backgrounds {
                assert_eq!(rect.left(), clip.left());
                assert_eq!(rect.right(), clip.right());
            }
            assert!(backgrounds.windows(2).all(|pair| pair[0].1.bottom() < pair[1].1.top()));
            let text_index = shapes
                .iter()
                .position(|shape| matches!(shape, egui::Shape::Text(_)))
                .expect("selectable text");
            assert!(
                backgrounds.iter().all(|(index, _)| *index < text_index),
                "surfaces stay below text and native selection"
            );
        }
    }

    fn key(key: egui::Key, command: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                ctrl: command,
                command,
                ..Default::default()
            },
        }
    }

    fn click(pos: egui::Pos2, button: egui::PointerButton, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Default::default(),
            },
        ]
    }

    #[test]
    fn one_left_click_opens_the_attachment_directly_in_editable_and_read_only_notes() {
        for editable in [true, false] {
            for marker in ['\u{100000}', '\u{100001}', '\u{100002}'] {
                let ctx = egui::Context::default();
                let (mut source, mut text) = fixture();
                let original = text.clone();
                for _ in 0..5 {
                    frame(&ctx, &mut source, &mut text, editable, vec![]);
                }
                let attachment = ctx
                    .data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::unique("note").with(("demo_attachment", marker))))
                    .expect("attachment");
                let pressed = frame(
                    &ctx,
                    &mut source,
                    &mut text,
                    editable,
                    click(attachment.center(), egui::PointerButton::Primary, true),
                );
                assert!(pressed.open.is_none());
                let released = frame(
                    &ctx,
                    &mut source,
                    &mut text,
                    editable,
                    click(attachment.center(), egui::PointerButton::Primary, false),
                );
                assert_eq!(released.open, Some(marker));
                assert!(released.copy.is_none());
                assert!(!egui::Popup::is_any_open(&ctx), "left click must open directly without a menu");
                assert_eq!(text, original);
                assert!(!released.changed);
            }
        }
    }

    #[test]
    fn right_click_keeps_open_and_copy_actions_in_editable_and_read_only_notes() {
        for editable in [true, false] {
            let button = egui::PointerButton::Secondary;
            for action in ["open", "copy"] {
                let ctx = egui::Context::default();
                let (mut source, mut text) = fixture();
                let original = text.clone();
                for _ in 0..5 {
                    frame(&ctx, &mut source, &mut text, editable, vec![]);
                }
                let id = egui::Id::unique("note");
                let marker = '\u{100000}';
                let attachment = ctx
                    .data_mut(|data| data.get_temp::<egui::Rect>(id.with(("demo_attachment", marker))))
                    .expect("image");
                frame(&ctx, &mut source, &mut text, editable, click(attachment.center(), button, true));
                let result = frame(&ctx, &mut source, &mut text, editable, click(attachment.center(), button, false));
                assert!(egui::Popup::is_any_open(&ctx), "{button:?} must open the menu");
                assert!(
                    result.open.is_none() && result.copy.is_none(),
                    "opening the menu must not execute an action"
                );
                for _ in 0..2 {
                    frame(&ctx, &mut source, &mut text, editable, vec![]);
                }
                for entry in ["demo_attachment_open", "demo_attachment_copy"] {
                    assert!(ctx.data_mut(|data| data.get_temp::<egui::Rect>(id.with((entry, marker)))).is_some());
                }
                let key = if action == "open" {
                    "demo_attachment_open"
                } else {
                    "demo_attachment_copy"
                };
                let row = ctx
                    .data_mut(|data| data.get_temp::<egui::Rect>(id.with((key, marker))))
                    .expect("menu action");
                frame(
                    &ctx,
                    &mut source,
                    &mut text,
                    editable,
                    click(row.center(), egui::PointerButton::Primary, true),
                );
                let result = frame(
                    &ctx,
                    &mut source,
                    &mut text,
                    editable,
                    click(row.center(), egui::PointerButton::Primary, false),
                );
                assert_eq!(result.open, (action == "open").then_some(marker));
                assert_eq!(result.copy, (action == "copy").then_some(marker));
                assert!(!egui::Popup::is_any_open(&ctx), "selecting an action must close the menu");
                assert_eq!(text, original);
                assert!(!result.changed);
            }
        }
    }

    #[test]
    fn inventory_drop_below_the_text_uses_character_positions() {
        let ctx = egui::Context::default();
        let (mut source, mut text) = fixture();
        for _ in 0..5 {
            frame(&ctx, &mut source, &mut text, true, vec![]);
        }
        let id = egui::Id::unique("note");
        // Drag-and-drop remains available in blank space below the text.
        let rect = ctx
            .data_mut(|data| data.get_temp::<egui::Rect>(id.with("demo_rect")))
            .expect("editor");
        let pos = rect.center();
        egui::DragAndDrop::set_payload(&ctx, InvDrag(Uuid::from_u128(8104)));
        frame(
            &ctx,
            &mut source,
            &mut text,
            true,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
        );
        let result = frame(
            &ctx,
            &mut source,
            &mut text,
            true,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
        );
        assert_eq!(result.drop, Some((text.chars().count(), Uuid::from_u128(8104))));
    }

    #[test]
    fn attachments_reserve_their_width_inline_and_at_wrapped_row_starts() {
        let ctx = egui::Context::default();
        let mut source = fixture().0;
        for prefix in ["", "abc", "a ".repeat(60).as_str()] {
            let mut text = format!("{prefix}\u{100000} suite");
            let result = frame(&ctx, &mut source, &mut text, true, vec![]);
            let galley = result.galley.expect("layout");
            let mut cursor = CCursor::new(prefix.chars().count());
            cursor.prefer_next_row = true;
            let start = galley.pos_from_cursor(cursor);
            let end = galley.pos_from_cursor(cursor + 1);
            let rect = ctx
                .data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::unique("note").with(("demo_attachment", '\u{100000}'))))
                .expect("attachment");
            assert!(start.min.x >= 0.0, "attachment must not be trimmed at row start");
            assert!(
                (end.min.x - start.min.x - rect.width()).abs() < 2.0,
                "following text must clear the full label"
            );
            assert!((start.min.y - end.min.y).abs() < 1.0, "attachment is indivisible");
            assert!(start.height() < 30.0, "blank glyph must retain normal row height");
        }
    }

    #[test]
    fn deleting_and_undoing_an_attachment_preserves_its_metadata() {
        let ctx = egui::Context::default();
        let (mut source, mut text) = fixture();
        let original = text.clone();
        for _ in 0..5 {
            frame(&ctx, &mut source, &mut text, true, vec![]);
        }
        let id = egui::Id::unique("note");
        let index = text.chars().position(|c| c == '\u{100000}').expect("marker");
        let mut state = egui::TextEdit::load_state(&ctx, id).expect("editor");
        state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(index), CCursor::new(index + 1))));
        state.store(&ctx, id);
        ctx.memory_mut(|m| m.request_focus(id));
        assert!(frame(&ctx, &mut source, &mut text, true, vec![key(egui::Key::Backspace, false)]).changed);
        assert!(!text.contains('\u{100000}'));
        for _ in 0..5 {
            frame(&ctx, &mut source, &mut text, true, vec![]);
        }
        frame(&ctx, &mut source, &mut text, true, vec![key(egui::Key::Z, true)]);
        assert_eq!(text, original);
        assert_eq!(
            Notecard::decode(&source.encode(&text).expect("save"))
                .expect("read")
                .attachment('\u{100000}')
                .expect("item")
                .item
                .asset_type,
            0
        );
    }

    #[test]
    fn read_only_editor_keeps_the_text_and_rejects_inventory_drops() {
        let ctx = egui::Context::default();
        let (mut source, mut text) = fixture();
        let original = text.clone();
        for _ in 0..5 {
            frame(&ctx, &mut source, &mut text, false, vec![]);
        }
        ctx.memory_mut(|m| m.request_focus(egui::Id::unique("note")));
        frame(&ctx, &mut source, &mut text, false, vec![egui::Event::Text("changed".into())]);
        assert_eq!(text, original);
        egui::DragAndDrop::set_payload(&ctx, InvDrag(Uuid::from_u128(8104)));
        let pos = egui::pos2(100.0, 100.0);
        let result = frame(
            &ctx,
            &mut source,
            &mut text,
            false,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
        );
        assert!(result.drop.is_none());
        assert_eq!(text, original);
    }

    #[test]
    fn clipboard_from_a_read_only_note_preserves_items_in_another_document() {
        let ctx = egui::Context::default();
        let (mut source, mut text) = fixture();
        for _ in 0..5 {
            frame(&ctx, &mut source, &mut text, false, vec![]);
        }
        let id = egui::Id::unique("note");
        let index = text.chars().position(|c| c == '\u{100000}').expect("image");
        let mut state = egui::TextEdit::load_state(&ctx, id).expect("editor");
        state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(index), CCursor::new(index + 1))));
        state.store(&ctx, id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, &mut source, &mut text, false, vec![egui::Event::Copy]);
        let mut other = Notecard::default();
        let mut draft = String::new();
        let mut state = egui::TextEdit::load_state(&ctx, id).expect("editor after copy");
        state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(0))));
        state.store(&ctx, id);
        frame(&ctx, &mut other, &mut draft, true, vec![egui::Event::Paste("\u{100000}".into())]);
        assert_eq!(draft, "\u{100000}");
        assert_eq!(other.attachment('\u{100000}').expect("pasted image").item.asset_type, 0);
        assert!(!other.attachment('\u{100000}').expect("new item").saved);
        frame(&ctx, &mut other, &mut draft, true, vec![egui::Event::Paste("\u{100001}".into())]);
        assert_eq!(draft, "\u{100000}");
    }
}

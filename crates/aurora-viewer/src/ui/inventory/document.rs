//! LLPreviewNotecard document lifecycle (Firestorm, originally LGPL 2.1).
//! Keep the draft until its own upload is acknowledged, including on close.

use super::*;
use aurora_net::inventory::notecard::Notecard;

pub(super) struct Description {
    pub item: Uuid,
    pub original: String,
    pub text: String,
}

pub fn document_demo_events(ctx: &egui::Context, frame: u64, st: &mut InventoryUi) -> Vec<egui::Event> {
    let Ok(mode) = std::env::var("AURORA_DEMO_INVENTORY") else {
        return Vec::new();
    };
    if matches!(
        mode.as_str(),
        "note-markdown" | "note-source" | "note-classic" | "note-format" | "note-remember" | "note-lsl" | "note-lsl-markdown"
    ) {
        if frame < 600 && mode != "note-remember" {
            st.notecard_view = Some(match mode.as_str() {
                "note-markdown" | "note-lsl-markdown" => super::markdown::View::Markdown,
                "note-source" | "note-format" | "note-lsl" => super::markdown::View::Edit,
                _ => super::markdown::View::Classic,
            });
        }
        if frame == 610
            && let Some(item) = st.preview
        {
            // Saved auxiliary panels can overlap the scenario's document.
            ctx.move_to_top(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::unique(preview_window_id(st.ui_id, item, 7)),
            ));
        }
        if mode == "note-format"
            && let Some(item) = st.preview
        {
            let id = egui::Id::unique(("inventory_document_text", st.ui_id, item));
            if frame == 650
                && let Some(Ok(text)) = &st.preview_text
                && let Some(start) = text.find("- ").map(|at| at + 2)
            {
                let end = text[start..].find(char::is_whitespace).map_or(text.len(), |at| start + at);
                let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(text[..start].chars().count()),
                    egui::text::CCursor::new(text[..end].chars().count()),
                )));
                state.store(ctx, id);
                ctx.memory_mut(|memory| memory.request_focus(id));
            }
            let button = match frame {
                670 | 671 => Some("demo_format_bold"),
                710 | 711 => Some("demo_format_help"),
                _ => None,
            };
            if let Some(rect) = button.and_then(|key| ctx.data_mut(|data| data.get_temp::<egui::Rect>(id.with(key)))) {
                return vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed: matches!(frame, 670 | 710),
                        modifiers: egui::Modifiers::NONE,
                    },
                ];
            }
        }
        return Vec::new();
    }
    if !matches!(
        mode.as_str(),
        "note-edit"
            | "note-save"
            | "note-close"
            | "note-error"
            | "note-embedded"
            | "note-task-save"
            | "note-task-error"
            | "note-drop"
            | "note-image"
            | "note-landmark"
            | "note-copy"
            | "note-menu-right"
    ) {
        return Vec::new();
    }
    let Some(item) = st.preview else { return Vec::new() };
    let editor = egui::Id::unique(("inventory_document_text", st.ui_id, item));
    if mode == "note-drop" {
        let Some(rect) = ctx.data_mut(|data| data.get_temp::<egui::Rect>(editor.with("demo_rect"))) else {
            return Vec::new();
        };
        let pos = rect.min + egui::vec2(24.0, 12.0);
        if frame == 650 {
            egui::DragAndDrop::set_payload(ctx, InvDrag(Uuid::from_u128(8104)));
        }
        if matches!(frame, 650 | 651) {
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 650,
                    modifiers: Default::default(),
                },
            ];
        }
        if frame == 670 {
            ctx.memory_mut(|m| m.request_focus(editor));
        }
    }
    if matches!(mode.as_str(), "note-image" | "note-landmark" | "note-copy" | "note-menu-right") {
        if mode == "note-copy"
            && matches!(frame, 670 | 671)
            && let Some(rect) = ctx.data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("notecard_copy_confirm", st.ui_id))))
        {
            return vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed: frame == 670,
                    modifiers: Default::default(),
                },
            ];
        }
        let marker = if matches!(mode.as_str(), "note-image" | "note-menu-right") {
            '\u{100000}'
        } else if mode == "note-landmark" {
            '\u{100001}'
        } else {
            '\u{100002}'
        };
        if !matches!(frame, 650 | 651) {
            return Vec::new();
        }
        let Some(rect) = ctx.data_mut(|data| data.get_temp::<egui::Rect>(editor.with(("demo_attachment", marker)))) else {
            return Vec::new();
        };
        let pos = rect.center();
        return vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: if mode == "note-menu-right" {
                    egui::PointerButton::Secondary
                } else {
                    egui::PointerButton::Primary
                },
                pressed: frame.is_multiple_of(2),
                modifiers: Default::default(),
            },
        ];
    }
    let key = |key| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        },
    };
    if frame == 650 && mode != "note-drop" {
        ctx.memory_mut(|m| m.request_focus(egui::Id::unique(("inventory_document_text", st.ui_id, item))));
        return vec![
            key(egui::Key::End),
            egui::Event::Text(aurora_i18n::tr!("notecard-demo-edit").into()),
        ];
    }
    if frame == 670
        && matches!(
            mode.as_str(),
            "note-save" | "note-error" | "note-task-save" | "note-task-error" | "note-drop"
        )
    {
        return vec![key(egui::Key::S)];
    }
    if frame == 670 && mode == "note-close" {
        st.dialog = Some(EditDialog::Discard(item));
    }
    Vec::new()
}

impl InventoryUi {
    /// LLFloaterReg::showInstance: each item owns its document. Opening an
    /// existing item raises/restores it without replacing an unsaved draft.
    pub fn open_document(&mut self, ctx: &egui::Context, item: Uuid, kind: i32) -> bool {
        let existing = if self.preview == Some(item) {
            Some((self.ui_id, self.preview_text.is_none()))
        } else {
            self.windows
                .iter()
                .find(|w| w.state.preview == Some(item))
                .map(|w| (w.state.ui_id, w.state.preview_text.is_none()))
        };
        if let Some((editor, loading)) = existing {
            let id = preview_window_id(editor, item, kind);
            super::super::widgets::open_floater(ctx, &id, &mut true);
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(id)));
            return loading;
        }
        let state = Self::document(item);
        self.windows.push(InventoryWindow {
            id: state.ui_id,
            root: Uuid::nil(),
            additional: false,
            state: Box::new(state),
            open: false,
        });
        true
    }

    pub fn document(item: Uuid) -> Self {
        Self {
            ui_id: Uuid::new_v4(),
            preview: Some(item),
            ..Default::default()
        }
    }

    pub fn reset_document(&mut self) {
        self.preview_text = None;
        self.preview_can_edit = false;
        self.preview_source = None;
        self.notecard_view = Default::default();
        self.notecard_markdown = Default::default();
        self.attachment_preview = None;
        self.preview_original.clear();
        self.preview_description = None;
        self.preview_message.clear();
        self.preview_dirty = false;
        self.save_pending = false;
        self.content_save = None;
        self.close_after_save = false;
    }

    pub fn load_document(&mut self, kind: i32, can_edit: bool, result: &Result<Vec<u8>, String>) {
        // A late download (another window or a refresh) must not erase a draft.
        if self.preview_dirty || self.save_pending {
            return;
        }
        if self.preview_text.as_ref().is_some_and(Result::is_ok)
            && let Err(reason) = result
        {
            self.preview_message.clone_from(reason);
            return;
        }
        self.preview_source = None;
        self.preview_can_edit = false;
        let result = result.as_ref().map_err(Clone::clone).and_then(|data| {
            if kind == 7 {
                let note = Notecard::decode(data)?;
                let text = note.text.clone();
                self.preview_source = Some(note);
                self.preview_can_edit = can_edit;
                Ok(text)
            } else {
                let (text, editable) = crate::world::inventory::preview::text(kind, data)?;
                self.preview_can_edit = can_edit && editable;
                Ok(text)
            }
        });
        if let Ok(text) = &result {
            self.preview_original.clone_from(text);
        }
        self.preview_text = Some(result);
    }

    pub fn document_changed(&mut self) {
        self.preview_dirty = self
            .preview_text
            .as_ref()
            .is_some_and(|r| r.as_ref().is_ok_and(|t| t != &self.preview_original));
        self.preview_message.clear();
    }

    pub fn start_content_save(&mut self, request: Uuid, kind: i32, text: String, close: bool) -> Result<Vec<u8>, String> {
        if self.save_pending
            || !self.preview_can_edit
            || !self.preview_dirty
            || self.preview_text.as_ref().and_then(|r| r.as_ref().ok()) != Some(&text)
        {
            return Err(aurora_i18n::tr!("notecard-edit-denied").into());
        }
        let data = if kind == 7 {
            self.preview_source
                .as_ref()
                .ok_or_else(|| aurora_i18n::tr!("world-inventory-preview-invalid-notecard").to_owned())?
                .encode(&text)?
        } else {
            text.as_bytes().to_vec()
        };
        self.content_save = Some((request, data.clone(), text));
        self.save_pending = true;
        self.close_after_save = close;
        self.preview_message.clear();
        Ok(data)
    }

    pub fn finish_content_save(&mut self, request: Uuid, error: Option<&str>) -> bool {
        if self.content_save.as_ref().is_none_or(|(id, _, _)| *id != request) {
            return false;
        }
        let Some((_, data, saved_text)) = self.content_save.take() else {
            return false;
        };
        self.save_pending = false;
        if let Some(reason) = error {
            self.close_after_save = false;
            self.preview_message = aurora_i18n::tr!("notecard-save-failed", reason = reason);
            return true;
        }
        self.preview_original.clone_from(&saved_text);
        self.document_changed();
        if !self.preview_dirty
            && self.preview_source.is_some()
            && let Ok(note) = Notecard::decode(&data)
        {
            self.preview_original.clone_from(&note.text);
            self.preview_text = Some(Ok(note.text.clone()));
            self.preview_source = Some(note);
        }
        self.preview_message = aurora_i18n::tr!("app-document-saved").into();
        if self.close_after_save && !self.preview_dirty {
            self.preview = None;
            self.reset_document();
            self.dialog = None;
        }
        self.close_after_save = false;
        true
    }
}

pub(super) fn preview_window_id(editor: Uuid, item: Uuid, kind: i32) -> String {
    format!(
        "{}_{}_{}",
        if kind == 7 { "notecard_preview" } else { "inventory_preview" },
        editor,
        item
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cascade_notecards_keep_independent_drafts_and_restore_existing_items() {
        let ctx = egui::Context::default();
        let mut inv = Inventory::default();
        let items = [Uuid::from_u128(1), Uuid::from_u128(2)];
        inv.add_items(
            items
                .map(|id| aurora_net::inventory::InvItem {
                    id,
                    asset_type: 7,
                    name: format!("Note {id}"),
                    ..Default::default()
                })
                .to_vec(),
        );
        let mut state = InventoryUi::default();
        let mut frame = 0;
        for (n, item) in items.into_iter().enumerate() {
            assert!(state.open_document(&ctx, item, 7));
            state.windows[n].state.load_document(7, true, &Ok(b"Original".to_vec()));
            for _ in 0..8 {
                frame += 1;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1200.0))),
                        time: Some(frame as f64 / 60.0),
                        ..Default::default()
                    },
                    |ui| {
                        for w in &mut state.windows {
                            show_document(
                                ui.ctx(),
                                &crate::theme::Theme::default().palette(),
                                &Icons::default(),
                                &inv,
                                &mut w.state,
                                &inv,
                                &Default::default(),
                                &mut InventoryPreferences::default(),
                                &Default::default(),
                            );
                        }
                    },
                );
                output.textures_delta.clear();
            }
            state.windows[n].state.preview_text = Some(Ok(format!("Draft {n}")));
            state.windows[n].state.document_changed();
        }
        let ids: Vec<_> = state
            .windows
            .iter()
            .zip(items)
            .map(|(w, item)| preview_window_id(w.id, item, 7))
            .collect();
        let rects: Vec<_> = ids
            .iter()
            .map(|id| ctx.memory(|m| m.area_rect(egui::Id::unique(id)).expect("note window")))
            .collect();
        assert!(((rects[1].min - rects[0].min) - egui::Vec2::splat(32.0)).length() < 1.0);
        crate::ui::widgets::minimize_floater(&ctx, &ids[0]);
        assert!(!state.open_document(&ctx, items[0], 7));
        assert_eq!(state.windows.len(), 2);
        assert!(state.windows.iter().all(|w| w.root.is_nil() && !w.open && w.state.preview_dirty));
        assert_eq!(state.windows[0].state.preview_text, Some(Ok("Draft 0".into())));
        assert_eq!(state.windows[1].state.preview_text, Some(Ok("Draft 1".into())));
        assert_eq!(ctx.memory(|m| m.area_rect(egui::Id::unique(&ids[0]))), Some(rects[0]));
    }

    fn draft(text: &str) -> InventoryUi {
        let mut st = InventoryUi {
            preview: Some(Uuid::from_u128(1)),
            ..Default::default()
        };
        st.load_document(7, true, &Ok(Vec::new()));
        st.preview_text = Some(Ok(text.into()));
        st.document_changed();
        st
    }

    #[test]
    fn failed_save_keeps_the_draft_and_success_closes_only_after_acknowledgement() {
        let mut st = draft("Été 🐺");
        let request = Uuid::from_u128(2);
        st.start_content_save(request, 7, "Été 🐺".into(), true).expect("upload");
        assert!(st.preview.is_some());
        assert!(!st.finish_content_save(Uuid::from_u128(3), None));
        assert!(st.save_pending);
        assert!(st.finish_content_save(request, Some("HTTP 503")));
        assert!(st.preview_dirty && st.preview.is_some());
        assert_eq!(st.preview_text, Some(Ok("Été 🐺".into())));
        assert!(!st.save_pending);
        st.start_content_save(request, 7, "Été 🐺".into(), true).expect("retry");
        st.finish_content_save(request, None);
        assert!(st.preview.is_none());
        assert!(!st.preview_dirty);
    }

    #[test]
    fn undo_restores_pristine_and_refresh_preserves_unsaved_changes() {
        let mut st = draft("Texte modifié");
        st.load_document(7, true, &Ok(b"old content".to_vec()));
        assert_eq!(st.preview_text, Some(Ok("Texte modifié".into())));
        st.preview_text = Some(Ok(String::new()));
        st.document_changed();
        assert!(!st.preview_dirty);
    }

    #[test]
    fn saving_one_window_does_not_mark_a_second_draft_saved() {
        let mut a = draft("Première fenêtre");
        let mut b = draft("Autre brouillon");
        let request = Uuid::from_u128(2);
        a.start_content_save(request, 7, "Première fenêtre".into(), false).expect("save");
        assert!(!b.finish_content_save(request, None));
        a.finish_content_save(request, None);
        assert!(!a.preview_dirty && b.preview_dirty);
        assert_eq!(b.preview_text, Some(Ok("Autre brouillon".into())));
    }
}

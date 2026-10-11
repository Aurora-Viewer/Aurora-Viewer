//! LLPreviewNotecard document lifecycle (Firestorm, originally LGPL 2.1).
//! Keep the draft until its own upload is acknowledged, including on close.

use super::*;
use aurora_net::inventory::notecard::Notecard;

pub fn document_demo_events(ctx: &egui::Context, frame: u64, st: &mut InventoryUi) -> Vec<egui::Event> {
    let Ok(mode) = std::env::var("AURORA_DEMO_INVENTORY") else {
        return Vec::new();
    };
    if !matches!(
        mode.as_str(),
        "note-edit" | "note-save" | "note-close" | "note-error" | "note-embedded" | "note-task-save" | "note-task-error"
    ) {
        return Vec::new();
    }
    let Some(item) = st.preview else { return Vec::new() };
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
    if frame == 650 {
        ctx.memory_mut(|m| m.request_focus(egui::Id::new(("inventory_document_text", st.ui_id, item))));
        return vec![
            key(egui::Key::End),
            egui::Event::Text(aurora_i18n::tr!("notecard-demo-edit").into()),
        ];
    }
    if frame == 670 && matches!(mode.as_str(), "note-save" | "note-error" | "note-task-save" | "note-task-error") {
        return vec![key(egui::Key::S)];
    }
    if frame == 670 && mode == "note-close" {
        st.dialog = Some(EditDialog::Discard(item));
    }
    Vec::new()
}

impl InventoryUi {
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
        self.preview_original.clear();
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

#[cfg(test)]
mod tests {
    use super::*;

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

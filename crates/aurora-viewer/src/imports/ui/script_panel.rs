//! The inspector of a script row: its size and lines, and a read-only view of
//! the start of its source. Name, description and destination are the common
//! controls above it. Nothing here is a preview of how the script runs.

use super::*;

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, id: Uuid, draft: &script::Draft) {
    widgets::section_header(ui, p, "file-code-filled", aurora_i18n::tr!("import-script-source"));
    ui.label(
        RichText::new(aurora_i18n::tr!(
            "import-script-info",
            lines = draft.lines,
            size = size_label(draft.bytes())
        ))
        .color(p.muted),
    );
    ui.add_space(6.0);
    let (head, truncated) = draft.head();
    let mut shown = head;
    egui::ScrollArea::both()
        .id_salt(("import_script_source", id))
        .max_height(360.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            // a `&str` is a read-only text buffer: selectable and copyable, never edited
            ui.add(
                egui::TextEdit::multiline(&mut shown)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(8),
            );
        });
    if truncated {
        ui.label(
            RichText::new(aurora_i18n::tr!("import-script-truncated", lines = script::PREVIEW_LINES))
                .small()
                .color(p.muted),
        );
    }
}

/// Sizes are bounded by the grid's 1 MiB source limit.
fn size_label(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_in_bytes_then_kibibytes() {
        assert_eq!(size_label(0), "0 B");
        assert_eq!(size_label(1023), "1023 B");
        assert_eq!(size_label(1536), "1.5 KiB");
        assert_eq!(size_label(script::MAX_SOURCE), "1024.0 KiB");
    }
}

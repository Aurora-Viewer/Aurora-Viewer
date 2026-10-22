//! Cached waveform, synchronized playhead and non-destructive crop / gain.
use super::*;

#[cfg(test)]
mod tests;

pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Uuid,
    name: &str,
    source: &Sound,
    editor: &mut sound::Editor,
    limit: u32,
    actions: &mut Vec<Action>,
) {
    let duration = source.duration();
    if duration <= 0.0 {
        return;
    }
    widgets::section_header(ui, p, "file-music-filled", aurora_i18n::tr!("import-preview"));
    ui.label(aurora_i18n::tr!(
        "import-sound-info",
        duration = format!("{duration:.2}"),
        channels = source.source_channels,
        bits = source.source_bits
    ))
    .on_hover_text(aurora_i18n::tr!("import-sound-conversion"));
    let before = editor.options;
    let elapsed = editor.elapsed();
    let minimum = duration.min(0.001);
    widgets::surface(ui, p, |ui| {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 104.0), egui::Sense::click_and_drag());
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique(("sound_waveform", id)), rect));
        let start = rect.left() + rect.width() * editor.options.start / duration;
        let end = rect.left() + rect.width() * editor.options.end.unwrap_or(duration) / duration;
        let playhead = elapsed.map(|elapsed| editor.options.start + elapsed);
        let cursor = playhead.map(|time| rect.left() + rect.width() * time / duration);
        let bars = 160;
        for index in 0..bars {
            let peak_start = index * editor.peaks.len() / bars;
            let peak_end = ((index + 1) * editor.peaks.len() / bars)
                .max(peak_start + 1)
                .min(editor.peaks.len());
            let peak = editor
                .peaks
                .get(peak_start..peak_end)
                .unwrap_or(&[])
                .iter()
                .copied()
                .fold(0.0_f32, f32::max);
            let x = rect.left() + (index as f32 + 0.5) * rect.width() / bars as f32;
            let amplified = peak * editor.options.gain;
            let height = (amplified.min(1.0) * rect.height() * 0.46).max(1.0);
            let color = if x < start || x > end {
                p.muted_dim
            } else if amplified > 1.0 {
                p.warn
            } else if cursor.is_some_and(|cursor| x <= cursor) {
                p.teal
            } else {
                p.violet_light
            };
            ui.painter().line_segment(
                [egui::pos2(x, rect.center().y - height), egui::pos2(x, rect.center().y + height)],
                egui::Stroke::new(2.0, color),
            );
        }
        for x in [start, end] {
            ui.painter().vline(x, rect.y_range(), egui::Stroke::new(2.0, p.violet));
        }
        if let Some(seconds) = editor.options.segment {
            let count = editor.options.pieces(source).unwrap_or(1);
            for index in 1..count {
                let time = editor.options.start + index as f32 * seconds;
                let x = rect.left() + rect.width() * time / duration;
                ui.painter().vline(x, rect.y_range(), egui::Stroke::new(1.0, p.muted));
            }
        }
        if let Some(cursor) = cursor {
            ui.painter().vline(cursor, rect.y_range(), egui::Stroke::new(1.5, p.ink));
        }
        let handle = egui::Id::unique(("sound_crop_handle", id));
        if (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            if response.clicked() || response.drag_started() {
                ui.ctx()
                    .data_mut(|data| data.insert_temp(handle, (pointer.x - start).abs() <= (pointer.x - end).abs()));
            }
            let time = ((pointer.x - rect.left()) / rect.width() * duration).clamp(0.0, duration);
            if ui.ctx().data(|data| data.get_temp::<bool>(handle)).unwrap_or(true) {
                editor.options.start = time.min(editor.options.end.unwrap_or(duration) - minimum);
            } else {
                editor.options.end = Some(time.max(editor.options.start + minimum));
            }
        }
        response
            .on_hover_text(aurora_i18n::tr!("import-sound-crop-help"))
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    });
    ui.horizontal(|ui| {
        if action_button_if(
            ui,
            p,
            "player-play-filled",
            aurora_i18n::tr!("common-play"),
            ActionKind::Flat,
            Vec2::new(0.0, 26.0),
            !editor.preparing,
        )
        .clicked()
        {
            actions.push(Action::Play(id, true));
        }
        if action_button_if(
            ui,
            p,
            "player-stop-filled",
            aurora_i18n::tr!("common-stop"),
            ActionKind::Flat,
            Vec2::new(0.0, 26.0),
            elapsed.is_some() || editor.preparing,
        )
        .clicked()
        {
            actions.push(Action::Play(id, false));
        }
        if editor.preparing {
            ui.spinner();
        }
        ui.label(aurora_i18n::tr!(
            "import-sound-selection",
            duration = format!("{:.3}", editor.options.duration(source))
        ));
    });
    let mut commit_gain = false;
    let mut dragging_gain = false;
    ui.horizontal(|ui| {
        ui.label(aurora_i18n::tr!("import-sound-volume"));
        let mut volume = editor.options.gain * 100.0;
        let slider = ui.add(egui::Slider::new(&mut volume, 0.0..=sound::MAX_GAIN * 100.0).suffix(" %"));
        dragging_gain = slider.dragged();
        commit_gain = slider.drag_stopped();
        if slider.changed() {
            editor.options.gain = volume / 100.0;
        }
    });
    ui.horizontal(|ui| {
        ui.label(aurora_i18n::tr!("import-sound-start"));
        ui.add(
            egui::DragValue::new(&mut editor.options.start)
                .range(0.0..=editor.options.end.unwrap_or(duration) - minimum)
                .speed(0.01)
                .fixed_decimals(3)
                .suffix(" s"),
        );
        ui.label(aurora_i18n::tr!("import-sound-end"));
        let mut end = editor.options.end.unwrap_or(duration);
        if ui
            .add(
                egui::DragValue::new(&mut end)
                    .range(editor.options.start + minimum..=duration)
                    .speed(0.01)
                    .fixed_decimals(3)
                    .suffix(" s"),
            )
            .changed()
        {
            editor.options.end = Some(end);
        }
        if widgets::action_button(
            ui,
            p,
            "refresh",
            aurora_i18n::tr!("common-reset"),
            ActionKind::Flat,
            Vec2::new(0.0, 26.0),
        )
        .clicked()
        {
            editor.options = Default::default();
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let mut split = editor.options.segment.is_some();
        if checkbox(ui, p, &mut split, aurora_i18n::tr!("import-sound-split")).changed() {
            editor.options.segment = split.then_some((limit as f32).min(editor.options.duration(source)).max(0.1));
        }
        if let Some(seconds) = &mut editor.options.segment {
            ui.label(aurora_i18n::tr!("import-sound-piece-max"));
            ui.add(
                egui::DragValue::new(seconds)
                    .range(0.1..=limit as f32)
                    .speed(0.1)
                    .max_decimals(3)
                    .suffix(" s"),
            );
        }
    });
    if editor.options.segment.is_some() {
        let count = editor.options.pieces(source).unwrap_or(1);
        ui.label(
            RichText::new(aurora_i18n::tr!(
                "import-sound-pieces",
                count = count,
                first = sound::piece_name(name, 0),
                last = sound::piece_name(name, count - 1)
            ))
            .small()
            .color(p.muted),
        );
    }
    if !editor.options.importable(source, limit) {
        ui.label(
            RichText::new(aurora_i18n::tr!("import-sound-too-long", limit = limit))
                .small()
                .color(p.warn),
        );
    }
    let peaks = &editor.peaks;
    let from = (editor.options.start / duration * peaks.len() as f32).floor() as usize;
    let to = (editor.options.end.unwrap_or(duration) / duration * peaks.len() as f32).ceil() as usize;
    if peaks
        .get(from..to.min(peaks.len()))
        .is_some_and(|peaks| peaks.iter().any(|peak| peak * editor.options.gain > 1.0))
    {
        ui.label(RichText::new(aurora_i18n::tr!("import-sound-clipping")).small().color(p.warn));
    }
    if before.start != editor.options.start || before.end != editor.options.end {
        actions.push(Action::Play(id, false));
    }
    if commit_gain || (before.gain != editor.options.gain && !dragging_gain) {
        actions.push(Action::SoundGain(id));
    }
    if elapsed.is_some() || editor.preparing {
        ui.ctx().request_repaint();
    }
}

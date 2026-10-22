use super::*;

#[test]
fn waveform_crop_and_reset_dispatch_the_audio_controls_without_changing_pcm() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let source = Sound {
        source_channels: 1,
        source_bits: 16,
        mono: vec![0.5; 44100],
    };
    let id = Uuid::from_u128(50);
    let mut editor = sound::Editor::new(Some(&Source::Sound(Arc::new(source.clone()))));
    let frame = |editor: &mut sound::Editor, events| {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(600.0, 500.0))),
                events,
                ..Default::default()
            },
            |ui| {
                ui.set_width(540.0);
                show(ui, &theme.palette(), id, "Sound", &source, editor, 30, &mut actions);
            },
        );
        output.textures_delta.clear();
        (output, actions)
    };
    for _ in 0..3 {
        drop(frame(&mut editor, Vec::new()));
    }
    let wave = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("sound_waveform", id))))
        .expect("waveform");
    let crop = wave.left_center() + Vec2::new(wave.width() * 0.25, 0.0);
    let click = |pos, pressed| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    let mut actions = Vec::new();
    for pressed in [true, false] {
        actions.extend(frame(&mut editor, click(crop, pressed)).1);
    }
    assert!((editor.options.start - 0.25).abs() < 0.001);
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, Action::Play(entry, false) if *entry == id))
    );
    editor.options.gain = 0.4;
    let (output, _) = frame(&mut editor, Vec::new());
    let reset = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("common-reset") => Some(text.pos + text.galley.size() * 0.5),
            _ => None,
        })
        .expect("reset sound edit");
    actions.clear();
    for pressed in [true, false] {
        actions.extend(frame(&mut editor, click(reset, pressed)).1);
    }
    assert_eq!(editor.options, sound::Options::default());
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, Action::SoundGain(entry) if *entry == id))
    );
    assert_eq!(source.mono, vec![0.5; 44100], "the original PCM is intact");
    let (output, _) = frame(&mut editor, Vec::new());
    let split = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("import-sound-split") => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("split option");
    for pressed in [true, false] {
        drop(frame(&mut editor, click(split, pressed)));
    }
    assert_eq!(editor.options.segment, Some(1.0));
    editor.options.gain = 2.5;
    let (output, _) = frame(&mut editor, Vec::new());
    assert!(
        output.shapes.iter().any(
            |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("import-sound-clipping"))
        )
    );
}

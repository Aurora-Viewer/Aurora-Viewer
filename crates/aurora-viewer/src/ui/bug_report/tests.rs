use super::*;

#[test]
fn interface_toggle_recaptures_only_camera_images_in_both_directions() {
    let mut state = BugReportUi::default();
    state.capture_option_changed(true);
    assert_eq!(state.capture_requested, None);
    state.image_origin = Some(ImageOrigin::File);
    state.include_ui = true;
    state.capture_option_changed(true);
    assert_eq!(state.capture_requested, None);
    state.image_origin = Some(ImageOrigin::Capture(false));
    state.capture_option_changed(false);
    assert_eq!(state.capture_requested, None);
    state.capture_option_changed(true);
    assert_eq!(state.capture_requested.take(), Some(true));
    state.image_origin = Some(ImageOrigin::Capture(true));
    state.include_ui = false;
    state.capture_option_changed(true);
    assert_eq!(state.capture_requested.take(), Some(false));
    state.capture_pending = true;
    state.capture_option_changed(true);
    assert_eq!(state.capture_requested, None);
}

#[test]
fn recapture_keeps_the_picture_excluded_and_replaces_its_pixels() {
    let mut state = BugReportUi {
        image_origin: Some(ImageOrigin::Capture(false)),
        include_image: false,
        include_ui: true,
        ..Default::default()
    };
    state.captured(1, 1, vec![21, 42, 63, 255]);
    let job = state.image_job.take().unwrap();
    assert!(!job.include);
    assert!(job.origin == ImageOrigin::Capture(true));
    let result = job.rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    let (tx, rx) = crossbeam_channel::bounded(1);
    tx.send(result).unwrap();
    state.image_job = Some(ImageJob { rx, ..job });
    state.poll(&egui::Context::default());
    assert!(!state.include_image);
    assert_eq!(state.image.as_ref().unwrap().preview.pixels[0], egui::Color32::from_rgb(21, 42, 63));
}

#[test]
fn language_changes_refresh_demo_labels_without_overwriting_edits_or_selections() {
    let old: [String; 4] =
        ["bug-demo-title", "bug-demo-steps", "bug-demo-expected", "bug-demo-actual"].map(|key| aurora_i18n::text_in("fr", key).into());
    let mut state = BugReportUi {
        report: Report {
            title: "My own title".into(),
            steps: old[1].clone(),
            expected: old[2].clone(),
            actual: old[3].clone(),
            fields: vec![Field::new("bug-language", "Viewer language", "fr", false)],
            ..Default::default()
        },
        language: "fr",
        demo_text: Some(old),
        ..Default::default()
    };
    state.refresh_language("en");
    assert_eq!(state.report.title, "My own title");
    assert_eq!(state.report.expected, aurora_i18n::text_in("en", "bug-demo-expected"));
    assert_eq!(state.report.fields[0].value, "en");
    assert!(!state.report.fields[0].included);
    state.report.actual = "My own description".into();
    state.refresh_language("fr");
    assert_eq!(state.report.actual, "My own description");
    assert_eq!(state.report.expected, aurora_i18n::text_in("fr", "bug-demo-expected"));
}

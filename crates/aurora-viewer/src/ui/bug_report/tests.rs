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

fn draw_report(ctx: &egui::Context, state: &mut BugReportUi) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(900.0, 900.0))),
            ..Default::default()
        },
        |ui| {
            state.show(
                ui.ctx(),
                &crate::theme::Theme::default().palette(),
                &super::super::icons::Icons::default(),
                true,
            )
        },
    );
    output.textures_delta.clear();
}

#[test]
fn new_report_opens_description_even_without_automatic_fields() {
    let mut state = BugReportUi {
        tab: 2,
        ..Default::default()
    };
    state.begin(vec![], false);
    assert_eq!(state.tab, 0);
    assert!(state.delivery == Delivery::Direct);
    state.report.title = "Draft with no automatic fields".into();
    state.tab = 1;
    state.open = false;
    state.begin(vec![], false);
    assert_eq!(state.report.title, "Draft with no automatic fields");
    assert_eq!(state.tab, 1);
}

#[test]
fn closing_and_reopening_preserves_draft_image_selection_and_authorization_step() {
    let ctx = egui::Context::default();
    let mut state = BugReportUi::default();
    state.begin(vec![Field::new("bug-location", "Location", "ORIGINAL_LOCATION", false)], false);
    state.diagnostics = None;
    state.report.title = "My draft".into();
    state.report.actual = "My description".into();
    state.report.include_excerpt = true;
    state.tab = 2;
    state.delivery = Delivery::Direct;
    state.github.user_code = Some("EXAMPLE-CODE".into());
    state.image = Some(files::capture(1, 1, vec![21, 42, 63, 255]).unwrap());
    state.image_origin = Some(ImageOrigin::Capture(true));
    state.include_ui = true;
    state.include_image = false;
    state.open = false;
    // Closed floaters are still polled every frame. They must not cancel the
    // authorization or discard the image and draft while hidden.
    draw_report(&ctx, &mut state);
    state.begin(vec![Field::new("bug-location", "Location", "NEW_LOCATION", true)], false);
    assert!(state.open);
    assert_eq!(state.tab, 2);
    assert!(state.delivery == Delivery::Direct);
    assert_eq!(state.github.user_code.as_deref(), Some("EXAMPLE-CODE"));
    assert_eq!(state.report.title, "My draft");
    assert_eq!(state.report.actual, "My description");
    assert!(state.report.include_excerpt);
    assert_eq!(state.report.fields[0].value, "ORIGINAL_LOCATION");
    assert!(!state.report.fields[0].included);
    assert!(!state.include_image);
    assert!(state.include_ui);
    assert!(state.image.is_some());
}

#[test]
fn failed_delivery_keeps_draft_but_confirmed_issue_starts_a_fresh_description() {
    let mut state = BugReportUi::default();
    state.begin(vec![Field::new("bug-location", "Location", "OLD_LOCATION", false)], false);
    state.report.title = "Unsent report".into();
    state.tab = 1;
    state.github.failure = Some(github::Failure::Ambiguous);
    state.open = false;
    state.begin(vec![], false);
    assert_eq!(state.report.title, "Unsent report");
    assert_eq!(state.tab, 1);
    state.github.created = Some(42);
    state.open = false;
    state.begin(vec![Field::new("bug-location", "Location", "NEW_LOCATION", true)], false);
    assert!(state.open);
    assert_eq!(state.tab, 0);
    assert!(state.delivery == Delivery::Direct);
    assert!(state.report.title.is_empty());
    assert_eq!(state.report.fields[0].value, "NEW_LOCATION");
    assert!(state.report.fields[0].included);
    assert_eq!(state.github.created, None);
    assert_eq!(state.github.failure, None);
}

#[test]
fn browser_draft_is_only_reset_after_explicit_publication_confirmation() {
    let mut state = BugReportUi::default();
    state.begin(vec![], false);
    state.report.title = "Browser draft".into();
    state.tab = 2;
    state.delivery = Delivery::Browser;
    state.confirm_browser_publication();
    assert_eq!(state.report.title, "Browser draft");
    state.browser_opened = true;
    state.open = false;
    state.begin(vec![], false);
    assert_eq!(state.report.title, "Browser draft");
    assert_eq!(state.tab, 2);
    assert!(state.delivery == Delivery::Browser);
    state.confirm_browser_publication();
    state.begin(vec![], false);
    assert_eq!(state.tab, 0);
    assert!(state.delivery == Delivery::Direct);
    assert!(state.report.title.is_empty());
    assert!(!state.browser_opened);
}

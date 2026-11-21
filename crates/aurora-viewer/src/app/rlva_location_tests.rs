//! RLVa `@showloc` in the app's own windows: the photo's name, the About
//! window's Location card and the Monitor's regions, as the Legacy Viewer
//! names the place there (LLAgentUI::buildLocationString, llagentui.cpp;
//! LLAppViewer::getViewerInfo, llappviewer.cpp).

use crate::world::rlva_location_tests::showloc_world;

/// LLSnapshotLivePreview::saveTexture names a photo after
/// buildLocationString(LOCATION_FORMAT_FULL), which @showloc turns into
/// "(Hidden parcel), (Hidden region)" with no position.
#[test]
fn showloc_names_a_photo_after_the_hidden_place() {
    let mut w = showloc_world();
    let place = super::snapshot::photo_place(&w);
    let hidden = format!(
        "{}, {}",
        aurora_i18n::tr!("rlva-hidden-parcel"),
        aurora_i18n::tr!("rlva-hidden-region")
    );
    assert!(place.starts_with(&hidden), "{place}");
    assert!(!place.contains(&w.region_name()), "{place}");
    assert!(
        place == hidden || place.strip_prefix(&hidden).is_some_and(|rest| rest.starts_with(" - ")),
        "no position: {place}"
    );
    w.rlva_console_input("@showloc=y");
    let place = super::snapshot::photo_place(&w);
    assert!(place.contains(&w.region_name()) && place.contains(" ("), "{place}");
}

/// AboutPositionRLVShowLoc: "You are in (Hidden region)", no position.
#[test]
fn showloc_about_names_the_hidden_region_without_a_position() {
    use crate::ui::about::{Row, Value};
    let at = glam::Vec3::new(1.0, 2.0, 3.0);
    let rows = super::about::place_rows(true, "Aurora Demo", at);
    assert_eq!(rows, vec![Row::key("about-region", "rlva-hidden-region")]);
    let rows = super::about::place_rows(false, "Aurora Demo", at);
    assert_eq!(rows[0].value, Value::Text("Aurora Demo".into()));
    assert_eq!(rows.len(), 2);
}

/// The Monitor names the agent's simulator and those it has a circuit with
/// "(Hidden region)" under @showloc.
#[test]
fn showloc_hides_the_monitor_regions() {
    let mut w = showloc_world();
    let main = w.main_region.expect("arrived");
    w.estate.stats = Some((main, std::sync::Arc::new(Default::default())));
    let hidden = aurora_i18n::tr!("rlva-hidden-region");
    assert_eq!(super::monitor::region_health(&w).map(|r| r.name).as_deref(), Some(hidden));
    assert_eq!(super::monitor::link_name(&w, Some(main)), hidden);
    w.rlva_console_input("@showloc=y");
    assert_eq!(super::monitor::link_name(&w, Some(main)), w.region_name());
    assert_eq!(super::monitor::region_health(&w).map(|r| r.name), Some(w.region_name()));
}

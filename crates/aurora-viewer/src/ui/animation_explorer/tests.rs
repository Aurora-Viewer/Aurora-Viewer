use super::*;
use std::time::Duration;

fn row(id: u64, name: &str, own: bool, playing: bool, ago: u64, priority: Option<i32>) -> Row {
    Row {
        id,
        animation: Uuid::from_u128(u128::from(id)),
        source: Uuid::from_u128(1000 + u128::from(id)),
        name: name.into(),
        own,
        playing,
        started: Instant::now() - Duration::from_secs(ago),
        priority,
    }
}

fn ids(explorer: &AnimationExplorer) -> Vec<u64> {
    explorer.visible().into_iter().map(|i| explorer.rows[i].id).collect()
}

#[test]
fn newest_first_and_the_check_box_hides_our_own_motions() {
    let mut explorer = AnimationExplorer {
        rows: vec![
            row(3, "Pose ball", false, true, 1, Some(4)),
            row(2, "Me", true, true, 5, Some(0)),
            row(1, "Dance HUD", false, false, 30, None),
        ],
        ..Default::default()
    };
    assert_eq!(ids(&explorer), [3, 2, 1]);
    explorer.no_owned = true;
    assert_eq!(ids(&explorer), [3, 1]);
    explorer.sort = Some((Column::PlayedBy, false));
    assert_eq!(ids(&explorer), [1, 3]);
    explorer.sort = Some((Column::Priority, true));
    assert_eq!(ids(&explorer), [3, 1], "unknown priorities sort below known ones");
    explorer.no_owned = false;
    explorer.selected = Some(2);
    assert_eq!(explorer.selected_row().map(|r| r.name.as_str()), Some("Me"));
}

#[test]
fn played_says_still_playing_or_seconds_since_the_start() {
    let mut r = row(1, "Pose ball", false, true, 0, Some(3));
    let now = r.started + Duration::from_secs(42);
    assert_eq!(played_text(&r, now), aurora_i18n::tr!("animation-explorer-still-playing"));
    r.playing = false;
    assert!(played_text(&r, now).contains("42"));
    assert_eq!(priority_text(&r), "3");
    r.priority = None;
    assert_eq!(priority_text(&r), aurora_i18n::tr!("animation-explorer-unknown-priority"));
}

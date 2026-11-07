//! Parcel radio intent survives the unknown parcel during a crossing.
//! Legacy Viewer: LLStatusBar::toggleStream and
//! LLViewerParcelMgr::optionallyStartMusic keep the user's radio toggle;
//! LLViewerAudio avoids restarting an unchanged URL.

/// `None` means the next parcel has not arrived yet, not an empty station.
pub(super) fn update_parcel(
    previous: &mut String,
    playing: &mut bool,
    manual: &mut Option<String>,
    parcel_url: Option<&str>,
    autoplay: bool,
) {
    let Some(url) = parcel_url else { return };
    let url = url.trim();
    if url == previous.as_str() {
        return;
    }
    *previous = url.to_owned();
    *manual = None;
    if !url.is_empty() && autoplay {
        *playing = true;
    }
}

/// Bounded session history: retries and revisits never repeat a station notice.
/// Once the history is full, stop notices rather than evicting/repeating them.
pub(super) fn first_error_notice(seen: &mut std::collections::HashSet<String>, url: &str) -> bool {
    !url.is_empty() && seen.len() < 256 && seen.insert(url.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parcel_music_error_notice_is_once_per_url_and_bounded() {
        let mut seen = std::collections::HashSet::new();
        assert!(first_error_notice(&mut seen, "https://radio.example/a"));
        assert!(!first_error_notice(&mut seen, "https://radio.example/a"));
        assert!(!first_error_notice(&mut seen, ""));
        for i in 0..300 {
            first_error_notice(&mut seen, &format!("https://radio.example/{i}"));
        }
        assert_eq!(seen.len(), 256);
        assert!(!first_error_notice(&mut seen, "https://radio.example/a"));
    }

    #[test]
    fn parcel_music_crossing_keeps_intent_and_same_stream() {
        let mut previous = "https://radio.example/stream".to_owned();
        let mut playing = true;
        let mut manual = None;
        let mut stream = previous.clone();
        let mut changes = 0;
        for parcel in [None, Some("https://radio.example/stream")] {
            update_parcel(&mut previous, &mut playing, &mut manual, parcel, false);
            let want = if playing { previous.clone() } else { String::new() };
            if want != stream {
                changes += 1;
                stream = want;
            }
        }
        assert!(playing);
        assert_eq!(changes, 0, "no stop/play pair while the parcel is unknown");
    }

    #[test]
    fn parcel_music_empty_station_stops_audio_but_keeps_radio_intent() {
        let mut previous = "https://radio.example/a".to_owned();
        let mut playing = true;
        let mut manual = None;
        update_parcel(&mut previous, &mut playing, &mut manual, Some(""), false);
        assert!(previous.is_empty());
        assert!(playing);
        update_parcel(&mut previous, &mut playing, &mut manual, Some("https://radio.example/b"), false);
        assert!(playing);
        playing = false;
        update_parcel(&mut previous, &mut playing, &mut manual, None, true);
        update_parcel(&mut previous, &mut playing, &mut manual, Some("https://radio.example/b"), true);
        assert!(!playing, "autoplay must not undo an explicit stop on the same station");
    }

    #[test]
    fn parcel_music_manual_station_survives_unknown_parcel() {
        let mut previous = "https://radio.example/a".to_owned();
        let mut playing = true;
        let mut manual = Some("https://radio.example/manual".to_owned());
        update_parcel(&mut previous, &mut playing, &mut manual, None, false);
        assert!(manual.is_some());
        update_parcel(&mut previous, &mut playing, &mut manual, Some("https://radio.example/b"), false);
        assert!(manual.is_none());
    }
}

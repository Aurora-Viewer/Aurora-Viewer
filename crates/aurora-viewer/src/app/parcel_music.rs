//! Radio intent is independent of parcel discovery and of the audio connection.
//!
//! LLViewerParcelMgr::optionallyStartMusic and LLViewerAudio::onTeleportStarted
//! keep an enabled radio through a teleport, without restarting an unchanged URL.

#[derive(Default)]
pub(super) struct Radio {
    requested: bool,
    parcel: Option<String>,
    manual: Option<String>,
}

impl Radio {
    pub fn enter(&mut self, requested: bool) {
        *self = Self {
            requested,
            ..Self::default()
        };
    }

    pub fn requested(&self) -> bool {
        self.requested
    }

    pub fn toggle(&mut self) {
        self.requested = !self.requested;
    }

    pub fn play(&mut self, url: String) {
        self.manual = Some(url);
        self.requested = true;
    }

    pub fn update(&mut self, parcel: Option<&str>, autoplay: bool) {
        let Some(url) = parcel.map(str::trim) else { return };
        if self.parcel.as_deref() == Some(url) {
            return;
        }
        self.parcel = Some(url.to_owned());
        self.manual = None;
        if autoplay && !self.station().is_empty() {
            self.requested = true;
        }
    }

    /// Keep the existing HTTP(S)-only policy; no protocol rewrite or URL is saved.
    pub fn station(&self) -> &str {
        let url = self.manual.as_deref().or(self.parcel.as_deref()).unwrap_or("").trim();
        if url.starts_with("http://") || url.starts_with("https://") {
            url
        } else {
            ""
        }
    }

    pub fn wanted(&self, in_world: bool, channel_enabled: bool) -> &str {
        if in_world && channel_enabled && self.requested {
            self.station()
        } else {
            ""
        }
    }

    /// The only target the app may submit to the engine. A disabled channel
    /// neither connects nor asks for permission; an unknown site never loads
    /// before consent, including when restoring an enabled radio after login.
    pub fn target(
        &self,
        in_world: bool,
        channel_enabled: bool,
        filter: Option<&mut crate::media::filter::MediaFilter>,
    ) -> (&str, crate::media::filter::Verdict) {
        use crate::media::filter::{Source, Verdict};
        let url = self.wanted(in_world, channel_enabled);
        let verdict = if let Some(filter) = filter.filter(|_| !url.is_empty()) {
            filter.check(url, Source::Music)
        } else {
            Verdict::Allow
        };
        (if verdict == Verdict::Allow { url } else { "" }, verdict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parcel_music_resuming_and_enabling_never_bypass_media_filter_consent() {
        use crate::media::filter::{Action, Edit, MediaFilter, Remember, Verdict};
        let mut radio = Radio::default();
        radio.enter(true);
        radio.update(Some("https://radio.example/live"), false);
        let mut filter = MediaFilter::default();
        assert_eq!(radio.target(true, false, Some(&mut filter)), ("", Verdict::Allow));
        assert!(filter.question().is_none());
        assert_eq!(radio.target(true, true, Some(&mut filter)), ("", Verdict::Ask));
        filter.answer("https://radio.example/live", false, Remember::Once);
        assert_eq!(radio.target(true, true, Some(&mut filter)), ("", Verdict::Deny));
        assert!(radio.requested(), "refusing one site does not erase the resident's radio choice");
        filter.apply(Edit::Set("radio.example".into(), Action::Allow));
        assert_eq!(
            radio.target(true, true, Some(&mut filter)),
            ("https://radio.example/live", Verdict::Allow)
        );
        filter.apply(Edit::Set("radio.example".into(), Action::Deny));
        assert_eq!(radio.target(true, true, Some(&mut filter)), ("", Verdict::Deny));
        assert_eq!(radio.target(false, true, Some(&mut filter)), ("", Verdict::Allow));
        radio.enter(true);
        let mut next_session = MediaFilter::default();
        radio.update(Some("https://radio.example/live"), false);
        assert_eq!(radio.target(true, true, Some(&mut next_session)), ("", Verdict::Ask));
    }

    #[test]
    fn parcel_music_crossings_preserve_the_station_until_the_parcel_is_known() {
        let mut radio = Radio::default();
        radio.enter(true);
        radio.update(Some("https://radio.example/a"), false);
        let before = radio.wanted(true, true).to_owned();
        radio.update(None, false);
        assert_eq!(radio.wanted(true, true), before);
        radio.update(Some("https://radio.example/a"), false);
        assert_eq!(radio.wanted(true, true), before);
        radio.update(Some(""), false);
        assert_eq!(radio.wanted(true, true), "");
        assert!(radio.requested());
        radio.update(Some("https://radio.example/b"), false);
        assert_eq!(radio.wanted(true, true), "https://radio.example/b");
    }

    #[test]
    fn parcel_music_an_explicit_stop_survives_unknown_parcels_and_same_url_updates() {
        let mut radio = Radio::default();
        radio.update(Some("https://radio.example/a"), true);
        radio.toggle();
        radio.update(None, true);
        radio.update(Some("https://radio.example/a"), true);
        assert!(!radio.requested());
        radio.update(Some("https://radio.example/b"), true);
        assert!(radio.requested(), "autoplay applies to a newly discovered station");
    }

    #[test]
    fn parcel_music_disabled_channel_and_logout_never_request_a_connection() {
        let mut radio = Radio::default();
        radio.enter(true);
        radio.update(Some("https://radio.example/a"), false);
        assert_eq!(radio.wanted(true, false), "");
        assert_eq!(radio.wanted(false, true), "");
        radio.enter(false);
        assert_eq!(radio.station(), "");
        assert!(!radio.requested());
    }

    #[test]
    fn parcel_music_manual_urls_are_session_local_and_never_rewrite_https() {
        let mut radio = Radio::default();
        radio.update(Some("https://radio.example/a"), false);
        radio.play("https://radio.example:8443/private".into());
        radio.update(None, false);
        assert_eq!(radio.wanted(true, true), "https://radio.example:8443/private");
        radio.update(Some("https://radio.example/b"), false);
        assert_eq!(radio.station(), "https://radio.example/b");
        for url in ["file:///private", "javascript:alert(1)", "ftp://radio.example/a"] {
            radio.play(url.into());
            assert_eq!(radio.wanted(true, true), "");
        }
        radio.enter(true);
        assert_eq!(radio.station(), "", "a new session must await its own parcel");
    }

    #[test]
    fn parcel_music_relogin_restores_only_the_same_accounts_explicit_choice() {
        let first = uuid::Uuid::from_u128(1);
        let second = uuid::Uuid::from_u128(2);
        let mut settings = crate::settings::Settings::default();
        let mut radio = Radio::default();
        radio.enter(false);
        radio.play("https://radio.example/private-session-url".into());
        settings.parcel_music_accounts.insert(first);
        let saved = serde_json::to_string(&settings.to_saved()).expect("synthetic settings");
        assert!(!saved.contains("private-session-url"));
        let mut loaded: crate::settings::Settings = serde_json::from_str(&saved).expect("settings round trip");
        radio.enter(loaded.parcel_music_accounts.contains(&second));
        radio.update(Some("https://radio.example/other-account"), false);
        assert_eq!(radio.wanted(true, true), "", "another account never inherits radio intent");
        radio.enter(loaded.parcel_music_accounts.contains(&first));
        assert_eq!(radio.station(), "", "never reconnect to the previous session's URL");
        radio.update(Some("https://radio.example/new-session"), false);
        assert_eq!(radio.wanted(true, true), "https://radio.example/new-session");
        radio.toggle();
        loaded.parcel_music_accounts.remove(&first);
        let saved = serde_json::to_string(&loaded.to_saved()).expect("synthetic settings");
        let loaded: crate::settings::Settings = serde_json::from_str(&saved).expect("settings round trip");
        radio.enter(loaded.parcel_music_accounts.contains(&first));
        radio.update(Some("https://radio.example/new-session"), false);
        assert_eq!(radio.wanted(true, true), "", "an explicit stop is remembered too");
        let old: crate::settings::Settings = serde_json::from_str("{}").expect("old settings");
        assert!(old.parcel_music_accounts.is_empty());
        assert!(!old.audio.music_autoplay, "existing defaults must not start music on their own");
    }
}

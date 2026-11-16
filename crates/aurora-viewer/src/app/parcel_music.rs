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

/// Port of LLViewerAudio's stream fading (indra/newview/llvieweraudio.cpp,
/// FSFadeAudioStream, FSAudioMusicFadeIn / FSAudioMusicFadeOut): a station
/// that changes fades out, then the next one fades in. Stopping by hand,
/// turning the Music channel off or logging out cuts at once
/// (stopInternetStreamWithAutoFade); starting a station, by hand or with
/// the parcel, fades in (startInternetStreamWithAutoFade).
#[derive(Default)]
pub(super) struct Fade {
    state: FadeState,
    /// What the engine plays (getInternetStreamURL).
    playing: String,
    /// The station asked for last (mNextStreamURI).
    next: String,
    /// When the fade under way started, and how long it lasts.
    started: Option<(std::time::Instant, f32)>,
}

#[derive(Default, Clone, Copy, PartialEq, Debug)]
enum FadeState {
    #[default]
    Idle,
    Out,
    In,
}

/// The fade's settings: on, fade in and fade out in seconds.
#[derive(Clone, Copy)]
pub(super) struct FadeTimes {
    pub on: bool,
    pub fade_in: f32,
    pub fade_out: f32,
}

/// What the engine is told to do this frame.
#[derive(Debug, PartialEq)]
pub(super) enum StreamChange {
    Play(String),
    Stop,
}

/// AUDIO_MUSIC_MINIMUM_FADE_TIME: no division by zero for a fade set to 0.
const MINIMUM_FADE: f32 = 0.01;

impl Fade {
    /// The URL the engine plays.
    #[cfg(test)]
    pub fn playing(&self) -> &str {
        &self.playing
    }

    /// One frame: `want` the station to play ("" none); `smooth` false when
    /// the change comes from a stop by hand, a channel turned off or a
    /// logout, which never fade. Returns the engine's change, if any, and
    /// the music's fade volume (getFadeVolume).
    pub fn step(&mut self, want: &str, smooth: bool, times: FadeTimes, now: std::time::Instant) -> (Option<StreamChange>, f32) {
        let mut change = None;
        if want != self.next {
            self.next = want.to_owned();
            if !times.on || !smooth {
                self.state = FadeState::Idle;
                self.started = None;
                if self.playing != want {
                    change = Some(self.switch_to(want.to_owned()));
                }
                return (change, 1.0);
            }
            // startInternetStreamWithAutoFade: a fade under way goes on
            if self.state == FadeState::Idle {
                if self.playing.is_empty() {
                    change = Some(self.switch_to(self.next.clone()));
                    self.start(FadeState::In, times, now);
                } else {
                    self.start(FadeState::Out, times, now);
                }
            }
        }
        let Some((started, length)) = self.started else {
            return (change, 1.0);
        };
        let t = now.duration_since(started).as_secs_f32() / length;
        if t < 1.0 {
            let gain = if self.state == FadeState::Out { 1.0 - t } else { t };
            return (change, gain);
        }
        // onIdleUpdate: the fade is done
        match self.state {
            FadeState::Out if self.next.is_empty() => {
                self.state = FadeState::Idle;
                self.started = None;
                (Some(self.switch_to(String::new())), 1.0)
            }
            FadeState::Out => {
                let next = self.next.clone();
                self.start(FadeState::In, times, now);
                (Some(self.switch_to(next)), 0.0)
            }
            FadeState::In if self.next != self.playing => {
                self.start(FadeState::Out, times, now);
                (change, 1.0)
            }
            _ => {
                self.state = FadeState::Idle;
                self.started = None;
                (change, 1.0)
            }
        }
    }

    fn start(&mut self, state: FadeState, times: FadeTimes, now: std::time::Instant) {
        let length = if state == FadeState::Out { times.fade_out } else { times.fade_in };
        self.state = state;
        self.started = Some((now, length.max(MINIMUM_FADE)));
        log::info!(
            "parcel music: fade {} over {length:.2} s",
            if state == FadeState::Out { "out" } else { "in" }
        );
    }

    fn switch_to(&mut self, url: String) -> StreamChange {
        self.playing = url;
        if self.playing.is_empty() {
            StreamChange::Stop
        } else {
            StreamChange::Play(self.playing.clone())
        }
    }

    /// A logout: nothing plays, nothing is awaited.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// An error worth a local chat line. A dropped stream (`Interrupted`) is
/// reconnected on its own: it stays in the status tooltip only, so a
/// station that was playing does not say "Can't play" while it reconnects.
pub(super) fn error_is_told(error: &aurora_audio::StreamError) -> bool {
    !matches!(error, aurora_audio::StreamError::Interrupted)
}

/// Bounded session history: retries and revisits never repeat a station notice.
/// Once the history is full, stop notices rather than evicting/repeating them.
pub(super) fn first_error_notice(seen: &mut std::collections::HashSet<String>, url: &str) -> bool {
    !url.is_empty() && seen.len() < 256 && seen.insert(url.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMES: FadeTimes = FadeTimes {
        on: true,
        fade_in: 3.0,
        fade_out: 2.0,
    };

    fn at(start: std::time::Instant, secs: f32) -> std::time::Instant {
        start + std::time::Duration::from_secs_f32(secs)
    }

    #[test]
    fn a_station_fades_in_then_a_new_one_fades_the_old_out_first() {
        let t0 = std::time::Instant::now();
        let mut fade = Fade::default();
        let (a, b) = ("https://radio.example/a", "https://radio.example/b");
        assert_eq!(fade.step(a, true, TIMES, t0), (Some(StreamChange::Play(a.into())), 0.0));
        let (change, gain) = fade.step(a, true, TIMES, at(t0, 1.5));
        assert!(change.is_none() && (gain - 0.5).abs() < 1e-3, "{gain}");
        assert_eq!(fade.step(a, true, TIMES, at(t0, 3.0)), (None, 1.0));
        // the parcel changes: a fades out over 2 s, still playing
        assert_eq!(fade.step(b, true, TIMES, at(t0, 10.0)), (None, 1.0));
        let (change, gain) = fade.step(b, true, TIMES, at(t0, 11.0));
        assert!(change.is_none() && (gain - 0.5).abs() < 1e-3, "{gain}");
        assert_eq!(fade.playing(), a);
        // then b starts silent and fades in over 3 s
        assert_eq!(fade.step(b, true, TIMES, at(t0, 12.0)), (Some(StreamChange::Play(b.into())), 0.0));
        let (_, gain) = fade.step(b, true, TIMES, at(t0, 13.5));
        assert!((gain - 0.5).abs() < 1e-3, "{gain}");
        assert_eq!(fade.step(b, true, TIMES, at(t0, 15.0)), (None, 1.0));
        // a parcel with no music: b fades out, then stops
        fade.step("", true, TIMES, at(t0, 20.0));
        assert_eq!(fade.step("", true, TIMES, at(t0, 22.0)), (Some(StreamChange::Stop), 1.0));
        assert_eq!(fade.playing(), "");
    }

    #[test]
    fn a_change_during_a_fade_in_waits_for_its_end() {
        let t0 = std::time::Instant::now();
        let mut fade = Fade::default();
        let (a, b) = ("https://radio.example/a", "https://radio.example/b");
        fade.step(a, true, TIMES, t0);
        assert_eq!(fade.step(b, true, TIMES, at(t0, 1.0)).0, None, "a fade in goes on");
        assert_eq!(fade.step(b, true, TIMES, at(t0, 3.0)), (None, 1.0), "then a fades out");
        assert_eq!(fade.step(b, true, TIMES, at(t0, 5.0)).0, Some(StreamChange::Play(b.into())));
    }

    #[test]
    fn a_stop_by_hand_or_the_fade_off_cut_at_once() {
        let t0 = std::time::Instant::now();
        let mut fade = Fade::default();
        let (a, b) = ("https://radio.example/a", "https://radio.example/b");
        fade.step(a, true, TIMES, t0);
        fade.step(b, true, TIMES, at(t0, 4.0));
        assert_eq!(fade.step("", false, TIMES, at(t0, 4.5)), (Some(StreamChange::Stop), 1.0));
        assert_eq!(fade.step("", true, TIMES, at(t0, 9.0)), (None, 1.0), "no fade left behind");
        let off = FadeTimes { on: false, ..TIMES };
        assert_eq!(fade.step(a, true, off, at(t0, 10.0)), (Some(StreamChange::Play(a.into())), 1.0));
        assert_eq!(fade.step(b, true, off, at(t0, 10.1)), (Some(StreamChange::Play(b.into())), 1.0));
        // a fade set to 0 s ends at the next frame
        let zero = FadeTimes {
            on: true,
            fade_in: 0.0,
            fade_out: 0.0,
        };
        fade.step(a, true, zero, at(t0, 11.0));
        assert_eq!(fade.step(a, true, zero, at(t0, 11.02)).0, Some(StreamChange::Play(a.into())));
    }

    #[test]
    fn an_interrupted_stream_stays_in_the_tooltip() {
        assert!(!error_is_told(&aurora_audio::StreamError::Interrupted));
        assert!(error_is_told(&aurora_audio::StreamError::Connection));
        assert!(error_is_told(&aurora_audio::StreamError::HttpStatus(503)));
    }

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

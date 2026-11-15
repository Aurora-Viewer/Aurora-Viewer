//! The nearby media list: every media around (the parcel's, and the media
//! on each prim face), what the panel under the top bar's media button
//! shows and changes.
//!
//! Port of LLPanelNearByMedia (indra/newview/llpanelnearbymedia.cpp:
//! refreshList, refreshParcelItems, updateListItem, shouldShow, setDisabled,
//! getNameAndUrlHelper, onMenuAction) and LLViewerMedia::setAllMediaEnabled
//! (indra/newview/llviewermedia.cpp), originally LGPL 2.1.
//!
//! One difference: the Legacy Viewer ticks a media that is merely not
//! disabled, playing or not, and dims it when it does not show. Here the
//! tick is the media being on (wanted): a media that waits for a click or
//! for Start all is unticked, and ticking it starts it, as the Legacy
//! Viewer's tick does (setDisabled with forcePlayOnEnable).

use super::*;
use crate::scene::Scene;
use base64::Engine as _;

/// Where a media is, for the list's Show choice (MediaClass).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// In the parcel the agent stands on (isInAgentParcel); the parcel's
    /// own media, the HUDs and our own attachments too.
    InParcel,
    OutsideParcel,
    /// On another avatar's attachment (isAttachedToAnotherAvatar).
    OnOthers,
}

/// The Show choice (show_combo: All, WithinParcel, OutsideParcel, OnOthers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Show {
    #[default]
    All,
    InParcel,
    OutsideParcel,
    OnOthers,
}

impl Show {
    pub const ALL: [Show; 4] = [Show::All, Show::InParcel, Show::OutsideParcel, Show::OnOthers];

    /// LLPanelNearByMedia::shouldShow; the parcel's media shows with All and
    /// In this parcel only (refreshParcelItems).
    pub fn shows(self, class: Class) -> bool {
        match self {
            Show::All => true,
            Show::InParcel => class == Class::InParcel,
            Show::OutsideParcel => class == Class::OutsideParcel,
            Show::OnOthers => class == Class::OnOthers,
        }
    }
}

/// One row of the list.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub key: MediaKey,
    /// The page's title, else its address (getNameAndUrlHelper); empty when
    /// it has neither (the panel writes "<empty>").
    pub name: String,
    pub url: String,
    pub class: Class,
    /// Ticked: the media is on.
    pub on: bool,
    /// A plugin shows it (hasMedia): the row is not dimmed.
    pub showing: bool,
    /// Video or sound media (isMediaTimeBased): Pause shows.
    pub time_based: bool,
    pub paused: bool,
    pub volume: f32,
    pub muted: bool,
    /// On a HUD: no zoom (nextZoomLevel never zooms on a HUD).
    pub hud: bool,
    /// Metres from the camera; the parcel's media comes first.
    pub distance: f32,
}

impl Row {
    /// Zoom applies to media on a prim in the world.
    pub fn zoomable(&self) -> bool {
        matches!(self.key, MediaKey::Prim { .. }) && !self.hud && self.on
    }
}

/// The parcel media's row: its fixed name, the parcel's address.
fn parcel_row(m: Option<&MediaImpl>, url: &str) -> Row {
    Row {
        key: MediaKey::Parcel,
        name: String::new(),
        url: m
            .map(|m| if m.location.is_empty() { m.url.clone() } else { m.location.clone() })
            .unwrap_or_else(|| url.to_string()),
        class: Class::InParcel,
        on: m.is_some_and(|m| m.wanted),
        showing: m.is_some_and(|m| m.is_loaded()),
        time_based: m.is_some_and(MediaImpl::is_time_media),
        paused: m.is_some_and(|m| m.paused),
        volume: m.map_or(1.0, |m| m.volume),
        muted: m.is_some_and(|m| m.muted),
        hud: false,
        distance: -1.0,
    }
}

/// getNameAndUrlHelper: the loaded address, else the entry's current one,
/// else its home page; the title, else that address.
fn name_and_url(m: &MediaImpl) -> (String, String) {
    let mut url = m.location.trim().to_string();
    if url.is_empty() {
        url = m.url.trim().to_string();
    }
    if url.is_empty()
        && let Some(e) = m.entry.as_ref()
    {
        url = e.home_url.trim().to_string();
    }
    let title = m.title.trim();
    let name = if title.is_empty() { url.clone() } else { title.to_string() };
    (name, url)
}

/// What Copy data puts on the clipboard for a `data:` address
/// (onMenuAction "copy_data"): the base64 part decoded, else the address
/// unescaped. `None` for any other address (onMenuVisible hides it).
pub fn copy_data(url: &str) -> Option<String> {
    if !url.starts_with("data:") {
        return None;
    }
    const BASE64: &str = "base64,";
    Some(match url.find(BASE64) {
        Some(at) => {
            let raw = &url[at + BASE64.len()..];
            match base64::engine::general_purpose::STANDARD.decode(raw.trim()) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(_) => String::new(),
            }
        }
        None => crate::slapp::unescape(url),
    })
}

/// Rows in the list's order: the parcel's media first, then by distance
/// (the proximity column the Legacy Viewer sorts by).
pub fn sort_rows(rows: &mut [Row]) {
    rows.sort_by(|a, b| {
        (a.key != MediaKey::Parcel)
            .cmp(&(b.key != MediaKey::Parcel))
            .then(a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal))
    });
}

impl MediaManager {
    /// The rows of the nearby media list (refreshList). The parcel's media
    /// has a row while the parcel has one and media are allowed
    /// (AudioStreamingMedia); the media on prims have theirs while they are
    /// tracked, which `reconcile_prims` already limits to the media allowed
    /// (media on a prim, on others' attachments).
    pub fn nearby_rows(&self, world: &World, settings: &MediaSettings, now: Instant) -> Vec<Row> {
        let mut rows = Vec::new();
        if !settings.enabled {
            return rows;
        }
        let agent_parcel = world.parcel.as_ref().and_then(|p| {
            let handle = world.main_region?;
            Some((p, aurora_net::handle_to_origin(handle)))
        });
        if let Some((p, _)) = agent_parcel {
            let url = effective_parcel_url(&p.media, &p.media_url);
            if !url.is_empty() && !p.media.mime.eq_ignore_ascii_case(NONE_MIME) {
                rows.push(parcel_row(self.find(&MediaKey::Parcel), &url));
            }
        }
        for m in &self.impls {
            let MediaKey::Prim { object, .. } = m.key else {
                continue;
            };
            let Some(om) = self.objects.get(&object) else {
                continue;
            };
            let class = if om.on_other_avatar {
                Class::OnOthers
            } else if om.hud || om.on_own_avatar {
                Class::InParcel
            } else {
                match (agent_parcel, global_position(world, om.idx, now)) {
                    (Some((p, origin)), Some(pos)) if crate::world::landmarks::in_parcel(&p.bitmap, origin, pos) => Class::InParcel,
                    _ => Class::OutsideParcel,
                }
            };
            let (name, url) = name_and_url(m);
            rows.push(Row {
                key: m.key,
                name,
                url,
                class,
                on: m.wanted,
                showing: m.is_loaded(),
                time_based: m.is_time_media(),
                paused: m.paused,
                volume: m.volume,
                muted: m.muted,
                hud: om.hud,
                distance: if om.hud { 0.0 } else { m.distance },
            });
        }
        sort_rows(&mut rows);
        rows
    }

    /// A row's tick (onCheckItem → setDisabled): off unloads the media,
    /// on starts it. The parcel's media goes through `play_parcel`, and so
    /// through the media filter, like the top bar's button.
    pub fn set_media_on(&mut self, key: MediaKey, on: bool, world: &World, settings: &MediaSettings) {
        match key {
            MediaKey::Parcel if on => {
                let paused = self.find(&key).is_some_and(|m| m.paused);
                match self.find_mut(&key) {
                    Some(m) if paused => m.play(),
                    _ => self.play_parcel(world, settings),
                }
            }
            MediaKey::Parcel => self.stop_parcel(),
            MediaKey::Prim { .. } => {
                if let Some(m) = self.find_mut(&key) {
                    if on {
                        m.failed = None;
                        m.play();
                    } else {
                        m.wanted = false;
                        m.paused = false;
                    }
                }
                if !on && self.focus == Some(key) {
                    self.unfocus();
                }
            }
        }
    }

    /// Start all / Stop all and the top bar's media button
    /// (LLViewerMedia::setAllMediaEnabled): the tentative autoplay follows,
    /// Stop all unloads every media and the parcel's; Start all starts the
    /// parcel's media and lets the media that play by themselves start
    /// again (isAutoPlayable), the others waiting for a click.
    pub fn set_all_media_on(&mut self, on: bool, world: &World, settings: &MediaSettings, autoplay: bool) {
        self.tentative_autoplay = on;
        let autoplay = autoplay || self.force_autoplay;
        let huds: Vec<Uuid> = self.objects.iter().filter(|(_, o)| o.hud).map(|(id, _)| *id).collect();
        for m in &mut self.impls {
            let MediaKey::Prim { object, .. } = m.key else {
                continue;
            };
            if on {
                let auto = m.entry.as_ref().is_some_and(|e| e.auto_play);
                let hud = huds.contains(&object);
                if auto && (autoplay || (hud && settings.autoplay_huds)) {
                    m.failed = None;
                    m.play();
                }
            } else {
                m.wanted = false;
                m.paused = false;
            }
        }
        if on {
            if !self.parcel_playing() {
                self.play_parcel(world, settings);
            }
        } else {
            self.stop_parcel();
            self.unfocus();
        }
    }

    /// Something to start or stop with the top bar's media button: a media
    /// on a prim or the parcel's (LLStatusBar::refresh: hasInWorldMedia,
    /// hasParcelMedia).
    pub fn any_in_world_media(&self) -> bool {
        self.impls.iter().any(|m| matches!(m.key, MediaKey::Prim { .. }))
    }

    /// The button's state: a media is on (isAnyMediaPlaying /
    /// isAnyMediaShowing, isParcelMediaPlaying).
    pub fn any_media_on(&self) -> bool {
        self.impls.iter().any(|m| m.wanted && !m.paused)
    }

    /// Where a prim media's face is, to zoom on it (focusZoomOnMedia): the
    /// prim's index and face. `None` on a HUD.
    pub fn media_face(&self, key: &MediaKey) -> Option<(usize, u8)> {
        let MediaKey::Prim { object, face } = *key else {
            return None;
        };
        let om = self.objects.get(&object).filter(|o| !o.hud)?;
        Some((om.idx, face))
    }
}

/// An object's position on the grid (global metres), for the parcel test
/// (LLViewerMediaImpl::isInAgentParcel takes the media's object position).
fn global_position(world: &World, idx: usize, now: Instant) -> Option<glam::DVec3> {
    let o = world.objects.get(idx)?;
    let (pos, _, hud) = Scene::object_transform(world, idx, now, 0)?;
    if hud {
        return None;
    }
    let local = pos - world.region_offset(o.key.region)?;
    let (x, y) = aurora_net::handle_to_origin(o.key.region);
    Some(glam::DVec3::new(f64::from(x), f64::from(y), 0.0) + local.as_dvec3())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: MediaKey, distance: f32) -> Row {
        Row {
            key,
            name: String::new(),
            url: String::new(),
            class: Class::InParcel,
            on: false,
            showing: false,
            time_based: false,
            paused: false,
            volume: 1.0,
            muted: false,
            hud: false,
            distance,
        }
    }

    fn prim(n: u128) -> MediaKey {
        MediaKey::Prim {
            object: Uuid::from_u128(n),
            face: 0,
        }
    }

    #[test]
    fn show_choices_follow_the_media_class() {
        for class in [Class::InParcel, Class::OutsideParcel, Class::OnOthers] {
            assert!(Show::All.shows(class));
        }
        assert!(Show::InParcel.shows(Class::InParcel));
        assert!(!Show::InParcel.shows(Class::OutsideParcel));
        assert!(Show::OutsideParcel.shows(Class::OutsideParcel));
        assert!(!Show::OutsideParcel.shows(Class::OnOthers));
        assert!(Show::OnOthers.shows(Class::OnOthers));
        assert!(!Show::OnOthers.shows(Class::InParcel));
    }

    #[test]
    fn the_parcel_media_comes_first_then_the_nearest() {
        let mut rows = vec![
            row(prim(2), 12.0),
            row(prim(1), 3.0),
            row(MediaKey::Parcel, -1.0),
            row(prim(3), 7.5),
        ];
        sort_rows(&mut rows);
        let keys: Vec<MediaKey> = rows.iter().map(|r| r.key).collect();
        assert_eq!(keys, vec![MediaKey::Parcel, prim(1), prim(3), prim(2)]);
    }

    #[test]
    fn copy_data_decodes_data_addresses_only() {
        assert_eq!(copy_data("https://example.invalid/a%20b"), None);
        assert_eq!(copy_data("data:text/html;base64,PGI+aGk8L2I+").as_deref(), Some("<b>hi</b>"));
        assert_eq!(
            copy_data("data:text/html,%3Cb%3Ehi%3C%2Fb%3E").as_deref(),
            Some("data:text/html,<b>hi</b>")
        );
        assert_eq!(copy_data("data:text/html;base64,***").as_deref(), Some(""));
    }

    #[test]
    fn names_fall_back_from_the_title_to_the_loaded_then_the_entry_address() {
        let mut m = MediaImpl::new(prim(1), "https://example.invalid/entry".into(), String::new());
        assert_eq!(
            name_and_url(&m),
            ("https://example.invalid/entry".into(), "https://example.invalid/entry".into())
        );
        m.location = "https://example.invalid/page".into();
        assert_eq!(name_and_url(&m).1, "https://example.invalid/page");
        m.title = "Cinema".into();
        assert_eq!(name_and_url(&m), ("Cinema".into(), "https://example.invalid/page".into()));
        m.location.clear();
        m.url.clear();
        m.entry = Some(MediaEntry {
            home_url: "https://example.invalid/home".into(),
            ..Default::default()
        });
        assert_eq!(name_and_url(&m).1, "https://example.invalid/home");
    }

    #[test]
    fn ticking_starts_a_media_and_stop_all_unloads_everything() {
        let w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let settings = MediaSettings::default();
        let mut mm = MediaManager::default();
        let mut a = MediaImpl::new(prim(1), "https://example.invalid/a".into(), String::new());
        a.entry = Some(MediaEntry {
            auto_play: true,
            ..Default::default()
        });
        let b = MediaImpl::new(prim(2), "https://example.invalid/b".into(), String::new());
        mm.impls.push(a);
        mm.impls.push(b);
        mm.set_media_on(prim(2), true, &w, &settings);
        assert!(mm.find(&prim(2)).is_some_and(|m| m.wanted));
        assert!(mm.any_media_on());
        mm.set_all_media_on(false, &w, &settings, true);
        assert!(!mm.any_media_on());
        assert!(!mm.tentative_autoplay);
        // Start all: only the media that plays by itself starts again
        mm.set_all_media_on(true, &w, &settings, true);
        assert!(mm.tentative_autoplay);
        assert!(mm.find(&prim(1)).is_some_and(|m| m.wanted));
        assert!(mm.find(&prim(2)).is_some_and(|m| !m.wanted));
        // without the autoplay preference nothing starts by itself
        mm.set_all_media_on(false, &w, &settings, true);
        mm.set_all_media_on(true, &w, &settings, false);
        assert!(!mm.any_media_on());
        mm.set_media_on(prim(1), true, &w, &settings);
        mm.set_media_on(prim(1), false, &w, &settings);
        assert!(mm.find(&prim(1)).is_some_and(|m| !m.wanted));
    }
}

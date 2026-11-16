//! The nearby media panel under the top bar's media button
//! (`ui/nearby_media.rs`, rows from `media::nearby`): what its buttons do
//! to the media, the camera zoom on a media face, and the button's click
//! that starts or stops all media (LLStatusBar::onClickMediaToggle).
//!
//! `AURORA_DEMO_NEARBY_MEDIA` (docs/TESTING.md): three media screens, two
//! in the plaza's parcel and one outside it, and the panel pinned open.

use super::*;
use crate::media::MediaKey;
use crate::media::nearby::Show;
use crate::ui::nearby_media::{Action, NearbyMediaUi};

#[derive(Default)]
pub(super) struct State {
    ui: NearbyMediaUi,
    /// The media the camera zoomed on (isZoomedOnMedia), until the camera
    /// follows the avatar again.
    zoomed: Option<MediaKey>,
    /// Demo: frames the panel has shown, for its zoom or Stop all.
    demo_frames: u32,
}

/// `AURORA_DEMO_NEARBY_MEDIA`: `open` (the panel pinned open), `select` (a
/// media selected), `outside` (Show: outside this parcel), `zoom` (Zoom on
/// the nearest screen) and `stop-all` (Stop all), 300 frames after the
/// panel first showed.
pub(super) fn demo_mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_NEARBY_MEDIA").ok()).as_deref()
}

/// The demo's zoom or Stop all, once the panel has shown that many frames
/// (the top bar is not drawn while the world loads).
const DEMO_ACTION_FRAMES: u32 = 300;

/// What the panel reads, borrowed while the bars are drawn.
pub(super) struct Inputs<'a> {
    pub media: &'a crate::media::MediaManager,
    pub world: &'a World,
    pub settings: &'a crate::media::MediaSettings,
    pub camera: &'a Camera,
    pub demo: bool,
}

/// The panel for this frame, after the top bar drew the media button.
pub(super) fn panel(ctx: &egui::Context, p: &Palette, st: &mut State, audio_ui: &mut crate::ui::audio::AudioUi, i: Inputs) -> Vec<Action> {
    let demo = if i.demo { demo_mode() } else { None };
    // a zoom ends when the camera goes back to the avatar
    if i.camera.focus_on_avatar() {
        st.zoomed = None;
    }
    let wanted = st.ui.open || audio_ui.media_button_hovered || demo.is_some();
    let rows = if wanted {
        i.media.nearby_rows(i.world, i.settings, Instant::now())
    } else {
        Vec::new()
    };
    if let Some(mode) = demo {
        match mode {
            "select" | "zoom" => {
                if st.ui.selected.is_none() {
                    st.ui.selected = rows.iter().find(|r| matches!(r.key, MediaKey::Prim { .. })).map(|r| r.key);
                }
            }
            "outside" => st.ui.show = Show::OutsideParcel,
            _ => {}
        }
    }
    let mut out = crate::ui::nearby_media::show(ctx, p, &mut st.ui, audio_ui, &rows, st.zoomed, demo.is_some());
    if demo.is_some() && audio_ui.media_button.is_some() {
        st.demo_frames += 1;
    }
    if st.demo_frames == DEMO_ACTION_FRAMES {
        log::info!("nearby media: demo {} on {} rows", demo.unwrap_or_default(), rows.len());
        match demo {
            Some("zoom") => {
                if let Some(key) = st.ui.selected {
                    out.push(Action::Zoom(key));
                }
            }
            Some("stop-all") => out.push(Action::All(false)),
            _ => {}
        }
    }
    out
}

impl App {
    /// The media button's click: all media off when one is on, else all on
    /// (LLViewerMedia::setAllMediaEnabled).
    pub(super) fn toggle_all_media(&mut self) {
        let on = !self.media.any_media_on();
        self.media
            .set_all_media_on(on, &self.world, &self.settings.media, self.settings.audio.media_autoplay);
    }

    pub(super) fn apply_nearby_media(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::All(on) => {
                    log::info!("nearby media: {} all", if on { "start" } else { "stop" });
                    self.media
                        .set_all_media_on(on, &self.world, &self.settings.media, self.settings.audio.media_autoplay);
                }
                Action::On(key, on) => self.media.set_media_on(key, on, &self.world, &self.settings.media),
                Action::Play(key) => {
                    // onClickSelectedMediaPlay: a paused video resumes, an
                    // unloaded media starts
                    self.media.set_media_on(key, true, &self.world, &self.settings.media);
                }
                Action::Pause(key) => {
                    if let Some(m) = self.media.find_mut(&key) {
                        m.pause();
                    }
                }
                Action::Volume(key, volume) => {
                    if let Some(m) = self.media.find_mut(&key) {
                        m.volume = volume.clamp(0.0, 1.0);
                        m.muted = false;
                    }
                }
                Action::Mute(key, muted) => {
                    if let Some(m) = self.media.find_mut(&key) {
                        m.muted = muted;
                    }
                }
                Action::Zoom(key) => self.zoom_on_media(key),
                Action::Unzoom => {
                    // LLPanelPrimMediaControls::updateZoom, ZOOM_NONE
                    self.nearby_media.zoomed = None;
                    self.camera
                        .set_focus_on_avatar(true, true, true, &mut self.world.agent, &self.settings.camera);
                }
                Action::Copy(text) => self.egui_ctx.copy_text(text),
            }
        }
    }

    /// LLViewerMediaFocus::focusZoomOnMedia: the camera frames the prim the
    /// media is on, looking at its face. The Legacy Viewer also gives the
    /// face the media focus; here the page keeps the keyboard where it was
    /// until it is clicked, as the list is not the page.
    fn zoom_on_media(&mut self, key: MediaKey) {
        let Some((idx, face)) = self.media.media_face(&key) else {
            return;
        };
        let now = Instant::now();
        let Some((center, rotation, hud)) = Scene::object_transform(&self.world, idx, now, 0) else {
            return;
        };
        let Some(size) = self.world.objects.get(idx).map(|o| o.scale) else {
            return;
        };
        if hud {
            return;
        }
        // getApproximateFaceNormal, else the camera's back
        // (focusZoomOnMedia: the inverse of the look-at axis)
        let normal = self
            .media
            .face_normal(&self.world, &self.scene, idx, face)
            .map(|n| rotation * n)
            .unwrap_or(-self.camera.forward());
        let object = crate::camera::pick_focus_object(&self.world, idx, now);
        let aspect = self.view_aspect();
        if self.camera.zoom_media(
            center,
            rotation,
            size,
            normal,
            object,
            aspect,
            &mut self.world.agent,
            &self.settings.camera,
        ) {
            log::info!("nearby media: zoom on {key:?}");
            self.nearby_media.zoomed = Some(key);
        }
    }
}

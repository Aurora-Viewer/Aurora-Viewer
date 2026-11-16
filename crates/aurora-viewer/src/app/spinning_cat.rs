//! The spinning cat in the app (`crate::oiia`): the keys it watches, where
//! it starts, the video its music plays from, and its frame.

use super::App;
use crate::oiia::konami::Key;
use crate::oiia::{Source, VideoNow, video};
use glam::Vec3;
use std::time::Instant;
use winit::keyboard::KeyCode;

/// The key of the Konami code a press stands for: the arrows where they
/// are, B and A by the letter they type, so that the code is the same on
/// every keyboard layout.
pub(super) fn konami_key(code: KeyCode, logical: &winit::keyboard::Key) -> Key {
    match code {
        KeyCode::ArrowUp => return Key::Up,
        KeyCode::ArrowDown => return Key::Down,
        KeyCode::ArrowLeft => return Key::Left,
        KeyCode::ArrowRight => return Key::Right,
        _ => {}
    }
    match logical {
        winit::keyboard::Key::Character(text) if text.eq_ignore_ascii_case("b") => Key::B,
        winit::keyboard::Key::Character(text) if text.eq_ignore_ascii_case("a") => Key::A,
        _ => Key::Other,
    }
}

/// `AURORA_DEMO_OIIA=<seconds>` (demo only): the show starts by itself,
/// without its warning, from that second of its music, on the clock and
/// without the video or its sound: for the screenshots.
fn demo_start(value: &str) -> Option<f64> {
    value.trim().parse::<f64>().ok().filter(|s| s.is_finite() && *s >= 0.0)
}

impl App {
    /// A key went down in the world: the spinning cat watches for its code.
    pub(super) fn spinning_cat_key(&mut self, code: KeyCode, logical: &winit::keyboard::Key) {
        if !self.in_world() {
            return;
        }
        let typing = self.egui_ctx.text_edit_focused();
        self.oiia.key(konami_key(code, logical), Instant::now(), typing);
    }

    /// The show's video can play: without it there is no show, and the
    /// code's window says why.
    pub(super) fn spinning_cat_video(&mut self) -> bool {
        self.media.plugins(&self.settings.media).is_some() && self.chat_ui.previews.can_play_video()
    }

    /// Starts the show in front of the avatar, at its feet, the cat facing
    /// it, and its video in the video window. `demo` (`demo_start`): the
    /// video's timings from that second instead, without the video.
    pub(super) fn start_spinning_cat(&mut self, demo: Option<f64>) {
        let agent = &self.world.agent;
        // Ahead of the avatar and a little to its right: straight ahead, the
        // avatar itself hides the cat from the camera behind it.
        let ahead = Vec3::new(agent.yaw.cos(), agent.yaw.sin(), 0.0);
        let right = Vec3::new(agent.yaw.sin(), -agent.yaw.cos(), 0.0);
        let offset = ahead * crate::oiia::AHEAD + right * crate::oiia::ASIDE;
        let spot = agent.position + offset;
        // On what lies under that spot (the land, a floor, a platform) when
        // it is within a step of the avatar's feet, at their level otherwise.
        let feet = agent.position.z - self.pelvis_to_foot();
        let above = Vec3::new(spot.x, spot.y, agent.position.z + 0.5);
        let surface = self
            .scene
            .creation_surface(&self.world, (above, -Vec3::Z), 4.0, &mut Vec::new())
            .map(|(point, _)| point.z)
            .filter(|z| (feet - 1.0..=feet + 1.0).contains(z));
        let centre = Vec3::new(spot.x, spot.y, surface.unwrap_or(feet));
        // The cat looks back at the avatar.
        let facing = (-offset.y).atan2(-offset.x);
        let (source, from) = match demo {
            Some(from) => (Source::Cues, from),
            None if self.spinning_cat_video()
                && self
                    .chat_ui
                    .previews
                    .play_video(video::VIDEO, video::START, "W&W - OIIA OIIA (Spinning Cat)") =>
            {
                (Source::Video, 0.0)
            }
            None => {
                self.oiia.decline();
                return;
            }
        };
        self.oiia.start(centre, facing, self.world.main_region, source, from);
    }

    /// The show's frame: what the scene draws of it, before the frame's
    /// lists are built.
    pub(super) fn spinning_cat_frame(&mut self, renderer: &mut aurora_render::Renderer, dt: f32) {
        if self.demo
            && self.frame_count == 300
            && let Some(from) = std::env::var("AURORA_DEMO_OIIA").ok().as_deref().and_then(demo_start)
        {
            self.start_spinning_cat(Some(from));
        }
        let now = VideoNow {
            heard: crate::link_preview::video_time().map(|t| (t.seconds, t.state == 1, t.at.elapsed().as_secs_f64())),
            open: self.chat_ui.previews.own_video_playing(),
        };
        let stage = self
            .oiia
            .tick(dt, self.settings.reduce_motion, self.world.main_region, now)
            .filter(|_| self.in_world())
            .map(|stage| crate::scene::spinning_cat::Stage {
                centre: stage.centre,
                facing: stage.facing,
                frame: stage.frame,
                time: stage.time,
            });
        if self.oiia.take_close_video() {
            self.chat_ui.previews.stop_own_video();
        }
        self.scene.spinning_cat.update(renderer, stage.as_ref());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::{Key as Logical, NamedKey, SmolStr};

    #[test]
    fn the_letters_of_the_code_are_the_ones_typed_whatever_the_layout() {
        let letter = |text: &str| Logical::Character(SmolStr::new(text));
        // AZERTY: the key at the QWERTY "Q" types "a".
        assert_eq!(konami_key(KeyCode::KeyQ, &letter("a")), Key::A);
        assert_eq!(konami_key(KeyCode::KeyA, &letter("q")), Key::Other);
        assert_eq!(konami_key(KeyCode::KeyB, &letter("B")), Key::B);
        assert_eq!(konami_key(KeyCode::ArrowUp, &Logical::Named(NamedKey::ArrowUp)), Key::Up);
        assert_eq!(konami_key(KeyCode::ArrowLeft, &Logical::Named(NamedKey::ArrowLeft)), Key::Left);
        assert_eq!(konami_key(KeyCode::Enter, &Logical::Named(NamedKey::Enter)), Key::Other);
    }

    #[test]
    fn the_demo_starts_the_show_from_a_second_of_its_music() {
        assert_eq!(demo_start("46"), Some(46.0));
        assert_eq!(demo_start(" 80.5 "), Some(80.5));
        assert_eq!(demo_start("-3"), None);
        assert_eq!(demo_start("soon"), None);
        assert_eq!(demo_start(""), None);
    }
}

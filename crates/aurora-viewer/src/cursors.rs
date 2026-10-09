//! Original Firestorm Windows cursors: res/toolsit.cur and res/toolbuy.cur
//! (LGPL 2.1), exported as RGBA PNG with their original hotspots.
//! Firestorm's modern Pay cursor uses the same artwork as Buy.
use crate::interaction::Action;

pub struct Cursors {
    sit: Option<egui::CustomCursorImage>,
    money: Option<egui::CustomCursorImage>,
    open: Option<egui::CustomCursorImage>,
    play: Option<egui::CustomCursorImage>,
    pause: Option<egui::CustomCursorImage>,
    media_open: Option<egui::CustomCursorImage>,
    zoom: Option<egui::CustomCursorImage>,
    grab: Option<egui::CustomCursorImage>,
}
impl Default for Cursors {
    fn default() -> Self {
        fn load(bytes: &[u8], hotspot: [u16; 2]) -> Option<egui::CustomCursorImage> {
            match image::load_from_memory(bytes) {
                Ok(i) if i.width() == 32 && i.height() == 32 => Some(egui::CustomCursorImage {
                    rgba: i.into_rgba8().into_raw().into(),
                    size: [32, 32],
                    hotspot,
                }),
                Ok(_) => {
                    log::warn!("invalid Firestorm cursor dimensions");
                    None
                }
                Err(e) => {
                    log::warn!("cannot load Firestorm cursor: {e}");
                    None
                }
            }
        }
        Self {
            sit: load(include_bytes!("../assets/cursors/sit.png"), [20, 15]),
            money: load(include_bytes!("../assets/cursors/money.png"), [20, 15]),
            open: load(include_bytes!("../assets/cursors/open.png"), [20, 15]),
            play: load(include_bytes!("../assets/cursors/play.png"), [1, 1]),
            pause: load(include_bytes!("../assets/cursors/pause.png"), [1, 1]),
            media_open: load(include_bytes!("../assets/cursors/media-open.png"), [1, 1]),
            zoom: load(include_bytes!("../assets/cursors/zoom.png"), [7, 5]),
            grab: load(include_bytes!("../assets/cursors/grab.png"), [2, 13]),
        }
    }
}
impl Cursors {
    pub fn image(&self, action: Action, playing: bool) -> Option<egui::CustomCursorImage> {
        match action {
            Action::Sit => self.sit.clone(),
            Action::Buy | Action::Pay => self.money.clone(),
            Action::Open => self.open.clone(),
            Action::Play if playing => self.pause.clone(),
            Action::OpenMedia if playing => self.media_open.clone(),
            Action::Play | Action::OpenMedia => self.play.clone(),
            Action::Zoom => self.zoom.clone(),
            Action::Grab => self.grab.clone(),
            Action::Touch | Action::Disabled => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_artwork_hotspot_and_shared_buy_pay_buffer() {
        let c = Cursors::default();
        let buy = c.image(Action::Buy, false).unwrap();
        assert_eq!(buy.hotspot, [20, 15]);
        assert_eq!(buy.size, [32, 32]);
        assert!(std::sync::Arc::ptr_eq(&buy.rgba, &c.image(Action::Pay, false).unwrap().rgba));
        for (action, playing, hotspot) in [
            (Action::Sit, false, [20, 15]),
            (Action::Open, false, [20, 15]),
            (Action::Play, false, [1, 1]),
            (Action::Play, true, [1, 1]),
            (Action::OpenMedia, true, [1, 1]),
            (Action::Zoom, false, [7, 5]),
            (Action::Grab, false, [2, 13]),
        ] {
            let image = c.image(action, playing).unwrap();
            assert_eq!(image.hotspot, hotspot);
            assert_eq!(image.rgba.len(), 32 * 32 * 4);
            assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
        }
    }
}

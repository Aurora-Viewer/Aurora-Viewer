//! Original Firestorm Windows cursors: res/toolsit.cur and res/toolbuy.cur
//! (LGPL 2.1), exported as RGBA PNG with their original hotspots.
//! Firestorm's modern Pay cursor uses the same artwork as Buy.
//! Tabler outline HUD / pipette cursors use the skin's ink and dark outline.
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
    hud_drag: Option<egui::CustomCursorImage>,
    pipette: Option<egui::CustomCursorImage>,
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
            hud_drag: hud_drag_image(&crate::theme::Theme::default().palette()),
            pipette: pipette_image(&crate::theme::Theme::default().palette()),
        }
    }
}
impl Cursors {
    pub fn with_palette(palette: &crate::theme::Palette) -> Self {
        Self {
            hud_drag: hud_drag_image(palette),
            pipette: pipette_image(palette),
            ..Self::default()
        }
    }

    pub fn set_palette(&mut self, palette: &crate::theme::Palette) {
        self.hud_drag = hud_drag_image(palette);
        self.pipette = pipette_image(palette);
    }
    /// Windows maps the system Grabbing icon to arrows, so use the existing
    /// Tabler outline grasping hand with a contrasting native-cursor outline.
    pub fn hud_drag(&self) -> Option<egui::CustomCursorImage> {
        self.hud_drag.clone()
    }
    /// LLToolPipette uses UI_CURSOR_PIPETTE; the tip of the Tabler pipette is the hotspot.
    pub fn pipette(&self) -> Option<egui::CustomCursorImage> {
        self.pipette.clone()
    }
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

fn hud_drag_image(palette: &crate::theme::Palette) -> Option<egui::CustomCursorImage> {
    tabler_image(include_bytes!("../assets/icons/tabler/outline/hand-grab.svg"), palette, [16, 16])
}

fn pipette_image(palette: &crate::theme::Palette) -> Option<egui::CustomCursorImage> {
    tabler_image(include_bytes!("../assets/icons/tabler/outline/color-picker.svg"), palette, [5, 27])
}

/// A Tabler outline icon (24 px grid, 2 px stroke) as a 32 px cursor: a wide
/// dark stroke under the ink one gives the contrasting outline.
fn tabler_image(svg: &[u8], palette: &crate::theme::Palette, hotspot: [u16; 2]) -> Option<egui::CustomCursorImage> {
    let svg = std::str::from_utf8(svg).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(32, 32)?;
    let options = resvg::usvg::Options::default();
    for (color, width) in [(palette.bar, "4"), (palette.ink, "2")] {
        let color = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
        let source = svg
            .replace("currentColor", &color)
            .replace("stroke-width=\"2\"", &format!("stroke-width=\"{width}\""));
        let tree = resvg::usvg::Tree::from_data(source.as_bytes(), &options).ok()?;
        let scale = 32.0 / 24.0;
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    }
    // resvg pixels are premultiplied; native cursor pixels are straight RGBA.
    let rgba: Vec<u8> = pixmap
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            let straight = |c: u8| {
                if p[3] == 0 {
                    0
                } else {
                    ((c as u32 * 255) / p[3] as u32).min(255) as u8
                }
            };
            [straight(p[0]), straight(p[1]), straight(p[2]), p[3]]
        })
        .collect();
    Some(egui::CustomCursorImage {
        rgba: rgba.into(),
        size: [32, 32],
        hotspot,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_artwork_hotspot_and_shared_buy_pay_buffer() {
        let c = Cursors::default();
        let hand = c.hud_drag().unwrap();
        assert_eq!(hand.size, [32, 32]);
        assert_eq!(hand.hotspot, [16, 16]);
        assert!(hand.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
        let pipette = c.pipette().unwrap();
        assert_eq!(pipette.size, [32, 32]);
        assert_eq!(pipette.hotspot, [5, 27]);
        assert!(pipette.rgba[(27 * 32 + 5) * 4 + 3] > 0, "the hotspot must lie on the pipette tip");
        assert!(std::sync::Arc::ptr_eq(&pipette.rgba, &c.pipette().unwrap().rgba));
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

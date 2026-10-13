//! World cursors drawn from Tabler outline icons, in the skin's ink with a
//! dark outline. Firestorm shows its own artwork for the same states
//! (LLToolPie click actions, LLToolCamera, LLToolPipette); Aurora keeps its
//! rules and replaces the drawings with the interface's icons, at the
//! human's request.
use crate::interaction::Action;

pub struct Cursors {
    touch: Option<egui::CustomCursorImage>,
    sit: Option<egui::CustomCursorImage>,
    buy: Option<egui::CustomCursorImage>,
    pay: Option<egui::CustomCursorImage>,
    open: Option<egui::CustomCursorImage>,
    play: Option<egui::CustomCursorImage>,
    pause: Option<egui::CustomCursorImage>,
    media_open: Option<egui::CustomCursorImage>,
    zoom: Option<egui::CustomCursorImage>,
    hud_drag: Option<egui::CustomCursorImage>,
    inventory_copy: Option<egui::CustomCursorImage>,
    inventory_move: Option<egui::CustomCursorImage>,
    pipette: Option<egui::CustomCursorImage>,
    /// Zoom, orbit, pan (`ToolMode` order).
    camera: [Option<egui::CustomCursorImage>; 3],
}
impl Default for Cursors {
    fn default() -> Self {
        Self::with_palette(&crate::theme::Theme::default().palette())
    }
}
impl Cursors {
    pub fn with_palette(palette: &crate::theme::Palette) -> Self {
        macro_rules! icon {
            ($name:literal) => {
                include_bytes!(concat!("../assets/icons/tabler/outline/", $name, ".svg"))
            };
        }
        Self {
            // the fingertip is the hotspot, like the system hand
            touch: tabler_image(icon!("hand-finger"), palette, [12, 4]),
            sit: badge_image(icon!("armchair"), palette),
            buy: badge_image(icon!("shopping-cart"), palette),
            pay: badge_image(icon!("coin"), palette),
            open: badge_image(icon!("package"), palette),
            play: badge_image(icon!("player-play"), palette),
            pause: badge_image(icon!("player-pause"), palette),
            media_open: badge_image(icon!("device-tv"), palette),
            zoom: badge_image(icon!("zoom-in"), palette),
            hud_drag: tabler_image(icon!("hand-grab"), palette, [16, 16]),
            inventory_copy: badge_image(include_bytes!("../assets/icons/tabler/filled/copy.svg"), palette),
            inventory_move: badge_image(icon!("arrows-move"), palette),
            pipette: tabler_image(icon!("color-picker"), palette, [5, 27]),
            camera: [
                tabler_image(icon!("focus-centered"), palette, [16, 16]),
                tabler_image(icon!("view-360"), palette, [16, 16]),
                tabler_image(icon!("arrows-move"), palette, [16, 16]),
            ],
        }
    }

    pub fn set_palette(&mut self, palette: &crate::theme::Palette) {
        *self = Self::with_palette(palette);
    }
    /// Windows maps the system Grabbing icon to arrows, so use the Tabler
    /// grasping hand with a contrasting native-cursor outline.
    pub fn hud_drag(&self) -> Option<egui::CustomCursorImage> {
        self.hud_drag.clone()
    }
    /// Windows falls back to its ordinary arrow for Copy. Keep the copy / move
    /// badges visible like LLToolDragAndDrop::acceptanceToCursor (Firestorm).
    pub fn inventory_drag(&self, cursor: egui::CursorIcon) -> Option<egui::CustomCursorImage> {
        match cursor {
            egui::CursorIcon::Copy => self.inventory_copy.clone(),
            egui::CursorIcon::Move => self.inventory_move.clone(),
            egui::CursorIcon::Grabbing => self.hud_drag.clone(),
            _ => None,
        }
    }
    /// LLToolPipette uses UI_CURSOR_PIPETTE; the tip of the Tabler pipette is the hotspot.
    pub fn pipette(&self) -> Option<egui::CustomCursorImage> {
        self.pipette.clone()
    }
    /// LLToolCamera::handleHover shows which drag the modifiers give
    /// (UI_CURSOR_TOOLZOOMIN / TOOLCAMERA / TOOLPAN); the middle of the icon
    /// is the point the camera locks on.
    pub fn camera(&self, mode: crate::camera::ToolMode) -> Option<egui::CustomCursorImage> {
        use crate::camera::ToolMode;
        let index = match mode {
            ToolMode::Zoom => 0,
            ToolMode::Orbit => 1,
            ToolMode::Pan => 2,
        };
        self.camera[index].clone()
    }
    /// Cursor of a click action (LLToolPie::handleHover). Firestorm draws Buy
    /// and Pay alike; here each has the icon of its menu entry.
    pub fn image(&self, action: Action, playing: bool) -> Option<egui::CustomCursorImage> {
        match action {
            Action::Touch => self.touch.clone(),
            Action::Sit => self.sit.clone(),
            Action::Buy => self.buy.clone(),
            Action::Pay => self.pay.clone(),
            Action::Open => self.open.clone(),
            Action::Play if playing => self.pause.clone(),
            Action::OpenMedia if playing => self.media_open.clone(),
            Action::Play | Action::OpenMedia => self.play.clone(),
            Action::Zoom => self.zoom.clone(),
            Action::Grab => self.hud_drag.clone(),
            Action::Disabled => None,
        }
    }
}

fn hex(color: egui::Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
}

/// Draw a Tabler icon (24 px grid): a dark stroke under the ink drawing gives
/// the contrasting outline, including around filled icons.
fn draw_icon(pixmap: &mut resvg::tiny_skia::Pixmap, svg: &[u8], palette: &crate::theme::Palette, scale: f32, offset: f32) -> Option<()> {
    let svg = std::str::from_utf8(svg).ok()?;
    let options = resvg::usvg::Options::default();
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(offset, offset);
    for (color, width) in [(palette.bar, 4), (palette.ink, 2)] {
        let source = if svg.contains("icons-tabler-filled") {
            svg.replace(
                "fill=\"currentColor\"",
                &format!(
                    "fill=\"{}\" stroke=\"{}\" stroke-width=\"{}\" stroke-linejoin=\"round\"",
                    hex(color),
                    hex(color),
                    width - 2
                ),
            )
        } else {
            svg.replace("currentColor", &hex(color))
                .replace("stroke-width=\"2\"", &format!("stroke-width=\"{width}\""))
        };
        let tree = resvg::usvg::Tree::from_data(source.as_bytes(), &options).ok()?;
        resvg::render(&tree, transform, &mut pixmap.as_mut());
    }
    Some(())
}

/// Draw the small arrow of the badge cursors (Tabler's filled pointer), its
/// tip at (1, 1).
fn draw_arrow(pixmap: &mut resvg::tiny_skia::Pixmap, palette: &crate::theme::Palette) -> Option<()> {
    let svg = std::str::from_utf8(include_bytes!("../assets/icons/tabler/filled/pointer.svg")).ok()?;
    let options = resvg::usvg::Options::default();
    let transform = resvg::tiny_skia::Transform::from_scale(0.62, 0.62).post_translate(-1.2, -1.2);
    let bar = hex(palette.bar);
    let outline = format!("fill=\"{bar}\" stroke=\"{bar}\" stroke-width=\"3.2\" stroke-linejoin=\"round\"");
    let ink = format!("fill=\"{}\"", hex(palette.ink));
    for fill in [outline, ink] {
        let source = svg.replace("fill=\"currentColor\"", &fill);
        let tree = resvg::usvg::Tree::from_data(source.as_bytes(), &options).ok()?;
        resvg::render(&tree, transform, &mut pixmap.as_mut());
    }
    Some(())
}

fn finish(pixmap: &resvg::tiny_skia::Pixmap, hotspot: [u16; 2]) -> egui::CustomCursorImage {
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
    egui::CustomCursorImage {
        rgba: rgba.into(),
        size: [32, 32],
        hotspot,
    }
}

/// A Tabler outline icon alone as a 32 px cursor.
fn tabler_image(svg: &[u8], palette: &crate::theme::Palette, hotspot: [u16; 2]) -> Option<egui::CustomCursorImage> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(32, 32)?;
    draw_icon(&mut pixmap, svg, palette, 32.0 / 24.0, 0.0)?;
    Some(finish(&pixmap, hotspot))
}

/// An arrow whose tip is the clicked point, with the action's icon as a
/// badge at its lower right (the layout of Firestorm's action cursors).
fn badge_image(svg: &[u8], palette: &crate::theme::Palette) -> Option<egui::CustomCursorImage> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(32, 32)?;
    draw_icon(&mut pixmap, svg, palette, 0.86, 11.0)?;
    draw_arrow(&mut pixmap, palette)?;
    Some(finish(&pixmap, [1, 1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opaque(image: &egui::CustomCursorImage, x: usize, y: usize) -> bool {
        image.rgba[(y * 32 + x) * 4 + 3] > 0
    }

    #[test]
    fn hotspots_lie_on_the_drawings() {
        let c = Cursors::default();
        let hand = c.hud_drag().unwrap();
        assert_eq!(hand.size, [32, 32]);
        assert_eq!(hand.hotspot, [16, 16]);
        assert!(hand.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
        let pipette = c.pipette().unwrap();
        assert_eq!(pipette.size, [32, 32]);
        assert_eq!(pipette.hotspot, [5, 27]);
        assert!(opaque(&pipette, 5, 27), "the hotspot must lie on the pipette tip");
        assert!(std::sync::Arc::ptr_eq(&pipette.rgba, &c.pipette().unwrap().rgba));
        use crate::camera::ToolMode;
        let zoom = c.camera(ToolMode::Zoom).unwrap();
        assert_eq!((zoom.size, zoom.hotspot), ([32, 32], [16, 16]));
        assert!(opaque(&zoom, 16, 16), "the hotspot must lie on the focus dot");
        for mode in [ToolMode::Orbit, ToolMode::Pan] {
            let image = c.camera(mode).unwrap();
            assert_eq!(image.hotspot, [16, 16]);
            assert_ne!(image.rgba, zoom.rgba);
        }
        let touch = c.image(Action::Touch, false).unwrap();
        assert_eq!(touch.hotspot, [12, 4]);
        assert!(opaque(&touch, 12, 4), "the hotspot must lie on the fingertip");
        assert!(!opaque(&touch, 12, 0), "nothing is drawn above the fingertip");
    }

    #[test]
    fn action_cursors_point_with_the_arrow_tip() {
        let c = Cursors::default();
        let mut seen: Vec<egui::CustomCursorImage> = Vec::new();
        for (action, playing) in [
            (Action::Sit, false),
            (Action::Buy, false),
            (Action::Pay, false),
            (Action::Open, false),
            (Action::Play, false),
            (Action::Play, true),
            (Action::OpenMedia, true),
            (Action::Zoom, false),
        ] {
            let image = c.image(action, playing).unwrap();
            assert_eq!((image.size, image.hotspot), ([32, 32], [1, 1]), "{action:?}");
            assert!(opaque(&image, 1, 1), "{action:?}: the arrow tip is the hotspot");
            assert!(opaque(&image, 3, 3), "{action:?}: the arrow is drawn");
            assert!(seen.iter().all(|other| other.rgba != image.rgba), "{action:?} has its own badge");
            seen.push(image);
        }
        // a media that is not playing yet starts with the same click
        assert_eq!(
            c.image(Action::OpenMedia, false).unwrap().rgba,
            c.image(Action::Play, false).unwrap().rgba
        );
        // a physical object is grabbed with the hand that moves HUDs
        assert!(std::sync::Arc::ptr_eq(
            &c.image(Action::Grab, false).unwrap().rgba,
            &c.hud_drag().unwrap().rgba
        ));
        assert!(c.image(Action::Disabled, false).is_none());
    }

    #[test]
    fn inventory_cursors_keep_distinct_cached_badges_and_leave_rejection_to_the_system() {
        let c = Cursors::default();
        let copy = c.inventory_drag(egui::CursorIcon::Copy).unwrap();
        let moved = c.inventory_drag(egui::CursorIcon::Move).unwrap();
        for image in [&copy, &moved] {
            assert_eq!((image.size, image.hotspot), ([32, 32], [1, 1]));
            assert!(opaque(image, 1, 1));
            assert!(
                image.rgba.as_chunks::<4>().0.iter().skip(16 * 32).any(|p| p[3] > 0),
                "visible badge below the arrow"
            );
        }
        assert_ne!(copy.rgba, moved.rgba);
        assert!(std::sync::Arc::ptr_eq(
            &copy.rgba,
            &c.inventory_drag(egui::CursorIcon::Copy).unwrap().rgba
        ));
        assert!(std::sync::Arc::ptr_eq(
            &moved.rgba,
            &c.inventory_drag(egui::CursorIcon::Move).unwrap().rgba
        ));
        assert!(std::sync::Arc::ptr_eq(
            &c.hud_drag().unwrap().rgba,
            &c.inventory_drag(egui::CursorIcon::Grabbing).unwrap().rgba
        ));
        assert!(c.inventory_drag(egui::CursorIcon::NotAllowed).is_none());
        assert!(c.inventory_drag(egui::CursorIcon::Default).is_none());
    }
}

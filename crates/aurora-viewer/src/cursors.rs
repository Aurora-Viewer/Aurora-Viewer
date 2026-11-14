//! World cursors drawn from Tabler outline icons, in the skin's ink with a
//! dark outline. Firestorm shows its own artwork for the same states
//! (LLToolPie click actions, LLToolCamera, LLToolPipette); Aurora keeps its
//! rules and replaces the drawings with the interface's icons, at the
//! human's request.
use crate::interaction::Action;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum PointerLook {
    Arrow,
    Crosshair,
    Ring,
    Heart,
    Sparkle,
    Bow,
    /// The classic white pointer, rendered at the selected size.
    Original,
    /// Defer to the operating system's native pointer and its size setting.
    #[default]
    System,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum TouchLook {
    #[default]
    Finger,
    Click,
    Move,
    Heart,
    Sparkle,
    NormalPointer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Preferences {
    /// Native pointer bitmap size in physical pixels.
    pub size: u16,
    pub look: PointerLook,
    /// Physical pixel size for the in-world touch hand.
    pub touch_size: u16,
    pub touch_look: TouchLook,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            size: 32,
            look: PointerLook::System,
            touch_size: 32,
            touch_look: TouchLook::Finger,
        }
    }
}

impl Preferences {
    pub fn size(&self) -> u32 {
        self.size.clamp(16, 64).into()
    }
}

pub struct Cursors {
    pointer: Option<egui::CustomCursorImage>,
    preferences: Preferences,
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
        Self::with_preferences(palette, &Preferences::default())
    }

    pub fn with_preferences(palette: &crate::theme::Palette, preferences: &Preferences) -> Self {
        macro_rules! icon {
            ($name:literal) => {
                include_bytes!(concat!("../assets/icons/tabler/outline/", $name, ".svg"))
            };
        }
        Self {
            pointer: pointer_image(palette, preferences),
            preferences: preferences.clone(),
            // the fingertip is the hotspot, like the system hand
            touch: touch_image(palette, preferences),
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
        *self = Self::with_preferences(palette, &self.preferences);
    }
    pub fn set_preferences(&mut self, palette: &crate::theme::Palette, preferences: &Preferences) {
        if self.preferences == *preferences {
            return;
        }
        self.pointer = pointer_image(palette, preferences);
        self.touch = touch_image(palette, preferences);
        self.preferences.clone_from(preferences);
    }
    /// The normal arrow used by the platform when no special action cursor applies.
    pub fn pointer(&self) -> Option<egui::CustomCursorImage> {
        self.pointer.clone()
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
        size: [pixmap.width() as u16, pixmap.height() as u16],
        hotspot,
    }
}

const POINTER_RING: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8" fill="none" stroke="currentColor" stroke-width="2"/><circle cx="12" cy="12" r="1.5" fill="currentColor"/></svg>"#;

fn pointer_image(palette: &crate::theme::Palette, preferences: &Preferences) -> Option<egui::CustomCursorImage> {
    let size = preferences.size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
    let hotspot = match preferences.look {
        PointerLook::Arrow => {
            draw_pointer_arrow(&mut pixmap, palette)?;
            let point = ((size as f32 / 32.0).round() as u16).max(1);
            [point, point]
        }
        PointerLook::Crosshair => {
            draw_icon(
                &mut pixmap,
                include_bytes!("../assets/icons/tabler/outline/crosshair.svg"),
                palette,
                size as f32 / 24.0,
                0.0,
            )?;
            [(size / 2) as u16; 2]
        }
        PointerLook::Ring => {
            draw_icon(&mut pixmap, POINTER_RING, palette, size as f32 / 24.0, 0.0)?;
            [(size / 2) as u16; 2]
        }
        PointerLook::Heart => {
            draw_pointer_arrow(&mut pixmap, palette)?;
            draw_pointer_badge(&mut pixmap, include_bytes!("../assets/icons/tabler/filled/heart.svg"), palette)?;
            [1, 1]
        }
        PointerLook::Sparkle => {
            draw_pointer_arrow(&mut pixmap, palette)?;
            draw_pointer_badge(&mut pixmap, include_bytes!("../assets/icons/tabler/filled/sparkles.svg"), palette)?;
            [1, 1]
        }
        PointerLook::Bow => {
            draw_pointer_arrow(&mut pixmap, palette)?;
            draw_pointer_badge(&mut pixmap, POINTER_BOW, palette)?;
            [1, 1]
        }
        PointerLook::Original => {
            draw_classic_pointer(&mut pixmap, palette)?;
            [((size * 4) / 32) as u16, ((size * 2) / 32) as u16]
        }
        PointerLook::System => return None,
    };
    Some(finish(&pixmap, hotspot))
}

fn draw_classic_pointer(pixmap: &mut resvg::tiny_skia::Pixmap, palette: &crate::theme::Palette) -> Option<()> {
    // A contrasting classic pointer silhouette, using the active palette so it
    // remains visible with custom themes and high contrast.
    let fill = hex(palette.ink);
    let outline = hex(palette.bar);
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 32 32\"><path d=\"M4 2v23l6-6 5 10 4-2-5-10h9L4 2z\" fill=\"{fill}\" stroke=\"{outline}\" stroke-width=\"2.5\" stroke-linejoin=\"round\"/><path d=\"M5.2 4.2v17.9l4.2-4.2 5.1 10.1 1.6-.8-5.2-10.5h7.6L5.2 4.2z\" fill=\"{fill}\" stroke=\"{fill}\" stroke-width=\".6\" stroke-linejoin=\"round\"/></svg>"
    );
    let tree = resvg::usvg::Tree::from_data(svg.as_bytes(), &resvg::usvg::Options::default()).ok()?;
    let scale = pixmap.width() as f32 / 32.0;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(())
}

const POINTER_BOW: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 11C9 7 4 6 3 7c-1 2 1 6 8 6M12 11c3-4 8-5 9-4 1 2-1 6-8 6M12 11l-3 9 3-2 3 2-3-9z" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/><circle cx="12" cy="11" r="2" fill="currentColor"/></svg>"#;

fn touch_image(palette: &crate::theme::Palette, preferences: &Preferences) -> Option<egui::CustomCursorImage> {
    use TouchLook::*;

    let size = preferences.touch_size.clamp(16, 64).into();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
    if preferences.touch_look == NormalPointer {
        draw_classic_pointer(&mut pixmap, palette)?;
        let hotspot = [((4 * size) / 32) as u16, ((2 * size) / 32) as u16];
        return Some(finish(&pixmap, hotspot));
    }
    let hand: &[u8] = match preferences.touch_look {
        Finger | Heart | Sparkle | NormalPointer => include_bytes!("../assets/icons/tabler/outline/hand-finger.svg"),
        Click => include_bytes!("../assets/icons/tabler/outline/hand-click.svg"),
        Move => include_bytes!("../assets/icons/tabler/outline/hand-move.svg"),
    };
    draw_icon(&mut pixmap, hand, palette, size as f32 / 24.0, 0.0)?;
    match preferences.touch_look {
        Heart => draw_touch_badge(&mut pixmap, include_bytes!("../assets/icons/tabler/filled/heart.svg"), palette)?,
        Sparkle => draw_touch_badge(&mut pixmap, include_bytes!("../assets/icons/tabler/filled/sparkles.svg"), palette)?,
        Finger | Click | Move | NormalPointer => {}
    }
    let hotspot = [((12 * size) / 32).min(size - 1) as u16, ((4 * size) / 32).min(size - 1) as u16];
    Some(finish(&pixmap, hotspot))
}

fn draw_touch_badge(pixmap: &mut resvg::tiny_skia::Pixmap, svg: &[u8], palette: &crate::theme::Palette) -> Option<()> {
    let size = pixmap.width() as f32;
    draw_icon(pixmap, svg, palette, size / 24.0 * 0.38, size * 0.58)
}

fn draw_pointer_badge(pixmap: &mut resvg::tiny_skia::Pixmap, svg: &[u8], palette: &crate::theme::Palette) -> Option<()> {
    let scale = pixmap.width() as f32 / 32.0;
    draw_icon(pixmap, svg, palette, 0.48 * scale, 14.0 * scale)
}

fn draw_pointer_arrow(pixmap: &mut resvg::tiny_skia::Pixmap, palette: &crate::theme::Palette) -> Option<()> {
    let svg = std::str::from_utf8(include_bytes!("../assets/icons/tabler/filled/pointer.svg")).ok()?;
    let options = resvg::usvg::Options::default();
    let scale = pixmap.width() as f32 / 32.0;
    let transform = resvg::tiny_skia::Transform::from_scale(0.62 * scale, 0.62 * scale).post_translate(-1.2 * scale, -1.2 * scale);
    let bar = hex(palette.bar);
    let outline = format!(
        "fill=\"{bar}\" stroke=\"{bar}\" stroke-width=\"{}\" stroke-linejoin=\"round\"",
        3.2 * scale
    );
    let ink = format!("fill=\"{}\"", hex(palette.ink));
    for fill in [outline, ink] {
        let source = svg.replace("fill=\"currentColor\"", &fill);
        let tree = resvg::usvg::Tree::from_data(source.as_bytes(), &options).ok()?;
        resvg::render(&tree, transform, &mut pixmap.as_mut());
    }
    Some(())
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
        image.rgba[(y * image.size[0] as usize + x) * 4 + 3] > 0
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
    fn pointer_preferences_resize_the_pointer_and_keep_each_look_hotspot_visible() {
        let palette = crate::theme::Theme::default().palette();
        for size in [16, 32, 48, 64] {
            for look in [
                PointerLook::Arrow,
                PointerLook::Crosshair,
                PointerLook::Ring,
                PointerLook::Heart,
                PointerLook::Sparkle,
                PointerLook::Bow,
                PointerLook::Original,
            ] {
                let image = pointer_image(
                    &palette,
                    &Preferences {
                        size,
                        look,
                        ..Default::default()
                    },
                )
                .expect("pointer image");
                assert_eq!(image.size, [size; 2]);
                let point = image.hotspot;
                assert!(opaque(&image, point[0] as usize, point[1] as usize), "{look:?} hotspot at {size}px");
                assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
            }
        }
        assert_eq!(Preferences::default().look, PointerLook::System);
        assert_eq!(Preferences::default().touch_look, TouchLook::Finger);
        assert!(pointer_image(&palette, &Preferences::default()).is_none());
        let arrow = pointer_image(
            &palette,
            &Preferences {
                look: PointerLook::Arrow,
                ..Default::default()
            },
        )
        .unwrap();
        let crosshair = pointer_image(
            &palette,
            &Preferences {
                size: 32,
                look: PointerLook::Crosshair,
                ..Default::default()
            },
        )
        .unwrap();
        let ring = pointer_image(
            &palette,
            &Preferences {
                size: 32,
                look: PointerLook::Ring,
                ..Default::default()
            },
        )
        .unwrap();
        let heart = pointer_image(
            &palette,
            &Preferences {
                size: 32,
                look: PointerLook::Heart,
                ..Default::default()
            },
        )
        .unwrap();
        let sparkle = pointer_image(
            &palette,
            &Preferences {
                size: 32,
                look: PointerLook::Sparkle,
                ..Default::default()
            },
        )
        .unwrap();
        let bow = pointer_image(
            &palette,
            &Preferences {
                size: 32,
                look: PointerLook::Bow,
                ..Default::default()
            },
        )
        .unwrap();
        assert_ne!(arrow.rgba, crosshair.rgba);
        assert_ne!(crosshair.rgba, ring.rgba);
        assert_ne!(arrow.rgba, heart.rgba);
        assert_ne!(heart.rgba, sparkle.rgba);
        assert_ne!(sparkle.rgba, bow.rgba);
        let classic = pointer_image(
            &palette,
            &Preferences {
                size: 32,
                look: PointerLook::Original,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(classic.size, [32, 32]);
        assert_ne!(classic.rgba, arrow.rgba);
        let fill = classic.rgba.as_chunks::<4>().0[10 * 32 + 7];
        assert!(
            u16::from(fill[0]) + u16::from(fill[1]) + u16::from(fill[2]) > 450,
            "classic arrow fill should be light"
        );
        assert!(
            pointer_image(
                &palette,
                &Preferences {
                    look: PointerLook::System,
                    ..Default::default()
                }
            )
            .is_none()
        );
    }

    #[test]
    fn changing_pointer_preferences_keeps_fixed_action_cursors_cached() {
        let palette = crate::theme::Theme::default().palette();
        let mut cursors = Cursors::with_palette(&palette);
        let pipette = cursors.pipette().unwrap();
        let preferences = Preferences {
            size: 48,
            look: PointerLook::Arrow,
            ..Preferences::default()
        };

        cursors.set_preferences(&palette, &preferences);

        assert_eq!(cursors.pointer().unwrap().size, [48, 48]);
        assert!(std::sync::Arc::ptr_eq(&pipette.rgba, &cursors.pipette().unwrap().rgba));
        let unchanged = cursors.pointer().unwrap();
        cursors.set_preferences(&palette, &preferences);
        assert!(std::sync::Arc::ptr_eq(&unchanged.rgba, &cursors.pointer().unwrap().rgba));
    }

    #[test]
    fn touch_cursor_preferences_resize_and_change_the_hover_hand() {
        let palette = crate::theme::Theme::default().palette();
        let mut images = Vec::new();
        for size in [16, 32, 48, 64] {
            for look in [
                TouchLook::Finger,
                TouchLook::Click,
                TouchLook::Move,
                TouchLook::Heart,
                TouchLook::Sparkle,
                TouchLook::NormalPointer,
            ] {
                let image = touch_image(
                    &palette,
                    &Preferences {
                        touch_size: size,
                        touch_look: look,
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(image.size, [size; 2]);
                assert!(
                    opaque(&image, image.hotspot[0] as usize, image.hotspot[1] as usize),
                    "{look:?} hotspot at {size}px"
                );
                images.push(image);
            }
        }
        assert!(images.windows(2).all(|pair| pair[0].rgba != pair[1].rgba));
        let mouse = touch_image(
            &palette,
            &Preferences {
                touch_look: TouchLook::NormalPointer,
                ..Default::default()
            },
        )
        .unwrap();
        let classic = pointer_image(
            &palette,
            &Preferences {
                look: PointerLook::Original,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(mouse.rgba, classic.rgba, "touch mouse pointer uses the same classic arrow art");
        assert_eq!(mouse.hotspot, classic.hotspot);
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

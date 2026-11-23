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
    /// The larger Tabler arrow in the active theme.
    Original,
    /// Defer to the operating system's native pointer and its size setting.
    /// The decorative looks of the test builds (Bow, Heart, Sparkle) were
    /// dropped: Accessibility keeps the sizes and the plain looks only, so a
    /// saved one loads as the default.
    #[default]
    #[serde(alias = "Bow", alias = "Heart", alias = "Sparkle")]
    System,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum TouchLook {
    /// A saved decorative hand of the test builds (Heart, Sparkle) loads as
    /// the default pointing hand.
    #[default]
    #[serde(alias = "Heart", alias = "Sparkle")]
    Finger,
    Click,
    Move,
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

/// Draw Tabler's filled pointer and return the pixel at its tip.
fn draw_arrow(pixmap: &mut resvg::tiny_skia::Pixmap, palette: &crate::theme::Palette, scale: f32) -> Option<[u16; 2]> {
    let svg = std::str::from_utf8(include_bytes!("../assets/icons/tabler/filled/pointer.svg")).ok()?;
    let options = resvg::usvg::Options::default();
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
    Some([(scale.round() as u16).max(1); 2])
}

/// The theme arrow takes up more of the bitmap than the compact arrow.
const THEME_ARROW_SCALE: f32 = 1.45;

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

fn pointer_image(palette: &crate::theme::Palette, preferences: &Preferences) -> Option<egui::CustomCursorImage> {
    let size = preferences.size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
    let scale = size as f32 / 32.0;
    let hotspot = match preferences.look {
        PointerLook::Arrow => draw_arrow(&mut pixmap, palette, scale)?,
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
            draw_icon(
                &mut pixmap,
                include_bytes!("../assets/icons/tabler/outline/circle-dot.svg"),
                palette,
                size as f32 / 24.0,
                0.0,
            )?;
            [(size / 2) as u16; 2]
        }
        PointerLook::Original => draw_arrow(&mut pixmap, palette, scale * THEME_ARROW_SCALE)?,
        PointerLook::System => return None,
    };
    Some(finish(&pixmap, hotspot))
}

fn touch_image(palette: &crate::theme::Palette, preferences: &Preferences) -> Option<egui::CustomCursorImage> {
    use TouchLook::*;

    let size = preferences.touch_size.clamp(16, 64).into();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
    if preferences.touch_look == NormalPointer {
        let hotspot = draw_arrow(&mut pixmap, palette, size as f32 / 32.0 * THEME_ARROW_SCALE)?;
        return Some(finish(&pixmap, hotspot));
    }
    let hand: &[u8] = match preferences.touch_look {
        Finger | NormalPointer => include_bytes!("../assets/icons/tabler/outline/hand-finger.svg"),
        Click => include_bytes!("../assets/icons/tabler/outline/hand-click.svg"),
        Move => include_bytes!("../assets/icons/tabler/outline/hand-move.svg"),
    };
    draw_icon(&mut pixmap, hand, palette, size as f32 / 24.0, 0.0)?;
    let hotspot = [((12 * size) / 32).min(size - 1) as u16, ((4 * size) / 32).min(size - 1) as u16];
    Some(finish(&pixmap, hotspot))
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
    let hotspot = draw_arrow(&mut pixmap, palette, 1.0)?;
    Some(finish(&pixmap, hotspot))
}

/// The cursor the window was last asked for. egui-winit sends a cursor
/// picture again at every frame (a system cursor only when it changes), and
/// each time Wayland allocates its pixels and commits its surface: hundreds
/// of them a second stall the desktop for as long as the picture is up, with
/// the viewer's own frames on time. The pictures are sent from here instead,
/// when they change; egui-winit keeps the system cursors.
#[derive(Default)]
pub struct Shown {
    /// The picture on screen (its number in `made`), None for a system cursor.
    picture: Option<u32>,
    /// The system cursor egui-winit has sent: it sends another one, and the
    /// same one again after a time without a pointer.
    icon: Option<egui::CursorIcon>,
    /// egui-winit has a pointer position: without one it leaves the cursor
    /// alone, and forgets what it sent.
    pointer: bool,
    /// The finger egui-winit follows as its pointer (its `pointer_touch_id`).
    finger: Option<u64>,
    /// The window's cursor of each picture, by its pixels.
    made: Vec<(std::sync::Arc<[u8]>, winit::window::CustomCursor, u32)>,
    numbered: u32,
    refused: bool,
}

/// What a frame has left to send once egui-winit has sent its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    Nothing,
    /// The frame's picture.
    Picture,
    /// The frame's system cursor: egui-winit believes it is still up.
    System,
}

impl Shown {
    /// A window event egui-winit receives: its pointer position follows
    /// these, and of several fingers the first one only (its `on_touch`).
    pub fn event(&mut self, event: &winit::event::WindowEvent) {
        use winit::event::{TouchPhase, WindowEvent};
        match event {
            WindowEvent::CursorMoved { .. } => self.pointer = true,
            WindowEvent::CursorLeft { .. } => self.pointer = false,
            WindowEvent::Touch(touch) if self.finger.is_none_or(|id| id == touch.id) => match touch.phase {
                TouchPhase::Started => {
                    self.finger = Some(touch.id);
                    self.pointer = true;
                }
                TouchPhase::Moved => self.pointer = true,
                TouchPhase::Ended | TouchPhase::Cancelled => {
                    self.finger = None;
                    self.pointer = false;
                }
            },
            WindowEvent::ScaleFactorChanged { .. } => self.rescaled(),
            _ => {}
        }
    }

    /// The window's scale changed. Wayland sizes a picture from the scale
    /// the window has when it is sent (winit's `apply_custom_cursor`), and
    /// nothing sends it again by itself: the one on screen goes out anew.
    fn rescaled(&mut self) {
        self.picture = None;
    }

    /// A new egui-winit state, or a new window: nothing is sent yet, and it
    /// has no pointer before the next move.
    pub fn forget(&mut self) {
        self.picture = None;
        self.icon = None;
        self.pointer = false;
        self.finger = None;
    }

    /// The window's cursor of a picture and its number, made once.
    pub fn cursor(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        image: &egui::CustomCursorImage,
    ) -> Option<(u32, winit::window::CustomCursor)> {
        if let Some((_, cursor, number)) = self.made.iter().find(|(rgba, ..)| std::sync::Arc::ptr_eq(rgba, &image.rgba)) {
            return Some((*number, cursor.clone()));
        }
        let [w, h] = image.size;
        let [x, y] = image.hotspot;
        match winit::window::CustomCursor::from_rgba(image.rgba.to_vec(), w, h, x, y) {
            Ok(source) => {
                // (a change of skin draws them all again)
                if self.made.len() >= 64 {
                    self.made.clear();
                }
                self.numbered += 1;
                let cursor = event_loop.create_custom_cursor(source);
                self.made.push((image.rgba.clone(), cursor.clone(), self.numbered));
                Some((self.numbered, cursor))
            }
            Err(e) => {
                if !std::mem::replace(&mut self.refused, true) {
                    log::warn!("cursor: a picture was refused ({e}), the system cursor stays");
                }
                None
            }
        }
    }

    /// A frame whose `icon` egui-winit was just given, without its picture:
    /// what is left to send.
    pub fn step(&mut self, picture: Option<u32>, icon: egui::CursorIcon) -> Change {
        if !self.pointer {
            self.picture = None;
            self.icon = None;
            return Change::Nothing;
        }
        // egui-winit sent the system cursor at this frame, over a picture too
        let sent = self.icon.replace(icon) != Some(icon);
        match picture {
            Some(_) if sent || self.picture != picture => {
                self.picture = picture;
                Change::Picture
            }
            Some(_) => Change::Nothing,
            None if self.picture.take().is_some() && !sent => Change::System,
            None => Change::Nothing,
        }
    }
}

/// Show a system cursor, as egui-winit does (its `translate_cursor` is
/// private).
pub fn set_system(window: &winit::window::Window, icon: egui::CursorIcon) {
    match system(icon) {
        Some(cursor) => {
            window.set_cursor_visible(true);
            window.set_cursor(cursor);
        }
        None => window.set_cursor_visible(false),
    }
}

fn system(icon: egui::CursorIcon) -> Option<winit::window::CursorIcon> {
    use egui::CursorIcon as E;
    use winit::window::CursorIcon as W;
    Some(match icon {
        E::None => return None,
        E::Default => W::Default,
        E::ContextMenu => W::ContextMenu,
        E::Help => W::Help,
        E::PointingHand => W::Pointer,
        E::Progress => W::Progress,
        E::Wait => W::Wait,
        E::Cell => W::Cell,
        E::Crosshair => W::Crosshair,
        E::Text => W::Text,
        E::VerticalText => W::VerticalText,
        E::Alias => W::Alias,
        E::Copy => W::Copy,
        E::Move => W::Move,
        E::NoDrop => W::NoDrop,
        E::NotAllowed => W::NotAllowed,
        E::Grab => W::Grab,
        E::Grabbing => W::Grabbing,
        E::AllScroll => W::AllScroll,
        E::ResizeHorizontal => W::EwResize,
        E::ResizeNeSw => W::NeswResize,
        E::ResizeNwSe => W::NwseResize,
        E::ResizeVertical => W::NsResize,
        E::ResizeEast => W::EResize,
        E::ResizeSouthEast => W::SeResize,
        E::ResizeSouth => W::SResize,
        E::ResizeSouthWest => W::SwResize,
        E::ResizeWest => W::WResize,
        E::ResizeNorthWest => W::NwResize,
        E::ResizeNorth => W::NResize,
        E::ResizeNorthEast => W::NeResize,
        E::ResizeColumn => W::ColResize,
        E::ResizeRow => W::RowResize,
        E::ZoomIn => W::ZoomIn,
        E::ZoomOut => W::ZoomOut,
    })
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
            for look in [PointerLook::Arrow, PointerLook::Crosshair, PointerLook::Ring, PointerLook::Original] {
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
        assert_ne!(arrow.rgba, crosshair.rgba);
        assert_ne!(crosshair.rgba, ring.rgba);
        let theme_arrow = pointer_image(
            &palette,
            &Preferences {
                size: 32,
                look: PointerLook::Original,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(theme_arrow.size, [32, 32]);
        assert_ne!(theme_arrow.rgba, arrow.rgba);
        let light = crate::theme::Palette {
            ink: palette.bar,
            bar: palette.ink,
            ..palette
        };
        let light_arrow = pointer_image(
            &light,
            &Preferences {
                look: PointerLook::Original,
                ..Default::default()
            },
        )
        .unwrap();
        assert_ne!(light_arrow.rgba, theme_arrow.rgba);
        assert_eq!(light_arrow.hotspot, theme_arrow.hotspot);
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
    fn dropped_decorative_looks_load_as_the_defaults() {
        for old in ["Bow", "Heart", "Sparkle"] {
            let look = serde_json::from_str::<PointerLook>(&format!("\"{old}\"")).unwrap();
            assert_eq!(look, PointerLook::System, "pointer {old}");
        }
        for old in ["Heart", "Sparkle"] {
            let look = serde_json::from_str::<TouchLook>(&format!("\"{old}\"")).unwrap();
            assert_eq!(look, TouchLook::Finger, "touch {old}");
        }
        let saved: Preferences = serde_json::from_str(r#"{"size":40,"look":"Heart","touch_size":48,"touch_look":"Sparkle"}"#).unwrap();
        assert_eq!(
            saved,
            Preferences {
                size: 40,
                look: PointerLook::System,
                touch_size: 48,
                touch_look: TouchLook::Finger,
            },
            "the sizes stay, the looks fall back"
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
            for look in [TouchLook::Finger, TouchLook::Click, TouchLook::Move, TouchLook::NormalPointer] {
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
        let theme_arrow = pointer_image(
            &palette,
            &Preferences {
                look: PointerLook::Original,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(mouse.rgba, theme_arrow.rgba, "the touch pointer uses the same theme arrow");
        assert_eq!(mouse.hotspot, theme_arrow.hotspot);
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
    fn with_pointer() -> Shown {
        Shown {
            pointer: true,
            ..Default::default()
        }
    }

    #[test]
    fn a_picture_is_sent_once_for_as_long_as_it_stays() {
        let mut shown = with_pointer();
        let sent: Vec<Change> = (0..300).map(|_| shown.step(Some(1), egui::CursorIcon::ZoomIn)).collect();
        assert_eq!(sent[0], Change::Picture);
        assert!(sent[1..].iter().all(|c| *c == Change::Nothing), "sent again while unchanged");
        // another picture, then the first one back
        assert_eq!(shown.step(Some(2), egui::CursorIcon::ZoomIn), Change::Picture);
        assert_eq!(shown.step(Some(2), egui::CursorIcon::ZoomIn), Change::Nothing);
        assert_eq!(shown.step(Some(1), egui::CursorIcon::ZoomIn), Change::Picture);
    }

    #[test]
    fn a_picture_comes_back_over_the_system_cursor_egui_winit_sends() {
        let mut shown = with_pointer();
        assert_eq!(shown.step(Some(1), egui::CursorIcon::ZoomIn), Change::Picture);
        // its fallback icon changes under the same picture: egui-winit sends it
        assert_eq!(shown.step(Some(1), egui::CursorIcon::Move), Change::Picture);
        assert_eq!(shown.step(Some(1), egui::CursorIcon::Move), Change::Nothing);
    }

    #[test]
    fn the_system_cursor_returns_when_the_picture_ends() {
        let mut shown = with_pointer();
        assert_eq!(shown.step(None, egui::CursorIcon::Default), Change::Nothing);
        assert_eq!(shown.step(Some(1), egui::CursorIcon::Default), Change::Picture);
        // egui-winit holds Default as sent: it would leave the picture up
        assert_eq!(shown.step(None, egui::CursorIcon::Default), Change::System);
        assert_eq!(shown.step(None, egui::CursorIcon::Default), Change::Nothing);
        // (it sends a changed one itself)
        assert_eq!(shown.step(Some(1), egui::CursorIcon::Default), Change::Picture);
        assert_eq!(shown.step(None, egui::CursorIcon::Text), Change::Nothing);
    }

    #[test]
    fn nothing_is_sent_without_a_pointer_and_the_picture_returns_with_it() {
        let mut shown = Shown::default();
        assert_eq!(shown.step(Some(1), egui::CursorIcon::ZoomIn), Change::Nothing);
        let moved = winit::event::WindowEvent::CursorMoved {
            device_id: winit::event::DeviceId::dummy(),
            position: winit::dpi::PhysicalPosition::new(10.0, 10.0),
        };
        let left = winit::event::WindowEvent::CursorLeft {
            device_id: winit::event::DeviceId::dummy(),
        };
        shown.event(&moved);
        assert_eq!(shown.step(Some(1), egui::CursorIcon::ZoomIn), Change::Picture);
        assert_eq!(shown.step(Some(1), egui::CursorIcon::ZoomIn), Change::Nothing);
        shown.event(&left);
        assert_eq!(shown.step(Some(1), egui::CursorIcon::ZoomIn), Change::Nothing);
        // back in the window egui-winit sends its icon again, over the picture
        shown.event(&moved);
        assert_eq!(shown.step(Some(1), egui::CursorIcon::ZoomIn), Change::Picture);
        assert_eq!(shown.step(Some(1), egui::CursorIcon::ZoomIn), Change::Nothing);
    }

    #[test]
    fn a_new_state_sends_the_picture_again_at_the_first_move() {
        // (a new window opens under the pointer: its move can come before
        // the next frame, or after it)
        for frame_between in [true, false] {
            let mut shown = with_pointer();
            assert_eq!(shown.step(Some(1), egui::CursorIcon::ZoomIn), Change::Picture);
            shown.forget();
            if frame_between {
                assert_eq!(shown.step(Some(1), egui::CursorIcon::ZoomIn), Change::Nothing);
            }
            shown.event(&winit::event::WindowEvent::CursorMoved {
                device_id: winit::event::DeviceId::dummy(),
                position: winit::dpi::PhysicalPosition::new(10.0, 10.0),
            });
            assert_eq!(
                shown.step(Some(1), egui::CursorIcon::ZoomIn),
                Change::Picture,
                "a frame between: {frame_between}"
            );
        }
    }

    #[test]
    fn the_pointer_follows_the_first_finger_only() {
        let touch = |id, phase| {
            winit::event::WindowEvent::Touch(winit::event::Touch {
                device_id: winit::event::DeviceId::dummy(),
                phase,
                location: winit::dpi::PhysicalPosition::new(10.0, 10.0),
                force: None,
                id,
            })
        };
        use winit::event::TouchPhase::{Ended, Started};
        let mut shown = Shown::default();
        shown.event(&touch(1, Started));
        assert_eq!(shown.step(Some(1), egui::CursorIcon::ZoomIn), Change::Picture);
        // a second finger comes and goes: egui-winit keeps its pointer
        shown.event(&touch(2, Started));
        shown.event(&touch(2, Ended));
        assert_eq!(shown.step(Some(1), egui::CursorIcon::ZoomIn), Change::Nothing);
        // so the picture's end still brings the system cursor back
        assert_eq!(shown.step(None, egui::CursorIcon::ZoomIn), Change::System);
        shown.event(&touch(1, Ended));
        assert_eq!(shown.step(Some(1), egui::CursorIcon::ZoomIn), Change::Nothing);
    }

    #[test]
    fn a_new_scale_sends_the_picture_again_once() {
        let mut shown = with_pointer();
        assert_eq!(shown.step(Some(1), egui::CursorIcon::ZoomIn), Change::Picture);
        shown.rescaled();
        assert_eq!(shown.step(Some(1), egui::CursorIcon::ZoomIn), Change::Picture);
        assert_eq!(shown.step(Some(1), egui::CursorIcon::ZoomIn), Change::Nothing);
        // a system cursor is the system's to size
        assert_eq!(shown.step(None, egui::CursorIcon::ZoomIn), Change::System);
        shown.rescaled();
        assert_eq!(shown.step(None, egui::CursorIcon::ZoomIn), Change::Nothing);
    }

    #[test]
    fn every_system_cursor_has_its_window_cursor() {
        for icon in egui::CursorIcon::ALL {
            assert_eq!(system(icon).is_none(), icon == egui::CursorIcon::None, "{icon:?}");
        }
    }
}

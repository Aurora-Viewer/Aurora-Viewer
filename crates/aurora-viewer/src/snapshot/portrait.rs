//! Auto portrait of an outfit (Fider #447, an Aurora addition: the Legacy
//! Viewer's outfit gallery only takes a snapshot of the current view,
//! LLOutfitGallery / LLFloaterSimpleSnapshot): our avatar framed on its
//! own from its drawn joints, straight in front, on a studio backdrop, for
//! the outfit's free 256 × 256 inventory thumbnail. The pose, the backdrop
//! and the light are local: nothing is sent, others see us as usual.
//!
//! This module holds the rules the window and the app share: the choices,
//! the framing from the joints, the backdrop colours and the flattening of
//! the cut-out photo over them.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// How much of the avatar the photo holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Framing {
    #[default]
    Body,
    Bust,
    Face,
}

/// The pose shown while the portrait is set up, on our screen only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pose {
    /// Arms lowered halfway from the skeleton's T: the outfit's sleeves and
    /// sides both show.
    #[default]
    A,
    /// Arms along the body.
    ArmsDown,
    /// Whatever plays now (the animations, a pose stand, the poser).
    Current,
}

/// Where the camera stands around the avatar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Angle {
    #[default]
    Front,
    ThreeQuarter,
    Profile,
}

/// What is behind the avatar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backdrop {
    #[default]
    Dark,
    Grey,
    Light,
    /// The region around, as it is.
    Scene,
}

/// The light of the photo.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lighting {
    /// The library's midday sky, over whatever the region shows.
    #[default]
    Studio,
    Region,
}

/// The portrait's choices; kept in the settings when "Keep these settings"
/// is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PortraitSettings {
    pub framing: Framing,
    pub pose: Pose,
    pub angle: Angle,
    pub backdrop: Backdrop,
    pub lighting: Lighting,
    /// The other avatars are left out (the backdrop's colours always leave
    /// everything but our avatar out).
    pub hide_others: bool,
    /// The choices come back at the next portrait.
    pub keep: bool,
    /// The paid HD picture sent with the free thumbnail, and its side;
    /// both kept whatever "keep" says (`opening`).
    pub hd: bool,
    pub hd_side: HdSide,
}

/// The paid HD picture (the user's choice, 2026-11-10): besides the free
/// 256 × 256 thumbnail, a texture of this side uploaded into the outfit's
/// folder at the grid's texture upload price, for the showcase's large
/// preview. Both sides are within the plain texture price (1024 × 1024 and
/// under, `upload::Prices::cost`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HdSide {
    #[default]
    Side512,
    Side1024,
}

impl HdSide {
    pub fn side(self) -> u32 {
        match self {
            Self::Side512 => 512,
            Self::Side1024 => 1024,
        }
    }
}

/// The side the confirmed photo is drawn at: the largest HD picture's, so
/// that the HD choice can still be made once the photo is shown; the free
/// thumbnail and a 512 picture are reduced from it.
pub const SHOT_SIDE: u32 = 1024;

/// The description of the HD pictures this viewer uploads: how the
/// showcase finds them among the folder's textures, and which older one a
/// new picture sends to the Trash (one picture an outfit, as
/// LLAppearanceMgr::removeOutfitPhoto removed the previous photo link).
/// Data, never shown as interface text.
pub const HD_PICTURE_DESC: &str = "Outfit HD picture (Aurora Viewer)";

/// Whether the HD picture can be bought now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HdOffer {
    /// Its price (0 on a grid where uploads are free).
    Ready(i32),
    /// The grid's price is not known (not logged in): never promise a
    /// free upload (`upload::Prices`).
    Unknown,
    /// The balance cannot pay it (can_afford_transaction,
    /// ErrorPhotoCannotAfford); the price.
    Short(i32),
}

/// The offer from the texture price and the balance; an unknown balance
/// leaves the check to the server, as "Save to inventory" does.
pub fn hd_offer(cost: Option<i32>, balance: Option<i32>) -> HdOffer {
    match cost {
        None => HdOffer::Unknown,
        Some(cost) if balance.is_some_and(|b| b < cost) => HdOffer::Short(cost),
        Some(cost) => HdOffer::Ready(cost),
    }
}

/// The HD picture's inventory name, from the Legacy Viewer's
/// `outfit_photo_string` ("Photo of "[OUTFIT_NAME]" outfit",
/// panel_outfit_gallery.xml), cut to the 63 bytes an item name holds.
pub fn hd_picture_name(outfit: &str) -> String {
    let mut name = aurora_i18n::tr!("ui-appearance-portrait-hd-item-name", outfit = outfit).to_string();
    if name.len() > 63 {
        let mut end = 63;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
    }
    let name = name.trim().to_owned();
    if name.is_empty() { "Outfit photo".to_owned() } else { name }
}

impl Default for PortraitSettings {
    fn default() -> Self {
        Self {
            framing: Framing::Body,
            pose: Pose::A,
            angle: Angle::Front,
            backdrop: Backdrop::Dark,
            lighting: Lighting::Studio,
            hide_others: true,
            keep: true,
            hd: false,
            hd_side: HdSide::Side512,
        }
    }
}

impl PortraitSettings {
    /// What the next portrait starts from: the kept choices, else the
    /// defaults (with "keep" left as the user set it).
    pub fn opening(saved: &Self) -> Self {
        if saved.keep {
            *saved
        } else {
            Self {
                keep: false,
                hd: saved.hd,
                hd_side: saved.hd_side,
                ..Self::default()
            }
        }
    }
}

/// The library's midday preset (`Panels::time_of_day`), the studio's light.
pub const STUDIO_TIME: u8 = 2;

/// The backdrop's colour, a photo's content (not the interface's theme:
/// the picture keeps it whatever the skin). None: the scene.
pub fn backdrop_rgb(backdrop: Backdrop) -> Option<[u8; 3]> {
    match backdrop {
        Backdrop::Dark => Some([0x1b, 0x1e, 0x26]),
        Backdrop::Grey => Some([0x4a, 0x51, 0x60]),
        Backdrop::Light => Some([0xc9, 0xce, 0xd9]),
        Backdrop::Scene => None,
    }
}

/// The photo's cut-out pixels (straight alpha, as the photo keeps them)
/// laid over `rgb`, opaque.
pub fn flatten(pixels: &mut [u8], rgb: [u8; 3]) {
    for px in pixels.as_chunks_mut::<4>().0 {
        let a = u32::from(px[3]);
        for c in 0..3 {
            px[c] = ((u32::from(px[c]) * a + u32::from(rgb[c]) * (255 - a) + 127) / 255) as u8;
        }
        px[3] = 255;
    }
}

/// How far the shoulders turn down from the skeleton's T, in radians.
/// None: the pose is left alone.
pub fn arms_lowered(pose: Pose) -> Option<f32> {
    match pose {
        Pose::A => Some(42f32.to_radians()),
        Pose::ArmsDown => Some(80f32.to_radians()),
        Pose::Current => None,
    }
}

/// The joints the framing reads, where they are drawn (`Scene::joint_world`).
#[derive(Debug, Clone, Default)]
pub struct Joints {
    pub skull: Vec3,
    pub head: Vec3,
    pub neck: Vec3,
    pub chest: Vec3,
    pub pelvis: Vec3,
    /// The feet and toes: the lowest is the sole's height.
    pub feet: Vec<Vec3>,
    /// The shoulders, elbows and wrists: how wide the arms reach.
    pub arms: Vec<Vec3>,
    /// The way the avatar faces (its root's +X), on the ground plane.
    pub facing: f32,
}

/// The joint names `Joints` is read from (the Legacy Viewer's skeleton,
/// avatar_skeleton.xml).
pub const FEET: [&str; 4] = ["mFootLeft", "mFootRight", "mToeLeft", "mToeRight"];
pub const ARMS: [&str; 6] = [
    "mShoulderLeft",
    "mShoulderRight",
    "mElbowLeft",
    "mElbowRight",
    "mWristLeft",
    "mWristRight",
];

/// The top of the head above the skull joint, and the sole below the
/// lowest foot joint, in metres: the joints sit inside the body.
const CROWN: f32 = 0.11;
const SOLE: f32 = 0.10;
/// The room left around the subject, a share of its half size.
const MARGIN: f32 = 0.10;
/// The hands' own reach past the wrists.
const HAND: f32 = 0.12;

/// The camera's eye and target for a square photo of vertical field of view
/// `fov` (radians): the chosen part of the avatar fills the frame with a
/// small margin, the camera level with its middle, from the chosen side.
pub fn frame(j: &Joints, framing: Framing, angle: Angle, fov: f32) -> (Vec3, Vec3) {
    let turn = match angle {
        Angle::Front => 0.0,
        Angle::ThreeQuarter => 35f32.to_radians(),
        Angle::Profile => 90f32.to_radians(),
    };
    let toward = Quat::from_rotation_z(j.facing + turn) * Vec3::X;
    let right = toward.cross(Vec3::Z).normalize_or_zero();
    let crown = j.skull.z + CROWN;
    let sole = j.feet.iter().map(|f| f.z).fold(f32::INFINITY, f32::min);
    let sole = if sole.is_finite() { sole - SOLE } else { j.pelvis.z - 1.0 };
    let (top, bottom, sides): (f32, f32, Vec<Vec3>) = match framing {
        Framing::Body => {
            let mut sides = j.arms.clone();
            sides.push(j.pelvis);
            sides.push(j.skull);
            (crown, sole, sides)
        }
        Framing::Bust => {
            // the head and the chest down to its middle
            let bottom = j.chest.z - (crown - j.chest.z) * 0.45;
            let shoulders: Vec<Vec3> = j.arms.iter().take(2).copied().chain([j.chest, j.skull]).collect();
            (crown, bottom, shoulders)
        }
        Framing::Face => {
            // the whole head, its hair, and the neck down to the collar
            let bottom = j.neck.z - 0.12;
            let face = vec![j.head + right * 0.16, j.head - right * 0.16];
            (crown + 0.03, bottom, face)
        }
    };
    // across the frame: the sides measured from the middle line of the
    // part framed (a head leant forward stays in the middle)
    let middle = match framing {
        Framing::Body => j.pelvis,
        Framing::Bust => j.chest,
        Framing::Face => j.head,
    }
    .with_z(0.0);
    let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
    for s in &sides {
        let along = (*s - middle).dot(right);
        let reach = if framing == Framing::Body { HAND } else { 0.0 };
        low = low.min(along - reach);
        high = high.max(along + reach);
    }
    if !low.is_finite() {
        (low, high) = (-0.3, 0.3);
    }
    let across = (low + high) * 0.5;
    let half = ((top - bottom) * 0.5).max((high - low) * 0.5).max(0.05) * (1.0 + MARGIN);
    let target = Vec3::new(middle.x, middle.y, (top + bottom) * 0.5) + right * across;
    let distance = half / (fov * 0.5).tan().max(0.05);
    (target + toward * distance, target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standing() -> Joints {
        Joints {
            skull: Vec3::new(0.0, 0.0, 1.70),
            head: Vec3::new(0.0, 0.0, 1.62),
            neck: Vec3::new(0.0, 0.0, 1.50),
            chest: Vec3::new(0.0, 0.0, 1.30),
            pelvis: Vec3::new(0.0, 0.0, 1.0),
            feet: vec![Vec3::new(0.05, 0.1, 0.08), Vec3::new(0.05, -0.1, 0.08)],
            arms: vec![
                Vec3::new(0.0, 0.18, 1.42),
                Vec3::new(0.0, -0.18, 1.42),
                Vec3::new(0.0, 0.4, 1.2),
                Vec3::new(0.0, -0.4, 1.2),
                Vec3::new(0.0, 0.6, 1.0),
                Vec3::new(0.0, -0.6, 1.0),
            ],
            facing: 0.0,
        }
    }

    /// Where a point lands in the square frame: -1..1 on both axes.
    fn project(eye: Vec3, target: Vec3, fov: f32, point: Vec3) -> (f32, f32) {
        let forward = (target - eye).normalize();
        let right = forward.cross(Vec3::Z).normalize();
        let up = right.cross(forward);
        let rel = point - eye;
        let depth = rel.dot(forward);
        let t = (fov * 0.5).tan();
        (rel.dot(right) / depth / t, rel.dot(up) / depth / t)
    }

    #[test]
    fn the_whole_body_fills_the_frame_with_its_margin() {
        let j = standing();
        let fov = 1.0;
        let (eye, target) = frame(&j, Framing::Body, Angle::Front, fov);
        assert!(eye.x > target.x + 1.0, "in front, along the way it faces");
        assert!((eye.z - target.z).abs() < 1e-5, "a level camera");
        let (_, top) = project(eye, target, fov, Vec3::new(0.0, 0.0, j.skull.z + CROWN));
        let (_, bottom) = project(eye, target, fov, Vec3::new(0.0, 0.0, 0.08 - SOLE));
        assert!((top - 0.926).abs() < 0.02 && (bottom + 0.926).abs() < 0.02, "{top} {bottom}");
        for hand in [Vec3::new(0.0, 0.6 + HAND, 1.0), Vec3::new(0.0, -0.6 - HAND, 1.0)] {
            let (x, _) = project(eye, target, fov, hand);
            assert!(x.abs() < 1.0, "the hands inside: {x}");
        }
    }

    #[test]
    fn bust_and_face_come_closer_and_turn_with_the_angle() {
        let j = standing();
        let body = frame(&j, Framing::Body, Angle::Front, 1.0);
        let bust = frame(&j, Framing::Bust, Angle::Front, 1.0);
        let face = frame(&j, Framing::Face, Angle::Front, 1.0);
        let d = |(e, t): (Vec3, Vec3)| e.distance(t);
        assert!(d(body) > d(bust) && d(bust) > d(face));
        assert!(face.1.z > bust.1.z && bust.1.z > body.1.z);
        let side = frame(&j, Framing::Bust, Angle::Profile, 1.0);
        let dir = (side.0 - side.1).normalize();
        assert!(dir.x.abs() < 1e-4 && dir.y.abs() > 0.99, "a quarter turn");
        let mut turned = standing();
        turned.facing = std::f32::consts::FRAC_PI_2;
        let (eye, target) = frame(&turned, Framing::Face, Angle::Front, 1.0);
        assert!((eye - target).normalize().y > 0.99, "in front of where it faces");
    }

    #[test]
    fn flatten_lays_the_cut_out_over_the_backdrop() {
        let mut px = vec![255, 0, 0, 255, 0, 0, 0, 0, 0, 255, 0, 128];
        flatten(&mut px, [10, 20, 30]);
        assert_eq!(&px[0..4], &[255, 0, 0, 255], "the subject kept");
        assert_eq!(&px[4..8], &[10, 20, 30, 255], "the empty background is the backdrop");
        assert_eq!(&px[8..12], &[5, 138, 15, 255], "edges blended");
    }

    #[test]
    fn kept_settings_come_back_and_others_start_over() {
        let mut saved = PortraitSettings {
            framing: Framing::Face,
            backdrop: Backdrop::Light,
            ..Default::default()
        };
        assert_eq!(PortraitSettings::opening(&saved), saved);
        saved.keep = false;
        let opened = PortraitSettings::opening(&saved);
        assert_eq!(opened.framing, Framing::Body);
        assert!(!opened.keep);
        let old: PortraitSettings = serde_json::from_str("{}").expect("old settings");
        assert_eq!(old, PortraitSettings::default());
        assert!(backdrop_rgb(Backdrop::Scene).is_none());
        assert_eq!(arms_lowered(Pose::Current), None);
    }

    #[test]
    fn the_hd_choice_survives_settings_not_kept() {
        let saved = PortraitSettings {
            keep: false,
            hd: true,
            hd_side: HdSide::Side1024,
            framing: Framing::Face,
            ..Default::default()
        };
        let opened = PortraitSettings::opening(&saved);
        assert!(opened.hd);
        assert_eq!(opened.hd_side, HdSide::Side1024);
        assert_eq!(opened.framing, Framing::Body);
        assert!(!PortraitSettings::default().hd, "off unless chosen");
        assert_eq!(HdSide::Side512.side(), 512);
        assert_eq!(HdSide::Side1024.side(), SHOT_SIDE, "the photo is drawn at the largest side");
    }

    #[test]
    fn the_hd_offer_needs_a_known_price_and_the_balance() {
        assert_eq!(hd_offer(None, Some(1000)), HdOffer::Unknown);
        assert_eq!(hd_offer(Some(10), Some(5)), HdOffer::Short(10));
        assert_eq!(hd_offer(Some(10), Some(10)), HdOffer::Ready(10));
        // an unknown balance leaves the check to the server
        assert_eq!(hd_offer(Some(10), None), HdOffer::Ready(10));
        assert_eq!(hd_offer(Some(0), Some(0)), HdOffer::Ready(0));
    }

    #[test]
    fn the_hd_picture_name_fits_an_item_name() {
        let name = hd_picture_name("Velvet evening");
        assert!(name.contains("Velvet evening"));
        let long = hd_picture_name(&"é".repeat(80));
        assert!(long.len() <= 63);
        assert!(!long.is_empty());
    }
}
